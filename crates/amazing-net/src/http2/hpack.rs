//! Hand-written RFC7541. All lengths are checked before copying/reserving.
use super::tables::{HUFFMAN, STATIC};
use super::{Error, Field};
use std::collections::VecDeque;

fn bad() -> Error {
    Error::Compression
}
fn integer(input: &[u8], pos: &mut usize, prefix: u8) -> Result<usize, Error> {
    let first = *input.get(*pos).ok_or_else(bad)?;
    *pos += 1;
    let mask = (1usize << prefix) - 1;
    let mut value = usize::from(first) & mask;
    if value < mask {
        return Ok(value);
    }
    let mut shift = 0;
    loop {
        let byte = *input.get(*pos).ok_or_else(bad)?;
        *pos += 1;
        let part = usize::from(byte & 127);
        if shift >= usize::BITS || part > (usize::MAX >> shift) {
            return Err(bad());
        }
        value = value.checked_add(part << shift).ok_or_else(bad)?;
        if byte & 128 == 0 {
            return Ok(value);
        }
        shift += 7;
    }
}
fn emit_integer(out: &mut Vec<u8>, mut value: usize, prefix: u8, tag: u8) {
    let mask = (1usize << prefix) - 1;
    if value < mask {
        out.push(tag | value as u8);
        return;
    }
    out.push(tag | mask as u8);
    value -= mask;
    while value >= 128 {
        out.push((value & 127) as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn huffman(input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    // Static binary trie: at most 1 + sum(code lengths), built once. No
    // attacker-sized allocation or quadratic prefix scan in the decoding loop.
    #[derive(Clone, Copy)]
    struct Node {
        next: [usize; 2],
        symbol: Option<usize>,
    }
    static TRIE: std::sync::OnceLock<Vec<Node>> = std::sync::OnceLock::new();
    let trie = TRIE.get_or_init(|| {
        let empty = Node {
            next: [0; 2],
            symbol: None,
        };
        let mut nodes = vec![empty];
        for (symbol, &(code, bits)) in HUFFMAN.iter().enumerate() {
            let mut at = 0;
            for shift in (0..bits).rev() {
                let bit = ((code >> shift) & 1) as usize;
                if nodes[at].next[bit] == 0 {
                    nodes[at].next[bit] = nodes.len();
                    nodes.push(empty);
                }
                at = nodes[at].next[bit];
            }
            nodes[at].symbol = Some(symbol);
        }
        nodes
    });
    let mut out = Vec::new();
    let mut at = 0;
    let mut tail_bits = 0;
    let mut tail_ones = true;
    for &byte in input {
        for shift in (0..8).rev() {
            let bit = ((byte >> shift) & 1) as usize;
            at = trie[at].next[bit];
            if at == 0 {
                return Err(bad());
            }
            tail_bits += 1;
            tail_ones &= bit == 1;
            if let Some(symbol) = trie[at].symbol {
                if symbol == 256 {
                    return Err(bad());
                } // EOS is never a literal.
                if out.len() == limit {
                    return Err(Error::Limit);
                }
                out.try_reserve_exact(1).map_err(|_| Error::Limit)?;
                out.push(symbol as u8);
                at = 0;
                tail_bits = 0;
                tail_ones = true;
            }
        }
    }
    if tail_bits > 7 || !tail_ones {
        return Err(bad());
    }
    Ok(out)
}
fn string(input: &[u8], pos: &mut usize, limit: usize) -> Result<Vec<u8>, Error> {
    let coded = input.get(*pos).ok_or_else(bad)? & 128 != 0;
    let len = integer(input, pos, 7)?;
    let end = pos.checked_add(len).ok_or_else(bad)?;
    let data = input.get(*pos..end).ok_or_else(bad)?;
    *pos = end;
    if coded {
        huffman(data, limit)
    } else {
        if len > limit {
            return Err(Error::Limit);
        }
        Ok(data.to_vec())
    }
}
#[derive(Default)]
struct Table {
    entries: VecDeque<Field>,
    size: usize,
    max: usize,
}
impl Table {
    fn resize(&mut self, max: usize) {
        self.max = max;
        self.evict(0);
    }
    fn evict(&mut self, incoming: usize) {
        while self.size + incoming > self.max {
            let Some((name, value)) = self.entries.pop_back() else {
                break;
            };
            self.size -= name.len() + value.len() + 32;
        }
    }
    fn insert(&mut self, name: &[u8], value: &[u8]) {
        let size = name.len() + value.len() + 32;
        self.evict(size);
        if size <= self.max {
            self.entries.push_front((name.to_vec(), value.to_vec()));
            self.size += size;
        }
    }
    fn get(&self, index: usize) -> Result<(&[u8], &[u8]), Error> {
        if index == 0 {
            return Err(bad());
        }
        if index <= STATIC.len() {
            return Ok(STATIC[index - 1]);
        }
        self.entries
            .get(index - STATIC.len() - 1)
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
            .ok_or_else(bad)
    }
    fn index(&self, name: &[u8], value: Option<&[u8]>) -> Option<usize> {
        STATIC
            .iter()
            .copied()
            .chain(
                self.entries
                    .iter()
                    .map(|(n, v)| (n.as_slice(), v.as_slice())),
            )
            .position(|(n, v)| n == name && value.is_none_or(|wanted| wanted == v))
            .map(|i| i + 1)
    }
}
pub(super) struct Decoder {
    table: Table,
    ceiling: usize,
}
impl Decoder {
    pub fn new(ceiling: usize) -> Self {
        Self {
            table: Table {
                max: ceiling,
                ..Table::default()
            },
            ceiling,
        }
    }
    pub fn decode(&mut self, input: &[u8], limit: usize) -> Result<Vec<Field>, Error> {
        let mut pos = 0;
        let mut size = 0usize;
        let mut fields = Vec::new();
        let mut seen = false;
        while pos < input.len() {
            let tag = input[pos];
            if tag & 0xe0 == 0x20 {
                if seen {
                    return Err(bad());
                }
                let max = integer(input, &mut pos, 5)?;
                if max > self.ceiling {
                    return Err(bad());
                }
                self.table.resize(max);
                continue;
            }
            seen = true;
            if limit.saturating_sub(size) < 32 {
                return Err(Error::Limit);
            }
            let remaining = limit - size - 32;
            let indexed = tag & 128 != 0;
            let insert = tag & 0xc0 == 0x40;
            let field = if indexed {
                let index = integer(input, &mut pos, 7)?;
                let (name, value) = self.table.get(index)?;
                if name.len() + value.len() > remaining {
                    return Err(Error::Limit);
                }
                (name.to_vec(), value.to_vec())
            } else {
                let index = integer(input, &mut pos, if insert { 6 } else { 4 })?;
                let name = if index == 0 {
                    string(input, &mut pos, remaining)?
                } else {
                    let (name, _) = self.table.get(index)?;
                    if name.len() > remaining {
                        return Err(Error::Limit);
                    }
                    name.to_vec()
                };
                let value = string(input, &mut pos, remaining - name.len())?;
                (name, value)
            };
            size += field.0.len() + field.1.len() + 32;
            if insert {
                self.table.insert(&field.0, &field.1);
            }
            fields.push(field);
        }
        Ok(fields)
    }
}
pub(super) struct Encoder {
    table: Table,
    pending_min: Option<usize>,
    pending: Option<usize>,
}
impl Encoder {
    pub fn new() -> Self {
        Self {
            table: Table {
                max: 4096,
                ..Table::default()
            },
            pending_min: None,
            pending: None,
        }
    }
    pub fn resize(&mut self, max: usize) {
        let max = max.min(4096);
        self.table.resize(max);
        self.pending_min = Some(self.pending_min.unwrap_or(max).min(max));
        self.pending = Some(max);
    }
    pub fn encode(&mut self, fields: &[Field], limit: usize) -> Result<Vec<u8>, Error> {
        let size = fields
            .iter()
            .try_fold(0usize, |sum, (n, v)| {
                sum.checked_add(n.len())?
                    .checked_add(v.len())?
                    .checked_add(32)
            })
            .ok_or(Error::Limit)?;
        if size > limit {
            return Err(Error::Limit);
        }
        let mut out = Vec::new();
        // Uncompressed literals plus integer overhead fit within list accounting
        // (32 bytes per field), plus at most two size updates of six bytes each.
        out.try_reserve_exact(size.checked_add(12).ok_or(Error::Limit)?)
            .map_err(|_| Error::Limit)?;
        if let Some(min) = self.pending_min.take() {
            emit_integer(&mut out, min, 5, 0x20);
            let last = self.pending.take().unwrap();
            if last != min {
                emit_integer(&mut out, last, 5, 0x20);
            }
        }
        for (name, value) in fields {
            let sensitive = matches!(
                name.as_slice(),
                b"authorization" | b"cookie" | b"proxy-authorization"
            );
            if !sensitive {
                if let Some(index) = self.table.index(name, Some(value)) {
                    emit_integer(&mut out, index, 7, 0x80);
                    continue;
                }
            }
            let index = self.table.index(name, None).unwrap_or(0);
            emit_integer(
                &mut out,
                index,
                if sensitive { 4 } else { 6 },
                if sensitive { 0x10 } else { 0x40 },
            );
            if index == 0 {
                emit_integer(&mut out, name.len(), 7, 0);
                out.extend_from_slice(name);
            }
            emit_integer(&mut out, value.len(), 7, 0);
            out.extend_from_slice(value);
            if !sensitive {
                self.table.insert(name, value);
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests;
