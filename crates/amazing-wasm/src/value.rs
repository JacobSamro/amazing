//! Runtime values and the reference type.
//!
//! WebAssembly has four numeric value types plus two reference types. All of
//! them live in [`Value`]; a reference is either null (typed), a function
//! address, or an opaque host value (the `externref` testsuite uses small
//! integers). The float helpers implement the specification's exact handling
//! of NaN, signed zero, rounding and trapping conversions, because the
//! testsuite asserts those bit-for-bit.

use crate::error::{Result, WasmError};
use crate::types::ValType;

/// A reference: null, a function address, or an opaque host value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ref {
    /// A typed null reference (`ref.null t`).
    Null(ValType),
    /// A non-null function reference (an index into the function address space).
    Func(u32),
    /// A non-null host reference.
    Extern(u64),
}

impl Ref {
    /// The static type of this reference.
    pub fn ty(self) -> ValType {
        match self {
            Ref::Null(t) => t,
            Ref::Func(_) => ValType::FuncRef,
            Ref::Extern(_) => ValType::ExternRef,
        }
    }

    /// True for the null reference.
    pub fn is_null(self) -> bool {
        matches!(self, Ref::Null(_))
    }
}

/// A runtime value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    /// 32-bit integer.
    I32(i32),
    /// 64-bit integer.
    I64(i64),
    /// 32-bit float.
    F32(f32),
    /// 64-bit float.
    F64(f64),
    /// A reference.
    Ref(Ref),
}

impl Value {
    /// The value type of this value.
    pub fn ty(self) -> ValType {
        match self {
            Value::I32(_) => ValType::I32,
            Value::I64(_) => ValType::I64,
            Value::F32(_) => ValType::F32,
            Value::F64(_) => ValType::F64,
            Value::Ref(r) => r.ty(),
        }
    }

    /// The `i32` payload, or a type mismatch error.
    pub fn as_i32(self) -> Result<i32> {
        match self {
            Value::I32(v) => Ok(v),
            _ => Err(WasmError::invalid("type mismatch: expected i32")),
        }
    }

    /// The `i64` payload, or a type mismatch error.
    pub fn as_i64(self) -> Result<i64> {
        match self {
            Value::I64(v) => Ok(v),
            _ => Err(WasmError::invalid("type mismatch: expected i64")),
        }
    }

    /// The `f32` payload, or a type mismatch error.
    pub fn as_f32(self) -> Result<f32> {
        match self {
            Value::F32(v) => Ok(v),
            _ => Err(WasmError::invalid("type mismatch: expected f32")),
        }
    }

    /// The `f64` payload, or a type mismatch error.
    pub fn as_f64(self) -> Result<f64> {
        match self {
            Value::F64(v) => Ok(v),
            _ => Err(WasmError::invalid("type mismatch: expected f64")),
        }
    }

    /// The reference payload, or a type mismatch error.
    pub fn as_ref(self) -> Result<Ref> {
        match self {
            Value::Ref(r) => Ok(r),
            _ => Err(WasmError::invalid("type mismatch: expected a reference")),
        }
    }

    /// A zero value of a numeric type (used by `table.grow`/`memory` locals).
    pub fn default_for(ty: ValType) -> Value {
        match ty {
            ValType::I32 => Value::I32(0),
            ValType::I64 => Value::I64(0),
            ValType::F32 => Value::F32(0.0),
            ValType::F64 => Value::F64(0.0),
            ValType::V128 => Value::I32(0),
            ValType::FuncRef | ValType::ExternRef => Value::Ref(Ref::Null(ty)),
        }
    }

    /// A null reference of the given type, for `table.grow`-style defaults.
    pub fn default_ref(ty: ValType) -> Value {
        Value::Ref(Ref::Null(ty))
    }
}

/// The canonical quiet NaN (`0x7fc00000` / `0x7ff8...`).
pub const F32_CANONICAL_NAN: u32 = 0x7fc0_0000;
/// The canonical quiet NaN for `f64`.
pub const F64_CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;

/// Canonical `f32` NaN.
pub fn canonical_f32_nan() -> f32 {
    f32::from_bits(F32_CANONICAL_NAN)
}

/// Canonical `f64` NaN.
pub fn canonical_f64_nan() -> f64 {
    f64::from_bits(F64_CANONICAL_NAN)
}

/// True if the `f32` bit pattern is a canonical NaN.
pub fn is_canonical_f32(bits: u32) -> bool {
    bits & 0x7fff_ffff == F32_CANONICAL_NAN
}

/// True if the `f64` bit pattern is a canonical NaN.
pub fn is_canonical_f64(bits: u64) -> bool {
    bits & 0x7fff_ffff_ffff_ffff == F64_CANONICAL_NAN
}

/// True if the `f32` bit pattern is any quiet NaN.
pub fn is_arithmetic_f32(bits: u32) -> bool {
    (bits & 0x7f80_0000) == 0x7f80_0000 && (bits & 0x0040_0000) != 0
}

/// True if the `f64` bit pattern is any quiet NaN.
pub fn is_arithmetic_f64(bits: u64) -> bool {
    (bits & 0x7ff0_0000_0000_0000) == 0x7ff0_0000_0000_0000 && (bits & 0x0008_0000_0000_0000) != 0
}

/// `f32.min` with WebAssembly's NaN and signed-zero rules.
pub fn f32_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return canonical_f32_nan();
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() || b.is_sign_negative() { -0.0 } else { 0.0 };
    }
    if a < b { a } else { b }
}

/// `f32.max` with WebAssembly's NaN and signed-zero rules.
pub fn f32_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        return canonical_f32_nan();
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() || b.is_sign_positive() { 0.0 } else { -0.0 };
    }
    if a > b { a } else { b }
}

/// `f64.min` with WebAssembly's NaN and signed-zero rules.
pub fn f64_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return canonical_f64_nan();
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() || b.is_sign_negative() { -0.0 } else { 0.0 };
    }
    if a < b { a } else { b }
}

/// `f64.max` with WebAssembly's NaN and signed-zero rules.
pub fn f64_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return canonical_f64_nan();
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() || b.is_sign_positive() { 0.0 } else { -0.0 };
    }
    if a > b { a } else { b }
}

/// `f32.copysign`, preserving the payload of `a` and the sign of `b`.
pub fn f32_copysign(a: f32, b: f32) -> f32 {
    f32::from_bits((a.to_bits() & 0x7fff_ffff) | (b.to_bits() & 0x8000_0000))
}

/// `f64.copysign`, preserving the payload of `a` and the sign of `b`.
pub fn f64_copysign(a: f64, b: f64) -> f64 {
    f64::from_bits((a.to_bits() & 0x7fff_ffff_ffff_ffff) | (b.to_bits() & 0x8000_0000_0000_0000))
}

/// `f32.abs`.
pub fn f32_abs(a: f32) -> f32 {
    f32::from_bits(a.to_bits() & 0x7fff_ffff)
}

/// `f64.abs`.
pub fn f64_abs(a: f64) -> f64 {
    f64::from_bits(a.to_bits() & 0x7fff_ffff_ffff_ffff)
}

/// `f32.neg`, which flips the sign bit even for NaN.
pub fn f32_neg(a: f32) -> f32 {
    f32::from_bits(a.to_bits() ^ 0x8000_0000)
}

/// Quiet a NaN result (set the quiet bit) as arithmetic operations require.
pub fn quiet_f32(a: f32) -> f32 {
    if a.is_nan() {
        f32::from_bits(a.to_bits() | 0x0040_0000)
    } else {
        a
    }
}

/// Quiet an `f64` NaN result.
pub fn quiet_f64(a: f64) -> f64 {
    if a.is_nan() {
        f64::from_bits(a.to_bits() | 0x0008_0000_0000_0000)
    } else {
        a
    }
}

/// `f64.neg`.
pub fn f64_neg(a: f64) -> f64 {
    f64::from_bits(a.to_bits() ^ 0x8000_0000_0000_0000)
}

/// `f32.nearest` (round to nearest, ties to even).
pub fn f32_nearest(a: f32) -> f32 {
    if a.is_nan() || a.is_infinite() || a == 0.0 {
        return a;
    }
    a.round_ties_even()
}

/// `f64.nearest` (round to nearest, ties to even).
pub fn f64_nearest(a: f64) -> f64 {
    if a.is_nan() || a.is_infinite() || a == 0.0 {
        return a;
    }
    a.round_ties_even()
}

/// `i32.trunc_f32_s`; traps on NaN or out-of-range.
pub fn i32_trunc_f32_s(a: f32) -> Result<i32> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t >= -2147483648.0 && t < 2147483648.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as i32)
}

/// `i32.trunc_f32_u`; traps on NaN or out-of-range.
pub fn i32_trunc_f32_u(a: f32) -> Result<u32> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t > -1.0 && t < 4294967296.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as u32)
}

/// `i32.trunc_f64_s`.
pub fn i32_trunc_f64_s(a: f64) -> Result<i32> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t >= -2147483648.0 && t < 2147483648.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as i32)
}

/// `i32.trunc_f64_u`.
pub fn i32_trunc_f64_u(a: f64) -> Result<u32> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t > -1.0 && t < 4294967296.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as u32)
}

/// `i64.trunc_f32_s`.
pub fn i64_trunc_f32_s(a: f32) -> Result<i64> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t >= -9223372036854775808.0 && t < 9223372036854775808.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as i64)
}

/// `i64.trunc_f32_u`.
pub fn i64_trunc_f32_u(a: f32) -> Result<u64> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t > -1.0 && t < 18446744073709551616.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as u64)
}

/// `i64.trunc_f64_s`.
pub fn i64_trunc_f64_s(a: f64) -> Result<i64> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t >= -9223372036854775808.0 && t < 9223372036854775808.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as i64)
}

/// `i64.trunc_f64_u`.
pub fn i64_trunc_f64_u(a: f64) -> Result<u64> {
    if a.is_nan() {
        return Err(WasmError::trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(t > -1.0 && t < 18446744073709551616.0) {
        return Err(WasmError::trap("integer overflow"));
    }
    Ok(t as u64)
}

/// `i32.trunc_sat_f32_s`.
pub fn i32_trunc_sat_f32_s(a: f32) -> i32 {
    if a.is_nan() {
        0
    } else if a <= -2147483648.0 {
        i32::MIN
    } else if a >= 2147483647.0 + 1.0 {
        i32::MAX
    } else {
        a.trunc() as i32
    }
}

/// `i32.trunc_sat_f32_u`.
pub fn i32_trunc_sat_f32_u(a: f32) -> u32 {
    if a.is_nan() || a <= -1.0 {
        0
    } else if a >= 4294967296.0 {
        u32::MAX
    } else {
        a.trunc() as u32
    }
}

/// `i32.trunc_sat_f64_s`.
pub fn i32_trunc_sat_f64_s(a: f64) -> i32 {
    if a.is_nan() {
        0
    } else if a <= -2147483648.0 {
        i32::MIN
    } else if a >= 2147483648.0 {
        i32::MAX
    } else {
        a.trunc() as i32
    }
}

/// `i32.trunc_sat_f64_u`.
pub fn i32_trunc_sat_f64_u(a: f64) -> u32 {
    if a.is_nan() || a <= -1.0 {
        0
    } else if a >= 4294967296.0 {
        u32::MAX
    } else {
        a.trunc() as u32
    }
}

/// `i64.trunc_sat_f32_s`.
pub fn i64_trunc_sat_f32_s(a: f32) -> i64 {
    if a.is_nan() {
        0
    } else if a <= -9223372036854775808.0 {
        i64::MIN
    } else if a >= 9223372036854775808.0 {
        i64::MAX
    } else {
        a.trunc() as i64
    }
}

/// `i64.trunc_sat_f32_u`.
pub fn i64_trunc_sat_f32_u(a: f32) -> u64 {
    if a.is_nan() || a <= -1.0 {
        0
    } else if a >= 18446744073709551616.0 {
        u64::MAX
    } else {
        a.trunc() as u64
    }
}

/// `i64.trunc_sat_f64_s`.
pub fn i64_trunc_sat_f64_s(a: f64) -> i64 {
    if a.is_nan() {
        0
    } else if a <= -9223372036854775808.0 {
        i64::MIN
    } else if a >= 9223372036854775808.0 {
        i64::MAX
    } else {
        a.trunc() as i64
    }
}

/// `i64.trunc_sat_f64_u`.
pub fn i64_trunc_sat_f64_u(a: f64) -> u64 {
    if a.is_nan() || a <= -1.0 {
        0
    } else if a >= 18446744073709551616.0 {
        u64::MAX
    } else {
        a.trunc() as u64
    }
}
