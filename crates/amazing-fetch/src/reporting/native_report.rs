//! Registered ordinary native Reporting bodies, without DOM/JS dependency.
//! COEP report construction follows HTML 7.1.4.2 and Fetch's CORP report step.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoepReportDisposition {
    Enforce,
    Reporting,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReportVisibility {
    ObserverVisible,
}
/// Real response provenance is supplied by the native Fetch/navigation/worker
/// owner. A CORP report additionally carries the authentic request destination.
pub enum CoepReportBody<'a> {
    Navigation {
        response_url_list: &'a [ParsedUrl],
        disposition: CoepReportDisposition,
    },
    WorkerInitialization {
        response_url_list: &'a [ParsedUrl],
        disposition: CoepReportDisposition,
    },
    Corp {
        response_url_list: &'a [ParsedUrl],
        disposition: CoepReportDisposition,
        destination: &'a str,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeReportSnapshot {
    pub report_type: Vec<u16>,
    pub report_url: Vec<u16>,
    pub visibility: NativeReportVisibility,
    /// Exact immutable dictionary string keys/values for native Observer mapping.
    pub body: Vec<(Vec<u16>, Vec<u16>)>,
    /// Only network admission; observer creation is independent of this result.
    pub delivery: Result<ReportId, ReportError>,
}
impl NetworkService {
    /// Construct registered observer-visible type coep using the original live
    /// same-service settings. No request URL supplies a receiving-global identity.
    pub fn generate_coep_report(
        &mut self,
        document: ReportingDocument,
        body: CoepReportBody<'_>,
        group: &[u16],
        queued_at: Instant,
        now: Instant,
    ) -> Result<NativeReportSnapshot, ReportError> {
        let saved = self.reporting.document(document)?.clone();
        self.generate_saved_coep_report(document, &saved, body, group, queued_at, now, true)
    }
    pub(super) fn generate_saved_coep_report(
        &mut self, document: ReportingDocument, saved: &Document,
        body: CoepReportBody<'_>, group: &[u16], queued_at: Instant, now: Instant, admit: bool,
    ) -> Result<NativeReportSnapshot, ReportError> {
        let (kind, response_url_list, disposition, destination) = match body {
            CoepReportBody::Navigation {
                response_url_list,
                disposition,
            } => ("navigation", response_url_list, disposition, None),
            CoepReportBody::WorkerInitialization {
                response_url_list,
                disposition,
            } => (
                "worker initialization",
                response_url_list,
                disposition,
                None,
            ),
            CoepReportBody::Corp {
                response_url_list,
                disposition,
                destination,
            } => ("corp", response_url_list, disposition, Some(destination)),
        };
        // Fetch serializes URL list[0], never the final redirect target.
        let mut first = response_url_list
            .first()
            .ok_or(ReportError::InvalidReport)?
            .clone();
        first.set_username("");
        first.set_password("");
        first.set_hash("");
        let blocked = first.serialize();
        let report_url = strip_url(&saved.creation_url.serialize());
        if blocked.len().saturating_add(report_url.len()) > MAX_REPORT_BYTES / 2 {
            return Err(ReportError::Limit);
        }
        let mut fields = vec![
            ("type", kind.to_owned()),
            ("blockedURL", blocked),
            (
                "disposition",
                match disposition {
                    CoepReportDisposition::Enforce => "enforce",
                    CoepReportDisposition::Reporting => "reporting",
                }
                .to_owned(),
            ),
        ];
        if let Some(destination) = destination {
            fields.push((
                "destination",
                validated_destination(destination)?.to_owned(),
            ));
        }
        let body_json = format!(
            "{{{}}}",
            fields
                .iter()
                .map(|(key, value)| format!("{}:{}", quote(key), quote(value)))
                .collect::<Vec<_>>()
                .join(",")
        );
        let delivery = if admit { self.admit_native_report(
            document,
            &saved,
            "coep",
            &body_json,
            group,
            &report_url,
            queued_at,
            now,
        ) } else { Err(ReportError::NoEndpoint) };
        Ok(NativeReportSnapshot {
            report_type: "coep".encode_utf16().collect(),
            report_url: report_url.encode_utf16().collect(),
            visibility: NativeReportVisibility::ObserverVisible,
            body: fields
                .into_iter()
                .map(|(key, value)| (key.encode_utf16().collect(), value.encode_utf16().collect()))
                .collect(),
            delivery,
        })
    }
    pub(super) fn admit_native_report(
        &mut self,
        document: ReportingDocument,
        saved: &Document,
        report_type: &str,
        body_json: &str,
        group: &[u16],
        report_url: &str,
        queued_at: Instant,
        now: Instant,
    ) -> Result<ReportId, ReportError> {
        if group.len() > MAX_REPORT_BYTES || group.iter().any(|unit| *unit > 0x7f) {
            return Err(ReportError::NoEndpoint);
        }
        let name: String = group.iter().map(|unit| char::from(*unit as u8)).collect();
        let (registered_endpoint, endpoint_lifetime) = saved.named_endpoint(&name)?;
        let body = format!(
            "[{{\"age\":{},\"type\":{},\"url\":{},\"user_agent\":{},\"body\":{}}}]",
            now.saturating_duration_since(queued_at).as_millis(),
            quote(report_type),
            quote(report_url),
            quote(amazing_net::http::USER_AGENT),
            body_json,
        )
        .into_bytes();
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
            endpoint_lifetime: Some(endpoint_lifetime),
            endpoint: registered_endpoint,
            request,
            context: saved.context.clone(),
            preflight: false,
            request_includes_credentials: false,
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
fn validated_destination(destination: &str) -> Result<&str, ReportError> {
    if matches!(
        destination,
        "" | "audio"
            | "audioworklet"
            | "document"
            | "embed"
            | "font"
            | "frame"
            | "iframe"
            | "image"
            | "json"
            | "manifest"
            | "object"
            | "paintworklet"
            | "report"
            | "script"
            | "serviceworker"
            | "sharedworker"
            | "style"
            | "text"
            | "track"
            | "video"
            | "webidentity"
            | "worker"
            | "xslt"
    ) {
        Ok(destination)
    } else {
        Err(ReportError::InvalidReport)
    }
}
