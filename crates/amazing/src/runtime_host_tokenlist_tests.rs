use super::*;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
fn scheduler() -> DriverScheduler {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><div id='tokens'></div><a id='a'></a><form id='f'></form><iframe id='i'></iframe><link id='l'><output id='o'></output></body>"),
        native_forwarded: false,
    }));
    DriverScheduler::new(host, JS_CAPACITY).unwrap()
}
#[test]
fn installed_token_list_distinguishes_utf16_indexed_tokens_and_raw_value() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let list=document.getElementById('tokens').classList;
        list.value=' \uD800 \uDC00 \uFFFD \uD800 ';
        list.length===3 && list[0]==='\uD800' && list[1]==='\uDC00' && list[2]==='\uFFFD' &&
        Object.keys(list).join(',')==='0,1,2' && list.item(99)===null &&
        list.value===' \uD800 \uDC00 \uFFFD \uD800 '
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_conversions_read_live_values_and_validate_atomically() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let e=document.getElementById('tokens'), list=e.classList;
        list.value='before'; let order=[];
        list.add({toString(){order.push(1);list.value='after';return '\uD800';}},
                 {toString(){order.push(2);return '\uDC00';}});
        let live=list.value==='after \uD800 \uDC00';
        let marker={}; try {list.add('new',{toString(){throw marker;}});} catch(e){live=live&&e===marker;}
        let unchanged=list.value==='after \uD800 \uDC00';
        let syntax=false;try{list.add('new','');}catch(e){syntax=e.name==='SyntaxError';}
        live&&unchanged&&syntax&&list.value==='after \uD800 \uDC00'&&order.join(',')==='1,2'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_force_and_replace_preserve_presence_and_order() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let e=document.getElementById('tokens'), list=e.classList;
        let absent=list.toggle('a',false)===false&&!e.hasAttribute('class');
        list.value=' a a b c ';let noChange=list.toggle('a',true)&&list.value===' a a b c ';
        let earliest=list.replace('c','a')&&list.value==='a b';
        let missing=!list.replace('missing','d')&&list.value==='a b';
        list.remove('a','b');absent&&noChange&&earliest&&missing&&e.hasAttribute('class')&&list.value===''
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_observer_receives_exact_old_values_once() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let e=document.getElementById('tokens'),list=e.classList;
        let observer=new MutationObserver(()=>{});observer.observe(e,{attributes:true,attributeOldValue:true});
        list.value='\uD800';list.add('\uDC00');let records=observer.takeRecords();
        records.length===2&&records[0].oldValue===null&&records[1].oldValue==='\uD800'&&
          records[0].target===e&&records[1].attributeName==='class'&&list.value==='\uD800 \uDC00'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_attribute_and_reflection_writes_admit_exact_values_once() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let calls=[];class XAttributeUnits extends HTMLElement {
          static get observedAttributes(){return ['title'];}
          attributeChangedCallback(name,oldValue,newValue){calls.push([oldValue,newValue]);}
        }
        customElements.define('x-attribute-units',XAttributeUnits);let e=document.createElement('x-attribute-units');
        let observer=new MutationObserver(()=>{});observer.observe(e,{attributes:true,attributeOldValue:true});
        e.title='\uD800';e.setAttribute('title','\uDC00');e.getAttributeNode('title').value='\uD800\u0000';
        let records=observer.takeRecords();
        calls.length===3&&calls[0][0]===null&&calls[0][1]==='\uD800'&&
          calls[1][0]==='\uD800'&&calls[1][1]==='\uDC00'&&calls[2][0]==='\uDC00'&&calls[2][1]==='\uD800\u0000'&&
          records.length===3&&records[0].oldValue===null&&records[1].oldValue==='\uD800'&&records[2].oldValue==='\uDC00'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_all_reflection_views_are_live_canonical_exact() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let views=[document.getElementById('a').relList, document.getElementById('f').relList,
          document.getElementById('i').sandbox, document.getElementById('l').sizes,
          document.getElementById('o').htmlFor];
        for(let view of views){view.value='\uD800 \uDC00';view.add('last');}
        views.every(view=>view.length===3&&view[0]==='\uD800'&&view[1]==='\uDC00'&&view.value==='\uD800 \uDC00 last')&&
          views[0]===document.getElementById('a').relList&&views[1]===document.getElementById('f').relList&&
          views[2]===document.getElementById('i').sandbox&&views[3]===document.getElementById('l').sizes&&
          views[4]===document.getElementById('o').htmlFor
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_custom_element_callbacks_receive_exact_values_once() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let calls=[];class XTokens extends HTMLElement {
          static get observedAttributes(){return ['class'];}
          attributeChangedCallback(name,oldValue,newValue){calls.push([name,oldValue,newValue]);}
        }
        customElements.define('x-tokens',XTokens);
        let element=document.createElement('x-tokens'), list=element.classList;
        list.value='\uD800'; list.add('\uDC00'); list.remove('\uD800');
        calls.length===3&&calls[0][1]===null&&calls[0][2]==='\uD800'&&
          calls[1][1]==='\uD800'&&calls[1][2]==='\uD800 \uDC00'&&
          calls[2][1]==='\uD800 \uDC00'&&calls[2][2]==='\uDC00'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_upgrade_callbacks_capture_existing_units() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let element=document.createElement('x-existing'); element.classList.value='\uD800 \uDC00';
        let calls=[];class XExisting extends HTMLElement {
          static get observedAttributes(){return ['class'];}
          attributeChangedCallback(name,oldValue,newValue){calls.push([oldValue,newValue]);}
        }
        customElements.define('x-existing',XExisting);customElements.upgrade(element);
        calls.length===1&&calls[0][0]===null&&calls[0][1]==='\uD800 \uDC00'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_upgrade_receipt_precedes_author_constructor_mutation() {
    let mut driver = scheduler();
    assert_eq!(driver.run_script(r#"
        let element=document.createElement('x-before-constructor'); element.classList.value='\uD800';
        let calls=[];class XBeforeConstructor extends HTMLElement {
          constructor(){super();this.classList.value='\uDC00';}
          static get observedAttributes(){return ['class'];}
          attributeChangedCallback(name,oldValue,newValue){calls.push([oldValue,newValue]);}
        }
        customElements.define('x-before-constructor',XBeforeConstructor);customElements.upgrade(element);
        calls.length===1&&calls[0][0]===null&&calls[0][1]==='\uD800'&&element.classList.value==='\uDC00'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn installed_token_list_owner_reference_is_private_canonical_and_nonreflective() {
    let mut driver = scheduler();
    let list = driver.run_script("document.getElementById('tokens').classList").unwrap();
    let bindings = driver.test_bindings().clone();
    let state = bindings.state.borrow();
    let name = state.tokenlist.owner_name.clone();
    let JsValue::Object(object) = &list else { panic!("token list wrapper"); };
    let owner = match &object.borrow().private_find(&name).expect("associated-element edge").kind {
        PrivateElementKind::Field(owner) => owner.clone(), _ => panic!("private value edge"),
    };
    drop(state);
    let element = driver.run_script("document.getElementById('tokens')").unwrap();
    assert_eq!(owner, element);
    assert_eq!(driver.run_script("Reflect.ownKeys(document.getElementById('tokens').classList).length").unwrap(), JsValue::Num(0.0));
    driver.run_script("document.getElementById('tokens').classList").unwrap();
    let object = object.borrow();
    let PrivateElementKind::Field(receipt) = &object.private_find(&name).unwrap().kind else { panic!("associated-element edge"); };
    assert_eq!(receipt, &owner);
}
#[test]
fn installed_token_list_original_dom_capture_rejects_foreign_interpreters() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<body><div></div></body>"), native_forwarded: false }));
    let mut js = Interpreter::new();let bindings = super::super::install(&mut js, host.clone()).unwrap();
    let arena = bindings.state.borrow().arena;
    let capture = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() }.tokenlist_capture_dom(arena).unwrap();
    let other = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<body><div></div></body>"), native_forwarded: false }));
    let mut other_js = Interpreter::new();let other_bindings = super::super::install(&mut other_js, other.clone()).unwrap();
    let mut adapter = RuntimeBindingHost { js: &mut other_js, state: other_bindings.state.clone() };
    assert!(adapter.tokenlist_capture_dom(arena).is_err());
    let mut called = false;
    assert!(adapter.tokenlist_with_captured_dom(&capture, |_| { called = true;Ok(()) }).is_err());
    assert!(!called);
    assert_eq!(host.borrow().dom.node_count(), other.borrow().dom.node_count());
}

#[test]
fn installed_captured_node_wrapper_is_canonical_and_generation_fenced() {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><div></div></body>"), native_forwarded: false,
    }));
    let mut js = Interpreter::new();
    js.set_dom_host(host.clone());
    let bindings = super::super::install(&mut js, host.clone()).unwrap();
    let arena = bindings.state.borrow().arena;
    let node = host.borrow().dom.tree_order(NodeIdx(0)).into_iter()
        .find(|node| host.borrow().dom.local_name(*node) == Some("div")).unwrap();
    let generation = host.borrow().dom.node_generation(node).unwrap();
    let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
    let capture = adapter.tokenlist_capture_dom(arena).unwrap();
    let receipt = amazing_dom::render_mutation::NodeGeneration { node, generation };
    let first = adapter.tokenlist_wrap_captured_node(&capture, receipt).unwrap_or_else(|e| panic!("{}", e.describe()));
    let second = adapter.tokenlist_wrap_captured_node(&capture, receipt).unwrap();
    assert_eq!(first, second);
    assert!(adapter.tokenlist_wrap_captured_node(&capture,
        amazing_dom::render_mutation::NodeGeneration { generation: generation + 1, ..receipt }).is_err());
    let other_host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body><div></div></body>"), native_forwarded: false,
    }));
    let mut other_js = Interpreter::new();
    other_js.set_dom_host(other_host.clone());
    let other_bindings = super::super::install(&mut other_js, other_host).unwrap();
    let mut other_adapter = RuntimeBindingHost { js: &mut other_js, state: other_bindings.state.clone() };
    assert!(other_adapter.tokenlist_wrap_captured_node(&capture, receipt).is_err());
}
