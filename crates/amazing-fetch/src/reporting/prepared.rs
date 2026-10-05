//! Original-global CSP report settings survive its author event callbacks.
//! This is a one-shot queued-report capability, never fresh-global authority.
use super::*;
use std::sync::{Arc, atomic::AtomicUsize};

/// Move-only native plan. HTML captures it against the authentic original global
/// before SPVE callbacks, then consumes it after dispatch (including exceptions).
/// Dropping the plan cancels only its prepared admission and releases its credit.
/// No runtime Window roots, creator lookup, Clone or public constructor exists.
/// ```compile_fail
/// use amazing_fetch::reporting::PreparedCspReportPlan;
/// fn cannot_duplicate(plan: PreparedCspReportPlan) { let _ = plan.clone(); }
/// ```
#[must_use]
pub struct PreparedCspReportPlan {
    service: u64,
    document: ReportingDocument,
    settings: Document,
    violation: CspInlineViolation,
    queued_at: Instant,
    credits: PreparedCredits,
}
pub(super) struct PreparedCredits {
    pub(super) quota: Arc<AtomicUsize>,
    pub(super) bytes: usize,
}
impl Drop for PreparedCredits {
    fn drop(&mut self) {
        self.quota.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
impl NetworkService {
    /// Capture only while the actual original global's handle is live. The
    /// snapshot retains immutable scalar settings/configuration and violation
    /// units; it neither submits a request nor registers another global.
    pub fn prepare_csp_report(
        &mut self,
        document: ReportingDocument,
        violation: &CspInlineViolation,
        queued_at: Instant,
    ) -> Result<PreparedCspReportPlan, ReportError> {
        let settings = self.reporting.document(document)?;
        let bytes = snapshot_bytes(settings, violation).ok_or(ReportError::Limit)?;
        let quota = &self.reporting.prepared_bytes;
        let mut used = quota.load(Ordering::Acquire);
        loop {
            let next = used
                .checked_add(bytes)
                .filter(|next| *next <= MAX_REPORT_BYTES)
                .ok_or(ReportError::Limit)?;
            match quota.compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(actual) => used = actual,
            }
        }
        let credits = PreparedCredits {
            quota: quota.clone(),
            bytes,
        };
        Ok(PreparedCspReportPlan {
            service: self.reporting.service,
            document,
            settings: settings.clone(),
            violation: violation.clone(),
            queued_at,
            credits,
        })
    }
    /// Consume once after actual author event dispatch. Original retirement does
    /// not erase already prepared settings, and does not restore a live global.
    /// ReportFinished retains the original handle; an admitted request drains
    /// through the ordinary Reporting transport even after the global is gone.
    pub fn queue_prepared_csp_report(
        &mut self,
        plan: PreparedCspReportPlan,
        now: Instant,
    ) -> Result<Vec<ReportId>, ReportError> {
        let PreparedCspReportPlan {
            service,
            document,
            settings,
            violation,
            queued_at,
            credits,
        } = plan;
        if service != self.reporting.service {
            return Err(ReportError::StaleDocument);
        }
        drop(credits);
        self.queue_saved_csp_report(document, settings, &violation, queued_at, now)
    }
}
/// Application snapshot content budget, separate from the existing serialized
/// delivery-body queue budget. Counts owned scalar records and string/unit
/// content before cloning, not total allocator/provider/socket memory.
fn snapshot_bytes(settings: &Document, violation: &CspInlineViolation) -> Option<usize> {
    let mut bytes =
        std::mem::size_of::<Document>().checked_add(std::mem::size_of::<CspInlineViolation>())?;
    let mut add = |amount: usize| -> Option<()> {
        bytes = bytes.checked_add(amount)?;
        (bytes <= MAX_REPORT_BYTES).then_some(())
    };
    for value in [
        &violation.effective_directive,
        &violation.violated_directive,
        &violation.source_directive,
        &violation.original_policy,
        &violation.blocked_uri,
        &violation.document_uri,
        &violation.report_url,
        &violation.referrer,
        &violation.source_file,
    ] {
        add(value.len())?;
    }
    add(violation.original_policy_units.len().checked_mul(2)?)?;
    add(violation.sample.len().checked_mul(2)?)?;
    if let Some(name) = &violation.report_to {
        add(name.len())?;
    }
    for uri in &violation.report_uris {
        add(std::mem::size_of::<String>())?;
        add(uri.len())?;
    }
    add(settings.creation_url.serialize().len())?;
    add(settings.origin.serialize().len())?;
    for (name, url) in &settings.endpoints {
        add(std::mem::size_of::<(String, HttpUrl)>())?;
        add(name.len())?;
        add(url.serialize().len())?;
    }
    for name in settings.failures.keys() {
        add(std::mem::size_of::<(String, u64)>())?;
        add(name.len())?;
    }
    if let crate::cookies::CookieSite::Origin(origin) = &settings.context.cookie_site {
        add(origin.serialize().len())?;
    }
    if let Some(crate::Site::Tuple { scheme, domain }) = &settings.context.partition_key {
        add(scheme.len())?;
        add(domain.len())?;
    }
    for origin in &settings.context.frame_ancestors {
        add(std::mem::size_of::<Origin>())?;
        add(origin.serialize().len())?;
    }
    for policy in &settings.context.csp.policies {
        add(std::mem::size_of::<crate::CspPolicy>())?;
        add(policy.original_policy.len())?;
        add(policy.original_policy_units.len().checked_mul(2)?)?;
        for directive in &policy.directives {
            add(std::mem::size_of::<crate::CspDirective>())?;
            add(directive.name.len())?;
            for source in &directive.sources {
                add(std::mem::size_of::<String>())?;
                add(source.len())?;
            }
        }
    }
    // Exact serialized report-body size is independently bounded before the
    // immutable violation clone. Event-to-submit age may grow later; submission
    // still checks the actual final body against the delivery queue budget.
    if serialize(violation, violation.report_to.is_some(), 0).len() > MAX_REPORT_BYTES {
        return None;
    }
    Some(bytes)
}
