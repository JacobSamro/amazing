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
    let buffer = view.buffer.borrow();
    if buffer.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    let end = offset.checked_add(kind.size()).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
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
    let end = offset.checked_add(kind.size()).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
    let mut buffer = view.buffer.borrow_mut();
    if buffer.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
    let start = view.offset + offset;
    buffer.bytes[start..start + kind.size()].copy_from_slice(&binary_encode_number(value, kind, little));
    Ok(())
}
fn binary_write_value(i: &mut Interpreter, view: &BinaryView, kind: BinaryKind, offset: usize, little: bool, value: &JsValue) -> Result<(), Throw> {
    if !kind.big() { return binary_write(view, kind, offset, little, i.to_number(value)?); }
    let bigint = i.to_bigint(value)?;
    let end = offset.checked_add(8).ok_or_else(|| Throw::range_error("view offset"))?;
    if end > binary_view_bytes(view) { return Err(Throw::range_error("view offset")); }
    let mut buffer = view.buffer.borrow_mut();
    if buffer.detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
    let mut bytes = bigint.low_u64().to_le_bytes();
    if !little { bytes.reverse(); }
    let start = view.offset + offset;
    buffer.bytes[start..start + 8].copy_from_slice(&bytes);
    Ok(())
}

fn binary_property(value: JsValue) -> Property {
    Property { value, writable: false, enumerable: false, configurable: true, getter: None, setter: None }
}
fn binary_writable_property(value: JsValue) -> Property {
    Property { value, writable: true, enumerable: false, configurable: true, getter: None, setter: None }
}
fn binary_locked_property(value: JsValue) -> Property {
    Property { value, writable: false, enumerable: false, configurable: false, getter: None, setter: None }
}
fn binary_configure_constructor(object: &Obj, name: &str, length: usize, prototype: &Obj) {
    let mut object = object.borrow_mut();
    let _ = object.define("name", binary_property(JsValue::str(name)));
    let _ = object.define("length", binary_property(JsValue::num(length as f64)));
    let _ = object.define("prototype", binary_locked_property(JsValue::Object(prototype.clone())));
}
fn binary_accessor(function: JsFunction, setter: Option<JsFunction>, enumerable: bool) -> Property {
    Property { value: JsValue::Undefined, writable: false, enumerable, configurable: true,
        getter: Some(Box::new(function)), setter: setter.map(Box::new) }
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
    let JsValue::Object(callable) = method else { unreachable!() };
    let function = callable.borrow().func.clone().unwrap();
    let _ = object.borrow_mut().define(name, binary_accessor(function, None, false));
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
                if let Some(JsValue::Object(constructor)) = self.global_env.resolve(&name) {
                    if let Some(JsValue::Object(prototype)) = constructor.borrow().get_own("prototype") {
                        object.borrow_mut().proto = Some(prototype);
                    }
                }
            }
        }
        thrown
    }
    fn binary_new_buffer(&mut self, bytes: Vec<u8>, max: usize, resizable: bool, shared: bool) -> JsValue {
        let name = if shared { "SharedArrayBuffer" } else { "ArrayBuffer" };
        let prototype = self.global_env.resolve(name)
            .and_then(|ctor| self.get_prop(&ctor, "prototype").ok())
            .and_then(|proto| if let JsValue::Object(o) = proto { Some(o) } else { None })
            .unwrap_or_else(|| self.object_proto.clone());
        let mut raw = JsObject::ordinary(); raw.proto = Some(prototype);
        let object = Obj(Rc::new(RefCell::new(raw)));
        binary_register(&object, BinaryState::Buffer(Rc::new(RefCell::new(BinaryBuffer {
            bytes, max, resizable, shared, detached: false, immutable: false,
        }))));
        JsValue::Object(object)
    }
    fn binary_new_view(&mut self, buffer_object: Obj, offset: usize, length: Option<usize>, kind: Option<BinaryKind>) -> Result<JsValue, Throw> {
        let buffer = binary_buffer(&JsValue::Object(buffer_object.clone()))?;
        let size = kind.map_or(1, BinaryKind::size);
        let bytes_len = buffer.borrow().bytes.len();
        if buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
        if offset > bytes_len || offset % size != 0
            || length.is_some_and(|len| len > bytes_len - offset) {
            return Err(Throw::range_error("invalid view bounds"));
        }
        if kind.is_some() && length.is_none() && (bytes_len - offset) % size != 0 && !buffer.borrow().resizable {
            return Err(Throw::range_error("unaligned TypedArray length"));
        }
        let name = kind.map_or("DataView", BinaryKind::name);
        let proto = self.global_env.resolve(name)
            .and_then(|ctor| self.get_prop(&ctor, "prototype").ok())
            .and_then(|value| if let JsValue::Object(o) = value { Some(o) } else { None })
            .unwrap_or_else(|| self.object_proto.clone());
        let mut raw = JsObject::ordinary(); raw.proto = Some(proto);
        if kind.is_some() { raw.exotic = Exotic::TypedArray; }
        let object = Obj(Rc::new(RefCell::new(raw)));
        binary_register(&object, BinaryState::View(BinaryView { buffer, buffer_object, offset, length, kind }));
        Ok(JsValue::Object(object))
    }
    pub fn binding_create_array_buffer(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        let len = bytes.len(); Ok(self.binary_new_buffer(bytes, len, false, false))
    }
    pub fn binding_create_uint8_array(&mut self, bytes: Vec<u8>) -> Result<JsValue, Throw> {
        let buffer = self.binding_create_array_buffer(bytes)?;
        self.binary_new_view(binary_obj(&buffer)?, 0, None, Some(BinaryKind::Uint8))
    }
    pub fn binding_detach_array_buffer(&mut self, value: &JsValue) -> Result<(), Throw> {
        let buffer = binary_buffer(value)?;
        let mut buffer = buffer.borrow_mut();
        if buffer.shared { return Err(Throw::type_error("SharedArrayBuffer cannot be detached")); }
        if buffer.detached { return Ok(()); }
        buffer.bytes.clear();
        buffer.detached = true;
        Ok(())
    }

    fn install_binary_builtins(&mut self) {
        let ab = Self::native("ArrayBuffer", binary_array_buffer_ctor, &self.function_proto);
        let sab = Self::native("SharedArrayBuffer", binary_shared_buffer_ctor, &self.function_proto);
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

        let data = Self::native("DataView", binary_data_view_ctor, &self.function_proto);
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

        let intrinsic = Self::native("TypedArray", binary_typed_ctor, &self.function_proto);
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
            ("toString", 0), ("values", 0), ("with", 2),
        ] { binary_method(self, &typed_proto, name, binary_typed_method, arity); }
        let values_method = typed_proto.borrow().get_own("values").unwrap();
        let _ = typed_proto.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), binary_writable_property(values_method));
        let _ = typed_proto.borrow_mut().define("constructor", binary_writable_property(intrinsic.clone()));
        binary_configure_constructor(intrinsic_obj, "TypedArray", 0, &typed_proto);
        let tag_getter = Self::native("get [Symbol.toStringTag]", binary_typed_tag, &self.function_proto);
        if let JsValue::Object(getter) = tag_getter {
            let _ = typed_proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                binary_accessor(getter.borrow().func.clone().unwrap(), None, false));
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
            let ctor = Self::native(name, binary_typed_ctor, &self.function_proto);
            let JsValue::Object(function) = &ctor else { unreachable!() };
            function.borrow_mut().proto = Some(intrinsic_obj.clone());
            let JsValue::Object(proto) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
            proto.borrow_mut().proto = Some(typed_proto.clone());
            let _ = proto.borrow_mut().define("constructor", binary_writable_property(ctor.clone()));
            binary_configure_constructor(function, name, 3, &proto);
            binary_species(self, function);
            let _ = function.borrow_mut().define("BYTES_PER_ELEMENT", binary_property(JsValue::num(kind.size() as f64)));
            let _ = proto.borrow_mut().define("BYTES_PER_ELEMENT", binary_property(JsValue::num(kind.size() as f64)));
            self.define_global(name, ctor);
        }
        let atomics = self.object_value();
        if let JsValue::Object(object) = &atomics {
            for (name, arity) in [
                ("add", 3), ("and", 3), ("compareExchange", 4), ("exchange", 3),
                ("isLockFree", 1), ("load", 2), ("notify", 3), ("or", 3),
                ("store", 3), ("sub", 3), ("wait", 4), ("waitAsync", 4), ("xor", 3),
            ] { binary_method(self, object, name, binary_atomics, arity); }
        }
        self.define_global("Atomics", atomics);
    }
}

fn binary_species(i: &Interpreter, constructor: &Obj) {
    let getter = Interpreter::native("get [Symbol.species]", binary_species_getter, &i.function_proto);
    if let JsValue::Object(getter) = getter {
        let _ = constructor.borrow_mut().define(WellKnownSymbol::Species.symbol(),
            binary_accessor(getter.borrow().func.clone().unwrap(), None, false));
    }
}
fn binary_species_getter(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> { Ok(this) }
fn binary_typed_tag(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(binary_typed(&this).map(|(_, kind)| JsValue::str(kind.name())).unwrap_or(JsValue::Undefined))
}

// CanonicalNumericIndexString includes negative zero, fractional and infinity
// spellings. Those are reserved keys on integer-indexed exotic objects, even
// though no valid integer index can name an element there.
fn binary_canonical_key(key: &PropertyKey) -> Option<Option<usize>> {
    let PropertyKey::String(text) = key else { return None };
    let text = text.to_utf8_lossy();
    if text == "-0" { return Some(None); }
    let number = crate::value::string_to_number(&text);
    if text == number_to_string(number) || (number.is_nan() && text == "NaN") {
        Some(if number.is_finite() && number >= 0.0 && number.fract() == 0.0
            && number < usize::MAX as f64 { Some(number as usize) } else { None })
    } else { None }
}
fn binary_index_view(object: &Obj, key: &PropertyKey) -> Option<(BinaryView, BinaryKind, Option<usize>)> {
    let Some(BinaryState::View(view)) = binary_state(object) else { return None };
    let kind = view.kind?;
    let index = binary_canonical_key(key)?;
    Some((view, kind, index))
}
fn binary_get_index(_i: &mut Interpreter, object: &Obj, key: &PropertyKey) -> Option<Result<JsValue, Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some(Ok(match index.filter(|index| *index < binary_length(&view)) {
        Some(index) => binary_read(&view, kind, index * kind.size(), true).unwrap_or(JsValue::Undefined),
        None => JsValue::Undefined,
    }))
}
fn binary_set_index(i: &mut Interpreter, object: &Obj, key: &PropertyKey, value: JsValue) -> Option<Result<(), Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some((|| {
        let converted = if kind.big() { Interpreter::bigint_value(i.to_bigint(&value)?) }
            else { JsValue::num(i.to_number(&value)?) };
        if let Some(index) = index.filter(|index| *index < binary_length(&view)) {
            binary_write_value(i, &view, kind, index * kind.size(), true, &converted)?;
        }
        Ok(())
    })())
}
fn binary_index_descriptor(object: &Obj, key: &PropertyKey) -> Option<Option<Property>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some(index.filter(|index| *index < binary_length(&view)).map(|index| Property {
        value: binary_read(&view, kind, index * kind.size(), true).unwrap_or(JsValue::Undefined),
        writable: true, enumerable: true, configurable: true, getter: None, setter: None,
    }))
}
fn binary_delete_index(object: &Obj, key: &PropertyKey) -> Option<bool> {
    binary_index_descriptor(object, key).map(|property| property.is_none())
}
fn binary_define_index(i: &mut Interpreter, object: &Obj, key: &PropertyKey, property: &Property) -> Option<Result<(), Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some((|| {
        let Some(index) = index.filter(|index| *index < binary_length(&view)) else {
            return Err(Throw::type_error("invalid TypedArray index"));
        };
        if property.getter.is_some() || property.setter.is_some() || !property.configurable
            || !property.enumerable || !property.writable {
            return Err(Throw::type_error("incompatible TypedArray element descriptor"));
        }
        binary_write_value(i, &view, kind, index * kind.size(), true, &property.value)
    })())
}
fn binary_define_index_object(i: &mut Interpreter, object: &Obj, key: &PropertyKey, descriptor: &Obj) -> Option<Result<bool, Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some((|| {
        let fields = i.std_descriptor_fields(&JsValue::Object(descriptor.clone()))?;
        let Some(index) = index.filter(|index| *index < binary_length(&view)) else {
            return Ok(false);
        };
        if fields.configurable == Some(false) || fields.enumerable == Some(false)
            || fields.get.is_some() || fields.set.is_some() || fields.writable == Some(false) {
            return Ok(false);
        }
        if let Some(value) = fields.value {
            binary_write_value(i, &view, kind, index * kind.size(), true, &value)?;
        }
        Ok(true)
    })())
}
fn binary_own_keys(object: &Obj) -> Option<Vec<PropertyKey>> {
    let Some(BinaryState::View(view)) = binary_state(object) else { return None };
    view.kind?;
    let mut keys: Vec<_> = (0..binary_length(&view).min(1_000_000))
        .map(|index| PropertyKey::from(index.to_string())).collect();
    keys.extend(object.borrow().own_property_keys().into_iter().filter(|key| binary_canonical_key(key).is_none()));
    Some(keys)
}

fn binary_current_name(i: &Interpreter) -> String {
    let name = i.native_stack.last().and_then(|o| o.borrow().func.as_ref().map(|f| f.name.clone()))
        .unwrap_or_default();
    name.strip_prefix("get ").unwrap_or(&name).to_string()
}
fn binary_require_new(this: &JsValue) -> Result<(), Throw> {
    if matches!(this, JsValue::Object(_)) { Ok(()) }
    else { Err(Throw::type_error("binary data constructor requires new")) }
}
fn binary_array_buffer_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(&this)?;
    let length = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    if length > 256 * 1024 * 1024 { return Err(Throw::range_error("ArrayBuffer too large")); }
    let max_arg = args.get(1).map(|options| i.get_prop(options, "maxByteLength")).transpose()?;
    let resizable = max_arg.as_ref().is_some_and(|x| !matches!(x, JsValue::Undefined));
    let max = if let Some(value) = max_arg.as_ref().filter(|x| !matches!(x, JsValue::Undefined)) {
        binary_index(i, value)?
    } else { length };
    if max < length || max > 256 * 1024 * 1024 { return Err(Throw::range_error("invalid maxByteLength")); }
    Ok(i.binary_new_buffer(vec![0; length], max, resizable, false))
}
fn binary_shared_buffer_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(&this)?;
    let length = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    if length > 256 * 1024 * 1024 { return Err(Throw::range_error("SharedArrayBuffer too large")); }
    let max_arg = args.get(1).map(|options| i.get_prop(options, "maxByteLength")).transpose()?;
    let growable = max_arg.as_ref().is_some_and(|x| !matches!(x, JsValue::Undefined));
    let max = if let Some(value) = max_arg.as_ref().filter(|x| !matches!(x, JsValue::Undefined)) {
        binary_index(i, value)?
    } else { length };
    if max < length || max > 256 * 1024 * 1024 { return Err(Throw::range_error("invalid maxByteLength")); }
    Ok(i.binary_new_buffer(vec![0; length], max, growable, true))
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
    if buffer.borrow().detached { return Err(Throw::type_error("detached ArrayBuffer")); }
    match name.as_str() {
        "resize" | "grow" => {
            if !buffer.borrow().resizable || buffer.borrow().immutable {
                return Err(Throw::type_error("buffer is not resizable"));
            }
            let length = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
            let mut b = buffer.borrow_mut();
            if length > b.max || (b.shared && length < b.bytes.len()) { return Err(Throw::range_error("invalid buffer size")); }
            b.bytes.resize(length, 0);
            Ok(JsValue::Undefined)
        }
        "slice" | "sliceToImmutable" => {
            let (length, shared) = { let b = buffer.borrow(); (b.bytes.len() as i64, b.shared) };
            let begin = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length)? } else { 0 };
            let end = binary_relative(i, args.get(1), length)?;
            let amount = end.max(begin) - begin;
            let copied = if name == "slice" { binary_species_buffer(i, &this, amount, shared)? }
                else { i.binary_new_buffer(vec![0; amount], amount, false, shared) };
            let source = {
                let b = buffer.borrow();
                if b.detached { return Err(Throw::type_error("source detached during slice")); }
                if b.bytes.len() < end { return Err(Throw::range_error("source shrank during slice")); }
                b.bytes[begin..begin+amount].to_vec()
            };
            {
                let target = binary_buffer(&copied)?;
                let mut target = target.borrow_mut();
                if target.detached || target.immutable || target.bytes.len() < amount || target.shared != shared {
                    return Err(Throw::type_error("invalid buffer species result"));
                }
                target.bytes[..amount].copy_from_slice(&source);
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
            if length > b.max && name == "transfer" && b.resizable { return Err(Throw::range_error("transfer exceeds maxByteLength")); }
            if length > 256 * 1024 * 1024 { return Err(Throw::range_error("transfer too large")); }
            let mut bytes = b.bytes.clone(); bytes.resize(length, 0);
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
    let default = i.global_env.resolve(if shared { "SharedArrayBuffer" } else { "ArrayBuffer" })
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
fn binary_data_view_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(&this)?;
    let buffer_object = binary_obj(args.first().unwrap_or(&JsValue::Undefined))?;
    let buffer = binary_buffer(&JsValue::Object(buffer_object.clone()))?;
    let offset = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    let length = if let Some(arg) = args.get(2).filter(|x| !matches!(x, JsValue::Undefined)) {
        Some(binary_index(i, arg)?)
    } else if buffer.borrow().resizable { None }
    else { Some(buffer.borrow().bytes.len().saturating_sub(offset)) };
    i.binary_new_view(buffer_object, offset, length, None)
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
    let offset = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let little = args.get(if set { 2 } else { 1 }).is_some_and(JsValue::to_boolean);
    if set {
        binary_write_value(i, &view, kind, offset, little, args.get(1).unwrap_or(&JsValue::Undefined))?;
        Ok(JsValue::Undefined)
    } else { binary_read(&view, kind, offset, little) }
}

fn binary_typed_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    binary_require_new(&this)?;
    let name = binary_current_name(i);
    let kind = BinaryKind::from_name(&name).ok_or_else(|| Throw::type_error("TypedArray requires a concrete constructor"))?;
    let first = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let JsValue::Object(object) = &first {
        match binary_state(object) {
            Some(BinaryState::Buffer(buffer)) => {
                let offset = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
                let length = if let Some(arg) = args.get(2).filter(|x| !matches!(x, JsValue::Undefined)) {
                    Some(binary_index(i, arg)?.checked_mul(kind.size()).ok_or_else(|| Throw::range_error("view too large"))?)
                } else if buffer.borrow().resizable { None }
                else { Some(buffer.borrow().bytes.len().saturating_sub(offset)) };
                return i.binary_new_view(object.clone(), offset, length, Some(kind));
            }
            Some(BinaryState::View(view)) if view.kind.is_some() => {
                let old_kind = view.kind.unwrap();
                if old_kind.big() != kind.big() { return Err(Throw::type_error("mixed BigInt and Number TypedArrays")); }
                let values = (0..binary_length(&view)).map(|index|
                    binary_read(&view, old_kind, index * old_kind.size(), true)
                ).collect::<Result<Vec<_>, _>>()?;
                return binary_typed_from_values(i, kind, values);
            }
            _ => {}
        }
    }
    if matches!(first, JsValue::Undefined | JsValue::Num(_) | JsValue::BigInt(_) | JsValue::Symbol(_) | JsValue::Bool(_) | JsValue::Null) {
        let length = binary_index(i, &first)?;
        let byte_length = length.checked_mul(kind.size()).ok_or_else(|| Throw::range_error("TypedArray too large"))?;
        if byte_length > 256 * 1024 * 1024 { return Err(Throw::range_error("TypedArray too large")); }
        let buffer = i.binary_new_buffer(vec![0; byte_length], byte_length, false, false);
        return i.binary_new_view(binary_obj(&buffer)?, 0, Some(byte_length), Some(kind));
    }
    let values = binary_collect_source(i, &first)?;
    binary_typed_from_values(i, kind, values)
}
fn binary_collect_source(i: &mut Interpreter, source: &JsValue) -> Result<Vec<JsValue>, Throw> {
    let iterator = i.get_prop(source, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&iterator).is_some() { return i.iterate(source); }
    if !iterator.is_undefined() && !iterator.is_null() {
        return Err(Throw::type_error("source @@iterator is not callable"));
    }
    if matches!(source, JsValue::Null | JsValue::Undefined) { return Err(Throw::type_error("source is nullish")); }
    let length = i.get_prop(source, "length")?;
    let length = i.to_number(&length)?;
    let length = if length.is_nan() || length < 0.0 { 0 }
        else if !length.is_finite() || length > 1_000_000.0 { return Err(Throw::range_error("array-like source too large")); }
        else { length.trunc() as usize };
    (0..length).map(|index| i.get_prop(source, index.to_string())).collect()
}
fn binary_typed_from_values(i: &mut Interpreter, kind: BinaryKind, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let bytes = values.len().checked_mul(kind.size()).ok_or_else(|| Throw::range_error("TypedArray too large"))?;
    if bytes > 256 * 1024 * 1024 { return Err(Throw::range_error("TypedArray too large")); }
    let buffer = i.binary_new_buffer(vec![0; bytes], bytes, false, false);
    let view = i.binary_new_view(binary_obj(&buffer)?, 0, Some(bytes), Some(kind))?;
    let state = binary_typed(&view)?.0;
    for (index, value) in values.into_iter().enumerate() {
        binary_write_value(i, &state, kind, index * kind.size(), true, &value)?;
    }
    Ok(view)
}
fn binary_typed_static(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let method = binary_current_name(i);
    binary_obj(&this)?;
    let values = if method == "of" { args } else {
        let source = args.first().cloned().unwrap_or(JsValue::Undefined);
        let mut values = binary_collect_source(i, &source)?;
        if let Some(callback) = args.get(1).filter(|v| !matches!(v, JsValue::Undefined)) {
            for (index, value) in values.iter_mut().enumerate() {
                *value = i.call_value(callback, args.get(2).cloned().unwrap_or(JsValue::Undefined),
                    vec![value.clone(), JsValue::num(index as f64)])?;
            }
        }
        values
    };
    let result = i.construct(&this, vec![JsValue::num(values.len() as f64)])?;
    let (view, kind) = binary_typed(&result)?;
    if binary_length(&view) < values.len() { return Err(Throw::type_error("TypedArray constructor returned a short array")); }
    for (index, value) in values.iter().enumerate() {
        binary_write_value(i, &view, kind, index * kind.size(), true, value)?;
    }
    Ok(result)
}

fn binary_typed_values(view: &BinaryView, kind: BinaryKind) -> Result<Vec<JsValue>, Throw> {
    (0..binary_length(view)).map(|j| binary_read(view, kind, j * kind.size(), true)).collect()
}
fn binary_typed_species(i: &mut Interpreter, source: &JsValue, kind: BinaryKind) -> Result<JsValue, Throw> {
    let default = i.global_env.resolve(kind.name()).ok_or_else(|| Throw::type_error("missing TypedArray constructor"))?;
    let constructor = i.get_prop(source, "constructor")?;
    if matches!(constructor, JsValue::Undefined) { return Ok(default); }
    if !constructor.is_object() { return Err(Throw::type_error("TypedArray constructor is not an object")); }
    let species = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
    Ok(if matches!(species, JsValue::Null | JsValue::Undefined) { default } else { species })
}
fn binary_typed_species_from_values(i: &mut Interpreter, source: &JsValue, kind: BinaryKind, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let constructor = binary_typed_species(i, source, kind)?;
    let result = i.construct(&constructor, vec![JsValue::num(values.len() as f64)])?;
    let (view, result_kind) = binary_typed(&result)?;
    if result_kind.big() != kind.big() || binary_length(&view) < values.len() {
        return Err(Throw::type_error("incompatible TypedArray species result"));
    }
    for (index, value) in values.iter().enumerate() {
        binary_write_value(i, &view, result_kind, index * result_kind.size(), true, value)?;
    }
    Ok(result)
}
fn binary_typed_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    let (view, kind) = binary_typed(&this)?;
    if binary_view_out_of_bounds(&view) { return Err(Throw::type_error("out-of-bounds TypedArray")); }
    let length = binary_length(&view);
    let values = binary_typed_values(&view, kind)?;
    let arg = |n: usize| args.get(n).cloned().unwrap_or(JsValue::Undefined);
    match name.as_str() {
        "at" => {
            let number = i.to_number(&arg(0))?;
            let index = if number < 0.0 { length as i64 + number as i64 } else { number as i64 };
            if index >= 0 && (index as usize) < length && (index as usize) < binary_length(&view) {
                binary_read(&view, kind, index as usize * kind.size(), true)
            } else { Ok(JsValue::Undefined) }
        }
        "join" | "toString" | "toLocaleString" => {
            let separator = if name == "join" && !matches!(arg(0), JsValue::Undefined) { i.to_string(&arg(0))? } else { ",".into() };
            let mut strings = Vec::with_capacity(length);
            for index in 0..length {
                let value = i.get_prop(&this, index.to_string())?;
                strings.push(if matches!(value, JsValue::Undefined | JsValue::Null) { String::new() } else { i.to_string(&value)? });
            }
            Ok(JsValue::str(strings.join(&separator)))
        }
        "includes" | "indexOf" | "lastIndexOf" => {
            let needle = arg(0);
            let start = if args.get(1).is_some_and(|x| !matches!(x, JsValue::Undefined)) {
                binary_relative(i, args.get(1), length as i64)?
            } else if name == "lastIndexOf" { length.saturating_sub(1) } else { 0 };
            let found = if name == "lastIndexOf" { values.iter().enumerate().rev().find(|(j, v)| *j <= start && i.strict_eq(v, &needle)).map(|(j, _)| j) }
                else { values.iter().enumerate().find(|(j, v)| *j >= start && (i.strict_eq(v, &needle) || name == "includes" && matches!((v, &needle), (JsValue::Num(a), JsValue::Num(b)) if a.is_nan() && b.is_nan()))).map(|(j, _)| j) };
            Ok(if name == "includes" { JsValue::Bool(found.is_some()) } else { JsValue::num(found.map_or(-1, |x| x as i64) as f64) })
        }
        "fill" => {
            let fill = if kind.big() { Interpreter::bigint_value(i.to_bigint(&arg(0))?) }
                else { JsValue::num(i.to_number(&arg(0))?) };
            let start = if args.get(1).is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.get(1), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(2), length as i64)?;
            for j in start..end { binary_write_value(i, &view, kind, j * kind.size(), true, &fill)?; }
            Ok(this)
        }
        "reverse" | "toReversed" => {
            if name == "toReversed" {
                binary_typed_from_values(i, kind, values.into_iter().rev().collect())
            } else {
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
        }
        "slice" | "subarray" => {
            let start = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(1), length as i64)?.max(start);
            if name == "subarray" {
                let constructor = binary_typed_species(i, &this, kind)?;
                let result = i.construct(&constructor, vec![JsValue::Object(view.buffer_object),
                    JsValue::num((view.offset + start * kind.size()) as f64), JsValue::num((end - start) as f64)])?;
                let (result_view, result_kind) = binary_typed(&result)?;
                if result_kind.big() != kind.big() || binary_length(&result_view) < end - start {
                    return Err(Throw::type_error("incompatible TypedArray subarray species"));
                }
                Ok(result)
            } else { binary_typed_species_from_values(i, &this, kind, values[start..end].to_vec()) }
        }
        "set" => {
            let source = arg(0);
            let source_values = if let Ok((source_view, source_kind)) = binary_typed(&source) {
                binary_typed_values(&source_view, source_kind)?
            } else {
                let len = i.get_prop(&source, "length")?;
                let len = binary_index(i, &len)?.min(1_000_000);
                (0..len).map(|j| i.get_prop(&source, j.to_string())).collect::<Result<Vec<_>, _>>()?
            };
            let offset = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
            if offset > length || source_values.len() > length - offset { return Err(Throw::range_error("source does not fit TypedArray")); }
            for (j, value) in source_values.iter().enumerate() { binary_write_value(i, &view, kind, (offset+j)*kind.size(), true, value)?; }
            Ok(JsValue::Undefined)
        }
        "copyWithin" => {
            let target = if args.first().is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.first(), length as i64)? } else { 0 };
            let start = if args.get(1).is_some_and(|x| !matches!(x, JsValue::Undefined)) { binary_relative(i, args.get(1), length as i64)? } else { 0 };
            let end = binary_relative(i, args.get(2), length as i64)?;
            let count = end.saturating_sub(start).min(length.saturating_sub(target));
            if binary_view_out_of_bounds(&view) || target + count > binary_length(&view)
                || start + count > binary_length(&view) { return Err(Throw::type_error("view changed during copyWithin")); }
            let mut buffer = view.buffer.borrow_mut();
            if buffer.immutable { return Err(Throw::type_error("immutable ArrayBuffer")); }
            let from = view.offset + start * kind.size();
            let to = view.offset + target * kind.size();
            buffer.bytes.copy_within(from..from + count * kind.size(), to);
            Ok(this)
        }
        "with" => {
            let index = i.to_number(&arg(0))?;
            let index = if index < 0.0 { length as i64 + index as i64 } else { index as i64 };
            if index < 0 || index as usize >= length { return Err(Throw::range_error("index outside TypedArray")); }
            let mut copy = values; copy[index as usize] = arg(1);
            binary_typed_from_values(i, kind, copy)
        }
        "sort" | "toSorted" => {
            let compare = arg(0);
            let mut copy = values;
            for index in 1..copy.len() {
                let mut cursor = index;
                while cursor > 0 {
                    let ordering = if matches!(compare, JsValue::Undefined) {
                        if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (&copy[cursor - 1], &copy[cursor]) {
                            a.cmp(b)
                        } else {
                        let a = copy[cursor - 1].to_number()?;
                        let b = copy[cursor].to_number()?;
                        if a.is_nan() { if b.is_nan() { std::cmp::Ordering::Equal } else { std::cmp::Ordering::Greater } }
                        else if b.is_nan() { std::cmp::Ordering::Less }
                        else { a.total_cmp(&b) }
                        }
                    } else {
                        let compared = i.call_value(&compare, JsValue::Undefined,
                            vec![copy[cursor - 1].clone(), copy[cursor].clone()])?;
                        let compared = i.to_number(&compared)?;
                        compared.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
                    };
                    if ordering != std::cmp::Ordering::Greater { break; }
                    copy.swap(cursor - 1, cursor);
                    cursor -= 1;
                }
            }
            if name == "toSorted" { binary_typed_from_values(i, kind, copy) }
            else { for (j,value) in copy.iter().enumerate() { binary_write_value(i, &view, kind, j*kind.size(), true, value)?; } Ok(this) }
        }
        "map" | "filter" | "forEach" | "every" | "some" | "find" | "findIndex" | "findLast" | "findLastIndex" => {
            let callback = arg(0);
            let receiver = arg(1);
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
            let iterator_kind = match name.as_str() {
                "keys" => SequenceIteratorKind::ArrayKeys,
                "entries" => SequenceIteratorKind::ArrayEntries,
                _ => SequenceIteratorKind::ArrayValues,
            };
            Ok(i.sequence_iterator(this, iterator_kind))
        }
        _ => Err(Throw::type_error("unknown TypedArray operation")),
    }
}

fn binary_atomics(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = binary_current_name(i);
    if name == "isLockFree" {
        let size = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
        return Ok(JsValue::Bool(matches!(size.trunc() as i64, 1 | 2 | 4 | 8)));
    }
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (view, kind) = binary_typed(&target)?;
    if !kind.integer() || kind == BinaryKind::Uint8Clamped || (matches!(name.as_str(), "wait" | "waitAsync" | "notify")
        && !matches!(kind, BinaryKind::Int32 | BinaryKind::BigInt64)) {
        return Err(Throw::type_error("invalid Atomics TypedArray"));
    }
    if matches!(name.as_str(), "wait" | "waitAsync" | "notify") && !view.buffer.borrow().shared {
        return Err(Throw::type_error("Atomics wait/notify requires SharedArrayBuffer"));
    }
    let index = binary_index(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    if index >= binary_length(&view) { return Err(Throw::range_error("Atomics index outside TypedArray")); }
    if name == "wait" { return Err(Throw::type_error("Atomics.wait cannot block this agent")); }
    if name == "notify" { return Ok(JsValue::num(0.0)); }
    let offset = index * kind.size();
    let old = binary_read(&view, kind, offset, true)?;
    if name == "load" { return Ok(old); }
    if kind.big() {
        let input = i.to_bigint(args.get(2).unwrap_or(&JsValue::Undefined))?;
        let JsValue::BigInt(old_big) = &old else { unreachable!() };
        let next = match name.as_str() {
            "store" | "exchange" => input.clone(),
            "add" => old_big.add(&input), "sub" => old_big.sub(&input),
            "and" => old_big.bitwise(&input, ast::BinOp::BitAnd),
            "or" => old_big.bitwise(&input, ast::BinOp::BitOr),
            "xor" => old_big.bitwise(&input, ast::BinOp::BitXor),
            "compareExchange" => {
                let replacement = i.to_bigint(args.get(3).unwrap_or(&JsValue::Undefined))?;
                if old_big.as_ref() != &input { return Ok(old); }
                replacement
            }
            "waitAsync" => return Err(Throw::type_error("Atomics.waitAsync is unavailable")),
            _ => return Err(Throw::type_error("unknown Atomics operation")),
        };
        binary_write_value(i, &view, kind, offset, true, &Interpreter::bigint_value(next))?;
        return Ok(if name == "store" { Interpreter::bigint_value(input) } else { old });
    }
    let input = i.to_number(args.get(2).unwrap_or(&JsValue::Undefined))?;
    let old_number = old.to_number()?;
    let next = match name.as_str() {
        "store" | "exchange" => input,
        "add" => old_number + input,
        "sub" => old_number - input,
        "and" => ((old_number as i64) & (input as i64)) as f64,
        "or" => ((old_number as i64) | (input as i64)) as f64,
        "xor" => ((old_number as i64) ^ (input as i64)) as f64,
        "compareExchange" => {
            if old_number != input { return Ok(old); }
            i.to_number(args.get(3).unwrap_or(&JsValue::Undefined))?
        }
        "waitAsync" => return Err(Throw::type_error("Atomics.waitAsync is unavailable")),
        _ => return Err(Throw::type_error("unknown Atomics operation")),
    };
    binary_write(&view, kind, offset, true, next)?;
    Ok(if name == "store" { JsValue::num(input) } else { old })
}

#[cfg(test)]
mod binary_tests {
    use super::*;

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
