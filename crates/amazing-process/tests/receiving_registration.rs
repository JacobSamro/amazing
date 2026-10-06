//! Real child-process IPC + HTTP controls for server-local registration custody.
//! The test browser supplies explicit native global captures; no network URL
//! manufactures a Realm/Window/Document identity or a ReportingDocument.
use amazing_fetch::api::NavigableContext;
use amazing_fetch::cookies::CookieSite;
use amazing_fetch::reporting::NavigationEmbeddingElement;
use amazing_fetch::{
    ClientContext, CspDestination, CspPolicyList, Destination, EmbedderPolicy,
    EmbedderPolicyContainer, HttpUrl, NoCorsMediaState, Origin, ReceivingDocumentClient,
    ReferrerPolicy, SecurityContext,
};
use amazing_net::ParsedUrl;
use amazing_process::{network_v2::*, protocol::*};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

fn response(fields: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\n{fields}Content-Length: 2\r\nConnection: close\r\n\r\nok")
        .into_bytes()
}
fn server(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut requests = Vec::new();
        for bytes in responses {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("accept {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 64 * 1024);
                let mut b = [0];
                stream.read_exact(&mut b).unwrap();
                request.push(b[0]);
            }
            requests.push(String::from_utf8(request).unwrap());
            stream.write_all(&bytes).unwrap();
        }
        requests
    });
    (url, task)
}
fn session(id: u64) -> NetworkSession {
    NetworkSession::launch(
        std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child")),
        id,
        Duration::from_secs(10),
    )
    .unwrap()
}
std::thread_local! {
    static BROWSER_GLOBALS:std::cell::RefCell<std::collections::HashMap<u64,(amazing_js::Interpreter,amazing_dom::Dom,NativeReceivingIdentity)>>=Default::default();
}
fn identity(id: u64) -> NativeReceivingIdentity {
    BROWSER_GLOBALS.with(|globals| {
        let mut globals = globals.borrow_mut();
        if let Some((_, _, identity)) = globals.get(&id) {
            return *identity;
        }
        let interpreter = amazing_js::Interpreter::new();
        let mut dom = amazing_dom::Dom::new();
        let document = amazing_dom::NodeIdx(0);
        let generation = dom.node_generation(document).unwrap();
        let window = dom.create_window_event_target();
        dom.associate_document_window_event_target(document, generation, window)
            .unwrap();
        let identity = NativeReceivingIdentity {
            realm: interpreter.binding_current_realm(),
            window,
            document,
            generation,
        };
        globals.insert(id, (interpreter, dom, identity));
        identity
    })
}
fn context(origin: &Origin, ancestors: Vec<Origin>, policy: EmbedderPolicy) -> ClientContext {
    ClientContext {
        cookie_site: CookieSite::Origin(origin.clone()),
        partition_key: None,
        frame_ancestors: ancestors,
        security: SecurityContext {
            client_restricts_mixed_content: false,
            destination: Destination::Document,
            image_set_initiator: false,
            top_level_navigation: false,
            embedder_policy: policy,
            no_cors_media_state: NoCorsMediaState::NotMedia,
        },
        csp: CspPolicyList::default(),
        csp_destination: CspDestination::Frame,
        now_unix: 0,
    }
}
fn client(
    id: u64,
    url: &str,
    origin: Origin,
    ancestors: Vec<Origin>,
    policy: EmbedderPolicy,
) -> ReceivingDocumentClient<NativeReceivingIdentity> {
    let url = ParsedUrl::parse(url).unwrap();
    ReceivingDocumentClient {
        identity: identity(id),
        creation_url: url.clone(),
        document_url: url.clone(),
        base_url: url,
        csp_self_origin: origin.clone(),
        context: context(&origin, ancestors, policy),
        origin,
        referrer_policy: ReferrerPolicy::StrictOriginWhenCrossOrigin,
    }
}
fn parent(origin: &Origin) -> NavigableContext {
    NavigableContext {
        top_level_origin: origin.clone(),
        parent_origin: origin.clone(),
        ancestor_origins: vec![origin.clone()],
        sandbox_flags: 0,
        frame_csp: CspPolicyList::default(),
        referrer: format!("{}/", origin.serialize()),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        now_unix: 0,
    }
}
fn complete(
    session: &NetworkSession,
    started: &NetworkStarted,
) -> (Option<NetworkHead>, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut head = None;
    loop {
        assert!(Instant::now() < deadline);
        match session.read(started, NETWORK_CHUNK).unwrap() {
            NetworkReply::Pending(()) => std::thread::yield_now(),
            NetworkReply::Head(v) => head = Some(v),
            NetworkReply::Body(_) => {}
            NetworkReply::Complete(v) => return (head, v.error),
        }
    }
}
fn navigation(url: &str, origin: &Origin) -> NetworkStart {
    let mut request = get_request(
        url,
        url,
        "document",
        1024,
        NetworkContext {
            origin: origin_to_wire(origin),
            cookie_site: WireCookieSite::Origin(origin_to_wire(origin)),
            top_level_origin: Some(origin_to_wire(origin)),
            ancestor_origins: Vec::new(),
            sandbox_flags: 0,
            now_unix: 0,
            restricts_mixed_content: false,
            image_set_initiator: false,
            top_level_navigation: true,
            embedder_policy: "unsafe-none".into(),
            no_cors_media_state: "none".into(),
            csp_headers: Vec::new(),
        },
    );
    request.mode = "navigate".into();
    request.redirect = "manual".into();
    request.credentials = "include".into();
    request
}
fn admitted_owner(
    session: &NetworkSession,
    id: u64,
    url: &str,
) -> (ReceivingGlobalHandle, Origin, NetworkStarted) {
    let origin = HttpUrl::parse(url).unwrap().origin().clone();
    let started = session
        .capture_receiving_navigation(navigation(url, &origin))
        .unwrap();
    let (head, error) = complete(session, &started);
    assert!(error.is_none());
    assert!(head.unwrap().navigation.is_some());
    let owner = client(
        id,
        url,
        origin.clone(),
        Vec::new(),
        EmbedderPolicy::RequireCorp,
    );
    let coep = EmbedderPolicyContainer {
        value: EmbedderPolicy::RequireCorp,
        reporting_endpoint: None,
        report_only_value: EmbedderPolicy::UnsafeNone,
        report_only_reporting_endpoint: None,
    };
    let handle = session
        .register_receiving_global(&owner, &coep, true, Some(&started))
        .unwrap();
    (handle, origin, started)
}
#[test]
fn accepted_head_is_retained_after_real_completion_and_registration_is_connection_bound() {
    let (url, http) = server(vec![response(
        "Cross-Origin-Embedder-Policy: require-corp\r\n",
    )]);
    let first = session(701);
    let (handle, origin, started) = admitted_owner(&first, 101, &format!("{url}/owner"));
    assert!(first.release_receiving_navigation(&started).unwrap());
    let other = session(702);
    assert!(other.poll_receiving_reports(&handle).is_err());
    let capture = client(
        101,
        &format!("{url}/owner"),
        origin.clone(),
        Vec::new(),
        EmbedderPolicy::RequireCorp,
    );
    let coep = EmbedderPolicyContainer {
        value: EmbedderPolicy::RequireCorp,
        reporting_endpoint: None,
        report_only_value: EmbedderPolicy::UnsafeNone,
        report_only_reporting_endpoint: None,
    };
    // Reuse preserves server reporting settings even after raw head release.
    let repeated = first
        .register_receiving_global(&capture, &coep, true, Some(&started))
        .unwrap();
    assert_eq!(repeated.identity(), handle.identity());
    assert!(first.retire_receiving_global(&handle).unwrap());
    assert!(!first.retire_receiving_global(&handle).unwrap());
    assert!(
        first
            .register_receiving_global(&capture, &coep, true, Some(&started))
            .is_err()
    );
    assert_eq!(http.join().unwrap().len(), 1);
}
#[test]
fn fresh_initial_is_explicit_and_never_a_failed_http_response_fallback() {
    let session = session(703);
    let origin = Origin::opaque();
    let initial = client(
        201,
        "about:blank",
        origin.clone(),
        Vec::new(),
        EmbedderPolicy::UnsafeNone,
    );
    let handle = session
        .register_receiving_global(&initial, &EmbedderPolicyContainer::default(), false, None)
        .unwrap();
    assert!(session.poll_receiving_reports(&handle).unwrap().is_empty());
    let http = client(
        202,
        "http://127.0.0.1:43123/never-requested",
        origin,
        Vec::new(),
        EmbedderPolicy::UnsafeNone,
    );
    assert!(
        session
            .register_receiving_global(&http, &EmbedderPolicyContainer::default(), false, None)
            .is_err()
    );
}
#[test]
fn source_and_embedding_origin_are_distinct_and_retired_owner_loan_survives() {
    for corp in ["same-origin", "cross-origin"] {
        let (owner_url, owner_http) = server(vec![response(
            "Cross-Origin-Embedder-Policy: require-corp\r\n",
        )]);
        let (target_url, target_http) = server(vec![response(&format!(
            "Cross-Origin-Resource-Policy: {corp}\r\n"
        ))]);
        let session = session(if corp == "same-origin" { 704 } else { 705 });
        let (owner, origin, started) = admitted_owner(&session, 301, &format!("{owner_url}/owner"));
        let source_origin = HttpUrl::parse(&target_url).unwrap().origin().clone();
        let source_client = client(
            401,
            "about:blank",
            source_origin,
            vec![origin.clone()],
            EmbedderPolicy::UnsafeNone,
        );
        let source = session
            .register_receiving_global(
                &source_client,
                &EmbedderPolicyContainer::default(),
                false,
                None,
            )
            .unwrap();
        let plan = session
            .prepare_receiving_owner(
                &owner,
                ReceivingFrameIdentity {
                    frame: 31,
                    generation: 2,
                },
                &parent(&origin),
                NavigationEmbeddingElement::Iframe,
            )
            .unwrap();
        assert!(session.retire_receiving_global(&owner).unwrap());
        let navigation = session
            .start_receiving_navigation(
                &source,
                plan,
                &HttpUrl::parse(&format!("{target_url}/child")).unwrap(),
                1024,
            )
            .unwrap();
        let (_, error) = complete(&session, &navigation);
        assert_eq!(error.is_none(), corp == "cross-origin");
        assert!(session.release_receiving_navigation(&navigation).unwrap());
        assert!(session.release_receiving_navigation(&started).unwrap());
        assert_eq!(owner_http.join().unwrap().len(), 1);
        assert_eq!(target_http.join().unwrap().len(), 1);
    }
}
#[test]
fn captured_owner_ancestry_frame_and_full_policy_are_validated() {
    let (url, http) = server(vec![response(
        "Cross-Origin-Embedder-Policy: require-corp\r\n",
    )]);
    let session = session(706);
    let (owner, origin, started) = admitted_owner(&session, 501, &format!("{url}/owner"));
    let mut wrong = parent(&origin);
    wrong.parent_origin = Origin::opaque();
    assert!(
        session
            .prepare_receiving_owner(
                &owner,
                ReceivingFrameIdentity {
                    frame: 41,
                    generation: 1
                },
                &wrong,
                NavigationEmbeddingElement::Frame
            )
            .is_err()
    );
    assert!(
        session
            .prepare_receiving_owner(
                &owner,
                ReceivingFrameIdentity {
                    frame: 0,
                    generation: 1
                },
                &parent(&origin),
                NavigationEmbeddingElement::Frame
            )
            .is_err()
    );
    let plan = session
        .prepare_receiving_owner(
            &owner,
            ReceivingFrameIdentity {
                frame: 41,
                generation: 1,
            },
            &parent(&origin),
            NavigationEmbeddingElement::Frame,
        )
        .unwrap();
    assert!(session.release_receiving_owner(plan).unwrap());
    assert!(session.release_receiving_navigation(&started).unwrap());
    assert_eq!(http.join().unwrap().len(), 1);
}

#[test]
fn manual_redirect_uses_retained_private_head_and_admitted_loan_after_retirement() {
    let (owner_url, owner_http) = server(vec![response(
        "Cross-Origin-Embedder-Policy: require-corp\r\n",
    )]);
    let redirect=b"HTTP/1.1 302 Found\r\nLocation: /final\r\nCross-Origin-Resource-Policy: cross-origin\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (target_url, target_http) = server(vec![
        redirect,
        response("Cross-Origin-Resource-Policy: cross-origin\r\n"),
    ]);
    let session = session(707);
    let (owner, origin, registration) =
        admitted_owner(&session, 601, &format!("{owner_url}/owner"));
    let frame = ReceivingFrameIdentity {
        frame: 51,
        generation: 3,
    };
    let plan = session
        .prepare_receiving_owner(
            &owner,
            frame.clone(),
            &parent(&origin),
            NavigationEmbeddingElement::Iframe,
        )
        .unwrap();
    let first = session
        .start_receiving_navigation(
            &owner,
            plan,
            &HttpUrl::parse(&format!("{target_url}/redirect")).unwrap(),
            1024,
        )
        .unwrap();
    let (head, error) = complete(&session, &first);
    assert!(error.is_none());
    let head = head.unwrap();
    assert_eq!(head.navigation.unwrap().status, 302);
    assert!(
        session
            .continue_receiving_navigation(
                &first,
                &owner,
                ReceivingFrameIdentity {
                    frame: 51,
                    generation: 4
                },
                1024
            )
            .is_err()
    );
    assert!(session.retire_receiving_global(&owner).unwrap());
    let final_request = session
        .continue_receiving_navigation(&first, &owner, frame.clone(), 1024)
        .unwrap();
    assert!(
        session
            .continue_receiving_navigation(&first, &owner, frame, 1024)
            .is_err()
    );
    let (head, error) = complete(&session, &final_request);
    assert!(error.is_none());
    let head = head.unwrap();
    assert_eq!(head.url, format!("{target_url}/final"));
    assert_eq!(
        head.url_list,
        vec![
            format!("{target_url}/redirect"),
            format!("{target_url}/final")
        ]
    );
    assert!(session.release_receiving_navigation(&first).unwrap());
    assert!(
        session
            .release_receiving_navigation(&final_request)
            .unwrap()
    );
    assert!(session.release_receiving_navigation(&registration).unwrap());
    let requests = target_http.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].to_ascii_lowercase().contains("\r\nreferer:"));
    assert_eq!(owner_http.join().unwrap().len(), 1);
}

#[test]
fn coep_observer_receipts_keep_original_global_and_report_only_disposition() {
    let fields = "Cross-Origin-Embedder-Policy: require-corp\r\nCross-Origin-Embedder-Policy-Report-Only: require-corp; report-to=\"reports\"\r\n";
    let (owner_url, owner_http) = server(vec![response(fields)]);
    let (target_url, target_http) = server(vec![response("")]);
    let session = session(708);
    let owner_url = format!("{owner_url}/owner");
    let origin = HttpUrl::parse(&owner_url).unwrap().origin().clone();
    let first = session
        .capture_receiving_navigation(navigation(&owner_url, &origin))
        .unwrap();
    assert!(complete(&session, &first).1.is_none());
    let captured = client(
        701,
        &owner_url,
        origin.clone(),
        Vec::new(),
        EmbedderPolicy::RequireCorp,
    );
    let coep = EmbedderPolicyContainer {
        value: EmbedderPolicy::RequireCorp,
        reporting_endpoint: None,
        report_only_value: EmbedderPolicy::RequireCorp,
        report_only_reporting_endpoint: Some("reports".into()),
    };
    let owner = session
        .register_receiving_global(&captured, &coep, true, Some(&first))
        .unwrap();
    let plan = session
        .prepare_receiving_owner(
            &owner,
            ReceivingFrameIdentity {
                frame: 61,
                generation: 1,
            },
            &parent(&origin),
            NavigationEmbeddingElement::Frame,
        )
        .unwrap();
    let request = session
        .start_receiving_navigation(
            &owner,
            plan,
            &HttpUrl::parse(&format!("{target_url}/denied")).unwrap(),
            1024,
        )
        .unwrap();
    assert!(session.retire_receiving_global(&owner).unwrap());
    assert!(complete(&session, &request).1.is_some());
    let reports = session.poll_receiving_reports(&owner).unwrap();
    let observers = reports
        .iter()
        .filter_map(|event| match event {
            ReceivingReportEvent::Observer(report) => Some(report),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(observers.len(), 2);
    let mut dispositions = Vec::new();
    for report in observers {
        assert_eq!(String::from_utf16(&report.report_type).unwrap(), "coep");
        assert_eq!(String::from_utf16(&report.report_url).unwrap(), owner_url);
        assert_eq!(report.request_id, request.id);
        assert!(report.delivery_id.is_none());
        assert!(report.delivery_error.is_some());
        let field = report
            .body
            .iter()
            .find(|field| String::from_utf16(&field.name).unwrap() == "disposition")
            .unwrap();
        dispositions.push(String::from_utf16(&field.value).unwrap());
    }
    dispositions.sort();
    assert_eq!(dispositions, vec!["enforce", "reporting"]);
    assert!(session.poll_receiving_reports(&owner).unwrap().is_empty());
    assert_eq!(owner_http.join().unwrap().len(), 1);
    assert_eq!(target_http.join().unwrap().len(), 1);
}

#[test]
fn global_identity_cannot_replace_accepted_creation_origin_or_full_coep() {
    let (url, http) = server(vec![response(
        "Cross-Origin-Embedder-Policy: require-corp\r\n",
    )]);
    let session = session(709);
    let (_, origin, started) = admitted_owner(&session, 801, &format!("{url}/owner"));
    let coep = EmbedderPolicyContainer {
        value: EmbedderPolicy::RequireCorp,
        reporting_endpoint: None,
        report_only_value: EmbedderPolicy::UnsafeNone,
        report_only_reporting_endpoint: None,
    };
    let original = client(
        801,
        &format!("{url}/owner"),
        origin,
        Vec::new(),
        EmbedderPolicy::RequireCorp,
    );
    let mut replacement = original.clone();
    replacement.origin = Origin::opaque();
    assert!(
        session
            .register_receiving_global(&replacement, &coep, true, Some(&started))
            .is_err()
    );
    let mut replacement = original.clone();
    replacement.creation_url = ParsedUrl::parse(&format!("{url}/other")).unwrap();
    assert!(
        session
            .register_receiving_global(&replacement, &coep, true, Some(&started))
            .is_err()
    );
    let mut replacement = coep.clone();
    replacement.report_only_value = EmbedderPolicy::RequireCorp;
    assert!(
        session
            .register_receiving_global(&original, &replacement, true, Some(&started))
            .is_err()
    );
    let new = client(
        802,
        &format!("{url}/owner"),
        Origin::opaque(),
        Vec::new(),
        EmbedderPolicy::RequireCorp,
    );
    assert!(
        session
            .register_receiving_global(&new, &coep, false, Some(&started))
            .is_err()
    );
    assert_eq!(http.join().unwrap().len(), 1);
}

#[test]
fn full_native_context_units_are_carried_without_url_or_scalar_policy_substitution() {
    use amazing_fetch::Site;
    use amazing_fetch::csp::{CspDirective, CspDisposition, CspPolicy, CspPolicySource};
    use amazing_ipc::codec::{decode, encode};
    let origin = Origin::opaque();
    let mut native = client(
        901,
        "about:srcdoc",
        origin.clone(),
        vec![Origin::opaque(), Origin::opaque()],
        EmbedderPolicy::Credentialless,
    );
    native.context.partition_key = Some(Site::Opaque(origin.opaque_id().unwrap()));
    native.context.cookie_site = CookieSite::CrossSite;
    native.context.security.client_restricts_mixed_content = true;
    native.context.security.destination = Destination::SharedWorker;
    native.context.security.image_set_initiator = true;
    native.context.security.no_cors_media_state = NoCorsMediaState::Subsequent;
    native.context.csp_destination = CspDestination::ScriptElement;
    let units = vec![
        0x64, 0x65, 0x66, 0x61, 0x75, 0x6c, 0x74, 0x2d, 0x73, 0x72, 0x63, 0x20, 0xD800,
    ];
    native.context.csp = CspPolicyList {
        policies: vec![CspPolicy {
            source: CspPolicySource::Meta,
            disposition: CspDisposition::Enforce,
            original_policy: String::from_utf16_lossy(&units),
            original_policy_units: units.clone(),
            directives: vec![CspDirective {
                name: "default-src".into(),
                sources: vec!["'self'".into()],
            }],
        }],
    };
    let capture = ReceivingClientCapture::from_native(&native, 1);
    let roundtrip: ReceivingClientCapture = decode(&encode(&capture).unwrap()).unwrap();
    assert_eq!(roundtrip, capture);
    assert_eq!(roundtrip.context.csp[0].original_policy_units, units);
    assert_eq!(roundtrip.context.embedder_policy, "credentialless");
    assert_eq!(roundtrip.context.destination, "sharedworker");
    assert_eq!(roundtrip.context.csp_destination, "script-element");
    assert_eq!(
        roundtrip.context.partition_key,
        Some(ReceivingSite::Opaque(origin.opaque_id().unwrap()))
    );
}

fn csp_source(
    session: &NetworkSession,
    id: u64,
    url: &str,
    enforced: bool,
) -> ReceivingGlobalHandle {
    let origin = HttpUrl::parse(url).unwrap().origin().clone();
    let accepted = session
        .capture_receiving_navigation(navigation(url, &origin))
        .unwrap();
    assert!(complete(session, &accepted).1.is_none());
    let mut source = client(id, url, origin, Vec::new(), EmbedderPolicy::RequireCorp);
    let mut fields = amazing_fetch::HeaderList::default();
    fields
        .append(
            if enforced {
                "content-security-policy"
            } else {
                "content-security-policy-report-only"
            },
            "frame-src 'none'",
        )
        .unwrap();
    source.context.csp = CspPolicyList::from_headers(&fields).unwrap();
    session
        .register_receiving_global(
            &source,
            &EmbedderPolicyContainer {
                value: EmbedderPolicy::RequireCorp,
                reporting_endpoint: None,
                report_only_value: EmbedderPolicy::UnsafeNone,
                report_only_reporting_endpoint: None,
            },
            true,
            Some(&accepted),
        )
        .unwrap()
}
fn csp_owner(
    session: &NetworkSession,
    source: &ReceivingGlobalHandle,
    origin: &Origin,
    frame: u64,
) -> ReceivingOwnerHandle {
    session
        .prepare_receiving_owner(
            source,
            ReceivingFrameIdentity {
                frame,
                generation: 1,
            },
            &parent(origin),
            NavigationEmbeddingElement::Iframe,
        )
        .unwrap()
}
fn csp_field<'a>(snapshot: &'a ReceivingCspSnapshot, name: &str) -> &'a ReceivingCspValue {
    &snapshot
        .body
        .iter()
        .find(|field| field.name == name.encode_utf16().collect::<Vec<_>>())
        .unwrap()
        .value
}
fn csp_snapshots(events: &[ReceivingReportEvent]) -> Vec<&ReceivingCspSnapshot> {
    events
        .iter()
        .filter_map(|event| match event {
            ReceivingReportEvent::Csp(value) => Some(value),
            _ => None,
        })
        .collect()
}
#[test]
fn csp_admission_failure_has_no_invented_service_id_and_keeps_original_document_snapshot() {
    let (base, http) = server(vec![response(
        "Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy: frame-src 'none'\r\n",
    )]);
    let session = session(730);
    let source_url = format!("{base}/protecting?original");
    let source = csp_source(&session, 931, &source_url, true);
    let origin = HttpUrl::parse(&base).unwrap().origin().clone();
    let target = HttpUrl::parse(&format!("{base}/blocked")).unwrap();
    let failed = match session
        .start_receiving_navigation_outcome(
            &source,
            csp_owner(&session, &source, &origin, 731),
            &target,
            1024,
        )
        .unwrap()
    {
        ReceivingNavigationOutcome::Failed(value) => value,
        ReceivingNavigationOutcome::Started(_) => panic!("enforced admission admitted a socket"),
    };
    assert!(failed.request.is_none());
    assert!(failed.checked);
    assert_eq!(failed.url_list, vec![target.serialize()]);
    let events = session.poll_receiving_reports(&source).unwrap();
    let snapshots = csp_snapshots(&events);
    assert_eq!(snapshots.len(), 1);
    let report = snapshots[0];
    assert!(report.request.is_none());
    assert!(report.element.is_none());
    assert!(report.location.is_none());
    assert_eq!(
        report.source.identity.generation,
        source.identity().generation
    );
    assert_eq!(report.phase, "pre-request");
    assert_eq!(report.effective_directive, "frame-src");
    assert_eq!(
        csp_field(report, "documentURL"),
        &ReceivingCspValue::String(source_url.encode_utf16().collect())
    );
    assert_eq!(
        csp_field(report, "statusCode"),
        &ReceivingCspValue::NumberBits(200f64.to_bits())
    );
    assert_eq!(
        csp_field(report, "sourceFile"),
        &ReceivingCspValue::Null(())
    );
    assert_eq!(
        csp_field(report, "lineNumber"),
        &ReceivingCspValue::Null(())
    );
    let plan = report.prepared_report.unwrap();
    assert!(session.retire_receiving_global(&source).unwrap());
    let delivery = session.finish_receiving_csp_report(&source, plan).unwrap();
    assert_eq!(delivery.error.as_deref(), Some("NoEndpoint"));
    assert!(session.finish_receiving_csp_report(&source, plan).is_err());
    assert_eq!(http.join().unwrap().len(), 1);
}
#[test]
fn csp_report_only_response_snapshots_keep_protecting_status_not_target_status() {
    let (base, http) = server(vec![
        response(
            "Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy-Report-Only: frame-src 'none'\r\n",
        ),
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno".to_vec(),
    ]);
    let session = session(732);
    let source_url = format!("{base}/source");
    let source = csp_source(&session, 932, &source_url, false);
    let origin = HttpUrl::parse(&base).unwrap().origin().clone();
    let request = match session
        .start_receiving_navigation_outcome(
            &source,
            csp_owner(&session, &source, &origin, 732),
            &HttpUrl::parse(&format!("{base}/not-found")).unwrap(),
            1024,
        )
        .unwrap()
    {
        ReceivingNavigationOutcome::Started(value) => value,
        _ => panic!("report-only blocked admission"),
    };
    let (head, error) = complete(&session, &request);
    assert!(error.is_none());
    assert_eq!(head.unwrap().status, 404);
    let events = session.poll_receiving_reports(&source).unwrap();
    let reports = csp_snapshots(&events);
    assert_eq!(reports.len(), 2);
    for report in reports {
        assert_eq!(report.request.as_ref(), Some(&request));
        assert_eq!(
            csp_field(report, "statusCode"),
            &ReceivingCspValue::NumberBits(200f64.to_bits())
        );
        assert_eq!(
            csp_field(report, "disposition"),
            &ReceivingCspValue::String("report".encode_utf16().collect())
        );
        assert!(report.element.is_none() && report.location.is_none());
        let delivery = session
            .finish_receiving_csp_report(&source, report.prepared_report.unwrap())
            .unwrap();
        assert_eq!(delivery.error.as_deref(), Some("NoEndpoint"));
    }
    assert!(session.poll_receiving_reports(&source).unwrap().is_empty());
    assert_eq!(http.join().unwrap().len(), 2);
}
#[test]
fn csp_connection_and_framing_failures_preserve_actual_checked_vs_accepted_progress() {
    for framing in [false, true] {
        let mut responses = vec![response(
            "Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy-Report-Only: frame-src 'none'\r\n",
        )];
        if framing {
            responses.push(
                b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort".to_vec(),
            );
        }
        let (base, http) = server(responses);
        let session = session(733 + u64::from(framing));
        let source = csp_source(
            &session,
            933 + u64::from(framing),
            &format!("{base}/source"),
            false,
        );
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = closed.local_addr().unwrap();
        drop(closed);
        let target = if framing {
            format!("{base}/truncated")
        } else {
            format!("http://{address}/refused")
        };
        let origin = HttpUrl::parse(&base).unwrap().origin().clone();
        let request = session
            .start_receiving_navigation(
                &source,
                csp_owner(&session, &source, &origin, 733),
                &HttpUrl::parse(&target).unwrap(),
                1024,
            )
            .unwrap();
        assert!(complete(&session, &request).1.is_some());
        let events = session.poll_receiving_reports(&source).unwrap();
        let reports = csp_snapshots(&events);
        assert_eq!(reports.len(), if framing { 2 } else { 1 });
        let failure = events
            .iter()
            .find_map(|event| match event {
                ReceivingReportEvent::NavigationFailed(value) => Some(value),
                _ => None,
            })
            .unwrap();
        assert_eq!(failure.request.as_ref(), Some(&request));
        assert!(failure.checked);
        assert_eq!(failure.url_list, vec![target]);
        assert_eq!(http.join().unwrap().len(), if framing { 2 } else { 1 });
    }
}
#[test]
fn csp_manual_continuation_deduplicates_native_prefix_without_rechecking_redirect_url() {
    let (base,http)=server(vec![response("Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy-Report-Only: frame-src 'none'\r\n"),
        b"HTTP/1.1 302 Found\r\nLocation: /last\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),response("")]);
    let session = session(735);
    let source = csp_source(&session, 935, &format!("{base}/source"), false);
    let origin = HttpUrl::parse(&base).unwrap().origin().clone();
    let frame = ReceivingFrameIdentity {
        frame: 735,
        generation: 1,
    };
    let first = session
        .start_receiving_navigation(
            &source,
            csp_owner(&session, &source, &origin, 735),
            &HttpUrl::parse(&format!("{base}/first")).unwrap(),
            1024,
        )
        .unwrap();
    assert!(complete(&session, &first).1.is_none());
    let before = session.poll_receiving_reports(&source).unwrap();
    assert_eq!(csp_snapshots(&before).len(), 2);
    let second = session
        .continue_receiving_navigation(&first, &source, frame, 1024)
        .unwrap();
    assert!(complete(&session, &second).1.is_none());
    let after = session.poll_receiving_reports(&source).unwrap();
    let reports = csp_snapshots(&after);
    assert_eq!(reports.len(), 2);
    for report in reports {
        assert_eq!(
            report.url_list,
            vec![format!("{base}/first"), format!("{base}/last")]
        );
        assert_eq!(report.request.as_ref(), Some(&second));
    }
    let requests = http.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(!requests[2].to_ascii_lowercase().contains("\r\nreferer:"));
}

#[test]
fn csp_source_settings_loan_survives_retirement_before_response_and_manual_continuation() {
    for redirect in [false, true] {
        let mut responses = vec![response(
            "Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy-Report-Only: frame-src 'none'\r\n",
        )];
        if redirect {
            responses.push(b"HTTP/1.1 302 Found\r\nLocation: /last\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
            responses.push(response(""));
        } else {
            responses.push(response(""));
        }
        let (base, http) = server(responses);
        let session = session(740 + u64::from(redirect));
        let source = csp_source(
            &session,
            940 + u64::from(redirect),
            &format!("{base}/source"),
            false,
        );
        let origin = HttpUrl::parse(&base).unwrap().origin().clone();
        let frame = ReceivingFrameIdentity {
            frame: 740,
            generation: 1,
        };
        let first = session
            .start_receiving_navigation(
                &source,
                csp_owner(&session, &source, &origin, 740),
                &HttpUrl::parse(&format!("{base}/first")).unwrap(),
                1024,
            )
            .unwrap();
        // The loan was captured at initial admission. Retirement precedes the
        // response's post-request policy receipt and every continuation receipt.
        assert!(session.retire_receiving_global(&source).unwrap());
        assert!(complete(&session, &first).1.is_none());
        let mut all = session.poll_receiving_reports(&source).unwrap();
        let terminal = if redirect {
            let second = session
                .continue_receiving_navigation(&first, &source, frame, 1024)
                .unwrap();
            assert!(session.release_receiving_navigation(&first).unwrap());
            assert!(complete(&session, &second).1.is_none());
            all.extend(session.poll_receiving_reports(&source).unwrap());
            second
        } else {
            first
        };
        let reports = csp_snapshots(&all);
        assert_eq!(reports.len(), if redirect { 4 } else { 2 });
        for report in reports {
            assert!(report.delivery_error.is_none());
            assert_eq!(
                csp_field(report, "statusCode"),
                &ReceivingCspValue::NumberBits(200f64.to_bits())
            );
            let plan = report.prepared_report.unwrap();
            let delivery = session.finish_receiving_csp_report(&source, plan).unwrap();
            assert_eq!(delivery.error.as_deref(), Some("NoEndpoint"));
            assert!(session.finish_receiving_csp_report(&source, plan).is_err());
        }
        assert!(session.release_receiving_navigation(&terminal).unwrap());
        assert!(!session.release_receiving_navigation(&terminal).unwrap());
        assert_eq!(http.join().unwrap().len(), if redirect { 3 } else { 2 });
    }
}


/// Real original-global admission, retired capability, POST bytes and terminal
/// IPC completion. No synthetic ReportFinished or URL-derived global identity.
#[test]
fn report_completion_preserves_native_wire_observations_after_original_retirement() {
    use amazing_ipc::codec::{decode, encode};
    for modern in [false, true] {
        for (terminal, redirect) in [(200u16,None), (410,None), (599,None),
            (200,Some(301u16)),(200,Some(302)),(200,Some(303)),(200,Some(307)),(200,Some(308))] {
            if !modern && redirect.is_some() { continue; }
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let policy = if modern { "frame-src 'none'; report-to reports" }
                else { "frame-src 'none'; report-uri /collector" };
            let fields = format!("Cross-Origin-Embedder-Policy: require-corp\r\nContent-Security-Policy: {policy}\r\nReporting-Endpoints: reports=\"/collector\"\r\n");
            let head = response(&fields);
            let peer = std::thread::spawn(move || {
                let mut requests = Vec::new();
                for index in 0..(3 + usize::from(redirect.is_some())) {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                    let mut headers = Vec::new();
                    while !headers.ends_with(b"\r\n\r\n") {
                        assert!(headers.len() < 64*1024);
                        let mut byte = [0]; stream.read_exact(&mut byte).unwrap(); headers.push(byte[0]);
                    }
                    let headers = String::from_utf8(headers).unwrap();
                    let size = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).unwrap_or(0);
                    let mut body = vec![0; size]; stream.read_exact(&mut body).unwrap();
                    requests.push((headers, body));
                    if index == 0 { stream.write_all(&head).unwrap(); }
                    else if index == 1 && redirect.is_some() {
                        stream.write_all(format!("HTTP/1.1 {} Redirect\r\nLocation: /terminal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", redirect.unwrap()).as_bytes()).unwrap();
                    }
                    else if index == 1 + usize::from(redirect.is_some()) {
                        if terminal == 599 {
                            // Genuine incomplete framing, rather than a fabricated error event.
                            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nx").unwrap();
                        } else {
                            stream.write_all(format!("HTTP/1.1 {terminal} Outcome\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
                        }
                    } else { stream.write_all(&response("")).unwrap(); }
                }
                requests
            });
            let session = session(1800 + u64::from(modern)*1000 + u64::from(terminal));
            let source_url = format!("{base}/original");
            let origin = HttpUrl::parse(&base).unwrap().origin().clone();
            let accepted = session.capture_receiving_navigation(navigation(&source_url, &origin)).unwrap();
            assert!(complete(&session, &accepted).1.is_none());
            let mut receiver = client(2800 + u64::from(modern)*1000 + u64::from(terminal), &source_url,
                origin.clone(), Vec::new(), EmbedderPolicy::RequireCorp);
            let mut policy_headers = amazing_fetch::HeaderList::default();
            policy_headers.append("content-security-policy", policy).unwrap();
            receiver.context.csp = CspPolicyList::from_headers(&policy_headers).unwrap();
            let container = EmbedderPolicyContainer { value: EmbedderPolicy::RequireCorp,
                reporting_endpoint: None, report_only_value: EmbedderPolicy::UnsafeNone,
                report_only_reporting_endpoint: None };
            let source = session.register_receiving_global(&receiver, &container, true, Some(&accepted)).unwrap();
            let target = HttpUrl::parse(&format!("{base}/blocked")).unwrap();
            let result = session.start_receiving_navigation_outcome(&source,
                csp_owner(&session, &source, &origin, 1800), &target, 1024).unwrap();
            assert!(matches!(result, ReceivingNavigationOutcome::Failed(_)));
            let events = session.poll_receiving_reports(&source).unwrap();
            let snapshots = csp_snapshots(&events); assert_eq!(snapshots.len(), 1);
            let plan = snapshots[0].prepared_report.unwrap();
            assert!(session.retire_receiving_global(&source).unwrap());
            let admitted = session.finish_receiving_csp_report(&source, plan).unwrap();
            assert!(admitted.error.is_none());
            assert!(session.finish_receiving_csp_report(&source, plan).is_err());
            let deadline = Instant::now() + Duration::from_secs(10);
            let completion = loop {
                assert!(Instant::now() < deadline);
                let events = session.poll_receiving_reports(&source).unwrap();
                if let Some(value) = events.into_iter().find_map(|event| match event {
                    ReceivingReportEvent::Completion(value) => Some(value), _ => None,
                }) { break value; }
                std::thread::yield_now();
            };
            assert_eq!(completion.wire.version, 1);
            assert_eq!(completion.wire.report_url, source_url);
            assert_eq!(completion.wire.owner_origin, origin.serialize());
            assert_eq!(completion.wire.request_origin, origin.serialize());
            assert_eq!(completion.wire.registered_endpoint, format!("{base}/collector"));
            let expected_lineage = if redirect.is_some() { vec![format!("{base}/collector"),format!("{base}/terminal")] }
                else { vec![format!("{base}/collector")] };
            assert_eq!(completion.wire.url_list, expected_lineage);
            assert_eq!(completion.wire.terminal_url, format!("{base}/{}",if redirect.is_some() {"terminal"} else {"collector"}));
            assert!(!completion.wire.terminal_is_preflight);
            assert!(completion.wire.terminal_includes_credentials);
            assert_eq!(completion.route, if modern { "reporting-api" } else { "legacy-csp" });
            if terminal == 599 { assert!(completion.error.is_some()); assert!(!completion.accepted); }
            else {
                assert_eq!(completion.status, Some(terminal));
                assert_eq!(completion.accepted, !modern || terminal == 200);
                assert!(completion.error.is_none());
            }
            let bytes = encode(&completion).unwrap();
            assert_eq!(decode::<ReceivingReportCompletion>(&bytes).unwrap(), completion);
            assert!(decode::<ReceivingReportCompletion>(&bytes[..bytes.len()-1]).is_err());
            assert!(session.poll_receiving_reports(&source).unwrap().is_empty());
            // An unrelated genuine sibling request proves the old completion
            // did not poison the process session or transfer endpoint authority.
            let sibling = session.capture_receiving_navigation(navigation(&format!("{base}/sibling"), &origin)).unwrap();
            assert!(complete(&session, &sibling).1.is_none());
            let requests = peer.join().unwrap(); assert_eq!(requests.len(), 3 + usize::from(redirect.is_some()));
            assert!(requests[1].0.starts_with("POST /collector "));
            assert!(requests[1].0.to_ascii_lowercase().contains(if modern {
                "content-type: application/reports+json" } else { "content-type: application/csp-report" }));
            assert!(!requests[1].1.is_empty());
            if let Some(status) = redirect {
                if matches!(status,301|302|303) {
                    assert!(requests[2].0.starts_with("GET /terminal "));
                    assert!(requests[2].1.is_empty());
                } else {
                    assert!(requests[2].0.starts_with("POST /terminal "));
                    assert_eq!(requests[2].1,requests[1].1);
                }
            }
            assert!(requests[2 + usize::from(redirect.is_some())].0.starts_with("GET /sibling "));
            assert!(session.poll_receiving_reports(&source).unwrap().is_empty());
        }
    }
}
