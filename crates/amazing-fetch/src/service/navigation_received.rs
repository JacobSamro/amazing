//! Opt-in native navigation outcomes retain real checks before later failures.
//! The browser supplies its original saved receiving client; no URL lookup or
//! unchecked receiving-policy default creates reporting authority.
use super::*;
use crate::ReceivingDocumentClient;

#[derive(Debug, Clone)]
pub struct NavigationFailure {
    pub error: ApiError,
    /// None means no native CSP check occurred, including an aborted admission.
    pub csp_provenance: Option<crate::CspResponseProvenance>,
}
#[derive(Debug)]
pub enum NavigationReceivedStartError {
    Reporting(crate::reporting::ReportError),
    Fetch(NavigationFailure),
}
impl NetworkService {
    pub fn start_navigation_received<Identity>(&mut self, mut request: ApiRequest,
        receiving: &ReceivingDocumentClient<Identity>, now: Instant,
    ) -> Result<ServiceId, NavigationReceivedStartError> {
        request.apply_navigation_receiving_client(receiving).map_err(|_| NavigationReceivedStartError::Fetch(
            NavigationFailure {error:ApiError::Network(FetchError::FrameEmbedding),csp_provenance:None}))?;
        if request.mode != crate::RequestMode::Navigate || request.redirect != crate::RedirectMode::Manual {
            return Err(NavigationReceivedStartError::Reporting(crate::reporting::ReportError::InvalidReport));
        }
        let owner=request.embedding_owner.as_ref().ok_or(NavigationReceivedStartError::Reporting(
            crate::reporting::ReportError::InvalidReport))?;
        let settings=owner.loan(self).map_err(NavigationReceivedStartError::Reporting)?;
        let mut context=receiving.context.clone();
        context.csp_destination=CspDestination::Frame;
        let mut policy=request.policy_request().map_err(|error| NavigationReceivedStartError::Fetch(
            NavigationFailure {error,csp_provenance:None}))?;
        policy.security=context.security;
        settings.validate(self,&policy).map_err(NavigationReceivedStartError::Reporting)?;
        let mut progress=None;
        let id=self.start_at_inner(request,context,now,Some(settings),&mut progress)
            .map_err(|error| NavigationReceivedStartError::Fetch(NavigationFailure {error,csp_provenance:progress.clone()}))?;
        if let Some(task)=self.active.get_mut(&id) {task.received_navigation=true;}
        else if let Some(event)=self.ready.iter_mut().find(|event|matches!(event,ServiceEvent::Finished{id:found,..} if *found==id)) {
            let ServiceEvent::Finished{result,..}=std::mem::replace(event,ServiceEvent::StreamFinished{id}) else {unreachable!()};
            *event=ServiceEvent::NavigationFinished{id,result:result.map_err(|error|NavigationFailure{error,csp_provenance:progress})};
        }
        Ok(id)
    }
    pub fn start_navigation_received_stream<Identity>(&mut self, request:ApiRequest,
        receiving:&ReceivingDocumentClient<Identity>,credit:u64,now:Instant,
    )->Result<ServiceId,NavigationReceivedStartError> {
        let credit=usize::try_from(credit).map_err(|_|NavigationReceivedStartError::Fetch(
            NavigationFailure{error:ApiError::Network(FetchError::ResponseTooLarge),csp_provenance:None}))?;
        let id=self.start_navigation_received(request,receiving,now)?;
        let mut stream=StreamTask::new(credit);stream.received_navigation=true;
        self.streams.insert(id,stream);Ok(id)
    }
    pub(super) fn finish_navigation_stream(&mut self,id:ServiceId,result:Result<ApiResponse,NavigationFailure>,output:&mut Vec<ServiceEvent>) {
        match result {
            Ok(response)=> {
                let progress=response.csp_provenance.clone();
                self.finish_stream(id,Ok(response),output);
                if let Some(stream)=self.streams.get_mut(&id) {
                    if let Some(progress)=progress {stream.navigation_progress=Some(progress);}
                    if let Some(Err(error))=&stream.terminal {
                        stream.navigation_failure=Some(NavigationFailure{error:error.clone(),csp_provenance:stream.navigation_progress.clone()});
                    }
                }
            }
            Err(failure)=> {
                if let Some(stream)=self.streams.get_mut(&id) {
                    stream.navigation_failure=Some(failure.clone());
                }
                self.finish_stream(id,Err(failure.error),output);
            }
        }
    }
}
pub(super) fn pending_navigation_failure(task:&Task,stream:Option<&StreamTask>,error:ApiError)->NavigationFailure {
    navigation_failure_with_progress(task,stream,None,error)
}
pub(super) fn navigation_failure_with_progress(task:&Task,stream:Option<&StreamTask>,
    terminal_progress:Option<&crate::CspResponseProvenance>,error:ApiError)->NavigationFailure {
    // A streamed head can perform genuine post-response checks before a later
    // framing/transport failure. Retain that actual receipt, alongside the
    // pending request's real pre/hop records; never repeat a policy check.
    let mut actual=Vec::new();
    for progress in stream.and_then(|stream|stream.navigation_progress.as_ref()).into_iter().chain(terminal_progress) {
        if progress.url_list()==task.pending.policy().url_list() {actual.extend_from_slice(progress.violations());}
    }
    let csp_provenance=task.pending.policy().csp_pre_request_checked.then(||
        crate::CspResponseProvenance::checked(task.pending.policy(),actual));
    NavigationFailure{error,csp_provenance}
}

#[cfg(test)]
#[path="navigation_received_tests.rs"]
mod tests;
