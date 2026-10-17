//! Binding table of one environment frame.
//!
//! Most frames (function bodies, blocks, loop iterations) declare a handful of
//! names. A linear scan over a short vector compares a length and a few bytes
//! per entry and beats hashing the looked-up name on every identifier read.
//! Large frames (the global environment, module scopes) switch to a hash map.
use crate::env::BindingCell;
use crate::fast_hash::FastMap;

const SMALL_LIMIT: usize = 12;

pub(crate) enum VarTable {
    Small(Vec<(String, BindingCell)>),
    Large(FastMap<String, BindingCell>),
}

impl Default for VarTable {
    fn default() -> Self { VarTable::Small(Vec::new()) }
}

impl VarTable {
    pub(crate) fn len(&self) -> usize {
        match self { VarTable::Small(v) => v.len(), VarTable::Large(m) => m.len() }
    }

    #[inline]
    pub(crate) fn get(&self, name: &str) -> Option<&BindingCell> {
        match self {
            VarTable::Small(v) => v.iter().find(|(k, _)| k.len() == name.len() && k.as_bytes() == name.as_bytes()).map(|(_, c)| c),
            VarTable::Large(m) => m.get(name),
        }
    }

    pub(crate) fn contains_key(&self, name: &str) -> bool { self.get(name).is_some() }

    fn grow_if_full(&mut self) {
        if let VarTable::Small(v) = self {
            if v.len() >= SMALL_LIMIT {
                let mut map: FastMap<String, BindingCell> = FastMap::default();
                map.extend(std::mem::take(v));
                *self = VarTable::Large(map);
            }
        }
    }

    pub(crate) fn insert(&mut self, name: String, cell: BindingCell) -> Option<BindingCell> {
        if let VarTable::Small(v) = self {
            if let Some(slot) = v.iter_mut().find(|(k, _)| *k == name) {
                return Some(std::mem::replace(&mut slot.1, cell));
            }
        }
        self.grow_if_full();
        match self {
            VarTable::Small(v) => { v.push((name, cell)); None }
            VarTable::Large(m) => m.insert(name, cell),
        }
    }

    pub(crate) fn remove(&mut self, name: &str) -> Option<BindingCell> {
        match self {
            VarTable::Small(v) => {
                let at = v.iter().position(|(k, _)| k == name)?;
                Some(v.remove(at).1)
            }
            VarTable::Large(m) => m.remove(name),
        }
    }

    pub(crate) fn or_insert(&mut self, name: String, cell: BindingCell) {
        if !self.contains_key(&name) { self.insert(name, cell); }
    }

    pub(crate) fn or_insert_with(&mut self, name: String, make: impl FnOnce() -> BindingCell) {
        if !self.contains_key(&name) { self.insert(name, make()); }
    }
}

impl std::ops::Index<&str> for VarTable {
    type Output = BindingCell;
    fn index(&self, name: &str) -> &BindingCell { self.get(name).expect("binding exists") }
}
