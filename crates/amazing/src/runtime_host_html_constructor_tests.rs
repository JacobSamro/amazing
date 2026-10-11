//! Installed construction uses real registered constructors and native nodes.
use super::*;
use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
fn scheduler(markup: &str) -> DriverScheduler {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(markup), native_forwarded: false,
    }));
    DriverScheduler::new(host, JS_CAPACITY).unwrap()
}
#[test]
fn autonomous_direct_and_document_creation_return_actual_custom_nodes() {
    let mut driver = scheduler("<body></body>");
    assert_eq!(driver.run_script(r#"
        let calls=[];
        class NativeCard extends HTMLElement {
            static get observedAttributes(){return ['data-a'];}
            constructor(){super();calls.push([this,this.attributes.length,this.childNodes.length]);}
            attributeChangedCallback(n,o,v){calls.push([n,o,v]);}
        }
        customElements.define('x-native-card',NativeCard);
        let direct=new NativeCard(), made=document.createElement('x-native-card');
        direct.setAttribute('data-a','direct'); made.setAttribute('data-a','made');
        direct instanceof NativeCard && made instanceof NativeCard &&
        direct.ownerDocument===document && made.ownerDocument===document &&
        direct.localName==='x-native-card' && made.localName==='x-native-card' &&
        direct!==made && calls.length===4 && calls[0][0]===direct && calls[1][0]===made &&
        calls[0][1]===0 && calls[1][1]===0 && calls[0][2]===0 && calls[1][2]===0 &&
        calls[2].join('|')==='data-a||direct' && calls[3].join('|')==='data-a||made'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn true_upgrade_retains_existing_identity_and_preconstructor_attribute_snapshot() {
    let mut driver = scheduler("<body><x-native-upgrade id='old' data-a='before'></x-native-upgrade></body>");
    assert_eq!(driver.run_script(r#"
        let old=document.getElementById('old'), values=[], constructed;
        class NativeUpgrade extends HTMLElement {
            static get observedAttributes(){return ['data-a'];}
            constructor(){super();constructed=this;this.setAttribute('data-a','during');}
            attributeChangedCallback(n,o,v){values.push([n,o,v]);}
        }
        customElements.define('x-native-upgrade',NativeUpgrade);customElements.upgrade(old);
        let snapshot=values.length===1 && values[0][1]===null && values[0][2]==='before';
        old.setAttribute('data-a','after');
        snapshot && constructed===old && old instanceof NativeUpgrade && old.getAttribute('data-a')==='after' &&
        values.length===2 && values[0][0]==='data-a' && values[0][1]===null && values[0][2]==='before' &&
        values[1][0]==='data-a' && values[1][1]==='during' && values[1][2]==='after'
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn constructor_registry_and_native_interface_checks_precede_prototype_getter() {
    let mut driver = scheduler("<body></body>");
    assert_eq!(driver.run_script(r#"
        let gets=0;
        class NativeProxy extends HTMLElement {}
        let target=new Proxy(NativeProxy,{get(t,k,r){if(k==='prototype')gets++;return Reflect.get(t,k,r);}});
        customElements.define('x-native-proxy',target); gets=0;
        let node=Reflect.construct(HTMLElement,[],target), exact=gets===1;
        gets=0;let wrong=false;try{Reflect.construct(HTMLDivElement,[],target);}catch(e){wrong=e instanceof TypeError;}
        exact && node.localName==='x-native-proxy' && node instanceof NativeProxy && wrong && gets===0
    "#).unwrap(), JsValue::Bool(true));
}
#[test]
fn customized_generic_html_constructor_and_illegal_intrinsics_remain_distinct() {
    let mut driver = scheduler("<body></body>");
    assert_eq!(driver.run_script(r#"
        class NativeArticle extends HTMLElement {}
        customElements.define('x-native-article',NativeArticle,{extends:'article'});
        let node=new NativeArticle(), illegal=0;
        for(let ctor of [HTMLElement,HTMLDivElement,Element,Node,HTMLUnknownElement]){
            try{new ctor();}catch(e){if(e instanceof TypeError)illegal++;}
        }
        node.localName==='article' && node.ownerDocument===document && node instanceof NativeArticle && illegal===5
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn nonextensible_upgrade_preserves_internal_setprototype_false_without_new_exception() {
    let mut driver = scheduler("<body><x-native-fixed id='fixed'></x-native-fixed></body>");
    assert_eq!(driver.run_script(r#"
        let element=document.getElementById('fixed'), original=Object.getPrototypeOf(element), actual;
        Object.preventExtensions(element);
        class NativeFixed extends HTMLElement {constructor(){super();actual=this;}}
        customElements.define('x-native-fixed',NativeFixed);customElements.upgrade(element);
        actual===element && Object.getPrototypeOf(element)===original && !(element instanceof NativeFixed)
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn original_active_registry_crosses_foreign_super_realm_and_restores_after_throw() {
    let mut driver = scheduler("<body><iframe id='constructor-frame'></iframe></body>");
    driver.set_location_url("https://example.test/constructor-frame");
    driver.run_script("void document.getElementById('constructor-frame').contentWindow").unwrap();
    assert_eq!(driver.run_script(r#"
        let foreign=document.getElementById('constructor-frame').contentWindow, called=0;
        class NativeForeign extends foreign.HTMLElement {constructor(){super();called++;}}
        customElements.define('x-native-foreign',NativeForeign);
        let held, errors=[];window.addEventListener('error',e=>{errors.push(e.error);e.preventDefault();});
        let node=document.createElement('x-native-foreign'), unregistered=false;
        try{new NativeForeign();}catch(e){unregistered=e.name==='TypeError';}
        let upgrade=document.createElement('section');upgrade.innerHTML='<x-native-foreign></x-native-foreign>';
        let old=upgrade.firstChild;customElements.upgrade(old);
        let marker={}, after=0;
        class NativeForeignThrow extends foreign.HTMLElement {constructor(){super();after++;throw marker;}}
        customElements.define('x-native-foreign-throw',NativeForeignThrow);
        let failed=document.createElement('x-native-foreign-throw');
        let restored=false;try{new NativeForeignThrow();}catch(e){restored=e.name==='TypeError';}
        node.ownerDocument===document && !(node instanceof NativeForeign) && node instanceof HTMLUnknownElement &&
        old.ownerDocument===document && old instanceof NativeForeign && called===2 && unregistered &&
        after===1 && restored && errors.length===2 && errors[1]===marker && failed instanceof HTMLUnknownElement
    "#).unwrap_or_else(|e| panic!("{}", e.describe())), JsValue::Bool(true));
}

#[test]
fn caught_second_super_during_fresh_creation_is_html_typeerror_and_preserves_first_node() {
    let mut driver = scheduler("<body></body>");
    assert_eq!(driver.run_script(r#"
        let actual, error;
        class NativeTwice extends HTMLElement {
            constructor(){super();actual=this;try{super();}catch(e){error=e.name;}}
        }
        customElements.define('x-native-twice',NativeTwice);
        let element=document.createElement('x-native-twice');
        let fresh=actual===element && element instanceof NativeTwice && error==='ReferenceError';
        let container=document.createElement('section');container.innerHTML='<x-native-twice></x-native-twice>';
        let upgrade=container.firstChild;error=undefined;customElements.upgrade(upgrade);
        let upgraded=actual===upgrade && upgrade instanceof NativeTwice && error==='TypeError';
        error=undefined;let direct=new NativeTwice();
        fresh && upgraded && actual===direct && direct instanceof NativeTwice && error==='ReferenceError'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn installed_root_parser_constructs_before_token_attributes_and_children() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var parseTrace=[], parseNodes=[];
        class ParserNode extends HTMLElement {
            static get observedAttributes(){return ['data-a'];}
            constructor(){super();parseNodes.push(this);parseTrace.push(['construct',this.attributes.length,this.childNodes.length]);
                try{document.write('<b>forbidden</b>');}catch(e){parseTrace.push(['write',e.name]);}
                try{document.open();}catch(e){parseTrace.push(['open',e.name]);}}
            attributeChangedCallback(n,o,v){parseTrace.push(['attribute',n,o,v,this.childNodes.length]);}
        }
        customElements.define('x-parser-node',ParserNode);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-parser-node data-a='outer'><x-parser-node data-a='inner'></x-parser-node></x-parser-node></body>",
        |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        parseNodes.length===2 && parseNodes[0]===document.querySelector('x-parser-node') &&
        parseNodes[1]===parseNodes[0].firstChild && parseTrace.length===8 &&
        parseTrace[0].join('|')==='construct|0|0' && parseTrace[1].join('|')==='write|InvalidStateError' &&
        parseTrace[2].join('|')==='open|InvalidStateError' && parseTrace[3].join('|')==='attribute|data-a||outer|0' &&
        parseTrace[4].join('|')==='construct|0|0' && parseTrace[7].join('|')==='attribute|data-a||inner|0' &&
        document.querySelector('b')===null
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn installed_parser_reports_original_error_before_fresh_failed_node_attributes() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var parserMarker={}, parserHeld, parserErrors=[];
        window.addEventListener('error',e=>{parserErrors.push([e.error,e.isTrusted,document.querySelector('x-parser-fail')]);e.preventDefault();});
        class ParserFail extends HTMLElement {constructor(){super();parserHeld=this;this.setAttribute('held','yes');throw parserMarker;}}
        customElements.define('x-parser-fail',ParserFail);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-parser-fail data-a='selected'></x-parser-fail><p>after</p></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        let selected=document.querySelector('x-parser-fail');
        parserErrors.length===1 && parserErrors[0][0]===parserMarker && parserErrors[0][1]===true && parserErrors[0][2]===null &&
        selected!==parserHeld && selected instanceof HTMLUnknownElement && selected.getAttribute('data-a')==='selected' &&
        selected.getAttribute('held')===null && parserHeld.getAttribute('held')==='yes' && parserHeld instanceof ParserFail &&
        parserHeld.parentNode===null && document.querySelector('p').textContent==='after'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn connected_parser_callback_writes_into_actual_prefix_before_outer_stack_push() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var connectedTrace=[], connectedNode;
        class PrefixWriter extends HTMLElement {
            constructor(){super();connectedNode=this;}
            connectedCallback(){
                connectedTrace.push(this.childNodes.length);
                document.write('<b id="written">nested</b><script>connectedTrace.push(document.currentScript.localName);document.write("<em id=scriptWritten>script</em>");</script>');
                connectedTrace.push(document.getElementById('written').parentNode===this.parentNode);
            }
        }
        customElements.define('x-prefix-writer',PrefixWriter);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-prefix-writer><i id=outerTail>tail</i></x-prefix-writer><p id=after>after</p></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        connectedTrace.length===3 && connectedTrace[0]===0 && connectedTrace[1]==='script' && connectedTrace[2]===true &&
        document.getElementById('written').parentNode===document.body &&
        document.getElementById('scriptWritten').parentNode===document.body &&
        document.getElementById('outerTail').parentNode===connectedNode && document.getElementById('after').parentNode===document.body
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn connected_callback_open_replaces_real_parser_and_stops_original_source() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var replacedCalls=0, retiredElement;
        class ParserReplacement extends HTMLElement {
            constructor(){super();retiredElement=this;}
            connectedCallback(){replacedCalls++;document.open();document.write('<body><p id=replacement>new</p></body>');document.close();}
        }
        customElements.define('x-parser-replacement',ParserReplacement);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-parser-replacement></x-parser-replacement><p id=oldTail>must not parse</p></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        replacedCalls===1 && !retiredElement.isConnected && retiredElement.parentNode.localName==='body' && document.getElementById('oldTail')===null &&
        document.getElementById('replacement').textContent==='new'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn parser_blocking_script_open_is_ignored_without_affecting_later_connection_open() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><p id=prefix>keep</p><script>document.open();document.write('<b id=blockingWrite>written</b>');</script><p id=tail>tail</p></body>",
        |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|error| error.describe())).unwrap();
    assert_eq!(driver.run_script(r#"
        document.getElementById('prefix').textContent==='keep' && document.getElementById('blockingWrite').textContent==='written' &&
        document.getElementById('tail').textContent==='tail'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn script_created_stream_close_during_constructor_defers_until_tree_frame_returns() {
    let mut driver = scheduler("<body></body>");
    assert_eq!(driver.run_script(r#"
        var closeHeld, other=document.createElement('section'), closes=0;
        class ConstructorClose extends HTMLElement {
            constructor(){super();closeHeld=this;other.setAttribute('other','retained');document.close();closes++;}
        }
        customElements.define('x-constructor-close',ConstructorClose);
        document.open();document.write('<body><x-constructor-close data-a="final"></x-constructor-close><p id=closeTail>tail</p></body>');
        closes===1 && document.querySelector('x-constructor-close')===closeHeld && closeHeld.getAttribute('data-a')==='final' &&
        document.getElementById('closeTail').textContent==='tail' && other.getAttribute('other')==='retained'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn connected_callback_exception_reports_saved_realm_and_continues_owned_queue() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var connectionMarker={}, connectionErrors=[], connectionCalls=[];
        window.addEventListener('error',e=>{connectionErrors.push([e.error,e.isTrusted]);e.preventDefault();});
        class ThrowingConnection extends HTMLElement {
            connectedCallback(){connectionCalls.push(this.id);if(this.id==='first')throw connectionMarker;}
        }
        customElements.define('x-throwing-connection',ThrowingConnection);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-throwing-connection id=first></x-throwing-connection><x-throwing-connection id=second><b>child</b></x-throwing-connection></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        connectionCalls.join('|')==='first|second' && connectionErrors.length===1 &&
        connectionErrors[0][0]===connectionMarker && connectionErrors[0][1]===true &&
        document.getElementById('second').firstChild.textContent==='child'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn child_parser_creation_and_connection_use_actual_imported_node_and_child_realm() {
    let mut driver = scheduler(r#"<body><iframe srcdoc="<body><script>
        window.childTrace=[];window.childHeld=null;
        class ChildConstructor extends HTMLElement {
            static get observedAttributes(){return ['data-a'];}
            constructor(){super();childHeld=this;childTrace.push(this.ownerDocument===document);}
            attributeChangedCallback(n,o,v){childTrace.push(v);}
            connectedCallback(){childTrace.push(this.childNodes.length);document.write('<b id=childWritten>written</b>');}
        }
        customElements.define('x-child-constructor',ChildConstructor);
        </script><x-child-constructor data-a=token><i id=childTail>tail</i></x-child-constructor></body>"></iframe></body>"#);
    driver.set_location_url("https://example.test/constructor-child");
    driver.run_script("void document.querySelector('iframe').contentWindow").unwrap();
    driver.dispatch_pending_iframe_loads().unwrap();
    assert_eq!(driver.run_script(r#"
        let child=document.querySelector('iframe').contentWindow;
        child.childTrace.length===3 && child.childTrace[0]===true && child.childTrace[1]==='token' && child.childTrace[2]===0 &&
        child.document.querySelector('x-child-constructor')===child.childHeld && child.childHeld.ownerDocument===child.document &&
        child.document.getElementById('childWritten').parentNode===child.document.body &&
        child.document.getElementById('childTail').parentNode===child.childHeld && document.getElementById('childWritten')===null
    "#).unwrap_or_else(|e| panic!("{}", e.describe())), JsValue::Bool(true));
}

#[test]
fn parser_checkpoint_uses_admitted_promise_and_mutation_fifo_under_positive_counter() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var checkpointTrace=[], checkpointWrites=[];
        class CheckpointElement extends HTMLElement {constructor(){super();checkpointTrace.push('construct');}}
        customElements.define('x-checkpoint-element',CheckpointElement);
        Promise.resolve().then(()=>{checkpointTrace.push('promise');try{document.write('forbidden');}catch(e){checkpointWrites.push(e.name);}
            queueMicrotask(()=>checkpointTrace.push('nested'));});
        var watched=document.createElement('section');
        var checkpointObserver=new MutationObserver(records=>{
            checkpointTrace.push('mutation');try{document.write('forbidden observer');}catch(e){checkpointWrites.push(e.name);}
        });
        checkpointObserver.observe(watched,{attributes:true});watched.setAttribute('data-a','before');
        queueMicrotask(()=>{checkpointTrace.push('queued');try{document.open();}catch(e){checkpointWrites.push(e.name);}});
        setTimeout(()=>checkpointTrace.push('timer'),0);
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-checkpoint-element></x-checkpoint-element><p id=checkpointTail>tail</p></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        checkpointTrace.join('|')==='promise|mutation|queued|nested|construct' && checkpointWrites.join('|')==='InvalidStateError|InvalidStateError|InvalidStateError' &&
        document.getElementById('checkpointTail').textContent==='tail'
    "#).unwrap(), JsValue::Bool(true));
    driver.pump_at(Duration::ZERO,false).unwrap();
    assert_eq!(driver.run_script("checkpointTrace[checkpointTrace.length-1]==='timer'").unwrap(), JsValue::Bool(true));
}

#[test]
fn parser_checkpoint_reports_actual_callback_error_before_constructor_and_continues_input() {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut driver = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
    driver.run_script(r#"
        var callbackError={}, callbackTrace=[], callbackCounterErrors=[];
        window.addEventListener('error', event=>{
            callbackTrace.push(event.error===callbackError && event.isTrusted ? 'report' : 'wrong');
            try{document.write('forbidden reporting write');}catch(error){callbackCounterErrors.push(error.name);}
            event.preventDefault();
        });
        class CallbackCheckpoint extends HTMLElement {constructor(){super();callbackTrace.push('construct');}}
        customElements.define('x-callback-checkpoint',CallbackCheckpoint);
        queueMicrotask(()=>{callbackTrace.push('throw');throw callbackError;});
        queueMicrotask(()=>callbackTrace.push('later'));
    "#).unwrap();
    crate::cmd::js::run_interleaved_parse(&host, &mut driver,
        "<body><x-callback-checkpoint></x-callback-checkpoint><p id=callbackTail>tail</p></body>", |_, _| Ok(())).unwrap();
    assert_eq!(driver.run_script(r#"
        callbackTrace.join('|')==='throw|report|later|construct' &&
        callbackCounterErrors.join('|')==='InvalidStateError' &&
        document.getElementById('callbackTail').textContent==='tail'
    "#).unwrap(), JsValue::Bool(true));
}

#[test]
fn parser_checkpoint_reports_in_retained_child_callback_global_without_parent_retargeting() {
    let mut driver = scheduler(r#"<body><iframe srcdoc="<script>
        var retainedMarker={}, retainedErrors=[];
        window.addEventListener('error',e=>{retainedErrors.push(e.error===retainedMarker && e.isTrusted);e.preventDefault();});
        function retainedCallback(){throw retainedMarker;}
    </script>"></iframe></body>"#);
    driver.set_location_url("https://example.test/constructor-child");
    driver.run_script("void document.querySelector('iframe').contentWindow").unwrap();
    driver.dispatch_pending_iframe_loads().unwrap();
    driver.run_script(r#"
        var retainedWindow=document.querySelector('iframe').contentWindow, parentErrors=0, retainedAfter=0;
        window.addEventListener('error',e=>{parentErrors++;e.preventDefault();});
        queueMicrotask(retainedWindow.retainedCallback);
        queueMicrotask(()=>retainedAfter++);
        document.querySelector('iframe').remove();
    "#).unwrap_or_else(|e| panic!("{}", e.describe()));
    let bindings=driver.bindings_for_parser_checkpoint();
    bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
    assert_eq!(driver.run_script(r#"
        retainedWindow.retainedErrors.length===1 && retainedWindow.retainedErrors[0]===true &&
        parentErrors===0 && retainedAfter===1
    "#).unwrap(), JsValue::Bool(true));
    // The bound object's corresponding realm is its own parent creation realm,
    // independently of the retired child target it invokes.
    driver.run_script(r#"
        queueMicrotask(Function.prototype.bind.call(retainedWindow.retainedCallback,null));
        queueMicrotask(()=>retainedAfter++);
    "#).unwrap();
    bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
    assert_eq!(driver.run_script(r#"
        retainedWindow.retainedErrors.length===1 && parentErrors===1 && retainedAfter===2
    "#).unwrap(), JsValue::Bool(true));
    // Revocation must not prevent corresponding-object realm capture. The
    // actual one-shot callback still throws and reports to its own global.
    driver.run_script(r#"
        var revokedCheckpoint=Proxy.revocable(()=>{},{});
        queueMicrotask(revokedCheckpoint.proxy);revokedCheckpoint.revoke();
        queueMicrotask(()=>retainedAfter++);
    "#).unwrap();
    bindings.parser_microtask_checkpoint(&mut driver.js).unwrap();
    assert_eq!(driver.run_script(r#"
        retainedWindow.retainedErrors.length===1 && parentErrors===2 && retainedAfter===3
    "#).unwrap(), JsValue::Bool(true));
}
