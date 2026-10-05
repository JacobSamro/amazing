//! Ordinary native Reporting §7.1 report generation. Automation supplies the
//! authentic saved global capability; no CSP violation is fabricated.
use super::*;
use crate::ReceivingDocumentClient;

/// Immutable creation receipt for the owning HTML adapter's observer admission.
/// Type is `test`, visibility is ObserverVisible, body has only `body_message`.
/// The adapter must notify the saved global even when `delivery` is an error.
/// Net never calls DOM callbacks or infers a Window from this report URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestReportSnapshot {
    pub report_url: String,
    pub message: Vec<u16>,
    /// Network admission only. Success finishes through actual ReportFinished;
    /// absence, limits and transport failure do not revoke observer creation.
    pub delivery: Result<ReportId, ReportError>,
}

impl NetworkService {
    /// Register headers only after actual response admission to `client.identity`.
    /// The receiving-document owner captures origin/context/creation URL; the
    /// accepted final response URL is used only to resolve/trust endpoint URLs.
    pub fn register_reporting_client_response<Identity>(
        &mut self,
        client: &ReceivingDocumentClient<Identity>,
        accepted_final_url: &HttpUrl,
        headers: &HeaderList,
    ) -> Result<ReportingDocument, ReportError> {
        if !matches!(accepted_final_url.as_parsed().scheme(), "http" | "https") {
            return Err(ReportError::InvalidEndpoint);
        }
        self.reporting.insert(Document {
            creation_url: client.creation_url.clone(),
            origin: client.origin.clone(),
            context: client.context.clone(),
            endpoints: endpoints(accepted_final_url, headers),
            failures: BTreeMap::new(),
            referrer_policy: client.referrer_policy,
        })
    }

    /// Register a genuinely fresh global with its initially empty endpoint list.
    /// The owner calls this at global creation, never as a failed-response fallback.
    /// Actual response admission uses register_reporting_client_response instead.
    pub fn register_reporting_initial_client<Identity>(
        &mut self,
        client: &ReceivingDocumentClient<Identity>,
    ) -> Result<ReportingDocument, ReportError> {
        self.reporting.insert(Document {
            creation_url: client.creation_url.clone(),
            origin: client.origin.clone(),
            context: client.context.clone(),
            endpoints: BTreeMap::new(),
            failures: BTreeMap::new(),
            referrer_policy: client.referrer_policy,
        })
    }

    /// Register a fresh about child with its own initially empty endpoints.
    /// Creator custody is validated for this immediate compatibility API only;
    /// deferred commits call register_reporting_initial_client with the child.
    pub fn inherit_reporting_client<Identity>(
        &mut self,
        creator: ReportingDocument,
        client: &ReceivingDocumentClient<Identity>,
    ) -> Result<ReportingDocument, ReportError> {
        let url = client.creation_url.serialize();
        if !matches!(
            url.split(['#', '?']).next(),
            Some("about:blank" | "about:srcdoc")
        ) {
            return Err(ReportError::InvalidEndpoint);
        }
        self.reporting.document(creator)?;
        self.register_reporting_initial_client(client)
    }

    /// Generate type `test` from the actual global's saved creation URL. Group
    /// defaults to "default" in the WebDriver adapter; this native API preserves
    /// every supplied unit and never turns invalid UTF-16 into another endpoint.
    /// Returning this receipt is native creation, independent of HTTP outcome.
    /// HTML owns notification/buffering/tasks against the authenticated global.
    pub fn generate_test_report(
        &mut self,
        document: ReportingDocument,
        message: &[u16],
        group: &[u16],
        now: Instant,
    ) -> Result<TestReportSnapshot, ReportError> {
        let saved = self.reporting.document(document)?.clone();
        // Bound retained message allocation before JSON expansion. This is an
        // application snapshot limit, not an exact provider memory accounting.
        if message.len() > MAX_REPORT_BYTES / 2 {
            return Err(ReportError::Limit);
        }
        let report_url = strip_url(&saved.creation_url.serialize());
        let delivery = self.admit_test_report(document, &saved, message, group, &report_url, now);
        Ok(TestReportSnapshot {
            report_url,
            message: message.to_vec(),
            delivery,
        })
    }

    fn admit_test_report(
        &mut self,
        document: ReportingDocument,
        saved: &Document,
        message: &[u16],
        group: &[u16],
        report_url: &str,
        now: Instant,
    ) -> Result<ReportId, ReportError> {
        if group.len() > MAX_REPORT_BYTES || group.iter().any(|unit| *unit > 0x7f) {
            return Err(ReportError::NoEndpoint);
        }
        let name: String = group.iter().map(|unit| char::from(*unit as u8)).collect();
        let registered_endpoint = saved
            .endpoints
            .get(&name)
            .cloned()
            .ok_or(ReportError::NoEndpoint)?;
        let body = format!(
            "[{{\"age\":0,\"type\":\"test\",\"url\":{},\"user_agent\":{},\"body\":{{\"body_message\":{}}}}}]",
            quote(report_url), quote(amazing_net::http::USER_AGENT), quote_units(message),
        ).into_bytes();
        if body.len() > MAX_REPORT_BYTES.saturating_sub(self.reporting.retained_bytes) {
            return Err(ReportError::Limit);
        }
        let endpoint = self
            .hsts
            .upgrade(&registered_endpoint, saved.context.now_unix)
            .unwrap_or_else(|| registered_endpoint.clone());
        let mut request = Request::new(endpoint, saved.origin.clone());
        request
            .set_method_body("POST", Some(Body::Bytes(body.clone())))
            .map_err(ReportError::Policy)?;
        request
            .headers
            .append("content-type", "application/reports+json")
            .map_err(ReportError::Policy)?;
        request.security = saved.context.security;
        request.security.destination = crate::Destination::Other;
        request.security.top_level_navigation = false;
        request.credentials = CredentialsMode::SameOrigin;
        request.mode = RequestMode::Cors;
        request.redirect = RedirectMode::Follow;
        let id = ReportId(self.reporting.allocate());
        let mut delivery = Delivery {
            id,
            document,
            route: ReportRoute::ReportingApi,
            endpoint_name: Some(name),
            endpoint: registered_endpoint,
            request,
            context: saved.context.clone(),
            preflight: false,
            main_ready: false,
            deadline: now + std::time::Duration::from_secs(10),
            head: None,
            bytes: body.len(),
        };
        let wire = self.submit_reporting(&mut delivery, now)?;
        self.reporting.retained_bytes += delivery.bytes;
        self.reporting.deliveries.insert(wire, delivery);
        Ok(id)
    }
}
