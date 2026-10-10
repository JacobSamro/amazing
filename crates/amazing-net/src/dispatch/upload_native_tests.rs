//! Real authenticated PendingTls peers; no adopted/preauthenticated socket.
use super::*;
use std::sync::mpsc;
fn frame(kind:u8,flags:u8,stream:u32,payload:&[u8])->Vec<u8> {
    let length=payload.len();let mut bytes=vec![(length>>16) as u8,(length>>8) as u8,length as u8,kind,flags];
    bytes.extend_from_slice(&stream.to_be_bytes());bytes.extend_from_slice(payload);bytes
}
fn read_frame(io:&mut impl Read)->(u8,u8,u32,Vec<u8>) {
    let mut head=[0;9];io.read_exact(&mut head).unwrap();
    let length=((head[0] as usize)<<16)|((head[1] as usize)<<8)|head[2] as usize;
    let mut payload=vec![0;length];io.read_exact(&mut payload).unwrap();
    (head[3],head[4],u32::from_be_bytes(head[5..].try_into().unwrap())&0x7fff_ffff,payload)
}
fn policy()->ClientPolicy {
    ClientPolicy::from_der_trust_anchors([include_bytes!("../../../amazing-net-tls/tests/fixtures/ca.der").to_vec()]).unwrap()
}
#[test]
fn authenticated_h2_upload_flow_window_sibling_and_end_before_response() {
    let (release,grant)=mpsc::channel();let (ready,seen)=mpsc::channel();
    let size=96*1024;
    let (port,peer)=amazing_net_tls::test_support::serve(vec![b"h2".to_vec()],move|io|{
        assert_eq!(io.protocol(),Some(b"h2".as_slice()));
        let mut preface=[0;24];io.read_exact(&mut preface).unwrap();assert_eq!(&preface,http2::PREFACE);
        assert_eq!(read_frame(io).0,4);io.write_all(&frame(4,0,0,&[])).unwrap();io.flush().unwrap();
        let mut body=0;let mut sibling=false;
        while body<65535 || !sibling {
            let (kind,flags,stream,payload)=read_frame(io);
            if kind==0 && stream==1 {assert_eq!(flags&1,0);assert!(payload.iter().all(|byte|*byte==b'x'));body+=payload.len();}
            if kind==1 && stream==3 {sibling=true;io.write_all(&frame(1,5,3,&[0x88])).unwrap();io.flush().unwrap();}
        }
        assert_eq!(body,65535);ready.send(()).unwrap();grant.recv_timeout(Duration::from_secs(5)).unwrap();
        io.write_all(&frame(8,0,0,&(size as u32).to_be_bytes())).unwrap();
        io.write_all(&frame(8,0,1,&(size as u32).to_be_bytes())).unwrap();io.flush().unwrap();
        loop {let (kind,flags,stream,payload)=read_frame(io);
            if kind==0 && stream==1 {assert!(payload.iter().all(|byte|*byte==b'x'));body+=payload.len();if flags&1!=0 {break;}}
        }
        assert_eq!(body,size);ready.send(()).unwrap();grant.recv_timeout(Duration::from_secs(5)).unwrap();
        io.write_all(&frame(1,5,1,&[0x88])).unwrap();io.flush().unwrap();
    });
    let mut dispatcher=Dispatcher::with_tls_policy_pending_io(DispatchLimits::default(),policy()).unwrap();
    let now=Instant::now();
    let first=dispatcher.submit(&format!("https://localhost:{port}/upload"),FetchOptions{method:"POST".into(),body:vec![b'x';size],..FetchOptions::default()},now).unwrap();
    assert!(dispatcher.track_request_upload(first));
    let second=dispatcher.submit(&format!("https://localhost:{port}/sibling"),FetchOptions::default(),now).unwrap();
    let deadline=Instant::now()+Duration::from_secs(5);let mut sibling_done=false;let mut sent=0;
    while !sibling_done || sent!=65535 {
        for event in dispatcher.step(4096,Instant::now()) {
            match event {NetEvent::Done{id} if id==second=>sibling_done=true,
                NetEvent::Error{error,..}=>panic!("genuine TLS/H2 peer failed {error:?}"),
                NetEvent::Head{id,..} if id==first=>panic!("held upload response leaked"),_=>{}}
        }
        for progress in dispatcher.take_request_upload_progress() {assert_eq!(progress.id,first);assert!(!progress.complete);sent=progress.sent;}
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
    }
    seen.recv_timeout(Duration::from_secs(5)).unwrap();release.send(()).unwrap();
    let mut complete=false;
    while !complete {
        for event in dispatcher.step(4096,Instant::now()) {
            assert!(!matches!(event,NetEvent::Head{id,..} if id==first));
            if let NetEvent::Error{error,..}=event {panic!("upload failed {error:?}");}
        }
        for progress in dispatcher.take_request_upload_progress() {
            assert_eq!(progress.id,first);assert_eq!(progress.total,size as u64);
            if progress.complete {assert_eq!(progress.sent,size as u64);complete=true;}
        }
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
    }
    seen.recv_timeout(Duration::from_secs(5)).unwrap();release.send(()).unwrap();
    let mut done=false;
    while !done {
        for event in dispatcher.step(4096,Instant::now()) {
            if matches!(event,NetEvent::Done{id} if id==first) {done=true;}
            if let NetEvent::Error{error,..}=event {panic!("response failed {error:?}");}
        }
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
    }
    peer.join().unwrap().unwrap();
}
