//! Authentic Script graph/source settings lifecycle and server request controls.
use super::*;
use crate::{api::{NavigationRequestInit,NavigableContext},EmbedderPolicyContainer,ReferrerPolicy,
    service::{ServiceId,NavigationFailure,NavigationReceivedStartError},CspPolicyList};
use amazing_net::dispatch::DispatchLimits;
use std::{io::{Read,Write},net::TcpListener,thread,time::Duration,sync::{Mutex,mpsc}};
struct Peer {base:String,stop:mpsc::Sender<()>,thread:thread::JoinHandle<()>,requests:Arc<Mutex<Vec<String>>>}
impl Peer {
    fn new(endpoint:bool)->Self { Self::with_policy(endpoint,"Content-Security-Policy-Report-Only","script-src 'none'; report-to default") }
    fn with_policy(endpoint:bool,header:&'static str,policy:&'static str)->Self {
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
                let request=String::from_utf8(bytes).unwrap();let path=request.split_whitespace().nth(1).unwrap().to_owned();
                let method=request.split_whitespace().next().unwrap().to_owned();
                let unsafe_names=head.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("access-control-request-headers"))
                    .map(|(_,value)|value.trim().to_owned())).unwrap_or_default();
                saved.lock().unwrap().push(request);
                let response=match path.as_str() {
                    "/cross-script"|"/cross-allowed" if method=="OPTIONS"=>format!("HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: {unsafe_names}\r\nConnection: close\r\n\r\n"),
                    "/cross-allowed"=>"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    "/source"=>format!("HTTP/1.1 200 OK\r\n{header}: {policy}\r\n{}Content-Length: 0\r\nConnection: close\r\n\r\n",if endpoint {"Reporting-Endpoints: default=\"/report\"\r\n"}else{""}),
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
fn script_request(receiver:&ReceivingDocumentClient<u64>,target:HttpUrl)->ApiRequest {
    crate::api::ApiRequest::new(crate::api::ApiRequestInput::Url(&target.serialize().encode_utf16().collect::<Vec<_>>()),
        crate::api::ApiRequestInit::default(),&target,receiver.origin.clone()).unwrap()
}
fn script_outcome(service:&mut NetworkService,id:crate::service::ServiceId)->Result<crate::service::SubresourceResponse,crate::service::SubresourceFailure> {
    let deadline=Instant::now()+Duration::from_secs(8);
    while Instant::now()<deadline {for event in service.step(4096,Instant::now()) {match event {
        ServiceEvent::SubresourceFinished{id:found,response} if found==id=>return Ok(response),
        ServiceEvent::SubresourceFailed{id:found,failure} if found==id=>return Err(failure),_=>{}
    }}thread::yield_now();}panic!("native Script outcome deadline")
}
fn deliver(service:&mut NetworkService,count:usize,original:ReportingDocument) {
    let deadline=Instant::now()+Duration::from_secs(8);let mut done=0;
    while done<count&&Instant::now()<deadline {for event in service.step(4096,Instant::now()) {
        if let ServiceEvent::ReportFinished(result)=event {assert_eq!(result.document,original);
            assert!(matches!(result.outcome,ReportOutcome::Response{status:204,accepted:true}));done+=1;}
    }thread::yield_now();}assert_eq!(done,count);
}
fn plans(service:&mut NetworkService,loan:&mut ScriptCspRequestLoan,receipt:&CspResponseProvenance,now:Instant)->usize {
    let mut count=0;for index in 0..receipt.violations().len() {
        let plan=service.prepare_script_csp_report(loan,receipt,index,now).unwrap();
        assert!(plan.violation().document_uri.ends_with("/source"));
        assert_eq!(plan.violation().status_code,200);
        assert!(String::from_utf16(&plan.report_snapshot().url).unwrap().ends_with("/source"));
        let author_result:Result<(),&str>=Err("listener error");assert!(author_result.is_err());
        count+=service.queue_prepared_csp_report(plan,now).unwrap().len();
        assert!(matches!(service.prepare_script_csp_report(loan,receipt,index,now),Err(ReportError::InvalidReport)));
    }count
}
#[test]
fn script_source_receipts_report_real_success_after_original_retirement_and_author_error() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
    let mut request=script_request(&receiver,peer.url("/ok"));let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
    let now=Instant::now();let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);
    assert!(service.forget_reporting_document(handle));
    assert_eq!(service.release_retired_reporting_document(handle),Ok(ReportingRelease::Pending));
    let outcome=script_outcome(&mut service,id).unwrap();let receipt=outcome.head.csp_provenance.as_ref().unwrap();
    assert!(!receipt.violations().is_empty());let count=plans(&mut service,&mut loan,receipt,now);assert!(count>0);deliver(&mut service,count,handle);
    service.release_script_csp_request_reporting(loan).unwrap();service.release_script_csp_graph_reporting(graph,graph_id).unwrap();drop(outcome);
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert_eq!(service.release_retired_reporting_document(handle),Ok(ReportingRelease::Released));
    let requests=peer.close();assert_eq!(requests.iter().filter(|r|r.starts_with("POST /report ")).count(),count);
}
#[test]
fn script_graph_can_issue_genuine_later_dependency_after_retirement_without_new_global() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
    assert!(service.forget_reporting_document(handle));
    assert!(matches!(service.prepare_script_csp_graph_reporting(handle,&receiver,&protected),Err(ReportError::StaleDocument)));
    for path in ["/root.js","/dependency.js","/nested.js"] {
        let mut request=script_request(&receiver,peer.url(path));let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
        let now=Instant::now();let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);
        let outcome=script_outcome(&mut service,id).unwrap();let count=plans(&mut service,&mut loan,outcome.head.csp_provenance.as_ref().unwrap(),now);
        deliver(&mut service,count,handle);service.release_script_csp_request_reporting(loan).unwrap();
    }
    service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let requests=peer.close();for path in ["/root.js","/dependency.js","/nested.js"] {assert!(requests.iter().any(|r|r.starts_with(&format!("GET {path} "))));}
}
#[test]
fn script_source_loan_keeps_native_prior_hop_receipts_on_non_csp_terminal_error() {
    for target in ["/broken","/first"] {
        let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
        let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
        let mut request=script_request(&receiver,peer.url(target));let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
        let now=Instant::now();let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);assert!(service.forget_reporting_document(handle));
        let outcome=script_outcome(&mut service,id);
        let receipt=match &outcome {Ok(response)=>response.head.csp_provenance.as_ref().unwrap(),Err(failure)=>{assert!(!matches!(failure.error,crate::api::ApiError::Network(crate::FetchError::Csp(_))));failure.csp_provenance.as_ref().unwrap()}};
        if target=="/broken" {assert!(outcome.is_err());}else{assert_eq!(receipt.url_list().len(),2);}
        let count=plans(&mut service,&mut loan,receipt,now);assert!(count>0);deliver(&mut service,count,handle);
        service.release_script_csp_request_reporting(loan).unwrap();service.release_script_csp_graph_reporting(graph,graph_id).unwrap();drop(outcome);
        assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);peer.close();
    }
}

#[test]
fn script_source_wrong_graph_service_receiver_and_navigation_cannot_mint_loans() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,parent,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let id=graph.id();
    let other=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();
    let mut request=script_request(&receiver,peer.url("/ok"));
    assert!(matches!(service.issue_script_csp_request_reporting(&mut graph,other.id(),&mut request),Err(ReportError::StaleDocument)));
    let mut foreign=NetworkService::new(DispatchLimits::default());
    assert!(matches!(foreign.issue_script_csp_request_reporting(&mut graph,id,&mut request),Err(ReportError::StaleDocument)));
    assert!(matches!(foreign.prepare_script_csp_graph_reporting(handle,&receiver,&protected),Err(ReportError::StaleDocument)));
    let mut wrong=receiver.clone();wrong.origin=Origin::opaque();
    assert!(matches!(service.prepare_script_csp_graph_reporting(handle,&wrong,&protected),Err(ReportError::InvalidReport)));
    let mut nav=ApiRequest::child_navigation(peer.url("/nav"),&parent).unwrap();
    assert!(matches!(service.issue_script_csp_request_reporting(&mut graph,id,&mut nav),Err(ReportError::InvalidReport)));
    let loan=service.issue_script_csp_request_reporting(&mut graph,id,&mut request).unwrap();
    assert!(matches!(service.issue_script_csp_request_reporting(&mut graph,id,&mut request),Err(ReportError::InvalidReport)));
    drop((loan,request,other));service.release_script_csp_graph_reporting(graph,id).unwrap();
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);assert_eq!(peer.close().len(),1);
}
#[test]
fn script_source_admission_rejects_changed_client_and_preclaim_clones_without_poisoning_original() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let id=graph.id();
    let mut request=script_request(&receiver,peer.url("/ok"));let old_clone=request.clone_request().unwrap();
    let mut loan=service.issue_script_csp_request_reporting(&mut graph,id,&mut request).unwrap();
    let old_id=service.start_subresource(old_clone,receiver.context.clone(),Destination::Script);
    assert!(matches!(script_outcome(&mut service,old_id).unwrap_err().error,crate::api::ApiError::InvalidWindow));
    let mut wrong=receiver.context.clone();wrong.csp=CspPolicyList::default();
    let wrong_id=service.start_subresource(request.clone_request().unwrap(),wrong,Destination::Script);
    assert!(matches!(script_outcome(&mut service,wrong_id).unwrap_err().error,crate::api::ApiError::InvalidWindow));
    let mut foreign=NetworkService::new(DispatchLimits::default());let wrong_id=foreign.start_subresource(request.clone_request().unwrap(),receiver.context.clone(),Destination::Script);
    assert!(matches!(script_outcome(&mut foreign,wrong_id).unwrap_err().error,crate::api::ApiError::InvalidWindow));
    let replay=request.clone_request().unwrap();let legacy_projection=replay.clone_request().unwrap();
    assert!(legacy_projection.policy_request().unwrap().native_script_report_token.is_none());
    let good_id=service.start_subresource(request,receiver.context.clone(),Destination::Script);
    let result=script_outcome(&mut service,good_id).unwrap();let receipt=result.head.csp_provenance.as_ref().unwrap();
    assert!(legacy_projection.policy_request().unwrap().native_script_report_token.is_none());
    drop(legacy_projection);
    let replay_id=service.start_subresource(replay,receiver.context.clone(),Destination::Script);
    assert!(matches!(script_outcome(&mut service,replay_id).unwrap_err().error,crate::api::ApiError::InvalidWindow));
    let plan=service.prepare_script_csp_report(&mut loan,receipt,0,Instant::now()).unwrap();drop(plan);
    drop((result,loan));service.release_script_csp_graph_reporting(graph,id).unwrap();
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert_eq!(peer.close().iter().filter(|r|r.starts_with("GET /ok ")).count(),1);
}
#[test]
fn script_source_genuine_receipts_reject_foreign_request_service_and_duplicate_consumption() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let id=graph.id();let mut records=Vec::new();
    for _ in 0..2 {let mut request=script_request(&receiver,peer.url("/same"));let loan=service.issue_script_csp_request_reporting(&mut graph,id,&mut request).unwrap();
        let request_id=service.start_subresource(request,receiver.context.clone(),Destination::Script);records.push((loan,script_outcome(&mut service,request_id).unwrap()));}
    let ((mut first,response),(_,other))=(records.remove(0),records.remove(0));
    let receipt=response.head.csp_provenance.as_ref().unwrap();
    assert!(matches!(service.prepare_script_csp_report(&mut first,other.head.csp_provenance.as_ref().unwrap(),0,Instant::now()),Err(ReportError::StaleDocument)));
    let mut foreign=NetworkService::new(DispatchLimits::default());
    assert!(matches!(foreign.prepare_script_csp_report(&mut first,receipt,0,Instant::now()),Err(ReportError::StaleDocument)));
    assert!(matches!(service.prepare_script_csp_report(&mut first,receipt,usize::MAX,Instant::now()),Err(ReportError::InvalidReport)));
    let plan=service.prepare_script_csp_report(&mut first,receipt,0,Instant::now()).unwrap();drop(plan);
    assert!(matches!(service.prepare_script_csp_report(&mut first,receipt,0,Instant::now()),Err(ReportError::InvalidReport)));
    drop((first,response,other));service.release_script_csp_graph_reporting(graph,id).unwrap();
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);peer.close();
}
#[test]
fn script_source_no_endpoint_snapshot_survives_original_retirement_and_fresh_about_is_empty() {
    let peer=Peer::new(false);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
    let mut request=script_request(&receiver,peer.url("/ok"));let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
    let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);assert!(service.forget_reporting_document(handle));
    let result=script_outcome(&mut service,id).unwrap();let plan=service.prepare_script_csp_report(&mut loan,result.head.csp_provenance.as_ref().unwrap(),0,Instant::now()).unwrap();
    assert!(String::from_utf16(&plan.report_snapshot().url).unwrap().ends_with("/source"));
    assert!(matches!(service.queue_prepared_csp_report(plan,Instant::now()),Err(ReportError::NoEndpoint)));
    let mut child=receiver.clone();child.identity=32;child.creation_url=ParsedUrl::parse("about:blank").unwrap();child.document_url=child.creation_url.clone();
    let child_handle=service.register_reporting_initial_client(&child).unwrap();assert!(service.reporting.document(child_handle).unwrap().endpoints.is_empty());
    drop((loan,result));service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    assert!(!service.has_pending_reporting_io());assert!(peer.close().iter().all(|r|!r.starts_with("POST ")));
}
#[test]
fn script_graph_source_quota_is_shared_bounded_and_explicit_cleanup_releases_it() {
    let peer=Peer::new(true);let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let globals=service.reporting.documents.len();let mut graphs=Vec::new();
    loop {match service.prepare_script_csp_graph_reporting(handle,&receiver,&protected) {Ok(graph)=>graphs.push(graph),Err(ReportError::Limit)=>break,Err(error)=>panic!("graph: {error:?}")}}
    assert!(!graphs.is_empty());assert_eq!(service.reporting.documents.len(),globals);
    assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)<=MAX_REPORT_BYTES);
    for graph in graphs {let id=graph.id();service.release_script_csp_graph_reporting(graph,id).unwrap();}
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();let mut requests=Vec::new();
    loop {let mut request=script_request(&receiver,peer.url("/dependency"));match service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request) {
        Ok(loan)=>requests.push((loan,request)),Err(ReportError::Limit)=>break,Err(error)=>panic!("node: {error:?}")}}
    assert!(!requests.is_empty());assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)<=MAX_REPORT_BYTES);
    service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)>0);
    drop(requests);assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);assert_eq!(peer.close().len(),1);
}
#[test]
fn script_graph_abort_deadline_early_transport_and_enforced_policy_keep_true_native_receipts() {
    for cause in ["abort","deadline","bad-port","enforced"] {
        let peer=if cause=="enforced" {Peer::with_policy(true,"Content-Security-Policy","script-src 'none'; report-to default")}else{Peer::new(true)};let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
        let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
        let target=if cause=="bad-port" {HttpUrl::parse("http://127.0.0.1:25/rejected").unwrap()}else{peer.url("/hold")};
        let mut request=script_request(&receiver,target);let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
        let now=Instant::now();let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);assert!(service.forget_reporting_document(handle));
        let failure=if cause=="deadline" {service.step(4096,now+Duration::from_secs(3600)).into_iter().find_map(|event|match event {ServiceEvent::SubresourceFailed{id:found,failure} if found==id=>Some(failure),_=>None}).unwrap()}
            else {if cause=="abort" {assert!(service.abort(id));}script_outcome(&mut service,id).unwrap_err()};
        let receipt=failure.csp_provenance.as_ref().unwrap();assert!(!receipt.violations().is_empty());
        let count=plans(&mut service,&mut loan,receipt,now);assert!(count>0);deliver(&mut service,count,handle);
        drop((loan,failure));service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
        let requests=peer.close();if cause=="enforced"||cause=="bad-port" {assert!(requests.iter().all(|r|!r.starts_with("GET /hold ")));}
    }
}

#[test]
fn script_source_preserves_nonceability_and_exact_original_policy_from_received_header() {
    let peer=Peer::with_policy(true,"Content-Security-Policy","script-src 'nonce-YWJj'; report-to default");
    let mut service=NetworkService::new(DispatchLimits::default());let (handle,receiver,_,protected)=source(&mut service,&peer);
    let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
    for (name,units,duplicate,allowed) in [("valid",vec![89,87,74,106],false,true),("duplicate",vec![89,87,74,106],true,false),("surrogate",vec![0xd800],false,false),("empty",vec![],false,false)] {
        let mut request=script_request(&receiver,peer.url(&format!("/{name}")));
        request.apply_csp_request_element(crate::CspExternalElementKind::Script,&crate::CspRequestElement {nonce:Some(&units),nonce_attribute_present:true,attributes:&[],duplicate_attribute:duplicate,parser_inserted:true});
        let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);
        let result=script_outcome(&mut service,id);
        if allowed {assert!(result.is_ok());assert!(result.as_ref().unwrap().head.csp_provenance.as_ref().unwrap().violations().is_empty());}
        else {let failure=result.as_ref().unwrap_err();assert!(matches!(failure.error,crate::api::ApiError::Network(FetchError::Csp(_))));
            let receipt=failure.csp_provenance.as_ref().unwrap();let plan=service.prepare_script_csp_report(&mut loan,receipt,0,Instant::now()).unwrap();
            assert_eq!(plan.violation().original_policy_units,"script-src 'nonce-YWJj'; report-to default".encode_utf16().collect::<Vec<_>>());
            let count=service.queue_prepared_csp_report(plan,Instant::now()).unwrap().len();deliver(&mut service,count,handle);}
    }
    service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let requests=peer.close();assert!(requests.iter().any(|r|r.starts_with("GET /valid ")));
    for name in ["duplicate","surrogate","empty"] {assert!(requests.iter().all(|r|!r.starts_with(&format!("GET /{name} "))));}
}
#[test]
fn script_source_cors_terminal_failure_retains_original_receipt_and_reporting_endpoint() {
    let source_peer=Peer::new(true);let target_peer=Peer::new(false);let mut service=NetworkService::new(DispatchLimits::default());
    let (handle,receiver,_,protected)=source(&mut service,&source_peer);let mut graph=service.prepare_script_csp_graph_reporting(handle,&receiver,&protected).unwrap();let graph_id=graph.id();
    let mut reports=0;
    for path in ["/cross-script","/cross-preflight","/cross-allowed"] {
        let mut request=script_request(&receiver,target_peer.url(path));
        // An actual author header requires a genuine CORS preflight. The peer
        // grants it only for the main-response denial and success siblings.
        request.headers.append_string("x-script-csp-control","native").unwrap();
        let mut loan=service.issue_script_csp_request_reporting(&mut graph,graph_id,&mut request).unwrap();
        let id=service.start_subresource(request,receiver.context.clone(),Destination::Script);
        if path=="/cross-script" {assert!(service.forget_reporting_document(handle));}
        let outcome=script_outcome(&mut service,id);
        let receipt=if path=="/cross-allowed" {let response=outcome.as_ref().unwrap();assert_eq!(response.response_type,crate::ResponseType::Cors);assert_eq!(response.head.status,200);response.head.csp_provenance.as_ref().unwrap()}
            else {let failure=outcome.as_ref().unwrap_err();assert!(matches!(&failure.error,crate::api::ApiError::Network(FetchError::Cors)),"{failure:?}");failure.csp_provenance.as_ref().unwrap()};
        assert_eq!(receipt.url_list().last().unwrap(),&target_peer.url(path));
        let count=plans(&mut service,&mut loan,receipt,Instant::now());assert!(count>0);deliver(&mut service,count,handle);reports+=count;
        drop((loan,outcome));
    }
    service.release_script_csp_graph_reporting(graph,graph_id).unwrap();assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
    let requests=target_peer.close();
    assert!(requests.iter().any(|r|r.starts_with("GET /cross-script ")));
    assert!(requests.iter().any(|r|r.starts_with("OPTIONS /cross-script ")));
    assert!(requests.iter().any(|r|r.starts_with("OPTIONS /cross-preflight ")));
    assert!(!requests.iter().any(|r|r.starts_with("GET /cross-preflight ")));
    assert!(requests.iter().any(|r|r.starts_with("GET /cross-allowed ")));
    assert!(requests.iter().filter(|r|r.starts_with("GET ")).all(|r|r.lines().any(|line|line.eq_ignore_ascii_case("x-script-csp-control: native"))));
    assert_eq!(source_peer.close().iter().filter(|r|r.starts_with("POST /report ")).count(),reports);
}
