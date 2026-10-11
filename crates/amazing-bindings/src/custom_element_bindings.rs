//! CustomElementRegistry and reaction delivery. User code always runs outside
//! the DOM borrow; the host owns constructor, callback, and promise values.
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{custom_elements::{CustomElementDefinition, CustomElementReaction,
    CustomElementState, LifecycleCallbacks}, Dom, DomError, NodeIdx};
use std::collections::VecDeque;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::CustomElementRegistryDefine | O::CustomElementRegistryGet
        | O::CustomElementRegistryGetName | O::CustomElementRegistryWhenDefined
        | O::CustomElementRegistryUpgrade)
}

pub fn registry_for_document<H: BindingHost>(host: &mut H, document: NodeIdx)
    -> Result<H::Value, H::Error> {
    host.wrap_platform_object(host.node_object_key(document), InterfaceBrand::CUSTOM_ELEMENT_REGISTRY)
}
fn registry_document<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    receiver_to_document(host, receiver).ok_or_else(|| host.type_error("Expected a CustomElementRegistry"))
}
fn receiver_to_document<H: BindingHost>(host: &H, receiver: &H::Value) -> Option<NodeIdx> {
    host.platform_object(receiver, InterfaceBrand::CUSTOM_ELEMENT_REGISTRY)
        .and_then(|key| host.node_from_object_key(key))
}
fn registry_dom<H: BindingHost, R>(host: &mut H, capture: &H::DomCapture, key: PlatformKey,
    callback: impl FnOnce(&mut Dom) -> Result<R, DomError>) -> Result<R, H::Error> {
    host.with_captured_dom(capture, |dom| {
        let document = NodeIdx(u32::try_from(key.slot).map_err(|_| DomError::InvalidNode)?);
        if dom.node_generation(document) != Some(key.generation)
            || !matches!(dom.get(document).kind, amazing_dom::NodeKind::Document) {
            return Err(DomError::InvalidStateError);
        }
        callback(dom)
    })
}
fn name<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<String, H::Error> {
    let value = value.ok_or_else(|| host.type_error("Not enough arguments"))?;
    let units = host.to_dom_string(value)?;
    String::from_utf16(&units).map_err(|_| host.type_error("Invalid custom element name"))
}
fn callback<H: BindingHost>(host: &mut H, object: &H::Value, property: &'static str)
    -> Result<Option<H::Value>, H::Error> {
    let value = host.get_property(object, property)?;
    if host.is_undefined(&value) { return Ok(None); }
    if !host.is_callable(&value) { return Err(host.type_error("Custom element callback is not callable")); }
    Ok(Some(value))
}
struct Extracted<V> {
    connected: Option<V>, disconnected: Option<V>, connected_move: Option<V>,
    adopted: Option<V>, attribute_changed: Option<V>,
    form_associated_callback: Option<V>, form_reset: Option<V>,
    form_disabled: Option<V>, form_state_restore: Option<V>,
    observed: Vec<Vec<u16>>, disable_internals: bool, disable_shadow: bool,
    form_associated: bool,
}
fn extract<H: BindingHost>(host: &mut H, constructor: &H::Value) -> Result<Extracted<H::Value>, H::Error> {
    let prototype = host.get_property(constructor, "prototype")?;
    host.require_object(&prototype)?;
    let connected = callback(host, &prototype, "connectedCallback")?;
    let disconnected = callback(host, &prototype, "disconnectedCallback")?;
    let connected_move = callback(host, &prototype, "connectedMoveCallback")?;
    let adopted = callback(host, &prototype, "adoptedCallback")?;
    let attribute_changed = callback(host, &prototype, "attributeChangedCallback")?;
    let observed = if attribute_changed.is_some() {
        let values = host.get_property(constructor, "observedAttributes")?;
        if host.is_undefined(&values) { Vec::new() } else {
            host.sequence_dom_strings(&values)?
        }
    } else { Vec::new() };
    let disabled = host.get_property(constructor, "disabledFeatures")?;
    let disabled = if host.is_undefined(&disabled) { Vec::new() }
        else { host.sequence_dom_strings(&disabled)? };
    let disable_internals = disabled.iter().any(|value| value == &"internals".encode_utf16().collect::<Vec<_>>());
    let disable_shadow = disabled.iter().any(|value| value == &"shadow".encode_utf16().collect::<Vec<_>>());
    let associated = host.get_property(constructor, "formAssociated")?;
    let form_associated = host.to_boolean(&associated);
    let (form_associated_callback, form_reset, form_disabled, form_state_restore) = if form_associated {
        (callback(host, &prototype, "formAssociatedCallback")?,
            callback(host, &prototype, "formResetCallback")?,
            callback(host, &prototype, "formDisabledCallback")?,
            callback(host, &prototype, "formStateRestoreCallback")?)
    } else { (None, None, None, None) };
    Ok(Extracted { connected, disconnected, connected_move, adopted, attribute_changed,
        form_associated_callback, form_reset, form_disabled, form_state_restore,
        observed, disable_internals, disable_shadow, form_associated })
}
fn define<H: BindingHost>(host: &mut H, document: NodeIdx, capture: &H::DomCapture, document_key: PlatformKey, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    let name = name(host, args.first())?;
    let constructor = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
    if !host.is_constructor(constructor)? { return Err(host.type_error("Expected a constructor")); }
    if !Dom::valid_custom_element_name(&name) {
        return registry_dom::<H, H::Value>(host, capture, document_key, |_| Err(DomError::SyntaxError));
    }
    let definitions = registry_dom(host, capture, document_key, |dom| Ok(dom.custom_element_definitions(document)
        .into_iter().map(|(_, def)| (def.name.clone(), def.constructor)).collect::<Vec<_>>()))?;
    if definitions.iter().any(|(existing, _)| existing == &name) {
        return registry_dom::<H, H::Value>(host, capture, document_key, |_| Err(DomError::NotSupportedError));
    }
    for (_, root) in &definitions {
        if let Some(existing) = host.root_value(*root) {
            if host.same_value(&existing, constructor) {
                return registry_dom::<H, H::Value>(host, capture, document_key, |_| Err(DomError::NotSupportedError));
            }
        }
    }
    let extends = if let Some(options) = args.get(2) {
        let value = host.get_property(options, "extends")?;
        if host.is_undefined(&value) { None }
        else { Some(String::from_utf16_lossy(&host.to_dom_string(&value)?)) }
    } else { None };
    let local_name = if let Some(extends) = extends {
        if Dom::valid_custom_element_name(&extends)
            || crate::html_element_interface(&extends) == crate::host::Interface::HTMLUnknownElement {
            return registry_dom::<H, H::Value>(host, capture, document_key, |_| Err(DomError::NotSupportedError));
        }
        extends
    } else { name.clone() };
    registry_dom(host, capture, document_key, |dom| dom.custom_element_begin_definition(document))?;
    let extracted = extract(host, constructor);
    registry_dom(host, capture, document_key, |dom| { dom.custom_element_end_definition(document); Ok(()) })?;
    let extracted = extracted?;
    let constructor_root = host.retain_value(constructor.clone());
    let mut definition = CustomElementDefinition::new(document, name.clone(), local_name, constructor_root);
    let mut roots = Vec::new();
    let mut retain = |value: Option<H::Value>| value.map(|value| {
        let root = host.retain_value(value); roots.push(root); root
    });
    definition.callbacks = LifecycleCallbacks {
        connected: retain(extracted.connected), disconnected: retain(extracted.disconnected),
        connected_move: retain(extracted.connected_move), adopted: retain(extracted.adopted),
        attribute_changed: retain(extracted.attribute_changed),
        form_associated: retain(extracted.form_associated_callback), form_reset: retain(extracted.form_reset),
        form_disabled: retain(extracted.form_disabled), form_state_restore: retain(extracted.form_state_restore),
    };
    definition.set_observed_attribute_units(extracted.observed);
    definition.disable_internals = extracted.disable_internals;
    definition.disable_shadow = extracted.disable_shadow;
    definition.form_associated = extracted.form_associated;
    if let Err(error) = registry_dom(host, capture, document_key, |dom| dom.define_custom_element(definition)) {
        host.release_value(constructor_root);
        for root in roots { host.release_value(root); }
        return Err(error);
    }
    let promises = registry_dom(host, capture, document_key, |dom| Ok(dom.take_custom_element_when_defined(document, &name)))?;
    for root in promises {
        if let Some(promise) = host.root_value(root) {
            host.resolve_promise(&promise, constructor.clone())?;
        }
        host.release_value(root);
    }
    Ok(host.undefined())
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    let document = registry_document(host, &receiver)?;
    let document_key = host.node_object_key(document);
    let capture = host.capture_dom(document_key.arena)?;
    match op {
        O::CustomElementRegistryDefine => define(host, document, &capture, document_key, args),
        O::CustomElementRegistryGet => {
            let name = name(host, args.first())?;
            let root = registry_dom(host, &capture, document_key, |dom| Ok(dom.custom_element_definition_by_name(document, &name)
                .map(|(_, definition)| definition.constructor)))?;
            Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.undefined()))
        }
        O::CustomElementRegistryGetName => {
            let constructor = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_constructor(constructor)? { return Err(host.type_error("Expected a constructor")); }
            let definitions = registry_dom(host, &capture, document_key, |dom| Ok(dom.custom_element_definitions(document).into_iter()
                .map(|(_, definition)| (definition.name.clone(), definition.constructor)).collect::<Vec<_>>()))?;
            for (name, root) in definitions {
                if let Some(existing) = host.root_value(root) {
                    if host.same_value(&existing, constructor) {
                        return host.string(&name.encode_utf16().collect::<Vec<_>>());
                    }
                }
            }
            Ok(host.null())
        }
        O::CustomElementRegistryWhenDefined => {
            let name = name(host, args.first())?;
            if let Some(root) = registry_dom(host, &capture, document_key, |dom| Ok(dom.custom_element_pending_promise(document, &name)))? {
                if let Some(promise) = host.root_value(root) { return Ok(promise); }
            }
            let promise = host.new_promise()?;
            if !Dom::valid_custom_element_name(&name) {
                let reason = host.dom_exception_value(DomError::SyntaxError)?;
                host.reject_promise(&promise, reason)?;
                return Ok(promise);
            }
            let constructor = registry_dom(host, &capture, document_key, |dom| Ok(dom.custom_element_definition_by_name(document, &name)
                .map(|(_, definition)| definition.constructor)))?;
            if let Some(root) = constructor {
                if let Some(value) = host.root_value(root) { host.resolve_promise(&promise, value)?; }
            } else {
                let root = host.retain_value(promise.clone());
                registry_dom(host, &capture, document_key, |dom| { dom.custom_element_when_defined(document, name, root); Ok(()) })?;
            }
            Ok(promise)
        }
        O::CustomElementRegistryUpgrade => {
            let root = args.first().and_then(|value| host.node_brand(value))
                .ok_or_else(|| host.type_error("Expected a Node"))?;
            registry_dom(host, &capture, document_key, |dom| dom.queue_custom_element_upgrades_for_tree(root))?;
            Ok(host.undefined())
        }
        _ => unreachable!(),
    }
}
fn callback_value<H: BindingHost>(host: &mut H, callback: amazing_dom::events::CallbackId,
    element: NodeIdx, args: &[H::Value]) -> Result<(), H::Error> {
    let function = host.root_value(callback).ok_or_else(|| host.type_error("Released custom element callback"))?;
    let this = host.wrap_node(element)?;
    host.call(&function, &this, args).map(|_| ())
}
/// Drain reactions after a [CEReactions] boundary or parser task. The driver
/// can call this after native parser mutations as well.
pub fn deliver_custom_element_reactions<H: BindingHost>(host: &mut H) -> Result<(), H::Error> {
    deliver_custom_element_reactions_for_created(host, None).map(|_| ())
}
pub(crate) fn deliver_custom_element_reactions_for_created<H: BindingHost>(host: &mut H,
    _created: Option<NodeIdx>) -> Result<Option<H::Value>, H::Error> {
    let replacement = None;
    loop {
        let reactions = host.with_dom(|dom| Ok(dom.take_custom_element_reactions()))?;
        if reactions.is_empty() { return Ok(replacement); }
        let mut reactions: VecDeque<_> = reactions.into();
        while let Some(reaction) = reactions.pop_front() {
            let upgrade = matches!(reaction, CustomElementReaction::Upgrade { .. });
            let capture = host.capture_dom_for_reactions()?;
            let result = invoke_custom_element_reaction(host, &capture, reaction);
            if let Err(error) = result { let _ = host.report_custom_element_exception(error); }
            let fresh = host.with_dom(|dom| Ok(dom.take_custom_element_reactions()))?;
            if upgrade {
                for reaction in fresh.into_iter().rev() { reactions.push_front(reaction); }
            } else {
                reactions.extend(fresh);
            }
        }
    }
}

fn frame_wrap_node<H: BindingHost>(host: &mut H, capture: &H::DomCapture, node: NodeIdx) -> Result<H::Value, H::Error> {
    let generation = host.with_captured_dom(capture, |dom| dom.node_generation(node).ok_or(DomError::InvalidNode))?;
    let key = PlatformKey { arena: host.dom_capture_arena(capture), slot: u64::from(node.0), generation };
    host.wrap_node_in_captured_dom(capture, key)
}
fn callback_value_captured<H: BindingHost>(host: &mut H, capture: &H::DomCapture,
    callback: amazing_dom::events::CallbackId, element: NodeIdx, args: &[H::Value]) -> Result<(), H::Error> {
    let function = host.root_value(callback).ok_or_else(|| host.type_error("Released custom element callback"))?;
    let this = frame_wrap_node(host, capture, element)?;
    let exception_context = host.capture_custom_element_constructor_exception_context(&function)?;
    match host.call(&function, &this, args) {
        Ok(_) => Ok(()),
        Err(error) => host.report_custom_element_constructor_exception(exception_context, error),
    }
}

fn invoke_custom_element_reaction<H: BindingHost>(host: &mut H, capture: &H::DomCapture, reaction: CustomElementReaction) -> Result<(), H::Error> {
    match reaction {
                CustomElementReaction::Upgrade { element, definition } => {
                    let state = host.with_captured_dom(capture, |dom| Ok(dom.custom_element_state(element)))?;
                    if state != CustomElementState::Undefined { return Ok(()); }
                    let generation = host.with_captured_dom(capture, |dom| dom.node_generation(element).ok_or(DomError::InvalidNode))?;
                    let key = PlatformKey { arena: host.dom_capture_arena(capture), slot: u64::from(element.0), generation };
                    let constructor_root = match host.with_captured_dom(capture, |dom|
                        dom.begin_custom_element_upgrade(element, definition)) {
                        Ok(root) => root,
                        Err(error) => { let _ = host.report_custom_element_exception(error); return Ok(()); }
                    };
                    let Some(constructor) = host.root_value(constructor_root) else { return Ok(()) };
                    let exception_context = host.capture_custom_element_constructor_exception_context(&constructor)?;
                    let constructed = host.construct_custom_element(&constructor, definition, element);
                    let same = match &constructed {
                        Ok(value) => { let expected = host.wrap_node_in_captured_dom(capture, key)?; host.same_value(value, &expected) },
                        Err(_) => false,
                    };
                    let finish = host.with_captured_dom(capture, |dom|
                        dom.finish_custom_element_upgrade(element, definition, same));
                    if same { if let Ok(value) = &constructed { pin_constructed_wrapper(host, value); } }
                    if let Err(error) = constructed {
                        let _ = host.report_custom_element_constructor_exception(exception_context, error);
                        Ok(())
                    } else { finish }
                }
                CustomElementReaction::Connected { element, callback }
                | CustomElementReaction::Disconnected { element, callback }
                | CustomElementReaction::ConnectedMove { element, callback } =>
                    callback_value_captured(host, capture, callback, element, &[]),
                CustomElementReaction::Adopted { element, callback, old_document, new_document } => {
                    let old = frame_wrap_node(host, capture, old_document)?;
                    let new = frame_wrap_node(host, capture, new_document)?;
                    callback_value_captured(host, capture, callback, element, &[old, new])
                }
                CustomElementReaction::AttributeChanged { element, callback, name, old_value, new_value, namespace } => {
                    let name = host.string(&name.encode_utf16().collect::<Vec<_>>())?;
                    let old = match old_value { Some(value) => host.string(&value)?,
                        None => host.null() };
                    let new = match new_value { Some(value) => host.string(&value)?,
                        None => host.null() };
                    let namespace = match namespace { Some(value) => host.string(&value.encode_utf16().collect::<Vec<_>>())?,
                        None => host.null() };
                    callback_value_captured(host, capture, callback, element, &[name, old, new, namespace])
                }
                CustomElementReaction::FormAssociated { element, callback, form } => {
                    let form = match form { Some(form) => frame_wrap_node(host, capture, form)?, None => host.null() };
                    callback_value_captured(host, capture, callback, element, &[form])
                }
                CustomElementReaction::FormDisabled { element, callback, disabled } => {
                    let disabled = host.boolean(disabled);
                    callback_value_captured(host, capture, callback, element, &[disabled])
                }
                CustomElementReaction::FormReset { element, callback } =>
                    callback_value_captured(host, capture, callback, element, &[]),
            }
}

/// Invoke exactly the popped frame in its original native arena. Fresh nested
/// upgrade reactions belong to an explicit owned frame, not the backup queue.
pub fn deliver_custom_element_reaction_frame<H: BindingHost>(host: &mut H,
    capture: &H::DomCapture, mut frame: amazing_dom::custom_elements::CustomElementReactionFrame) -> Result<(), H::Error> {
    if frame.is_empty() { return Ok(()); }
    while let Some(reaction) = host.with_captured_dom(capture, |dom| dom.custom_element_next_isolated_reaction(&mut frame))? {
        let upgrade = matches!(reaction, CustomElementReaction::Upgrade { .. });
        if upgrade { host.with_captured_dom(capture, |dom| { dom.custom_element_push_reaction_frame(); Ok(()) })?; }
        let result = invoke_custom_element_reaction(host, capture, reaction);
        let nested = if upgrade {
            Some(host.with_captured_dom(capture, |dom| dom.custom_element_pop_isolated_reaction_frame())?)
        } else { None };
        if let Err(error) = result { let _ = host.report_custom_element_exception(error); }
        if let Some(nested) = nested { deliver_custom_element_reaction_frame(host, capture, nested)?; }
    }
    Ok(())
}

/// A successfully constructed custom element's `this` wrapper carries the
/// author class prototype and own properties. The runtime's wrapper cache is
/// weak, so pin it for the document's lifetime or a later lookup would return
/// a plain HTMLElement wrapper.
fn pin_constructed_wrapper<H: BindingHost>(host: &mut H, value: &H::Value) {
    let _ = host.retain_value(value.clone());
}
fn creation_error<H: BindingHost>(host: &mut H, capture: &H::DomCapture, error: DomError) -> H::Error {
    match host.with_captured_dom::<()>(capture, |_| Err(error)) { Err(error) => error, Ok(()) => unreachable!() }
}
/// Whole synchronous create-element consumer of the native allocation plan.
/// It never exposes a scratch element as the result of an author constructor.
pub fn create_element_from_plan<H: BindingHost>(host: &mut H, capture: &H::DomCapture,
    arena: u64, plan: amazing_dom::custom_elements::ElementCreationPlan) -> Result<H::Value, H::Error> {
    use amazing_dom::custom_elements::{ElementCreationPlan as Plan, CustomElementCreationCompletion as Completion};
    // The constructor's `this` wrapper (and its author-defined own properties)
    // must stay alive until the result is wrapped, or a fresh wrapper is returned.
    let mut keep_alive: Option<H::Value> = None;
    let element = match plan {
        Plan::Element(element) => element,
        Plan::Upgrade { element, definition, constructor } => {
            let value = host.root_value(constructor).ok_or_else(|| host.type_error("Registered constructor root is missing"))?;
            host.with_captured_dom(capture, |dom| dom.begin_custom_element_upgrade(element.node, definition))?;
            let exception_context = host.capture_custom_element_constructor_exception_context(&value)?;
            let constructed = host.construct_custom_element(&value, definition, element.node);
            let expected = host.wrap_node_in_captured_dom(capture, PlatformKey { arena,
                slot: u64::from(element.node.0), generation: element.generation })?;
            let same = constructed.as_ref().is_ok_and(|value| host.same_value(value, &expected));
            let finished = host.with_captured_dom(capture, |dom|
                dom.finish_custom_element_upgrade(element.node, definition, same));
            match constructed {
                Err(error) => { let _ = host.report_custom_element_constructor_exception(exception_context, error); }
                Ok(value) => {
                    if same { pin_constructed_wrapper(host, &value); }
                    keep_alive = Some(value);
                    if let Err(error) = finished { let _ = host.report_custom_element_constructor_exception(exception_context, error); }
                }
            }
            element
        }
        Plan::Autonomous(context) => {
            let constructor = host.root_value(context.constructor())
                .ok_or_else(|| host.type_error("Registered constructor root is missing"))?;
            let exception_context = host.capture_custom_element_constructor_exception_context(&constructor)?;
            let constructed = host.construct_custom_element_autonomous(&constructor, &context, capture);
            let returned = constructed.as_ref().ok().and_then(|value| host.platform_object(value, InterfaceBrand::NODE))
                .filter(|key| key.arena == arena).and_then(|key| u32::try_from(key.slot).ok()
                    .map(|node| amazing_dom::render_mutation::NodeGeneration { node: NodeIdx(node), generation: key.generation }));
            let completed = host.with_captured_dom(capture, |dom| dom.complete_custom_element_creation(context, returned))?;
            match completed {
                Completion::Created(element) => {
                    keep_alive = constructed.ok();
                    if let Some(value) = &keep_alive { pin_constructed_wrapper(host, value); }
                    element
                }
                Completion::Failed(failure) => {
                    let error = match constructed { Err(error) => error, Ok(_) => creation_error(host, capture, failure.reason()) };
                    let _ = host.report_custom_element_constructor_exception(exception_context, error);
                    host.with_captured_dom(capture, |dom| dom.finish_custom_element_creation_failure(failure))?
                }
            }
        }
    };
    let wrapped = host.wrap_node_in_captured_dom(capture, PlatformKey { arena, slot: u64::from(element.node.0), generation: element.generation });
    drop(keep_alive);
    wrapped
}
