//! DOM bindings driver additions owned by the dom-bindings lane.
use super::*;
use amazing_dom::events::{EventClass, EventId};

impl RuntimeBindingHost<'_> {
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
        let global = self.js.binding_global_object();
        for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
            let method = self.js.binding_get_property(&prototype, name)?;
            self.define_data_property(&global, name, method, true, false, true)?;
        }
        Ok(())
    }
}
