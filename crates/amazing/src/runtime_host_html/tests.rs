//! Native document and browsing-context integration regressions.
use super::*;
use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener};

#[test]
fn document_policy_snapshot_uses_actual_origins_ancestry_and_generations() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
        "<!doctype html><base href='https://example.test/saved/'><iframe id=a></iframe><iframe id=b sandbox='allow-scripts'></iframe>"), native_forwarded: false }));
    let mut js = Interpreter::new();
    js.set_dom_host(host.clone());
    let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
    bindings.set_document_url(&mut js, "https://example.test/committed/page");
    let state = bindings.state.borrow();
    let borrowed = host.borrow();
    let root = state.frames.document_context(&borrowed.dom, NodeIdx(0)).unwrap();
    assert_eq!(root.document, NodeIdx(0));
    assert_eq!(Some(root.document_generation), borrowed.dom.node_generation(root.document));
    assert_eq!(root.document_url, "https://example.test/committed/page");
    assert_eq!(root.origin.serialize(), "https://example.test");
    assert!(root.ancestor_origins.is_empty());
    let children: Vec<_> = state.frames.documents.values().filter_map(|entry| entry.document).collect();
    let snapshots: Vec<_> = children.iter().map(|document| state.frames.document_context(&borrowed.dom, *document).unwrap()).collect();
    for (document, snapshot) in children.iter().zip(&snapshots) {
        assert_eq!(snapshot.document, *document);
        assert_eq!(Some(snapshot.document_generation), borrowed.dom.node_generation(*document));
    }
    assert_eq!(snapshots.len(), 2);
    for (realm, context) in &state.frames.contexts {
        assert_eq!(state.frames.document_creation_origin(&borrowed.dom, context.document, *realm),
            Some(context.fetch_origin.clone()));
        let by_realm = state.frames.document_context_for_realm(&borrowed.dom, *realm).unwrap();
        let by_document = state.frames.document_context(&borrowed.dom, context.document).unwrap();
        assert_eq!(by_realm.document_url, by_document.document_url);
        assert_eq!(by_realm.origin, by_document.origin);
        assert_eq!(by_realm.top_level_origin, by_document.top_level_origin);
    }
    for snapshot in &snapshots {
        assert_eq!(snapshot.document_url, "about:blank");
        assert_eq!(snapshot.fallback_base_url.as_deref(), Some("https://example.test/saved/"));
        assert_eq!(snapshot.top_level_origin, root.origin);
        assert_eq!(snapshot.ancestor_origins, vec![root.origin.clone()]);
    }
    let opaque = snapshots.iter().find(|snapshot| snapshot.origin.opaque_id().is_some()).unwrap();
    assert_ne!(opaque.origin, root.origin);
    assert_ne!(opaque.origin, amazing_fetch::Origin::opaque());
    let retired = children[0];
    let realm = *state.frames.contexts.iter().find(|(_, context)| context.document == retired).unwrap().0;
    drop(borrowed);
    drop(state);
    // Pending URL changes cannot alter the committed snapshot.
    js.set_location_url("https://pending.test/not-committed");
    let mut state = bindings.state.borrow_mut();
    assert_eq!(state.frames.document_context(&host.borrow().dom, NodeIdx(0)).unwrap().origin, root.origin);
    state.frames.contexts.get_mut(&realm).unwrap().document_generation += 1;
    assert!(state.frames.document_creation_origin(&host.borrow().dom, retired, realm).is_none());
    assert!(state.frames.document_context(&host.borrow().dom, retired).is_none());
    assert!(state.frames.document_context_for_realm(&host.borrow().dom, realm).is_none());
    state.frames.contexts.get_mut(&realm).unwrap().document_generation -= 1;
    let frame = state.frames.contexts[&realm].frame.unwrap();
    let creation_origin = state.frames.document_creation_origin(&host.borrow().dom, retired, realm).unwrap();
    state.frames.set_inert_document_origin(&host.borrow().dom, retired, creation_origin.clone()).unwrap();
    state.frames.documents.get_mut(&frame).unwrap().document = None;
    // Context allocation precedes the frame-document commit; its exact
    // native origin remains available without inventing a live context.
    assert_eq!(state.frames.document_creation_origin(&host.borrow().dom, retired, realm), Some(creation_origin.clone()));
    assert!(state.frames.document_context(&host.borrow().dom, retired).is_none());
    assert!(state.frames.document_context_for_realm(&host.borrow().dom, realm).is_none());
    state.frames.windows.remove(&frame);
    assert!(state.frames.document_creation_origin(&host.borrow().dom, retired, realm).is_none());
    assert_eq!(state.frames.document_origin(&host.borrow().dom, retired), Some(creation_origin));
    // A stale root ancestor invalidates the entire child policy snapshot.
    let root_realm = *state.frames.contexts.iter().find(|(_, context)| context.frame.is_none()).unwrap().0;
    state.frames.contexts.get_mut(&root_realm).unwrap().document_generation += 1;
    assert!(state.frames.document_context(&host.borrow().dom, children[1]).is_none());
    assert!(state.frames.document_context_for_realm(&host.borrow().dom, root_realm).is_none());
}

#[test]
fn inert_document_origins_keep_opaque_identity_without_fabricating_contexts() {
    let mut dom = Dom::new();
    let mut frames = FrameRuntime::default();
    let inert = dom.create_document();
    let opaque = amazing_fetch::Origin::opaque();
    assert!(frames.document_origin(&dom, inert).is_none());
    frames.set_inert_document_origin(&dom, inert, opaque.clone()).unwrap();
    assert_eq!(frames.document_origin(&dom, inert), Some(opaque.clone()));
    assert!(frames.document_context(&dom, inert).is_none());
    assert!(frames.set_inert_document_origin(&dom, inert, amazing_fetch::Origin::opaque()).is_err());
    let retained = frames.document_origin(&dom, inert).unwrap();
    frames.inert_document_origins.get_mut(&inert).unwrap().0 += 1;
    assert!(frames.document_origin(&dom, inert).is_none());
    assert_eq!(retained, opaque);
    let element = dom.create_element(NodeIdx(0), "div");
    assert!(frames.set_inert_document_origin(&dom, element, retained).is_err());
    assert!(frames.document_origin(&dom, element).is_none());
    assert!(frames.document_origin(&dom, NodeIdx(u32::MAX)).is_none());
    assert!(frames.set_inert_document_origin(&dom, NodeIdx(u32::MAX), amazing_fetch::Origin::opaque()).is_err());
}

#[test]
fn about_documents_capture_creation_base_and_full_blank_url() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
        "<!doctype html><base href='https://example.test/first/'><iframe id=a></iframe><iframe id=b src='about:blank?query#fragment'></iframe><iframe id=c srcdoc='<p>child'></iframe>"), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/root/page.html");
    assert_eq!(scheduler.run_script(r#"
        const a=document.getElementById('a'), b=document.getElementById('b'), c=document.getElementById('c');
        const old=a.contentDocument;
        if(old.baseURI!=='https://example.test/first/' || old.compatMode!=='BackCompat') throw new Error('initial creation snapshot');
        if(b.contentDocument.URL!=='about:blank?query#fragment' || b.contentDocument.baseURI!==old.baseURI) throw new Error('full blank URL');
        if(c.contentDocument.URL!=='about:srcdoc' || c.contentDocument.baseURI!==old.baseURI) throw new Error('srcdoc snapshot');
        document.querySelector('base').href='https://example.test/second/';
        if(old.baseURI!=='https://example.test/first/' || c.contentDocument.baseURI!==old.baseURI) throw new Error('mutable creator');
        const own=old.createElement('base'); own.href='relative/'; old.head.appendChild(own);
        if(own.href!=='https://example.test/first/relative/' || old.baseURI!==own.href) throw new Error('base bypass and contribution');
        const clone=old.cloneNode(true); if(clone.baseURI===old.baseURI) throw new Error('clone copied context snapshot');
        'captured';
    "#).unwrap(), JsValue::str("captured"));
}

#[test]
fn queued_child_location_captures_actual_source_and_last_request() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
        "<!doctype html><base href='https://example.test/root-base/'><iframe id=a></iframe><iframe id=b></iframe>"), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/root/page.html");
    scheduler.run_script(r#"
        const a=document.getElementById('a'), b=document.getElementById('b');
        const w=a.contentWindow, old=a.contentDocument;
        const source=b.contentDocument, base=source.createElement('base');
        base.href='https://example.test/source-base/'; source.head.appendChild(base);
        b.contentWindow.eval("parent.frames[0].location.href='about:blank?one'");
        b.contentWindow.eval("parent.frames[0].location.replace('about:blank?two#last')");
        base.href='https://example.test/changed/';
    "#).unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    scheduler.run_script("a.contentWindow.location.reload()").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(r#"
        const observations=[a.contentWindow===w,a.contentDocument!==old,a.contentDocument.URL,
            a.contentDocument.baseURI,old.baseURI];
        location.href='https://other.test/pending';
        const fresh=document.createElement('iframe'); document.body.appendChild(fresh);
        observations.push(fresh.contentDocument.baseURI, a.contentDocument.location===w.location, old.location===null,
            Object.getOwnPropertyDescriptor(location,'href').get.call(w.location));
        observations.join('|');
    "#).unwrap(), JsValue::str("true|true|about:blank?two#last|https://example.test/source-base/|https://example.test/root-base/|https://example.test/root-base/|true|true|about:blank?two#last"));
}

#[test]
fn genuine_cross_origin_location_receivers_preserve_policy_and_source() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
        "<!doctype html><iframe sandbox></iframe>"), native_forwarded: false }));
    let mut js = Interpreter::new_with_external_scheduler(JS_CAPACITY).unwrap();
    js.set_dom_host(host.clone());
    let bindings = crate::runtime_binding_host::install(&mut js, host).unwrap();
    js.set_location_url("https://example.test/main.html");
    let root = js.binding_current_realm();
    js.binding_set_realm_origin(root, "https://example.test").unwrap();
    bindings.set_document_url(&mut js, "https://example.test/main.html");
    let (realm, document) = bindings.state.borrow().frames.contexts.iter()
        .find(|(_, context)| context.frame.is_some()).map(|(realm, context)| (*realm, context.document)).unwrap();
    // Expose the real foreign Location for borrowed-call policy checks;
    // ordinary cross-origin Window.document access remains blocked.
    let location = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() }
        .html_document_location(document).unwrap().unwrap();
    let global = js.binding_global_object();
    js.binding_set_property(&global, "foreignLocation", location).unwrap();
    assert_eq!(js.run_script(r#"
        let rejected=0;
        for (const check of [()=>foreignLocation.href, ()=>foreignLocation.assign('about:blank'), ()=>foreignLocation.reload()]) {
            try { check(); } catch(e) { if(e instanceof DOMException && e.name==='SecurityError') rejected++; }
        }
        let wrong=false; try { Object.getOwnPropertyDescriptor(location,'href').get.call({}); } catch(e) { wrong=e instanceof TypeError; }
        Location.prototype.replace.call(foreignLocation,'about:blank?allowed');
        rejected+'|'+wrong;
    "#).unwrap(), JsValue::str("3|true"));
    let state = bindings.state.borrow();
    let navigation = state.frames.pending_navigations.get(&realm).unwrap();
    assert_eq!(navigation.source.document, NodeIdx(0));
    assert_eq!(navigation.source.generation, state.host.borrow().dom.node_generation(NodeIdx(0)).unwrap());
    assert_eq!(navigation.source.base_url, "https://example.test/main.html");
    assert_eq!(navigation.url, "about:blank?allowed");
}

#[test]
fn windows_expose_actual_live_children_and_proxy_identity() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe id=a></iframe><iframe id=b></iframe>"), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(r#"
        const a=document.getElementById('a'), b=document.getElementById('b');
        const w=a.contentWindow;
        let checkpoint=0; function check(x) { checkpoint++; if(!x) throw new Error('live Window topology '+checkpoint); }
        check(window===self && window===globalThis && window===document.defaultView);
        check(window.frames===window && window.parent===window && window.top===window);
        check(window.length===2 && window[0]===w && window[1]===b.contentWindow);
        check(w.frames===w && w.parent===window && w.top===window && w.length===0);
        const grand=w.document.createElement('iframe'); w.document.body.appendChild(grand);
        const grandWindow=grand.contentWindow;
        check(w.length===1 && w[0]===grandWindow && window.length===2);
        check(grandWindow.parent===w && grandWindow.top===window);
        check(grand.contentDocument===grandWindow.document && grand.contentDocument.defaultView===grandWindow);
        check(w.eval('parent.document.defaultView===parent'));
        const inert=document.implementation.createHTMLDocument('inert');
        const hidden=inert.createElement('iframe'); inert.body.appendChild(hidden);
        check(hidden.contentWindow===null && hidden.contentDocument===null && window.length===2);
        const d=Object.getOwnPropertyDescriptor(window,'0');
        check(d.value===w && !d.writable && d.enumerable && d.configurable);
        check(!Reflect.defineProperty(window,'0',{value:42}));
        check(!Reflect.set(window,'0',42)); check(!Reflect.deleteProperty(window,'0'));
        a.srcdoc='<!doctype html><p>new'; check(a.contentWindow===w && window[0]===w);
        check(grandWindow.parent===null && grandWindow.top===null && grandWindow.length===0);
        a.remove(); check(window.length===1 && window[0]===b.contentWindow && !('1' in window));
        check(w.parent===null && w.top===null && w.length===0 && w.frames===w);
        true
    "#).unwrap_or_else(|error| panic!("Window topology failed: {:?}", scheduler.js.binding_get_property(&error.0, "message"))), JsValue::Bool(true));
}

#[test]
fn window_getters_check_receiver_and_replaceable_descriptors() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe></iframe>"), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(r#"
        (function() {
        function check(x) { if(!x) throw new Error('Window descriptors'); }
        const child=document.querySelector('iframe').contentWindow;
        for(const name of ['frames','length','parent']) {
            const d=Object.getOwnPropertyDescriptor(window,name);
            check(typeof d.get==='function' && typeof d.set==='function' && d.enumerable && d.configurable);
            let rejected=false; try { d.get.call({}); } catch(e) { rejected=e instanceof TypeError; } check(rejected);
            check(d.get.call(child)===(name==='frames'?child:name==='length'?0:window));
            const ordinary={}; d.set.call(ordinary,456); check(ordinary[name]===456);
            d.set.call(child,123); check(child[name]===123);
            const replaced=Object.getOwnPropertyDescriptor(child,name);
            check(replaced.writable && replaced.enumerable && replaced.configurable);
        }
        const top=Object.getOwnPropertyDescriptor(window,'top');
        check(top.get.call(child)===window && top.set===undefined && !top.configurable && top.enumerable);
        const setter=Object.getOwnPropertyDescriptor(window,'frames').set;
        const target={}; let traps=0;
        const proxy=new Proxy(target,{defineProperty(object,key,descriptor) {
            traps++; check(key==='frames' && descriptor.value===789);
            check(descriptor.writable && descriptor.enumerable && descriptor.configurable);
            return Reflect.defineProperty(object,key,descriptor);
        }});
        setter.call(proxy,789); check(traps===1 && target.frames===789);
        const sentinel=new Error('trap error');
        let propagated=false; try { setter.call(new Proxy({}, {defineProperty(){throw sentinel;}}),0); }
        catch(error) { propagated=error===sentinel; } check(propagated);
        for(const receiver of [null, 1, Object.preventExtensions({}),
            new Proxy({}, {defineProperty(){return false;}})]) {
            let rejected=false; try { setter.call(receiver,0); } catch(error) { rejected=error instanceof TypeError; }
            check(rejected);
        }
        return true;
        })()
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn cross_origin_window_topology_is_readable_but_document_is_protected() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe sandbox></iframe>"), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(r#"
        const child=document.querySelector('iframe').contentWindow;
        const valid=child.frames===child && child.parent===window && child.top===window && child.length===0;
        let rejected=false; try { child.document; } catch(e) { rejected=e instanceof DOMException && e.name==='SecurityError'; }
        const setter=Object.getOwnPropertyDescriptor(window,'frames').set;
        let setterRejected=false; try { setter.call(child,42); }
        catch(e) { setterRejected=e instanceof DOMException && e.name==='SecurityError'; }
        const descriptors=['frames','length','parent','top','window','self'].every(name=>{
            const d=Object.getOwnPropertyDescriptor(child,name);
            return d.enumerable===false && d.configurable===true && d.set===undefined &&
                typeof d.get==='function' && d.get instanceof Function &&
                d.get===Object.getOwnPropertyDescriptor(child,name).get &&
                d.get.call({})===child[name];
        });
        globalThis.oldCrossOriginGetter=Object.getOwnPropertyDescriptor(child,'frames').get;
        valid && rejected && setterRejected && descriptors
    "#).unwrap(), JsValue::Bool(true));
    scheduler.run_script("document.querySelector('iframe').srcdoc='<!doctype html><p>new target</p>'").unwrap();
    assert_eq!(scheduler.run_script(r#"
        const current=document.querySelector('iframe').contentWindow;
        const fresh=Object.getOwnPropertyDescriptor(current,'frames');
        fresh.get!==oldCrossOriginGetter && fresh.get.call(null)===current &&
            fresh.set===undefined && Object.getOwnPropertyDescriptor(current,'top').configurable
    "#).unwrap(), JsValue::Bool(true));
}

fn response_with_mime(mime: Option<&str>, no_sniff: bool) -> amazing_net::Response {
    let mut headers = amazing_net::Headers::new();
    if let Some(mime) = mime { headers.append("content-type", mime.into()).unwrap(); }
    if no_sniff { headers.append("x-content-type-options", "NoSniff".into()).unwrap(); }
    amazing_net::Response { url: "https://example.test/encoding.py?label=big5".into(),
        status: 200, reason: "OK".into(), headers,
        body: b"<!doctype html><meta charset=big5><body>decoded".to_vec() }
}

#[test]
fn missing_or_unknown_response_mime_sniffs_real_html_bytes_and_encoding() {
    for supplied in [None, Some("unknown/unknown"), Some("application/unknown"), Some("*/*")] {
        let response = response_with_mime(supplied, false);
        let mime = content_type(&response, &response.url, false).unwrap();
        assert_eq!(mime, "text/html");
        let (dom, encoding) = parse_response(&response.body, &mime, None).unwrap();
        assert!(dom.dump(amazing_dom::NodeIdx(0)).contains("decoded"));
        assert_eq!(encoding, "Big5");
    }
}

#[test]
fn explicit_mime_and_nosniff_are_authoritative_for_html_signatures() {
    for mime in ["text/plain", "application/octet-stream", "image/png", "application/xml"] {
        let response = response_with_mime(Some(mime), false);
        let actual = content_type(&response, &response.url, false);
        assert_eq!(actual.as_deref(), if mime == "application/octet-stream" { None } else { Some(mime) });
    }
    assert_eq!(content_type(&response_with_mime(None, true), "https://example.test/page", false), None);
    let response = response_with_mime(Some("text/html"), true);
    assert_eq!(content_type(&response, &response.url, false).as_deref(), Some("text/html"));
}

#[test]
fn html_signature_matching_uses_exact_whitespace_and_tag_boundaries() {
    for html in ["<!doctype html>", " \t\n\u{c}\r<HTML lang=en>", "<p>", "<!-- comment -->", "<sCrIpT >"] {
        assert!(has_html_signature(html.as_bytes()), "{html:?}");
    }
    for other in ["<htmlish>", "<html/>", "<html\t>", "<!--comment-->", "prefix<html>", "\u{b}<html>", "<html", "<?xml version='1.0'?>", ""] {
        assert!(!has_html_signature(other.as_bytes()), "{other:?}");
    }
    assert!(!has_html_signature(format!("{}<html>", " ".repeat(1445)).as_bytes()));
}

#[test]
fn iframe_document_metadata_uses_canonical_names_after_decode_precedence() {
    for (bytes, transport, name) in [
        (&b"\xef\xbb\xbf<p>bom"[..], Some("big5"), "UTF-8"),
        (&b"<meta charset=big5><p>meta"[..], None, "Big5"),
        (&b"<meta charset=big5><p>transport"[..], Some("shift_jis"), "Shift_JIS"),
        (&b"<p>fallback"[..], None, "windows-1252"),
    ] {
        assert_eq!(parse_response(bytes, "text/html", transport).unwrap().1, name);
    }
    assert_eq!(parse_response(b"<root/>", "application/xml", Some("iso-8859-2")).unwrap().1, "ISO-8859-2");
    assert_eq!(amazing_html::encoding::Encoding::Utf8.name(), "utf-8");
}

#[test]
fn iframe_scripts_observe_mode_at_each_graft_and_srcdoc_is_no_quirks() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-mode-{}.html", std::process::id()));
    std::fs::write(&path, "<script>window.seenMode = document.compatMode;</script><p>child").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<!doctype html><iframe src=child.html></iframe><iframe srcdoc='<script>window.seenMode = document.compatMode;</script><p>srcdoc'></iframe>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resource = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resource.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("document.querySelectorAll('iframe')[0].contentWindow.seenMode + '|' + document.querySelectorAll('iframe')[0].contentDocument.compatMode + '|' + document.querySelectorAll('iframe')[1].contentWindow.seenMode").unwrap(),
        JsValue::str("BackCompat|BackCompat|CSS1Compat"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn root_document_compat_mode_reflects_the_actual_parser_mode() {
    for (source, expected) in [
        ("<body><p>no doctype", "BackCompat"),
        ("<!doctype html><body><p>standard", "CSS1Compat"),
        ("<!doctype html PUBLIC '-//W3C//DTD XHTML 1.0 Transitional//EN' 'x'><p>limited", "CSS1Compat"),
    ] {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(source), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        assert_eq!(scheduler.run_script("document.compatMode").unwrap(), JsValue::str(expected));
    }
}

#[test]
fn sandboxed_iframe_uses_disabled_noscript_tree_rules() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<!doctype html><iframe sandbox='allow-same-origin' srcdoc='<noscript><b id=visible>fallback</b></noscript><script>window.shouldNotRun=true;</script>'></iframe>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("document.querySelector('iframe').contentDocument.getElementById('visible').textContent").unwrap(), JsValue::str("fallback"));
    assert_eq!(scheduler.run_script("document.querySelector('iframe').contentWindow.shouldNotRun === undefined").unwrap(), JsValue::Bool(true));
}

#[test]
fn iframe_scripting_state_comes_from_context_for_html_xml_xhtml_and_blank() {
    let xml = std::env::temp_dir().join(format!("amazing-scripting-context-{}.xml", std::process::id()));
    let xhtml = xml.with_extension("xhtml");
    std::fs::write(&xml, "<root/>").unwrap();
    std::fs::write(&xhtml, "<html xmlns='http://www.w3.org/1999/xhtml'><body/></html>").unwrap();
    for (sandbox, expected) in [("", true), (" sandbox='allow-scripts allow-same-origin'", true),
        (" sandbox='allow-same-origin'", false), (" sandbox", false)] {
        for (source, mime) in [("", "text/html"), (" srcdoc='<noscript><b>x</b></noscript>'", "text/html"),
            (" src='child.xml'", "application/xml"), (" src='child.xhtml'", "application/xhtml+xml")] {
            let host = Rc::new(RefCell::new(BrowserDomHost {
                dom: amazing_html::parse(&format!("<!doctype html><iframe{sandbox}{source}></iframe>")), native_forwarded: false,
            }));
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
            let xml_path = xml.clone(); let xhtml_path = xhtml.clone();
            scheduler.set_frame_resource_resolver(Rc::new(move |url| Some(
                if url.ends_with(".xhtml") { xhtml_path.to_string_lossy().into_owned() }
                else { xml_path.to_string_lossy().into_owned() })));
            let documents = Rc::new(RefCell::new(Vec::new()));
            let observed = documents.clone();
            scheduler.set_child_navigation_hook(Rc::new(move |navigation| {
                if let Some(document) = navigation.document { observed.borrow_mut().push(document); }
            }));
            scheduler.set_location_url("https://example.test/main.html");
            scheduler.run_script("void document.querySelector('iframe')").unwrap();
            assert!(!documents.borrow().is_empty());
            for &document in documents.borrow().iter() {
                let dom = &host.borrow().dom;
                assert_eq!(dom.document_scripting_enabled(document).unwrap(), expected, "{source}{sandbox}");
                assert_eq!(dom.document_metadata(document).unwrap().unwrap().content_type, mime);
            }
            scheduler.dispatch_pending_iframe_loads().unwrap();
            for &document in documents.borrow().iter() {
                assert_eq!(host.borrow().dom.document_scripting_enabled(document).unwrap(), expected);
            }
        }
    }
    std::fs::remove_file(xml).unwrap(); std::fs::remove_file(xhtml).unwrap();
}

#[test]
fn inert_document_write_uses_disabled_noscript_and_does_not_execute_scripts() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<!doctype html><body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    assert_eq!(scheduler.run_script(r#"
        var inert = document.implementation.createHTMLDocument('inert');
        inert.write('<noscript><b id="fallback">visible</b></noscript><script>window.inertWriteRan=true;</script>');
        inert.getElementById('fallback').textContent === 'visible' && window.inertWriteRan === undefined;
    "#).unwrap(), JsValue::Bool(true));
}

fn serve_xml_once(extra_header: &str) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/child.xml", listener.local_addr().unwrap());
    let extra_header = extra_header.to_owned();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if request.read_line(&mut line).unwrap() == 0 || line == "\r\n" { break; }
        }
        let body = "<?xml version='1.0'?><root>network child</root>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
            body.len(), extra_header, body).unwrap();
        stream.flush().unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let _ = stream.read_to_end(&mut Vec::new());
    });
    (url, worker)
}

#[test]
fn iframe_network_navigation_applies_embedding_policy() {
    for (header, visible) in [("", true), ("X-Frame-Options: DENY\r\n", false)] {
        let (url, worker) = serve_xml_once(header);
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(&format!("<body><iframe src='{url}'></iframe></body>")),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url(url.replace("/child.xml", "/parent.html"));
        let result = scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentDocument !== null && \
             frame.contentDocument.documentElement.textContent === 'network child'")
            .unwrap();
        assert_eq!(result, JsValue::Bool(visible));
        worker.join().unwrap();
    }
}

#[test]
fn iframe_loads_xml_document_before_its_element_load_event() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()));
    std::fs::write(&path, "<?xml version='1.0'?><root>child text</root>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe src='/child.xml'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
    let navigations = Rc::new(RefCell::new(Vec::<ChildNavigation>::new()));
    let observed = navigations.clone();
    scheduler.set_child_navigation_hook(Rc::new(move |navigation| {
        observed.borrow_mut().push(navigation);
    }));
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); var order = []; \
         frame.contentDocument.addEventListener('load', () => order.push('child')); \
         frame.addEventListener('load', () => order.push('frame')); \
         frame.contentDocument.contentType + '|' + \
         frame.contentDocument.documentElement.textContent"
    ).unwrap(), JsValue::str("application/xml|child text"));
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("order.join(',')").unwrap(), JsValue::str("child,frame"));
    assert_eq!(navigations.borrow().len(), 1);
    assert_eq!(navigations.borrow()[0].committed_url, "https://example.test/child.xml");
    std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()))).unwrap();
}

#[test]
fn removing_a_freshly_appended_iframe_before_its_load_dispatch_does_not_panic() {
    // WPT regression (html/dom/elements/global-attributes/
    // lang-attribute-document-element-replacement.html): appending a
    // bare `<iframe>` creates its realm synchronously in
    // `navigate_iframes` and queues a `pending_loads` entry;
    // `iframe.remove()` immediately after (still in the same script,
    // before the harness's next `dispatch_pending_iframe_loads` tick)
    // runs `navigate_iframes` again and evicts the now-disconnected
    // frame's entry from `frames.windows` via `retain`. The stale
    // `pending_loads` entry then makes `dispatch_pending_iframe_loads`
    // call `setup_iframe_window`, which finds no realm and falls back to
    // `ensure_realm` -- previously a `RefCell` double-borrow panic
    // ("already borrowed") because the fallback lookup's `match`
    // scrutinee kept its `Ref` alive across the arm that calls
    // `ensure_realm`'s own `borrow_mut()`.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "var iframe = document.createElement('iframe'); document.body.append(iframe); \
         var doc = iframe.contentDocument; doc.documentElement.lang = 'de'; \
         var div = document.createElement('div'); doc.body.append(div); \
         iframe.remove(); \
         div.getAttribute !== undefined"
    ).unwrap(), JsValue::Bool(true));
    // The stale queue entry drains without panicking.
    scheduler.dispatch_pending_iframe_loads().unwrap();
}

#[test]
fn iframe_content_document_enforces_same_origin_after_src_change() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()));
    std::fs::write(&path, "<root/>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe></iframe></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); \
         frame.contentDocument !== null && frame.contentDocument.URL === 'about:blank'"
    ).unwrap(), JsValue::Bool(true));
    assert_eq!(scheduler.run_script(
        "frame.src = 'https://other.test/child.xml'; \
         frame.getAttribute('src') === 'https://other.test/child.xml' && \
         frame.contentDocument === null"
    ).unwrap(), JsValue::Bool(true));
    std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()))).unwrap();
}

#[test]
fn iframe_content_window_runs_scripts_and_keeps_identity_across_navigation() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-window-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><script>window.markedByChild = true; document.title = 'child';</script></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe src='child.html'></iframe></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    scheduler.run_script("var frame = document.querySelector('iframe');").unwrap();
    assert_eq!(scheduler.run_script("frame.contentWindow !== null").unwrap(), JsValue::Bool(true), "contentWindow non-null");
    assert_eq!(scheduler.run_script("frame.contentWindow.markedByChild").unwrap(), JsValue::Bool(true), "child global set");
    assert_eq!(scheduler.run_script("frame.contentDocument !== null").unwrap(), JsValue::Bool(true), "contentDocument non-null");
    assert_eq!(scheduler.run_script("typeof frame.contentWindow.document").unwrap(), JsValue::str("object"), "child document typeof");
    assert_eq!(scheduler.run_script("frame.contentWindow.document !== null").unwrap(), JsValue::Bool(true), "child document non-null");
    assert_eq!(scheduler.run_script("frame.contentWindow.document === frame.contentDocument").unwrap(), JsValue::Bool(true), "document identity");
    assert_eq!(scheduler.run_script("frame.contentWindow !== window").unwrap(), JsValue::Bool(true), "distinct from parent window");
    assert_eq!(scheduler.run_script("frame.contentWindow.Object !== window.Object").unwrap(), JsValue::Bool(true), "distinct intrinsics");
    assert_eq!(scheduler.run_script(
        "frame.contentWindow === frame.contentWindow"
    ).unwrap(), JsValue::Bool(true));
    scheduler.run_script("globalThis.__savedWin = frame.contentWindow;").unwrap();

    // Re-navigate: the WindowProxy keeps its identity but targets a fresh realm.
    let path2 = std::env::temp_dir().join(format!("amazing-iframe-window2-{}.html", std::process::id()));
    std::fs::write(&path2,
        "<body><script>window.markedByChild = false; window.secondLoad = true;</script></body>").unwrap();
    let resolved2 = path2.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved2.to_string_lossy().into_owned())));
    scheduler.run_script("frame.src = 'other.html';").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "frame.contentWindow === globalThis.__savedWin && \
         frame.contentWindow.secondLoad === true && frame.contentWindow.markedByChild === false"
    ).unwrap(), JsValue::Bool(true));

    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&path2).unwrap();
}

#[test]
fn iframe_created_element_has_a_working_local_name_in_an_xml_child_document() {
    // WPT regression (dom/nodes/Document-createElement.html /
    // -createElementNS.html's XML/XHTML-document parametrizations, 488
    // failing subtests flagged by dom-bindings): the actual per-realm
    // wrapping/accessor machinery this exercises (`materialize_accessors`,
    // per-realm `Interface::Element` prototypes from HTML-27) is fine --
    // confirmed by this same scenario passing here. The real bug was in
    // `amazing wpt-harness`'s `FixtureServer` (cmd/wpt_harness.rs), which
    // sent no `Content-Type` header at all, so `content_type()`'s
    // extension fallback (gated on a resolver or `file:` URL, neither
    // true for that loopback HTTP fetch) never ran either: every XML/XHTML
    // iframe failed to navigate at all ("iframe response has no supported
    // media type"), leaving `contentDocument`/`contentWindow.document`
    // pointing at a document that never loaded -- not a cross-realm
    // wrapping bug. Fixed by sending a real extension-derived
    // `Content-Type`. This test keeps the underlying per-realm path
    // covered directly, independent of the fixture server.
    let path = std::env::temp_dir().join(format!("amazing-iframe-localname-{}.xml", std::process::id()));
    std::fs::write(&path, "<foo>Dummy XML document</foo>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe src='child.xml'></iframe></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "document.querySelector('iframe').contentDocument.documentElement.textContent"
    ).unwrap(), JsValue::str("Dummy XML document"));
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); \
         var el = frame.contentWindow.document.createElement('div'); \
         el.localName === 'div' && el instanceof frame.contentWindow.Element"
    ).unwrap(), JsValue::Bool(true));
    assert_eq!(scheduler.run_script(
        "var child = frame.contentDocument; \
         child.defaultView === frame.contentWindow && \
         child.defaultView.document === child && \
         child.defaultView.DOMException === frame.contentWindow.DOMException"
    ).unwrap(), JsValue::Bool(true));
    assert_eq!(scheduler.run_script(
        "var caught; try { child.createElementNS('bad', 'xml:x'); } \
         catch (e) { caught = e; } \
         caught.constructor === child.defaultView.DOMException"
    ).unwrap(), JsValue::Bool(true));
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn iframe_srcdoc_runs_scripts_same_origin_with_no_network_fetch() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(
            "<body><iframe srcdoc=\"<script>window.markedByChild = true;</script>\"></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    // No resolver and no network reachable: srcdoc must not attempt a fetch.
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); \
         frame.contentDocument.URL === 'about:srcdoc' && \
         frame.contentWindow.markedByChild === true"
    ).unwrap(), JsValue::Bool(true), "srcdoc uses about:srcdoc and runs inline scripts");
    // Same-origin as the parent (not opaque), so the parent can read into it.
    assert_eq!(scheduler.run_script(
        "frame.contentDocument.body.textContent !== undefined"
    ).unwrap(), JsValue::Bool(true), "srcdoc document readable from the parent realm");
}

#[test]
fn bare_iframe_with_no_src_is_about_blank_and_same_origin_with_its_parent() {
    // A same-origin, unsandboxed `<iframe>` with no `src`/`srcdoc` at all
    // navigates to `about:blank`, which is same-origin with whatever
    // created it -- not opaque -- so `contentWindow` access must not
    // trip JSRT-9's cross-origin WindowProxy gate (`ensure_realm` used to
    // special-case only `about:srcdoc`, leaving `about:blank` to fall
    // through to a mismatched origin from `ParsedUrl::parse`).
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe></iframe></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); \
         frame.contentWindow.document === frame.contentDocument && \
         frame.contentDocument.URL === 'about:blank'"
    ).unwrap(), JsValue::Bool(true), "bare iframe's contentWindow is reachable and about:blank");
}

/// HTML-34: the actual bug this ticket's item 1 fixes -- before, an
/// iframe's document was fully built (`import_inert_document`, one-shot)
/// BEFORE any of its own scripts ran, so a script could always see its
/// own later siblings already in the tree. With `run_interleaved_iframe_
/// parse`, a script must see only what a real parser would have built
/// by the time it reaches that `<script>` tag: `<div id=a>` yes,
/// `<div id=b>` no (not parsed yet).
#[test]
fn iframe_interleaved_script_sees_document_so_far_not_the_finished_tree() {
    let path = std::env::temp_dir()
        .join(format!("amazing-iframe-interleave-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><div id=a></div><script>\
         window.sawBBeforeItExisted = (document.getElementById('b') === null);\
         </script><div id=b></div></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe src='child.html'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); frame.contentWindow.sawBBeforeItExisted"
    ).unwrap(), JsValue::Bool(true), "script ran before its own later sibling was parsed");
    // And by the time the (now finished) parse reached `Eof`, `#b` exists.
    assert_eq!(scheduler.run_script(
        "frame.contentDocument.getElementById('b') !== null"
    ).unwrap(), JsValue::Bool(true), "later sibling exists once parsing has finished");
    std::fs::remove_file(&path).unwrap();
}

/// HTML-34 item 2: `localStorage`/`sessionStorage` must be keyed by the
/// CALLING realm's own origin, not the top-level page's -- before this
/// ticket's `document_url` scoping fix, `Storage::storage_key()` always
/// read the single, host-wide `document_url` field, so a cross-origin
/// iframe's own script would have silently written into the PARENT's
/// storage area instead of its own. A same-origin case can't tell the
/// fix apart from the bug (both resolve to the same origin either way),
/// so this uses a genuinely cross-origin `src` and checks for the
/// absence of cross-contamination from the parent's own script, which
/// needs no cross-origin `contentWindow` property access at all (that
/// stays gated, correctly, by JSRT-9 -- unrelated to this fix).
#[test]
fn iframe_localstorage_is_keyed_by_the_frames_own_origin_not_the_parents() {
    let path = std::env::temp_dir()
        .join(format!("amazing-iframe-storage-origin-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><script>localStorage.setItem('cross-origin-leak', 'leaked');</script></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe src='https://other.test/child.html'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script("localStorage.getItem('cross-origin-leak')").unwrap(),
        JsValue::Null, "baseline: nothing there yet");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("localStorage.getItem('cross-origin-leak')").unwrap(),
        JsValue::Null, "a cross-origin child's write must not land in the parent's own storage area");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn iframe_sandbox_without_allow_same_origin_is_cross_origin_from_parent() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><script>window.markedByChild = true;</script></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe sandbox='allow-scripts' src='child.html'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    // Without `allow-same-origin` the child gets a fresh opaque origin
    // every navigation, so it is cross-origin from the parent even
    // though both come from the same URL origin: `contentDocument` is
    // null, and an arbitrary property read through `contentWindow`
    // (not on the small cross-origin allowlist) throws.
    assert_eq!(scheduler.run_script(
        "document.querySelector('iframe').contentDocument"
    ).unwrap(), JsValue::Null, "opaque-origin child has no contentDocument for the parent");
    assert!(scheduler.run_script(
        "document.querySelector('iframe').contentWindow.markedByChild"
    ).is_err(), "arbitrary property read across an opaque origin throws");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn iframe_sandbox_allow_same_origin_without_allow_scripts_blocks_scripts() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-so-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><script>window.markedByChild = true;</script></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe sandbox='allow-same-origin' src='child.html'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    // `allow-same-origin` alone keeps the child visible to the parent
    // but does not itself permit scripting; only `allow-scripts` does.
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); \
         frame.contentDocument !== null && frame.contentWindow.markedByChild"
    ).unwrap(), JsValue::Undefined, "same-origin visible, but no script ran without allow-scripts");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn iframe_sandbox_allow_scripts_and_allow_same_origin_runs_scripts() {
    let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-scripts-{}.html", std::process::id()));
    std::fs::write(&path,
        "<body><script>window.markedByChild = true;</script></body>").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(
            "<body><iframe sandbox='allow-scripts allow-same-origin' src='child.html'></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let resolved = path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script(
        "var frame = document.querySelector('iframe'); frame.contentWindow.markedByChild"
    ).unwrap(), JsValue::Bool(true), "allow-scripts runs the child's scripts");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn form_target_navigates_named_iframe_instead_of_top_level() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(
            "<body><iframe name='result'></iframe>\
             <form action='https://example.test/submitted' target='result'></form></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "document.forms[0].submit(); \
         document.querySelector('iframe').getAttribute('src')"
    ).unwrap(), JsValue::str("https://example.test/submitted?"),
        "target matching a frame name navigates that frame, not top-level");
    assert_eq!(scheduler.run_script("location.href")
        .unwrap(), JsValue::str("https://example.test/main.html"), "top-level location is untouched");
}

#[test]
fn form_target_blank_still_navigates_top_level() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(
            "<body><iframe name='_blank'></iframe>\
             <form action='https://example.test/submitted' target='_blank'></form></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("document.forms[0].submit();").unwrap();
    // `_blank` is a special keyword, never a frame name match, even when
    // (invalidly) reused as one.
    assert_eq!(scheduler.run_script("document.querySelector('iframe').getAttribute('src')")
        .unwrap(), JsValue::Null, "the '_blank' keyword never matches a frame name");
    assert_eq!(scheduler.run_script("location.href")
        .unwrap(), JsValue::str("https://example.test/submitted?"), "falls through to top-level navigation");
}

#[test]
fn document_write_runs_its_inserted_inline_script_synchronously_and_in_order() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    // Matches WPT's `execution-timing` idiom: a script that calls
    // `document.write` with an embedded `<script>` observes that script
    // run before `write()` returns, not merely at some later point.
    assert_eq!(scheduler.run_script(
        "var order = []; order.push('before'); \
         document.write('<body><script>order.push(\"written\")<\\/script>'); \
         order.push('after'); order.join(',')"
    ).unwrap(), JsValue::str("before,written,after"));
    // Explicit body markup preserves this test's body placement intent:
    // a script alone at the start of a reopened document belongs in head.
    // The written markup is really inserted into the document, not just
    // executed and discarded.
    assert_eq!(scheduler.run_script("document.body.children.length")
        .unwrap(), JsValue::num(1.0));
}

#[test]
fn document_writeln_appends_a_newline_and_write_does_not() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "document.write('a'); document.writeln('b'); document.write('c'); \
         document.body.textContent"
    ).unwrap(), JsValue::str("ab\nc"));
}

#[test]
fn document_open_and_close_do_not_throw_and_open_returns_the_document() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script("document.open() === document")
        .unwrap(), JsValue::Bool(true));
    assert_eq!(scheduler.run_script("document.close()")
        .unwrap(), JsValue::Undefined);
}

#[test]
fn document_write_from_a_top_level_script_inserts_at_the_parser_insertion_point() {
    // HTML-29: a classic top-level script's `document.write` inserts
    // right after that script's own position (the real parser's
    // insertion point while it is the one running), not always at the
    // end of the body the way HTML-28's implementation did -- so
    // content written by a script between two elements ends up between
    // them, not after both. Exercised through `prepare_page_scripts`
    // (cmd/js.rs), the same top-level script-phase wiring a real page
    // load uses (`begin_document_parsing`/`narrow_write_insertion_point`).
    // HTML-32: `prepare_page_scripts` now parses `html` itself through
    // the interleaved parser, so `host` starts with an empty `Dom` and
    // the source that used to be pre-parsed via `amazing_html::parse`
    // is passed straight through as its new `html` argument instead.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(),
        native_forwarded: false,
    }));
    let mut scheduler = crate::cmd::js::prepare_page_scripts(
        &host,
        "https://example.test/main.html",
        "<body><div id=a></div><script>document.write('<div id=written></div>')</script><div id=b></div></body>",
        false,
    ).unwrap().unwrap();
    // The writing `<script>` element itself stays in the tree (as an
    // empty-id element between "a" and "written") -- only the *written*
    // content lands at its insertion point, right after it and before
    // "b", not at the end of the body.
    assert_eq!(scheduler.run_script(
        "Array.from(document.body.children).map(e => e.id).join(',')"
    ).unwrap(), JsValue::str("a,,written,b"));
}

#[test]
fn document_write_after_load_implicitly_reopens_the_document() {
    // HTML-29: once a document's top-level script phase has finished and
    // its load event fired, `end_document_parsing` has cleared its
    // marker, so a later `document.write` (the way a `setTimeout`
    // callback firing after load would call it) implicitly performs
    // `document.open()`'s full-document replace instead of appending
    // next to whatever content the page already had.
    // HTML-32: same reshaping as the test above -- `host` starts empty
    // and the source is passed as `prepare_page_scripts`'s `html` arg.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(),
        native_forwarded: false,
    }));
    let mut scheduler = crate::cmd::js::prepare_page_scripts(
        &host,
        "https://example.test/main.html",
        "<body><div id=original></div></body>",
        true,
    ).unwrap().unwrap();
    scheduler.run_script("document.write('hello')").unwrap();
    assert_eq!(scheduler.run_script("document.body.textContent").unwrap(), JsValue::str("hello"));
    assert_eq!(scheduler.run_script("document.getElementById('original')").unwrap(), JsValue::Null);
}

/// HTML-31: unlike `prepare_page_scripts`'s tests above (which start from
/// an already-fully-parsed DOM, since that function's own callers parse
/// upfront), this drives the real, script-interleaved parser
/// (`crate::cmd::js::run_interleaved_parse`, wired into
/// `cmd/wpt_harness.rs::execute_page` for the measured WPT corpora) --
/// so a script's own later siblings genuinely do not exist yet when it
/// runs, matching a real browser and fixing the architectural gap
/// `dom/nodes/MutationObserver-document.html` exposed.
#[test]
fn interleaved_parse_runs_a_script_against_the_dom_built_so_far_not_the_finished_document() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    let mut seen_before_later_sibling_exists = None;
    crate::cmd::js::run_interleaved_parse(
        &host, &mut scheduler,
        "<body><div id=a></div><script>var probe = 1;</script><div id=b></div></body>",
        |scheduler, script| {
            seen_before_later_sibling_exists = Some(
                scheduler.run_script("document.getElementById('b') === null").unwrap()
            );
            scheduler.run_script(&script.body).map_err(|e| e.describe())?;
            Ok(())
        },
    ).unwrap();
    assert_eq!(seen_before_later_sibling_exists, Some(JsValue::Bool(true)));
    // The full document is there once the interleaved parse returns.
    assert_eq!(scheduler.run_script("document.getElementById('b') !== null").unwrap(), JsValue::Bool(true));
}

#[test]
fn parser_created_frames_have_contexts_before_scripts_and_keep_loads_deferred() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
        r#"<!doctype html><body><iframe id=bare></iframe><iframe id=same sandbox="allow-same-origin"></iframe><iframe id=opaque sandbox></iframe><script>
            var bare=document.getElementById('bare'),same=document.getElementById('same'),opaque=document.getElementById('opaque'),loads=0;
            bare.addEventListener('load',()=>loads++);
            if(!bare.contentWindow || bare.contentWindow.Document===Document || bare.contentWindow.Array===Array) throw new Error('missing child realm');
            if(bare.contentWindow.document!==bare.contentDocument || bare.contentDocument.URL!=='about:blank') throw new Error('blank identity');
            if(bare.contentWindow.parent!==window || bare.contentWindow.top!==window || same.contentDocument===null) throw new Error('ancestry/origin');
            if(!opaque.contentWindow || opaque.contentDocument!==null) throw new Error('sandbox context');
            var dynamic=document.createElement('iframe');document.body.appendChild(dynamic);
            if(!dynamic.contentWindow.Document) throw new Error('dynamic context');
            dynamic.remove();if(dynamic.contentWindow!==null || dynamic.contentDocument!==null) throw new Error('removed context');
            if(loads!==0) throw new Error('early load');
        </script>"#,
        |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|error| error.describe()),
    ).unwrap();
    assert_eq!(scheduler.run_script("loads").unwrap(), JsValue::num(0.0));
    scheduler.dispatch_pending_iframe_loads().unwrap();
}

#[test]
fn parser_context_refresh_preserves_author_vars_and_mutation_observer_targets() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
        r#"<!doctype html><body><p id=n10>abc</p><div id=n00></div><script>
            var n10=document.getElementById('n10').firstChild;
            var n00=document.getElementById('n00');
            var observer=new MutationObserver(function(){});
            observer.observe(n10,{characterData:true});
            observer.observe(n00,{attributes:true});
            n10.appendData('d');n10.insertData(0,'x');n10.deleteData(0,1);n10.replaceData(0,1,'A');
            n00.id='foo';
            var records=observer.takeRecords();
            if(n10.data!=='Abcd' || n00.id!=='foo' || records.length!==5) throw new Error('author vars or mutation targets replaced');
            if(records[0].target!==n10 || records[4].target!==n00) throw new Error('mutation target identity');
        </script>"#,
        |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|error| error.describe()),
    ).unwrap();
}

#[test]
fn nested_srcdoc_parser_allocates_its_frames_before_child_scripts() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
        r#"<!doctype html><body><iframe id=outer srcdoc="&lt;iframe id=inner&gt;&lt;/iframe&gt;&lt;script&gt;var child=document.getElementById('inner').contentWindow;window.probe=!!child.Document &amp;&amp; child.Document!==Document &amp;&amp; child.document===document.getElementById('inner').contentDocument;&lt;/script&gt;"></iframe><script>
            var outer=document.getElementById('outer');
            if(!outer.contentWindow.Document || outer.contentDocument.URL!=='about:srcdoc') throw new Error('srcdoc context');
            if(typeof outer.contentWindow.probe!=='undefined') throw new Error('child ran before pending load');
        </script>"#,
        |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|error| error.describe()),
    ).unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("outer.contentWindow.probe").unwrap(), JsValue::Bool(true));
}

/// The real per-spec behavior HTML-29 approximated with a DOM splice:
/// `document.write` while paused at a blocking script inserts into the
/// input stream at the real tokenizer position, so the written markup
/// (and any further source after the script) both get tokenized and
/// built the normal way, in the right order.
#[test]
fn interleaved_parse_document_write_inserts_at_the_real_tokenizer_position() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    crate::cmd::js::run_interleaved_parse(
        &host, &mut scheduler,
        "<body><div id=a></div><script>document.write('<div id=written></div>')</script><div id=b></div></body>",
        |scheduler, script| { scheduler.run_script(&script.body).map_err(|e| e.describe())?; Ok(()) },
    ).unwrap();
    assert_eq!(
        scheduler.run_script("Array.from(document.body.children).map(e => e.id).join(',')").unwrap(),
        JsValue::str("a,,written,b"),
    );
}

/// `defer` only exempts a script with a `src` from blocking (per spec it
/// is ignored on an inline script, which still blocks like any other);
/// `async` exempts either. Neither ever pauses construction; both run
/// after the whole document reaches `Eof`, in document order -- unlike
/// the blocking script above, which runs synchronously, mid-parse.
#[test]
fn interleaved_parse_defer_and_async_scripts_run_after_eof_in_document_order() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    let mut ran = Vec::new();
    // Only the parser-blocking script pauses construction and runs
    // through this closure; `defer`/`async` scripts are returned instead
    // (see below), never invoking it -- exactly the assertion this test
    // makes.
    let deferred = crate::cmd::js::run_interleaved_parse(
        &host, &mut scheduler,
        "<body><script src=deferred.js defer></script>\
         <script src=async.js async></script>\
         <script>1</script></body>",
        |_scheduler, script| {
            ran.push(script.src.clone().unwrap_or_else(|| "blocking".to_owned()));
            Ok(())
        },
    ).unwrap();
    for script in &deferred {
        ran.push(script.src.clone().unwrap_or_else(|| "blocking".to_owned()));
    }
    assert_eq!(ran, vec!["blocking", "deferred.js", "async.js"]);
}

/// An inline script's own `defer` attribute is ignored (no `src`): it
/// still pauses construction like any other classic script, unlike the
/// external `defer`/`async` cases above.
#[test]
fn interleaved_parse_inline_defer_without_src_still_blocks() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_dom::Dom::new(), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    let mut ran = Vec::new();
    crate::cmd::js::run_interleaved_parse(
        &host, &mut scheduler,
        "<body><script defer>1</script></body>",
        |_scheduler, script| { ran.push(script.body.clone()); Ok(()) },
    ).unwrap();
    assert_eq!(ran, vec!["1"], "inline defer (no src) must block like an ordinary script");
}

// HTML-35: "prepare the script element" for non-parser-inserted scripts.

#[test]
fn dynamic_script_runs_once_on_connection_and_never_again_on_reinsertion() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = 0; \
         var el = document.createElement('script'); \
         el.textContent = 'window.ran += 1;'; \
         document.body.appendChild(el); \
         document.body.removeChild(el); \
         document.body.appendChild(el); \
         window.ran"
    ).unwrap(), JsValue::num(1.0), "already-started must survive removal and reinsertion");
}

#[test]
fn dynamic_script_inserted_via_inner_html_never_executes_even_if_moved_out_afterward() {
    // WHATWG HTML: the fragment-parsing algorithm behind the `innerHTML`
    // setter marks every script element it produces already-started,
    // unconditionally -- unlike a `createElement`-built script, it never
    // gets to run, however it is later moved.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = 0; \
         document.body.innerHTML = '<script>window.ran = 1;<\\/script>'; \
         window.ran"
    ).unwrap(), JsValue::num(0.0), "innerHTML-produced scripts must not execute");
    // Moving it into a fresh connection point afterward still must not
    // run it -- the flag, not the location, is what blocks it.
    assert_eq!(scheduler.run_script(
        "var s = document.body.querySelector('script'); \
         document.body.removeChild(s); \
         document.body.appendChild(s); \
         window.ran"
    ).unwrap(), JsValue::num(0.0), "already-started must stick even once moved out and back in");
}

#[test]
fn template_inner_html_produced_scripts_never_execute_even_after_moving_content_out() {
    // Follow-up from HTML-35: `template.innerHTML`'s fragment-parsing
    // result lands in the template's own `.content`, a detached
    // fragment `connected_script_nodes`'s before/after diff never
    // walks -- must still be marked already-started, or moving the
    // content into a real, connected document later would wrongly run
    // it.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><template id=t></template></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = 0; \
         document.getElementById('t').innerHTML = '<script>window.ran = 1;<\\/script>'; \
         document.body.appendChild(document.getElementById('t').content); \
         window.ran"
    ).unwrap(), JsValue::num(0.0), "template content scripts must not execute once moved into a live document");
}

#[test]
fn dom_parser_parse_from_string_produced_scripts_never_execute_even_after_adoption() {
    // Follow-up from HTML-35: a `DOMParser`-produced document has no
    // browsing context (already blocked from running while it stays
    // orphaned by `has_active_document_root`), but a script from it
    // can still be adopted into a real, connected document -- must be
    // marked already-started at creation time so it never runs even
    // then.
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = 0; \
         var doc = new DOMParser().parseFromString('<script>window.ran = 1;<\\/script>', 'text/html'); \
         var s = doc.querySelector('script'); \
         document.adoptNode(s); \
         document.body.appendChild(s); \
         window.ran"
    ).unwrap(), JsValue::num(0.0), "DOMParser-produced scripts must not execute even once adopted and connected");
}

#[test]
fn dynamic_script_type_and_nomodule_filtering_matches_is_javascript_mime_type() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = []; \
         function make(type, nomodule) { \
           var s = document.createElement('script'); \
           if (type !== undefined) s.type = type; \
           if (nomodule) s.setAttribute('nomodule', ''); \
           s.textContent = 'window.ran.push(\"' + (type || 'default') + '\");'; \
           document.body.appendChild(s); \
         } \
         make(undefined, false); \
         make('application/javascript', false); \
         make('module', false); \
         make('text/plain', false); \
         make('text/javascript', true); \
         window.ran.join(',')"
    ).unwrap(), JsValue::str("default,application/javascript,module"),
        "classic and module scripts run; data blocks and classic nomodule fallbacks are skipped");
}

#[test]
fn dynamic_script_with_external_src_fetches_fires_load_and_runs_in_order() {
    let script_path = std::env::temp_dir()
        .join(format!("amazing-html35-ext-{}.js", std::process::id()));
    std::fs::write(&script_path, "window.extRan = true;").unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let path = script_path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "var events = []; \
         var s = document.createElement('script'); \
         s.src = 'https://example.test/ext.js'; \
         s.addEventListener('load', () => events.push('load')); \
         s.addEventListener('error', () => events.push('error')); \
         document.body.appendChild(s); \
         events.join(',') + '|' + window.extRan"
    ).unwrap(), JsValue::str("load|true"));
    std::fs::remove_file(&script_path).unwrap();
}

#[test]
fn dynamic_script_with_failing_external_src_fires_error_and_never_runs() {
    let missing_path = std::env::temp_dir()
        .join(format!("amazing-html35-missing-{}.js", std::process::id()));
    let _ = std::fs::remove_file(&missing_path); // ensure it does not exist
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    let path = missing_path.clone();
    scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "var events = []; window.extRan2 = false; \
         var s = document.createElement('script'); \
         s.src = 'https://example.test/missing.js'; \
         s.addEventListener('load', () => events.push('load')); \
         s.addEventListener('error', () => events.push('error')); \
         document.body.appendChild(s); \
         events.join(',') + '|' + window.extRan2"
    ).unwrap(), JsValue::str("error|false"));
}

#[test]
fn dynamic_script_mutating_text_or_src_after_it_already_ran_does_not_rerun() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    assert_eq!(scheduler.run_script(
        "window.ran = 0; \
         var s = document.createElement('script'); \
         s.textContent = 'window.ran += 1;'; \
         document.body.appendChild(s); \
         s.textContent = 'window.ran += 100;'; \
         s.src = 'https://example.test/should-not-fetch.js'; \
         document.body.appendChild(document.createElement('div')); \
         window.ran"
    ).unwrap(), JsValue::num(1.0),
        "textContent/src set after a script already ran must not re-trigger it");
}
fn synchronous_write_fixture() -> &'static str {
    r#"<!doctype html><body><script>
        window.syncPass = true;
        document.write('<div id="written">&am');
        syncPass = syncPass && !!document.getElementById('written')
            && document.getElementById('written').textContent === '';
        document.write('p;</div>');
        syncPass = syncPass && document.getElementById('written').textContent === '&';
        document.write('<scr' + 'ipt>document.write("<i id=inner>inner</i>");'
            + 'window.nestedVisible=!!document.getElementById("inner");</scr' + 'ipt><b id=outer>outer</b>');
        syncPass = syncPass && nestedVisible && !!document.getElementById('outer')
            && document.getElementById('tail') === null;
    </script><p id=tail>tail</p>"#
}

#[test]
fn active_top_level_write_is_visible_before_return_and_preserves_tail() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    crate::cmd::js::run_interleaved_parse(&host, &mut scheduler, synchronous_write_fixture(),
        |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|e| format!("{e:?}"))).unwrap();
    assert_eq!(scheduler.run_script("syncPass && !!document.getElementById('tail')").unwrap(), JsValue::Bool(true));
}

#[test]
fn active_iframe_write_grafts_synchronously_and_nested_writes_keep_identity() {
    let mut dom = amazing_html::parse("<!doctype html><iframe></iframe>");
    let frame = dom.children(dom.document_body(NodeIdx(0)).unwrap().unwrap()).next().unwrap();
    dom.set_attribute(frame, "srcdoc", synchronous_write_fixture()).unwrap();
    let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/main.html");
    scheduler.run_script("void document.querySelector('iframe')").unwrap();
    scheduler.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(scheduler.run_script("document.querySelector('iframe').contentWindow.syncPass && !!document.querySelector('iframe').contentDocument.getElementById('tail')").unwrap(), JsValue::Bool(true));
}

#[test]
fn selection_context_helpers_preserve_actual_associations_and_receiver_policy() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
        "<!doctype html><body><iframe id=same></iframe><iframe id=opaque sandbox='allow-scripts'></iframe>"), native_forwarded: false }));
    let mut js = Interpreter::new();
    js.set_dom_host(host.clone());
    let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
    bindings.set_document_url(&mut js, "https://example.test/page.html");
    let root_window = js.binding_global_this();
    let root_origin = bindings.state.borrow().frames.document_context(&host.borrow().dom, NodeIdx(0)).unwrap().origin;
    let children = bindings.state.borrow().frames.contexts.values()
        .filter(|context| context.frame.is_some()).cloned().collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    let same = children.iter().find(|context| context.fetch_origin == root_origin).unwrap().clone();
    let opaque = children.iter().find(|context| context.fetch_origin != root_origin).unwrap().clone();
    let inert = host.borrow_mut().dom.create_document();
    host.borrow_mut().dom.set_document_ready_state(NodeIdx(0), ReadyState::Loading).unwrap();
    let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
    assert!(adapter.html_document_has_browsing_context(NodeIdx(0)));
    assert!(adapter.html_document_has_browsing_context(same.document));
    assert!(!adapter.html_document_has_browsing_context(inert));
    assert!(!adapter.html_document_has_browsing_context(NodeIdx(u32::MAX)));
    assert_eq!(adapter.html_window_document_for_selection(&root_window).unwrap(), NodeIdx(0));
    assert_eq!(adapter.html_window_document_for_selection(&same.window).unwrap(), same.document);
    assert_eq!(adapter.html_window_document_for_selection(&same.global).unwrap(), same.document);
    for fake in [JsValue::Undefined, JsValue::Null, JsValue::num(1.0)] {
        assert!(adapter.html_window_document_for_selection(&fake).is_err());
    }
    adapter.js.run_script("Object.defineProperty(window,'DOMException',{configurable:true,get(){throw new Error('author constructor getter entered')}})").unwrap();
    let error = adapter.html_window_document_for_selection(&opaque.window).unwrap_err();
    assert_eq!(adapter.js.binding_get_property(&error.0, "name").unwrap(), JsValue::str("SecurityError"));
    assert_eq!(adapter.js.binding_get_property(&error.0, "code").unwrap(), JsValue::num(18.0));
    // Generation equality is required even while the context's Window identity remains registered.
    let realm = *adapter.state.borrow().frames.contexts.iter()
        .find(|(_, context)| context.document == same.document).unwrap().0;
    adapter.state.borrow_mut().frames.contexts.get_mut(&realm).unwrap().document_generation += 1;
    assert!(!adapter.html_document_has_browsing_context(same.document));
    assert!(adapter.html_window_document_for_selection(&same.window).is_err());
    adapter.state.borrow_mut().frames.contexts.get_mut(&realm).unwrap().document_generation -= 1;
    drop(adapter);
    // A root caller executing a borrowed child-realm operation gets the callee's exception prototype.
    let state = bindings.state.clone();
    let native = js.binding_with_realm(realm, |js| {
        let normal = crate::runtime_binding_host::build_host_native_dispatcher(state.clone());
        let state = state.clone();
        js.set_host_native_dispatcher(Rc::new(move |js, id, receiver, args, new_target| {
            if id != HostFunctionId(u64::MAX) { return normal(js, id, receiver, args, new_target); }
            let mut adapter = RuntimeBindingHost { js, state: state.clone() };
            adapter.html_window_document_for_selection(&receiver).map(|document| JsValue::num(document.0 as f64))
        }));
        Ok(js.binding_create_native_function(HostFunctionId(u64::MAX), "selectionBorrowed", 0, JsCallPolicy::CallOnly))
    }).unwrap();
    js.binding_define_data_property(&root_window, "selectionBorrowed", native, BindingPropertyFlags { writable: true, enumerable: true, configurable: true }).unwrap();
    js.binding_define_data_property(&root_window, "selectionReceiver", opaque.window.clone(), BindingPropertyFlags { writable: true, enumerable: true, configurable: true }).unwrap();
    let denied = js.run_script("try { selectionBorrowed.call(selectionReceiver); throw new Error('access allowed'); } catch(error) { error; }").unwrap();
    let expected = bindings.state.borrow().interfaces[&realm].iter()
        .find(|(interface, _, _)| *interface == Interface::DOMException).unwrap().1.clone();
    let (JsValue::Object(denied), JsValue::Object(expected)) = (denied, expected) else { panic!("native DOMException objects"); };
    assert!(denied.borrow().proto.as_ref().is_some_and(|prototype| Rc::ptr_eq(&prototype.0, &expected.0)));
    js.run_script("document.getElementById('same').remove()").unwrap();
    bindings.navigate_iframes(&mut js);
    let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
    assert!(!adapter.html_document_has_browsing_context(same.document));
    assert_eq!(adapter.html_window_document_for_selection(&same.window).unwrap(), same.document);
    assert!(adapter.html_document_has_browsing_context(NodeIdx(0)));
}
