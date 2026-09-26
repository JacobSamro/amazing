//! DOM bindings driver additions owned by the dom-bindings lane.
use super::*;
use amazing_dom::events::{EventClass, EventId};

impl RuntimeBindingHost<'_> {
    pub(super) fn dom_uses_inherited_accessors(brand: InterfaceBrand) -> bool {
        matches!(brand, InterfaceBrand::DOM_EXCEPTION | InterfaceBrand::DOM_STRING_MAP
            | InterfaceBrand::COLLECTION | InterfaceBrand::FORM_CONTROLS
            | InterfaceBrand::OPTIONS | InterfaceBrand::RADIO_NODE_LIST
            | InterfaceBrand::NODE_LIST | InterfaceBrand::STATIC_NODE_LIST
            | InterfaceBrand::FILTERED_NODE_LIST | InterfaceBrand::NAMED_NODE_MAP
            | InterfaceBrand::DOM_TOKEN_LIST | InterfaceBrand::DOM_STRING_LIST)
    }

    pub(super) fn dom_install_dataset_hooks(&mut self) -> Result<(), Throw> {
        let state = self.state.clone();
        let names = Rc::new(move |handle: HostHandle| {
            let state = state.borrow();
            let mut host = state.host.borrow_mut();
            if handle.arena != state.arena || handle.generation != 1
                || handle.brand != InterfaceBrand::DOM_STRING_MAP.0
                || handle.slot >= host.dom.node_count() as u64 {
                return Err(Throw::type_error("Invalid DOMStringMap"));
            }
            host.dom.dataset_names(NodeIdx(handle.slot as u32))
                .map_err(|_| Throw::type_error("Invalid DOMStringMap element"))
        });
        let get_state = self.state.clone();
        let named_getter = Rc::new(move |js: &mut Interpreter, handle: HostHandle, name: &[u16]| {
            let mut adapter = RuntimeBindingHost { js, state: get_state.clone() };
            let receiver = adapter.hook_receiver(handle)?;
            let name = adapter.string(name)?;
            invoke(&mut adapter, Operation::DatasetGet, receiver, &[name])
        });
        let set_state = self.state.clone();
        let named_setter = Rc::new(move |js: &mut Interpreter, handle: HostHandle, name: &[u16], value: JsValue| {
            let mut adapter = RuntimeBindingHost { js, state: set_state.clone() };
            let receiver = adapter.hook_receiver(handle)?;
            let name = adapter.string(name)?;
            invoke(&mut adapter, Operation::DatasetSet, receiver, &[name, value])?;
            Ok(())
        });
        let delete_state = self.state.clone();
        let named_deleter = Rc::new(move |js: &mut Interpreter, handle: HostHandle, name: &[u16]| {
            let mut adapter = RuntimeBindingHost { js, state: delete_state.clone() };
            let receiver = adapter.hook_receiver(handle)?;
            let name = adapter.string(name)?;
            invoke(&mut adapter, Operation::DatasetDelete, receiver, &[name])?;
            Ok(true)
        });
        self.js.binding_define_legacy_platform_object(InterfaceBrand::DOM_STRING_MAP.0, RuntimeLegacyHooks {
            supported_index_count: Rc::new(|_| Ok(0)), supported_names: Some(names),
            indexed_getter: Rc::new(|_, _, _| Ok(JsValue::Undefined)),
            named_getter: Some(named_getter), indexed_setter: None,
            named_setter: Some(named_setter), named_deleter: Some(named_deleter),
            unenumerable_names: false, override_builtins: true,
        });
        Ok(())
    }


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
    fn document_collections_are_live_alias_correct_and_rooted_in_their_document() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('document collection conformance'); }
            const doc = document.implementation.createHTMLDocument('collections');
            for (const name of ['images', 'embeds', 'plugins', 'links', 'forms', 'scripts', 'anchors', 'applets']) {
                check(doc[name] instanceof HTMLCollection && doc[name] === doc[name]);
                check(doc[name].length === 0);
            }
            check(doc.embeds === doc.plugins);
            const embeds = doc.embeds;
            const anchors = doc.anchors;
            const embed = doc.createElement('embed');
            embed.id = 'plugin';
            doc.body.appendChild(embed);
            check(embeds.length === 1 && embeds[0] === embed && embeds.namedItem('plugin') === embed);
            const foreign = doc.createElementNS('urn:foreign', 'embed');
            doc.body.appendChild(foreign);
            check(embeds.length === 1);
            const anchor = doc.createElement('a');
            doc.body.appendChild(anchor);
            check(anchors.length === 0);
            anchor.setAttribute('name', '');
            check(anchors.length === 1 && anchors[0] === anchor);
            anchor.setAttribute('href', '');
            check(doc.links.length === 1);
            anchor.removeAttribute('name');
            check(anchors.length === 0 && doc.links.length === 1);
            doc.body.appendChild(doc.createElement('applet'));
            doc.body.appendChild(doc.createElement('object'));
            check(doc.applets.length === 0);
            embed.remove();
            check(embeds.length === 0);
            const other = document.implementation.createHTMLDocument('other');
            other.body.appendChild(embed);
            check(embeds.length === 0 && other.embeds.length === 1);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn collection_members_stay_on_prototypes_and_live_keys_track_mutations() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('collection own properties'); }
            const root = document.createElement('div');
            for (const list of [root.children, root.childNodes, root.attributes, root.classList]) {
                check(Object.getOwnPropertyNames(list).length === 0);
                check(list.length === 0 && !list.hasOwnProperty('length'));
            }
            root.setAttribute('data-name', 'one');
            const attrs = root.attributes;
            check(Object.getOwnPropertyNames(attrs).join(',') === '0,data-name');
            check(Object.keys(attrs).join(',') === '0');
            root.removeAttribute('data-name');
            check(attrs.length === 0 && Object.getOwnPropertyNames(attrs).length === 0);
            const children = root.children;
            const nodes = root.childNodes;
            const child = document.createElement('span');
            child.id = 'named';
            root.appendChild(child);
            check(children.length === 1 && nodes.length === 1);
            check(Object.getOwnPropertyNames(children).join(',') === '0,named');
            check(Object.getOwnPropertyNames(nodes).join(',') === '0');
            check(Object.keys(children).join(',') === '0');
            check(children.named === child && children[0] === child);
            child.remove();
            check(children.length === 0 && nodes.length === 0);
            check(Object.getOwnPropertyNames(children).length === 0);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn dataset_is_live_same_object_with_named_property_descriptors() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('dataset conformance'); }
            check(document.createElementNS('urn:random', 'element').dataset === undefined);
            check(document.createElementNS(null, 'element').dataset === undefined);
            for (const element of [document.createElement('div'),
                document.createElementNS('http://www.w3.org/2000/svg', 'g'),
                document.createElementNS('http://www.w3.org/1998/Math/MathML', 'math')]) {
                const map = element.dataset;
                check(map === element.dataset && map instanceof DOMStringMap);
                element.setAttribute('data-first-name', 'Ada');
                check(map.firstName === 'Ada');
                map.firstName = null;
                check(element.getAttribute('data-first-name') === 'null');
                const desc = Object.getOwnPropertyDescriptor(map, 'firstName');
                check(desc.value === 'null' && desc.writable && desc.enumerable && desc.configurable);
                check(Object.keys(map).join(',') === 'firstName');
                element.setAttribute('data-to-string', 'override');
                check(map.toString === 'override');
                check(delete map.firstName && !element.hasAttribute('data-first-name'));
                map[''] = 'empty';
                check(element.getAttribute('data-') === 'empty');
                let rejected = false;
                try { map['bad-name'] = 'x'; }
                catch (error) { rejected = error instanceof DOMException && error.name === 'SyntaxError'; }
                check(rejected);
                Object.defineProperty(map, 'defined', {value: 42, configurable: true});
                check(element.getAttribute('data-defined') === '42');
                element.removeAttribute('data-defined');
                check(map.defined === undefined);
            }
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn dom_exception_uses_inherited_accessors_and_native_error_identity() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('DOMException conformance'); }
            const e = new DOMException(undefined, undefined);
            check(e.name === 'Error' && e.message === '' && e.code === 0);
            check(e instanceof DOMException && e instanceof Error);
            check(Object.getPrototypeOf(DOMException.prototype) === Error.prototype);
            check(Object.getPrototypeOf(DOMException) === Function.prototype);
            check(DOMException.length === 0);
            for (const name of ['name', 'message', 'code']) {
                check(!e.hasOwnProperty(name));
                const desc = Object.getOwnPropertyDescriptor(DOMException.prototype, name);
                check(typeof desc.get === 'function' && desc.set === undefined);
                check(desc.enumerable && desc.configurable);
                let rejected = false;
                try { desc.get.call({}); } catch (error) { rejected = error instanceof TypeError; }
                check(rejected);
            }
            for (const name of Object.keys(DOMException)) {
                if (!name.endsWith('_ERR')) continue;
                for (const object of [DOMException, DOMException.prototype]) {
                    const desc = Object.getOwnPropertyDescriptor(object, name);
                    check(!desc.writable && desc.enumerable && !desc.configurable);
                }
            }
            const named = new DOMException('message', 'NotSupportedError');
            check(named.toString() === 'NotSupportedError: message' && named.code === 9);
            Object.defineProperty(named, 'name', {value: 'other'});
            check(named.code === 9 && named.toString() === 'other: message');
            let caught = false;
            try { document.createElement(''); }
            catch (error) { caught = error instanceof DOMException && error.name === 'InvalidCharacterError' && error.code === 5; }
            check(caught);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

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
