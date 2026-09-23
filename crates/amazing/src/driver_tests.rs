//! Driver adapter regression tests.
use crate::{dom_host::BrowserDomHost, page::load_page_text};
use amazing_js::DomHost;
use amazing_text::TextSystem;
use std::{cell::RefCell, rc::Rc};

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

    assert_eq!(text.provider.generation(), baseline_generation);
    assert_eq!(text.warnings.len(), 1);
    assert!(text.warnings[0].contains("broken.woff2"));
    let _ = std::fs::remove_dir_all(directory);
}
