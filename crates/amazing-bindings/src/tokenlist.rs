use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{DomError, NodeIdx, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementClassList | O::ElementClassName | O::ElementSetClassName
        | O::TokenListLength | O::TokenListItem | O::TokenListIndexedGetter | O::TokenListContains
        | O::TokenListAdd | O::TokenListRemove | O::TokenListToggle
        | O::TokenListReplace | O::TokenListSupports | O::TokenListValue
        | O::TokenListSetValue)
}
fn element<H: BindingHost>(host: &mut H, receiver: &H::Value, token_list: bool) -> Result<(NodeIdx, String), H::Error> {
    let (node, attribute) = if token_list {
        let key = host.platform_object(receiver, InterfaceBrand::DOM_TOKEN_LIST)
            .ok_or_else(|| host.type_error("Expected a DOMTokenList"))?;
        if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
            return Err(host.type_error("Invalid DOMTokenList"));
        }
        host.with_dom(|dom| dom.token_list_info(key.slot as usize).map(|(node, attribute)| (node, attribute.to_owned()))
            .ok_or(DomError::InvalidStateError))?
    } else {
        (host.node_brand(receiver).ok_or_else(|| host.type_error("Expected an Element"))?, "class".into())
    };
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    Ok((node, attribute))
}
fn attribute_value<H: BindingHost>(host: &mut H, element: NodeIdx, attribute: &str) -> Result<Vec<u16>, H::Error> {
    host.with_dom(|dom| Ok(dom.get_attribute_node(element, None, attribute)
        .and_then(|attr| dom.attr_snapshot(attr).ok()).map_or(Vec::new(), |attr| attr.value)))
}
fn set_attribute_value<H: BindingHost>(host: &mut H, element: NodeIdx, attribute: &str, units: Vec<u16>) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        dom.set_attribute(element, attribute, &String::from_utf16_lossy(&units))?;
        let attr = dom.get_attribute_node(element, None, attribute).ok_or(DomError::InvalidStateError)?;
        dom.store_attr_value_units(attr, units)
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
pub(crate) fn supported_index_count<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<u32, H::Error> {
    let (element, attribute) = element(host, receiver, true)?;
    let count = tokens(&attribute_value(host, element, &attribute)?).len();
    Ok(u32::try_from(count).expect("DOMTokenList length exceeds u32"))
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
    let (element, attribute) = element(host, &receiver, !is_element)?;
    if op == O::ElementClassList {
        let slot = host.with_dom(|dom| dom.token_list(element, "class"))?;
        let mut key = host.node_platform_key(element);
        key.slot = slot as u64;
        return host.wrap_platform_object(key, InterfaceBrand::DOM_TOKEN_LIST);
    }
    if matches!(op, O::ElementClassName | O::TokenListValue) {
        let units = attribute_value(host, element, &attribute)?;
        return host.string(&units);
    }
    if matches!(op, O::ElementSetClassName | O::TokenListSetValue) {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let units = host.to_dom_string(value)?;
        set_attribute_value(host, element, &attribute, units)?;
        return Ok(host.undefined());
    }
    let original = attribute_value(host, element, &attribute)?;
    let mut current = tokens(&original);
    if op == O::TokenListLength { return Ok(host.number(current.len() as f64)); }
    if matches!(op, O::TokenListItem | O::TokenListIndexedGetter) {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let number = host.to_number(value)?;
        let index = if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as usize } else { 0 };
        return match current.get(index) { Some(token) => host.string(token), None => Ok(host.null()) };
    }
    if op == O::TokenListSupports {
        let value = required_token(host, args, 0)?;
        if attribute != "rel" {
            return Err(host.type_error("This DOMTokenList has no supported tokens"));
        }
        let tag = host.with_dom(|dom| Ok(dom.local_name(element).unwrap_or_default().to_owned()))?;
        let keyword: String = value.iter().map(|unit| {
            if (b'A' as u16..=b'Z' as u16).contains(unit) { (*unit as u8 + 32) as char }
            else if *unit <= 0x7f { *unit as u8 as char } else { '\u{fffd}' }
        }).collect();
        let supported = match tag.as_str() {
            "a" | "area" => matches!(keyword.as_str(), "noreferrer" | "noopener" | "opener"),
            "link" => matches!(keyword.as_str(), "alternate" | "dns-prefetch" | "icon"
                | "manifest" | "modulepreload" | "preconnect" | "prefetch"
                | "preload" | "stylesheet"),
            _ => false,
        };
        return Ok(host.boolean(supported));
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
            if force.is_some() && add == present { return Ok(host.boolean(add)); }
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
    set_attribute_value(host, element, &attribute, serialize(&current))?;
    Ok(match result { Some(value) => host.boolean(value), None => host.undefined() })
}
