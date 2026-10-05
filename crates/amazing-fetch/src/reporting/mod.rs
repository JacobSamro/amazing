//! Native document Reporting endpoints and CSP request delivery.
//! The embedding supplies accepted response headers and binds each capability
//! to its actual Document lifetime. A URL is never a document identity.
mod prepared;
pub mod structured;
mod test_report;
pub use prepared::PreparedCspReportPlan;
pub use test_report::TestReportSnapshot;
#[cfg(test)]
mod tests;
use crate::{
    Body, ClientContext, CredentialsMode, CspDisposition, CspInlineViolation, FetchError,
    HeaderList, HttpUrl, InternalResponse, NetworkService, Origin, RedirectAction, RedirectMode,
    Request, RequestMode, ServiceEvent,
};
use amazing_net::{
    FetchOptions, ParsedUrl,
    dispatch::{NetEvent, RequestId},
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

const MAX_REPORT_BYTES: usize = 64 * 1024;
const MAX_DOCUMENTS: usize = 1024;
const MAX_ENDPOINTS: usize = 64;
static NEXT_SERVICE: AtomicU64 = AtomicU64::new(1);

/// A service-local native capability. It cannot be reconstructed from a URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportingDocument {
    service: u64,
    id: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportRoute {
    LegacyCsp,
    ReportingApi,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    StaleDocument,
    NoEndpoint,
    InvalidEndpoint,
    Limit,
    Policy(FetchError),
    Transport,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportOutcome {
    /// An actual HTTP response was received. Legacy CSP ignores its status;
    /// Reporting API acceptance requires a 2xx response after CORS validation.
    Response {
        status: u16,
        accepted: bool,
    },
    Failed(ReportError),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportFinished {
    pub id: ReportId,
    pub document: ReportingDocument,
    pub route: ReportRoute,
    pub outcome: ReportOutcome,
}
#[derive(Clone)]
struct Document {
    creation_url: ParsedUrl,
    origin: Origin,
    context: ClientContext,
    endpoints: BTreeMap<String, HttpUrl>,
    failures: BTreeMap<String, u64>,
    referrer_policy: crate::ReferrerPolicy,
}
struct Delivery {
    id: ReportId,
    document: ReportingDocument,
    route: ReportRoute,
    endpoint_name: Option<String>,
    endpoint: HttpUrl,
    request: Request,
    context: ClientContext,
    preflight: bool,
    main_ready: bool,
    deadline: Instant,
    head: Option<InternalResponse>,
    bytes: usize,
}
pub(crate) struct ReportingState {
    service: u64,
    next: u64,
    documents: BTreeMap<u64, Document>,
    deliveries: BTreeMap<RequestId, Delivery>,
    retained_bytes: usize,
    prepared_bytes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl Default for ReportingState {
    fn default() -> Self {
        Self {
            service: NEXT_SERVICE.fetch_add(1, Ordering::Relaxed),
            next: 1,
            documents: BTreeMap::new(),
            deliveries: BTreeMap::new(),
            retained_bytes: 0,
            prepared_bytes: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}
impl ReportingState {
    fn allocate(&mut self) -> u64 {
        let id = self.next;
        self.next = self.next.checked_add(1).expect("report id exhausted");
        id
    }
    fn document(&self, handle: ReportingDocument) -> Result<&Document, ReportError> {
        if handle.service != self.service {
            return Err(ReportError::StaleDocument);
        }
        self.documents
            .get(&handle.id)
            .ok_or(ReportError::StaleDocument)
    }
    fn insert(&mut self, document: Document) -> Result<ReportingDocument, ReportError> {
        if self.documents.len() >= MAX_DOCUMENTS {
            return Err(ReportError::Limit);
        }
        let id = self.allocate();
        self.documents.insert(id, document);
        Ok(ReportingDocument {
            service: self.service,
            id,
        })
    }
}

/// Parse the complete RFC 9651 dictionary, including ignored members/parameters.
/// One syntax error invalidates the entire header. Last duplicate member wins,
/// even when its value is not a String. No obsolete Report-To cache is inferred.
fn endpoints(base: &HttpUrl, headers: &HeaderList) -> BTreeMap<String, HttpUrl> {
    let mut output = BTreeMap::new();
    if !crate::a_priori_authenticated(base) {
        return output;
    }
    let Some(input) = headers.get("reporting-endpoints") else {
        return output;
    };
    let mut parser = structured::Parser::new(input.as_bytes());
    loop {
        let member = match parser.parse_dict() {
            Ok(Some(member)) => member,
            Ok(None) => return output,
            Err(_) => return BTreeMap::new(),
        };
        let (key, value) = member;
        output.remove(key);
        if let structured::Value::String { range, escape } = value {
            let mut value = input[range].to_owned();
            if escape {
                let mut decoded = String::new();
                let mut chars = value.chars();
                while let Some(c) = chars.next() {
                    decoded.push(if c == '\\' {
                        chars.next().expect("validated string escape")
                    } else {
                        c
                    });
                }
                value = decoded;
            }
            if let Ok(url) = base.resolve(&value) {
                if crate::a_priori_authenticated(&url)
                    && !url.includes_credentials()
                    && matches!(url.as_parsed().scheme(), "http" | "https")
                {
                    if output.len() >= MAX_ENDPOINTS {
                        return BTreeMap::new();
                    }
                    output.insert(key.to_owned(), url);
                }
            }
        }
    }
}
impl NetworkService {
    /// Call only after the native navigation response has been accepted for the
    /// actual target Document. Headers are the unfiltered ordered network head.
    pub fn register_reporting_response(
        &mut self,
        final_url: HttpUrl,
        headers: &HeaderList,
        context: ClientContext,
    ) -> Result<ReportingDocument, ReportError> {
        if !matches!(final_url.as_parsed().scheme(), "http" | "https") {
            return Err(ReportError::InvalidEndpoint);
        }
        self.reporting.insert(Document {
            creation_url: final_url.as_parsed().clone(),
            origin: final_url.origin().clone(),
            endpoints: endpoints(&final_url, headers),
            failures: BTreeMap::new(),
            referrer_policy: headers
                .get("referrer-policy")
                .as_deref()
                .and_then(crate::ReferrerPolicy::parse_header)
                .unwrap_or(crate::ReferrerPolicy::StrictOriginWhenCrossOrigin),
            context,
        })
    }
    /// Fresh about:blank/srcdoc globals do not inherit Reporting endpoints.
    /// This compatibility API retains creator policy context but starts empty.
    /// The caller verifies creator custody and binds this distinct child handle.
    pub fn inherit_reporting_document(
        &mut self,
        creator: ReportingDocument,
        actual_url: &ParsedUrl,
        context: ClientContext,
    ) -> Result<ReportingDocument, ReportError> {
        let url = actual_url.serialize();
        if !matches!(
            url.split(['#', '?']).next(),
            Some("about:blank" | "about:srcdoc")
        ) {
            return Err(ReportError::InvalidEndpoint);
        }
        let mut document = self.reporting.document(creator)?.clone();
        document.context = context;
        document.creation_url = actual_url.clone();
        document.endpoints.clear();
        document.failures.clear();
        self.reporting.insert(document)
    }
    /// Native policy-container changes (for example an accepted meta policy)
    /// affect future reports only. The embedding supplies its authentic policy.
    pub fn set_reporting_referrer_policy(
        &mut self,
        document: ReportingDocument,
        policy: crate::ReferrerPolicy,
    ) -> Result<(), ReportError> {
        self.reporting.document(document)?;
        self.reporting
            .documents
            .get_mut(&document.id)
            .expect("checked document")
            .referrer_policy = policy;
        Ok(())
    }
    /// Retirement prevents new admission. Admitted keepalive requests retain
    /// their immutable origin/context and finish without the retired Document.
    pub fn forget_reporting_document(&mut self, document: ReportingDocument) -> bool {
        document.service == self.reporting.service
            && self.reporting.documents.remove(&document.id).is_some()
    }
    pub fn has_pending_reporting_io(&self) -> bool {
        !self.reporting.deliveries.is_empty()
    }

    /// Admission is atomic and bounded across the service, not a delivery result.
    /// `queued_at` is the actual time the violation was queued by the embedding.
    pub fn queue_csp_report(
        &mut self,
        document: ReportingDocument,
        violation: &CspInlineViolation,
        queued_at: Instant,
        now: Instant,
    ) -> Result<Vec<ReportId>, ReportError> {
        let plan = self.prepare_csp_report(document, violation, queued_at)?;
        self.queue_prepared_csp_report(plan, now)
    }
    fn queue_saved_csp_report(
        &mut self,
        document: ReportingDocument,
        saved: Document,
        violation: &CspInlineViolation,
        queued_at: Instant,
        now: Instant,
    ) -> Result<Vec<ReportId>, ReportError> {
        let modern = violation.report_to.is_some();
        let targets = if let Some(name) = &violation.report_to {
            vec![(
                Some(name.clone()),
                saved
                    .endpoints
                    .get(name)
                    .cloned()
                    .ok_or(ReportError::NoEndpoint)?,
            )]
        } else {
            let base = HttpUrl::parse(&violation.document_uri).ok();
            violation
                .report_uris
                .iter()
                .filter_map(|value| {
                    base.as_ref()
                        .and_then(|base| base.resolve(value).ok())
                        .or_else(|| HttpUrl::parse(value).ok())
                        .filter(|url| {
                            matches!(url.as_parsed().scheme(), "http" | "https")
                                && !url.includes_credentials()
                        })
                        .map(|url| (None, url))
                })
                .collect::<Vec<_>>()
        };
        if targets.is_empty() {
            return Err(ReportError::NoEndpoint);
        }
        let body = serialize(
            violation,
            modern,
            now.saturating_duration_since(queued_at).as_millis(),
        );
        let retained = body
            .len()
            .checked_mul(targets.len())
            .ok_or(ReportError::Limit)?;
        if retained > MAX_REPORT_BYTES.saturating_sub(self.reporting.retained_bytes) {
            return Err(ReportError::Limit);
        }
        let mut admitted = Vec::new();
        for (endpoint_name, mut endpoint) in targets {
            let registered_endpoint = endpoint.clone();
            if let Some(upgraded) = self.hsts.upgrade(&endpoint, saved.context.now_unix) {
                endpoint = upgraded;
            }
            let mut request = Request::new(endpoint.clone(), saved.origin.clone());
            request
                .set_method_body("POST", Some(Body::Bytes(body.clone())))
                .map_err(ReportError::Policy)?;
            request
                .headers
                .append(
                    "content-type",
                    if modern {
                        "application/reports+json"
                    } else {
                        "application/csp-report"
                    },
                )
                .map_err(ReportError::Policy)?;
            request.credentials = CredentialsMode::SameOrigin;
            request.mode = if modern {
                RequestMode::Cors
            } else {
                RequestMode::NoCors
            };
            request.redirect = if modern {
                RedirectMode::Follow
            } else {
                RedirectMode::Error
            };
            request.security = saved.context.security;
            request.security.destination = crate::Destination::Other;
            request.security.top_level_navigation = false;
            if !modern {
                request.referrer = HttpUrl::parse(&violation.document_uri)
                    .ok()
                    .filter(|url| matches!(url.as_parsed().scheme(), "http" | "https"));
                request.referrer_policy = saved.referrer_policy;
                request.security = saved.context.security;
                request.security.destination = crate::Destination::Other;
                request.security.top_level_navigation = false;
            }
            let id = ReportId(self.reporting.allocate());
            let mut delivery = Delivery {
                id,
                document,
                route: if modern {
                    ReportRoute::ReportingApi
                } else {
                    ReportRoute::LegacyCsp
                },
                endpoint_name,
                endpoint: registered_endpoint,
                request,
                context: saved.context.clone(),
                preflight: false,
                main_ready: false,
                deadline: now + std::time::Duration::from_secs(10),
                head: None,
                bytes: body.len(),
            };
            match self.submit_reporting(&mut delivery, now) {
                Ok(wire) => {
                    self.reporting.retained_bytes += delivery.bytes;
                    self.reporting.deliveries.insert(wire, delivery);
                    admitted.push((id, wire));
                }
                Err(error) => {
                    for (_, wire) in admitted {
                        self.dispatcher.cancel(wire);
                        if let Some(delivery) = self.reporting.deliveries.remove(&wire) {
                            self.reporting.retained_bytes -= delivery.bytes;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Ok(admitted.into_iter().map(|(id, _)| id).collect())
    }
    fn submit_reporting(
        &mut self,
        delivery: &mut Delivery,
        now: Instant,
    ) -> Result<RequestId, ReportError> {
        if let Some(upgraded) = self
            .hsts
            .upgrade(delivery.request.url(), delivery.context.now_unix)
        {
            *delivery
                .request
                .url_list
                .last_mut()
                .expect("nonempty URL list") = upgraded;
        }
        // Legacy CSP is a browser-internal no-cors request with an explicitly
        // unsafe Content-Type and redirect:error. JS Request's no-cors guard is
        // intentionally not relaxed to express this native specification path.
        let timeout = delivery.deadline.saturating_duration_since(now);
        if timeout.is_zero() {
            return Err(ReportError::Transport);
        }
        let mut options = FetchOptions {
            timeout,
            max_response_bytes: MAX_REPORT_BYTES,
            ..Default::default()
        };
        let preflight = if delivery.route == ReportRoute::ReportingApi {
            delivery.request.prepare().map_err(ReportError::Policy)?;
            if delivery.main_ready || self.preflight_cache.contains(&delivery.request, now) {
                None
            } else {
                crate::preflight(&delivery.request).map_err(ReportError::Policy)?
            }
        } else {
            let mut security = delivery.request.clone();
            security.mode = RequestMode::Cors;
            security.redirect = RedirectMode::Follow;
            security.prepare().map_err(ReportError::Policy)?;
            None
        };
        delivery.preflight = preflight.is_some();
        delivery.main_ready = false;
        delivery.head = None;
        if let Some(preflight) = preflight {
            options.method = preflight.method.into();
            options.request_headers = preflight
                .headers
                .iter()
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
        } else {
            options.method = delivery.request.method().to_owned();
            options.request_headers = delivery
                .request
                .headers
                .iter()
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            if delivery.request.taint == crate::ResponseType::Cors
                || !matches!(delivery.request.method(), "GET" | "HEAD")
            {
                options
                    .request_headers
                    .push(("origin".into(), crate::cors_origin(&delivery.request)));
            }
            if delivery.route == ReportRoute::LegacyCsp {
                if let Some(referrer) = crate::determine_referrer(
                    delivery.request.referrer.as_ref(),
                    delivery.request.url(),
                    delivery.request.referrer_policy,
                ) {
                    options.request_headers.push(("referer".into(), referrer));
                }
            }
            if let Some(Body::Bytes(body)) = delivery.request.body() {
                options.body = body.clone();
            }
            if delivery.request.url().origin() == &delivery.request.origin {
                let access = crate::cookies::CookieAccess {
                    url: delivery.request.url(),
                    site_for_cookies: delivery.context.cookie_site.clone(),
                    http_api: true,
                    top_level_navigation: false,
                    method: delivery.request.method(),
                    partition_key: delivery.context.partition_key.clone(),
                };
                let cookie = self
                    .cookies
                    .cookie_header(&access, delivery.context.now_unix)
                    .map_err(|_| ReportError::Transport)?;
                if !cookie.is_empty() {
                    options.request_headers.push((
                        "cookie".into(),
                        String::from_utf8(cookie).map_err(|_| ReportError::Transport)?,
                    ));
                }
            }
        }
        self.dispatcher
            .submit(&delivery.request.url().serialize(), options, now)
            .map_err(|_| ReportError::Transport)
    }
    pub(crate) fn on_reporting_event(
        &mut self,
        event: &NetEvent,
        now: Instant,
        output: &mut Vec<ServiceEvent>,
    ) -> bool {
        let wire = match event {
            NetEvent::Head { id, .. }
            | NetEvent::Body { id, .. }
            | NetEvent::Done { id }
            | NetEvent::Error { id, .. }
            | NetEvent::Aborted { id } => *id,
        };
        let Some(mut delivery) = self.reporting.deliveries.remove(&wire) else {
            return false;
        };
        let result = match event {
            NetEvent::Head {
                status,
                reason,
                headers,
                ..
            } => {
                let mut fields = HeaderList::default();
                let valid = headers.iter().try_for_each(|(n, v)| fields.append(n, v));
                if valid.is_err() {
                    Some(ReportOutcome::Failed(ReportError::Transport))
                } else {
                    if !delivery.preflight
                        && delivery.request.url().origin() == &delivery.request.origin
                    {
                        let access = crate::cookies::CookieAccess {
                            url: delivery.request.url(),
                            site_for_cookies: delivery.context.cookie_site.clone(),
                            http_api: true,
                            top_level_navigation: false,
                            method: delivery.request.method(),
                            partition_key: delivery.context.partition_key.clone(),
                        };
                        for cookie in headers.get_all("set-cookie") {
                            let _ = self.cookies.set_cookie(
                                cookie.as_bytes(),
                                &access,
                                delivery.context.now_unix,
                            );
                        }
                        self.persist_cookies(delivery.context.now_unix);
                    }
                    self.observe_reporting_hsts(
                        delivery.request.url(),
                        &fields,
                        delivery.context.now_unix,
                    );
                    delivery.head = Some(InternalResponse {
                        status: *status,
                        status_text: reason.clone(),
                        headers: fields,
                        body: None,
                    });
                    None
                }
            }
            NetEvent::Body { .. } => None,
            NetEvent::Done { .. } => self.finish_reporting_hop(&mut delivery, now),
            NetEvent::Error { .. } | NetEvent::Aborted { .. } => {
                Some(ReportOutcome::Failed(ReportError::Transport))
            }
        };
        // A successful preflight/redirect moved this report to a new wire id.
        if self
            .reporting
            .deliveries
            .values()
            .any(|task| task.id == delivery.id)
        {
            return true;
        }
        if let Some(outcome) = result {
            self.dispatcher.cancel(wire);
            self.reporting.retained_bytes -= delivery.bytes;
            if delivery.route == ReportRoute::ReportingApi
                && !matches!(&outcome, ReportOutcome::Response { accepted: true, .. })
            {
                if let (Some(document), Some(name)) = (
                    self.reporting.documents.get_mut(&delivery.document.id),
                    &delivery.endpoint_name,
                ) {
                    if document.endpoints.get(name) == Some(&delivery.endpoint) {
                        let failures = document.failures.entry(name.clone()).or_default();
                        *failures = failures.saturating_add(1);
                    }
                }
            }
            output.push(ServiceEvent::ReportFinished(ReportFinished {
                id: delivery.id,
                document: delivery.document,
                route: delivery.route,
                outcome,
            }));
        } else {
            self.reporting.deliveries.insert(wire, delivery);
        }
        true
    }
    fn finish_reporting_hop(
        &mut self,
        delivery: &mut Delivery,
        now: Instant,
    ) -> Option<ReportOutcome> {
        let Some(response) = delivery.head.take() else {
            return Some(ReportOutcome::Failed(ReportError::Transport));
        };
        if delivery.preflight {
            if let Err(error) = crate::check_preflight(&delivery.request, &response) {
                return Some(ReportOutcome::Failed(ReportError::Policy(error)));
            }
            // The checked grant is reused only for this exact request/hop.
            self.preflight_cache
                .insert(&delivery.request, &response, now);
            delivery.preflight = false;
            delivery.main_ready = true;
            // submit_reporting consults this exact cache grant before OPTIONS.
            return self.resubmit_reporting(delivery, now);
        }
        if delivery.route == ReportRoute::ReportingApi {
            if delivery.request.taint == crate::ResponseType::Cors {
                if let Err(error) = crate::cors_check(&delivery.request, &response.headers) {
                    return Some(ReportOutcome::Failed(ReportError::Policy(error)));
                }
            }
            match crate::redirect_location_with_upgrade(
                &mut delivery.request,
                &response,
                response.headers.get("location").as_deref(),
                |url| {
                    self.hsts
                        .upgrade(&url, delivery.context.now_unix)
                        .unwrap_or(url)
                },
            ) {
                Ok(RedirectAction::Follow) => return self.resubmit_reporting(delivery, now),
                Err(error) => return Some(ReportOutcome::Failed(ReportError::Policy(error))),
                _ => {}
            }
            if response.status == 410 {
                if let Some(document) = self.reporting.documents.get_mut(&delivery.document.id) {
                    if let Some(name) = &delivery.endpoint_name {
                        if document.endpoints.get(name) == Some(&delivery.endpoint) {
                            document.endpoints.remove(name);
                            document.failures.remove(name);
                        }
                    }
                }
            }
        } else if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Some(ReportOutcome::Failed(ReportError::Policy(
                FetchError::Redirect,
            )));
        }
        Some(ReportOutcome::Response {
            status: response.status,
            accepted: delivery.route == ReportRoute::LegacyCsp
                || (200..300).contains(&response.status),
        })
    }
    fn resubmit_reporting(
        &mut self,
        delivery: &mut Delivery,
        now: Instant,
    ) -> Option<ReportOutcome> {
        // Move the whole immutable report metadata onto the next wire request.
        match self.submit_reporting(delivery, now) {
            Ok(wire) => {
                let next = Delivery {
                    id: delivery.id,
                    document: delivery.document,
                    route: delivery.route,
                    endpoint_name: delivery.endpoint_name.clone(),
                    endpoint: delivery.endpoint.clone(),
                    request: delivery.request.clone(),
                    context: delivery.context.clone(),
                    preflight: delivery.preflight,
                    main_ready: false,
                    deadline: delivery.deadline,
                    head: None,
                    bytes: delivery.bytes,
                };
                self.reporting.deliveries.insert(wire, next);
                None
            }
            Err(error) => Some(ReportOutcome::Failed(error)),
        }
    }
}

fn quote_units(value: &[u16]) -> String {
    let mut out = String::from("\"");
    for unit in value {
        match *unit {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from_u32(u32::from(*unit)).unwrap()),
            _ => out.push_str(&format!("\\u{unit:04x}")),
        }
    }
    out.push('"');
    out
}
fn quote(value: &str) -> String {
    quote_units(&value.encode_utf16().collect::<Vec<_>>())
}
fn strip_url(value: &str) -> String {
    let Ok(mut url) = ParsedUrl::parse(value) else {
        return value.to_owned();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return url.scheme().to_owned();
    }
    url.set_username("");
    url.set_password("");
    url.set_hash("");
    url.serialize()
}
fn serialize(v: &CspInlineViolation, modern: bool, age: u128) -> Vec<u8> {
    let disposition = match v.disposition {
        CspDisposition::Enforce => "enforce",
        CspDisposition::ReportOnly => "report",
    };
    let source = if v.source_file.is_empty() {
        "null".to_owned()
    } else {
        quote(&strip_url(&v.source_file))
    };
    let sample = quote_units(&v.sample);
    let body = if modern {
        format!(
            "{{\"documentURL\":{},\"referrer\":{},\"blockedURL\":{},\"effectiveDirective\":{},\"originalPolicy\":{},\"sourceFile\":{},\"sample\":{},\"disposition\":{},\"statusCode\":{},\"lineNumber\":{},\"columnNumber\":{}}}",
            quote(&strip_url(&v.document_uri)),
            quote(&strip_url(&v.referrer)),
            quote(&strip_url(&v.blocked_uri)),
            quote(&v.effective_directive),
            quote_units(&v.original_policy_units),
            source,
            sample,
            quote(disposition),
            v.status_code,
            if v.source_file.is_empty() {
                "null".into()
            } else {
                v.line_number.to_string()
            },
            if v.source_file.is_empty() {
                "null".into()
            } else {
                v.column_number.to_string()
            }
        )
    } else {
        let mut body = format!(
            "{{\"document-uri\":{},\"referrer\":{},\"blocked-uri\":{},\"effective-directive\":{},\"violated-directive\":{},\"original-policy\":{},\"disposition\":{},\"status-code\":{},\"script-sample\":{}",
            quote(&strip_url(&v.document_uri)),
            quote(&strip_url(&v.referrer)),
            quote(&strip_url(&v.blocked_uri)),
            quote(&v.effective_directive),
            quote(&v.effective_directive),
            quote_units(&v.original_policy_units),
            quote(disposition),
            v.status_code,
            sample
        );
        if !v.source_file.is_empty() {
            body.push_str(&format!(
                ",\"source-file\":{},\"line-number\":{},\"column-number\":{}",
                source, v.line_number, v.column_number
            ));
        }
        body.push('}');
        body
    };
    if modern {
        format!("[{{\"age\":{age},\"type\":\"csp-violation\",\"url\":{},\"user_agent\":{},\"body\":{body}}}]",quote(&v.report_url),quote(amazing_net::http::USER_AGENT)).into_bytes()
    } else {
        format!("{{\"csp-report\":{body}}}").into_bytes()
    }
}
