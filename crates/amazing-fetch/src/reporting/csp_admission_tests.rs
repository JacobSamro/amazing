//! Real native report peers; no installed Window/SPVE/Worker claim.
use super::*;
use crate::{CspPolicyList,CspInlineContext,CspInlineKind};

fn original(url:&HttpUrl, tokens:&[&str])->(crate::ReceivingDocumentClient<(u64,u64)>,HeaderList,CspInlineViolation) {
    original_policy(url,tokens,None)
}
fn original_policy(url:&HttpUrl,tokens:&[&str],group:Option<&str>)->(crate::ReceivingDocumentClient<(u64,u64)>,HeaderList,CspInlineViolation) {
    let policy=format!("script-src 'none'; report-uri {}{}",tokens.join(" "),group.map(|name|format!("; report-to {name}")).unwrap_or_default());
    let mut fields=headers("csp=\"/modern\"");fields.append("content-security-policy",&policy).unwrap();
    let mut client=receiving_client(url,&url.serialize());client.context.csp=CspPolicyList::from_headers(&fields).unwrap();
    let source="blocked()".encode_utf16().collect::<Vec<_>>();let text=url.serialize();
    let check=client.context.csp.check_inline(&CspInlineContext{kind:CspInlineKind::Script,source:&source,
        nonce:None,nonce_attribute_present:false,attributes:&[],duplicate_attribute:false,parser_inserted:true,
        element:Some(77),document_url:&text,creation_url:&text,referrer:"",status_code:200,source_file:&text,line_number:3,column_number:5});
    assert!(check.blocked);assert_eq!(check.violations.len(),1);
    (client,fields,check.violations.into_iter().next().unwrap())
}
fn success_peer(listener:TcpListener,count:usize)->thread::JoinHandle<Vec<String>> {
    thread::spawn(move|| {
        listener.set_nonblocking(true).unwrap();let deadline=Instant::now()+Duration::from_secs(12);let mut requests=Vec::new();
        while requests.len()<count {
            let (mut stream,_)=match listener.accept(){Ok(value)=>value,Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{
                assert!(Instant::now()<deadline,"missing genuine sibling POST");thread::sleep(Duration::from_millis(1));continue;},Err(error)=>panic!("{error}")};
            requests.push(read_request(&mut stream));stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").unwrap();
        }
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);requests
    })
}
fn setup(listener:&TcpListener,tokens:&[&str],limits:DispatchLimits)->(NetworkService,ReportingDocument,CspInlineViolation,HttpUrl) {
    let url=HttpUrl::parse(&format!("http://{}/original",listener.local_addr().unwrap())).unwrap();
    let(client,fields,v)=original(&url,tokens);let mut service=NetworkService::new(limits);
    let doc=service.register_reporting_client_response(&client,&url,&fields).unwrap();(service,doc,v,url)
}

#[test]
fn every_token_has_ordered_outcome_and_capacity_failure_keeps_real_admitted_id() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let mut limits=DispatchLimits::default();limits.max_in_flight=1;
    let(mut service,doc,v,url)=setup(&listener,&["http://[bad","/one","ftp://example.test/ignored","/two"],limits);
    let peer=success_peer(listener,1);let now=Instant::now();
    let admission=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
    assert_eq!(admission.document(),doc);assert_eq!(admission.route(),ReportRoute::LegacyCsp);
    assert_eq!(admission.attempts().len(),4);
    for(index,attempt)in admission.attempts().iter().enumerate(){assert_eq!(attempt.token_index(),index);}
    assert_eq!(admission.attempts()[0].outcome(),&Err(ReportError::InvalidEndpoint));
    assert!(admission.attempts()[0].endpoint().is_none());
    assert_eq!(admission.attempts()[2].outcome(),&Err(ReportError::InvalidEndpoint));
    assert_eq!(admission.attempts()[3].outcome(),&Err(ReportError::Transport));
    assert_eq!(admission.attempts()[3].endpoint().unwrap(),&url.resolve("/two").unwrap());
    let ids=admission.admitted_ids().collect::<Vec<_>>();assert_eq!(ids.len(),1);
    assert_eq!(service.reporting.deliveries.len(),1);let finished=run(&mut service,1);
    assert_eq!(finished[0].id,ids[0]);assert_eq!(finished[0].document,doc);
    assert_eq!(finished[0].outcome,ReportOutcome::Response{status:204,accepted:true});
    assert!(peer.join().unwrap()[0].starts_with("POST /one "));
    assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)>0);drop(admission);
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);
}
#[test]
fn malformed_unsupported_credentials_and_duplicates_preserve_valid_relative_tokens() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let(mut service,doc,mut v,url)=setup(&listener,&["http://[bad","next","ftp://example.test/r","next","http://user:pass@example.test/r","/last"],DispatchLimits::default());
    // Actual saved violation URL, distinct from original creation URL and HTML base.
    v.document_uri=url.resolve("/history/current").unwrap().serialize();
    let peer=success_peer(listener,3);let now=Instant::now();let record=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
    assert_eq!(record.attempts().len(),6);assert_eq!(record.admitted_ids().count(),3);
    for index in [0,2,4]{assert_eq!(record.attempts()[index].outcome(),&Err(ReportError::InvalidEndpoint));}
    assert_eq!(record.attempts()[1].endpoint().unwrap(),&url.resolve("/history/next").unwrap());
    assert_eq!(record.attempts()[3].endpoint(),record.attempts()[1].endpoint());
    let ids=record.admitted_ids().collect::<Vec<_>>();assert_ne!(ids[0],ids[1]);
    let finished=run(&mut service,3);assert!(finished.iter().all(|done|done.document==doc&&ids.contains(&done.id)));
    let requests=peer.join().unwrap();assert_eq!(requests.iter().filter(|r|r.starts_with("POST /history/next ")).count(),2);
    assert_eq!(requests.iter().filter(|r|r.starts_with("POST /last ")).count(),1);
    v.report_uris=vec!["http://[bad".into(),"data:text/plain,no".into()];
    assert_eq!(service.queue_csp_report(doc,&v,now,now),Err(ReportError::NoEndpoint));
}
#[test]
fn original_mixed_content_policy_rejects_siblings_without_cancelling_safe_loopback_request() {
    for denied_first in [false,true] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}",listener.local_addr().unwrap());
        let protected=HttpUrl::parse(&base.replace("http:","https:")).unwrap();
        let allowed=format!("{base}/safe");let denied="http://untrusted.example/report";
        let tokens=if denied_first{vec![denied,allowed.as_str(),denied]}else{vec![allowed.as_str(),denied]};
        let(mut client,fields,v)=original(&protected,&tokens);client.context.security.client_restricts_mixed_content=true;
        let mut service=NetworkService::new(DispatchLimits::default());let doc=service.register_reporting_client_response(&client,&protected,&fields).unwrap();
        let peer=success_peer(listener,1);let now=Instant::now();let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
        assert_eq!(batch.admitted_ids().count(),1);
        for attempt in batch.attempts(){if attempt.endpoint().unwrap().serialize()==denied{assert_eq!(attempt.outcome(),&Err(ReportError::Policy(FetchError::MixedContent)));}}
        let delivery=service.reporting.deliveries.values().next().unwrap();
        assert_eq!(delivery.request.origin,client.origin);assert!(delivery.request.security.client_restricts_mixed_content);
        assert_eq!(delivery.request.security.destination,crate::Destination::Report);
        assert_eq!(delivery.request.credentials,CredentialsMode::SameOrigin);assert_eq!(delivery.request.referrer_policy,client.referrer_policy);
        let done=run(&mut service,1);assert_eq!(done[0].id,batch.admitted_ids().next().unwrap());
        let request=&peer.join().unwrap()[0];assert!(request.starts_with("POST /safe "));
        assert!(!request.to_ascii_lowercase().contains("\r\ncookie:"));assert!(!request.to_ascii_lowercase().contains("\r\nreferer:"));
        assert_eq!(done[0].wire().owner_origin(),&client.origin);
    }
}
#[test]
fn real_pending_report_bytes_bound_each_sibling_without_atomic_credit_rejection() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let tokens=(0..16).map(|_|"/legacy").collect::<Vec<_>>();
    let(mut service,doc,v,_url)=setup(&listener,&tokens,DispatchLimits::default());let now=Instant::now();
    // Genuine standard test-report producer fills the application queue; no
    // invented pending markers or direct retained-byte counter injection.
    let first=service.generate_test_report(doc,&vec![0x01;10400],&"csp".encode_utf16().collect::<Vec<_>>(),now).unwrap().delivery.unwrap();
    let used=service.reporting.retained_bytes;assert!(used>62000&&used<MAX_REPORT_BYTES);
    let body=serialize(&v,false,0);let bytes=body.len()+v.report_url.len()+service.reporting.document(doc).unwrap().origin.serialize().len();
    let expected=(MAX_REPORT_BYTES-used)/bytes;assert!(expected>0&&expected<tokens.len());
    let peer=success_peer(listener,1+expected);
    let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
    assert_eq!(batch.admitted_ids().count(),expected);assert_eq!(service.reporting.retained_bytes,used+expected*bytes);
    assert!(service.reporting.retained_bytes<=MAX_REPORT_BYTES);
    for attempt in &batch.attempts()[expected..]{assert_eq!(attempt.outcome(),&Err(ReportError::Limit));}
    let ids=batch.admitted_ids().collect::<Vec<_>>();let done=run(&mut service,1+expected);
    assert!(done.iter().any(|report|report.id==first));assert!(ids.iter().all(|id|done.iter().any(|r|r.id==*id)));
    assert_eq!(service.reporting.retained_bytes,0);assert_eq!(peer.join().unwrap().len(),1+expected);
}
#[test]
fn retired_original_prepared_partial_admission_and_result_hold_actual_lifetime_and_credit() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let mut limits=DispatchLimits::default();limits.max_in_flight=1;
    let(mut service,doc,v,_)=setup(&listener,&["/one","/two"],limits);let now=Instant::now();
    let plan=service.prepare_csp_report(doc,&v,now).unwrap();assert!(service.forget_reporting_document(doc));
    let peer=success_peer(listener,1);let batch=service.queue_prepared_csp_report_admissions(plan,now).unwrap();
    assert_eq!(batch.admitted_ids().count(),1);assert_eq!(batch.attempts()[1].outcome(),&Err(ReportError::Transport));
    assert_eq!(service.release_retired_reporting_document(doc),Ok(ReportingRelease::Pending));
    assert!(service.reporting.prepared_bytes.load(Ordering::Acquire)>0);
    drop(run(&mut service,1));assert_eq!(service.release_retired_reporting_document(doc),Ok(ReportingRelease::Pending));
    drop(batch);assert_eq!(service.release_retired_reporting_document(doc),Ok(ReportingRelease::Released));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);peer.join().unwrap();
    let other_url=HttpUrl::parse("http://localhost/foreign").unwrap();let(client,fields,v)=original(&other_url,&["/r"]);
    let handle=service.register_reporting_client_response(&client,&other_url,&fields).unwrap();let plan=service.prepare_csp_report(handle,&v,now).unwrap();
    let mut foreign=NetworkService::new(DispatchLimits::default());assert!(matches!(foreign.queue_prepared_csp_report_admissions(plan,now),Err(ReportError::StaleDocument)));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire),0);assert_eq!(foreign.dispatcher.in_flight(),0);
}
#[test]
fn all_five_legacy_redirect_errors_leave_the_actual_successful_sibling_unchanged() {
    for status in [301u16,302,303,307,308] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let(mut service,doc,v,_)=setup(&listener,&["/reject","/ok"],DispatchLimits::default());
        let peer=thread::spawn(move||{let mut requests=Vec::new();for _ in 0..2{let(mut socket,_)=listener.accept().unwrap();let r=read_request(&mut socket);
            let response=if r.starts_with("POST /reject "){format!("HTTP/1.1 {status} Redirect\r\nLocation: /never\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")}else{assert!(r.starts_with("POST /ok "));"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into()};
            socket.write_all(response.as_bytes()).unwrap();requests.push(r);}listener.set_nonblocking(true).unwrap();assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);requests});
        let now=Instant::now();let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();assert_eq!(batch.admitted_ids().count(),2);
        let done=run(&mut service,2);for(index,path)in["/reject","/ok"].iter().enumerate(){let id=*batch.attempts()[index].outcome().as_ref().unwrap();let report=done.iter().find(|r|r.id==id).unwrap();assert_eq!(report.document,doc);
            assert!(report.wire().registered_endpoint().serialize().ends_with(path));assert_eq!(report.wire().url_list().len(),1);
            assert_eq!(report.outcome,if index==0{ReportOutcome::Failed(ReportError::Policy(FetchError::Redirect))}else{ReportOutcome::Response{status:204,accepted:true}});}
        assert_eq!(service.reporting.retained_bytes,0);assert_eq!(peer.join().unwrap().len(),2);
    }
}
#[test]
fn modern_report_to_presence_keeps_legacy_tokens_out_of_admission_and_transport() {
    for group in ["missing","","csp"] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let url=HttpUrl::parse(&format!("http://{}/original",listener.local_addr().unwrap())).unwrap();
        let(client,fields,v)=original_policy(&url,&["/legacy","http://[bad"],Some(group));
        assert_eq!(v.report_to.as_deref(),Some(group));
        let mut service=NetworkService::new(DispatchLimits::default());let doc=service.register_reporting_client_response(&client,&url,&fields).unwrap();
        let now=Instant::now();let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
        assert_eq!(batch.route(),ReportRoute::ReportingApi);assert_eq!(batch.attempts().len(),1);
        if group!="csp" {
            assert_eq!(batch.attempts()[0].outcome(),&Err(ReportError::NoEndpoint));assert_eq!(service.dispatcher.in_flight(),0);
            listener.set_nonblocking(true).unwrap();assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        } else {
            let peer=success_peer(listener,1);assert_eq!(batch.admitted_ids().count(),1);
            let done=run(&mut service,1);assert_eq!(done[0].route,ReportRoute::ReportingApi);
            let request=&peer.join().unwrap()[0];assert!(request.starts_with("POST /modern "));assert!(request.to_ascii_lowercase().contains("application/reports+json"));
        }
    }
}
#[test]
fn actual_terminal_transport_failure_does_not_erase_admitted_live_sibling() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let closed=TcpListener::bind("127.0.0.1:0").unwrap();let endpoint=format!("http://{}/closed",closed.local_addr().unwrap());drop(closed);
    let(mut service,doc,v,_)=setup(&listener,&[endpoint.as_str(),"/live"],DispatchLimits::default());let peer=success_peer(listener,1);let now=Instant::now();
    let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();assert_eq!(batch.admitted_ids().count(),2);
    let done=run(&mut service,2);let failed=*batch.attempts()[0].outcome().as_ref().unwrap();let live=*batch.attempts()[1].outcome().as_ref().unwrap();
    assert_eq!(done.iter().find(|r|r.id==failed).unwrap().outcome,ReportOutcome::Failed(ReportError::Transport));
    assert_eq!(done.iter().find(|r|r.id==live).unwrap().outcome,ReportOutcome::Response{status:204,accepted:true});
    assert!(done.iter().all(|r|r.document==doc));assert_eq!(service.reporting.retained_bytes,0);peer.join().unwrap();
}

#[test]
fn header_comma_policy_boundary_is_not_invented_as_a_report_uri_token() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let(mut service,doc,v,_)=setup(&listener,
        &["http://[bad","/one","data:text/plain,no","/two"],DispatchLimits::default());
    // Serialized HTTP CSP is a comma-separated policy list. The second policy
    // starts with the unknown directive "no"; /two is not in report-uri.
    assert_eq!(service.reporting.document(doc).unwrap().context.csp.policies.len(),2);
    assert_eq!(v.report_uris,vec!["http://[bad","/one","data:text/plain"]);
    let peer=success_peer(listener,1);let now=Instant::now();let batch=service.queue_csp_report_admissions(doc,&v,now,now).unwrap();
    assert_eq!(batch.attempts().len(),3);assert_eq!(batch.admitted_ids().count(),1);
    assert_eq!(batch.attempts()[0].outcome(),&Err(ReportError::InvalidEndpoint));
    assert_eq!(batch.attempts()[2].outcome(),&Err(ReportError::InvalidEndpoint));
    let done=run(&mut service,1);assert_eq!(done[0].id,batch.admitted_ids().next().unwrap());
    assert!(peer.join().unwrap()[0].starts_with("POST /one "));
}
