//! A bounded byte codec for `amazing_js::SerializedValue` (the HTML
//! structured-clone graph), for the demo JS realm's cross-process
//! `postMessage`/`MessagePort` proof (`demo_realm`, IPC-11c follow-up).
//!
//! `amazing`'s driver has its own copy of this exact codec
//! (`navigator::state_wire`), private to that crate; this one exists because
//! amazing-process cannot depend on the `amazing` binary crate (the
//! dependency runs the other way). Keep the two in lock-step if the
//! structured-clone node/atom set changes.
use amazing_ipc::codec::{self, ByteBuf, Decoder, Encoder, Error, Wire};
use amazing_js::{
    SerializedAtom as Atom, SerializedBoxKind as BoxKind, SerializedHostValue,
    SerializedNode as Node, SerializedValue,
};

fn size(value: usize) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::Limit)
}
fn index(value: u32) -> usize {
    value as usize
}
fn number(value: f64, e: &mut Encoder) -> Result<(), Error> {
    value.to_bits().encode(e)
}
fn read_number(d: &mut Decoder<'_>) -> Result<f64, Error> {
    Ok(f64::from_bits(u64::decode(d)?))
}
fn atom(value: &Atom, e: &mut Encoder) -> Result<(), Error> {
    match value {
        Atom::Undefined => 0u8.encode(e),
        Atom::Null => 1u8.encode(e),
        Atom::Boolean(v) => {
            2u8.encode(e)?;
            v.encode(e)
        }
        Atom::Number(v) => {
            3u8.encode(e)?;
            number(*v, e)
        }
        Atom::BigInt { negative, limbs } => {
            4u8.encode(e)?;
            negative.encode(e)?;
            limbs.encode(e)
        }
        Atom::String(v) => {
            5u8.encode(e)?;
            v.encode(e)
        }
        Atom::Reference(v) => {
            6u8.encode(e)?;
            size(*v)?.encode(e)
        }
    }
}
fn read_atom(d: &mut Decoder<'_>) -> Result<Atom, Error> {
    Ok(match u8::decode(d)? {
        0 => Atom::Undefined,
        1 => Atom::Null,
        2 => Atom::Boolean(bool::decode(d)?),
        3 => Atom::Number(read_number(d)?),
        4 => Atom::BigInt {
            negative: bool::decode(d)?,
            limbs: Vec::<u32>::decode(d)?,
        },
        5 => Atom::String(Vec::<u16>::decode(d)?),
        6 => Atom::Reference(index(u32::decode(d)?)),
        _ => return Err(Error::Invalid),
    })
}
fn atoms(values: &[Atom], e: &mut Encoder) -> Result<(), Error> {
    size(values.len())?.encode(e)?;
    for value in values {
        atom(value, e)?;
    }
    Ok(())
}
fn read_atoms(d: &mut Decoder<'_>) -> Result<Vec<Atom>, Error> {
    let len = u32::decode(d)? as usize;
    if len > codec::MAX_ITEMS || len > d.remaining() {
        return Err(Error::Limit);
    }
    (0..len).map(|_| read_atom(d)).collect()
}
fn props(values: &[(Vec<u16>, Atom)], e: &mut Encoder) -> Result<(), Error> {
    size(values.len())?.encode(e)?;
    for (key, value) in values {
        key.encode(e)?;
        atom(value, e)?;
    }
    Ok(())
}
fn read_props(d: &mut Decoder<'_>) -> Result<Vec<(Vec<u16>, Atom)>, Error> {
    let len = u32::decode(d)? as usize;
    if len > codec::MAX_ITEMS || len > d.remaining() / 2 {
        return Err(Error::Limit);
    }
    (0..len)
        .map(|_| Ok((Vec::<u16>::decode(d)?, read_atom(d)?)))
        .collect()
}
fn node(value: &Node, e: &mut Encoder) -> Result<(), Error> {
    match value {
        Node::Object { properties } => {
            0u8.encode(e)?;
            props(properties, e)
        }
        Node::Array { length, properties } => {
            1u8.encode(e)?;
            size(*length)?.encode(e)?;
            props(properties, e)
        }
        Node::Boxed { kind, value } => {
            2u8.encode(e)?;
            (match kind {
                BoxKind::Boolean => 0u8,
                BoxKind::Number => 1,
                BoxKind::BigInt => 2,
                BoxKind::String => 3,
            })
            .encode(e)?;
            atom(value, e)
        }
        Node::Date(value) => {
            3u8.encode(e)?;
            number(*value, e)
        }
        Node::RegExp { pattern, flags } => {
            4u8.encode(e)?;
            pattern.encode(e)?;
            flags.encode(e)
        }
        Node::ArrayBuffer {
            bytes,
            max_byte_length,
            resizable,
        } => {
            5u8.encode(e)?;
            ByteBuf(bytes.clone()).encode(e)?;
            size(*max_byte_length)?.encode(e)?;
            resizable.encode(e)
        }
        Node::ArrayBufferView {
            buffer,
            byte_offset,
            byte_length,
            kind,
        } => {
            6u8.encode(e)?;
            size(*buffer)?.encode(e)?;
            size(*byte_offset)?.encode(e)?;
            byte_length.map(size).transpose()?.encode(e)?;
            kind.encode(e)
        }
        Node::Map(entries) => {
            7u8.encode(e)?;
            size(entries.len())?.encode(e)?;
            for (key, value) in entries {
                atom(key, e)?;
                atom(value, e)?;
            }
            Ok(())
        }
        Node::Set(entries) => {
            8u8.encode(e)?;
            atoms(entries, e)
        }
        Node::Error {
            name,
            message,
            cause,
        } => {
            9u8.encode(e)?;
            name.encode(e)?;
            message.encode(e)?;
            match cause {
                Some(v) => {
                    true.encode(e)?;
                    atom(v, e)
                }
                None => false.encode(e),
            }
        }
        Node::Host(value) => {
            10u8.encode(e)?;
            value.type_name.encode(e)?;
            ByteBuf(value.bytes.clone()).encode(e)
        }
    }
}
fn read_node(d: &mut Decoder<'_>) -> Result<Node, Error> {
    Ok(match u8::decode(d)? {
        0 => Node::Object {
            properties: read_props(d)?,
        },
        1 => Node::Array {
            length: index(u32::decode(d)?),
            properties: read_props(d)?,
        },
        2 => Node::Boxed {
            kind: match u8::decode(d)? {
                0 => BoxKind::Boolean,
                1 => BoxKind::Number,
                2 => BoxKind::BigInt,
                3 => BoxKind::String,
                _ => return Err(Error::Invalid),
            },
            value: read_atom(d)?,
        },
        3 => Node::Date(read_number(d)?),
        4 => Node::RegExp {
            pattern: Vec::<u16>::decode(d)?,
            flags: String::decode(d)?,
        },
        5 => Node::ArrayBuffer {
            bytes: ByteBuf::decode(d)?.0,
            max_byte_length: index(u32::decode(d)?),
            resizable: bool::decode(d)?,
        },
        6 => Node::ArrayBufferView {
            buffer: index(u32::decode(d)?),
            byte_offset: index(u32::decode(d)?),
            byte_length: Option::<u32>::decode(d)?.map(index),
            kind: Option::<String>::decode(d)?,
        },
        7 => {
            let len = u32::decode(d)? as usize;
            if len > codec::MAX_ITEMS || len > d.remaining() / 2 {
                return Err(Error::Limit);
            }
            Node::Map(
                (0..len)
                    .map(|_| Ok((read_atom(d)?, read_atom(d)?)))
                    .collect::<Result<_, Error>>()?,
            )
        }
        8 => Node::Set(read_atoms(d)?),
        9 => {
            let name = String::decode(d)?;
            let message = Option::<Vec<u16>>::decode(d)?;
            let cause = if bool::decode(d)? {
                Some(read_atom(d)?)
            } else {
                None
            };
            Node::Error {
                name,
                message,
                cause,
            }
        }
        10 => Node::Host(SerializedHostValue {
            type_name: String::decode(d)?,
            bytes: ByteBuf::decode(d)?.0,
        }),
        _ => return Err(Error::Invalid),
    })
}
fn valid_atom(atom: &Atom, count: usize) -> bool {
    !matches!(atom, Atom::Reference(index) if *index >= count)
}
pub fn encode(value: &SerializedValue) -> Result<Vec<u8>, String> {
    let mut e = Encoder::new();
    1u16.encode(&mut e).map_err(|e| e.to_string())?;
    atom(&value.root, &mut e).map_err(|e| e.to_string())?;
    size(value.nodes.len())
        .map_err(|e| e.to_string())?
        .encode(&mut e)
        .map_err(|e| e.to_string())?;
    for value in &value.nodes {
        node(value, &mut e).map_err(|e| e.to_string())?;
    }
    let bytes = e.finish();
    if bytes.len() > crate::history::MAX_STATE_BYTES {
        return Err("structured-clone message limit".into());
    }
    Ok(bytes)
}
pub fn decode(bytes: &[u8]) -> Result<SerializedValue, String> {
    if bytes.len() > crate::history::MAX_STATE_BYTES {
        return Err("structured-clone message limit".into());
    }
    let mut d = Decoder::new(bytes).map_err(|e| e.to_string())?;
    if u16::decode(&mut d).map_err(|e| e.to_string())? != 1 {
        return Err("structured-clone message version".into());
    }
    let root = read_atom(&mut d).map_err(|e| e.to_string())?;
    let len = u32::decode(&mut d).map_err(|e| e.to_string())? as usize;
    if len > codec::MAX_ITEMS || len > d.remaining() {
        return Err("structured-clone message node limit".into());
    }
    let nodes = (0..len)
        .map(|_| read_node(&mut d))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    d.finish().map_err(|e| e.to_string())?;
    if !valid_atom(&root, len) {
        return Err("structured-clone message reference".into());
    }
    for node in &nodes {
        let valid = match node {
            Node::Object { properties } | Node::Array { properties, .. } => {
                properties.iter().all(|(_, v)| valid_atom(v, len))
            }
            Node::Boxed { value, .. } => valid_atom(value, len),
            Node::Map(entries) => entries
                .iter()
                .all(|(k, v)| valid_atom(k, len) && valid_atom(v, len)),
            Node::Set(entries) => entries.iter().all(|v| valid_atom(v, len)),
            Node::Error { cause, .. } => cause.as_ref().is_none_or(|v| valid_atom(v, len)),
            Node::ArrayBufferView { buffer, .. } => *buffer < len,
            _ => true,
        };
        if !valid {
            return Err("structured-clone message reference".into());
        }
    }
    Ok(SerializedValue { root, nodes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_js::Interpreter;

    #[test]
    fn round_trips_a_plain_object_through_the_wire() {
        let mut source = Interpreter::new();
        let value = source
            .run_script("({greeting: 'hi', n: 41, nested: {ok: true}})")
            .unwrap();
        let serialized = source.structured_serialize(&value).unwrap();
        let bytes = encode(&serialized).unwrap();
        let restored = decode(&bytes).unwrap();
        let mut target = Interpreter::new();
        let value = target.structured_deserialize(&restored).unwrap();
        target.global_obj.borrow_mut().set_own("v", value.clone());
        target.global_env.declare_var("v", value).unwrap();
        assert_eq!(
            target
                .run_script("v.greeting === 'hi' && v.n === 41 && v.nested.ok === true")
                .unwrap(),
            amazing_js::value::JsValue::Bool(true)
        );
    }

    #[test]
    fn rejects_a_bad_reference_without_panicking() {
        let invalid = SerializedValue {
            root: Atom::Reference(3),
            nodes: Vec::new(),
        };
        // encode() doesn't validate references (decode()'s job, matching
        // amazing::navigator::state_wire); the wire bytes it produces must
        // still be rejected on the way back in.
        let bytes = encode(&invalid).unwrap();
        assert!(decode(&bytes).is_err());
    }
}
