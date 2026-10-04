//! Genuine AbortSignal allocation realms and WebIDL iterable conversion.
use super::*;
impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn dom_register_abort(&mut self, signal: AbortSignalId)
        -> Result<(), Throw> {
        self.with_dom(|dom| dom.abort_signal_checked(signal))?;
        let realm = self.js.binding_current_realm();
        let mut state = self.state.borrow_mut();
        if state.abort_creation_realms.contains_key(&signal) {
            return Err(Throw::type_error("AbortSignal allocation already registered"));
        }
        state.abort_creation_realms.insert(signal, realm);
        Ok(())
    }
    pub(in crate::runtime_binding_host) fn dom_abort_realm(&self, signal: AbortSignalId)
        -> Result<RealmHandle, Throw> {
        let state = self.state.borrow();
        state.host.borrow().dom.abort_signal_checked(signal)
            .map_err(|_| Throw::type_error("Invalid AbortSignal"))?;
        state.abort_creation_realms.get(&signal).copied()
            .ok_or_else(|| Throw::type_error("AbortSignal creation realm is unregistered"))
    }
    pub(in crate::runtime_binding_host) fn dom_fire_abort(&mut self, signal: AbortSignalId)
        -> Result<(), Throw> {
        let realm = self.dom_abort_realm(signal)?;
        let slot = self.with_dom(|dom| dom.abort_signal_target(signal))?;
        let state = self.state.clone();
        self.js.binding_with_realm(realm, |js| {
            let mut host = RuntimeBindingHost { js, state };
            amazing_bindings::dispatch_user_agent_event(&mut host,
                amazing_dom::events::EventTarget::External(slot), "abort",
                amazing_dom::events::EventInit::default()).map(|_| ())
        })
    }
    pub(in crate::runtime_binding_host) fn dom_abort_sequence(&mut self, value: &JsValue)
        -> Result<Vec<AbortSignalId>, Throw> {
        self.require_object(value)?;
        let symbol = amazing_js::value::WellKnownSymbol::Iterator.symbol();
        let method = self.js.binding_get_property_key(value, symbol.into())?;
        if !self.js.binding_is_callable(&method) {
            return Err(Throw::type_error("AbortSignal.any requires an iterable"));
        }
        let iterator = self.js.binding_call(&method, value.clone(), Vec::new())?;
        self.require_object(&iterator)?;
        let next = self.js.binding_get_property(&iterator, "next")?;
        let mut signals = Vec::new();
        loop {
            let step = self.js.binding_call(&next, iterator.clone(), Vec::new())?;
            self.require_object(&step)?;
            let done = self.js.binding_get_property(&step, "done")?;
            if self.js.binding_to_boolean(&done) { return Ok(signals); }
            let value = self.js.binding_get_property(&step, "value")?;
            let result = (|| {
                let key = self.platform_object(&value, InterfaceBrand::ABORT_SIGNAL)
                    .ok_or_else(|| Throw::type_error("Expected an AbortSignal"))?;
                if key.arena != self.state.borrow().arena || key.generation != 1 {
                    return Err(Throw::type_error("Invalid AbortSignal"));
                }
                let signal = AbortSignalId(key.slot as usize);
                self.with_dom(|dom| dom.abort_signal_checked(signal))?;
                Ok(signal)
            })();
            match result {
                Ok(signal) => signals.push(signal),
                Err(original) => {
                    if let Ok(close) = self.js.binding_get_property(&iterator, "return") {
                        if self.js.binding_is_callable(&close) {
                            let _ = self.js.binding_call(&close, iterator.clone(), Vec::new());
                        }
                    }
                    return Err(original);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scheduler() -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 128).unwrap();
        scheduler.set_location_url("https://example.test/abort");
        scheduler
    }
    fn yes(scheduler: &mut crate::js_scheduler::DriverScheduler, source: &str) {
        let value = scheduler.run_script(source).unwrap();
        assert!(matches!(value, JsValue::Bool(true)), "{value:?}");
    }
    #[test]
    fn abort_interface_uses_inherited_accessors_and_one_handler_position() {
        yes(&mut scheduler(), "var c=new AbortController(),s=c.signal,trace=[];s.addEventListener('abort',()=>trace.push('first'));s.onabort=()=>trace.push('old');s.addEventListener('abort',()=>trace.push('last'));s.onabort=()=>trace.push('new');var stable=c.signal===s;var inherited=!Object.prototype.hasOwnProperty.call(s,'onabort')&&!Object.prototype.hasOwnProperty.call(c,'signal');c.abort('reason');stable&&inherited&&s instanceof EventTarget&&trace.join(',')==='first,new,last'&&s.reason==='reason'");
    }
    #[test]
    fn abort_handler_noncallable_objects_and_clear_readd_use_canonical_registry() {
        yes(&mut scheduler(), "var c=new AbortController(),s=c.signal,trace=[],obj={get handleEvent(){throw 'wrong';}};s.onabort=obj;var retained=s.onabort===obj;s.addEventListener('abort',()=>trace.push('first'));s.onabort=null;s.onabort=()=>{trace.push('handler');s.onabort=null;};s.addEventListener('abort',()=>trace.push('last'));c.abort();retained&&s.onabort===null&&trace.join(',')==='first,handler,last'");
    }
    #[test]
    fn abort_any_observes_iterator_protocol_and_closes_failed_brand_conversion() {
        yes(&mut scheduler(), "var c=new AbortController(),trace=[],iterable={get length(){throw 'arraylike';},get [Symbol.iterator](){trace.push('iterator');return function(){var n=0;return {get next(){trace.push('next');return function(){trace.push('step');return {done:n++>0,value:c.signal};};}};}}};var s=AbortSignal.any(iterable);c.abort(7);trace.join(',')==='iterator,next,step,step'&&s.aborted&&s.reason===7");
        yes(&mut scheduler(), "var trace=[],error=null,iterable={[Symbol.iterator](){return {next(){return {done:false,value:{}};},get return(){trace.push('return');return function(){trace.push('close');throw 'suppressed';};}};}};try{AbortSignal.any(iterable);}catch(e){error=e;}error instanceof TypeError&&trace.join(',')==='return,close'");
    }
    #[test]
    fn abort_any_preserves_getter_throws_and_rejects_arraylike_only_values() {
        yes(&mut scheduler(), "var token={},caught=null,returns=0,x={[Symbol.iterator](){return {next(){return {done:false,get value(){throw token;}};},return(){returns++;return {};}};}};try{AbortSignal.any(x);}catch(e){caught=e;}var rejected=false;try{AbortSignal.any({length:0});}catch(e){rejected=e instanceof TypeError;}caught===token&&returns===0&&rejected");
    }
    #[test]
    fn dependent_abort_signals_share_reason_and_finish_state_before_events() {
        yes(&mut scheduler(), "var c=new AbortController(),d=new AbortController(),a=AbortSignal.any([c.signal,c.signal,d.signal]),b=AbortSignal.any([a]),reason={},trace=[],early=false;c.signal.addEventListener('abort',e=>{trace.push('source');early=a.aborted&&b.aborted;});a.onabort=()=>trace.push('a');b.onabort=()=>trace.push('b');c.abort(reason);d.abort('other');var thrown=null;try{b.throwIfAborted();}catch(e){thrown=e;}var already=AbortSignal.any([c.signal]);a.reason===reason&&b.reason===reason&&already.reason===reason&&thrown===reason&&early&&trace.join(',')==='source,a,b'");
    }
    #[test]
    fn abortable_listener_duplicates_and_reentrant_abort_keep_native_lifetime() {
        yes(&mut scheduler(), "var target=new EventTarget(),a=new AbortController(),b=new AbortController(),hits=0,fn=()=>hits++;target.addEventListener('x',fn,{signal:a.signal});target.addEventListener('x',fn,{signal:b.signal});b.abort();target.dispatchEvent(new Event('x'));a.abort();target.dispatchEvent(new Event('x'));var c=new AbortController(),order=[];c.signal.addEventListener('abort',()=>{order.push('first');c.abort('again');});c.signal.onabort=()=>order.push('handler');c.abort(4);hits===1&&c.signal.reason===4&&order.join(',')==='first,handler'");
    }
    #[test]
    fn abort_timeout_enforce_range_truncates_before_bounds_and_keeps_conversion_abrupt() {
        yes(&mut scheduler(), "var s=AbortSignal.timeout(-0.9),bad=0;for(var x of [-1,NaN,Infinity,18446744073709551616]){try{AbortSignal.timeout(x);}catch(e){if(e instanceof TypeError)bad++;}}var token={},caught=null;try{AbortSignal.timeout({valueOf(){throw token;}});}catch(e){caught=e;}s instanceof AbortSignal&&bad===4&&caught===token");
    }
    #[test]
    fn request_and_observable_native_allocations_register_before_signal_exposure() {
        yes(&mut scheduler(), "var c=new AbortController(),r=new Request('https://example.test/a',{signal:c.signal}),copy=r.clone(),plain=new Request('https://example.test/b');var a=r.signal,b=copy.signal;var before=a!==c.signal&&b!==a&&a instanceof AbortSignal&&b instanceof AbortSignal&&plain.signal instanceof AbortSignal;c.abort('request-reason');before&&a.aborted&&b.aborted&&a.reason==='request-reason'&&b.reason==='request-reason'");
        yes(&mut scheduler(), "var signals=[],values=[],source=new Observable(s=>{signals.push(s.signal);s.next(2);s.complete();});source.map(x=>x+1).subscribe(x=>values.push(x));source.flatMap(x=>new Observable(s=>{signals.push(s.signal);s.next(x*2);s.complete();})).subscribe(x=>values.push(x));signals.length===3&&signals.every(s=>s instanceof AbortSignal)&&values.join(',')==='3,4'");
    }
    #[test]
    fn foreign_abort_allocation_getters_and_native_targets_keep_genuine_creation_realm() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe srcdoc=''></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 128).unwrap();
        scheduler.set_location_url("https://example.test/foreign-abort");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let state = scheduler.test_bindings().state.clone();
        let root = scheduler.js.binding_current_realm();
        let child = *state.borrow().realm_documents.keys().find(|realm| **realm != root).unwrap();
        let values = scheduler.js.binding_with_realm(child, |js| {
            Ok([js.run_script("AbortController")?, js.run_script("AbortSignal.prototype")?,
                js.run_script("Event.prototype")?, js.run_script("DOMException.prototype")?])
        }).unwrap();
        let global = scheduler.js.binding_global_object();
        for (name, value) in ["ChildController", "childSignalPrototype", "childEventPrototype", "childExceptionPrototype"].into_iter().zip(values) {
            scheduler.js.binding_define_data_property(&global, name, value, BindingPropertyFlags::METHOD).unwrap();
        }
        yes(&mut scheduler, "var foreignController=new ChildController(),foreignSignal=foreignController.signal,rootGetter=Object.getOwnPropertyDescriptor(AbortController.prototype,'signal').get,seen=null;var same=rootGetter.call(foreignController)===foreignSignal;foreignSignal.addEventListener('abort',e=>{seen=e;});foreignController.abort();same&&Object.getPrototypeOf(foreignSignal)===childSignalPrototype&&Object.getPrototypeOf(seen)===childEventPrototype&&seen.target===foreignSignal&&seen.isTrusted&&Object.getPrototypeOf(foreignSignal.reason)===childExceptionPrototype");
        let timeout = scheduler.js.binding_with_realm(child, |js| js.run_script("AbortSignal.timeout(100000)")).unwrap();
        let handle = scheduler.js.binding_host_handle(&timeout, InterfaceBrand::ABORT_SIGNAL.0).unwrap();
        scheduler.js.binding_define_data_property(&global, "childTimeout", timeout, BindingPropertyFlags::METHOD).unwrap();
        // This invokes the actual native expiry callback boundary, independently
        // of the still-limited scheduler's global eligibility policy.
        scheduler.test_bindings().clone().fire_abort_timeout(&mut scheduler.js, AbortSignalId(handle.slot as usize)).unwrap();
        yes(&mut scheduler, "childTimeout.aborted&&childTimeout.reason.name==='TimeoutError'&&Object.getPrototypeOf(childTimeout.reason)===childExceptionPrototype");
    }

}
