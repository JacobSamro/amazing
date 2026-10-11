//! Original receiving-arena custody for DOMTokenList author conversions.
use super::*;
use amazing_js::value::{PrivateName, PrivateElementKind};

pub(super) struct TokenListRuntime { owner_name: PrivateName }
impl Default for TokenListRuntime {
    fn default() -> Self { Self { owner_name: PrivateName::new("DOMTokenList associated element") } }
}
#[derive(Clone)]
pub(crate) struct RuntimeDomCapture {
    host: Rc<RefCell<BrowserDomHost>>,
    state: Rc<RefCell<BindingState>>,
    arena: u64,
    realm: RealmHandle,
}
impl RuntimeDomCapture {
    pub(super) fn arena(&self) -> u64 { self.arena }
}
impl RuntimeBindingHost<'_> {
    pub(super) fn tokenlist_capture_dom(&self, arena: u64) -> Result<RuntimeDomCapture, Throw> {
        let state = self.state.borrow();
        if arena != state.arena { return Err(Throw::type_error("Invalid original DOM arena")); }
        Ok(RuntimeDomCapture { host: state.host.clone(), state: self.state.clone(), arena, realm: self.js.binding_current_realm() })
    }
    pub(super) fn tokenlist_with_captured_dom<R>(&mut self, capture: &RuntimeDomCapture,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>) -> Result<R, Throw> {
        self.js.binding_realm_global(capture.realm)?;
        if capture.state.borrow().arena != capture.arena { return Err(Throw::type_error("Original DOM arena retired")); }
        self.tokenlist_with_native_host(capture.host.clone(), capture.state.clone(), callback)
    }
    pub(super) fn tokenlist_wrap_captured_key(&mut self, capture: &RuntimeDomCapture,
        key: PlatformKey) -> Result<JsValue, Throw> {
        if key.arena != capture.arena { return Err(Throw::type_error("Invalid original node arena")); }
        let node = NodeIdx(u32::try_from(key.slot).map_err(|_| Throw::type_error("Invalid original node slot"))?);
        self.tokenlist_wrap_captured_node(capture, amazing_dom::render_mutation::NodeGeneration {
            node, generation: key.generation })
    }
    /// Canonical original-arena wrapper after author conversion. Never selects
    /// the caller's possibly replaced current state from a bare NodeIdx.
    pub(super) fn tokenlist_wrap_captured_node(&mut self, capture: &RuntimeDomCapture,
        element: amazing_dom::render_mutation::NodeGeneration) -> Result<JsValue, Throw> {
        self.js.binding_realm_global(capture.realm)?;
        {
            let state = capture.state.borrow();
            if state.arena != capture.arena || !Rc::ptr_eq(&state.host, &capture.host)
                || capture.host.borrow().dom.node_generation(element.node) != Some(element.generation) {
                return Err(Throw::type_error("Original node arena retired"));
            }
        }
        let mut adapter = RuntimeBindingHost { js: self.js, state: capture.state.clone() };
        adapter.wrap_platform_object(PlatformKey { arena: capture.arena,
            slot: u64::from(element.node.0), generation: element.generation }, InterfaceBrand::NODE)
    }
    pub(super) fn tokenlist_with_current_dom<R>(&mut self,
        callback: impl FnOnce(&mut Dom) -> Result<R, DomError>) -> Result<R, Throw> {
        let host = self.state.borrow().host.clone();
        self.tokenlist_with_native_host(host, self.state.clone(), callback)
    }
    fn tokenlist_with_native_host<R>(&mut self, host: Rc<RefCell<BrowserDomHost>>,
        state: Rc<RefCell<BindingState>>, callback: impl FnOnce(&mut Dom) -> Result<R, DomError>) -> Result<R, Throw> {
        let mut adapter = RuntimeBindingHost { js: self.js, state: state.clone() };
        let result = callback(&mut host.borrow_mut().dom);
        // Preserve the order in which script queues JS jobs and native DOM
        // mutations. Collecting only at the end of the script would move the
        // mutation checkpoint behind jobs queued after this operation.
        // A committed Selection mutation enters the shared FIFO before any
        // later author registration, even when this operation returns an error.
        let emitted = RuntimeBindings { state }.emit_native_selection_tasks(adapter.js);
        let microtasks = (|| {
            let pending = host.borrow().pending_native_microtasks();
            for request in pending {
                adapter.js.enqueue_native_microtask(request.global, request.kind)?;
                host.borrow_mut().acknowledge_native_microtask(request);
            }
            Ok::<(), Throw>(())
        })();
        if result.is_ok() { emitted?; microtasks?; }
        // Failed emissions remain pending for checkpoint retry. An earlier DOM
        // error takes precedence over a later scheduling error.
        // HTML-35: named access on window (SS7.2.2.3). Every DOM-mutating
        // binding call passes through this one function, so this is the
        // single choke point that can keep window's named properties live
        // without a hook into amazing-js's property lookup itself. The flag
        // check is one bool per call in the overwhelming common case
        // (nothing id/name/tree-membership-relevant changed); the O(document
        // size) rescan behind it only runs when `Dom::mark_named_objects_dirty`
        // actually fired.
        let suppressed = adapter.state.borrow().html_constructor.named_refresh_suppressed > 0;
        if result.is_ok() && !suppressed && host.borrow_mut().dom.take_named_objects_dirty() {
            adapter.dom_refresh_window_named_properties()?;
        }
        result.map_err(|error| throw_dom_error(&mut adapter, error))
    }
    pub(super) fn tokenlist_supported_tokens(&mut self, capture: &RuntimeDomCapture,
        list: &amazing_dom::token_list::TokenListCapture) -> Result<Option<Vec<Vec<u16>>>, Throw> {
        self.tokenlist_with_captured_dom(capture, |dom| {
            let node = dom.token_list_owner(list)?.node;
            let attribute = dom.token_list_attribute(list)?;
            if dom.namespace_uri(node) != Some("http://www.w3.org/1999/xhtml") { return Ok(None); }
            // Preserve the embedding's existing explicit supported-token
            // registry during this custody migration. Completing each HTML
            // processing model is a separate prerequisite, not inferred here
            // from an allowed keyword or an element's spelling.
            let words: Option<&[&str]> = match (attribute, dom.local_name(node)) {
                ("rel", Some("a" | "area")) => Some(&["noreferrer", "noopener", "opener"]),
                ("rel", Some("link")) => Some(&["alternate", "dns-prefetch", "icon", "manifest", "modulepreload", "preconnect", "prefetch", "preload", "stylesheet"]),
                ("rel", _) => Some(&[]),
                _ => None,
            };
            Ok(words.map(|words| words.iter().map(|word| word.encode_utf16().collect()).collect()))
        })
    }
    pub(super) fn tokenlist_wrap_in_owner_realm(&mut self, key: PlatformKey) -> Result<Option<JsValue>, Throw> {
        let capture = self.tokenlist_capture_dom(key.arena)?;
        let owner = {
            let host = capture.host.borrow();
            let view = host.dom.token_list_capture(usize::try_from(key.slot).map_err(|_| Throw::type_error("Invalid token view"))?)
                .map_err(|_| Throw::type_error("Invalid token view"))?;
            if view.view_generation() != key.generation { return Err(Throw::type_error("Invalid token view generation")); }
            host.dom.token_list_owner(&view).map_err(|_| Throw::type_error("Retired token owner"))?.node
        };
        let realm = self.dom_node_realm(owner)?;
        if realm == self.js.binding_current_realm() { return Ok(None); }
        let state = capture.state.clone();
        self.js.binding_with_realm(realm, |js| RuntimeBindingHost { js, state }
            .wrap_platform_object(key, InterfaceBrand::DOM_TOKEN_LIST)).map(Some)
    }
    pub(super) fn tokenlist_associate_element(&mut self, value: &JsValue, key: PlatformKey) -> Result<(), Throw> {
        let capture = self.tokenlist_capture_dom(key.arena)?;
        let owner = {
            let host = capture.host.borrow();
            let view = host.dom.token_list_capture(key.slot as usize).map_err(|_| Throw::type_error("Invalid token view"))?;
            if view.view_generation() != key.generation { return Err(Throw::type_error("Invalid token view generation")); }
            host.dom.token_list_owner(&view).map_err(|_| Throw::type_error("Retired token owner"))?.node
        };
        let name = capture.state.borrow().tokenlist.owner_name.clone();
        let JsValue::Object(object) = value else { return Err(Throw::type_error("Invalid token wrapper")); };
        if object.borrow().private_has(&name) { return Ok(()); }
        let element = self.wrap_node(owner)?;
        // A real private value edge, traced by the JS collector and invisible
        // to properties/reflection. The weak canonical wrapper cache cannot
        // replace this associated-element ownership edge.
        object.borrow_mut().private_add(name, PrivateElementKind::Field(element))
    }
}

#[cfg(test)]
#[path = "runtime_host_tokenlist_tests.rs"]
mod tests;
