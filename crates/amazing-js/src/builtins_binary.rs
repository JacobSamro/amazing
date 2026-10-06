// Portions ported from boa-dev/boa (core/engine/src/builtins/array/array_iterator.rs,
// array_buffer/mod.rs) @ ef40c63283e1bb73f371d93372e9e95217a36945, MIT.
// ECMAScript binary data. The byte store is private to this module until the
// shared value model grows internal slots. Weak object keys avoid retaining JS
// objects merely because a backing store exists.
use std::thread_local;

#[derive(Clone)]
struct BinaryBuffer {
    bytes: Vec<u8>,
    max: usize,
    resizable: bool,
    shared: bool,
    detached: bool,
    immutable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BinaryKind {
    Int8, Uint8, Uint8Clamped, Int16, Uint16, Int32, Uint32,
    BigInt64, BigUint64, Float16, Float32, Float64,
}
impl BinaryKind {
    fn name(self) -> &'static str {
        match self {
            Self::Int8 => "Int8Array", Self::Uint8 => "Uint8Array",
            Self::Uint8Clamped => "Uint8ClampedArray", Self::Int16 => "Int16Array",
            Self::Uint16 => "Uint16Array", Self::Int32 => "Int32Array",
            Self::Uint32 => "Uint32Array", Self::BigInt64 => "BigInt64Array",
            Self::BigUint64 => "BigUint64Array", Self::Float16 => "Float16Array",
            Self::Float32 => "Float32Array", Self::Float64 => "Float64Array",
        }
    }
    fn size(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 | Self::Uint8Clamped => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::BigInt64 | Self::BigUint64 | Self::Float64 => 8,
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Int8Array" => Self::Int8, "Uint8Array" => Self::Uint8,
            "Uint8ClampedArray" => Self::Uint8Clamped,
            "Int16Array" => Self::Int16, "Uint16Array" => Self::Uint16,
            "Int32Array" => Self::Int32, "Uint32Array" => Self::Uint32,
            "BigInt64Array" => Self::BigInt64, "BigUint64Array" => Self::BigUint64,
            "Float16Array" => Self::Float16, "Float32Array" => Self::Float32,
            "Float64Array" => Self::Float64, _ => return None,
        })
    }
    fn integer(self) -> bool { !matches!(self, Self::Float16 | Self::Float32 | Self::Float64) }
    fn big(self) -> bool { matches!(self, Self::BigInt64 | Self::BigUint64) }
}

#[derive(Clone)]
struct BinaryView {
    buffer: Rc<RefCell<BinaryBuffer>>,
    buffer_object: Obj,
    offset: usize,
    length: Option<usize>, // None tracks a resizable backing store.
    kind: Option<BinaryKind>, // None is DataView.
}

#[derive(Clone)]
enum BinaryState { Buffer(Rc<RefCell<BinaryBuffer>>), View(BinaryView) }

thread_local! {
    static BINARY_OBJECTS: RefCell<HashMap<usize, (Weak<RefCell<JsObject>>, BinaryState)>> =
        RefCell::new(HashMap::new());
}

fn binary_identity(object: &Obj) -> usize { Rc::as_ptr(&object.0) as usize }
fn binary_register(object: &Obj, state: BinaryState) {
    BINARY_OBJECTS.with(|entries| {
        entries.borrow_mut().insert(binary_identity(object), (Rc::downgrade(&object.0), state));
    });
}
fn binary_state(object: &Obj) -> Option<BinaryState> {
    BINARY_OBJECTS.with(|entries| {
        let entries = entries.borrow();
        let (weak, state) = entries.get(&binary_identity(object))?;
        weak.upgrade().filter(|live| Rc::ptr_eq(live, &object.0)).map(|_| state.clone())
    })
}
fn binary_obj(value: &JsValue) -> Result<Obj, Throw> {
    match value { JsValue::Object(object) => Ok(object.clone()),
        _ => Err(Throw::type_error("incompatible binary data receiver")) }
}
fn binary_buffer(value: &JsValue) -> Result<Rc<RefCell<BinaryBuffer>>, Throw> {
    match binary_state(&binary_obj(value)?) {
        Some(BinaryState::Buffer(buffer)) => Ok(buffer),
        _ => Err(Throw::type_error("incompatible ArrayBuffer receiver")),
    }
}
fn binary_view(value: &JsValue) -> Result<BinaryView, Throw> {
    match binary_state(&binary_obj(value)?) {
        Some(BinaryState::View(view)) => Ok(view),
        _ => Err(Throw::type_error("incompatible view receiver")),
    }
}
fn binary_typed(value: &JsValue) -> Result<(BinaryView, BinaryKind), Throw> {
    let view = binary_view(value)?;
    let kind = view.kind.ok_or_else(|| Throw::type_error("not a TypedArray"))?;
    Ok((view, kind))
}
fn binary_index(i: &mut Interpreter, value: &JsValue) -> Result<usize, Throw> {
    if matches!(value, JsValue::Undefined) { return Ok(0); }
    let primitive = i.primitive_number_hint(value.clone())?;
    let n = i.to_number(&primitive)?;
    if n.is_nan() || n == 0.0 { return Ok(0); }
    let integer = n.trunc();
    if !integer.is_finite() || integer < 0.0 || integer > 9_007_199_254_740_991.0 {
        return Err(Throw::range_error("invalid binary data index"));
    }
    Ok(integer as usize)
}
fn binary_length(view: &BinaryView) -> usize {
    let buffer = view.buffer.borrow();
    if buffer.detached || view.offset > buffer.bytes.len() { return 0; }
    let available = buffer.bytes.len() - view.offset;
    match view.length {
        Some(length) if length > available => 0,
        Some(length) => length / view.kind.map_or(1, BinaryKind::size),
        None => available / view.kind.map_or(1, BinaryKind::size),
    }
}
fn binary_view_bytes(view: &BinaryView) -> usize {
    binary_length(view) * view.kind.map_or(1, BinaryKind::size)
}
fn binary_view_out_of_bounds(view: &BinaryView) -> bool {
    let buffer = view.buffer.borrow();
    buffer.detached || view.offset > buffer.bytes.len()
        || view.length.is_some_and(|length| view.offset.checked_add(length).is_none_or(|end| end > buffer.bytes.len()))
}

fn binary_number(bytes: &[u8], kind: BinaryKind, little: bool) -> f64 {
    let mut raw = [0u8; 8];
    let n = kind.size();
    if little { raw[..n].copy_from_slice(bytes); }
    else { for j in 0..n { raw[j] = bytes[n - j - 1]; } }
    match kind {
        BinaryKind::Int8 => (raw[0] as i8) as f64,
        BinaryKind::Uint8 | BinaryKind::Uint8Clamped => raw[0] as f64,
        BinaryKind::Int16 => i16::from_le_bytes([raw[0], raw[1]]) as f64,
        BinaryKind::Uint16 => u16::from_le_bytes([raw[0], raw[1]]) as f64,
        BinaryKind::Int32 => i32::from_le_bytes(raw[..4].try_into().unwrap()) as f64,
        BinaryKind::Uint32 => u32::from_le_bytes(raw[..4].try_into().unwrap()) as f64,
        BinaryKind::BigInt64 => i64::from_le_bytes(raw) as f64,
        BinaryKind::BigUint64 => u64::from_le_bytes(raw) as f64,
        BinaryKind::Float16 => binary_decode_f16(u16::from_le_bytes([raw[0], raw[1]])),
        BinaryKind::Float32 => f32::from_le_bytes(raw[..4].try_into().unwrap()) as f64,
        BinaryKind::Float64 => f64::from_le_bytes(raw),
    }
}
fn binary_encode_number(value: f64, kind: BinaryKind, little: bool) -> Vec<u8> {
    let n = kind.size();
    // Integer element conversion is modulo 2^N, including values outside the
    // range of Rust's integer casts. Saturating a large f64 changes the low
    // bits and breaks ToIntN/ToUintN.
    let integer = if value.is_finite() {
        value.trunc().rem_euclid(4_294_967_296.0) as u32
    } else { 0 };
    let mut bytes = match kind {
        BinaryKind::Int8 | BinaryKind::Uint8 => vec![integer as u8],
        BinaryKind::Uint8Clamped => vec![if value.is_nan() { 0 } else { value.round_ties_even().clamp(0.0, 255.0) as u8 }],
        BinaryKind::Int16 | BinaryKind::Uint16 => (integer as u16).to_le_bytes().to_vec(),
        BinaryKind::Int32 | BinaryKind::Uint32 => integer.to_le_bytes().to_vec(),
        BinaryKind::BigInt64 | BinaryKind::BigUint64 => unreachable!("BigInt elements use ToBigInt"),
        BinaryKind::Float16 => binary_encode_f16(value).to_le_bytes().to_vec(),
        BinaryKind::Float32 => (value as f32).to_le_bytes().to_vec(),
        BinaryKind::Float64 => value.to_le_bytes().to_vec(),
    };
    if !little { bytes.reverse(); }
    debug_assert_eq!(bytes.len(), n);
    bytes
}
fn binary_decode_f16(bits: u16) -> f64 {
    let sign = ((bits & 0x8000) as u32) << 16;
    let exp = ((bits >> 10) & 31) as i32;
    let frac = (bits & 1023) as u32;
    let f32_bits = if exp == 0 {
        if frac == 0 { sign } else {
            let mut mant = frac;
            let mut e = -14;
            while mant & 1024 == 0 { mant <<= 1; e -= 1; }
            sign | (((e + 127) as u32) << 23) | ((mant & 1023) << 13)
        }
    } else if exp == 31 { sign | 0x7f80_0000 | (frac << 13) }
    else { sign | (((exp - 15 + 127) as u32) << 23) | (frac << 13) };
    f32::from_bits(f32_bits) as f64
}
fn binary_encode_f16(value: f64) -> u16 {
    // Round the original f64 directly. An intermediate f32 can round a value
    // onto a half-precision midpoint and then round it a second time.
    let bits = value.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0x7ff { return sign | 0x7c00 | if fraction != 0 { 0x0200 } else { 0 }; }
    if exponent == 0 { return sign; }
    let unbiased = exponent - 1023;
    if unbiased > 15 { return sign | 0x7c00; }
    let significand = (1u64 << 52) | fraction;
    if unbiased < -14 {
        if unbiased < -25 { return sign; }
        return sign | binary_round_shift(significand, (28 - unbiased) as u32) as u16;
    }
    let rounded = binary_round_shift(significand, 42);
    let half_exponent = unbiased + 15 + i32::from(rounded == 2048);
    if half_exponent >= 31 { return sign | 0x7c00; }
    sign | ((half_exponent as u16) << 10) | ((rounded as u16) & 1023)
}
fn binary_round_shift(value: u64, shift: u32) -> u64 {
    let truncated = value >> shift;
    let remainder = value & ((1u64 << shift) - 1);
    let halfway = 1u64 << (shift - 1);
    truncated + u64::from(remainder > halfway || remainder == halfway && truncated & 1 != 0)
}

fn binary_read(view: &BinaryView, kind: BinaryKind, offset: usize, little: bool) -> Result<JsValue, Throw> {
    // An out-of-bounds (or detached) DataView throws TypeError before any
    // element access; a request past the declared view length throws RangeError.
    if binary_view_out_of_bounds(view) { return Err(Throw::type_error("out-of-bounds DataView")); }
    let end = offset.checked_add(kind.size()).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
    let buffer = view.buffer.borrow();
    let start = view.offset + offset;
    if kind.big() {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&buffer.bytes[start..start + 8]);
        if !little { raw.reverse(); }
        let bits = u64::from_le_bytes(raw);
        return Ok(Interpreter::bigint_value(if kind == BinaryKind::BigInt64 {
            BinaryBigInt::from_i64(bits as i64)
        } else { BinaryBigInt::from_u64(bits) }));
    }
    Ok(JsValue::num(binary_number(&buffer.bytes[start..start + kind.size()], kind, little)))
}
fn binary_write(view: &BinaryView, kind: BinaryKind, offset: usize, little: bool, value: f64) -> Result<(), Throw> {
    if view.buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
    if binary_view_out_of_bounds(view) { return Err(Throw::type_error("out-of-bounds DataView")); }
    let end = offset.checked_add(kind.size()).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
    let mut buffer = view.buffer.borrow_mut();
    let start = view.offset + offset;
    buffer.bytes[start..start + kind.size()].copy_from_slice(&binary_encode_number(value, kind, little));
    Ok(())
}
fn binary_write_value(i: &mut Interpreter, view: &BinaryView, kind: BinaryKind, offset: usize, little: bool, value: &JsValue) -> Result<(), Throw> {
    if !kind.big() { return binary_write(view, kind, offset, little, i.to_number(value)?); }
    let bigint = i.to_bigint(value)?;
    if view.buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
    if binary_view_out_of_bounds(view) { return Err(Throw::type_error("out-of-bounds DataView")); }
    let end = offset.checked_add(8).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
    let mut buffer = view.buffer.borrow_mut();
    let mut bytes = bigint.low_u64().to_le_bytes();
    if !little { bytes.reverse(); }
    let start = view.offset + offset;
    buffer.bytes[start..start + 8].copy_from_slice(&bytes);
    Ok(())
}

fn binary_property(value: JsValue) -> Property {
    Property { accessor: false, value, writable: false, enumerable: false, configurable: true, getter: None, setter: None }
}
fn binary_writable_property(value: JsValue) -> Property {
    Property { accessor: false, value, writable: true, enumerable: false, configurable: true, getter: None, setter: None }
}
fn binary_locked_property(value: JsValue) -> Property {
    Property { accessor: false, value, writable: false, enumerable: false, configurable: false, getter: None, setter: None }
}
fn binary_configure_constructor(object: &Obj, name: &str, length: usize, prototype: &Obj) {
    let mut object = object.borrow_mut();
    let _ = object.define("name", binary_property(JsValue::str(name)));
    let _ = object.define("length", binary_property(JsValue::num(length as f64)));
    let _ = object.define("prototype", binary_locked_property(JsValue::Object(prototype.clone())));
}
fn binary_accessor(function: JsValue, setter: Option<JsValue>, enumerable: bool) -> Property {
    Property { accessor: true, value: JsValue::Undefined, writable: false, enumerable, configurable: true,
        getter: Some(function), setter }
}
fn binary_method(i: &Interpreter, object: &Obj, name: &str, f: crate::value::NativeFn, arity: usize) {
    let method = Interpreter::native(name, f, &i.function_proto);
    if let JsValue::Object(callable) = &method {
        callable.borrow_mut().constructible = false;
        callable.borrow_mut().delete_own("prototype");
        let _ = callable.borrow_mut().define("length", binary_property(JsValue::num(arity as f64)));
        let _ = callable.borrow_mut().define("name", binary_property(JsValue::str(name)));
    }
    let _ = object.borrow_mut().define(name, binary_writable_property(method));
}
fn binary_getter(i: &Interpreter, object: &Obj, name: &str, f: crate::value::NativeFn) {
    let method = Interpreter::native(&format!("get {name}"), f, &i.function_proto);
    if let JsValue::Object(callable) = &method {
        // Accessor functions are built-in functions that are not constructors:
        // remove the prototype and every own property a constructor would keep.
        callable.borrow_mut().constructible = false;
        callable.borrow_mut().delete_own("prototype");
    }
    let _ = object.borrow_mut().define(name, binary_accessor(method, None, false));
}

impl Interpreter {
    fn normalize_throw(&self, thrown: Throw) -> Throw {
        if let JsValue::Object(object) = &thrown.0 {
            let name = {
                let object = object.borrow();
                if !matches!(object.exotic, Exotic::Error) || object.proto.is_some() { None }
                else { match object.get_own("name") {
                    Some(JsValue::Str(name)) => Some(name.to_utf8_lossy()),
                    _ => None,
                } }
            };
            if let Some(name) = name {
                if let Some((_, prototype)) = self.intrinsic_constructors.get(name.as_str()) {
                    object.borrow_mut().proto = Some(prototype.clone());
                }
            }
        }
        thrown
    }
    fn binary_new_buffer(&mut self, bytes: Vec<u8>, max: usize, resizable: bool, shared: bool) -> JsValue {
        let name = if shared { "SharedArrayBuffer" } else { "ArrayBuffer" };
        let prototype = self.intrinsic_constructors.get(name).map(|(_, prototype)| prototype.clone())
            .unwrap_or_else(|| self.object_proto.clone());
        let mut raw = JsObject::ordinary(); raw.proto = Some(prototype);
        let object = Obj(Rc::new(RefCell::new(raw)));
        binary_register(&object, BinaryState::Buffer(Rc::new(RefCell::new(BinaryBuffer {
            bytes, max, resizable, shared, detached: false, immutable: false,
        }))));
        JsValue::Object(object)
    }
    fn binary_view_on(&mut self, object: Obj, buffer_object: Obj, offset: usize, length: Option<usize>, kind: Option<BinaryKind>) -> Result<JsValue, Throw> {
        let buffer = binary_buffer(&JsValue::Object(buffer_object.clone()))?;
        let size = kind.map_or(1, BinaryKind::size);
        let bytes_len = buffer.borrow().bytes.len();
        if buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
        if offset > bytes_len || offset % size != 0
            || length.is_some_and(|len| len > bytes_len - offset) {
            return Err(Throw::range_error("invalid view bounds"));
        }
        if kind.is_some() && length.is_some_and(|length| length % size != 0) {
            return Err(Throw::range_error("unaligned TypedArray length"));
        }
        if kind.is_some() { object.borrow_mut().exotic = Exotic::TypedArray; }
        binary_register(&object, BinaryState::View(BinaryView { buffer, buffer_object, offset, length, kind }));
        Ok(JsValue::Object(object))
    }
    fn binary_new_view(&mut self, buffer_object: Obj, offset: usize, length: Option<usize>, kind: Option<BinaryKind>) -> Result<JsValue, Throw> {
        let name = kind.map_or("DataView", BinaryKind::name);
        let proto = self.intrinsic_constructors.get(name).map(|(_, prototype)| prototype.clone())
            .unwrap_or_else(|| self.object_proto.clone());
        let mut raw = JsObject::ordinary(); raw.proto = Some(proto);
        let object = Obj(Rc::new(RefCell::new(raw)));
        self.binary_view_on(object, buffer_object, offset, length, kind)
    }
    pub fn binding_create_array_buffer(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        let len = bytes.len(); Ok(self.binary_new_buffer(bytes, len, false, false))
    }
    pub fn binding_create_uint8_array(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        let length = bytes.len();
        let buffer = self.binding_create_array_buffer(bytes)?;
        self.binary_new_view(binary_obj(&buffer)?, 0, Some(length), Some(BinaryKind::Uint8))
    }
    /// Validate the internal Uint8Array view brand for a host operation.
    /// Prototype identity is deliberately irrelevant, including across realms.
    pub fn binding_uint8_array_info(&self, value: &JsValue) -> Result<Option<usize>, Throw> {
        let JsValue::Object(object) = value else { return Ok(None); };
        let Some(BinaryState::View(view)) = binary_state(object) else { return Ok(None); };
        if view.kind != Some(BinaryKind::Uint8) { return Ok(None); }
        if binary_view_out_of_bounds(&view) {
            return Err(Throw::type_error("detached or out-of-bounds Uint8Array"));
        }
        Ok(Some(binary_view_bytes(&view)))
    }
    /// Write through an existing Uint8Array's view, preserving bytes outside it.
    pub fn binding_write_uint8_array(&mut self, value: &JsValue, offset: usize, bytes: &[u8]) -> Result<(), Throw> {
        let length = self.binding_uint8_array_info(value)?
            .ok_or_else(|| Throw::type_error("destination is not a Uint8Array"))?;
        if offset.checked_add(bytes.len()).is_none_or(|end| end > length) {
            return Err(Throw::range_error("write exceeds Uint8Array view"));
        }
        let Some(BinaryState::View(view)) = binary_state(&binary_obj(value)?) else { unreachable!(); };
        let mut buffer = view.buffer.borrow_mut();
        if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
        let start = view.offset + offset;
        buffer.bytes[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    /// Return a byte snapshot of an ArrayBuffer or ArrayBufferView for host
    /// bindings. A detached backing store is an error even for an empty view.
    pub fn binding_buffer_source_bytes(&self, value: &JsValue) -> Result<Option<Vec<u8>>, Throw> {
        let JsValue::Object(object) = value else { return Ok(None) };
        match binary_state(object) {
            Some(BinaryState::Buffer(buffer)) => {
                let buffer = buffer.borrow();
                if buffer.shared { return Ok(None); }
                if buffer.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
                Ok(Some(buffer.bytes.clone()))
            }
            Some(BinaryState::View(view)) => {
                let buffer = view.buffer.borrow();
                if buffer.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
                if binary_view_out_of_bounds(&view) { return Ok(Some(Vec::new())); }
                let byte_length = binary_view_bytes(&view);
                Ok(Some(buffer.bytes[view.offset..view.offset + byte_length].to_vec()))
            }
            None => Ok(None),
        }
    }
    pub fn binding_detach_array_buffer(&mut self, value: &JsValue) -> Result<(), Throw> {
        let buffer = binary_buffer(value)?;
        let mut buffer = buffer.borrow_mut();
        if buffer.shared { return Err(Throw::type_error("SharedArrayBuffer cannot be detached")); }
        if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer cannot be detached")); }
        if buffer.detached { return Ok(()); }
        buffer.bytes.clear();
        buffer.detached = true;
        Ok(())
    }

    fn install_binary_builtins(&mut self) {
        let ab = Self::native_constructor("ArrayBuffer", binary_array_buffer_ctor, &self.function_proto);
        let sab = Self::native_constructor("SharedArrayBuffer", binary_shared_buffer_ctor, &self.function_proto);
        for (name, ctor, shared) in [("ArrayBuffer", &ab, false), ("SharedArrayBuffer", &sab, true)] {
            let JsValue::Object(function) = ctor else { unreachable!() };
            let JsValue::Object(proto) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
            proto.borrow_mut().proto = Some(self.object_proto.clone());
            let _ = proto.borrow_mut().define("constructor", binary_writable_property(ctor.clone()));
            let _ = proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), binary_property(JsValue::str(name)));
            binary_species(self, function);
            let getter = if shared { binary_shared_buffer_getter as crate::value::NativeFn } else { binary_buffer_getter as crate::value::NativeFn };
            binary_getter(self, &proto, "byteLength", getter);
            binary_getter(self, &proto, "maxByteLength", getter);
            binary_getter(self, &proto, if shared { "growable" } else { "resizable" }, getter);
            if !shared {
                binary_getter(self, &proto, "detached", binary_buffer_getter);
                binary_getter(self, &proto, "immutable", binary_buffer_getter);
                binary_method(self, &proto, "resize", binary_buffer_method, 1);
                binary_method(self, &proto, "slice", binary_buffer_method, 2);
                binary_method(self, &proto, "sliceToImmutable", binary_buffer_method, 2);
                binary_method(self, &proto, "transfer", binary_buffer_method, 0);
                binary_method(self, &proto, "transferToFixedLength", binary_buffer_method, 0);
                binary_method(self, &proto, "transferToImmutable", binary_buffer_method, 0);
            } else {
                binary_method(self, &proto, "grow", binary_shared_buffer_method, 1);
                binary_method(self, &proto, "slice", binary_shared_buffer_method, 2);
            }
            binary_configure_constructor(function, name, 1, &proto);
            if !shared { binary_method(self, function, "isView", binary_is_view, 1); }
            self.define_global(name, ctor.clone());
        }

        let data = Self::native_constructor("DataView", binary_data_view_ctor, &self.function_proto);
        if let JsValue::Object(function) = &data {
            let JsValue::Object(proto) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
            proto.borrow_mut().proto = Some(self.object_proto.clone());
            let _ = proto.borrow_mut().define("constructor", binary_writable_property(data.clone()));
            binary_configure_constructor(function, "DataView", 1, &proto);
            let _ = proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), binary_property(JsValue::str("DataView")));
            for name in ["buffer", "byteOffset", "byteLength"] { binary_getter(self, &proto, name, binary_data_view_getter); }
            for name in ["Int8", "Uint8", "Int16", "Uint16", "Int32", "Uint32", "BigInt64", "BigUint64", "Float16", "Float32", "Float64"] {
                binary_method(self, &proto, &format!("get{name}"), binary_data_view_method, 1);
                binary_method(self, &proto, &format!("set{name}"), binary_data_view_method, 2);
            }
        }
        self.define_global("DataView", data);

        let intrinsic = Self::native_constructor("TypedArray", binary_typed_ctor, &self.function_proto);
        let JsValue::Object(intrinsic_obj) = &intrinsic else { unreachable!() };
        let JsValue::Object(typed_proto) = intrinsic_obj.borrow().get_own("prototype").unwrap() else { unreachable!() };
        typed_proto.borrow_mut().proto = Some(self.object_proto.clone());
        for name in ["buffer", "byteOffset", "byteLength", "length"] { binary_getter(self, &typed_proto, name, binary_view_getter); }
        for (name, arity) in [
            ("at", 1), ("copyWithin", 2), ("entries", 0), ("every", 1), ("fill", 1),
            ("filter", 1), ("find", 1), ("findIndex", 1), ("findLast", 1), ("findLastIndex", 1),
            ("forEach", 1), ("includes", 1), ("indexOf", 1), ("join", 1), ("keys", 0),
            ("lastIndexOf", 1), ("map", 1), ("reduce", 1), ("reduceRight", 1),
            ("reverse", 0), ("set", 1), ("slice", 2), ("some", 1), ("sort", 1),
            ("subarray", 2), ("toLocaleString", 0), ("toReversed", 0), ("toSorted", 1),
            ("values", 0), ("with", 2),
        ] { binary_method(self, &typed_proto, name, binary_typed_method, arity); }
        let values_method = typed_proto.borrow().get_own("values").unwrap();
        let _ = typed_proto.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), binary_writable_property(values_method));
        let _ = typed_proto.borrow_mut().define("constructor", binary_writable_property(intrinsic.clone()));
        binary_configure_constructor(intrinsic_obj, "TypedArray", 0, &typed_proto);
        let tag_getter = Self::native("get [Symbol.toStringTag]", binary_typed_tag, &self.function_proto);
        if let JsValue::Object(getter) = tag_getter {
            let _ = typed_proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                binary_accessor(JsValue::Object(getter.clone()), None, false));
        }
        binary_species(self, intrinsic_obj);
        binary_method(self, intrinsic_obj, "from", binary_typed_static, 1);
        binary_method(self, intrinsic_obj, "of", binary_typed_static, 0);
        self.define_global("TypedArray", intrinsic.clone());
        for kind in [
            BinaryKind::Int8, BinaryKind::Uint8, BinaryKind::Uint8Clamped,
            BinaryKind::Int16, BinaryKind::Uint16, BinaryKind::Int32, BinaryKind::Uint32,
            BinaryKind::BigInt64, BinaryKind::BigUint64, BinaryKind::Float16,
            BinaryKind::Float32, BinaryKind::Float64,
        ] {
            let name = kind.name();
            let ctor = Self::native_constructor(name, binary_typed_ctor, &self.function_proto);
            let JsValue::Object(function) = &ctor else { unreachable!() };
            function.borrow_mut().proto = Some(intrinsic_obj.clone());
            let JsValue::Object(proto) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
            proto.borrow_mut().proto = Some(typed_proto.clone());
            let _ = proto.borrow_mut().define("constructor", binary_writable_property(ctor.clone()));
            binary_configure_constructor(function, name, 3, &proto);
            binary_species(self, function);
            let _ = function.borrow_mut().define("BYTES_PER_ELEMENT", binary_locked_property(JsValue::num(kind.size() as f64)));
            let _ = proto.borrow_mut().define("BYTES_PER_ELEMENT", binary_locked_property(JsValue::num(kind.size() as f64)));
            if kind == BinaryKind::Uint8 { self.install_uint8_base64_hex(function, &proto); }
            self.define_global(name, ctor);
        }
        let atomics = self.object_value();
        if let JsValue::Object(object) = &atomics {
            for (name, arity) in [
                ("add", 3), ("and", 3), ("compareExchange", 4), ("exchange", 3),
                ("isLockFree", 1), ("load", 2), ("notify", 3), ("or", 3),
                ("pause", 0), ("store", 3), ("sub", 3), ("wait", 4), ("waitAsync", 4), ("xor", 3),
            ] { binary_method(self, object, name, binary_atomics, arity); }
            let _ = object.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                binary_property(JsValue::str("Atomics")));
        }
        self.define_global("Atomics", atomics);
    }
}

fn binary_species(i: &Interpreter, constructor: &Obj) {
    let getter = Interpreter::native("get [Symbol.species]", binary_species_getter, &i.function_proto);
    if let JsValue::Object(getter) = getter {
        let _ = constructor.borrow_mut().define(WellKnownSymbol::Species.symbol(),
            binary_accessor(JsValue::Object(getter.clone()), None, false));
    }
}
fn binary_species_getter(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> { Ok(this) }
fn binary_typed_tag(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(binary_typed(&this).map(|(_, kind)| JsValue::str(kind.name())).unwrap_or(JsValue::Undefined))
}

include!("builtins_binary_exotic.rs");

fn binary_current_name(i: &Interpreter) -> String {
    let name = i.native_stack.last().and_then(|o| o.borrow().func.as_ref().map(|f| f.name.clone()))
        .unwrap_or_default();
    name.strip_prefix("get ").unwrap_or(&name).to_string()
}
fn binary_require_new(i: &Interpreter) -> Result<(), Throw> {
    if i.current_new_target().is_some() { Ok(()) }
    else { Err(Throw::type_error("binary data constructor requires new")) }
}
fn binary_array_buffer_ctor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(i)?;
    let length = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let (max, resizable) = binary_maximum_length(i, args.get(1), length)?;
    binary_allocate_buffer(i, length, max, resizable, false)
}
fn binary_shared_buffer_ctor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(i)?;
    let length = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let (max, growable) = binary_maximum_length(i, args.get(1), length)?;
    binary_allocate_buffer(i, length, max, growable, true)
}
fn binary_buffer_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let buffer = binary_buffer(&this)?;
    let b = buffer.borrow();
    if b.shared { return Err(Throw::type_error("ArrayBuffer getter on SharedArrayBuffer")); }
    Ok(match name.as_str() {
        "byteLength" => JsValue::num(if b.detached { 0 } else { b.bytes.len() } as f64),
        "maxByteLength" => JsValue::num(if b.detached { 0 } else { b.max } as f64),
        "resizable" | "growable" => JsValue::Bool(b.resizable),
        "detached" => JsValue::Bool(b.detached),
        "immutable" => JsValue::Bool(b.immutable),
        _ => JsValue::Undefined,
    })
}
fn binary_shared_buffer_getter(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let buffer = binary_buffer(&this)?;
    if !buffer.borrow().shared { return Err(Throw::type_error("SharedArrayBuffer getter on ArrayBuffer")); }
    let name = binary_current_name(i);
    let b = buffer.borrow();
    Ok(match name.as_str() {
        "byteLength" => JsValue::num(b.bytes.len() as f64),
        "maxByteLength" => JsValue::num(b.max as f64),
        "growable" => JsValue::Bool(b.resizable),
        _ => { let _ = args; JsValue::Undefined },
    })
}
fn binary_shared_buffer_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_buffer_operation(i, this, args, true)
}
fn binary_buffer_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_buffer_operation(i, this, args, false)
}
fn binary_buffer_operation(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, expect_shared: bool) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let buffer = binary_buffer(&this)?;
    if buffer.borrow().shared != expect_shared { return Err(Throw::type_error("wrong buffer brand")); }
    if name != "resize" && buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    match name.as_str() {
        "resize" | "grow" => binary_resize_buffer(i, &buffer, args.first().unwrap_or(&JsValue::Undefined)),
        "slice" | "sliceToImmutable" => {
            let (length, shared) = { let b = buffer.borrow(); (b.bytes.len() as i64, b.shared) };
            let begin = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length)? } else { 0 };
            let end = binary_relative(i, args.get(1), length)?;
            let amount = end.max(begin) - begin;
            let copied = if name == "slice" { Some(binary_species_buffer(i, &this, amount, shared)?) }
                else { None };
            let source = {
                let b = buffer.borrow();
                if b.detached { return Err(Throw::type_error("source detached during slice")); }
                // Immutable slicing requires every originally resolved byte;
                // ordinary slice alone clips after author-side shrinking.
                if name == "sliceToImmutable" && b.bytes.len() < end {
                    return Err(Throw::range_error("source shrank below immutable slice bounds"));
                }
                let copied_end = end.min(b.bytes.len());
                if begin < copied_end { binary_copy_bytes(&b.bytes[begin..copied_end])? } else { Vec::new() }
            };
            let copied = copied.unwrap_or_else(|| i.binary_new_buffer(vec![0; amount], amount, false, shared));
            {
                let target = binary_buffer(&copied)?;
                let mut target = target.borrow_mut();
                if target.detached || target.immutable || target.bytes.len() < amount || target.shared != shared {
                    return Err(Throw::type_error("invalid buffer species result"));
                }
                target.bytes[..source.len()].copy_from_slice(&source);
            }
            if name == "sliceToImmutable" { binary_buffer(&copied)?.borrow_mut().immutable = true; }
            Ok(copied)
        }
        "transfer" | "transferToFixedLength" | "transferToImmutable" => {
            let length = if args.first().is_none_or(|v| matches!(v, JsValue::Undefined)) {
                buffer.borrow().bytes.len()
            } else { binary_index(i, &args[0])? };
            if buffer.borrow().immutable { return Err(Throw::type_error("cannot transfer immutable buffer")); }
            let mut b = buffer.borrow_mut();
            if b.detached { return Err(Throw::type_error("source detached during transfer")); }
            if length > b.max && name == "transfer" && b.resizable { return Err(Throw::range_error("transfer exceeds maxByteLength")); }
            if length > 256 * 1024 * 1024 { return Err(Throw::range_error("transfer too large")); }
            let mut bytes = binary_copy_bytes(&b.bytes)?;
            bytes.try_reserve(length.saturating_sub(bytes.len())).map_err(|_| Throw::range_error("buffer allocation failed"))?;
            bytes.resize(length, 0);
            let (max, resizable) = if name == "transfer" && b.resizable { (b.max, true) } else { (length, false) };
            b.bytes.clear(); b.detached = true;
            let moved = i.binary_new_buffer(bytes, max, resizable, false);
            if name == "transferToImmutable" { binary_buffer(&moved)?.borrow_mut().immutable = true; }
            Ok(moved)
        }
        _ => Err(Throw::type_error("unknown ArrayBuffer operation")),
    }
}
fn binary_species_buffer(i: &mut Interpreter, source: &JsValue, length: usize, shared: bool) -> Result<JsValue, Throw> {
    let default = i.intrinsic_constructors.get(if shared { "SharedArrayBuffer" } else { "ArrayBuffer" }).map(|(constructor, _)| JsValue::Object(constructor.clone()))
        .ok_or_else(|| Throw::type_error("missing buffer constructor"))?;
    let constructor = i.get_prop(source, "constructor")?;
    let selected = if matches!(constructor, JsValue::Undefined) { default.clone() }
        else {
            if !constructor.is_object() { return Err(Throw::type_error("buffer constructor is not an object")); }
            let species = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
            if matches!(species, JsValue::Undefined | JsValue::Null) { default.clone() } else { species }
        };
    let result = i.construct(&selected, vec![JsValue::num(length as f64)])?;
    if result == *source { return Err(Throw::type_error("buffer species returned source")); }
    let target = binary_buffer(&result)?;
    if target.borrow().shared != shared { return Err(Throw::type_error("wrong buffer species")); }
    Ok(result)
}
fn binary_relative(i: &mut Interpreter, arg: Option<&JsValue>, length: i64) -> Result<usize, Throw> {
    let n = match arg { None | Some(JsValue::Undefined) => length as f64,
        Some(value) => i.to_number(value)? };
    let integer = if n.is_nan() { 0 } else if n.is_infinite() { if n.is_sign_negative() { i64::MIN } else { i64::MAX } }
        else { n.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64 };
    Ok(if integer < 0 { length.saturating_add(integer).max(0) } else { integer.min(length) } as usize)
}
fn binary_is_view(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(args.first().and_then(|v| if let JsValue::Object(o) = v { binary_state(o) } else { None })
        .is_some_and(|state| matches!(state, BinaryState::View(_)))))
}
fn binary_data_view_ctor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("DataView requires new")); }
    let buffer_object = binary_obj(args.first().unwrap_or(&JsValue::Undefined))?;
    let buffer = binary_buffer(&JsValue::Object(buffer_object.clone()))?;
    let offset = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    let buffer_byte_length = {
        let b = buffer.borrow();
        if b.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
        if offset > b.bytes.len() { return Err(Throw::range_error("invalid view offset")); }
        b.bytes.len()
    };
    let length = if let Some(arg) = args.get(2).filter(|x| !matches!(x, JsValue::Undefined)) {
        Some(binary_index(i, arg)?)
    } else if buffer.borrow().resizable { None }
    else { Some(buffer_byte_length - offset) };
    {
        if length.is_some_and(|length| offset.checked_add(length).is_none_or(|end| end > buffer_byte_length)) {
            return Err(Throw::range_error("invalid DataView length"));
        }
    }
    let object = binary_constructor_object(i)?;
    i.binary_view_on(object, buffer_object, offset, length, None)
}
fn binary_view_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let view = binary_view(&this)?;
    if view.kind.is_none() { return Err(Throw::type_error("TypedArray getter on DataView")); }
    Ok(match name.as_str() {
        "buffer" => JsValue::Object(view.buffer_object),
        "byteOffset" => JsValue::num(if binary_view_out_of_bounds(&view) { 0 } else { view.offset } as f64),
        "byteLength" => JsValue::num(binary_view_bytes(&view) as f64),
        "length" => JsValue::num(binary_length(&view) as f64),
        _ => JsValue::Undefined,
    })
}
fn binary_data_view_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let view = binary_view(&this)?;
    if view.kind.is_some() { return Err(Throw::type_error("DataView getter on TypedArray")); }
    let buffer_length = view.buffer.borrow().bytes.len();
    if name != "buffer" && (view.buffer.borrow().detached || view.offset > buffer_length
        || view.length.is_some_and(|length| view.offset.checked_add(length).is_none_or(|end| end > buffer_length))) {
        return Err(Throw::type_error("out-of-bounds DataView"));
    }
    Ok(match name.as_str() {
        "buffer" => JsValue::Object(view.buffer_object),
        "byteOffset" => JsValue::num(view.offset as f64),
        "byteLength" => JsValue::num(binary_view_bytes(&view) as f64),
        _ => JsValue::Undefined,
    })
}
fn binary_data_view_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let (set, suffix) = if let Some(suffix) = name.strip_prefix("set") { (true, suffix) }
        else { (false, name.strip_prefix("get").unwrap_or("")) };
    let kind = match suffix {
        "Int8" => BinaryKind::Int8, "Uint8" => BinaryKind::Uint8,
        "Int16" => BinaryKind::Int16, "Uint16" => BinaryKind::Uint16,
        "Int32" => BinaryKind::Int32, "Uint32" => BinaryKind::Uint32,
        "BigInt64" => BinaryKind::BigInt64, "BigUint64" => BinaryKind::BigUint64,
        "Float16" => BinaryKind::Float16, "Float32" => BinaryKind::Float32,
        "Float64" => BinaryKind::Float64,
        _ => return Err(Throw::type_error("unknown DataView operation")),
    };
    let view = binary_view(&this)?;
    if view.kind.is_some() { return Err(Throw::type_error("not a DataView")); }
    // SetViewValue checks IsImmutableBuffer before converting any argument.
    if set && view.buffer.borrow().immutable {
        return Err(Throw::type_error("immutable DataView buffer"));
    }
    let offset = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let little = args.get(if set { 2 } else { 1 }).is_some_and(JsValue::to_boolean);
    if set {
        binary_write_value(i, &view, kind, offset, little, args.get(1).unwrap_or(&JsValue::Undefined))?;
        Ok(JsValue::Undefined)
    } else { binary_read(&view, kind, offset, little) }
}

fn binary_typed_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("TypedArray requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("TypedArray requires new")); };
    let name = binary_current_name(i);
    let kind = BinaryKind::from_name(&name).ok_or_else(|| Throw::type_error("TypedArray requires a concrete constructor"))?;
    let first = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(first_object) = &first {
        match binary_state(first_object) {
            Some(BinaryState::Buffer(buffer)) => {
                let offset = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
                if offset % kind.size() != 0 { return Err(Throw::range_error("unaligned TypedArray offset")); }
                let length = if let Some(arg) = args.get(2).filter(|x| !matches!(x, JsValue::Undefined)) {
                    Some(binary_index(i, arg)?.checked_mul(kind.size()).ok_or_else(|| Throw::range_error("view too large"))?)
                } else if buffer.borrow().resizable { None }
                else { Some(buffer.borrow().bytes.len().saturating_sub(offset)) };
                return i.binary_view_on(object, first_object.clone(), offset, length, Some(kind));
            }
            Some(BinaryState::View(view)) if view.kind.is_some() => {
                if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds source TypedArray")); }
                let old_kind = view.kind.unwrap();
                if old_kind == kind {
                    let bytes = {
                        let source = view.buffer.borrow();
                        binary_copy_bytes(&source.bytes[view.offset..view.offset + binary_view_bytes(&view)])?
                    };
                    let length = bytes.len();
                    let buffer = i.binary_new_buffer(bytes, length, false, false);
                    return i.binary_view_on(object, binary_obj(&buffer)?, 0, Some(length), Some(kind));
                }
                if old_kind.big() != kind.big() { return Err(Throw::type_error("mixed BigInt and Number TypedArrays")); }
                let values = (0..binary_length(&view)).map(|index|
                    binary_read(&view, old_kind, index * old_kind.size(), true)
                ).collect::<Result<Vec<_>, _>>()?;
                return binary_typed_fill(i, object, kind, values);
            }
            _ => {}
        }
    }
    if matches!(first, JsValue::Undefined | JsValue::Num(_) | JsValue::BigInt(_) | JsValue::Symbol(_) | JsValue::Bool(_) | JsValue::Null) {
        let length = binary_index(i, &first)?;
        let byte_length = length.checked_mul(kind.size()).ok_or_else(|| Throw::range_error("TypedArray too large"))?;
        if byte_length > 256 * 1024 * 1024 { return Err(Throw::range_error("TypedArray too large")); }
        let buffer = i.binary_new_buffer(vec![0; byte_length], byte_length, false, false);
        return i.binary_view_on(object, binary_obj(&buffer)?, 0, Some(byte_length), Some(kind));
    }
    let method = binary_source_method(i, &first)?;
    if !method.is_undefined() && !method.is_null() {
        let values = binary_iterator_values(i, &first, &method)?;
        return binary_typed_fill(i, object, kind, values);
    }
    let source = array_receiver(i, first)?;
    let length = binary_array_like_length(i, &source)?;
    let byte_length = length.checked_mul(kind.size()).ok_or_else(|| Throw::range_error("TypedArray too large"))?;
    if byte_length > 256 * 1024 * 1024 { return Err(Throw::range_error("TypedArray too large")); }
    let buffer = i.binary_new_buffer(vec![0; byte_length], byte_length, false, false);
    let result = i.binary_view_on(object, binary_obj(&buffer)?, 0, Some(byte_length), Some(kind))?;
    for index in 0..length {
        let value = i.get_prop(&source, index.to_string())?;
        i.set_prop(&result, index.to_string(), value)?;
    }
    Ok(result)
}
fn binary_typed_fill(i: &mut Interpreter, object: Obj, kind: BinaryKind, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let bytes = values.len().checked_mul(kind.size()).ok_or_else(|| Throw::range_error("TypedArray too large"))?;
    if bytes > 256 * 1024 * 1024 { return Err(Throw::range_error("TypedArray too large")); }
    let buffer = i.binary_new_buffer(vec![0; bytes], bytes, false, false);
    let view = i.binary_view_on(object, binary_obj(&buffer)?, 0, Some(bytes), Some(kind))?;
    let state = binary_typed(&view)?.0;
    for (index, value) in values.into_iter().enumerate() {
        binary_write_value(i, &state, kind, index * kind.size(), true, &value)?;
    }
    Ok(view)
}
fn binary_typed_from_values(i: &mut Interpreter, kind: BinaryKind, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (result, view) = binary_typed_intrinsic_length(i, kind, values.len())?;
    for (index, value) in values.iter().enumerate() {
        binary_set_element(i, &view, kind, Some(index), value)?;
    }
    Ok(result)
}
fn binary_typed_static(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let method = binary_current_name(i);
    if !is_constructor(&this) { return Err(Throw::type_error("TypedArray factory receiver is not a constructor")); }
    if method == "of" {
        let values = args;
        let (result, view, kind) = binary_construct_typed_object(i, &this, values.len())?;
        if binary_length(&view) < values.len() { return Err(Throw::type_error("TypedArray constructor returned a short array")); }
        for (index, value) in values.into_iter().enumerate() {
            i.set_prop(&result, index.to_string(), value)?;
        }
        return Ok(result);
    }
    // %TypedArray%.from: IsCallable(mapfn) is checked before Get(items, @@iterator).
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    let callback = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)).cloned();
    if let Some(callback) = &callback {
        if callable_function(callback).is_none() {
            return Err(Throw::type_error("mapfn is not a function"));
        }
    }
    let method = binary_source_method(i, &source)?;
    let values = if !method.is_undefined() && !method.is_null() {
        Some(binary_iterator_values(i, &source, &method)?)
    } else { None };
    let source = if values.is_none() { array_receiver(i, source)? } else { source };
    let length = if let Some(values) = &values { values.len() } else { binary_array_like_length(i, &source)? };
    let (result, _, _) = binary_construct_typed_object(i, &this, length)?;
    let this_arg = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    for index in 0..length {
        let value = if let Some(values) = &values { values[index].clone() } else { i.get_prop(&source, index.to_string())? };
        let mapped = match &callback {
            Some(callback) => i.call_value(callback, this_arg.clone(), vec![value, JsValue::num(index as f64)])?,
            None => value,
        };
        // IntegerIndexedElementSet converts even when callback-side resizing
        // invalidated the index. Missing writes then succeed without mutation.
        i.set_prop(&result, index.to_string(), mapped)?;
    }
    Ok(result)
}

fn binary_typed_values(view: &BinaryView, kind: BinaryKind) -> Result<Vec<JsValue>, Throw> {
    (0..binary_length(view)).map(|j| binary_read(view, kind, j * kind.size(), true)).collect()
}

include!("builtins_binary_prototype.rs");

fn binary_array(i: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut raw = JsObject::array();
    raw.proto = Some(i.array_proto.clone());
    let count = values.len();
    for (index, value) in values.into_iter().enumerate() { raw.set_own(index.to_string(), value); }
    let mut length = Property::data(JsValue::num(count as f64));
    length.enumerable = false;
    length.configurable = false;
    raw.define("length", length).unwrap();
    Ok(JsValue::object(raw))
}

fn binary_typed_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    if name == "toSorted" && args.first().is_some_and(|value|
        !value.is_undefined() && callable_function(value).is_none()) {
        return Err(Throw::type_error("comparefn must be a function"));
    }
    let (view, kind) = binary_typed(&this)?;
    let out_of_bounds = binary_view_out_of_bounds(&view);
    if out_of_bounds && name != "subarray" {
        return Err(Throw::type_error("out-of-bounds TypedArray"));
    }
    let length = binary_length(&view);
    // Iterators and callback methods must not eagerly snapshot their elements.
    // Only algorithms explicitly creating a value list do so before callbacks.
    let values = if matches!(name.as_str(), "sort") {
        binary_typed_values(&view, kind)?
    } else { Vec::new() };
    let arg = |n: usize| args.get(n).cloned().unwrap_or(JsValue::Undefined);
    match name.as_str() {
        "at" => {
            let number = i.to_number(&arg(0))?;
            let relative = if number.is_nan() { 0.0 } else { number.trunc() };
            let index = if relative < 0.0 { length as f64 + relative } else { relative };
            if index >= 0.0 && index < length as f64 && index < binary_length(&view) as f64 {
                binary_read(&view, kind, index as usize * kind.size(), true)
            } else { Ok(JsValue::Undefined) }
        }
        "join" => {
            let separator = if !matches!(arg(0), JsValue::Undefined) { i.to_js_string(&arg(0))? } else { crate::value::JsString::from(",") };
            let mut units = Vec::new();
            for index in 0..length {
                if index != 0 { units.extend_from_slice(separator.units()); }
                let value = i.get_prop(&this, index.to_string())?;
                if !matches!(value, JsValue::Undefined | JsValue::Null) { units.extend_from_slice(i.to_js_string(&value)?.units()); }
            }
            Ok(JsValue::Str(crate::value::JsString::from_units(units)))
        }
        "toLocaleString" => {
            let mut units = Vec::new();
            for index in 0..length {
                if index != 0 { units.push(44); }
                let value = i.get_prop(&this, index.to_string())?;
                if matches!(value, JsValue::Undefined | JsValue::Null) { continue; }
                let method = i.get_prop(&value, "toLocaleString")?;
                let rendered = i.call_value(&method, value.clone(), args.clone())?;
                units.extend_from_slice(i.to_js_string(&rendered)?.units());
            }
            Ok(JsValue::Str(crate::value::JsString::from_units(units)))
        }
        "includes" | "indexOf" | "lastIndexOf" => {
            let needle = arg(0);
            if length == 0 { return Ok(if name == "includes" { JsValue::Bool(false) } else { JsValue::num(-1.0) }); }
            let relative = if args.get(1).is_none() && name == "lastIndexOf" {
                length as f64 - 1.0
            } else { let n = i.to_number(&arg(1))?; if n.is_nan() { 0.0 } else { n.trunc() } };
            let start = if relative < 0.0 { length as f64 + relative } else { relative };
            if (name == "lastIndexOf" && start < 0.0) || (name != "lastIndexOf" && start >= length as f64) {
                return Ok(if name == "includes" { JsValue::Bool(false) } else { JsValue::num(-1.0) });
            }
            let indices: Vec<usize> = if name == "lastIndexOf" {
                (0..=start.min(length as f64 - 1.0) as usize).rev().collect()
            } else { (start.max(0.0) as usize..length).collect() };
            for index in indices {
                // HasProperty skips absent integer indices for indexOf, while
                // includes uses Get and can match undefined after a shrink.
                if name != "includes" && (binary_view_out_of_bounds(&view) || index >= binary_length(&view)) { continue; }
                let value = i.get_prop(&this, index.to_string())?;
                let equal = i.strict_eq(&value, &needle) || (name == "includes" &&
                    matches!((&value, &needle), (JsValue::Num(a), JsValue::Num(b)) if a.is_nan() && b.is_nan()));
                if equal { return Ok(if name == "includes" { JsValue::Bool(true) } else { JsValue::num(index as f64) }); }
            }
            Ok(if name == "includes" { JsValue::Bool(false) } else { JsValue::num(-1.0) })
        }
        "fill" => {
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let fill = if kind.big() { Interpreter::bigint_value(i.to_bigint(&arg(0))?) }
                else { JsValue::num(i.to_number(&arg(0))?) };
            let start = if args.get(1).is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.get(1), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(2), length as i64)?;
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("view changed during fill")); }
            for j in start..end.min(binary_length(&view)) { binary_write_value(i, &view, kind, j * kind.size(), true, &fill)?; }
            Ok(this)
        }
        "toReversed" => {
            let (result, target) = binary_typed_intrinsic_length(i, kind, length)?;
            for index in 0..length {
                let value = binary_get_index(i, &binary_obj(&this)?, &PropertyKey::from((length-index-1).to_string())).unwrap()?;
                binary_set_element(i, &target, kind, Some(index), &value)?;
            }
            Ok(result)
        }
        "reverse" => {
            let mut buffer = view.buffer.borrow_mut();
            if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            for index in 0..length / 2 {
                for byte in 0..kind.size() {
                    buffer.bytes.swap(view.offset + index * kind.size() + byte,
                        view.offset + (length - 1 - index) * kind.size() + byte);
                }
            }
            Ok(this)
        }
        "slice" => binary_typed_slice(i, &this, &view, kind, length, &args),
        "subarray" => {
            let start = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(1), length as i64)?.max(start);
            let mut arguments = vec![JsValue::Object(view.buffer_object),
                JsValue::num((view.offset + start * kind.size()) as f64)];
            if view.length.is_some() || args.get(1).is_some_and(|value| !value.is_undefined()) {
                arguments.push(JsValue::num((end - start) as f64));
            }
            let (result, _, _) = binary_typed_species_create(i, &this, kind, arguments, BinaryTypedAccess::Read)?;
            Ok(result)
        }
        "set" => {
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            // ToIntegerOrInfinity(offset) precedes source length/Get. A negative
            // offset rejects immediately; positive infinity rejects after the
            // genuine target/source witness checks, before indexed source Gets.
            let n = i.to_number(&arg(1))?;
            let offset = if n.is_nan() { 0.0 } else { n.trunc() };
            if offset < 0.0 { return Err(Throw::range_error("negative TypedArray offset")); }
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("view changed during set")); }
            let current_length = binary_length(&view);
            let source = arg(0);
            if let Ok((source_view, source_kind)) = binary_typed(&source) {
                if binary_view_out_of_bounds(&source_view) { return Err(Throw::type_error("out-of-bounds source TypedArray")); }
                if source_kind.big() != kind.big() { return Err(Throw::type_error("mixed BigInt and Number TypedArrays")); }
                let source_length = binary_length(&source_view);
                if offset > current_length as f64 || source_length > current_length - offset as usize {
                    return Err(Throw::range_error("source does not fit TypedArray"));
                }
                if source_kind == kind {
                    // Same-element-type copies retain the bytes, including NaN
                    // payloads. Copy before writing to handle overlapping views.
                    let bytes = {
                        let source = source_view.buffer.borrow();
                        binary_copy_bytes(&source.bytes[source_view.offset..source_view.offset + source_length * kind.size()])?
                    };
                    let begin = view.offset + offset as usize * kind.size();
                    view.buffer.borrow_mut().bytes[begin..begin + bytes.len()].copy_from_slice(&bytes);
                } else {
                    let source_values = binary_typed_values(&source_view, source_kind)?;
                    for (index, value) in source_values.into_iter().enumerate() { i.set_prop(&this, (offset as usize + index).to_string(), value)?; }
                }
            } else {
                let source = array_receiver(i, source)?;
                let source_length = array_search_length(i, &source)?;
                if offset > current_length as f64 || source_length > current_length - offset as usize {
                    return Err(Throw::range_error("source does not fit TypedArray"));
                }
                for index in 0..source_length {
                    let value = i.get_prop(&source, index.to_string())?;
                    i.set_prop(&this, (offset as usize + index).to_string(), value)?;
                }
            }
            Ok(JsValue::Undefined)
        }
        "copyWithin" => {
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let target = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length as i64)? } else { 0 };
            let start = if args.get(1).is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.get(1), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(2), length as i64)?;
            let count = end.saturating_sub(start).min(length.saturating_sub(target));
            if count == 0 { return Ok(this); }
            if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("view changed during copyWithin")); }
            let live_length = binary_length(&view);
            let count = count.min(live_length.saturating_sub(target)).min(live_length.saturating_sub(start));
            if count == 0 { return Ok(this); }
            let mut buffer = view.buffer.borrow_mut();
            if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let from = view.offset + start * kind.size();
            let to = view.offset + target * kind.size();
            buffer.bytes.copy_within(from..from + count * kind.size(), to);
            Ok(this)
        }
        "with" => binary_typed_with(i, &this, &view, kind, length, &args),
        "toSorted" => {
            let compare = arg(0);
            if !matches!(compare, JsValue::Undefined) && callable_function(&compare).is_none() {
                return Err(Throw::type_error("comparefn must be a function"));
            }
            let (result, target) = binary_typed_intrinsic_length(i, kind, length)?;
            let mut copy = binary_typed_values(&view, kind)?;
            binary_typed_stable_sort(i, &mut copy, &compare)?;
            for (index, value) in copy.iter().enumerate() { binary_set_element(i, &target, kind, Some(index), value)?; }
            Ok(result)
        }
        "sort" => {
            if view.buffer.borrow().immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let compare = arg(0);
            if !matches!(compare, JsValue::Undefined) && callable_function(&compare).is_none() {
                return Err(Throw::type_error("comparefn must be a function"));
            }
            let mut copy = values;
            binary_typed_stable_sort(i, &mut copy, &compare)?;
            for (j,value) in copy.into_iter().enumerate() { binary_set_element(i, &view, kind, Some(j), &value)?; }
            Ok(this)
        }
        "map" | "filter" | "forEach" | "every" | "some" | "find" | "findIndex" | "findLast" | "findLastIndex" => {
            let callback = arg(0);
            if callable_function(&callback).is_none() {
                return Err(Throw::type_error("callback is not a function"));
            }
            let receiver = arg(1);
            if name == "map" {
                return binary_typed_map(i, &this, &view, kind, length, callback, receiver);
            }
            let indices: Vec<_> = if name.ends_with("Last") || name.ends_with("LastIndex") { (0..length).rev().collect() } else { (0..length).collect() };
            let mut selected = Vec::new();
            for j in indices {
                let value = i.get_prop(&this, j.to_string())?;
                let result = i.call_value(&callback, receiver.clone(), vec![value.clone(), JsValue::num(j as f64), this.clone()])?;
                match name.as_str() {
                    "map" => selected.push(result),
                    "filter" if result.to_boolean() => selected.push(value),
                    "every" if !result.to_boolean() => return Ok(JsValue::Bool(false)),
                    "some" if result.to_boolean() => return Ok(JsValue::Bool(true)),
                    "find" | "findLast" if result.to_boolean() => return Ok(value),
                    "findIndex" | "findLastIndex" if result.to_boolean() => return Ok(JsValue::num(j as f64)),
                    _ => {}
                }
            }
            match name.as_str() {
                "map" | "filter" => binary_typed_species_from_values(i, &this, kind, selected),
                "every" => Ok(JsValue::Bool(true)), "some" => Ok(JsValue::Bool(false)),
                "find" | "findLast" => Ok(JsValue::Undefined),
                "findIndex" | "findLastIndex" => Ok(JsValue::num(-1.0)),
                _ => Ok(JsValue::Undefined),
            }
        }
        "reduce" | "reduceRight" => {
            let callback = arg(0);
            if callable_function(&callback).is_none() {
                return Err(Throw::type_error("callback is not a function"));
            }
            let indices: Vec<_> = if name == "reduceRight" { (0..length).rev().collect() }
                else { (0..length).collect() };
            let mut iter = indices.into_iter();
            let mut accumulator = if let Some(initial) = args.get(1) { initial.clone() }
                else {
                    let first = iter.next().ok_or_else(|| Throw::type_error("reduce of empty TypedArray"))?;
                    i.get_prop(&this, first.to_string())?
                };
            for index in iter {
                let value = i.get_prop(&this, index.to_string())?;
                accumulator = i.call_value(&callback, JsValue::Undefined,
                    vec![accumulator, value, JsValue::num(index as f64), this.clone()])?;
            }
            Ok(accumulator)
        }
        "keys" | "values" | "entries" => {
            let iteration = match name.as_str() {
                "keys" => SequenceIteratorKind::ArrayKeys,
                "entries" => SequenceIteratorKind::ArrayEntries,
                _ => SequenceIteratorKind::ArrayValues,
            };
            Ok(i.sequence_iterator(this, iteration))
        }
        _ => Err(Throw::type_error("unknown TypedArray operation")),
    }
}

fn binary_wait_async_result(i: &mut Interpreter, async_value: bool, value: JsValue) -> Result<JsValue, Throw> {
    let object = i.object_value();
    i.set_prop(&object, "async", JsValue::Bool(async_value))?;
    i.set_prop(&object, "value", value)?;
    Ok(object)
}

// This engine is single-threaded: there is no other real agent that could
// call Atomics.notify while Atomics.wait blocks (blocking is the only thing
// happening). But a *pending* Atomics.waitAsync promise is a same-thread
// bookkeeping detail, not a second agent -- the calling script gets control
// back immediately and can go on to call Atomics.notify itself before the
// microtask queue ever runs. ATOMICS_WAITERS tracks exactly that: pending
// waitAsync promises keyed by the buffer they watch and the byte offset
// within it (matching WaiterList semantics -- two different TypedArray views
// over the same SharedArrayBuffer at the same byte position share a waiter
// list), so Atomics.notify can resolve them to "ok" in FIFO order. Holding a
// clone of the buffer's Rc also means a stale pointer can never alias a new
// allocation while a waiter for it is still registered.
struct BinaryWaiter { buffer: Rc<RefCell<BinaryBuffer>>, byte_offset: usize, promise: Obj }
thread_local! {
    static ATOMICS_WAITERS: RefCell<Vec<BinaryWaiter>> = RefCell::new(Vec::new());
}
fn binary_atomics_register_waiter(view: &BinaryView, byte_offset: usize, promise: Obj) {
    ATOMICS_WAITERS.with(|w| w.borrow_mut().push(BinaryWaiter { buffer: view.buffer.clone(), byte_offset, promise }));
}
fn binary_atomics_wake(i: &mut Interpreter, buffer: &Rc<RefCell<BinaryBuffer>>, byte_offset: usize, max: f64) -> f64 {
    let mut woken = Vec::new();
    ATOMICS_WAITERS.with(|w| {
        let mut waiters = w.borrow_mut();
        let mut cursor = 0;
        while cursor < waiters.len() && (woken.len() as f64) < max {
            if Rc::ptr_eq(&waiters[cursor].buffer, buffer) && waiters[cursor].byte_offset == byte_offset {
                woken.push(waiters.remove(cursor).promise);
            } else {
                cursor += 1;
            }
        }
    });
    let count = woken.len();
    for promise in woken {
        let _ = settle_promise(&promise, PromiseFate::Fulfilled(JsValue::str("ok")), i);
    }
    count as f64
}

// Atomics.wait/waitAsync/notify share ValidateAtomicAccess: the value and
// timeout arguments are coerced in that order (both able to run user code
// and observably ordered by validate-arraytype-before-*-coercion.js /
// symbol-for-timeout-throws.js), and a coercion side effect must never
// change which index/value/timeout was already fixed.
fn binary_atomics_value(i: &mut Interpreter, kind: BinaryKind, value_arg: Option<&JsValue>) -> Result<JsValue, Throw> {
    let value = value_arg.unwrap_or(&JsValue::Undefined);
    if kind.big() { Ok(Interpreter::bigint_value(i.to_bigint(value)?)) }
    else { Ok(JsValue::num(i.to_int32(value)? as f64)) }
}
fn binary_atomics_equal(old: &JsValue, v: &JsValue) -> bool {
    match (old, v) {
        (JsValue::Num(a), JsValue::Num(b)) => a == b,
        (JsValue::BigInt(a), JsValue::BigInt(b)) => a.as_ref() == b.as_ref(),
        _ => false,
    }
}
fn binary_atomics_timeout(i: &mut Interpreter, timeout_arg: Option<&JsValue>) -> Result<f64, Throw> {
    let q = i.to_number(timeout_arg.unwrap_or(&JsValue::Undefined))?;
    Ok(if q.is_nan() { f64::INFINITY } else { q.max(0.0) })
}
fn binary_atomics_sleep(timeout_ms: f64) {
    // Honours the timeout for real (rather than resolving instantly), capped
    // so a huge or infinite requested timeout can never hang a test run --
    // nothing can wake this synchronous wait early regardless of duration.
    if timeout_ms > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(timeout_ms.min(50.0) / 1000.0));
    }
}

fn binary_atomics_wait(i: &mut Interpreter, view: &BinaryView, kind: BinaryKind, index: usize,
    value_arg: Option<&JsValue>, timeout_arg: Option<&JsValue>,
) -> Result<JsValue, Throw> {
    let v = binary_atomics_value(i, kind, value_arg)?;
    let timeout = binary_atomics_timeout(i, timeout_arg)?;
    let old = binary_read(view, kind, index * kind.size(), true)?;
    if !binary_atomics_equal(&old, &v) { return Ok(JsValue::str("not-equal")); }
    binary_atomics_sleep(timeout);
    Ok(JsValue::str("timed-out"))
}

fn binary_wait_async(i: &mut Interpreter, view: &BinaryView, kind: BinaryKind, index: usize,
    value_arg: Option<&JsValue>, timeout_arg: Option<&JsValue>,
) -> Result<JsValue, Throw> {
    let v = binary_atomics_value(i, kind, value_arg)?;
    let timeout = binary_atomics_timeout(i, timeout_arg)?;
    let old = binary_read(view, kind, index * kind.size(), true)?;
    if !binary_atomics_equal(&old, &v) { return binary_wait_async_result(i, false, JsValue::str("not-equal")); }
    if timeout <= 0.0 { return binary_wait_async_result(i, false, JsValue::str("timed-out")); }
    let promise_value = new_promise(i, PromiseFate::Pending);
    let promise_obj = promise_object(&promise_value)?;
    binary_atomics_register_waiter(view, view.offset + index * kind.size(), promise_obj);
    binary_wait_async_result(i, true, promise_value)
}

fn binary_atomics_notify(i: &mut Interpreter, view: &BinaryView, kind: BinaryKind, index: usize,
    count_arg: Option<&JsValue>,
) -> Result<JsValue, Throw> {
    // The count argument is coerced -- and can throw -- regardless of
    // whether the buffer turns out to be shared (non-shared-bufferdata-
    // count-evaluation-throws.js); only the final outcome depends on that.
    let count = match count_arg.filter(|value| !matches!(value, JsValue::Undefined)) {
        None => f64::INFINITY,
        Some(value) => {
            let n = i.to_number(value)?;
            if n.is_nan() { 0.0 } else { n.trunc().max(0.0) }
        }
    };
    if !view.buffer.borrow().shared { return Ok(JsValue::num(0.0)); }
    let byte_offset = view.offset + index * kind.size();
    Ok(JsValue::num(binary_atomics_wake(i, &view.buffer, byte_offset, count)))
}

include!("builtins_binary_atomics.rs");

fn binary_atomics(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    if let Some(operation) = BinaryAtomicOperation::named(&name) {
        return binary_atomic_integer(i, operation, &args);
    }
    if name == "isLockFree" {
        let size = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
        return Ok(JsValue::Bool(matches!(size.trunc() as i64, 1 | 2 | 4 | 8)));
    }
    if name == "pause" {
        return Ok(JsValue::Undefined);
    }
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (view, kind) = binary_typed(&target)?;
    if !kind.integer() || kind == BinaryKind::Uint8Clamped || (matches!(name.as_str(), "wait" | "waitAsync" | "notify")
        && !matches!(kind, BinaryKind::Int32 | BinaryKind::BigInt64)) {
        return Err(Throw::type_error("invalid Atomics TypedArray"));
    }
    // Mutability is checked before any argument coercion for the read-modify-write
    // operations. Atomics.load is a pure read and notify/waitAsync are handled below.
    if matches!(name.as_str(), "add" | "and" | "compareExchange" | "exchange" | "or" | "store" | "sub" | "xor")
        && view.buffer.borrow().immutable
    {
        return Err(Throw::type_error("immutable ArrayBuffer"));
    }
    // A detached buffer fails ValidateTypedArray -- common to every
    // TypedArray-taking operation, including notify, and always checked
    // before any argument coercion (notify/null-bufferdata-throws.js: the
    // poisoned index/count args must never be evaluated). This is a
    // distinct condition from "not shared": notify only returns 0 for a
    // buffer that is merely non-shared, but still throws for one that is
    // outright detached.
    if view.buffer.borrow().detached {
        return Err(Throw::type_error("detached ArrayBuffer"));
    }
    // wait/waitAsync require a SharedArrayBuffer up front, before any
    // index/value/timeout coercion. notify does NOT throw here: it fully
    // validates index and count and only then returns 0 for a non-shared
    // buffer (see binary_atomics_notify) -- non-shared-bufferdata-
    // returns-0.js vs non-shared-bufferdata-throws.js (wait/waitAsync).
    if matches!(name.as_str(), "wait" | "waitAsync") && !view.buffer.borrow().shared {
        return Err(Throw::type_error("Atomics wait/waitAsync requires SharedArrayBuffer"));
    }
    // ValidateAtomicAccess reads TypedArrayLength before `index` is coerced,
    // so a coercion side effect that grows, shrinks, resizes or detaches the
    // buffer must not change the bound this index is checked against
    // (retrieve-length-before-index-coercion.js, wait/notify/waitAsync and
    // general Atomics ops alike).
    let length_before_index = binary_length(&view);
    let index = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    if index >= length_before_index { return Err(Throw::range_error("Atomics index outside TypedArray")); }
    if name == "wait" { return binary_atomics_wait(i, &view, kind, index, args.get(2), args.get(3)); }
    if name == "waitAsync" { return binary_wait_async(i, &view, kind, index, args.get(2), args.get(3)); }
    if name == "notify" { return binary_atomics_notify(i, &view, kind, index, args.get(2)); }
    Err(Throw::type_error("unknown Atomics operation"))
}

#[cfg(test)]
mod binary_tests {
    use super::*;

    #[test]
    fn binding_uint8_writes_observe_brand_and_view_bounds() {
        let mut i = Interpreter::new();
        let buffer = i.binding_create_array_buffer(vec![9; 8]).unwrap();
        let value = i.binary_new_view(binary_obj(&buffer).unwrap(), 2, Some(3), Some(BinaryKind::Uint8)).unwrap();
        assert_eq!(i.binding_uint8_array_info(&value).unwrap(), Some(3));
        i.binding_write_uint8_array(&value, 1, &[1, 2]).unwrap();
        assert_eq!(i.binding_buffer_source_bytes(&buffer).unwrap().unwrap(), [9,9,9,1,2,9,9,9]);
        assert!(i.binding_write_uint8_array(&value, 2, &[1,2]).is_err());
        assert!(i.binding_write_uint8_array(&value, usize::MAX, &[1]).is_err());
        i.binding_write_uint8_array(&value, 3, &[]).unwrap();
        for kind in [None, Some(BinaryKind::Int8), Some(BinaryKind::Uint8Clamped), Some(BinaryKind::Uint16)] {
            let other = i.binary_new_view(binary_obj(&buffer).unwrap(), 0, Some(8), kind).unwrap();
            assert_eq!(i.binding_uint8_array_info(&other).unwrap(), None);
            assert!(i.binding_write_uint8_array(&other, 0, &[]).is_err());
        }
        assert_eq!(i.binding_uint8_array_info(&buffer).unwrap(), None);
        assert_eq!(i.binding_uint8_array_info(&JsValue::Undefined).unwrap(), None);
        // Altering the prototype cannot change the internal view brand.
        binary_obj(&value).unwrap().borrow_mut().proto = None;
        assert_eq!(i.binding_uint8_array_info(&value).unwrap(), Some(3));
        i.binding_detach_array_buffer(&buffer).unwrap();
        assert!(i.binding_uint8_array_info(&value).is_err());
        assert!(i.binding_write_uint8_array(&value, 0, &[]).is_err());
        let mut other_realm = Interpreter::new();
        let foreign = other_realm.binding_create_uint8_array(vec![8; 2]).unwrap();
        assert_eq!(i.binding_uint8_array_info(&foreign).unwrap(), Some(2));
        i.binding_write_uint8_array(&foreign, 0, &[4]).unwrap();
        assert_eq!(other_realm.binding_buffer_source_bytes(&foreign).unwrap().unwrap(), [4,8]);
        let shared = i.binary_new_buffer(vec![7; 4], 4, false, true);
        let shared_view = i.binary_new_view(binary_obj(&shared).unwrap(), 1, Some(2), Some(BinaryKind::Uint8)).unwrap();
        i.binding_write_uint8_array(&shared_view, 0, &[5,6]).unwrap();
        assert_eq!(binary_buffer(&shared).unwrap().borrow().bytes, [7,5,6,7]);
        let resizable = i.binary_new_buffer(vec![0; 4], 8, true, false);
        let fixed = i.binary_new_view(binary_obj(&resizable).unwrap(), 2, Some(2), Some(BinaryKind::Uint8)).unwrap();
        binary_buffer(&resizable).unwrap().borrow_mut().bytes.truncate(3);
        assert!(i.binding_uint8_array_info(&fixed).is_err());
        assert!(i.binding_write_uint8_array(&fixed, 0, &[]).is_err());
    }

    #[test]
    fn binding_array_buffer_contains_exact_bytes() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.binding_create_array_buffer(vec![0, 127, 255]).unwrap();
        let buffer = binary_buffer(&value).unwrap();
        assert_eq!(buffer.borrow().bytes, [0, 127, 255]);
        assert!(!buffer.borrow().shared);
        assert!(!buffer.borrow().resizable);
        let len = interpreter.get_prop(&value, "byteLength").unwrap();
        assert_eq!(len, JsValue::num(3.0));
    }

    #[test]
    fn binding_uint8_array_views_full_fresh_buffer() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.binding_create_uint8_array(vec![2, 17, 255]).unwrap();
        assert_eq!(interpreter.get_prop(&value, "length").unwrap(), JsValue::num(3.0));
        assert_eq!(interpreter.get_prop(&value, "0").unwrap(), JsValue::num(2.0));
        assert_eq!(interpreter.get_prop(&value, "2").unwrap(), JsValue::num(255.0));
        interpreter.set_prop(&value, "1", JsValue::num(42.0)).unwrap();
        let view = binary_typed(&value).unwrap().0;
        assert_eq!(view.buffer.borrow().bytes, [2, 42, 255]);
    }

    #[test]
    fn binding_buffer_source_snapshots_buffers_and_view_windows() {
        let mut interpreter = Interpreter::new();
        let buffer = interpreter.binding_create_array_buffer(vec![1, 2, 3, 4, 5, 6]).unwrap();
        let object = binary_obj(&buffer).unwrap();
        let data_view = interpreter.binary_new_view(object.clone(), 1, Some(3), None).unwrap();
        let typed_view = interpreter.binary_new_view(object, 2, Some(4), Some(BinaryKind::Uint16)).unwrap();
        let mut snapshot = interpreter.binding_buffer_source_bytes(&buffer).unwrap().unwrap();
        assert_eq!(snapshot, [1, 2, 3, 4, 5, 6]);
        snapshot[0] = 99;
        assert_eq!(interpreter.binding_buffer_source_bytes(&buffer).unwrap(), Some(vec![1, 2, 3, 4, 5, 6]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&data_view).unwrap(), Some(vec![2, 3, 4]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&typed_view).unwrap(), Some(vec![3, 4, 5, 6]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&JsValue::num(7.0)).unwrap(), None);
        assert_eq!(interpreter.binding_buffer_source_bytes(&interpreter.object_value()).unwrap(), None);
    }

    #[test]
    fn binding_buffer_source_tracks_resize_and_out_of_bounds_views() {
        let mut interpreter = Interpreter::new();
        let buffer = interpreter.binary_new_buffer(vec![1, 2, 3, 4, 5, 6], 10, true, false);
        let object = binary_obj(&buffer).unwrap();
        let tracking = interpreter.binary_new_view(object.clone(), 2, None, Some(BinaryKind::Uint16)).unwrap();
        let data_view = interpreter.binary_new_view(object.clone(), 2, None, None).unwrap();
        let fixed = interpreter.binary_new_view(object, 2, Some(4), Some(BinaryKind::Uint8)).unwrap();
        assert_eq!(interpreter.binding_buffer_source_bytes(&tracking).unwrap(), Some(vec![3, 4, 5, 6]));
        binary_buffer(&buffer).unwrap().borrow_mut().bytes.truncate(5);
        assert_eq!(interpreter.binding_buffer_source_bytes(&tracking).unwrap(), Some(vec![3, 4]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&data_view).unwrap(), Some(vec![3, 4, 5]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&fixed).unwrap(), Some(vec![]));
        binary_buffer(&buffer).unwrap().borrow_mut().bytes.truncate(1);
        assert_eq!(interpreter.binding_buffer_source_bytes(&tracking).unwrap(), Some(vec![]));
        assert_eq!(interpreter.binding_buffer_source_bytes(&data_view).unwrap(), Some(vec![]));
    }

    #[test]
    fn binding_buffer_source_rejects_detached_backing_stores() {
        let mut interpreter = Interpreter::new();
        let buffer = interpreter.binding_create_array_buffer(vec![8, 9]).unwrap();
        let view = interpreter.binary_new_view(binary_obj(&buffer).unwrap(), 0, None, None).unwrap();
        interpreter.binding_detach_array_buffer(&buffer).unwrap();
        for value in [&buffer, &view] {
            let error = interpreter.binding_buffer_source_bytes(value).unwrap_err();
            let JsValue::Object(object) = error.0 else { panic!("expected TypeError object") };
            assert_eq!(object.borrow().get_own("name"), Some(JsValue::str("TypeError")));
        }
        let shared = interpreter.binary_new_buffer(vec![1, 2], 2, false, true);
        assert_eq!(interpreter.binding_buffer_source_bytes(&shared).unwrap(), None);
    }

    #[test]
    fn resizable_view_tracks_length_and_detachment() {
        let mut interpreter = Interpreter::new();
        let ctor = interpreter.global_env.resolve("ArrayBuffer").unwrap();
        let options = interpreter.object_value();
        interpreter.set_prop(&options, "maxByteLength", JsValue::num(8.0)).unwrap();
        let buffer = interpreter.construct(&ctor, vec![JsValue::num(2.0), options]).unwrap();
        let typed = interpreter.global_env.resolve("Uint8Array").unwrap();
        let view = interpreter.construct(&typed, vec![buffer.clone()]).unwrap();
        assert_eq!(interpreter.get_prop(&view, "length").unwrap(), JsValue::num(2.0));
        let resize = interpreter.get_prop(&buffer, "resize").unwrap();
        interpreter.call_value(&resize, buffer.clone(), vec![JsValue::num(5.0)]).unwrap();
        assert_eq!(interpreter.get_prop(&view, "length").unwrap(), JsValue::num(5.0));
        let transfer = interpreter.get_prop(&buffer, "transfer").unwrap();
        interpreter.call_value(&transfer, buffer.clone(), vec![]).unwrap();
        assert_eq!(interpreter.get_prop(&view, "length").unwrap(), JsValue::num(0.0));
    }

    #[test]
    fn internal_binary_errors_have_realm_error_prototype() {
        let mut interpreter = Interpreter::new();
        let result = interpreter.run_script("try { new ArrayBuffer(-1); } catch (error) { error.constructor === RangeError && error instanceof RangeError; }").unwrap();
        assert_eq!(result, JsValue::Bool(true));
    }

    #[test]
    fn typed_array_sources_and_copy_within() {
        for source in [
            "[1,2,3]",
            "{0:1,1:2,2:3,length:3}",
            "{[Symbol.iterator]:function(){return [1,2,3][Symbol.iterator]()}}",
        ] {
            let mut interpreter = Interpreter::new();
            let script = format!("let a = new Uint8Array({source}); a.copyWithin(1,0); a[0]===1 && a[1]===1 && a[2]===2");
            assert_eq!(interpreter.run_script(&script).unwrap(), JsValue::Bool(true), "{source}");
        }
    }
    #[test]
    fn splice_preserves_getter_only_length_error() {
        let mut interpreter = Interpreter::new();
        let script = "let a = { get length() { return 0; }, splice: Array.prototype.splice }; \
            let descriptor = Object.getOwnPropertyDescriptor(a, 'length'); \
            let threw = false; try { a.splice(1, 2, 4); } catch (error) { threw = error instanceof TypeError; } \
            descriptor.set === undefined && descriptor.get !== undefined && threw";
        assert_eq!(interpreter.run_script(script).unwrap(), JsValue::Bool(true));
        assert_eq!(interpreter.run_script("let data = { get foo() { return 0; }, foo: 1 }; data.foo === 1").unwrap(),
            JsValue::Bool(true));
    }
    #[test]
    fn float16_rounding_carries_into_exponent() {
        assert_eq!(binary_decode_f16(binary_encode_f16(32767.0)), 32768.0,
            "encoded={:04x}, f32={:08x}", binary_encode_f16(32767.0), (32767.0f64 as f32).to_bits());
        assert_eq!(binary_decode_f16(binary_encode_f16(-0.0)).to_bits(), (-0.0f64).to_bits());
        assert_eq!(binary_decode_f16(binary_encode_f16(2.980232238769532e-8)),
            5.960464477539063e-8);
    }
}

include!("runtime_binary_integrity.rs");
include!("runtime_binary_integrity_tests.rs");
