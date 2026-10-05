//! Actual native source HTTP admission/request/report controls. No Window proof.
use super::*;
use crate::{api::{NavigationRequestInit,NavigableContext},EmbedderPolicyContainer,ReferrerPolicy,
    service::{ServiceId,NavigationFailure,NavigationReceivedStartError},CspPolicyList};
use amazing_net::dispatch::DispatchLimits;
use std::{io::{Read,Write},net::TcpListener,thread,time::Duration,sync::{Mutex,mpsc}};
struct Peer {base:String,stop:mpsc::Sender<()>,thread:thread::JoinHandle<()>,requests:Arc<Mutex<Vec<String>>>}
impl Peer {
    fn new(endpoint:bool)->Self {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let base=format!("http://127.0.0.1:{}",listener.local_addr().unwrap().port());let (stop,rx)=mpsc::channel();
        let requests=Arc::new(Mutex::new(Vec::new()));let saved=requests.clone();
        let thread=thread::spawn(move|| {
            let deadline=Instant::now()+Duration::from_secs(30);
            while Instant::now()<deadline {
                match rx.try_recv(){Err(mpsc::TryRecvError::Empty)=>{},_=>return}
                let (mut socket,_)=match listener.accept(){Ok(value)=>value,Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{thread::sleep(Duration::from_millis(1));continue},Err(error)=>panic!("accept: {error}")};
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();let mut bytes=Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {let mut byte=[0];socket.read_exact(&mut byte).unwrap();bytes.push(byte[0]);}
                let head=String::from_utf8(bytes.clone()).unwrap();let len=head.lines().find_map(|line|line.split_once(':')
                    .filter(|(name,_)|name.eq_ignore_ascii_case("content-length")).map(|(_,value)|value.trim().parse::<usize>().unwrap())).unwrap_or(0);
                let mut body=vec![0;len];socket.read_exact(&mut body).unwrap();bytes.extend(body);
                let request=String::from_utf8(bytes).unwrap();let path=request.split_whitespace().nth(1).unwrap().to_owned();saved.lock().unwrap().push(request);
                let response=match path.as_str() {
                    "/source"=>format!("HTTP/1.1 200 OK\r\nContent-Security-Policy-Report-Only: frame-src 'none'; report-to default\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",if endpoint {"Reporting-Endpoints: default=\"/report\"\r\n"}else{""}),
                    "/first"=>"HTTP/1.1 302 Found\r\nLocation: /second\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/report"=>"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into(),
                    "/broken"=>"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nshort".into(),
                    "/hold"=>{thread::sleep(Duration::from_millis(150));"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()},
                    _=>"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                };let _=socket.write_all(response.as_bytes());
            }panic!("peer deadline")
        });Self {base,stop,thread,requests}
    }
    fn url(&self,path:&str)->HttpUrl {HttpUrl::parse(&format!("{}{path}",self.base)).unwrap()}
    fn close(self)->Vec<String>{self.stop.send(()).unwrap();self.thread.join().unwrap();Arc::try_unwrap(self.requests).unwrap().into_inner().unwrap()}
}
fn source(service:&mut NetworkService,peer:&Peer)->(ReportingDocument,ReceivingDocumentClient<u64>,NavigableContext,CspRequestReportContext<'static>) {
    let url=peer.url("/source");let request=ApiRequest::navigation(url.clone(),NavigationRequestInit {method:"GET".into(),body:vec![],
        referrer:String::new(),referrer_policy:"no-referrer".into(),initiator_origin:url.origin().clone(),reload_or_history:false,client_reserved_id:None}).unwrap();
    let id=service.start_at(request,ClientContext::default(),Instant::now()).unwrap();let deadline=Instant::now()+Duration::from_secs(8);let mut response=None;
    while response.is_none()&&Instant::now()<deadline {for event in service.step(4096,Instant::now()) {
        if let ServiceEvent::Finished{id:found,result}=event {
            if found==id {response=Some(result.unwrap());}
        }
    }thread::yield_now();}
    let response=response.unwrap();let metadata=response.navigation_metadata().unwrap();let mut headers=HeaderList::default();
    for (name,value) in &metadata.headers {headers.append(name,value).unwrap();}
    let mut context=ClientContext::default();context.csp=CspPolicyList::from_headers(&headers).unwrap();context.now_unix=1800000000;
    let receiver=ReceivingDocumentClient {identity:31,creation_url:url.as_parsed().clone(),document_url:url.as_parsed().clone(),base_url:url.as_parsed().clone(),
        origin:url.origin().clone(),csp_self_origin:url.origin().clone(),context,referrer_policy:ReferrerPolicy::NoReferrer};
    let handle=service.register_reporting_client_response(&receiver,&url,&headers).unwrap();
    let parent=NavigableContext {top_level_origin:receiver.origin.clone(),parent_origin:receiver.origin.clone(),ancestor_origins:vec![receiver.origin.clone()],
        sandbox_flags:0,frame_csp:receiver.context.csp.clone(),referrer:String::new(),referrer_policy:"no-referrer".into(),now_unix:1800000000};
    assert_eq!(metadata.referrer,"");let protected=CspRequestReportContext {element:None,referrer:"",status_code:metadata.status,source_file:"",line_number:0,column_number:0};
    (handle,receiver,parent,protected)
}
fn owned_request(service:&mut NetworkService,handle:ReportingDocument,receiver:&ReceivingDocumentClient<u64>,parent:&NavigableContext,url:HttpUrl)->ApiRequest {
    let owner=service.prepare_navigation_embedding_owner(handle,receiver,&EmbedderPolicyContainer::default(),parent,NavigationEmbeddingElement::Iframe).unwrap();
    ApiRequest::child_navigation_with_embedding_owner(url,parent,&receiver.origin,owner).unwrap()
}
fn outcome(service:&mut NetworkService,id:ServiceId,stream:bool)->Result<crate::api::ApiResponse,NavigationFailure> {
    let deadline=Instant::now()+Duration::from_secs(8);
    while Instant::now()<deadline {for event in service.step(4096,Instant::now()) {match event {
        ServiceEvent::NavigationFinished{id:found,result} if found==id=>{assert!(!stream);return result},
        ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>{assert!(stream);return Err(failure)},_=>{}
    }}thread::yield_now();}panic!("native outcome deadline")
}
fn submit_all(service:&mut NetworkService,loan:&mut NavigationCspSourceLoan,receipt:&CspResponseProvenance,now:Instant)->usize {
    let mut count=0;for index in 0..receipt.violations().len() {
        let plan=service.prepare_navigation_csp_report(loan,receipt,index,now).unwrap();
        assert_eq!(plan.violation().document_uri,loan.receiver.document_url.serialize());
        assert_eq!(plan.violation().status_code,loan.protected.status_code);
        assert_eq!(plan.report_snapshot().url,loan.receiver.creation_url.serialize().encode_utf16().collect::<Vec<_>>());
        // Actual adapter fires SPVE here; this native control models an error
        // outcome without fabricating an installed event/Window-retirement proof.
        let author_result:Result<(),&str>=Err("listener error");assert!(author_result.is_err());
        count+=service.queue_prepared_csp_report(plan,now).unwrap().len();
        assert!(matches!(service.prepare_navigation_csp_report(loan,receipt,index,now),Err(ReportError::InvalidReport)));
    }count
}
fn deliveries(service:&mut NetworkService,count:usize,original:ReportingDocument) {
    let deadline=Instant::now()+Duration::from_secs(8);let mut done=0;
    while done<count&&Instant::now()<deadline {for event in service.step(4096,Instant::now()) {
        if let ServiceEvent::ReportFinished(result)=event {assert_eq!(result.document,original);assert!(matches!(result.outcome,ReportOutcome::Response{status:204,accepted:true}));done+=1;}
    }thread::yield_now();}assert_eq!(done,count);
}
#[test]
fn navigation_source_loan_real_posts_after_retirement_and_listener_error_preserve_original_settings() {
    for stream in [false,true] {
        let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
        let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
        let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();let now=Instant::now();
        let id=if stream {service.start_navigation_received_stream(request,&receiver,100,now).unwrap()}else{service.start_navigation_received(request,&receiver,now).unwrap()};
        assert!(service.forget_reporting_document(handle));let failure=outcome(&mut service,id,stream).unwrap_err();let receipt=failure.csp_provenance.unwrap();
        let count=submit_all(&mut service,&mut loan,&receipt,now);assert!(count>0);deliveries(&mut service,count,handle);
        assert!(service.reporting.document(handle).is_err());service.release_navigation_csp_reporting(loan).unwrap();
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        let requests=peer.close();let posts=requests.iter().filter(|request|request.starts_with("POST /report ")).collect::<Vec<_>>();assert_eq!(posts.len(),count);
        for post in posts {assert!(post.to_ascii_lowercase().contains("content-type: application/reports+json"));assert!(post.contains("\"type\":\"csp-violation\""));assert!(post.contains("\"statusCode\":200"));assert!(post.contains("/source"));assert!(!post.contains("\"documentURL\":\"/broken"));}
    }
}
#[test]
fn navigation_source_loan_no_endpoint_creation_is_independent_and_original_global_stays_retired() {
    let peer=Peer::new(false);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
    let id=service.start_navigation_received(request,&receiver,Instant::now()).unwrap();assert!(service.forget_reporting_document(handle));
    let receipt=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    let plan=service.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()).unwrap();
    assert_eq!(plan.report_snapshot().visibility,crate::CspReportVisibility::ObserverVisible);
    assert_eq!(plan.violation().effective_directive,"frame-src");assert_eq!(plan.violation().status_code,200);
    assert_eq!(service.queue_prepared_csp_report(plan,Instant::now()),Err(ReportError::NoEndpoint));
    assert!(matches!(service.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()),Err(ReportError::InvalidReport)));
    assert!(service.reporting.document(handle).is_err());service.release_navigation_csp_reporting(loan).unwrap();
    let requests=peer.close();assert!(requests.iter().all(|request|!request.starts_with("POST ")));
}
#[test]
fn navigation_source_loan_abort_deadline_and_early_policy_errors_retain_genuine_source_reports() {
    for stream in [false,true] {for cause in ["abort","deadline","bad-port"] {
        let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
        let target=if cause=="bad-port" {HttpUrl::parse("http://127.0.0.1:25/rejected").unwrap()}else{peer.url("/hold")};
        let mut request=owned_request(&mut service,handle,&receiver,&parent,target);
        let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();let now=Instant::now();
        assert!(service.forget_reporting_document(handle));
        let started=if stream {service.start_navigation_received_stream(request,&receiver,0,now)}else{service.start_navigation_received(request,&receiver,now)};
        let failure=if cause=="bad-port" {match started.unwrap_err(){NavigationReceivedStartError::Fetch(failure)=>failure,_=>panic!("wrong native error")}}
            else {let id=started.unwrap();if cause=="abort" {assert!(service.cancel(id));outcome(&mut service,id,stream).unwrap_err()}
            else {service.step(4096,now+Duration::from_secs(3600)).into_iter().find_map(|event|match event {
                ServiceEvent::NavigationFinished{id:found,result:Err(failure)} if found==id=>Some(failure),
                ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>Some(failure),_=>None}).unwrap()}};
        let receipt=failure.csp_provenance.unwrap();let count=submit_all(&mut service,&mut loan,&receipt,now);assert!(count>0);deliveries(&mut service,count,handle);
        service.release_navigation_csp_reporting(loan).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        assert_eq!(peer.close().iter().filter(|request|request.starts_with("POST /report ")).count(),count);
    }}
}
#[test]
fn navigation_source_loan_stale_foreign_unrelated_and_duplicate_admissions_have_no_reporting_authority() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));let mut prior_clone=request.clone_request().unwrap();
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
    assert!(matches!(service.prepare_navigation_csp_reporting(handle,&receiver,&mut prior_clone,&protected),Err(ReportError::InvalidReport)));
    let mut submitted_clone=request.clone_request().unwrap();let id=service.start_navigation_received(request,&receiver,Instant::now()).unwrap();
    assert!(matches!(service.start_navigation_received(submitted_clone.clone_request().unwrap(),&receiver,Instant::now()),Err(NavigationReceivedStartError::Fetch(_))));
    assert!(matches!(service.prepare_navigation_csp_reporting(handle,&receiver,&mut submitted_clone,&protected),Err(ReportError::InvalidReport)));
    let receipt=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    let mut foreign=NetworkService::new(DispatchLimits::default());
    assert!(matches!(foreign.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()),Err(ReportError::StaleDocument)));
    let other_request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let id=service.start_navigation_received(other_request,&receiver,Instant::now()).unwrap();let unrelated=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    assert!(matches!(service.prepare_navigation_csp_report(&mut loan,&unrelated,0,Instant::now()),Err(ReportError::StaleDocument)));
    assert!(matches!(service.prepare_navigation_csp_report(&mut loan,&receipt,usize::MAX,Instant::now()),Err(ReportError::InvalidReport)));
    assert!(service.forget_reporting_document(handle));
    let violation=receipt.violations()[0].enrich_saved_request_report(&receiver,&protected).unwrap();
    assert!(matches!(service.prepare_csp_report(handle,&violation,Instant::now()),Err(ReportError::StaleDocument)));
    let mut new_request=prior_clone;
    assert!(matches!(service.prepare_navigation_csp_reporting(handle,&receiver,&mut new_request,&protected),Err(ReportError::StaleDocument)));
    drop(submitted_clone);service.release_navigation_csp_reporting(loan).unwrap();let requests=peer.close();assert!(requests.iter().all(|request|!request.starts_with("POST ")));
}
#[test]
fn navigation_source_loan_manual_hops_keep_one_original_source_without_duplicate_prefix_delivery() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/first"));
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();let now=Instant::now();
    let id=service.start_navigation_received(request.clone_request().unwrap(),&receiver,now).unwrap();assert!(service.forget_reporting_document(handle));
    let first=outcome(&mut service,id,false).unwrap();let first_receipt=first.csp_provenance.as_ref().unwrap();
    let first_count=submit_all(&mut service,&mut loan,first_receipt,now);deliveries(&mut service,first_count,handle);
    let next=request.continue_navigation(&first).unwrap();let id=service.start_navigation_received(next,&receiver,Instant::now()).unwrap();
    let second=outcome(&mut service,id,false).unwrap();let receipt=second.csp_provenance.as_ref().unwrap();assert_eq!(receipt.url_list().len(),2);
    let mut second_count=0;let mut duplicates=0;
    for index in 0..receipt.violations().len() {match service.prepare_navigation_csp_report(&mut loan,receipt,index,now) {
        Ok(plan)=>{assert_eq!(plan.violation().status_code,200);assert_eq!(plan.violation().document_uri,receiver.document_url.serialize());second_count+=service.queue_prepared_csp_report(plan,Instant::now()).unwrap().len()},
        Err(ReportError::InvalidReport)=>duplicates+=1,Err(error)=>panic!("genuine hop rejected: {error:?}"),
    }}assert_eq!(duplicates,first_receipt.violations().len());assert!(second_count>0);deliveries(&mut service,second_count,handle);
    drop(first);drop(second);drop(request);service.release_navigation_csp_reporting(loan).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let requests=peer.close();assert_eq!(requests.iter().filter(|request|request.starts_with("POST /report ")).count(),first_count+second_count);
}
#[test]
fn navigation_source_loan_quota_and_release_preserve_shared_limits_without_new_global_registration() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut retained=Vec::new();let globals=service.reporting.documents.len();
    loop {
        let owner=match service.prepare_navigation_embedding_owner(handle,&receiver,&EmbedderPolicyContainer::default(),&parent,NavigationEmbeddingElement::Iframe) {
            Ok(owner)=>owner,Err(ReportError::Limit)=>break,Err(error)=>panic!("capture: {error:?}"),
        };
        let mut request=ApiRequest::child_navigation_with_embedding_owner(peer.url("/never-submit"),&parent,&receiver.origin,owner).unwrap();
        match service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected) {
            Ok(loan)=>retained.push((loan,request)),Err(ReportError::Limit)=>break,Err(error)=>panic!("loan: {error:?}"),
        }
    }
    assert!(!retained.is_empty());assert_eq!(service.reporting.documents.len(),globals);
    assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)<=MAX_REPORT_BYTES);
    for (loan,request) in retained {service.release_navigation_csp_reporting(loan).unwrap();drop(request);}
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let requests=peer.close();assert_eq!(requests.len(),1);assert!(requests[0].starts_with("GET /source "));
}
#[test]
fn navigation_source_loan_never_transfers_endpoints_to_a_fresh_about_global() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
    let id=service.start_navigation_received(request,&receiver,Instant::now()).unwrap();assert!(service.forget_reporting_document(handle));
    let mut fresh=receiver.clone();fresh.identity=32;let about=ParsedUrl::parse("about:blank").unwrap();
    fresh.creation_url=about.clone();fresh.document_url=about;let child=service.register_reporting_initial_client(&fresh).unwrap();
    assert!(service.reporting.document(child).unwrap().endpoints.is_empty());
    let receipt=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    let count=submit_all(&mut service,&mut loan,&receipt,Instant::now());deliveries(&mut service,count,handle);
    assert!(service.reporting.document(child).unwrap().endpoints.is_empty());assert!(service.reporting.document(handle).is_err());
    service.release_navigation_csp_reporting(loan).unwrap();let requests=peer.close();assert_eq!(requests.iter().filter(|request|request.starts_with("POST /report ")).count(),count);
}

#[test]
fn navigation_source_loan_foreign_owner_and_already_admitted_request_cannot_capture_original_settings() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());
    let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut foreign=NetworkService::new(DispatchLimits::default());
    let (foreign_handle,foreign_receiver,foreign_parent,_)=source(&mut foreign,&peer);
    let mut foreign_request=owned_request(&mut foreign,foreign_handle,&foreign_receiver,&foreign_parent,peer.url("/broken"));
    assert!(matches!(service.prepare_navigation_csp_reporting(handle,&receiver,&mut foreign_request,&protected),Err(ReportError::StaleDocument)));
    // Failed foreign validation cannot claim the native source token.
    let loan=foreign.prepare_navigation_csp_reporting(foreign_handle,&foreign_receiver,&mut foreign_request,&protected).unwrap();
    foreign.release_navigation_csp_reporting(loan).unwrap();drop(foreign_request);
    let request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let mut retained=request.clone_request().unwrap();
    let id=service.start_navigation_received(request,&receiver,Instant::now()).unwrap();
    assert!(matches!(service.prepare_navigation_csp_reporting(handle,&receiver,&mut retained,&protected),Err(ReportError::InvalidReport)));
    let failure=outcome(&mut service,id,false).unwrap_err();assert!(!failure.csp_provenance.unwrap().violations().is_empty());
    drop(retained);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert!(peer.close().iter().all(|request|!request.starts_with("POST ")));
}
#[test]
fn navigation_source_loan_rejects_changed_policy_receipts_and_foreign_report_plan_consumption() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());
    let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
    let mut changed=receiver.clone();let mut headers=HeaderList::default();
    headers.append("content-security-policy-report-only","frame-src 'none'; report-to other").unwrap();
    changed.context.csp=CspPolicyList::from_headers(&headers).unwrap();
    let id=service.start_navigation_received(request,&changed,Instant::now()).unwrap();
    let receipt=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    assert!(matches!(service.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()),Err(ReportError::InvalidReport)));
    service.release_navigation_csp_reporting(loan).unwrap();
    let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/broken"));
    let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
    let id=service.start_navigation_received(request,&receiver,Instant::now()).unwrap();
    let receipt=outcome(&mut service,id,false).unwrap_err().csp_provenance.unwrap();
    let plan=service.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()).unwrap();
    let mut foreign=NetworkService::new(DispatchLimits::default());
    assert_eq!(foreign.queue_prepared_csp_report(plan,Instant::now()),Err(ReportError::StaleDocument));
    // A consumed native receipt cannot be minted again after a foreign queue error.
    assert!(matches!(service.prepare_navigation_csp_report(&mut loan,&receipt,0,Instant::now()),Err(ReportError::InvalidReport)));
    service.release_navigation_csp_reporting(loan).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert!(peer.close().iter().all(|request|!request.starts_with("POST ")));
}

#[test]
fn navigation_source_loan_successful_buffered_and_zero_credit_streamed_heads_keep_actual_reporting_custody() {
    for stream in [false,true] {
        let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());
        let (handle,receiver,parent,protected)=source(&mut service,&peer);
        let mut request=owned_request(&mut service,handle,&receiver,&parent,peer.url("/successful"));
        let mut loan=service.prepare_navigation_csp_reporting(handle,&receiver,&mut request,&protected).unwrap();
        let now=Instant::now();let id=if stream {service.start_navigation_received_stream(request,&receiver,0,now).unwrap()}
            else {service.start_navigation_received(request,&receiver,now).unwrap()};
        assert!(service.forget_reporting_document(handle));
        let receipt=if !stream {outcome(&mut service,id,false).unwrap().csp_provenance.unwrap()}
            else {
                let deadline=Instant::now()+Duration::from_secs(8);let mut receipt=None;let mut done=false;
                while !done&&Instant::now()<deadline {for event in service.step(4096,Instant::now()) {match event {
                    ServiceEvent::ResponseHead{id:found,head} if found==id=>{assert_eq!(head.status,200);receipt=head.csp_provenance;},
                    ServiceEvent::StreamFinished{id:found} if found==id=>done=true,
                    ServiceEvent::NavigationStreamFailed{id:found,failure} if found==id=>panic!("native success failed: {failure:?}"),_=>{}
                }}thread::yield_now();}
                assert!(done);receipt.unwrap()
            };
        let count=submit_all(&mut service,&mut loan,&receipt,now);assert!(count>0);deliveries(&mut service,count,handle);
        service.release_navigation_csp_reporting(loan).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        let requests=peer.close();assert_eq!(requests.iter().filter(|request|request.starts_with("POST /report ")).count(),count);
        assert!(requests.iter().any(|request|request.starts_with("GET /successful ")));
    }
}
