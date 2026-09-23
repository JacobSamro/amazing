//! CharacterData's UTF-16 indexed methods and Text.splitText.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, NodeIdx};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::CharacterDataSubstring | O::CharacterDataAppend
        | O::CharacterDataInsert | O::CharacterDataDelete
        | O::CharacterDataReplace | O::TextSplit)
}

fn unsigned<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<usize, H::Error> {
    let value = value.ok_or_else(|| host.type_error("Not enough arguments"))?;
    let number = host.to_number(value)?;
    let value = if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.) as u32 } else { 0 };
    Ok(value as usize)
}

fn required_string<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<Vec<u16>, H::Error> {
    let value = value.ok_or_else(|| host.type_error("Not enough arguments"))?;
    host.to_dom_string(value)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let node: NodeIdx = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected CharacterData"))?;
    host.with_dom(|dom| {
        if node.0 as usize >= dom.node_count() { return Err(DomError::InvalidNode); }
        if dom.character_data(node).is_none() || (op == O::TextSplit && !dom.is_text(node)) {
            return Err(DomError::TypeError);
        }
        Ok(())
    })?;
    let offset = match op {
        O::CharacterDataAppend => None,
        _ => Some(unsigned(host, args.first())?),
    };
    let count = match op {
        O::CharacterDataSubstring | O::CharacterDataDelete | O::CharacterDataReplace =>
            Some(unsigned(host, args.get(1))?),
        _ => None,
    };
    let value = match op {
        O::CharacterDataAppend => required_string(host, args.first())?,
        O::CharacterDataInsert => required_string(host, args.get(1))?,
        O::CharacterDataReplace => required_string(host, args.get(2))?,
        _ => Vec::new(),
    };
    match op {
        O::CharacterDataSubstring => {
            let units = host.with_dom(|dom| {
                let data = dom.character_data_utf16(node).unwrap();
                let offset = offset.unwrap();
                if offset > data.len() { return Err(DomError::IndexSizeError); }
                Ok(data[offset..offset + count.unwrap().min(data.len() - offset)].to_vec())
            })?;
            host.string(&units)
        }
        O::TextSplit => {
            let new = host.with_dom(|dom| dom.split_text(node, offset.unwrap()))?;
            host.wrap_node(new)
        }
        _ => {
            host.with_dom(|dom| {
                let offset = offset.unwrap_or_else(|| dom.character_data_utf16(node).unwrap().len());
                dom.replace_character_data(node, offset, count.unwrap_or(0), &value)
            })?;
            Ok(host.undefined())
        }
    }
}
