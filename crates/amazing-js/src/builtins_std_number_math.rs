// Portions ported from boa-dev/boa (core/engine/src/builtins/number/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Formatting entry conversion/validation order adapted to the existing exact
// decimal formatter; upstream formatting dependencies are not imported.
// Portions ported from boa-dev/boa (core/engine/src/builtins/math/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Number's decimal formatting works from the exact binary64 rational. This
// avoids host formatting ties-to-even behavior in toFixed/toPrecision.

struct StdDecimalBig(Vec<u32>);
impl StdDecimalBig {
    fn from_u64(value: u64) -> Self { Self(vec![value as u32, (value >> 32) as u32]) }
    fn mul_small(&mut self, factor: u32) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let value = *limb as u64 * factor as u64 + carry;
            *limb = value as u32; carry = value >> 32;
        }
        if carry != 0 { self.0.push(carry as u32); }
    }
    fn shift_left(&mut self, bits: usize) {
        for _ in 0..bits { self.mul_small(2); }
    }
    fn to_decimal(mut self) -> String {
        while self.0.last() == Some(&0) && self.0.len() > 1 { self.0.pop(); }
        if self.0.iter().all(|limb| *limb == 0) { return "0".to_string(); }
        let mut digits = Vec::new();
        while self.0.iter().any(|limb| *limb != 0) {
            let mut remainder = 0u64;
            for limb in self.0.iter_mut().rev() {
                let value = (remainder << 32) | *limb as u64;
                *limb = (value / 10) as u32; remainder = value % 10;
            }
            digits.push((b'0' + remainder as u8) as char);
        }
        digits.iter().rev().collect()
    }
}

fn std_number_exact_decimal(value: f64) -> (String, i32) {
    if value == 0.0 { return ("0".to_string(), 1); }
    let bits = value.abs().to_bits();
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if exponent_bits == 0 { (fraction, -1074) }
        else { (fraction | (1u64 << 52), exponent_bits - 1023 - 52) };
    let mut number = StdDecimalBig::from_u64(mantissa);
    let scale = if exponent < 0 {
        for _ in 0..-exponent { number.mul_small(5); }
        -exponent
    } else { number.shift_left(exponent as usize); 0 };
    let digits = number.to_decimal();
    let point = digits.len() as i32 - scale;
    (digits, point)
}

fn std_decimal_increment(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit < b'9' { *digit += 1; return; }
        *digit = b'0';
    }
    digits.insert(0, b'1');
}

fn std_number_fixed(value: f64, places: usize) -> String {
    if value.is_nan() { return "NaN".to_string(); }
    if value.is_infinite() { return if value < 0.0 { "-Infinity" } else { "Infinity" }.to_string(); }
    if value.abs() >= 1e21 { return crate::value::number_to_string(value); }
    let (digits, point) = std_number_exact_decimal(value);
    let cut = point + places as i32;
    let mut kept = if cut <= 0 { Vec::new() }
        else { digits.as_bytes()[..(cut as usize).min(digits.len())].to_vec() };
    if cut > digits.len() as i32 { kept.extend(std::iter::repeat_n(b'0', cut as usize - digits.len())); }
    if cut >= 0 && digits.as_bytes().get(cut as usize).is_some_and(|digit| *digit >= b'5') {
        std_decimal_increment(&mut kept);
    }
    if kept.is_empty() { kept.push(b'0'); }
    if kept.len() <= places {
        let mut padded = vec![b'0'; places + 1 - kept.len()]; padded.extend(kept); kept = padded;
    }
    let mut out = String::from_utf8(kept).unwrap_or_default();
    if places > 0 { out.insert(out.len() - places, '.'); }
    if value < 0.0 { out.insert(0, '-'); }
    out
}

fn std_number_significant(value: f64, precision: usize) -> (String, i32) {
    if value == 0.0 { return ("0".repeat(precision), 0); }
    let (digits, point) = std_number_exact_decimal(value);
    let mut kept = digits.as_bytes()[..precision.min(digits.len())].to_vec();
    if precision > digits.len() { kept.extend(std::iter::repeat_n(b'0', precision - digits.len())); }
    let mut exponent = point - 1;
    if digits.as_bytes().get(precision).is_some_and(|digit| *digit >= b'5') {
        std_decimal_increment(&mut kept);
        if kept.len() > precision { kept.pop(); exponent += 1; }
    }
    (String::from_utf8(kept).unwrap_or_default(), exponent)
}

fn std_number_exponential_text(value: f64, precision: Option<usize>) -> String {
    if value.is_nan() { return "NaN".to_string(); }
    if value.is_infinite() { return if value < 0.0 { "-Infinity" } else { "Infinity" }.to_string(); }
    let (mut digits, exponent) = if let Some(places) = precision {
        std_number_significant(value, places + 1)
    } else if value == 0.0 { ("0".to_string(), 0) }
    else {
        let source = crate::value::number_to_string(value.abs());
        let (base, explicit) = source.split_once('e').unwrap_or((&source, ""));
        let explicit = explicit.parse::<i32>().unwrap_or(0);
        let before = base.find('.').unwrap_or(base.len()) as i32;
        let digits = base.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
        let leading = digits.bytes().take_while(|c| *c == b'0').count() as i32;
        (digits.trim_start_matches('0').trim_end_matches('0').to_string(), before - leading - 1 + explicit)
    };
    if digits.is_empty() { digits.push('0'); }
    let mut text = String::new();
    if value < 0.0 { text.push('-'); }
    text.push(digits.as_bytes()[0] as char);
    if digits.len() > 1 { text.push('.'); text.push_str(&digits[1..]); }
    text.push_str(&format!("e{}{exponent}", if exponent >= 0 { "+" } else { "" }));
    text
}

fn std_number_receiver(this: JsValue) -> Result<f64, Throw> {
    match this {
        JsValue::Num(number) => Ok(number),
        JsValue::Object(object) => match object.borrow().boxed_primitive.clone() {
            Some(JsValue::Num(number)) => Ok(number),
            _ => Err(Throw::type_error("incompatible Number receiver")),
        },
        _ => Err(Throw::type_error("incompatible Number receiver")),
    }
}

impl Interpreter {
    fn install_std_number_math(&mut self) {
        let replacement = Interpreter::native_constructor("Number", std_number_constructor, &self.function_proto);
        self.define_global("Number", replacement.clone());
        if let JsValue::Object(number) = replacement {
            number.borrow_mut().define("length", symbol_data(JsValue::num(1.0), false, true)).unwrap();
            let prototype = self.number_proto.clone();
            number.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
            prototype.borrow_mut().define("constructor", symbol_data(JsValue::Object(number.clone()), true, true)).unwrap();
            for (name, value) in [
                ("EPSILON", f64::EPSILON), ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
                ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0), ("MAX_VALUE", f64::MAX),
                ("MIN_VALUE", f64::from_bits(1)), ("NaN", f64::NAN),
                ("POSITIVE_INFINITY", f64::INFINITY), ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ] { number.borrow_mut().define(name, symbol_data(JsValue::num(value), false, false)).unwrap(); }
            for (name, length, native) in [
                ("isNaN", 1, native_number_is_nan as crate::value::NativeFn),
                ("isFinite", 1, native_number_is_finite as crate::value::NativeFn),
                ("isInteger", 1, std_number_is_integer as crate::value::NativeFn),
                ("isSafeInteger", 1, std_number_is_safe_integer as crate::value::NativeFn),
            ] { std_core_define(self, &number, name, length, native); }
            for (name, length, native) in [
                ("toString", 1, std_number_to_string as crate::value::NativeFn),
                ("toLocaleString", 0, std_number_to_locale_string as crate::value::NativeFn),
                ("valueOf", 0, std_number_value_of as crate::value::NativeFn),
                ("toFixed", 1, std_number_to_fixed as crate::value::NativeFn),
                ("toExponential", 1, std_number_to_exponential as crate::value::NativeFn),
                ("toPrecision", 1, std_number_to_precision as crate::value::NativeFn),
            ] { std_core_define(self, &prototype, name, length, native); }
            if let Some(parse_int) = self.global_env.resolve("parseInt") {
                number.borrow_mut().define("parseInt", symbol_data(parse_int, true, true)).unwrap();
            }
            if let Some(parse_float) = self.global_env.resolve("parseFloat") {
                number.borrow_mut().define("parseFloat", symbol_data(parse_float, true, true)).unwrap();
            }
        }
        if let Some(JsValue::Object(math)) = self.global_env.resolve("Math") {
            for (name, value) in [
                ("E", std::f64::consts::E), ("LN10", std::f64::consts::LN_10),
                ("LN2", std::f64::consts::LN_2), ("LOG10E", std::f64::consts::LOG10_E),
                ("LOG2E", std::f64::consts::LOG2_E), ("PI", std::f64::consts::PI),
                ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2), ("SQRT2", std::f64::consts::SQRT_2),
            ] { math.borrow_mut().define(name, symbol_data(JsValue::num(value), false, false)).unwrap(); }
            for (name, length) in [
                ("acos", 1), ("acosh", 1), ("asin", 1), ("asinh", 1), ("atan", 1),
                ("atan2", 2), ("atanh", 1), ("cbrt", 1), ("clz32", 1),
                ("cos", 1), ("cosh", 1), ("exp", 1), ("expm1", 1),
                ("f16round", 1), ("fround", 1), ("hypot", 2), ("imul", 2),
                ("log", 1), ("log10", 1), ("log1p", 1), ("log2", 1),
                ("sin", 1), ("sinh", 1), ("sumPrecise", 1), ("tan", 1), ("tanh", 1),
            ] { std_core_define(self, &math, name, length, std_math_dispatch); }
            std_core_define(self, &math, "max", 2, std_math_max);
            std_core_define(self, &math, "min", 2, std_math_min);
            math.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                symbol_data(JsValue::str("Math"), false, true)).unwrap();
        }
    }
}

fn std_number_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = if let Some(value) = args.first() {
        // Number(value) is ToNumeric(value) (21.1.1.1) -- ToPrimitive with the number
        // hint, not default.
        let primitive = i.primitive_number_hint(value.clone())?;
        if let JsValue::BigInt(bigint) = &primitive { bigint.to_f64() }
        else { i.to_number(&primitive)? }
    } else { 0.0 };
    if let JsValue::Object(object) = &this {
        if object.borrow().constructed {
            object.borrow_mut().boxed_primitive = Some(JsValue::num(value));
            return Ok(this);
        }
    }
    Ok(JsValue::num(value))
}

fn std_number_is_integer(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(matches!(args.first(), Some(JsValue::Num(n)) if n.is_finite() && n.fract() == 0.0)))
}
fn std_number_is_safe_integer(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(matches!(args.first(), Some(JsValue::Num(n)) if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)))
}
fn std_number_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(std_number_receiver(this)?))
}
fn std_number_to_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_number_receiver(this)?;
    let radix_value = std_reflect_arg(&args, 0);
    let radix = if radix_value.is_undefined() { 10 } else { i.to_number(&radix_value)? as i32 };
    if !(2..=36).contains(&radix) { return Err(Throw::range_error("invalid radix")); }
    if radix == 10 || !value.is_finite() || value == 0.0 {
        return Ok(JsValue::str(crate::value::number_to_string(value)));
    }
    let mut whole = value.abs().trunc();
    let mut digits = Vec::new();
    while whole >= 1.0 && digits.len() < 1024 {
        let digit = (whole % radix as f64) as u8;
        digits.push(if digit < 10 { b'0' + digit } else { b'a' + digit - 10 });
        whole = (whole / radix as f64).floor();
    }
    if digits.is_empty() { digits.push(b'0'); }
    digits.reverse();
    let mut text = String::from_utf8(digits).unwrap_or_default();
    let mut fractional = value.abs().fract();
    if fractional != 0.0 {
        text.push('.');
        for _ in 0..64 {
            fractional *= radix as f64;
            let digit = fractional.floor() as u8;
            text.push(if digit < 10 { (b'0' + digit) as char } else { (b'a' + digit - 10) as char });
            fractional -= digit as f64;
            if fractional == 0.0 { break; }
        }
    }
    if value < 0.0 { text.insert(0, '-'); }
    Ok(JsValue::str(text))
}
fn std_number_to_locale_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_number_to_string(i, this, vec![])
}
fn std_number_digits(number: f64, minimum: usize) -> Result<usize, Throw> {
    if !(minimum as f64..=100.0).contains(&number) {
        return Err(Throw::range_error("invalid precision"));
    }
    Ok(number as usize)
}
fn std_number_to_fixed(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_number_receiver(this)?;
    let fraction = to_integer_or_infinity(i, &std_reflect_arg(&args, 0))?;
    // toFixed validates the range even for NaN/infinity.
    let digits = std_number_digits(fraction, 0)?;
    Ok(JsValue::str(std_number_fixed(value, digits)))
}
fn std_number_to_exponential(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_number_receiver(this)?;
    let argument = std_reflect_arg(&args, 0);
    let fraction = to_integer_or_infinity(i, &argument)?;
    // Argument coercion is observable, but range validation follows this return.
    if !value.is_finite() { return Ok(JsValue::str(crate::value::number_to_string(value))); }
    let digits = std_number_digits(fraction, 0)?;
    let fraction = if argument.is_undefined() { None } else { Some(digits) };
    Ok(JsValue::str(std_number_exponential_text(value, fraction)))
}
fn std_number_to_precision(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_number_receiver(this)?;
    let precision = std_reflect_arg(&args, 0);
    if precision.is_undefined() { return Ok(JsValue::str(crate::value::number_to_string(value))); }
    let precision = to_integer_or_infinity(i, &precision)?;
    if !value.is_finite() { return Ok(JsValue::str(crate::value::number_to_string(value))); }
    let precision = std_number_digits(precision, 1)?;
    let (digits, exponent) = std_number_significant(value, precision);
    let mut text = if exponent < -6 || exponent >= precision as i32 {
        let mut out = digits[..1].to_string();
        if precision > 1 { out.push('.'); out.push_str(&digits[1..]); }
        out.push_str(&format!("e{}{exponent}", if exponent >= 0 { "+" } else { "" }));
        out
    } else {
        let point = exponent + 1;
        if point <= 0 { format!("0.{}{}", "0".repeat((-point) as usize), digits) }
        else if point as usize >= digits.len() { format!("{}{}", digits, "0".repeat(point as usize - digits.len())) }
        else { format!("{}.{}", &digits[..point as usize], &digits[point as usize..]) }
    };
    if value < 0.0 { text.insert(0, '-'); }
    Ok(JsValue::str(text))
}

fn std_math_dispatch(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    if name == "sumPrecise" {
        return native_math_sum_precise(i, std_reflect_arg(&args, 0));
    }
    if name == "hypot" {
        let mut numbers = Vec::new();
        for arg in args { numbers.push(i.to_number(&arg)?); }
        if numbers.iter().any(|n| n.is_infinite()) { return Ok(JsValue::num(f64::INFINITY)); }
        let max = numbers.iter().copied().fold(0.0f64, |a, b| a.max(b.abs()));
        return Ok(JsValue::num(if max == 0.0 { 0.0 }
            else { max * numbers.iter().map(|n| (n / max).powi(2)).sum::<f64>().sqrt() }));
    }
    let x = i.to_number(&std_reflect_arg(&args, 0))?;
    let result = match name.as_str() {
        "acos" => x.acos(), "acosh" => x.acosh(), "asin" => x.asin(),
        "asinh" => x.asinh(), "atan" => x.atan(), "atanh" => x.atanh(),
        "cbrt" => x.cbrt(), "cos" => x.cos(), "cosh" => x.cosh(),
        "exp" => x.exp(), "expm1" => x.exp_m1(), "fround" => (x as f32) as f64,
        "f16round" => std_math_f16_round(x),
        "log" => x.ln(), "log10" => x.log10(), "log1p" => x.ln_1p(),
        "log2" => x.log2(), "sin" => x.sin(), "sinh" => x.sinh(),
        "tan" => x.tan(), "tanh" => x.tanh(),
        "clz32" => {
            let bits = if x.is_finite() { x.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 };
            bits.leading_zeros() as f64
        }
        "imul" => {
            let y = i.to_number(&std_reflect_arg(&args, 1))?;
            let a = if x.is_finite() { x.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 };
            let b = if y.is_finite() { y.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 };
            a.wrapping_mul(b) as i32 as f64
        }
        "atan2" => { let y = i.to_number(&std_reflect_arg(&args, 1))?; x.atan2(y) }
        _ => return Err(Throw::type_error("unknown Math function")),
    };
    Ok(JsValue::num(result))
}

fn std_math_max(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut result = f64::NEG_INFINITY;
    for arg in args {
        let value = i.to_number(&arg)?;
        if value.is_nan() { result = f64::NAN; }
        else if !result.is_nan() && (value > result || (value == 0.0 && result == 0.0 && value.is_sign_positive())) {
            result = value;
        }
    }
    Ok(JsValue::num(result))
}
fn std_math_min(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut result = f64::INFINITY;
    for arg in args {
        let value = i.to_number(&arg)?;
        if value.is_nan() { result = f64::NAN; }
        else if !result.is_nan() && (value < result || (value == 0.0 && result == 0.0 && value.is_sign_negative())) {
            result = value;
        }
    }
    Ok(JsValue::num(result))
}
fn std_math_even_round(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 || (fraction == 0.5 && floor.rem_euclid(2.0) == 0.0) { floor }
    else { floor + 1.0 }
}
fn std_math_f16_round(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 { return value; }
    let magnitude = value.abs();
    let sign = value.signum();
    if magnitude < 2f64.powi(-14) {
        let rounded = std_math_even_round(magnitude * 2f64.powi(24));
        return sign * rounded * 2f64.powi(-24);
    }
    let exponent = magnitude.log2().floor() as i32;
    if exponent > 15 { return sign * f64::INFINITY; }
    let significand = std_math_even_round(magnitude / 2f64.powi(exponent) * 1024.0);
    let rounded = significand * 2f64.powi(exponent - 10);
    if rounded > 65504.0 { sign * f64::INFINITY } else { sign * rounded }
}

#[cfg(test)]
mod number_format_entry_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn digit_conversion_uses_integer_or_infinity_and_complete_ranges() {
        check(r#"
          var valid=true,f=Number.prototype.toFixed,e=Number.prototype.toExponential,p=Number.prototype.toPrecision;
          for(var digits of [NaN,undefined,null,false,'bad',-0,-0.9]){
            valid=valid && f.call(12.25,digits)==='12' && e.call(12.25,digits)===(digits===undefined?'1.225e+1':'1e+1');
          }
          for(var digits of [1,1.9,true,'1'])valid=valid && f.call(12.25,digits)==='12.3' && e.call(12.25,digits)==='1.2e+1' && p.call(12.25,digits)==='1e+1';
          valid=valid && f.call(0,100.9).length===102 && e.call(0,100.9).length===105 && p.call(0,100.9).length===101;
          for(var method of [f,e,p])for(var digits of [101,Infinity,-Infinity,-1]){
            try{method.call(1,digits);valid=false;}catch(error){valid=valid && error instanceof RangeError;}
          }
          for(var digits of [0,NaN,null,false,-0.9]){try{p.call(1,digits);valid=false;}catch(error){valid=valid && error instanceof RangeError;}}
          valid=valid && p.call(12.25)==='12.25';valid;
        "#);
    }
    #[test]
    fn nonfinite_receiver_returns_follow_coercion_before_distinct_range_checks() {
        check(r#"
          var valid=true,error={},f=Number.prototype.toFixed,e=Number.prototype.toExponential,p=Number.prototype.toPrecision;
          for(var number of [NaN,Infinity,-Infinity]){
            var expected=number!==number?'NaN':number<0?'-Infinity':'Infinity';
            for(var method of [f,e,p]){
              var calls=0,arg={valueOf(){calls++;return 1;}};
              valid=valid && method.call(number,arg)===expected && calls===1;
              var arg={valueOf(){throw error;}};
              try{method.call(number,arg);valid=false;}catch(thrown){valid=valid && thrown===error;}
              for(var arg of [Symbol(),1n]){try{method.call(number,arg);valid=false;}catch(thrown){valid=valid && thrown instanceof TypeError;}}
            }
            for(var digits of [-1,101,Infinity,-Infinity]){
              valid=valid && e.call(number,digits)===expected && p.call(number,digits)===expected;
              try{f.call(number,digits);valid=false;}catch(thrown){valid=valid && thrown instanceof RangeError;}
            }
            valid=valid && f.call(number,NaN)===expected && e.call(number,NaN)===expected && p.call(number,NaN)===expected;
          }
          valid;
        "#);
    }
    #[test]
    fn number_brand_checks_precede_argument_coercion_and_preserve_abrupt_identity() {
        check(r#"
          var valid=true,error={},calls=0,arg={valueOf(){calls++;throw error;}},methods=[Number.prototype.toFixed,Number.prototype.toExponential,Number.prototype.toPrecision];
          for(var method of methods){
            for(var receiver of [undefined,null,true,'1',1n,Symbol(),{},Object.create(Number.prototype),new Proxy(new Number(1),{})]){
              try{method.call(receiver,arg);valid=false;}catch(thrown){valid=valid && thrown instanceof TypeError && thrown!==error;}
            }
            valid=valid && calls===0;
          }
          for(var method of methods){try{method.call(new Number(1),arg);valid=false;}catch(thrown){valid=valid && thrown===error;}}
          valid=valid && calls===3;
          var log=[],precision={[Symbol.toPrimitive](hint){log.push(hint);return 2;},valueOf(){throw 'unused';}};
          valid=valid && Number.prototype.toFixed.call(1.25,precision)==='1.25' && log.join(',')==='number';valid;
        "#);
    }
    #[test]
    fn exact_decimal_formatter_sign_ties_and_shortest_paths_stay_intact() {
        check(r#"
          var valid=true;
          for(var sign of [1,-1]){
            valid=valid && (sign*2.5).toFixed(0)===(sign===1?'3':'-3');
            valid=valid && (sign*1.005).toFixed(2)===(sign===1?'1.00':'-1.00');
            valid=valid && (sign*9.99).toPrecision(2)===(sign===1?'10':'-10');
            valid=valid && (sign*9.99).toExponential(1)===(sign===1?'1.0e+1':'-1.0e+1');
          }
          valid=valid && (-0).toFixed(2)==='0.00' && (-0).toExponential(2)==='0.00e+0' && (-0).toPrecision(2)==='0.0';
          valid=valid && (1e21).toFixed(2)==='1e+21' && (0.000001).toPrecision(1)==='0.000001' && (0.0000001).toPrecision(1)==='1e-7';
          valid=valid && (123.25).toExponential()==='1.2325e+2' && Number.prototype.toPrecision.call(new Number(1.25),3)==='1.25';
          valid=valid && Number.prototype.toFixed.call(Number.prototype,1)==='0.0';valid;
        "#);
    }
    #[test]
    fn formatting_metadata_and_foreign_builtin_errors_keep_intrinsic_identity() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            for(name,source) in [("foreignFixed","Number.prototype.toFixed"),("foreignExp","Number.prototype.toExponential"),("foreignPrecision","Number.prototype.toPrecision"),("foreignRange","RangeError.prototype"),("foreignType","TypeError.prototype")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,error={};
              for(var method of [foreignFixed,foreignExp,foreignPrecision]){
                try{method.call(1,101);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignRange;}
                try{method.call({},1);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
                try{method.call(1,{valueOf(){throw error;}});valid=false;}catch(e){valid=valid && e===error;}
              }
              for(var name of ['toFixed','toExponential','toPrecision']){
                var d=Object.getOwnPropertyDescriptor(Number.prototype,name),method=d.value;
                valid=valid && d.writable && !d.enumerable && d.configurable && method.length===1 && method.name===name && !Object.hasOwn(method,'prototype');
                try{new method(1);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
              }
              valid;
            "#,backend).unwrap(),JsValue::Bool(true));
        }
    }
}
