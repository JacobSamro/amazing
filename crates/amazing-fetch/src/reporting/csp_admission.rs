//! CSP 5.5 per-token request admission. These are observations, not fresh authority.
use super::*;

/// One token in original order (or the one modern report-to route). A rejected
/// token has no ReportId; admitted IDs remain visible even when siblings fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CspReportAttempt {
    pub(super) token_index: usize,
    pub(super) endpoint: Option<HttpUrl>,
    pub(super) outcome: Result<ReportId, ReportError>,
}
impl CspReportAttempt {
    pub fn token_index(&self) -> usize { self.token_index }
    pub fn endpoint(&self) -> Option<&HttpUrl> { self.endpoint.as_ref() }
    pub fn outcome(&self) -> &Result<ReportId, ReportError> { &self.outcome }
}
/// Move-only original-global admission facts. Original prepared scalar credit
/// and its lifecycle remain retained until the actual consumer drops this
/// record. It cannot create another global or submit another report.
/// ```compile_fail
/// use amazing_fetch::reporting::CspReportAdmissions;
/// fn duplicate(value: CspReportAdmissions) { let _ = value.clone(); }
/// ```
#[must_use]
pub struct CspReportAdmissions {
    pub(super) document: ReportingDocument,
    pub(super) route: ReportRoute,
    pub(super) attempts: Vec<CspReportAttempt>,
    pub(super) _lifetime: std::sync::Arc<()>,
    pub(super) credits: Option<prepared::PreparedCredits>,
}
impl CspReportAdmissions {
    pub fn document(&self) -> ReportingDocument { self.document }
    pub fn route(&self) -> ReportRoute { self.route }
    pub fn attempts(&self) -> &[CspReportAttempt] { &self.attempts }
    pub fn admitted_ids(&self) -> impl Iterator<Item = ReportId> + '_ {
        self.attempts.iter().filter_map(|attempt| attempt.outcome.as_ref().ok().copied())
    }
    /// The compatibility API never hides a genuine partial admission behind
    /// Err. When nothing was admitted, preserve its first typed rejection (or
    /// NoEndpoint for an empty list). Consuming this releases observation credit.
    pub(super) fn into_compatible_ids(self) -> Result<Vec<ReportId>, ReportError> {
        let ids = self.admitted_ids().collect::<Vec<_>>();
        if !ids.is_empty() { return Ok(ids); }
        Err(self.attempts.iter().find_map(|attempt| match &attempt.outcome {
            Err(ReportError::InvalidEndpoint) if self.route == ReportRoute::LegacyCsp => None,
            Err(error) => Some(error.clone()), _ => None,
        })
            .unwrap_or(ReportError::NoEndpoint))
    }
}
impl NetworkService {
    /// Capture the live original settings and return every actual token outcome.
    /// An admitted sibling is never cancelled by a later token or quota failure.
    pub fn queue_csp_report_admissions(&mut self, document: ReportingDocument,
        violation: &CspInlineViolation, queued_at: Instant, now: Instant)
        -> Result<CspReportAdmissions, ReportError>
    {
        let plan = self.prepare_csp_report(document, violation, queued_at)?;
        self.queue_prepared_csp_report_admissions(plan, now)
    }
}
