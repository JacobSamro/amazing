//! Native original-settings custody. No Window/SPVE/worker completion claim.
use super::*;
use crate::CspPolicyList;

#[test]
fn actual_original_task_consumer_retains_settings_and_credit_until_same_service_closure() {
    let url = HttpUrl::parse("http://localhost/original").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let receiver = receiving_client(&url, &url.serialize());
    let document = service.register_reporting_client_response(&receiver, &url, &headers("")).unwrap();
    assert_eq!(service.release_retired_reporting_document(document), Ok(ReportingRelease::Active));
    let mut consumer = Some(service.prepare_reporting_task_consumer(document).unwrap());
    let used = service.reporting.prepared_bytes.load(Ordering::Acquire);
    assert!(used > 0);
    assert!(service.forget_reporting_document(document));
    assert_eq!(service.release_retired_reporting_document(document), Ok(ReportingRelease::Pending));
    let mut foreign = NetworkService::new(DispatchLimits::default());
    assert_eq!(foreign.close_reporting_task_consumer(&mut consumer), Err(ReportError::StaleDocument));
    assert!(consumer.is_some());
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), used);
    assert_eq!(service.close_reporting_task_consumer(&mut consumer), Ok(true));
    assert_eq!(service.close_reporting_task_consumer(&mut consumer), Ok(false));
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
    assert_eq!(service.release_retired_reporting_document(document), Ok(ReportingRelease::Released));
    assert_eq!(foreign.release_retired_reporting_document(document), Err(ReportError::StaleDocument));
    assert!(matches!(service.generate_test_report(document, &[], &[], Instant::now()), Err(ReportError::StaleDocument)));
}

#[test]
fn real_post_and_native_terminal_event_keep_original_custody_until_consumer_drop() {
    for status in [204u16, 410] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!("http://{}/original", listener.local_addr().unwrap())).unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            stream.write_all(format!("HTTP/1.1 {status} Outcome\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            request
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let receiver = receiving_client(&url, &url.serialize());
        let original = service.register_reporting_client_response(&receiver, &url, &headers("csp=\"/collector\"")).unwrap();
        let now = Instant::now();
        let id = service.generate_test_report(original, &[0xd800, 0x61], &"csp".encode_utf16().collect::<Vec<_>>(), now).unwrap().delivery.unwrap();
        assert!(service.forget_reporting_document(original));
        assert_eq!(service.release_retired_reporting_document(original), Ok(ReportingRelease::Pending));
        let events = run(&mut service, 1);
        assert_eq!(events[0].id, id);
        assert_eq!(events[0].document, original);
        assert_eq!(events[0].outcome, ReportOutcome::Response { status, accepted: status == 204 });
        assert_eq!(events[0].wire().report_url(), url.serialize());
        assert!(!service.has_pending_reporting_io());
        // A terminal socket alone does not release unconsumed native facts.
        assert_eq!(service.release_retired_reporting_document(original), Ok(ReportingRelease::Pending));
        let retained = events[0].clone(); drop(events);
        assert_eq!(service.release_retired_reporting_document(original), Ok(ReportingRelease::Pending));
        drop(retained);
        assert_eq!(service.release_retired_reporting_document(original), Ok(ReportingRelease::Released));
        assert!(peer.join().unwrap().starts_with("POST /collector "));
    }
}

#[test]
fn completed_original_bookkeeping_recycles_without_raising_native_document_cap() {
    let url = HttpUrl::parse("http://localhost/original").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let receiver = receiving_client(&url, &url.serialize());
    let mut last = None;
    for _ in 0..(MAX_DOCUMENTS + 16) {
        let doc = service.register_reporting_client_response(&receiver, &url, &headers("")).unwrap();
        assert!(service.forget_reporting_document(doc));
        assert_eq!(service.release_retired_reporting_document(doc), Ok(ReportingRelease::Released));
        if let Some(previous) = last { assert_ne!(doc, previous); }
        last = Some(doc);
    }
    assert!(service.reporting.documents.is_empty());
    assert!(service.reporting.lifetimes.is_empty());
}

#[test]
fn task_consumer_snapshot_obeys_existing_shared_prepared_credit_limit() {
    let url = HttpUrl::parse("http://localhost/original").unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let receiver = receiving_client(&url, &url.serialize());
    let doc = service.register_reporting_client_response(&receiver, &url, &headers("")).unwrap();
    // Existing original prepared quota is full: capture cannot bypass it.
    service.reporting.prepared_bytes.store(MAX_REPORT_BYTES, Ordering::Release);
    assert!(matches!(service.prepare_reporting_task_consumer(doc), Err(ReportError::Limit)));
    assert_eq!(service.release_retired_reporting_document(doc), Ok(ReportingRelease::Active));
    service.reporting.prepared_bytes.store(0, Ordering::Release);
    let mut consumer = Some(service.prepare_reporting_task_consumer(doc).unwrap());
    assert!(service.close_reporting_task_consumer(&mut consumer).unwrap());
    assert_eq!(service.reporting.prepared_bytes.load(Ordering::Acquire), 0);
}

#[test]
fn genuine_inline_legacy_report_uses_report_destination_original_client_and_rejects_all_redirects() {
    use crate::csp::{CspInlineContext,CspInlineKind};
    for status in [301u16,302,303,307,308] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = HttpUrl::parse(&format!("http://{}/original",listener.local_addr().unwrap())).unwrap();
        let peer = thread::spawn(move || {
            let deadline = Instant::now()+Duration::from_secs(8);
            let (mut socket,_) = loop {
                match listener.accept() {
                    Ok(value)=>break value,
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{
                        assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(1));
                    },Err(error)=>panic!("accept {error}"),
                }
            };
            let request=read_request(&mut socket);
            socket.write_all(format!("HTTP/1.1 {status} Redirect\r\nLocation: /never\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            (request,listener)
        });
        let mut service=NetworkService::new(DispatchLimits::default());
        let mut receiver=receiving_client(&url,&url.serialize());
        receiver.context.security.embedder_policy=crate::EmbedderPolicy::RequireCorp;
        receiver.referrer_policy=crate::ReferrerPolicy::NoReferrer;
        let mut fields=HeaderList::default();fields.append("content-security-policy","script-src 'none'; report-uri /collector").unwrap();
        receiver.context.csp=CspPolicyList::from_headers(&fields).unwrap();
        let document=service.register_reporting_client_response(&receiver,&url,&fields).unwrap();
        let source="alert(1)".encode_utf16().collect::<Vec<_>>();let original=url.serialize();
        let check=receiver.context.csp.check_inline(&CspInlineContext {
            kind:CspInlineKind::Script,source:&source,nonce:None,nonce_attribute_present:false,
            attributes:&[],duplicate_attribute:false,parser_inserted:true,element:None,
            document_url:&original,creation_url:&original,referrer:"",status_code:200,
            source_file:&original,line_number:1,column_number:1,
        });
        assert!(check.blocked);assert_eq!(check.violations.len(),1);
        let now=Instant::now();let plan=service.prepare_csp_report(document,&check.violations[0],now).unwrap();
        assert!(service.forget_reporting_document(document));
        service.queue_prepared_csp_report(plan,now).unwrap();
        let delivery=service.reporting.deliveries.values().next().unwrap();
        let mut expected=receiver.context.security;expected.destination=crate::Destination::Report;expected.top_level_navigation=false;
        assert_eq!(delivery.request.security,expected);
        assert_eq!(delivery.request.origin,receiver.origin);
        assert!(matches!(&delivery.context,DeliveryContext::Legacy(_)));
        assert_eq!(delivery.request.mode,RequestMode::NoCors);
        assert_eq!(delivery.request.redirect,RedirectMode::Error);
        assert_eq!(delivery.request.credentials,CredentialsMode::SameOrigin);
        assert_eq!(delivery.request.referrer_policy,crate::ReferrerPolicy::NoReferrer);
        let finished=run(&mut service,1);
        assert_eq!(finished[0].outcome,ReportOutcome::Failed(ReportError::Policy(FetchError::Redirect)));
        assert_eq!(finished[0].document,document);
        assert_eq!(finished[0].wire().url_list().len(),1);
        assert!(finished[0].wire().terminal_url().serialize().ends_with("/collector"));
        let(request,listener)=peer.join().unwrap();
        assert!(request.starts_with("POST /collector "));
        assert!(!request.to_ascii_lowercase().contains("\r\nreferer:"));
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
        drop(finished);
        assert_eq!(service.release_retired_reporting_document(document),Ok(ReportingRelease::Released));
    }
}
