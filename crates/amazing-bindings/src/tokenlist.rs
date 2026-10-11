//! DOMTokenList conversions retain the original genuine receiver arena.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{DomError, NodeKind};
use amazing_dom::token_list::{TokenListCapture, TokenListSupport};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementClassList | O::ElementClassName | O::ElementSetClassName
        | O::TokenListLength | O::TokenListItem | O::TokenListIndexedGetter | O::TokenListContains
        | O::TokenListAdd | O::TokenListRemove | O::TokenListToggle | O::TokenListReplace
        | O::TokenListSupports | O::TokenListValue | O::TokenListSetValue)
}
fn receiver<H: BindingHost>(host: &mut H, value: &H::Value, element: bool)
    -> Result<(H::DomCapture, TokenListCapture), H::Error> {
    let key = host.platform_object(value, if element { InterfaceBrand::NODE } else { InterfaceBrand::DOM_TOKEN_LIST })
        .ok_or_else(|| host.type_error("Incompatible DOMTokenList receiver"))?;
    let arena = host.capture_dom(key.arena)?;
    let capture = host.with_captured_dom(&arena, |dom| {
        if element {
            let node = amazing_dom::NodeIdx(u32::try_from(key.slot).map_err(|_| DomError::InvalidStateError)?);
            if dom.node_generation(node) != Some(key.generation) { return Err(DomError::InvalidStateError); }
            if !matches!(dom.get(node).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
            let slot = dom.token_list(node, "class")?;
            dom.token_list_capture(slot)
        } else {
            let slot = usize::try_from(key.slot).map_err(|_| DomError::InvalidStateError)?;
            let capture = dom.token_list_capture(slot)?;
            if capture.view_generation() != key.generation { return Err(DomError::InvalidStateError); }
            Ok(capture)
        }
    })?;
    Ok((arena, capture))
}
pub(crate) fn wrap<H: BindingHost>(host: &mut H, node: amazing_dom::NodeIdx, attribute: &str)
    -> Result<H::Value, H::Error> {
    let mut key = host.node_object_key(node);
    let arena = host.capture_dom(key.arena)?;
    let capture = host.with_captured_dom(&arena, |dom| {
        if dom.node_generation(node) != Some(key.generation) { return Err(DomError::InvalidStateError); }
        let slot = dom.token_list(node, attribute)?;
        dom.token_list_capture(slot)
    })?;
    key.slot = capture.slot() as u64;
    key.generation = capture.view_generation();
    host.wrap_platform_object(key, InterfaceBrand::DOM_TOKEN_LIST)
}
pub(crate) fn supported_index_count<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<u32, H::Error> {
    let (arena, capture) = receiver(host, value, false)?;
    let count = host.with_captured_dom(&arena, |dom| dom.token_list_length(&capture))?;
    u32::try_from(count).map_err(|_| host.type_error("DOMTokenList length exceeds u32"))
}
fn required_string<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<Vec<u16>, H::Error> {
    let value = args.get(index).ok_or_else(|| host.type_error("Not enough arguments"))?;
    host.to_dom_string(value)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, value: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    let element = matches!(op, O::ElementClassList | O::ElementClassName | O::ElementSetClassName);
    let (arena, capture) = receiver(host, &value, element)?;
    if op == O::ElementClassList {
        let node = host.with_captured_dom(&arena, |dom| dom.token_list_owner(&capture))?.node;
        return wrap(host, node, "class");
    }
    if matches!(op, O::ElementClassName | O::TokenListValue) {
        let units = host.with_captured_dom(&arena, |dom| dom.token_list_value(&capture))?;
        return host.string(&units);
    }
    if matches!(op, O::ElementSetClassName | O::TokenListSetValue) {
        let units = required_string(host, args, 0)?;
        host.with_captured_dom(&arena, |dom| dom.token_list_set_value(&capture, units))?;
        return Ok(host.undefined());
    }
    if op == O::TokenListLength {
        let count = host.with_captured_dom(&arena, |dom| dom.token_list_length(&capture))?;
        return Ok(host.number(count as f64));
    }
    if matches!(op, O::TokenListItem | O::TokenListIndexedGetter) {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let number = host.to_number(arg)?;
        let index = if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 };
        let token = host.with_captured_dom(&arena, |dom| dom.token_list_item(&capture, index))?;
        return match token { Some(units) => host.string(&units), None => Ok(host.null()) };
    }
    if op == O::TokenListSupports {
        let token = required_string(host, args, 0)?;
        let policy = host.token_list_supported_tokens(&arena, &capture)?;
        let slices = policy.as_ref().map(|tokens| tokens.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let support = match &slices { Some(tokens) => TokenListSupport::Defined(tokens), None => TokenListSupport::Undefined };
        let answer = host.with_captured_dom(&arena, |dom| dom.token_list_supports(&capture, &token, support))?;
        return Ok(host.boolean(answer));
    }
    // All author conversions finish before validation or reading the live set.
    let converted = if matches!(op, O::TokenListAdd | O::TokenListRemove) {
        args.iter().map(|arg| host.to_dom_string(arg)).collect::<Result<Vec<_>, _>>()?
    } else if op == O::TokenListReplace {
        if args.len() < 2 { return Err(host.type_error("Not enough arguments")); }
        vec![required_string(host, args, 0)?, required_string(host, args, 1)?]
    } else { vec![required_string(host, args, 0)?] };
    let force = if op == O::TokenListToggle { args.get(1).map(|value| host.to_boolean(value)) } else { None };
    let answer = host.with_captured_dom(&arena, |dom| match op {
        O::TokenListContains => dom.token_list_contains(&capture, &converted[0]).map(Some),
        O::TokenListAdd => dom.token_list_add(&capture, &converted).map(|_| None),
        O::TokenListRemove => dom.token_list_remove(&capture, &converted).map(|_| None),
        O::TokenListToggle => dom.token_list_toggle(&capture, &converted[0], force).map(Some),
        O::TokenListReplace => dom.token_list_replace(&capture, &converted[0], &converted[1]).map(Some),
        _ => Err(DomError::NotSupportedError),
    })?;
    Ok(answer.map_or_else(|| host.undefined(), |answer| host.boolean(answer)))
}
