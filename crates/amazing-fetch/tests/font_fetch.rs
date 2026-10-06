use amazing_fetch::{
    api::ApiError,
    service::{ClientContext, FontRequest, NetworkService, ServiceEvent, ServiceId},
    CredentialsMode, FetchError, HttpUrl, RequestMode,
};
use amazing_net::dispatch::DispatchLimits;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn server(response: String) -> (HttpUrl, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = HttpUrl::parse(&format!(
        "http://127.0.0.1:{}/style.css",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = [0; 4096];
                    let n = stream.read(&mut bytes).unwrap();
                    stream.write_all(response.as_bytes()).unwrap();
                    return String::from_utf8_lossy(&bytes[..n]).into_owned();
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("font fixture accept: {e}"),
            }
        }
    });
    (url, handle)
}
fn response(extra: &str, body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}
fn finish(
    service: &mut NetworkService,
    id: ServiceId,
) -> Result<amazing_fetch::api::ApiResponse, ApiError> {
    let start = Instant::now();
    for tick in 0..4000 {
        for event in service.step(8192, start + Duration::from_millis(tick)) {
            if let ServiceEvent::Finished { id: found, result } = event {
                assert_eq!(found, id);
                return result;
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("font fetch did not finish");
}
fn fetch(
    font_url: &str,
    stylesheet: &HttpUrl,
    document: &HttpUrl,
    max: usize,
) -> Result<amazing_fetch::api::ApiResponse, ApiError> {
    let font = FontRequest::new(font_url, stylesheet, document.origin().clone(), max).unwrap();
    assert_eq!(font.request().mode, RequestMode::Cors);
    assert_eq!(font.request().credentials, CredentialsMode::SameOrigin);
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_font(font, ClientContext::default()).unwrap();
    finish(&mut service, id)
}

#[test]
fn same_origin_font_allows_non_font_mime_and_credentials() {
    let (sheet, server) = server(response("", "font"));
    let mut result = fetch("font.woff2", &sheet, &sheet, 4).unwrap();
    assert_eq!(result.body.bytes().unwrap(), b"font");
    assert!(server.join().unwrap().starts_with("GET /font.woff2 "));
}
#[test]
fn cross_origin_font_requires_cors() {
    let sheet = HttpUrl::parse("http://127.0.0.1:39821/style.css").unwrap();
    let (allowed, yes) = server(response("Access-Control-Allow-Origin: *\r\n", "font"));
    let (denied, no) = server(response("", "font"));
    let mut result = fetch(
        &allowed.resolve("font.woff2").unwrap().serialize(),
        &sheet,
        &sheet,
        4,
    )
    .unwrap();
    assert_eq!(result.body.bytes().unwrap(), b"font");
    let seen = yes.join().unwrap();
    assert!(
        seen.to_ascii_lowercase()
            .contains("origin: http://127.0.0.1:"),
        "{seen}"
    );
    assert!(matches!(
        fetch(
            &denied.resolve("font.woff2").unwrap().serialize(),
            &sheet,
            &sheet,
            4
        ),
        Err(ApiError::Network(FetchError::Cors))
    ));
    no.join().unwrap();
}
#[test]
fn cross_origin_redirect_is_cors_tainted() {
    let (target, target_server) = server(response("Access-Control-Allow-Origin: *\r\n", "font"));
    let location = target.resolve("font.woff2").unwrap().serialize();
    let redirect = format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (sheet, source_server) = server(redirect);
    let mut result = fetch("redirect", &sheet, &sheet, 4).unwrap();
    assert_eq!(result.body.bytes().unwrap(), b"font");
    source_server.join().unwrap();
    target_server.join().unwrap();
}

#[test]
fn cross_origin_then_same_origin_stays_cors_tainted() {
    let (sheet, final_server) = server(response("", "font"));
    let location = sheet.resolve("font.woff2").unwrap().serialize();
    let redirect = format!("HTTP/1.1 302 Found\r\nAccess-Control-Allow-Origin: *\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (cross, first_server) = server(redirect);
    let url = cross.resolve("redirect").unwrap().serialize();
    assert!(matches!(
        fetch(&url, &sheet, &sheet, 4),
        Err(ApiError::Network(FetchError::Cors))
    ));
    first_server.join().unwrap();
    final_server.join().unwrap();
}
#[test]
fn font_body_limit_fails_before_body_buffering() {
    let (sheet, server) = server(response("", "too-big"));
    assert!(matches!(
        fetch("font.woff2", &sheet, &sheet, 3),
        Err(ApiError::Network(FetchError::ResponseTooLarge))
    ));
    server.join().unwrap();
}

#[test]
fn data_font_obeys_caller_bound() {
    let sheet = HttpUrl::parse("http://127.0.0.1:39821/style.css").unwrap();
    assert!(matches!(
        fetch("data:font/woff;base64,Zm9udA==", &sheet, &sheet, 3),
        Err(ApiError::Network(FetchError::ResponseTooLarge))
    ));
    let mut result = fetch("data:font/woff;base64,Zm9udA==", &sheet, &sheet, 4).unwrap();
    assert_eq!(result.body.bytes().unwrap(), b"font");
}


#[test]
fn pending_native_service_fonts_preserve_cors_body_limits_and_cancel_delivery() {
    let sheet=HttpUrl::parse("http://127.0.0.1:39821/style.css").unwrap();
    for (headers,max,allowed) in [("Access-Control-Allow-Origin: *\r\n",4,true),("",4,false),("Access-Control-Allow-Origin: *\r\n",3,false)] {
        let (target,peer)=server(response(headers,"font"));
        let font=FontRequest::new(&target.resolve("font.woff2").unwrap().serialize(),&sheet,sheet.origin().clone(),max).unwrap();
        let mut service=NetworkService::new_pending_io(DispatchLimits::default()).unwrap();
        assert!(service.pending_io_snapshot().is_some());
        let id=service.start_font(font,ClientContext::default()).unwrap();
        let result=finish(&mut service,id);
        if allowed {let mut result=result.unwrap();assert_eq!(result.body.bytes().unwrap(),b"font");}
        else if headers.is_empty() {assert!(matches!(result,Err(ApiError::Network(FetchError::Cors))));}
        else {assert!(matches!(result,Err(ApiError::Network(FetchError::ResponseTooLarge))));}
        let request=peer.join().unwrap();assert!(request.starts_with("GET /font.woff2 "));assert!(request.to_ascii_lowercase().contains("origin: http://127.0.0.1:39821"));
    }
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    let target=format!("http://{}/cancel.woff2",listener.local_addr().unwrap());
    let font=FontRequest::new(&target,&sheet,sheet.origin().clone(),4).unwrap();
    let mut service=NetworkService::new_pending_io(DispatchLimits::default()).unwrap();
    let id=service.start_font(font,ClientContext::default()).unwrap();assert!(service.abort(id));
    for _ in 0..20 {for event in service.step(128,Instant::now()) {if let ServiceEvent::Finished{id:got,result}=event {assert_eq!(got,id);assert!(result.is_err());}}thread::sleep(Duration::from_millis(1));}
    assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock,"aborted font opened a socket");
}
