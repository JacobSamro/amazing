//! DOM bindings driver additions owned by the dom-bindings lane.
use super::*;
use amazing_dom::events::{EventClass, EventId};

impl RuntimeBindingHost<'_> {
    /// Register actual allocation context before importing or exposing nodes.
    /// Copy realm handles introduce no additional JS object or Rc roots.
    pub(super) fn dom_register_document_realm(&mut self, document: NodeIdx, realm: RealmHandle)
        -> Result<(), Throw> {
        let (anchor, generation) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let dom = &host.dom;
            let anchor = dom.node_creation_document(document)
                .map_err(|_| Throw::type_error("Expected a live realm Document"))?;
            if !matches!(dom.get(document).kind, NodeKind::Document) {
                return Err(Throw::type_error("Expected a realm Document"));
            }
            let generation = dom.node_generation(document)
                .ok_or_else(|| Throw::type_error("Expected a live realm Document"))?;
            (anchor, generation)
        };
        // A newly created child context may precede the navigation's final
        // active-frame bookkeeping. Read its actual validated creation record,
        // never infer an Origin from its URL or its parent.
        let origin = self.html_document_creation_origin(document, realm);
        if let Some(origin) = origin {
            self.html_set_inert_document_origin(document, origin)
                .map_err(|_| Throw::type_error("Document origin registration failed"))?;
        }
        let mut state = self.state.borrow_mut();
        if state.node_creation_realms.get(&anchor).is_some_and(|existing| *existing != realm) {
            return Err(Throw::type_error("Document allocation realm already registered"));
        }
        state.node_creation_realms.insert(anchor, realm);
        state.realm_documents.entry(realm).or_insert((document, generation));
        Ok(())
    }

    pub(super) fn dom_node_realm(&self, node: NodeIdx) -> Result<RealmHandle, Throw> {
        let state = self.state.borrow();
        let anchor = state.host.borrow().dom.node_creation_document(node)
            .map_err(|_| Throw::type_error("Expected a live node"))?;
        state.node_creation_realms.get(&anchor).copied()
            .ok_or_else(|| Throw::type_error("Node creation realm is not registered"))
    }

    pub(super) fn dom_current_document_for_nodes(&self) -> Result<NodeIdx, Throw> {
        let state = self.state.borrow();
        let (document, generation) = state.realm_documents.get(&self.js.binding_current_realm())
            .copied().ok_or_else(|| Throw::type_error("Realm Document is not registered"))?;
        let host = state.host.borrow();
        if host.dom.node_generation(document) != Some(generation)
            || !matches!(host.dom.get(document).kind, NodeKind::Document) {
            return Err(Throw::type_error("Realm Document is stale"));
        }
        Ok(document)
    }

    pub(super) fn dom_parser_new_in_relevant_realm(&mut self) -> Result<JsValue, Throw> {
        let document = self.dom_current_document_for_nodes()?;
        let generation = self.state.borrow().host.borrow().dom.node_generation(document)
            .ok_or_else(|| Throw::type_error("Realm Document is stale"))?;
        let origin = self.html_document_origin(document)
            .ok_or_else(|| Throw::type_error("DOMParser relevant document origin is unavailable"))?;
        self.web_allocate(WebObject::DomParser {
            realm: self.js.binding_current_realm(), document: (document, generation), origin,
        }, InterfaceBrand::DOM_PARSER)
    }

    pub(super) fn dom_parser_document(&self, parser: &JsValue) -> Result<NodeIdx, Throw> {
        let slot = self.web_slot(parser, InterfaceBrand::DOM_PARSER)?;
        let state = self.state.borrow();
        let WebObject::DomParser { document: (document, generation), .. } = &state.web_objects[slot] else {
            return Err(Throw::type_error("Expected a DOMParser"));
        };
        let host = state.host.borrow();
        if host.dom.node_generation(*document) != Some(*generation)
            || !matches!(host.dom.get(*document).kind, NodeKind::Document) {
            return Err(Throw::type_error("DOMParser relevant Document is stale"));
        }
        Ok(*document)
    }

    pub(super) fn dom_parse_for_parser(&mut self, parser: &JsValue, bytes: &[u8], content_type: &str,
        url: &str, charset: Option<&str>) -> Result<NodeIdx, DomError> {
        self.dom_parser_document(parser).map_err(|_| DomError::TypeError)?;
        let slot = self.web_slot(parser, InterfaceBrand::DOM_PARSER).map_err(|_| DomError::TypeError)?;
        let (realm, origin) = {
            let state = self.state.borrow();
            let WebObject::DomParser { realm, origin, .. } = &state.web_objects[slot] else {
                return Err(DomError::TypeError);
            };
            (*realm, origin.clone())
        };
        // Argument conversion is already complete; only inert native parsing
        // enters the receiver's actual saved realm. No script is executed.
        let state = self.state.clone();
        let document = self.js.binding_with_realm(realm, |js| {
            Ok(RuntimeBindingHost { js, state }.create_inert_document(bytes, content_type, url, charset))
        }).map_err(|_| DomError::TypeError)??;
        self.html_set_inert_document_origin(document, origin)?;
        Ok(document)
    }

    /// Only this explicit DOM creation algorithm has an uninitialized default
    /// opaque origin. Never fill arbitrary missing document origins.
    pub(super) fn dom_initialize_template_document_origin(&self, document: NodeIdx) -> Result<(), Throw> {
        let template_owner={
            let state=self.state.borrow();
            let host=state.host.borrow();
            host.dom.is_template_contents_owner_document(document)
        };
        if template_owner && self.html_document_origin(document).is_none() {
            self.html_set_inert_document_origin(document,amazing_fetch::Origin::opaque())
                .map_err(|_| Throw::type_error("Template document origin initialization failed"))?;
        }
        Ok(())
    }

    pub(super) fn dom_xml_serializer_new(&mut self) -> Result<JsValue, Throw> {
        self.web_allocate(WebObject::XmlSerializer, InterfaceBrand::XML_SERIALIZER)
    }

    pub(super) fn dom_document_compat_mode(&mut self, document: NodeIdx) -> Result<Vec<u16>, Throw> {
        let mode = self.with_dom(|dom| dom.document_quirks_mode(document))?;
        Ok(mode.compat_mode().encode_utf16().collect())
    }

    pub(super) fn dom_resolve_reflected_url(&mut self, input: &[u16]) -> Vec<u16> {
        // URL parsing consumes scalar values even when the content attribute
        // itself is DOMString. Invalid URLs reflect the original string.
        let scalar = String::from_utf16_lossy(input);
        let base = self.state.borrow().document_url.as_deref()
            .and_then(|base| amazing_net::url::ParsedUrl::parse(base).ok());
        let charset = self.state.borrow().document_character_set.clone().unwrap_or_else(|| "UTF-8".into());
        let encoder = |query: &str| super::runtime_host_net::encode_document_query(query, &charset);
        let resolved = match base {
            Some(base) => base.resolve_with_query_encoder(&scalar, &encoder),
            None => amazing_net::url::ParsedUrl::parse_with_query_encoder(&scalar, None, &encoder),
        };
        resolved.map(|url| url.serialize().encode_utf16().collect())
            .unwrap_or_else(|_| input.to_vec())
    }

    fn dom_document_url_context(&self, document: NodeIdx) -> Result<(String, String), DomError> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        let metadata = host.dom.document_metadata(document)?;
        let (url, charset) = if document == NodeIdx(0) {
            (state.document_url.clone().unwrap_or_else(|| "about:blank".into()),
                state.document_character_set.clone().unwrap_or_else(|| "UTF-8".into()))
        } else {
            (metadata.map(|data| data.url.clone()).unwrap_or_else(|| "about:blank".into()),
                metadata.map(|data| data.character_set.clone()).unwrap_or_else(|| "UTF-8".into()))
        };
        drop(host);
        drop(state);
        // The live document context owns its committed URL. Shared driver URL
        // state can temporarily name a child script's execution scope.
        let url = self.html_document_url(document).unwrap_or(url);
        Ok((self.html_document_fallback_base_url(document).unwrap_or(url), charset))
    }

    /// Pure document base lookup, safe during navigation before script callbacks.
    pub(super) fn dom_document_base_url(&self, document: NodeIdx) -> Result<String, DomError> {
        let (fallback, charset) = self.dom_document_url_context(document)?;
        let href = self.state.borrow().host.borrow().dom.document_base_href(document)?;
        let Some(href) = href else { return Ok(fallback); };
        let encoder = |query: &str| super::runtime_host_net::encode_document_query(query, &charset);
        let parsed = amazing_net::ParsedUrl::parse(&fallback).ok();
        let resolved = amazing_net::ParsedUrl::parse_with_query_encoder(
            &String::from_utf16_lossy(&href), parsed.as_ref(), &encoder);
        Ok(match resolved {
            Ok(url) if !matches!(url.scheme(), "data" | "javascript") => url.serialize(),
            _ => fallback,
        })
    }

    pub(super) fn dom_resolve_element_url(&mut self, element: NodeIdx, input: &[u16]) -> Result<Vec<u16>, Throw> {
        let (document, is_base) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            (host.dom.owner_document(element).unwrap_or(NodeIdx(0)),
                host.dom.namespace_uri(element) == Some("http://www.w3.org/1999/xhtml")
                    && host.dom.local_name(element) == Some("base"))
        };
        let (fallback, charset) = self.dom_document_url_context(document)
            .map_err(|error| throw_dom_error(self, error))?;
        // A base element's own href always resolves against fallback, avoiding
        // circular consumption of its own frozen base contribution.
        let base = if is_base { fallback } else {
            self.dom_document_base_url(document).map_err(|error| throw_dom_error(self, error))?
        };
        let base = amazing_net::ParsedUrl::parse(&base).ok();
        let encoder = |query: &str| super::runtime_host_net::encode_document_query(query, &charset);
        let scalar = String::from_utf16_lossy(input);
        let resolved = amazing_net::ParsedUrl::parse_with_query_encoder(&scalar, base.as_ref(), &encoder);
        Ok(resolved.map(|url| url.serialize().encode_utf16().collect())
            .unwrap_or_else(|_| scalar.encode_utf16().collect()))
    }

    pub(super) fn dom_document_default_view(&mut self, document: NodeIdx) -> Result<Option<JsValue>, Throw> {
        // Only an actual live browsing-context association supplies defaultView.
        // A retained creation realm permits allocation without creating a Window.
        Ok(self.iframe_document_window(document))
    }

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
                if property.is_data() { property.writable = false; }
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
        if self.is_nullish(value) || self.same_value(value, &self.js.binding_global_this())
            || self.same_value(value, &self.js.binding_global_object()) {
            Some(self.state.borrow().window_event_target_slot)
        } else { None }
    }

    pub(super) fn dom_window_event_target_value(&self, slot: u64) -> Option<JsValue> {
        (self.state.borrow().window_event_target_slot == slot)
            .then(|| self.js.binding_global_this())
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
            EventClass::HashChange => Interface::HashChangeEvent,
            EventClass::PopState => Interface::PopStateEvent,
            EventClass::PageTransition => Interface::PageTransitionEvent,
            EventClass::BeforeUnload => Interface::BeforeUnloadEvent,
            EventClass::Error => Interface::ErrorEvent,
            EventClass::PromiseRejection => Interface::PromiseRejectionEvent,
            EventClass::DeviceMotion => Interface::DeviceMotionEvent,
            EventClass::DeviceOrientation => Interface::DeviceOrientationEvent,
            EventClass::Drag => Interface::DragEvent,
            EventClass::Storage => Interface::StorageEvent,
            EventClass::Text => Interface::TextEvent,

        })
    }

    pub(super) fn install_selection_global(&mut self) -> Result<(), Throw> {
        let function = self.native("getSelection", 0,
            NativeEntry::Operation(Operation::WindowGetSelection), HostCallPolicy::CallOnly);
        let global = self.js.binding_global_object();
        self.define_data_property(&global, "getSelection", function, true, false, true)?;
        for (name, event) in [("onselectionchange", "selectionchange"), ("onselectstart", "selectstart")] {
            let getter = self.native(name, 0, NativeEntry::Operation(Operation::SelectionEventHandlerGet(event)), HostCallPolicy::CallOnly);
            let setter = self.native(name, 1, NativeEntry::Operation(Operation::SelectionEventHandlerSet(event)), HostCallPolicy::CallOnly);
            self.define_accessor_property(&global, name, Some(getter), Some(setter), true, true)?;
        }
        Ok(())
    }

    pub(super) fn install_window_event_target(&mut self) -> Result<(), Throw> {
        let global = self.js.binding_global_object();
        for name in ["captureEvents", "releaseEvents"] {
            let function = self.native(name, 0, NativeEntry::Operation(Operation::WindowLegacyNoop), HostCallPolicy::CallOnly);
            self.define_data_property(&global, name, function, true, true, true)?;
        }
        let prototype = self.prototype(Interface::EventTarget)
            .ok_or_else(|| Throw::type_error("Missing EventTarget prototype"))?;
        self.js.binding_set_global_prototype(&prototype)
    }

    /// `navigator.clipboard` (DOM-B-REFLECT). `navigator` is an ad-hoc plain
    /// object (not a WebIDL interface installed via `install_interfaces`), so
    /// this mirrors the storage lane's lazy-singleton-getter pattern in
    /// `runtime_host_storage.rs` rather than going through `interfaces.rs`.
    pub(super) fn install_navigator_clipboard(&mut self) -> Result<(), Throw> {
        let global = self.js.binding_global_object();
        let navigator = self.js.binding_get_property(&global, "navigator")?;
        let navigator = if matches!(navigator, JsValue::Object(_)) { navigator } else {
            let value = self.js.binding_create_object(None)?;
            self.define_data_property(&global, "navigator", value.clone(), true, false, true)?;
            value
        };
        let getter = self.native("clipboard", 0,
            NativeEntry::Operation(Operation::NavigatorClipboard), HostCallPolicy::CallOnly);
        self.define_accessor_property(&navigator, "clipboard", Some(getter), None, true, true)
    }

    /// HTML-35: named access on `window` (HTML SS7.2.2.3). Call site is
    /// `with_dom`'s dirty-flag check in `runtime_binding_host.rs`, the one
    /// choke point every DOM-mutating binding call passes through; see
    /// `Dom::window_named_objects`/`mark_named_objects_dirty` for the spec
    /// algorithm and its one deliberate omission (nested browsing contexts
    /// by name -- this engine has no iframe content navigables yet).
    pub(super) fn dom_refresh_window_named_properties(&mut self) -> Result<(), Throw> {
        let host = self.state.borrow().host.clone();
        let entries = host.borrow().dom.window_named_objects(NodeIdx(0));
        let global = self.js.binding_global_object();
        let previous = std::mem::take(&mut self.state.borrow_mut().window_named_property_keys);
        for stale in previous.iter().filter(|name| !entries.iter().any(|(fresh, _)| fresh == *name)) {
            self.js.binding_delete_property(&global, stale)?;
        }
        // Only names we actually define below (not skipped for colliding
        // with a pre-existing property) count as "owned" for next time.
        let mut owned = Vec::with_capacity(entries.len());
        for (name, nodes) in entries {
            // Named access is a [[Get]] fallback in the spec (Window's named
            // properties object is only consulted when ordinary property
            // lookup fails), so it must never shadow a property that is
            // already there for a different reason -- a `var`/function
            // declaration, a built-in like `document`, or a testharness.js
            // global such as `test`/`async_test`. `<div id="test">` is
            // common in WPT fixtures and would otherwise clobber the
            // `test()` function every such page relies on. We can only
            // eagerly materialize properties (no engine-level [[Get]] hook
            // for window -- see the with_dom call site), so approximate the
            // fallback rule by skipping any name we did not already own
            // from a previous refresh when something is already there.
            if !previous.contains(&name) && self.js.binding_get_own_property(&global, &name)?.is_some() {
                continue;
            }
            let value = if let [single] = nodes[..] {
                self.wrap_node(single)?
            } else {
                let slot = host.borrow_mut().dom.create_static_node_list(nodes);
                let mut key = self.node_platform_key(self.current_document());
                key.slot = slot as u64;
                self.wrap_platform_object(key, InterfaceBrand::STATIC_NODE_LIST)?
            };
            self.js.binding_define_data_property(&global, &name, value,
                BindingPropertyFlags { writable: false, enumerable: true, configurable: true })?;
            owned.push(name);
        }
        self.state.borrow_mut().window_named_property_keys = owned;
        Ok(())
    }

    /// WebIDL `sequence<ShadowRoot>` for `Element.getHTML({shadowRoots})`
    /// (DOM-18). Array-like walk mirrors `sequence_values`/
    /// `sequence_dom_strings` in `runtime_binding_host.rs`; each item only
    /// needs to resolve to a Node platform object here -- `markup_bindings
    /// ::invoke`'s `O::ElementGetHTML` arm already re-checks every returned
    /// `NodeIdx` with `Dom::is_shadow_root` and throws `TypeError` for the
    /// whole list if any item isn't actually a shadow root, so this doesn't
    /// duplicate that check per element.
    pub(super) fn dom_sequence_shadow_roots(&mut self, value: &JsValue) -> Result<Vec<NodeIdx>, Throw> {
        self.require_object(value)?;
        let length = self.js.binding_get_property(value, "length")?;
        let length = self.js.binding_to_number(&length)?;
        if !length.is_finite() || length < 0.0 || length > u32::MAX as f64 {
            return Err(Throw::type_error("Invalid sequence length"));
        }
        let mut roots = Vec::new();
        for index in 0..length.trunc() as u32 {
            let item = self.js.binding_get_property(value, &index.to_string())?;
            let key = self.platform_object(&item, InterfaceBrand::NODE)
                .ok_or_else(|| Throw::type_error("Expected a ShadowRoot"))?;
            let node = self.node_from_platform_key(key)
                .ok_or_else(|| Throw::type_error("Expected a ShadowRoot"))?;
            roots.push(node);
        }
        Ok(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_host_keeps_empty_accessors_in_cache_and_freezes_data_arrays() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.create_element(html, "body");
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        let prototype = js.run_script(r#"
            Object.defineProperties({}, {
                empty: {get: undefined, set: undefined, enumerable: true},
                value: {value: 12, enumerable: true},
                hidden: {get: undefined, set: undefined}
            })
        "#).unwrap();
        let JsValue::Object(prototype) = prototype else { panic!("prototype object") };
        let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
        let cached = adapter.prototype_accessors(&prototype);
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].0, PropertyKey::from("empty"));
        assert!(cached[0].1.is_accessor());
        assert!(cached[0].1.getter.is_none() && cached[0].1.setter.is_none());
        assert!(Rc::ptr_eq(&cached, &adapter.prototype_accessors(&prototype)));
        let frozen = adapter.dom_frozen_array(vec![JsValue::Num(3.0)]).unwrap();
        let JsValue::Object(frozen) = frozen else { panic!("frozen array") };
        let frozen = frozen.borrow();
        assert!(!frozen.extensible);
        for key in frozen.own_property_keys() {
            let property = frozen.own_prop(key).unwrap();
            assert!(property.is_data() && !property.writable && !property.configurable);
        }
    }

    #[test]
    fn checkbox_radio_activation_orders_events_rolls_back_and_handles_tree_changes() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.create_element(html, "body");
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'activation check ' + step; }
            function input(type, attached = true) {
                const node = document.createElement('input'); node.type = type;
                if (attached) document.body.append(node); return node;
            }
            const checkbox = input('checkbox'); checkbox.indeterminate = true;
            const log = [];
            checkbox.addEventListener('click', e => {
                log.push('click:' + checkbox.checked + ':' + checkbox.indeterminate + ':' + e.isTrusted);
                checkbox.click();
            });
            checkbox.addEventListener('input', e => log.push('input:' + e.bubbles + ':' + e.composed + ':' + e.isTrusted));
            checkbox.addEventListener('change', e => log.push('change:' + e.bubbles + ':' + e.composed));
            checkbox.click();
            check(log.join(',') === 'click:true:false:false,input:true:true:true,change:true:false');
            const canceled = input('checkbox'); canceled.indeterminate = true; let seen = false, changed = false;
            canceled.addEventListener('click', e => { seen = canceled.checked && !canceled.indeterminate; e.preventDefault(); });
            canceled.addEventListener('input', () => changed = true);
            canceled.click(); check(seen && !changed && !canceled.checked && canceled.indeterminate);
            const copy = canceled.cloneNode(); check(copy.indeterminate);
            const detached = input('checkbox', false); detached.addEventListener('input', () => changed = true);
            detached.click(); check(detached.checked && !changed);
            const disabled = input('checkbox'); disabled.disabled = true; let clicks = 0, inputs = 0;
            disabled.addEventListener('click', () => clicks++); disabled.addEventListener('input', () => inputs++);
            disabled.click(); check(!disabled.checked && clicks === 0);
            disabled.dispatchEvent(new MouseEvent('click')); check(disabled.checked && clicks === 1 && inputs === 1);
            disabled.dispatchEvent(new Event('click')); check(disabled.checked && inputs === 1);
            const a = input('radio'), b = input('radio'); a.checked = true; b.checked = true;
            check(a.checked && b.checked); a.name = 'group'; b.name = 'group'; a.checked = true;
            check(a.checked && !b.checked);
            let rename = false;
            b.addEventListener('click', e => { if (rename) a.name = 'elsewhere'; e.preventDefault(); });
            b.click(); check(a.checked && !b.checked);
            rename = true; b.click(); check(!a.checked && !b.checked);
            const parent = input('checkbox'); const textInput = input('text', false); parent.append(textInput);
            textInput.dispatchEvent(new MouseEvent('click', {bubbles: true})); check(!parent.checked);
            const text = document.createTextNode('child'); parent.append(text);
            text.dispatchEvent(new MouseEvent('click')); check(!parent.checked);
            const click = new MouseEvent('click', {bubbles: true}); let cleaned = false;
            parent.addEventListener('input', () => cleaned = click.eventPhase === 0 && click.currentTarget === null && click.composedPath().length === 0);
            text.dispatchEvent(click); check(parent.checked && cleaned);
            const shell = document.createElement('div'); document.body.append(shell);
            const shadow = shell.attachShadow({mode: 'closed'}); const shadowInput = input('checkbox', false); shadow.append(shadowInput);
            let hostInput = 0, hostChange = 0;
            shell.addEventListener('input', () => hostInput++); shell.addEventListener('change', () => hostChange++);
            shadowInput.click(); check(shadowInput.checked && hostInput === 1 && hostChange === 0);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn onclick_ordering_replacement_removal_and_non_callable_interop_addeventlistener() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        dom.create_element(html, "body");
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'onclick check ' + step; }
            const div = document.createElement('div'); document.body.append(div);

            // onclick registers at the position it is first assigned, ordered
            // among addEventListener registrations; getter reads it back.
            const log = [];
            div.addEventListener('click', () => log.push('before'));
            div.onclick = () => log.push('onclick-1');
            check(div.onclick && log.length === 0);
            div.addEventListener('click', () => log.push('after'));
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,onclick-1,after');

            // Reassigning onclick replaces in place (position preserved).
            log.length = 0;
            div.onclick = () => log.push('onclick-2');
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,onclick-2,after');

            // Clearing then re-adding moves the registration to the end.
            log.length = 0;
            div.onclick = null;
            check(div.onclick === null);
            div.onclick = () => log.push('onclick-3');
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,after,onclick-3');

            // The same function used as both onclick and via addEventListener
            // is two independent registrations; removeEventListener never
            // touches the onclick slot.
            log.length = 0;
            function shared() { log.push('shared'); }
            div.onclick = shared;
            div.addEventListener('click', shared);
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,after,shared,shared');
            log.length = 0;
            div.removeEventListener('click', shared);
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,after,shared' && div.onclick === shared);

            // Returning exactly `false` cancels the event; any other return
            // value (including a falsy non-boolean) does not.
            log.length = 0;
            div.onclick = () => false;
            const cancel = new MouseEvent('click', {cancelable: true});
            div.dispatchEvent(cancel);
            check(cancel.defaultPrevented === true);
            div.onclick = () => 0;
            const noCancel = new MouseEvent('click', {cancelable: true});
            div.dispatchEvent(noCancel);
            check(noCancel.defaultPrevented === false);

            // A thrown handler reports and continues; onclick keeps running
            // on later dispatches and does not disturb sibling listeners.
            log.length = 0;
            div.onclick = () => { throw 'boom'; };
            div.dispatchEvent(new MouseEvent('click'));
            check(log.join(',') === 'before,after');

            // Assigning a non-callable object never throws and is silently
            // ignored: its `handleEvent` (unlike a real EventListener) must
            // never be read.
            log.length = 0;
            let handleEventRead = false;
            const notCallable = { get handleEvent() { handleEventRead = true; return () => log.push('handleEvent'); } };
            div.onclick = notCallable;
            check(div.onclick === notCallable);
            div.dispatchEvent(new MouseEvent('click'));
            check(!handleEventRead && log.join(',') === 'before,after');

            // A non-object primitive assigned to onclick clears the handler
            // (EventHandlerNonNull's [LegacyTreatNonObjectAsNull]).
            div.onclick = 5;
            check(div.onclick === null);
            div.onclick = undefined;
            check(div.onclick === null);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn error_event_payloads_preserve_identity_defaults_and_conversion_order() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'error event check ' + step; }
            check(ErrorEvent.length === 1 && PromiseRejectionEvent.length === 2);
            const empty = new ErrorEvent('error');
            check(empty instanceof Event && empty instanceof ErrorEvent);
            check(empty.message === '' && empty.filename === '' && empty.lineno === 0 && empty.colno === 0);
            check(empty.error === undefined && !empty.bubbles && !empty.cancelable && !empty.isTrusted);
            const payload = {};
            const event = new ErrorEvent('\uD800', {message: '\uD800', filename: '\uD800',
                error: payload, lineno: -1, colno: 4294967297, cancelable: true});
            check(event.type === '\uD800' && event.message === '\uD800' && event.filename === '\uFFFD');
            check(event.error === payload && event.lineno === 4294967295 && event.colno === 1);
            check(new ErrorEvent('x', {error: null}).error === null);
            check(new ErrorEvent('x', {error: false}).error === false);
            check(Object.is(new ErrorEvent('x', {colno: -0.5}).colno, 0));
            const order = [];
            const dict = {};
            for (const name of ['bubbles', 'cancelable', 'composed', 'colno', 'error', 'filename', 'lineno', 'message']) {
                Object.defineProperty(dict, name, {get() { order.push(name); return undefined; }});
            }
            new ErrorEvent('x', dict);
            check(order.join(',') === 'bubbles,cancelable,composed,colno,error,filename,lineno,message');
            const promise = {}; // The IDL requires an object, not a Promise brand.
            const rejection = new PromiseRejectionEvent('unhandledrejection', {promise, reason: payload});
            check(rejection instanceof Event && rejection.promise === promise && rejection.reason === payload);
            check(new PromiseRejectionEvent('x', {promise}).reason === undefined);
            check(new PromiseRejectionEvent('x', {promise, reason: null}).reason === null);
            for (const dict of [undefined, null, {}, {promise: null}, {promise: 1}]) {
                let rejected = false;
                try { new PromiseRejectionEvent('x', dict); } catch (e) { rejected = e instanceof TypeError; }
                check(rejected);
            }
            let brand = false;
            try { Object.getOwnPropertyDescriptor(ErrorEvent.prototype, 'error').get.call(rejection); }
            catch (e) { brand = e instanceof TypeError; }
            check(brand);
            const target = new EventTarget(); let seen;
            target.addEventListener('\uD800', e => { seen = e; e.preventDefault(); });
            check(target.dispatchEvent(event) === false && seen === event && seen.error === payload);
            event.initEvent('again', false, false); check(event.error === payload && event.message === '\uD800');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }


    #[test]
    fn obsolete_reflection_converts_types_preserves_aliases_and_checks_receiver_interfaces() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/base/");
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'obsolete reflection check ' + step; }
            const script = document.createElement('script');
            script.htmlFor = 'window'; script.event = 'onload'; script.charset = 'utf-8';
            check(script.getAttribute('for') === 'window' && script.getAttribute('event') === 'onload');
            script.setAttribute('for', 'target'); check(script.htmlFor === 'target');
            const col = document.createElement('col'); col.ch = '.'; col.chOff = '12';
            check(col.getAttribute('char') === '.' && col.getAttribute('charoff') === '12');
            const table = document.createElement('table');
            table.bgColor = null; table.cellPadding = null; table.cellSpacing = undefined;
            check(table.getAttribute('bgcolor') === '' && table.cellPadding === '' && table.cellSpacing === 'undefined');
            const hr = document.createElement('hr'); hr.color = null;
            check(hr.color === 'null'); hr.noShade = 'false'; check(hr.noShade && hr.getAttribute('noshade') === '');
            hr.noShade = 0; check(!hr.hasAttribute('noshade'));
            const td = document.createElement('td'); td.noWrap = true; check(td.hasAttribute('nowrap'));
            td.noWrap = false; check(!td.hasAttribute('nowrap'));
            const image = document.createElement('img'); image.hspace = -1; image.vspace = 4294967297;
            check(image.getAttribute('hspace') === '0' && image.vspace === 1);
            image.setAttribute('hspace', '  +12px'); check(image.hspace === 12);
            image.setAttribute('hspace', '2147483648'); check(image.hspace === 0);
            image.hspace = 2147483647; check(image.hspace === 2147483647);
            const pre = document.createElement('pre'); pre.width = 4294967295; check(pre.width === -1);
            pre.setAttribute('width', ' -7px'); check(pre.width === -7);
            const iframe = document.createElement('iframe'); iframe.longDesc = 'help';
            check(iframe.getAttribute('longdesc') === 'help' && iframe.longDesc === 'https://example.test/base/help');
            iframe.marginWidth = null; check(iframe.marginWidth === '');
            image.lowsrc = '\uD800'; check(image.getAttribute('lowsrc') === '\uFFFD');
            const object = document.createElement('object'); object.codeBase = 'code/';
            check(object.codeBase === 'https://example.test/base/code/');
            object.codeBase = 'http://['; check(object.codeBase === 'http://[');
            object.codeBase = '\uD800'; check(object.getAttribute('codebase') === '\uD800');
            check(object.codeBase.indexOf('%EF%BF%BD') !== -1);
            object.useMap = '#map'; check(object.useMap === '#map');
            const body = document.createElement('body'); body.background = 'relative'; check(body.background === 'relative');
            body.background = '\uD800'; check(body.background === '\uD800');
            const param = document.createElement('param'); param.name = 'key'; param.value = 'value'; param.valueType = 'ref';
            check(param.getAttribute('name') === 'key' && param.getAttribute('value') === 'value' && param.getAttribute('valuetype') === 'ref');
            const area = document.createElement('area'); area.noHref = true; check(area.noHref);
            const anchor = document.createElement('a'); anchor.name = 'named'; check(anchor.getAttribute('name') === 'named');
            check(Object.getOwnPropertyDescriptor(HTMLAreaElement.prototype, 'name') === undefined);
            let converted = false, rejected = false;
            try { Object.getOwnPropertyDescriptor(HTMLAnchorElement.prototype, 'name').set.call(area,
                {toString() { converted = true; return 'wrong'; }}); }
            catch (e) { rejected = e instanceof TypeError; }
            check(rejected && !converted);
            const link = document.createElement('link'); link.target = '_blank'; check(link.getAttribute('target') === '_blank');
            const clone = script.cloneNode(); check(clone.htmlFor === 'target' && clone.event === 'onload');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }


    #[test]
    fn event_names_preserve_utf16_through_construction_init_and_listener_lifetime() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'event UTF16 check ' + step; }
            const names = ['\uD800', '\uDC00', '\uD801', '\uFFFD', '\uD800x\uDC00', '😀'];
            for (const Constructor of [Event, CustomEvent, UIEvent, FocusEvent, MouseEvent, WheelEvent,
                KeyboardEvent, CompositionEvent, HashChangeEvent, PopStateEvent, PageTransitionEvent]) {
                for (const name of names) check(new Constructor(name).type === name);
            }
            const target = new EventTarget(), seen = [];
            function listener(e) { seen.push(e.type); }
            for (const name of names) target.addEventListener(name, listener);
            for (const name of names) target.dispatchEvent(new Event(name));
            check(seen.join('|') === names.join('|'));
            target.removeEventListener('\uFFFD', listener);
            seen.length = 0; target.dispatchEvent(new Event('\uD800'));
            check(seen.length === 1 && seen[0] === '\uD800');
            target.removeEventListener('\uD800', listener);
            seen.length = 0; target.dispatchEvent(new Event('\uD800')); check(seen.length === 0);
            const controller = new AbortController();
            target.addEventListener('\uD800', listener, {signal: controller.signal});
            controller.abort(); target.dispatchEvent(new Event('\uD800')); check(seen.length === 0);
            target.addEventListener('\uD800', listener, {once: true});
            target.dispatchEvent(new Event('\uD800')); target.dispatchEvent(new Event('\uD800')); check(seen.length === 1);
            const legacy = document.createEvent('Event');
            legacy.initEvent('\uD800'); check(legacy.type === '\uD800');
            legacy.initEvent('\uFFFD'); check(legacy.type === '\uFFFD');
            const custom = document.createEvent('CustomEvent');
            custom.initCustomEvent('\uDC00', false, false, {}); check(custom.type === '\uDC00');
            const message = document.createEvent('MessageEvent');
            message.initMessageEvent('\uD800'); check(message.type === '\uD800');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn navigation_events_keep_native_payload_identity_and_legacy_return_value() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(v) { step++; if (!v) throw 'navigation event check ' + step; }
            const payload = {}, order = [];
            const pop = new PopStateEvent('popstate', {
                get bubbles() { order.push('bubbles'); return true; },
                get cancelable() { order.push('cancelable'); return true; },
                get composed() { order.push('composed'); return true; },
                get hasUAVisualTransition() { order.push('transition'); return true; },
                get state() { order.push('state'); return payload; }
            });
            check(order.join(',') === 'bubbles,cancelable,composed,transition,state');
            check(pop instanceof Event && pop instanceof PopStateEvent && !(pop instanceof UIEvent));
            check(pop.state === payload && pop.hasUAVisualTransition && pop.bubbles && pop.composed);
            const target = new EventTarget(); let seen = 0;
            target.addEventListener('popstate', function(e) { check(e === pop && e.state === payload); seen++; });
            target.dispatchEvent(pop); target.dispatchEvent(pop); check(seen === 2);
            check(new PopStateEvent('x', {state: undefined}).state === null);
            check(new PopStateEvent('x', {state: 7}).state === 7);
            const hash = new HashChangeEvent('hashchange', {oldURL: '\uD800', newURL: '#relative'});
            check(hash.oldURL === '\uFFFD' && hash.newURL === '#relative');
            const legacyHash = document.createEvent('hAsHcHaNgEeVeNt');
            check(legacyHash instanceof HashChangeEvent && legacyHash.oldURL === '' && legacyHash.newURL === '');
            const page = new PageTransitionEvent('pageshow', {persisted: true});
            check(page instanceof Event && page.persisted && !page.bubbles && !page.cancelable);
            check(!new PageTransitionEvent('pageshow').persisted);
            const before = document.createEvent('BeforeUnloadEvent');
            check(before instanceof BeforeUnloadEvent && before.returnValue === '');
            before.initEvent('beforeunload', false, true); before.returnValue = false;
            check(before.returnValue === 'false' && !before.defaultPrevented);
            before.preventDefault(); check(before.returnValue === 'false' && before.defaultPrevented);
            before.returnValue = '\uD800'; check(before.returnValue === '\uD800');
            let rejected = false;
            try { new BeforeUnloadEvent(); } catch (e) { rejected = e instanceof TypeError; }
            check(rejected);
            for (const constructor of [HashChangeEvent, PopStateEvent, PageTransitionEvent]) check(constructor.length === 1);
            for (const pair of [[HashChangeEvent, 'oldURL'], [PopStateEvent, 'state'],
                [PageTransitionEvent, 'persisted'], [BeforeUnloadEvent, 'returnValue']]) {
                rejected = false;
                try { Object.getOwnPropertyDescriptor(pair[0].prototype, pair[1]).get.call(new Event('x')); }
                catch (e) { rejected = e instanceof TypeError; }
                check(rejected);
            }
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn processing_instruction_constructor_and_attributes_share_native_character_data() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0;
            function check(value) { step++; if (!value) throw 'PI check ' + step; }
            const pi = new ProcessingInstruction('xml', 'a="&amp;" b="two"');
            check(ProcessingInstruction.length === 1 && pi instanceof CharacterData);
            check(pi.ownerDocument === document && pi.target === 'xml');
            check(pi.getAttribute('a') === '&' && pi.hasAttributes());
            check(pi.getAttributeNames().join(',') === 'a,b');
            const observer = new MutationObserver(function() {});
            observer.observe(pi, {characterData: true, characterDataOldValue: true, attributes: true});
            pi.setAttribute('a', '<"&>');
            check(pi.data === 'a="&lt;&quot;&amp;&gt;" b="two"');
            let records = observer.takeRecords();
            check(records.length === 1 && records[0].type === 'characterData' && records[0].target === pi);
            check(records[0].oldValue === 'a="&amp;" b="two"');
            check(pi.toggleAttribute('a', true) && observer.takeRecords().length === 0);
            check(!pi.toggleAttribute('a', undefined));
            check(pi.toggleAttribute('a') && pi.getAttributeNames().join(',') === 'b,a');
            const names = pi.getAttributeNames(); names.push('extra');
            check(pi.getAttributeNames().length === 2);
            pi.data = 'raw="new"';
            check(pi.getAttribute('b') === null && pi.getAttribute('raw') === 'new');
            pi.appendData(' broken'); check(!pi.hasAttributes());
            pi.setAttribute('$', '\uD800');
            check(pi.getAttribute('$') === '\uD800' && pi.cloneNode().getAttribute('$') === '\uD800');
            let calls = '', rejected = false;
            try { pi.setAttribute({toString() {calls += 'n'; return 'bad name';}},
                {toString() {calls += 'v'; return 'x';}}); }
            catch (e) { rejected = e instanceof DOMException && e.name === 'InvalidCharacterError'; }
            check(rejected && calls === 'nv');
            calls = ''; rejected = false;
            try { ProcessingInstruction.prototype.setAttribute.call(document.createTextNode(''),
                {toString() { calls += 'x'; return 'a';}}, 'b'); }
            catch (e) { rejected = e instanceof TypeError; }
            check(rejected && calls === '');
            rejected = false;
            try { pi.setAttribute({toString() { calls += 'x'; return 'a';}}); }
            catch (e) { rejected = e instanceof TypeError; }
            check(rejected && calls === '');
            for (const args of [[], ['0'], ['x', '?>']]) {
                rejected = false;
                try { Reflect.construct(ProcessingInstruction, args); }
                catch (e) { rejected = args.length === 0 ? e instanceof TypeError : e instanceof DOMException; }
                check(rejected);
            }
            check(new ProcessingInstruction('target', undefined).data === '');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn legacy_elements_reflect_attributes_and_keep_interface_identity_when_cloned() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/base/");
        let result = js.run_script(r#"
            let step = 0;
            function check(value) { step++; if (!value) throw 'legacy element reflection check ' + step; }
            const groups = [
                ['dir', HTMLDirectoryElement, ['compact']],
                ['font', HTMLFontElement, ['color', 'face', 'size']],
                ['frameset', HTMLFrameSetElement, ['cols', 'rows']],
                ['frame', HTMLFrameElement, ['name', 'scrolling', 'src', 'frameBorder',
                    'longDesc', 'noResize', 'marginHeight', 'marginWidth']]
            ];
            for (const group of groups) {
                const tag = group[0], ctor = group[1], element = document.createElement(tag);
                check(element instanceof ctor && element instanceof HTMLElement);
                check(!(element instanceof HTMLUnknownElement));
                for (const prop of group[2]) {
                    const attr = prop.toLowerCase();
                    const descriptor = Object.getOwnPropertyDescriptor(ctor.prototype, prop);
                    check(descriptor.enumerable && descriptor.configurable);
                    const boolean = prop === 'compact' || prop === 'noResize';
                    check(element[prop] === (boolean ? false : ''));
                    element[prop] = boolean ? 'false' : 'https://example.test/path';
                    check(element.getAttribute(attr) === (boolean ? '' : 'https://example.test/path'));
                    check(element[prop] === (boolean ? true : 'https://example.test/path'));
                    element.setAttribute(attr, boolean ? 'false' : 'https://example.test/next');
                    check(element[prop] === (boolean ? true : 'https://example.test/next'));
                    const clone = element.cloneNode(true);
                    check(clone instanceof ctor && clone[prop] === element[prop]);
                    element.removeAttribute(attr);
                    check(element[prop] === (boolean ? false : ''));
                    let converted = false, rejected = false;
                    try { descriptor.set.call(document.createElement('div'), {toString() { converted = true; return 'x'; }}); }
                    catch (e) { rejected = e instanceof TypeError; }
                    check(rejected && !converted);
                    rejected = false;
                    try { descriptor.get.call(document.createElementNS('urn:test', tag)); }
                    catch (e) { rejected = e instanceof TypeError; }
                    check(rejected);
                    if (boolean) {
                        element[prop] = 1; element[prop] = 0;
                        check(!element.hasAttribute(attr));
                    } else {
                        element[prop] = null;
                        const legacy = prop === 'color' || prop === 'marginHeight' || prop === 'marginWidth';
                        check(element.getAttribute(attr) === (legacy ? '' : 'null'));
                        element[prop] = undefined;
                        check(element.getAttribute(attr) === 'undefined');
                        element[prop] = '\uD800';
                        check(element.getAttribute(attr) === (prop === 'src' || prop === 'longDesc' ? '\uFFFD' : '\uD800'));
                    }
                }
            }
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn installed_modifier_queries_and_numeric_dictionaries_follow_webidl() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('event dictionary types'); }
            const mouse = new MouseEvent('x', {clientX: 1.5, clientY: -0,
                button: 65535, buttons: -1, modifierAltGraph: true, modifierCapsLock: true});
            check(mouse.clientX === 1.5 && Object.is(mouse.clientY, -0));
            check(mouse.button === -1 && mouse.buttons === 65535);
            check(mouse.getModifierState('AltGraph') && mouse.getModifierState('CapsLock'));
            check(!mouse.getModifierState('Alt') && !mouse.getModifierState('altgraph'));
            const key = new KeyboardEvent('x', {location: -1, keyCode: 4294967297,
                charCode: NaN, which: 4294967298, modifierFn: true});
            check(key.location === 4294967295 && key.keyCode === 1 && key.charCode === 0);
            check(key.which === 2 && key.getModifierState('Fn'));
            const wheel = new WheelEvent('x', {deltaMode: -1, deltaX: 0.25, momentum: true, ctrlKey: true});
            check(wheel.deltaMode === 4294967295 && wheel.deltaX === 0.25 && wheel.momentum);
            check(wheel.getModifierState('Control'));
            let rejected = 0;
            try { new MouseEvent('x', {clientX: Infinity}); } catch (e) { if (e instanceof TypeError) rejected++; }
            try { new WheelEvent('x', {deltaX: NaN}); } catch (e) { if (e instanceof TypeError) rejected++; }
            try { MouseEvent.prototype.getModifierState.call(key, 'Control'); } catch (e) { if (e instanceof TypeError) rejected++; }
            try { mouse.getModifierState(); } catch (e) { if (e instanceof TypeError) rejected++; }
            check(rejected === 4);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn focus_and_mouse_related_targets_follow_native_shadow_dispatch_state() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('shadow event target'); }
            const host = document.createElement('div');
            const root = host.attachShadow({mode: 'closed'});
            const inner = root.appendChild(document.createElement('span'));
            const external = new EventTarget();
            for (const Constructor of [FocusEvent, MouseEvent]) {
                const event = new Constructor('probe', {relatedTarget: inner});
                check(event.relatedTarget === inner);
                let observed = false;
                external.addEventListener('probe', value => {
                    observed = value.relatedTarget === host && value.target === external;
                }, {once: true});
                external.dispatchEvent(event);
                check(observed && event.relatedTarget === host && event.target === external);
                root.dispatchEvent(event);
                check(event.relatedTarget === null && event.target === null);
                check(event.currentTarget === null && event.composedPath().length === 0);
            }
            const event = new FocusEvent('move', {relatedTarget: external});
            inner.addEventListener('move', () => host.appendChild(inner), {once: true});
            inner.dispatchEvent(event);
            check(inner.parentNode === host && event.target === null && event.relatedTarget === null);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn hyperlink_query_encoding_uses_owner_document_and_live_base() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<head><base href='https://cdn.test/dir/'></head><body></body>"),
            native_forwarded: false,
        }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/page");
        bindings.set_document_metadata("windows-1252", "text/html", "");
        assert_eq!(js.run_script(r#"
            function check(value) { if (!value) throw new Error('hyperlink encoding'); }
            for (const tag of ['a', 'area']) {
                const a = document.createElement(tag);
                a.href = '€/next?€#€';
                check(a.href === 'https://cdn.test/dir/%E2%82%AC/next?%80#%E2%82%AC');
                check(a.search === '?%80' && a.pathname === '/dir/%E2%82%AC/next');
                check(a.hash === '#%E2%82%AC' && a.origin === 'https://cdn.test');
                a.search = '?€'; // Component setters use the URL parser's UTF-8 default.
                check(a.search === '?%E2%82%AC');
                a.href = {toString() {
                    document.querySelector('base').setAttribute('href', 'https://changed.test/');
                    return 'next?€';
                }};
                check(a.href === 'https://changed.test/next?%80');
                document.querySelector('base').setAttribute('href', 'https://cdn.test/dir/');
            }
            const child = document.implementation.createHTMLDocument('');
            const childLink = child.createElement('a');
            childLink.href = 'https://child.test/?あ';
            check(childLink.search === '?%E3%81%82');
            const opaque = document.createElement('a');
            opaque.href = 'MAILTO:user@example.test';
            opaque.hostname = 'ignored.test';
            opaque.username = 'ignored';
            opaque.pathname = 'ignored';
            check(opaque.getAttribute('href') === 'MAILTO:user@example.test');
            true;
        "#).unwrap(), JsValue::Bool(true));
        let document = host.borrow_mut().dom.create_document();
        host.borrow_mut().dom.set_document_metadata(document, amazing_dom::document_surface::DocumentMetadata {
            url: "https://child.test/base/".into(), content_type: "text/html".into(),
            character_set: "Shift_JIS".into(),
        }).unwrap();
        let link = host.borrow_mut().dom.create_element(document, "a");
        let resolved = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() }
            .dom_resolve_element_url(link, &"next?あ".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(String::from_utf16(&resolved).unwrap(), "https://child.test/base/next?%82%A0");
    }

    #[test]
    fn iframe_document_default_view_preserves_child_proxy_and_inert_nulls() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe srcdoc='child'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host,
            crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(r#"
            const frame = document.querySelector('iframe');
            const child = frame.contentDocument;
            child.defaultView === frame.contentWindow &&
            child.defaultView.document === child &&
            child.defaultView !== window &&
            child.implementation.createHTMLDocument('').defaultView === null &&
            child.cloneNode(false).defaultView === null;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn document_default_view_preserves_window_identity_and_inert_document_nulls() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('document defaultView'); }
            check(document.defaultView === window);
            check(document.defaultView.document === document);
            check(document.defaultView.DOMException === DOMException);
            check(new Document().defaultView === null);
            check(document.implementation.createHTMLDocument('').defaultView === null);
            check(document.implementation.createDocument(null, 'root').defaultView === null);
            check(document.cloneNode(false).defaultView === null);
            const getter = Object.getOwnPropertyDescriptor(Document.prototype, 'defaultView').get;
            let rejected = false;
            try { getter.call({}); } catch (error) { rejected = error instanceof TypeError; }
            check(rejected);
            for (const [uri, name, expected, code] of [
                [null, '1foo', 'InvalidCharacterError', 5],
                [null, 'p:foo', 'NamespaceError', 14],
                ['urn:test', 'xml:foo', 'NamespaceError', 14]]) {
                let caught;
                try { document.createElementNS(uri, name); } catch (error) { caught = error; }
                check(caught.name === expected && caught.code === code);
                check(caught.constructor === document.defaultView.DOMException);
            }
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn token_list_reads_live_state_after_argument_conversion() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('token list conversion'); }
            const element = document.createElement('div');
            const list = element.classList;
            element.className = 'old';
            list.add({toString() { element.className = 'fresh'; return 'added'; }});
            check(list.value === 'fresh added');
            check(list.item({valueOf() { element.className = 'changed'; return 0; }}) === 'changed');
            let marker = {};
            let caught;
            try { list.add('', {toString() { throw marker; }}); } catch (error) { caught = error; }
            check(caught === marker && list.value === 'changed');
            element.removeAttribute('class');
            list.remove('missing');
            check(!element.hasAttribute('class'));
            check(list.contains('') === false && list.contains('a b') === false);
            element.className = 'c b a';
            check(list.replace('c', 'a') && list.value === 'a b');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn abortable_listeners_handle_recursive_dispatch_dependents_and_conversion_order() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('listener registration'); }
            const target = new EventTarget();
            const controller = new AbortController();
            const signal = AbortSignal.any([controller.signal]);
            let calls = 0;
            target.addEventListener('x', () => {
                calls++;
                if (calls === 6) controller.abort();
                target.dispatchEvent(new Event('x'));
            }, {signal});
            target.dispatchEvent(new Event('x'));
            check(calls === 6);
            let reads = '';
            const options = {
                get capture() { reads += 'c'; return false; },
                get once() { reads += 'o'; return false; },
                get passive() { reads += 'p'; return false; },
                get signal() { reads += 's'; return signal; }
            };
            target.addEventListener('x', null, options);
            check(reads === 'cops');
            let rejected = false;
            try { target.addEventListener('x', null, {signal: null}); }
            catch (error) { rejected = error instanceof TypeError; }
            check(rejected);
            const later = new AbortController();
            let once = 0;
            target.addEventListener('once', () => once++, {once: true, signal: later.signal});
            target.dispatchEvent(new Event('once'));
            later.abort();
            target.dispatchEvent(new Event('once'));
            check(once === 1);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn legacy_document_and_body_colors_share_attributes_and_noops_ignore_arguments() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            function check(value) { if (!value) throw new Error('legacy document reflection'); }
            const doc = document.implementation.createHTMLDocument('legacy');
            const body = doc.body;
            for (const [documentName, bodyName, attribute] of [
                ['fgColor', 'text', 'text'], ['bgColor', 'bgColor', 'bgcolor'],
                ['linkColor', 'link', 'link'], ['vlinkColor', 'vLink', 'vlink'],
                ['alinkColor', 'aLink', 'alink']]) {
                doc[documentName] = 'red';
                check(body[bodyName] === 'red' && body.getAttribute(attribute) === 'red');
                body[bodyName] = null;
                check(doc[documentName] === '' && body.getAttribute(attribute) === '');
                body[bodyName] = undefined;
                check(doc[documentName] === 'undefined');
            }
            body.remove();
            let conversions = 0;
            doc.fgColor = {toString() { conversions++; return 'ignored'; }};
            check(conversions === 1 && doc.fgColor === '');
            doc.body = body;
            check(doc.fgColor === 'undefined');
            const poison = {toString() { throw new Error('must not coerce'); }};
            for (const name of ['clear', 'captureEvents', 'releaseEvents']) {
                check(doc[name](poison) === undefined && doc[name].length === 0);
            }
            check(window.captureEvents(poison) === undefined);
            check(window.releaseEvents(poison) === undefined);
            let rejected = false;
            try { doc.clear.call({}); } catch (error) { rejected = error instanceof TypeError; }
            check(rejected);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

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
    #[test]
    fn document_compat_mode_reads_each_document_mode() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        host.borrow_mut().dom.set_document_quirks_mode(NodeIdx(0), amazing_dom::QuirksMode::Quirks).unwrap();
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host.clone()).unwrap();
        assert!(matches!(js.run_script("document.compatMode === 'BackCompat'").unwrap(), JsValue::Bool(true)));
        host.borrow_mut().dom.set_document_quirks_mode(NodeIdx(0), amazing_dom::QuirksMode::LimitedQuirks).unwrap();
        assert!(matches!(js.run_script("document.compatMode === 'CSS1Compat' && document.implementation.createHTMLDocument('').compatMode === 'CSS1Compat'").unwrap(), JsValue::Bool(true)));
    }

    #[test]
    fn installed_table_collections_parts_conversion_and_live_indices() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host).unwrap();
        let result = js.run_script(r#"
            let step = 0; function check(v) { step++; if (!v) throw 'table check ' + step; }
            const table = document.createElement('table');
            const rows = table.rows, bodies = table.tBodies;
            check(rows instanceof HTMLCollection && rows === table.rows && rows.length === 0);
            table.deleteRow(-1);
            const first = table.insertRow(undefined);
            check(bodies === table.tBodies && bodies.length === 1 && rows.length === 1);
            const cell = first.insertCell(), cells = first.cells;
            const th = document.createElement('th'); first.appendChild(th);
            check(cells === first.cells && cells.length === 2 && th.cellIndex === 1);
            const head = table.createTHead(), hr = head.insertRow();
            const foot = table.createTFoot(), fr = foot.insertRow();
            check(rows[0] === hr && rows[1] === first && rows[2] === fr);
            check(first.rowIndex === 1 && first.sectionRowIndex === 0);
            check(table.insertRow(-1).parentNode === foot);
            check(head.rows === head.rows && head.rows[0] === hr);
            const cap = table.createCaption(); check(cap === table.caption && table.firstChild === cap);
            let rejected = false; try { table.tHead = bodies[0]; } catch(e) { rejected = e.name === 'HierarchyRequestError'; }
            check(rejected && table.tHead === head);
            rejected = false; try { table.caption = head; } catch(e) { rejected = e instanceof TypeError; } check(rejected);
            first.deleteCell(0); check(cell.cellIndex === -1 && th.cellIndex === 0 && cells.length === 1);
            const secondBody = table.createTBody(); check(bodies.length === 2 && secondBody.previousSibling === bodies[0]);
            let converted = 0; const at = { valueOf() { converted++; return 0; } };
            const newRow = table.insertRow(at); check(converted === 1 && newRow.parentNode === head);
            table.deleteTHead(); check(table.tHead === null && hr.rowIndex === -1 && hr.sectionRowIndex === 1);
            check(rows.length === 3); table.caption = null; check(cap.parentNode === null);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
    }

    #[test]
    fn serialization_context_distinguishes_active_and_inert_document_parsing() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/serialization");
        let result = js.run_script(r#"
            let step=0; function check(v){step++;if(!v)throw 'serialization context '+step;} try {
            const active=document.createElement('div'); active.innerHTML='<noscript><&></noscript>';
            check(active.innerHTML === '<noscript><&></noscript>');
            const inert=document.implementation.createHTMLDocument('');
            inert.body.innerHTML='<noscript>&lt;&amp;&gt;</noscript>';
            check(inert.body.firstChild.textContent === '<&>');
            check(inert.body.innerHTML === '<noscript>&lt;&amp;&gt;</noscript>');
            const parsed=new DOMParser().parseFromString('<body><noscript>&lt;&amp;&gt;</noscript></body>','text/html');
            check(parsed.body.firstChild.textContent === '<&>');
            check(parsed.body.innerHTML === '<noscript>&lt;&amp;&gt;</noscript>');
            const clone=document.cloneNode(false), div=clone.createElement('div');
            div.innerHTML='<noscript>&lt;&amp;&gt;</noscript>';
            check(div.firstChild.textContent === '<&>');
            true; } catch(e) { String(e)+" step="+step; }
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
    }

    #[test]
    fn template_markup_uses_contents_document_and_preserves_fragment_context() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/active");
        let result = js.run_script(r#"
            let step=0; function check(v){step++;if(!v)throw 'template context '+step;}
            const template=document.createElement('template');
            const contents=template.content, owner=contents.ownerDocument;
            check(owner !== document && owner.URL === 'about:blank' && owner.characterSet === 'UTF-8');
            template.innerHTML='<noscript>&lt;&amp;&gt;</noscript><template><b>nested</b></template>';
            check(template.content === contents && template.childNodes.length === 0);
            check(contents.firstChild.textContent === '<&>');
            check(contents.lastChild.content.ownerDocument === owner);
            check(template.innerHTML === '<noscript>&lt;&amp;&gt;</noscript><template><b>nested</b></template>');
            const copy=template.cloneNode(true);check(copy.content !== contents && copy.content.ownerDocument === owner);
            contents.firstChild.outerHTML='<noscript>&lt;&amp;&gt;</noscript>';
            check(contents.firstChild.textContent === '<&>');
            contents.firstChild.insertAdjacentHTML('afterend','<noscript>&lt;&amp;&gt;</noscript>');
            check(contents.childNodes[1].textContent === '<&>');
            template.insertAdjacentHTML('beforeend','<noscript>&lt;&amp;&gt;</noscript>');
            check(template.firstChild.ownerDocument === document && template.firstChild.textContent === '&lt;&amp;&gt;');
            const inert=document.implementation.createHTMLDocument('');inert.adoptNode(template);
            check(template.content === contents && contents.ownerDocument !== owner && contents.ownerDocument !== inert);
            check(contents.lastChild.content.ownerDocument === contents.ownerDocument);
            const fragment=document.createDocumentFragment(), span=document.createElement('span');fragment.append(span);
            span.outerHTML='<p>replacement</p>';check(fragment.firstChild.localName === 'p');
            fragment.firstChild.insertAdjacentHTML('beforebegin','<b>before</b>');check(fragment.firstChild.localName === 'b');
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
    }

    #[test]
    fn template_fragment_parser_reads_explicit_document_scripting_state() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let template = dom.create_element(html, "template");
        let contents = dom.ensure_template_contents(template).unwrap();
        let owner = dom.owner_document(contents).unwrap();
        dom.set_document_scripting_enabled(owner, true).unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let _bindings = super::super::install(&mut js, host.clone()).unwrap();
        let result = js.run_script(r#"
            const t=document.documentElement.firstChild;
            t.innerHTML='<noscript>&lt;&amp;&gt;</noscript>';
            t.content.firstChild.textContent === '&lt;&amp;&gt;';
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
        host.borrow_mut().dom.set_document_scripting_enabled(owner, false).unwrap();
        let result = js.run_script(r#"
            t.innerHTML='<noscript>&lt;&amp;&gt;</noscript>';
            t.content.firstChild.textContent === '<&>';
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
    }

    #[test]
    fn contextual_range_fragments_use_live_owner_and_conversion_context() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html"); dom.create_element(html, "body");
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/range");
        let result = js.run_script(r#"
            let step=0;function check(v){step++;if(!v)throw 'range context '+step;}
            const r=document.createRange();r.selectNodeContents(document.body);
            const before=r.startContainer, offset=r.startOffset;
            const a=r.createContextualFragment('<p>one<p>two');
            const b=r.createContextualFragment('<p>one<p>two');
            check(a !== b && a.ownerDocument === document && a.firstChild.ownerDocument === document);
            check(a.childNodes.length === 2 && r.startContainer === before && r.startOffset === offset);
            r.selectNodeContents(document.documentElement);
            check(r.createContextualFragment('<span>x</span>').firstChild.localName === 'span');
            r.setStart(document.createTextNode('detached'),0);
            check(r.createContextualFragment('<body><b>x</b>').firstChild.localName === 'b');
            const table=document.createElement('table'), body=document.createElement('tbody');table.append(body);
            r.selectNodeContents(document.body);
            const changing={toString(){r.selectNodeContents(body);return '<tr><td>cell</td></tr>';}};
            check(r.createContextualFragment(changing).firstChild.firstChild.localName === 'td');
            const t=document.createElement('template');r.selectNodeContents(t.content);
            const inert=r.createContextualFragment('<noscript>&amp;</noscript>');
            check(inert.ownerDocument === t.content.ownerDocument && inert.firstChild.textContent === '&');
            r.selectNodeContents(t);
            check(r.createContextualFragment('<noscript>&amp;</noscript>').firstChild.textContent === '&amp;');
            const form=document.createElement('form'), context=document.createElement('div');form.append(context);
            r.selectNodeContents(context);
            check(r.createContextualFragment('<form><input></form>').firstChild.localName === 'input');
            const math=document.createElementNS('http://www.w3.org/1998/Math/MathML','annotation-xml');
            math.setAttribute('encoding','text/html');r.selectNodeContents(math);
            check(r.createContextualFragment('<foo></foo>').firstChild.namespaceURI === 'http://www.w3.org/1999/xhtml');
            math.setAttribute('encoding','application/xml');
            check(r.createContextualFragment('<foo></foo>').firstChild.namespaceURI === math.namespaceURI);
            const uppercase=document.createElementNS('http://www.w3.org/1999/xhtml','TITLE');
            r.selectNodeContents(uppercase);
            check(r.createContextualFragment('<b>x</b>').firstChild.localName === 'b');
            const svgContext=document.createElementNS('http://www.w3.org/2000/svg','foreignObject');
            r.selectNodeContents(svgContext);
            check(r.createContextualFragment('<b>x</b>').firstChild.namespaceURI === 'http://www.w3.org/1999/xhtml');
            const comment=document.createComment('context');body.append(comment);r.selectNodeContents(comment);
            check(r.createContextualFragment('<tr><td>x</td></tr>').firstChild.localName === 'tr');
            const foreign=document.createElementNS('urn:actual-context','MixedCase');r.selectNodeContents(foreign);
            check(r.createContextualFragment('<foo></foo>').firstChild.namespaceURI === 'urn:actual-context');
            check(r.createContextualFragment('<body><p>x').firstChild.namespaceURI === 'http://www.w3.org/1999/xhtml');
            r.selectNodeContents(document.body);globalThis.contextualRuns=0;
            const script=r.createContextualFragment('<script>contextualRuns++</script>');check(contextualRuns === 0);
            document.body.append(script);check(contextualRuns === 1);
            let missing=false;try{r.createContextualFragment();}catch(e){missing=e instanceof TypeError;}check(missing);
            let branded=false;try{Range.prototype.createContextualFragment.call({},'');}catch(e){branded=e instanceof TypeError;}check(branded);
            check(r.createContextualFragment(null).firstChild.data === 'null');
            r.detach();check(r.createContextualFragment('').childNodes.length === 0);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
    }

    #[test]
    fn contextual_range_xml_fragments_keep_namespace_scope_and_fatal_errors() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/range-xml");
        let result = js.run_script(r#"
            (function(){let step=0;try{function check(v){step++;if(!v)throw 'range XML context '+step;}
            const doc=new DOMParser().parseFromString('<p:root xmlns:p="urn:parent" xmlns="urn:default" xmlns:x="urn:outer"><c:child xmlns:c="urn:child" xmlns:x="urn:local"/></p:root>','application/xml');
            const child=doc.documentElement.firstChild,r=doc.createRange();r.selectNodeContents(child);
            const f=r.createContextualFragment('<x:item x:value="ok"/><plain/><![CDATA[cdata]]>tail');
            check(f.ownerDocument === doc && f.childNodes.length === 4);
            check(f.firstChild.namespaceURI === 'urn:local' && f.firstChild.getAttributeNS('urn:local','value') === 'ok');
            check(f.childNodes[1].namespaceURI === 'urn:default' && f.childNodes[2].nodeType === 4 && f.lastChild.data === 'tail');
            child.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns','');
            check(r.createContextualFragment('<plain/>').firstChild.namespaceURI === null);
            const bom=r.createContextualFragment('\uFEFF<plain/>');check(bom.firstChild.data === '\uFEFF');
            check(r.createContextualFragment('').childNodes.length === 0);
            for(const source of ['<unclosed>','<unbound:item/>','&#0;','<?xml version="1.0"?>','<!DOCTYPE a><a/>','<a xmlns:x="urn:dup" xmlns:y="urn:dup" x:k="1" y:k="2"/>',String.fromCharCode(0xd800)]){
                let invalid=false;try{r.createContextualFragment(source);}catch(e){invalid=e.name === 'SyntaxError';}check(invalid);
            }
            check(child.parentNode === doc.documentElement && r.startContainer === child);
            return true;}catch(e){return String(e)+' at XML context step '+step;}})();
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "{result:?}");
    }

    #[test]
    fn contextual_xml_fragment_scripts_remain_started_after_html_adoption() {
        let host=Rc::new(RefCell::new(BrowserDomHost {
            dom:amazing_html::parse("<body></body>"),native_forwarded:false,
        }));
        let mut js=Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings=super::super::install(&mut js,host).unwrap();
        bindings.set_document_url(&mut js,"https://example.test/range-script-policy");
        let result=js.run_script(r#"
            const xml=new DOMParser().parseFromString('<root xmlns="http://www.w3.org/1999/xhtml"/>','application/xml');
            const range=xml.createRange();range.selectNodeContents(xml.documentElement);
            globalThis.xmlContextualRuns=0;
            const fragment=range.createContextualFragment('<script>xmlContextualRuns++</script><template><script>xmlContextualRuns++</script></template>');
            const template=fragment.lastChild;
            document.body.append(fragment);
            document.body.append(template.content);
            if(xmlContextualRuns!==0)throw 'XML fragment script executed after adoption';
            range.selectNodeContents(document.body);
            const html=range.createContextualFragment('<script>xmlContextualRuns++</script>');
            document.body.append(html);
            xmlContextualRuns===1;
        "#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn contextual_range_fragment_inherits_current_document_quirks_mode() {
        use amazing_dom::document_surface::QuirksMode;
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");dom.create_element(html,"body");
        dom.set_document_quirks_mode(NodeIdx(0),QuirksMode::Quirks).unwrap();
        let host=Rc::new(RefCell::new(BrowserDomHost {dom,native_forwarded:false}));
        let mut js=Interpreter::new_with_external_scheduler(32).unwrap();js.set_dom_host(host.clone());
        let _bindings=super::super::install(&mut js,host.clone()).unwrap();
        let result=js.run_script(r#"
            const r=document.createRange();r.selectNodeContents(document.body);
            const quirky=r.createContextualFragment('<p>before<table><tr><td>cell</td></tr></table>after');
            quirky.firstChild.firstElementChild.localName === 'table';
        "#).unwrap();assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
        host.borrow_mut().dom.set_document_quirks_mode(NodeIdx(0),QuirksMode::NoQuirks).unwrap();
        let result=js.run_script(r#"
            const standard=r.createContextualFragment('<p>before<table><tr><td>cell</td></tr></table>after');
            standard.firstChild.nextSibling.localName === 'table' && standard.firstChild.firstElementChild === null;
        "#).unwrap();assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn base_reflection_uses_fallback_url_and_preserves_webidl_strings() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/dir/page?old#fragment");
        let result = js.run_script(r#"
            let step=0;function check(v){step++;if(!v)throw 'base reflection '+step;}
            const root=document.createElement('html');document.appendChild(root);
            const first=document.createElement('base'),second=document.createElement('base');
            check(first.href==='https://example.test/dir/page?old');check(first.target==='');
            first.href='nested/';root.appendChild(first);second.href='next';root.appendChild(second);
            check(first.href==='https://example.test/dir/nested/');
            check(second.href==='https://example.test/dir/next');
            const a=document.createElement('a');a.href='next';check(a.href==='https://example.test/dir/nested/next');
            first.href='http://[';check(first.href==='http://[');check(a.href==='https://example.test/dir/next');
            first.href='data:text/plain,hello';check(a.href==='https://example.test/dir/next');
            first.href='javascript:alert(1)';check(a.href==='https://example.test/dir/next');
            second.href='\uD800';check(second.getAttribute('href')==='\uFFFD');
            second.target='\uD800';check(second.target==='\uD800');
            let converted=0;second.target={toString(){converted++;return '_parent';}};check(converted===1&&second.target==='_parent');
            let rejected=false;try{Object.getOwnPropertyDescriptor(HTMLBaseElement.prototype,'href').get.call(a);}catch(e){rejected=e instanceof TypeError;}check(rejected);
            true;
        "#).unwrap();
        assert!(matches!(result, JsValue::Bool(true)));
        let doc = host.borrow_mut().dom.create_document();
        host.borrow_mut().dom.set_document_metadata(doc, amazing_dom::document_surface::DocumentMetadata {
            url:"https://child.test/base/page".into(),content_type:"text/html".into(),character_set:"Shift_JIS".into(),
        }).unwrap();
        let base=host.borrow_mut().dom.create_element_ns_in(doc,Some("http://www.w3.org/1999/xhtml"),"base").unwrap();
        let other=host.borrow_mut().dom.create_element_ns_in(doc,Some("http://www.w3.org/1999/xhtml"),"base").unwrap();
        host.borrow_mut().dom.set_attribute(other,"href","https://ignored.test/").unwrap();
        host.borrow_mut().dom.pre_insert(other,doc,None).unwrap();
        let resolved=RuntimeBindingHost { js:&mut js,state:bindings.state.clone() }
            .dom_resolve_element_url(base,&"next?あ".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(String::from_utf16(&resolved).unwrap(),"https://child.test/base/next?%82%A0");
    }

    #[test]
    fn document_base_lookup_uses_actual_document_metadata_and_first_href() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://root.test/dir/page?x#fragment");
        let document = {
            let mut host = host.borrow_mut();
            let doc = host.dom.create_document();
            host.dom.set_document_metadata(doc, amazing_dom::document_surface::DocumentMetadata {
                url: "https://child.test/dir/page?x#child".into(),
                content_type: "text/html".into(), character_set: "Shift_JIS".into(),
            }).unwrap();
            doc
        };
        let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
        assert_eq!(adapter.dom_document_base_url(NodeIdx(0)).unwrap(), "https://root.test/dir/page?x#fragment");
        assert_eq!(adapter.dom_document_base_url(document).unwrap(), "https://child.test/dir/page?x#child");
        let first = {
            let mut host = host.borrow_mut();
            let first = host.dom.create_element_ns_in(document, Some("http://www.w3.org/1999/xhtml"), "base").unwrap();
            host.dom.set_attribute(first, "href", "next?あ#retained").unwrap();
            host.dom.pre_insert(first, document, None).unwrap();
            first
        };
        assert_eq!(adapter.dom_document_base_url(document).unwrap(), "https://child.test/dir/next?%82%A0#retained");
        let units = DocumentHost::document_base_url(&mut adapter, document).unwrap();
        assert_eq!(String::from_utf16(&units).unwrap(), "https://child.test/dir/next?%82%A0#retained");
        for invalid in ["http://[", "data:text/plain,ignored", "javascript:ignored"] {
            host.borrow_mut().dom.set_attribute(first, "href", invalid).unwrap();
            assert_eq!(adapter.dom_document_base_url(document).unwrap(), "https://child.test/dir/page?x#child");
        }
    }

    #[test]
    fn xml_serializer_installed_brand_nodes_utf16_and_namespace_scope() {
        let host=Rc::new(RefCell::new(BrowserDomHost { dom:Dom::new(),native_forwarded:false }));
        let mut js=Interpreter::new_with_external_scheduler(32).unwrap();
        js.set_dom_host(host.clone());let bindings=super::super::install(&mut js,host).unwrap();
        bindings.set_document_url(&mut js,"https://example.test/xml-serializer");
        let result=js.run_script(r#"
            (function(){let step=0;try{function check(v){step++;if(!v)throw 'XML serialization '+step;}
            const serializer=new XMLSerializer();check(serializer instanceof XMLSerializer);
            check(XMLSerializer.length === 0 && XMLSerializer.prototype.serializeToString.length === 1);
            const root=document.createElementNS('urn:root','p:root');
            root.setAttributeNS('http://www.w3.org/2000/xmlns/','xmlns:p','urn:other');
            root.setAttributeNS('urn:other','p:value','a\t\n\r<&>"');
            check(serializer.serializeToString(root) === '<ns1:root xmlns:ns1="urn:root" xmlns:p="urn:other" p:value="a&#9;&#xA;&#xD;&lt;&amp;&gt;&quot;"/>');
            const foreign=new DOMParser().parseFromString('<r xmlns="urn:r"><child/></r>','application/xml');
            check(serializer.serializeToString(foreign) === '<r xmlns="urn:r"><child/></r>');
            check(serializer.serializeToString(root.getAttributeNodeNS('urn:other','value')) === '');
            check(serializer.serializeToString(document.createTextNode('\uD800<&>')) === '\uD800&lt;&amp;&gt;');
            check(serializer.serializeToString(document.createComment('a--b-')) === '<!--a--b--->');
            let rejected=0;for(const value of [undefined,null,{},'node',1]){
                try{serializer.serializeToString(value);}catch(e){if(e instanceof TypeError)rejected++;}
            }check(rejected===5);
            let missing=false;try{serializer.serializeToString();}catch(e){missing=e instanceof TypeError;}check(missing);
            let wrong=false;try{XMLSerializer.prototype.serializeToString.call({},root);}catch(e){wrong=e instanceof TypeError;}check(wrong);
            let construct=false;try{XMLSerializer();}catch(e){construct=e instanceof TypeError;}check(construct);
            check(new XMLSerializer() !== serializer);
            const custom={};function Target(){}Target.prototype=custom;
            const reflected=Reflect.construct(XMLSerializer,[],Target);
            check(Object.getPrototypeOf(reflected) === custom);
            check(XMLSerializer.prototype.serializeToString.call(reflected,root).startsWith('<ns1:root'));
            class Derived extends XMLSerializer {}const derived=new Derived();
            check(derived instanceof Derived && derived instanceof XMLSerializer);
            return true;}catch(e){return String(e)+" at step "+step;}})();
        "#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn xml_serializer_cross_realm_nodes_receivers_and_prototypes() {
        let host=Rc::new(RefCell::new(BrowserDomHost {
            dom:amazing_html::parse("<body><iframe></iframe></body>"),native_forwarded:false }));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/xml-serializer");
        scheduler.run_script("globalThis.frame=document.querySelector('iframe');void frame.contentWindow").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let result=scheduler.run_script(r#"
            let step=0;function check(v){step++;if(!v)throw 'XML realm '+step;}
            const child=frame.contentWindow;
            const serializer=new child.XMLSerializer();
            check(child.XMLSerializer !== XMLSerializer);
            check(Object.getPrototypeOf(serializer) === child.XMLSerializer.prototype);
            check(Object.getPrototypeOf(serializer) !== XMLSerializer.prototype);
            const local=document.createElementNS('urn:local','root');
            const foreign=frame.contentDocument.createElementNS('urn:foreign','child');
            check(serializer.serializeToString(local) === '<root xmlns="urn:local"/>');
            check(XMLSerializer.prototype.serializeToString.call(serializer,foreign) === '<child xmlns="urn:foreign"/>');
            check(child.XMLSerializer.prototype.serializeToString.call(new XMLSerializer(),foreign) === '<child xmlns="urn:foreign"/>');
            const explicitPrototype={};let successfulReads=0;
            const successful=new Proxy(child.Function('return 1;'),{get(t,k){
                if(k==='prototype'){successfulReads++;return explicitPrototype;}
                return Reflect.get(t,k);
            }});
            const successfulResult=Reflect.construct(XMLSerializer,[],successful);
            check(successfulReads===1 && Object.getPrototypeOf(successfulResult)===explicitPrototype);
            const target=child.Function('return 1;');target.prototype=7;
            for(const constructor of [target,target.bind(null),new Proxy(target,{})]){
                const reflected=Reflect.construct(XMLSerializer,[],constructor);
                check(Object.getPrototypeOf(reflected) === child.XMLSerializer.prototype);
                check(reflected.serializeToString(foreign) === '<child xmlns="urn:foreign"/>');
            }
            const marker={};let reads=0;
            const abrupt=new Proxy(target,{get(t,k){if(k==='prototype'){reads++;throw marker;}return Reflect.get(t,k);}});
            let caught=false;try{Reflect.construct(XMLSerializer,[],abrupt);}catch(e){caught=e===marker;}
            check(caught && reads===1);
            const revoked=Proxy.revocable(target,{});revoked.revoke();
            let invalid=false;try{Reflect.construct(XMLSerializer,[],revoked.proxy);}catch(e){invalid=e instanceof TypeError;}
            check(invalid);
            true;
        "#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn node_factories_use_document_realm_and_child_constructors_use_child_document() {
        let host=Rc::new(RefCell::new(BrowserDomHost { dom:amazing_html::parse("<body><div id='namedBeforeSetup'></div><iframe></iframe></body>"),native_forwarded:false }));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/node-realms");
        scheduler.run_script("globalThis.frame=document.querySelector('iframe');void frame.contentWindow").unwrap();
        let result=scheduler.run_script(r#"(function(){let step=0;try{
            function check(v){step++;if(!v)throw 'factory '+step;}
            check(window.namedBeforeSetup===document.getElementById('namedBeforeSetup'));
            const inner=frame.contentWindow;
            const borrowedImplementation=Object.getOwnPropertyDescriptor(inner.Document.prototype,'implementation').get.call(document);
            check(borrowedImplementation===document.implementation && Object.getPrototypeOf(borrowedImplementation)===DOMImplementation.prototype);
            for(const [method,iface,args] of [['createTextNode','Text',['x']],['createComment','Comment',['x']],
                ['createProcessingInstruction','ProcessingInstruction',['t','d']],['createDocumentFragment','DocumentFragment',[]],
                ['createAttribute','Attr',['a']]]){
                const node=inner.Document.prototype[method].apply(document,args);
                check(Object.getPrototypeOf(node)===window[iface].prototype);
                check(node.ownerDocument===document);
            }
            const xml=document.implementation.createDocument(null,'r',null);
            check(Object.getPrototypeOf(xml)===XMLDocument.prototype);
            check(inner.Document.prototype.createCDATASection.call(xml,'x') instanceof CDATASection);
            const doctype=inner.DOMImplementation.prototype.createDocumentType.call(document.implementation,'html','','');
            check(Object.getPrototypeOf(doctype)===DocumentType.prototype);
            const made=inner.DOMImplementation.prototype.createHTMLDocument.call(document.implementation,'title');
            check(Object.getPrototypeOf(made)===Document.prototype);
            check(made.documentElement instanceof HTMLHtmlElement && made.querySelector('title').firstChild instanceof Text);
            const childValues=inner.Function('return [new Text("x"),new Comment("x"),new DocumentFragment(),new Document(),new Option("x","v",true,true),new ProcessingInstruction("t","d")]')();
            for(let n=0;n<3;n++)check(childValues[n].ownerDocument===inner.document);
            check(Object.getPrototypeOf(childValues[0])===inner.Text.prototype);
            check(Object.getPrototypeOf(childValues[1])===inner.Comment.prototype);
            check(Object.getPrototypeOf(childValues[2])===inner.DocumentFragment.prototype);
            check(Object.getPrototypeOf(childValues[3])===inner.Document.prototype);
            check(Object.getPrototypeOf(childValues[3].cloneNode())===inner.Document.prototype);
            check(childValues[4].ownerDocument===inner.document && Object.getPrototypeOf(childValues[4])===inner.HTMLOptionElement.prototype);
            check(Object.getPrototypeOf(childValues[4].firstChild)===inner.Text.prototype);
            check(childValues[5].ownerDocument===inner.document && Object.getPrototypeOf(childValues[5])===inner.ProcessingInstruction.prototype);
            return true;}catch(e){return String(e)+' at step '+step;}})()"#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn adoption_keeps_creation_realm_while_clone_import_and_new_markup_use_current_document() {
        let host=Rc::new(RefCell::new(BrowserDomHost { dom:amazing_html::parse("<body><iframe></iframe></body>"),native_forwarded:false }));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/adoption-realms");
        scheduler.run_script("globalThis.frame=document.querySelector('iframe');void frame.contentWindow").unwrap();
        let result=scheduler.run_script(r#"(function(){let step=0;try{
            function check(v){step++;if(!v)throw 'adopt '+step;}
            const inner=frame.contentWindow;
            const foreign=inner.document.createElement('div');foreign.innerHTML='text<!--comment--><span a="b"></span>';
            const original=Object.getPrototypeOf(foreign);
            check(document.body.appendChild(foreign)===foreign);
            check(foreign.ownerDocument===document && Object.getPrototypeOf(foreign)===original);
            check(Object.getPrototypeOf(foreign.firstChild)===inner.Text.prototype);
            check(Object.getPrototypeOf(foreign.childNodes[1])===inner.Comment.prototype);
            check(foreign.firstChild===foreign.childNodes[0]);
            const originalAttribute=foreign.lastChild.getAttributeNode('a');
            check(originalAttribute.ownerDocument===document && Object.getPrototypeOf(originalAttribute)===inner.Attr.prototype);
            foreign.setAttribute('later','root');
            inner.document.adoptNode(foreign);
            check(Object.getPrototypeOf(foreign.getAttributeNode('later'))===Attr.prototype);
            document.adoptNode(foreign);
            const attached=inner.document.createAttribute('attached');
            foreign.setAttributeNode(attached);
            check(attached.ownerDocument===document && Object.getPrototypeOf(attached)===inner.Attr.prototype);
            inner.document.adoptNode(attached);
            check(attached.ownerElement===null && !foreign.hasAttribute('attached'));
            const clone=inner.Node.prototype.cloneNode.call(foreign,true);
            check(Object.getPrototypeOf(clone)===HTMLDivElement.prototype);
            check(Object.getPrototypeOf(clone.firstChild)===Text.prototype);
            check(Object.getPrototypeOf(clone.lastChild.getAttributeNode('a'))===Attr.prototype);
            const imported=inner.document.importNode(clone,true);
            check(Object.getPrototypeOf(imported)===inner.HTMLDivElement.prototype);
            check(Object.getPrototypeOf(imported.firstChild)===inner.Text.prototype);
            const template=inner.document.createElement('template');template.innerHTML='<span>old</span>';
            const content=template.content;const old=content.firstChild;
            check(Object.getPrototypeOf(content.ownerDocument)===inner.Document.prototype);
            document.body.appendChild(template);check(template.content===content);
            check(Object.getPrototypeOf(old)===inner.HTMLSpanElement.prototype);
            template.innerHTML='<span>new</span>';check(Object.getPrototypeOf(template.content.firstChild)===HTMLSpanElement.prototype);
            foreign.innerHTML='new<!--new-->';check(Object.getPrototypeOf(foreign.firstChild)===Text.prototype);
            const range=document.createRange();range.selectNodeContents(foreign);
            check(Object.getPrototypeOf(inner.Range.prototype.cloneContents.call(range))===DocumentFragment.prototype);
            const retained=foreign.firstChild;frame.remove();
            check(foreign.firstChild===retained && Object.getPrototypeOf(imported)===inner.HTMLDivElement.prototype);
            check(imported.ownerDocument===inner.document);
            return true;}catch(e){return String(e)+' at step '+step;}})()"#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
    }

    #[test]
    fn domparser_receiver_realm_survives_borrowed_method_conversion_and_removed_frame() {
        let host=Rc::new(RefCell::new(BrowserDomHost { dom:amazing_html::parse("<body><iframe srcdoc=''></iframe></body>"),native_forwarded:false }));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/parser-realms");
        scheduler.run_script("globalThis.frame=document.querySelector('iframe');void frame.contentWindow").unwrap();
        let original=scheduler.run_script("frame.contentDocument").unwrap();
        let state=scheduler.test_bindings().state.clone();
        let original_origin={
            let adapter=RuntimeBindingHost {js:&mut scheduler.js,state:state.clone()};
            let document=adapter.node_brand(&original).unwrap();
            adapter.html_document_origin(document).unwrap()
        };
        let result=scheduler.run_script(r#"(function(){let step=0;try{
            function check(v){step++;if(!v)throw 'parser '+step;}
            check(document.defaultView===window);
            const inner=frame.contentWindow;const parser=new inner.DOMParser();
            const retainedDocument=inner.document;
            check(retainedDocument.defaultView===inner);
            let converted=0;const source={toString(){converted++;frame.remove();
                check(retainedDocument.defaultView===null);
                check(inner.Function('return document.defaultView')()===null);
                const text=inner.Function('return new Text("retained")')();
                check(text.ownerDocument===inner.document && Object.getPrototypeOf(text)===inner.Text.prototype);
                return '<r><child/></r>';}};
            const parsed=DOMParser.prototype.parseFromString.call(parser,source,'application/xml');
            check(converted===1 && Object.getPrototypeOf(parsed)===inner.XMLDocument.prototype);
            check(parsed.URL==='about:srcdoc' && parsed.baseURI==='about:srcdoc');
            check(Object.getPrototypeOf(parsed.documentElement)===inner.Element.prototype);
            check(parsed.defaultView===null);
            const clone=Node.prototype.cloneNode.call(parsed,true);
            check(Object.getPrototypeOf(clone)===inner.XMLDocument.prototype);
            const error=DOMParser.prototype.parseFromString.call(parser,'<broken>','application/xml');
            check(Object.getPrototypeOf(error)===inner.XMLDocument.prototype);
            check(error.documentElement.localName==='parsererror');
            let wrong=false;try{DOMParser.prototype.parseFromString.call({},source,'text/html');}catch(e){wrong=e instanceof TypeError;}
            check(wrong && converted===1);
            const retainedParser=new inner.DOMParser();
            check(Object.getPrototypeOf(retainedParser.parseFromString('<retained/>','application/xml'))===inner.XMLDocument.prototype);
            globalThis.parsedDoc=parsed;
            return true;}catch(e){return String(e)+' at step '+step;}})()"#).unwrap();
        assert!(matches!(result,JsValue::Bool(true)),"{result:?}");
        let parsed=scheduler.run_script("parsedDoc").unwrap();
        let adapter=RuntimeBindingHost {js:&mut scheduler.js,state};
        let document=adapter.node_brand(&parsed).unwrap();
        assert_eq!(adapter.html_document_origin(document),Some(original_origin));

    }

    #[test]
    fn missing_stale_realm_document_and_stale_node_identity_fail_before_dom_mutation() {
        let host=Rc::new(RefCell::new(BrowserDomHost { dom:Dom::new(),native_forwarded:false }));
        let mut js=Interpreter::new_with_external_scheduler(32).unwrap();js.set_dom_host(host.clone());
        let bindings=super::super::install(&mut js,host.clone()).unwrap();
        bindings.set_document_url(&mut js,"https://example.test/stale-realms");
        let realm=js.binding_current_realm();let actual=bindings.state.borrow_mut().realm_documents.remove(&realm).unwrap();
        let count=host.borrow().dom.node_count();
        let result=js.run_script("(function(){let n=0;for(const make of [()=>new Text('x'),()=>new Comment('x'),()=>new DocumentFragment(),()=>new Document(),()=>new DOMParser(),()=>new Option('x'),()=>new ProcessingInstruction('t','d')]){try{make()}catch(e){if(e instanceof TypeError)n++;}}return n;})()").unwrap();
        assert_eq!(result,JsValue::Num(7.0));assert_eq!(host.borrow().dom.node_count(),count);
        bindings.state.borrow_mut().realm_documents.insert(realm,(actual.0,actual.1+1));
        assert_eq!(js.run_script("(function(){try{document.createElement('div')}catch(e){return e instanceof TypeError;}return false;})()").unwrap(),JsValue::Bool(true));
        assert_eq!(host.borrow().dom.node_count(),count);bindings.state.borrow_mut().realm_documents.insert(realm,actual);
        let mut adapter=RuntimeBindingHost {js:&mut js,state:bindings.state.clone()};
        let mut key=adapter.node_object_key(NodeIdx(0));key.generation+=1;
        assert!(adapter.node_from_object_key(key).is_none());
        let prototype=adapter.prototype(Interface::Document).unwrap();
        let stale=adapter.js.binding_wrap_host(HostHandle { arena:key.arena,slot:key.slot,generation:key.generation,brand:InterfaceBrand::NODE.0 },&prototype).unwrap();
        assert!(adapter.node_brand(&stale).is_none());
        let implementation_prototype=adapter.prototype(Interface::DOMImplementation).unwrap();
        let stale_implementation=adapter.js.binding_wrap_host(HostHandle {
            arena:key.arena,slot:key.slot,generation:key.generation,brand:InterfaceBrand::DOM_IMPLEMENTATION.0 },&implementation_prototype).unwrap();
        assert!(adapter.platform_object(&stale_implementation,InterfaceBrand::DOM_IMPLEMENTATION).is_none());
        adapter.js.global_obj.borrow_mut().set_own("staleImplementation",stale_implementation);
        assert_eq!(adapter.js.run_script("(function(){try{DOMImplementation.prototype.createDocument.call(staleImplementation,null,'r',null)}catch(e){return e instanceof TypeError;}return false;})()").unwrap(),JsValue::Bool(true));
        adapter.js.global_obj.borrow_mut().set_own("staleNode",stale);
        assert_eq!(adapter.js.run_script("(function(){try{Node.prototype.appendChild.call(staleNode,document.createComment('x'));}catch(e){return e instanceof TypeError;}return false;})()").unwrap(),JsValue::Bool(true));
    }

    #[test]
    fn document_factory_and_clone_origins_use_actual_constructor_and_receiver_context() {
        let host=Rc::new(RefCell::new(BrowserDomHost {
            dom:amazing_html::parse("<body><iframe sandbox='allow-scripts' srcdoc=''></iframe></body>"),native_forwarded:false }));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/opaque-document-factories");
        let root_constructor=scheduler.run_script("Document").unwrap();
        let root_factory=scheduler.run_script("DOMImplementation.prototype.createDocument").unwrap();
        let root_html_factory=scheduler.run_script("DOMImplementation.prototype.createHTMLDocument").unwrap();
        scheduler.run_script("void document.querySelector('iframe').contentWindow").unwrap();
        let state=scheduler.test_bindings().state.clone();
        let root=scheduler.js.binding_current_realm();
        let (child,document)={
            let bindings=state.borrow();
            bindings.realm_documents.iter().find(|(realm,_)| **realm!=root)
                .map(|(realm,(document,_))| (*realm,*document)).unwrap()
        };
        let (root_origin,child_origin)={
            let adapter=RuntimeBindingHost {js:&mut scheduler.js,state:state.clone()};
            (adapter.html_document_origin(NodeIdx(0)).unwrap(),adapter.html_document_origin(document).unwrap())
        };
        assert_ne!(root_origin,child_origin);
        let docs=scheduler.js.binding_with_realm(child,|js| {
            js.global_obj.borrow_mut().set_own("foreignDocument",root_constructor);
            js.global_obj.borrow_mut().set_own("foreignCreateDocument",root_factory);
            js.global_obj.borrow_mut().set_own("foreignCreateHTMLDocument",root_html_factory);
            js.run_script(r#"(function(){
                const own=new Document();
                const xml=foreignCreateDocument.call(document.implementation,null,'r',null);
                const html=foreignCreateHTMLDocument.call(document.implementation,{toString(){new Document();return 'title';}});
                const parsed=new DOMParser().parseFromString('<r/>','application/xml');
                if(!(own instanceof Document) || !(xml instanceof XMLDocument) || !(html instanceof Document))throw 'prototype';
                return [own,xml,html,parsed,own.cloneNode(true),xml.cloneNode(true),html.cloneNode(true),parsed.cloneNode(true),new foreignDocument()];
            })()"#)
        }).unwrap();
        let mut adapter=RuntimeBindingHost {js:&mut scheduler.js,state};
        for index in 0..9 {
            let value=adapter.get_property(&docs,&index.to_string()).unwrap();
            let document=adapter.node_brand(&value).unwrap();
            assert_eq!(adapter.html_document_origin(document),Some(if index==8 {root_origin.clone()} else {child_origin.clone()}));
        }
    }

    #[test]
    fn template_owner_documents_have_distinct_stable_default_opaque_origins_and_document_class() {
        let host=Rc::new(RefCell::new(BrowserDomHost {dom:amazing_html::parse("<body></body>"),native_forwarded:false}));
        let mut scheduler=crate::js_scheduler::DriverScheduler::new(host,32).unwrap();
        scheduler.set_location_url("https://example.test/template-origin");
        let docs=scheduler.run_script(r#"(function(){
            const template=document.createElement('template');
            const owner=template.content.ownerDocument;
            const nested=owner.createElement('template');
            if(nested.content.ownerDocument!==owner)throw 'owner reuse';
            const other=document.implementation.createHTMLDocument('other');
            const otherOwner=other.createElement('template').content.ownerDocument;
            const xml=document.implementation.createDocument(null,'r',null);
            const xmlOwner=xml.createElementNS('http://www.w3.org/1999/xhtml','template').content.ownerDocument;
            const xmlClone=xmlOwner.cloneNode(true);
            if(Object.getPrototypeOf(xmlOwner)!==Document.prototype || Object.getPrototypeOf(xmlClone)!==Document.prototype)throw 'generic Document class';
            return [owner,owner.cloneNode(true),nested.content.ownerDocument,otherOwner,xmlOwner,xmlClone];
        })()"#).unwrap();
        let state=scheduler.test_bindings().state.clone();
        let mut adapter=RuntimeBindingHost {js:&mut scheduler.js,state};
        let root_origin=adapter.html_document_origin(NodeIdx(0)).unwrap();
        let mut origins=Vec::new();
        for index in 0..6 {
            let value=adapter.get_property(&docs,&index.to_string()).unwrap();
            let document=adapter.node_brand(&value).unwrap();
            let origin=adapter.html_document_origin(document).unwrap();
            assert!(origin.opaque_id().is_some());assert_ne!(origin,root_origin);
            adapter.dom_initialize_template_document_origin(document).unwrap();
            assert_eq!(adapter.html_document_origin(document),Some(origin.clone()));
            origins.push(origin);
        }
        assert_eq!(origins[0],origins[1]);assert_eq!(origins[0],origins[2]);
        assert_ne!(origins[0],origins[3]);assert_ne!(origins[0],origins[4]);
        assert_ne!(origins[3],origins[4]);assert_eq!(origins[4],origins[5]);
    }

    #[test]
    fn selection_programmatic_direction_conversion_and_retained_range_identity() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<body></body>"), native_forwarded: false }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 64).unwrap();
        scheduler.set_location_url("https://example.test/selection");
        let result = scheduler.run_script(r#"(function(){
            function check(v,n){if(!v)throw new Error(n);}
            const s=document.getSelection(); check(s===getSelection(),'singleton');
            const t=document.createTextNode('abcdef');document.body.appendChild(t);
            s.setBaseAndExtent(t,5,t,1);check(s.direction==='backward'&&s.anchorOffset===5&&s.focusOffset===1,'direction');
            const r=s.getRangeAt(0);r.setStart(t,2);check(s.getRangeAt(0)===r&&s.direction==='backward','live range');
            s.extend(t,6);check(s.direction==='forward'&&s.anchorOffset===5&&s.focusOffset===6,'extend');
            check(s.getRangeAt(0)!==r&&r.startOffset===2&&r.endOffset===5,'replace not mutate');
            const newer=s.getRangeAt(0);s.collapseToEnd();check(s.type==='Caret'&&s.getRangeAt(0)!==newer,'collapse replacement');
            check(s.getRangeAt(4294967296)===s.getRangeAt(0),'ToUint32 index');
            const marker={};let caught=false;
            try{s.collapse(t,{valueOf(){throw marker;}});}catch(e){caught=e===marker;}check(caught,'abrupt identity');
            caught=false;try{s.getRangeAt({valueOf(){s.removeAllRanges();return 0;}});}catch(e){caught=e.name==='IndexSizeError';}
            check(caught&&s.type==='None','conversion observes current selection');
            s.setPosition(t,4294967297);check(s.anchorOffset===1,'alias uint32');
            s.setBaseAndExtent(t,1,t,4);const deleted=s.getRangeAt(0);s.deleteFromDocument();
            check(t.data==='aef'&&s.getRangeAt(0)===deleted&&s.isCollapsed,'delete retained identity');
            check(document.implementation.createHTMLDocument('').getSelection()===null,'inert null');
            caught=false;try{Document.prototype.getSelection.call({});}catch(e){caught=e instanceof TypeError;}check(caught,'wrong receiver');
            return true;
        })()"#).unwrap();
        assert_eq!(result, JsValue::Bool(true));
        assert_eq!(scheduler.test_bindings().state.borrow().selection_range_roots.len(), 1);
        scheduler.run_script("getSelection().removeAllRanges()").unwrap();
        assert!(scheduler.test_bindings().state.borrow().selection_range_roots.is_empty());
    }

    #[test]
    fn selection_task_is_coalesced_trusted_and_allows_listener_reentry() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<body></body>"), native_forwarded: false }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 64).unwrap();
        scheduler.set_location_url("https://example.test/selection-events");
        scheduler.run_script(r#"
            var selectionEvents=0, badSelectionEvent=false, bubbledSelectionEvent=0;
            var selectionText=document.createTextNode('abc');document.body.appendChild(selectionText);
            document.body.addEventListener('selectionchange',()=>++bubbledSelectionEvent);
            document.onselectionchange=function(e){
                ++selectionEvents;
                if(e.target!==document||e.bubbles||e.cancelable||!e.isTrusted)badSelectionEvent=true;
                if(selectionEvents===1){getSelection().setPosition(selectionText,0);getSelection().setPosition(selectionText,1);}
            };
            getSelection().setPosition(selectionText,1);getSelection().setPosition(selectionText,2);
            if(selectionEvents!==0)throw 'synchronous event';
        "#).unwrap();
        scheduler.pump_at(std::time::Duration::from_millis(1), false).unwrap();
        assert_eq!(scheduler.run_script("selectionEvents===2&&!badSelectionEvent&&bubbledSelectionEvent===0").unwrap(), JsValue::Bool(true));
        scheduler.run_script("document.onselectionchange=null;getSelection().empty();").unwrap();
        scheduler.pump_at(std::time::Duration::from_millis(2), false).unwrap();
        assert_eq!(scheduler.run_script("selectionEvents").unwrap(), JsValue::num(2.));
    }

    #[test]
    fn selection_cross_realm_ranges_keep_creation_prototypes_and_canonical_identity() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<body></body>"), native_forwarded: false }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, 64).unwrap();
        scheduler.set_location_url("https://example.test/selection-realms");
        let result = scheduler.run_script(r#"(function(){
            function check(v,n){if(!v)throw new Error(n);}
            const f=document.createElement('iframe');document.body.appendChild(f);
            const w=f.contentWindow,d=w.document,s=w.getSelection();
            check(s instanceof w.Selection&&!(s instanceof Selection),'Selection realm');
            const t=d.createTextNode('abc');d.body.appendChild(t);
            const childRange=new w.Range();childRange.selectNodeContents(t);
            Selection.prototype.addRange.call(s,childRange);
            check(Selection.prototype.getRangeAt.call(s,0)===childRange,'foreign retained Range');
            check(s.getRangeAt(0)===childRange&&Object.getPrototypeOf(childRange)===w.Range.prototype,'foreign prototype');
            const rootFactory=Document.prototype.createRange.call(d);
            check(Object.getPrototypeOf(rootFactory)===w.Range.prototype,'receiver createRange realm');
            const childFactory=w.Document.prototype.createRange.call(document);
            check(Object.getPrototypeOf(childFactory)===Range.prototype,'borrowed child factory receiver');
            const rootClone=Range.prototype.cloneRange.call(childRange);
            check(Object.getPrototypeOf(rootClone)===w.Range.prototype,'borrowed clone source realm');
            function Target(){}const customPrototype=Object.create(w.Range.prototype);Target.prototype=customPrototype;
            const custom=Reflect.construct(w.Range,[],Target);
            custom.selectNodeContents(t);s.removeAllRanges();s.addRange(custom);
            check(s.getRangeAt(0)===custom&&Object.getPrototypeOf(custom)===customPrototype,'NewTarget prototype identity');
            let bareError=false;try{w.Range();}catch(e){bareError=e instanceof w.TypeError;}
            check(bareError,'constructor requires new');
            s.removeAllRanges();w.Selection.prototype.collapse.call(s,t,1);
            const childCreated=s.getRangeAt(0);check(Object.getPrototypeOf(childCreated)===w.Range.prototype,'child Selection allocation');
            Selection.prototype.collapse.call(s,t,2);
            const rootCreated=w.Selection.prototype.getRangeAt.call(s,0);
            check(Object.getPrototypeOf(rootCreated)===w.Range.prototype&&rootCreated!==childCreated,'borrowed Selection receiver allocation');
            const shadowHost=d.createElement('div');d.body.appendChild(shadowHost);
            const closed=shadowHost.attachShadow({mode:'closed'}),shadowText=d.createTextNode('closed');closed.appendChild(shadowText);
            s.setBaseAndExtent(shadowText,0,shadowText,6);
            const rootSnapshots=Selection.prototype.getComposedRanges.call(s,{shadowRoots:[closed]});
            check(Object.getPrototypeOf(rootSnapshots)===Array.prototype,'result array callee realm');
            check(Object.getPrototypeOf(rootSnapshots[0])===w.StaticRange.prototype&&rootSnapshots[0].startContainer===shadowText,'snapshot Selection realm closed exposure');
            const childSnapshots=w.Selection.prototype.getComposedRanges.call(s);
            check(Object.getPrototypeOf(childSnapshots)===w.Array.prototype&&childSnapshots[0].startContainer===d.body,'child result realm rescope');
            check(Object.getPrototypeOf(childSnapshots[0])===w.StaticRange.prototype,'rescope snapshot receiver realm');
            s.removeAllRanges();const held=s;f.remove();
            check(d.getSelection()===null&&held.rangeCount===0,'retained inactive selection');
            return true;
        })()"#).unwrap();
        assert_eq!(result, JsValue::Bool(true));
    }

}


impl RuntimeBindingHost<'_> {
    pub(super) fn dom_selection_from_key(&self, key: PlatformKey)
        -> Result<amazing_dom::selection::SelectionId, Throw> {
        let state = self.state.borrow();
        if key.arena != state.arena || key.generation != 1 || key.slot > usize::MAX as u64 {
            return Err(Throw::type_error("Invalid Selection wrapper"));
        }
        let selection = amazing_dom::selection::SelectionId::from_slot(key.slot as usize);
        state.host.borrow().dom.selection_state(selection)
            .map_err(|_| Throw::type_error("Stale Selection document"))?;
        Ok(selection)
    }
}
impl RuntimeBindings {
    pub(crate) fn take_selection_change_tasks(&self) -> Vec<amazing_dom::selection::SelectionChangeTask> {
        let host = self.state.borrow().host.clone();
        host.borrow_mut().dom.drain_selection_change_tasks()
    }
    pub(crate) fn fire_selection_change(&self, js: &mut Interpreter,
        task: amazing_dom::selection::SelectionChangeTask) -> Result<(), Throw> {
        let host = self.state.borrow().host.clone();
        // Clear the document flag before availability checks, realm entry or author callbacks.
        if !host.borrow_mut().dom.begin_selection_change(task) { return Ok(()); }
        let adapter = RuntimeBindingHost { js, state: self.state.clone() };
        if !adapter.html_document_has_browsing_context(task.document) { return Ok(()); }
        let realm = adapter.dom_node_realm(task.document)?;
        let state = self.state.clone();
        js.binding_with_realm(realm, |js| {
            let mut adapter = RuntimeBindingHost { js, state };
            amazing_bindings::dispatch_user_agent_event(&mut adapter,
                amazing_dom::events::EventTarget::Node(task.document), "selectionchange",
                amazing_dom::events::EventInit { bubbles: false, cancelable: false, composed: false })
        })
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn dom_register_range_realm(&mut self, range: amazing_dom::range::Range,
        context: amazing_bindings::host::RangeCreationContext) -> Result<(), Throw> {
        use amazing_bindings::host::RangeCreationContext as Context;
        let realm = match context {
            Context::Constructor => self.js.binding_current_realm(),
            Context::Document(document) => self.dom_node_realm(document)?,
            Context::Clone(source) => {
                let state = self.state.borrow();
                state.host.borrow().dom.range(source).map_err(|_| Throw::type_error("Invalid source Range"))?;
                state.range_creation_realms.get(&source).copied()
                    .ok_or_else(|| Throw::type_error("Source Range realm is unregistered"))?
            }
            Context::Selection(selection) => {
                let document = self.state.borrow().host.borrow().dom.selection_document(selection)
                    .map_err(|_| Throw::type_error("Invalid Selection"))?;
                self.dom_node_realm(document)?
            }
        };
        let state = self.state.borrow();
        state.host.borrow().dom.range(range).map_err(|_| Throw::type_error("Invalid fresh Range"))?;
        if state.range_creation_realms.contains_key(&range) { return Err(Throw::type_error("Range allocation already registered")); }
        drop(state);
        self.state.borrow_mut().range_creation_realms.insert(range, realm);
        Ok(())
    }
    pub(super) fn dom_range_realm_from_key(&self, key: PlatformKey) -> Result<RealmHandle, Throw> {
        let state = self.state.borrow();
        if key.arena != state.arena || key.generation != 1 || key.slot > usize::MAX as u64 {
            return Err(Throw::type_error("Invalid Range wrapper"));
        }
        let range = amazing_dom::range::Range::from_slot(key.slot as usize);
        state.host.borrow().dom.range(range).map_err(|_| Throw::type_error("Released Range"))?;
        state.range_creation_realms.get(&range).copied().ok_or_else(|| Throw::type_error("Range creation realm is unregistered"))
    }
    pub(super) fn dom_selection_range_value(&mut self, selection: amazing_dom::selection::SelectionId,
        range: amazing_dom::range::Range) -> Result<Option<JsValue>, Throw> {
        let state = self.state.borrow();
        let selected = state.host.borrow().dom.selection_state(selection)
            .map_err(|_| Throw::type_error("Invalid Selection"))?;
        if selected.range != Some(range) || !state.range_creation_realms.contains_key(&range) {
            return Err(Throw::type_error("Range is not the registered Selection association"));
        }
        let root = state.selection_range_roots.get(&selection).filter(|(stored, _)| *stored == range).map(|(_, root)| *root);
        drop(state);
        Ok(root.and_then(|root| self.root_value(root)))
    }
    pub(super) fn dom_selection_set_range_value(&mut self, selection: amazing_dom::selection::SelectionId,
        range: amazing_dom::range::Range, value: &JsValue) -> Result<(), Throw> {
        let handle = self.platform_object(value, InterfaceBrand::RANGE).ok_or_else(|| Throw::type_error("Expected a genuine Range"))?;
        if handle.slot != range.slot() as u64 { return Err(Throw::type_error("Range wrapper does not match association")); }
        if let Some(existing) = self.dom_selection_range_value(selection, range)? {
            if !self.same_value(&existing, value) { return Err(Throw::type_error("Selection Range identity changed")); }
            return Ok(());
        }
        let root = self.retain_value(value.clone());
        let old = self.state.borrow_mut().selection_range_roots.insert(selection, (range, root));
        if let Some((_, old)) = old { self.release_value(old); }
        Ok(())
    }
}
