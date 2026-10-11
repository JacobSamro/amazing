//! XHR's native, policy-validated Fetch stream. The driver owns callbacks and
//! must fence each returned event against the original receiving generation.
use super::*;
use crate::service::{ApiResponseHead, NetworkService, ServiceEvent, ServiceId};
use std::time::Instant;

pub(super) struct StreamResponse {
    pub head: ApiResponseHead,
    pub headers: ApiHeaders,
    pub url: String,
    pub body_present: bool,
}

/// One original send's ownership. No constructor or Clone can mint another
/// consumer for a service request. Cancellation suppresses subsequent receipts.
pub struct XhrServiceRequest {
    service: u64,
    id: ServiceId,
    generation: u64,
    active: bool,
    streaming: bool,
}
impl XhrServiceRequest {
    pub fn id(&self) -> ServiceId { self.id }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn is_active(&self) -> bool { self.active }
    pub fn cancel(&mut self, service: &mut NetworkService) -> bool {
        if !self.active || self.service != service.reporting.service_id() { return false; }
        self.active = false;
        service.cancel(self.id);
        true
    }
}
impl Xhr {
    /// The caller captures the actual receiving client before loadstart author
    /// callbacks. Target URLs supply no receiving origin or policy authority.
    pub fn start_service_received<Identity>(&mut self, service: &mut NetworkService,
        receiving: &crate::ReceivingDocumentClient<Identity>, now_ms: u64, now: Instant,
    ) -> Result<XhrServiceRequest, XhrError> {
        if !self.asynchronous || self.state != XhrState::Opened || !self.send_flag {
            return Err(XhrError::InvalidStateSend);
        }
        if self.expired(now_ms) { return Err(XhrError::Timeout); }
        self.prepare_service_received(receiving)?;
        let request=self.request.as_ref().ok_or(XhrError::InvalidStateSend)?.clone_request().map_err(XhrError::from)?;
        let context=self.native_client.clone().ok_or(XhrError::InvalidStateSend)?;
        // XHR 'handle response body chunk': the first chunk is never throttled.
        self.stream_progress_ms = u64::MAX;
        self.stream_upload_ms = now_ms;
        let id = service.start_stream_at(request, context, now).map_err(XhrError::from)?;
        if !self.upload_complete && !service.track_stream_upload(id) {service.cancel(id);return Err(XhrError::InvalidStateSend);}
        Ok(XhrServiceRequest {service: service.reporting.service_id(), id,
            generation: self.generation, active: true, streaming: true})
    }

    /// Convert an actual service admission failure into the request error
    /// steps. Browser drivers enqueue these events on the original task scope.
    pub fn service_start_failed(&mut self,error:XhrError)->Result<Vec<XhrEvent>,XhrError> {
        if !self.send_flag || self.state!=XhrState::Opened {return Err(XhrError::InvalidStateSend);}
        self.fail(error)
    }
    /// Capture ordinary scalar Fetch policy before any loadstart callback.
    /// The driver separately retains the immutable native receiving identity.
    pub fn prepare_service_received<Identity>(&mut self,
        receiving:&crate::ReceivingDocumentClient<Identity>)->Result<(),XhrError> {
        if self.state!=XhrState::Opened || !self.send_flag {return Err(XhrError::InvalidStateSend);}
        let request=self.request.as_mut().ok_or(XhrError::InvalidStateSend)?;
        request.apply_receiving_csp_client(receiving).map_err(|_|XhrError::InvalidStateSend)?;
        request.referrer_policy=receiving.referrer_policy;
        request.referrer=HttpUrl::from_parsed(receiving.document_url.clone()).ok()
            .filter(|url|matches!(url.as_parsed().scheme(),"http"|"https"))
            .map(|url|url.serialize()).unwrap_or_default();
        let mut context=receiving.context.clone();context.csp_destination=crate::CspDestination::Connect;
        self.native_client=Some(context);
        Ok(())
    }
    /// Installed synchronous XHR uses the same service's policy/cookie/cache
    /// transport, while its driver retains unrelated asynchronous receipts.
    /// No default client can authorize this route.
    pub fn start_buffered_service(&mut self,service:&mut NetworkService,now_ms:u64,now:Instant)
        ->Result<XhrServiceRequest,XhrError> {
        if self.asynchronous || self.state!=XhrState::Opened || !self.send_flag {return Err(XhrError::InvalidStateSend);}
        if self.expired(now_ms) {return Err(XhrError::Timeout);}
        let request=self.request.as_ref().ok_or(XhrError::InvalidStateSend)?.clone_request().map_err(XhrError::from)?;
        let context=self.native_client.clone().ok_or(XhrError::InvalidStateSend)?;
        let id=service.start_at(request,context,now).map_err(XhrError::from)?;
        Ok(XhrServiceRequest{service:service.reporting.service_id(),id,generation:self.generation,active:true,streaming:false})
    }
    pub fn on_buffered_service_event(&mut self,pending:&mut XhrServiceRequest,service:&mut NetworkService,
        event:ServiceEvent,now_ms:u64)->Result<Vec<XhrEvent>,XhrError> {
        if pending.streaming || pending.service!=service.reporting.service_id() {return Err(XhrError::InvalidStateSend);}
        if !pending.active {return Ok(Vec::new());}
        if pending.generation!=self.generation || !self.send_flag {pending.cancel(service);return Ok(Vec::new());}
        if self.expired(now_ms) {pending.cancel(service);return self.fail(XhrError::Timeout);}
        let ServiceEvent::Finished{id,result}=event else {return Ok(Vec::new());};
        if id!=pending.id {return Err(XhrError::InvalidStateSend);}
        pending.active=false;
        match result {Ok(response)=>self.install_response(response),Err(error)=>self.fail(XhrError::from(error))}
    }

    /// Consume only this send's actual service receipts. Foreign events remain
    /// the caller's responsibility; they must not be discarded by a shared pump.
    pub fn on_service_event(&mut self, pending: &mut XhrServiceRequest,
        service: &mut NetworkService, event: &ServiceEvent, now_ms: u64,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        if !pending.streaming || pending.service != service.reporting.service_id() { return Err(XhrError::InvalidStateSend); }
        if !pending.active { return Ok(Vec::new()); }
        if pending.generation != self.generation || !self.send_flag {
            pending.cancel(service);
            return Ok(Vec::new());
        }
        let id = match event {
            ServiceEvent::RequestBodyProgress {id,..} | ServiceEvent::ResponseHead {id,..} | ServiceEvent::BodyChunk {id,..}
            | ServiceEvent::StreamFinished {id} | ServiceEvent::StreamFailed {id,..} => *id,
            _ => return Ok(Vec::new()),
        };
        if id != pending.id { return Ok(Vec::new()); }
        if self.expired(now_ms) {
            pending.cancel(service);
            return self.fail(XhrError::Timeout);
        }
        match event {
            ServiceEvent::RequestBodyProgress {sent,total,complete,..} => {
                if self.upload_complete {return Ok(Vec::new());}
                self.upload_total=Some(*total);
                if *complete {return Ok(self.upload_success_events());}
                if !self.upload_has_listeners || now_ms.saturating_sub(self.stream_upload_ms)<50 {return Ok(Vec::new());}
                self.stream_upload_ms=now_ms;
                Ok(vec![self.event(XhrEventTarget::Upload,XhrEventKind::Progress,*sent,Some(*total))])
            }
            ServiceEvent::ResponseHead {head,..} => {
                if self.state != XhrState::Opened || self.stream_response.is_some() {
                    pending.cancel(service);
                    return self.fail(XhrError::BodyRead);
                }
                let mut headers = ApiHeaders::new(HeaderGuard::None);
                for (name,value) in &head.headers {
                    headers.append_network(name,value).map_err(XhrError::from)?;
                }
                headers.set_guard(HeaderGuard::Immutable);
                let url = head.final_url.split('#').next().unwrap_or("").to_owned();
                let body_present = self.method != "HEAD" && !matches!(head.status,101|103|204|205|304);
                self.stream_response = Some(StreamResponse {head:head.clone(),headers,url,body_present});
                self.state = XhrState::HeadersReceived;
                Ok(vec![self.event(XhrEventTarget::Request,XhrEventKind::ReadyStateChange,0,None)])
            }
            ServiceEvent::BodyChunk {bytes,..} => {
                if self.stream_response.is_none() || !matches!(self.state,XhrState::HeadersReceived|XhrState::Loading) {
                    pending.cancel(service);
                    return self.fail(XhrError::BodyRead);
                }
                // Accumulate every byte immediately; only event delivery is
                // throttled. responseText decodes this full prefix per XHR.
                self.received.extend_from_slice(bytes);
                service.grant_credit(pending.id,bytes.len());
                if self.stream_progress_ms != u64::MAX && now_ms.saturating_sub(self.stream_progress_ms) < 50 { return Ok(Vec::new()); }
                self.stream_progress_ms = now_ms;
                self.state = XhrState::Loading;
                Ok(vec![
                    self.event(XhrEventTarget::Request,XhrEventKind::ReadyStateChange,self.received.len() as u64,self.response_length()),
                    self.event(XhrEventTarget::Request,XhrEventKind::Progress,self.received.len() as u64,self.response_length()),
                ])
            }
            ServiceEvent::StreamFinished {..} => {
                pending.active = false;
                if self.stream_response.is_none() { return self.fail(XhrError::BodyRead); }
                Ok(self.complete())
            }
            ServiceEvent::StreamFailed {error,..} => {
                pending.active = false;
                self.fail(if matches!(error,ApiError::Aborted) {XhrError::Abort} else {XhrError::from(error.clone())})
            }
            _ => unreachable!(),
        }
    }
    pub fn check_service_timeout(&mut self, pending:&mut XhrServiceRequest,
        service:&mut NetworkService, now_ms:u64) -> Result<Option<Vec<XhrEvent>>,XhrError> {
        if pending.service != service.reporting.service_id() { return Err(XhrError::InvalidStateSend); }
        if !pending.active { return Ok(None); }
        if pending.generation != self.generation || !self.send_flag {
            pending.cancel(service);
            return Ok(None);
        }
        if self.expired(now_ms) {
            pending.cancel(service);
            return self.fail(XhrError::Timeout).map(Some);
        }
        Ok(None)
    }
    pub(super) fn response_headers(&self) -> Option<&ApiHeaders> {
        self.response.as_ref().map(|r| &r.headers)
            .or_else(|| self.stream_response.as_ref().map(|r| &r.headers))
    }
    pub(super) fn response_document_url(&self)->&str {
        self.response.as_ref().map(|response|response.url.as_str())
            .or_else(||self.stream_response.as_ref().map(|response|response.head.final_url.as_str())).unwrap_or("")
    }
    pub(super) fn has_response(&self) -> bool { self.response.is_some() || self.stream_response.is_some() }
    pub(super) fn response_body_present(&self) -> bool {
        self.response.as_ref().map(|r| r.body.shared_present())
            .or_else(|| self.stream_response.as_ref().map(|r| r.body_present)).unwrap_or(false)
    }
}
