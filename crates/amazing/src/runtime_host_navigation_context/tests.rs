use super::*;
use std::{io::{Read,Write},net::TcpListener};
fn fixture(html:&str)->(Rc<RefCell<BrowserDomHost>>,Interpreter,RuntimeBindings) {
    let host=Rc::new(RefCell::new(BrowserDomHost{dom:amazing_html::parse(html),native_forwarded:false}));
    let mut js=Interpreter::new();js.set_dom_host(host.clone());
    let bindings=crate::runtime_binding_host::install(&mut js,host.clone()).unwrap();
    (host,js,bindings)
}
#[test]
fn missing_admission_never_becomes_default_client_authority() {
    let (_,mut js,bindings)=fixture("");
    bindings.set_document_url(&mut js,"https://example.test/unadmitted");
    let identity=bindings.receiving_document_identity(NodeIdx(0)).unwrap();
    assert!(bindings.receiving_document_client(&mut js,&identity).is_err());
    assert!(bindings.state.borrow().frames.receiving_admissions.is_empty());
}
#[test]
fn genuine_initial_root_keeps_opaque_creation_and_rejects_re_admission() {
    let (_,mut js,bindings)=fixture("");
    let identity=bindings.initialize_initial_root_document(&mut js,"about:blank").unwrap();
    let before=bindings.receiving_document_client(&mut js,&identity).unwrap();
    assert!(before.origin.opaque_id().is_some());
    assert_eq!(before.creation_url.serialize(),"about:blank");
    assert_eq!(before.origin,before.csp_self_origin);
    bindings.set_document_url(&mut js,"about:blank#same-document");
    let after=bindings.receiving_document_client(&mut js,&identity).unwrap();
    assert_eq!(after.origin,before.origin);assert_eq!(after.csp_self_origin,before.csp_self_origin);
    assert_eq!(after.creation_url,before.creation_url);assert_eq!(after.document_url.serialize(),"about:blank#same-document");
    assert!(bindings.initialize_initial_root_document(&mut js,"https://foreign.test/new").is_err());
    let mut stale=identity;stale.generation+=1;
    assert!(bindings.receiving_document_client(&mut js,&stale).is_err());
    let mut foreign=identity;foreign.window+=1;
    assert!(bindings.receiving_document_client(&mut js,&foreign).is_err());
}
#[test]
fn actual_http_receipt_admits_policy_and_referrer_without_replacing_creation_origin() {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let response_url=format!("http://{}/received",listener.local_addr().unwrap());
    let peer=std::thread::spawn(move||{
        let (mut socket,_)=listener.accept().unwrap();let mut request=vec![];let mut byte=[0;1];
        while !request.ends_with(b"\r\n\r\n") {socket.read_exact(&mut byte).unwrap();request.push(byte[0]);}
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 18\r\nContent-Security-Policy: connect-src 'self'\r\nContent-Security-Policy-Report-Only: connect-src 'none'\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Embedder-Policy: require-corp\r\n\r\n<base href='sub/'>").unwrap();
    });
    let received=amazing_net::fetch_received(&response_url,&amazing_net::FetchOptions::default()).unwrap();
    assert!(!received.source().is_local_file());
    let (_,mut js,bindings)=fixture(std::str::from_utf8(&received.response().body).unwrap());
    let identity=bindings.initialize_received_root_document(&mut js,&response_url,&received).unwrap();
    let client=bindings.receiving_document_client(&mut js,&identity).unwrap();
    assert_eq!(client.creation_url.serialize(),response_url);
    assert_eq!(client.origin,HttpUrl::parse(&response_url).unwrap().origin().clone());
    assert_eq!(client.csp_self_origin,HttpUrl::parse(&response_url).unwrap().origin().clone());
    assert_eq!(client.context.csp.policies.len(),2);assert_eq!(client.referrer_policy,amazing_fetch::ReferrerPolicy::NoReferrer);
    assert_eq!(client.base_url.serialize(),format!("{}sub/",response_url.trim_end_matches("received")));
    assert_eq!(client.context.security.embedder_policy,EmbedderPolicy::RequireCorp); // loopback creation URL is potentially trustworthy
    assert!(client.context.frame_ancestors.is_empty());
    assert!(bindings.initialize_received_root_document(&mut js,&response_url,&received).is_err());
    peer.join().unwrap();
}

#[test]
fn actual_about_child_keeps_opaque_receiver_separate_from_creator_policy() {
    let (host,mut js,bindings)=fixture("<iframe sandbox='allow-scripts' srcdoc='<p>child</p>'></iframe>");
    let root=bindings.initialize_initial_root_document(&mut js,"https://creator.test/root").unwrap();
    let original=bindings.receiving_document_client(&mut js,&root).unwrap();
    let child={let state=bindings.state.borrow();state.frames.contexts.iter()
        .find(|(_,context)|context.frame.is_some()).map(|(realm,context)|ReceivingDocumentIdentity{
            realm:*realm,window:context.event_target_slot,document:context.document,generation:context.document_generation}).unwrap()};
    let receiver=bindings.receiving_document_client(&mut js,&child).unwrap();
    assert!(receiver.origin.opaque_id().is_some());assert_ne!(receiver.origin,original.origin);
    assert_eq!(receiver.csp_self_origin,original.csp_self_origin);
    assert_eq!(receiver.context.frame_ancestors,vec![original.origin.clone()]);
    assert_eq!(receiver.context.csp,original.context.csp);
    assert_eq!(receiver.creation_url.serialize(),"about:srcdoc");
    assert_eq!(receiver.context.security.embedder_policy,original.context.security.embedder_policy);
    assert_eq!(bindings.state.borrow().frames.contexts[&child.realm].receiving_environment.as_ref()
        .unwrap().top_level_creation_url.serialize(),"https://creator.test/root");
    let frame=bindings.state.borrow().frames.contexts[&child.realm].frame.unwrap();
    host.borrow_mut().dom.remove(frame).unwrap();
    assert!(bindings.receiving_document_client(&mut js,&child).is_err());
}
