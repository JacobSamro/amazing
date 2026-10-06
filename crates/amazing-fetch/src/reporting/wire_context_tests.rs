//! Real server-request controls for Reporting's independent wire settings.
use super::*;

fn queue_kind(service: &mut NetworkService, handle: ReportingDocument, kind: &str, url: &str) {
    let now = Instant::now();
    match kind {
        "csp" => { service.queue_csp_report(handle, &violation(url), now, now).unwrap(); }
        "test" => { service.generate_test_report(handle, &[0xd800, 0x61], &"csp".encode_utf16().collect::<Vec<_>>(), now).unwrap().delivery.unwrap(); }
        "coep" => {
            let urls = [ParsedUrl::parse("https://blocked.test/resource#hidden").unwrap()];
            service.generate_coep_report(handle, CoepReportBody::Corp {
                response_url_list: &urls, disposition: CoepReportDisposition::Enforce, destination: "script",
            }, &"csp".encode_utf16().collect::<Vec<_>>(), now, now).unwrap().delivery.unwrap();
        }
        _ => panic!("invalid native control kind"),
    }
}

#[test]
fn actual_opaque_global_modern_report_families_use_creation_origin_and_null_client_cookies() {
    for kind in ["csp", "test", "coep"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = HttpUrl::parse(&format!("http://{}/creation", listener.local_addr().unwrap())).unwrap();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream.write_all(b"HTTP/1.1 204 Accepted\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&url, &format!("{}#private", url.serialize()));
        client.origin = Origin::opaque();
        client.context.cookie_site = crate::cookies::CookieSite::CrossSite;
        client.context.security.client_restricts_mixed_content = true;
        client.context.security.embedder_policy = crate::EmbedderPolicy::RequireCorp;
        client.context.partition_key = Some(url.origin().site(&crate::cookies::ConservativeDomains));
        let cookie_access = crate::cookies::CookieAccess {
            url: &url, site_for_cookies: crate::cookies::CookieSite::NoClient,
            http_api: true, top_level_navigation: false, method: "POST", partition_key: None,
        };
        service.cookies.set_cookie(b"strict=one; Path=/; SameSite=Strict; HttpOnly", &cookie_access, client.context.now_unix).unwrap();
        let original_origin = client.origin.clone();
        let handle = service.register_reporting_client_response(&client, &url, &headers(r#"csp="/report""#)).unwrap();
        queue_kind(&mut service, handle, kind, &url.serialize());
        let delivery = service.reporting.deliveries.values().next().unwrap();
        assert_eq!(delivery.request.origin, *url.origin());
        assert_eq!(delivery.request.security.destination, crate::Destination::Report);
        assert!(!delivery.request.security.client_restricts_mixed_content);
        assert_eq!(delivery.request.security.embedder_policy, crate::EmbedderPolicy::UnsafeNone);
        assert!(matches!(&delivery.context, DeliveryContext::NoClient { .. }));
        assert!(delivery.context.partition_key().is_none());
        assert_eq!(service.reporting.document(handle).unwrap().origin, original_origin);
        let completed = run(&mut service, 1);
        assert_eq!(completed[0].outcome, ReportOutcome::Response {status:204,accepted:true});
        assert_eq!(completed[0].document, handle);
        assert_eq!(completed[0].wire().report_url(), url.serialize());
        assert_eq!(completed[0].wire().owner_origin(), &original_origin);
        assert_eq!(completed[0].wire().request_origin(), url.origin());
        assert_eq!(completed[0].wire().url_list().len(), 1);
        assert!(completed[0].wire().terminal_includes_credentials());
        assert!(!completed[0].wire().terminal_is_preflight());
        let wire = rx.recv().unwrap(); peer.join().unwrap();
        let lower = wire.to_ascii_lowercase();
        assert!(lower.starts_with("post /report "));
        assert!(lower.contains(&format!("\r\norigin: {}", url.origin().serialize())));
        assert!(lower.contains("\r\ncookie: strict=one"));
        assert!(!lower.contains("\r\nreferer:"));
        assert!(wire.contains(&quote(&url.serialize())));
        assert!(!wire.contains("#private"));
    }
}

#[test]
fn actual_cross_origin_report_families_preflight_immutable_report_origin_and_omit_credentials() {
    for kind in ["csp", "test", "coep"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let creation = HttpUrl::parse(&format!("http://localhost:{port}/creation")).unwrap();
        let endpoint = HttpUrl::parse(&format!("http://127.0.0.1:{port}/report")).unwrap();
        let origin = creation.origin().serialize();
        let response_origin = origin.clone();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            for preflight in [true, false] {
                let (mut stream, _) = listener.accept().unwrap();
                let wire = read_request(&mut stream);
                assert_eq!(wire.starts_with("OPTIONS "), preflight);
                tx.send(wire).unwrap();
                stream.write_all(format!("HTTP/1.1 204 Accepted\r\nAccess-Control-Allow-Origin: {response_origin}\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
            }
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&creation, &creation.serialize()); client.origin = Origin::opaque();
        let access = crate::cookies::CookieAccess {url:&endpoint,site_for_cookies:crate::cookies::CookieSite::NoClient,http_api:true,top_level_navigation:false,method:"POST",partition_key:None};
        service.cookies.set_cookie(b"cross=secret; Path=/", &access, client.context.now_unix).unwrap();
        let handle = service.register_reporting_client_response(&client, &creation, &headers(&format!("csp=\"{}\"",endpoint.serialize()))).unwrap();
        queue_kind(&mut service, handle, kind, &creation.serialize());
        assert_eq!(run(&mut service,1)[0].outcome, ReportOutcome::Response {status:204,accepted:true});
        for _ in 0..2 {
            let wire=rx.recv().unwrap().to_ascii_lowercase();
            assert!(wire.contains(&format!("\r\norigin: {origin}")));
            assert!(!wire.contains("\r\ncookie:")); assert!(!wire.contains("\r\nreferer:"));
        }
        peer.join().unwrap();
        assert!(service.reporting.document(handle).unwrap().origin.opaque_id().is_some());
    }
}

#[test]
fn actual_retired_csp_plan_keeps_endpoint_owner_but_uses_saved_report_url_wire_origin() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let url=HttpUrl::parse(&format!("http://{}/original",listener.local_addr().unwrap())).unwrap();
    let (tx,rx)=mpsc::channel();
    let peer=thread::spawn(move || {let(mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();stream.write_all(b"HTTP/1.1 410 Gone\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();});
    let mut service=NetworkService::new(DispatchLimits::default());
    let mut client=receiving_client(&url,&url.serialize());client.origin=Origin::opaque();
    let handle=service.register_reporting_client_response(&client,&url,&headers(r#"csp="/report""#)).unwrap();
    let now=Instant::now();let violation=violation(&url.serialize());
    let first=service.prepare_csp_report(handle,&violation,now).unwrap();
    let second=service.prepare_csp_report(handle,&violation,now).unwrap();
    assert!(service.forget_reporting_document(handle));
    service.queue_prepared_csp_report(first,now).unwrap();
    let finished=run(&mut service,1);assert_eq!(finished[0].document,handle);
    assert_eq!(finished[0].outcome,ReportOutcome::Response {status:410,accepted:false});
    assert_eq!(service.queue_prepared_csp_report(second,Instant::now()),Err(ReportError::NoEndpoint));
    assert!(matches!(service.generate_test_report(handle,&[],&[],now),Err(ReportError::StaleDocument)));
    let wire=rx.recv().unwrap().to_ascii_lowercase();peer.join().unwrap();
    assert!(wire.contains(&format!("\r\norigin: {}",url.origin().serialize())));
    assert_eq!(client.origin.opaque_id().is_some(),true);
}

#[test]
fn actual_legacy_report_uri_preserves_original_opaque_origin_and_client_cookie_rules() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let url=HttpUrl::parse(&format!("http://{}/original",listener.local_addr().unwrap())).unwrap();
    let(tx,rx)=mpsc::channel();let peer=thread::spawn(move || {let(mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();stream.write_all(b"HTTP/1.1 204 Accepted\r\nConnection: close\r\n\r\n").unwrap();});
    let mut service=NetworkService::new(DispatchLimits::default());let mut client=receiving_client(&url,&url.serialize());client.origin=Origin::opaque();
    let handle=service.register_reporting_client_response(&client,&url,&headers(r#"csp="/unused""#)).unwrap();
    let mut v=violation(&url.serialize());v.report_to=None;v.report_uris=vec!["/legacy".into()];
    let now=Instant::now();service.queue_csp_report(handle,&v,now,now).unwrap();
    let delivery=service.reporting.deliveries.values().next().unwrap();
    assert_eq!(delivery.request.origin,client.origin);assert!(matches!(&delivery.context,DeliveryContext::Legacy(_)));
    assert_eq!(run(&mut service,1)[0].route,ReportRoute::LegacyCsp);
    let wire=rx.recv().unwrap().to_ascii_lowercase();peer.join().unwrap();
    assert!(wire.contains("\r\norigin: null"));assert!(!wire.contains("\r\ncookie:"));
}

#[test]
fn actual_modern_redirect_recomputes_credentials_without_changing_report_origin_or_endpoint_owner() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let url=HttpUrl::parse(&format!("http://127.0.0.1:{port}/creation")).unwrap();
    let origin=url.origin().serialize();let peer_origin=origin.clone();
    let(tx,rx)=mpsc::channel();let peer=thread::spawn(move || {
        for hop in 0..3 {
            let(mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();
            let response=if hop==0 {format!("HTTP/1.1 307 Redirect\r\nLocation: http://localhost:{port}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")}
                else {format!("HTTP/1.1 {} Result\r\nAccess-Control-Allow-Origin: {peer_origin}\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",if hop==1{204}else{410})};
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    let mut service=NetworkService::new(DispatchLimits::default());let mut client=receiving_client(&url,&url.serialize());client.origin=Origin::opaque();
    let access=crate::cookies::CookieAccess {url:&url,site_for_cookies:crate::cookies::CookieSite::NoClient,http_api:true,top_level_navigation:false,method:"POST",partition_key:None};
    service.cookies.set_cookie(b"same=one; Path=/; SameSite=Strict",&access,client.context.now_unix).unwrap();
    let handle=service.register_reporting_client_response(&client,&url,&headers(r#"csp="/first""#)).unwrap();let now=Instant::now();
    let report=service.generate_test_report(handle,&[0xd800],&"csp".encode_utf16().collect::<Vec<_>>(),now).unwrap();
    assert_eq!(run(&mut service,1)[0].outcome,ReportOutcome::Response {status:410,accepted:false});
    let first=rx.recv().unwrap();let options=rx.recv().unwrap();let final_post=rx.recv().unwrap();peer.join().unwrap();
    assert!(first.starts_with("POST /first "));assert!(first.to_ascii_lowercase().contains("\r\ncookie: same=one"));
    assert!(options.starts_with("OPTIONS /final "));assert!(final_post.starts_with("POST /final "));
    for wire in [&first,&options,&final_post] {assert!(wire.to_ascii_lowercase().contains(&format!("\r\norigin: {origin}")));}
    for wire in [&options,&final_post] {assert!(!wire.to_ascii_lowercase().contains("\r\ncookie:"));}
    assert_eq!(first.split("\r\n\r\n").nth(1),final_post.split("\r\n\r\n").nth(1));
    assert_eq!(report.report_url,url.serialize());assert_eq!(service.reporting.document(handle).unwrap().origin,client.origin);
    assert_eq!(service.generate_test_report(handle,&[],&"csp".encode_utf16().collect::<Vec<_>>(),now).unwrap().delivery,Err(ReportError::NoEndpoint));
}

#[test]
fn actual_modern_preflight_denial_retains_observer_snapshot_and_original_endpoint_failure() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let port=listener.local_addr().unwrap().port();
    let creation=HttpUrl::parse(&format!("http://localhost:{port}/creation")).unwrap();let endpoint=format!("http://127.0.0.1:{port}/report");
    let(tx,rx)=mpsc::channel();let peer=thread::spawn(move || {let(mut stream,_)=listener.accept().unwrap();tx.send(read_request(&mut stream)).unwrap();stream.write_all(b"HTTP/1.1 204 Accepted\r\nAccess-Control-Allow-Origin: null\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\nConnection: close\r\n\r\n").unwrap();listener.set_nonblocking(true).unwrap();listener});
    let mut service=NetworkService::new(DispatchLimits::default());let mut client=receiving_client(&creation,&creation.serialize());client.origin=Origin::opaque();
    let handle=service.register_reporting_client_response(&client,&creation,&headers(&format!("csp=\"{endpoint}\""))).unwrap();let message=[0xd800,0x61];
    let report=service.generate_test_report(handle,&message,&"csp".encode_utf16().collect::<Vec<_>>(),Instant::now()).unwrap();assert!(report.delivery.is_ok());
    assert_eq!(report.message,message);assert_eq!(report.report_url,creation.serialize());
    let finished=run(&mut service,1);assert!(matches!(finished[0].outcome,ReportOutcome::Failed(ReportError::Policy(FetchError::Cors))));
    assert_eq!(finished[0].document, handle);
    assert_eq!(finished[0].wire().report_url(), creation.serialize());
    assert_eq!(finished[0].wire().owner_origin(), &client.origin);
    assert_eq!(finished[0].wire().request_origin(), creation.origin());
    assert!(finished[0].wire().terminal_is_preflight());
    assert!(!finished[0].wire().terminal_includes_credentials());
    let wire=rx.recv().unwrap().to_ascii_lowercase();assert!(wire.starts_with("options "));assert!(wire.contains(&format!("\r\norigin: {}",creation.origin().serialize())));
    let listener=peer.join().unwrap();assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
    let saved=service.reporting.document(handle).unwrap();assert_eq!(saved.failures.get("csp"),Some(&1));assert!(saved.endpoints.contains_key("csp"));assert_eq!(saved.origin,client.origin);
}

#[test]
fn actual_fresh_non_http_global_has_observer_report_without_modern_upload_authority() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    let transport=HttpUrl::parse(&format!("http://{}/unused",listener.local_addr().unwrap())).unwrap();
    let mut service=NetworkService::new(DispatchLimits::default());
    for creation in ["about:blank","about:srcdoc","data:text/html,initial"] {
        let mut client=receiving_client(&transport,creation);client.document_url=ParsedUrl::parse(creation).unwrap();client.base_url=client.document_url.clone();client.origin=Origin::opaque();
        let handle=service.register_reporting_initial_client(&client).unwrap();let report=service.generate_test_report(handle,&[0xdfff],&"default".encode_utf16().collect::<Vec<_>>(),Instant::now()).unwrap();
        assert_eq!(report.report_url,client.creation_url.scheme());assert_eq!(report.message,vec![0xdfff]);assert_eq!(report.delivery,Err(ReportError::NoEndpoint));assert!(service.forget_reporting_document(handle));
    }
    assert!(!service.has_pending_reporting_io());assert_eq!(service.dispatcher.in_flight(),0);assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
}


#[test]
fn actual_cross_origin_bounce_never_restores_modern_credentials_or_global_authority() {
    for status in [307, 308] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let creation = HttpUrl::parse(&format!("http://127.0.0.1:{port}/creation")).unwrap();
        let first = HttpUrl::parse(&format!("http://127.0.0.1:{port}/first")).unwrap();
        let middle = HttpUrl::parse(&format!("http://localhost:{port}/middle")).unwrap();
        let final_url = HttpUrl::parse(&format!("http://127.0.0.1:{port}/final")).unwrap();
        let middle_location = middle.serialize();
        let final_location = final_url.serialize();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            for stage in 0..5 {
                let (mut stream, _) = listener.accept().unwrap();
                let wire = read_request(&mut stream);
                tx.send(wire).unwrap();
                let response = match stage {
                    0 => format!("HTTP/1.1 {status} Redirect\r\nLocation: {middle_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    1 | 3 => "HTTP/1.1 204 Grant\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\nConnection: close\r\n\r\n".to_owned(),
                    2 => format!("HTTP/1.1 {status} Redirect\r\nAccess-Control-Allow-Origin: *\r\nLocation: {final_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    _ => "HTTP/1.1 204 Accepted\r\nAccess-Control-Allow-Origin: *\r\nSet-Cookie: returned=forbidden; Path=/\r\nConnection: close\r\n\r\n".to_owned(),
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&creation, &creation.serialize());
        client.origin = Origin::opaque();
        let original_origin = client.origin.clone();
        let access = crate::cookies::CookieAccess { url:&creation, site_for_cookies:crate::cookies::CookieSite::NoClient,
            http_api:true, top_level_navigation:false, method:"POST", partition_key:None };
        service.cookies.set_cookie(b"original=one; Path=/", &access, client.context.now_unix).unwrap();
        let handle = service.register_reporting_client_response(&client, &creation,
            &headers(&format!("csp=\"{}\"", first.serialize()))).unwrap();
        queue_kind(&mut service, handle, "test", &creation.serialize());
        // Retirement cannot retarget the queued request to a fresh global.
        assert!(service.forget_reporting_document(handle));
        let fresh = service.register_reporting_initial_client(&client).unwrap();
        let completed = run(&mut service, 1);
        peer.join().unwrap();
        assert_eq!(completed[0].outcome, ReportOutcome::Response {status:204,accepted:true});
        assert_eq!(completed[0].document, handle);
        let receipt = completed[0].wire();
        assert_eq!(receipt.report_url(), creation.serialize());
        assert_eq!(receipt.owner_origin(), &original_origin);
        assert_eq!(receipt.request_origin(), creation.origin());
        assert_eq!(receipt.registered_endpoint(), &first);
        assert_eq!(receipt.url_list(), &[first, middle, final_url]);
        assert!(!receipt.terminal_includes_credentials());
        assert!(!receipt.terminal_is_preflight());
        let wires: Vec<_> = rx.try_iter().collect();
        assert_eq!(wires.len(), 5);
        assert!(wires[0].to_ascii_lowercase().contains("\r\ncookie: original=one"));
        for (index, wire) in wires.iter().enumerate().skip(1) {
            let lower = wire.to_ascii_lowercase();
            assert!(!lower.contains("\r\ncookie:"));
            assert!(!lower.contains("\r\nreferer:"));
            assert_eq!(lower.starts_with("options "), index == 1 || index == 3);
        }
        assert!(wires[4].to_ascii_lowercase().contains("\r\norigin: null"));
        assert!(wires[4].contains(&quote(&creation.serialize())));
        let cookie = service.cookies.cookie_header(&access, client.context.now_unix).unwrap();
        assert_eq!(cookie, b"original=one");
        assert_eq!(service.generate_test_report(fresh, &[0xd800], &"csp".encode_utf16().collect::<Vec<_>>(), Instant::now()).unwrap().delivery, Err(ReportError::NoEndpoint));
    }
}


#[test]
fn actual_terminal_transport_failure_preserves_retired_original_report_wire_custody() {
    for kind in ["csp", "test", "coep"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let creation = HttpUrl::parse(&format!("http://{}/creation", listener.local_addr().unwrap())).unwrap();
        let endpoint = HttpUrl::parse(&format!("http://{}/report", listener.local_addr().unwrap())).unwrap();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            // Authentic incomplete HTTP framing, not a synthetic event injection.
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nx").unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&creation, &creation.serialize());
        client.origin = Origin::opaque();
        let owner = client.origin.clone();
        let handle = service.register_reporting_client_response(&client, &creation,
            &headers(&format!("csp=\"{}\"", endpoint.serialize()))).unwrap();
        queue_kind(&mut service, handle, kind, &creation.serialize());
        assert!(service.forget_reporting_document(handle));
        let fresh = service.register_reporting_client_response(&client, &creation,
            &headers(&format!("csp=\"{}\"", endpoint.serialize()))).unwrap();
        let completed = run(&mut service, 1);
        peer.join().unwrap();
        assert!(matches!(completed[0].outcome, ReportOutcome::Failed(ReportError::Transport)));
        assert_eq!(completed[0].document, handle);
        assert_ne!(completed[0].document, fresh);
        assert_eq!(completed[0].wire().report_url(), creation.serialize());
        assert_eq!(completed[0].wire().owner_origin(), &owner);
        assert_eq!(completed[0].wire().request_origin(), creation.origin());
        assert_eq!(completed[0].wire().registered_endpoint(), &endpoint);
        assert_eq!(completed[0].wire().url_list(), &[endpoint]);
        assert!(completed[0].wire().terminal_includes_credentials());
        assert!(!completed[0].wire().terminal_is_preflight());
        assert!(rx.recv().unwrap().contains(&quote(&creation.serialize())));
        let saved = service.reporting.document(fresh).unwrap();
        assert!(saved.failures.is_empty());
        assert!(saved.named_endpoint("csp").unwrap().1.is_active());
        assert_eq!(service.reporting.retained_bytes, 0);
    }
}


#[test]
fn actual_stalled_report_deadline_keeps_original_custody_after_global_retirement() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let creation = HttpUrl::parse(&format!("http://{}/creation", listener.local_addr().unwrap())).unwrap();
    let endpoint = HttpUrl::parse(&format!("http://{}/report", listener.local_addr().unwrap())).unwrap();
    let (tx, rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        tx.send(read_request(&mut stream)).unwrap();
        release_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        drop(stream);
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let mut client = receiving_client(&creation, &creation.serialize());
    client.origin = Origin::opaque();
    let owner = client.origin.clone();
    let handle = service.register_reporting_client_response(&client, &creation,
        &headers(&format!("csp=\"{}\"", endpoint.serialize()))).unwrap();
    queue_kind(&mut service, handle, "test", &creation.serialize());
    let mut wire = None;
    for _ in 0..500 {
        assert!(service.step(4096, Instant::now()).is_empty());
        if let Ok(received) = rx.try_recv() { wire = Some(received); break; }
        thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(wire.unwrap().starts_with("POST /report "));
    assert!(service.forget_reporting_document(handle));
    let fresh = service.register_reporting_initial_client(&client).unwrap();
    let events = service.step(4096, Instant::now() + std::time::Duration::from_secs(11));
    release_tx.send(()).unwrap();
    peer.join().unwrap();
    let completed: Vec<_> = events.into_iter().filter_map(|event| match event {
        ServiceEvent::ReportFinished(report) => Some(report), _ => None,
    }).collect();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].outcome, ReportOutcome::Failed(ReportError::Transport));
    assert_eq!(completed[0].document, handle);
    assert_ne!(completed[0].document, fresh);
    assert_eq!(completed[0].wire().owner_origin(), &owner);
    assert_eq!(completed[0].wire().report_url(), creation.serialize());
    assert_eq!(completed[0].wire().request_origin(), creation.origin());
    assert_eq!(completed[0].wire().url_list(), &[endpoint]);
    assert!(completed[0].wire().terminal_includes_credentials());
    assert_eq!(service.reporting.retained_bytes, 0);
    assert!(!service.has_pending_reporting_io());
    assert!(service.step(4096, Instant::now() + std::time::Duration::from_secs(12)).is_empty());
}


#[test]
fn actual_rejected_redirect_hop_does_not_relabel_last_transport_credentials() {
    for status in [301, 302, 303, 307, 308] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let creation = HttpUrl::parse(&format!("http://{}/creation", listener.local_addr().unwrap())).unwrap();
        let endpoint = HttpUrl::parse(&format!("http://{}/report", listener.local_addr().unwrap())).unwrap();
        let rejected = HttpUrl::parse("http://127.0.0.1:25/rejected").unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("POST /report "));
            stream.write_all(format!("HTTP/1.1 {status} Redirect\r\nLocation: http://127.0.0.1:25/rejected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&creation, &creation.serialize());
        client.origin = Origin::opaque();
        let original_origin = client.origin.clone();
        let handle = service.register_reporting_client_response(&client, &creation,
            &headers(&format!("csp=\"{}\"", endpoint.serialize()))).unwrap();
        queue_kind(&mut service, handle, "test", &creation.serialize());
        assert!(service.forget_reporting_document(handle));
        let completed = run(&mut service, 1);
        peer.join().unwrap();
        assert_eq!(completed[0].outcome, ReportOutcome::Failed(ReportError::Policy(FetchError::BadPort)));
        let receipt = completed[0].wire();
        assert_eq!(completed[0].document, handle);
        assert_eq!(receipt.report_url(), creation.serialize());
        assert_eq!(receipt.owner_origin(), &original_origin);
        assert_eq!(receipt.request_origin(), creation.origin());
        assert_eq!(receipt.registered_endpoint(), &endpoint);
        assert_eq!(receipt.url_list(), &[endpoint.clone(), rejected]);
        assert_eq!(receipt.terminal_url(), &endpoint);
        assert!(!receipt.terminal_is_preflight());
        assert!(receipt.terminal_includes_credentials());
        assert_eq!(service.dispatcher.in_flight(), 0);
        assert_eq!(service.reporting.retained_bytes, 0);
    }
}


#[test]
fn actual_redirect_status_family_keeps_report_provenance_through_method_rewrite() {
    for status in [301, 302, 303, 307, 308] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let creation = HttpUrl::parse(&format!("http://{}/creation", listener.local_addr().unwrap())).unwrap();
        let endpoint = HttpUrl::parse(&format!("http://{}/report", listener.local_addr().unwrap())).unwrap();
        let final_url = endpoint.resolve("/final").unwrap();
        let (tx, rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            for final_hop in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                tx.send(read_request(&mut stream)).unwrap();
                let response = if final_hop {
                    "HTTP/1.1 204 Accepted\r\nConnection: close\r\n\r\n".to_owned()
                } else {
                    format!("HTTP/1.1 {status} Redirect\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let mut service = NetworkService::new(DispatchLimits::default());
        let mut client = receiving_client(&creation, &creation.serialize());
        client.origin = Origin::opaque();
        let owner = client.origin.clone();
        let handle = service.register_reporting_client_response(&client, &creation,
            &headers(&format!("csp=\"{}\"", endpoint.serialize()))).unwrap();
        queue_kind(&mut service, handle, "test", &creation.serialize());
        assert!(service.forget_reporting_document(handle));
        let completed = run(&mut service, 1);
        peer.join().unwrap();
        assert_eq!(completed[0].outcome, ReportOutcome::Response {status:204,accepted:true});
        assert_eq!(completed[0].document, handle);
        let receipt = completed[0].wire();
        assert_eq!(receipt.report_url(), creation.serialize());
        assert_eq!(receipt.owner_origin(), &owner);
        assert_eq!(receipt.request_origin(), creation.origin());
        assert_eq!(receipt.registered_endpoint(), &endpoint);
        assert_eq!(receipt.url_list(), &[endpoint, final_url.clone()]);
        assert_eq!(receipt.terminal_url(), &final_url);
        assert!(receipt.terminal_includes_credentials());
        assert!(!receipt.terminal_is_preflight());
        let first = rx.recv().unwrap();
        let final_wire = rx.recv().unwrap();
        assert!(first.starts_with("POST /report "));
        let rewrites = matches!(status, 301 | 302 | 303);
        assert_eq!(final_wire.starts_with("GET /final "), rewrites);
        assert_eq!(final_wire.starts_with("POST /final "), !rewrites);
        assert_eq!(final_wire.contains(&quote(&creation.serialize())), !rewrites);
        assert_eq!(final_wire.to_ascii_lowercase().contains("\r\ncontent-type: application/reports+json"), !rewrites);
        assert!(!final_wire.to_ascii_lowercase().contains("\r\nreferer:"));
        assert_eq!(service.reporting.retained_bytes, 0);
    }
}
