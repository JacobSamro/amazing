//! DOM bindings driver additions owned by the dom-bindings lane.
use super::*;
use amazing_dom::events::{EventClass, EventId};

impl RuntimeBindingHost<'_> {
    pub(super) fn dom_frozen_array(&mut self, values: Vec<JsValue>) -> Result<JsValue, Throw> {
        let array = self.js.binding_create_array(values)?;
        let JsValue::Object(object) = &array else { unreachable!() };
        {
            let mut object = object.borrow_mut();
            for key in object.own_property_keys() {
                let mut property = object.own_prop(key.clone()).unwrap().clone();
                property.configurable = false;
                if property.getter.is_none() && property.setter.is_none() { property.writable = false; }
                object.define(key, property).map_err(|_| Throw::type_error("Cannot freeze message ports"))?;
            }
            object.extensible = false;
        }
        Ok(array)
    }

    pub(super) fn install_dom_unscopables(&mut self) -> Result<(), Throw> {
        for interface in [Interface::Document, Interface::DocumentFragment, Interface::Element,
            Interface::CharacterData, Interface::DocumentType] {
            let prototype = self.prototype(interface)
                .ok_or_else(|| Throw::type_error("Missing DOM interface prototype"))?;
            let unscopables = self.js.binding_create_object(None)?;
            for name in amazing_bindings::unscopable_members(interface) {
                self.define_data_property(&unscopables, name, JsValue::Bool(true),
                    true, true, true)?;
            }
            self.js.binding_define_symbol_data_property(&prototype,
                amazing_js::value::WellKnownSymbol::Unscopables.symbol(), unscopables,
                BindingPropertyFlags { writable: false, enumerable: false, configurable: true })?;
        }
        Ok(())
    }

    pub(super) fn dom_window_event_target_slot(&self, value: &JsValue) -> Option<u64> {
        if self.is_nullish(value) || self.same_value(value, &self.js.binding_global_object()) {
            Some(self.state.borrow().window_event_target_slot)
        } else { None }
    }

    pub(super) fn dom_window_event_target_value(&self, slot: u64) -> Option<JsValue> {
        (self.state.borrow().window_event_target_slot == slot)
            .then(|| self.js.binding_global_object())
    }
    pub(super) fn dom_event_interface(&self, key: PlatformKey) -> Result<Interface, Throw> {
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        let event = host.dom.events().event_checked(EventId::from_slot(key.slot as usize))
            .ok_or_else(|| Throw::type_error("Invalid Event wrapper"))?;
        Ok(match event.class() {
            EventClass::Event => Interface::Event,
            EventClass::Custom => Interface::CustomEvent,
            EventClass::Ui => Interface::UIEvent,
            EventClass::Focus => Interface::FocusEvent,
            EventClass::Mouse => Interface::MouseEvent,
            EventClass::Wheel => Interface::WheelEvent,
            EventClass::Keyboard => Interface::KeyboardEvent,
            EventClass::Composition => Interface::CompositionEvent,
            EventClass::Message => Interface::MessageEvent,
        })
    }

    pub(super) fn install_selection_global(&mut self) -> Result<(), Throw> {
        let function = self.native("getSelection", 0,
            NativeEntry::Operation(Operation::GetSelection), HostCallPolicy::CallOnly);
        let global = self.js.binding_global_object();
        self.define_data_property(&global, "getSelection", function, true, false, true)
    }

    pub(super) fn install_window_event_target(&mut self) -> Result<(), Throw> {
        let prototype = self.prototype(Interface::EventTarget)
            .ok_or_else(|| Throw::type_error("Missing EventTarget prototype"))?;
        self.js.binding_set_global_prototype(&prototype)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_message_event_preserves_identity_and_freezes_ports_without_user_code() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        // IPC's temporary global installer is adopted separately by its owner.
        // Exercise the real registered binding constructor and host adapter here.
        let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
        let prototype = adapter.prototype(Interface::MessageEvent).unwrap();
        let constructor = adapter.get_property(&prototype, "constructor").unwrap();
        let global = adapter.js.binding_global_object();
        adapter.define_data_property(&global, "NativeMessageEvent", constructor, true, false, true).unwrap();
        let result = js.run_script(r#"
            Object.freeze = function() { throw new Error('must not call user freeze'); };
            const legacy = document.createEvent('MessageEvent');
            let nullPortsTypeError = false;
            try { legacy.initMessageEvent('message', false, false, null, '', '', null, null); }
            catch (e) { nullPortsTypeError = e instanceof TypeError; }
            const payload = {};
            const event = new NativeMessageEvent('message', {data: payload, source: window});
            let rejected = false;
            try { event.ports.push(1); } catch (e) { rejected = true; }
            event instanceof Event && event instanceof NativeMessageEvent &&
                NativeMessageEvent.length === 1 && event.data === payload &&
                event.source === window && event.ports === event.ports &&
                Object.isFrozen(event.ports) && event.ports.length === 0 && rejected && nullPortsTypeError;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }
}
