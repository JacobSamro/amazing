use super::*;
use crate::core::{self, Reader};

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
