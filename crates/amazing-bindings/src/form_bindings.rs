//! HTML §4.10 form control WebIDL facade. Coercions precede DOM borrows.
use crate::host::{BindingHost, FormNavigationRequest, InterfaceBrand as B, Operation as O};
use amazing_fetch::api::{File, FormData, FormValue};
use amazing_dom::{
    forms::{SelectionDirection, ValidityState},
    DomError, NodeIdx, NodeOrString,
};

pub(crate) fn handles(op: O) -> bool {
    matches!(
        op,
        O::FormElements
            | O::FormDataFormConstructor
            | O::FormAutocompleteGet
            | O::FormAutocompleteSet
            | O::FormLength
            | O::FormReset
            | O::FormSubmit
            | O::FormRequestSubmit
            | O::FormCheckValidity
            | O::FormReportValidity
            | O::FormControlForm
            | O::FormControlName
            | O::FormControlSetName
            | O::LabelControl
            | O::LabelForm
            | O::ProgressValue
            | O::ProgressSetValue
            | O::ProgressMax
            | O::ProgressSetMax
            | O::ProgressPosition
            | O::MeterValue
            | O::MeterSetValue
            | O::MeterMin
            | O::MeterSetMin
            | O::MeterMax
            | O::MeterSetMax
            | O::MeterLow
            | O::MeterSetLow
            | O::MeterHigh
            | O::MeterSetHigh
            | O::MeterOptimum
            | O::MeterSetOptimum
            | O::InputValue
            | O::InputSetValue
            | O::InputDefaultValue
            | O::InputSetDefaultValue
            | O::InputChecked
            | O::InputSetChecked
            | O::InputDefaultChecked
            | O::InputSetDefaultChecked
            | O::InputValueAsNumber
            | O::InputSetValueAsNumber
            | O::InputValueAsDate
            | O::InputSetValueAsDate
            | O::InputStepUp
            | O::InputStepDown
            | O::InputSelect
            | O::InputSetSelectionRange
            | O::InputSetRangeText
            | O::InputSelectionStart
            | O::InputSetSelectionStart
            | O::InputSelectionEnd
            | O::InputSetSelectionEnd
            | O::InputSelectionDirection
            | O::InputSetSelectionDirection
            | O::TextAreaValue
            | O::TextAreaSetValue
            | O::TextAreaDefaultValue
            | O::TextAreaSetDefaultValue
            | O::SelectValue
            | O::SelectSetValue
            | O::SelectSelectedIndex
            | O::SelectSetSelectedIndex
            | O::SelectOptions
            | O::SelectLength
            | O::SelectMultiple
            | O::SelectSetMultiple
            | O::SelectAdd
            | O::SelectRemove
            | O::OptionValue
            | O::OptionSetValue
            | O::OptionSelected
            | O::OptionSetSelected
            | O::OptionDefaultSelected
            | O::OptionSetDefaultSelected
            | O::OptionText
            | O::OptionSetText
            | O::OptionIndex
            | O::OutputValue
            | O::OutputSetValue
            | O::OutputDefaultValue
            | O::OutputSetDefaultValue
            | O::ButtonType
            | O::ButtonSetType
            | O::ControlWillValidate
            | O::ControlValidity
            | O::ControlValidationMessage
            | O::ControlCheckValidity
            | O::ControlReportValidity
            | O::ControlSetCustomValidity
            | O::ValidityValueMissing
            | O::ValidityTypeMismatch
            | O::ValidityPatternMismatch
            | O::ValidityTooLong
            | O::ValidityTooShort
            | O::ValidityRangeUnderflow
            | O::ValidityRangeOverflow
            | O::ValidityStepMismatch
            | O::ValidityBadInput
            | O::ValidityCustomError
            | O::ValidityValid
            | O::FormCollectionLength
            | O::FormCollectionItem
            | O::FormCollectionNamedItem
            | O::FormCollectionIndexedGetter
            | O::FormCollectionNamedGetter
            | O::OptionsLength
            | O::OptionsSetLength
            | O::OptionsSelectedIndex
            | O::OptionsSetSelectedIndex
            | O::OptionsItem
            | O::OptionsNamedItem
            | O::OptionsIndexedGetter
            | O::OptionsNamedGetter
            | O::OptionsIndexedSetter
            | O::RadioNodeListLength
            | O::RadioNodeListItem
            | O::RadioNodeListIndexedGetter
            | O::RadioNodeListValue
            | O::RadioNodeListSetValue
    )
}
pub(crate) fn handles_supported_names(op: O) -> bool {
    matches!(
        op,
        O::FormCollectionSupportedNames | O::OptionsSupportedNames
    )
}
fn node<H: BindingHost>(
    host: &mut H,
    value: &H::Value,
    tags: &[&str],
) -> Result<NodeIdx, H::Error> {
    let node = host
        .node_brand(value)
        .ok_or_else(|| host.type_error("Expected a form control"))?;
    host.with_dom(|dom| {
        if tags.iter().any(|tag| dom.html_tag(node, tag)) {
            Ok(())
        } else {
            Err(DomError::TypeError)
        }
    })?;
    Ok(node)
}
fn arg<'a, H: BindingHost>(
    host: &mut H,
    args: &'a [H::Value],
    at: usize,
) -> Result<&'a H::Value, H::Error> {
    args.get(at)
        .ok_or_else(|| host.type_error("Not enough arguments"))
}
fn string<H: BindingHost>(
    host: &mut H,
    args: &[H::Value],
    at: usize,
) -> Result<Vec<u16>, H::Error> {
    let value = arg(host, args, at)?;
    host.to_dom_string(value)
}
fn number<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<f64, H::Error> {
    let value = arg(host, args, at)?;
    host.to_number(value)
}
/// WebIDL `double` (not `unrestricted double`) rejects non-finite values.
fn finite_number<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<f64, H::Error> {
    let value = number(host, args, at)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(host.type_error("The provided double value is non-finite"))
    }
}
fn boolean<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<bool, H::Error> {
    let value = arg(host, args, at)?;
    Ok(host.to_boolean(value))
}
fn attr<H: BindingHost>(host: &mut H, node: NodeIdx, name: &str) -> Result<Vec<u16>, H::Error> {
    host.with_dom(|dom| Ok(dom.attribute_utf16(node, name).unwrap_or_default()))
}
fn set_attr<H: BindingHost>(
    host: &mut H,
    node: NodeIdx,
    name: &str,
    value: &[u16],
) -> Result<(), H::Error> {
    host.with_dom(|dom| crate::html_reflection::set(dom, node, name, value.to_vec()))
}
fn parsed_attr_number<H: BindingHost>(
    host: &mut H,
    node: NodeIdx,
    name: &str,
) -> Result<Option<f64>, H::Error> {
    let raw = attr(host, node, name)?;
    Ok(String::from_utf16_lossy(&raw)
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite()))
}
fn set_attr_number<H: BindingHost>(
    host: &mut H,
    node: NodeIdx,
    name: &str,
    value: f64,
) -> Result<(), H::Error> {
    let text = if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    };
    set_attr(host, node, name, &text.encode_utf16().collect::<Vec<_>>())
}
fn progress_max<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    Ok(match parsed_attr_number(host, node, "max")? {
        Some(value) if value > 0. => value,
        _ => 1.,
    })
}
fn progress_value<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let max = progress_max(host, node)?;
    let value = parsed_attr_number(host, node, "value")?
        .filter(|value| *value >= 0.)
        .unwrap_or(0.);
    Ok(value.min(max))
}
fn meter_min<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    Ok(parsed_attr_number(host, node, "min")?.unwrap_or(0.))
}
fn meter_max<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let min = meter_min(host, node)?;
    let max = parsed_attr_number(host, node, "max")?.unwrap_or(1.);
    Ok(if max < min { min } else { max })
}
fn meter_value<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let min = meter_min(host, node)?;
    let max = meter_max(host, node)?;
    let value = parsed_attr_number(host, node, "value")?.unwrap_or(0.);
    Ok(value.max(min).min(max))
}
fn meter_low<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let min = meter_min(host, node)?;
    let max = meter_max(host, node)?;
    let low = parsed_attr_number(host, node, "low")?.unwrap_or(min);
    Ok(low.max(min).min(max))
}
fn meter_high<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let min = meter_min(host, node)?;
    let max = meter_max(host, node)?;
    let low = meter_low(host, node)?;
    let high = parsed_attr_number(host, node, "high")?.unwrap_or(max);
    Ok(high.max(min).min(max).max(low))
}
fn meter_optimum<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<f64, H::Error> {
    let min = meter_min(host, node)?;
    let max = meter_max(host, node)?;
    let optimum = parsed_attr_number(host, node, "optimum")?.unwrap_or((min + max) / 2.);
    Ok(optimum.max(min).min(max))
}
fn bool_attr<H: BindingHost>(
    host: &mut H,
    node: NodeIdx,
    name: &str,
    value: bool,
) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        if value {
            dom.set_attribute(node, name, "")
        } else {
            dom.remove_attribute(node, name)
        }
    })
}
fn wrap<H: BindingHost>(host: &mut H, result: Option<NodeIdx>) -> Result<H::Value, H::Error> {
    match result {
        Some(node) => host.wrap_node(node),
        None => Ok(host.null()),
    }
}
fn key_node<H: BindingHost>(host: &mut H, node: NodeIdx, brand: B) -> Result<H::Value, H::Error> {
    host.wrap_platform_object(host.node_platform_key(node), brand)
}
fn collection_owner<H: BindingHost>(
    host: &mut H,
    receiver: &H::Value,
    brand: B,
    tag: &str,
) -> Result<NodeIdx, H::Error> {
    let key = host
        .platform_object(receiver, brand)
        .ok_or_else(|| host.type_error("Expected a form collection"))?;
    let node = host
        .node_from_platform_key(key)
        .ok_or_else(|| host.type_error("Invalid form collection"))?;
    host.with_dom(|dom| {
        if dom.html_tag(node, tag) {
            Ok(())
        } else {
            Err(DomError::TypeError)
        }
    })?;
    Ok(node)
}
fn radio_slot<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<usize, H::Error> {
    let key = host
        .platform_object(receiver, B::RADIO_NODE_LIST)
        .ok_or_else(|| host.type_error("Expected RadioNodeList"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid RadioNodeList"));
    }
    Ok(key.slot as usize)
}
fn index<H: BindingHost>(host: &mut H, args: &[H::Value], at: usize) -> Result<usize, H::Error> {
    Ok(unsigned(number(host, args, at)?) as usize)
}
fn unsigned(n: f64) -> u32 {
    if n.is_finite() {
        n.trunc().rem_euclid(4_294_967_296.) as u32
    } else {
        0
    }
}
fn signed(n: f64) -> i32 {
    unsigned(n) as i32
}
fn named(nodes: &[NodeIdx], dom: &amazing_dom::Dom, name: &[u16]) -> Vec<NodeIdx> {
    if name.is_empty() {
        return Vec::new();
    }
    nodes
        .iter()
        .copied()
        .filter(|n| {
            dom.attribute_utf16(*n, "id").as_deref() == Some(name)
                || dom.attribute_utf16(*n, "name").as_deref() == Some(name)
        })
        .collect()
}
fn collection_items<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: &H::Value,
) -> Result<Vec<NodeIdx>, H::Error> {
    match op {
        O::FormCollectionLength
        | O::FormCollectionItem
        | O::FormCollectionNamedItem
        | O::FormCollectionIndexedGetter
        | O::FormCollectionNamedGetter
        | O::FormCollectionSupportedIndexCount
        | O::FormCollectionSupportedNames => {
            let owner = collection_owner(host, receiver, B::FORM_CONTROLS, "form")?;
            host.with_dom(|dom| dom.form_controls(owner))
        }
        O::OptionsLength
        | O::OptionsItem
        | O::OptionsNamedItem
        | O::OptionsIndexedGetter
        | O::OptionsNamedGetter
        | O::OptionsIndexedSetter
        | O::OptionsSupportedIndexCount
        | O::OptionsSupportedNames => {
            let owner = collection_owner(host, receiver, B::OPTIONS, "select")?;
            host.with_dom(|dom| dom.select_options(owner))
        }
        _ => {
            let slot = radio_slot(host, receiver)?;
            host.with_dom(|dom| dom.radio_list_items(slot))
        }
    }
}
pub(crate) fn supported_index_count<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: &H::Value,
) -> Result<u32, H::Error> {
    let items = collection_items(host, op, receiver)?;
    Ok(u32::try_from(items.len()).expect("form collection exceeds u32"))
}
pub(crate) fn supported_names<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: &H::Value,
) -> Result<Vec<Vec<u16>>, H::Error> {
    let items = collection_items(host, op, receiver)?;
    host.with_dom(|dom| {
        let mut result = Vec::new();
        for node in items {
            for value in [
                dom.attribute_utf16(node, "id"),
                dom.attribute_utf16(node, "name"),
            ] {
                if let Some(value) = value.filter(|v| !v.is_empty()) {
                    if !result.contains(&value) {
                        result.push(value);
                    }
                }
            }
        }
        Ok(result)
    })
}
fn validity<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<ValidityState, H::Error> {
    let (pattern, value, multiple_email) = host.with_dom(|dom| {
        let is_input = dom.html_tag(node, "input");
        let input_type = if is_input { Some(dom.input_type(node)?) } else { None };
        let pattern = if input_type.as_deref().is_some_and(|kind| {
            matches!(kind, "text" | "search" | "tel" | "url" | "email" | "password")
        }) {
            dom.attribute_utf16(node, "pattern")
        } else {
            None
        };
        let value = if is_input {
            dom.input_value(node)?
        } else if dom.html_tag(node, "textarea") {
            dom.textarea_value(node)?
        } else {
            Vec::new()
        };
        let multiple_email = input_type.as_deref() == Some("email")
            && dom.attribute_utf16(node, "multiple").is_some();
        Ok((pattern, value, multiple_email))
    })?;
    // A pattern on a multiple email control is matched against each
    // comma-separated address after stripping surrounding whitespace; any
    // mismatching address makes the whole control mismatch.
    let match_result = match pattern.filter(|_| !value.is_empty()) {
        Some(pattern) => {
            let mut mismatched = false;
            if multiple_email {
                let text = String::from_utf16_lossy(&value);
                for address in text.split(',').map(|part| part.trim_matches(|c: char| c.is_ascii_whitespace())) {
                    let units: Vec<u16> = address.encode_utf16().collect();
                    if host.form_pattern_matches(&pattern, &units)? == Some(false) {
                        mismatched = true;
                        break;
                    }
                }
            } else {
                mismatched = host.form_pattern_matches(&pattern, &value)? == Some(false);
            }
            Some(!mismatched)
        }
        None => None,
    };
    host.with_dom(|dom| dom.validity(node, |_, _| match_result))
}
fn validate<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<bool, H::Error> {
    // A control that is not a candidate for constraint validation satisfies its
    // constraints for the purposes of checkValidity/reportValidity and does not
    // fire an invalid event, even though validity flags may still be set.
    if !host.with_dom(|dom| dom.will_validate(node))? {
        return Ok(true);
    }
    let valid = validity(host, node)?.valid();
    if !valid {
        crate::event_bindings::dispatch_invalid_event(host, node)?;
    }
    Ok(valid)
}
fn validity_flag(v: &ValidityState, op: O) -> bool {
    match op {
        O::ValidityValueMissing => v.value_missing,
        O::ValidityTypeMismatch => v.type_mismatch,
        O::ValidityPatternMismatch => v.pattern_mismatch,
        O::ValidityTooLong => v.too_long,
        O::ValidityTooShort => v.too_short,
        O::ValidityRangeUnderflow => v.range_underflow,
        O::ValidityRangeOverflow => v.range_overflow,
        O::ValidityStepMismatch => v.step_mismatch,
        O::ValidityBadInput => v.bad_input,
        O::ValidityCustomError => v.custom_error,
        O::ValidityValid => v.valid(),
        _ => unreachable!(),
    }
}
fn autocomplete_value(raw: &[u16], form: bool, hidden: bool) -> Vec<u16> {
    let value = String::from_utf16_lossy(raw);
    let tokens: Vec<_> = value.split_ascii_whitespace()
        .map(str::to_ascii_lowercase).collect();
    if form {
        return if tokens.len() == 1 && tokens[0] == "off" {
            "off".encode_utf16().collect()
        } else {
            "on".encode_utf16().collect()
        };
    }
    if tokens.is_empty() { return Vec::new(); }
    let single = tokens.len() == 1;
    if single && matches!(tokens[0].as_str(), "on" | "off") && !hidden {
        return tokens[0].encode_utf16().collect();
    }
    if single && tokens[0] == "webauthn" { return tokens[0].encode_utf16().collect(); }
    let mut at = 0;
    if tokens[at].starts_with("section-") && tokens[at].len() > 8 { at += 1; }
    if tokens.get(at).is_some_and(|token| matches!(token.as_str(), "shipping" | "billing")) { at += 1; }
    if tokens.get(at).is_some_and(|token| matches!(token.as_str(), "home" | "work" | "mobile" | "fax" | "pager")) { at += 1; }
    const FIELDS: &[&str] = &[
        "name", "honorific-prefix", "given-name", "additional-name", "family-name", "honorific-suffix",
        "nickname", "username", "new-password", "current-password", "one-time-code",
        "organization-title", "organization", "street-address", "address-line1", "address-line2",
        "address-line3", "address-level4", "address-level3", "address-level2", "address-level1",
        "country", "country-name", "postal-code", "cc-name", "cc-given-name", "cc-additional-name",
        "cc-family-name", "cc-number", "cc-exp", "cc-exp-month", "cc-exp-year", "cc-csc",
        "cc-type", "transaction-currency", "transaction-amount", "language", "bday", "bday-day",
        "bday-month", "bday-year", "sex", "url", "photo", "tel", "tel-country-code",
        "tel-national", "tel-area-code", "tel-local", "tel-local-prefix", "tel-local-suffix",
        "tel-extension", "email", "impp",
    ];
    if !tokens.get(at).is_some_and(|token| FIELDS.contains(&token.as_str())) {
        return Vec::new();
    }
    at += 1;
    if tokens.get(at).is_some_and(|token| token == "webauthn") { at += 1; }
    if at == tokens.len() { tokens.join(" ").encode_utf16().collect() } else { Vec::new() }
}
fn submission_data(dom: &amazing_dom::Dom, form: NodeIdx,
    submitter: Option<NodeIdx>) -> Result<FormData, DomError> {
    let mut data = FormData::default();
    let mut controls = dom.form_controls(form)?;
    if let Some(submitter) = submitter.filter(|node| dom.html_tag(*node, "input")
        && dom.input_type(*node).ok() == Some("image")) { controls.push(submitter); }
    for control in controls {
        if dom.get_attribute_ns(control, None, "disabled").is_some() { continue; }
        let name = dom.attribute_utf16(control, "name").unwrap_or_default();
        let name = String::from_utf16_lossy(&name);
        if dom.html_tag(control, "input") {
            let kind = dom.input_type(control)?;
            if matches!(kind, "button" | "reset")
                || matches!(kind, "submit" | "image") && submitter != Some(control) { continue; }
            if matches!(kind, "checkbox" | "radio") && !dom.input_checked(control)? { continue; }
            if kind == "image" {
                let prefix = if name.is_empty() { String::new() } else { format!("{name}.") };
                data.append_text(&format!("{prefix}x"), "0");
                data.append_text(&format!("{prefix}y"), "0");
                continue;
            }
            if name.is_empty() { continue; }
            if kind == "file" {
                data.append_file(&name, File::new([], "", "application/octet-stream", 0), None);
            } else {
                let value = if kind == "hidden" && name.eq_ignore_ascii_case("_charset_") {
                    "UTF-8".into()
                } else {
                    String::from_utf16_lossy(&dom.input_value(control)?)
                };
                data.append_text(&name, &value);
            }
        } else if dom.html_tag(control, "button") {
            if submitter == Some(control) && !name.is_empty() {
                data.append_text(&name, &String::from_utf16_lossy(
                    &dom.attribute_utf16(control, "value").unwrap_or_default()));
            }
        } else if dom.html_tag(control, "textarea") {
            if !name.is_empty() {
                data.append_text(&name, &String::from_utf16_lossy(&dom.textarea_value(control)?));
            }
        } else if dom.html_tag(control, "select") && !name.is_empty() {
            for option in dom.select_options(control)? {
                if dom.option_selected(option)?
                    && dom.get_attribute_ns(option, None, "disabled").is_none() {
                    data.append_text(&name, &String::from_utf16_lossy(&dom.option_value(option)?));
                }
            }
        }
        if let Some(dirname) = dom.attribute_utf16(control, "dirname") {
            if !dirname.is_empty() {
                data.append_text(&String::from_utf16_lossy(&dirname), "ltr");
            }
        }
    }
    Ok(data)
}
fn is_submit_button(dom: &amazing_dom::Dom, node: NodeIdx) -> Result<bool, DomError> {
    Ok((dom.html_tag(node, "input")
        && matches!(dom.input_type(node)?, "submit" | "image"))
        || (dom.html_tag(node, "button")
            && !matches!(dom.get_attribute_ns(node, None, "type")
                .map(str::to_ascii_lowercase).as_deref(), Some("reset" | "button"))))
}
fn form_urlencoded(data: &FormData) -> String {
    amazing_net::url::serialize_form_urlencoded(&data.entries.iter().map(|(name, value)| {
        (name.clone(), match value {
            FormValue::Text(text) => text.clone(),
            FormValue::File(file) => file.name.clone(),
        })
    }).collect::<Vec<_>>())
}
fn text_plain(data: &FormData) -> Vec<u8> {
    let mut out = String::new();
    for (name, value) in &data.entries {
        let value = match value { FormValue::Text(text) => text, FormValue::File(file) => &file.name };
        out.push_str(name);
        out.push('=');
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.into_bytes()
}
pub(crate) fn invoke<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: H::Value,
    args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::FormDataFormConstructor {
        let Some(value) = args.first().filter(|value| !host.is_undefined(value)) else {
            return host.form_data_new(FormData::default());
        };
        let form = node(host, value, &["form"])?;
        let submitter = args.get(1).filter(|value| !host.is_nullish(value))
            .map(|value| node(host, value, &["input", "button"]))
            .transpose()?;
        if let Some(submitter) = submitter {
            if !host.with_dom(|dom| is_submit_button(dom, submitter))? {
                return Err(host.type_error("Expected a submit button"));
            }
            let owner = host.with_dom(|dom| Ok(dom.form_owner(submitter)))?;
            if owner != Some(form) {
                return host.with_dom(|_| Err(DomError::NotFoundError));
            }
        }
        if !host.with_dom(|dom| dom.begin_form_entry_list(form))? {
            return host.form_data_new(FormData::default());
        }
        let result = (|| {
            let data = host.with_dom(|dom| submission_data(dom, form, submitter))?;
            let data = host.form_data_event(form, data)?;
            host.form_data_new(data)
        })();
        host.with_dom(|dom| dom.end_form_entry_list(form))?;
        return result;
    }
    if matches!(op, O::SelectAdd | O::SelectRemove)
        && host.platform_object(&receiver, B::OPTIONS).is_some()
    {
        let select = collection_owner(host, &receiver, B::OPTIONS, "select")?;
        let node = host.wrap_node(select)?;
        return invoke(host, op, node, args);
    }
    if matches!(
        op,
        O::OptionsSetLength | O::OptionsSelectedIndex | O::OptionsSetSelectedIndex
    ) {
        let select = collection_owner(host, &receiver, B::OPTIONS, "select")?;
        return match op {
            O::OptionsSelectedIndex => {
                let value = host.with_dom(|dom| dom.selected_index(select))?;
                Ok(host.number(value as f64))
            }
            O::OptionsSetSelectedIndex => {
                let value = signed(number(host, args, 0)?);
                host.with_dom(|dom| dom.set_selected_index(select, value))?;
                Ok(host.undefined())
            }
            _ => {
                let desired = unsigned(number(host, args, 0)?);
                host.with_dom(|dom| {
                    let options = dom.select_options(select)?;
                    for old in options.iter().skip(desired as usize).rev() {
                        if let Some(parent) = dom.get(*old).parent {
                            dom.pre_remove(*old, parent)?;
                        }
                    }
                    for _ in options.len()..desired as usize {
                        dom.create_element(select, "option");
                    }
                    Ok(())
                })?;
                Ok(host.undefined())
            }
        };
    }
    if matches!(
        op,
        O::FormAutocompleteGet | O::FormAutocompleteSet
    ) {
        let element = node(host, &receiver, &["form", "input", "select", "textarea"])?;
        if op == O::FormAutocompleteSet {
            let value = string(host, args, 0)?;
            set_attr(host, element, "autocomplete", &value)?;
            return Ok(host.undefined());
        }
        let (raw, form, hidden) = host.with_dom(|dom| Ok((
            dom.attribute_utf16(element, "autocomplete").unwrap_or_default(),
            dom.html_tag(element, "form"),
            dom.html_tag(element, "input") && dom.input_type(element)? == "hidden",
        )))?;
        return host.string(&autocomplete_value(&raw, form, hidden));
    }
    if matches!(
        op,
        O::ValidityValueMissing
            | O::ValidityTypeMismatch
            | O::ValidityPatternMismatch
            | O::ValidityTooLong
            | O::ValidityTooShort
            | O::ValidityRangeUnderflow
            | O::ValidityRangeOverflow
            | O::ValidityStepMismatch
            | O::ValidityBadInput
            | O::ValidityCustomError
            | O::ValidityValid
    ) {
        let key = host
            .platform_object(&receiver, B::VALIDITY_STATE)
            .ok_or_else(|| host.type_error("Expected ValidityState"))?;
        let owner = host
            .node_from_platform_key(key)
            .ok_or_else(|| host.type_error("Invalid ValidityState"))?;
        let state = validity(host, owner)?;
        return Ok(host.boolean(validity_flag(&state, op)));
    }
    if matches!(
        op,
        O::FormCollectionLength
            | O::FormCollectionItem
            | O::FormCollectionNamedItem
            | O::FormCollectionIndexedGetter
            | O::FormCollectionNamedGetter
            | O::OptionsLength
            | O::OptionsItem
            | O::OptionsNamedItem
            | O::OptionsIndexedGetter
            | O::OptionsNamedGetter
            | O::RadioNodeListLength
            | O::RadioNodeListItem
            | O::RadioNodeListIndexedGetter
            | O::RadioNodeListValue
            | O::RadioNodeListSetValue
    ) {
        let items = collection_items(host, op, &receiver)?;
        if matches!(
            op,
            O::FormCollectionLength | O::OptionsLength | O::RadioNodeListLength
        ) {
            return Ok(host.number(items.len() as f64));
        }
        if op == O::RadioNodeListValue {
            let value = host.with_dom(|dom| {
                for node in &items {
                    if dom.html_tag(*node, "input")
                        && dom.input_type(*node)? == "radio"
                        && dom.input_checked(*node)?
                    {
                        return dom.input_value(*node);
                    }
                }
                Ok(Vec::new())
            })?;
            return host.string(&value);
        }
        if op == O::RadioNodeListSetValue {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| {
                for node in &items {
                    if dom.html_tag(*node, "input")
                        && dom.input_type(*node)? == "radio"
                        && dom.input_value(*node)? == value
                    {
                        dom.set_input_checked(*node, true)?;
                        break;
                    }
                }
                Ok(())
            })?;
            return Ok(host.undefined());
        }
        if matches!(
            op,
            O::FormCollectionNamedItem
                | O::FormCollectionNamedGetter
                | O::OptionsNamedItem
                | O::OptionsNamedGetter
        ) {
            let name = string(host, args, 0)?;
            let found = host.with_dom(|dom| Ok(named(&items, dom, &name)))?;
            if found.len() > 1
                && matches!(
                    op,
                    O::FormCollectionNamedItem | O::FormCollectionNamedGetter
                )
            {
                let form = collection_owner(host, &receiver, B::FORM_CONTROLS, "form")?;
                let slot = host.with_dom(|dom| dom.radio_list(form, name))?;
                let mut key = host.node_platform_key(form);
                key.slot = slot as u64;
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
        let option = if host.is_nullish(value) {
            None
        } else {
            Some(node(host, value, &["option"])?)
        };
        host.with_dom(|dom| {
            let options = dom.select_options(select)?;
            if let Some(option) = option {
                if let Some(old) = options.get(position) {
                    let parent = dom.get(*old).parent.ok_or(DomError::NotFoundError)?;
                    dom.replace(*old, option, parent)?;
                } else {
                    for _ in options.len()..position {
                        dom.create_element(select, "option");
                    }
                    dom.pre_insert(option, select, None)?;
                }
            } else if let Some(old) = options.get(position) {
                let parent = dom.get(*old).parent.ok_or(DomError::NotFoundError)?;
                dom.pre_remove(*old, parent)?;
            }
            Ok(())
        })?;
        return Ok(host.undefined());
    }
    if matches!(
        op,
        O::FormElements
            | O::FormLength
            | O::FormReset
            | O::FormSubmit
            | O::FormRequestSubmit
            | O::FormCheckValidity
            | O::FormReportValidity
    ) {
        let form = node(host, &receiver, &["form"])?;
        if matches!(op, O::FormSubmit | O::FormRequestSubmit)
            && host.with_dom(|dom| dom.form_entry_list_construction_started(form))? {
            return Ok(host.undefined());
        }
        return match op {
            O::FormElements => key_node(host, form, B::FORM_CONTROLS),
            O::FormLength => {
                let len = host.with_dom(|dom| Ok(dom.form_controls(form)?.len()))?;
                Ok(host.number(len as f64))
            }
            O::FormReset => {
                if crate::event_bindings::dispatch_reset_event(host, form)? {
                    host.with_dom(|dom| dom.reset_form(form))?;
                }
                Ok(host.undefined())
            }
            O::FormSubmit | O::FormRequestSubmit => {
                let submitter = if op == O::FormRequestSubmit {
                    args.first().filter(|value| !host.is_nullish(value))
                        .map(|value| node(host, value, &["input", "button"]))
                        .transpose()?
                } else { None };
                if let Some(submitter) = submitter {
                    let submit_button = host.with_dom(|dom| is_submit_button(dom, submitter))?;
                    if !submit_button { return Err(host.type_error("Expected a submit button")); }
                    let owner = host.with_dom(|dom| Ok(dom.form_owner(submitter)))?;
                    if owner != Some(form) {
                        return host.with_dom(|_| Err(DomError::NotFoundError));
                    }
                }
                if !host.with_dom(|dom| Ok(dom.is_connected(form)))? {
                    return Ok(host.undefined());
                }
                if op == O::FormRequestSubmit {
                    if !host.with_dom(|dom| dom.begin_form_submission_events(form))? {
                        return Ok(host.undefined());
                    }
                    let should_submit = (|| {
                        let no_validate = host.with_dom(|dom| Ok(dom.get_attribute_ns(form, None, "novalidate").is_some()
                            || submitter.is_some_and(|button| dom.get_attribute_ns(button, None, "formnovalidate").is_some())))?;
                        if !no_validate {
                            let controls = host.with_dom(|dom| dom.form_controls(form))?;
                            let mut valid = true;
                            for control in controls {
                                if host.with_dom(|dom| dom.will_validate(control))?
                                    && !validate(host, control)? { valid = false; }
                            }
                            if !valid { return Ok(false); }
                        }
                        host.form_submit_event(form, submitter)
                    })();
                    host.with_dom(|dom| dom.end_form_submission_events(form))?;
                    if !should_submit? { return Ok(host.undefined()); }
                }
                if !host.with_dom(|dom| dom.begin_form_entry_list(form))? {
                    return Ok(host.undefined());
                }
                let data = (|| {
                    let data = host.with_dom(|dom| submission_data(dom, form, submitter))?;
                    host.form_data_event(form, data)
                })();
                host.with_dom(|dom| dom.end_form_entry_list(form))?;
                let data = data?;
                let (action, method, enctype, target_name) = host.with_dom(|dom| {
                    let inherited = |name| dom.attribute_utf16(form, name);
                    let override_attr = |name| submitter.and_then(|node| dom.attribute_utf16(node, name))
                        .filter(|value| !value.is_empty());
                    Ok((override_attr("formaction").or_else(|| inherited("action")),
                        override_attr("formmethod").or_else(|| inherited("method")),
                        override_attr("formenctype").or_else(|| inherited("enctype")),
                        override_attr("formtarget").or_else(|| inherited("target"))))
                })?;
                let base = host.location_href()?;
                let action = action.filter(|value| !value.is_empty())
                    .unwrap_or_else(|| base.encode_utf16().collect());
                let url = amazing_net::url::ParsedUrl::parse(&base)
                    .and_then(|base| base.resolve(&String::from_utf16_lossy(&action)))
                    .map_err(|_| host.type_error("Invalid form action URL"))?
                    .serialize();
                let method = method.map(|value| String::from_utf16_lossy(&value).to_ascii_uppercase())
                    .filter(|value| matches!(value.as_str(), "GET" | "POST" | "DIALOG"))
                    .unwrap_or_else(|| "GET".into());
                if method == "DIALOG" { return Ok(host.undefined()); }
                let enctype = enctype.map(|value| String::from_utf16_lossy(&value).to_ascii_lowercase())
                    .filter(|value| matches!(value.as_str(), "application/x-www-form-urlencoded"
                        | "multipart/form-data" | "text/plain"))
                    .unwrap_or_else(|| "application/x-www-form-urlencoded".into());
                let target_name = target_name.map(|value| String::from_utf16_lossy(&value))
                    .unwrap_or_default();
                let (url, body, content_type) = if method == "GET" {
                    let encoded = form_urlencoded(&data);
                    let (head, fragment) = url.split_once('#').map_or((url.as_str(), None),
                        |(head, fragment)| (head, Some(fragment)));
                    let path = head.split_once('?').map_or(head, |(path, _)| path);
                    let mut url = format!("{path}?{encoded}");
                    if let Some(fragment) = fragment { url.push('#'); url.push_str(fragment); }
                    (url, Vec::new(), None)
                } else if enctype == "multipart/form-data" {
                    let (body, content_type) = data.encode_multipart();
                    (url, body, Some(content_type))
                } else if enctype == "text/plain" {
                    (url, text_plain(&data), Some("text/plain;charset=UTF-8".into()))
                } else {
                    (url, form_urlencoded(&data).into_bytes(),
                        Some("application/x-www-form-urlencoded;charset=UTF-8".into()))
                };
                host.form_navigate(FormNavigationRequest {
                    url, method, enctype, target: target_name, body, content_type,
                })?;
                Ok(host.undefined())
            }
            _ => {
                let controls = host.with_dom(|dom| dom.form_controls(form))?;
                let mut valid = true;
                for control in controls {
                    if host.with_dom(|dom| Ok(dom.html_tag(control, "object")))? {
                        continue;
                    }
                    if host.with_dom(|dom| dom.will_validate(control))? && !validate(host, control)?
                    {
                        valid = false;
                    }
                }
                Ok(host.boolean(valid))
            }
        };
    }
    if matches!(op, O::LabelControl | O::LabelForm) {
        let label = node(host, &receiver, &["label"])?;
        let control = if op == O::LabelControl {
            host.with_dom(|dom| dom.label_control(label))?
        } else {
            host.with_dom(|dom| dom.label_form(label))?
        };
        return wrap(host, control);
    }
    let target = node(
        host,
        &receiver,
        &[
            "input", "textarea", "select", "option", "button", "fieldset", "output",
            "object", "progress", "meter",
        ],
    )?;
    match op {
        O::FormControlForm => {
            let form = host.with_dom(|dom| Ok(dom.form_owner(target)))?;
            wrap(host, form)
        }
        O::ProgressValue => { let value = progress_value(host, target)?; Ok(host.number(value)) }
        O::ProgressSetValue => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "value", value)?;
            Ok(host.undefined())
        }
        O::ProgressMax => { let value = progress_max(host, target)?; Ok(host.number(value)) }
        O::ProgressSetMax => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "max", value)?;
            Ok(host.undefined())
        }
        O::ProgressPosition => {
            if parsed_attr_number(host, target, "value")?.is_none() {
                return Ok(host.number(-1.));
            }
            let value = progress_value(host, target)?;
            let max = progress_max(host, target)?;
            Ok(host.number(value / max))
        }
        O::MeterValue => { let value = meter_value(host, target)?; Ok(host.number(value)) }
        O::MeterSetValue => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "value", value)?;
            Ok(host.undefined())
        }
        O::MeterMin => { let value = meter_min(host, target)?; Ok(host.number(value)) }
        O::MeterSetMin => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "min", value)?;
            Ok(host.undefined())
        }
        O::MeterMax => { let value = meter_max(host, target)?; Ok(host.number(value)) }
        O::MeterSetMax => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "max", value)?;
            Ok(host.undefined())
        }
        O::MeterLow => { let value = meter_low(host, target)?; Ok(host.number(value)) }
        O::MeterSetLow => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "low", value)?;
            Ok(host.undefined())
        }
        O::MeterHigh => { let value = meter_high(host, target)?; Ok(host.number(value)) }
        O::MeterSetHigh => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "high", value)?;
            Ok(host.undefined())
        }
        O::MeterOptimum => { let value = meter_optimum(host, target)?; Ok(host.number(value)) }
        O::MeterSetOptimum => {
            let value = finite_number(host, args, 0)?;
            set_attr_number(host, target, "optimum", value)?;
            Ok(host.undefined())
        }
        O::FormControlName => {
            let value = attr(host, target, "name")?;
            host.string(&value)
        }
        O::FormControlSetName => {
            let value = string(host, args, 0)?;
            set_attr(host, target, "name", &value)?;
            Ok(host.undefined())
        }
        O::OutputValue | O::OutputDefaultValue => {
            let text = host.with_dom(|dom| {
                let mut value = String::new();
                for child in dom.tree_order(target).into_iter().skip(1) {
                    if let Some(units) = dom.character_data_utf16(child) {
                        value.push_str(&String::from_utf16_lossy(&units));
                    }
                }
                Ok(value.encode_utf16().collect::<Vec<u16>>())
            })?;
            host.string(&text)
        }
        O::OutputSetValue | O::OutputSetDefaultValue => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| dom.replace_children(target, &[NodeOrString::Utf16(value)]))?;
            Ok(host.undefined())
        }
        O::InputValue | O::TextAreaValue | O::SelectValue | O::OptionValue => {
            let value = host.with_dom(|dom| match op {
                O::InputValue => dom.input_value(target),
                O::TextAreaValue => dom.textarea_value(target),
                O::SelectValue => dom.select_value(target),
                _ => dom.option_value(target),
            })?;
            host.string(&value)
        }
        O::InputSetValue | O::TextAreaSetValue | O::SelectSetValue | O::OptionSetValue => {
            let value = string(host, args, 0)?;
            if op == O::OptionSetValue {
                set_attr(host, target, "value", &value)?;
                return Ok(host.undefined());
            }
            host.with_dom(|dom| match op {
                O::InputSetValue => dom.set_input_value(target, value),
                O::TextAreaSetValue => dom.set_textarea_value(target, value),
                O::SelectSetValue => dom.set_select_value(target, &value),
                _ => unreachable!(),
            })?;
            Ok(host.undefined())
        }
        O::InputDefaultValue => {
            let value = attr(host, target, "value")?;
            host.string(&value)
        }
        O::InputSetDefaultValue => {
            let value = string(host, args, 0)?;
            set_attr(host, target, "value", &value)?;
            Ok(host.undefined())
        }
        O::InputChecked | O::OptionSelected => {
            let value = host.with_dom(|dom| {
                if op == O::InputChecked {
                    dom.input_checked(target)
                } else {
                    dom.option_selected(target)
                }
            })?;
            Ok(host.boolean(value))
        }
        O::InputSetChecked | O::OptionSetSelected => {
            let checked = boolean(host, args, 0)?;
            host.with_dom(|dom| {
                if op == O::InputSetChecked {
                    dom.set_input_checked(target, checked)
                } else {
                    dom.set_option_selected(target, checked)
                }
            })?;
            Ok(host.undefined())
        }
        O::InputDefaultChecked | O::OptionDefaultSelected => {
            let name = if op == O::InputDefaultChecked {
                "checked"
            } else {
                "selected"
            };
            let value =
                host.with_dom(|dom| Ok(dom.get_attribute_ns(target, None, name).is_some()))?;
            Ok(host.boolean(value))
        }
        O::InputSetDefaultChecked | O::OptionSetDefaultSelected => {
            let name = if op == O::InputSetDefaultChecked {
                "checked"
            } else {
                "selected"
            };
            let value = boolean(host, args, 0)?;
            bool_attr(host, target, name, value)?;
            Ok(host.undefined())
        }
        O::SelectSelectedIndex => {
            let value = host.with_dom(|dom| dom.selected_index(target))?;
            Ok(host.number(value as f64))
        }
        O::SelectSetSelectedIndex => {
            let n = number(host, args, 0)?;
            let index = signed(n);
            host.with_dom(|dom| dom.set_selected_index(target, index))?;
            Ok(host.undefined())
        }
        O::SelectOptions => key_node(host, target, B::OPTIONS),
        O::SelectLength => {
            let len = host.with_dom(|dom| Ok(dom.select_options(target)?.len()))?;
            Ok(host.number(len as f64))
        }
        O::SelectMultiple => {
            let value =
                host.with_dom(|dom| Ok(dom.get_attribute_ns(target, None, "multiple").is_some()))?;
            Ok(host.boolean(value))
        }
        O::SelectSetMultiple => {
            let value = boolean(host, args, 0)?;
            bool_attr(host, target, "multiple", value)?;
            Ok(host.undefined())
        }
        O::SelectAdd => {
            let first = arg(host, args, 0)?;
            let option = node(host, first, &["option", "optgroup"])?;
            let before = if let Some(value) = args.get(1) {
                if host.is_nullish(value) {
                    None
                } else if let Some(node) = host.node_brand(value) {
                    Some(node)
                } else {
                    let n = host.to_number(value)?;
                    host.with_dom(|dom| Ok(dom.select_options(target)?.get(n as usize).copied()))?
                }
            } else {
                None
            };
            host.with_dom(|dom| {
                dom.pre_insert(option, target, before)?;
                Ok(())
            })?;
            Ok(host.undefined())
        }
        O::SelectRemove => {
            let index = if args.is_empty() {
                0
            } else {
                index(host, args, 0)?
            };
            host.with_dom(|dom| {
                if let Some(option) = dom.select_options(target)?.get(index) {
                    let parent = dom.get(*option).parent.ok_or(DomError::NotFoundError)?;
                    dom.pre_remove(*option, parent)?;
                }
                Ok(())
            })?;
            Ok(host.undefined())
        }
        O::OptionText => {
            let value = host.with_dom(|dom| dom.option_text(target))?;
            host.string(&value)
        }
        O::OptionSetText => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| dom.replace_children(target, &[NodeOrString::Utf16(value)]))?;
            Ok(host.undefined())
        }
        O::OptionIndex => {
            let index = host.with_dom(|dom| dom.option_index(target))?;
            Ok(host.number(index as f64))
        }
        O::ControlWillValidate => {
            let value = host.with_dom(|dom| dom.will_validate(target))?;
            Ok(host.boolean(value))
        }
        O::ControlValidity => key_node(host, target, B::VALIDITY_STATE),
        O::ControlValidationMessage => {
            if !host.with_dom(|dom| dom.will_validate(target))? {
                return host.string(&[]);
            }
            let custom = host.with_dom(|dom| dom.validation_message(target))?;
            if !custom.is_empty() {
                return host.string(&custom);
            }
            let message = if validity(host, target)?.valid() {
                Vec::new()
            } else {
                "Value is invalid.".encode_utf16().collect()
            };
            host.string(&message)
        }
        O::ControlSetCustomValidity => {
            let value = string(host, args, 0)?;
            host.with_dom(|dom| dom.set_custom_validity(target, value))?;
            Ok(host.undefined())
        }
        O::ControlCheckValidity | O::ControlReportValidity => {
            let value = validate(host, target)?;
            Ok(host.boolean(value))
        }
        O::InputSelectionStart | O::InputSelectionEnd | O::InputSelectionDirection => {
            if !host.with_dom(|dom| dom.selection_applicable(target))? {
                return Ok(host.null());
            }
            let (start, end, direction) = host.with_dom(|dom| dom.selection(target))?;
            match op {
                O::InputSelectionStart => Ok(host.number(start as f64)),
                O::InputSelectionEnd => Ok(host.number(end as f64)),
                _ => host.string(match direction {
                    SelectionDirection::None => &['n' as u16, 'o' as u16, 'n' as u16, 'e' as u16],
                    SelectionDirection::Forward => &[
                        'f' as u16, 'o' as u16, 'r' as u16, 'w' as u16, 'a' as u16, 'r' as u16,
                        'd' as u16,
                    ],
                    SelectionDirection::Backward => &[
                        'b' as u16, 'a' as u16, 'c' as u16, 'k' as u16, 'w' as u16, 'a' as u16,
                        'r' as u16, 'd' as u16,
                    ],
                }),
            }
        }
        O::InputSelect
        | O::InputSetSelectionRange
        | O::InputSetSelectionStart
        | O::InputSetSelectionEnd
        | O::InputSetSelectionDirection => {
            if op == O::InputSelect && !host.with_dom(|dom| dom.selection_applicable(target))? {
                return Ok(host.undefined());
            }
            let (mut start, mut end, mut direction) = host.with_dom(|dom| dom.selection(target))?;
            match op {
                O::InputSelect => {
                    start = 0;
                    end = u32::MAX;
                    direction = SelectionDirection::None;
                }
                O::InputSetSelectionRange => {
                    start = unsigned(number(host, args, 0)?);
                    end = unsigned(number(host, args, 1)?);
                    direction = args.get(2).map(|value| host.to_dom_string(value))
                        .transpose()?.as_deref().map(direction_from)
                        .unwrap_or(SelectionDirection::None);
                }
                O::InputSetSelectionStart => {
                    start = unsigned(number(host, args, 0)?);
                    end = end.max(start);
                }
                O::InputSetSelectionEnd => {
                    end = unsigned(number(host, args, 0)?);
                    start = start.min(end);
                }
                _ => direction = direction_from(&string(host, args, 0)?),
            }
            let old = host.with_dom(|dom| dom.selection(target))?;
            host.with_dom(|dom| dom.set_selection(target, start, end, direction))?;
            if host.with_dom(|dom| dom.selection(target))? != old {
                host.form_selection_changed(target)?;
            }
            Ok(host.undefined())
        }
        O::InputSetRangeText => {
            let replacement = string(host, args, 0)?;
            let range = if args.len() >= 2 {
                let start = unsigned(number(host, args, 1)?);
                let end = unsigned(number(host, args, 2)?);
                Some((start, end))
            } else {
                None
            };
            let mode = if let Some(value) = args.get(3) {
                String::from_utf16_lossy(&host.to_dom_string(value)?)
            } else {
                "preserve".to_owned()
            };
            if !matches!(mode.as_str(), "select" | "start" | "end" | "preserve") {
                return Err(host.type_error("Invalid selection mode"));
            }
            let old = host.with_dom(|dom| dom.selection(target))?;
            host.with_dom(|dom| dom.set_range_text(target, replacement, range, &mode))?;
            if host.with_dom(|dom| dom.selection(target))? != old {
                host.form_selection_changed(target)?;
            }
            Ok(host.undefined())
        }
        O::InputValueAsNumber => {
            let number = host.with_dom(|dom| dom.input_value_as_number(target))?;
            Ok(host.number(number))
        }
        O::InputSetValueAsNumber => {
            let value = number(host, args, 0)?;
            if !value.is_finite() {
                return Err(host.type_error("Invalid valueAsNumber"));
            }
            host.with_dom(|dom| dom.set_input_value_as_number(target, value))?;
            Ok(host.undefined())
        }
        O::InputValueAsDate => {
            let value = host.with_dom(|dom| dom.input_value_as_date_ms(target))?;
            match value {
                Some(ms) => host.form_date_from_epoch_ms(ms),
                None => Ok(host.null()),
            }
        }
        O::InputSetValueAsDate => {
            let value = arg(host, args, 0)?;
            let date = if host.is_nullish(value) {
                None
            } else {
                host.form_date_to_epoch_ms(value)?
            };
            host.with_dom(|dom| dom.set_input_value_as_date_ms(target, date))?;
            Ok(host.undefined())
        }
        O::InputStepUp | O::InputStepDown => {
            let count = if args.is_empty() { 1 } else { signed(number(host, args, 0)?) };
            let count = if op == O::InputStepDown { count.wrapping_neg() } else { count };
            host.with_dom(|dom| dom.step_input(target, count))?;
            Ok(host.undefined())
        }
        O::TextAreaDefaultValue => {
            let value = host.with_dom(|dom| {
                let mut raw = Vec::new();
                for child in dom.tree_order(target).into_iter().skip(1) {
                    raw.extend(dom.character_data_utf16(child).unwrap_or_default());
                }
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
            let kind = host.with_dom(|dom| {
                Ok(dom
                    .get_attribute_ns(target, None, "type")
                    .unwrap_or("submit")
                    .to_ascii_lowercase())
            })?;
            let value: Vec<u16> = if ["submit", "reset", "button"].contains(&kind.as_str()) {
                kind
            } else {
                "submit".into()
            }
            .encode_utf16()
            .collect();
            host.string(&value)
        }
        O::ButtonSetType => {
            let value = string(host, args, 0)?;
            set_attr(host, target, "type", &value)?;
            Ok(host.undefined())
        }
        _ => Err(host.type_error("Unsupported form operation")),
    }
}
fn direction_from(value: &[u16]) -> SelectionDirection {
    if value == "forward".encode_utf16().collect::<Vec<_>>() {
        SelectionDirection::Forward
    } else if value == "backward".encode_utf16().collect::<Vec<_>>() {
        SelectionDirection::Backward
    } else {
        SelectionDirection::None
    }
}
