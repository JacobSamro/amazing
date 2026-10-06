//! Real IPC/HTTP contrast: spelling a destination cannot mint Reporting settings.
use amazing_ipc::codec::ByteBuf;
use amazing_process::{network_v2::{get_request, NetworkSession}, protocol::*};
use std::{io::{Read,Write}, net::TcpListener, time::{Duration,Instant}};

fn context(origin:&str)->NetworkContext {
    NetworkContext {
        origin:WireOrigin::Tuple(origin.into()),
        cookie_site:WireCookieSite::Origin(WireOrigin::Tuple(origin.into())),
        top_level_origin:Some(WireOrigin::Tuple(origin.into())),
        ancestor_origins:Vec::new(),sandbox_flags:0,now_unix:1800000000,
        restricts_mixed_content:false,image_set_initiator:false,
        top_level_navigation:false,embedder_policy:"unsafe-none".into(),
        no_cors_media_state:"none".into(),csp_headers:Vec::new(),
    }
}

#[test]
fn report_destination_cannot_adopt_null_client_authority_through_ordinary_ipc_start() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin=format!("http://{}",listener.local_addr().unwrap());
    let session=NetworkSession::launch(std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child")),99173,Duration::from_secs(10)).unwrap();
    let mut report=get_request(format!("{origin}/report"),format!("{origin}/creation"),"report",1024,context(&origin));
    report.method="POST".into();report.body_present=true;report.body=ByteBuf(b"[]".to_vec());
    report.headers.push(HeaderField{name:"content-type".into(),value:"application/reports+json".into()});
    let error=session.start(report.clone()).unwrap_err();
    assert!(format!("{error:?}").contains("native registered Reporting admission"));
    assert!(session.fetch_subresource(report).is_err());
    assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
    // An ordinary sibling still has its real client and works on the same IPC
    // session. No fake Report event, registry handle, or null ClientContext.
    let peer=std::thread::spawn(move||{
        let deadline=Instant::now()+Duration::from_secs(10);
        let (mut stream,_)=loop {
            match listener.accept() {
                Ok(pair)=>break pair,
                Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{
                    assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
                },
                Err(error)=>panic!("accept {error}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut headers=Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            assert!(headers.len()<16*1024);let mut byte=[0];stream.read_exact(&mut byte).unwrap();headers.push(byte[0]);
        }
        let request=String::from_utf8(headers).unwrap();
        assert!(request.starts_with("GET /allowed "));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
        listener
    });
    let ordinary=get_request(format!("{origin}/allowed"),format!("{origin}/creation"),"empty",1024,context(&origin));
    let response=session.fetch(ordinary).unwrap();
    assert_eq!(response.head.status,200);assert_eq!(response.body,b"ok");
    let listener=peer.join().unwrap();
    assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
}
