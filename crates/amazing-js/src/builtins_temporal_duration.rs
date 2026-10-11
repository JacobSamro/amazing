// Temporal.Duration with exact (wide) field storage.
//
// A Duration Record's ten fields are float64 integers, and IsValidDuration
// bounds only the exact *total* of days..nanoseconds (below 2**53 seconds),
// never the individual time fields: 9e21 microseconds or 9e24 nanoseconds are
// legal. The shared `[i64; 10]` calendar fields cannot hold those, so a
// Duration keeps its true values in `StdTemporalData::wide` (`StdDuration`,
// ten i128 fields) and a *fitted* `[i64; 10]` copy in `fields` for the
// calendar/relativeTo helpers. The fitted copy has identical totals; it only
// differs from the wide record when hours..nanoseconds do not fit an i64, in
// which case that sub-day span is rebalanced from hours downward.
//
// Structure (exact time-duration total as a wide integer, separate calendar
// part, validity checked on the exact total) follows boa-dev/temporal
// (temporal_rs) @ eadf8039d2577e4375493a0eb3f46a66f25616ec,
// src/builtins/core/duration/{normalized.rs,mod.rs}, Apache-2.0
// (NormalizedTimeDuration / InternalDurationRecord / is_valid_duration),
// rewritten onto this engine's own types. See THIRD_PARTY_NOTICES.md.

type StdDuration = [i128; 10];

const STD_TEMPORAL_DURATION_FIELD_NAMES: [(usize, &str); 10] = [
    (3, "days"), (4, "hours"), (8, "microseconds"), (7, "milliseconds"),
    (5, "minutes"), (1, "months"), (9, "nanoseconds"), (6, "seconds"),
    (2, "weeks"), (0, "years"),
];

// 2**53 seconds minus one nanosecond: the exclusive bound on the exact total.
const STD_TEMPORAL_DURATION_MAX_NS: i128 = 9_007_199_254_740_991 * 1_000_000_000 + 999_999_999;

fn std_temporal_duration_widen(fields: &[i64; 10]) -> StdDuration {
    fields.map(|value| value as i128)
}

// Exact ns total of hours..nanoseconds only.
fn std_temporal_duration_clock_ns(w: &StdDuration) -> i128 {
    w[4].saturating_mul(3_600 * STD_TEMPORAL_NS_SECOND)
        .saturating_add(w[5].saturating_mul(60 * STD_TEMPORAL_NS_SECOND))
        .saturating_add(w[6].saturating_mul(STD_TEMPORAL_NS_SECOND))
        .saturating_add(w[7].saturating_mul(1_000_000))
        .saturating_add(w[8].saturating_mul(1_000))
        .saturating_add(w[9])
}

// Exact ns total of days..nanoseconds (the IsValidDuration quantity).
fn std_temporal_duration_span_ns(w: &StdDuration) -> i128 {
    w[3].saturating_mul(STD_TEMPORAL_NS_DAY).saturating_add(std_temporal_duration_clock_ns(w))
}

// Total ns of weeks..nanoseconds, weeks counted as 7 days (compare, add).
fn std_temporal_duration_total_ns(w: &StdDuration) -> i128 {
    w[2].saturating_mul(7 * STD_TEMPORAL_NS_DAY).saturating_add(std_temporal_duration_span_ns(w))
}

fn std_temporal_duration_fit(w: &StdDuration) -> [i64; 10] {
    let narrow = |value: i128| value.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    let mut fields = w.map(narrow);
    let fits = |value: i128| i64::try_from(value).is_ok();
    if !w[4..10].iter().all(|value| fits(*value)) {
        let split = std_temporal_split_duration_ns_wide(std_temporal_duration_clock_ns(w), 4);
        for index in 4..10 { fields[index] = narrow(split[index]); }
    }
    fields
}

fn std_temporal_validate_duration(w: &StdDuration) -> Result<(), Throw> {
    let sign = w.iter().find(|value| **value != 0).map(|value| value.signum()).unwrap_or(0);
    if w.iter().any(|value| *value != 0 && value.signum() != sign)
        || w[..3].iter().any(|value| value.unsigned_abs() > 4_294_967_295)
        || std_temporal_duration_span_ns(w).unsigned_abs() > STD_TEMPORAL_DURATION_MAX_NS as u128 {
        return Err(Throw::range_error("invalid Temporal duration"));
    }
    Ok(())
}

// Splits a signed ns total into fields `largest..=9` (2 = weeks, 3 = days).
fn std_temporal_split_duration_ns_wide(total: i128, largest: usize) -> StdDuration {
    let mut fields = [0i128; 10];
    let sign = total.signum();
    let mut remaining = total.unsigned_abs() as i128;
    for (index, unit) in [(2, 7 * STD_TEMPORAL_NS_DAY), (3, STD_TEMPORAL_NS_DAY), (4, 3_600 * STD_TEMPORAL_NS_SECOND),
        (5, 60 * STD_TEMPORAL_NS_SECOND), (6, STD_TEMPORAL_NS_SECOND),
        (7, 1_000_000), (8, 1_000), (9, 1)] {
        if index == 2 && largest != 2 { continue; }
        if index < largest { continue; }
        fields[index] = remaining / unit * sign;
        remaining %= unit;
    }
    fields
}

fn std_temporal_split_duration_ns(total: i128, largest: usize) -> [i64; 10] {
    let narrow = |value: i128| value.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    std_temporal_split_duration_ns_wide(total, largest).map(narrow)
}

// CreateTemporalDuration: every field is canonicalized to the float64 it
// would have been in the spec (a later add()/toString()/compare must see the
// already-lossy value, never a hidden exact one), then validated.
fn std_temporal_make_duration(i: &Interpreter, mut w: StdDuration) -> Result<JsValue, Throw> {
    for field in w.iter_mut() { *field = (*field as f64) as i128; }
    std_temporal_validate_duration(&w)?;
    let prototype = STD_TEMPORAL_PROTOS.with(|registry| registry.borrow().get(&std_weak_id(&i.global_obj))
        .filter(|entry| std_weak_same(&entry.owner, &i.global_obj))
        .and_then(|entry| entry.entries.iter().find(|(candidate, _)| *candidate == StdTemporalKind::Duration))
        .map(|(_, prototype)| prototype.clone()))
        .ok_or_else(|| Throw::type_error("Temporal realm is unavailable"))?;
    let mut object = JsObject::ordinary();
    object.proto = Some(prototype);
    let object = Obj(Rc::new(RefCell::new(object)));
    STD_TEMPORAL_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(&object),
        StdTemporalData { owner: Rc::downgrade(&object.0), kind: StdTemporalKind::Duration,
            fields: std_temporal_duration_fit(&w), calendar: "iso8601".to_string(), wide: w }));
    Ok(JsValue::Object(object))
}

// ToIntegerIfIntegral: any finite integral Number. There is deliberately no
// per-field magnitude cap here; IsValidDuration bounds the exact total.
fn std_temporal_duration_number_wide(i: &mut Interpreter, value: JsValue, default: Option<i128>) -> Result<i128, Throw> {
    if value.is_undefined() {
        if let Some(default) = default { return Ok(default); }
    }
    let number = i.to_number(&value)?;
    if !number.is_finite() || number.fract() != 0.0 {
        return Err(Throw::range_error("duration field must be an integer"));
    }
    Ok(number as i128)
}

fn std_temporal_duration_bag_wide(i: &mut Interpreter, value: &JsValue) -> Result<StdDuration, Throw> {
    let mut w = [0i128; 10];
    let mut present = false;
    // ToTemporalDuration reads all ten properties in alphabetical order.
    for (index, name) in STD_TEMPORAL_DURATION_FIELD_NAMES {
        let property = i.get_prop(value, name)?;
        if !property.is_undefined() {
            w[index] = std_temporal_duration_number_wide(i, property, None)?;
            present = true;
        }
    }
    if !present { return Err(Throw::type_error("duration-like object has no fields")); }
    std_temporal_validate_duration(&w)?;
    Ok(w)
}

// ToTemporalDuration (wide).
fn std_temporal_to_duration_wide(i: &mut Interpreter, value: JsValue) -> Result<StdDuration, Throw> {
    if let Ok(data) = std_temporal_record(&value, Some(StdTemporalKind::Duration)) { return Ok(data.wide); }
    match &value {
        JsValue::Str(text) => {
            let w = std_temporal_parse_duration_wide(&text.to_utf8_lossy())?;
            std_temporal_validate_duration(&w)?;
            Ok(w)
        }
        JsValue::Object(_) => std_temporal_duration_bag_wide(i, &value),
        _ => Err(Throw::type_error("invalid Temporal input")),
    }
}

// Duration.prototype.with: the alphabetical reads of the ten properties, each
// replacing the receiver's exact field; validity is checked on the result.
fn std_temporal_duration_with(i: &mut Interpreter, data: &StdTemporalData, bag: &JsValue) -> Result<JsValue, Throw> {
    let mut w = data.wide;
    let mut present = false;
    for (index, name) in STD_TEMPORAL_DURATION_FIELD_NAMES {
        let property = i.get_prop(bag, name)?;
        if property.is_undefined() { continue; }
        present = true;
        w[index] = std_temporal_duration_number_wide(i, property, None)?;
    }
    if !present { return Err(Throw::type_error("no Temporal fields provided")); }
    std_temporal_make_duration(i, w)
}

// AddDurations without relativeTo: calendar units on either side throw; the
// day..nanosecond total is exact and rebalanced to the coarser of the two
// operands' largest units (never coarser than days).
fn std_temporal_duration_add_wide(i: &mut Interpreter, data: StdTemporalData, other: JsValue, factor: i64) -> Result<JsValue, Throw> {
    let other = std_temporal_to_duration_wide(i, other)?;
    let this = data.wide;
    if this[..3].iter().chain(other[..3].iter()).any(|value| *value != 0) {
        return Err(Throw::range_error("calendar units require relativeTo"));
    }
    let largest_rank = |w: &StdDuration| (3..10).find(|&index| w[index] != 0).unwrap_or(9);
    let rank = largest_rank(&this).min(largest_rank(&other)).max(3);
    let total = std_temporal_duration_span_ns(&this)
        .saturating_add(std_temporal_duration_span_ns(&other).saturating_mul(factor as i128));
    std_temporal_make_duration(i, std_temporal_split_duration_ns_wide(total, rank))
}

// The float64 nearest the exact quotient n / d (round-half-even), for
// Duration.prototype.total. `n as f64 / d as f64` rounds twice once |n|
// exceeds 2**53; this keeps every quotient bit and a sticky remainder.
fn std_temporal_exact_ratio(n: i128, d: i128) -> f64 {
    if d == 0 { return f64::NAN; }
    if n == 0 { return 0.0; }
    let negative = (n < 0) != (d < 0);
    let (a, b) = (n.unsigned_abs(), d.unsigned_abs());
    let bits = |value: u128| 128 - value.leading_zeros() as i32;
    let q0 = a / b;
    let shift = if q0 == 0 { 60 + (bits(b) - bits(a)).max(0) } else { (60 - bits(q0)).max(0) };
    let shift = shift.min(127 - bits(a).max(1)).max(0) as u32;
    let scaled = a << shift;
    let mut quotient = scaled / b;
    if scaled % b != 0 { quotient |= 1; }
    let mut value = quotient as f64;
    // Exact power-of-two scaling, split so 2**shift never overflows a double.
    let mut remaining = shift;
    while remaining > 0 {
        let step = remaining.min(60);
        value /= (1u64 << step) as f64;
        remaining -= step;
    }
    if negative { -value } else { value }
}

#[cfg(test)]
mod duration_exact_tests {
    use super::*;

    #[test]
    fn exact_ratio_rounds_once() {
        assert_eq!(std_temporal_exact_ratio(1, 3), 1.0 / 3.0);
        assert_eq!(std_temporal_exact_ratio(-7, 2), -3.5);
        assert_eq!(std_temporal_exact_ratio(0, 5), 0.0);
        // 9007199254740993 ns in seconds is not representable; the nearest
        // double to 9007199254740.993 must come out, not a double-rounded one.
        assert_eq!(std_temporal_exact_ratio(9_007_199_254_740_993, 1_000), 9_007_199_254_740.993);
        let big = 9_007_199_254_740_991_463_129_088i128;
        assert_eq!(std_temporal_exact_ratio(big, 1), big as f64);
    }

    #[test]
    fn fit_preserves_totals_and_validation_uses_exact_total() {
        let mut w = [0i128; 10];
        w[8] = 4_503_599_627_370_494_951_424; // microseconds beyond i64
        w[7] = 4_503_599_627_370_497_024; // milliseconds
        assert!(std_temporal_validate_duration(&w).is_ok());
        let fitted = std_temporal_duration_fit(&w);
        assert_eq!(std_temporal_duration_time_ns(&fitted), std_temporal_duration_total_ns(&w));
        w[3] = 1_000_000_000_000;
        assert!(std_temporal_validate_duration(&w).is_err());
        let mut m = [0i128; 10];
        m[9] = i128::MIN;
        assert!(std_temporal_validate_duration(&m).is_err());
    }
}
