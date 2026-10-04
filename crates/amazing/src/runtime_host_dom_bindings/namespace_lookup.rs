//! Installed whole Node namespace lookup contract.
use super::*;
fn check(source: &str) {
    let host=Rc::new(RefCell::new(BrowserDomHost{dom:amazing_html::parse("<body></body>"),native_forwarded:false}));
    let mut js=Interpreter::new_with_external_scheduler(128).unwrap();js.set_dom_host(host.clone());
    let bindings=super::super::install(&mut js,host).unwrap();
    bindings.set_document_url(&mut js,"https://example.test/namespaces");
    let value=js.run_script(source).unwrap();assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
}
#[test]
fn all_namespace_methods_select_actual_node_context_and_parent_element_boundaries() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
    const ns='http://www.w3.org/2000/xmlns/';
    const d=document.implementation.createDocument('urn:r','p:r',null),r=d.documentElement;
    r.setAttributeNS(ns,'xmlns:q','urn:q');r.setAttributeNS(ns,'xmlns','urn:default');
    const t=d.createTextNode('x'),c=d.createComment('c'),pi=d.createProcessingInstruction('t','x');
    r.append(t,c,pi);
    for(const n of [r,d,t,c,pi]){ok(n.lookupPrefix('urn:r')==='p');ok(n.lookupNamespaceURI('q')==='urn:q');ok(n.isDefaultNamespace('urn:default'));}
    const a=d.createAttribute('a');ok(a.lookupPrefix('urn:r')===null);r.setAttributeNode(a);
    ok(a.lookupPrefix('urn:r')==='p'&&a.lookupNamespaceURI('q')==='urn:q');r.removeAttributeNode(a);ok(a.lookupPrefix('urn:r')===null);
    const f=d.createDocumentFragment();f.append(t);ok(t.lookupPrefix('urn:r')===null&&f.lookupNamespaceURI('xml')===null);
    const host=d.createElementNS('http://www.w3.org/1999/xhtml','div'),shadow=host.attachShadow({mode:'closed'});r.append(host);
    const x=d.createElement('x');shadow.append(x);ok(x.lookupPrefix('urn:r')===null&&shadow.lookupNamespaceURI('q')===null);
    const empty=new Document();ok(empty.lookupPrefix('urn:r')===null&&empty.lookupNamespaceURI('xml')===null);
    const dt=d.implementation.createDocumentType('r','','');ok(dt.lookupPrefix('urn:r')===null&&dt.isDefaultNamespace(null));
    return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn prefix_lookup_obeys_own_prefix_and_ordered_declarations_without_default_or_reserved_guesses() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
    const ns='http://www.w3.org/2000/xmlns/',d=document.implementation.createDocument('urn:r','p:r',null),r=d.documentElement;
    r.setAttributeNS(ns,'xmlns:z','urn:r');r.setAttributeNS(ns,'xmlns:a','urn:x');r.setAttributeNS(ns,'xmlns:b','urn:x');
    ok(r.lookupPrefix('urn:r')==='p'&&r.lookupPrefix('urn:x')==='a');
    const x=d.createElementNS('urn:child','q:x');r.append(x);x.setAttributeNS(ns,'xmlns:a','urn:shadow');
    ok(x.lookupPrefix('urn:x')==='a'&&x.lookupPrefix('urn:shadow')==='a');
    r.removeAttributeNS(ns,'a');ok(x.lookupPrefix('urn:x')==='b');
    const plain=d.createElement('plain');plain.setAttributeNS(ns,'xmlns','urn:default');
    ok(plain.lookupPrefix('urn:default')===null);ok(plain.lookupPrefix('http://www.w3.org/XML/1998/namespace')===null);
    ok(plain.lookupNamespaceURI('xml')==='http://www.w3.org/XML/1998/namespace');
    for(const n of [r,x,plain]){ok(n.lookupPrefix(null)===null&&n.lookupPrefix('')===null);}
    return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn namespace_domstrings_keep_lone_units_distinct_from_replacement_and_empty_null() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
    const ns='http://www.w3.org/2000/xmlns/',d=document.implementation.createDocument(null,'r',null),r=d.documentElement;
    const high=String.fromCharCode(0xd800),low=String.fromCharCode(0xdc00);
    r.setAttributeNS(ns,'xmlns:p',high);r.setAttributeNS(ns,'xmlns',low);
    ok(r.lookupNamespaceURI('p')===high&&r.lookupNamespaceURI(null)===low&&r.lookupNamespaceURI('')===low);
    ok(r.lookupPrefix(high)==='p'&&r.lookupPrefix('�')===null);
    ok(r.isDefaultNamespace(low)&&!r.isDefaultNamespace('�')&&!r.isDefaultNamespace(high));
    ok(r.lookupNamespaceURI(high)===null&&r.lookupNamespaceURI(low)===null);
    r.setAttributeNS(ns,'xmlns','');ok(r.lookupNamespaceURI(null)===null&&r.isDefaultNamespace(null)&&r.isDefaultNamespace(''));
    return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
#[test]
fn lookup_conversion_reentrancy_and_abrupt_identity_precede_live_tree_reads() {
    check(r#"(function(){let step=0;try{function ok(v){step++;if(!v)throw 'step '+step;}
    const ns='http://www.w3.org/2000/xmlns/',d=document.implementation.createDocument(null,'r',null),r=d.documentElement;
    const a=d.createElement('a'),b=d.createElement('b'),x=d.createElement('x');r.append(a,b);a.append(x);
    a.setAttributeNS(ns,'xmlns:p','urn:a');b.setAttributeNS(ns,'xmlns:q','urn:b');
    let count=0;ok(x.lookupPrefix({toString(){count++;b.append(x);return 'urn:b';}})==='q'&&count===1);
    ok(x.lookupNamespaceURI({toString(){a.append(x);return 'p';}})==='urn:a');
    const thrown={sentinel:true};let same=false;try{x.isDefaultNamespace({toString(){x.setAttributeNS(ns,'xmlns','urn:prefix');throw thrown;}});}catch(e){same=e===thrown;}
    ok(same&&x.lookupNamespaceURI(null)==='urn:prefix');
    for(const name of ['lookupPrefix','lookupNamespaceURI','isDefaultNamespace']){let called=false,rejected=false;
      try{Node.prototype[name].call({}, {toString(){called=true;return 'x';}});}catch(e){rejected=e.name==='TypeError';}ok(rejected&&!called);
      let missing=false;try{x[name]();}catch(e){missing=e.name==='TypeError';}ok(missing);
      ok(Node.prototype[name].length===1);}
    return true;}catch(e){return String(e)+' at '+step;}})()"#);
}
