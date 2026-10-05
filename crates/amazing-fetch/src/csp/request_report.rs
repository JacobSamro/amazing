//! Canonical CSP request violation custody from real pre/post checks.
use super::*;
use crate::ReceivingDocumentClient;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspRequestPhase {
    PreRequest,
    PostRequest,
    WebSocketPreRequest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspRequestEnrichmentError {
    ReceivingClientMismatch,
    RequestMismatch,
    PolicyMismatch,
}
/// Native source-element provenance at external request preparation; include
/// tokenizer duplicate receipts, internal nonce and actual unconverted units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CspExternalElementKind {
    Script,
    Style,
}
pub struct CspRequestElement<'a> {
    pub nonce: Option<&'a [u16]>,
    pub nonce_attribute_present: bool,
    pub attributes: &'a [CspInlineAttribute<'a>],
    pub duplicate_attribute: bool,
    pub parser_inserted: bool,
}
/// Protected Document/source metadata saved by its native loader. Document and
/// creation URLs/origins come exclusively from ReceivingDocumentClient.
pub struct CspRequestReportContext<'a> {
    pub element: Option<u64>,
    pub referrer: &'a str,
    pub status_code: u16,
    pub source_file: &'a str,
    pub line_number: u32,
    pub column_number: u32,
}
impl Request {
    /// Apply the shared native nonceability rule before the ordinary Fetch CSP
    /// checks. Invalid UTF16 never becomes another nonce; duplicates or dangling
    /// markup cannot grant nonce authority. Other enforcement algorithms remain.
    pub fn apply_csp_request_element(&mut self, element: &CspRequestElement<'_>) {
        let script = matches!(
            self.csp_destination,
            CspDestination::Script | CspDestination::ScriptElement
        ) || self.security.destination == crate::Destination::Script;
        let element_destination = matches!(
            self.csp_destination,
            CspDestination::Script
                | CspDestination::ScriptElement
                | CspDestination::Style
                | CspDestination::StyleElement
        ) || matches!(
            self.security.destination,
            crate::Destination::Script | crate::Destination::Style
        );
        self.csp_nonce = if element_destination
            && csp_element_nonceable(
                script,
                element.nonce_attribute_present,
                element.attributes,
                element.duplicate_attribute,
            ) {
            element
                .nonce
                .and_then(|units| String::from_utf16(units).ok())
        } else {
            None
        };
        self.csp_parser_inserted = element.parser_inserted;
    }
}
impl CspViolation {
    pub fn policy_index(&self) -> usize {
        self.policy_index
    }
    pub fn effective_directive(&self) -> &str {
        &self.effective_directive
    }
    pub fn request_phase(&self) -> CspRequestPhase {
        self.phase
    }
    /// Actual native request lineage at this check, also available when the
    /// transport terminates with typed FetchError::Csp and no response head.
    pub fn request_url_list(&self) -> &[HttpUrl] {
        &self.request_urls
    }
    /// Enrich only an actual native check receipt using the matching saved
    /// Request and receiving client. Retained immutable policy, not diagnostics
    /// or a guessed fallback directive, supplies original policy/report routes.
    pub fn enrich_request_report<Identity>(
        &self,
        receiving: &ReceivingDocumentClient<Identity>,
        request: &Request,
        source: &CspRequestReportContext<'_>,
    ) -> Result<CspReportViolation, CspRequestEnrichmentError> {
        if request.origin != self.request_origin
            || request.csp_self_origin != self.csp_self_origin
            || request.url_list() != self.request_urls
        {
            return Err(CspRequestEnrichmentError::RequestMismatch);
        }
        if request.csp.policies.get(self.policy_index) != Some(&self.policy_snapshot) {
            return Err(CspRequestEnrichmentError::PolicyMismatch);
        }
        self.enrich_saved_request_report(receiving, source)
    }
    /// Ordinary service completions may have followed redirects after the
    /// loader's initial Request capture. Use this authentic immutable native
    /// check receipt rather than reconstructing its URL chain in the driver.
    /// Only scalar URL/origin/policy settings were saved, never a request body
    /// or runtime roots. HTML separately validates the associated Doc/generation.
    pub fn enrich_saved_request_report<Identity>(
        &self,
        receiving: &ReceivingDocumentClient<Identity>,
        source: &CspRequestReportContext<'_>,
    ) -> Result<CspReportViolation, CspRequestEnrichmentError> {
        if receiving.origin != self.request_origin
            || receiving.csp_self_origin != self.csp_self_origin
        {
            return Err(CspRequestEnrichmentError::ReceivingClientMismatch);
        }
        if receiving.context.csp.policies.get(self.policy_index) != Some(&self.policy_snapshot) {
            return Err(CspRequestEnrichmentError::PolicyMismatch);
        }
        let policy = &self.policy_snapshot;
        let reporting = |name: &str| {
            policy
                .directives
                .iter()
                .find(|directive| directive.name == name)
        };
        Ok(CspReportViolation {
            effective_directive: self.effective_directive.clone(),
            violated_directive: self.effective_directive.clone(),
            source_directive: self.source_directive.clone(),
            original_policy: policy.original_policy.clone(),
            original_policy_units: policy.original_policy_units.clone(),
            policy_index: self.policy_index,
            disposition: policy.disposition,
            // CSP2.4.2 uses request URL (first URL list entry), including post-check
            // failures, not a substituted response/final redirect target.
            blocked_uri: super::inline::strip_report_url(&self.resource_url.serialize()),
            element: source.element,
            document_uri: super::inline::strip_report_url(&receiving.document_url.serialize()),
            report_url: super::inline::strip_report_url(&receiving.creation_url.serialize()),
            referrer: super::inline::strip_report_url(source.referrer),
            status_code: source.status_code,
            source_file: super::inline::strip_report_url(source.source_file),
            line_number: source.line_number,
            column_number: source.column_number,
            sample: Vec::new(),
            report_to: reporting("report-to").map(|directive| directive.sources.join(" ")),
            report_uris: reporting("report-uri")
                .map_or_else(Vec::new, |directive| directive.sources.clone()),
        })
    }
}
