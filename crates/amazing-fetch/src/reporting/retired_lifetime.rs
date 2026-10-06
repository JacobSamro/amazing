//! Original-global scalar custody, without Window roots or URL authority.
use super::*;

/// A release checkpoint, never a new global/settings capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingRelease {
    /// Retirement has not occurred. No live settings are removed here.
    Active,
    /// Original prepared/graph/request/delivery/completion custody remains.
    Pending,
    /// No live or outstanding native settings remain; bookkeeping was released.
    Released,
}
impl NetworkService {
    /// Only a genuine same-service capability can query original retirement.
    /// Private Document clones automatically retain the original lease through
    /// CSP/COEP plans, module graphs, requests and admitted report redirects.
    /// ReportFinished retains it until an actual consumer drops that completion.
    /// A driver must additionally finish its authentic original task/observer
    /// queue before releasing its receiving registration; no driver ACK is
    /// inferred by this native checkpoint. Existing StaleDocument admission is
    /// unchanged and this API cannot revive a retired global.
    pub fn release_retired_reporting_document(
        &mut self, document: ReportingDocument,
    ) -> Result<ReportingRelease, ReportError> {
        if document.service != self.reporting.service {
            return Err(ReportError::StaleDocument);
        }
        if self.reporting.documents.contains_key(&document.id) {
            return Ok(ReportingRelease::Active);
        }
        if self.reporting.lifetimes.get(&document.id)
            .is_some_and(|lifetime| lifetime.strong_count() != 0)
        {
            return Ok(ReportingRelease::Pending);
        }
        self.reporting.lifetimes.remove(&document.id);
        Ok(ReportingRelease::Released)
    }
}

/// Bounded original scalar settings for the authentic task consumer. It grants
/// no report-plan minting, current-global lookup, Clone, or Window root.
/// ```compile_fail
/// use amazing_fetch::reporting::ReportingTaskConsumer;
/// fn duplicate(value: ReportingTaskConsumer) { let _ = value.clone(); }
/// ```
#[must_use]
pub struct ReportingTaskConsumer {
    document: ReportingDocument,
    _settings: Document,
    _credits: prepared::PreparedCredits,
}
impl NetworkService {
    pub fn prepare_reporting_task_consumer(&mut self, document: ReportingDocument)
        -> Result<ReportingTaskConsumer, ReportError>
    {
        let settings = self.reporting.document(document)?;
        let bytes = prepared::document_snapshot_bytes(settings).ok_or(ReportError::Limit)?;
        let credits = coep_request::graph_credit(&self.reporting.prepared_bytes, bytes)?;
        Ok(ReportingTaskConsumer { document, _settings: settings.clone(), _credits: credits })
    }
    /// Only the actual original task owner closes after task processing/drain.
    /// A foreign service cannot consume or release this owner's settings.
    pub fn close_reporting_task_consumer(&mut self, consumer: &mut Option<ReportingTaskConsumer>)
        -> Result<bool, ReportError>
    {
        let Some(saved) = consumer.as_ref() else { return Ok(false); };
        if saved.document.service != self.reporting.service { return Err(ReportError::StaleDocument); }
        consumer.take();
        Ok(true)
    }
}
