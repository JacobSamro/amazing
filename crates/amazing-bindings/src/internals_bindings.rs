//! `HTMLElement.attachInternals()`, `ElementInternals` and `CustomStateSet`
//! (HTML §4.13.7). Coercions precede DOM borrows; user code never runs inside
//! `with_dom`.
use crate::host::{BindingHost, InterfaceBrand as B, Operation as O};
use amazing_dom::{collection::CollectionFilter, element_internals::SubmissionValue,
    forms::ValidityState, DomError, NodeIdx, NodeKind};
use std::sync::Arc;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::HTMLElementAttachInternals | O::FieldsetElements | O::InternalsShadowRoot | O::InternalsForm
        | O::InternalsSetFormValue | O::InternalsSetValidity | O::InternalsWillValidate
        | O::InternalsValidity | O::InternalsValidationMessage | O::InternalsCheckValidity
        | O::InternalsReportValidity | O::InternalsLabels | O::InternalsStates | O::InternalsBehaviors
        | O::InternalsAriaGet(_) | O::InternalsAriaSet(_) | O::InternalsAriaElementGet(_) | O::InternalsAriaElementSet(_) | O::StateSetSize | O::StateSetAdd
        | O::StateSetDelete | O::StateSetHas | O::StateSetClear | O::StateSetForEach
        | O::StateSetValues | O::StateSetEntries)
}

fn usv(units: Vec<u16>) -> Vec<u16> {
    char::decode_utf16(units).flat_map(|unit| {
        let c = unit.unwrap_or('\u{FFFD}');
        let mut buffer = [0u16; 2];
        c.encode_utf16(&mut buffer).to_vec()
    }).collect()
}
fn submission<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Option<SubmissionValue>, H::Error> {
    if host.is_nullish(value) { return Ok(None); }
    if host.platform_object(value, B::FILE).is_some() {
        let file = host.file_record(value)?;
        return Ok(Some(SubmissionValue::Opaque(Arc::new(file))));
    }
    if host.platform_object(value, B::FORM_DATA).is_some() {
        let data = host.form_data_record(value)?;
        return Ok(Some(SubmissionValue::Opaque(Arc::new(data))));
    }
    let units = host.to_dom_string(value)?;
    Ok(Some(SubmissionValue::Text(usv(units))))
}
fn require_face<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        if dom.is_form_associated_custom_element(node) { Ok(()) } else { Err(DomError::NotSupportedError) }
    })
}
fn flag<H: BindingHost>(host: &mut H, flags: &H::Value, name: &str) -> Result<bool, H::Error> {
    let value = host.get_property(flags, name)?;
    Ok(host.to_boolean(&value))
}
fn element_arg<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<NodeIdx, H::Error> {
    let node = host.node_brand(value).ok_or_else(|| host.type_error("Expected an Element"))?;
    host.with_dom(|dom| {
        if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) } else { Err(DomError::TypeError) }
    })?;
    Ok(node)
}
fn string_array<H: BindingHost>(host: &mut H, values: Vec<Vec<u16>>) -> Result<H::Value, H::Error> {
    let mut items = Vec::with_capacity(values.len());
    for value in values { items.push(host.string(&value)?); }
    host.create_array(items)
}

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    if op == O::HTMLElementAttachInternals {
        let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Illegal invocation"))?;
        // ElementInternalsInit: `behaviors` takes ElementInternalsBehavior instances;
        // this engine defines no behavior interface, so any listed item is invalid.
        if let Some(init) = args.first().filter(|value| !host.is_nullish(value)) {
            if !host.is_object_value(init)? { return Err(host.type_error("ElementInternalsInit is not an object")); }
            let behaviors = host.get_property(init, "behaviors")?;
            if !host.is_undefined(&behaviors) {
                if !host.sequence_values(&behaviors)?.is_empty() {
                    return Err(host.type_error("Not an ElementInternalsBehavior"));
                }
            }
        }
        host.with_dom(|dom| {
            if !matches!(dom.get(node).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
            dom.attach_internals(node)
        })?;
        let key = host.node_platform_key(node);
        return host.wrap_platform_object(key, B::ELEMENT_INTERNALS);
    }
    if op == O::FieldsetElements {
        let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Illegal invocation"))?;
        let slot = host.with_dom(|dom| {
            if !dom.html_tag(node, "fieldset") { return Err(DomError::TypeError); }
            dom.collection(node, CollectionFilter::FieldsetElements)
        })?;
        let mut key = host.node_platform_key(node);
        key.slot = slot as u64;
        return host.wrap_platform_object(key, B::COLLECTION);
    }
    let state_set = matches!(op, O::StateSetSize | O::StateSetAdd | O::StateSetDelete | O::StateSetHas
        | O::StateSetClear | O::StateSetForEach | O::StateSetValues | O::StateSetEntries);
    let brand = if state_set { B::CUSTOM_STATE_SET } else { B::ELEMENT_INTERNALS };
    let key = host.platform_object(&receiver, brand).ok_or_else(|| host.type_error("Illegal invocation"))?;
    let node = host.node_from_platform_key(key).ok_or_else(|| host.type_error("Invalid ElementInternals"))?;
    match op {
        O::InternalsShadowRoot => {
            let root = host.with_dom(|dom| Ok(dom.internals_shadow_root(node)))?;
            match root { Some(root) => host.wrap_node(root), None => Ok(host.null()) }
        }
        O::InternalsForm => {
            require_face(host, node)?;
            let form = host.with_dom(|dom| Ok(dom.form_owner(node)))?;
            match form { Some(form) => host.wrap_node(form), None => Ok(host.null()) }
        }
        O::InternalsSetFormValue => {
            let value = match args.first() {
                Some(value) => submission(host, value)?,
                None => return Err(host.type_error("setFormValue requires a value")),
            };
            let state = match args.get(1) {
                Some(state) if !host.is_undefined(state) => submission(host, state)?,
                _ => value.clone(),
            };
            host.with_dom(|dom| dom.internals_set_form_value(node, value, state))?;
            Ok(host.undefined())
        }
        O::InternalsSetValidity => {
            let flags_value = args.first().ok_or_else(|| host.type_error("setValidity requires flags"))?;
            let mut flags = ValidityState::default();
            if !host.is_nullish(flags_value) {
                if !host.is_object_value(flags_value)? { return Err(host.type_error("flags is not an object")); }
                flags.value_missing = flag(host, flags_value, "valueMissing")?;
                flags.type_mismatch = flag(host, flags_value, "typeMismatch")?;
                flags.pattern_mismatch = flag(host, flags_value, "patternMismatch")?;
                flags.too_long = flag(host, flags_value, "tooLong")?;
                flags.too_short = flag(host, flags_value, "tooShort")?;
                flags.range_underflow = flag(host, flags_value, "rangeUnderflow")?;
                flags.range_overflow = flag(host, flags_value, "rangeOverflow")?;
                flags.step_mismatch = flag(host, flags_value, "stepMismatch")?;
                flags.bad_input = flag(host, flags_value, "badInput")?;
                flags.custom_error = flag(host, flags_value, "customError")?;
            }
            let message = match args.get(1) {
                Some(value) if !host.is_undefined(value) => host.to_dom_string(value)?,
                _ => Vec::new(),
            };
            let anchor = match args.get(2) {
                Some(value) if !host.is_nullish(value) => {
                    let anchor = host.node_brand(value).ok_or_else(|| host.type_error("anchor is not an Element"))?;
                    host.with_dom(|dom| {
                        if matches!(dom.get(anchor).kind, NodeKind::Element { namespace: amazing_dom::Namespace::Html, .. }) { Ok(()) }
                        else { Err(DomError::TypeError) }
                    })?;
                    Some(anchor)
                }
                _ => None,
            };
            require_face(host, node)?;
            if !flags.valid() && message.is_empty() {
                return Err(host.type_error("setValidity requires a message when any flag is set"));
            }
            host.with_dom(|dom| dom.internals_set_validity(node, flags, message, anchor))?;
            Ok(host.undefined())
        }
        O::InternalsWillValidate => {
            require_face(host, node)?;
            let value = host.with_dom(|dom| dom.will_validate(node))?;
            Ok(host.boolean(value))
        }
        O::InternalsValidity => {
            require_face(host, node)?;
            host.wrap_platform_object(host.node_platform_key(node), B::VALIDITY_STATE)
        }
        O::InternalsValidationMessage => {
            require_face(host, node)?;
            let message = host.with_dom(|dom| {
                if dom.will_validate(node)? { Ok(dom.validation_message(node)?) } else { Ok(Vec::new()) }
            })?;
            host.string(&message)
        }
        O::InternalsCheckValidity | O::InternalsReportValidity => {
            require_face(host, node)?;
            let valid = crate::form_bindings::validate(host, node)?;
            Ok(host.boolean(valid))
        }
        O::InternalsLabels => {
            require_face(host, node)?;
            let (root, slot) = host.with_dom(|dom| {
                let root = dom.root(node);
                let slot = dom.collection(root, CollectionFilter::Labels(node))?;
                Ok((root, slot))
            })?;
            let mut key = host.node_platform_key(root);
            key.slot = slot as u64;
            host.wrap_platform_object(key, B::FILTERED_NODE_LIST)
        }
        O::InternalsBehaviors => host.create_frozen_array(Vec::new()),
        O::InternalsStates => {
            let key = host.node_platform_key(node);
            host.wrap_platform_object(key, B::CUSTOM_STATE_SET)
        }
        O::InternalsAriaGet(name) => {
            let value = host.with_dom(|dom| Ok(dom.internals_aria_get(node, name)))?;
            match value { Some(units) => host.string(&units), None => Ok(host.null()) }
        }
        O::InternalsAriaSet(name) => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let units = if host.is_nullish(value) { None } else { Some(host.to_dom_string(value)?) };
            host.with_dom(|dom| dom.internals_aria_set(node, name, units))?;
            Ok(host.undefined())
        }
        O::InternalsAriaElementGet(name) => {
            let single = name == "ariaActiveDescendantElement";
            let stored = host.with_dom(|dom| Ok(dom.internals_aria_elements(node, name)))?;
            let Some((nodes, cache)) = stored else { return Ok(host.null()); };
            if single {
                return match nodes.first() { Some(first) => host.wrap_node(*first), None => Ok(host.null()) };
            }
            if let Some(cache) = cache { if let Some(array) = host.root_value(cache) { return Ok(array); } }
            let mut values = Vec::with_capacity(nodes.len());
            for item in nodes { values.push(host.wrap_node(item)?); }
            let array = host.create_frozen_array(values)?;
            let root = host.retain_value(array.clone());
            host.with_dom(|dom| dom.internals_aria_elements_cache(node, name, root))?;
            Ok(array)
        }
        O::InternalsAriaElementSet(name) => {
            let single = name == "ariaActiveDescendantElement";
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let nodes = if host.is_nullish(value) { None } else if single {
                Some(vec![element_arg(host, value)?])
            } else {
                let items = host.sequence_values(value)?;
                let mut nodes = Vec::with_capacity(items.len());
                for item in &items { nodes.push(element_arg(host, item)?); }
                Some(nodes)
            };
            let old = host.with_dom(|dom| dom.internals_aria_elements_set(node, name, nodes))?;
            if let Some(old) = old { host.release_value(old); }
            Ok(host.undefined())
        }
        O::StateSetSize => {
            let count = host.with_dom(|dom| Ok(dom.internals_states(node).len()))?;
            Ok(host.number(count as f64))
        }
        O::StateSetAdd => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let name = host.to_dom_string(value)?;
            host.with_dom(|dom| dom.internals_state_add(node, name))?;
            Ok(receiver)
        }
        O::StateSetDelete => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let name = host.to_dom_string(value)?;
            let removed = host.with_dom(|dom| dom.internals_state_delete(node, &name))?;
            Ok(host.boolean(removed))
        }
        O::StateSetHas => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let name = host.to_dom_string(value)?;
            let found = host.with_dom(|dom| Ok(dom.internals_states(node).contains(&name)))?;
            Ok(host.boolean(found))
        }
        O::StateSetClear => {
            host.with_dom(|dom| dom.internals_state_clear(node))?;
            Ok(host.undefined())
        }
        O::StateSetForEach => {
            let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(callback) { return Err(host.type_error("forEach callback is not callable")); }
            let this_arg = args.get(1).cloned().unwrap_or_else(|| host.undefined());
            let mut index = 0;
            loop {
                let next = host.with_dom(|dom| Ok(dom.internals_states(node).get(index).cloned()))?;
                let Some(name) = next else { break };
                let value = host.string(&name)?;
                host.call(callback, &this_arg, &[value.clone(), value, receiver.clone()])?;
                index += 1;
            }
            Ok(host.undefined())
        }
        O::StateSetValues | O::StateSetEntries => {
            let names = host.with_dom(|dom| Ok(dom.internals_states(node)))?;
            let array = if op == O::StateSetValues { string_array(host, names)? } else {
                let mut pairs = Vec::with_capacity(names.len());
                for name in names {
                    let value = host.string(&name)?;
                    pairs.push(host.create_array(vec![value.clone(), value])?);
                }
                host.create_array(pairs)?
            };
            let values = host.get_property(&array, "values")?;
            host.call(&values, &array, &[])
        }
        _ => unreachable!("routed by handles()"),
    }
}
