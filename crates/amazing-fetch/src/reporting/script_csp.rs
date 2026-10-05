//! Original same-global scalar CSP settings for native classic/module requests.
//! Graph membership is checked by the browser owner, not reconstructed from URLs.
use super::*;
use crate::{api::ApiRequest,ReceivingDocumentClient,CspRequestReportContext,
    CspResponseProvenance,CspViolation,Destination,CspDestination};
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct ScriptCspGraphId {service:u64,generation:u64}
struct ProtectedContext {
    element:Option<u64>,referrer:String,status_code:u16,source_file:String,line_number:u32,column_number:u32,
}
struct SavedSource {
    service:u64,document:ReportingDocument,settings:Document,
    receiver:ReceivingDocumentClient<()>,protected:ProtectedContext,_credits:prepared::PreparedCredits,
}
/// NonClone original-source graph custody, captured only while its actual
/// registered receiving global is live. Neither this nor its loans retain roots.
/// ```compile_fail
/// use amazing_fetch::reporting::ScriptCspGraphReportSettings;
/// fn duplicate(settings:ScriptCspGraphReportSettings){let _=settings.clone();}
/// ```
#[must_use]
pub struct ScriptCspGraphReportSettings {id:ScriptCspGraphId,saved:Arc<SavedSource>}
impl ScriptCspGraphReportSettings {pub fn id(&self)->ScriptCspGraphId {self.id}}
/// A move-only per-request source settings loan; only its authentic native
/// policy receipts can construct plans after the original global retires.
/// ```compile_fail
/// use amazing_fetch::reporting::ScriptCspRequestLoan;
/// fn duplicate(loan:ScriptCspRequestLoan){let _=loan.clone();}
/// ```
#[must_use]
pub struct ScriptCspRequestLoan {
    binding:Arc<NativeScriptCspBinding>,used:Vec<(CspViolation,prepared::PreparedCredits)>,
}
pub(crate) struct NativeScriptCspBinding {
    graph:ScriptCspGraphId,saved:Arc<SavedSource>,admitted:AtomicBool,_credits:prepared::PreparedCredits,
}
impl std::fmt::Debug for NativeScriptCspBinding {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("NativeScriptCspBinding").finish_non_exhaustive()}
}
impl PartialEq for NativeScriptCspBinding {fn eq(&self,other:&Self)->bool {std::ptr::eq(self,other)}}
impl Eq for NativeScriptCspBinding {}
fn same_cookie_site(left:&crate::cookies::CookieSite,right:&crate::cookies::CookieSite)->bool {
    use crate::cookies::CookieSite::*;
    match (left,right) {(NoClient,NoClient)|(CrossSite,CrossSite)=>true,(Origin(a),Origin(b))=>a==b,_=>false}
}
impl NativeScriptCspBinding {
    pub(crate) fn matches_admission(&self,service:u64,origin:&Origin,self_origin:Option<&Origin>,
        mode:RequestMode,destination:Destination,embedding_owner:bool,context:&ClientContext)->bool {
        let receiver=&self.saved.receiver;
        let expected=&receiver.context;
        let mut security=expected.security;security.destination=Destination::Script;
        service==self.saved.service && self.graph.service==service
            && mode!=RequestMode::Navigate && origin==&receiver.origin
            && self_origin==Some(&receiver.csp_self_origin)
            && !embedding_owner && destination==Destination::Script
            && context.security==security && context.csp_destination==CspDestination::Script
            && context.csp==expected.csp && context.frame_ancestors==expected.frame_ancestors
            && context.partition_key==expected.partition_key && same_cookie_site(&context.cookie_site,&expected.cookie_site)
    }
    pub(crate) fn mark_admitted(&self){self.admitted.store(true,Ordering::Release);}
}
impl NetworkService {
    /// Capture at genuine classic preparation or root module-graph admission.
    /// The browser supplies its original protected Document context and retains
    /// actual graph/node membership, nonce and parser/source-element custody.
    pub fn prepare_script_csp_graph_reporting<Identity>(&mut self,document:ReportingDocument,
        receiver:&ReceivingDocumentClient<Identity>,protected:&CspRequestReportContext<'_>)
        ->Result<ScriptCspGraphReportSettings,ReportError> {
        let settings=self.reporting.document(document)?;
        if settings.creation_url!=receiver.creation_url || settings.origin!=receiver.origin {
            return Err(ReportError::InvalidReport);
        }
        let mut bytes=std::mem::size_of::<SavedSource>()
            .checked_add(prepared::document_snapshot_bytes(settings).ok_or(ReportError::Limit)?)
            .ok_or(ReportError::Limit)?;
        let mut add=|n:usize|->Result<(),ReportError>{bytes=bytes.checked_add(n).filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;Ok(())};
        add(std::mem::size_of::<ReceivingDocumentClient<()>>())?;
        add(prepared::client_context_bytes(&receiver.context).ok_or(ReportError::Limit)?.checked_mul(2).ok_or(ReportError::Limit)?)?;
        for url in [&receiver.creation_url,&receiver.document_url,&receiver.base_url] {add(url.serialize().len())?;}
        for origin in [&receiver.origin,&receiver.csp_self_origin] {add(origin.serialize().len())?;}
        add(protected.referrer.len())?;add(protected.source_file.len())?;
        let credits=coep_request::graph_credit(&self.reporting.prepared_bytes,bytes)?;
        let mut settings=settings.clone();
        let generation=self.reporting.next_graph;
        self.reporting.next_graph=generation.checked_add(1).ok_or(ReportError::Limit)?;
        settings.context=receiver.context.clone();settings.referrer_policy=receiver.referrer_policy;
        Ok(ScriptCspGraphReportSettings {id:ScriptCspGraphId {service:self.reporting.service,generation},
            saved:Arc::new(SavedSource {service:self.reporting.service,document,settings,
                receiver:ReceivingDocumentClient {identity:(),creation_url:receiver.creation_url.clone(),
                    document_url:receiver.document_url.clone(),base_url:receiver.base_url.clone(),origin:receiver.origin.clone(),
                    csp_self_origin:receiver.csp_self_origin.clone(),context:receiver.context.clone(),referrer_policy:receiver.referrer_policy},
                protected:ProtectedContext {element:protected.element,referrer:protected.referrer.into(),status_code:protected.status_code,
                    source_file:protected.source_file.into(),line_number:protected.line_number,column_number:protected.column_number},_credits:credits})})
    }
    /// Issue only for the browser's validated actual Script graph node. The
    /// real request's nonce/parser data remains unchanged. Source origin and
    /// protected CSP self-origin are validated separately; no target derivation.
    pub fn issue_script_csp_request_reporting(&mut self,graph:&mut ScriptCspGraphReportSettings,
        id:ScriptCspGraphId,request:&mut ApiRequest)->Result<ScriptCspRequestLoan,ReportError> {
        if id!=graph.id || id.service!=self.reporting.service {return Err(ReportError::StaleDocument);}
        if !request.can_claim_script_csp_reporting() {return Err(ReportError::InvalidReport);}
        request.apply_receiving_csp_client(&graph.saved.receiver).map_err(|_|ReportError::InvalidReport)?;
        let bytes=std::mem::size_of::<NativeScriptCspBinding>()+std::mem::size_of::<ScriptCspRequestLoan>();
        let credits=coep_request::graph_credit(&self.reporting.prepared_bytes,bytes)?;
        let binding=Arc::new(NativeScriptCspBinding {graph:id,saved:graph.saved.clone(),admitted:AtomicBool::new(false),_credits:credits});
        if !request.claim_script_csp_reporting(binding.clone()) {return Err(ReportError::InvalidReport);}
        Ok(ScriptCspRequestLoan {binding,used:Vec::new()})
    }
    /// Capture plans before SPVE; queue each plan after the actual callback,
    /// including author errors/retirement. Observer snapshots come from these
    /// canonical plans independently of endpoint admission or delivery outcomes.
    pub fn prepare_script_csp_report(&mut self,loan:&mut ScriptCspRequestLoan,
        receipt:&CspResponseProvenance,index:usize,queued_at:Instant)->Result<PreparedCspReportPlan,ReportError> {
        let binding=&loan.binding;
        if binding.saved.service!=self.reporting.service || !binding.admitted.load(Ordering::Acquire)
            || !receipt.native_script_report_token.as_ref().is_some_and(|token|Arc::ptr_eq(token,binding)) {
            return Err(ReportError::StaleDocument);
        }
        let violation=receipt.violations().get(index).ok_or(ReportError::InvalidReport)?;
        if loan.used.iter().any(|(used,_)|used==violation) {return Err(ReportError::InvalidReport);}
        let saved=&binding.saved;let protected=&saved.protected;
        let source=CspRequestReportContext {element:protected.element,referrer:&protected.referrer,status_code:protected.status_code,
            source_file:&protected.source_file,line_number:protected.line_number,column_number:protected.column_number};
        let report=violation.enrich_saved_request_report(&saved.receiver,&source).map_err(|_|ReportError::InvalidReport)?;
        let bytes=prepared::snapshot_bytes(&saved.settings,&report).ok_or(ReportError::Limit)?;
        let used_bytes=violation.snapshot_content_bytes().and_then(|n|n.checked_add(std::mem::size_of::<prepared::PreparedCredits>()))
            .filter(|n|*n<=MAX_REPORT_BYTES).ok_or(ReportError::Limit)?;
        let used_credit=coep_request::graph_credit(&self.reporting.prepared_bytes,used_bytes)?;
        let credits=coep_request::graph_credit(&self.reporting.prepared_bytes,bytes)?;
        loan.used.push((violation.clone(),used_credit));
        Ok(prepared::saved_plan(self.reporting.service,saved.document,saved.settings.clone(),report,queued_at,credits))
    }
    pub fn release_script_csp_request_reporting(&mut self,loan:ScriptCspRequestLoan)->Result<(),ReportError> {
        if loan.binding.saved.service!=self.reporting.service {return Err(ReportError::StaleDocument);}drop(loan);Ok(())
    }
    /// Consume graph admission at genuine terminal cleanup. Existing node loans
    /// drain under their original settings; no future dependency can be issued.
    pub fn release_script_csp_graph_reporting(&mut self,graph:ScriptCspGraphReportSettings,
        id:ScriptCspGraphId)->Result<(),ReportError> {
        if id!=graph.id || id.service!=self.reporting.service {return Err(ReportError::StaleDocument);}drop(graph);Ok(())
    }
}

#[cfg(test)]
#[path="script_csp_tests.rs"]
mod tests;
