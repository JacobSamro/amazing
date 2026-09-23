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
    let duration = std_temporal_from_fields(i, StdTemporalKind::Duration, std_reflect_arg(&args, 0))?;
    let mut result = data.fields;
    match data.kind {
        StdTemporalKind::Duration => {
            // Preserve each unit when the result has a consistent sign. A later
            // normalization step handles mixed-sign time units.
            for index in 0..10 {
                result[index] = result[index].checked_add(factor.checked_mul(duration[index])
                    .ok_or_else(|| Throw::range_error("duration overflow"))?)
                    .ok_or_else(|| Throw::range_error("duration overflow"))?;
            }
            if std_temporal_validate(StdTemporalKind::Duration, &result).is_err() {
                if result[0] != 0 || result[1] != 0 {
                    return Err(Throw::range_error("calendar units require relativeTo"));
                }
                let total = std_temporal_duration_time_ns(&data.fields)
                    + factor as i128 * std_temporal_duration_time_ns(&duration);
                result = std_temporal_split_duration_ns(total, 3);
            }
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
    let maximum = match smallest.as_str() {
        "hour" => 24, "minute" | "second" => 60,
        "millisecond" | "microsecond" | "nanosecond" => 1_000,
        _ => i64::MAX,
    };
    if increment < 1 || increment >= maximum || (maximum != i64::MAX && maximum % increment != 0) {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }
    Ok((smallest, increment, mode))
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
    let other = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0))?;
    let options = std_reflect_arg(&args, 1);
    let smallest_default = if matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth) {
        if data.kind == StdTemporalKind::PlainYearMonth { "month" } else { "day" }
    } else { "nanosecond" };
    let (smallest, increment, mode) = std_temporal_rounding_options(i, &options, smallest_default, "trunc")?;
    let mut result = match data.kind {
        StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth => {
            let default = if data.kind == StdTemporalKind::PlainYearMonth { "month" } else { "day" };
            let largest = std_temporal_largest_unit(i, &options, default)?;
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

fn std_temporal_with(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let bag = std_reflect_arg(&args, 0);
    if !bag.is_object() { return Err(Throw::type_error("Temporal fields must be an object")); }
    let mut result = data.fields;
    let mut present = false;
    let mut month_seen = None;
    let names: &[(&str, usize)] = if data.kind == StdTemporalKind::Duration {
        &[("years", 0), ("months", 1), ("weeks", 2), ("days", 3), ("hours", 4),
          ("minutes", 5), ("seconds", 6), ("milliseconds", 7), ("microseconds", 8), ("nanoseconds", 9)]
    } else {
        &[("year", 0), ("month", 1), ("day", 2), ("hour", 3), ("minute", 4),
          ("second", 5), ("millisecond", 6), ("microsecond", 7), ("nanosecond", 8)]
    };
    for (name, index) in names {
        let property = i.get_prop(&bag, *name)?;
        if !property.is_undefined() {
            result[*index] = if data.kind == StdTemporalKind::Duration {
                std_temporal_duration_number(i, property, None)?
            } else { std_temporal_number(i, property, None)? };
            if *name == "month" { month_seen = Some(result[*index]); }
            present = true;
        }
    }
    if data.kind != StdTemporalKind::Duration {
        let month_code = i.get_prop(&bag, "monthCode")?;
        if !month_code.is_undefined() {
            let code = i.to_string(&month_code)?;
            let month = code.strip_prefix('M').and_then(|digits| digits.parse::<i64>().ok())
                .filter(|month| code.len() == 3 && (1..=12).contains(month))
                .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
            if month_seen.is_some_and(|value| value != month) {
                return Err(Throw::range_error("month and monthCode disagree"));
            }
            result[1] = month; present = true;
        }
    }
    if !present { return Err(Throw::type_error("no Temporal fields provided")); }
    let reject = std_temporal_overflow(i, std_reflect_arg(&args, 1))?;
    if !reject && data.kind != StdTemporalKind::Duration {
        result[1] = result[1].clamp(1, 12);
        let year = if data.kind == StdTemporalKind::PlainMonthDay { result[9] } else { result[0] };
        result[2] = result[2].clamp(1, std_temporal_days_in_month(year, result[1]));
        result[3] = result[3].clamp(0, 23);
        result[4] = result[4].clamp(0, 59);
        result[5] = result[5].clamp(0, 59);
        for field in &mut result[6..9] { *field = (*field).clamp(0, 999); }
    }
    std_temporal_make(i, data.kind, result)
}

fn std_temporal_datetime_with_time(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?.fields;
    let time = std_reflect_arg(&args, 0);
    if time.is_undefined() { fields[3..9].fill(0); }
    else {
        let values = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time)?;
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
    if (fields[0] != 0 || fields[1] != 0 || matches!(unit.as_str(), "year" | "years" | "month" | "months"))
        && relative.is_undefined() {
        return Err(Throw::range_error("calendar units require relativeTo"));
    }
    let total_ns = if fields[0] != 0 || fields[1] != 0 {
        let base = std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative.clone())?;
        let target = std_temporal_add_calendar(&base, &fields, 1, false)?;
        let days = std_temporal_epoch_day(target[0], target[1], target[2])
            - std_temporal_epoch_day(base[0], base[1], base[2]);
        days as i128 * STD_TEMPORAL_NS_DAY
            + std_temporal_duration_time_ns(&fields)
            - (fields[2] as i128 * 7 + fields[3] as i128) * STD_TEMPORAL_NS_DAY
    } else { std_temporal_duration_time_ns(&fields) };
    let unit_ns = if matches!(unit.as_str(), "year" | "years" | "month" | "months") {
        let base = std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative)?;
        let mut single = [0; 10];
        single[if unit.starts_with("year") { 0 } else { 1 }] = 1;
        let next = std_temporal_add_calendar(&base, &single, 1, false)?;
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

fn std_temporal_round(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if option.is_undefined() { return Err(Throw::type_error("round options are required")); }
    let unit = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "smallestUnit")?
            .unwrap_or_else(|| if data.kind == StdTemporalKind::Duration { "nanosecond".to_string() }
                else { String::new() }) };
    let unit = unit.strip_suffix('s').unwrap_or(&unit);
    let unit_ns = std_temporal_unit_ns(unit).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
    let (increment, mode) = if option.is_object() {
        let increment = std_temporal_property(i, &option, "roundingIncrement")?.unwrap_or(1);
        let mode = std_temporal_option_string(i, &option, "roundingMode")?
            .unwrap_or_else(|| "halfExpand".to_string());
        (increment, mode)
    } else { (1, "halfExpand".to_string()) };
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
        StdTemporalKind::Duration => {
            if fields[0] != 0 || fields[1] != 0 {
                return Err(Throw::range_error("calendar units require relativeTo"));
            }
            let rounded = std_temporal_round_integer(std_temporal_duration_time_ns(&fields), step, &mode)?;
            let default_largest = fields.iter().position(|value| *value != 0).unwrap_or(9).max(2);
            let largest = if option.is_object() {
                let name = std_temporal_largest_unit(i, &option, "auto")?;
                if name == "auto" { default_largest }
                else { match name.as_str() { "week" => 2, "day" => 3, "hour" => 4,
                    "minute" => 5, "second" => 6, "millisecond" => 7,
                    "microsecond" => 8, "nanosecond" => 9,
                    _ => return Err(Throw::range_error("invalid largestUnit")) } }
            } else { default_largest };
            fields = std_temporal_split_duration_ns(rounded, largest);
        }
        _ => return Err(Throw::type_error("incompatible Temporal receiver")),
    }
    std_temporal_make(i, data.kind, fields)
}
