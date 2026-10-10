//! Installed Interpreter/Window XHR over the shared native service and real peers.
use super::*;
use crate::js_scheduler::DriverScheduler;
use std::{io::{Read,Write},net::{TcpListener,TcpStream},sync::mpsc};
fn request(socket:&mut TcpStream)->String {
    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut bytes=vec![];let mut byte=[0;1];
    while !bytes.ends_with(b"\r\n\r\n") {socket.read_exact(&mut byte).unwrap();bytes.push(byte[0]);}
    String::from_utf8(bytes).unwrap()
}
fn page()->(DriverScheduler,String) {page_with_policy("")}
fn page_with_policy(csp:&str)->(DriverScheduler,String) {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let url=format!("http://{}/root",listener.local_addr().unwrap());
    let csp=csp.to_owned();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();assert!(request(&mut socket).starts_with("GET /root "));
        let csp=if csp.is_empty(){String::new()}else{format!("Content-Security-Policy: {csp}\r\n")};
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Type: text/html\r\nReferrer-Policy: no-referrer\r\n{csp}\r\n<body>").as_bytes()).unwrap();
    });
    let received=amazing_net::fetch_received(&url,&amazing_net::FetchOptions::default()).unwrap();
    peer.join().unwrap();
    let host=Rc::new(RefCell::new(BrowserDomHost{dom:amazing_html::parse(std::str::from_utf8(&received.response().body).unwrap()),native_forwarded:false}));
    let mut scheduler=DriverScheduler::new(host,crate::js_scheduler::JS_CAPACITY).unwrap();
    scheduler.set_location_url_with_response(&url,&received).unwrap();
    (scheduler,url)
}
fn pump(scheduler:&mut DriverScheduler,condition:&str) {
    let deadline=Instant::now()+Duration::from_secs(5);
    while scheduler.run_script(condition).unwrap()!=JsValue::Bool(true) {
        scheduler.pump_now(false).unwrap();
        assert!(Instant::now()<deadline,"installed XHR missed checkpoint {condition}");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn real_window_headers_loading_prefix_and_terminal_before_held_body_release() {
    let (mut scheduler,_)=page();let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let target=format!("http://{}/body#fragment",listener.local_addr().unwrap());
    let (release,hold)=mpsc::channel();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let wire=request(&mut socket);
        assert!(wire.starts_with("GET /body "));assert!(!wire.to_ascii_lowercase().contains("referer:"));
        let origin=wire.lines().filter_map(|line|line.split_once(':')).find(|(name,_)|name.eq_ignore_ascii_case("origin")).unwrap().1.trim();
        let head=format!("HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Type: text/plain;charset=utf-8\r\nAccess-Control-Allow-Origin: {origin}\r\n\r\n");
        socket.write_all(head.as_bytes()).unwrap();hold.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(70));
        socket.write_all(&[0xe2]).unwrap();hold.recv_timeout(Duration::from_secs(5)).unwrap();
        socket.write_all(&[0x82,0xac]).unwrap();
    });
    scheduler.run_script(format!("var states=[],progress=[],done=false;var xhr=new XMLHttpRequest();xhr.onreadystatechange=function(){{states.push(xhr.readyState);}};xhr.onprogress=function(e){{progress.push([e.loaded,e.total,e.lengthComputable,xhr.responseText]);}};xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();")).unwrap();
    pump(&mut scheduler,"xhr.readyState===2");
    assert_eq!(scheduler.run_script("xhr.status===200 && xhr.responseText==='' && !done && xhr.responseURL.indexOf('#')===-1").unwrap(),JsValue::Bool(true));
    release.send(()).unwrap();pump(&mut scheduler,"xhr.readyState===3 && progress.length>0");
    assert_eq!(scheduler.run_script("xhr.responseText==='\\ufffd' && progress[0][0]===1 && progress[0][1]===3 && progress[0][2]===true").unwrap(),JsValue::Bool(true));
    release.send(()).unwrap();pump(&mut scheduler,"done");
    assert_eq!(scheduler.run_script("xhr.responseText==='€' && states.indexOf(2)<states.indexOf(3) && states[states.length-1]===4").unwrap(),JsValue::Bool(true));
    assert!(scheduler.test_bindings().state.borrow().net.xhr_pending.is_empty());peer.join().unwrap();
}
#[test]
fn loadstart_abort_and_reopen_never_admit_old_socket() {
    for reopen in [false,true] {
        let (mut scheduler,_)=page();let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
        let target=format!("http://{}/forbidden",listener.local_addr().unwrap());
        let callback=if reopen {"xhr.open('GET','data:text/plain,replacement')"} else {"xhr.abort()"};
        scheduler.run_script(format!("var calls=0;var xhr=new XMLHttpRequest();xhr.onloadstart=function(){{calls++;{callback};}};xhr.open('GET','{target}');xhr.send();")).unwrap();
        for _ in 0..10 {scheduler.pump_now(false).unwrap();}
        assert_eq!(scheduler.run_script("calls===1").unwrap(),JsValue::Bool(true));
        assert!(listener.accept().is_err());
        let state=scheduler.test_bindings().state.borrow();assert!(state.net.xhr_prepared.is_empty());assert!(state.net.xhr_pending.is_empty());
        assert_eq!(state.net.fetch_network.dispatcher.http_requests_in_flight(),0);
    }
}
#[test]
fn callback_exception_is_reported_to_original_window_and_does_not_stop_fetch() {
    let (mut scheduler,_)=page();let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let target=format!("http://{}/ok",listener.local_addr().unwrap());
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let wire=request(&mut socket);
        let origin=wire.lines().filter_map(|line|line.split_once(':')).find(|(name,_)|name.eq_ignore_ascii_case("origin")).unwrap().1.trim();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nAccess-Control-Allow-Origin: {origin}\r\n\r\nok").as_bytes()).unwrap();
    });
    scheduler.run_script(format!("var reported=0,done=false;window.addEventListener('error',function(e){{reported++;e.preventDefault();}});var xhr=new XMLHttpRequest();xhr.onloadstart=function(){{throw new Error('original-loadstart');}};xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();")).unwrap();
    pump(&mut scheduler,"done");
    assert_eq!(scheduler.run_script("reported===1 && xhr.status===200 && xhr.responseText==='ok'").unwrap(),JsValue::Bool(true));peer.join().unwrap();
}

#[test]
fn accepted_root_csp_rejects_before_socket_and_cors_rejects_before_visible_headers() {
    let (mut scheduler,_)=page_with_policy("connect-src 'none'");
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();listener.set_nonblocking(true).unwrap();
    let target=format!("http://{}/blocked",listener.local_addr().unwrap());
    scheduler.run_script(format!("var errors=0,done=false,states=[];var xhr=new XMLHttpRequest();xhr.onreadystatechange=function(){{states.push(xhr.readyState);}};xhr.onerror=function(){{errors++;}};xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();")).unwrap();
    pump(&mut scheduler,"done");
    assert_eq!(scheduler.run_script("errors===1 && xhr.status===0 && states.indexOf(2)===-1 && xhr.responseText===''").unwrap(),JsValue::Bool(true));
    assert!(listener.accept().is_err());assert_eq!(scheduler.test_bindings().state.borrow().net.fetch_network.dispatcher.http_requests_in_flight(),0);
    let (mut scheduler,_)=page();let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let target=format!("http://{}/denied",listener.local_addr().unwrap());
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();request(&mut socket);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nAccess-Control-Allow-Origin: http://wrong.invalid\r\nX-Secret: denied\r\n\r\nsecret").unwrap();
    });
    scheduler.run_script(format!("var errors=0,done=false,states=[];var xhr=new XMLHttpRequest();xhr.onreadystatechange=function(){{states.push(xhr.readyState);}};xhr.onerror=function(){{errors++;}};xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();")).unwrap();pump(&mut scheduler,"done");
    assert_eq!(scheduler.run_script("errors===1 && xhr.status===0 && xhr.responseURL==='' && xhr.getResponseHeader('x-secret')===null && states.indexOf(2)===-1").unwrap(),JsValue::Bool(true));peer.join().unwrap();
}

#[test]
fn actual_sandbox_opaque_receiver_and_retirement_keep_original_task_identity() {
    let (mut scheduler,_)=page();
    scheduler.run_script("var f=document.createElement('iframe');f.setAttribute('sandbox','allow-scripts');f.srcdoc='<p>child</p>';document.body.appendChild(f);").unwrap();
    let realm={let state=scheduler.test_bindings().state.borrow();state.frames.contexts.iter()
        .find(|(_,context)|context.frame.is_some()).map(|(realm,_)|*realm).unwrap()};
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();let target=format!("http://{}/opaque",listener.local_addr().unwrap());
    let (release,hold)=mpsc::channel();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let wire=request(&mut socket);
        let origin=wire.lines().filter_map(|line|line.split_once(':')).find(|(name,_)|name.eq_ignore_ascii_case("origin")).unwrap().1.trim();
        assert_eq!(origin,"null");
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nAccess-Control-Allow-Origin: null\r\n\r\n").unwrap();
        hold.recv_timeout(Duration::from_secs(5)).unwrap();let _=socket.write_all(b"late");
    });
    scheduler.js.binding_with_realm(realm,|js|js.run_script(&format!("var done=false;var xhr=new XMLHttpRequest();xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();"))).unwrap();
    let deadline=Instant::now()+Duration::from_secs(5);
    loop {
        let received=scheduler.js.binding_with_realm(realm,|js|js.run_script("xhr.readyState===2")).unwrap();
        if received==JsValue::Bool(true){break;}
        scheduler.pump_now(false).unwrap();assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));
    }
    scheduler.run_script("f.remove()").unwrap();
    scheduler.pump_now(false).unwrap();scheduler.pump_now(false).unwrap();
    assert!(scheduler.test_bindings().state.borrow().net.xhr_pending.is_empty());
    release.send(()).unwrap();peer.join().unwrap();scheduler.pump_now(false).unwrap();
    // Native fixture inspection reads the retained original realm, not an
    // author bypass of the cross-origin WindowProxy access policy.
    assert_eq!(scheduler.js.binding_with_realm(realm,|js|js.run_script("done===false && xhr.readyState===0 && xhr.status===0")).unwrap(),JsValue::Bool(true));
}

#[test]
fn timeout_after_visible_head_cancels_original_service_and_clears_response() {
    let (mut scheduler,_)=page();let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let target=format!("http://{}/timeout",listener.local_addr().unwrap());let (release,hold)=mpsc::channel();
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let wire=request(&mut socket);
        let origin=wire.lines().filter_map(|line|line.split_once(':')).find(|(name,_)|name.eq_ignore_ascii_case("origin")).unwrap().1.trim();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nAccess-Control-Allow-Origin: {origin}\r\n\r\n").as_bytes()).unwrap();
        hold.recv_timeout(Duration::from_secs(5)).unwrap();let _=socket.write_all(b"late");
    });
    scheduler.run_script(format!("var timed=0,loaded=0,done=false;var xhr=new XMLHttpRequest();xhr.ontimeout=function(){{timed++;}};xhr.onload=function(){{loaded++;}};xhr.onloadend=function(){{done=true;}};xhr.open('GET','{target}');xhr.send();")).unwrap();
    pump(&mut scheduler,"xhr.readyState===2");scheduler.run_script("xhr.timeout=30").unwrap();
    pump(&mut scheduler,"done");
    assert_eq!(scheduler.run_script("timed===1 && loaded===0 && xhr.status===0 && xhr.responseURL==='' && xhr.responseText===''").unwrap(),JsValue::Bool(true));
    release.send(()).unwrap();peer.join().unwrap();scheduler.pump_now(false).unwrap();
    assert_eq!(scheduler.run_script("timed===1 && loaded===0").unwrap(),JsValue::Bool(true));
}

#[test]
fn synchronous_service_admission_failures_run_error_steps_before_throwing_and_reopen_cleanly() {
    for denied_by_policy in [true, false] {
        let (mut scheduler, root) = if denied_by_policy {
            page_with_policy("connect-src 'none'")
        } else {
            page()
        };
        if !denied_by_policy {
            let mut limits = amazing_net::dispatch::DispatchLimits::default();
            limits.max_in_flight = 0;
            scheduler
                .test_bindings()
                .state
                .borrow_mut()
                .net
                .fetch_network = amazing_fetch::service::NetworkService::new(limits);
        }
        let script = format!(
            "var xhr=new XMLHttpRequest();var failed=false;var events=0;xhr.onreadystatechange=function(){{events++}};xhr.open('GET','{root}',false);try{{xhr.send()}}catch(e){{failed=e.name==='NetworkError'}};var terminal=failed&&xhr.readyState===4&&xhr.status===0&&xhr.responseURL===''&&xhr.responseText==='';xhr.open('GET','{root}');var reopened=xhr.readyState===1;terminal&&reopened"
        );
        assert_eq!(scheduler.run_script(script).unwrap(), JsValue::Bool(true));
        assert!(
            scheduler
                .test_bindings()
                .state
                .borrow()
                .net
                .xhr_prepared
                .is_empty()
        );
        assert!(
            scheduler
                .test_bindings()
                .state
                .borrow()
                .net
                .xhr_pending
                .is_empty()
        );
    }
}
