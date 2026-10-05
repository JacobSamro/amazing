//! Native-only real socket controls. These fixtures supply explicit captured
//! clients; they do not claim installed Window/Document admission.
use super::*;
use crate::{EmbedderPolicyContainer, ReceivingDocumentClient, ReferrerPolicy,
    reporting::NavigationEmbeddingElement, api::NavigableContext, CspDisposition};
use std::{io::{Read,Write},net::TcpListener,thread,time::Duration,sync::mpsc};

fn fixture(service:&mut NetworkService,base:&str)->(crate::reporting::ReportingDocument,ReceivingDocumentClient<u64>,NavigableContext) {
    let url=HttpUrl::parse(&format!("{base}/source?private")).unwrap();
    let mut fields=HeaderList::default();
    fields.append("content-security-policy-report-only","frame-src 'none'").unwrap();
    let mut context=ClientContext::default();context.csp=CspPolicyList::from_headers(&fields).unwrap();
    let source=ReceivingDocumentClient {identity:7,creation_url:url.as_parsed().clone(),
        document_url:url.as_parsed().clone(),base_url:url.as_parsed().clone(),origin:url.origin().clone(),
        csp_self_origin:url.origin().clone(),context,referrer_policy:ReferrerPolicy::UnsafeUrl};
    let parent=NavigableContext {top_level_origin:source.origin.clone(),parent_origin:source.origin.clone(),
        ancestor_origins:vec![source.origin.clone()],sandbox_flags:0,frame_csp:Default::default(),
        referrer:format!("{base}/wrong-parent"),referrer_policy:"no-referrer".into(),now_unix:10};
    let handle=service.register_reporting_initial_client(&source).unwrap();(handle,source,parent)
}
fn owned_request(service:&mut NetworkService,handle:crate::reporting::ReportingDocument,source:&ReceivingDocumentClient<u64>,
    parent:&NavigableContext,target:&str)->ApiRequest {
    let owner=service.prepare_navigation_embedding_owner(handle,source,&EmbedderPolicyContainer::default(),parent,
        NavigationEmbeddingElement::Iframe).unwrap();
    ApiRequest::child_navigation_with_embedding_owner(HttpUrl::parse(target).unwrap(),parent,&source.origin,owner).unwrap()
}
fn read_request(socket:&mut std::net::TcpStream)->String {
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let mut bytes=Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {let mut byte=[0];socket.read_exact(&mut byte).unwrap();bytes.push(byte[0]);}
    String::from_utf8(bytes).unwrap()
}
fn receipt(failure:&NavigationFailure,expected_urls:usize,post:bool) {
    let progress=failure.csp_provenance.as_ref().expect("actual checked request receipt");
    assert_eq!(progress.url_list().len(),expected_urls);
    assert!(!progress.violations().is_empty());
    assert!(progress.violations().iter().all(|value|value.disposition==CspDisposition::ReportOnly));
    assert!(progress.violations().iter().any(|value|value.request_phase()==crate::CspRequestPhase::PreRequest));
    if post {assert!(progress.violations().iter().any(|value|value.request_phase()==crate::CspRequestPhase::PostRequest));}
}
fn terminal(service:&mut NetworkService,id:ServiceId,stream:bool)->Result<ApiResponse,NavigationFailure> {
    let deadline=Instant::now()+Duration::from_secs(10);
    while Instant::now()<deadline {
        for event in service.step(4096,Instant::now()) {match event {
            ServiceEvent::NavigationFinished{id:found,result} if found==id=>{assert!(!stream);return result;},
            ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>{assert!(stream);return Err(failure);},
            ServiceEvent::StreamFinished{id:found} if found==id=>panic!("expected native failure"),
            ServiceEvent::Finished{id:found,..}|ServiceEvent::StreamFailed{id:found,..} if found==id=>panic!("legacy terminal from opt-in request"),
            _=>{}
        }}thread::yield_now();
    }panic!("native terminal timed out")
}
#[test]
fn received_navigation_real_http_terminal_matrix_preserves_checked_policy_and_source_referrer() {
    for streamed in [false,true] {for case in ["truncated","malformed","frame","corp","limit","chunk"] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());
        let (tx,rx)=mpsc::channel();let peer=thread::spawn(move|| {
            let (mut socket,_)=listener.accept().unwrap();tx.send(read_request(&mut socket)).unwrap();
            let response=match case {
                "truncated"=>"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort",
                "malformed"=>"invalid HTTP head\r\n\r\n",
                "frame"=>"HTTP/1.1 200 OK\r\nX-Frame-Options: DENY\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "corp"=>"HTTP/1.1 200 OK\r\nCross-Origin-Resource-Policy: same-origin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "limit"=>"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n12345678901234567890",
                _=>"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\ndata\r\ninvalid\r\n",
            };socket.write_all(response.as_bytes()).unwrap();
        });
        let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,mut parent)=fixture(&mut service,&base);
        // CORP uses the actual owner origin, independently of source origin.
        let owner_container=if case=="corp" {
            let mut fields=HeaderList::default();fields.append("cross-origin-embedder-policy","require-corp").unwrap();
            // Explicit native loopback fixture environment; no installed context claim.
            EmbedderPolicyContainer::from_response(&fields,true)
        }else{EmbedderPolicyContainer::default()};
        let owner_source=if case=="corp" {let mut owner=source.clone();
            owner.context.security.embedder_policy=owner_container.value;let url=HttpUrl::parse("http://localhost:1234/document").unwrap();
            owner.origin=url.origin().clone();owner.csp_self_origin=owner.origin.clone();owner.creation_url=url.as_parsed().clone();
            owner.document_url=url.as_parsed().clone();owner.base_url=url.as_parsed().clone();
            parent.top_level_origin=owner.origin.clone();parent.parent_origin=owner.origin.clone();parent.ancestor_origins=vec![owner.origin.clone()];owner
        }else{source.clone()};
        let owner_handle=if case=="corp" {service.register_reporting_initial_client(&owner_source).unwrap()}else{handle};
        let plan=service.prepare_navigation_embedding_owner(owner_handle,&owner_source,&owner_container,&parent,NavigationEmbeddingElement::Iframe).unwrap();
        let mut request=ApiRequest::child_navigation_with_embedding_owner(HttpUrl::parse(&format!("{base}/{case}")).unwrap(),&parent,&source.origin,plan).unwrap();
        if case=="limit" {request.set_response_body_limit(3).unwrap();}
        let id=if streamed {service.start_navigation_received_stream(request,&source,100,Instant::now()).unwrap()}
            else {service.start_navigation_received(request,&source,Instant::now()).unwrap()};
        assert!(service.forget_reporting_document(handle));if owner_handle!=handle {assert!(service.forget_reporting_document(owner_handle));}
        let failure=terminal(&mut service,id,streamed).unwrap_err();
        receipt(&failure,1,streamed&&matches!(case,"truncated"|"chunk"));
        if case=="frame" {assert_eq!(failure.error,ApiError::Network(FetchError::FrameEmbedding));}
        if case=="corp" {assert_eq!(failure.error,ApiError::Network(FetchError::CrossOriginResourcePolicy));}
        if case=="limit" {assert_eq!(failure.error,ApiError::Network(FetchError::ResponseTooLarge));}
        let wire=rx.recv_timeout(Duration::from_secs(5)).unwrap();assert!(wire.contains(&format!("Referer: {base}/source?private\r\n"))||wire.contains(&format!("referer: {base}/source?private\r\n")),"{wire}");
        assert!(!wire.contains("wrong-parent"));peer.join().unwrap();assert!(service.active.is_empty());
    }}
}
#[test]
fn received_navigation_abort_deadline_and_pending_credit_keep_actual_receipts() {
    for streamed in [false,true] {for after_head in [false,true] {for expire in [false,true] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());
        let (seen_tx,seen_rx)=mpsc::channel();let (release_tx,release_rx)=mpsc::channel();
        let peer=thread::spawn(move|| {let (mut socket,_)=listener.accept().unwrap();read_request(&mut socket);
            if after_head {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\npart").unwrap();}
            seen_tx.send(()).unwrap();release_rx.recv_timeout(Duration::from_secs(10)).unwrap();});
        let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,parent)=fixture(&mut service,&base);
        let request=owned_request(&mut service,handle,&source,&parent,&format!("{base}/hold"));let now=Instant::now();
        let id=if streamed {service.start_navigation_received_stream(request,&source,0,now).unwrap()}else{service.start_navigation_received(request,&source,now).unwrap()};
        let deadline=Instant::now()+Duration::from_secs(5);let mut seen=false;let mut head=false;
        while Instant::now()<deadline && (!seen||(streamed&&after_head&&!head)) {
            if seen_rx.try_recv().is_ok(){seen=true;}
            for event in service.step(4096,Instant::now()) {if matches!(event,ServiceEvent::ResponseHead{id:found,..} if found==id){head=true;}}
            thread::yield_now();
        }assert!(seen);if streamed&&after_head {assert!(head);}
        let failure=if expire {
            let events=service.step(4096,now+Duration::from_secs(3600));
            events.into_iter().find_map(|event|match event {
                ServiceEvent::NavigationFinished{id:found,result:Err(value)} if found==id=>Some(value),
                ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>Some(failure),_=>None,
            }).unwrap_or_else(|| {if streamed {service.grant_credit(id,100);}terminal(&mut service,id,streamed).unwrap_err()})
        }else {assert!(service.cancel(id));if streamed {service.grant_credit(id,100);}terminal(&mut service,id,streamed).unwrap_err()};
        receipt(&failure,1,streamed&&after_head);if !expire {assert_eq!(failure.error,ApiError::Aborted);}
        release_tx.send(()).unwrap();peer.join().unwrap();assert!(service.active.is_empty());
    }}}
}
#[test]
fn received_navigation_admission_errors_preserve_checked_vs_unchecked_and_do_not_admit_socket() {
    let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,parent)=fixture(&mut service,"http://127.0.0.1:8080");
    for bad_port in [true,false] {
        let mut request=owned_request(&mut service,handle,&source,&parent,if bad_port {"http://127.0.0.1:25/blocked"}else{"http://127.0.0.1:8080/aborted"});
        request.signal_aborted=!bad_port;
        let failure=match service.start_navigation_received(request,&source,Instant::now()).unwrap_err() {
            NavigationReceivedStartError::Fetch(value)=>value,_=>panic!("expected native admission error"),
        };
        if bad_port {receipt(&failure,1,false);assert_eq!(failure.error,ApiError::Network(FetchError::BadPort));}
        else {assert_eq!(failure.error,ApiError::Aborted);assert!(failure.csp_provenance.is_none());}
        assert!(service.active.is_empty());assert_eq!(service.dispatcher.in_flight(),0);
    }
    let request=owned_request(&mut service,handle,&source,&parent,"http://127.0.0.1:8080/foreign");
    let mut other=NetworkService::new(DispatchLimits::default());
    assert!(matches!(other.start_navigation_received(request,&source,Instant::now()),Err(NavigationReceivedStartError::Reporting(_))));
    assert_eq!(other.dispatcher.in_flight(),0);
    let request=owned_request(&mut service,handle,&source,&parent,"http://127.0.0.1:8080/mismatch");
    let mut wrong=source.clone();wrong.origin=HttpUrl::parse("http://wrong.test/").unwrap().origin().clone();
    assert!(matches!(service.start_navigation_received(request,&wrong,Instant::now()),Err(NavigationReceivedStartError::Fetch(NavigationFailure{csp_provenance:None,..}))));
    assert_eq!(service.dispatcher.in_flight(),0);
}
#[test]
fn received_navigation_manual_multihop_failure_retains_native_prefix_and_referrer_reduction() {
    for streamed in [false,true] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());
        let (tx,rx)=mpsc::channel();let peer=thread::spawn(move|| {
            for hop in 0..3 {let (mut socket,_)=listener.accept().unwrap();tx.send(read_request(&mut socket)).unwrap();
                let response=match hop {
                    0=>"HTTP/1.1 302 Found\r\nLocation: /second\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    1=>"HTTP/1.1 307 Temporary Redirect\r\nLocation: /third#saved\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    _=>"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nshort",
                };socket.write_all(response.as_bytes()).unwrap();
            }
        });
        let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,parent)=fixture(&mut service,&base);
        let mut request=owned_request(&mut service,handle,&source,&parent,&format!("{base}/first"));
        for hop in 0..3 {
            let id=if streamed {service.start_navigation_received_stream(request.clone_request().unwrap(),&source,100,Instant::now()).unwrap()}
                else {service.start_navigation_received(request.clone_request().unwrap(),&source,Instant::now()).unwrap()};
            if hop==0 {assert!(service.forget_reporting_document(handle));}
            if hop==2 {let failure=terminal(&mut service,id,streamed).unwrap_err();receipt(&failure,3,streamed);
                assert_eq!(failure.csp_provenance.as_ref().unwrap().url_list().last().unwrap().serialize(),format!("{base}/third#saved"));break;}
            let deadline=Instant::now()+Duration::from_secs(10);let mut next=None;let mut done=false;
            while !done && Instant::now()<deadline {
                for event in service.step(4096,Instant::now()) {match event {
                    ServiceEvent::NavigationFinished{id:found,result:Ok(response)} if found==id=>{next=Some(request.continue_navigation(&response).unwrap());done=true;},
                    ServiceEvent::ResponseHead{id:found,head} if found==id=>{assert!(streamed);next=Some(request.continue_navigation_head(&head).unwrap());},
                    ServiceEvent::StreamFinished{id:found} if found==id=>{assert!(streamed);done=true;},
                    ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>panic!("redirect failed: {failure:?}"),
                    _=>{}
                }}thread::yield_now();
            }assert!(done);request=next.unwrap();
        }
        let first=rx.recv_timeout(Duration::from_secs(5)).unwrap();assert!(first.to_ascii_lowercase().contains("referer:"));
        for expected in ["/second","/third"] {let wire=rx.recv_timeout(Duration::from_secs(5)).unwrap();assert!(wire.starts_with(&format!("GET {expected} HTTP/1.1")));assert!(!wire.to_ascii_lowercase().contains("referer:"));}
        peer.join().unwrap();assert!(service.active.is_empty());
    }
}
#[test]
fn received_navigation_buffered_success_fresh_cache_and_queued_abort_keep_native_receipts() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());
    let peer=thread::spawn(move|| {let (mut socket,_)=listener.accept().unwrap();read_request(&mut socket);
        socket.write_all(b"HTTP/1.1 200 OK\r\nCache-Control: max-age=600\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata").unwrap();});
    let mut service=NetworkService::new(DispatchLimits::default());let (handle,mut source,parent)=fixture(&mut service,&base);
    source.context.now_unix=1800000000;
    let request=owned_request(&mut service,handle,&source,&parent,&format!("{base}/cached"));
    let original=request.clone_request().unwrap();let id=service.start_navigation_received(request,&source,Instant::now()).unwrap();
    let response=terminal(&mut service,id,false).unwrap();assert_eq!(response.status,200);
    let progress=response.csp_provenance.as_ref().unwrap();assert_eq!(progress.url_list().len(),1);
    assert!(progress.violations().iter().any(|value|value.request_phase()==crate::CspRequestPhase::PostRequest));
    assert!(response.navigation_metadata().is_some());assert!(response.credential_receipt().is_some());peer.join().unwrap();
    for streamed in [false,true] {
        source.context.now_unix+=1;
        let id=if streamed {service.start_navigation_received_stream(original.clone_request().unwrap(),&source,0,Instant::now()).unwrap()}
            else {service.start_navigation_received(original.clone_request().unwrap(),&source,Instant::now()).unwrap()};
        assert!(service.active.is_empty(),"fresh response must not open socket");assert!(service.cancel(id));
        let failure=terminal(&mut service,id,streamed).unwrap_err();assert_eq!(failure.error,ApiError::Aborted);receipt(&failure,1,true);
    }
}
#[test]
fn received_navigation_connection_failure_keeps_actual_pre_request_receipts() {
    for streamed in [false,true] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());drop(listener);
        let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,parent)=fixture(&mut service,&base);
        let request=owned_request(&mut service,handle,&source,&parent,&format!("{base}/refused"));
        let id=if streamed {service.start_navigation_received_stream(request,&source,0,Instant::now()).unwrap()}
            else {service.start_navigation_received(request,&source,Instant::now()).unwrap()};
        let failure=terminal(&mut service,id,streamed).unwrap_err();receipt(&failure,1,false);
        assert_eq!(failure.error,ApiError::Network(FetchError::Transport));assert!(service.active.is_empty());
    }
}
#[test]
fn received_navigation_literal_data_has_real_policy_receipts_and_no_http_head_or_socket() {
    for streamed in [false,true] {
        let mut service=NetworkService::new(DispatchLimits::default());let (handle,source,parent)=fixture(&mut service,"http://127.0.0.1:8080");
        let request=owned_request(&mut service,handle,&source,&parent,"data:text/plain,literal");
        let id=if streamed {service.start_navigation_received_stream(request,&source,0,Instant::now()).unwrap()}
            else {service.start_navigation_received(request,&source,Instant::now()).unwrap()};
        assert!(service.active.is_empty());assert_eq!(service.dispatcher.in_flight(),0);
        assert!(service.abort(id));let failure=terminal(&mut service,id,streamed).unwrap_err();
        assert_eq!(failure.error,ApiError::Aborted);receipt(&failure,1,false);
    }
}
