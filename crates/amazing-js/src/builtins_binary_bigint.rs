// Signed arbitrary-precision integers for the ECMAScript BigInt primitive.
// Magnitudes use little-endian base-2^32 limbs; zero always has no sign.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryBigInt {
    negative: bool,
    limbs: Vec<u32>,
}

impl BinaryBigInt {
    fn zero() -> Self { Self { negative: false, limbs: Vec::new() } }
    fn one() -> Self { Self { negative: false, limbs: vec![1] } }
    fn new(negative: bool, mut limbs: Vec<u32>) -> Self {
        while limbs.last() == Some(&0) { limbs.pop(); }
        Self { negative: negative && !limbs.is_empty(), limbs }
    }
    fn from_u64(value: u64) -> Self {
        Self::new(false, vec![value as u32, (value >> 32) as u32])
    }
    fn from_i64(value: i64) -> Self {
        Self::new(value < 0, Self::from_u64(value.unsigned_abs()).limbs)
    }
    fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() || value.fract() != 0.0 { return None; }
        if value == 0.0 { return Some(Self::zero()); }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
        let significand = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let shift = exponent - 52;
        let magnitude = if shift >= 0 { Self::from_u64(significand).shl_abs(shift as usize) }
            else { Self::from_u64(significand >> (-shift as u32)) };
        Some(Self::new(value.is_sign_negative(), magnitude.limbs))
    }
    pub(crate) fn is_zero(&self) -> bool { self.limbs.is_empty() }
    fn is_odd(&self) -> bool { self.limbs.first().is_some_and(|limb| limb & 1 != 0) }
    fn abs(&self) -> Self { Self { negative: false, limbs: self.limbs.clone() } }
    fn negated(&self) -> Self { Self::new(!self.negative, self.limbs.clone()) }
    fn bit_len(&self) -> usize {
        self.limbs.last().map_or(0, |last| (self.limbs.len() - 1) * 32 + 32 - last.leading_zeros() as usize)
    }
    fn bit(&self, index: usize) -> bool {
        self.limbs.get(index / 32).is_some_and(|limb| limb & (1u32 << (index % 32)) != 0)
    }
    fn abs_cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.limbs.len().cmp(&other.limbs.len()).then_with(||
            self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.negative, other.negative) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => other.abs_cmp(self),
            _ => self.abs_cmp(other),
        }
    }
    fn add_abs(&self, other: &Self) -> Self {
        let mut out = Vec::with_capacity(self.limbs.len().max(other.limbs.len()) + 1);
        let mut carry = 0u64;
        for index in 0..self.limbs.len().max(other.limbs.len()) {
            let sum = self.limbs.get(index).copied().unwrap_or(0) as u64
                + other.limbs.get(index).copied().unwrap_or(0) as u64 + carry;
            out.push(sum as u32);
            carry = sum >> 32;
        }
        if carry != 0 { out.push(carry as u32); }
        Self::new(false, out)
    }
    fn sub_abs(&self, other: &Self) -> Self {
        debug_assert!(self.abs_cmp(other) != std::cmp::Ordering::Less);
        let mut out = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0u64;
        for index in 0..self.limbs.len() {
            let left = self.limbs[index] as u64;
            let right = other.limbs.get(index).copied().unwrap_or(0) as u64 + borrow;
            out.push(left.wrapping_sub(right) as u32);
            borrow = u64::from(left < right);
        }
        Self::new(false, out)
    }
    fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            let sum = self.add_abs(other);
            Self::new(self.negative, sum.limbs)
        } else {
            match self.abs_cmp(other) {
                std::cmp::Ordering::Less => Self::new(other.negative, other.sub_abs(self).limbs),
                _ => Self::new(self.negative, self.sub_abs(other).limbs),
            }
        }
    }
    fn sub(&self, other: &Self) -> Self { self.add(&other.negated()) }
    fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() { return Self::zero(); }
        let mut out = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, &left) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &right) in other.limbs.iter().enumerate() {
                let index = i + j;
                let product = left as u64 * right as u64 + out[index] as u64 + carry;
                out[index] = product as u32;
                carry = product >> 32;
            }
            out[i + other.limbs.len()] = carry as u32;
        }
        Self::new(self.negative != other.negative, out)
    }
    fn shl_abs(&self, bits: usize) -> Self {
        if self.is_zero() { return Self::zero(); }
        let words = bits / 32;
        let shift = bits % 32;
        let mut out = vec![0u32; words + self.limbs.len() + 1];
        let mut carry = 0u64;
        for (index, &limb) in self.limbs.iter().enumerate() {
            let value = ((limb as u64) << shift) | carry;
            out[words + index] = value as u32;
            carry = value >> 32;
        }
        out[words + self.limbs.len()] = carry as u32;
        Self::new(false, out)
    }
    fn shr_abs(&self, bits: usize) -> Self {
        let words = bits / 32;
        if words >= self.limbs.len() { return Self::zero(); }
        let shift = bits % 32;
        let mut out = vec![0u32; self.limbs.len() - words];
        for index in words..self.limbs.len() {
            let high = if shift != 0 { self.limbs.get(index + 1).copied().unwrap_or(0) as u64 } else { 0 };
            out[index - words] = ((self.limbs[index] as u64 >> shift)
                | (high << (32 - shift) % 32)) as u32;
        }
        Self::new(false, out)
    }
    fn shr_signed(&self, bits: usize) -> Self {
        if !self.negative { return self.shr_abs(bits); }
        let shifted = self.abs().shr_abs(bits);
        let discarded = (0..bits.min(self.bit_len())).any(|bit| self.bit(bit));
        let magnitude = if discarded { shifted.add_abs(&Self::one()) } else { shifted };
        Self::new(true, magnitude.limbs)
    }
    fn div_rem(&self, other: &Self) -> Result<(Self, Self), Throw> {
        if other.is_zero() { return Err(Throw::range_error("BigInt division by zero")); }
        if self.abs_cmp(other) == std::cmp::Ordering::Less {
            return Ok((Self::zero(), self.clone()));
        }
        let divisor = other.abs();
        let mut remainder = Self::zero();
        let mut quotient = vec![0u32; self.bit_len().div_ceil(32)];
        for bit in (0..self.bit_len()).rev() {
            remainder = remainder.shl_abs(1);
            if self.bit(bit) { remainder = remainder.add_abs(&Self::one()); }
            if remainder.abs_cmp(&divisor) != std::cmp::Ordering::Less {
                remainder = remainder.sub_abs(&divisor);
                quotient[bit / 32] |= 1u32 << (bit % 32);
            }
        }
        Ok((Self::new(self.negative != other.negative, quotient),
            Self::new(self.negative, remainder.limbs)))
    }
    fn mul_small(&self, value: u32) -> Self { self.mul(&Self::from_u64(value as u64)) }
    fn add_small(&self, value: u32) -> Self { self.add_abs(&Self::from_u64(value as u64)) }
    fn div_rem_small(&self, divisor: u32) -> (Self, u32) {
        let mut out = vec![0u32; self.limbs.len()];
        let mut remainder = 0u64;
        for index in (0..self.limbs.len()).rev() {
            let value = (remainder << 32) | self.limbs[index] as u64;
            out[index] = (value / divisor as u64) as u32;
            remainder = value % divisor as u64;
        }
        (Self::new(false, out), remainder as u32)
    }
    fn parse_radix(digits: &str, radix: u32) -> Option<Self> {
        if digits.is_empty() { return None; }
        let mut value = Self::zero();
        for character in digits.chars() {
            let digit = character.to_digit(radix)?;
            value = value.mul_small(radix).add_small(digit);
        }
        Some(value)
    }
    fn parse_string(source: &str) -> Option<Self> {
        let trimmed = source.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if trimmed.is_empty() { return Some(Self::zero()); }
        let (negative, unsigned) = if let Some(rest) = trimmed.strip_prefix('-') { (true, rest) }
            else if let Some(rest) = trimmed.strip_prefix('+') { (false, rest) }
            else { (false, trimmed) };
        let (radix, digits) = if !negative && unsigned == trimmed {
            if let Some(rest) = unsigned.strip_prefix("0x").or_else(|| unsigned.strip_prefix("0X")) { (16, rest) }
            else if let Some(rest) = unsigned.strip_prefix("0o").or_else(|| unsigned.strip_prefix("0O")) { (8, rest) }
            else if let Some(rest) = unsigned.strip_prefix("0b").or_else(|| unsigned.strip_prefix("0B")) { (2, rest) }
            else { (10, unsigned) }
        } else { (10, unsigned) };
        let parsed = Self::parse_radix(digits, radix)?;
        Some(Self::new(negative, parsed.limbs))
    }
    pub(crate) fn to_radix_string(&self, radix: u32) -> String {
        if self.is_zero() { return "0".into(); }
        let mut value = self.abs();
        let mut digits = Vec::new();
        while !value.is_zero() {
            let (quotient, remainder) = value.div_rem_small(radix);
            digits.push(char::from_digit(remainder, radix).unwrap());
            value = quotient;
        }
        if self.negative { digits.push('-'); }
        digits.into_iter().rev().collect()
    }
    fn low_u64(&self) -> u64 {
        let low = self.limbs.first().copied().unwrap_or(0) as u64
            | ((self.limbs.get(1).copied().unwrap_or(0) as u64) << 32);
        if self.negative { low.wrapping_neg() } else { low }
    }
    fn twos_complement(&self, width: usize) -> Vec<u32> {
        let mut words = vec![0u32; width];
        words[..self.limbs.len().min(width)].copy_from_slice(&self.limbs[..self.limbs.len().min(width)]);
        if self.negative {
            for word in &mut words { *word = !*word; }
            let mut carry = true;
            for word in &mut words {
                if !carry { break; }
                let (sum, overflow) = word.overflowing_add(1);
                *word = sum;
                carry = overflow;
            }
        }
        words
    }
    fn from_twos_complement(mut words: Vec<u32>) -> Self {
        let negative = words.last().is_some_and(|word| word & 0x8000_0000 != 0);
        if negative {
            for word in &mut words { *word = !*word; }
            let mut carry = true;
            for word in &mut words {
                if !carry { break; }
                let (sum, overflow) = word.overflowing_add(1);
                *word = sum;
                carry = overflow;
            }
        }
        Self::new(negative, words)
    }
    fn bitwise(&self, other: &Self, op: ast::BinOp) -> Self {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let left = self.twos_complement(width);
        let right = other.twos_complement(width);
        let words = left.iter().zip(right.iter()).map(|(a, b)| match op {
            ast::BinOp::BitAnd => a & b,
            ast::BinOp::BitOr => a | b,
            ast::BinOp::BitXor => a ^ b,
            _ => unreachable!(),
        }).collect();
        Self::from_twos_complement(words)
    }
    fn bit_not(&self) -> Self { self.negated().sub(&Self::one()) }
    fn pow(&self, exponent: &Self) -> Result<Self, Throw> {
        if exponent.negative { return Err(Throw::range_error("negative BigInt exponent")); }
        if exponent.is_zero() { return Ok(Self::one()); }
        if self.is_zero() { return Ok(Self::zero()); }
        if self.abs() == Self::one() {
            return Ok(if self.negative && exponent.is_odd() { Self::from_i64(-1) } else { Self::one() });
        }
        let power = exponent.limbs.first().copied().unwrap_or(0) as usize;
        if exponent.limbs.len() > 1 || power.saturating_mul(self.bit_len()) > 1_048_576 {
            return Err(Throw::range_error("BigInt result exceeds implementation limit"));
        }
        let mut result = Self::one();
        let mut base = self.clone();
        let mut remaining = power;
        while remaining != 0 {
            if remaining & 1 != 0 { result = result.mul(&base); }
            remaining >>= 1;
            if remaining != 0 { base = base.mul(&base); }
        }
        Ok(result)
    }
    fn to_usize(&self) -> Option<usize> {
        if self.negative || self.bit_len() > usize::BITS as usize { return None; }
        Some(self.limbs.iter().rev().fold(0usize, |value, limb| (value << 32) | *limb as usize))
    }
    fn as_uint_n(&self, bits: usize) -> Self {
        if bits == 0 { return Self::zero(); }
        let count = bits.div_ceil(32);
        let mut limbs = self.limbs.iter().copied().take(count).collect::<Vec<_>>();
        limbs.resize(count, 0);
        if bits % 32 != 0 { limbs[count - 1] &= (1u32 << (bits % 32)) - 1; }
        let magnitude = Self::new(false, limbs);
        if !self.negative || magnitude.is_zero() { return magnitude; }
        Self::one().shl_abs(bits).sub_abs(&magnitude)
    }
    fn as_int_n(&self, bits: usize) -> Self {
        if bits == 0 { return Self::zero(); }
        let unsigned = self.as_uint_n(bits);
        if !unsigned.bit(bits - 1) { unsigned }
        else { unsigned.sub(&Self::one().shl_abs(bits)) }
    }
    fn to_f64(&self) -> f64 {
        if self.is_zero() { return 0.0; }
        let bits = self.bit_len();
        let shift = bits.saturating_sub(53);
        let top = self.shr_abs(shift).low_u64();
        let round_up = shift != 0 && {
            let halfway = self.bit(shift - 1);
            let lower = (0..shift - 1).any(|bit| self.bit(bit));
            halfway && (lower || top & 1 != 0)
        };
        let magnitude = ((top + u64::from(round_up)) as f64) * 2f64.powi(shift as i32);
        if self.negative { -magnitude } else { magnitude }
    }
}

impl Interpreter {
    fn bigint_value(value: BinaryBigInt) -> JsValue { JsValue::BigInt(Rc::new(value)) }
    fn to_bigint(&mut self, value: &JsValue) -> Result<BinaryBigInt, Throw> {
        let primitive = self.primitive_number_hint(value.clone())?;
        match primitive {
            JsValue::BigInt(value) => Ok((*value).clone()),
            JsValue::Bool(value) => Ok(BinaryBigInt::from_u64(u64::from(value))),
            JsValue::Str(value) => BinaryBigInt::parse_string(&value.to_utf8_lossy())
                .ok_or_else(|| Throw::syntax_error("invalid BigInt string")),
            _ => Err(Throw::type_error("cannot convert value to BigInt")),
        }
    }
    fn install_bigint_builtins(&mut self) {
        let constructor = Self::native("BigInt", bigint_constructor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let prototype = self.bigint_proto.clone();
        binary_configure_constructor(function, "BigInt", 1, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("BigInt")));
        binary_method(self, &prototype, "toString", bigint_to_string, 0);
        binary_method(self, &prototype, "toLocaleString", bigint_to_locale_string, 0);
        binary_method(self, &prototype, "valueOf", bigint_value_of, 0);
        binary_method(self, function, "asIntN", bigint_as_int_n, 2);
        binary_method(self, function, "asUintN", bigint_as_uint_n, 2);
        self.define_global("BigInt", constructor);
    }
    fn bigint_operator(&mut self, op: ast::BinOp, left: JsValue, right: JsValue) -> Result<JsValue, Throw> {
        let (JsValue::BigInt(left), JsValue::BigInt(right)) = (&left, &right) else {
            // ToNumeric of the other operand happens before the mixed-type
            // error, and may invoke a user coercion or throw itself.
            if !matches!(left, JsValue::BigInt(_)) { self.to_number(&left)?; }
            if !matches!(right, JsValue::BigInt(_)) { self.to_number(&right)?; }
            return Err(Throw::type_error("cannot mix BigInt and Number"));
        };
        use ast::BinOp::*;
        let result = match op {
            Add => left.add(right), Sub => left.sub(right), Mul => left.mul(right),
            Div => left.div_rem(right)?.0, Rem => left.div_rem(right)?.1,
            Pow => left.pow(right)?,
            BitAnd | BitOr | BitXor => left.bitwise(right, op),
            Shl | Shr => {
                let count = right.abs().to_usize();
                let shift_left = (op == Shl) != right.negative;
                if shift_left {
                    if left.is_zero() { BinaryBigInt::zero() }
                    else {
                        let bits = count.ok_or_else(|| Throw::range_error("BigInt shift exceeds implementation limit"))?;
                        if bits.saturating_add(left.bit_len()) > 1_048_576 {
                            return Err(Throw::range_error("BigInt shift exceeds implementation limit"));
                        }
                        BinaryBigInt::new(left.negative, left.abs().shl_abs(bits).limbs)
                    }
                } else {
                    let bits = count.unwrap_or(usize::MAX);
                    left.shr_signed(bits)
                }
            }
            UShr => return Err(Throw::type_error("BigInt has no unsigned right shift")),
            _ => unreachable!(),
        };
        Ok(Self::bigint_value(result))
    }
    fn compare_bigint_number(big: &BinaryBigInt, number: f64) -> Option<std::cmp::Ordering> {
        if number.is_nan() { return None; }
        if number == f64::INFINITY { return Some(std::cmp::Ordering::Less); }
        if number == f64::NEG_INFINITY { return Some(std::cmp::Ordering::Greater); }
        let integer = BinaryBigInt::from_f64(number.trunc())?;
        let comparison = big.cmp(&integer);
        if comparison != std::cmp::Ordering::Equal { return Some(comparison); }
        if number.fract() > 0.0 { Some(std::cmp::Ordering::Less) }
        else if number.fract() < 0.0 { Some(std::cmp::Ordering::Greater) }
        else { Some(std::cmp::Ordering::Equal) }
    }
    fn compare_bigint_primitives(&mut self, left: &JsValue, right: &JsValue) -> Result<Option<std::cmp::Ordering>, Throw> {
        match (left, right) {
            (JsValue::BigInt(a), JsValue::BigInt(b)) => Ok(Some(a.cmp(b))),
            (JsValue::BigInt(a), JsValue::Str(b)) => Ok(BinaryBigInt::parse_string(&b.to_utf8_lossy()).map(|b| a.cmp(&b))),
            (JsValue::Str(a), JsValue::BigInt(b)) => Ok(BinaryBigInt::parse_string(&a.to_utf8_lossy()).map(|a| a.cmp(b))),
            (JsValue::BigInt(a), b) => Ok(Self::compare_bigint_number(a, self.to_number(b)?)),
            (a, JsValue::BigInt(b)) => Ok(Self::compare_bigint_number(b, self.to_number(a)?).map(std::cmp::Ordering::reverse)),
            _ => unreachable!(),
        }
    }
}

fn bigint_constructor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let argument = args.first().unwrap_or(&JsValue::Undefined);
    let primitive = i.primitive_number_hint(argument.clone())?;
    let value = match primitive {
        JsValue::Num(number) => BinaryBigInt::from_f64(number)
            .ok_or_else(|| Throw::range_error("BigInt number must be an integer"))?,
        other => i.to_bigint(&other)?,
    };
    Ok(Interpreter::bigint_value(value))
}
fn bigint_this(this: &JsValue) -> Result<Rc<BinaryBigInt>, Throw> {
    match this {
        JsValue::BigInt(value) => Ok(value.clone()),
        JsValue::Object(object) => match object.borrow().get_own("[[BigIntData]]") {
            Some(JsValue::BigInt(value)) => Ok(value),
            _ => Err(Throw::type_error("BigInt method called on incompatible receiver")),
        },
        _ => Err(Throw::type_error("BigInt method called on incompatible receiver")),
    }
}
fn bigint_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::BigInt(bigint_this(&this)?))
}
fn bigint_to_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = bigint_this(&this)?;
    let radix = if let Some(arg) = args.first().filter(|v| !v.is_undefined()) {
        i.to_number(arg)?
    } else { 10.0 };
    if !radix.is_finite() || !(2.0..=36.0).contains(&radix.trunc()) {
        return Err(Throw::range_error("invalid BigInt radix"));
    }
    Ok(JsValue::str(value.to_radix_string(radix.trunc() as u32)))
}
fn bigint_to_locale_string(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str(bigint_this(&this)?.to_radix_string(10)))
}
fn bigint_width(i: &mut Interpreter, args: &[JsValue], signed: bool) -> Result<JsValue, Throw> {
    let bits = binary_index(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let value = i.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))?;
    if bits > 1_048_576 { return Err(Throw::range_error("BigInt width exceeds implementation limit")); }
    Ok(Interpreter::bigint_value(if signed { value.as_int_n(bits) } else { value.as_uint_n(bits) }))
}
fn bigint_as_int_n(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    bigint_width(i, &args, true)
}
fn bigint_as_uint_n(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    bigint_width(i, &args, false)
}

#[cfg(test)]
mod bigint_limb_tests {
    use super::BinaryBigInt as Big;

    #[test]
    fn wide_arithmetic_and_radix() {
        let a = Big::parse_radix("100000000000000000000000000000001", 16).unwrap();
        let b = Big::from_i64(13);
        assert_eq!(a.sub(&Big::one()).to_radix_string(16), "100000000000000000000000000000000");
        assert_eq!(a.add(&b).sub(&b), a);
        assert_eq!(a.mul(&b).div_rem(&b).unwrap(), (a, Big::zero()));
    }
    #[test]
    fn signed_division_and_width() {
        let minus_five = Big::from_i64(-5);
        assert_eq!(minus_five.div_rem(&Big::from_i64(2)).unwrap(),
            (Big::from_i64(-2), Big::from_i64(-1)));
        assert_eq!(minus_five.shr_signed(1), Big::from_i64(-3));
        assert_eq!(minus_five.as_uint_n(8), Big::from_i64(251));
        assert_eq!(Big::from_i64(251).as_int_n(8), minus_five);
    }
}
