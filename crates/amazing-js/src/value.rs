//! The ECMAScript value model: language values, objects, and the abstract
//! coercion operations (spec §6, §7.1).
//!
//! Design decisions (locked):
//! - [`JsValue::Object`] is `Rc<RefCell<JsObject>>`: shared mutable identity
//!   is observable (`a === b` for the same object; mutation through one
//!   alias visible via another) and `Rc<RefCell>` is std-only.
//! - Property storage is an insertion-ordered `Vec` of entries, linearly
//!   searched. Real engines hash; we need *observable key order* (for-in,
//!   `Object.keys`) more than throughput, and a Vec gives it for free.
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

impl From<&str> for JsString {
    fn from(value: &str) -> Self { Self::from_units(value.encode_utf16().collect::<Vec<_>>()) }
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
    Dispose,
}

impl WellKnownSymbol {
    pub const ALL: [Self; 14] = [
        Self::Iterator, Self::AsyncIterator, Self::HasInstance, Self::ToPrimitive,
        Self::ToStringTag, Self::IsConcatSpreadable, Self::Species, Self::Unscopables,
        Self::Match, Self::MatchAll, Self::Replace, Self::Search, Self::Split,
        Self::Dispose,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Iterator => "iterator", Self::AsyncIterator => "asyncIterator",
            Self::HasInstance => "hasInstance", Self::ToPrimitive => "toPrimitive",
            Self::ToStringTag => "toStringTag", Self::IsConcatSpreadable => "isConcatSpreadable",
            Self::Species => "species", Self::Unscopables => "unscopables",
            Self::Match => "match", Self::MatchAll => "matchAll", Self::Replace => "replace",
            Self::Search => "search", Self::Split => "split",
            Self::Dispose => "dispose",
        }
    }
    pub fn symbol(self) -> JsSymbol {
        let id = match self {
            Self::Iterator => 1, Self::AsyncIterator => 2, Self::HasInstance => 3,
            Self::ToPrimitive => 4, Self::ToStringTag => 5, Self::IsConcatSpreadable => 6,
            Self::Species => 7, Self::Unscopables => 8, Self::Match => 9,
            Self::MatchAll => 10, Self::Replace => 11, Self::Search => 12, Self::Split => 13,
            Self::Dispose => 14,
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
            (Exotic::ModuleNamespace(_), _) => write!(f, "[module namespace]"),
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
            JsValue::Object(_) => true,
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

/// One own property (spec §6.1.7 data descriptor; accessors stored but
/// unexercised until class accessors land).
#[derive(Debug, Clone)]
pub struct Property {
    pub value: JsValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
    /// Original callable objects retain strictness, identity, and private slots.
    pub getter: Option<JsValue>,
    pub setter: Option<JsValue>,
}

impl Property {
    /// Writable/enumerable/configurable defaults for assignment-created
    /// and literal-created data properties (true/true/true).
    pub fn data(value: JsValue) -> Property {
        Property {
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

/// Internal class-constructor metadata kept on the function object, so the
/// embedding-facing `JsFunction` initializer remains independent of classes.
#[derive(Debug, Clone)]
pub(crate) struct DerivedConstructor {
    pub parent: JsValue,
    pub implicit: bool,
}

/// Private elements use identities scoped to one class evaluation. They are
/// separate from ordinary and Symbol properties and cannot be reflected.
#[derive(Debug, Clone)]
pub(crate) enum PrivateElement {
    Field(JsValue),
    Method(JsValue),
    Accessor { getter: Option<JsValue>, setter: Option<JsValue> },
}

#[derive(Debug, Clone)]
pub(crate) enum ClassFieldKey {
    Public(PropertyKey),
    Private(JsSymbol),
}

#[derive(Debug, Clone)]
pub(crate) struct ClassFieldDefinition {
    pub key: ClassFieldKey,
    pub initializer: Option<ast::Expr>,
    pub accessor: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassInstanceElements {
    pub fields: Vec<ClassFieldDefinition>,
    pub methods: Vec<(JsSymbol, PrivateElement)>,
    pub env: crate::env::EnvHandle,
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
    Array,
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
    ModuleNamespace(std::collections::BTreeMap<String, crate::env::BindingCell>),
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

/// One queued `then` reaction: callback + resolution capability placeholder.
#[derive(Debug, Clone)]
pub struct PendingReaction {
    pub handler: Option<JsValue>,
    /// Which promise the reaction settles (chained .then result).
    pub capability: Obj,
    /// `true` when this is the reject path of a `then(on_ful, on_rej)` pair.
    pub is_reject_handler: bool,
}

/// A JavaScript object: own properties in insertion order, a prototype
/// slot, and an exotic payload.
#[derive(Debug, Clone)]
pub struct JsObject {
    props: Vec<(PropertyKey, Property)>,
    pub proto: Option<Obj>,
    pub exotic: Exotic,
    /// Whether new own properties may be created.
    pub extensible: bool,
    /// Callable payload; `Some` iff this object is a function.
    pub func: Option<JsFunction>,
    /// True once this object has been invoked via `new` as a constructor
    /// (cheap stand-in for the internal Slots checks the spec performs).
    pub constructed: bool,
    /// Built-in methods are callable but lack [[Construct]].
    pub(crate) constructible: bool,
    pub(crate) derived_constructor: Option<DerivedConstructor>,
    pub(crate) class_constructor: bool,
    pub(crate) class_instance_elements: Option<ClassInstanceElements>,
    pub(crate) private_elements: Vec<(JsSymbol, PrivateElement)>,
    pub(crate) lexical_super_constructor: Option<JsValue>,
    pub(crate) lexical_new_target: Option<JsValue>,
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
}

impl Default for JsObject {
    fn default() -> Self {
        Self::ordinary()
    }
}

impl JsObject {
    pub fn ordinary() -> JsObject {
        JsObject {
            props: Vec::new(),
            proto: None,
            exotic: Exotic::Ordinary,
            extensible: true,
            func: None,
            constructed: false,
            constructible: true,
            derived_constructor: None,
            class_constructor: false,
            class_instance_elements: None,
            private_elements: Vec::new(),
            lexical_super_constructor: None,
            lexical_new_target: None,
            host_identity: None,
            is_html_dda: false,
            strict_function: false,
            async_body_code: None,
            promise_target: None,
        }
    }

    /// Initialize a fresh String wrapper in linear time. Its code-unit indices
    /// use ordinary immutable descriptors, shared by every property operation.
    pub(crate) fn initialize_string_data(&mut self, string: JsString) {
        debug_assert!(self.props.is_empty());
        self.props.extend(string.units().iter().enumerate().map(|(index, unit)| {
            (PropertyKey::from(index.to_string()), Property {
                value: JsValue::str_units(vec![*unit]), writable: false,
                enumerable: true, configurable: false, getter: None, setter: None,
            })
        }));
        self.props.push((PropertyKey::from("length"), Property {
            value: JsValue::num(string.units().len() as f64), writable: false,
            enumerable: false, configurable: false, getter: None, setter: None,
        }));
        self.props.push((PropertyKey::from("[[StringData]]"), Property {
            value: JsValue::Str(string), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        }));
    }

    pub fn array() -> JsObject {
        let mut o = Self::ordinary();
        o.exotic = Exotic::Array;
        // length is a real own property: writable but non-enumerable per
        // spec §10.4.2.5 property attributes.
        o.props.push((
            PropertyKey::from("length"),
            Property {
                value: JsValue::num(0.0),
                writable: true,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        ));
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
        let length = func.params.iter().take_while(|param| !param.rest && param.default.is_none()).count();
        o.define("name", Property {
            value: JsValue::str(&func.name), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        o.define("length", Property {
            value: JsValue::num(length as f64), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        o.func = Some(func);
        o
    }

    /// Define an own property, replacing any existing entry in place
    /// (preserving its position). Infallible for our ordinary-model subset.
    pub fn define(&mut self, key: impl Into<PropertyKey>, prop: Property) -> Result<(), ()> {
        let key = key.into();
        if let Some(slot) = self.props.iter_mut().find(|(k, _)| k == &key) {
            slot.1 = prop;
        } else {
            self.props.push((key, prop));
        }
        Ok(())
    }

    /// Shorthand for a full-rights data property.
    pub fn set_own(&mut self, key: impl Into<PropertyKey>, value: JsValue) {
        let _ = self.define(key, Property::data(value));
    }

    pub fn get_own(&self, key: impl Into<PropertyKey>) -> Option<JsValue> {
        let key = key.into();
        self.props
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, p)| p.value.clone())
    }

    pub fn own_prop(&self, key: impl Into<PropertyKey>) -> Option<&Property> {
        let key = key.into();
        self.props.iter().find(|(k, _)| k == &key).map(|(_, p)| p)
    }

    pub fn delete_own(&mut self, key: impl Into<PropertyKey>) -> bool {
        let key = key.into();
        if let Some(pos) = self.props.iter().position(|(k, _)| k == &key) {
            if !self.props[pos].1.configurable {
                return false;
            }
            self.props.remove(pos);
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
