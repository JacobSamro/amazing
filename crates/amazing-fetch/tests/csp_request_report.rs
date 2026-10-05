use amazing_fetch::api::{ApiError, ApiRequest, ApiRequestInit, ApiRequestInput};
use amazing_fetch::cookies::CookieSite;
use amazing_fetch::*;
use amazing_net::{ParsedUrl, dispatch::DispatchLimits};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};
fn units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
fn no_cors_init() -> ApiRequestInit {
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    init
}
fn receiving(policies: CspPolicyList) -> ReceivingDocumentClient<(u64, u64)> {
    let document = ParsedUrl::parse("https://receiving.test/current#history").unwrap();
    let origin = HttpUrl::parse(&document.serialize())
        .unwrap()
        .origin()
        .clone();
    ReceivingDocumentClient {
        identity: (17, 4),
        creation_url: ParsedUrl::parse("https://user:secret@receiving.test/creation#old").unwrap(),
        document_url: document.clone(),
        base_url: document,
        origin: origin.clone(),
        csp_self_origin: origin.clone(),
        context: ClientContext {
            cookie_site: CookieSite::Origin(origin),
            partition_key: None,
            frame_ancestors: Vec::new(),
            security: SecurityContext {
                destination: Destination::Script,
                ..Default::default()
            },
            csp: policies,
            csp_destination: CspDestination::ScriptElement,
            now_unix: 1_800_000_000,
        },
        referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
    }
}
fn source() -> CspRequestReportContext<'static> {
    CspRequestReportContext {
        element: Some(21),
        referrer: "https://referrer.test/path#fragment",
        status_code: 200,
        source_file: "https://receiving.test/loader.js#secret",
        line_number: 7,
        column_number: 3,
    }
}
#[test]
fn native_request_receipt_keeps_independent_policy_index_effective_directive_disposition_and_exact_meta_units()
 {
    let mut headers = HeaderList::default();
    headers
        .append(
            "content-security-policy",
            "default-src 'none';report-to first;report-uri /first",
        )
        .unwrap();
    headers
        .append(
            "content-security-policy-report-only",
            "default-src 'none';report-to second;report-uri /second",
        )
        .unwrap();
    let mut policies = CspPolicyList::from_headers(&headers).unwrap();
    let mut raw = units("unknown ");
    raw.push(0xdfff);
    raw.extend(units(";script-src 'none'; report-to third"));
    policies.append_meta_utf16(&raw).unwrap();
    let client = receiving(policies);
    let request = client.policy_request(
        HttpUrl::parse("https://resource.test/external.js#secret").unwrap(),
        CspDestination::ScriptElement,
    );
    let check = request.csp.pre_request(&request);
    assert!(check.blocked);
    assert_eq!(check.violations.len(), 3);
    for (index, mut receipt) in check.violations.into_iter().enumerate() {
        assert_eq!(receipt.policy_index(), index);
        assert_eq!(receipt.effective_directive(), "script-src-elem");
        assert_eq!(receipt.request_phase(), CspRequestPhase::PreRequest);
        // Public compatibility diagnostics cannot replace saved native authority.
        receipt.directive = "guessed".into();
        receipt.disposition = CspDisposition::ReportOnly;
        receipt.blocked_url = "https://wrong.test/".into();
        let report = receipt
            .enrich_request_report(&client, &request, &source())
            .unwrap();
        assert_eq!(report.policy_index, index);
        assert_eq!(report.effective_directive, "script-src-elem");
        assert_eq!(report.violated_directive, "script-src-elem");
        assert_eq!(
            report.source_directive,
            if index == 2 {
                "script-src"
            } else {
                "default-src"
            }
        );
        assert_eq!(
            report.original_policy_units,
            client.context.csp.policies[index].original_policy_units
        );
        assert_eq!(
            report.disposition,
            if index == 1 {
                CspDisposition::ReportOnly
            } else {
                CspDisposition::Enforce
            }
        );
        assert_eq!(
            report.report_to.as_deref(),
            Some(["first", "second", "third"][index])
        );
        assert_eq!(report.blocked_uri, "https://resource.test/external.js");
        assert_eq!(report.document_uri, "https://receiving.test/current");
        assert_eq!(report.report_url, "https://receiving.test/creation");
        assert_eq!(report.element, Some(21));
        assert_eq!(report.line_number, 7);
        assert!(report.sample.is_empty());
        assert_eq!(report.report_snapshot().report_type, units("csp-violation"));
    }
}
#[test]
fn native_pre_post_request_and_websocket_receipts_preserve_checked_phase_and_original_resource_privacy()
 {
    let client = receiving(
        CspPolicyList::from_meta("script-src https://resource.test/allowed;connect-src 'none'")
            .unwrap(),
    );
    let request = client.policy_request(
        HttpUrl::parse("https://resource.test/allowed").unwrap(),
        CspDestination::ScriptElement,
    );
    assert!(!request.csp.pre_request(&request).blocked);
    let substituted = HttpUrl::parse("https://redirect.test/private-final#secret").unwrap();
    let check = request.csp.post_request(&request, &substituted);
    assert!(check.blocked);
    assert_eq!(
        check.violations[0].request_phase(),
        CspRequestPhase::PostRequest
    );
    let report = check.violations[0]
        .enrich_saved_request_report(&client, &source())
        .unwrap();
    assert_eq!(report.blocked_uri, "https://resource.test/allowed");
    assert!(!report.blocked_uri.contains("private-final"));
    let mut changed = request.clone();
    changed.csp.policies.clear();
    assert_eq!(
        check.violations[0].enrich_request_report(&client, &changed, &source()),
        Err(CspRequestEnrichmentError::PolicyMismatch)
    );
    let mut changed_client = client.clone();
    changed_client.context.csp.policies.clear();
    assert_eq!(
        check.violations[0].enrich_saved_request_report(&changed_client, &source()),
        Err(CspRequestEnrichmentError::PolicyMismatch)
    );
    changed_client = client.clone();
    changed_client.origin = Origin::opaque();
    assert_eq!(
        check.violations[0].enrich_saved_request_report(&changed_client, &source()),
        Err(CspRequestEnrichmentError::ReceivingClientMismatch)
    );
    let ws = ParsedUrl::parse("wss://socket.test/native#fragment").unwrap();
    let socket_request = client.policy_request(
        HttpUrl::parse("https://socket.test/native").unwrap(),
        CspDestination::Connect,
    );
    let check = socket_request.csp.pre_websocket(&socket_request, &ws);
    assert!(check.blocked);
    assert_eq!(
        check.violations[0].request_phase(),
        CspRequestPhase::WebSocketPreRequest
    );
    assert_eq!(
        check.violations[0]
            .enrich_saved_request_report(&client, &source())
            .unwrap()
            .blocked_uri,
        "wss"
    );
}
#[test]
fn external_request_nonceability_uses_native_duplicate_markup_presence_and_exact_units_before_fetch_check()
 {
    let client = receiving(CspPolicyList::from_meta("script-src 'nonce-YWJj'").unwrap());
    let target = HttpUrl::parse("https://resource.test/external.js").unwrap();
    for (nonce, present, duplicate, markup, allowed) in [
        (units("YWJj"), true, false, false, true),
        (units("YWJj"), false, false, false, false),
        (units("YWJj"), true, true, false, false),
        (units("YWJj"), true, false, true, false),
        (vec![0xdfff], true, false, false, false),
        (units("wrong"), true, false, false, false),
    ] {
        let attribute_name = units("data-markup");
        let attribute_value = units(if markup { "<ScRiPt" } else { "ordinary" });
        let attributes = [CspInlineAttribute {
            name: &attribute_name,
            value: &attribute_value,
        }];
        let element = CspRequestElement {
            nonce: Some(&nonce),
            nonce_attribute_present: present,
            attributes: &attributes,
            duplicate_attribute: duplicate,
            parser_inserted: true,
        };
        let mut request = client.policy_request(target.clone(), CspDestination::ScriptElement);
        request.apply_csp_request_element(&element);
        assert_eq!(request.prepare().is_ok(), allowed);
        assert!(request.csp_parser_inserted);
    }
}
#[test]
fn ordinary_api_native_nonce_receipt_survives_constructor_clone_and_reaches_real_network_pre_admission_check()
 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = HttpUrl::parse(&format!(
        "http://{}/external.js",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let mut client = receiving(CspPolicyList::from_meta("script-src 'nonce-YWJj'").unwrap());
    client.context.security.client_restricts_mixed_content = false;
    let nonce = units("YWJj");
    let element = CspRequestElement {
        nonce: Some(&nonce),
        nonce_attribute_present: true,
        attributes: &[],
        duplicate_attribute: false,
        parser_inserted: true,
    };
    let mut api = ApiRequest::new(
        ApiRequestInput::Url(&units(&target.serialize())),
        no_cors_init(),
        &target,
        client.origin.clone(),
    )
    .unwrap();
    api.apply_receiving_csp_client(&client).unwrap();
    api.apply_csp_request_element(CspExternalElementKind::Script, &element);
    let cloned = api.clone_request().unwrap();
    let constructed = ApiRequest::new(
        ApiRequestInput::Request(&cloned),
        ApiRequestInit::default(),
        &target,
        client.origin.clone(),
    )
    .unwrap();
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_at(constructed, client.context.clone(), Instant::now())
        .unwrap();
    let peer_listener = listener.try_clone().unwrap();
    listener.set_nonblocking(false).unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = peer_listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            assert!(head.len() < 64 * 1024);
        }
        assert!(
            String::from_utf8(head)
                .unwrap()
                .starts_with("GET /external.js HTTP/1.1")
        );
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 1\r\nConnection: close\r\n\r\n1").unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut completed = false;
    while !completed && Instant::now() < deadline {
        for event in service.step(4096, Instant::now()) {
            if let ServiceEvent::Finished {
                id: finished,
                result,
            } = event
            {
                assert_eq!(finished, id);
                assert!(result.is_ok());
                completed = true;
            }
        }
        std::thread::yield_now();
    }
    assert!(
        completed,
        "valid authentic nonce request finishes through genuine peer"
    );
    peer.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    let duplicate = CspRequestElement {
        duplicate_attribute: true,
        ..element
    };
    api.apply_csp_request_element(CspExternalElementKind::Script, &duplicate);
    let violations = match service.start_at(api, client.context.clone(), Instant::now()) {
        Err(ApiError::Network(FetchError::Csp(violations))) => violations,
        _ => panic!("non-nonceable native request must fail CSP before socket admission"),
    };
    assert_eq!(violations.len(), 1);
    let report = violations[0]
        .enrich_saved_request_report(&client, &source())
        .unwrap();
    assert_eq!(report.effective_directive, "script-src-elem");
    assert_eq!(report.element, Some(21));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn sandbox_opaque_receiving_origin_remains_distinct_from_genuine_protected_policy_self_origin() {
    let target = HttpUrl::parse("http://127.0.0.1:39000/script.js").unwrap();
    let mut client = receiving(CspPolicyList::from_meta("script-src 'self'").unwrap());
    client.origin = Origin::opaque();
    client.csp_self_origin = target.origin().clone();
    client.context.cookie_site = CookieSite::Origin(client.origin.clone());
    let mut api = ApiRequest::new(
        ApiRequestInput::Url(&units(&target.serialize())),
        no_cors_init(),
        &target,
        client.origin.clone(),
    )
    .unwrap();
    api.apply_receiving_csp_client(&client).unwrap();
    let mut wrong = client.clone();
    wrong.origin = Origin::opaque();
    assert_eq!(
        api.apply_receiving_csp_client(&wrong),
        Err(CspRequestEnrichmentError::ReceivingClientMismatch)
    );
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service
        .start_at(api, client.context.clone(), Instant::now())
        .unwrap();
    assert!(service.cancel(id));
    let policy = client.policy_request(target, CspDestination::ScriptElement);
    assert_eq!(policy.origin, client.origin);
    assert_ne!(policy.origin, policy.csp_self_origin);
    assert!(!policy.csp.pre_request(&policy).blocked);
}

#[test]
fn filtered_native_csp_provenance_retains_report_only_receipts_without_exposing_opaque_data() {
    let mut headers = HeaderList::default();
    headers.append("content-security-policy-report-only", "script-src 'none'; report-to native").unwrap();
    let policies = CspPolicyList::from_headers(&headers).unwrap();
    let client = receiving(policies);
    for mode in [RequestMode::SameOrigin, RequestMode::NoCors] {
        let target = HttpUrl::parse(if mode == RequestMode::SameOrigin {
            "https://receiving.test/resource.js"
        } else { "https://other.test/resource.js" }).unwrap();
        let mut request = client.policy_request(target.clone(), CspDestination::ScriptElement);
        request.mode = mode;
        request.prepare().unwrap();
        let mut fields = HeaderList::default();
        fields.append("content-type", "application/javascript").unwrap();
        fields.append("x-hidden", "secret").unwrap();
        let response = InternalResponse { status: 200, status_text: "OK".into(), headers: fields, body: Some(b"1".to_vec()) };
        let filtered = filter_response(&request, &response).unwrap();
        let provenance = filtered.csp_provenance.as_ref().unwrap();
        assert_eq!(provenance.url_list(), &[target]);
        assert_eq!(provenance.violations().len(), 2, "one pre and one post receipt, not repeated prepare checks");
        assert_eq!(provenance.violations()[0].request_phase(), CspRequestPhase::PreRequest);
        assert_eq!(provenance.violations()[1].request_phase(), CspRequestPhase::PostRequest);
        for receipt in provenance.violations() {
            let full = receipt.enrich_saved_request_report(&client, &source()).unwrap();
            assert_eq!(full.report_to.as_deref(), Some("native"));
        }
        if mode == RequestMode::NoCors {
            assert_eq!(filtered.response_type, ResponseType::Opaque);
            assert_eq!(filtered.status, 0);
            assert!(filtered.url_list.is_empty());
            assert!(filtered.headers.get("x-hidden").is_none());
            assert!(filtered.body.is_none());
        }
    }
    assert!(amazing_fetch::api::ApiResponse::new(None, Default::default()).unwrap().csp_provenance.is_none());
    assert!(amazing_fetch::api::ApiResponse::error().csp_provenance.is_none());
}

#[test]
fn real_redirect_stream_cache_and_subresource_preserve_native_report_only_receipts_and_lineage() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = HttpUrl::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let peer = std::thread::spawn(move || {
        for _ in 0..7 {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0]; socket.read_exact(&mut byte).unwrap(); head.push(byte[0]);
                assert!(head.len() < 64*1024);
            }
            let text = String::from_utf8(head).unwrap();
            let path = text.split_whitespace().nth(1).unwrap();
            if let Some(mode) = path.strip_prefix("/start/") {
                write!(socket, "HTTP/1.1 302 Found\r\nLocation: /final/{mode}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                assert!(path.starts_with("/final/"));
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=600\r\nContent-Length: 1\r\nConnection: close\r\n\r\n1").unwrap();
            }
        }
    });
    let mut fields = HeaderList::default();
    fields.append("content-security-policy-report-only", "script-src 'none';report-to native").unwrap();
    let context = ClientContext { csp: CspPolicyList::from_headers(&fields).unwrap(), csp_destination: CspDestination::Script,
        security: SecurityContext { destination: Destination::Script, ..Default::default() }, now_unix: 1_800_000_000, ..Default::default() };
    let mut service = NetworkService::new(DispatchLimits::default());
    for mode in 0..3 {
        let api = ApiRequest::new(ApiRequestInput::Url(&units(&format!("/start/{mode}"))), ApiRequestInit::default(), &base, base.origin().clone()).unwrap();
        let id = match mode { 0 => service.start(api, context.clone()).unwrap(), 1 => service.start_stream(api, context.clone()).unwrap(), _ => service.start_subresource(api, context.clone(), Destination::Script) };
        let deadline = Instant::now()+Duration::from_secs(15);
        let mut done = false; let mut provenance = None;
        while !done && Instant::now() < deadline {
            for event in service.step(8192, Instant::now()) {
                match event {
                    ServiceEvent::Finished { id: actual, result } if actual == id => { provenance = result.unwrap().csp_provenance; done = true; }
                    ServiceEvent::ResponseHead { id: actual, head } if actual == id => { provenance = head.csp_provenance; }
                    ServiceEvent::StreamFinished { id: actual } if actual == id => done = true,
                    ServiceEvent::SubresourceFinished { id: actual, response } if actual == id => { assert_eq!(response.body, b"1"); provenance = response.head.csp_provenance; done = true; }
                    ServiceEvent::StreamFailed { error, .. } => panic!("{error:?}"),
                    ServiceEvent::SubresourceFailed { failure, .. } => panic!("{:?}",failure.error),
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(done, "native consumer {mode} did not finish");
        let provenance = provenance.unwrap();
        assert_eq!(provenance.url_list().len(), 2);
        assert_eq!(provenance.url_list()[0].serialize(), base.resolve(&format!("/start/{mode}")).unwrap().serialize());
        assert_eq!(provenance.url_list()[1].serialize(), base.resolve(&format!("/final/{mode}")).unwrap().serialize());
        assert_eq!(provenance.violations().len(), 4);
        assert_eq!(provenance.violations().iter().filter(|v| v.request_phase()==CspRequestPhase::PostRequest).count(), 2);
        if mode == 0 {
            let api = ApiRequest::new(ApiRequestInput::Url(&units("/final/0")), ApiRequestInit::default(), &base, base.origin().clone()).unwrap();
            let prime = service.start(api, context.clone()).unwrap();
            let deadline = Instant::now()+Duration::from_secs(15);
            let mut primed = false;
            while !primed && Instant::now()<deadline {
                for event in service.step(8192, Instant::now()) {
                    if let ServiceEvent::Finished { id, result } = event {
                        if id==prime { assert!(result.is_ok()); primed=true; }
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(primed);
            let api = ApiRequest::new(ApiRequestInput::Url(&units("/final/0")), ApiRequestInit::default(), &base, base.origin().clone()).unwrap();
            let cached = service.start(api, context.clone()).unwrap();
            let events = service.step(8192, Instant::now());
            let response = events.into_iter().find_map(|e| match e { ServiceEvent::Finished { id, result } if id==cached => Some(result.unwrap()), _ => None }).expect("fresh cache finishes without another peer request");
            let receipts = response.csp_provenance.as_ref().unwrap();
            assert_eq!(receipts.url_list().len(), 1);
            assert_eq!(receipts.violations().len(), 2);
            assert_eq!(response.clone_response().unwrap().csp_provenance, response.csp_provenance);
        }
    }
    peer.join().unwrap();
}

#[test]
fn blocked_redirect_preserves_prior_report_only_receipts_and_each_authentic_check_lineage() {
    let mut fields = HeaderList::default();
    fields.append("content-security-policy", "script-src https://allowed.example;report-to enforce").unwrap();
    fields.append("content-security-policy-report-only", "script-src 'none';report-to reportonly").unwrap();
    let client = receiving(CspPolicyList::from_headers(&fields).unwrap());
    let first = HttpUrl::parse("https://allowed.example/start.js").unwrap();
    let blocked = HttpUrl::parse("https://blocked.example/final.js").unwrap();
    let mut request = client.policy_request(first.clone(), CspDestination::ScriptElement);
    request.mode = RequestMode::NoCors;
    request.prepare().unwrap();
    let response = InternalResponse { status: 302, status_text: "Found".into(), headers: HeaderList::default(), body: Some(Vec::new()) };
    assert_eq!(redirect(&mut request, &response, Some(blocked.clone())).unwrap(), RedirectAction::Follow);
    let FetchError::Csp(receipts) = request.prepare().unwrap_err() else { panic!("expected genuine CSP block") };
    assert_eq!(receipts.len(), 4);
    assert_eq!(receipts[0].request_url_list(), &[first.clone()]);
    assert_eq!(receipts[1].request_phase(), CspRequestPhase::PostRequest);
    for receipt in &receipts[2..] {
        assert_eq!(receipt.request_url_list(), &[first.clone(), blocked.clone()]);
        assert_eq!(receipt.request_phase(), CspRequestPhase::PreRequest);
    }
    let groups: Vec<_> = receipts.iter().map(|receipt| receipt.enrich_saved_request_report(&client, &source()).unwrap().report_to.unwrap()).collect();
    assert_eq!(groups, ["reportonly", "reportonly", "enforce", "reportonly"]);
}

fn report_only_client() -> ReceivingDocumentClient<(u64,u64)> {
    let mut fields=HeaderList::default();
    fields.append("content-security-policy-report-only","script-src 'none';report-to reportonly").unwrap();
    receiving(CspPolicyList::from_headers(&fields).unwrap())
}
fn read_native_get(stream:&mut std::net::TcpStream)->String {
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut bytes=Vec::new();let mut buffer=[0;1024];
    while !bytes.windows(4).any(|w|w==b"\r\n\r\n") {
        let count=stream.read(&mut buffer).unwrap();assert_ne!(count,0);bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).unwrap()
}
fn accept_native_preflight_then_get(listener:&TcpListener,path:&str)->(std::net::TcpStream,String) {
    // Trusted loader NoStore adds Cache-Control/Pragma. A real Cors request
    // must perform its native OPTIONS exchange before the retained GET control.
    for _ in 0..2 {
        let (mut stream,_)=listener.accept().unwrap();let request=read_native_get(&mut stream);
        if request.starts_with("OPTIONS ") {
            assert!(request.starts_with(&format!("OPTIONS {path} HTTP/1.1")));
            let lower=request.to_ascii_lowercase();assert!(lower.contains("access-control-request-method: get\r\n"));
            let fields=lower.lines().find_map(|line|line.strip_prefix("access-control-request-headers:")).unwrap();
            assert!(fields.contains("cache-control"));assert!(fields.contains("pragma"));
            let origin=request.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("origin")).map(|(_,value)|value.trim())).unwrap();
            stream.write_all(format!("HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: cache-control, pragma\r\nAccess-Control-Max-Age: 600\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        } else {assert!(request.starts_with(&format!("GET {path} HTTP/1.1")));return (stream,request);}
    }
    panic!("genuine native preflight did not lead to GET")
}
fn native_subresource_failure(service:&mut NetworkService,id:RequestId)->SubresourceFailure {
    let deadline=Instant::now()+Duration::from_secs(15);
    while Instant::now()<deadline {
        for event in service.step(8192,Instant::now()) {
            match event {
                ServiceEvent::SubresourceFailed{id:found,failure} if found==id=>return failure,
                ServiceEvent::SubresourceFinished{id:found,..} if found==id=>panic!("expected typed native failure"),
                ServiceEvent::Finished{id:found,..} if found==id=>panic!("trusted native request must retain its terminal route"),_=>{}
            }
        }
        std::thread::yield_now();
    }
    panic!("native terminal deadline")
}
fn assert_failure_receipts(failure:&SubresourceFailure,client:&ReceivingDocumentClient<(u64,u64)>,phases:&[CspRequestPhase],url_lengths:&[usize]) {
    let provenance=failure.csp_provenance.as_ref().expect("genuine native checks retain receipts");
    assert_eq!(provenance.violations().len(),phases.len());
    for ((receipt,phase),length) in provenance.violations().iter().zip(phases).zip(url_lengths) {
        assert_eq!(receipt.request_phase(),*phase);assert_eq!(receipt.request_url_list().len(),*length);
        let report=receipt.enrich_saved_request_report(client,&source()).unwrap();
        assert_eq!(report.disposition,CspDisposition::ReportOnly);assert_eq!(report.policy_index,0);
        assert_eq!(report.original_policy_units,client.context.csp.policies[0].original_policy_units);
        assert_eq!(report.report_to.as_deref(),Some("reportonly"));
    }
}
#[test]
fn native_subresource_failure_retains_actual_post_check_before_cors_mime_nosniff_integrity_and_transport() {
    for (case,expected) in [("cors",FetchError::Cors),("mime",FetchError::MimeType),("nosniff",FetchError::Nosniff),("integrity",FetchError::Integrity),("transport",FetchError::Transport)] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let peer=std::thread::spawn(move||{
            let (mut stream,request)=accept_native_preflight_then_get(&listener,"/script.js");assert!(request.starts_with("GET /script.js HTTP/1.1"));
            if case!="transport" {
                let mime=match case {"mime"=>"image/png","nosniff"=>"text/plain",_=>"application/javascript"};
                let nosniff=if case=="nosniff" {"X-Content-Type-Options: nosniff\r\n"}else{""};
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\n{nosniff}Content-Length: 4\r\nConnection: close\r\n\r\ntrue").as_bytes()).unwrap();
            }
        });
        let client=report_only_client();let base=HttpUrl::parse(&client.base_url.serialize()).unwrap();
        let mut init=ApiRequestInit::default();init.mode=Some(if case=="cors" {"cors"}else{"no-cors"}.into());
        if case=="integrity" {init.integrity=Some("sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=".into());}
        let input=units(&format!("http://127.0.0.1:{port}/script.js#private"));
        let mut request=ApiRequest::new(ApiRequestInput::Url(&input),init,&base,client.origin.clone()).unwrap();request.apply_receiving_csp_client(&client).unwrap();
        let mut service=NetworkService::new(DispatchLimits::default());let id=service.start_subresource(request,client.context.clone(),Destination::Script);
        let failure=native_subresource_failure(&mut service,id);assert_eq!(failure.error,ApiError::Network(expected));
        if case=="transport" {assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest],&[1]);}
        else {assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest,CspRequestPhase::PostRequest],&[1,1]);}
        assert_eq!(failure.csp_provenance.as_ref().unwrap().url_list().len(),1);peer.join().unwrap();
    }
}
#[test]
fn native_subresource_redirect_failure_retains_original_and_current_hop_check_lineages() {
    for cors_failure in [false,true] {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
        let peer=std::thread::spawn(move||{
            let (mut first,request)=accept_native_preflight_then_get(&listener,"/first");assert!(request.starts_with("GET /first HTTP/1.1"));
            first.write_all(b"HTTP/1.1 302 Found\r\nLocation: /second\r\nAccess-Control-Allow-Origin: https://receiving.test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();drop(first);
            let (mut second,request)=accept_native_preflight_then_get(&listener,"/second");assert!(request.starts_with("GET /second HTTP/1.1"));
            if cors_failure {second.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntrue").unwrap();}
        });
        let client=report_only_client();let base=HttpUrl::parse(&client.base_url.serialize()).unwrap();let input=units(&format!("http://127.0.0.1:{port}/first"));
        let mut init=ApiRequestInit::default();init.mode=Some(if cors_failure {"cors"}else{"no-cors"}.into());
        let request=ApiRequest::new(ApiRequestInput::Url(&input),init,&base,client.origin.clone()).unwrap();
        let mut service=NetworkService::new(DispatchLimits::default());let id=service.start_subresource(request,client.context.clone(),Destination::Script);
        let failure=native_subresource_failure(&mut service,id);assert_eq!(failure.error,ApiError::Network(if cors_failure {FetchError::Cors}else{FetchError::Transport}));
        if cors_failure {assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest,CspRequestPhase::PostRequest,CspRequestPhase::PreRequest,CspRequestPhase::PostRequest],&[1,1,2,2]);}
        else {assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest,CspRequestPhase::PostRequest,CspRequestPhase::PreRequest],&[1,1,2]);}
        assert_eq!(failure.csp_provenance.as_ref().unwrap().url_list().len(),2);peer.join().unwrap();
    }
}
#[test]
fn native_subresource_admission_and_cancellation_keep_genuine_checked_receipts_without_fabricating_unchecked_authority() {
    let client=report_only_client();let base=HttpUrl::parse(&client.base_url.serialize()).unwrap();let input=units("http://resource.test:1/script");
    let request=ApiRequest::new(ApiRequestInput::Url(&input),no_cors_init(),&base,client.origin.clone()).unwrap();
    let mut service=NetworkService::new(DispatchLimits::default());let id=service.start_subresource(request,client.context.clone(),Destination::Script);
    let failure=native_subresource_failure(&mut service,id);assert_eq!(failure.error,ApiError::Network(FetchError::BadPort));assert!(!service.has_pending_request_io(id));
    assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest],&[1]);
    let mut request=ApiRequest::new(ApiRequestInput::Url(&input),no_cors_init(),&base,client.origin.clone()).unwrap();request.signal_aborted=true;
    let id=service.start_subresource(request,client.context.clone(),Destination::Script);let failure=native_subresource_failure(&mut service,id);
    assert_eq!(failure.error,ApiError::Aborted);assert!(failure.csp_provenance.is_none());
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();let (tx,rx)=std::sync::mpsc::channel();
    let peer=std::thread::spawn(move||{
        let (mut first,_)=listener.accept().unwrap();assert!(read_native_get(&mut first).starts_with("GET /first HTTP/1.1"));
        first.write_all(b"HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();drop(first);
        let (mut second,_)=listener.accept().unwrap();assert!(read_native_get(&mut second).starts_with("GET /second HTTP/1.1"));tx.send(()).unwrap();
        let mut byte=[0];let count=second.read(&mut byte).unwrap();assert_eq!(count,0,"actual canceled socket closes");
    });
    let input=units(&format!("http://127.0.0.1:{port}/first"));let request=ApiRequest::new(ApiRequestInput::Url(&input),no_cors_init(),&base,client.origin.clone()).unwrap();
    let id=service.start_subresource(request,client.context.clone(),Destination::Script);let deadline=Instant::now()+Duration::from_secs(15);let mut admitted=false;
    while Instant::now()<deadline {let events=service.step(8192,Instant::now());assert!(events.iter().all(|event|!matches!(event,ServiceEvent::SubresourceFinished{..}|ServiceEvent::SubresourceFailed{..})));if rx.try_recv().is_ok(){admitted=true;break;}std::thread::yield_now();}
    assert!(admitted);assert!(service.abort(id));let failure=native_subresource_failure(&mut service,id);assert_eq!(failure.error,ApiError::Aborted);
    assert_failure_receipts(&failure,&client,&[CspRequestPhase::PreRequest,CspRequestPhase::PostRequest,CspRequestPhase::PreRequest],&[1,1,2]);peer.join().unwrap();
}
