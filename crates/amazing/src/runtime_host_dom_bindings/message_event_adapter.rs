//! Canonical MessageEvent payload and messaging EventTarget adapter.
use super::*;
use amazing_dom::events::EventTarget;
impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn dom_materialize_message_event(&mut self,value:&JsValue)
        -> Result<(),Throw> {
        let JsValue::Object(object)=value else { return Err(Throw::type_error("Expected a MessageEvent")); };
        if object.borrow().own_prop("isTrusted").is_none() {
            let getter=self.messaging_trusted_getter()?;
            self.define_accessor_property(value,"isTrusted",Some(getter),None,true,false)?;
        }
        Ok(())
    }

    /// WebIDL sequence<MessagePort>. Read @@iterator with the engine's actual
    /// Get, preserving Proxy/getter behavior; close on element conversion failure.
    pub(in crate::runtime_binding_host) fn dom_message_ports(&mut self, value: &JsValue)
        -> Result<Vec<JsValue>, Throw> {
        self.require_object(value)?;
        let symbol=amazing_js::value::WellKnownSymbol::Iterator.symbol();
        let method=self.js.binding_get_property_key(value,symbol.into())?;
        if !self.js.binding_is_callable(&method) {
            return Err(Throw::type_error("MessageEvent ports must be iterable"));
        }
        let iterator=self.js.binding_call(&method,value.clone(),Vec::new())?;
        self.require_object(&iterator)?;
        let next=self.js.binding_get_property(&iterator,"next")?;
        let mut ports=Vec::new();
        loop {
            let step=self.js.binding_call(&next,iterator.clone(),Vec::new())?;
            self.require_object(&step)?;
            let done=self.js.binding_get_property(&step,"done")?;
            if self.js.binding_to_boolean(&done) { return Ok(ports); }
            let port=self.js.binding_get_property(&step,"value")?;
            if !self.messaging_is_port(&port) {
                let original=Throw::type_error("Expected a MessagePort");
                // With an abrupt throw completion, IteratorClose preserves that
                // completion even when retrieving/calling return throws.
                if let Ok(close)=self.js.binding_get_property(&iterator,"return") {
                    if self.js.binding_is_callable(&close) {
                        let _=self.js.binding_call(&close,iterator.clone(),Vec::new());
                    }
                }
                return Err(original);
            }
            ports.push(port);
        }
    }
    pub(in crate::runtime_binding_host) fn dom_message_handler(&mut self, value:&JsValue,
        name:&str, setter:bool, new:Option<&JsValue>) -> Result<JsValue,Throw> {
        let window_receiver=if self.js.binding_is_nullish(value) {
            self.js.binding_global_object()
        } else {value.clone()};
        let slot=self.messaging_event_target_slot(value)
            .or_else(|| self.dom_window_event_target_slot(&window_receiver))
            .ok_or_else(|| Throw::type_error("Expected a messaging EventTarget"))?;
        let target=EventTarget::External(slot);
        if !setter {
            let root=self.with_dom(|dom| Ok(dom.events().event_handler(target,name)))?;
            return Ok(root.and_then(|r| self.root_value(r)).unwrap_or(JsValue::Null));
        }
        let value=new.cloned().unwrap_or(JsValue::Undefined);
        // EventHandler's legacy TreatNonObjectAsNull conversion keeps object
        // callback values; non-callable objects are never invoked as handlers.
        let root=matches!(value,JsValue::Object(_)).then(|| self.retain_value(value));
        let old=self.with_dom(|dom| Ok(dom.events_mut().set_event_handler(target,name,root)));
        match old {
            Ok(old)=>{if let Some(old)=old {self.release_value(old);}},
            Err(error)=>{if let Some(root)=root {self.release_value(root);}return Err(error);}
        }
        Ok(JsValue::Undefined)
    }
    pub(in crate::runtime_binding_host) fn dom_fire_message_event(&mut self, slot:u64,
        kind:&str,data:JsValue,origin:&str,ports:Vec<JsValue>)->Result<(),Throw> {
        amazing_bindings::dispatch_message_event(self,EventTarget::External(slot),kind,
            amazing_bindings::MessageEventPayload{data,origin:origin.encode_utf16().collect(),
                last_event_id:Vec::new(),source:None,ports})?;
        Ok(())
    }
}
