//! Driver adapter regression tests.
use crate::{dom_host::BrowserDomHost, page::load_page_text};
use amazing_js::DomHost;
use amazing_text::TextSystem;
use std::{cell::RefCell, rc::Rc};

#[test]
fn focus_updates_active_element_and_dispatches_ordered_focus_events() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><input id='a'><input id='b' disabled><div id='c' tabindex='-1'></div></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    let result = scheduler.run_script(
        "let a=document.getElementById('a'), b=document.getElementById('b'), c=document.getElementById('c'); \
         let events=[]; for (let el of [a,c]) for (let type of ['focus','focusin','blur','focusout']) \
         el.addEventListener(type, function(e) { events.push(e.type+':'+this.id+':'+(e.relatedTarget&&e.relatedTarget.id)); }); \
         a.focus(); let first=document.activeElement===a; b.focus(); let disabled=document.activeElement===a; \
         c.focus({preventScroll:true,focusVisible:true}); let second=document.activeElement===c; \
         c.blur(); [first,disabled,second,document.activeElement===document.body,events.join(',')].join('|')"
    ).unwrap();
    assert_eq!(result, amazing_js::value::JsValue::str(
        "true|true|true|true|focus:a:null,focusin:a:null,blur:a:c,focusout:a:c,focus:c:a,focusin:c:a,blur:c:null,focusout:c:null"));
}

#[test]
fn detached_created_element_has_live_style() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><div id='content'></div></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    assert_eq!(scheduler.run_script("document.compatMode").unwrap(),
        amazing_js::value::JsValue::str("CSS1Compat"));
    assert_eq!(scheduler.run_script(
        "let cell = document.createElement('td'); let style = cell.style; \
         style.cssText = 'font-variant-ligatures: normal'; \
         document.getElementById('content').appendChild(cell); \
         cell.style === style && cell.getAttribute('style') === 'font-variant-ligatures: normal'"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn constructed_document_has_xml_metadata_without_a_browsing_context() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    assert_eq!(scheduler.run_script(
        "let doc = new Document(); let xml = document.implementation.createDocument(null, 'root', null); \
         [doc instanceof Node, doc instanceof Document, doc instanceof XMLDocument, \
         xml instanceof XMLDocument, Object.getPrototypeOf(doc) === Document.prototype, \
         doc.URL, doc.documentURI, doc.contentType, doc.characterSet, doc.charset, \
         doc.inputEncoding, doc.compatMode, doc.location].join('|')"
    ).unwrap(), amazing_js::value::JsValue::str(
        "true|true|false|true|true|about:blank|about:blank|application/xml|UTF-8|UTF-8|UTF-8|CSS1Compat|"));
}

#[test]
fn location_exposes_live_url_components_navigation_and_ancestor_origins() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"), native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test:8443/a?x=1#old");
    assert_eq!(scheduler.run_script(
        "[location === window.location, document.location === location, location.href, location.origin, location.protocol, \
          location.host, location.hostname, location.port, location.pathname, location.search, \
          location.hash, location.toString(), location.ancestorOrigins.length, \
          location.ancestorOrigins.item(0), location.ancestorOrigins.contains('x'), \
          location.ancestorOrigins === location.ancestorOrigins, \
          Object.getOwnPropertyDescriptor(location, 'origin').configurable, \
          Object.getOwnPropertyDescriptor(window, 'location').configurable, \
          Object.getOwnPropertyDescriptor(document, 'location').configurable].join('|')"
    ).unwrap(), amazing_js::value::JsValue::str(
        "true|true|https://example.test:8443/a?x=1#old|https://example.test:8443|https:|example.test:8443|example.test|8443|/a|?x=1|#old|https://example.test:8443/a?x=1#old|0||false|true|false|false|false"));
    assert_eq!(scheduler.run_script(
        "location.protocol = 'http:'; location.host = 'other.test:8080'; \
         location.hostname = 'third.test'; location.port = '9000'; location.pathname = '/b'; \
         location.search = '?y=2'; location.hash = '#new'; location.href"
    ).unwrap(), amazing_js::value::JsValue::str("http://third.test:9000/b?y=2#new"));
    assert_eq!(scheduler.run_script(
        "window.location = '/next'; location.href"
    ).unwrap(), amazing_js::value::JsValue::str("http://third.test:9000/next"));
    assert_eq!(scheduler.run_script(
        "document.location = '/from-document'; location.href"
    ).unwrap(), amazing_js::value::JsValue::str("http://third.test:9000/from-document"));
    assert_eq!(scheduler.run_script(
        "location.assign('after'); location.replace('/replaced'); location.reload(); location.href"
    ).unwrap(), amazing_js::value::JsValue::str("http://third.test:9000/replaced"));
    assert_eq!(scheduler.js.take_navigation(), Some("http://third.test:9000/replaced".into()));
}

#[test]
fn put_forwards_assignments_keep_live_token_lists_and_location() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><a id='a'></a><form id='f'></form><output id='o'></output><iframe id='i'></iframe><link id='l'></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    scheduler.js.set_location_url("https://example.test/start");
    assert_eq!(scheduler.run_script(
        "let a = document.getElementById('a'), f = document.getElementById('f'), o = document.getElementById('o'), i = document.getElementById('i'), l = document.getElementById('l'); \
         let r = a.relList, s = i.sandbox, h = o.htmlFor, z = l.sizes; \
         a.relList = 'noopener noreferrer'; f.relList = 'external'; o.htmlFor = 'one two'; i.sandbox = 'allow-scripts'; l.sizes = '16x16'; \
         let live = a.relList === r && r.value === 'noopener noreferrer' && f.relList.value === 'external' && \
             o.htmlFor === h && h.value === 'one two' && i.sandbox === s && s.value === 'allow-scripts' && \
             l.sizes === z && z.value === '16x16'; \
         let style = a.style; a.style = 'color: red;'; \
         location = 'https://example.test/next'; let bareLocation = location.href; \
         window.location = 'https://example.test/final'; \
         function shadowLocation() { let location = 'local'; location = 'still local'; return location; } \
         [live, a.relList === r, f.relList.value, o.htmlFor === h, i.sandbox === s, l.sizes === z, a.style === style, style.cssText, bareLocation, window.location.href, shadowLocation()].join('|')"
    ).unwrap(), amazing_js::value::JsValue::str("true|true|external|true|true|true|true|color: red;|https://example.test/next|https://example.test/final|still local"));
}

#[test]
fn bare_window_event_target_calls_coerce_nullish_this() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    assert_eq!(scheduler.run_script(
        "window instanceof EventTarget && Object.getPrototypeOf(window) === EventTarget.prototype && \
         !window.hasOwnProperty('addEventListener') && !window.hasOwnProperty('removeEventListener')"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
    assert_eq!(scheduler.run_script(
        "let hit = 0; function one() { hit += 1; } function two() { hit += 10; } \
         addEventListener('load', one); addEventListener.call(undefined, 'load', two); \
         addEventListener.call(null, 'load', one); \
         removeEventListener.call(null, 'load', one); \
         typeof hit"
    ).unwrap(), amazing_js::value::JsValue::str("number"));
    scheduler.dispatch_dom_event(0, "load").unwrap();
    assert_eq!(scheduler.run_script("hit").unwrap(), amazing_js::value::JsValue::num(10.0));
    assert_eq!(scheduler.run_script(
        "let rejected = false; try { addEventListener.call({}, 'load', one); } catch (error) { rejected = true; } rejected"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn dom_mixin_unscopables_are_on_the_declaring_prototype() {
    let host = std::rc::Rc::new(std::cell::RefCell::new(crate::dom_host::BrowserDomHost {
        dom: amazing_dom::Dom::new(), native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host, crate::js_scheduler::JS_CAPACITY).unwrap();
    assert_eq!(scheduler.run_script(
        "let u = Element.prototype[Symbol.unscopables]; \
         Object.getPrototypeOf(u) === null && u.before && u.after && u.replaceWith && \
         u.remove && u.prepend && u.append && u.replaceChildren && u.slot && \
         !Object.getOwnPropertyDescriptor(Element.prototype, Symbol.unscopables).writable && \
         Document.prototype[Symbol.unscopables].prepend && \
         DocumentFragment.prototype[Symbol.unscopables].append && \
         CharacterData.prototype[Symbol.unscopables].remove && \
         DocumentType.prototype[Symbol.unscopables].before"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn dom_parser_documents_share_the_arena_without_running_scripts() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://example.test/page");
    for (label, script) in [
        ("constructor", "var parser = new DOMParser();"),
        ("HTML", "var html = parser.parseFromString('<title>inert</title><script>window.ran = true</script><p id=inside>ok</p>', 'text/html');"),
        ("XML", "var xml = parser.parseFromString('<root xmlns=\"urn:x\"><child/></root>', 'application/xml');"),
        ("XML error", "var bad = parser.parseFromString('<root>', 'application/xml');"),
    ] {
        scheduler.run_script(script).unwrap_or_else(|error| panic!("DOMParser {label}: {}", error.describe()));
    }
    assert_eq!(scheduler.run_script(
        "let rejected = false; try { parser.parseFromString('x', 'text/plain'); } catch (error) { rejected = true; } \
         html !== document && html.URL === 'https://example.test/page' && html.contentType === 'text/html' && \
         html.characterSet === 'UTF-8' && html.readyState === 'complete' && html.location === null && html.getElementById('inside').textContent === 'ok' && \
         window.ran === undefined && xml.contentType === 'application/xml' && xml.documentElement.namespaceURI === 'urn:x' && \
         bad.documentElement.localName === 'parsererror' && rejected"
    ).unwrap_or_else(|error| panic!("DOMParser script: {}", error.describe())), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn xhr_inert_document_hook_decodes_xml_and_reports_parse_failure() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
    let document = bindings.create_inert_document(&mut js, b"<root>caf\xe9</root>",
        "application/xml", "https://example.test/response", Some("windows-1252")).unwrap();
    let borrowed = host.borrow();
    let dom = &borrowed.dom;
    assert_eq!(dom.document_metadata(document).unwrap().unwrap().character_set, "windows-1252");
    assert_eq!(dom.document_metadata(document).unwrap().unwrap().url, "https://example.test/response");
    let root = dom.document_element(document).unwrap().unwrap();
    assert_eq!(dom.local_name(root), Some("root"));
    assert_eq!(dom.character_data(dom.children(root).next().unwrap()).as_deref(), Some("café"));
    drop(borrowed);
    assert_eq!(bindings.create_inert_document(&mut js, b"<root>", "application/xml",
        "https://example.test/response", None), Err(amazing_dom::DomError::SyntaxError));
}

#[test]
fn form_collections_and_validity_use_native_brands_and_live_hooks() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<form id='f'><input id='a' name='choice' type='radio' value='one' checked><input id='b' name='choice' type='radio' value='two'><input id='required' required></form><select id='s'><option value='old'>old</option></select>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host, crate::js_scheduler::JS_CAPACITY,
    ).unwrap();
    let result = scheduler.run_script(
        "let f=document.getElementById('f'); let c=f.elements; \
         let a=document.getElementById('a'); let s=document.getElementById('s'); \
         let group=c.choice; let before=c.length===3 && c[0]===a && group instanceof RadioNodeList \
           && group.length===2 && group.value==='one' && c.required.validity.valueMissing; \
         group.value='two'; let chosen=group.value==='two'; \
         let replacement=document.createElement('option'); replacement.value='new'; \
         s.options[0]=replacement; \
         before && chosen && s.options instanceof HTMLOptionsCollection \
           && s.options[0]===replacement && s.options.length===1"
    ).unwrap();
    assert_eq!(result, amazing_js::value::JsValue::Bool(true));
}

#[test]
fn form_pattern_uses_v_mode_and_full_string_matching() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<input id='p' pattern='[a&&a]' value='b'>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host, crate::js_scheduler::JS_CAPACITY,
    ).unwrap();
    assert_eq!(scheduler.run_script(
        "let p=document.getElementById('p'); \
         let invalidValue=p.validity.patternMismatch; \
         p.value='a'; let matchingValue=!p.validity.patternMismatch; \
         p.setAttribute('pattern','a|ab'); p.value='ab'; let fullAlternative=!p.validity.patternMismatch; \
         p.setAttribute('pattern','['); let invalidPattern=!p.validity.patternMismatch; \
         (invalidValue ? 1 : 0) + (matchingValue ? 2 : 0) + \
         (fullAlternative ? 4 : 0) + (invalidPattern ? 8 : 0)"
    ).unwrap(), amazing_js::value::JsValue::num(15.0));
}

#[test]
fn custom_elements_registry_installs_and_when_defined_resolves() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host, crate::js_scheduler::JS_CAPACITY,
    ).unwrap();
    assert_eq!(scheduler.run_script(
        "function X(){}; let ready=false; \
         let pending=window.customElements.whenDefined('x-card'); \
         pending.then(value=>{ready=value===X}); \
         customElements.define('x-card',X); \
         customElements===window.customElements \
           && customElements instanceof CustomElementRegistry \
           && customElements.get('x-card')===X \
           && customElements.getName(X)==='x-card'"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
    scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
    assert_eq!(scheduler.run_script("ready").unwrap(), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn form_date_and_document_metadata_use_current_driver_state() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><input id='date' type='date'></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host, crate::js_scheduler::JS_CAPACITY,
    ).unwrap();
    scheduler.set_location_url("https://example.test/path/page.html");
    scheduler.set_document_metadata("UTF-8", "text/html", "https://referrer.test/");
    assert_eq!(scheduler.run_script(
        "let input=document.getElementById('date'); \
         input.valueAsDate=new Date(0); \
         let link=document.createElement('a'); link.href='../next'; \
         input.value==='1970-01-01' && input.valueAsDate instanceof Date \
           && document.URL==='https://example.test/path/page.html' \
           && document.characterSet==='UTF-8' && document.contentType==='text/html' \
           && document.referrer==='https://referrer.test/' \
           && link.href==='https://example.test/next'"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
}

#[test]
fn shadow_root_uses_its_interface_and_slotchange_reaches_the_event_loop() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><div id='host'></div></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(
        host,
        crate::js_scheduler::JS_CAPACITY,
    ).unwrap_or_else(|error| panic!("binding installation: {}", error.describe()));
    assert_eq!(scheduler.run_script(
        "let parent = document.getElementById('host'); \
         let shadow = parent.attachShadow({mode: 'open'}); \
         let slot = document.createElement('slot'); \
         let changes = 0; \
         slot.addEventListener('slotchange', () => { changes += 1; }); \
         shadow.appendChild(slot); \
         let child = document.createElement('span'); \
         parent.appendChild(child); \
         shadow instanceof ShadowRoot && parent.shadowRoot === shadow \
         && slot.assignedNodes()[0] === child"
    ).unwrap(), amazing_js::value::JsValue::Bool(true));
    scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
    assert_eq!(scheduler.run_script("changes").unwrap(), amazing_js::value::JsValue::num(1.0));
}

#[test]
fn binding_install_mutates_the_parsed_dom_and_keeps_child_nodes_live() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler =
        crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY)
            .unwrap();
    let result = scheduler
        .run_script(
            "let list = document.body.childNodes; let same = document.body.childNodes; \
         let text = new Text('hello'); document.body.appendChild(text); \
         list === same && list.length === 1 && text === list.item(0) && text === list[0]",
        )
        .unwrap();
    assert_eq!(result, amazing_js::value::JsValue::Bool(true));
    let body = host.borrow_mut().query_selector("body").unwrap();
    assert_eq!(
        host.borrow()
            .dom
            .children(amazing_dom::NodeIdx(body))
            .count(),
        1
    );
}

#[test]
fn binding_string_boundary_round_trips_lone_surrogate_without_replacement() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler =
        crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY)
            .unwrap();
    assert_eq!(
        scheduler
            .run_script("let text = new Text('ok'); document.body.appendChild(text); text.data",)
            .unwrap(),
        amazing_js::value::JsValue::str("ok")
    );
    let body = host.borrow_mut().query_selector("body").unwrap();
    let text = host
        .borrow()
        .dom
        .children(amazing_dom::NodeIdx(body))
        .next()
        .unwrap();
    assert_eq!(
        scheduler
            .run_script("text.data = String.fromCharCode(0xd800); text.data")
            .unwrap(),
        amazing_js::value::JsValue::str_units(vec![0xd800])
    );
    assert_eq!(host.borrow().dom.character_data_utf16(text), Some(vec![0xd800]));
}

#[test]
fn document_title_ignores_svg_title_elements() {
    let mut host = BrowserDomHost {
        dom: amazing_html::parse("<svg><title>foreign</title></svg>"),
        native_forwarded: false,
    };
    assert_eq!(
        <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
        ""
    );
    <BrowserDomHost as amazing_js::DomHost>::set_document_title(&mut host, "html title")
        .expect("set title");
    assert_eq!(
        <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
        "html title"
    );
}

#[test]
fn dom_host_set_attribute_queues_same_value_and_only_named_attribute() {
    let mut host = BrowserDomHost {
        dom: amazing_html::parse("<body><div id='same' class='old'></div></body>"),
        native_forwarded: false,
    };
    let element = amazing_dom::NodeIdx(host.query_selector("div").unwrap());
    let observer = host.dom.create_mutation_observer(1);
    host.dom
        .observe(
            observer,
            element,
            amazing_dom::mutation::ObserveOptions {
                attributes: Some(true),
                attribute_old_value: Some(true),
                ..Default::default()
            },
        )
        .unwrap();

    host.set_attribute(element.0, "id", "same").unwrap();
    host.set_attribute(element.0, "class", "new").unwrap();
    let records = host.dom.take_mutation_records(observer);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].attribute_name.as_deref(), Some("id"));
    assert_eq!(records[0].old_value.as_deref(), Some("same"));
    assert_eq!(records[1].attribute_name.as_deref(), Some("class"));
    assert_eq!(records[1].old_value.as_deref(), Some("old"));
}

#[test]
fn malformed_page_font_warns_without_advancing_font_generation() {
    let directory =
        std::env::temp_dir().join(format!("amazing-text-provider-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let font_path = directory.join("broken.woff2");
    std::fs::write(&font_path, [0u8; 64]).unwrap();
    let page_path = directory.join("page.html");
    let page = page_path.to_string_lossy();
    let baseline_generation = load_page_text("", &page).provider.generation();
    let text = load_page_text(
        "@font-face { font-family: Broken; src: url('broken.woff2') format('woff2') }",
        &page,
    );

    // CSS Fonts 4 §4.1: the declared author family shadows an installed
    // family even when all its sources fail; matching can use the next family.
    assert_eq!(text.provider.generation(), baseline_generation + 1);
    assert_eq!(text.provider.font_face_registrations().len(), 1);
    assert!(text.provider.font_face_registrations()[0].key.is_none());
    assert!(text.warnings.iter().any(|warning| warning.contains("broken.woff2")));
    let _ = std::fs::remove_dir_all(directory);
}
