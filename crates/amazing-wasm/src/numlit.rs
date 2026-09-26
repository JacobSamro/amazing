//! Numeric literal parsing for the WebAssembly text format.
//!
//! Handles underscores, `0x` integers, the `inf`/`nan`/`nan:0x...` float
//! spellings and hexadecimal floats (`0x1.8p3`), which the testsuite uses
//! heavily in `float_literals.wast`.

use crate::error::{Result, WasmError};

fn malformed(s: &str) -> WasmError {
    WasmError::malformed(format!("malformed literal: {s}"))
}

fn strip_underscores(s: &str) -> String {
    s.chars().filter(|c| *c != '_').collect()
}

/// True when every `_` sits between two digits of `radix` (no leading,
/// trailing or doubled separators, and none next to `.`, signs or letters).
fn separators_ok(s: &str, radix: u32) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return false;
    }
    for (i, c) in chars.iter().enumerate() {
        if *c == '_' {
            if i == 0 || chars[i - 1].to_digit(radix).is_none() {
                return false;
            }
            if i + 1 >= chars.len() || chars[i + 1].to_digit(radix).is_none() {
                return false;
            }
        }
    }
    true
}

/// Validate the underscore placement of a float literal.
fn float_separators_ok(s: &str) -> bool {
    if !s.contains('_') || s.starts_with("nan") || s == "inf" {
        return true;
    }
    if s.starts_with("0x") || s.starts_with("0X") {
        let rest = &s[2..];
        match rest.split_once(['p', 'P']) {
            Some((mant, exp)) => separators_ok(mant, 16) && separators_ok(exp, 10),
            None => separators_ok(rest, 16),
        }
    } else {
        match s.split_once(['e', 'E']) {
            Some((mant, exp)) => separators_ok(mant, 10) && separators_ok(exp, 10),
            None => separators_ok(s, 10),
        }
    }
}

/// Parse an optionally signed integer into `(negative, magnitude)`.
fn parse_int(s: &str) -> Result<(bool, u128)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (radix, digits) =
        if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16u32, h)
        } else {
            (10u32, body)
        };
    if digits.is_empty() {
        return Err(malformed(s));
    }
    if !separators_ok(digits, radix) {
        return Err(malformed(s));
    }
    let digits = digits.replace('_', "");
    let mut mag: u128 = 0;
    for c in digits.chars() {
        let d = c.to_digit(radix).ok_or_else(|| malformed(s))?;
        mag = mag
            .checked_mul(u128::from(radix))
            .and_then(|m| m.checked_add(u128::from(d)))
            .ok_or_else(|| malformed(s))?;
    }
    Ok((neg, mag))
}

/// Parse an unsigned 32-bit index.
pub fn parse_u32(s: &str) -> Result<u32> {
    let (neg, mag) = parse_int(s)?;
    if neg || mag > u128::from(u32::MAX) {
        return Err(malformed(s));
    }
    Ok(mag as u32)
}

/// Parse a 32-bit integer constant (wrapping), rejecting out-of-range signs.
pub fn parse_i32(s: &str) -> Result<i32> {
    let (neg, mag) = parse_int(s)?;
    if neg {
        if mag > (1u128 << 31) {
            return Err(malformed(s));
        }
        Ok((-(mag as i128)) as i64 as i32)
    } else {
        if mag > u128::from(u32::MAX) {
            return Err(malformed(s));
        }
        Ok(mag as i64 as i32)
    }
}

/// Parse a 64-bit integer constant (wrapping), rejecting out-of-range signs.
pub fn parse_i64(s: &str) -> Result<i64> {
    let (neg, mag) = parse_int(s)?;
    if neg {
        if mag > (1u128 << 63) {
            return Err(malformed(s));
        }
        Ok((-(mag as i128)) as i64)
    } else {
        if mag > u128::from(u64::MAX) {
            return Err(malformed(s));
        }
        Ok(mag as i64)
    }
}

fn split_sign(s: &str) -> (bool, &str) {
    if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    }
}

fn hex_digit(c: char) -> Option<u32> {
    c.to_digit(16)
}

/// Parse a hexadecimal float body (without sign) into an exact significand
/// and a power-of-two exponent: `value = significand * 2^exponent`. If the
/// literal carries more significant bits than fit in a `u128`, the extra low
/// digits are dropped and reported through the sticky flag so rounding stays
/// correct.
fn hex_sig_exp(body: &str) -> Result<(u128, i32, bool)> {
    let s = strip_underscores(body);
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .ok_or_else(|| malformed(body))?;
    let (mantissa, exp) = match s.split_once(['p', 'P']) {
        Some((m, e)) => (m, e.parse::<i32>().map_err(|_| malformed(body))?),
        None => (s, 0),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((a, b)) => (a, b),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(malformed(body));
    }
    let mut sig: u128 = 0;
    let mut dropped: i32 = 0;
    let mut sticky = false;
    for c in int_part.chars().chain(frac_part.chars()) {
        let d = u128::from(hex_digit(c).ok_or_else(|| malformed(body))?);
        if dropped == 0 && (sig >> 124) == 0 {
            sig = sig * 16 + d;
        } else {
            dropped += 1;
            if d != 0 {
                sticky = true;
            }
        }
    }
    let scale = 4i32
        .checked_mul(frac_part.len() as i32)
        .ok_or_else(|| malformed(body))?;
    let e = exp
        .checked_sub(scale)
        .and_then(|e| e.checked_add(4 * dropped))
        .ok_or_else(|| malformed(body))?;
    Ok((sig, e, sticky))
}

/// Round the exact value `sig * 2^e2` to an IEEE binary float, returning its
/// bit pattern (without a sign bit). Uses round-to-nearest, ties-to-even, so
/// there is no double rounding through an intermediate `f64`.
#[allow(clippy::too_many_arguments)]
fn assemble(
    sig: u128,
    e2: i32,
    sticky_input: bool,
    mant_bits: u32,
    exp_bits: u32,
    sub_ulp_exp: i32,
    min_exp: i32,
    max_exp: i32,
) -> u64 {
    let sign_shift = mant_bits + exp_bits;
    let _ = sign_shift;
    let inf = ((1u64 << exp_bits) - 1) << mant_bits;
    if sig == 0 {
        return 0;
    }
    let bl = (128 - sig.leading_zeros()) as i32;
    let msb_exp = bl - 1 + e2;
    if msb_exp > max_exp {
        return inf;
    }
    if msb_exp >= min_exp {
        let shift = bl - (mant_bits as i32 + 1);
        let kept = if shift <= 0 {
            sig << ((-shift) as u32)
        } else {
            let kept = sig >> (shift as u32);
            let rem = sig & ((1u128 << (shift as u32)) - 1);
            let half = 1u128 << (shift as u32 - 1);
            if rem > half || (rem == half && ((kept & 1) == 1 || sticky_input)) { kept + 1 } else { kept }
        };
        let mut exp_field = (i64::from(msb_exp) + i64::from((1i32 << (exp_bits - 1)) - 1)) as u64;
        let mut kept = kept as u64;
        if kept == (1u64 << (mant_bits + 1)) {
            kept >>= 1;
            exp_field += 1;
        }
        if exp_field >= (1u64 << exp_bits) {
            return inf;
        }
        return (exp_field << mant_bits) | (kept - (1u64 << mant_bits));
    }
    // Subnormal (or zero): represent as m * 2^sub_ulp_exp.
    let shift = sub_ulp_exp - e2;
    if shift > 127 {
        return 0;
    }
    let m = if shift <= 0 {
        sig << ((-shift) as u32)
    } else {
        let kept = sig >> (shift as u32);
        let rem = sig & ((1u128 << (shift as u32)) - 1);
        let half = 1u128 << (shift as u32 - 1);
        if rem > half || (rem == half && ((kept & 1) == 1 || sticky_input)) { kept + 1 } else { kept }
    };
    if m == 0 {
        return 0;
    }
    let bias_field = 1u64 << mant_bits;
    if m >= (1u128 << mant_bits) {
        (m as u64) - (1u64 << mant_bits) + bias_field
    } else {
        m as u64
    }
}

/// Parse an `f32` literal.
pub fn parse_f32(s: &str) -> Result<f32> {
    let (neg, body) = split_sign(s);
    if !float_separators_ok(body) {
        return Err(malformed(s));
    }
    let sign = if neg { 0x8000_0000u32 } else { 0 };
    let bits = if body == "inf" {
        0x7f80_0000
    } else if body == "nan" {
        0x7fc0_0000
    } else if let Some(p) = body.strip_prefix("nan:0x").or_else(|| body.strip_prefix("nan:0X")) {
        let payload = u32::from_str_radix(&strip_underscores(p), 16).map_err(|_| malformed(s))?;
        if payload == 0 || payload >= 0x0080_0000 {
            return Err(malformed(s));
        }
        0x7f80_0000 | payload
    } else if body.starts_with("0x") || body.starts_with("0X") {
        let (sig, e2, sticky) = hex_sig_exp(body)?;
        let b = assemble(sig, e2, sticky, 23, 8, -149, -126, 127) as u32;
        if b == 0x7f80_0000 {
            return Err(malformed(s));
        }
        b
    } else {
        if !decimal_mantissa_ok(body) {
            return Err(malformed(s));
        }
        let v = strip_underscores(body).parse::<f32>().map_err(|_| malformed(s))?;
        if v.is_infinite() {
            return Err(malformed(s));
        }
        v.to_bits()
    };
    Ok(f32::from_bits(bits | sign))
}

/// True when a decimal (non-hex) float body has a mandatory leading digit
/// before any decimal point, as the text-format grammar requires (`.0` and
/// `.0e0` are malformed, `0.0` is not).
fn decimal_mantissa_ok(body: &str) -> bool {
    let mantissa = match body.split_once(['e', 'E']) {
        Some((m, _)) => m,
        None => body,
    };
    mantissa.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Parse an `f64` literal.
pub fn parse_f64(s: &str) -> Result<f64> {
    let (neg, body) = split_sign(s);
    if !float_separators_ok(body) {
        return Err(malformed(s));
    }
    let sign = if neg { 0x8000_0000_0000_0000u64 } else { 0 };
    let bits = if body == "inf" {
        0x7ff0_0000_0000_0000
    } else if body == "nan" {
        0x7ff8_0000_0000_0000
    } else if let Some(p) = body.strip_prefix("nan:0x").or_else(|| body.strip_prefix("nan:0X")) {
        let payload = u64::from_str_radix(&strip_underscores(p), 16).map_err(|_| malformed(s))?;
        if payload == 0 || payload >= 0x0010_0000_0000_0000 {
            return Err(malformed(s));
        }
        0x7ff0_0000_0000_0000 | payload
    } else if body.starts_with("0x") || body.starts_with("0X") {
        let (sig, e2, sticky) = hex_sig_exp(body)?;
        let b = assemble(sig, e2, sticky, 52, 11, -1074, -1022, 1023);
        if b == 0x7ff0_0000_0000_0000 {
            return Err(malformed(s));
        }
        b
    } else {
        if !decimal_mantissa_ok(body) {
            return Err(malformed(s));
        }
        let v = strip_underscores(body).parse::<f64>().map_err(|_| malformed(s))?;
        if v.is_infinite() {
            return Err(malformed(s));
        }
        v.to_bits()
    };
    Ok(f64::from_bits(bits | sign))
}
