//! HTML constructors retain the actual registry/arena across prototype getters.
use super::*;
use amazing_bindings::html_constructor::{HtmlConstructorHost, HtmlConstructorIdentity};
use amazing_dom::render_mutation::NodeGeneration;

#[derive(Default)]
pub(crate) struct HtmlConstructorRuntime {
    next: u64,
    active: Vec<HtmlConstructorInvocation>,
    reporting: std::collections::HashSet<RealmHandle>,
    parser_creation_documents: Vec<NodeGeneration>,
    /// Parser-internal element creation and insertion calls must not run the
    /// lazy Window named-property refresh: a refresh before the next script
    /// would shadow that script's own `var`/function declarations.
    pub(crate) named_refresh_suppressed: u32,
}
#[derive(Clone)]
struct HtmlConstructorInvocation {
    serial: u64,
    identity: HtmlConstructorIdentity,
    dom: <RuntimeBindingHost<'static> as BindingHost>::DomCapture,
}
impl RuntimeBindingHost<'_> {
    pub(crate) fn html_parser_creation_is_active(&self, document: NodeIdx) -> bool {
        let state = self.state.borrow();
        let host = state.host.borrow();
        state.html_constructor.parser_creation_documents.iter().any(|owner|
            owner.node == document && host.dom.node_generation(document) == Some(owner.generation))
    }
    pub(crate) fn html_construct_custom_element(&mut self, constructor: &JsValue,
        definition: usize, element: NodeIdx) -> Result<JsValue, Throw> {
        let identity = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let native = host.dom.custom_element_definition(definition)
                .ok_or_else(|| Throw::type_error("Custom constructor definition is stale"))?;
            if host.dom.node_generation(element).is_none()
                || host.dom.custom_element_definition_of(element) != Some(definition)
                || host.dom.custom_element_state(element) != amazing_dom::custom_elements::CustomElementState::Precustomized {
                return Err(Throw::type_error("Custom constructor admission is stale"));
            }
            let document = host.dom.custom_element_definition_document(definition)
                .ok_or_else(|| Throw::type_error("Custom registry Document is stale"))?;
            HtmlConstructorIdentity { document, creation_document: document, definition, constructor: native.constructor }
        };
        let arena = self.state.borrow().arena;
        let dom = self.capture_dom(arena)?;
        self.html_construct_with_identity(constructor, identity, dom)
    }
    pub(crate) fn html_construct_autonomous(&mut self, constructor: &JsValue,
        context: &amazing_dom::custom_elements::AutonomousCreationContext,
        dom: &<RuntimeBindingHost<'static> as BindingHost>::DomCapture) -> Result<JsValue, Throw> {
        self.with_captured_dom(dom, |native| {
            native.html_constructor_definition(context.document(), context.definition(), context.constructor())?;
            Ok(())
        })?;
        let identity = HtmlConstructorIdentity { document: context.document(),
            creation_document: context.document(), definition: context.definition(), constructor: context.constructor() };
        self.html_construct_with_identity(constructor, identity, dom.clone())
    }
    fn html_construct_with_identity(&mut self, constructor: &JsValue, identity: HtmlConstructorIdentity,
        dom: <RuntimeBindingHost<'static> as BindingHost>::DomCapture) -> Result<JsValue, Throw> {
        let registered = self.root_value(identity.constructor)
            .ok_or_else(|| Throw::type_error("Custom constructor root is missing"))?;
        if !self.same_value(&registered, constructor) {
            return Err(Throw::type_error("Custom constructor identity differs"));
        }
        let serial = {
            let mut state = self.state.borrow_mut();
            let runtime = &mut state.html_constructor;
            runtime.next = runtime.next.checked_add(1)
                .ok_or_else(|| Throw::type_error("Custom constructor scope limit"))?;
            let serial = runtime.next;
            runtime.active.push(HtmlConstructorInvocation { serial, identity, dom });
            serial
        };
        // The active constructor map exists only around the actual Construct.
        // An abrupt author completion restores the previous nested entry too.
        let result = self.js.binding_construct(constructor, Vec::new());
        let restored = {
            let mut state = self.state.borrow_mut();
            let active = &mut state.html_constructor.active;
            if active.last().is_some_and(|entry| entry.serial == serial) {
                active.pop(); true
            } else {
                if let Some(index) = active.iter().position(|entry| entry.serial == serial) { active.remove(index); }
                false
            }
        };
        if !restored { return Err(Throw::type_error("Custom constructor scope is inconsistent")); }
        result
    }
}

pub(crate) struct HtmlConstructorCapture {
    identity: HtmlConstructorIdentity,
    dom: <RuntimeBindingHost<'static> as BindingHost>::DomCapture,
    interface: Interface,
    new_target: JsValue,
}

impl HtmlConstructorHost for RuntimeBindingHost<'_> {
    type HtmlConstructorCapture = HtmlConstructorCapture;

    fn capture_html_constructor(&mut self, interface: Interface, new_target: &JsValue)
        -> Result<HtmlConstructorCapture, Throw> {
        // The dispatcher enters the actual native constructor's realm. Its
        // associated registry is independent of newTarget's function realm.
        let active = self.constructor(interface)
            .ok_or_else(|| Throw::type_error("HTML constructor intrinsic is unavailable"))?;
        if self.same_value(&active, new_target) {
            return Err(Throw::type_error("Illegal constructor"));
        }
        let creation_node = self.dom_current_document_for_nodes()?;
        let creation_document = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            host.dom.node_generation(creation_node).map(|generation| NodeGeneration { node: creation_node, generation })
                .ok_or_else(|| Throw::type_error("Callee creation Document is stale"))?
        };
        // Upgrade/creation installs the actual original registry for this
        // constructor, including a super interface from a different realm.
        let active = self.state.borrow().html_constructor.active.iter().rev().cloned().collect::<Vec<_>>();
        for entry in active {
            if self.root_value(entry.identity.constructor).is_some_and(|value| self.same_value(&value, new_target)) {
                return Ok(HtmlConstructorCapture { identity: HtmlConstructorIdentity { creation_document, ..entry.identity }, dom: entry.dom,
                    interface, new_target: new_target.clone() });
            }
        }
        let document = self.dom_current_document_for_nodes()?;
        let (generation, definitions) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let generation = host.dom.node_generation(document)
                .ok_or_else(|| Throw::type_error("Constructor Document is stale"))?;
            let definitions = host.dom.custom_element_definitions(document).into_iter()
                .map(|(index, definition)| (index, definition.constructor)).collect::<Vec<_>>();
            (generation, definitions)
        };
        let mut matched = None;
        for (definition, constructor) in definitions {
            if self.root_value(constructor).is_some_and(|value| self.same_value(&value, new_target)) {
                matched = Some((definition, constructor));
                break;
            }
        }
        let (definition, constructor) = matched
            .ok_or_else(|| Throw::type_error("Illegal constructor"))?;
        let arena = self.state.borrow().arena;
        let dom = self.capture_dom(arena)?;
        Ok(HtmlConstructorCapture {
            identity: HtmlConstructorIdentity {
                document: NodeGeneration { node: document, generation }, creation_document, definition, constructor,
            }, dom, interface, new_target: new_target.clone(),
        })
    }
    fn html_constructor_identity(&self, capture: &HtmlConstructorCapture) -> HtmlConstructorIdentity {
        capture.identity
    }
    fn html_constructor_dom_capture(&self, capture: &HtmlConstructorCapture) -> Self::DomCapture {
        capture.dom.clone()
    }
    fn html_constructor_intrinsic_prototype(&mut self, capture: &HtmlConstructorCapture)
        -> Result<JsValue, Throw> {
        // GetFunctionRealm belongs to the fallback branch AFTER the author
        // prototype getter, including a Proxy revoked by that getter.
        let realm = self.js.binding_function_realm(&capture.new_target)?;
        self.state.borrow().interfaces.get(&realm)
            .and_then(|interfaces| interfaces.iter().find(|(interface, _, _)| *interface == capture.interface))
            .map(|(_, prototype, _)| prototype.clone())
            .ok_or_else(|| Throw::type_error("HTML intrinsic prototype is unavailable in newTarget realm"))
    }
    fn html_constructor_wrap(&mut self, capture: &HtmlConstructorCapture,
        element: NodeGeneration, prototype: &JsValue) -> Result<JsValue, Throw> {
        // This projection owns the original host/state, arena and expected
        // generation throughout wrapping; no current-host NodeIdx lookup.
        let value = self.tokenlist_wrap_captured_node(&capture.dom, element)?;
        // HTML constructor step14 performs the internal method and propagates
        // an abrupt completion. It does not convert its boolean false to throw.
        let _ = self.js.binding_set_prototype_of(&value, prototype)?;
        Ok(value)
    }
}

/// Move-only constructor-associated global captured before any authored Construct.
pub(crate) struct HtmlConstructorExceptionContext {
    realm: RealmHandle,
    window: u64,
    document: NodeGeneration,
    dom: <RuntimeBindingHost<'static> as BindingHost>::DomCapture,
}
impl RuntimeBindingHost<'_> {
    pub(crate) fn html_capture_constructor_exception(&mut self, constructor: &JsValue)
        -> Result<HtmlConstructorExceptionContext, Throw> {
        let realm = self.js.binding_object_creation_realm(constructor)?;
        let (window, document, arena) = {
            let state = self.state.borrow();
            let context = state.frames.contexts.get(&realm)
                .ok_or_else(|| Throw::type_error("Constructor has no associated Window"))?;
            (context.event_target_slot, NodeGeneration { node: context.document, generation: context.document_generation }, state.arena)
        };
        let dom = self.capture_dom(arena)?;
        self.with_captured_dom(&dom, |native| {
            if native.node_generation(document.node) != Some(document.generation) { return Err(amazing_dom::DomError::InvalidStateError); }
            Ok(())
        })?;
        Ok(HtmlConstructorExceptionContext { realm, window, document, dom })
    }
    /// Reporting uses the corresponding callback object's retained global,
    /// including a global whose Document has already been retired. This does
    /// not create Document/Fetch authority or promote a stale node generation.
    pub(crate) fn html_capture_microtask_exception(&mut self, callback: &JsValue)
        -> Result<HtmlConstructorExceptionContext, Throw> {
        let realm = self.js.binding_object_creation_realm(callback)?;
        let (window, document, arena) = {
            let state = self.state.borrow();
            let context = state.frames.contexts.get(&realm)
                .ok_or_else(|| Throw::type_error("Microtask callback has no retained Window"))?;
            (context.event_target_slot, NodeGeneration { node: context.document,
                generation: context.document_generation }, state.arena)
        };
        let dom = self.capture_dom(arena)?;
        Ok(HtmlConstructorExceptionContext { realm, window, document, dom })
    }
    pub(crate) fn html_report_constructor_exception(&mut self, context: HtmlConstructorExceptionContext, error: Throw)
        -> Result<(), Throw> {
        // A retained original Window is the destination even if author code
        // navigated it. Never look up its replacement Document or current proxy.
        if !self.state.borrow_mut().html_constructor.reporting.insert(context.realm) {
            eprintln!("Uncaught custom element exception: {}", error.describe());
            return Ok(());
        }
        let state = self.state.clone();
        let realm = context.realm;
        let result = self.js.binding_with_realm(realm, |js| {
            let mut host = RuntimeBindingHost { js, state };
            // The original associated global, not its current Document, is
            // the report destination. Its creation identity was checked before
            // author execution; later Document retirement cannot retarget it.
            let root = host.retain_value(error.0.clone());
            use amazing_dom::events::{Event, EventClass, EventExtra, EventInit, EventTarget};
            let mut event = Event::new("error", EventInit { cancelable: true, bubbles: false, composed: false }, std::time::Duration::ZERO);
            event.set_class(EventClass::Error);
            event.set_extra("error", EventExtra::Root(Some(root)));
            event.set_extra("message", EventExtra::String(error.describe().encode_utf16().collect()));
            // This exception has no parser/source-location receipt. Unknown is
            // represented as such, never inferred from the Document URL.
            event.set_extra("filename", EventExtra::String(Vec::new()));
            event.set_extra("lineno", EventExtra::Number(0.0));
            event.set_extra("colno", EventExtra::Number(0.0));
            let uncanceled = amazing_bindings::dispatch_user_agent_event_with_payload(&mut host, EventTarget::External(context.window), event)?;
            if uncanceled { eprintln!("Uncaught custom element exception: {}", error.describe()); }
            Ok(())
        });
        self.state.borrow_mut().html_constructor.reporting.remove(&realm);
        result
    }
}

#[cfg(test)]
#[path = "runtime_host_html_constructor_tests.rs"]
mod tests;

/// Scoped installed parser bridge: no Interpreter reference is stored inside
/// the persistent parser or callback registry.
pub(crate) struct HtmlParserCreationHost<'a> {
    pub(crate) js: &'a mut Interpreter,
    pub(crate) bindings: RuntimeBindings,
    pub(crate) realm: RealmHandle,
    pub(crate) import: Option<IncrementalImportRef>,
    pub(crate) parser: Rc<RefCell<amazing_html::ResumableParser>>,
}
impl amazing_html::ParserElementCreationHost for HtmlParserCreationHost<'_> {
    fn create(&mut self, source: &mut Dom, request: amazing_html::ParserElementCreation)
        -> Result<NodeGeneration, amazing_dom::DomError> {
        // Only native arena/map borrows exist here; drop both before author JS.
        let document = {
            let state = self.bindings.state.borrow();
            let mut host = state.host.borrow_mut();
            if source.node_generation(request.document.node) != Some(request.document.generation) {
                return Err(amazing_dom::DomError::InvalidStateError);
            }
            if let Some(import) = &self.import {
                let mut import = import.borrow_mut();
                import.sync(&mut host.dom, source)?;
                let document = import.imported(request.document.node).ok_or(amazing_dom::DomError::InvalidStateError)?;
                NodeGeneration { node: document, generation: host.dom.node_generation(document)
                    .ok_or(amazing_dom::DomError::InvalidStateError)? }
            } else {
                // Root parsing owns this actual native arena, not a copy.
                std::mem::swap(source, &mut host.dom);
                request.document
            }
        };
        let state = self.bindings.state.clone();
        state.borrow_mut().html_constructor.named_refresh_suppressed += 1;
        let result = self.js.binding_with_realm(self.realm, |js| {
            let mut adapter = RuntimeBindingHost { js, state };
            let arena = adapter.state.borrow().arena;
            let capture = adapter.capture_dom(arena)?;
            let plan = adapter.with_captured_dom(&capture, |dom|
                dom.prepare_custom_element_creation(document, Some("http://www.w3.org/1999/xhtml"), None,
                    &request.local_name, request.is_value.as_deref(), !request.fragment))?;
            let will_execute = !request.fragment && !matches!(&plan,
                amazing_dom::custom_elements::ElementCreationPlan::Element(_));
            let depth = adapter.state.borrow().html_constructor.parser_creation_documents.len();
            if will_execute {
                adapter.state.borrow_mut().html_constructor.parser_creation_documents.push(document);
                if adapter.js.binding_execution_context_stack_is_empty() {
                    if let Err(error) = self.bindings.parser_microtask_checkpoint(adapter.js) {
                        adapter.state.borrow_mut().html_constructor.parser_creation_documents.truncate(depth);
                        return Err(error);
                    }
                }
                if let Err(error) = adapter.with_captured_dom(&capture, |dom| { dom.custom_element_push_reaction_frame(); Ok(()) }) {
                    adapter.state.borrow_mut().html_constructor.parser_creation_documents.truncate(depth);
                    return Err(error);
                }
            }
            let constructed = (|| {
                let value = amazing_bindings::create_element_from_plan(&mut adapter, &capture, arena, plan)?;
                let key = adapter.platform_object(&value, InterfaceBrand::NODE)
                    .filter(|key| key.arena == arena).ok_or_else(|| Throw::type_error("Parser creation returned a foreign wrapper"))?;
                let node = u32::try_from(key.slot).map(NodeIdx).map_err(|_| Throw::type_error("Parser creation node is invalid"))?;
                adapter.with_captured_dom(&capture, |dom| {
                    if dom.node_generation(node) != Some(key.generation) { return Err(amazing_dom::DomError::InvalidStateError); }
                    for (name, value) in &request.attributes { dom.set_attribute(node, name, value)?; }
                    Ok(())
                })?;
                Ok(NodeGeneration { node, generation: key.generation })
            })();
            let reactions = if will_execute {
                adapter.with_captured_dom(&capture, |dom| dom.custom_element_pop_isolated_reaction_frame())
                    .and_then(|frame| amazing_bindings::deliver_custom_element_reaction_frame(&mut adapter, &capture, frame))
            } else { Ok(()) };
            // Attribute callbacks also run under the original counter; normal
            // and abrupt completions both restore the exact previous depth.
            adapter.state.borrow_mut().html_constructor.parser_creation_documents.truncate(depth);
            constructed.and_then(|selected| reactions.map(|_| selected))
        });
        self.bindings.state.borrow_mut().html_constructor.named_refresh_suppressed -= 1;
        let state = self.bindings.state.borrow();
        let mut host = state.host.borrow_mut();
        if let Some(import) = &self.import {
            let result = result.map_err(|_| amazing_dom::DomError::InvalidStateError)?;
            let mut import = import.borrow_mut();
            let selected = import.reverse_sync_node(source, &host.dom, result)?;
            import.reverse_sync(source, &host.dom)?;
            Ok(selected)
        } else {
            // Restore the original arena on both normal and abrupt completion.
            std::mem::swap(source, &mut host.dom);
            result.map_err(|_| amazing_dom::DomError::InvalidStateError)
        }
    }
    fn inserted(&mut self, tree: &mut amazing_html::TreeBuilder<'_>,
        queue: amazing_dom::custom_elements::CustomElementReactionFrame) -> Result<(), amazing_dom::DomError> {
        // Publish this exact prefix before author callbacks. The outer Rust
        // call frames resume against the mutated state reclaimed afterwards.
        self.parser.borrow_mut().publish_execution_frame(tree.take_execution_frame())?;
        let state = self.bindings.state.clone();
        state.borrow_mut().html_constructor.named_refresh_suppressed += 1;
        let result = self.js.binding_with_realm(self.realm, |js| {
            let mut adapter = RuntimeBindingHost { js, state };
            let arena = adapter.state.borrow().arena;
            let capture = adapter.capture_dom(arena)?;
            let native_queue = {
                let binding = adapter.state.borrow();
                let mut host = binding.host.borrow_mut();
                let mut parser = self.parser.borrow_mut();
                if let Some(import) = &self.import {
                    // The private parser arena has no registry authority. The
                    // actual connection and queue arise from the native graft.
                    host.dom.custom_element_push_reaction_frame();
                    let graft = import.borrow_mut().sync(&mut host.dom, parser.dom());
                    let native_queue = host.dom.custom_element_pop_isolated_reaction_frame();
                    graft.map_err(|error| Throw::type_error(format!("Parser insertion graft: {error:?}")))?;
                    native_queue.map_err(|error| Throw::type_error(format!("Parser insertion queue: {error:?}")))?
                } else {
                    parser.swap_dom(&mut host.dom);
                    queue
                }
            };
            // No parser, arena, or map borrow survives this authored boundary.
            let delivered = amazing_bindings::deliver_custom_element_reaction_frame(&mut adapter, &capture, native_queue);
            let restored = {
                let binding = adapter.state.borrow();
                let host = binding.host.clone();
                let mut host = host.borrow_mut();
                let mut parser = self.parser.borrow_mut();
                if let Some(import) = &self.import {
                    import.borrow_mut().reverse_sync(parser.dom_mut(), &host.dom)
                        .map_err(|error| Throw::type_error(format!("Parser insertion reverse graft: {error:?}")))
                } else { parser.swap_dom(&mut host.dom); Ok(()) }
            };
            delivered.and(restored)
        });
        self.bindings.state.borrow_mut().html_constructor.named_refresh_suppressed -= 1;
        // Both normal and abrupt completion restore the exact execution state,
        // including nested document.write's real stack/tokenizer mutations.
        let frame = self.parser.borrow_mut().reclaim_execution_frame()?;
        tree.restore_execution_frame(frame);
        result.map_err(|_| amazing_dom::DomError::InvalidStateError)
    }

}
