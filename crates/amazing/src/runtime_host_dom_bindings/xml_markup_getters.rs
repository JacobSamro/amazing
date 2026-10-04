//! Actual installed getter dispatch, node-document adoption and intrinsic errors.
use super::*;
use crate::js_scheduler::DriverScheduler;
fn scheduler() -> DriverScheduler {
    let host=Rc::new(RefCell::new(BrowserDomHost {dom:amazing_html::parse("<body><iframe></iframe></body>"),native_forwarded:false}));
    let mut scheduler=DriverScheduler::new(host,128).unwrap();
    scheduler.set_location_url("https://example.test/xml-getters");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    scheduler
}
#[test]
fn xml_getters_use_actual_associated_document_after_adoption_and_foreign_borrow() {
    let mut scheduler=scheduler();
    let result=scheduler.run_script(r#"(function(){let step=0;try{function check(v){step++;if(!v)throw 'getter '+step;}
        const child=document.querySelector('iframe').contentWindow;
        const getter=Object.getOwnPropertyDescriptor(Element.prototype,'outerHTML').get;
        const foreignGetter=Object.getOwnPropertyDescriptor(child.Element.prototype,'outerHTML').get;
        const xml=document.implementation.createDocument(null,'r',null);
        const element=xml.createElementNS('http://www.w3.org/1999/xhtml','a');
        check(element.outerHTML==='<a xmlns="http://www.w3.org/1999/xhtml"></a>');
        check(foreignGetter.call(element)===element.outerHTML);
        document.adoptNode(element);check(element.outerHTML==='<a></a>');
        xml.adoptNode(element);check(getter.call(element)==='<a xmlns="http://www.w3.org/1999/xhtml"></a>');
        const parsed=new DOMParser().parseFromString('<r><a/></r>','application/xml');
        check(parsed.documentElement.innerHTML==='<a/>');
        check(parsed.documentElement.outerHTML==='<r><a/></r>');
        return true;}catch(e){return String(e)+' at '+step;}})()"#).unwrap();
    assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
}
#[test]
fn xml_getters_template_shadow_and_gethtml_keep_distinct_fragment_boundaries() {
    let mut scheduler=scheduler();
    let result=scheduler.run_script(r#"(function(){let step=0;try{function check(v){step++;if(!v)throw 'getter '+step;}
        const xml=document.implementation.createDocument(null,'r',null);
        const template=xml.createElementNS('http://www.w3.org/1999/xhtml','template');
        template.content.appendChild(xml.createElementNS('http://www.w3.org/1999/xhtml','a'));
        check(template.innerHTML==='<a xmlns="http://www.w3.org/1999/xhtml"></a>');
        check(template.outerHTML==='<template xmlns="http://www.w3.org/1999/xhtml"><a></a></template>');
        check(template.getHTML()==='<a></a>');
        const host=xml.createElementNS('http://www.w3.org/1999/xhtml','div');
        const shadow=host.attachShadow({mode:'closed'});
        shadow.appendChild(xml.createElementNS('http://www.w3.org/1999/xhtml','b'));
        check(shadow.innerHTML==='<b xmlns="http://www.w3.org/1999/xhtml"></b>');
        check(host.innerHTML==='');check(shadow.getHTML()==='<b></b>');
        check(host.getHTML({shadowRoots:[shadow]})==='<template shadowrootmode="closed"><b></b></template>');
        return true;}catch(e){return String(e)+' at '+step;}})()"#).unwrap();
    assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
}
#[test]
fn xml_getter_strict_errors_use_callee_intrinsic_without_author_constructor_getters() {
    let mut scheduler=scheduler();
    let result=scheduler.run_script(r#"(function(){let step=0;try{function check(v){step++;if(!v)throw 'getter '+step;}
        const child=document.querySelector('iframe').contentWindow;
        const getter=Object.getOwnPropertyDescriptor(child.Element.prototype,'innerHTML').get;
        const intrinsic=child.DOMException.prototype;
        Object.defineProperty(child,'DOMException',{configurable:true,get(){throw 'author constructor entered';}});
        const xml=document.implementation.createDocument(null,'r',null);const root=xml.documentElement;
        for(const text of ['\u0000','\u0001','\uFFFE','\uD800']){
            root.replaceChildren(xml.createTextNode(text));let rejected=false;
            try{getter.call(root);}catch(e){rejected=e.name==='InvalidStateError'&&Object.getPrototypeOf(e)===intrinsic;}
            check(rejected);
            check(new XMLSerializer().serializeToString(root.firstChild)===text);
        }
        root.replaceChildren(xml.createComment('bad--comment'));let rejected=false;
        try{getter.call(root);}catch(e){rejected=e.name==='InvalidStateError'&&Object.getPrototypeOf(e)===intrinsic;}check(rejected);
        const text=document.createElement('div');text.appendChild(document.createComment('bad--comment'));
        check(text.innerHTML==='<!--bad--comment-->');
        return true;}catch(e){return String(e)+' at '+step;}})()"#).unwrap();
    assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
}
