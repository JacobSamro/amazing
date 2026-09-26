// ISO arithmetic shared by the plain Temporal types. Epoch nanoseconds stay
// i128 throughout; no f64 conversion occurs until Duration.total returns a
// Number, as required by its JavaScript API.

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

fn std_temporal_largest_unit(i: &mut Interpreter, option: &JsValue, default: &str) -> Result<String, Throw> {
    let unit = std_temporal_option_string(i, option, "largestUnit")?.unwrap_or_else(|| default.to_string());
    let unit = if unit == "auto" { default.to_string() } else { unit };
    Ok(unit.strip_suffix('s').unwrap_or(&unit).to_string())
}

fn std_temporal_rounding_options(i: &mut Interpreter, option: &JsValue,
    smallest_default: &str, mode_default: &str) -> Result<(String, i64, String), Throw> {
    let smallest = std_temporal_option_string(i, option, "smallestUnit")?
        .unwrap_or_else(|| smallest_default.to_string());
    let smallest = smallest.strip_suffix('s').unwrap_or(&smallest).to_string();
    let increment = std_temporal_option_value(i, option, "roundingIncrement")?;
    let increment = std_temporal_number(i, increment, Some(1))?;
    let mode = std_temporal_option_string(i, option, "roundingMode")?
        .unwrap_or_else(|| mode_default.to_string());
    if !std_temporal_round_mode_valid(&mode) {
        return Err(Throw::range_error("invalid roundingMode"));
    }
    let maximum = match smallest.as_str() {
        "hour" => 24, "minute" | "second" => 60,
        "millisecond" | "microsecond" | "nanosecond" => 1_000,
        _ => i64::MAX,
    };
    if increment < 1 || increment > 1_000_000_000 || increment >= maximum
        || (maximum != i64::MAX && maximum % increment != 0) {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }
    Ok((smallest, increment, mode))
}

fn std_temporal_round_mode_valid(mode: &str) -> bool {
    matches!(mode, "ceil" | "floor" | "expand" | "trunc" | "halfCeil"
        | "halfFloor" | "halfExpand" | "halfTrunc" | "halfEven")
}

fn std_temporal_calendar_difference(first: &[i64; 10], second: &[i64; 10], largest: &str) -> Result<[i64; 10], Throw> {
    let mut result = [0; 10];
    let first_day = std_temporal_epoch_day(first[0], first[1], first[2]);
    let second_day = std_temporal_epoch_day(second[0], second[1], second[2]);
    if !matches!(largest, "year" | "month" | "week" | "day") {
        return Err(Throw::range_error("invalid largestUnit for date difference"));
    }
    if largest == "day" { result[3] = second_day - first_day; return Ok(result); }
    if largest == "week" {
        let days = second_day - first_day;
        result[2] = days / 7; result[3] = days % 7;
        return Ok(result);
    }
    let sign = if second_day < first_day { -1 } else { 1 };
    let (start, end) = if sign > 0 { (first, second) } else { (second, first) };
    let mut months = (end[0] - start[0]) * 12 + end[1] - start[1];
    let mut month_duration = [0; 10]; month_duration[1] = months;
    let mut cursor = std_temporal_add_calendar(start, &month_duration, 1, false)?;
    if std_temporal_epoch_day(cursor[0], cursor[1], cursor[2])
        > std_temporal_epoch_day(end[0], end[1], end[2]) {
        months -= 1; month_duration[1] = months;
        cursor = std_temporal_add_calendar(start, &month_duration, 1, false)?;
    }
    let days = std_temporal_epoch_day(end[0], end[1], end[2])
        - std_temporal_epoch_day(cursor[0], cursor[1], cursor[2]);
    if largest == "year" {
        result[0] = sign * (months / 12); result[1] = sign * (months % 12);
    } else { result[1] = sign * months; }
    result[3] = sign * days;
    Ok(result)
}

fn std_temporal_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, since: bool) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let other = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0), false)?;
    let options = std_reflect_arg(&args, 1);
    let smallest_default = if matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth) {
        if data.kind == StdTemporalKind::PlainYearMonth { "month" } else { "day" }
    } else { "nanosecond" };
    let (smallest, increment, mode) = std_temporal_rounding_options(i, &options, smallest_default, "trunc")?;
    if data.kind == StdTemporalKind::PlainYearMonth {
        let largest = std_temporal_largest_unit(i, &options, "year")?;
        if !matches!(largest.as_str(), "year" | "month")
            || !matches!(smallest.as_str(), "year" | "month")
            || (largest == "month" && smallest == "year") {
            return Err(Throw::range_error("invalid PlainYearMonth difference units"));
        }
        for fields in [&data.fields, &other] {
            if !(-100_000_000..=100_000_000).contains(&std_temporal_epoch_day(fields[0], fields[1], 1)) {
                return Err(Throw::range_error("PlainYearMonth difference outside date range"));
            }
        }
        let months = (other[0] - data.fields[0]) as i128 * 12 + (other[1] - data.fields[1]) as i128;
        let months = if since { -months } else { months };
        let step = increment as i128 * if smallest == "year" { 12 } else { 1 };
        let rounded = std_temporal_round_integer(months, step, &mode)?;
        let mut fields = [0; 10];
        if largest == "year" {
            fields[0] = (rounded / 12) as i64;
            fields[1] = (rounded % 12) as i64;
        } else { fields[1] = rounded as i64; }
        return std_temporal_make(i, StdTemporalKind::Duration, fields);
    }
    let mut result = match data.kind {
        StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth => {
            let default = if data.kind == StdTemporalKind::PlainYearMonth { "year" } else { "day" };
            let largest = std_temporal_largest_unit(i, &options, default)?;
            if data.kind == StdTemporalKind::PlainYearMonth
                && !matches!(largest.as_str(), "year" | "month") {
                return Err(Throw::range_error("invalid largestUnit for PlainYearMonth"));
            }
            if data.kind == StdTemporalKind::PlainYearMonth
                && !matches!(smallest.as_str(), "year" | "month") {
                return Err(Throw::range_error("invalid smallestUnit for PlainYearMonth"));
            }
            if matches!(smallest.as_str(), "day" | "week") && matches!(largest.as_str(), "day" | "week") {
                let days = std_temporal_epoch_day(other[0], other[1], other[2])
                    - std_temporal_epoch_day(data.fields[0], data.fields[1], data.fields[2]);
                let signed = if since { -days } else { days };
                let step = if smallest == "week" { 7 } else { 1 } * increment;
                let rounded = std_temporal_round_integer(signed as i128, step as i128, &mode)?;
                let mut fields = [0; 10];
                if largest == "week" { fields[2] = (rounded / 7) as i64; fields[3] = (rounded % 7) as i64; }
                else { fields[3] = rounded as i64; }
                return std_temporal_make(i, StdTemporalKind::Duration, fields);
            }
            std_temporal_calendar_difference(&data.fields, &other, &largest)?
        }
        StdTemporalKind::PlainTime | StdTemporalKind::PlainDateTime => {
            if data.kind == StdTemporalKind::PlainDateTime
                && matches!(smallest.as_str(), "year" | "month" | "week") {
                let mut fields = std_temporal_calendar_difference(&data.fields, &other, &smallest)?;
                if since { for field in &mut fields { *field = -*field; } }
                return std_temporal_make(i, StdTemporalKind::Duration, fields);
            }
            let default = if data.kind == StdTemporalKind::PlainTime { "hour" } else { "day" };
            let largest = std_temporal_largest_unit(i, &options, default)?;
            if data.kind == StdTemporalKind::PlainDateTime
                && matches!(largest.as_str(), "year" | "month" | "week") {
                // Full precision (smallestUnit defaulted to nanosecond, the only
                // case reaching here): an exact calendar+time rebalance to the
                // requested largestUnit, no rounding involved.
                let mut fields = std_temporal_epoch_breakdown(&data.fields, std_temporal_epoch_ns(&other), &largest)?;
                if since { for field in &mut fields { *field = -*field; } }
                return std_temporal_make(i, StdTemporalKind::Duration, fields);
            }
            let index = match largest.as_str() {
                "day" if data.kind == StdTemporalKind::PlainDateTime => 3,
                "hour" => 4, "minute" => 5, "second" => 6,
                "millisecond" => 7, "microsecond" => 8, "nanosecond" => 9,
                _ => return Err(Throw::range_error("invalid largestUnit for time difference")),
            };
            let mut delta = if data.kind == StdTemporalKind::PlainTime {
                std_temporal_time_ns(&other) - std_temporal_time_ns(&data.fields)
            } else { std_temporal_epoch_ns(&other) - std_temporal_epoch_ns(&data.fields) };
            if since { delta = -delta; }
            let unit = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
            delta = std_temporal_round_integer(delta, unit * increment as i128, &mode)?;
            if data.kind == StdTemporalKind::PlainTime && smallest == "day" {
                return Err(Throw::range_error("day is not a PlainTime unit"));
            }
            if index > match smallest.as_str() { "day" => 3, "hour" => 4, "minute" => 5,
                "second" => 6, "millisecond" => 7, "microsecond" => 8, "nanosecond" => 9,
                _ => return Err(Throw::range_error("invalid smallestUnit")) } {
                return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
            }
            std_temporal_split_duration_ns(delta, index)
        }
        _ => return Err(Throw::type_error("incompatible Temporal receiver")),
    };
    if since && matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth) {
        for field in &mut result { *field = -*field; }
    }
    std_temporal_make(i, StdTemporalKind::Duration, result)
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
    let fields: &[(&str, StdWithField)] = match data.kind {
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
    };
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
    let unit = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "unit")?
            .ok_or_else(|| Throw::range_error("unit is required"))? };
    let relative = if option.is_object() { std_temporal_option_value(i, &option, "relativeTo")? }
        else { JsValue::Undefined };
    // ToRelativeTemporalObject (spec) resolves relativeTo -- and so must
    // throw on a malformed value -- whenever it is present, not only when a
    // calendar unit ends up actually needing it. Same two checks as
    // Duration.prototype.round: a relativeTo property bag naming a
    // `timeZone` must still have that value resolved even though only the
    // local date ends up kept, and a relativeTo string carrying both a
    // bracketed zone annotation and an explicit numeric UTC offset must be
    // internally consistent.
    if relative.is_object() {
        let time_zone = i.get_prop(&relative, "timeZone")?;
        if !time_zone.is_undefined() { std_epoch_zone_value(i, &time_zone)?; }
    }
    if let JsValue::Str(text) = &relative {
        std_temporal_validate_relative_to_offset(&text.to_utf8_lossy())?;
    }
    let base = if relative.is_undefined() { None } else {
        Some(std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative, false)?)
    };
    if (fields[0] != 0 || fields[1] != 0 || matches!(unit.as_str(), "year" | "years" | "month" | "months"))
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
    let unit_ns = if matches!(unit.as_str(), "year" | "years" | "month" | "months") {
        let base = base.as_ref().unwrap();
        let mut single = [0; 10];
        single[if unit.starts_with("year") { 0 } else { 1 }] = 1;
        let next = std_temporal_add_calendar(base, &single, 1, false)?;
        (std_temporal_epoch_day(next[0], next[1], next[2])
            - std_temporal_epoch_day(base[0], base[1], base[2])) as i128 * STD_TEMPORAL_NS_DAY
    } else { std_temporal_unit_ns(&unit).ok_or_else(|| Throw::range_error("invalid unit"))? };
    Ok(JsValue::num(total_ns as f64 / unit_ns as f64))
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
        let unit_index = std_temporal_unit_rank(smallest).unwrap();
        let k = std_temporal_round_calendar_unit(base, total_ns, smallest, increment, mode)?;
        let mut single = [0i64; 10];
        single[unit_index] = k;
        let rounded_target_date = std_temporal_add_calendar(base, &single, 1, false)?;
        return std_temporal_calendar_difference(base, &rounded_target_date, largest);
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
    let mut single = [0i64; 10];
    single[1] = k;
    let rounded_target_date = std_temporal_add_calendar(&cursor, &single, 1, false)?;
    std_temporal_calendar_difference(base, &rounded_target_date, "year")
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
    let (smallest_raw, largest_raw) = if let JsValue::Str(value) = option {
        (Some(std_temporal_singular_unit(&value.to_utf8_lossy())), None)
    } else {
        (std_temporal_option_string(i, option, "smallestUnit")?.map(|u| std_temporal_singular_unit(&u)),
            std_temporal_option_string(i, option, "largestUnit")?.map(|u| std_temporal_singular_unit(&u)))
    };
    // The "at least one of largestUnit/smallestUnit is required" check is on
    // whether the option was given at all, not on its resolved value -- an
    // explicit largestUnit: "auto" still counts as present, unlike an absent
    // largestUnit that defaults to "auto" internally.
    if smallest_raw.is_none() && largest_raw.is_none() {
        return Err(Throw::range_error("one of largestUnit or smallestUnit is required"));
    }
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
    let (increment, mode) = if option.is_object() {
        let increment = std_temporal_property(i, option, "roundingIncrement")?.unwrap_or(1);
        let mode = std_temporal_option_string(i, option, "roundingMode")?.unwrap_or_else(|| "halfExpand".to_string());
        (increment, mode)
    } else { (1, "halfExpand".to_string()) };
    if !std_temporal_round_mode_valid(&mode) { return Err(Throw::range_error("invalid roundingMode")); }
    // Time units additionally cannot exceed (or fail to evenly divide) the
    // next-largest unit, matching std_temporal_rounding_options.
    let maximum = match smallest.as_str() {
        "hour" => 24, "minute" | "second" => 60,
        "millisecond" | "microsecond" | "nanosecond" => 1_000,
        _ => i64::MAX,
    };
    if increment < 1 || increment > 1_000_000_000 || increment >= maximum
        || (maximum != i64::MAX && maximum % increment != 0) {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }

    let needs_calendar = smallest_index <= 2 || largest_index <= 2 || fields[0] != 0 || fields[1] != 0;
    if !needs_calendar {
        let step = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?
            * increment as i128;
        let rounded = std_temporal_round_integer(std_temporal_duration_time_ns(&fields), step, &mode)?;
        return std_temporal_make(i, StdTemporalKind::Duration, std_temporal_split_duration_ns(rounded, largest_index));
    }
    let relative = if option.is_object() { std_temporal_option_value(i, option, "relativeTo")? } else { JsValue::Undefined };
    if relative.is_undefined() {
        return Err(Throw::range_error("calendar units require relativeTo"));
    }
    if relative.is_object() {
        // A relativeTo property bag naming a `timeZone` is conceptually a
        // ZonedDateTime-relative computation; this implementation only ever
        // keeps the local date, but the timeZone value must still be
        // resolved (and can still throw, e.g. a malformed date-time string
        // used as a time zone identifier) even though its result goes
        // unused below.
        let time_zone = i.get_prop(&relative, "timeZone")?;
        if !time_zone.is_undefined() { std_epoch_zone_value(i, &time_zone)?; }
    }
    if let JsValue::Str(text) = &relative {
        std_temporal_validate_relative_to_offset(&text.to_utf8_lossy())?;
    }
    let base = std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative, false)?;
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
