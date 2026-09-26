//! The IDB key view is derived from the JS structured clone graph. The graph
//! remains authoritative for values returned to script, including cycles.
use super::{Error, Key, Result, StructuredValue};
use amazing_js::{SerializedAtom as Atom, SerializedNode as Node, SerializedValue};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn view(wire: &SerializedValue) -> StructuredValue {
    project(&wire.root, wire, &mut BTreeSet::new())
}

fn project(atom: &Atom, wire: &SerializedValue, visiting: &mut BTreeSet<usize>) -> StructuredValue {
    match atom {
        Atom::Undefined => StructuredValue::Undefined,
        Atom::Null => StructuredValue::Null,
        Atom::Boolean(value) => StructuredValue::Bool(*value),
        Atom::Number(value) => StructuredValue::Number(*value),
        Atom::String(value) => StructuredValue::String(String::from_utf16_lossy(value)),
        Atom::BigInt { .. } => StructuredValue::Undefined,
        Atom::Reference(index) => {
            if !visiting.insert(*index) { return StructuredValue::Undefined; }
            let result = match wire.nodes.get(*index) {
                Some(Node::Object { properties }) => StructuredValue::Object(properties.iter()
                    .map(|(name, value)| (String::from_utf16_lossy(name), project(value, wire, visiting))).collect()),
                Some(Node::Array { length, properties }) => {
                    let mut values = vec![StructuredValue::Hole; (*length).min(1_000_000)];
                    for (name, value) in properties {
                        if let Ok(index) = String::from_utf16_lossy(name).parse::<usize>() {
                            if index < values.len() { values[index] = project(value, wire, visiting); }
                        }
                    }
                    StructuredValue::Array(values)
                }
                Some(Node::Date(value)) => StructuredValue::Date(*value),
                Some(Node::ArrayBuffer { bytes, .. }) => StructuredValue::Binary(bytes.clone()),
                Some(Node::ArrayBufferView { buffer, byte_offset, byte_length, .. }) => {
                    match wire.nodes.get(*buffer) {
                        Some(Node::ArrayBuffer { bytes, .. }) => {
                            let end = byte_length.map_or(bytes.len(), |length| byte_offset.saturating_add(length)).min(bytes.len());
                            StructuredValue::Binary(bytes.get(*byte_offset..end).unwrap_or_default().to_vec())
                        }
                        _ => StructuredValue::Undefined,
                    }
                }
                Some(Node::Host(host)) => project_host(host),
                _ => StructuredValue::Undefined,
            };
            visiting.remove(index);
            result
        }
    }
}

fn project_host(host: &amazing_js::SerializedHostValue) -> StructuredValue {
    let mut bytes = host.bytes.as_slice();
    let mut field = || {
        if bytes.len() < 4 { return None; }
        let len = u32::from_le_bytes(bytes[..4].try_into().ok()?) as usize;
        bytes = &bytes[4..];
        if bytes.len() < len { return None; }
        let value = bytes[..len].to_vec();
        bytes = &bytes[len..];
        Some(value)
    };
    let Some(data) = field() else { return StructuredValue::Undefined; };
    let Some(content_type) = field().and_then(|value| String::from_utf8(value).ok()) else { return StructuredValue::Undefined; };
    let mut values = BTreeMap::from([
        ("size".into(), StructuredValue::Number(data.len() as f64)),
        ("type".into(), StructuredValue::String(content_type)),
    ]);
    if host.type_name == "File" {
        let Some(name) = field().and_then(|value| String::from_utf8(value).ok()) else { return StructuredValue::Undefined; };
        drop(field);
        if bytes.len() != 8 { return StructuredValue::Undefined; }
        values.insert("name".into(), StructuredValue::String(name));
        values.insert("lastModified".into(), StructuredValue::Number(i64::from_le_bytes(bytes.try_into().unwrap()) as f64));
    }
    StructuredValue::Object(values)
}

fn key_atom(wire: &mut SerializedValue, key: &Key) -> Atom {
    match key {
        Key::Number(value) => Atom::Number(*value),
        Key::String(value) => Atom::String(value.encode_utf16().collect()),
        Key::Date(value) => {
            let index = wire.nodes.len();
            wire.nodes.push(Node::Date(*value));
            Atom::Reference(index)
        }
        Key::Binary(bytes) => {
            let index = wire.nodes.len();
            wire.nodes.push(Node::ArrayBuffer { bytes: bytes.clone(), max_byte_length: bytes.len(), resizable: false });
            Atom::Reference(index)
        }
        Key::Array(keys) => {
            let properties = keys.iter().enumerate().map(|(index, key)|
                (index.to_string().encode_utf16().collect(), key_atom(wire, key))).collect();
            let index = wire.nodes.len();
            wire.nodes.push(Node::Array { length: keys.len(), properties });
            Atom::Reference(index)
        }
    }
}

pub(super) fn assign_key(wire: &mut SerializedValue, path: &str, key: &Key) -> Result<()> {
    let mut current = match wire.root { Atom::Reference(index) => index, _ => return Err(Error::Data) };
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let name: Vec<u16> = part.encode_utf16().collect();
        if parts.peek().is_none() {
            let value = key_atom(wire, key);
            let Some(Node::Object { properties }) = wire.nodes.get_mut(current) else { return Err(Error::Data); };
            if let Some((_, existing)) = properties.iter_mut().find(|(field, _)| field == &name) { *existing = value; }
            else { properties.push((name, value)); }
            return Ok(());
        }
        let existing = match wire.nodes.get(current) {
            Some(Node::Object { properties }) => properties.iter().find(|(field, _)| field == &name).map(|(_, value)| value.clone()),
            _ => return Err(Error::Data),
        };
        current = match existing {
            Some(Atom::Reference(index)) => index,
            Some(_) => return Err(Error::Data),
            None => {
                let index = wire.nodes.len();
                wire.nodes.push(Node::Object { properties: Vec::new() });
                let Some(Node::Object { properties }) = wire.nodes.get_mut(current) else { return Err(Error::Data); };
                properties.push((name, Atom::Reference(index)));
                index
            }
        };
    }
    Err(Error::Data)
}
