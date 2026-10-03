// Portions ported from boa-dev/boa (core/engine/src/builtins/math/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Exact small superaccumulator ported from Gumichocopengin8/xsum.rs
// (src/xsum_small.rs, src/accumulators/small_accumulator.rs, src/constants.rs)
// @ 7668ca84cb00dc8f4a5fc69229d1d317e9735601, MIT.
// Copyright 2015, 2018, 2021, 2024 Radford M. Neal; Copyright 2025 Keita Nonaka.
// Adaptations: private inherent methods, always-small streaming storage (no
// observable iterator length heuristic), i64 negative rounding mask, and the
// missing negative halfway/odd condition in the pinned Rust translation.
// Carry scanning checks its upper bound before indexing; 68 chunks cover the
// full ECMAScript streaming count bound rather than only in-memory vectors.

const MATH_SUM_PRECISE_MAX: u64 = (1u64 << 53) - 1;

fn native_math_sum_precise(i: &mut Interpreter, items: JsValue) -> Result<JsValue, Throw> {
    if matches!(items, JsValue::Undefined | JsValue::Null) {
        return Err(Throw::type_error("Math.sumPrecise input is null or undefined"));
    }
    // GetIterator(sync), rather than Iterator.from's direct-iterator extension.
    let method = i.get_prop(&items, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("Math.sumPrecise input is not iterable"));
    }
    let iterator = i.call_value(&method, items, Vec::new())?;
    if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
    let next = i.get_prop(&iterator, "next")?;
    math_sum_precise_iterate(i, StdIteratorRecord { iterator, next }, 0)
}

fn math_sum_precise_iterate(
    i: &mut Interpreter, record: StdIteratorRecord, mut count: u64,
) -> Result<JsValue, Throw> {
    let mut sum = XsumSmall::new();
    // IteratorStepValue failures leave the iterator done and do not close it.
    while let Some(value) = std_iterator_step(i, &record)? {
        if count >= MATH_SUM_PRECISE_MAX {
            return Err(std_iterator_abrupt_close(i, &record,
                Throw::range_error("Math.sumPrecise exceeded maximum iterations")));
        }
        let JsValue::Num(number) = value else {
            return Err(std_iterator_abrupt_close(i, &record,
                Throw::type_error("Math.sumPrecise requires numbers")));
        };
        // Keep stepping and validating values after NaN or infinity.
        sum.add(number);
        count += 1;
    }
    Ok(JsValue::num(sum.sum()))
}

// CONSTANTS DEFINING THE FLOATING POINT FORMAT
const XSUM_MANTISSA_BITS: i64 = 52; // Bits in fp mantissa, excludes implict 1
const XSUM_EXP_BITS: i64 = 11; // Bits in fp exponent
const XSUM_MANTISSA_MASK: i64 = (1i64 << XSUM_MANTISSA_BITS) - 1; // Mask for mantissa bits
const XSUM_EXP_MASK: i64 = (1 << XSUM_EXP_BITS) - 1; // Mask for exponent
const XSUM_EXP_BIAS: i64 = (1 << (XSUM_EXP_BITS - 1)) - 1; // Bias added to signed exponent
const XSUM_SIGN_BIT: i64 = XSUM_MANTISSA_BITS + XSUM_EXP_BITS; // Position of sign bit
const XSUM_SIGN_MASK: i64 = 1i64 << XSUM_SIGN_BIT; // Mask for sign bit

// CONSTANTS DEFINING THE SMALL ACCUMULATOR FORMAT
const XSUM_SCHUNK_BITS: i64 = 64; // Bits in chunk of the small accumulator
const XSUM_LOW_EXP_BITS: i64 = 5; // # of low bits of exponent, in one chunk
const XSUM_LOW_EXP_MASK: i64 = (1 << XSUM_LOW_EXP_BITS) - 1; // Mask for low-order exponent bits
const XSUM_HIGH_EXP_BITS: i64 = XSUM_EXP_BITS - XSUM_LOW_EXP_BITS; // # of high exponent bits for index
// Finite binary64 inputs are integers in units of 2^-1075, with at most
// 2099 bits. At most 2^53-1 terms need at most 2152 bits: ceil(2152/32)=68.
// Upstream's 67 chunks cover in-memory vectors; the extra chunk covers the
// specification's streaming count bound and cancellation after large carries.
const XSUM_SCHUNKS: i32 = (1 << XSUM_HIGH_EXP_BITS) + 4;
const XSUM_LOW_MANTISSA_BITS: i64 = 1 << XSUM_LOW_EXP_BITS; // Bits in low part of mantissa
const XSUM_LOW_MANTISSA_MASK: i64 = (1i64 << XSUM_LOW_MANTISSA_BITS) - 1; // Mask for low bits
const XSUM_SMALL_CARRY_BITS: i64 = (XSUM_SCHUNK_BITS - 1) - XSUM_MANTISSA_BITS; // Bits sums can carry into
const XSUM_SMALL_CARRY_TERMS: i64 = (1 << XSUM_SMALL_CARRY_BITS) - 1; // # terms can add before need prop.


struct XsumSmall { m_sacc: SmallAccumulator }

impl XsumSmall {
    fn new() -> Self {
        Self {
            m_sacc: SmallAccumulator::new(),
        }
    }

    #[inline(always)]
    fn add(&mut self, value: f64) {
        self.m_sacc.increment_when_value_added(value);
        if self.m_sacc.m_adds_until_propagate == 0 {
            self.m_sacc.carry_propagate();
        }
        self.m_sacc.add1_no_carry(value);
        self.m_sacc.m_adds_until_propagate -= 1;
    }

    fn sum(&mut self) -> f64 {
        // See if we have a NaN from one of the numbers being a NaN, in
        // which case we return the NaN with largest payload, or an infinite
        // result (+Inf, -Inf, or a NaN if both +Inf and -Inf occurred).
        // Note that we do NOT return NaN if we have both an infinite number
        // and a sum of other numbers that overflows with opposite sign,
        // since there is no real ambiguity regarding the sign in such a case.

        if self.m_sacc.m_nan != 0 {
            return f64::from_bits(self.m_sacc.m_nan as u64);
        }

        if self.m_sacc.m_inf != 0 {
            return f64::from_bits(self.m_sacc.m_inf as u64);
        }

        if self.m_sacc.m_size_count == 0 {
            return -0.0;
        }

        // If none of the numbers summed were infinite or NaN, we proceed to
        // propagate carries, as a preliminary to finding the magnitude of
        // the sum.  This also ensures that the sign of the result can be
        // determined from the uppermost non-zero chunk.

        // We also find the index, i, of this uppermost non-zero chunk, as
        // the value returned by carryPropagate, and set ivalue to
        // m_sacc.chunk[i].  Note that ivalue will not be 0 or -1, unless
        // i is 0 (the lowest chunk), in which case it will be handled by
        // the code for denormalized numbers.
        let i: i32 = self.m_sacc.carry_propagate();
        let mut ivalue: i64 = self.m_sacc.m_chunk[i as usize];
        let mut intv: i64;

        // Handle a possible denormalized number, including zero.
        if i <= 1 {
            // Check for zero value, in which case we can return immediately.
            if ivalue == 0 {
                return if !self.m_sacc.m_has_pos_number {
                    -0.0
                } else {
                    0.0
                };
            }

            // Check if it is actually a denormalized number.  It always is if only
            // the lowest chunk is non-zero.  If the highest non-zero chunk is the
            // next-to-lowest, we check the magnitude of the absolute value.
            // Note that the real exponent is 1 (not 0), so we need to shift right
            // by 1 here.
            if i == 0 {
                intv = ivalue.abs();
                intv >>= 1;
                if ivalue < 0 {
                    intv |= XSUM_SIGN_MASK;
                }
                return f64::from_bits(intv as u64);
            } else {
                // Note: Left shift of -ve number is undefined, so do a multiply instead,
                // which is probably optimized to a shift.
                let mut intv: i64 =
                    ivalue * (1i64 << (XSUM_LOW_MANTISSA_BITS - 1)) + (self.m_sacc.m_chunk[0] >> 1);
                if intv < 0 {
                    if intv > -(1i64 << XSUM_MANTISSA_BITS) {
                        intv = (-intv) | XSUM_SIGN_MASK;
                        return f64::from_bits(intv as u64);
                    }
                } else {
                    // non-negative
                    if (intv as u64) < 1u64 << XSUM_MANTISSA_BITS {
                        return f64::from_bits(intv as u64);
                    }
                }
                // otherwise, it's not actually denormalized, so fall through to below
            }
        }

        // Find the location of the uppermost 1 bit in the absolute value of
        // the upper chunk by converting it (as a signed integer) to a
        // floating point value, and looking at the exponent.  Then set
        // 'more' to the number of bits from the lower chunk (and maybe the
        // next lower) that are needed to fill out the mantissa of the
        // result (including the top implicit 1 bit), plus two extra bits to
        // help decide on rounding.  For negative numbers, it may turn out
        // later that we need another bit, because negating a negative value
        // may carry out of the top here, but not carry out of the top once
        // more bits are shifted into the bottom later on.

        let fltv: f64 = ivalue as f64; // finds position of topmost 1 bit of |ivalue|
        intv = fltv.to_bits() as i64;
        let mut e: i32 = ((intv >> XSUM_MANTISSA_BITS) & XSUM_EXP_MASK) as i32; // e-bias is in 0..32
        let mut more: i32 = (2 + XSUM_MANTISSA_BITS + XSUM_EXP_BIAS - e as i64) as i32;

        // Change 'ivalue' to put in 'more' bits from lower chunks into the bottom.
        // Also set 'j' to the index of the lowest chunk from which these bits came,
        // and 'lower' to the remaining bits of that chunk not now in 'ivalue'.
        // Note that 'lower' initially has at least one bit in it, which we can
        // later move into 'ivalue' if it turns out that one more bit is needed.

        ivalue *= 1i64 << more; // multiply, since << of negative undefined
        let mut j: i32 = i - 1;
        let mut lower: i64 = self.m_sacc.m_chunk[j as usize]; // must exist, since denormalized if i==0
        if more >= XSUM_LOW_MANTISSA_BITS as i32 {
            more -= XSUM_LOW_MANTISSA_BITS as i32;
            ivalue += lower << more;
            j -= 1;
            lower = if j < 0 {
                0
            } else {
                self.m_sacc.m_chunk[j as usize]
            };
        }
        ivalue += lower >> (XSUM_LOW_MANTISSA_BITS - more as i64);
        lower &= (1i64 << (XSUM_LOW_MANTISSA_BITS - more as i64)) - 1;

        // Decide on rounding, with separate code for positive and negative values.
        // At this point, 'ivalue' has the signed mantissa bits, plus two extra
        // bits, with 'e' recording the exponent position for these within their
        // top chunk.  For positive 'ivalue', the bits in 'lower' and chunks
        // below 'j' add to the absolute value; for negative 'ivalue' they
        // subtract.
        // After setting 'ivalue' to the tentative unsigned mantissa
        // (shifted left 2), and 'intv' to have the correct sign, this
        // code goes to done_rounding if it finds that just discarding lower
        // order bits is correct, and to round_away_from_zero if instead the
        // magnitude should be increased by one in the lowest mantissa bit.
        let mut should_round_away_from_zero: bool = false;
        if 0 <= ivalue {
            // number is positive, lower bits are added to magnitude
            intv = 0; // positive sign

            if (ivalue & 2) == 0 {
                // extra bits are 0x
                // TODO: this is not required,
                // but removing the branch would change the logic
                should_round_away_from_zero = false;
            } else if (ivalue & 1) != 0 {
                // extra bits are 11
                should_round_away_from_zero = true;
            } else if (ivalue & 4) != 0 {
                // low bit is 1 (odd), extra bits are 10
                should_round_away_from_zero = true;
            } else {
                if lower == 0 {
                    // see if any lower bits are non-zero
                    while j > 0 {
                        j -= 1;
                        if self.m_sacc.m_chunk[j as usize] != 0 {
                            lower = 1;
                            break;
                        }
                    }
                }
                if lower != 0 {
                    // low bit 0 (even), extra bits 10, non-zero lower bits
                    should_round_away_from_zero = true;
                }
            }
        } else {
            // number is negative, lower bits are subtracted from magnitude
            // Check for a negative 'ivalue' that when negated doesn't contain a full
            // mantissa's worth of bits, plus one to help rounding.  If so, move one
            // more bit into 'ivalue' from 'lower' (and remove it from 'lower').
            // This happens when the negation of the upper part of 'ivalue' has the
            // form 10000... but the negation of the full 'ivalue' is not 10000...

            if ((-ivalue) & (1i64 << (XSUM_MANTISSA_BITS + 2))) == 0 {
                let pos: i64 = 1i64 << (XSUM_LOW_MANTISSA_BITS - 1i64 - more as i64);
                ivalue *= 2; // note that left shift undefined if ivalue is negative
                if lower & pos != 0 {
                    ivalue += 1;
                    lower &= !pos;
                }
                e -= 1;
            }

            intv = XSUM_SIGN_MASK; // negative sign
            ivalue = -ivalue; // ivalue now contains the absolute value

            if (ivalue & 3) == 3 {
                // extra bits are 11
                should_round_away_from_zero = true;
            }

            if lower == 0 {
                // see if any lower bits are non-zero
                while j > 0 {
                    j -= 1;
                    if self.m_sacc.m_chunk[j as usize] != 0 {
                        lower = 1;
                        break;
                    }
                }
            }

            if (ivalue & 3) == 2 && (ivalue & 4) != 0 && lower == 0 {
                // low bit 1 (odd), extra bits are 10, lower bits are all 0
                should_round_away_from_zero = true;
            }
        }

        if should_round_away_from_zero {
            // Round away from zero, then check for carry having propagated out the
            // top, and shift if so.
            ivalue += 4; // add 1 to low-order mantissa bit
            if ivalue & (1i64 << (XSUM_MANTISSA_BITS + 3)) != 0 {
                ivalue >>= 1;
                e += 1;
            }
        }

        // Get rid of the bottom 2 bits that were used to decide on rounding.
        ivalue >>= 2;

        // Adjust to the true exponent, accounting for where this chunk is.
        e += (i << XSUM_LOW_EXP_BITS) - XSUM_EXP_BIAS as i32 - XSUM_MANTISSA_BITS as i32;

        // If exponent has overflowed, change to plus or minus Inf and return.
        if e >= XSUM_EXP_MASK as i32 {
            intv |= XSUM_EXP_MASK << XSUM_MANTISSA_BITS;
            return f64::from_bits(intv as u64);
        }

        // Put exponent and mantissa into intv, which already has the sign,
        // then copy into fltv.

        intv += (e as i64) << XSUM_MANTISSA_BITS;
        intv += ivalue & XSUM_MANTISSA_MASK; // mask out the implicit 1 bit
        f64::from_bits(intv as u64)
    }

}

#[cfg_attr(debug_assertions, derive(Debug))]
struct SmallAccumulator {
    m_chunk: Vec<i64>, // Chunks making up small accumulator
    m_adds_until_propagate: i64, // Number of remaining adds before carry
    m_inf: i64,        // If non-zero, +Inf, -Inf, or NaN
    m_nan: i64,        // If non-zero, a NaN value with payload
    m_size_count: usize, // number of added values
    m_has_pos_number: bool, // check if added values have at least one positive number
}

impl SmallAccumulator {
    fn new() -> Self {
        Self {
            m_chunk: vec![0; XSUM_SCHUNKS as usize],
            m_adds_until_propagate: XSUM_SMALL_CARRY_TERMS,
            m_inf: 0,
            m_nan: 0,
            m_size_count: 0,
            m_has_pos_number: false,
        }
    }

    fn carry_propagate(&mut self) -> i32 {
        // Set u to the index of the uppermost non-zero (for now) chunk, or
        // return with value 0 if there is none.
        let mut u: i32 = XSUM_SCHUNKS - 1;
        while 0 <= u && self.m_chunk[u as usize] == 0 {
            if u == 0 {
                self.m_adds_until_propagate = XSUM_SMALL_CARRY_TERMS - 1;
                return 0;
            }
            u -= 1;
        }

        // At this point, m_chunk[u] must be non-zero
        assert!(self.m_chunk[u as usize] != 0, "m_chunk[u] must be non-zero");

        // Carry propagate, starting at the low-order chunks.  Note that the
        // loop limit of u may be increased inside the loop.
        let mut i: i32 = 0; // set to the index of the next non-zero chunck, from bottom
        let mut uix: i32 = -1; // indicates that a non-zero chunk has not been found yet

        loop {
            let mut c: i64; // Set to the chunk at index i (next non-zero one)

            // Find the next non-zero chunk, setting i to its index, or break out
            // of loop if there is none.  Note that the chunk at index u is not
            // necessarily non-zero - it was initially, but u or the chunk at u
            // may have changed.
            loop {
                if i > u { c = 0; break; }
                c = self.m_chunk[i as usize];
                if c != 0 {
                    break;
                }
                i += 1;
                if i > u {
                    break;
                }
            }

            if i > u {
                break;
            }

            let chigh: i64 = c >> XSUM_LOW_MANTISSA_BITS; // High-order bits of c
            if chigh == 0 {
                uix = i;
                i += 1;
                continue; // no need to change this chunk
            }

            if u == i {
                if chigh == -1 {
                    uix = i;
                    break; // don't propagate -1 into the region of all zeros above
                }
                u = i + 1; // we will change chunk[u+1], so we'll need to look at it
            }

            let clow: i64 = c & XSUM_LOW_MANTISSA_MASK; // Low-order bits of c
            if clow != 0 {
                uix = i;
            }

            // We now change chunk[i] and add to chunk[i+1]. Note that i+1 should be
            // in range (no bigger than XSUM_CHUNKS-1) if summing memory, since
            // the number of chunks is big enough to hold any sum, and we do not
            // store redundant chunks with values 0 or -1 above previously non-zero
            // chunks.  But other add operations might cause overflow, in which
            // case we produce a NaN with all 1s as payload.  (We can't reliably produce
            // an Inf of the right sign.)

            self.m_chunk[i as usize] = clow;
            if i + 1 >= XSUM_SCHUNKS {
                self.add_inf_nan((XSUM_EXP_MASK << XSUM_MANTISSA_BITS) | XSUM_MANTISSA_MASK);
                u = i;
            } else {
                self.m_chunk[(i + 1) as usize] += chigh; // note: this could make this chunk be zero
            }

            i += 1;

            if i > u {
                break;
            }
        }

        // Check again for the number being zero, since carry propagation might
        // have created zero from something that initially looked non-zero.
        if uix < 0 {
            uix = 0;
            self.m_adds_until_propagate = XSUM_SMALL_CARRY_TERMS - 1;
            return uix;
        }

        // While the uppermost chunk is negative, with value -1, combine it with
        // the chunk below (if there is one) to produce the same number but with
        // one fewer non-zero chunks.
        while self.m_chunk[uix as usize] == -1 && uix > 0 {
            // Left shift of a negative number is undefined according to the standard,
            // so do a multiply - it's all presumably constant-folded by the compiler.
            self.m_chunk[(uix - 1) as usize] += -((1i64) << XSUM_LOW_MANTISSA_BITS);
            self.m_chunk[uix as usize] = 0;
            uix -= 1;
        }

        self.m_adds_until_propagate = XSUM_SMALL_CARRY_TERMS - 1;
        uix // Return index of uppermost non-zero chunk
    }

    #[cold]
    fn add_inf_nan(&mut self, ivalue: i64) {
        let mantissa: i64 = ivalue & XSUM_MANTISSA_MASK;

        if mantissa == 0 {
            // Inf
            if self.m_inf == 0 {
                // no previous Inf
                self.m_inf = ivalue;
            } else if self.m_inf != ivalue {
                // previous Inf was opposite sign
                let mut fltv: f64 = f64::from_bits(ivalue as u64);
                fltv -= fltv; // result will be a NaN
                self.m_inf = fltv.to_bits() as i64;
            }
        } else {
            // NaN
            // Choose the NaN with the bigger payload and clear its sign.
            // Using <= ensures that we will choose the first NaN over the previous zero.
            if (self.m_nan & XSUM_MANTISSA_MASK) <= mantissa {
                self.m_nan = ivalue & !XSUM_SIGN_MASK;
            }
        }
    }

    fn add1_no_carry(&mut self, value: f64) {
        let ivalue: i64 = value.to_bits() as i64;

        // Extract exponent and mantissa.  Split exponent into high and low parts.
        let exp: i64 = (ivalue >> XSUM_MANTISSA_BITS) & XSUM_EXP_MASK;
        let mut mantissa: i64 = ivalue & XSUM_MANTISSA_MASK;
        let high_exp: usize = (exp >> XSUM_LOW_EXP_BITS) as usize;
        let mut low_exp: i64 = exp & XSUM_LOW_EXP_MASK;

        // Categorize number as normal, denormalized, or Inf/NaN according to
        // the value of the exponent field.
        if exp == 0 {
            // zero or denormalized
            // If it's a zero (positive or negative), we do nothing.
            if mantissa == 0 {
                return;
            }
            // Denormalized mantissa has no implicit 1, but exponent is 1 not 0.
            low_exp = 1;
        } else if exp == XSUM_EXP_MASK {
            // Inf or NaN
            // Just update flags in accumulator structure.
            self.add_inf_nan(ivalue);
            return;
        } else {
            // normalized
            // OR in implicit 1 bit at top of mantissa
            mantissa |= 1i64 << XSUM_MANTISSA_BITS;
        }

        // Separate mantissa into two parts, after shifting, and add to (or
        // subtract from) this chunk and the next higher chunk (which always
        // exists since there are three extra ones at the top).

        // Note that low_mantissa will have at most XSUM_LOW_MANTISSA_BITS bits,
        // while high_mantissa will have at most XSUM_MANTISSA_BITS bits, since
        // even though the high mantissa includes the extra implicit 1 bit, it will
        // also be shifted right by at least one bit.
        let split_mantissa: [i64; 2] = [
            (mantissa << low_exp) & XSUM_LOW_MANTISSA_MASK,
            mantissa >> (XSUM_LOW_MANTISSA_BITS - low_exp),
        ];

        // Add to, or subtract from, the two affected chunks.
        if ivalue < 0 {
            self.m_chunk[high_exp] -= split_mantissa[0];
            self.m_chunk[high_exp + 1] -= split_mantissa[1];
        } else {
            self.m_chunk[high_exp] += split_mantissa[0];
            self.m_chunk[high_exp + 1] += split_mantissa[1];
        }
    }

    #[inline(always)]
    fn increment_when_value_added(&mut self, value: f64) {
        self.m_size_count += 1;
        self.m_has_pos_number = self.m_has_pos_number || value.is_sign_positive();
    }
}

#[cfg(test)]
mod math_sum_precise_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap_or_else(|e| panic!("{backend:?}: {}\n{source}", e.describe())), JsValue::Bool(true), "{backend:?}");
        }
    }
    #[test]
    fn exact_sum_handles_cancellation_nonfinite_and_signed_zero() {
        check(r#"
          var sum=Math.sumPrecise,valid=true;
          for(var values of [[],[-0],[-0,-0]])valid=valid && Object.is(sum(values),-0);
          for(var values of [[0],[-0,0],[1,-1],[-1,1]])valid=valid && Object.is(sum(values),0);
          for(var values of [[NaN],[Infinity,-Infinity],[-Infinity,Infinity],[NaN,Infinity]])valid=valid && Number.isNaN(sum(values));
          valid=valid && sum([Infinity,Number.MAX_VALUE,Number.MAX_VALUE])===Infinity && sum([-Infinity,-Number.MAX_VALUE,-Number.MAX_VALUE])===-Infinity;
          valid=valid && sum([1e16,1,-1e16])===1 && sum([-1e16,-1,1e16])===-1;
          valid=valid && sum([Number.MAX_VALUE,Number.MAX_VALUE,-Number.MAX_VALUE])===Number.MAX_VALUE;
          valid=valid && sum([-Number.MAX_VALUE,-Number.MAX_VALUE,Number.MAX_VALUE])===-Number.MAX_VALUE;
          for(var n of [1,-1,0.1,-0.1,Number.MIN_VALUE,-Number.MIN_VALUE,Number.MAX_VALUE,-Number.MAX_VALUE])valid=valid && Object.is(sum([n]),n);
          valid;
        "#);
    }
    #[test]
    fn exact_rounding_covers_both_signs_ties_subnormal_and_overflow() {
        check(r#"
          var sum=Math.sumPrecise,valid=true,half=2**-53,ulp=2**-52,tiny=Number.MIN_VALUE;
          for(var sign of [1,-1]){
            valid=valid && sum([sign,sign*half])===sign;
            valid=valid && sum([sign,sign*ulp,sign*half])===sign*(1+2*ulp);
            valid=valid && sum([sign,sign*half,sign*tiny])===sign*(1+ulp);
            valid=valid && sum([sign,sign*half,-sign*tiny])===sign;
            valid=valid && sum([sign*tiny,sign*tiny,sign*tiny])===sign*3*tiny;
            valid=valid && sum([sign*Number.MAX_VALUE,sign*2**969])===sign*Number.MAX_VALUE;
            valid=valid && sum([sign*Number.MAX_VALUE,sign*2**970])===sign*Infinity;
            valid=valid && sum([sign*2**-1022,-sign*tiny])===sign*(2**-1022-tiny);
          }
          valid;
        "#);
    }
    #[test]
    fn iterator_protocol_is_streaming_cached_strict_and_never_coerces_values() {
        check(r#"
          var valid=true,log=[],n=0,source={get [Symbol.iterator](){log.push('iterator');return function(){valid=valid && this===source;return iterator;};}},iterator={
            get next(){log.push('next');return function(){valid=valid && this===iterator;log.push('call');return {get done(){log.push('done');return n===3;},get value(){log.push('value');return ++n;}};};},
            get length(){throw 'length must not be read';},return(){throw 'must not close exhausted iterator';}};
          valid=valid && Math.sumPrecise(source)===6 && log.join(',')==='iterator,next,call,done,value,call,done,value,call,done,value,call,done';
          valid;
        "#);
        check(r#"
          var valid=true;
          var uncoerced={valueOf(){throw 'coercion';},toString(){throw 'coercion';}};
          for(var v of [undefined,null,1,true,2n,Symbol(),{next(){return {done:true};}}]){
            try{Math.sumPrecise(v);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }
          for(var v of ['1',false,undefined,null,2n,Symbol(),uncoerced,new Number(1)]){
            try{Math.sumPrecise([v]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }
          valid=valid && Object.is(Math.sumPrecise(''),-0);
          try{Math.sumPrecise('1');valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid;
        "#);
    }
    #[test]
    fn close_preserves_number_error_and_step_failures_do_not_close() {
        check(r#"
          var valid=true,error={},closed=0;
          for(var ret of [function(){throw error;},function(){return 1;},3]){
            var iterator={next(){return {value:'bad',done:false};},get return(){closed++;return ret;}};
            try{Math.sumPrecise({[Symbol.iterator](){return iterator;}});valid=false;}catch(e){valid=valid && e instanceof TypeError && e!==error;}
          }
          valid=valid && closed===3;
          for(var kind of ['next','done','value']){
            var iterator={return(){closed++;return{};},next(){if(kind==='next')throw error;return {get done(){if(kind==='done')throw error;return false;},get value(){throw error;}};}};
            try{Math.sumPrecise({[Symbol.iterator](){return iterator;}});valid=false;}catch(e){valid=valid && e===error;}
          }
          valid=valid && closed===3;
          var n=0,iterator={next(){return {value:n++===0?NaN:'bad'};},return(){closed++;return{};}};
          try{Math.sumPrecise({[Symbol.iterator](){return iterator;}});valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid=valid && n===2 && closed===4;valid;
        "#);
    }
    #[test]
    fn long_stream_propagates_carries_without_input_length_or_storage() {
        check(r#"
          var n=0,valid=true,iterator={next(){if(n===12000)return {done:true};var v=n++%3;return {value:v===0?1e16:v===1?1:-1e16};},get length(){throw 'length';}};
          valid=valid && Math.sumPrecise({[Symbol.iterator](){return iterator;}})===4000 && n===12000;
          var d=Object.getOwnPropertyDescriptor(Math,'sumPrecise');
          valid=valid && d.writable && !d.enumerable && d.configurable && Math.sumPrecise.length===1 && Math.sumPrecise.name==='sumPrecise' && !Object.hasOwn(Math.sumPrecise,'prototype');
          try{new Math.sumPrecise([]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}valid;
        "#);
    }
    #[test]
    fn builtin_errors_use_callee_realm_and_count_guard_closes_after_step() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            for(name,source) in [("foreignSum","Math.sumPrecise"),("foreignTypeError","TypeError.prototype")] {
                let v=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,v).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,error={},closed=0,iterator={next(){return {value:'bad'};},return(){closed++;throw error;}};
              try{foreignSum({[Symbol.iterator](){return iterator;}});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeError;}
              try{foreignSum(null);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeError;}
              valid && closed===1;
            "#,backend).unwrap(),JsValue::Bool(true));
            let iterator=i.run_script_with_backend("var countLog=[];({next(){countLog.push('next');return {get done(){countLog.push('done');return false;},get value(){countLog.push('value');return 'bad';}};},return(){countLog.push('return');throw 'ignored';}})",backend).unwrap();
            let next=i.get_prop(&iterator,"next").unwrap();
            let error=math_sum_precise_iterate(&mut i,StdIteratorRecord{iterator,next},MATH_SUM_PRECISE_MAX).unwrap_err();
            assert!(error.describe().contains("RangeError"),"{}",error.describe());
            assert_eq!(i.run_script("countLog.join(',')==='next,done,value,return'").unwrap(),JsValue::Bool(true));
        }
    }
    #[test]
    fn permitted_stream_capacity_preserves_high_carries_and_cancellation() {
        for sign in [1i64,-1] {
            let mut sum=XsumSmall::new();
            // A 2^2151-unit intermediate has the largest permitted bit position.
            // Seed the equivalent accumulated chunks without 2^53 iterations.
            sum.m_sacc.m_chunk[66]=sign*(1i64<<39);
            sum.m_sacc.m_size_count=1;
            sum.m_sacc.m_has_pos_number=true;
            assert_eq!(sum.sum(),if sign>0 {f64::INFINITY} else {f64::NEG_INFINITY});
            assert_eq!(sum.m_sacc.m_nan,0);
            sum.m_sacc.m_chunk[66]-=sign*(1i64<<39);
            sum.add(1.0);
            assert_eq!(sum.sum(),1.0);
        }
    }
    #[test]
    fn accumulator_matches_exact_singletons_and_integer_sums_across_exponents() {
        // An independent i128 oracle covers signed rounding with bounded exact integers.
        for exponent in -100..=100 {
            for offset in -31..=31 {
                let base=1i128<<60;
                let terms=[base,offset,-base];
                let expected=(terms.iter().sum::<i128>() as f64)*2f64.powi(exponent);
                let mut sum=XsumSmall::new();
                for term in terms { sum.add((term as f64)*2f64.powi(exponent)); }
                assert_eq!(sum.sum().to_bits(),expected.to_bits(),"{exponent}/{offset}");
            }
        }
        let mut state=0x3456789abcdef123u64;
        for exponent in [-900,-512,-50,0,50,512,900] {
            for _ in 0..64 {
                let mut exact=0i128;let mut sum=XsumSmall::new();
                for _ in 0..128 {
                    state=state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let term=((state>>16) as i64)-(1i64<<47);
                    exact+=term as i128;
                    sum.add((term as f64)*2f64.powi(exponent));
                }
                assert_eq!(sum.sum().to_bits(),((exact as f64)*2f64.powi(exponent)).to_bits());
            }
        }
        for exponent in 0u64..2047 {
            for fraction in [0,1,(1u64<<51)-1,1u64<<51,(1u64<<52)-1] {
                for sign in [0,1u64<<63] {
                    let n=f64::from_bits(sign|(exponent<<52)|fraction);
                    let mut sum=XsumSmall::new();sum.add(n);
                    assert_eq!(sum.sum().to_bits(),n.to_bits(),"singleton {:x}",n.to_bits());
                }
            }
        }
    }
}
