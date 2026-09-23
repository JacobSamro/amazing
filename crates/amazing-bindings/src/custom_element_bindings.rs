//! CustomElementRegistry and reaction delivery. User code always runs outside
//! the DOM borrow; the host owns constructor, callback, and promise values.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{custom_elements::{CustomElementDefinition, CustomElementReaction,
    CustomElementState, LifecycleCallbacks}, Dom, DomError, NodeIdx};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::CustomElementRegistryDefine | O::CustomElementRegistryGet
        | O::CustomElementRegistryGetName | O::CustomElementRegistryWhenDefined
        | O::CustomElementRegistryUpgrade)
}

pub fn registry_for_document<H: BindingHost>(host: &mut H, document: NodeIdx)
    -> Result<H::Value, H::Error> {
    host.wrap_platform_object(host.node_platform_key(document), InterfaceBrand::CUSTOM_ELEMENT_REGISTRY)
}
fn registry_document<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    receiver_to_document(host, receiver).ok_or_else(|| host.type_error("Expected a CustomElementRegistry"))
}
fn receiver_to_document<H: BindingHost>(host: &H, receiver: &H::Value) -> Option<NodeIdx> {
    host.platform_object(receiver, InterfaceBrand::CUSTOM_ELEMENT_REGISTRY)
        .and_then(|key| host.node_from_platform_key(key))
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
    observed: Vec<String>, disable_internals: bool, disable_shadow: bool,
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
            host.sequence_dom_strings(&values)?.into_iter()
                .map(|units| String::from_utf16_lossy(&units)).collect()
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
fn define<H: BindingHost>(host: &mut H, document: NodeIdx, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    let name = name(host, args.first())?;
    let constructor = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
    if !host.is_constructor(constructor)? { return Err(host.type_error("Expected a constructor")); }
    if !Dom::valid_custom_element_name(&name) {
        return host.with_dom::<H::Value>(|_| Err(DomError::SyntaxError));
    }
    let definitions = host.with_dom(|dom| Ok(dom.custom_element_definitions(document)
        .into_iter().map(|(_, def)| (def.name.clone(), def.constructor)).collect::<Vec<_>>()))?;
    if definitions.iter().any(|(existing, _)| existing == &name) {
        return host.with_dom::<H::Value>(|_| Err(DomError::NotSupportedError));
    }
    for (_, root) in &definitions {
        if let Some(existing) = host.root_value(*root) {
            if host.same_value(&existing, constructor) {
                return host.with_dom::<H::Value>(|_| Err(DomError::NotSupportedError));
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
            return host.with_dom::<H::Value>(|_| Err(DomError::NotSupportedError));
        }
        extends
    } else { name.clone() };
    host.with_dom(|dom| dom.custom_element_begin_definition(document))?;
    let extracted = extract(host, constructor);
    host.with_dom(|dom| { dom.custom_element_end_definition(document); Ok(()) })?;
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
    definition.observed_attributes = extracted.observed;
    definition.disable_internals = extracted.disable_internals;
    definition.disable_shadow = extracted.disable_shadow;
    definition.form_associated = extracted.form_associated;
    if let Err(error) = host.with_dom(|dom| dom.define_custom_element(definition)) {
        host.release_value(constructor_root);
        for root in roots { host.release_value(root); }
        return Err(error);
    }
    let promises = host.with_dom(|dom| Ok(dom.take_custom_element_when_defined(document, &name)))?;
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
    match op {
        O::CustomElementRegistryDefine => define(host, document, args),
        O::CustomElementRegistryGet => {
            let name = name(host, args.first())?;
            let root = host.with_dom(|dom| Ok(dom.custom_element_definition_by_name(document, &name)
                .map(|(_, definition)| definition.constructor)))?;
            Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.undefined()))
        }
        O::CustomElementRegistryGetName => {
            let constructor = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_constructor(constructor)? { return Err(host.type_error("Expected a constructor")); }
            let definitions = host.with_dom(|dom| Ok(dom.custom_element_definitions(document).into_iter()
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
            if let Some(root) = host.with_dom(|dom| Ok(dom.custom_element_pending_promise(document, &name)))? {
                if let Some(promise) = host.root_value(root) { return Ok(promise); }
            }
            let promise = host.new_promise()?;
            if !Dom::valid_custom_element_name(&name) {
                let reason = host.dom_exception_value(DomError::SyntaxError)?;
                host.reject_promise(&promise, reason)?;
                return Ok(promise);
            }
            let constructor = host.with_dom(|dom| Ok(dom.custom_element_definition_by_name(document, &name)
                .map(|(_, definition)| definition.constructor)))?;
            if let Some(root) = constructor {
                if let Some(value) = host.root_value(root) { host.resolve_promise(&promise, value)?; }
            } else {
                let root = host.retain_value(promise.clone());
                host.with_dom(|dom| { dom.custom_element_when_defined(document, name, root); Ok(()) })?;
            }
            Ok(promise)
        }
        O::CustomElementRegistryUpgrade => {
            let root = args.first().and_then(|value| host.node_brand(value))
                .ok_or_else(|| host.type_error("Expected a Node"))?;
            host.with_dom(|dom| dom.queue_custom_element_upgrades_for_tree(root))?;
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
    loop {
        let reactions = host.with_dom(|dom| Ok(dom.take_custom_element_reactions()))?;
        if reactions.is_empty() { return Ok(()); }
        for reaction in reactions {
            let result = match reaction {
                CustomElementReaction::Upgrade { element, definition } => {
                    let state = host.with_dom(|dom| Ok(dom.custom_element_state(element)))?;
                    if state != CustomElementState::Undefined { continue; }
                    let constructor_root = match host.with_dom(|dom| dom.begin_custom_element_upgrade(element, definition)) {
                        Ok(root) => root,
                        Err(error) => { let _ = host.report_custom_element_exception(error); continue; }
                    };
                    let Some(constructor) = host.root_value(constructor_root) else { continue };
                    let constructed = host.construct_custom_element(&constructor, definition, element);
                    let same = match &constructed {
                        Ok(value) => { let expected = host.wrap_node(element)?; host.same_value(value, &expected) },
                        Err(_) => false,
                    };
                    let finish = host.with_dom(|dom| dom.finish_custom_element_upgrade(element, definition, same));
                    if let Err(error) = constructed { let _ = host.report_custom_element_exception(error); }
                    finish
                }
                CustomElementReaction::Connected { element, callback }
                | CustomElementReaction::Disconnected { element, callback }
                | CustomElementReaction::ConnectedMove { element, callback } =>
                    callback_value(host, callback, element, &[]),
                CustomElementReaction::Adopted { element, callback, old_document, new_document } => {
                    let old = host.wrap_node(old_document)?;
                    let new = host.wrap_node(new_document)?;
                    callback_value(host, callback, element, &[old, new])
                }
                CustomElementReaction::AttributeChanged { element, callback, name, old_value, new_value, namespace } => {
                    let name = host.string(&name.encode_utf16().collect::<Vec<_>>())?;
                    let old = match old_value { Some(value) => host.string(&value.encode_utf16().collect::<Vec<_>>())?,
                        None => host.null() };
                    let new = match new_value { Some(value) => host.string(&value.encode_utf16().collect::<Vec<_>>())?,
                        None => host.null() };
                    let namespace = match namespace { Some(value) => host.string(&value.encode_utf16().collect::<Vec<_>>())?,
                        None => host.null() };
                    callback_value(host, callback, element, &[name, old, new, namespace])
                }
            };
            if let Err(error) = result { let _ = host.report_custom_element_exception(error); }
        }
    }
}
