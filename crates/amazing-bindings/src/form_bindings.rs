//! HTML §4.10 form control WebIDL facade. Coercions precede DOM borrows.
use crate::host::{BindingHost, InterfaceBrand as B, Operation as O};
use amazing_dom::{forms::{SelectionDirection, ValidityState}, DomError, NodeIdx, NodeOrString};

pub(crate) fn handles(op: O) -> bool {
    matches!(op,
        O::FormElements | O::FormLength | O::FormReset | O::FormCheckValidity | O::FormReportValidity |
        O::FormControlForm | O::FormControlName | O::FormControlSetName |
        O::InputValue | O::InputSetValue | O::InputDefaultValue | O::InputSetDefaultValue |
        O::InputChecked | O::InputSetChecked | O::InputDefaultChecked | O::InputSetDefaultChecked |
        O::InputValueAsNumber | O::InputSetValueAsNumber | O::InputValueAsDate | O::InputSetValueAsDate |
        O::InputSelect | O::InputSetSelectionRange | O::InputSelectionStart | O::InputSetSelectionStart |
        O::InputSelectionEnd | O::InputSetSelectionEnd | O::InputSelectionDirection | O::InputSetSelectionDirection |
        O::TextAreaValue | O::TextAreaSetValue | O::TextAreaDefaultValue | O::TextAreaSetDefaultValue |
        O::SelectValue | O::SelectSetValue | O::SelectSelectedIndex | O::SelectSetSelectedIndex |
        O::SelectOptions | O::SelectLength | O::SelectMultiple | O::SelectSetMultiple | O::SelectAdd | O::SelectRemove |
        O::OptionValue | O::OptionSetValue | O::OptionSelected | O::OptionSetSelected |
        O::OptionDefaultSelected | O::OptionSetDefaultSelected | O::OptionText | O::OptionSetText |
        O::ButtonType | O::ButtonSetType |
        O::ControlWillValidate | O::ControlValidity | O::ControlValidationMessage |
        O::ControlCheckValidity | O::ControlReportValidity | O::ControlSetCustomValidity |
        O::ValidityValueMissing | O::ValidityTypeMismatch | O::ValidityPatternMismatch |
        O::ValidityTooLong | O::ValidityTooShort | O::ValidityRangeUnderflow | O::ValidityRangeOverflow |
        O::ValidityStepMismatch | O::ValidityBadInput | O::ValidityCustomError | O::ValidityValid |
        O::FormCollectionLength | O::FormCollectionItem | O::FormCollectionNamedItem |
        O::FormCollectionIndexedGetter | O::FormCollectionNamedGetter |
        O::OptionsLength | O::OptionsItem | O::OptionsNamedItem | O::OptionsIndexedGetter |
        O::OptionsIndexedSetter | O::RadioNodeListLength | O::RadioNodeListItem |
        O::RadioNodeListIndexedGetter | O::RadioNodeListValue | O::RadioNodeListSetValue)
}
pub(crate) fn handles_supported_names(op: O) -> bool {
    matches!(op, O::FormCollectionSupportedNames | O::OptionsSupportedNames)
}
fn node<H: BindingHost>(host: &mut H, value: &H::Value, tags: &[&str]) -> Result<NodeIdx, H::Error> {
    let node = host.node_brand(value).ok_or_else(|| host.type_error("Expected a form control"))?;
    host.with_dom(|dom| {
        if tags.iter().any(|tag| dom.html_tag(node, tag)) { Ok(()) } else { Err(DomError::TypeError) }
    })?;
    Ok(node)
}
fn arg<'a, H: BindingHost>(host: &mut H, args: &'a [H::Value], at: usize) -> Result<&'a H::Value, H::Error> {
    args.get(at).ok_or_else(|| host.type_error("Not enough arguments"))
}
fn string<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<Vec<u16>, H::Error> {
    let value = arg(host, args, at)?;
    host.to_dom_string(value)
}
fn number<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<f64, H::Error> {
    let value = arg(host, args, at)?;
    host.to_number(value)
}
fn boolean<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<bool, H::Error> {
    let value = arg(host, args, at)?;
    Ok(host.to_boolean(value))
}
fn attr<H: BindingHost>(host: &mut H, node: NodeIdx, name: &str) -> Result<Vec<u16>, H::Error> {
    host.with_dom(|dom| Ok(dom.attribute_utf16(node, name).unwrap_or_default()))
}
fn set_attr<H: BindingHost>(host: &mut H, node: NodeIdx, name: &str, value: &[u16]) -> Result<(), H::Error> {
    let text = String::from_utf16_lossy(value);
    host.with_dom(|dom| dom.set_attribute(node, name, &text))
}
fn bool_attr<H: BindingHost>(host: &mut H, node: NodeIdx, name: &str, value: bool) -> Result<(), H::Error> {
    host.with_dom(|dom| if value { dom.set_attribute(node, name, "") } else { dom.remove_attribute(node, name) })
}
fn wrap<H: BindingHost>(host: &mut H, result: Option<NodeIdx>) -> Result<H::Value, H::Error> {
    match result { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
}
fn key_node<H: BindingHost>(host: &mut H, node: NodeIdx, brand: B) -> Result<H::Value, H::Error> {
    host.wrap_platform_object(host.node_platform_key(node), brand)
}
fn collection_owner<H: BindingHost>(host: &mut H, receiver: &H::Value, brand: B, tag: &str) -> Result<NodeIdx, H::Error> {
    let key = host.platform_object(receiver, brand).ok_or_else(|| host.type_error("Expected a form collection"))?;
    let node = host.node_from_platform_key(key).ok_or_else(|| host.type_error("Invalid form collection"))?;
    host.with_dom(|dom| if dom.html_tag(node, tag) { Ok(()) } else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn radio_slot<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<usize, H::Error> {
    let key = host.platform_object(receiver, B::RADIO_NODE_LIST).ok_or_else(|| host.type_error("Expected RadioNodeList"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid RadioNodeList"));
    }
    Ok(key.slot as usize)
}
fn index<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<usize, H::Error> {
    let value = arg(host, args, at)?;
    let n = host.to_number(value)?;
    Ok(if n.is_finite() { n.trunc().rem_euclid(4_294_967_296.) as usize } else { 0 })
}
fn named(nodes: &[NodeIdx], dom: &amazing_dom::Dom, name: &[u16]) -> Vec<NodeIdx> {
    if name.is_empty() { return Vec::new(); }
    nodes.iter().copied().filter(|n| dom.attribute_utf16(*n, "id").as_deref() == Some(name)
        || dom.attribute_utf16(*n, "name").as_deref() == Some(name)).collect()
}
fn collection_items<H: BindingHost>(host: &mut H, op: O, receiver: &H::Value) -> Result<Vec<NodeIdx>, H::Error> {
    match op {
        O::FormCollectionLength | O::FormCollectionItem | O::FormCollectionNamedItem |
        O::FormCollectionIndexedGetter | O::FormCollectionNamedGetter |
        O::FormCollectionSupportedIndexCount | O::FormCollectionSupportedNames => {
            let owner = collection_owner(host, receiver, B::FORM_CONTROLS, "form")?;
            host.with_dom(|dom| dom.form_controls(owner))
        }
        O::OptionsLength | O::OptionsItem | O::OptionsNamedItem | O::OptionsIndexedGetter |
        O::OptionsIndexedSetter | O::OptionsSupportedIndexCount | O::OptionsSupportedNames => {
            let owner = collection_owner(host, receiver, B::OPTIONS, "select")?;
            host.with_dom(|dom| dom.select_options(owner))
        }
        _ => { let slot = radio_slot(host, receiver)?; host.with_dom(|dom| dom.radio_list_items(slot)) }
    }
}
pub(crate) fn supported_index_count<H: BindingHost>(host: &mut H, op: O, receiver: &H::Value) -> Result<u32, H::Error> {
    let items = collection_items(host, op, receiver)?;
    Ok(u32::try_from(items.len()).expect("form collection exceeds u32"))
}
pub(crate) fn supported_names<H: BindingHost>(host: &mut H, op: O, receiver: &H::Value) -> Result<Vec<Vec<u16>>, H::Error> {
    let items = collection_items(host, op, receiver)?;
    host.with_dom(|dom| {
        let mut result = Vec::new();
        for node in items {
            for value in [dom.attribute_utf16(node, "id"), dom.attribute_utf16(node, "name")] {
                if let Some(value) = value.filter(|v| !v.is_empty()) {
                    if !result.contains(&value) { result.push(value); }
                }
            }
        }
        Ok(result)
    })
}
fn validity<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<ValidityState, H::Error> {
    let (pattern, value) = host.with_dom(|dom| {
        let pattern = dom.attribute_utf16(node, "pattern");
        let value = if dom.html_tag(node, "input") { dom.input_value(node)? }
            else if dom.html_tag(node, "textarea") { dom.textarea_value(node)? } else { Vec::new() };
        Ok((pattern, value))
    })?;
    let match_result = match pattern.filter(|_| !value.is_empty()) {
        Some(pattern) => Some(host.form_pattern_matches(&pattern, &value)?),
        None => None,
    };
    host.with_dom(|dom| dom.validity(node, |_, _| match_result))
}
fn validate<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<bool, H::Error> {
    let valid = validity(host, node)?.valid();
    if !valid { crate::event_bindings::dispatch_invalid_event(host, node)?; }
    Ok(valid)
}
fn validity_flag(v: &ValidityState, op: O) -> bool {
    match op {
        O::ValidityValueMissing => v.value_missing, O::ValidityTypeMismatch => v.type_mismatch,
        O::ValidityPatternMismatch => v.pattern_mismatch, O::ValidityTooLong => v.too_long,
        O::ValidityTooShort => v.too_short, O::ValidityRangeUnderflow => v.range_underflow,
        O::ValidityRangeOverflow => v.range_overflow, O::ValidityStepMismatch => v.step_mismatch,
        O::ValidityBadInput => v.bad_input, O::ValidityCustomError => v.custom_error,
        O::ValidityValid => v.valid(), _ => unreachable!(),
    }
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if matches!(op, O::ValidityValueMissing | O::ValidityTypeMismatch | O::ValidityPatternMismatch |
        O::ValidityTooLong | O::ValidityTooShort | O::ValidityRangeUnderflow | O::ValidityRangeOverflow |
        O::ValidityStepMismatch | O::ValidityBadInput | O::ValidityCustomError | O::ValidityValid) {
        let owner = collection_owner(host, &receiver, B::VALIDITY_STATE, "input")
            .or_else(|_| collection_owner(host, &receiver, B::VALIDITY_STATE, "textarea"))
            .or_else(|_| collection_owner(host, &receiver, B::VALIDITY_STATE, "select"))
            .or_else(|_| collection_owner(host, &receiver, B::VALIDITY_STATE, "button"))?;
        let state = validity(host, owner)?;
        return Ok(host.boolean(validity_flag(&state, op)));
    }
    if matches!(op, O::FormCollectionLength | O::FormCollectionItem | O::FormCollectionNamedItem |
        O::FormCollectionIndexedGetter | O::FormCollectionNamedGetter | O::OptionsLength |
        O::OptionsItem | O::OptionsNamedItem | O::OptionsIndexedGetter | O::RadioNodeListLength |
        O::RadioNodeListItem | O::RadioNodeListIndexedGetter | O::RadioNodeListValue | O::RadioNodeListSetValue) {
        let items = collection_items(host, op, &receiver)?;
        if matches!(op, O::FormCollectionLength | O::OptionsLength | O::RadioNodeListLength) { return Ok(host.number(items.len() as f64)); }
        if op == O::RadioNodeListValue {
            let value = host.with_dom(|dom| {
                for node in &items { if dom.html_tag(*node, "input") && dom.input_type(*node)? == "radio" && dom.input_checked(*node)? {
                    return dom.input_value(*node);
                }}
                Ok(Vec::new())
            })?;
            return host.string(&value);
        }
        if op == O::RadioNodeListSetValue {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| {
                for node in &items { if dom.html_tag(*node, "input") && dom.input_type(*node)? == "radio" && dom.input_value(*node)? == value {
                    dom.set_input_checked(*node, true)?; break;
                }}
                Ok(())
            })?;
            return Ok(host.undefined());
        }
        if matches!(op, O::FormCollectionNamedItem | O::FormCollectionNamedGetter | O::OptionsNamedItem) {
            let name = string(host, args, 0)?;
            let found = host.with_dom(|dom| Ok(named(&items, dom, &name)))?;
            if found.len() > 1 && matches!(op, O::FormCollectionNamedItem | O::FormCollectionNamedGetter) {
                let form = collection_owner(host, &receiver, B::FORM_CONTROLS, "form")?;
                let slot = host.with_dom(|dom| dom.radio_list(form, name))?;
                let mut key = host.node_platform_key(form); key.slot = slot as u64;
                return host.wrap_platform_object(key, B::RADIO_NODE_LIST);
            }
            return wrap(host, found.first().copied());
        }
        let at = index(host, args, 0)?;
        return wrap(host, items.get(at).copied());
    }
    if op == O::OptionsIndexedSetter {
        let select = collection_owner(host, &receiver, B::OPTIONS, "select")?;
        let position = index(host, args, 0)?;
        let value = arg(host, args, 1)?;
        let option = if host.is_nullish(value) { None } else { Some(node(host, value, &["option"])?) };
        host.with_dom(|dom| {
            let options = dom.select_options(select)?;
            if let Some(option) = option {
                if let Some(old) = options.get(position) { dom.replace(*old, option, select)?; }
                else { dom.pre_insert(option, select, None)?; }
            } else if let Some(old) = options.get(position) { dom.pre_remove(*old, select)?; }
            Ok(())
        })?;
        return Ok(host.undefined());
    }
    if matches!(op, O::FormElements | O::FormLength | O::FormReset | O::FormCheckValidity | O::FormReportValidity) {
        let form = node(host, &receiver, &["form"])?;
        return match op {
            O::FormElements => key_node(host, form, B::FORM_CONTROLS),
            O::FormLength => { let len = host.with_dom(|dom| Ok(dom.form_controls(form)?.len()))?; Ok(host.number(len as f64)) },
            O::FormReset => { host.with_dom(|dom| dom.reset_form(form))?; Ok(host.undefined()) },
            _ => {
                let controls = host.with_dom(|dom| dom.form_controls(form))?;
                let mut valid = true;
                for control in controls { if host.with_dom(|dom| dom.will_validate(control)).unwrap_or(false) && !validate(host, control)? { valid = false; } }
                Ok(host.boolean(valid))
            }
        };
    }
    let target = node(host, &receiver, &["input", "textarea", "select", "option", "button", "fieldset", "output"])?;
    match op {
        O::FormControlForm => { let form = host.with_dom(|dom| Ok(dom.form_owner(target)))?; wrap(host, form) }
        O::FormControlName => { let value = attr(host, target, "name")?; host.string(&value) },
        O::FormControlSetName => { let value = string(host, args, 0)?; set_attr(host, target, "name", &value)?; Ok(host.undefined()) },
        O::InputValue | O::TextAreaValue | O::SelectValue | O::OptionValue => {
            let value = host.with_dom(|dom| match op {
                O::InputValue => dom.input_value(target), O::TextAreaValue => dom.textarea_value(target),
                O::SelectValue => dom.select_value(target), _ => dom.option_value(target),
            })?;
            host.string(&value)
        }
        O::InputSetValue | O::TextAreaSetValue | O::SelectSetValue | O::OptionSetValue => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| match op {
                O::InputSetValue => dom.set_input_value(target, value),
                O::TextAreaSetValue => dom.set_textarea_value(target, value),
                O::SelectSetValue => dom.set_select_value(target, &value),
                _ => dom.set_attribute(target, "value", &String::from_utf16_lossy(&value)),
            })?;
            Ok(host.undefined())
        }
        O::InputDefaultValue => { let value = attr(host, target, "value")?; host.string(&value) },
        O::InputSetDefaultValue => { let value = string(host, args, 0)?; set_attr(host, target, "value", &value)?; Ok(host.undefined()) },
        O::InputChecked | O::OptionSelected => { let value = host.with_dom(|dom| if op == O::InputChecked { dom.input_checked(target) } else { dom.option_selected(target) })?; Ok(host.boolean(value)) },
        O::InputSetChecked | O::OptionSetSelected => {
            let checked = boolean(host, args, 0)?;
            host.with_dom(|dom| if op == O::InputSetChecked { dom.set_input_checked(target, checked) } else { dom.set_option_selected(target, checked) })?;
            Ok(host.undefined())
        }
        O::InputDefaultChecked | O::OptionDefaultSelected => {
            let name = if op == O::InputDefaultChecked { "checked" } else { "selected" };
            let value = host.with_dom(|dom| Ok(dom.get_attribute_ns(target, None, name).is_some()))?;
            Ok(host.boolean(value))
        }
        O::InputSetDefaultChecked | O::OptionSetDefaultSelected => {
            let name = if op == O::InputSetDefaultChecked { "checked" } else { "selected" };
            let value = boolean(host, args, 0)?;
            bool_attr(host, target, name, value)?;
            Ok(host.undefined())
        }
        O::SelectSelectedIndex => { let value = host.with_dom(|dom| dom.selected_index(target))?; Ok(host.number(value as f64)) },
        O::SelectSetSelectedIndex => {
            let n = number(host, args, 0)?;
            let index = if n.is_finite() { n as i32 } else { 0 };
            host.with_dom(|dom| dom.set_selected_index(target, index))?;
            Ok(host.undefined())
        }
        O::SelectOptions => key_node(host, target, B::OPTIONS),
        O::SelectLength => { let len = host.with_dom(|dom| Ok(dom.select_options(target)?.len()))?; Ok(host.number(len as f64)) },
        O::SelectMultiple => { let value = host.with_dom(|dom| Ok(dom.get_attribute_ns(target, None, "multiple").is_some()))?; Ok(host.boolean(value)) },
        O::SelectSetMultiple => { let value = boolean(host, args, 0)?; bool_attr(host, target, "multiple", value)?; Ok(host.undefined()) },
        O::SelectAdd => {
            let first = arg(host, args, 0)?;
            let option = node(host, first, &["option", "optgroup"])?;
            let before = if let Some(value) = args.get(1) {
                if host.is_nullish(value) { None }
                else if let Some(node) = host.node_brand(value) { Some(node) }
                else { let n = host.to_number(value)?; host.with_dom(|dom| Ok(dom.select_options(target)?.get(n as usize).copied()))? }
            } else { None };
            host.with_dom(|dom| { dom.pre_insert(option, target, before)?; Ok(()) })?;
            Ok(host.undefined())
        }
        O::SelectRemove => {
            let index = if args.is_empty() { 0 } else { index(host, args, 0)? };
            host.with_dom(|dom| { if let Some(option) = dom.select_options(target)?.get(index) { dom.pre_remove(*option, target)?; } Ok(()) })?;
            Ok(host.undefined())
        }
        O::OptionText => { let value = host.with_dom(|dom| dom.option_value(target))?; host.string(&value) },
        O::OptionSetText => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| dom.replace_children(target, &[NodeOrString::Utf16(value)]))?;
            Ok(host.undefined())
        }
        O::ControlWillValidate => { let value = host.with_dom(|dom| dom.will_validate(target))?; Ok(host.boolean(value)) },
        O::ControlValidity => key_node(host, target, B::VALIDITY_STATE),
        O::ControlValidationMessage => { let message = host.with_dom(|dom| dom.validation_message(target))?; host.string(&message) },
        O::ControlSetCustomValidity => { let value = string(host, args, 0)?; host.with_dom(|dom| dom.set_custom_validity(target, value))?; Ok(host.undefined()) },
        O::ControlCheckValidity | O::ControlReportValidity => { let value = validate(host, target)?; Ok(host.boolean(value)) },
        O::InputSelectionStart | O::InputSelectionEnd | O::InputSelectionDirection => {
            let (start, end, direction) = host.with_dom(|dom| dom.selection(target))?;
            match op {
                O::InputSelectionStart => Ok(host.number(start as f64)),
                O::InputSelectionEnd => Ok(host.number(end as f64)),
                _ => host.string(match direction { SelectionDirection::None => &['n' as u16, 'o' as u16, 'n' as u16, 'e' as u16],
                    SelectionDirection::Forward => &['f' as u16, 'o' as u16, 'r' as u16, 'w' as u16, 'a' as u16, 'r' as u16, 'd' as u16],
                    SelectionDirection::Backward => &['b' as u16, 'a' as u16, 'c' as u16, 'k' as u16, 'w' as u16, 'a' as u16, 'r' as u16, 'd' as u16] }),
            }
        }
        O::InputSelect | O::InputSetSelectionRange | O::InputSetSelectionStart | O::InputSetSelectionEnd | O::InputSetSelectionDirection => {
            let (mut start, mut end, mut direction) = host.with_dom(|dom| dom.selection(target))?;
            match op {
                O::InputSelect => { start = 0; end = u32::MAX; },
                O::InputSetSelectionRange => {
                    start = number(host, args, 0)? as u32;
                    end = number(host, args, 1)? as u32;
                    if let Some(value) = args.get(2) { let s = host.to_dom_string(value)?; direction = direction_from(&s); }
                }
                O::InputSetSelectionStart => start = number(host, args, 0)? as u32,
                O::InputSetSelectionEnd => end = number(host, args, 0)? as u32,
                _ => direction = direction_from(&string(host, args, 0)?),
            }
            host.with_dom(|dom| dom.set_selection(target, start, end, direction))?;
            Ok(host.undefined())
        }
        O::InputValueAsNumber => {
            let value = host.with_dom(|dom| dom.input_value(target))?;
            let number = String::from_utf16_lossy(&value).parse::<f64>().unwrap_or(f64::NAN);
            Ok(host.number(number))
        }
        O::InputSetValueAsNumber => {
            let value = number(host, args, 0)?;
            if !value.is_finite() { return Err(host.type_error("Invalid valueAsNumber")); }
            host.with_dom(|dom| dom.set_input_value(target, value.to_string().encode_utf16().collect()))?;
            Ok(host.undefined())
        }
        O::InputValueAsDate | O::InputSetValueAsDate => Err(host.type_error("valueAsDate requires the date host seam")),
        O::TextAreaDefaultValue => {
            let value = host.with_dom(|dom| {
                let mut raw = Vec::new();
                for child in dom.tree_order(target).into_iter().skip(1) { raw.extend(dom.character_data_utf16(child).unwrap_or_default()); }
                Ok(raw)
            })?;
            host.string(&value)
        }
        O::TextAreaSetDefaultValue => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| dom.replace_children(target, &[NodeOrString::Utf16(value)]))?;
            Ok(host.undefined())
        }
        O::ButtonType => {
            let kind = host.with_dom(|dom| Ok(dom.get_attribute_ns(target, None, "type").unwrap_or("submit").to_ascii_lowercase()))?;
            let value: Vec<u16> = if ["submit", "reset", "button"].contains(&kind.as_str()) { kind } else { "submit".into() }.encode_utf16().collect();
            host.string(&value)
        }
        O::ButtonSetType => { let value = string(host, args, 0)?; set_attr(host, target, "type", &value)?; Ok(host.undefined()) },
        _ => Err(host.type_error("Unsupported form operation")),
    }
}
fn direction_from(value: &[u16]) -> SelectionDirection {
    if value == "forward".encode_utf16().collect::<Vec<_>>() { SelectionDirection::Forward }
    else if value == "backward".encode_utf16().collect::<Vec<_>>() { SelectionDirection::Backward }
    else { SelectionDirection::None }
}
