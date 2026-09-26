use super::*;
use crate::core::{self, Reader};
use amazing_js::{SerializedAtom as Atom, SerializedBoxKind as BoxKind, SerializedHostValue, SerializedNode as Node, SerializedValue};

fn n(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn string(out: &mut Vec<u8>, s: &str) {
    core::string(out, s);
}
fn read_string(r: &mut Reader<'_>) -> Result<String> {
    r.string().map_err(|_| Error::Data)
}
fn read_u32(r: &mut Reader<'_>) -> Result<u32> {
    r.u32().map_err(|_| Error::Data)
}
fn read_u64(r: &mut Reader<'_>) -> Result<u64> {
    r.u64().map_err(|_| Error::Data)
}
fn read_byte(r: &mut Reader<'_>) -> Result<u8> {
    Ok(r.take(1).map_err(|_| Error::Data)?[0])
}
fn read_bytes(r: &mut Reader<'_>) -> Result<Vec<u8>> {
    r.bytes().map_err(|_| Error::Data)
}
fn encode_key(out: &mut Vec<u8>, key: &Key) {
    out.push(key.kind());
    match key {
        Key::Number(x) | Key::Date(x) => out.extend_from_slice(&x.to_bits().to_le_bytes()),
        Key::String(s) => string(out, s),
        Key::Binary(b) => core::bytes(out, b),
        Key::Array(a) => {
            n(out, a.len() as u32);
            for k in a {
                encode_key(out, k)
            }
        }
    }
}
fn decode_key(r: &mut Reader<'_>, depth: u8) -> Result<Key> {
    if depth > 64 {
        return Err(Error::Data);
    }
    Ok(match read_byte(r)? {
        0 => Key::number(f64::from_bits(read_u64(r)?))?,
        1 => Key::date(f64::from_bits(read_u64(r)?))?,
        2 => Key::String(read_string(r)?),
        3 => Key::Binary(read_bytes(r)?),
        4 => {
            let len = read_u32(r)?;
            if len > 1_000_000 {
                return Err(Error::Data);
            }
            let mut a = Vec::new();
            for _ in 0..len {
                a.push(decode_key(r, depth + 1)?)
            }
            Key::Array(a)
        }
        _ => return Err(Error::Data),
    })
}
fn encode_value(out: &mut Vec<u8>, value: &StructuredValue) {
    match value {
        StructuredValue::Graph { wire, .. } => { out.push(10); encode_graph(out, wire); },
        StructuredValue::Hole => out.push(9),
        StructuredValue::Undefined => out.push(8),
        StructuredValue::Null => out.push(0),
        StructuredValue::Bool(v) => {
            out.push(1);
            out.push(*v as u8)
        }
        StructuredValue::Number(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_bits().to_le_bytes())
        }
        StructuredValue::String(v) => {
            out.push(3);
            string(out, v)
        }
        StructuredValue::Date(v) => {
            out.push(4);
            out.extend_from_slice(&v.to_bits().to_le_bytes())
        }
        StructuredValue::Binary(v) => {
            out.push(5);
            core::bytes(out, v)
        }
        StructuredValue::Array(a) => {
            out.push(6);
            n(out, a.len() as u32);
            for v in a {
                encode_value(out, v)
            }
        }
        StructuredValue::Object(o) => {
            out.push(7);
            n(out, o.len() as u32);
            for (k, v) in o {
                string(out, k);
                encode_value(out, v)
            }
        }
    }
}
fn decode_value(r: &mut Reader<'_>, depth: u8) -> Result<StructuredValue> {
    if depth > 64 {
        return Err(Error::Data);
    }
    Ok(match read_byte(r)? {
        10 => { let wire = decode_graph(r)?; let view = Box::new(super::graph::view(&wire)); StructuredValue::Graph { wire, view } },
        9 => StructuredValue::Hole,
        8 => StructuredValue::Undefined,
        0 => StructuredValue::Null,
        1 => StructuredValue::Bool(read_byte(r)? != 0),
        2 => StructuredValue::Number(f64::from_bits(read_u64(r)?)),
        3 => StructuredValue::String(read_string(r)?),
        4 => StructuredValue::Date(f64::from_bits(read_u64(r)?)),
        5 => StructuredValue::Binary(read_bytes(r)?),
        6 => {
            let len = read_u32(r)?;
            if len > 1_000_000 {
                return Err(Error::Data);
            }
            let mut a = Vec::new();
            for _ in 0..len {
                a.push(decode_value(r, depth + 1)?)
            }
            StructuredValue::Array(a)
        }
        7 => {
            let len = read_u32(r)?;
            if len > 1_000_000 {
                return Err(Error::Data);
            }
            let mut o = BTreeMap::new();
            for _ in 0..len {
                o.insert(read_string(r)?, decode_value(r, depth + 1)?);
            }
            StructuredValue::Object(o)
        }
        _ => return Err(Error::Data),
    })
}
fn count(out: &mut Vec<u8>, value: usize) { out.extend_from_slice(&(value as u64).to_le_bytes()); }
fn read_count(r: &mut Reader<'_>) -> Result<usize> {
    usize::try_from(read_u64(r)?).map_err(|_| Error::Data)
}
fn flag(out: &mut Vec<u8>, value: bool) { out.push(u8::from(value)); }
fn read_flag(r: &mut Reader<'_>) -> Result<bool> {
    match read_byte(r)? { 0 => Ok(false), 1 => Ok(true), _ => Err(Error::Data) }
}
fn units(out: &mut Vec<u8>, value: &[u16]) {
    n(out, value.len() as u32);
    for unit in value { out.extend_from_slice(&unit.to_le_bytes()); }
}
fn read_units(r: &mut Reader<'_>) -> Result<Vec<u16>> {
    let len = read_u32(r)? as usize;
    if len > 10_000_000 { return Err(Error::Data); }
    let mut result = Vec::with_capacity(len);
    for _ in 0..len { result.push(u16::from_le_bytes(r.take(2).map_err(|_| Error::Data)?.try_into().unwrap())); }
    Ok(result)
}
fn atom(out: &mut Vec<u8>, value: &Atom) {
    match value {
        Atom::Undefined => out.push(0), Atom::Null => out.push(1),
        Atom::Boolean(value) => { out.push(2); flag(out, *value); },
        Atom::Number(value) => { out.push(3); out.extend_from_slice(&value.to_bits().to_le_bytes()); },
        Atom::BigInt { negative, limbs } => { out.push(4); flag(out, *negative); n(out, limbs.len() as u32); for limb in limbs { n(out, *limb); } },
        Atom::String(value) => { out.push(5); units(out, value); },
        Atom::Reference(index) => { out.push(6); count(out, *index); },
    }
}
fn read_atom(r: &mut Reader<'_>) -> Result<Atom> {
    Ok(match read_byte(r)? {
        0 => Atom::Undefined, 1 => Atom::Null, 2 => Atom::Boolean(read_flag(r)?),
        3 => Atom::Number(f64::from_bits(read_u64(r)?)),
        4 => { let negative = read_flag(r)?; let len = read_u32(r)?; if len > 1_000_000 { return Err(Error::Data); }
            let mut limbs = Vec::new(); for _ in 0..len { limbs.push(read_u32(r)?); } Atom::BigInt { negative, limbs } },
        5 => Atom::String(read_units(r)?), 6 => Atom::Reference(read_count(r)?),
        _ => return Err(Error::Data),
    })
}
fn properties(out: &mut Vec<u8>, values: &[(Vec<u16>, Atom)]) {
    n(out, values.len() as u32);
    for (name, value) in values { units(out, name); atom(out, value); }
}
fn read_properties(r: &mut Reader<'_>) -> Result<Vec<(Vec<u16>, Atom)>> {
    let len = read_u32(r)?; if len > 1_000_000 { return Err(Error::Data); }
    let mut result = Vec::new();
    for _ in 0..len { result.push((read_units(r)?, read_atom(r)?)); }
    Ok(result)
}
fn encode_graph(out: &mut Vec<u8>, value: &SerializedValue) {
    atom(out, &value.root);
    n(out, value.nodes.len() as u32);
    for node in &value.nodes {
        match node {
            Node::Object { properties: values } => { out.push(0); properties(out, values); },
            Node::Array { length, properties: values } => { out.push(1); count(out, *length); properties(out, values); },
            Node::Boxed { kind, value } => { out.push(2); out.push(match kind { BoxKind::Boolean => 0, BoxKind::Number => 1, BoxKind::BigInt => 2, BoxKind::String => 3 }); atom(out, value); },
            Node::Date(value) => { out.push(3); out.extend_from_slice(&value.to_bits().to_le_bytes()); },
            Node::RegExp { pattern, flags } => { out.push(4); units(out, pattern); string(out, flags); },
            Node::ArrayBuffer { bytes, max_byte_length, resizable } => { out.push(5); core::bytes(out, bytes); count(out, *max_byte_length); flag(out, *resizable); },
            Node::ArrayBufferView { buffer, byte_offset, byte_length, kind } => { out.push(6); count(out, *buffer); count(out, *byte_offset); flag(out, byte_length.is_some()); if let Some(value) = byte_length { count(out, *value); } flag(out, kind.is_some()); if let Some(value) = kind { string(out, value); } },
            Node::Map(values) => { out.push(7); n(out, values.len() as u32); for (key, value) in values { atom(out, key); atom(out, value); } },
            Node::Set(values) => { out.push(8); n(out, values.len() as u32); for value in values { atom(out, value); } },
            Node::Error { name, message, cause } => { out.push(9); string(out, name); flag(out, message.is_some()); if let Some(value) = message { units(out, value); } flag(out, cause.is_some()); if let Some(value) = cause { atom(out, value); } },
            Node::Host(value) => { out.push(10); string(out, &value.type_name); core::bytes(out, &value.bytes); },
        }
    }
}
fn decode_graph(r: &mut Reader<'_>) -> Result<SerializedValue> {
    let root = read_atom(r)?;
    let len = read_u32(r)?; if len > 1_000_000 { return Err(Error::Data); }
    let mut nodes = Vec::new();
    for _ in 0..len {
        nodes.push(match read_byte(r)? {
            0 => Node::Object { properties: read_properties(r)? },
            1 => Node::Array { length: read_count(r)?, properties: read_properties(r)? },
            2 => { let kind = match read_byte(r)? { 0 => BoxKind::Boolean, 1 => BoxKind::Number, 2 => BoxKind::BigInt, 3 => BoxKind::String, _ => return Err(Error::Data) }; Node::Boxed { kind, value: read_atom(r)? } },
            3 => Node::Date(f64::from_bits(read_u64(r)?)),
            4 => Node::RegExp { pattern: read_units(r)?, flags: read_string(r)? },
            5 => Node::ArrayBuffer { bytes: read_bytes(r)?, max_byte_length: read_count(r)?, resizable: read_flag(r)? },
            6 => { let buffer = read_count(r)?; let byte_offset = read_count(r)?; let byte_length = read_flag(r)?.then(|| read_count(r)).transpose()?; let kind = read_flag(r)?.then(|| read_string(r)).transpose()?; Node::ArrayBufferView { buffer, byte_offset, byte_length, kind } },
            7 => { let len = read_u32(r)?; if len > 1_000_000 { return Err(Error::Data); } let mut values = Vec::new(); for _ in 0..len { values.push((read_atom(r)?, read_atom(r)?)); } Node::Map(values) },
            8 => { let len = read_u32(r)?; if len > 1_000_000 { return Err(Error::Data); } let mut values = Vec::new(); for _ in 0..len { values.push(read_atom(r)?); } Node::Set(values) },
            9 => { let name = read_string(r)?; let message = read_flag(r)?.then(|| read_units(r)).transpose()?; let cause = read_flag(r)?.then(|| read_atom(r)).transpose()?; Node::Error { name, message, cause } },
            10 => Node::Host(SerializedHostValue { type_name: read_string(r)?, bytes: read_bytes(r)? }),
            _ => return Err(Error::Data),
        });
    }
    let valid = |atom: &Atom| !matches!(atom, Atom::Reference(index) if *index >= nodes.len());
    if !valid(&root) { return Err(Error::Data); }
    for node in &nodes {
        let atoms: Vec<&Atom> = match node {
            Node::Object { properties } | Node::Array { properties, .. } => properties.iter().map(|(_, value)| value).collect(),
            Node::Boxed { value, .. } => vec![value], Node::Map(values) => values.iter().flat_map(|(a,b)| [a,b]).collect(),
            Node::Set(values) => values.iter().collect(), Node::Error { cause: Some(value), .. } => vec![value],
            _ => Vec::new(),
        };
        if atoms.iter().any(|atom| !valid(atom)) { return Err(Error::Data); }
        if let Node::ArrayBufferView { buffer, .. } = node { if *buffer >= nodes.len() { return Err(Error::Data); } }
    }
    Ok(SerializedValue { root, nodes })
}
#[cfg(test)]
mod graph_tests {
    use super::*;

    #[test]
    fn persisted_graph_round_trips_cycles_and_shared_references() {
        let wire = SerializedValue {
            root: Atom::Reference(0),
            nodes: vec![
                Node::Object { properties: vec![
                    ("self".encode_utf16().collect(), Atom::Reference(0)),
                    ("left".encode_utf16().collect(), Atom::Reference(1)),
                    ("right".encode_utf16().collect(), Atom::Reference(1)),
                ] },
                Node::Object { properties: vec![("value".encode_utf16().collect(), Atom::Number(7.0))] },
            ],
        };
        let value = StructuredValue::Graph { view: Box::new(super::super::graph::view(&wire)), wire };
        let mut bytes = Vec::new();
        encode_value(&mut bytes, &value);
        let decoded = decode_value(&mut Reader::new(&bytes), 0).unwrap();
        let StructuredValue::Graph { wire, view } = decoded else { panic!("graph lost on disk"); };
        assert!(matches!(wire.root, Atom::Reference(0)));
        let Node::Object { properties } = &wire.nodes[0] else { panic!("root lost"); };
        assert!(matches!(properties[0].1, Atom::Reference(0)));
        assert!(matches!(properties[1].1, Atom::Reference(1)));
        assert!(matches!(properties[2].1, Atom::Reference(1)));
        assert!(matches!(view.as_ref(), StructuredValue::Object(_)));
    }
}
fn encode_path(out: &mut Vec<u8>, path: &KeyPath) {
    match path {
        KeyPath::Single(p) => {
            out.push(0);
            string(out, p)
        }
        KeyPath::Compound(paths) => {
            out.push(1);
            n(out, paths.len() as u32);
            for p in paths {
                string(out, p)
            }
        }
    }
}
fn decode_path(r: &mut Reader<'_>) -> Result<KeyPath> {
    match read_byte(r)? {
        0 => Ok(KeyPath::Single(read_string(r)?)),
        1 => {
            let len = read_u32(r)?;
            if len > 1_000_000 {
                return Err(Error::Data);
            }
            let mut a = Vec::new();
            for _ in 0..len {
                a.push(read_string(r)?)
            }
            Ok(KeyPath::Compound(a))
        }
        _ => Err(Error::Data),
    }
}
pub(super) fn encode(db: &Database) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"AMIDB001");
    out.extend_from_slice(&db.version.to_le_bytes());
    n(&mut out, db.stores.len() as u32);
    for (name, store) in &db.stores {
        string(&mut out, name);
        out.push(store.key_path.is_some() as u8);
        if let Some(path) = &store.key_path {
            encode_path(&mut out, path)
        }
        out.push(store.auto_increment as u8);
        out.extend_from_slice(&store.generator.to_le_bytes());
        n(&mut out, store.records.len() as u32);
        for (key, value) in &store.records {
            encode_key(&mut out, key);
            encode_value(&mut out, value)
        }
        n(&mut out, store.indexes.len() as u32);
        for (name, index) in &store.indexes {
            string(&mut out, name);
            encode_path(&mut out, &index.key_path);
            out.push(index.unique as u8);
            out.push(index.multi_entry as u8)
        }
    }
    out
}
pub(super) fn decode(data: &[u8]) -> Result<Database> {
    let mut r = Reader::new(data);
    if r.take(8).map_err(|_| Error::Data)? != b"AMIDB001" {
        return Err(Error::Data);
    }
    let version = read_u64(&mut r)?;
    let count = read_u32(&mut r)?;
    if count > 100_000 {
        return Err(Error::Data);
    }
    let mut db = Database::new(version);
    for _ in 0..count {
        let name = read_string(&mut r)?;
        let path = if read_byte(&mut r)? == 1 {
            Some(decode_path(&mut r)?)
        } else {
            None
        };
        let auto = read_byte(&mut r)? != 0;
        let generator = read_u64(&mut r)?;
        let mut store = ObjectStore::new(path, auto)?;
        store.generator = generator;
        let records = read_u32(&mut r)?;
        if records > 1_000_000 {
            return Err(Error::Data);
        }
        for _ in 0..records {
            let key = decode_key(&mut r, 0)?;
            let value = decode_value(&mut r, 0)?;
            if store.records.insert(key, value).is_some() {
                return Err(Error::Data);
            }
        }
        let indexes = read_u32(&mut r)?;
        if indexes > 100_000 {
            return Err(Error::Data);
        }
        for _ in 0..indexes {
            let name = read_string(&mut r)?;
            let path = decode_path(&mut r)?;
            let unique = read_byte(&mut r)? != 0;
            let multi = read_byte(&mut r)? != 0;
            store.create_index(name, path, unique, multi)?;
        }
        if db.stores.insert(name, store).is_some() {
            return Err(Error::Data);
        }
    }
    if !r.done() {
        return Err(Error::Data);
    }
    Ok(db)
}
