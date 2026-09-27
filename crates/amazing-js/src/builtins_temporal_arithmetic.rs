// ISO arithmetic shared by the plain Temporal types. Epoch nanoseconds stay
// i128 throughout; no f64 conversion occurs until Duration.total returns a
// Number, as required by its JavaScript API.
//
// Portions of the since/until difference algorithm and its rounding-mode
// handling are ported from boa-dev/temporal (temporal_rs)
// @ eadf8039d2577e4375493a0eb3f46a66f25616ec, src/builtins/core/plain_date.rs
// and src/rounding.rs, Apache-2.0. See THIRD_PARTY_NOTICES.md.

const STD_TEMPORAL_NS_SECOND: i128 = 1_000_000_000;
const STD_TEMPORAL_NS_DAY: i128 = 86_400 * STD_TEMPORAL_NS_SECOND;

fn std_temporal_time_ns(fields: &[i64; 10]) -> i128 {
    fields[3] as i128 * 3_600 * STD_TEMPORAL_NS_SECOND
        + fields[4] as i128 * 60 * STD_TEMPORAL_NS_SECOND
        + fields[5] as i128 * STD_TEMPORAL_NS_SECOND
        + fields[6] as i128 * 1_000_000
        + fields[7] as i128 * 1_000
        + fields[8] as i128
}

fn std_temporal_set_time_ns(fields: &mut [i64; 10], nanos: i128) {
    let mut rest = nanos.rem_euclid(STD_TEMPORAL_NS_DAY);
    fields[3] = (rest / (3_600 * STD_TEMPORAL_NS_SECOND)) as i64;
    rest %= 3_600 * STD_TEMPORAL_NS_SECOND;
    fields[4] = (rest / (60 * STD_TEMPORAL_NS_SECOND)) as i64;
    rest %= 60 * STD_TEMPORAL_NS_SECOND;
    fields[5] = (rest / STD_TEMPORAL_NS_SECOND) as i64;
    rest %= STD_TEMPORAL_NS_SECOND;
    fields[6] = (rest / 1_000_000) as i64;
    fields[7] = ((rest / 1_000) % 1_000) as i64;
    fields[8] = (rest % 1_000) as i64;
}

fn std_temporal_duration_time_ns(fields: &[i64; 10]) -> i128 {
    ((fields[2] as i128 * 7 + fields[3] as i128) * 86_400
        + fields[4] as i128 * 3_600 + fields[5] as i128 * 60
        + fields[6] as i128) * STD_TEMPORAL_NS_SECOND
        + fields[7] as i128 * 1_000_000 + fields[8] as i128 * 1_000
        + fields[9] as i128
}

fn std_temporal_epoch_ns(fields: &[i64; 10]) -> i128 {
    std_temporal_epoch_day(fields[0], fields[1], fields[2]) as i128 * STD_TEMPORAL_NS_DAY
        + std_temporal_time_ns(fields)
}

fn std_temporal_set_epoch_ns(fields: &mut [i64; 10], epoch_ns: i128) -> Result<(), Throw> {
    let day = epoch_ns.div_euclid(STD_TEMPORAL_NS_DAY);
    let day = i64::try_from(day).map_err(|_| Throw::range_error("Temporal date outside valid range"))?;
    let (year, month, date) = std_temporal_civil_from_epoch_day(day);
    fields[0] = year; fields[1] = month; fields[2] = date;
    std_temporal_set_time_ns(fields, epoch_ns);
    Ok(())
}

fn std_temporal_overflow(i: &mut Interpreter, option: JsValue) -> Result<bool, Throw> {
    if option.is_undefined() { return Ok(false); }
    if !option.is_object() { return Err(Throw::type_error("Temporal options must be an object")); }
    let value = i.get_prop(&option, "overflow")?;
    if value.is_undefined() { return Ok(false); }
    match i.to_string(&value)?.as_str() {
        "constrain" => Ok(false), "reject" => Ok(true),
        _ => Err(Throw::range_error("invalid Temporal overflow option")),
    }
}

// ToRelativeTemporalObject (spec): resolves Duration.prototype.{compare,
// round,total}'s `relativeTo` into a plain calendar date. A value naming a
// time zone (a ZonedDateTime instance/record, a zone-annotated string, or a
// property bag with a `timeZone` property) first resolves as a full
// ZonedDateTime -- reusing std_epoch_from_value_with_options's validation
// and range checks (the offset-field type gate, disambiguation/overflow
// enum checks, the +-8.64e21ns instant bound) -- then keeps only its local
// date. Anything else resolves as a PlainDateTime bag/string (so a discarded
// time-of-day is still read and range-checked, e.g. an Infinity hour, and a
// bag missing year/month/day still throws) and keeps only the date portion.
// round/total/compare each used to have their own partial, ad-hoc version of
// this that duplicated none of that validation -- see
// docs/lanes/js-stdlib_TODO.md, JSS-temporal-port item 2.
fn std_temporal_resolve_relative_to(i: &mut Interpreter, relative: &JsValue) -> Result<Option<[i64; 10]>, Throw> {
    if relative.is_undefined() { return Ok(None); }
    if let Ok(data) = std_temporal_record(relative, None) {
        let mut fields = data.fields;
        return match data.kind {
            StdTemporalKind::PlainDate => Ok(Some(fields)),
            StdTemporalKind::PlainDateTime => { fields[3..9].fill(0); Ok(Some(fields)) }
            _ => Err(Throw::type_error("relativeTo must name a PlainDate, PlainDateTime, or ZonedDateTime")),
        };
    }
    if let Ok(data) = std_epoch_record(relative, Some(StdEpochKind::ZonedDateTime)) {
        let mut fields = std_epoch_local_fields(&data)?;
        fields[3..9].fill(0);
        return Ok(Some(fields));
    }
    match relative {
        JsValue::Str(text) => {
            let text = text.to_utf8_lossy();
            let (_, zone_annotation) = std_epoch_validate_annotations(&text)?;
            let mut fields = if zone_annotation.is_some() {
                let (epoch_ns, zone) = std_epoch_parse(&text, StdEpochKind::ZonedDateTime, "reject", "compatible")?;
                let mut f = [0i64; 10];
                std_temporal_set_epoch_ns(&mut f, epoch_ns + tz_offset_ns(&zone, epoch_ns))?;
                f
            } else {
                // No zone annotation: ToRelativeTemporalObject discards any
                // time-of-day the string carries and resolves as a PlainDate
                // (ParseISODateTime still validates a present time's syntax
                // and ranges, it is just not kept). Parsing as PlainDateTime
                // here would wrongly *require* a time-of-day component --
                // confirmed by relativeto-hour.js and largestunit-correct-
                // rebalancing.js, which pass bare date strings like
                // "2023-02-21".
                std_temporal_parse(StdTemporalKind::PlainDate, &text)?
            };
            fields[3..9].fill(0);
            Ok(Some(fields))
        }
        JsValue::Object(_) => {
            let time_zone = i.get_prop(relative, "timeZone")?;
            let mut fields = if !time_zone.is_undefined() {
                let (epoch_ns, zone) = std_epoch_from_value_with_options(i, relative.clone(), StdEpochKind::ZonedDateTime, JsValue::Undefined)?;
                let mut f = [0i64; 10];
                std_temporal_set_epoch_ns(&mut f, epoch_ns + tz_offset_ns(&zone, epoch_ns))?;
                f
            } else {
                let mut f = std_temporal_bag(i, StdTemporalKind::PlainDateTime, relative)?;
                std_temporal_regulate(StdTemporalKind::PlainDateTime, &mut f, false);
                std_temporal_validate(StdTemporalKind::PlainDateTime, &f)?;
                f
            };
            fields[3..9].fill(0);
            Ok(Some(fields))
        }
        _ => Err(Throw::type_error("relativeTo must be a Temporal object, string, or property bag")),
    }
}

fn std_temporal_option_string(i: &mut Interpreter, option: &JsValue, key: &str) -> Result<Option<String>, Throw> {
    if option.is_undefined() { return Ok(None); }
    if !option.is_object() { return Err(Throw::type_error("Temporal options must be an object")); }
    let value = i.get_prop(option, key)?;
    if value.is_undefined() { Ok(None) } else { i.to_string(&value).map(Some) }
}

fn std_temporal_option_value(i: &mut Interpreter, option: &JsValue, key: &str) -> Result<JsValue, Throw> {
    if option.is_undefined() { return Ok(JsValue::Undefined); }
    if !option.is_object() { return Err(Throw::type_error("Temporal options must be an object")); }
    i.get_prop(option, key)
}

fn std_temporal_add_calendar(base: &[i64; 10], duration: &[i64; 10], factor: i64,
    reject_overflow: bool) -> Result<[i64; 10], Throw> {
    let mut result = *base;
    let months = base[0] as i128 * 12 + base[1] as i128 - 1
        + factor as i128 * (duration[0] as i128 * 12 + duration[1] as i128);
    let year = i64::try_from(months.div_euclid(12)).map_err(|_| Throw::range_error("Temporal year outside valid range"))?;
    let month = (months.rem_euclid(12) + 1) as i64;
    let max_day = std_temporal_days_in_month(year, month);
    if result[2] > max_day {
        if reject_overflow { return Err(Throw::range_error("day is invalid in target month")); }
        result[2] = max_day;
    }
    result[0] = year; result[1] = month;
    let days = factor as i128 * (duration[2] as i128 * 7 + duration[3] as i128);
    let start = std_temporal_epoch_day(result[0], result[1], result[2]) as i128;
    let day = i64::try_from(start + days).map_err(|_| Throw::range_error("Temporal date outside valid range"))?;
    (result[0], result[1], result[2]) = std_temporal_civil_from_epoch_day(day);
    Ok(result)
}

fn std_temporal_add_values(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, factor: i64) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let duration = std_temporal_from_fields(i, StdTemporalKind::Duration, std_reflect_arg(&args, 0), false)?;
    let mut result = data.fields;
    match data.kind {
        StdTemporalKind::Duration => {
            // AddDurations (spec): a calendar unit (year/month/week) on
            // EITHER side always throws unconditionally -- add/subtract
            // never take a relativeTo, so there is no calendar to
            // redistribute those units through, regardless of whether the
            // resulting sum would itself be representable. Confirmed by
            // no-calendar-units.js.
            if result[0] != 0 || result[1] != 0 || result[2] != 0
                || duration[0] != 0 || duration[1] != 0 || duration[2] != 0 {
                return Err(Throw::range_error("calendar units require relativeTo"));
            }
            // The day-through-nanosecond result is always fully rebalanced
            // (BalanceTimeDuration), never left as a raw per-field sum --
            // confirmed by basic.js's "balancing positive" case (ms/us
            // overflow must carry into seconds even though the raw sum is
            // itself in safe-integer range) and balance-negative-result.js.
            // The largest unit that survives is the coarser (smaller rank)
            // of the two operands' own largest nonzero unit, but never
            // coarser than "day" (both are already known calendar-unit-free
            // above); a blank operand contributes nothing and never
            // coarsens the other side.
            let largest_rank = |f: &[i64; 10]| (3..10).find(|&idx| f[idx] != 0).unwrap_or(9);
            let balance_rank = largest_rank(&result).min(largest_rank(&duration)).max(3);
            let total_ns = std_temporal_duration_time_ns(&result)
                + factor as i128 * std_temporal_duration_time_ns(&duration);
            result = std_temporal_split_duration_ns(total_ns, balance_rank);
            std_temporal_validate(StdTemporalKind::Duration, &result)?;
        }
        StdTemporalKind::PlainTime => {
            if duration[0..4].iter().any(|value| *value != 0) {
                return Err(Throw::range_error("date units are not valid for PlainTime"));
            }
            let total = std_temporal_time_ns(&result) + factor as i128 * std_temporal_duration_time_ns(&duration);
            std_temporal_set_time_ns(&mut result, total);
        }
        StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime | StdTemporalKind::PlainYearMonth => {
            let reject = std_temporal_overflow(i, std_reflect_arg(&args, 1))?;
            result = std_temporal_add_calendar(&result, &duration, factor, reject)?;
            if data.kind == StdTemporalKind::PlainDateTime {
                let time_delta = factor as i128 * (duration[4] as i128 * 3_600_000_000_000
                    + duration[5] as i128 * 60_000_000_000
                    + duration[6] as i128 * 1_000_000_000
                    + duration[7] as i128 * 1_000_000
                    + duration[8] as i128 * 1_000 + duration[9] as i128);
                let epoch_ns = std_temporal_epoch_ns(&result) + time_delta;
                std_temporal_set_epoch_ns(&mut result, epoch_ns)?;
            } else if data.kind == StdTemporalKind::PlainDate {
                let time_delta = factor as i128 * (duration[4] as i128 * 3_600_000_000_000
                    + duration[5] as i128 * 60_000_000_000
                    + duration[6] as i128 * 1_000_000_000
                    + duration[7] as i128 * 1_000_000
                    + duration[8] as i128 * 1_000 + duration[9] as i128);
                let shifted = std_temporal_epoch_ns(&result) + time_delta;
                let day = i64::try_from(shifted.div_euclid(STD_TEMPORAL_NS_DAY))
                    .map_err(|_| Throw::range_error("Temporal date outside valid range"))?;
                (result[0], result[1], result[2]) = std_temporal_civil_from_epoch_day(day);
            } else { result[2] = data.fields[2]; }
        }
        StdTemporalKind::PlainMonthDay => return Err(Throw::type_error("PlainMonthDay does not support addition")),
    }
    std_temporal_make(i, data.kind, result)
}

fn std_temporal_add(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_add_values(i, this, args, 1)
}
fn std_temporal_subtract(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_add_values(i, this, args, -1)
}

fn std_temporal_split_duration_ns(total: i128, largest: usize) -> [i64; 10] {
    let mut fields = [0; 10];
    let sign = total.signum();
    let mut remaining = total.abs();
    for (index, unit) in [(2, 7 * STD_TEMPORAL_NS_DAY), (3, STD_TEMPORAL_NS_DAY), (4, 3_600 * STD_TEMPORAL_NS_SECOND),
        (5, 60 * STD_TEMPORAL_NS_SECOND), (6, STD_TEMPORAL_NS_SECOND),
        (7, 1_000_000), (8, 1_000), (9, 1)] {
        if index == 2 && largest != 2 { continue; }
        if index < largest { continue; }
        fields[index] = (remaining / unit * sign) as i64;
        remaining %= unit;
    }
    fields
}

// GetDifferenceSettings (spec): the four option properties are read and cast
// in this exact order -- largestUnit, roundingIncrement, roundingMode,
// smallestUnit -- and every one of the twelve Get/toString/valueOf calls
// they cause completes before ANY algorithmic validation (is this unit
// allowed for this operation, largestUnit >= smallestUnit, the
// roundingIncrement bound implied by the now-known smallestUnit) is allowed
// to throw. `since`/`until` used to read smallestUnit/roundingIncrement/
// roundingMode first and largestUnit only afterward, deep inside each
// per-kind branch -- confirmed wrong by PlainDate/PlainDateTime's
// `options-read-before-algorithmic-validation.js` and
// `order-of-operations.js`, which observe the four reads via a Proxy in this
// order and require all of them to complete even on a call that is going to
// throw. This function does the four raw reads only; every caller resolves
// its own per-kind defaults, disallowed-unit set and increment bound
// afterward, all deferred past this point. Ordering ported from
// boa-dev/temporal (temporal_rs) @ eadf8039d2577e4375493a0eb3f46a66f25616ec,
// src/options.rs's `DifferenceSettings` parsing, Apache-2.0.
fn std_temporal_difference_reads(i: &mut Interpreter, option: &JsValue, mode_default: &str, since: bool)
    -> Result<(Option<String>, i64, String, Option<String>), Throw> {
    let largest_raw = std_temporal_option_string(i, option, "largestUnit")?
        .map(|u| std_temporal_singular_unit(&u));
    let increment = std_temporal_option_value(i, option, "roundingIncrement")?;
    let increment = std_temporal_number(i, increment, Some(1))?;
    let mode = std_temporal_option_string(i, option, "roundingMode")?
        .unwrap_or_else(|| mode_default.to_string());
    if !std_temporal_round_mode_valid(&mode) {
        return Err(Throw::range_error("invalid roundingMode"));
    }
    let mode = if since { std_temporal_swap_directional_mode(&mode) } else { mode };
    let smallest_raw = std_temporal_option_string(i, option, "smallestUnit")?
        .map(|u| std_temporal_singular_unit(&u));
    Ok((largest_raw, increment, mode, smallest_raw))
}

// The roundingIncrement bound depends on the final resolved smallestUnit,
// which (per `std_temporal_difference_reads` above) is only known after all
// four option properties have already been read -- so this check is always
// one of the deferred "algorithmic validation" steps, never inline with a
// read.
fn std_temporal_validate_rounding_increment(smallest: &str, increment: i64) -> Result<(), Throw> {
    let maximum = match smallest {
        "hour" => 24, "minute" | "second" => 60,
        "millisecond" | "microsecond" | "nanosecond" => 1_000,
        _ => i64::MAX,
    };
    if increment < 1 || increment > 1_000_000_000 || increment >= maximum
        || (maximum != i64::MAX && maximum % increment != 0) {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }
    Ok(())
}

// Resolves an already-read, already-singularized largestUnit against its
// per-call default: absent or explicit "auto" both mean "use the default".
fn std_temporal_resolve_largest(largest_raw: &Option<String>, default: &str) -> String {
    match largest_raw.as_deref() {
        None | Some("auto") => default.to_string(),
        Some(u) => u.to_string(),
    }
}

fn std_temporal_round_mode_valid(mode: &str) -> bool {
    matches!(mode, "ceil" | "floor" | "expand" | "trunc" | "halfCeil"
        | "halfFloor" | "halfExpand" | "halfTrunc" | "halfEven")
}

// Ported from boa-dev/temporal (temporal_rs) @ eadf8039d2577e4375493a0eb3f46a66f25616ec,
// src/iso.rs `IsoDate::diff_iso_date` (spec 5.5.6 DifferenceISODate), Apache-2.0.
// Always anchored at `first` (never swapped to whichever endpoint is
// chronologically earlier): a month/year step's day-of-month clamping keeps
// using `first`'s own day throughout the bracket search, so which argument is
// `first` is observable in the day remainder whenever a month is shorter than
// `first`'s day-of-month, not just the overall sign of the result -- e.g.
// 2019-03-01 minus 2019-01-29 at largestUnit "month" is 1 month, 3 days
// (anchored at 2019-03-01, day 1, stepping backward), not 1 month, 1 day (the
// wrong answer produced by anchoring at whichever side is earlier and
// clamping with ITS day-of-month instead).
fn std_temporal_calendar_difference(first: &[i64; 10], second: &[i64; 10], largest: &str) -> Result<[i64; 10], Throw> {
    let mut result = [0; 10];
    let first_day = std_temporal_epoch_day(first[0], first[1], first[2]);
    let second_day = std_temporal_epoch_day(second[0], second[1], second[2]);
    if !matches!(largest, "year" | "month" | "week" | "day") {
        return Err(Throw::range_error("invalid largestUnit for date difference"));
    }
    if first_day == second_day { return Ok(result); }
    if largest == "day" { result[3] = second_day - first_day; return Ok(result); }
    if largest == "week" {
        let days = second_day - first_day;
        result[2] = days / 7; result[3] = days % 7;
        return Ok(result);
    }
    let sign: i64 = if second_day < first_day { -1 } else { 1 };
    let mut years = 0i64;
    let mut months = 0i64;
    // `years`/`months` are found by walking candidate offsets from `first` in
    // the `sign` direction until the next step would land strictly past
    // `second` (per `sign`); the comparison uses `first`'s own
    // (year, month, day) tuple unclamped (a raw ISO-date compare, not a valid
    // calendar date) exactly as the ported algorithm's `IsoDate::new_unchecked`
    // does, so a day-of-month that overflows the candidate month (e.g. day 31
    // against a candidate February) still orders correctly against `second`
    // without being silently clamped away first.
    let mut candidate_years = second[0] - first[0];
    if candidate_years != 0 { candidate_years -= sign; }
    loop {
        let (cy, cm) = std_temporal_balance_year_month(first[0] + candidate_years, first[1]);
        if std_temporal_calendar_surpasses((cy, cm, first[2]), (second[0], second[1], second[2]), sign) { break; }
        years = candidate_years;
        candidate_years += sign;
    }
    let mut candidate_months = sign;
    loop {
        let (cy, cm) = std_temporal_balance_year_month(first[0] + years, first[1] + candidate_months);
        if std_temporal_calendar_surpasses((cy, cm, first[2]), (second[0], second[1], second[2]), sign) { break; }
        months = candidate_months;
        candidate_months += sign;
    }
    if largest == "month" { months += years * 12; years = 0; }
    let mut single = [0i64; 10];
    single[0] = years; single[1] = months;
    let constrained = std_temporal_add_calendar(first, &single, 1, false)?;
    let constrained_day = std_temporal_epoch_day(constrained[0], constrained[1], constrained[2]);
    result[0] = years; result[1] = months; result[3] = second_day - constrained_day;
    Ok(result)
}

fn std_temporal_balance_year_month(year: i64, month: i64) -> (i64, i64) {
    (year + (month - 1).div_euclid(12), (month - 1).rem_euclid(12) + 1)
}

// Whether `candidate` (an unclamped, possibly-invalid ISO (year, month, day)
// tuple) has gone strictly past `other` in the `sign` direction -- landing
// exactly on `other` does not count, so the bracket-search loop above keeps
// the last candidate that lands on or short of the target and stops at the
// first step that overshoots it.
fn std_temporal_calendar_surpasses(candidate: (i64, i64, i64), other: (i64, i64, i64), sign: i64) -> bool {
    let cmp = if candidate.0 != other.0 { (candidate.0 - other.0).signum() }
        else if candidate.1 != other.1 { (candidate.1 - other.1).signum() }
        else { (candidate.2 - other.2).signum() };
    cmp != 0 && cmp == sign.signum()
}

fn std_temporal_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, since: bool) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let other = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0), false)?;
    let options = std_reflect_arg(&args, 1);
    // All four option properties are read here, in spec order, before any
    // per-kind default/validation logic below is allowed to throw -- see
    // `std_temporal_difference_reads`.
    let (largest_raw, increment, mode, smallest_raw) = std_temporal_difference_reads(i, &options, "trunc", since)?;
    // Ported from boa-dev/temporal (temporal_rs) @ eadf8039d2577e4375493a0eb3f46a66f25616ec,
    // src/builtins/core/plain_date.rs `diff_date`/`internal_diff_date` and the
    // RoundRelativeDuration algorithm they call into (Apache-2.0). `since` and
    // `until` share one exact-difference-then-round computation, always taken
    // in the receiver -> argument direction and always rounded/bracket-searched
    // relative to the RECEIVER (`self`, never the argument) -- calendar units
    // are variable-length (a month anchored at the receiver can be a different
    // number of days than the same span anchored at the argument), so which
    // side is used as the calendar anchor for the fractional/bracket
    // computation is observable, not just a sign convention (confirmed by
    // PlainDate/PlainDateTime's `rounding-relative.js`/`round-relative-to-
    // receiver.js`/`roundingmode-half-boundary.js`). Only at the very end does
    // `since` negate the whole rounded Duration. A directional roundingMode
    // (ceil/floor, halfCeil/halfFloor) must be swapped before that rounding
    // step for `since`, matching the identity floor(-x) = -ceil(x): otherwise
    // negating a `ceil`-rounded receiver->argument value at the end would
    // yield the same magnitude in the wrong direction relative to what the
    // caller asked for (confirmed by `roundingmode-ceil.js`, which expects
    // `since` and `until` on swapped receiver/argument to agree exactly, not
    // merely negate). `trunc`/`expand`/`halfExpand`/`halfTrunc`/`halfEven` are
    // already symmetric under negation and need no swap. The swap itself
    // already happened inside `std_temporal_difference_reads`.
    if data.kind == StdTemporalKind::PlainYearMonth {
        let smallest = smallest_raw.unwrap_or_else(|| "month".to_string());
        if !matches!(smallest.as_str(), "year" | "month") {
            return Err(Throw::range_error("invalid smallestUnit for PlainYearMonth difference"));
        }
        let largest = std_temporal_resolve_largest(&largest_raw, "year");
        if !matches!(largest.as_str(), "year" | "month") || (largest == "month" && smallest == "year") {
            return Err(Throw::range_error("invalid PlainYearMonth difference units"));
        }
        std_temporal_validate_rounding_increment(&smallest, increment)?;
        for fields in [&data.fields, &other] {
            if !(-100_000_000..=100_000_000).contains(&std_temporal_epoch_day(fields[0], fields[1], 1)) {
                return Err(Throw::range_error("PlainYearMonth difference outside date range"));
            }
        }
        let months = (other[0] - data.fields[0]) as i128 * 12 + (other[1] - data.fields[1]) as i128;
        let step = increment as i128 * if smallest == "year" { 12 } else { 1 };
        let rounded = std_temporal_round_integer(months, step, &mode)?;
        let rounded = if since { -rounded } else { rounded };
        let mut fields = [0; 10];
        if largest == "year" {
            fields[0] = (rounded / 12) as i64;
            fields[1] = (rounded % 12) as i64;
        } else { fields[1] = rounded as i64; }
        return std_temporal_make(i, StdTemporalKind::Duration, fields);
    }
    let mut result = match data.kind {
        StdTemporalKind::PlainDate => {
            let smallest = smallest_raw.unwrap_or_else(|| "day".to_string());
            if !matches!(smallest.as_str(), "year" | "month" | "week" | "day") {
                return Err(Throw::range_error("invalid smallestUnit for date difference"));
            }
            // GetDifferenceSettings: an absent/"auto" largestUnit defaults to
            // the larger of the operation's own default ("day") and
            // smallestUnit -- so smallestUnit "week" with no largestUnit
            // resolves to largestUnit "week", not "day", and does not throw
            // "largestUnit is smaller than smallestUnit".
            let largest_default = std_temporal_larger_unit("day", &smallest);
            let largest = std_temporal_resolve_largest(&largest_raw, &largest_default);
            if !matches!(largest.as_str(), "year" | "month" | "week" | "day") {
                return Err(Throw::range_error("invalid largestUnit for date difference"));
            }
            if std_temporal_unit_rank(&largest).unwrap() > std_temporal_unit_rank(&smallest).unwrap() {
                return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
            }
            std_temporal_validate_rounding_increment(&smallest, increment)?;
            if smallest == "day" {
                // A fixed-length unit: round the exact receiver->argument day
                // count to the requested increment/mode, then re-express that
                // many days from the receiver as a calendar breakdown at
                // `largest` (which may itself be coarser, e.g. smallestUnit
                // "day" with largestUnit "year").
                let base_day = std_temporal_epoch_day(data.fields[0], data.fields[1], data.fields[2]);
                let other_day = std_temporal_epoch_day(other[0], other[1], other[2]);
                let rounded = std_temporal_round_integer((other_day - base_day) as i128, increment as i128, &mode)? as i64;
                let (ry, rm, rd) = std_temporal_civil_from_epoch_day(base_day + rounded);
                let rounded_other = { let mut f = [0i64; 10]; f[0] = ry; f[1] = rm; f[2] = rd; f };
                std_temporal_calendar_difference(&data.fields, &rounded_other, &largest)?
            } else {
                // smallest is "week", "month" or "year": a real calendar-unit
                // rounding, bracket-searched from the RECEIVER (the same
                // machinery Duration.prototype.round uses for a calendar
                // smallestUnit against relativeTo). This also correctly
                // rounds a "week" remainder left over after an exact
                // whole-month/year breakdown, rather than re-rounding the
                // raw day count in weeks from the receiver (which would
                // spuriously add a week when the span is an exact multiple of
                // the larger unit, e.g. Jan 1 -> Feb 1 is exactly P1M with
                // zero weeks left over, even though 31 is not a multiple of
                // 7).
                let target_ns = std_temporal_epoch_day(other[0], other[1], other[2]) as i128 * STD_TEMPORAL_NS_DAY;
                std_temporal_round_to_calendar_unit(&data.fields, target_ns, &smallest, &largest, increment, &mode)?
            }
        }
        StdTemporalKind::PlainTime | StdTemporalKind::PlainDateTime => {
            let smallest = smallest_raw.unwrap_or_else(|| "nanosecond".to_string());
            if data.kind == StdTemporalKind::PlainTime && smallest == "day" {
                return Err(Throw::range_error("day is not a PlainTime unit"));
            }
            if data.kind == StdTemporalKind::PlainDateTime
                && matches!(smallest.as_str(), "year" | "month" | "week") {
                let largest_default = std_temporal_larger_unit("day", &smallest);
                let largest = std_temporal_resolve_largest(&largest_raw, &largest_default);
                if !matches!(largest.as_str(), "year" | "month" | "week" | "day") {
                    return Err(Throw::range_error("invalid largestUnit for date difference"));
                }
                if std_temporal_unit_rank(&largest).unwrap() > std_temporal_unit_rank(&smallest).unwrap() {
                    return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
                }
                std_temporal_validate_rounding_increment(&smallest, increment)?;
                std_temporal_round_to_calendar_unit(&data.fields, std_temporal_epoch_ns(&other), &smallest, &largest, increment, &mode)?
            } else {
                let default = if data.kind == StdTemporalKind::PlainTime { "hour" } else { "day" };
                let largest = std_temporal_resolve_largest(&largest_raw, default);
                if data.kind == StdTemporalKind::PlainDateTime
                    && matches!(largest.as_str(), "year" | "month" | "week") {
                    // smallestUnit here is a FIXED unit (day..nanosecond, not
                    // necessarily nanosecond -- any of them reach this branch
                    // once largestUnit is a calendar unit), so the exact
                    // receiver->argument delta must still be rounded to it
                    // before being re-expressed as a calendar+time breakdown
                    // at `largest`: a rounding that carries all the way
                    // through the sub-day remainder can cross into (or past)
                    // the next `largest`-unit boundary, e.g. rounding
                    // 1970-01-01 .. 1971-12-31T23:59:59.999999999 to whole
                    // microseconds with mode "expand" carries the trailing
                    // fractional microsecond all the way up into a full
                    // extra year (confirmed by
                    // `round-cross-unit-boundary.js`'s "rounding down 1 ns
                    // balances to -2 years" case). When smallestUnit is
                    // "nanosecond" with increment 1 this round is an
                    // identity (delta is already an exact integer of
                    // nanoseconds), so no separate unrounded-shortcut branch
                    // is needed.
                    std_temporal_validate_rounding_increment(&smallest, increment)?;
                    let unit = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
                    let delta = std_temporal_epoch_ns(&other) - std_temporal_epoch_ns(&data.fields);
                    let rounded = std_temporal_round_integer(delta, unit * increment as i128, &mode)?;
                    std_temporal_epoch_breakdown(&data.fields, std_temporal_epoch_ns(&data.fields) + rounded, &largest)?
                } else {
                    let index = match largest.as_str() {
                        "day" if data.kind == StdTemporalKind::PlainDateTime => 3,
                        "hour" => 4, "minute" => 5, "second" => 6,
                        "millisecond" => 7, "microsecond" => 8, "nanosecond" => 9,
                        _ => return Err(Throw::range_error("invalid largestUnit for time difference")),
                    };
                    let smallest_index = match smallest.as_str() { "day" => 3, "hour" => 4, "minute" => 5,
                        "second" => 6, "millisecond" => 7, "microsecond" => 8, "nanosecond" => 9,
                        _ => return Err(Throw::range_error("invalid smallestUnit")) };
                    if index > smallest_index {
                        return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
                    }
                    std_temporal_validate_rounding_increment(&smallest, increment)?;
                    let delta = if data.kind == StdTemporalKind::PlainTime {
                        std_temporal_time_ns(&other) - std_temporal_time_ns(&data.fields)
                    } else { std_temporal_epoch_ns(&other) - std_temporal_epoch_ns(&data.fields) };
                    let unit = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
                    let rounded = std_temporal_round_integer(delta, unit * increment as i128, &mode)?;
                    std_temporal_split_duration_ns(rounded, index)
                }
            }
        }
        _ => return Err(Throw::type_error("incompatible Temporal receiver")),
    };
    if since { for field in &mut result { *field = -*field; } }
    std_temporal_make(i, StdTemporalKind::Duration, result)
}

// The identity floor(-x) = -ceil(x) (and halfFloor(-x) = -halfCeil(x)) means a
// rounding mode that favors one side of zero must swap partners when the
// whole rounded value is later negated (as `since` does relative to `until`'s
// receiver -> argument computation), or the caller's requested direction
// would come out backwards. `trunc`, `expand` and the `half*` variants other
// than halfCeil/halfFloor are symmetric under negation and pass through
// unchanged. Ported from boa-dev/temporal (temporal_rs)
// @ eadf8039d2577e4375493a0eb3f46a66f25616ec's sign-aware rounding-mode
// handling in `src/rounding.rs`, Apache-2.0.
fn std_temporal_swap_directional_mode(mode: &str) -> String {
    match mode {
        "ceil" => "floor",
        "floor" => "ceil",
        "halfCeil" => "halfFloor",
        "halfFloor" => "halfCeil",
        other => other,
    }.to_string()
}

fn std_temporal_until(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_difference(i, this, args, false)
}
fn std_temporal_since(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_difference(i, this, args, true)
}

// The field this loop is currently reading, resolved per Temporal kind below
// in the exact alphabetical order PrepareTemporalFields/PrepareCalendarFields
// specifies (and that with()'s many order-of-operations.js tests observe via
// a Proxy). MonthCode and Year get their own arms because they need special
// handling (a calendar-format string; and, for PlainMonthDay, a value that
// regulates the result without being stored in it).
enum StdWithField { Plural(usize), Year, Month, MonthCode, Day, Hour, Minute, Second, Millisecond, Microsecond, Nanosecond }

// The per-kind field name table, already listed in the exact alphabetical
// order PrepareTemporalFields/PrepareCalendarFields/GetTemporalPlainTimeFields
// specifies (and that many order-of-operations.js tests observe via a Proxy).
// Shared by `with()` (merging into an existing record) and `std_temporal_bag`
// (building a fresh record for `from()`/since/until's `other` argument/
// constructors) -- both must read a property bag's OWN fields in this same
// order, not the fields' natural/positional order (year, month, day, ...),
// which `std_temporal_bag` got wrong until this port (confirmed by
// PlainDate/PlainDateTime/PlainTime's own `since`/`until`/`from`
// `order-of-operations.js`).
fn std_temporal_field_table(kind: StdTemporalKind) -> &'static [(&'static str, StdWithField)] {
    match kind {
        StdTemporalKind::Duration => &[
            ("days", StdWithField::Plural(3)), ("hours", StdWithField::Plural(4)),
            ("microseconds", StdWithField::Plural(8)), ("milliseconds", StdWithField::Plural(7)),
            ("minutes", StdWithField::Plural(5)), ("months", StdWithField::Plural(1)),
            ("nanoseconds", StdWithField::Plural(9)), ("seconds", StdWithField::Plural(6)),
            ("weeks", StdWithField::Plural(2)), ("years", StdWithField::Plural(0)),
        ],
        StdTemporalKind::PlainTime => &[
            ("hour", StdWithField::Hour), ("microsecond", StdWithField::Microsecond),
            ("millisecond", StdWithField::Millisecond), ("minute", StdWithField::Minute),
            ("nanosecond", StdWithField::Nanosecond), ("second", StdWithField::Second),
        ],
        StdTemporalKind::PlainDate => &[
            ("day", StdWithField::Day), ("month", StdWithField::Month),
            ("monthCode", StdWithField::MonthCode), ("year", StdWithField::Year),
        ],
        StdTemporalKind::PlainDateTime => &[
            ("day", StdWithField::Day), ("hour", StdWithField::Hour),
            ("microsecond", StdWithField::Microsecond), ("millisecond", StdWithField::Millisecond),
            ("minute", StdWithField::Minute), ("month", StdWithField::Month),
            ("monthCode", StdWithField::MonthCode), ("nanosecond", StdWithField::Nanosecond),
            ("second", StdWithField::Second), ("year", StdWithField::Year),
        ],
        StdTemporalKind::PlainYearMonth => &[
            ("month", StdWithField::Month), ("monthCode", StdWithField::MonthCode),
            ("year", StdWithField::Year),
        ],
        StdTemporalKind::PlainMonthDay => &[
            ("day", StdWithField::Day), ("month", StdWithField::Month),
            ("monthCode", StdWithField::MonthCode), ("year", StdWithField::Year),
        ],
    }
}

// The result of reading a property bag's own calendar/time fields, in
// alphabetical order, with each field's own immediate validation
// (month/day < 1 throws right at ITS OWN read, per
// ToPositiveIntegerWithTruncation) already applied. `monthCode` is only
// *read* here (ToString, an observable call); its calendar-format validation
// and cross-check against `month` is calendar-dependent resolution deferred
// to the caller, matching `options-read-before-algorithmic-validation.js`.
struct StdCalendarRead {
    year: Option<i64>,
    month: Option<i64>,
    month_code_text: Option<String>,
    day: Option<i64>,
    time: [i64; 6],
    any_present: bool,
}

fn std_temporal_read_calendar_fields(i: &mut Interpreter, kind: StdTemporalKind, value: &JsValue) -> Result<StdCalendarRead, Throw> {
    let mut read = StdCalendarRead { year: None, month: None, month_code_text: None, day: None, time: [0; 6], any_present: false };
    for (name, field) in std_temporal_field_table(kind) {
        let property = i.get_prop(value, *name)?;
        if property.is_undefined() { continue; }
        read.any_present = true;
        match field {
            StdWithField::Plural(_) => unreachable!("Duration is read directly by its own caller"),
            StdWithField::Year => read.year = Some(std_temporal_number(i, property, None)?),
            StdWithField::Month => {
                let value = std_temporal_number(i, property, None)?;
                if value < 1 { return Err(Throw::range_error("month must be a positive integer")); }
                read.month = Some(value);
            }
            // NOTE: a newer (2026) test (ZonedDateTime/from/month-code-wrong-
            // type.js) expects a non-string monthCode -- even an object with
            // a toString method -- to throw TypeError without ever calling
            // ToString. But the long-established order-of-operations.js
            // tests for PlainDate/PlainDateTime/PlainYearMonth's from/with/
            // since/until (2020) observe monthCode being ToString-coerced
            // like any other field, via propertyBagObserver-wrapped values.
            // The two are mutually exclusive for this shared reader; kept as
            // ToString-coercion since that side has far more passing
            // coverage in this pinned corpus (13 files vs 4). See
            // docs/lanes/js-stdlib_TODO.md, JSS-temporal-port item 2.
            StdWithField::MonthCode => read.month_code_text = Some(i.to_string(&property)?),
            StdWithField::Day => {
                let value = std_temporal_number(i, property, None)?;
                if value < 1 { return Err(Throw::range_error("day must be a positive integer")); }
                read.day = Some(value);
            }
            StdWithField::Hour => read.time[0] = std_temporal_number(i, property, None)?,
            StdWithField::Minute => read.time[1] = std_temporal_number(i, property, None)?,
            StdWithField::Second => read.time[2] = std_temporal_number(i, property, None)?,
            StdWithField::Millisecond => read.time[3] = std_temporal_number(i, property, None)?,
            StdWithField::Microsecond => read.time[4] = std_temporal_number(i, property, None)?,
            StdWithField::Nanosecond => read.time[5] = std_temporal_number(i, property, None)?,
        }
    }
    Ok(read)
}

fn std_temporal_with(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let bag = std_reflect_arg(&args, 0);
    if data.kind == StdTemporalKind::Duration {
        if !bag.is_object() { return Err(Throw::type_error("Temporal fields must be an object")); }
    } else {
        // IsPartialTemporalObject + RejectObjectWithCalendarOrTimeZone: a bag
        // that is itself a recognized Temporal object (of ANY kind, not just
        // this receiver's own) is rejected outright, without ever touching
        // its calendar/timeZone properties -- those may be throwing getters
        // on a real Temporal instance, and real internal-slot access never
        // triggers a getter. An ordinary object IS allowed to carry other,
        // unrelated properties (extras are just ignored), but not calendar
        // or timeZone specifically.
        if !bag.is_object()
            || std_temporal_record(&bag, None).is_ok()
            || std_epoch_record(&bag, Some(StdEpochKind::ZonedDateTime)).is_ok() {
            return Err(Throw::type_error("with() argument must be a plain object"));
        }
        let calendar = i.get_prop(&bag, "calendar")?;
        if !calendar.is_undefined() {
            return Err(Throw::type_error("with() argument must not have a calendar property"));
        }
        let time_zone = i.get_prop(&bag, "timeZone")?;
        if !time_zone.is_undefined() {
            return Err(Throw::type_error("with() argument must not have a timeZone property"));
        }
    }
    let fields: &[(&str, StdWithField)] = std_temporal_field_table(data.kind);
    let mut result = data.fields;
    let mut present = false;
    let mut month_seen = None;
    // PlainMonthDay only: a supplied "year" regulates (e.g. decides whether
    // Feb 29 constrains to Feb 28) without ever being stored -- the result's
    // reference year is always 1972, matching std_temporal_bag's convention.
    let mut year_hint = None;
    // The monthCode string is only *read* (ToString, an observable call)
    // during this loop; its calendar-format validation and disagreement
    // check against "month" are calendar-dependent resolution that must wait
    // until after options.overflow has been read, per
    // options-read-before-algorithmic-validation.js.
    let mut month_code_text: Option<String> = None;
    for (name, field) in fields {
        let property = i.get_prop(&bag, *name)?;
        if property.is_undefined() { continue; }
        present = true;
        match field {
            StdWithField::Plural(index) => result[*index] = std_temporal_duration_number(i, property, None)?,
            StdWithField::Year => {
                let value = std_temporal_number(i, property, None)?;
                if data.kind == StdTemporalKind::PlainMonthDay { year_hint = Some(value); }
                else { result[0] = value; }
            }
            // PrepareCalendarFields reads "month" and "day" via
            // ToPositiveIntegerWithTruncation: a value below 1 throws a
            // RangeError immediately, regardless of the overflow option --
            // unlike the upper bound, which regulates below only once
            // overflow is known.
            StdWithField::Month => {
                let value = std_temporal_number(i, property, None)?;
                if value < 1 { return Err(Throw::range_error("month must be a positive integer")); }
                result[1] = value;
                month_seen = Some(value);
            }
            // See std_temporal_read_calendar_fields: kept as ToString
            // coercion for the same reason.
            StdWithField::MonthCode => month_code_text = Some(i.to_string(&property)?),
            StdWithField::Day => {
                let value = std_temporal_number(i, property, None)?;
                if value < 1 { return Err(Throw::range_error("day must be a positive integer")); }
                result[2] = value;
            }
            StdWithField::Hour => result[3] = std_temporal_number(i, property, None)?,
            StdWithField::Minute => result[4] = std_temporal_number(i, property, None)?,
            StdWithField::Second => result[5] = std_temporal_number(i, property, None)?,
            StdWithField::Millisecond => result[6] = std_temporal_number(i, property, None)?,
            StdWithField::Microsecond => result[7] = std_temporal_number(i, property, None)?,
            StdWithField::Nanosecond => result[8] = std_temporal_number(i, property, None)?,
        }
    }
    if !present { return Err(Throw::type_error("no Temporal fields provided")); }
    let reject = if data.kind == StdTemporalKind::Duration { false }
        else { std_temporal_overflow(i, std_reflect_arg(&args, 1))? };
    if let Some(code) = month_code_text {
        let month = code.strip_prefix('M').and_then(|digits| digits.parse::<i64>().ok())
            .filter(|month| code.len() == 3 && (1..=12).contains(month))
            .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
        if month_seen.is_some_and(|value| value != month) {
            return Err(Throw::range_error("month and monthCode disagree"));
        }
        result[1] = month;
    }
    if data.kind == StdTemporalKind::PlainMonthDay {
        // The reference year is always 1972 in the stored result (and is
        // never itself range-checked against a supplied "year" -- only the
        // ISO reference year is), but a supplied "year" still decides, via
        // its own leap-ness, whether day 29 in February regulates. Applying
        // that decision through the shared std_temporal_regulate/validate
        // pair (which read the *stored* reference year for both jobs) would
        // either miss the supplied year's leap-ness or wrongly range-check
        // it, so PlainMonthDay resolves its day bound directly instead.
        if !reject { result[1] = result[1].clamp(1, 12); }
        if (1..=12).contains(&result[1]) {
            let max_day = std_temporal_days_in_month(year_hint.unwrap_or(1972), result[1]);
            if result[2] > max_day {
                if reject { return Err(Throw::range_error("day is invalid for month")); }
                result[2] = max_day;
            }
        }
        result[9] = 1972;
    } else {
        std_temporal_regulate(data.kind, &mut result, reject);
    }
    std_temporal_validate(data.kind, &result)?;
    std_temporal_make(i, data.kind, result)
}

fn std_temporal_datetime_with_time(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?.fields;
    let time = std_reflect_arg(&args, 0);
    if time.is_undefined() { fields[3..9].fill(0); }
    else {
        let values = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time, false)?;
        fields[3..9].copy_from_slice(&values[3..9]);
    }
    std_temporal_make(i, StdTemporalKind::PlainDateTime, fields)
}

fn std_temporal_unit_ns(name: &str) -> Option<i128> {
    match name {
        "week" | "weeks" => Some(7 * STD_TEMPORAL_NS_DAY),
        "day" | "days" => Some(STD_TEMPORAL_NS_DAY),
        "hour" | "hours" => Some(3_600 * STD_TEMPORAL_NS_SECOND),
        "minute" | "minutes" => Some(60 * STD_TEMPORAL_NS_SECOND),
        "second" | "seconds" => Some(STD_TEMPORAL_NS_SECOND),
        "millisecond" | "milliseconds" => Some(1_000_000),
        "microsecond" | "microseconds" => Some(1_000),
        "nanosecond" | "nanoseconds" => Some(1),
        _ => None,
    }
}

fn std_temporal_duration_total(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let fields = std_temporal_record(&this, Some(StdTemporalKind::Duration))?.fields;
    let option = std_reflect_arg(&args, 0);
    // relativeTo is read (and, per ToRelativeTemporalObject, fully resolved
    // -- so a malformed value must throw here) BEFORE `unit`, confirmed by
    // options-read-before-algorithmic-validation.js's exact property-read
    // order. Same two checks as Duration.prototype.round: a relativeTo
    // property bag naming a `timeZone` must still have that value resolved
    // even though only the local date ends up kept, and a relativeTo string
    // carrying both a bracketed zone annotation and an explicit numeric UTC
    // offset must be internally consistent.
    let relative = if option.is_object() { std_temporal_option_value(i, &option, "relativeTo")? }
        else { JsValue::Undefined };
    let base = std_temporal_resolve_relative_to(i, &relative)?;
    let unit = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "unit")?
            .ok_or_else(|| Throw::range_error("unit is required"))? };
    // relativeTo is required whenever a calendar unit is in play at all: the
    // requested `unit` is year/month/week (unconditionally -- even a
    // fixed-length duration like a single day needs it to convert into
    // weeks/months/years, per spec), or the duration itself carries a
    // nonzero year, month OR week component and is being read out in any
    // other unit (confirmed by `calendar-possibly-required.js`'s week-only
    // duration and `relativeto-undefined-throw-on-calendar-units.js`'s
    // per-unit sweep over every duration shape).
    if (fields[0] != 0 || fields[1] != 0 || fields[2] != 0
        || matches!(unit.as_str(), "year" | "years" | "month" | "months" | "week" | "weeks"))
        && base.is_none() {
        return Err(Throw::range_error("calendar units require relativeTo"));
    }
    let total_ns = if fields[0] != 0 || fields[1] != 0 {
        let base = base.as_ref().unwrap();
        let target = std_temporal_add_calendar(base, &fields, 1, false)?;
        let days = std_temporal_epoch_day(target[0], target[1], target[2])
            - std_temporal_epoch_day(base[0], base[1], base[2]);
        days as i128 * STD_TEMPORAL_NS_DAY
            + std_temporal_duration_time_ns(&fields)
            - (fields[2] as i128 * 7 + fields[3] as i128) * STD_TEMPORAL_NS_DAY
    } else { std_temporal_duration_time_ns(&fields) };
    if matches!(unit.as_str(), "year" | "years" | "month" | "months") {
        // A year/month has no fixed length, so the fractional part of the
        // total depends on WHICH year/month the span actually lands in, not
        // the length of the first one from `base` -- e.g. a duration that is
        // exactly 1 year plus 183 days, counted from a non-leap year's start,
        // spends those 183 days inside the following (leap) year, so the
        // fraction is 183/366 (= exactly 0.5), not 183/365 (confirmed by
        // since/until's `roundingmode-half-boundary.js`, which needs the
        // total to land on an exact 0.5 boundary for its half-rounding-mode
        // tie-break assertions to even be meaningful).
        let base = base.as_ref().unwrap();
        let calendar_unit = if unit.starts_with("year") { "year" } else { "month" };
        return Ok(JsValue::num(std_temporal_total_calendar_unit(base, total_ns, calendar_unit)?));
    }
    let unit_ns = std_temporal_unit_ns(&unit).ok_or_else(|| Throw::range_error("invalid unit"))?;
    Ok(JsValue::num(total_ns as f64 / unit_ns as f64))
}

// The real-valued position of `total_ns` (an exact epoch-nanosecond offset
// from `base`) in units of `unit` ("year" or "month"), for
// Temporal.Duration.prototype.total -- the fractional counterpart of
// `std_temporal_round_calendar_unit`'s integer bracket search: find the two
// adjacent whole-unit-count candidates (via real calendar arithmetic from
// `base`) that bracket `total_ns`, then interpolate linearly within that
// specific bracket's own length.
fn std_temporal_total_calendar_unit(base: &[i64; 10], total_ns: i128, unit: &str) -> Result<f64, Throw> {
    let unit_index = std_temporal_unit_rank(unit).filter(|r| *r <= 1)
        .ok_or_else(|| Throw::range_error("invalid unit for calendar total"))?;
    if total_ns == 0 { return Ok(0.0); }
    let single = std_temporal_calendar_step_ns(base, unit_index, 1)?.max(1);
    let mut lo = (total_ns as f64 / single as f64).floor() as i64;
    for _ in 0..1_000_000 {
        let lo_ns = std_temporal_calendar_step_ns(base, unit_index, lo)?;
        if lo_ns > total_ns { lo -= 1; continue; }
        let hi_ns = std_temporal_calendar_step_ns(base, unit_index, lo + 1)?;
        if hi_ns <= total_ns { lo += 1; continue; }
        if lo_ns == total_ns { return Ok(lo as f64); }
        let numerator = (total_ns - lo_ns) as f64;
        let denom = (hi_ns - lo_ns) as f64;
        return Ok(lo as f64 + numerator / denom);
    }
    Err(Throw::range_error("Temporal calendar total did not converge"))
}

fn std_temporal_round_integer(value: i128, increment: i128, mode: &str) -> Result<i128, Throw> {
    let quotient = value.div_euclid(increment);
    let remainder = value.rem_euclid(increment);
    if remainder == 0 { return Ok(value); }
    let lower = quotient * increment;
    let upper = lower + increment;
    let rounded = match mode {
        "floor" => lower,
        "ceil" => upper,
        "trunc" => if value < 0 { upper } else { lower },
        "expand" => if value < 0 { lower } else { upper },
        "halfExpand" | "halfEven" | "halfCeil" | "halfFloor" | "halfTrunc" => {
            let twice = remainder * 2;
            if twice < increment { lower } else if twice > increment { upper }
            else { match mode {
                "halfExpand" => if value < 0 { lower } else { upper },
                "halfEven" => if quotient % 2 == 0 { lower } else { upper },
                "halfCeil" => upper,
                "halfFloor" => lower,
                _ => if value < 0 { upper } else { lower },
            }}
        }
        _ => return Err(Throw::range_error("invalid roundingMode")),
    };
    Ok(rounded)
}

// Unit "rank": 0 = year (coarsest) .. 9 = nanosecond (finest), matching the
// Duration field-array convention directly.
fn std_temporal_unit_rank(unit: &str) -> Option<usize> {
    Some(match unit {
        "year" => 0, "month" => 1, "week" => 2, "day" => 3, "hour" => 4,
        "minute" => 5, "second" => 6, "millisecond" => 7, "microsecond" => 8,
        "nanosecond" => 9, _ => return None,
    })
}

fn std_temporal_unit_name(rank: usize) -> &'static str {
    ["year", "month", "week", "day", "hour", "minute", "second",
        "millisecond", "microsecond", "nanosecond"][rank]
}

fn std_temporal_larger_unit(a: &str, b: &str) -> String {
    let (ra, rb) = (std_temporal_unit_rank(a).unwrap_or(9), std_temporal_unit_rank(b).unwrap_or(9));
    std_temporal_unit_name(ra.min(rb)).to_string()
}

fn std_temporal_singular_unit(unit: &str) -> String {
    unit.strip_suffix('s').unwrap_or(unit).to_string()
}

// Sum of a Duration record's hour..nanosecond fields (indices 4..10) as
// nanoseconds -- the sub-day remainder that is calendar-independent even when
// years/months/weeks/days above it are not.
fn std_temporal_duration_subday_ns(fields: &[i64; 10]) -> i128 {
    fields[4] as i128 * 3_600 * STD_TEMPORAL_NS_SECOND + fields[5] as i128 * 60 * STD_TEMPORAL_NS_SECOND
        + fields[6] as i128 * STD_TEMPORAL_NS_SECOND + fields[7] as i128 * 1_000_000
        + fields[8] as i128 * 1_000 + fields[9] as i128
}

// Epoch nanoseconds of `base` (calendar-layout date, time assumed 0 unless
// `base` itself carries a time-of-day) with an entire Duration record applied:
// years/months/weeks/days through calendar arithmetic, the rest as an exact
// nanosecond offset.
fn std_temporal_duration_apply(base: &[i64; 10], duration: &[i64; 10]) -> Result<i128, Throw> {
    let after_date = std_temporal_add_calendar(base, duration, 1, false)?;
    Ok(std_temporal_epoch_ns(&after_date) + std_temporal_duration_subday_ns(duration))
}

// Express the span from `base` (calendar-layout, its own time-of-day
// included) to an absolute `target_epoch_ns` as a Duration record with the
// given largest unit -- the DifferenceISODateTime "day borrow" trick, so a
// negative sub-day time remainder that disagrees in sign with the date
// difference borrows one day from the date part instead of leaving a
// mixed-sign result.
fn std_temporal_epoch_breakdown(base: &[i64; 10], target_epoch_ns: i128, largest: &str) -> Result<[i64; 10], Throw> {
    if matches!(largest, "hour" | "minute" | "second" | "millisecond" | "microsecond" | "nanosecond") {
        let delta = target_epoch_ns - std_temporal_epoch_ns(base);
        let index = match largest { "hour" => 4, "minute" => 5, "second" => 6,
            "millisecond" => 7, "microsecond" => 8, _ => 9 };
        return Ok(std_temporal_split_duration_ns(delta, index));
    }
    let base_time_ns = std_temporal_time_ns(base);
    let mut target_fields = *base;
    std_temporal_set_epoch_ns(&mut target_fields, target_epoch_ns)?;
    let target_time_ns = std_temporal_time_ns(&target_fields);
    let mut time_delta = target_time_ns - base_time_ns;
    let time_sign = time_delta.signum();
    let base_day = std_temporal_epoch_day(base[0], base[1], base[2]);
    let target_day = std_temporal_epoch_day(target_fields[0], target_fields[1], target_fields[2]);
    let date_sign = (target_day - base_day).signum() as i128;
    let mut adjusted_day = target_day;
    if time_sign != 0 && time_sign == -date_sign {
        // Borrow one day in the direction the DATE part is heading (not the
        // time part): shrink the date difference by one day and hand that
        // day's worth of nanoseconds to the time remainder, in the same
        // direction, so both parts end up with matching (date_sign) signs.
        adjusted_day -= date_sign as i64;
        time_delta += date_sign as i128 * STD_TEMPORAL_NS_DAY;
    }
    let (ay, am, ad) = std_temporal_civil_from_epoch_day(adjusted_day);
    let mut adjusted_target = [0i64; 10];
    adjusted_target[0] = ay; adjusted_target[1] = am; adjusted_target[2] = ad;
    let mut base_date_only = [0i64; 10];
    base_date_only[0] = base[0]; base_date_only[1] = base[1]; base_date_only[2] = base[2];
    // `target_epoch_ns` can be arbitrarily large (e.g. balancing a duration
    // with a Number.MAX_SAFE_INTEGER-scale time component), landing the
    // civil date computed above far outside the ISO date range Temporal
    // actually supports; `std_temporal_set_epoch_ns` above only rejects an
    // i64 day-count overflow, not that narrower range, so check it here.
    std_temporal_validate(StdTemporalKind::PlainDate, &adjusted_target)?;
    let mut result = std_temporal_calendar_difference(&base_date_only, &adjusted_target, largest)?;
    let time_fields = std_temporal_split_duration_ns(time_delta, 4);
    result[4..10].copy_from_slice(&time_fields[4..10]);
    Ok(result)
}

// Epoch-nanosecond offset of `base` plus exactly `k` of a calendar unit
// (year/month/week, by Duration field index `unit_index`).
fn std_temporal_calendar_step_ns(base: &[i64; 10], unit_index: usize, k: i64) -> Result<i128, Throw> {
    let mut single = [0i64; 10];
    single[unit_index] = k;
    let target = std_temporal_add_calendar(base, &single, 1, false)?;
    // `std_temporal_add_calendar` itself only rejects an i64 day-count
    // overflow, not the narrower ISO date range Temporal actually supports --
    // an enormous candidate count (e.g. rounding Number.MIN_SAFE_INTEGER
    // seconds to whole years relative to a plain date) lands on a date far
    // outside that range and must be rejected here, at the source, rather
    // than silently bracket-searching to a bogus but in-magnitude Duration.
    std_temporal_validate(StdTemporalKind::PlainDate, &target)?;
    Ok(std_temporal_epoch_ns(&target) - std_temporal_epoch_ns(base))
}

// Round `total_ns` (an exact epoch-nanosecond offset from `base`) to the
// nearest multiple of `increment` calendar units, per `mode`. Calendar units
// are variable-length (a year or month has no fixed nanosecond size), so
// unlike `std_temporal_round_integer` this brackets the true value between
// two adjacent candidate unit-counts (computed via actual calendar
// arithmetic from `base`) and rounds within that bracket.
fn std_temporal_round_calendar_unit(base: &[i64; 10], total_ns: i128, unit: &str, increment: i64, mode: &str) -> Result<i64, Throw> {
    let unit_index = std_temporal_unit_rank(unit).filter(|r| *r <= 2)
        .ok_or_else(|| Throw::range_error("invalid unit for calendar rounding"))?;
    if total_ns == 0 { return Ok(0); }
    let single = std_temporal_calendar_step_ns(base, unit_index, 1)?.max(1);
    let mut lo = ((total_ns as f64 / single as f64) / increment as f64).floor() as i64 * increment;
    let neg = total_ns < 0;
    for _ in 0..1_000_000 {
        let lo_ns = std_temporal_calendar_step_ns(base, unit_index, lo)?;
        if lo_ns > total_ns { lo -= increment; continue; }
        let hi_ns = std_temporal_calendar_step_ns(base, unit_index, lo + increment)?;
        if hi_ns <= total_ns { lo += increment; continue; }
        let numerator = total_ns - lo_ns;
        let denom = hi_ns - lo_ns;
        if numerator == 0 { return Ok(lo); }
        return Ok(match mode {
            "floor" => lo,
            "ceil" => lo + increment,
            "trunc" => if neg { lo + increment } else { lo },
            "expand" => if neg { lo } else { lo + increment },
            "halfExpand" | "halfEven" | "halfCeil" | "halfFloor" | "halfTrunc" => {
                let twice = numerator * 2;
                if twice < denom { lo } else if twice > denom { lo + increment }
                else { match mode {
                    "halfExpand" => if neg { lo } else { lo + increment },
                    "halfEven" => if (lo / increment) % 2 == 0 { lo } else { lo + increment },
                    "halfCeil" => lo + increment,
                    "halfFloor" => lo,
                    _ => if neg { lo + increment } else { lo },
                }}
            }
            _ => return Err(Throw::range_error("invalid roundingMode")),
        });
    }
    Err(Throw::range_error("Temporal calendar rounding did not converge"))
}

// Express the span from `base` to the exact `target_ns` as a Duration record
// with the given `largest` unit, rounded at `smallest` unit granularity.
// `smallest` and `largest` are both in {year, month, week}. When they're
// equal, rounding "the total in smallestUnit's own units from base" and then
// splitting that into largestUnit's breakdown is exactly RoundDuration for
// this case. When they differ, that shortcut is wrong: e.g. smallestUnit
// "weeks" with largestUnit "months" must round only the *day remainder left
// over after the exact whole-month breakdown* to week granularity, not
// re-round the whole span in weeks from `base` -- 31 days is exactly one
// month with zero remainder, but is not a whole number of weeks, so rounding
// the raw week-count from `base` would spuriously add a week where none of
// the smallestUnit (weeks) actually remains.
fn std_temporal_round_to_calendar_unit(base: &[i64; 10], target_ns: i128, smallest: &str, largest: &str,
    increment: i64, mode: &str) -> Result<[i64; 10], Throw> {
    let total_ns = target_ns - std_temporal_epoch_ns(base);
    if largest == smallest {
        // `k` (found by bracket-searching real calendar steps from `base`) is
        // already the exact answer -- re-deriving it by adding `k` units to
        // `base` and then diffing back is not just redundant but lossy at an
        // end-of-month boundary: adding 11 months to a day-31 `base` lands on
        // a clamped shorter month (e.g. day 30), and diffing that clamped
        // date back against `base`'s own unclamped day 31 can read one unit
        // short (confirmed by `Duration/prototype/round/
        // end-of-month-round-up.js`, which needs "at the exact upper
        // bound" -- self.day=31 vs a 30-day target month -- to still report
        // the full 11 months rather than 10).
        let unit_index = std_temporal_unit_rank(smallest).unwrap();
        let k = std_temporal_round_calendar_unit(base, total_ns, smallest, increment, mode)?;
        let mut single = [0i64; 10];
        single[unit_index] = k;
        return Ok(single);
    }
    if smallest == "week" {
        // largest is "year" or "month": round the exact breakdown's day (+
        // any sub-day time) remainder to a whole number of weeks -- a fixed
        // 7-day ratio, no further calendar arithmetic needed.
        let mut exact = std_temporal_epoch_breakdown(base, target_ns, largest)?;
        let remainder_ns = exact[3] as i128 * STD_TEMPORAL_NS_DAY + std_temporal_duration_subday_ns(&exact);
        let step = 7 * STD_TEMPORAL_NS_DAY * increment as i128;
        let rounded = std_temporal_round_integer(remainder_ns, step, mode)?;
        exact[2] += (rounded / (7 * STD_TEMPORAL_NS_DAY)) as i64;
        exact[3..10].fill(0);
        return Ok(exact);
    }
    // smallest == "month", largest == "year": bracket-search the remainder
    // in months, anchored at the cursor after the exact whole-year part (a
    // month's length depends on where it starts, so this cannot use `base`
    // directly once a nonzero year count has already been consumed).
    let years = std_temporal_epoch_breakdown(base, target_ns, "year")?[0];
    let mut years_only = [0i64; 10];
    years_only[0] = years;
    let cursor = std_temporal_add_calendar(base, &years_only, 1, false)?;
    let remainder_ns = target_ns - std_temporal_epoch_ns(&cursor);
    let k = std_temporal_round_calendar_unit(&cursor, remainder_ns, "month", increment, mode)?;
    // As above: `years` and `k` are already exact (found by bracket-search),
    // so construct the result directly rather than re-deriving it through
    // `std_temporal_calendar_difference`, which would reintroduce the same
    // end-of-month clamping loss. Unlike the `largest == smallest` case,
    // rounding here CAN cross a year boundary (e.g. 1 year + 11 months
    // rounds up to 12 months, i.e. 2 years exactly), so the combined month
    // count still needs folding back into whole years -- confirmed by
    // `round-cross-unit-boundary.js`, which expects that carry.
    // Trunc (not Euclidean) division: the folded months must keep the sign
    // the overall span actually has, not always land in 0..12 -- e.g. -67
    // total months is years=-5, months=-7 (both negative, matching the
    // duration's own sign), not years=-6, months=+5.
    let total_months = years * 12 + k;
    let mut result = [0i64; 10];
    result[0] = total_months / 12; result[1] = total_months % 12;
    Ok(result)
}

// Temporal.Duration.prototype.round: a real RoundDuration + calendar-relative
// balancing, replacing the old stub that unconditionally threw whenever
// years/months were present. `smallestUnit`/`largestUnit` follow the spec's
// GetDifferenceSettings-style defaulting (each falls back to the other, or to
// the duration's own largest nonzero field), and any duration touching a
// calendar unit (or being re-expressed through one) resolves through
// `relativeTo` and real calendar arithmetic instead of a flat nanosecond
// count.
fn std_temporal_round_duration(i: &mut Interpreter, fields: [i64; 10], option: &JsValue) -> Result<JsValue, Throw> {
    // Every option property is read (and, for relativeTo, fully resolved --
    // so a malformed value must throw here) in this exact order --
    // largestUnit, relativeTo, roundingIncrement, roundingMode,
    // smallestUnit -- before ANY algorithmic validation (unit legality,
    // smallestUnit/largestUnit ordering, "one of the two is required",
    // rounding-increment divisibility, or "calendar units require
    // relativeTo"), confirmed by options-read-before-algorithmic-
    // validation.js's exact property-read trace. This also means
    // relativeTo is resolved unconditionally whenever given, even for a
    // duration/largestUnit combination that turns out not to need calendar
    // units at all -- confirmed by relativeto-required-properties.js.
    let largest_raw = if option.is_object() {
        std_temporal_option_string(i, option, "largestUnit")?.map(|u| std_temporal_singular_unit(&u))
    } else { None };
    let relative = if option.is_object() { std_temporal_option_value(i, option, "relativeTo")? } else { JsValue::Undefined };
    let base = std_temporal_resolve_relative_to(i, &relative)?;
    let (increment, mode) = if option.is_object() {
        let increment = std_temporal_property(i, option, "roundingIncrement")?.unwrap_or(1);
        let mode = std_temporal_option_string(i, option, "roundingMode")?.unwrap_or_else(|| "halfExpand".to_string());
        (increment, mode)
    } else { (1, "halfExpand".to_string()) };
    let smallest_raw = if let JsValue::Str(value) = option {
        Some(std_temporal_singular_unit(&value.to_utf8_lossy()))
    } else {
        std_temporal_option_string(i, option, "smallestUnit")?.map(|u| std_temporal_singular_unit(&u))
    };

    // The "at least one of largestUnit/smallestUnit is required" check is on
    // whether the option was given at all, not on its resolved value -- an
    // explicit largestUnit: "auto" still counts as present, unlike an absent
    // largestUnit that defaults to "auto" internally.
    if smallest_raw.is_none() && largest_raw.is_none() {
        return Err(Throw::range_error("one of largestUnit or smallestUnit is required"));
    }
    if !std_temporal_round_mode_valid(&mode) { return Err(Throw::range_error("invalid roundingMode")); }
    let largest_opt = largest_raw.filter(|u| u != "auto");
    for unit in smallest_raw.iter().chain(largest_opt.iter()) {
        if std_temporal_unit_rank(unit).is_none() { return Err(Throw::range_error("invalid Temporal unit")); }
    }
    let smallest = smallest_raw.unwrap_or_else(|| "nanosecond".to_string());
    let existing_largest = std_temporal_unit_name(fields.iter().position(|v| *v != 0).unwrap_or(9));
    let largest = largest_opt.unwrap_or_else(|| std_temporal_larger_unit(existing_largest, &smallest));
    let (smallest_index, largest_index) = (std_temporal_unit_rank(&smallest).unwrap(), std_temporal_unit_rank(&largest).unwrap());
    if largest_index > smallest_index {
        return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
    }
    // Time units additionally cannot exceed (or fail to evenly divide) the
    // next-largest unit, matching std_temporal_validate_rounding_increment.
    std_temporal_validate_rounding_increment(&smallest, increment)?;

    let needs_calendar = smallest_index <= 2 || largest_index <= 2
        || fields[0] != 0 || fields[1] != 0 || fields[2] != 0;
    if !needs_calendar {
        let step = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?
            * increment as i128;
        let rounded = std_temporal_round_integer(std_temporal_duration_time_ns(&fields), step, &mode)?;
        let split = std_temporal_split_duration_ns(rounded, largest_index);
        // BalanceTimeDuration's own float64-safe-integer requirement on the
        // field(s) it produces, stricter than (and on top of)
        // IsValidDuration's total-based bound in std_temporal_validate --
        // confirmed by round/out-of-range-when-converting-from-normalized-
        // duration.js, where Number.MAX_SAFE_INTEGER seconds (a legal total)
        // collapsed to largestUnit "nanoseconds" overflows float64 precision
        // in that one surviving field even though the total in seconds
        // stays within std_temporal_validate's own bound. Unlike
        // since/until's Duration output (`largestunit-plurals-accepted.js`/
        // `float64-representable-integer.js`), which never routes through
        // here, this check is specific to round()'s own conversion, not
        // Duration validity in general.
        if split.iter().any(|v| v.abs() > 9_007_199_254_740_991) {
            return Err(Throw::range_error("invalid Temporal duration"));
        }
        return std_temporal_make(i, StdTemporalKind::Duration, split);
    }
    let base = base.ok_or_else(|| Throw::range_error("calendar units require relativeTo"))?;
    let target_ns = std_temporal_duration_apply(&base, &fields)?;
    let result = if smallest_index <= 2 {
        std_temporal_round_to_calendar_unit(&base, target_ns, &smallest, &largest, increment, &mode)?
    } else {
        let total_ns = target_ns - std_temporal_epoch_ns(&base);
        let step = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?
            * increment as i128;
        let rounded_ns = std_temporal_round_integer(total_ns, step, &mode)?;
        std_temporal_epoch_breakdown(&base, std_temporal_epoch_ns(&base) + rounded_ns, &largest)?
    };
    // A duration whose time component is enormous relative to relativeTo
    // (e.g. Number.MAX_SAFE_INTEGER seconds) can balance out to a field
    // magnitude past what a Duration record may legally hold.
    std_temporal_validate(StdTemporalKind::Duration, &result)?;
    std_temporal_make(i, StdTemporalKind::Duration, result)
}

fn std_temporal_round(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if option.is_undefined() { return Err(Throw::type_error("round options are required")); }
    if data.kind == StdTemporalKind::Duration {
        return std_temporal_round_duration(i, data.fields, &option);
    }
    // Duration is fully handled by std_temporal_round_duration above; only
    // PlainTime/PlainDateTime (day and finer, no calendar units) reach here.
    let unit = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "smallestUnit")?.unwrap_or_default() };
    let unit = unit.strip_suffix('s').unwrap_or(&unit);
    if matches!(unit, "year" | "month") {
        return Err(Throw::range_error("calendar units require relativeTo"));
    }
    let unit_ns = std_temporal_unit_ns(unit).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
    let (increment, mode) = if option.is_object() {
        let increment = std_temporal_property(i, &option, "roundingIncrement")?.unwrap_or(1);
        let mode = std_temporal_option_string(i, &option, "roundingMode")?
            .unwrap_or_else(|| "halfExpand".to_string());
        (increment, mode)
    } else { (1, "halfExpand".to_string()) };
    if !std_temporal_round_mode_valid(&mode) {
        return Err(Throw::range_error("invalid roundingMode"));
    }
    if increment < 1 || increment > 1_000_000_000 {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }
    let step = unit_ns * increment as i128;
    let mut fields = data.fields;
    match data.kind {
        StdTemporalKind::PlainTime => {
            let rounded = std_temporal_round_integer(std_temporal_time_ns(&fields), step, &mode)?;
            std_temporal_set_time_ns(&mut fields, rounded);
        }
        StdTemporalKind::PlainDateTime => {
            let rounded = std_temporal_round_integer(std_temporal_epoch_ns(&fields), step, &mode)?;
            std_temporal_set_epoch_ns(&mut fields, rounded)?;
        }
        _ => return Err(Throw::type_error("incompatible Temporal receiver")),
    }
    std_temporal_make(i, data.kind, fields)
}
