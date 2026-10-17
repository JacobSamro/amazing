// ECMAScript Date construction, parsing and prototype methods. This module is
// in-lane for js-binary (builtins_intl_date_support) and feeds the Intl
// DateTimeFormat tests that build dates with `new Date(y, m, d, ...)`,
// `Date.parse` and `Date.UTC`. Local time equals UTC because the engine has no
// host time-zone offset: every civil computation uses offset zero.

const INTL_MS_PER_DAY: f64 = 86_400_000.0;
const INTL_MAX_TIME: f64 = 8.64e15;

fn intl_days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Howard Hinnant's days_from_civil; month is 1..=12.
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn intl_time_clip(milliseconds: f64) -> f64 {
    if !milliseconds.is_finite() || milliseconds.abs() > INTL_MAX_TIME {
        return f64::NAN;
    }
    milliseconds.trunc()
}

fn intl_make_day(year: f64, month: f64, day: f64) -> f64 {
    if !year.is_finite() || !month.is_finite() || !day.is_finite() {
        return f64::NAN;
    }
    let year = year.trunc();
    let month = month.trunc();
    let day = day.trunc();
    // ECMAScript MakeDay normalizes the month into 0..=11.
    let year = year + (month / 12.0).floor();
    let month = month.rem_euclid(12.0);
    if year.abs() > 400_000.0 {
        return f64::NAN;
    }
    let days = intl_days_from_civil(year as i64, month as i64 + 1, 1) as f64 + (day - 1.0);
    days * INTL_MS_PER_DAY
}

fn intl_make_time(hour: f64, minute: f64, second: f64, millisecond: f64) -> f64 {
    if !hour.is_finite() || !minute.is_finite() || !second.is_finite() || !millisecond.is_finite() {
        return f64::NAN;
    }
    hour.trunc() * 3_600_000.0 + minute.trunc() * 60_000.0 + second.trunc() * 1_000.0 + millisecond.trunc()
}

fn intl_make_full_year(year: f64) -> f64 {
    if !year.is_finite() { return f64::NAN; }
    let year = year.trunc();
    if (0.0..=99.0).contains(&year) && !year.is_sign_negative() {
        year + 1900.0
    } else {
        year
    }
}

fn intl_date_value_of(i: &mut Interpreter, this: &JsValue) -> Result<f64, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("this is not a Date object")); };
    let stored = {
        let object = object.borrow();
        object.get_own("[[DateValue]]")
    };
    match stored {
        Some(value) => i.to_number(&value),
        None => Err(Throw::type_error("this is not a Date object")),
    }
}

fn intl_date_construct(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() {
        // Date() called as a function returns the current time string.
        return Ok(JsValue::str(intl_date_to_string_value(intl_now_milliseconds())));
    }
    let mut milliseconds;
    if args.is_empty() {
        milliseconds = intl_now_milliseconds();
    } else if args.len() == 1 {
        milliseconds = match &args[0] {
            JsValue::Object(object) if object.borrow().get_own("[[DateValue]]").is_some() => {
                i.to_number(&object.borrow().get_own("[[DateValue]]").unwrap())?
            }
            JsValue::Str(_) => {
                let text = i.to_string(&args[0])?;
                intl_parse_date_string(&text)
            }
            value => i.to_number(value)?,
        };
    } else {
        let year = i.to_number(&args[0])?;
        let month = i.to_number(&args[1])?;
        let day = match args.get(2) { Some(value) => i.to_number(value)?, None => 1.0 };
        let hour = match args.get(3) { Some(value) => i.to_number(value)?, None => 0.0 };
        let minute = match args.get(4) { Some(value) => i.to_number(value)?, None => 0.0 };
        let second = match args.get(5) { Some(value) => i.to_number(value)?, None => 0.0 };
        let millisecond = match args.get(6) { Some(value) => i.to_number(value)?, None => 0.0 };
        let day_value = intl_make_day(intl_make_full_year(year), month, day);
        let time_value = intl_make_time(hour, minute, second, millisecond);
        milliseconds = if day_value.is_nan() || time_value.is_nan() { f64::NAN } else { day_value + time_value };
    }
    milliseconds = intl_time_clip(milliseconds);
    // DateConstructor owns allocation: the time value is computed (ToPrimitive / ToNumber) first.
    let _ = &this;
    let prototype = std_collection_constructor_prototype(i)?;
    let mut raw = JsObject::ordinary();
    raw.proto = Some(prototype);
    raw.constructed = true;
    let object = Obj(Rc::new(RefCell::new(raw)));
    object.borrow_mut().define("[[DateValue]]", symbol_data(JsValue::num(milliseconds), true, true)).unwrap();
    Ok(JsValue::Object(object))
}

fn intl_date_utc(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if args.len() < 2 {
        return Ok(JsValue::num(f64::NAN));
    }
    let year = i.to_number(&args[0])?;
    let month = i.to_number(&args[1])?;
    let day = match args.get(2) { Some(value) => i.to_number(value)?, None => 1.0 };
    let hour = match args.get(3) { Some(value) => i.to_number(value)?, None => 0.0 };
    let minute = match args.get(4) { Some(value) => i.to_number(value)?, None => 0.0 };
    let second = match args.get(5) { Some(value) => i.to_number(value)?, None => 0.0 };
    let millisecond = match args.get(6) { Some(value) => i.to_number(value)?, None => 0.0 };
    if [year, month, hour, minute, second, millisecond].iter().any(|value| value.is_nan()) {
        return Ok(JsValue::num(f64::NAN));
    }
    let day_value = intl_make_day(intl_make_full_year(year), month, day);
    let time_value = intl_make_time(hour, minute, second, millisecond);
    Ok(JsValue::num(intl_time_clip(if day_value.is_nan() || time_value.is_nan() { f64::NAN } else { day_value + time_value })))
}

fn intl_date_parse_fn(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let text = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(intl_parse_date_string(&text)))
}

/// Parse the ISO 8601 subset (and a few common legacy shapes) that the pinned
/// Test262 Intl cases use. Unsupported shapes return NaN rather than guessing.
fn intl_parse_date_string(source: &str) -> f64 {
    let text = source.trim();
    if text.is_empty() { return f64::NAN; }
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut sign = 1.0f64;
    if bytes[index] == b'+' { index += 1; }
    else if bytes[index] == b'-' { sign = -1.0; index += 1; }
    let digits_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() { index += 1; }
    let year_digits = index - digits_start;
    if year_digits != 4 && year_digits != 6 { return f64::NAN; }
    let year = sign * text[digits_start..index].parse::<f64>().unwrap_or(f64::NAN);
    let mut read_pair = |index: &mut usize| -> Option<f64> {
        if *index + 2 > bytes.len() || !bytes[*index].is_ascii_digit() || !bytes[*index + 1].is_ascii_digit() {
            return None;
        }
        let value = text[*index..*index + 2].parse::<f64>().ok()?;
        *index += 2;
        Some(value)
    };
    let month;
    let mut day = 1.0;
    let mut hour = 0.0;
    let mut minute = 0.0;
    let mut second = 0.0;
    let mut millisecond = 0.0;
    let mut offset_minutes = None;
    if index < bytes.len() && bytes[index] == b'-' {
        index += 1;
        month = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
        if index < bytes.len() && bytes[index] == b'-' {
            index += 1;
            day = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
        }
        if index < bytes.len() && (bytes[index] == b'T' || bytes[index] == b' ') {
            index += 1;
            hour = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
            if index >= bytes.len() || bytes[index] != b':' { return f64::NAN; }
            index += 1;
            minute = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
            if index < bytes.len() && bytes[index] == b':' {
                index += 1;
                second = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
                if index < bytes.len() && bytes[index] == b'.' {
                    index += 1;
                    let fraction_start = index;
                    while index < bytes.len() && bytes[index].is_ascii_digit() { index += 1; }
                    if index == fraction_start { return f64::NAN; }
                    let fraction = &text[fraction_start..index];
                    let take = fraction.len().min(3);
                    millisecond = fraction[..take].parse::<f64>().unwrap_or(0.0)
                        * 10f64.powi((3 - take) as i32);
                }
            }
            offset_minutes = Some(0.0);
        }
        if index < bytes.len() {
            match bytes[index] {
                b'Z' => { index += 1; offset_minutes = Some(0.0); }
                b'+' | b'-' => {
                    let offset_sign = if bytes[index] == b'-' { -1.0 } else { 1.0 };
                    index += 1;
                    let offset_hour = match read_pair(&mut index) { Some(value) => value, None => return f64::NAN };
                    if index < bytes.len() && bytes[index] == b':' { index += 1; }
                    let offset_minute = if index + 1 < bytes.len() && bytes[index].is_ascii_digit() && bytes[index + 1].is_ascii_digit() {
                        read_pair(&mut index).unwrap_or(0.0)
                    } else { 0.0 };
                    offset_minutes = Some(offset_sign * (offset_hour * 60.0 + offset_minute));
                }
                _ => return f64::NAN,
            }
        }
    } else {
        return f64::NAN;
    }
    if index != bytes.len() { return f64::NAN; }
    if !(1.0..=12.0).contains(&month) || !(1.0..=31.0).contains(&day)
        || hour > 24.0 || minute > 59.0 || second > 59.0 {
        return f64::NAN;
    }
    // Treat hour 24 as the start of the next civil day.
    let day_shift = if hour == 24.0 { hour = 0.0; 1.0 } else { 0.0 };
    let day_value = intl_make_day(year, month - 1.0, day + day_shift);
    let time_value = intl_make_time(hour, minute, second, millisecond);
    if day_value.is_nan() || time_value.is_nan() { return f64::NAN; }
    let local = day_value + time_value;
    intl_time_clip(local - offset_minutes.unwrap_or(0.0) * 60_000.0)
}

fn intl_date_get_time(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(intl_date_value_of(i, &this)?))
}

fn intl_date_component(i: &mut Interpreter, this: JsValue, field: usize) -> Result<JsValue, Throw> {
    let milliseconds = intl_date_value_of(i, &this)?;
    if !milliseconds.is_finite() { return Ok(JsValue::num(f64::NAN)); }
    let Some(civil) = intl_civil_date(milliseconds, 0) else { return Ok(JsValue::num(f64::NAN)); };
    Ok(JsValue::num(match field {
        0 => civil.year as f64,
        1 => (civil.month - 1) as f64,
        2 => civil.day as f64,
        3 => civil.weekday as f64,
        4 => civil.hour as f64,
        5 => civil.minute as f64,
        6 => civil.second as f64,
        7 => civil.millisecond as f64,
        _ => f64::NAN,
    }))
}

macro_rules! intl_date_field {
    ($name:ident, $field:expr) => {
        fn $name(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
            intl_date_component(i, this, $field)
        }
    };
}
intl_date_field!(intl_date_get_full_year, 0);
intl_date_field!(intl_date_get_month, 1);
intl_date_field!(intl_date_get_date, 2);
intl_date_field!(intl_date_get_day, 3);
intl_date_field!(intl_date_get_hours, 4);
intl_date_field!(intl_date_get_minutes, 5);
intl_date_field!(intl_date_get_seconds, 6);
intl_date_field!(intl_date_get_milliseconds, 7);

fn intl_date_get_year(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = intl_date_component(i, this, 0)?;
    Ok(match value { JsValue::Num(year) if year.is_finite() => JsValue::num(year - 1900.0), other => other })
}

fn intl_date_get_timezone_offset(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(0.0))
}

fn intl_date_now(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::num(intl_now_milliseconds()))
}

fn intl_iso_string(milliseconds: f64) -> String {
    let civil = match intl_civil_date(milliseconds, 0) {
        Some(civil) => civil,
        None => return "Invalid Date".into(),
    };
    let year = if (0..=9999).contains(&civil.year) {
        format!("{:04}", civil.year)
    } else if civil.year < 0 {
        format!("-{:06}", -(civil.year as i64))
    } else {
        format!("+{:06}", civil.year)
    };
    format!("{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        civil.month, civil.day, civil.hour, civil.minute, civil.second, civil.millisecond)
}

fn intl_date_to_iso_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let milliseconds = intl_date_value_of(i, &this)?;
    if !milliseconds.is_finite() { return Err(Throw::range_error("invalid time value")); }
    Ok(JsValue::str(intl_iso_string(milliseconds)))
}

fn intl_date_to_json(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let milliseconds = intl_date_value_of(i, &this)?;
    if !milliseconds.is_finite() { return Ok(JsValue::Null); }
    Ok(JsValue::str(intl_iso_string(milliseconds)))
}

fn intl_date_to_string_method(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let milliseconds = intl_date_value_of(i, &this)?;
    Ok(JsValue::str(intl_date_to_string_value(milliseconds)))
}

fn intl_date_to_primitive(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let hint = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match hint.as_str() {
        "number" => intl_date_get_time(i, this, Vec::new()),
        "string" | "default" => intl_date_to_string_method(i, this, Vec::new()),
        _ => Err(Throw::type_error("invalid date hint")),
    }
}

/// Civil fields for the engine's fixed zero offset.
fn intl_date_to_string_value(milliseconds: f64) -> String {
    if !milliseconds.is_finite() { return "Invalid Date".into(); }
    let Some(civil) = intl_civil_date(milliseconds, 0) else { return "Invalid Date".into(); };
    let data = intl_date_data("en");
    format!("{} {} {:02} {:04} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        data.weekdays_abbr[civil.weekday], data.months_abbr[(civil.month - 1) as usize],
        civil.day, civil.year, civil.hour, civil.minute, civil.second)
}

impl Interpreter {
    fn install_date_engine(&mut self, constructor: &JsValue) {
        let JsValue::Object(ctor) = constructor else { return; };
        for (name, native, length) in [
            ("now", intl_date_now as crate::value::NativeFn, 0u32),
            ("parse", intl_date_parse_fn as crate::value::NativeFn, 1),
            ("UTC", intl_date_utc as crate::value::NativeFn, 7),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            ctor.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        let prototype = self.date_proto.clone();
        for (name, native, length) in [
            ("getTime", intl_date_get_time as crate::value::NativeFn, 0u32),
            ("valueOf", intl_date_get_time as crate::value::NativeFn, 0),
            ("getFullYear", intl_date_get_full_year as crate::value::NativeFn, 0),
            ("getUTCFullYear", intl_date_get_full_year as crate::value::NativeFn, 0),
            ("getYear", intl_date_get_year as crate::value::NativeFn, 0),
            ("getMonth", intl_date_get_month as crate::value::NativeFn, 0),
            ("getUTCMonth", intl_date_get_month as crate::value::NativeFn, 0),
            ("getDate", intl_date_get_date as crate::value::NativeFn, 0),
            ("getUTCDate", intl_date_get_date as crate::value::NativeFn, 0),
            ("getDay", intl_date_get_day as crate::value::NativeFn, 0),
            ("getUTCDay", intl_date_get_day as crate::value::NativeFn, 0),
            ("getHours", intl_date_get_hours as crate::value::NativeFn, 0),
            ("getUTCHours", intl_date_get_hours as crate::value::NativeFn, 0),
            ("getMinutes", intl_date_get_minutes as crate::value::NativeFn, 0),
            ("getUTCMinutes", intl_date_get_minutes as crate::value::NativeFn, 0),
            ("getSeconds", intl_date_get_seconds as crate::value::NativeFn, 0),
            ("getUTCSeconds", intl_date_get_seconds as crate::value::NativeFn, 0),
            ("getMilliseconds", intl_date_get_milliseconds as crate::value::NativeFn, 0),
            ("getUTCMilliseconds", intl_date_get_milliseconds as crate::value::NativeFn, 0),
            ("getTimezoneOffset", intl_date_get_timezone_offset as crate::value::NativeFn, 0),
            ("toISOString", intl_date_to_iso_string as crate::value::NativeFn, 0),
            ("toJSON", intl_date_to_json as crate::value::NativeFn, 1),
            ("toString", intl_date_to_string_method as crate::value::NativeFn, 0),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            prototype.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        let to_primitive = symbol_method(self, "[Symbol.toPrimitive]", intl_date_to_primitive);
        symbol_method_length(&to_primitive, 1);
        prototype.borrow_mut().define(
            WellKnownSymbol::ToPrimitive.symbol(), symbol_data(to_primitive, false, true)).unwrap();
    }
}

#[cfg(test)]
mod intl_date_engine_tests {
    use super::*;

    fn eval(source: &str) -> String {
        let mut i = Interpreter::new();
        let result = i.run_script(source).unwrap_or_else(|error| panic!("date error: {error:?}"));
        i.to_string(&result).unwrap()
    }

    #[test]
    fn parse_and_utc_agree_on_iso_instants() {
        assert_eq!(eval("Date.parse('2019-01-03T00:00:00Z')"), "1546473600000");
        assert_eq!(eval("Date.UTC(2012, 11, 17, 3, 0, 42)"), "1355713242000");
        assert_eq!(eval("new Date(2017, 11, 12, 13, 0, 0, 0).getHours()"), "13");
        assert_eq!(eval("new Date('1989-11-09T17:57:00Z').getTime()"), "626637420000");
    }

    #[test]
    fn invalid_dates_and_time_clip() {
        assert_eq!(eval("new Date(NaN).toString()"), "Invalid Date");
        assert_eq!(eval("isNaN(new Date('not a date').getTime())"), "true");
        assert_eq!(eval("new Date(8.64e15).getTime()"), "8640000000000000");
        assert_eq!(eval("isNaN(new Date(8.64e15 + 1).getTime())"), "true");
    }

    #[test]
    fn iso_output_and_typeof() {
        assert_eq!(eval("new Date(0).toISOString()"), "1970-01-01T00:00:00.000Z");
        assert_eq!(eval("Date.now() > 1e12"), "true");
        assert_eq!(eval("typeof Date.parse"), "function");
        assert_eq!(eval("new Date(2017, 11, 12, 0).getDate()"), "12");
    }

    #[test]
    fn date_format_propagates_valueof_errors() {
        let mut i = Interpreter::new();
        let result = i.run_script(
            "function E() {} \
             let o = { valueOf() { throw new E(); } }; \
             let caught; \
             try { new Intl.DateTimeFormat('en').formatToParts(o); } catch (e) { caught = e; } \
             caught && caught.constructor === E").unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "unexpected: {result:?}");
    }
}
