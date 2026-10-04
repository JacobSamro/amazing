//! Whole XML markup setters: live context, fatal grammar and atomic replacement.
use super::*;
fn run(source: &str) -> JsValue {
    let host=Rc::new(RefCell::new(BrowserDomHost {
        dom:amazing_html::parse("<body></body>"),native_forwarded:false,
    }));
    let mut js=Interpreter::new_with_external_scheduler(128).unwrap();
    js.set_dom_host(host.clone());
    let bindings=super::super::install(&mut js,host).unwrap();
    bindings.set_document_url(&mut js,"https://example.test/xml-setters");
    js.run_script(source).unwrap()
}
fn check(source: &str) { let value=run(source);assert!(matches!(value,JsValue::Bool(true)),"{value:?}"); }
#[test]
fn xml_setters_preserve_context_namespace_case_attributes_and_character_kinds() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const d=new DOMParser().parseFromString('<r xmlns="urn:default" xmlns:p="urn:outer"><p:target xmlns:p="urn:near"/></r>','application/xml');
        const t=d.documentElement.firstChild;
        t.innerHTML='<p:Mixed p:k="&#x1F642;"/>text<![CDATA[a<&]]><!--c--><?target data?><plain/>';
        ok(t.childNodes.length===6);ok(t.firstChild.localName==='Mixed'&&t.firstChild.prefix==='p'&&t.firstChild.namespaceURI==='urn:near');
        ok(t.firstChild.getAttributeNS('urn:near','k')==='🙂');ok(t.childNodes[2].nodeType===4&&t.childNodes[2].data==='a<&');
        ok(t.childNodes[3].nodeType===8&&t.childNodes[4].nodeType===7);ok(t.lastChild.namespaceURI==='urn:default');
        t.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns','');t.innerHTML='<plain/>﻿';
        ok(t.firstChild.namespaceURI===null&&t.lastChild.data==='﻿');
        t.insertAdjacentHTML('afterbegin','<p:Before/>');t.insertAdjacentHTML('beforeend','<p:After/>');
        ok(t.firstChild.localName==='Before'&&t.lastChild.localName==='After'&&t.lastChild.namespaceURI==='urn:near');
        t.firstChild.outerHTML='<p:Replacement/><second/>';
        ok(t.firstChild.localName==='Replacement'&&t.childNodes[1].localName==='second'&&t.childNodes[1].namespaceURI===null);
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn xml_setter_syntax_failure_preserves_nodes_ranges_and_mutation_records() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const d=new DOMParser().parseFromString('<r xmlns="urn:r"><target>original</target><peer/></r>','application/xml');
        const t=d.documentElement.firstChild,old=t.firstChild,peer=t.nextSibling;
        const range=d.createRange();range.setStart(old,2);range.setEnd(old,5);
        const mo=new MutationObserver(function(){});mo.observe(d.documentElement,{childList:true,subtree:true,characterData:true,attributes:true});
        const bad=['&unknown;','<a xmlns:p=""/>','<unclosed>','<p:unbound/>','&#0;','<?xml version="1.0"?>','<!DOCTYPE a><a/>','<a x="1" x="2"/>','<a xmlns:p="urn:x" xmlns:q="urn:x" p:x="1" q:x="2"/>',String.fromCharCode(0xd800),'<a><!--bad--comment--></a>'];
        for(const source of bad){
            const operations=[function(){t.innerHTML=source;},function(){t.outerHTML=source;},function(){t.insertAdjacentHTML('beforebegin',source);},function(){t.insertAdjacentHTML('afterbegin',source);},function(){t.insertAdjacentHTML('beforeend',source);},function(){t.insertAdjacentHTML('afterend',source);}];
            for(const operation of operations){let rejected=false;try{operation();}catch(e){rejected=e.name==='SyntaxError';}ok(rejected);
                ok(t.firstChild===old&&t.nextSibling===peer&&t.parentNode===d.documentElement);ok(range.startContainer===old&&range.startOffset===2&&range.endOffset===5);ok(mo.takeRecords().length===0);}
        }
        t.innerHTML='<fresh/>';
        const records=mo.takeRecords();ok(records.length===1&&records[0].target===t&&records[0].removedNodes[0]===old&&records[0].addedNodes[0]===t.firstChild);
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn xml_outer_and_adjacent_setters_use_real_parent_or_prescribed_body_context() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const d=document.implementation.createDocument('urn:r','r',null),detached=d.createElementNS('urn:old','old');let conversions=0;
        detached.outerHTML={toString(){conversions++;return '<broken>';}};ok(conversions===1&&detached.parentNode===null);
        for(const operation of [function(){d.documentElement.outerHTML='<broken>';},function(){d.documentElement.insertAdjacentHTML('beforebegin','<broken>');},function(){d.documentElement.insertAdjacentHTML('afterend','<broken>');}]){
            let rejected=false;try{operation();}catch(e){rejected=e.name==='NoModificationAllowedError';}ok(rejected);
        }
        const fragment=d.createDocumentFragment();fragment.appendChild(detached);detached.outerHTML='<Mixed/>';
        ok(fragment.firstChild.localName==='Mixed'&&fragment.firstChild.namespaceURI==='http://www.w3.org/1999/xhtml');
        fragment.firstChild.insertAdjacentHTML('beforebegin','<Before/>');fragment.lastChild.insertAdjacentHTML('afterend','<After/>');
        ok(fragment.firstChild.localName==='Before'&&fragment.lastChild.localName==='After'&&fragment.lastChild.namespaceURI==='http://www.w3.org/1999/xhtml');
        const html=d.createElementNS('http://www.w3.org/1999/xhtml','h:html');html.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns','urn:explicit');d.documentElement.appendChild(html);
        html.insertAdjacentHTML('beforeend','<Item/>');ok(html.firstChild.namespaceURI==='urn:explicit'&&html.firstChild.localName==='Item');
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn xml_setters_snapshot_namespace_and_parent_after_reentrant_conversion() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const d=new DOMParser().parseFromString('<r xmlns="urn:new" xmlns:p="urn:p"><target/></r>','application/xml');
        const element=document.createElement('div');document.body.appendChild(element);
        element.innerHTML={toString(){d.documentElement.appendChild(element);return '<p:Case/>';}};
        ok(element.ownerDocument===d&&element.firstChild.namespaceURI==='urn:p'&&element.firstChild.localName==='Case');
        const child=element.firstChild;child.outerHTML={toString(){d.documentElement.appendChild(child);return '<Replacement/>';}};
        ok(d.documentElement.lastChild.localName==='Replacement'&&d.documentElement.lastChild.namespaceURI==='urn:new'&&element.childNodes.length===0);
        element.innerHTML={toString(){document.body.appendChild(element);return '<P>html';}};
        ok(element.ownerDocument===document&&element.firstChild.localName==='p'&&element.firstChild.namespaceURI==='http://www.w3.org/1999/xhtml');
        const abrupt={sentinel:true};let same=false;try{element.innerHTML={toString(){element.replaceChildren(document.createTextNode('author prefix'));throw abrupt;}};}catch(e){same=e===abrupt;}ok(same&&element.firstChild.data==='author prefix');
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn xml_template_and_closed_shadow_setters_keep_real_hosts_and_inert_owner_documents() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const d=document.implementation.createDocument(null,'r',null),ns='http://www.w3.org/1999/xhtml';
        const t=d.createElementNS(ns,'template');t.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns:p','urn:p');
        const contents=t.content;t.innerHTML='<p:Case/><template xmlns="http://www.w3.org/1999/xhtml"><p:Nested/></template>';
        ok(t.content===contents&&t.childNodes.length===0&&contents.firstChild.namespaceURI==='urn:p');
        ok(contents.lastChild.content.firstChild.localName==='Nested'&&contents.lastChild.content.firstChild.namespaceURI==='urn:p');
        const host=d.createElementNS(ns,'div');host.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns:p','urn:shadow');const shadow=host.attachShadow({mode:'closed'});
        shadow.innerHTML='<p:Case/><template xmlns="http://www.w3.org/1999/xhtml" shadowrootmode="open"><p:ordinary/></template>';
        ok(shadow.firstChild.namespaceURI==='urn:shadow'&&shadow.lastChild.localName==='template'&&host.shadowRoot===null);
        const old=shadow.firstChild;let rejected=false;try{shadow.innerHTML='<broken>';}catch(e){rejected=e.name==='SyntaxError';}ok(rejected&&shadow.firstChild===old);
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn foreign_xml_setters_use_actual_document_and_intrinsic_syntax_errors() {
    use crate::js_scheduler::DriverScheduler;
    let host=Rc::new(RefCell::new(BrowserDomHost {
        dom:amazing_html::parse("<body><iframe></iframe></body>"),native_forwarded:false,
    }));
    let mut scheduler=DriverScheduler::new(host,128).unwrap();
    scheduler.set_location_url("https://example.test/xml-setters");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    let value=scheduler.run_script(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        const child=document.querySelector('iframe').contentWindow;
        const setter=Object.getOwnPropertyDescriptor(child.Element.prototype,'innerHTML').set;
        const outer=Object.getOwnPropertyDescriptor(child.Element.prototype,'outerHTML').set;
        const intrinsic=child.DOMException.prototype;
        Object.defineProperty(child,'DOMException',{configurable:true,get(){throw 'author constructor called';}});
        const d=new DOMParser().parseFromString('<r xmlns:p="urn:p"><target/></r>','application/xml'),t=d.documentElement.firstChild;
        setter.call(t,'<p:Case/>');ok(t.firstChild.namespaceURI==='urn:p'&&t.firstChild.localName==='Case');
        for(const source of ['<broken>','<u:unbound/>',String.fromCharCode(0xd800)]){
            let rejected=false;try{setter.call(t,source);}catch(e){rejected=e.name==='SyntaxError'&&Object.getPrototypeOf(e)===intrinsic;}ok(rejected);
        }
        let rejected=false;try{outer.call(d.documentElement,'<broken>');}catch(e){rejected=e.name==='NoModificationAllowedError'&&Object.getPrototypeOf(e)===intrinsic;}ok(rejected);
        let fake=false;try{setter.call({},'<a/>');}catch(e){fake=e.name==='TypeError';}ok(fake);
        return true;}catch(e){return String(e)+' at '+step;}})()"#).unwrap();
    assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
}

#[test]
fn ordinary_xml_setter_scripts_stay_started_through_detachment_template_and_html_adoption() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
        globalThis.xmlSetterRuns=0;const ns='http://www.w3.org/1999/xhtml';
        const d=document.implementation.createDocument(ns,'html',null),root=d.documentElement;
        for(const operation of ['inner','outer','adjacent']){
            const host=d.createElementNS(ns,'div');root.appendChild(host);
            const markup='<script>xmlSetterRuns++</script><svg:script xmlns:svg="http://www.w3.org/2000/svg">xmlSetterRuns++</svg:script><template><script>xmlSetterRuns++</script></template>';
            if(operation==='inner')host.innerHTML=markup;
            else if(operation==='adjacent')host.insertAdjacentHTML('beforeend',markup);
            else {const old=d.createElementNS(ns,'span');host.appendChild(old);old.outerHTML=markup;}
            const template=host.lastChild;document.body.appendChild(host);document.body.appendChild(template.content);ok(xmlSetterRuns===0);
        }
        const template=d.createElementNS(ns,'template');template.innerHTML='<script>xmlSetterRuns++</script>';
        document.body.appendChild(template.content);ok(xmlSetterRuns===0);
        const disconnected=d.createElementNS(ns,'div');disconnected.innerHTML='<script>xmlSetterRuns++</script>';
        document.body.appendChild(disconnected);ok(xmlSetterRuns===0);
        const shadowHost=d.createElementNS(ns,'div'),shadow=shadowHost.attachShadow({mode:'closed'});
        shadow.innerHTML='<script>xmlSetterRuns++</script>';document.body.appendChild(shadowHost);document.body.appendChild(shadow.firstChild);ok(xmlSetterRuns===0);
        // Direct authored scripts remain executable: inertness is attached to
        // actual parsed nodes, never a document-wide or future-script flag.
        const authored=document.createElement('script');authored.textContent='xmlSetterRuns++';document.body.appendChild(authored);ok(xmlSetterRuns===1);
        return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
