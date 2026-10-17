// Portions ported from boa-dev/boa (core/engine/src/property/mod.rs,
// core/engine/src/builtins/math/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Completed descriptor kind adapted to a scalar tag without changing value roots.
//! The ECMAScript value model: language values, objects, and the abstract
//! coercion operations (spec §6, §7.1).
//!
//! Design decisions (locked):
//! - [`JsValue::Object`] is `Rc<RefCell<JsObject>>`: shared mutable identity
//!   is observable (`a === b` for the same object; mutation through one
//!   alias visible via another) and `Rc<RefCell>` is std-only.
//! - Property storage is an insertion-ordered `Vec` of entries (JSRT-20), with a
//!   side `HashMap<PropertyKey, usize>` giving O(1) lookup by key. The `Vec`
//!   alone still gives spec key order (for-in, `Object.keys`) for free via
//!   `own_property_keys`'s single classify-and-sort pass; the map exists purely
//!   to avoid the O(n) linear scan a lookup/define/delete used to pay on every
//!   call (previously O(n) per access, O(n^2) to build an n-property object).
//!   `props` itself is never tombstoned or compacted -- slot indices stay plain
//!   `usize` positions, which `amazing-js/src/vm/fast_path.rs`'s inline-cache
//!   slot accessors (`own_data_slot`/`own_slot_index`/`set_own_data_slot`)
//!   depend on; interior deletion shifts (`Vec::remove`), while tail deletion
//!   pops without moving other slots. Both bump `shape_version`, invalidating
//!   any cached slot.
//! - Arrays are ordinary objects whose `"length"` property is kept in sync
//!   by the array operations; there is no dense-elements fast path to
//!   invalidate.
//! - Functions carry their defining environment (closure), parameters, and
//!   body as cloned AST — AST nodes are small relative to source text and
//!   cloning keeps ownership simple.
//!
//! Deferred (documented): BigInt (literals parse but evaluate to a thrown
//! error), Proxies, getters/setters on ordinary objects (accessor
//! storage exists in [`Property`] but is only exercised by class accessors),
//! integer-indexed exotics, shared array buffers.

use std::cell::RefCell;
use crate::fast_hash::FastMap;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ast;

/// ECMAScript string data. The payload is UTF-16, including unpaired surrogates.
/// Conversion to Rust text is explicitly lossy and is only for diagnostics or
/// host interfaces that cannot represent arbitrary JavaScript strings.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsString(Rc<[u16]>);

impl JsString {
    pub fn from_units(units: impl Into<Rc<[u16]>>) -> Self { Self(units.into()) }
    pub fn units(&self) -> &[u16] { &self.0 }
    pub fn len(&self) -> usize { self.0.len() }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    pub fn to_utf8_lossy(&self) -> String { String::from_utf16_lossy(&self.0) }
    pub fn is_well_formed(&self) -> bool { String::from_utf16(&self.0).is_ok() }
    pub fn to_well_formed(&self) -> Self {
        Self::from(String::from_utf16_lossy(&self.0))
    }
}

/// Short strings that name properties and bindings (`length`, `prototype`,
/// identifiers from source) are converted from `&str` on hot paths. A small
/// direct-mapped per-thread cache hands back one shared `Rc<[u16]>` instead
/// of allocating and copying each time; a miss just replaces the slot, so
/// memory is bounded and a stream of unique strings costs one hash and store.
/// The table sits in `ManuallyDrop` so thread exit never decrements strings
/// that another (coroutine-handoff) thread may still be using.
const KEY_CACHE_SLOTS: usize = 1024;
const KEY_CACHE_MAX_BYTES: usize = 32;

thread_local! {
    static KEY_CACHE: std::mem::ManuallyDrop<RefCell<Vec<Option<JsString>>>> =
        const { std::mem::ManuallyDrop::new(RefCell::new(Vec::new())) };
}

thread_local! {
    static INTERNER: std::mem::ManuallyDrop<RefCell<Option<FastMap<Box<str>, JsString>>>> =
        const { std::mem::ManuallyDrop::new(RefCell::new(None)) };
}
const INTERNER_LIMIT: usize = 200_000;

impl JsString {
    /// Shared handle for a source identifier or engine-literal name. Equal
    /// names intern to one allocation, so comparing two interned strings is a
    /// pointer check and cloning is a reference-count bump. The table is
    /// per-thread and bounded; past the bound it hands out fresh strings.
    pub fn interned(value: &str) -> JsString {
        INTERNER.with(|table| {
            let mut table = table.borrow_mut();
            let map = table.get_or_insert_with(FastMap::default);
            if let Some(hit) = map.get(value) { return hit.clone(); }
            let fresh = units_from_str(value);
            if map.len() < INTERNER_LIMIT { map.insert(value.into(), fresh.clone()); }
            fresh
        })
    }
}

const INDEX_CACHE_SLOTS: usize = 4096;

thread_local! {
    static INDEX_CACHE: std::mem::ManuallyDrop<RefCell<Vec<Option<JsString>>>> =
        const { std::mem::ManuallyDrop::new(RefCell::new(Vec::new())) };
}

impl JsString {
    /// Canonical decimal text of an array index. Small indices share cached
    /// strings; larger ones are built directly in UTF-16 with one allocation.
    pub fn from_index(index: u32) -> JsString {
        let build = || {
            let mut digits = [0u16; 10];
            let mut at = digits.len();
            let mut rest = index;
            loop {
                at -= 1;
                digits[at] = 0x30 + (rest % 10) as u16;
                rest /= 10;
                if rest == 0 { break; }
            }
            JsString::from_units(&digits[at..])
        };
        if (index as usize) >= INDEX_CACHE_SLOTS { return build(); }
        INDEX_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.is_empty() { cache.resize(INDEX_CACHE_SLOTS, None); }
            cache[index as usize].get_or_insert_with(build).clone()
        })
    }
}

fn units_from_str(value: &str) -> JsString {
    if value.is_ascii() {
        // Exact-size iterator: one allocation, no intermediate Vec.
        JsString(value.bytes().map(u16::from).collect())
    } else {
        JsString::from_units(value.encode_utf16().collect::<Vec<_>>())
    }
}

impl From<&str> for JsString {
    fn from(value: &str) -> Self {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > KEY_CACHE_MAX_BYTES || !value.is_ascii() {
            return units_from_str(value);
        }
        let mut hash = bytes.len() as u32;
        for &byte in bytes {
            hash = (hash ^ byte as u32).wrapping_mul(0x0100_0193);
        }
        let slot = (hash ^ (hash >> 15)) as usize & (KEY_CACHE_SLOTS - 1);
        KEY_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.is_empty() {
                cache.resize(KEY_CACHE_SLOTS, None);
            }
            if let Some(hit) = &cache[slot] {
                let units = hit.units();
                if units.len() == bytes.len() && units.iter().zip(bytes).all(|(&unit, &byte)| unit == byte as u16) {
                    return hit.clone();
                }
            }
            let fresh = units_from_str(value);
            cache[slot] = Some(fresh.clone());
            fresh
        })
    }
}
impl From<String> for JsString {
    fn from(value: String) -> Self { Self::from(value.as_str()) }
}
impl From<&String> for JsString {
    fn from(value: &String) -> Self { Self::from(value.as_str()) }
}
impl From<Rc<str>> for JsString {
    fn from(value: Rc<str>) -> Self { Self::from(value.as_ref()) }
}
impl From<&JsString> for JsString {
    fn from(value: &JsString) -> Self { value.clone() }
}
impl PartialEq<&str> for JsString {
    fn eq(&self, other: &&str) -> bool { self.units().iter().copied().eq(other.encode_utf16()) }
}
impl PartialEq<String> for JsString {
    fn eq(&self, other: &String) -> bool { self.units().iter().copied().eq(other.encode_utf16()) }
}
impl fmt::Display for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.to_utf8_lossy()) }
}
impl fmt::Debug for JsString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.to_utf8_lossy().fmt(f) }
}

static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1024);

/// A Symbol primitive. Identity is independent of its optional description.
#[derive(Clone)]
pub struct JsSymbol {
    id: u64,
    description: Option<JsString>,
}

impl JsSymbol {
    pub fn new(description: Option<JsString>) -> Self {
        Self { id: NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed), description }
    }
    pub fn description(&self) -> Option<&JsString> { self.description.as_ref() }
    pub fn descriptive_units(&self) -> Vec<u16> {
        let mut units: Vec<u16> = "Symbol(".encode_utf16().collect();
        if let Some(description) = &self.description { units.extend_from_slice(description.units()); }
        units.push(b')' as u16);
        units
    }
    pub fn descriptive_string(&self) -> String {
        format!("Symbol({})", self.description.as_ref().map(JsString::to_utf8_lossy).unwrap_or_default())
    }
}
impl PartialEq for JsSymbol { fn eq(&self, other: &Self) -> bool { self.id == other.id } }
impl Eq for JsSymbol {}
impl std::hash::Hash for JsSymbol {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) { self.id.hash(state); }
}
impl fmt::Debug for JsSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.descriptive_string()) }
}

/// Well-known Symbols have stable identity across interpreters in this agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WellKnownSymbol {
    Iterator, AsyncIterator, HasInstance, ToPrimitive, ToStringTag,
    IsConcatSpreadable, Species, Unscopables, Match, MatchAll, Replace, Search, Split,
    Dispose, AsyncDispose,
}

impl WellKnownSymbol {
    pub const ALL: [Self; 15] = [
        Self::Iterator, Self::AsyncIterator, Self::HasInstance, Self::ToPrimitive,
        Self::ToStringTag, Self::IsConcatSpreadable, Self::Species, Self::Unscopables,
        Self::Match, Self::MatchAll, Self::Replace, Self::Search, Self::Split,
        Self::Dispose, Self::AsyncDispose,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Iterator => "iterator", Self::AsyncIterator => "asyncIterator",
            Self::HasInstance => "hasInstance", Self::ToPrimitive => "toPrimitive",
            Self::ToStringTag => "toStringTag", Self::IsConcatSpreadable => "isConcatSpreadable",
            Self::Species => "species", Self::Unscopables => "unscopables",
            Self::Match => "match", Self::MatchAll => "matchAll", Self::Replace => "replace",
            Self::Search => "search", Self::Split => "split",
            Self::Dispose => "dispose", Self::AsyncDispose => "asyncDispose",
        }
    }
    pub fn symbol(self) -> JsSymbol {
        let id = match self {
            Self::Iterator => 1, Self::AsyncIterator => 2, Self::HasInstance => 3,
            Self::ToPrimitive => 4, Self::ToStringTag => 5, Self::IsConcatSpreadable => 6,
            Self::Species => 7, Self::Unscopables => 8, Self::Match => 9,
            Self::MatchAll => 10, Self::Replace => 11, Self::Search => 12, Self::Split => 13,
            Self::Dispose => 14, Self::AsyncDispose => 15,
        };
        JsSymbol { id, description: Some(JsString::from(format!("Symbol.{}", self.name()))) }
    }
}

/// ECMAScript property keys are either UTF-16 strings or Symbol identities.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PropertyKey { String(JsString), Symbol(JsSymbol) }
impl PropertyKey {
    pub fn as_string(&self) -> Option<&JsString> {
        match self { Self::String(value) => Some(value), Self::Symbol(_) => None }
    }
}
impl From<JsString> for PropertyKey { fn from(value: JsString) -> Self { Self::String(value) } }
impl From<&JsString> for PropertyKey { fn from(value: &JsString) -> Self { Self::String(value.clone()) } }
impl From<&str> for PropertyKey { fn from(value: &str) -> Self { Self::String(value.into()) } }
impl From<String> for PropertyKey { fn from(value: String) -> Self { Self::String(value.into()) } }
impl From<&String> for PropertyKey { fn from(value: &String) -> Self { Self::String(value.into()) } }
impl From<JsSymbol> for PropertyKey { fn from(value: JsSymbol) -> Self { Self::Symbol(value) } }
impl From<&JsSymbol> for PropertyKey { fn from(value: &JsSymbol) -> Self { Self::Symbol(value.clone()) } }
impl From<&PropertyKey> for PropertyKey { fn from(value: &PropertyKey) -> Self { value.clone() } }

/// A JavaScript value (spec §6.1).
#[derive(Clone)]
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    BigInt(Rc<crate::interp::BinaryBigInt>),
    Str(JsString),
    Symbol(JsSymbol),
    Object(Obj),
}

/// Shared handle to a [`JsObject`].
#[derive(Clone)]
pub struct Obj(pub Rc<RefCell<JsObject>>);

/// Opaque identity supplied by an embedding for a live host object. The
/// runtime adds its own realm identity before storing this in an internal slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostHandle {
    pub arena: u64,
    pub slot: u64,
    pub generation: u64,
    pub brand: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostIdentity {
    pub realm: u64,
    pub handle: HostHandle,
}

impl Obj {
    /// Borrow the object (never held across user-code calls).
    pub fn borrow(&self) -> std::cell::Ref<'_, JsObject> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, JsObject> {
        self.0.borrow_mut()
    }
}

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Summarize rather than recurse (property graphs can be cyclic).
        let b = self.borrow();
        match (&b.exotic, &b.func) {
            (_, Some(func)) => write!(f, "[function {}]", func.name),
            (Exotic::Array, _) => write!(f, "[array; {} props]", b.props.len()),
            (Exotic::Arguments(_), _) => write!(f, "[mapped arguments]"),
            (Exotic::TypedArray, _) => write!(f, "[typed array]"),
            (Exotic::Error, _) => write!(f, "[error]"),
            (Exotic::Map(_), _) => write!(f, "[map]"),
            (Exotic::Set(_), _) => write!(f, "[set]"),
            (Exotic::RegExp(_), _) => write!(f, "[regexp]"),
            (Exotic::Promise(_), _) => write!(f, "[promise]"),
            (Exotic::LegacyIterator(_), _) => write!(f, "[array iterator]"),
            (Exotic::PairIterator(_), _) => write!(f, "[pair iterator]"),
            (Exotic::SequenceIterator(_), _) => write!(f, "[sequence iterator]"),
            (Exotic::CollectionIterator(_), _) => write!(f, "[collection iterator]"),
            (Exotic::SymbolObject(_), _) => write!(f, "[symbol object]"),
            (Exotic::Generator(_), _) => write!(f, "[generator]"),
            (Exotic::ThreadGenerator(_), _) => write!(f, "[generator]"),
            (Exotic::ModuleNamespace(_) | Exotic::DeferredModuleNamespace(_), _) => write!(f, "[module namespace]"),
            (Exotic::ModuleSource(_), _) => write!(f, "[module source]"),
            (Exotic::ModuleCallback(_), _) => write!(f, "[module continuation]"),
            (Exotic::ShadowRealm(_), _) => write!(f, "[ShadowRealm]"),
            (Exotic::WindowProxy(_), _) => write!(f, "[WindowProxy]"),
            (Exotic::Proxy(_), _) => write!(f, "[proxy]"),
            (Exotic::DisposableStack(_), _) => write!(f, "[disposable stack]"),
            (Exotic::Ordinary, _) => write!(f, "[object; {} props]", b.props.len()),
        }
    }
}

impl PartialEq for JsValue {
    /// Same-object identity for objects, structural for primitives
    /// (`+0 === -0`, NaN never equals itself — spec §7.2.13/§7.2.14 are
    /// implemented at operator level; this impl is strict equality).
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (JsValue::Undefined, JsValue::Undefined) | (JsValue::Null, JsValue::Null) => true,
            (JsValue::Bool(a), JsValue::Bool(b)) => a == b,
            (JsValue::Num(a), JsValue::Num(b)) => a == b, // f64 eq: NaN!=NaN, +0==-0
            (JsValue::BigInt(a), JsValue::BigInt(b)) => a == b,
            (JsValue::Str(a), JsValue::Str(b)) => a == b,
            (JsValue::Symbol(a), JsValue::Symbol(b)) => a == b,
            (JsValue::Object(a), JsValue::Object(b)) => Rc::ptr_eq(&a.0, &b.0),
            _ => false,
        }
    }
}

impl fmt::Debug for JsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsValue::Undefined => write!(f, "undefined"),
            JsValue::Null => write!(f, "null"),
            JsValue::Bool(b) => write!(f, "{b}"),
            JsValue::Num(n) => write!(f, "{n}"),
            JsValue::BigInt(n) => write!(f, "{}n", n.to_radix_string(10)),
            JsValue::Str(s) => write!(f, "{s:?}"),
            JsValue::Symbol(s) => write!(f, "{s:?}"),
            JsValue::Object(_) => write!(f, "[object]"),
        }
    }
}

impl JsValue {
    pub fn str(s: impl Into<JsString>) -> JsValue {
        JsValue::Str(s.into())
    }

    pub fn str_units(units: impl Into<Rc<[u16]>>) -> JsValue {
        JsValue::Str(JsString::from_units(units))
    }

    pub fn num(n: impl Into<f64>) -> JsValue {
        JsValue::Num(n.into())
    }

    /// A fresh ordinary object with `Object.prototype` set later by the
    /// caller (the global object owns that prototype).
    pub fn object(obj: JsObject) -> JsValue {
        JsValue::Object(Obj(Rc::new(RefCell::new(obj))))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, JsValue::Undefined)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsValue::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, JsValue::Object(_))
    }

    /// The string payload when this IS a string.
    pub fn as_str(&self) -> Option<&JsString> {
        match self {
            JsValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Whether this is a string (used by the additive operator's branch).
    pub fn is_str(&self) -> bool {
        self.as_str().is_some()
    }

    /// §7.1.2 ToBoolean: everything except `undefined`, `null`, `false`,
    /// `0`/`NaN`, and `""` is true.
    pub fn to_boolean(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Num(n) => *n != 0.0 && !n.is_nan(),
            JsValue::BigInt(n) => !n.is_zero(),
            JsValue::Str(s) => !s.is_empty(),
            JsValue::Symbol(_) => true,
            // Annex B.3.6.1: an [[IsHTMLDDA]] object is falsy.
            JsValue::Object(o) => o.0.try_borrow().map(|o| !o.is_html_dda).unwrap_or(true),
        }
    }

    /// §7.1.3 ToNumber for PRIMITIVES only. Objects must first go through
    /// ToPrimitive (§7.1.1), which can invoke user code (`valueOf`/
    /// `toString`) and therefore lives in `interp.rs` where calls are
    /// possible — calling into an interpreter from here would be a panic
    /// path, and panics on hostile input are forbidden.
    pub fn to_number(&self) -> Result<f64, Throw> {
        match self {
            JsValue::Undefined => Ok(f64::NAN),
            JsValue::Null => Ok(0.0),
            JsValue::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            JsValue::Num(n) => Ok(*n),
            JsValue::BigInt(_) => Err(Throw::type_error("cannot convert BigInt to number")),
            JsValue::Str(s) => Ok(string_to_number(&s.to_utf8_lossy())),
            JsValue::Symbol(_) => Err(Throw::type_error("cannot convert Symbol to number")),
            JsValue::Object(_) => Err(Throw::type_error(
                "object passed to primitive ToNumber without conversion",
            )),
        }
    }
}

/// §7.1.4.1.1 StringToNumber: trimmed, empty -> 0, "Infinity"/"+Infinity"/
/// "-Infinity" special-cased, else strict decimal grammar (no hex here —
/// that's literal-level). Anything else is NaN.
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim_matches(|c: char| c.is_ws_js());
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => {
            if let Some((radix, digits)) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B"))
                .map(|digits| (2u32, digits))
                .or_else(|| t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")).map(|digits| (8, digits)))
                .or_else(|| t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).map(|digits| (16, digits))) {
                if digits.is_empty() { return f64::NAN; }
                let mut value = 0.0;
                for digit in digits.bytes() {
                    let Some(digit) = (digit as char).to_digit(radix) else { return f64::NAN; };
                    value = value * radix as f64 + digit as f64;
                }
                return value;
            }
            let bytes = t.as_bytes();
            let mut at = 0;
            if bytes.first().is_some_and(|byte| matches!(byte, b'+' | b'-')) { at += 1; }
            let mut digits = 0;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; digits += 1; }
            if bytes.get(at) == Some(&b'.') {
                at += 1;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; digits += 1; }
            }
            if digits == 0 { return f64::NAN; }
            if bytes.get(at).is_some_and(|byte| matches!(byte, b'e' | b'E')) {
                at += 1;
                if bytes.get(at).is_some_and(|byte| matches!(byte, b'+' | b'-')) { at += 1; }
                let exponent_start = at;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) { at += 1; }
                if at == exponent_start { return f64::NAN; }
            }
            if at == bytes.len() { t.parse::<f64>().unwrap_or(f64::NAN) } else { f64::NAN }
        }
    }
}

trait JsWs {
    /// The exact JS whitespace + line-terminator set accepted by
    /// StringToNumber's trim (spec: WhiteSpace ∪ LineTerminator).
    fn is_ws_js(&self) -> bool;
}

impl JsWs for char {
    fn is_ws_js(&self) -> bool {
        matches!(
            self,
            '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
                ..='\u{200A}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202F}'
                    | '\u{205F}'
                    | '\u{3000}'
                    | '\u{FEFF}'
        )
    }
}

/// A runtime throw. Any [`JsValue`] can be thrown (`throw "str"`); this
/// newtype lets it ride `Result` without clashing with other error types.
#[derive(Debug, Clone)]
pub struct Throw(pub JsValue);

impl Throw {
    /// Builds an `Error`-shaped object throw with the given message, for
    /// internal spec-mandated errors (TypeErrors, ReferenceErrors, ...).
    /// The constructor wiring lives in `builtin.rs`; this fallback makes a
    /// plain object with `.name`/`.message` so even pre-global scripts see
    /// something inspectable.
    pub fn error(name: &str, message: impl Into<String>) -> Throw {
        let mut obj = JsObject::ordinary();
        obj.exotic = Exotic::Error;
        obj.set_own("name", JsValue::str(name));
        obj.set_own("message", JsValue::str(message.into()));
        Throw(JsValue::object(obj))
    }

    pub fn type_error(message: impl Into<String>) -> Throw {
        Throw::error("TypeError", message)
    }

    pub fn reference_error(message: impl Into<String>) -> Throw {
        Throw::error("ReferenceError", message)
    }

    pub fn syntax_error(message: impl Into<String>) -> Throw {
        Throw::error("SyntaxError", message)
    }

    pub fn range_error(message: impl Into<String>) -> Throw {
        Throw::error("RangeError", message)
    }

    /// Human summary for console output / debugging.
    pub fn describe(&self) -> String {
        match &self.0 {
            JsValue::Object(o) => {
                // `name` (and a default `message`) live on Error.prototype, so
                // look them up through the chain, as Error.prototype.toString does.
                let b = o.borrow();
                match (b.lookup("name"), b.lookup("message")) {
                    (Some(JsValue::Str(n)), Some(JsValue::Str(m))) if m.is_empty() => n.to_string(),
                    (Some(JsValue::Str(n)), Some(JsValue::Str(m))) => format!("{n}: {m}"),
                    (Some(JsValue::Str(n)), _) => n.to_string(),
                    _ => "[object]".into(),
                }
            }
            v => to_display_string(v),
        }
    }
}

/// §7.1.17 ToString → display string (console/debugging flavor; identical
/// for primitives, `[object Object]`-style for plain objects).
pub fn to_display_string(v: &JsValue) -> String {
    match v {
        JsValue::Undefined => "undefined".into(),
        JsValue::Null => "null".into(),
        JsValue::Bool(b) => b.to_string(),
        JsValue::Num(n) => number_to_string(*n),
        JsValue::BigInt(n) => n.to_radix_string(10),
        JsValue::Str(s) => s.to_string(),
        JsValue::Symbol(s) => s.descriptive_string(),
        JsValue::Object(_) => "[object Object]".into(),
    }
}

/// §7.1.12.1 Number::toString(x, 10): shortest round-tripping digits
/// (Rust's `{:e}` provides those), then the spec's fixed/exponential split:
/// fixed for 1e-6 <= |x| < 1e21, exponential outside.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == 0.0 {
        return "0".into(); // ToString(-0) is "0"
    }
    if n < 0.0 {
        return format!("-{}", number_to_string(-n));
    }
    if n.is_infinite() {
        return "Infinity".into();
    }
    // Shortest digits + decimal exponent: mantissa like "1.2345", exp e.
    let sci = format!("{:e}", n); // e.g. "1.23456e2"
    let (mantissa, exp) = match sci.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().unwrap_or(0)),
        None => return sci, // unreachable for finite f64; pass through
    };
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.ends_with('0') {
        digits.pop();
    }
    let k = digits.len() as i32;
    let point = exp + 1; // value == 0.<digits> × 10^point (spec's n)
    if k <= point && point <= 21 {
        // Digits followed by zeros ("120" -> "120", "1e20" -> 1 + 20 zeros).
        let mut s = digits;
        for _ in k..point {
            s.push('0');
        }
        s
    } else if 0 < point && point <= 21 {
        // Point inside the digit string ("123.456").
        let p = point as usize;
        format!("{}.{}", &digits[..p], &digits[p..])
    } else if -6 < point && point <= 0 {
        // "0." then zeros then digits ("0.0005").
        let mut s = String::from("0.");
        for _ in 0..-point {
            s.push('0');
        }
        s.push_str(&digits);
        s
    } else {
        // Exponential: "1e+21"-style but JS prints lowercase "e+21";
        // note ToString uses "e+21"/"e-7".
        let rest = &digits[1..];
        let frac = if rest.is_empty() { "" } else { "." };
        format!(
            "{}{}{}e{}{}",
            &digits[..1],
            frac,
            rest,
            if point - 1 >= 0 { "+" } else { "-" },
            (point - 1).abs()
        )
    }
}

/// Number::exponentiate (ECMA-262 §6.1.6.1.3). Rust's powf differs from the
/// language operation for NaN exponents and infinite powers of ±1.
#[inline]
pub fn number_exponentiate(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() || (base.abs() == 1.0 && exponent.is_infinite()) {
        f64::NAN
    } else {
        base.powf(exponent)
    }
}

/// A completed own property descriptor. Its kind remains distinct even when
/// an accessor has neither a getter nor a setter.
#[derive(Debug, Clone)]
pub struct Property {
    pub accessor: bool,
    pub value: JsValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
    /// Original callable objects retain strictness, identity, and private slots.
    pub getter: Option<JsValue>,
    pub setter: Option<JsValue>,
}

impl Property {
    /// Classification of a completed own descriptor. Callers should use this
    /// operation rather than infer the descriptor kind from individual slots.
    pub fn is_accessor(&self) -> bool {
        self.accessor
    }

    pub fn is_data(&self) -> bool {
        !self.is_accessor()
    }

    pub fn accessor(getter: Option<JsValue>, setter: Option<JsValue>) -> Property {
        Property { accessor: true, value: JsValue::Undefined, writable: false,
            enumerable: false, configurable: true, getter, setter }
    }

    /// Writable/enumerable/configurable defaults for assignment-created
    /// and literal-created data properties (true/true/true).
    pub fn data(value: JsValue) -> Property {
        Property {
            accessor: false,
            value,
            writable: true,
            enumerable: true,
            configurable: true,
            getter: None,
            setter: None,
        }
    }
}

/// What kind of callable lives inside an object.
#[derive(Debug, Clone)]
pub struct JsFunction {
    pub source: crate::source_text::SourceMetadata,
    /// Name for stack traces and `fn.name`.
    pub name: String,
    /// Parameter list incl. defaults/rest flags (cloned from the AST).
    pub params: Vec<ast::Param>,
    pub body: FunctionBody,
    /// Arrow functions capture `this` lexically and cannot be constructors.
    pub is_arrow: bool,
    pub is_async: bool,
    pub is_generator: bool,
    /// Environment in force where the function was defined.
    pub env: crate::env::EnvHandle,
    /// For methods: the home object prototype chain used by `super`.
    pub home: Option<Obj>,
    /// Constructor functions get this as their instances' prototype.
    pub proto_for_new: Option<Obj>,
}

impl JsFunction {
    /// Snapshot for a call or construct. Natives and bytecode carry their code
    /// by cheap handle and never read the name, source text or parameter list
    /// at call time, so those (a `String` and a deep `Vec<Param>` clone) are
    /// left empty. AST-bodied functions keep the full clone.
    pub(crate) fn clone_for_call(&self) -> JsFunction {
        if matches!(&self.body, FunctionBody::Block(_) | FunctionBody::Expr(_)) { return self.clone(); }
        JsFunction {
            source: Default::default(),
            name: String::new(),
            params: Vec::new(),
            body: self.body.clone(),
            is_arrow: self.is_arrow,
            is_async: self.is_async,
            is_generator: self.is_generator,
            env: self.env.clone(),
            home: self.home.clone(),
            proto_for_new: self.proto_for_new.clone(),
        }
    }
}

/// Internal class-constructor metadata kept on the function object, so the
/// embedding-facing `JsFunction` initializer remains independent of classes.
#[derive(Debug, Clone)]
pub(crate) struct DerivedConstructor {
    pub parent: JsValue,
    pub implicit: bool,
}

/// A lexically-scoped private name (`#x`). Identity is the `Rc` pointer, not
/// the text: two classes that each spell a field `#x` get distinct,
/// non-interchangeable names, matching §15.7's per-class private
/// environment. The text (with its leading `#`) is kept only for
/// diagnostics and for building the class's private-name lookup map.
#[derive(Clone)]
pub struct PrivateName(pub(crate) Rc<str>);

impl PrivateName {
    pub fn new(text: impl Into<Rc<str>>) -> PrivateName {
        PrivateName(text.into())
    }
    /// Includes the leading `#`.
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl PartialEq for PrivateName {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for PrivateName {}
impl std::hash::Hash for PrivateName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Rc::as_ptr(&self.0) as *const () as usize).hash(state);
    }
}
impl std::fmt::Debug for PrivateName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrivateName({})", self.0)
    }
}

/// A field/method/accessor whose brand is a [`PrivateName`] rather than a
/// [`PropertyKey`]. Kept off `JsObject::props` entirely (never a
/// `PropertyKey`, never enumerable, never visible to reflection) in a flat
/// `Vec` on the object itself, per §6.2.10's PrivateElement record.
#[derive(Debug, Clone)]
pub struct PrivateElement {
    pub name: PrivateName,
    pub kind: PrivateElementKind,
}

#[derive(Debug, Clone)]
pub enum PrivateElementKind {
    Field(JsValue),
    /// Shared by reference across every instance of the declaring class.
    Method(JsValue),
    Accessor {
        get: Option<JsValue>,
        set: Option<JsValue>,
    },
}

/// One instance-scoped class element, replayed into every new instance at
/// `InitializeInstanceElements` time (§15.7.11) in declaration order,
/// interleaved regardless of field/private-method kind.
#[derive(Debug, Clone)]
pub(crate) enum InstanceElement {
    Field {
        key: FieldKey,
        init: Option<ast::Expr>,
        name_key: Option<FieldKey>,
        initializers: Vec<JsValue>,
        extra_initializers: Vec<JsValue>,
    },
    PrivateMethod {
        name: PrivateName,
        value: JsValue,
    },
    PrivateAccessor {
        name: PrivateName,
        get: Option<JsValue>,
        set: Option<JsValue>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum FieldKey {
    Public(PropertyKey),
    Private(PrivateName),
}

/// One static-side class element, run once in declaration order right after
/// the class's own methods are installed, with `this` bound to the
/// constructor itself.
#[derive(Debug, Clone)]
pub(crate) enum StaticElement {
    Field {
        key: FieldKey,
        init: Option<ast::Expr>,
        name_key: Option<FieldKey>,
        initializers: Vec<JsValue>,
        extra_initializers: Vec<JsValue>,
    },
    PrivateMethod {
        name: PrivateName,
        value: JsValue,
    },
    PrivateAccessor {
        name: PrivateName,
        get: Option<JsValue>,
        set: Option<JsValue>,
    },
    Block(ast::Block),
}

/// Per-class instance-element template plus the environment private names
/// inside it (and its methods, and its field initializers) resolve against,
/// and the class's own prototype as the [[HomeObject]] `super` resolves
/// through inside a field initializer (matching an ordinary instance method).
#[derive(Debug, Clone)]
pub(crate) struct ClassElements {
    pub instance: Vec<InstanceElement>,
    pub method_extra_initializers: Vec<JsValue>,
    pub class_env: crate::env::EnvHandle,
    pub home: Obj,
}

/// Function body: block for declarations/functions/methods, expression for
/// concise arrows, native for built-ins.
#[derive(Debug, Clone)]
pub enum FunctionBody {
    Block(ast::Block),
    Expr(ast::Expr),
    Bytecode(crate::interp::vm::FunctionCode),
    /// Rust-implemented built-in (`Math.abs`, `Array.prototype.push`, ...).
    Native(NativeFn),
    /// Per-interpreter host operation, independent of AST and VM frame layout.
    HostNative(HostFunctionId, HostCallPolicy, u64),
}

/// An interpreter-local key for an embedding's native dispatcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostFunctionId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCallPolicy {
    CallOnly,
    ConstructOnly,
    /// The host owns allocation and GetPrototypeFromConstructor completely.
    ConstructOnlyAllocates,
    CallAndConstruct,
}

/// Signature of built-in implementations: receiver (`this`), arguments.
pub type NativeFn =
    fn(&mut crate::interp::Interpreter, JsValue, Vec<JsValue>) -> Result<JsValue, Throw>;

/// Exotic object flavors. Everything not listed is Ordinary.
#[derive(Debug, Clone, Default)]
pub enum Exotic {
    #[default]
    Ordinary,
    /// Private same-agent identity, never an authored property.
    ShadowRealm(crate::interp::RealmHandle),
    WindowProxy(Rc<RefCell<crate::interp::WindowProxyState>>),
    Array,
    /// Cell-only aliases preserve escaped arguments without retaining a whole environment.
    Arguments(HashMap<PropertyKey, crate::env::BindingCell>),
    TypedArray,
    Error,
    Map(MapState),
    Set(SetState),
    Promise(PromiseState),
    RegExp(Rc<amazing_regex::Regex>),
    LegacyIterator(LegacyIteratorState),
    PairIterator(PairIteratorState),
    SequenceIterator(SequenceIteratorState),
    CollectionIterator(CollectionIteratorState),
    SymbolObject(JsSymbol),
    Generator(Rc<RefCell<crate::interp::vm::GeneratorState>>),
    /// A generator/async-generator instance driven by a pooled-OS-thread coroutine on the
    /// default (tree-walking) execution path, as opposed to `Generator`'s bytecode-VM frame
    /// (JSVM-8; see `crates/amazing-js/src/interp_coroutine.rs`).
    ThreadGenerator(Rc<RefCell<crate::interp::ThreadGeneratorState>>),
    /// Object-owned genuine compiled source; its record contains only source/AST data.
    ModuleSource(Rc<crate::module::CompiledModuleSource>),
    ModuleNamespace(std::collections::BTreeMap<String, crate::env::BindingCell>),
    DeferredModuleNamespace(Rc<DeferredNamespaceState>),
    ModuleCallback(Rc<ModuleCallbackState>),
    Proxy(Rc<crate::interp::ProxyState>),
    DisposableStack(DisposableStackState),
}

/// `DisposableStack` / `AsyncDisposableStack` internal state (explicit resource management,
/// spec 11.4). `resources` runs LIFO on `dispose()`/`disposeAsync()`. The resource-record shape
/// is adapted from lucid-softworks/lumen's `Disposable`/stack-entry design (crates/lumen/src/
/// {interpreter,builtins/disposable}.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT, onto
/// this codebase's `Exotic`-variant internal-slot convention (see `interp_disposable.rs`).
#[derive(Debug, Clone, Default)]
pub struct DisposableStackState {
    pub is_async: bool,
    pub disposed: bool,
    pub resources: Vec<DisposableStackEntry>,
    /// An async stack saw a null/undefined `use` value: disposal must still await once.
    pub needs_await: bool,
}

/// One `use`/`adopt`/`defer` entry: `method` is called as `method.call(this_arg, ..arg)`.
/// `use(v)`: method = v's resolved dispose method, this_arg = v, arg = None.
/// `adopt(v, onDispose)`: method = onDispose, this_arg = Undefined, arg = Some(v).
/// `defer(onDispose)`: method = onDispose, this_arg = Undefined, arg = None.
#[derive(Debug, Clone)]
pub struct DisposableStackEntry {
    pub method: JsValue,
    pub this_arg: JsValue,
    pub arg: Option<JsValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceIteratorKind { ArrayKeys, ArrayValues, ArrayEntries, String }

#[derive(Debug, Clone)]
pub struct SequenceIteratorState {
    pub target: JsValue,
    pub next_index: usize,
    pub kind: SequenceIteratorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionIteratorKind { MapEntries, SetValues }

#[derive(Debug, Clone)]
pub struct CollectionIteratorState {
    pub target: Obj,
    pub next_index: usize,
    pub kind: CollectionIteratorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyIteratorKind { Keys, Values, Entries }

#[derive(Debug, Clone)]
pub struct LegacyIteratorState {
    pub target: Obj,
    pub next_index: u32,
    pub kind: LegacyIteratorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairIteratorKind { Keys, Values, Entries }

#[derive(Debug, Clone)]
pub struct PairIteratorState {
    pub target: Obj,
    pub next_index: u32,
    pub kind: PairIteratorKind,
}

/// Map internals: insertion-order key/value store keyed by display string
/// (key identity for -0/+0 and object keys follows SameValueZero via the
/// normalized key below).
#[derive(Debug, Clone, Default)]
pub struct MapState {
    pub entries: Vec<(MapKey, JsValue)>,
}

/// Map/Set keys need SameValueZero semantics (§7.2.10): NaN matches NaN,
/// +0 matches -0, objects match by identity. Derived `PartialEq` on an
/// f64 arm would break the first two, so equality is hand-written.
#[derive(Debug, Clone)]
pub enum MapKey {
    Str(JsString),
    Symbol(JsSymbol),
    Num(f64),
    BigInt(Rc<crate::interp::BinaryBigInt>),
    Bool(bool),
    Null,
    Undefined,
    Object(Obj),
}

impl MapKey {
    /// SameValueZero comparison.
    pub fn same(&self, other: &MapKey) -> bool {
        match (self, other) {
            (MapKey::Str(a), MapKey::Str(b)) => a == b,
            (MapKey::Symbol(a), MapKey::Symbol(b)) => a == b,
            // NaN === NaN under SameValueZero; -0 and +0 are one key.
            (MapKey::Num(a), MapKey::Num(b)) => (a.is_nan() && b.is_nan()) || (*a == *b),
            (MapKey::BigInt(a), MapKey::BigInt(b)) => a == b,
            (MapKey::Bool(a), MapKey::Bool(b)) => a == b,
            (MapKey::Null, MapKey::Null) => true,
            (MapKey::Undefined, MapKey::Undefined) => true,
            (MapKey::Object(a), MapKey::Object(b)) => Rc::ptr_eq(&a.0, &b.0),
            _ => false,
        }
    }

    /// The string form used for for-in style enumeration of Map contents.
    pub fn describe(&self) -> String {
        match self {
            MapKey::Str(s) => s.to_string(),
            MapKey::Symbol(symbol) => symbol.descriptive_string(),
            MapKey::Num(n) => number_to_string(*n),
            MapKey::BigInt(n) => n.to_radix_string(10),
            MapKey::Bool(b) => b.to_string(),
            MapKey::Null => "null".into(),
            MapKey::Undefined => "undefined".into(),
            MapKey::Object(_) => "[object Object]".into(),
        }
    }
}

impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}

/// Set internals: same key discipline as Map, no values.
#[derive(Debug, Clone, Default)]
pub struct SetState {
    pub entries: Vec<MapKey>,
}

/// Promise state (spec §27.6): pending with reaction lists, or settled.
#[derive(Debug, Clone, Default)]
pub struct PromiseState {
    pub state: PromiseFate,
    pub fulfill_reactions: Vec<PendingReaction>,
    pub reject_reactions: Vec<PendingReaction>,
    /// True while a resolve function is running (unhandled-rejection book-
    /// keeping is deferred).
    pub already_resolved: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum PromiseFate {
    #[default]
    Pending,
    Fulfilled(JsValue),
    Rejected(JsValue),
}

/// One queued Promise reaction: callback and optional result capability.
#[derive(Debug, Clone)]
pub struct PendingReaction {
    pub handler: Option<JsValue>,
    /// HostMakeJobCallback's active script; metadata only, no GC edges.
    pub source_context: Option<Rc<str>>,
    /// Chained .then result; internal await reactions have no result capability.
    pub capability: Option<Obj>,
    /// `true` when this is the reject path of a `then(on_ful, on_rej)` pair.
    pub is_reject_handler: bool,
}

/// Objects with at most this many own properties have no hash index: lookups
/// scan `props` directly. Invariant: `prop_index` is non-empty only while
/// `props.len() > SMALL_OBJECT_PROPS`, and then indexes every property.
const SMALL_OBJECT_PROPS: usize = 8;

/// A JavaScript object: own properties in insertion order, a prototype
/// slot, and an exotic payload.
#[derive(Debug, Clone)]
pub struct JsObject {
    props: Vec<(PropertyKey, Property)>,
    /// `key -> index into props`, kept in sync by every mutator below
    /// (JSRT-20). O(1) lookup; `props` stays the source of truth for order
    /// and for slot-index stability (see the module doc comment above).
    prop_index: FastMap<PropertyKey, usize>,
    /// Changes when the indexed layout or a property's descriptor changes.
    pub(crate) shape_version: u64,
    /// Object literals constructed by the VM can use ordinary-own-property caches.
    pub(crate) vm_cacheable: bool,
    /// Set once any engine-private `[[...]]` named property was stored here
    /// (DOM wrappers keep their node ids that way). Such objects always take
    /// the full property-access path.
    internal_slots: bool,
    /// Creation realm for callable dispatch and intrinsic allocation.
    pub(crate) realm: Option<u64>,
    pub proto: Option<Obj>,
    pub exotic: Exotic,
    pub(crate) arguments_object: bool,
    /// Engine-private primitive wrapper data; never a JavaScript property key.
    pub(crate) boxed_primitive: Option<JsValue>,
    /// Validated primitive JSON text; only JSON.rawJSON creates this brand.
    pub(crate) raw_json: Option<JsString>,
    /// Whether new own properties may be created.
    pub extensible: bool,
    /// Callable payload; `Some` iff this object is a function.
    pub func: Option<JsFunction>,
    /// True once this object has been invoked via `new` as a constructor
    /// (cheap stand-in for the internal Slots checks the spec performs).
    pub constructed: bool,
    /// Built-in methods are callable but lack [[Construct]].
    pub(crate) constructible: bool,
    pub(crate) class_constructor: bool,
    pub(crate) derived_constructor: Option<DerivedConstructor>,
    /// Instance-element template replayed into each new instance; `None` for
    /// a class (or plain function) with no instance fields/private methods.
    pub(crate) class_elements: Option<Rc<ClassElements>>,
    /// PrivateElement records (§6.2.10): fields/methods/accessors branded by
    /// a [`PrivateName`] instead of a [[PropertyKey]]. Never enumerable,
    /// never a `PropertyKey`, never visible to reflection.
    pub(crate) private_elements: Vec<PrivateElement>,
    pub(crate) lexical_super_constructor: Option<JsValue>,
    pub(crate) lexical_new_target: Option<JsValue>,
    /// Syntactic permissions inherited by an arrow, independent of actual new.target.
    pub(crate) lexical_new_target_allowed: bool,
    pub(crate) lexical_class_initializer: bool,
    /// Engine-private brand and identity; never reflected as a JS property.
    pub(crate) host_identity: Option<HostIdentity>,
    /// Annex B [[IsHTMLDDA]] internal slot, used by IsLooselyEqual.
    pub(crate) is_html_dda: bool,
    /// Strictness captured when an ECMAScript function is created.
    pub(crate) strict_function: bool,
    /// Cached compilation of an AST async body; parameters stay runtime-bound.
    pub(crate) async_body_code: Option<Result<crate::interp::vm::FunctionCode, crate::interp::vm::CompileError>>,
    /// Promise capability captured by a native resolve or reject function.
    pub(crate) promise_target: Option<Obj>,
    /// State a `Proxy.revocable` revoke function closes over. Orthogonal to
    /// `exotic`: the revoke function is an ordinary callable, not itself a
    /// Proxy exotic object, so it must not be caught by the Proxy trap hooks.
    pub(crate) proxy_revoke: Option<Rc<crate::interp::ProxyState>>,
}

impl Default for JsObject {
    fn default() -> Self {
        Self::ordinary()
    }
}

impl JsObject {
    /// Give this object the Annex B [[IsHTMLDDA]] internal slot (host hook for `$262.IsHTMLDDA`).
    pub fn mark_html_dda(&mut self) {
        self.is_html_dda = true;
    }

    pub fn ordinary() -> JsObject {
        JsObject {
            props: Vec::new(),
            prop_index: FastMap::default(),
            shape_version: 0,
            vm_cacheable: false,
            internal_slots: false,
            realm: None,
            proto: None,
            exotic: Exotic::Ordinary,
            arguments_object: false,
            boxed_primitive: None,
            raw_json: None,
            extensible: true,
            func: None,
            constructed: false,
            constructible: true,
            class_constructor: false,
            derived_constructor: None,
            class_elements: None,
            private_elements: Vec::new(),
            lexical_super_constructor: None,
            lexical_new_target: None,
            lexical_new_target_allowed: false,
            lexical_class_initializer: false,
            host_identity: None,
            is_html_dda: false,
            strict_function: false,
            async_body_code: None,
            promise_target: None,
            proxy_revoke: None,
        }
    }

    /// Initialize a fresh String wrapper in linear time. Its code-unit indices
    /// use ordinary immutable descriptors, shared by every property operation.
    pub(crate) fn initialize_string_data(&mut self, string: JsString) {
        debug_assert!(self.props.is_empty());
        for (index, unit) in string.units().iter().enumerate() {
            self.push_prop(PropertyKey::from(index.to_string()), Property {
                accessor: false,
                value: JsValue::str_units(vec![*unit]), writable: false,
                enumerable: true, configurable: false, getter: None, setter: None,
            });
        }
        self.push_prop(PropertyKey::from("length"), Property {
            accessor: false,
            value: JsValue::num(string.units().len() as f64), writable: false,
            enumerable: false, configurable: false, getter: None, setter: None,
        });
        self.boxed_primitive = Some(JsValue::Str(string));
    }

    /// Appends a brand-new own property (caller guarantees the key isn't
    /// already present) and records its slot in `prop_index`. Used by the
    /// few constructors that build `props` directly rather than through
    /// `define` (which already does its own existing-key check).
    fn push_prop(&mut self, key: PropertyKey, prop: Property) {
        if let PropertyKey::String(name) = &key {
            if let [0x5b, 0x5b, ..] = name.units() { self.internal_slots = true; }
        }
        let slot = self.props.len();
        if slot >= SMALL_OBJECT_PROPS {
            if self.prop_index.is_empty() {
                // Crossing the threshold: index everything already stored.
                self.prop_index = self.props.iter().enumerate()
                    .map(|(index, (k, _))| (k.clone(), index))
                    .collect();
            }
            self.prop_index.insert(key.clone(), slot);
        }
        self.props.push((key, prop));
    }

    /// Slot of an own property. Objects with at most `SMALL_OBJECT_PROPS`
    /// properties are scanned linearly (no hashing, no index allocation).
    #[inline]
    fn slot_of(&self, key: &PropertyKey) -> Option<usize> {
        if self.props.len() <= SMALL_OBJECT_PROPS {
            self.props.iter().position(|(existing, _)| existing == key)
        } else {
            self.prop_index.get(key).copied()
        }
    }

    pub fn array() -> JsObject {
        let mut o = Self::ordinary();
        o.exotic = Exotic::Array;
        // length is a real own property: writable but non-enumerable per
        // spec §10.4.2.5 property attributes.
        o.push_prop(
            PropertyKey::from("length"),
            Property {
                accessor: false,
                value: JsValue::num(0.0),
                writable: true,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        );
        o
    }

    pub fn error_obj(name: &str, message: JsValue) -> JsObject {
        let mut o = Self::ordinary();
        o.exotic = Exotic::Error;
        o.set_own("name", JsValue::str(name));
        o.set_own("message", message);
        o
    }

    pub fn function(func: JsFunction) -> JsObject {
        let mut o = Self::ordinary();
        // Native callbacks have [[Call]] only; spec constructors opt in
        // explicitly during installation. Host functions use their own policy.
        o.constructible = !func.is_arrow && !func.is_async && !func.is_generator
            && !matches!(&func.body, FunctionBody::Native(_));
        o.realm = func.env.realm();
        let length = func.params.iter().take_while(|param| !param.rest && param.default.is_none()).count();
        o.define("length", Property {
            accessor: false,
            value: JsValue::num(length as f64), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        o.define("name", Property {
            accessor: false,
            value: JsValue::str(&func.name), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        o.func = Some(func);
        o
    }

    pub fn private_find(&self, name: &PrivateName) -> Option<&PrivateElement> {
        self.private_elements.iter().find(|element| element.name == *name)
    }

    pub fn private_find_mut(&mut self, name: &PrivateName) -> Option<&mut PrivateElement> {
        self.private_elements.iter_mut().find(|element| element.name == *name)
    }

    pub fn private_has(&self, name: &PrivateName) -> bool {
        self.private_find(name).is_some()
    }

    /// PrivateFieldAdd / PrivateMethodOrAccessorAdd (§7.3.29 / §7.3.30):
    /// throws rather than overwriting if the brand is already present (the
    /// double-`super()`-with-private-fields case).
    pub fn private_add(&mut self, name: PrivateName, kind: PrivateElementKind) -> Result<(), Throw> {
        if matches!(self.exotic, Exotic::ModuleNamespace(_) | Exotic::DeferredModuleNamespace(_)) {
            return Err(Throw::type_error("cannot add private members to a module namespace"));
        }
        if !self.extensible {
            return Err(Throw::type_error("cannot add private members to a non-extensible object"));
        }
        if self.private_has(&name) {
            return Err(Throw::type_error(format!(
                "cannot add private member {} twice on the same object",
                name.text()
            )));
        }
        self.private_elements.push(PrivateElement { name, kind });
        Ok(())
    }

    /// Define an own property, replacing any existing entry in place
    /// (preserving its position). Infallible for our ordinary-model subset.
    pub fn define(&mut self, key: impl Into<PropertyKey>, prop: Property) -> Result<(), ()> {
        let key = key.into();
        if let Some(slot) = self.slot_of(&key) {
            let existing = &mut self.props[slot].1;
            if existing.writable != prop.writable || existing.enumerable != prop.enumerable
                || existing.configurable != prop.configurable
                || existing.accessor != prop.accessor
                || existing.getter.is_some() != prop.getter.is_some()
                || existing.setter.is_some() != prop.setter.is_some() {
                self.shape_version = self.shape_version.wrapping_add(1);
            }
            *existing = prop;
        } else {
            self.push_prop(key, prop);
            self.shape_version = self.shape_version.wrapping_add(1);
        }
        Ok(())
    }

    /// True when [[Get]] on this object is the plain own-then-prototype walk:
    /// no proxy, namespace, window, arguments or typed-array behaviour, no host
    /// identity and no DOM wrapper slots. `Map`/`Set` qualify except for the
    /// engine's synthetic `size` accessor.
    #[inline]
    fn plain_for_get(&self, key: &PropertyKey) -> bool {
        if self.host_identity.is_some() || self.internal_slots { return false; }
        match &self.exotic {
            Exotic::Ordinary | Exotic::Array | Exotic::Error | Exotic::Promise(_) | Exotic::RegExp(_)
            | Exotic::Generator(_) | Exotic::ThreadGenerator(_) | Exotic::SymbolObject(_)
            | Exotic::LegacyIterator(_) | Exotic::PairIterator(_) | Exotic::SequenceIterator(_)
            | Exotic::CollectionIterator(_) | Exotic::DisposableStack(_) => true,
            Exotic::Map(_) | Exotic::Set(_) => !matches!(key, PropertyKey::String(name) if name.units() == [0x73, 0x69, 0x7a, 0x65]),
            _ => false,
        }
    }

    /// Own data-property value of a plain object, for global-name reads.
    pub(crate) fn plain_own_data(&self, key: &PropertyKey) -> Option<JsValue> {
        if !self.plain_for_get(key) { return None; }
        let property = &self.props[self.slot_of(key)?].1;
        property.is_data().then(|| property.value.clone())
    }

    /// Ordinary [[Get]] for a data property found along an all-plain prototype
    /// chain. `None` means "take the general path" (an accessor, or any object
    /// that is not plain); `Some(Undefined)` is a plain miss.
    pub(crate) fn plain_get(&self, key: &PropertyKey) -> Option<JsValue> {
        if !self.plain_for_get(key) { return None; }
        if let Some(slot) = self.slot_of(key) {
            let property = &self.props[slot].1;
            return if property.is_data() { Some(property.value.clone()) } else { None };
        }
        // Bounded walk: long chains take the general path, which admits each step.
        let mut cursor = self.proto.clone();
        let mut hops = 0;
        while let Some(proto) = cursor {
            hops += 1;
            if hops > 32 { return None; }
            let proto = proto.borrow();
            if !proto.plain_for_get(key) { return None; }
            if let Some(slot) = proto.slot_of(key) {
                let property = &proto.props[slot].1;
                return if property.is_data() { Some(property.value.clone()) } else { None };
            }
            cursor = proto.proto.clone();
        }
        Some(JsValue::Undefined)
    }

    /// Ordinary [[Set]] with the object itself as receiver, for the common
    /// case: a plain object (arrays only for non-index, non-`length` names)
    /// whose own or inherited property is a writable data property, or absent
    /// from an all-plain chain on an extensible object. Returns `false`
    /// without side effects when the general algorithm must run.
    pub(crate) fn plain_set(&mut self, key: &PropertyKey, value: &JsValue) -> bool {
        if !self.plain_for_get(key) { return false; }
        if matches!(self.exotic, Exotic::Array) || matches!(self.exotic, Exotic::Map(_) | Exotic::Set(_)) {
            let PropertyKey::String(name) = key else { return false };
            let units = name.units();
            if matches!(self.exotic, Exotic::Array)
                && (units.first().is_none_or(|u| (0x30..=0x39).contains(u))
                    || units == [0x6c, 0x65, 0x6e, 0x67, 0x74, 0x68]) { return false; }
        }
        if let Some(slot) = self.slot_of(key) {
            let property = &mut self.props[slot].1;
            if property.is_accessor() || !property.writable { return false; }
            property.value = value.clone();
            return true;
        }
        if !self.extensible { return false; }
        let mut cursor = self.proto.clone();
        let mut hops = 0;
        while let Some(proto) = cursor {
            hops += 1;
            if hops > 32 { return false; }
            let proto = proto.borrow();
            if !proto.plain_for_get(key) { return false; }
            if let Some(slot) = proto.slot_of(key) {
                let property = &proto.props[slot].1;
                if property.is_accessor() || !property.writable { return false; }
                break;
            }
            cursor = proto.proto.clone();
        }
        self.set_own(key.clone(), value.clone());
        true
    }

    /// Slot access for VM inline caches. The key check also protects against
    /// a stale cache if a version ever wraps.
    pub(crate) fn own_data_slot(&self, key: &PropertyKey, slot: usize) -> Option<&Property> {
        let (actual, property) = self.props.get(slot)?;
        (actual == key && property.is_data())
            .then_some(property)
    }

    pub(crate) fn own_slot_index(&self, key: &PropertyKey) -> Option<usize> {
        self.slot_of(key)
    }

    pub(crate) fn set_own_data_slot(&mut self, key: &PropertyKey, slot: usize, value: JsValue) -> bool {
        let Some(property) = self.own_data_slot(key, slot) else { return false };
        if !property.writable { return false; }
        self.props[slot].1.value = value;
        true
    }

    /// Shorthand for a full-rights data property.
    pub fn set_own(&mut self, key: impl Into<PropertyKey>, value: JsValue) {
        let _ = self.define(key, Property::data(value));
    }

    pub fn get_own(&self, key: impl Into<PropertyKey>) -> Option<JsValue> {
        let key = key.into();
        let slot = self.slot_of(&key)?;
        Some(self.props[slot].1.value.clone())
    }

    pub fn own_prop(&self, key: impl Into<PropertyKey>) -> Option<&Property> {
        let key = key.into();
        let slot = self.slot_of(&key)?;
        Some(&self.props[slot].1)
    }

    pub fn delete_own(&mut self, key: impl Into<PropertyKey>) -> bool {
        let key = key.into();
        if let Some(pos) = self.slot_of(&key) {
            if !self.props[pos].1.configurable {
                return false;
            }
            if pos == self.props.len() - 1 {
                // Descending array truncation commonly removes the tail.
                // No remaining slot moves, so preserve its index entries.
                self.props.pop();
                self.prop_index.remove(&key);
            } else {
                self.props.remove(pos);
                // Interior removal shifts later entries down by one slot.
                self.prop_index = self.props.iter().enumerate()
                    .map(|(index, (k, _))| (k.clone(), index))
                    .collect();
            }
            if self.props.len() <= SMALL_OBJECT_PROPS {
                // Small objects are scanned; keep the index empty so that
                // `push_prop` rebuilds it when the threshold is crossed again.
                self.prop_index = FastMap::default();
            }
            self.shape_version = self.shape_version.wrapping_add(1);
            if let Exotic::Arguments(map) = &mut self.exotic { map.remove(&key); }
            true
        } else {
            true
        }
    }

    /// Own enumerable keys in insertion order (for-in / Object.keys).
    pub fn own_keys_enumerable(&self) -> Vec<JsString> {
        self.own_property_keys().into_iter().filter_map(|key| match key {
            PropertyKey::String(name) if self.own_prop(&name).is_some_and(|p| p.enumerable) => Some(name),
            _ => None,
        }).collect()
    }

    /// All own string keys in ECMAScript key order.
    pub fn own_keys(&self) -> Vec<JsString> {
        self.own_property_keys().into_iter().filter_map(|key| match key {
            PropertyKey::String(name) => Some(name), PropertyKey::Symbol(_) => None,
        }).collect()
    }

    /// [[OwnPropertyKeys]]: array indices ascending, other strings in insertion
    /// order, and Symbols in insertion order.
    pub fn own_property_keys(&self) -> Vec<PropertyKey> {
        let mut indices = Vec::new();
        let mut strings = Vec::new();
        let mut symbols = Vec::new();
        for (key, _) in &self.props {
            match key {
                PropertyKey::String(name) => match array_index(name) {
                    Some(index) => indices.push((index, key.clone())),
                    None => strings.push(key.clone()),
                },
                PropertyKey::Symbol(_) => symbols.push(key.clone()),
            }
        }
        indices.sort_by_key(|(index, _)| *index);
        indices.into_iter().map(|(_, key)| key).chain(strings).chain(symbols).collect()
    }

    /// §7.3.20/§10.4 proto-chain lookup: walks own props then prototypes.
    /// Returns None when the chain ends without the key anywhere.
    pub fn lookup(&self, key: impl Into<PropertyKey>) -> Option<JsValue> {
        let key = key.into();
        if let Some(v) = self.get_own(&key) {
            return Some(v);
        }
        match &self.proto {
            Some(p) => p.borrow().lookup(&key),
            None => None,
        }
    }

    /// Whether the key exists anywhere on the chain (the `in` operator).
    pub fn has_property(&self, key: impl Into<PropertyKey>) -> bool {
        let key = key.into();
        self.get_own(&key).is_some()
            || self
                .proto
                .as_ref()
                .is_some_and(|p| p.borrow().has_property(&key))
    }
}

fn array_index(name: &JsString) -> Option<u32> {
    let units = name.units();
    if units.is_empty() || units.len() > 10 || (units.len() > 1 && units[0] == b'0' as u16) {
        return None;
    }
    let mut value = 0u64;
    for unit in units {
        if !(*unit >= b'0' as u16 && *unit <= b'9' as u16) { return None; }
        value = value * 10 + u64::from(*unit - b'0' as u16);
    }
    (value < u64::from(u32::MAX)).then_some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_equality_table() {
        assert_eq!(JsValue::Undefined, JsValue::Undefined);
        assert_eq!(JsValue::Null, JsValue::Null);
        assert_ne!(JsValue::Undefined, JsValue::Null);
        assert_eq!(JsValue::num(0.0), JsValue::num(-0.0)); // +0 === -0
        assert_ne!(JsValue::num(f64::NAN), JsValue::num(f64::NAN)); // NaN !== NaN
        assert_eq!(JsValue::str("a"), JsValue::str("a"));
        let symbol = JsValue::Symbol(JsSymbol::new(Some("a".into())));
        assert_eq!(symbol, symbol.clone());
        assert_ne!(symbol, JsValue::Symbol(JsSymbol::new(Some("a".into()))));
        let o = JsValue::object(JsObject::ordinary());
        assert_eq!(o, o.clone()); // identity
        let p = JsValue::object(JsObject::ordinary());
        assert_ne!(o, p);
    }

    #[test]
    fn own_property_keys_order_indices_strings_and_symbols() {
        let first = JsSymbol::new(Some("first".into()));
        let second = JsSymbol::new(Some("second".into()));
        let mut object = JsObject::ordinary();
        for key in [PropertyKey::from("b"), PropertyKey::from("2"),
            PropertyKey::from(&first), PropertyKey::from("1"),
            PropertyKey::from("a"), PropertyKey::from(&second)] {
            object.set_own(key, JsValue::Bool(true));
        }
        assert_eq!(object.own_property_keys(), vec![
            PropertyKey::from("1"), PropertyKey::from("2"),
            PropertyKey::from("b"), PropertyKey::from("a"),
            PropertyKey::from(first), PropertyKey::from(second),
        ]);
        assert_eq!(object.own_keys(), vec!["1", "2", "b", "a"]);
    }

    #[test]
    fn to_boolean_matches_spec_table() {
        assert!(!JsValue::Undefined.to_boolean());
        assert!(!JsValue::Null.to_boolean());
        assert!(!JsValue::Bool(false).to_boolean());
        assert!(!JsValue::Num(0.0).to_boolean());
        assert!(!JsValue::Num(f64::NAN).to_boolean());
        assert!(!JsValue::str("").to_boolean());
        assert!(JsValue::Num(-0.0).to_boolean() == false);
        assert!(JsValue::str("0").to_boolean()); // non-empty string
        assert!(JsValue::object(JsObject::ordinary()).to_boolean());
    }

    #[test]
    fn string_to_number_is_strict() {
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("  42 "), 42.0);
        assert_eq!(string_to_number("1.5e2"), 150.0);
        assert_eq!(string_to_number("0x10"), 16.0);
        assert_eq!(string_to_number("0b1110"), 14.0);
        assert_eq!(string_to_number("0O17"), 15.0);
        assert_eq!(string_to_number("-.5"), -0.5);
        assert!(string_to_number("+0x10").is_nan());
        assert!(string_to_number("0b102").is_nan());
        assert!(string_to_number("1_000").is_nan());
        assert!(string_to_number("inf").is_nan()); // Rust-ism rejected
        assert!(string_to_number("4,5").is_nan());
        assert_eq!(string_to_number("-3"), -3.0);
        assert_eq!(string_to_number("Infinity"), f64::INFINITY);
    }

    #[test]
    fn number_to_string_drops_integral_point() {
        assert_eq!(number_to_string(1.0), "1");
        assert_eq!(number_to_string(-0.0), "0"); // ToString(-0) is "0"
        assert_eq!(number_to_string(f64::NAN), "NaN");
        assert_eq!(number_to_string(1e21), "1e+21"); // JS keeps the sign
        assert_eq!(number_to_string(1e20), "100000000000000000000");
        assert_eq!(number_to_string(1e-7), "1e-7");
        assert_eq!(number_to_string(0.000001), "0.000001"); // 1e-6 stays fixed
        assert_eq!(number_to_string(123.456), "123.456");
        assert_eq!(number_to_string(-2.5), "-2.5");
        assert_eq!(number_to_string(0.5), "0.5");
    }

    #[test]
    fn array_length_starts_nonenumerable_zero() {
        let a = JsObject::array();
        assert_eq!(a.get_own("length"), Some(JsValue::num(0.0)));
        assert!(!a.own_keys_enumerable().contains(&JsString::from("length")));
        assert!(a.own_keys().contains(&JsString::from("length")));
    }

    #[test]
    fn define_preserves_insertion_position_on_overwrite() {
        let mut o = JsObject::ordinary();
        o.set_own("a", JsValue::num(1.0));
        o.set_own("b", JsValue::num(2.0));
        o.set_own("a", JsValue::num(9.0));
        assert_eq!(o.own_keys(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(o.get_own("a"), Some(JsValue::num(9.0)));
    }

    #[test]
    fn delete_respects_configurable_flag() {
        let mut o = JsObject::ordinary();
        let frozen = Property {
            accessor: false,
            value: JsValue::num(1.0),
            writable: false,
            enumerable: true,
            configurable: false,
            getter: None,
            setter: None,
        };
        let _ = o.define("x", frozen);
        assert!(!o.delete_own("x")); // non-configurable stays
        assert!(o.delete_own("missing")); // absent deletes trivially
        o.set_own("y", JsValue::Null);
        assert!(o.delete_own("y"));
        assert!(o.get_own("y").is_none());
    }

    #[test]
    fn repeated_deletion_preserves_lookup_order_and_invalidates_cached_slots() {
        let mut o = JsObject::ordinary();
        for name in ["a", "b", "c", "d"] { o.set_own(name, JsValue::Str(name.into())); }
        let version = o.shape_version;
        assert!(o.delete_own("d"));
        assert!(o.delete_own("b"));
        assert!(o.delete_own("c"));
        assert_eq!(o.shape_version, version.wrapping_add(3));
        assert_eq!(o.get_own("a"), Some(JsValue::Str("a".into())));
        for name in ["b", "c", "d"] { assert!(o.get_own(name).is_none()); }
        o.set_own("b", JsValue::num(7.0));
        assert_eq!(o.own_keys(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(o.get_own("b"), Some(JsValue::num(7.0)));
        assert!(o.delete_own("b"));
        assert!(o.delete_own("a"));
        assert!(o.own_keys().is_empty());
    }

    #[test]
    fn prototype_chain_lookup_walks_then_gives_up() {
        let base = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        base.borrow_mut().set_own("inherited", JsValue::num(7.0));
        let child = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        child.borrow_mut().proto = Some(base.clone());
        let cb = child.borrow();
        assert_eq!(cb.lookup("inherited"), Some(JsValue::num(7.0)));
        assert_eq!(cb.lookup("absent"), None);
        assert!(cb.has_property("inherited"));
        assert!(!cb.has_property("absent"));
    }
}

#[cfg(test)]
mod small_object_index_tests {
    use super::*;

    fn check(o: &JsObject, live: &[usize]) {
        for index in 0..40 {
            let key = format!("p{index}");
            assert_eq!(o.get_own(key.as_str()).is_some(), live.contains(&index), "{key}");
            if live.contains(&index) {
                assert!(matches!(o.get_own(key.as_str()), Some(JsValue::Num(n)) if n == index as f64), "{key}");
            }
        }
        assert_eq!(o.props.len(), live.len());
        assert_eq!(o.prop_index.is_empty(), live.len() <= SMALL_OBJECT_PROPS);
    }

    #[test]
    fn index_appears_above_threshold_and_stays_consistent_through_deletes() {
        let mut o = JsObject::ordinary();
        let mut live: Vec<usize> = Vec::new();
        for index in 0..30 {
            o.set_own(format!("p{index}"), JsValue::num(index as f64));
            live.push(index);
            check(&o, &live);
        }
        // Interior, tail and head deletions across the threshold, both ways.
        for index in [12usize, 29, 0, 28, 5, 13, 14, 15, 16, 17, 18, 19, 20] {
            assert!(o.delete_own(format!("p{index}")));
            live.retain(|&x| x != index);
            check(&o, &live);
        }
        for index in 30..40 {
            o.set_own(format!("p{index}"), JsValue::num(index as f64));
            live.push(index);
            check(&o, &live);
        }
        // Redefining an existing key keeps its slot.
        o.set_own("p33", JsValue::num(33.0));
        check(&o, &live);
    }

    #[test]
    fn short_ascii_strings_share_storage_and_compare_equal_to_uncached_ones() {
        let a = JsString::from("length");
        let b = JsString::from("length");
        assert!(std::ptr::eq(a.units().as_ptr(), b.units().as_ptr()));
        assert_eq!(a, JsString::from_units("length".encode_utf16().collect::<Vec<_>>()));
        let long = "x".repeat(100);
        assert_eq!(JsString::from(long.as_str()).len(), 100);
        assert_eq!(JsString::from("h\u{e9}llo \u{1f600}").units(), "h\u{e9}llo \u{1f600}".encode_utf16().collect::<Vec<_>>().as_slice());
        assert_eq!(JsString::from("").len(), 0);
        // Colliding slots must not return the wrong string.
        for index in 0..5000 {
            let text = format!("k{index}");
            assert_eq!(JsString::from(text.as_str()).to_utf8_lossy(), text);
        }
    }
}

include!("value_module.rs");
