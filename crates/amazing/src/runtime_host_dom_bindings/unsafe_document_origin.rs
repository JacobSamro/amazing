//! Actual static factory callee origins, including opaque same-agent realms.
use super::*;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

#[test]
fn unsafe_document_factories_snapshot_callee_origin_after_conversion_and_preserve_clones() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><iframe sandbox='allow-scripts' srcdoc=''></iframe></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/unsafe-factory-origin");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    let root_factory = scheduler.run_script("Document.parseHTMLUnsafe").unwrap();
    let state = scheduler.test_bindings().state.clone();
    let root_realm = scheduler.js.binding_current_realm();
    let (child_realm, child_document) = {
        let bindings = state.borrow();
        bindings.realm_documents.iter().find(|(realm, _)| **realm != root_realm)
            .map(|(realm, (document, _))| (*realm, *document)).unwrap()
    };
    let (root_origin, child_origin) = {
        let adapter = RuntimeBindingHost { js: &mut scheduler.js, state: state.clone() };
        (adapter.html_document_origin(NodeIdx(0)).unwrap(),
            adapter.html_document_origin(child_document).unwrap())
    };
    assert!(child_origin.opaque_id().is_some());
    assert_ne!(root_origin, child_origin);
    let documents = scheduler.js.binding_with_realm(child_realm, |js| {
        let global = js.binding_global_object();
        js.binding_define_data_property(&global, "foreignUnsafe", root_factory,
            BindingPropertyFlags { writable: true, enumerable: true, configurable: true })?;
        js.run_script(r#"(function() {
            const markup='<div id=h><template shadowrootmode=open><b>shadow</b></template></div>';
            let conversions=0;
            const own=Document.parseHTMLUnsafe(markup);
            const ownClone=own.cloneNode(true);
            const foreign=foreignUnsafe.call(Document,{toString(){
                conversions++; document.createElement('i'); return markup;
            }});
            const foreignClone=foreign.cloneNode(true);
            if(conversions!==1 || !(own instanceof Document) || !(ownClone instanceof Document)) throw 'factory matrix';
            for(const doc of [own,ownClone]) {
                doc.open(); doc.write(markup); doc.close();
                if(!doc.getElementById('h').shadowRoot) throw 'opaque reopen policy';
            }
            return [own,ownClone,foreign,foreignClone];
        })()"#)
    }).unwrap();
    let mut adapter = RuntimeBindingHost { js: &mut scheduler.js, state };
    for index in 0..4 {
        let value = adapter.get_property(&documents, &index.to_string()).unwrap();
        let document = adapter.node_brand(&value).unwrap();
        let expected = if index < 2 { child_origin.clone() } else { root_origin.clone() };
        assert_eq!(adapter.html_document_origin(document), Some(expected));
        let host = adapter.state.borrow().host.clone();
        let host = host.borrow();
        assert!(host.dom.document_allows_declarative_shadow_roots(document).unwrap());
        assert!(!host.dom.document_scripting_enabled(document).unwrap());
    }
}
