//! Authenticated browser Manual-navigation continuation. No response constructor
//! or URL string can replace the actual native response/request binding.
use super::{ApiError, ApiRequest, ApiResponse};
use crate::{FetchError, HeaderList, HttpUrl, RedirectMode, ReferrerPolicy, RequestMode};
use std::sync::Arc;

impl ApiRequest {
    /// Continue the actual accepted Location head from this original native
    /// request. HTML validates its ServiceId and retains the original client.
    /// Owner/source custody, nonce/privacy state and native lineage survive;
    /// a fresh request identity prevents replay of an earlier hop's receipt.
    pub fn continue_navigation(&self, accepted: &ApiResponse) -> Result<Self, ApiError> {
        self.continue_navigation_received(accepted.navigation_metadata(),
            accepted.credential_receipt(), accepted.csp_provenance.as_ref())
    }
    /// The streaming controller uses only the genuinely policy-accepted native
    /// head, carrying the same private request binding as a buffered response.
    pub fn continue_navigation_head(&self, accepted: &crate::service::ApiResponseHead) -> Result<Self, ApiError> {
        self.continue_navigation_received(accepted.navigation.as_ref(),
            accepted.credential_receipt(), accepted.csp_provenance.as_ref())
    }
    fn continue_navigation_received(&self, metadata: Option<&super::NavigationResponseMetadata>,
        credentials: Option<crate::ResponseCredentialReceipt>,
        provenance: Option<&crate::CspResponseProvenance>) -> Result<Self, ApiError> {
        let invalid = || ApiError::Network(FetchError::FrameEmbedding);
        if self.mode != RequestMode::Navigate || self.redirect != RedirectMode::Manual {
            return Err(invalid());
        }
        let metadata = metadata.ok_or_else(invalid)?;
        let current = self.native_navigation_token.as_ref().ok_or_else(invalid)?;
        let received = metadata
            .native_navigation_token
            .as_ref()
            .ok_or_else(invalid)?;
        if !metadata.authentic_received_fields(provenance)
            || !Arc::ptr_eq(current, received)
            || metadata.request_origin != self.origin
            || metadata.owner != self.embedding_owner.as_ref().map(|owner| owner.id())
            || metadata.request_urls.is_empty()
            || metadata
                .request_urls
                .last()
                .map(HttpUrl::without_fragment)
                .as_deref()
                != Some(metadata.final_url.as_str())
            || credentials
                .map(|receipt| receipt.includes_credentials())
                != Some(metadata.request_includes_credentials)
            || provenance
                .map(|receipt| receipt.url_list())
                != Some(metadata.request_urls.as_slice())
        {
            return Err(invalid());
        }
        if !matches!(metadata.status, 301 | 302 | 303 | 307 | 308) {
            return Err(ApiError::Network(FetchError::Redirect));
        }
        if metadata.request_urls.len() > 20 {
            return Err(ApiError::Network(FetchError::RedirectLimit));
        }
        let mut fields = HeaderList::default();
        for (name, value) in &metadata.headers {
            fields.append(name, value).map_err(ApiError::Network)?;
        }
        let location = fields
            .get("location")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::Network(FetchError::Redirect))?;
        let base = metadata.request_urls.last().unwrap();
        let mut target = base.resolve(&location).map_err(ApiError::Network)?;
        if target.includes_credentials() {
            return Err(ApiError::UrlCredentials);
        }
        target.inherit_fragment(base);
        if metadata.status != 303 && self.stream_body {
            return Err(ApiError::Network(FetchError::BodyNotReplayable));
        }
        let rewrite = (matches!(metadata.status, 301 | 302) && self.method == "POST")
            || (metadata.status == 303 && !matches!(self.method.as_str(), "GET" | "HEAD"));
        let mut next = self.clone_request_inner(rewrite)?;
        if rewrite {
            next.method = "GET".into();
            let names = next
                .headers
                .raw_entries()
                .iter()
                .filter(|(name, _)| crate::headers::body_header(name))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            for name in names {
                next.headers.delete_string(&name)?;
            }
        }
        if base.origin() != target.origin() {
            next.headers.delete_string("authorization")?;
        }
        next.referrer = metadata.referrer.clone();
        if let Some(policy) = fields
            .get("referrer-policy")
            .and_then(|value| ReferrerPolicy::parse_header(&value))
        {
            next.referrer_policy = policy;
        }
        next.navigation_urls = metadata.request_urls.clone();
        next.navigation_urls.push(target.clone());
        next.navigation_csp_violations = provenance
            .unwrap()
            .violations()
            .to_vec();
        next.native_navigation_token = Some(Arc::new(crate::model::NativeNavigationToken::new()));
        next.url = target;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiResponseInit, NavigationRequestInit};
    use crate::service::{ClientContext, NetworkService, ServiceEvent, ServiceId};
    use amazing_net::dispatch::DispatchLimits;
    use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, thread, time::{Duration, Instant}};

    fn head(stream: &mut TcpStream) -> String {
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0]; stream.read_exact(&mut byte).unwrap(); bytes.push(byte[0]);
            assert!(bytes.len() < 65536);
        }
        String::from_utf8(bytes).unwrap()
    }
    fn finish(service: &mut NetworkService, id: ServiceId) -> ApiResponse {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            for event in service.step(97, Instant::now()) {
                if let ServiceEvent::Finished { id: found, result } = event {
                    assert_eq!(found,id); return result.unwrap();
                }
            }
            thread::yield_now();
        }
        panic!("actual navigation did not terminate")
    }
    fn request(url: HttpUrl, method: &str) -> ApiRequest {
        let origin = url.origin().clone();
        ApiRequest::navigation(url, NavigationRequestInit {
            method: method.into(), body: if method=="POST" {b"body".to_vec()} else {Vec::new()},
            referrer: "http://source.test/private?q=1".into(),
            referrer_policy: "unsafe-url".into(), initiator_origin: origin,
            reload_or_history: true, client_reserved_id: Some(71),
        }).unwrap()
    }
    #[test]
    fn actual_manual_navigation_continuation_preserves_custody_and_authenticates_each_hop() {
        for (status, method, next_method) in [(301,"POST","GET"),(302,"POST","GET"),(303,"POST","GET"),(307,"POST","POST"),(308,"POST","POST")] {
            let first=TcpListener::bind("127.0.0.1:0").unwrap();
            let second=TcpListener::bind("127.0.0.1:0").unwrap();
            let url=HttpUrl::parse(&format!("http://{}/first#saved",first.local_addr().unwrap())).unwrap();
            let next_url=format!("http://{}/second",second.local_addr().unwrap());
            let peer=thread::spawn(move|| {
                let (mut stream,_)=first.accept().unwrap();
                let wire=head(&mut stream);assert!(wire.starts_with(&format!("{method} /first HTTP/1.1")));
                assert!(wire.to_ascii_lowercase().contains("authorization: saved"));
                if method=="POST" {let mut body=[0;4];stream.read_exact(&mut body).unwrap();assert_eq!(&body,b"body");}
                stream.write_all(format!("HTTP/1.1 {status} Redirect\r\nLocation: {next_url}\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();drop(stream);
                let (mut stream,_)=second.accept().unwrap();let wire=head(&mut stream);
                assert!(wire.starts_with(&format!("{next_method} /second HTTP/1.1")));
                assert!(!wire.to_ascii_lowercase().contains("authorization:"));
                assert!(!wire.to_ascii_lowercase().contains("referer:"));
                if next_method=="POST" {let mut body=[0;4];stream.read_exact(&mut body).unwrap();assert_eq!(&body,b"body");}
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone").unwrap();
            });
            let mut original=request(url.clone(),method);
            original.headers.append_string("authorization","saved").unwrap();
            let nonce="nonce-exact".encode_utf16().collect::<Vec<_>>();
            original.apply_csp_request_element(crate::CspExternalElementKind::Script,
                &crate::CspRequestElement {nonce:Some(&nonce),nonce_attribute_present:true,
                    attributes:&[],duplicate_attribute:false,parser_inserted:true});
            original.destination=crate::Destination::Document;
            let mut service=NetworkService::new(DispatchLimits::default());
            let id=service.start(original.clone_request().unwrap(),ClientContext::default()).unwrap();
            let accepted=finish(&mut service,id);
            assert_eq!(accepted.navigation_metadata().unwrap().status,status);
            assert_eq!(accepted.csp_provenance.as_ref().unwrap().url_list().len(),1);
            let unrelated=request(url,method);
            assert!(matches!(unrelated.continue_navigation(&accepted),Err(ApiError::Network(FetchError::FrameEmbedding))));
            let synthetic=ApiResponse::new(None,ApiResponseInit::default()).unwrap();
            assert!(matches!(original.continue_navigation(&synthetic),Err(ApiError::Network(FetchError::FrameEmbedding))));
            let next=original.continue_navigation(&accepted).unwrap();
            assert_eq!(next.method(),next_method);assert_eq!(next.url().as_parsed().fragment(),Some("saved"));
            let next_policy=next.policy_request().unwrap();
            assert_eq!(next_policy.csp_nonce,original.policy_request().unwrap().csp_nonce);
            assert_eq!(next_policy.csp_nonce.as_deref(),Some("nonce-exact"));
            assert!(next_policy.csp_parser_inserted);
            assert_eq!(next.origin,original.origin);assert!(next.reload_or_history);assert_eq!(next.client_reserved_id,Some(71));
            assert_eq!(next.navigation_urls.len(),2);
            assert!(matches!(next.continue_navigation(&accepted),Err(ApiError::Network(FetchError::FrameEmbedding))));
            let id=service.start(next.clone_request().unwrap(),ClientContext::default()).unwrap();
            let completed=finish(&mut service,id);
            assert_eq!(completed.navigation_metadata().unwrap().status,200);
            assert_eq!(completed.csp_provenance.as_ref().unwrap().url_list().len(),2);
            assert!(completed.credential_receipt().unwrap().includes_credentials());
            assert!(matches!(next.continue_navigation(&completed),Err(ApiError::Network(FetchError::Redirect))));
            peer.join().unwrap();
        }
    }
    #[test]
    fn actual_retired_embedding_owner_survives_buffered_and_streamed_manual_handoffs() {
        use crate::{ReceivingDocumentClient, EmbedderPolicy, EmbedderPolicyContainer,
            reporting::NavigationEmbeddingElement, api::NavigableContext};
        for streamed in [false,true] {
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            let target=HttpUrl::parse(&format!("http://{}/first",listener.local_addr().unwrap())).unwrap();
            let peer=thread::spawn(move|| {
                for (path,response) in [
                    ("/first",b"HTTP/1.1 302 Found\r\nLocation: /second\r\nCross-Origin-Resource-Policy: cross-origin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()),
                    ("/second",b"HTTP/1.1 200 OK\r\nCross-Origin-Resource-Policy: cross-origin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice())] {
                    let (mut stream,_)=listener.accept().unwrap();assert!(head(&mut stream).starts_with(&format!("GET {path} HTTP/1.1")));
                    stream.write_all(response).unwrap();
                }
            });
            let owner_url=HttpUrl::parse("https://container.test/document").unwrap();
            let mut owner_context=ClientContext::default();owner_context.security.embedder_policy=EmbedderPolicy::RequireCorp;
            let owner=ReceivingDocumentClient {identity:(31u64,7u64),creation_url:owner_url.as_parsed().clone(),
                document_url:owner_url.as_parsed().clone(),base_url:owner_url.as_parsed().clone(),origin:owner_url.origin().clone(),
                csp_self_origin:owner_url.origin().clone(),context:owner_context,referrer_policy:ReferrerPolicy::NoReferrer};
            let container=EmbedderPolicyContainer {value:EmbedderPolicy::RequireCorp,..Default::default()};
            let parent=NavigableContext {top_level_origin:owner.origin.clone(),parent_origin:owner.origin.clone(),
                ancestor_origins:vec![owner.origin.clone()],sandbox_flags:1,frame_csp:Default::default(),
                referrer:String::new(),referrer_policy:"no-referrer".into(),now_unix:10};
            let mut service=NetworkService::new(DispatchLimits::default());
            let handle=service.register_reporting_initial_client(&owner).unwrap();
            let plan=service.prepare_navigation_embedding_owner(handle,&owner,&container,&parent,NavigationEmbeddingElement::Iframe).unwrap();
            let owner_id=plan.id();
            let source=HttpUrl::parse("https://source.test/incumbent").unwrap();
            let original=ApiRequest::child_navigation_with_embedding_owner(target,&parent,source.origin(),plan).unwrap();
            service.forget_reporting_document(handle);
            let source_context=ClientContext::default();
            let next=if streamed {
                let id=service.start_navigation_stream_with_embedding_owner(original.clone_request().unwrap(),source_context.clone(),0,Instant::now()).unwrap();
                let deadline=Instant::now()+Duration::from_secs(15);let mut accepted=None;let mut done=false;
                while !done && Instant::now()<deadline {
                    for event in service.step(97,Instant::now()) {match event {
                        ServiceEvent::ResponseHead{id:found,head}=>{assert_eq!(found,id);accepted=Some(head);},
                        ServiceEvent::StreamFinished{id:found}=>{assert_eq!(found,id);done=true;},
                        ServiceEvent::StreamFailed{error,..}=>panic!("actual accepted owner redirect failed: {error:?}"),_=>{}
                    }}thread::yield_now();
                }
                assert!(done);
                let accepted=accepted.unwrap();
                let mut altered=accepted.clone();
                altered.navigation.as_mut().unwrap().headers.push(("location".into(),"http://substitution.test/".into()));
                assert!(matches!(original.continue_navigation_head(&altered),Err(ApiError::Network(FetchError::FrameEmbedding))));
                let mut altered=accepted.clone();altered.navigation.as_mut().unwrap().referrer="http://restored.test/private".into();
                assert!(matches!(original.continue_navigation_head(&altered),Err(ApiError::Network(FetchError::FrameEmbedding))));
                let mut altered=accepted.clone();altered.csp_provenance=None;
                assert!(matches!(original.continue_navigation_head(&altered),Err(ApiError::Network(FetchError::FrameEmbedding))));
                original.continue_navigation_head(&accepted).unwrap()
            } else {
                let id=service.start_navigation_with_embedding_owner(original.clone_request().unwrap(),source_context.clone(),Instant::now()).unwrap();
                original.continue_navigation(&finish(&mut service,id)).unwrap()
            };
            assert_eq!(next.embedding_owner.as_ref().unwrap().id(),owner_id);
            assert_eq!(next.origin,*source.origin());assert!(next.nested_navigation);
            assert_eq!(next.navigation_urls.len(),2);
            let mut foreign=NetworkService::new(DispatchLimits::default());
            assert!(foreign.start_navigation_with_embedding_owner(next.clone_request().unwrap(),source_context.clone(),Instant::now()).is_err());
            assert_eq!(foreign.dispatcher.in_flight(),0);
            let id=service.start_navigation_with_embedding_owner(next,source_context,Instant::now()).unwrap();
            assert_eq!(finish(&mut service,id).navigation_metadata().unwrap().status,200);
            peer.join().unwrap();
        }
    }

}
