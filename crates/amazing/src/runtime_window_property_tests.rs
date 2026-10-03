use super::*;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

#[test]
fn installed_main_and_child_windows_preserve_ordinary_names_and_document_accessors() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<!doctype html><body><iframe id=child></iframe>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
    scheduler.set_location_url("https://window.example.test/root");
    scheduler.run_script(r#"
        var body='ordinary', documentElement='ordinary-root';
        var child=document.getElementById('child').contentWindow;
        child.body='child'; child.documentElement='child-root';
    "#).unwrap();
    assert_eq!(scheduler.run_script(r#"
        body==='ordinary' && window.body==='ordinary' &&
        documentElement==='ordinary-root' && window.documentElement==='ordinary-root' &&
        document.body.tagName==='BODY' && document.documentElement.tagName==='HTML' &&
        child.body==='child' && child.documentElement==='child-root' &&
        child.document.body.tagName==='BODY' && child.document.documentElement.tagName==='HTML'
    "#).unwrap(), JsValue::Bool(true));
    assert_eq!(scheduler.run_script(r#"
        child.eval("var body='child-eval'; body===window.body && document.body.tagName==='BODY'") &&
        child.body==='child-eval' && body==='ordinary' &&
        window===document.defaultView && child===child.document.defaultView &&
        !Object.prototype.hasOwnProperty.call(window,'[[DomNode]]')
    "#).unwrap(), JsValue::Bool(true));
}
