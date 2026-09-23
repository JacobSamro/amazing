use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{DomError, NodeIdx, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementClassList | O::ElementClassName | O::ElementSetClassName
        | O::TokenListLength | O::TokenListItem | O::TokenListSupportedIndices | O::TokenListIndexedGetter | O::TokenListContains
        | O::TokenListAdd | O::TokenListRemove | O::TokenListToggle
        | O::TokenListReplace | O::TokenListSupports | O::TokenListValue
        | O::TokenListSetValue)
}
fn element<H: BindingHost>(host: &mut H, receiver: &H::Value, token_list: bool) -> Result<NodeIdx, H::Error> {
    let node = if token_list {
        let key = host.platform_object(receiver, InterfaceBrand::DOM_TOKEN_LIST)
            .ok_or_else(|| host.type_error("Expected a DOMTokenList"))?;
        host.node_from_platform_key(key).ok_or_else(|| host.type_error("Invalid DOMTokenList"))?
    } else {
        host.node_brand(receiver).ok_or_else(|| host.type_error("Expected an Element"))?
    };
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn class_value<H: BindingHost>(host: &mut H, element: NodeIdx) -> Result<Vec<u16>, H::Error> {
    host.with_dom(|dom| Ok(dom.get_attribute_node(element, None, "class")
        .and_then(|attr| dom.attr_snapshot(attr).ok()).map_or(Vec::new(), |attr| attr.value)))
}
fn set_class_value<H: BindingHost>(host: &mut H, element: NodeIdx, units: Vec<u16>) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        if dom.get_attribute_node(element, None, "class").is_none() {
            dom.set_attribute(element, "class", "")?;
        }
        let attr = dom.get_attribute_node(element, None, "class").ok_or(DomError::InvalidStateError)?;
        dom.set_attr_value(attr, units)
    })
}
fn whitespace(unit: u16) -> bool { matches!(unit, 0x9 | 0xA | 0xC | 0xD | 0x20) }
fn tokens(units: &[u16]) -> Vec<Vec<u16>> {
    let mut result = Vec::new();
    for token in units.split(|unit| whitespace(*unit)).filter(|token| !token.is_empty()) {
        if !result.iter().any(|existing| existing == token) { result.push(token.to_vec()); }
    }
    result
}
fn serialize(tokens: &[Vec<u16>]) -> Vec<u16> {
    let mut result = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if index != 0 { result.push(b' ' as u16); }
        result.extend(token);
    }
    result
}
fn invalid_token<H: BindingHost>(host: &mut H, error: DomError) -> H::Error {
    match host.with_dom::<()>(|_| Err(error)) { Err(error) => error, Ok(()) => unreachable!() }
}
fn token<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Vec<u16>, H::Error> {
    let units = host.to_dom_string(value)?;
    if units.is_empty() { return Err(invalid_token(host, DomError::SyntaxError)); }
    if units.iter().any(|unit| whitespace(*unit)) { return Err(invalid_token(host, DomError::InvalidCharacterError)); }
    Ok(units)
}
fn required_token<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<Vec<u16>, H::Error> {
    let value = args.get(index).ok_or_else(|| host.type_error("Not enough arguments"))?;
    token(host, value)
}
pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let is_element = matches!(op, O::ElementClassList | O::ElementClassName | O::ElementSetClassName);
    let element = element(host, &receiver, !is_element)?;
    if op == O::ElementClassList {
        return host.wrap_platform_object(host.node_platform_key(element), InterfaceBrand::DOM_TOKEN_LIST);
    }
    if matches!(op, O::ElementClassName | O::TokenListValue) {
        let units = class_value(host, element)?;
        return host.string(&units);
    }
    if matches!(op, O::ElementSetClassName | O::TokenListSetValue) {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let units = host.to_dom_string(value)?;
        set_class_value(host, element, units)?;
        return Ok(host.undefined());
    }
    let original = class_value(host, element)?;
    let mut current = tokens(&original);
    if op == O::TokenListLength { return Ok(host.number(current.len() as f64)); }
    if op == O::TokenListSupportedIndices {
        return host.create_array((0..current.len()).map(|index| host.number(index as f64)).collect());
    }
    if matches!(op, O::TokenListItem | O::TokenListIndexedGetter) {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let number = host.to_number(value)?;
        let index = if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as usize } else { 0 };
        return match current.get(index) { Some(token) => host.string(token), None => Ok(host.null()) };
    }
    if op == O::TokenListSupports {
        required_token(host, args, 0)?;
        return Err(host.type_error("This DOMTokenList has no supported tokens"));
    }
    let converted = if matches!(op, O::TokenListAdd | O::TokenListRemove) {
        args.iter().map(|arg| token(host, arg)).collect::<Result<Vec<_>, _>>()?
    } else if op == O::TokenListReplace {
        vec![required_token(host, args, 0)?, required_token(host, args, 1)?]
    } else { vec![required_token(host, args, 0)?] };
    if converted.is_empty() { return Ok(host.undefined()); }
    let first = &converted[0];
    let present = current.contains(first);
    let result = match op {
        O::TokenListContains => return Ok(host.boolean(present)),
        O::TokenListAdd => {
            for token in converted { if !current.contains(&token) { current.push(token); } }
            None
        }
        O::TokenListRemove => {
            current.retain(|token| !converted.contains(token));
            None
        }
        O::TokenListToggle => {
            let force = args.get(1).map(|v| host.to_boolean(v));
            let add = force.unwrap_or(!present);
            if add { if !present { current.push(first.clone()); } }
            else { current.retain(|token| token != first); }
            Some(add)
        }
        O::TokenListReplace => {
            if present {
                let replacement = &converted[1];
                if current.contains(replacement) { current.retain(|token| token != first); }
                else if let Some(slot) = current.iter_mut().find(|token| *token == first) { *slot = replacement.clone(); }
            }
            Some(present)
        }
        _ => unreachable!(),
    };
    set_class_value(host, element, serialize(&current))?;
    Ok(match result { Some(value) => host.boolean(value), None => host.undefined() })
}
