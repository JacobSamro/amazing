//! Genuine native service peers for XHR streaming; driver/Window installation
//! is measured separately. No unvalidated head or synthetic transport receipt.
use amazing_fetch::{xhr::*,service::{NetworkService,ClientContext},
    ReceivingDocumentClient,HttpUrl,CspPolicyList,CspDestination,SecurityContext,
    Destination,EmbedderPolicy,NoCorsMediaState,ReferrerPolicy,cookies::CookieSite,api::BodyInput};
use amazing_net::dispatch::DispatchLimits;
use std::{io::{Read,Write},net::{TcpListener,TcpStream},sync::mpsc,time::{Duration,Instant}};
fn units(value:&str)->Vec<u16>{value.encode_utf16().collect()}
struct Decoder;
impl XhrTextDecoder for Decoder {
    fn decode(&self,bytes:&[u8],_:Option<&str>)->String {String::from_utf8_lossy(bytes).into_owned()}
}
fn client(url:&HttpUrl)->ReceivingDocumentClient<u64> {
    ReceivingDocumentClient{identity:7,creation_url:url.as_parsed().clone(),
        document_url:url.as_parsed().clone(),base_url:url.as_parsed().clone(),
        origin:url.origin().clone(),csp_self_origin:url.origin().clone(),
        referrer_policy:ReferrerPolicy::NoReferrer,
        context:ClientContext{cookie_site:CookieSite::Origin(url.origin().clone()),partition_key:Some(url.origin().site(&amazing_fetch::psl::PublicSuffixList)),
            frame_ancestors:vec![],now_unix:1_800_000_000,csp:CspPolicyList{policies:vec![]},csp_destination:CspDestination::None,
            security:SecurityContext{client_restricts_mixed_content:false,destination:Destination::Empty,
                image_set_initiator:false,top_level_navigation:false,embedder_policy:EmbedderPolicy::UnsafeNone,
                no_cors_media_state:NoCorsMediaState::NotMedia}}}
}
fn listener()->(TcpListener,HttpUrl) {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let url=HttpUrl::parse(&format!("http://{}/xhr",listener.local_addr().unwrap())).unwrap();
    (listener,url)
}
fn read_request(socket:&mut TcpStream)->Vec<u8> {
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut bytes=vec![];let mut b=[0;1];
    while !bytes.ends_with(b"\r\n\r\n") {socket.read_exact(&mut b).unwrap();bytes.push(b[0]);}
    bytes
}
fn pump(service:&mut NetworkService,xhr:&mut Xhr,pending:&mut XhrServiceRequest,now_ms:u64,
    until:impl Fn(&Xhr,&[XhrEvent])->bool)->Vec<XhrEvent> {
    let deadline=Instant::now()+Duration::from_secs(5);let mut events=vec![];
    loop {
        for event in service.step(64*1024,Instant::now()) {
            events.extend(xhr.on_service_event(pending,service,&event,now_ms).unwrap());
        }
        if until(xhr,&events) {return events;}
        assert!(Instant::now()<deadline,"native peer did not reach checkpoint");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn start(xhr:&mut Xhr,service:&mut NetworkService,url:&HttpUrl)->XhrServiceRequest {
    xhr.open(&units("GET"),&units(&url.serialize()),true,url,url.origin().clone()).unwrap();
    xhr.send(None,0).unwrap();
    xhr.start_service_received(service,&client(url),0,Instant::now()).unwrap()
}
#[test]
fn accepted_head_precedes_held_body_and_full_prefix_utf8_decode() {
    let (listener,url)=listener();let (release,wait)=mpsc::channel();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let request=read_request(&mut socket);
        assert!(!String::from_utf8(request).unwrap().to_ascii_lowercase().contains("referer:"));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain;charset=utf-8\r\nX-Native: yes\r\n\r\n").unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();socket.write_all(&[0xe2]).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();socket.write_all(&[0x82,0xac]).unwrap();
    });
    let mut service=NetworkService::new(DispatchLimits::default());let mut xhr=Xhr::default();
    let mut pending=start(&mut xhr,&mut service,&url);
    let events=pump(&mut service,&mut xhr,&mut pending,1,|xhr,_|xhr.state()==XhrState::HeadersReceived);
    assert_eq!(xhr.status(),200);assert_eq!(xhr.get_response_header(&units("x-native")).unwrap().as_deref(),Some("yes"));
    assert_eq!(events.iter().filter(|e|e.kind==XhrEventKind::ReadyStateChange).count(),1);
    assert_eq!(xhr.response_text(&Decoder).unwrap(),"");assert!(pending.is_active());
    release.send(()).unwrap();
    let events=pump(&mut service,&mut xhr,&mut pending,100,|_,events|events.iter().any(|e|e.kind==XhrEventKind::Progress&&e.loaded==1));
    assert_eq!(xhr.state(),XhrState::Loading);assert_eq!(xhr.response_text(&Decoder).unwrap(),"\u{fffd}");
    assert!(events.iter().any(|e|e.total==3&&e.length_computable));
    release.send(()).unwrap();
    let events=pump(&mut service,&mut xhr,&mut pending,101,|xhr,_|xhr.state()==XhrState::Done);
    assert_eq!(xhr.response_text(&Decoder).unwrap(),"€");assert!(!pending.is_active());
    assert_eq!(events.iter().filter(|e|e.kind==XhrEventKind::LoadEnd).count(),1);peer.join().unwrap();
}
#[test]
fn original_service_generation_abort_and_timeout_custody() {
    for timeout in [false,true] {
        let (listener,url)=listener();let (release,wait)=mpsc::channel();
        let peer=std::thread::spawn(move||{
            let (mut socket,_)=listener.accept().unwrap();read_request(&mut socket);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n").unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();let _=socket.write_all(b"late");
        });
        let mut service=NetworkService::new(DispatchLimits::default());let mut xhr=Xhr::default();
        let mut pending=start(&mut xhr,&mut service,&url);
        pump(&mut service,&mut xhr,&mut pending,1,|xhr,_|xhr.state()==XhrState::HeadersReceived);
        let mut foreign=NetworkService::new(DispatchLimits::default());
        assert!(xhr.check_service_timeout(&mut pending,&mut foreign,1).is_err());assert!(pending.is_active());
        if timeout {
            xhr.set_timeout(10).unwrap();
            let events=xhr.check_service_timeout(&mut pending,&mut service,10).unwrap().unwrap();
            assert!(events.iter().any(|e|e.kind==XhrEventKind::Timeout));
        } else {
            let events=xhr.abort();assert!(events.iter().any(|e|e.kind==XhrEventKind::Abort));
            assert!(pending.cancel(&mut service));assert!(!pending.cancel(&mut service));
        }
        assert_eq!(xhr.status(),0);assert_eq!(xhr.response_url(),"");assert!(!pending.is_active());
        release.send(()).unwrap();peer.join().unwrap();
        for event in service.step(64*1024,Instant::now()) {
            assert!(xhr.on_service_event(&mut pending,&mut service,&event,100).unwrap().is_empty());
        }
    }
}
#[test]
fn upload_end_is_native_socket_drain_before_response_head() {
    let (listener,url)=listener();let (received,seen)=mpsc::channel();let (release,wait)=mpsc::channel();
    let payload=vec![b'x';96*1024];let size=payload.len();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let request=read_request(&mut socket);
        assert!(String::from_utf8(request).unwrap().contains(&format!("Content-Length: {size}")));
        let mut body=vec![0;size];socket.read_exact(&mut body).unwrap();assert!(body.iter().all(|byte|*byte==b'x'));
        received.send(()).unwrap();wait.recv_timeout(Duration::from_secs(5)).unwrap();
        socket.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
    });
    let mut service=NetworkService::new(DispatchLimits::default());let mut xhr=Xhr::default();
    xhr.open(&units("POST"),&units(&url.serialize()),true,&url,url.origin().clone()).unwrap();
    xhr.set_upload_listeners(true);
    xhr.send(Some(BodyInput::Bytes(payload)),0).unwrap();
    let mut pending=xhr.start_service_received(&mut service,&client(&url),0,Instant::now()).unwrap();
    let events=pump(&mut service,&mut xhr,&mut pending,100,|_,events|events.iter().any(|e|e.target==XhrEventTarget::Upload&&e.kind==XhrEventKind::LoadEnd));
    assert_eq!(xhr.state(),XhrState::Opened);assert_eq!(xhr.status(),0);
    assert!(events.iter().any(|e|e.target==XhrEventTarget::Upload&&e.kind==XhrEventKind::Load&&e.loaded==size as u64));
    seen.recv_timeout(Duration::from_secs(5)).unwrap();release.send(()).unwrap();
    pump(&mut service,&mut xhr,&mut pending,101,|xhr,_|xhr.state()==XhrState::Done);
    assert_eq!(xhr.status(),204);assert!(xhr.document_source().is_none());peer.join().unwrap();
}

#[test]
fn synchronous_native_admission_and_transport_failures_clear_send_state_without_async_events() {
    for policy in [true, false] {
        let (_, url) = listener();
        let mut receiving = client(&url);
        let mut limits = DispatchLimits::default();
        if policy {
            let mut headers = amazing_fetch::HeaderList::default();
            headers
                .append("content-security-policy", "connect-src 'none'")
                .unwrap();
            receiving.context.csp = CspPolicyList::from_headers(&headers).unwrap();
        } else {
            limits.max_in_flight = 0;
        }
        let mut service = NetworkService::new(limits);
        let mut xhr = Xhr::default();
        xhr.open(
            &units("GET"),
            &units(&url.serialize()),
            false,
            &url,
            url.origin().clone(),
        )
        .unwrap();
        assert!(xhr.send(None, 0).unwrap().is_empty());
        xhr.prepare_service_received(&receiving).unwrap();
        let error = match xhr.start_buffered_service(&mut service, 0, Instant::now()) {
            Err(error) => error,
            Ok(_) => panic!("denied service admission succeeded"),
        };
        assert!(xhr.service_start_failed(error).is_err());
        assert_eq!(xhr.state(), XhrState::Done);
        assert!(!xhr.send_invoked());
        assert_eq!(xhr.status(), 0);
        assert_eq!(xhr.response_url(), "");
        assert_eq!(xhr.response_text(&Decoder).unwrap(), "");
        xhr.open(
            &units("GET"),
            &units(&url.serialize()),
            true,
            &url,
            url.origin().clone(),
        )
        .unwrap();
        assert!(xhr.send(None, 0).is_ok());
    }
    let (listener, url) = listener();
    let peer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nab")
            .unwrap();
    });
    let mut service = NetworkService::new(DispatchLimits::default());
    let mut xhr = Xhr::default();
    xhr.open(
        &units("GET"),
        &units(&url.serialize()),
        false,
        &url,
        url.origin().clone(),
    )
    .unwrap();
    xhr.send(None, 0).unwrap();
    xhr.prepare_service_received(&client(&url)).unwrap();
    let mut pending = xhr
        .start_buffered_service(&mut service, 0, Instant::now())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut failed = false;
    while !failed {
        for event in service.step(64 * 1024, Instant::now()) {
            match xhr.on_buffered_service_event(&mut pending, &mut service, event, 1) {
                Ok(events) => assert!(events.is_empty()),
                Err(_) => failed = true,
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(xhr.state(), XhrState::Done);
    assert!(!xhr.send_invoked());
    assert!(!pending.is_active());
    assert_eq!(xhr.status(), 0);
    peer.join().unwrap();
}
