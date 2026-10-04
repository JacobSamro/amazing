//! Canonical MessageEvent and messaging EventTarget integration.
use super::*;
fn scheduler() -> crate::js_scheduler::DriverScheduler {
    let host=Rc::new(RefCell::new(BrowserDomHost {
        dom:amazing_html::parse("<body></body>"),native_forwarded:false,
    }));
    let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,128).unwrap();
    scheduler.set_location_url("https://example.test/message-events");
    scheduler
}
fn check(source:&str) {
    let mut scheduler=scheduler();
    let value=scheduler.run_script(source).unwrap();
    assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
}
#[test]
fn canonical_constructor_legacy_alias_and_event_inheritance_agree() {
    check(r#"(function(){
        const e=new MessageEvent('custom',{data:42,bubbles:true,cancelable:true,composed:true});
        const a=document.createEvent('MessageEvent'),b=document.createEvent('messageevent');
        return MessageEvent.length===1&&Object.getPrototypeOf(MessageEvent)===Event&&Object.prototype.toString.call(e)==='[object MessageEvent]'&&e instanceof MessageEvent&&e instanceof Event&&
          Object.getPrototypeOf(MessageEvent.prototype)===Event.prototype&&
          Object.getPrototypeOf(a)===MessageEvent.prototype&&Object.getPrototypeOf(b)===MessageEvent.prototype&&
          e.type==='custom'&&e.data===42&&e.bubbles&&e.cancelable&&e.composed&&!e.isTrusted&&
          e.source===null&&e.origin===''&&e.lastEventId===''&&e.ports.length===0&&
          !Object.prototype.hasOwnProperty.call(e,'data')&&Object.getOwnPropertyDescriptor(e,'isTrusted').configurable===false;
    })()"#);
}
#[test]
fn dictionary_order_abrupt_identity_and_string_domains_are_canonical() {
    check(r#"(function(){
      let order=[];const dict={};
      for(const key of ['bubbles','cancelable','composed','data','lastEventId','origin','ports','source'])
        Object.defineProperty(dict,key,{get(){order.push(key);return undefined;}});
      const e=new MessageEvent('m',dict),token={};let same=false;
      try{new MessageEvent('m',{get origin(){throw token;}});}catch(x){same=x===token;}
      const units=new MessageEvent('\uD800',{origin:'\uD800',lastEventId:'\uD800'});
      let missing=false;try{new MessageEvent();}catch(x){missing=x instanceof TypeError;}
      return order.join(',')==='bubbles,cancelable,composed,data,lastEventId,origin,ports,source'&&same&&missing&&
        e.data===null&&units.type==='\uD800'&&units.origin==='\uFFFD'&&units.lastEventId==='\uD800';
    })()"#);
}
#[test]
fn port_union_iterable_conversion_close_and_frozen_identity_are_genuine() {
    check(r#"(function(){
      const p=new MessageChannel().port1;let gets=0,closed=0;
      const iterable={get [Symbol.iterator](){gets++;return function(){let n=0;return {
        next(){return n++===0?{value:p,done:false}:{done:true};}};};}};
      const e=new MessageEvent('m',{source:p,ports:iterable});
      const bad={[Symbol.iterator](){return {next(){return {value:{},done:false};},return(){closed++;return {};}};}};
      let rejected=false;try{new MessageEvent('m',{ports:bad});}catch(x){rejected=x instanceof TypeError;}
      let fake=false;try{new MessageEvent('m',{source:Object.create(MessagePort.prototype)});}catch(x){fake=x instanceof TypeError;}
      const w=new MessageEvent('m',{source:window});
      return gets===1&&closed===1&&rejected&&fake&&e.source===p&&e.ports[0]===p&&e.ports===e.ports&&
        Object.isFrozen(e.ports)&&w.source===window;
    })()"#);
}
#[test]
fn messaging_objects_share_eventtarget_identity_and_listener_protocol() {
    check(r#"(function(){
      const p=new MessageChannel().port1,bc=new BroadcastChannel('canonical');let calls=[];
      function once(e){calls.push('once');if(e.target!==p||e.currentTarget!==p||e.composedPath()[0]!==p)throw 'identity';}
      EventTarget.prototype.addEventListener.call(p,'x',once,{once:true});
      const obj={handleEvent(e){calls.push('obj');e.preventDefault();}};
      p.addEventListener('x',obj);
      const e=new MessageEvent('x',{cancelable:true});const result=p.dispatchEvent(e);
      p.removeEventListener('x',obj);p.dispatchEvent(new Event('x'));
      let wrong=false;try{p.dispatchEvent({type:'x'});}catch(x){wrong=x instanceof TypeError;}
      bc.close();return p instanceof EventTarget&&bc instanceof EventTarget&&p.addEventListener===EventTarget.prototype.addEventListener&&Object.getPrototypeOf(MessagePort)===EventTarget&&Object.getPrototypeOf(BroadcastChannel)===EventTarget&&Object.prototype.toString.call(p)==='[object MessagePort]'&&Object.prototype.toString.call(bc)==='[object BroadcastChannel]'&&calls.join(',')==='once,obj'&&
        result===false&&e.target===p&&e.currentTarget===null&&!e.isTrusted&&wrong;
    })()"#);
}
#[test]
fn handler_replacement_order_and_dispatch_time_legacy_init_use_one_registry() {
    check(r#"(function(){
      const p=new MessageChannel().port1;let trace=[],seen;
      p.onmessage=function(e){trace.push('old');};
      p.addEventListener('message',function(e){trace.push('listener');e.initMessageEvent('changed',false,false,9);seen=e;});
      const fn=function(e){trace.push('new');return false;};p.onmessage=fn;
      const e=new MessageEvent('message',{data:7,cancelable:true});const result=p.dispatchEvent(e);
      p.onmessage=null;
      const legacy=document.createEvent('MessageEvent');legacy.initMessageEvent('message',true,true,11,'o','id',p,[p]);
      return trace.join(',')==='new,listener'&&seen===e&&e.type==='message'&&e.data===7&&result===false&&
        p.onmessage===null&&legacy.data===11&&legacy.source===p&&legacy.ports[0]===p&&legacy.lastEventId==='id';
    })()"#);
}
#[test]
fn receiving_checkpoint_keeps_canonical_event_payload_and_trust() {
    let mut scheduler=scheduler();
    scheduler.run_script(r#"var delivered=null;var channel=new MessageChannel();
      channel.port1.onmessage=function(e){delivered=e;};channel.port2.postMessage({answer:42});"#).unwrap();
    scheduler.pump_now(true).unwrap();
    let value=scheduler.run_script(r#"delivered instanceof MessageEvent&&delivered instanceof Event&&delivered.isTrusted&&
      delivered.data.answer===42&&delivered.target===channel.port1&&delivered.currentTarget===null&&
      Object.isFrozen(delivered.ports)&&!delivered.bubbles&&!delivered.cancelable&&
      !Object.prototype.hasOwnProperty.call(delivered,'data')"#).unwrap();
    assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
}
#[test]
fn listener_options_abort_and_reentrant_mutation_follow_native_dispatch() {
    check(r#"(function(){
      const p=new MessageChannel().port1,controller=new AbortController();let trace=[];
      const removed=function(){trace.push('removed');};
      p.addEventListener('x',function(e){trace.push('first');p.removeEventListener('x',removed);},{once:true});
      p.addEventListener('x',removed);
      p.addEventListener('x',function(){trace.push('aborted');},{signal:controller.signal});controller.abort();
      p.addEventListener('x',function(e){trace.push('passive');e.preventDefault();},{passive:true});
      const first=new Event('x',{cancelable:true});let a=p.dispatchEvent(first);
      const second=new Event('x',{cancelable:true});let b=p.dispatchEvent(second);
      p.addEventListener('stop',function(e){trace.push('stop');e.stopImmediatePropagation();});
      p.addEventListener('stop',function(){trace.push('wrong');});p.dispatchEvent(new Event('stop'));
      return a&&b&&!first.defaultPrevented&&trace.join(',')==='first,passive,passive,stop';
    })()"#);
}
#[test]
fn actual_foreign_allocations_and_receiving_events_preserve_creation_prototypes() {
    let host=Rc::new(RefCell::new(BrowserDomHost {
        dom:amazing_html::parse("<body><iframe srcdoc=''></iframe></body>"),native_forwarded:false,
    }));
    let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,128).unwrap();
    scheduler.set_location_url("https://example.test/message-realms");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    let state=scheduler.test_bindings().state.clone();
    let root=scheduler.js.binding_current_realm();
    let child={let state=state.borrow();*state.realm_documents.keys().find(|realm|**realm!=root).unwrap()};
    let channel=scheduler.js.binding_with_realm(child,|js|js.run_script("new MessageChannel()")).unwrap();
    let global=scheduler.js.binding_global_object();
    scheduler.js.binding_define_data_property(&global,"foreignChannel",channel.clone(),BindingPropertyFlags::METHOD).unwrap();
    scheduler.run_script("var foreignEvent=null;foreignChannel.port1.onmessage=function(e){foreignEvent=e;};foreignChannel.port2.postMessage(17);").unwrap();
    scheduler.pump_now(true).unwrap();
    let event=scheduler.run_script("foreignEvent").unwrap();
    scheduler.js.binding_with_realm(child,|js| {
        let global=js.binding_global_object();
        js.binding_define_data_property(&global,"foreignChannel",channel,BindingPropertyFlags::METHOD)?;
        js.binding_define_data_property(&global,"foreignEvent",event,BindingPropertyFlags::METHOD)?;
        let value=js.run_script("foreignEvent instanceof MessageEvent&&foreignChannel.port1 instanceof MessagePort&&foreignChannel.port1 instanceof EventTarget&&foreignEvent.target===foreignChannel.port1&&foreignEvent.data===17&&foreignEvent.isTrusted")?;
        assert!(matches!(value,JsValue::Bool(true)),"{value:?}");Ok(())
    }).unwrap();
}
#[test]
fn foreign_message_redispatch_retains_exact_exported_wrapper_and_prototype() {
    let host=Rc::new(RefCell::new(BrowserDomHost {
        dom:amazing_html::parse("<body><iframe srcdoc=''></iframe></body>"),native_forwarded:false,
    }));
    let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,128).unwrap();
    scheduler.set_location_url("https://example.test/message-redispatch");
    scheduler.dispatch_pending_iframe_loads().unwrap();
    let state=scheduler.test_bindings().state.clone();let root=scheduler.js.binding_current_realm();
    let child={let state=state.borrow();*state.realm_documents.keys().find(|r|**r!=root).unwrap()};
    let event=scheduler.js.binding_with_realm(child,|js|js.run_script("new MessageEvent('x',{data:23})")).unwrap();
    let global=scheduler.js.binding_global_object();
    scheduler.js.binding_define_data_property(&global,"foreignMessage",event.clone(),BindingPropertyFlags::METHOD).unwrap();
    let value=scheduler.run_script("var observed=null;var target=new EventTarget();target.addEventListener('x',function(e){observed=e;});target.dispatchEvent(foreignMessage);observed===foreignMessage&&observed.data===23&&observed.target===target&&!observed.isTrusted").unwrap();
    assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
    let observed=scheduler.run_script("observed").unwrap();assert_eq!(observed,event);
    // Borrow the actual child constructor into the root caller. Allocation and
    // subsequent target wrapping must still use the genuine child callee realm.
    let constructor=scheduler.js.binding_with_realm(child,|js|js.run_script("EventTarget")).unwrap();
    scheduler.js.binding_define_data_property(&global,"childTargetConstructor",constructor,BindingPropertyFlags::METHOD).unwrap();
    let value=scheduler.run_script("var childTarget=new childTargetConstructor();var rootMessage=new MessageEvent('y',{data:29});var reverseObserved=null;childTarget.addEventListener('y',function(e){reverseObserved=e;});childTarget.dispatchEvent(rootMessage);reverseObserved===rootMessage&&reverseObserved.target===childTarget&&reverseObserved.data===29&&!reverseObserved.isTrusted").unwrap();
    assert!(matches!(value,JsValue::Bool(true)),"{value:?}");
    let target=scheduler.run_script("childTarget").unwrap();
    scheduler.js.binding_with_realm(child,|js| {
        let global=js.binding_global_object();
        js.binding_define_data_property(&global,"borrowedTarget",target,BindingPropertyFlags::METHOD)?;
        let value=js.run_script("Object.getPrototypeOf(borrowedTarget)===EventTarget.prototype&&borrowedTarget instanceof EventTarget")?;
        assert!(matches!(value,JsValue::Bool(true)),"{value:?}");Ok(())
    }).unwrap();
}
