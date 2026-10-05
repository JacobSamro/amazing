//! Request-scoped original SOURCE CSP settings, separate from embedding COEP.
//! Private native binding survives authentic Manual hops, never creates a global.
use super::*;
use crate::{api::ApiRequest, ReceivingDocumentClient, CspRequestReportContext, CspResponseProvenance, CspViolation};
use std::sync::Arc;

struct ProtectedContext {
    element:Option<u64>,referrer:String,status_code:u16,source_file:String,line_number:u32,column_number:u32,
}
/// Move-only captured native source. The browser retains this only for its real
/// admitted navigation and releases it at terminal graph/report task cleanup.
/// No Window roots, public constructor, Clone or endpoint-global transfer exists.
/// ```compile_fail
/// use amazing_fetch::reporting::NavigationCspSourceLoan;
/// fn cannot_copy(loan:NavigationCspSourceLoan){let _=loan.clone();}
/// ```
#[must_use]
pub struct NavigationCspSourceLoan {
    service:u64,document:ReportingDocument,settings:Document,
    receiver:ReceivingDocumentClient<()>,protected:ProtectedContext,
    token:Arc<crate::model::NativeNavigationToken>,
    used:Vec<(CspViolation,prepared::PreparedCredits)>,_credits:prepared::PreparedCredits,
}
impl std::fmt::Debug for NavigationCspSourceLoan {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("NavigationCspSourceLoan").finish_non_exhaustive()}
}
impl NetworkService {
    /// Capture against the LIVE original source at actual native admission,
    /// before IO/author callbacks. Protected status/referrer/location comes from
    /// that original Document, never the target head or embedding owner's node.
    pub fn prepare_navigation_csp_reporting<Identity>(&mut self,document:ReportingDocument,
        receiver:&ReceivingDocumentClient<Identity>,request:&mut ApiRequest,
        protected:&CspRequestReportContext<'_>)->Result<NavigationCspSourceLoan,ReportError> {
        let settings=self.reporting.document(document)?;
        if settings.creation_url!=receiver.creation_url || settings.origin!=receiver.origin {
            return Err(ReportError::InvalidReport);
        }
        request.apply_navigation_receiving_client(receiver).map_err(|_|ReportError::InvalidReport)?;
        let owner=request.embedding_owner.as_ref().ok_or(ReportError::InvalidReport)?;
        // A foreign owner's service cannot claim or poison this source binding.
        drop(owner.loan(self)?);
        let mut bytes=std::mem::size_of::<NavigationCspSourceLoan>()
            .checked_add(prepared::document_snapshot_bytes(settings).ok_or(ReportError::Limit)?)
            .ok_or(ReportError::Limit)?;
        let mut add=|n:usize|->Result<(),ReportError> {bytes=bytes.checked_add(n).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;Ok(())};
        add(std::mem::size_of::<ReceivingDocumentClient<()>>())?;
        // Receiver plus original settings each retain the actual current
        // source context, including Meta policies captured by the native factory.
        add(prepared::client_context_bytes(&receiver.context).ok_or(ReportError::Limit)?.checked_mul(2).ok_or(ReportError::Limit)?)?;
        for url in [&receiver.creation_url,&receiver.document_url,&receiver.base_url] {add(url.serialize().len())?;}
        add(receiver.origin.serialize().len())?;add(receiver.csp_self_origin.serialize().len())?;
        add(protected.referrer.len())?;add(protected.source_file.len())?;
        let credits=coep_request::graph_credit(&self.reporting.prepared_bytes,bytes)?;
        let token=request.claim_navigation_csp_reporting().ok_or(ReportError::InvalidReport)?;
        let mut saved_settings=settings.clone();
        saved_settings.context=receiver.context.clone();saved_settings.referrer_policy=receiver.referrer_policy;
        Ok(NavigationCspSourceLoan {service:self.reporting.service,document,settings:saved_settings,
            receiver:ReceivingDocumentClient {identity:(),creation_url:receiver.creation_url.clone(),
                document_url:receiver.document_url.clone(),base_url:receiver.base_url.clone(),origin:receiver.origin.clone(),
                csp_self_origin:receiver.csp_self_origin.clone(),context:receiver.context.clone(),referrer_policy:receiver.referrer_policy},
            protected:ProtectedContext {element:protected.element,referrer:protected.referrer.into(),status_code:protected.status_code,
                source_file:protected.source_file.into(),line_number:protected.line_number,column_number:protected.column_number},
            token,used:Vec::new(),_credits:credits})
    }
    /// Prepare only an authentic native receipt belonging to this admitted
    /// source/navigation. Caller fires its original SPVE before consuming the
    /// resulting existing move-only PreparedCspReportPlan. No live-global lookup
    /// occurs; retirement never restores that global or lends settings to a child.
    pub fn prepare_navigation_csp_report(&mut self,loan:&mut NavigationCspSourceLoan,
        receipt:&CspResponseProvenance,index:usize,queued_at:Instant)->Result<PreparedCspReportPlan,ReportError> {
        if loan.service!=self.reporting.service || !receipt.native_navigation_report_token.as_ref()
            .is_some_and(|token|Arc::ptr_eq(token,&loan.token)) {return Err(ReportError::StaleDocument);}
        let violation=receipt.violations().get(index).ok_or(ReportError::InvalidReport)?;
        if loan.used.iter().any(|(used,_)|used==violation) {return Err(ReportError::InvalidReport);}
        let protected=CspRequestReportContext {element:loan.protected.element,referrer:&loan.protected.referrer,
            status_code:loan.protected.status_code,source_file:&loan.protected.source_file,
            line_number:loan.protected.line_number,column_number:loan.protected.column_number};
        let report=violation.enrich_saved_request_report(&loan.receiver,&protected).map_err(|_|ReportError::InvalidReport)?;
        let bytes=prepared::snapshot_bytes(&loan.settings,&report).ok_or(ReportError::Limit)?;
        // Bound consumed-receipt custody separately, conservatively including
        // the saved policy/URLs. This is app content, not provider memory totals.
        let used_bytes=violation.snapshot_content_bytes().and_then(|n|n.checked_add(std::mem::size_of::<prepared::PreparedCredits>())).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;
        let used_credit=coep_request::graph_credit(&self.reporting.prepared_bytes,used_bytes)?;
        let credits=coep_request::graph_credit(&self.reporting.prepared_bytes,bytes)?;
        loan.used.push((violation.clone(),used_credit));
        Ok(prepared::saved_plan(self.reporting.service,loan.document,loan.settings.clone(),report,queued_at,credits))
    }
    pub fn release_navigation_csp_reporting(&mut self,loan:NavigationCspSourceLoan)->Result<(),ReportError> {
        if loan.service!=self.reporting.service {return Err(ReportError::StaleDocument);}
        drop(loan);Ok(())
    }
}

#[cfg(test)]
#[path="navigation_csp_tests.rs"]
mod tests;
