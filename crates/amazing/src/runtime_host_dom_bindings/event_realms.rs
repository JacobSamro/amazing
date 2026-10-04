//! Immutable native Event allocation realm, shared by all Event subclasses.
use super::*;
impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn dom_register_event(&mut self, event: EventId)
        -> Result<(), Throw> {
        let realm = self.js.binding_current_realm();
        self.dom_register_event_in_realm(event, realm)
    }
    pub(in crate::runtime_binding_host) fn dom_register_event_for_target(&mut self,
        event: EventId, target: amazing_dom::events::EventTarget) -> Result<(), Throw> {
        let realm = match target {
            amazing_dom::events::EventTarget::Node(node) => self.dom_node_realm(node)?,
            amazing_dom::events::EventTarget::External(slot) => {
                self.with_dom(|dom| if dom.has_event_target(slot) { Ok(()) }
                    else { Err(amazing_dom::DomError::InvalidStateError) })?;
                if let Some(realm) = self.html_window_event_target_realm(slot) { realm }
                else if let Some(realm) = self.messaging_target_allocation_realm(slot) { realm }
                else {
                    let signal = self.with_dom(|dom| Ok(dom.abort_signal_for_target(slot)))?;
                    match signal {
                        Some(signal) => self.dom_abort_realm(signal)?,
                        None => return Err(Throw::type_error("EventTarget creation realm is unregistered")),
                    }
                }
            }
        };
        self.dom_register_event_in_realm(event, realm)
    }
    fn dom_register_event_in_realm(&mut self, event: EventId, realm: RealmHandle)
        -> Result<(), Throw> {
        self.js.binding_realm_global(realm)?;
        self.with_dom(|dom| dom.events().event_checked(event)
            .map(|_| ()).ok_or(amazing_dom::DomError::InvalidStateError))?;
        let mut state = self.state.borrow_mut();
        if state.event_creation_realms.contains_key(&event.slot()) {
            return Err(Throw::type_error("Event allocation already registered"));
        }
        state.event_creation_realms.insert(event.slot(), realm);
        Ok(())
    }
    pub(in crate::runtime_binding_host) fn dom_event_realm(&self, key: PlatformKey)
        -> Result<RealmHandle, Throw> {
        let state = self.state.borrow();
        if key.arena != state.arena || key.generation != 1 {
            return Err(Throw::type_error("Invalid Event wrapper"));
        }
        state.host.borrow().dom.events().event_checked(EventId::from_slot(key.slot as usize))
            .ok_or_else(|| Throw::type_error("Invalid Event wrapper"))?;
        state.event_creation_realms.get(&(key.slot as usize)).copied()
            .ok_or_else(|| Throw::type_error("Event creation realm is unregistered"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::js_scheduler::DriverScheduler, RealmHandle) {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe srcdoc=''></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 128).unwrap();
        scheduler.set_location_url("https://example.test/event-realms");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let state = scheduler.test_bindings().state.clone();
        let root = scheduler.js.binding_current_realm();
        let child = *state.borrow().realm_documents.keys().find(|realm| **realm != root).unwrap();
        (scheduler, child)
    }
    fn expose(scheduler: &mut crate::js_scheduler::DriverScheduler, child: RealmHandle,
        name: &str, source: &str) {
        let value = scheduler.js.binding_with_realm(child, |js| js.run_script(source)).unwrap();
        let global = scheduler.js.binding_global_object();
        scheduler.js.binding_define_data_property(&global, name, value,
            BindingPropertyFlags::METHOD).unwrap();
    }
    fn yes(scheduler: &mut crate::js_scheduler::DriverScheduler, source: &str) {
        let value = scheduler.run_script(source).unwrap();
        assert!(matches!(value, JsValue::Bool(true)), "{value:?}");
    }
    #[test]
    fn every_event_constructor_keeps_foreign_identity_class_and_payload_on_redispatch() {
        let (mut scheduler, child) = fixture();
        for constructor in ["Event", "CustomEvent", "UIEvent", "FocusEvent", "MouseEvent",
            "WheelEvent", "KeyboardEvent", "CompositionEvent", "ErrorEvent",
            "HashChangeEvent", "PopStateEvent", "PageTransitionEvent", "MessageEvent", "PromiseRejectionEvent"] {
            expose(&mut scheduler, child, "foreignEvent", &format!(
                "new {constructor}('foreign',{{cancelable:true,detail:42,data:'payload',key:'K',deltaY:3,state:17,persisted:true,message:'error',promise:Promise.resolve(1),reason:'reason'}})"));
            expose(&mut scheduler, child, "foreignPrototype", &format!("{constructor}.prototype"));
            yes(&mut scheduler, "var target=new EventTarget(),seen=null,good=false;target.addEventListener('foreign',function(e){seen=e;good=e===foreignEvent&&Object.getPrototypeOf(e)===foreignPrototype&&e.target===target&&e.currentTarget===target&&this===target&&e.composedPath()[0]===target;e.preventDefault();});var result=target.dispatchEvent(foreignEvent);good&&seen===foreignEvent&&!result&&foreignEvent.defaultPrevented&&!foreignEvent.isTrusted&&foreignEvent.currentTarget===null");
        }
    }
    #[test]
    fn foreign_callback_and_object_listener_receive_the_original_root_event() {
        let (mut scheduler, child) = fixture();
        expose(&mut scheduler, child, "childDispatch", "EventTarget.prototype.dispatchEvent");
        expose(&mut scheduler, child, "ChildEvent", "Event");
        yes(&mut scheduler, "var rootEvent=new Event('root'),target=new EventTarget(),seen=null;target.addEventListener('root',{handleEvent(e){seen=e;}});childDispatch.call(target,rootEvent);seen===rootEvent&&Object.getPrototypeOf(seen)===Event.prototype");
        // A callback compiled in the child crosses back into its saved native Event realm.
        let event = scheduler.run_script("new Event('callback')").unwrap();
        let global = scheduler.js.binding_realm_global(child).unwrap();
        scheduler.js.binding_define_data_property(&global,"originalRootEvent",event.clone(),BindingPropertyFlags::METHOD).unwrap();
        expose(&mut scheduler, child, "childIdentityListener", "(function(e){globalThis.childSeen=e;globalThis.childIdentity=e===originalRootEvent;})");
        expose(&mut scheduler, child, "childObjectListener", "({handleEvent(e){globalThis.childObjectIdentity=e===originalRootEvent;}})");
        let root = scheduler.js.binding_global_object();
        scheduler.js.binding_define_data_property(&root,"originalRootEvent",event,BindingPropertyFlags::METHOD).unwrap();
        yes(&mut scheduler,"var target=new EventTarget();target.addEventListener('callback',childIdentityListener);target.addEventListener('callback',childObjectListener);target.dispatchEvent(originalRootEvent);originalRootEvent.target===target");
        let value=scheduler.js.binding_with_realm(child,|js|js.run_script("childIdentity&&childObjectIdentity&&childSeen===originalRootEvent")).unwrap();
        assert!(matches!(value,JsValue::Bool(true)));
    }
    #[test]
    fn legacy_created_events_preserve_foreign_identity_after_initialization() {
        let (mut scheduler, child) = fixture();
        for name in ["Event", "CustomEvent", "UIEvent", "MouseEvent", "KeyboardEvent", "FocusEvent", "MessageEvent"] {
            expose(&mut scheduler,child,"legacy",&format!("document.createEvent('{name}')"));
            yes(&mut scheduler,"var target=new EventTarget(),rejected=false,seen=null;try{target.dispatchEvent(legacy);}catch(e){rejected=e.name==='InvalidStateError';}legacy.initEvent('legacy',false,true);target.addEventListener('legacy',e=>{seen=e;});target.dispatchEvent(legacy);rejected&&seen===legacy&&!seen.isTrusted");
        }
    }
    #[test]
    fn borrowed_document_create_event_uses_actual_document_realm_in_both_directions() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"childDocument","document");
        expose(&mut scheduler,child,"childCreateEvent","Document.prototype.createEvent");
        expose(&mut scheduler,child,"childEventPrototype","Event.prototype");
        expose(&mut scheduler,child,"childMessagePrototype","MessageEvent.prototype");
        yes(&mut scheduler,"var a=document.createEvent.call(childDocument,'Event'),b=childCreateEvent.call(document,'Event'),m=document.createEvent.call(childDocument,'MessageEvent');Object.getPrototypeOf(a)===childEventPrototype&&Object.getPrototypeOf(b)===Event.prototype&&Object.getPrototypeOf(m)===childMessagePrototype");
    }
    #[test]
    fn javascript_event_subclass_keeps_custom_prototype_and_reentrant_dispatch_checks() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"derived","class Derived extends CustomEvent{};new Derived('derived',{detail:{value:9}})");
        expose(&mut scheduler,child,"derivedPrototype","Derived.prototype");
        yes(&mut scheduler,"var target=new EventTarget(),seen=null,rejected=false;target.addEventListener('derived',e=>{seen=e;try{target.dispatchEvent(e);}catch(error){rejected=error.name==='InvalidStateError';}});target.dispatchEvent(derived);seen===derived&&Object.getPrototypeOf(seen)===derivedPrototype&&seen.detail.value===9&&rejected");
    }
    #[test]
    fn retained_event_realm_survives_real_child_document_replacement() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"retainedEvent","new Event('retained')");
        expose(&mut scheduler,child,"oldEventPrototype","Event.prototype");
        scheduler.run_script("document.querySelector('iframe').srcdoc='<body>new</body>'").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        yes(&mut scheduler,"var target=new EventTarget(),seen=null;target.addEventListener('retained',e=>{seen=e;});target.dispatchEvent(retainedEvent);seen===retainedEvent&&Object.getPrototypeOf(seen)===oldEventPrototype");
    }
    #[test]
    fn native_payload_and_existing_object_dispatch_keep_allocation_and_trust() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"existing","new CustomEvent('native',{detail:23})");
        yes(&mut scheduler,"var target=new EventTarget(),seen=null;target.addEventListener('native',e=>{seen=e;});true");
        let event=scheduler.run_script("existing").unwrap();
        let target=scheduler.run_script("target").unwrap();
        let bindings=scheduler.test_bindings().clone();
        let state=bindings.state.clone();
        let mut host=RuntimeBindingHost { js:&mut scheduler.js,state };
        let slot=host.platform_object(&target,InterfaceBrand::EVENT_TARGET).unwrap().slot;
        amazing_bindings::dispatch_user_agent_event_object(&mut host,
            amazing_dom::events::EventTarget::External(slot),&event).unwrap();
        yes(&mut scheduler,"seen===existing&&seen.detail===23&&seen.isTrusted&&seen.target===target");
        yes(&mut scheduler,"target.dispatchEvent(existing);seen===existing&&!seen.isTrusted");
    }
    #[test]
    fn native_subclass_payload_allocates_before_foreign_listener_and_preserves_redispatch() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"childMousePrototype","MouseEvent.prototype");
        expose(&mut scheduler,child,"childTarget","new EventTarget()");
        yes(&mut scheduler,"var target=childTarget,seen=null;target.addEventListener('mouse',e=>{seen=e;});true");
        let target=scheduler.run_script("target").unwrap();
        let state=scheduler.test_bindings().state.clone();
        let slot=scheduler.js.binding_host_handle(&target,InterfaceBrand::EVENT_TARGET.0).unwrap().slot;
        {
            let mut host=RuntimeBindingHost { js:&mut scheduler.js,state };
            let mut event=amazing_dom::events::Event::new("mouse",
                amazing_dom::events::EventInit::default(),std::time::Duration::ZERO);
            event.set_class(EventClass::Mouse);
            amazing_bindings::dispatch_user_agent_event_with_payload(&mut host,
                amazing_dom::events::EventTarget::External(slot),event).unwrap();
        }
        yes(&mut scheduler,"var original=seen;Object.getPrototypeOf(seen)===childMousePrototype&&seen.isTrusted&&seen.target===target");
        yes(&mut scheduler,"target.dispatchEvent(original);seen===original&&!seen.isTrusted&&Object.getPrototypeOf(seen)===childMousePrototype");
        expose(&mut scheduler,child,"childMessagePrototype","MessageEvent.prototype");
        yes(&mut scheduler,"var messageSeen=null;target.addEventListener('message',e=>{messageSeen=e;});true");
        {
            let state=scheduler.test_bindings().state.clone();
            let mut host=RuntimeBindingHost { js:&mut scheduler.js,state };
            amazing_bindings::dispatch_message_event(&mut host,
                amazing_dom::events::EventTarget::External(slot),"message",
                amazing_bindings::MessageEventPayload { data:JsValue::num(37.),
                    origin:"https://sender.test".encode_utf16().collect(),
                    last_event_id:vec![],source:None,ports:vec![] }).unwrap();
        }
        yes(&mut scheduler,"var originalMessage=messageSeen;messageSeen.data===37&&messageSeen.origin==='https://sender.test'&&messageSeen.isTrusted&&messageSeen.target===target&&Object.getPrototypeOf(messageSeen)===childMessagePrototype");
        yes(&mut scheduler,"target.dispatchEvent(originalMessage);messageSeen===originalMessage&&!messageSeen.isTrusted&&Object.getPrototypeOf(messageSeen)===childMessagePrototype");
    }
    #[test]
    fn native_firing_uses_genuine_window_node_port_and_signal_target_realms() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"childEventPrototype","Event.prototype");
        for source in ["window", "document.createElement('button')",
            "new MessageChannel().port1", "new AbortController().signal"] {
            expose(&mut scheduler,child,"nativeTarget",source);
            yes(&mut scheduler,"var nativeSeen=null;nativeTarget.addEventListener('fired',e=>{nativeSeen=e;});true");
            let value=scheduler.run_script("nativeTarget").unwrap();
            let state=scheduler.test_bindings().state.clone();
            let mut host=RuntimeBindingHost { js:&mut scheduler.js,state };
            let target=if let Some(node)=host.node_brand(&value) {
                amazing_dom::events::EventTarget::Node(node)
            } else if let Some(slot)=host.window_event_target_slot(&value)
                .or_else(||host.external_event_target_slot(&value)) {
                amazing_dom::events::EventTarget::External(slot)
            } else {
                let key=host.platform_object(&value,InterfaceBrand::ABORT_SIGNAL).unwrap();
                let slot=host.with_dom(|dom|dom.abort_signal_target(AbortSignalId(key.slot as usize))).unwrap();
                amazing_dom::events::EventTarget::External(slot)
            };
            amazing_bindings::dispatch_user_agent_event(&mut host,target,"fired",
                amazing_dom::events::EventInit::default()).unwrap();
            yes(&mut scheduler,"nativeSeen.isTrusted&&nativeSeen.target===nativeTarget&&nativeSeen.currentTarget===null&&Object.getPrototypeOf(nativeSeen)===childEventPrototype");
            yes(&mut scheduler,"var original=nativeSeen;nativeTarget.dispatchEvent(original);nativeSeen===original&&!nativeSeen.isTrusted&&Object.getPrototypeOf(nativeSeen)===childEventPrototype");
        }
    }
    #[test]
    fn event_registration_rejects_forged_keys_and_never_overwrites_original_realm() {
        let (mut scheduler,child)=fixture();
        expose(&mut scheduler,child,"genuine","new Event('registered')");
        let value=scheduler.run_script("genuine").unwrap();
        let handle=scheduler.js.binding_host_handle(&value,InterfaceBrand::EVENT.0).unwrap();
        let state=scheduler.test_bindings().state.clone();
        let mut host=RuntimeBindingHost { js:&mut scheduler.js,state };
        let key=PlatformKey { arena:handle.arena,slot:handle.slot,generation:handle.generation };
        assert_eq!(host.dom_event_realm(key).unwrap(),child);
        assert!(host.dom_register_event(EventId::from_slot(handle.slot as usize)).is_err());
        assert_eq!(host.dom_event_realm(key).unwrap(),child);
        assert!(host.dom_event_realm(PlatformKey { arena:handle.arena.wrapping_add(1),..key }).is_err());
        assert!(host.dom_event_realm(PlatformKey { generation:2,..key }).is_err());
        assert!(host.dom_event_realm(PlatformKey { slot:u64::MAX,..key }).is_err());
    }

}
