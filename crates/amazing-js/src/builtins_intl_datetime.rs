// Gregorian Intl.DateTimeFormat with UTC and fixed-offset time zones.

#[derive(Clone)]
struct IntlDateState {
    locale: String,
    calendar: String,
    numbering_system: String,
    time_zone: String,
    date_style: Option<String>,
    time_style: Option<String>,
    weekday: Option<String>,
    era: Option<String>,
    year: Option<String>,
    month: Option<String>,
    day: Option<String>,
    day_period: Option<String>,
    hour: Option<String>,
    minute: Option<String>,
    second: Option<String>,
    fractional_second_digits: Option<usize>,
    time_zone_name: Option<String>,
    hour_cycle: String,
}

fn intl_optional_string(i: &mut Interpreter, options: &JsValue, key: &str, allowed: &[&str]) -> Result<Option<String>, Throw> {
    let value = i.get_prop(options, key)?;
    if matches!(value, JsValue::Undefined) { return Ok(None); }
    let value = i.to_string(&value)?;
    if !allowed.contains(&value.as_str()) { return Err(Throw::range_error(format!("invalid {key} option"))); }
    Ok(Some(value))
}

fn intl_date_state(this: &JsValue) -> Result<IntlDateState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible DateTimeFormat receiver")); };
    match intl_state(object) {
        Some(IntlState::DateTime(state)) | Some(IntlState::BoundDateTime(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible DateTimeFormat receiver")),
    }
}

fn intl_timezone(text: &str) -> Result<String, Throw> {
    tz_canonicalize(text).ok_or_else(|| Throw::range_error("invalid time zone"))
}

impl Interpreter {
    fn install_intl_datetime(&mut self, namespace: &Obj) {
        let constructor = Self::native("DateTimeFormat", intl_date_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "DateTimeFormat", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        binary_getter(self, &prototype, "format", intl_date_format_getter);
        intl_method(self, &prototype, "formatToParts", 1, intl_date_format_to_parts);
        intl_method(self, &prototype, "formatRange", 2, intl_date_format_range);
        intl_method(self, &prototype, "formatRangeToParts", 2, intl_date_format_range_to_parts);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_date_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.DateTimeFormat")));
        let _ = namespace.borrow_mut().define("DateTimeFormat", binary_writable_property(constructor));
    }
}

fn intl_date_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let locale = intl_lookup(&requested);
    let options = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)).cloned()
        .unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null DateTimeFormat options")); }
    let _matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let calendar_option = intl_string_option(i, &options, "calendar", &[], "gregory")?;
    if calendar_option.split('-').any(|part| !(3..=8).contains(&part.len())
        || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
        return Err(Throw::range_error("invalid calendar"));
    }
    // A well-formed but unavailable calendar falls back to the locale default.
    let calendar = if calendar_option == "gregorian" { "gregory".into() }
        else if calendar_option == "iso8601" { calendar_option } else { "gregory".into() };
    let numbering_system = intl_string_option(i, &options, "numberingSystem", &[], intl_data(&locale).numbering_system)?;
    if !(3..=8).contains(&numbering_system.len())
        || !numbering_system.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(Throw::range_error("invalid numbering system"));
    }
    let time_zone_option = intl_string_option(i, &options, "timeZone", &[], "UTC")?;
    let time_zone = intl_timezone(&time_zone_option)?;
    let date_style = intl_optional_string(i, &options, "dateStyle", &["full", "long", "medium", "short"])?;
    let time_style = intl_optional_string(i, &options, "timeStyle", &["full", "long", "medium", "short"])?;
    let weekday = intl_optional_string(i, &options, "weekday", &["narrow", "short", "long"])?;
    let era = intl_optional_string(i, &options, "era", &["narrow", "short", "long"])?;
    let mut year = intl_optional_string(i, &options, "year", &["2-digit", "numeric"])?;
    let mut month = intl_optional_string(i, &options, "month", &["2-digit", "numeric", "narrow", "short", "long"])?;
    let mut day = intl_optional_string(i, &options, "day", &["2-digit", "numeric"])?;
    let day_period = intl_optional_string(i, &options, "dayPeriod", &["narrow", "short", "long"])?;
    let hour = intl_optional_string(i, &options, "hour", &["2-digit", "numeric"])?;
    let minute = intl_optional_string(i, &options, "minute", &["2-digit", "numeric"])?;
    let second = intl_optional_string(i, &options, "second", &["2-digit", "numeric"])?;
    let fractional_second_value = i.get_prop(&options, "fractionalSecondDigits")?;
    let fractional_second_digits = if fractional_second_value.is_undefined() { None }
        else { Some(intl_integer_option(i, &options, "fractionalSecondDigits", 1, 3, 1)?) };
    let time_zone_name = intl_optional_string(i, &options, "timeZoneName",
        &["short", "long", "shortOffset", "longOffset", "shortGeneric", "longGeneric"])?;
    let _format_matcher = intl_string_option(i, &options, "formatMatcher", &["basic", "best fit"], "best fit")?;
    if (date_style.is_some() || time_style.is_some())
        && [weekday.as_ref(), era.as_ref(), year.as_ref(), month.as_ref(), day.as_ref(), day_period.as_ref(),
            hour.as_ref(), minute.as_ref(), second.as_ref(), time_zone_name.as_ref()].iter().any(|field| field.is_some())
            || (date_style.is_some() || time_style.is_some()) && fractional_second_digits.is_some()
    {
        return Err(Throw::type_error("dateStyle/timeStyle conflicts with components"));
    }
    if date_style.is_none() && time_style.is_none()
        && [weekday.as_ref(), era.as_ref(), year.as_ref(), month.as_ref(), day.as_ref(), day_period.as_ref(),
            hour.as_ref(), minute.as_ref(), second.as_ref(), time_zone_name.as_ref()].iter().all(|field| field.is_none())
        && fractional_second_digits.is_none() {
        year = Some("numeric".into());
        month = Some("numeric".into());
        day = Some("numeric".into());
    }
    let hour_cycle_option = intl_optional_string(i, &options, "hourCycle", &["h11", "h12", "h23", "h24"])?;
    let hour12 = i.get_prop(&options, "hour12")?;
    let default_cycle = if ["en", "ar"].contains(&locale.as_str()) { "h12" } else { "h23" };
    let hour_cycle = if !hour12.is_undefined() {
        if hour12.to_boolean() { if locale == "ja" { "h11" } else { "h12" } } else { "h23" }.to_string()
    } else { hour_cycle_option.unwrap_or_else(|| default_cycle.into()) };
    let state = IntlDateState {
        locale, calendar, numbering_system, time_zone,
        date_style, time_style, weekday, era, year, month, day, day_period, hour, minute,
        second, fractional_second_digits, time_zone_name, hour_cycle,
    };
    let object = if let JsValue::Object(object) = this { object } else {
        let intl = i.global_env.resolve("Intl").unwrap();
        let constructor = i.get_prop(&intl, "DateTimeFormat")?;
        let JsValue::Object(proto) = i.get_prop(&constructor, "prototype")? else { unreachable!() };
        let mut raw = JsObject::ordinary();
        raw.proto = Some(proto);
        Obj(Rc::new(RefCell::new(raw)))
    };
    intl_register(&object, IntlState::DateTime(state));
    Ok(JsValue::Object(object))
}

#[derive(Clone, Copy)]
struct IntlCivilDate {
    year: i32, month: u32, day: u32, weekday: usize,
    hour: u32, minute: u32, second: u32, millisecond: u32,
}

fn intl_civil_date(milliseconds: f64, offset_seconds: i64) -> Option<IntlCivilDate> {
    if !milliseconds.is_finite() || milliseconds.abs() > 8.64e15 { return None; }
    let local = milliseconds.trunc() as i64 + offset_seconds * 1_000;
    let days = local.div_euclid(86_400_000);
    let day_ms = local.rem_euclid(86_400_000);
    // Civil-from-days with the Unix epoch offset (Gregorian calendar).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    Some(IntlCivilDate {
        year: year as i32, month: month as u32, day: day as u32,
        weekday: (days + 4).rem_euclid(7) as usize,
        hour: (day_ms / 3_600_000) as u32,
        minute: (day_ms / 60_000 % 60) as u32,
        second: (day_ms / 1_000 % 60) as u32,
        millisecond: (day_ms % 1_000) as u32,
    })
}

fn intl_date_data(locale: &str) -> &'static IntlDateData {
    INTL_DATE_DATA.iter().find(|data| data.locale == locale).unwrap_or(&INTL_DATE_DATA[0])
}

fn intl_date_pattern(state: &IntlDateState) -> String {
    let data = intl_date_data(&state.locale);
    let style_index = |style: &str| match style { "full" => 0, "long" => 1, "medium" => 2, _ => 3 };
    let date = state.date_style.as_ref().map(|style| data.date[style_index(style)].to_string());
    let time = state.time_style.as_ref().map(|style| data.time[style_index(style)].to_string());
    if let (Some(date), Some(time)) = (&date, &time) {
        return data.date_time[style_index(state.date_style.as_deref().unwrap())]
            .replace("{1}", date).replace("{0}", time);
    }
    if let Some(date) = date { return date; }
    if let Some(time) = time { return time; }
    let has_date = state.weekday.is_some() || state.era.is_some() || state.year.is_some()
        || state.month.is_some() || state.day.is_some();
    let has_time = state.hour.is_some() || state.minute.is_some() || state.second.is_some()
        || state.day_period.is_some() || state.fractional_second_digits.is_some();
    if !has_date && !has_time {
        let mut default = state.clone();
        default.year = Some("numeric".into());
        default.month = Some("numeric".into());
        default.day = Some("numeric".into());
        return intl_date_pattern(&default);
    }
    let mut date = String::new();
    if let Some(weekday) = &state.weekday {
        date.push_str(if weekday == "long" { "EEEE, " } else { "EEE, " });
    }
    let month = match state.month.as_deref() {
        Some("long") => "MMMM", Some("short") => "MMM", Some("narrow") => "MMMMM",
        Some("2-digit") => "MM", Some(_) => "M", None => "",
    };
    let day = match state.day.as_deref() { Some("2-digit") => "dd", Some(_) => "d", None => "" };
    let year = match state.year.as_deref() { Some("2-digit") => "yy", Some(_) => "y", None => "" };
    let textual_month = ["MMM", "MMMM", "MMMMM"].contains(&month);
    let fields: Vec<&str> = if ["de", "fr", "es", "ru"].contains(&state.locale.as_str()) {
        [day, month, year].into_iter().filter(|part| !part.is_empty()).collect()
    } else if ["ja", "zh-Hans"].contains(&state.locale.as_str()) {
        [year, month, day].into_iter().filter(|part| !part.is_empty()).collect()
    } else {
        [month, day, year].into_iter().filter(|part| !part.is_empty()).collect()
    };
    if ["en", "en-GB"].contains(&state.locale.as_str()) && textual_month {
        date.push_str(month);
        if !day.is_empty() { date.push(' '); date.push_str(day); }
        if !year.is_empty() {
            if !day.is_empty() { date.push_str(", "); } else { date.push(' '); }
            date.push_str(year);
        }
    } else {
        date.push_str(&fields.join(if ["de", "fr", "es", "ru"].contains(&state.locale.as_str()) { "." } else { "/" }));
    }
    let mut time = String::new();
    if let Some(hour) = &state.hour { time.push_str(if hour == "2-digit" { "HH" } else { "H" }); }
    if let Some(minute) = &state.minute {
        if !time.is_empty() { time.push(':'); }
        time.push_str(if minute == "2-digit" { "mm" } else { "m" });
    }
    if let Some(second) = &state.second {
        if !time.is_empty() { time.push(':'); }
        time.push_str(if second == "2-digit" { "ss" } else { "s" });
    }
    if let Some(digits) = state.fractional_second_digits {
        time.push('.');
        time.push_str(&"S".repeat(digits));
    }
    if state.day_period.is_some() {
        if !time.is_empty() { time.push(' '); }
        time.push('B');
    } else if state.hour.is_some() && ["h11", "h12"].contains(&state.hour_cycle.as_str()) {
        time.push_str(" a");
    }
    if !date.is_empty() && !time.is_empty() { format!("{date}, {time}") }
    else { format!("{date}{time}") }
}

fn intl_render_date(state: &IntlDateState, milliseconds: f64) -> Vec<(&'static str, String)> {
    let offset_seconds = (tz_offset_ns(&state.time_zone, milliseconds.trunc() as i128 * 1_000_000)
        / 1_000_000_000) as i64;
    let Some(date) = intl_civil_date(milliseconds, offset_seconds) else {
        return vec![("literal", "Invalid Date".into())];
    };
    let pattern = intl_date_pattern(state);
    let data = intl_date_data(&state.locale);
    let mut parts: Vec<(&'static str, String)> = Vec::new();
    let mut chars = pattern.chars().peekable();
    let mut literal = String::new();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.peek() == Some(&'\'') { literal.push('\''); chars.next(); }
            else { quoted = !quoted; }
            continue;
        }
        if quoted || !ch.is_ascii_alphabetic() {
            literal.push(ch);
            continue;
        }
        if !literal.is_empty() { parts.push(("literal", std::mem::take(&mut literal))); }
        let mut count = 1;
        while chars.peek() == Some(&ch) { chars.next(); count += 1; }
        let (kind, value) = match ch {
            'y' => ("year", if count == 2 { format!("{:02}", date.year.rem_euclid(100)) } else { date.year.to_string() }),
            'M' | 'L' => ("month", if count >= 4 {
                data.months_wide[(date.month - 1) as usize].to_string()
            } else if count == 3 {
                data.months_abbr[(date.month - 1) as usize].to_string()
            } else if count == 2 { format!("{:02}", date.month) } else { date.month.to_string() }),
            'd' => ("day", if count == 2 { format!("{:02}", date.day) } else { date.day.to_string() }),
            'E' | 'e' | 'c' => ("weekday", if count >= 4 {
                data.weekdays_wide[date.weekday].to_string()
            } else { data.weekdays_abbr[date.weekday].to_string() }),
            'H' | 'h' | 'K' | 'k' => {
                let hour = if ["h11", "h12"].contains(&state.hour_cycle.as_str()) {
                    let hour = date.hour % 12;
                    if state.hour_cycle == "h12" && hour == 0 { 12 } else { hour }
                } else if state.hour_cycle == "h24" && date.hour == 0 { 24 } else { date.hour };
                ("hour", if count == 2 { format!("{hour:02}") } else { hour.to_string() })
            }
            'm' => ("minute", if count == 2 { format!("{:02}", date.minute) } else { date.minute.to_string() }),
            's' => ("second", if count == 2 { format!("{:02}", date.second) } else { date.second.to_string() }),
            'S' => ("fractionalSecond", format!("{:03}", date.millisecond)[..count.min(3)].to_string()),
            'a' | 'b' => ("dayPeriod", if date.hour < 12 { data.am.to_string() } else { data.pm.to_string() }),
            'B' => ("dayPeriod", match state.day_period.as_deref() {
                Some("narrow") => data.day_period_narrow[date.hour as usize],
                Some("short") => data.day_period_short[date.hour as usize],
                _ => data.day_period_long[date.hour as usize],
            }.to_string()),
            'G' => ("era", if date.year > 0 { "AD".into() } else { "BC".into() }),
            'z' | 'Z' | 'O' | 'v' | 'V' | 'X' | 'x' => ("timeZoneName", if offset_seconds == 0 {
                "UTC".into()
            } else {
                let minutes = offset_seconds.unsigned_abs() / 60;
                format!("GMT{}{:02}:{:02}", if offset_seconds < 0 { '-' } else { '+' }, minutes / 60, minutes % 60)
            }),
            _ => ("literal", ch.to_string().repeat(count)),
        };
        parts.push((kind, if ["year", "month", "day", "hour", "minute", "second", "fractionalSecond"].contains(&kind) {
            intl_localize_digits(&value, &state.numbering_system)
        } else { value }));
    }
    if !literal.is_empty() { parts.push(("literal", literal)); }
    parts
}

fn intl_date_value(i: &mut Interpreter, value: Option<&JsValue>) -> Result<f64, Throw> {
    match value {
        Some(JsValue::Object(object)) if object.borrow().get_own("[[DateValue]]").is_some() => {
            let stored = object.borrow().get_own("[[DateValue]]").unwrap();
            i.to_number(&stored)
        }
        Some(value) => i.to_number(value),
        None => Ok(0.0),
    }
}

fn intl_date_format_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(&this)?;
    let function = Interpreter::native("format", intl_date_bound_format, &i.function_proto);
    let JsValue::Object(object) = &function else { unreachable!() };
    object.borrow_mut().constructible = false;
    object.borrow_mut().delete_own("prototype");
    intl_register(object, IntlState::BoundDateTime(state));
    Ok(function)
}

fn intl_date_bound_format(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let function = i.native_stack.last().ok_or_else(|| Throw::type_error("missing bound DateTimeFormat"))?;
    let state = match intl_state(function) {
        Some(IntlState::BoundDateTime(state)) => state,
        _ => return Err(Throw::type_error("invalid bound DateTimeFormat")),
    };
    let timestamp = intl_date_value(i, args.first())?;
    let parts = intl_render_date(&state, timestamp);
    Ok(JsValue::str(parts.iter().map(|(_, value)| value.as_str()).collect::<String>()))
}

fn intl_date_format_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(&this)?;
    let timestamp = intl_date_value(i, args.first())?;
    intl_parts_array(i, intl_render_date(&state, timestamp))
}

fn intl_date_format_range(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(&this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRange requires two dates"));
    }
    let start = intl_date_value(i, args.first())?;
    let end = intl_date_value(i, args.get(1))?;
    if !start.is_finite() || !end.is_finite() { return Err(Throw::range_error("invalid date range")); }
    let parts = intl_date_range_parts(&state, start, end);
    Ok(JsValue::str(parts.iter().map(|(_, value, _)| value.as_str()).collect::<String>()))
}

fn intl_date_format_range_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(&this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRangeToParts requires two dates"));
    }
    let start = intl_date_value(i, args.first())?;
    let end = intl_date_value(i, args.get(1))?;
    if !start.is_finite() || !end.is_finite() { return Err(Throw::range_error("invalid date range")); }
    let mut values = Vec::new();
    for (kind, value, source) in intl_date_range_parts(&state, start, end) {
        let part = i.object_value();
        i.set_prop(&part, "type", JsValue::str(kind))?;
        i.set_prop(&part, "value", JsValue::str(value))?;
        i.set_prop(&part, "source", JsValue::str(source))?;
        values.push(part);
    }
    intl_array(i, values)
}

fn intl_date_range_parts(state: &IntlDateState, start: f64, end: f64) -> Vec<(&'static str, String, &'static str)> {
    let left = intl_render_date(state, start);
    let right = intl_render_date(state, end);
    if left == right {
        return left.into_iter().map(|(kind, value)| (kind, value, "shared")).collect();
    }
    let textual_month = ["long", "short", "narrow"].contains(&state.month.as_deref().unwrap_or(""));
    let mut prefix = 0;
    let mut suffix = 0;
    if textual_month {
        while prefix < left.len().min(right.len()) && left[prefix] == right[prefix] { prefix += 1; }
        while suffix < left.len().min(right.len()) - prefix
            && left[left.len() - suffix - 1] == right[right.len() - suffix - 1] { suffix += 1; }
    }
    let mut parts = Vec::new();
    parts.extend(left[..prefix].iter().cloned().map(|(kind, value)| (kind, value, "shared")));
    parts.extend(left[prefix..left.len() - suffix].iter().cloned().map(|(kind, value)| (kind, value, "startRange")));
    parts.push(("literal", " – ".into(), "shared"));
    parts.extend(right[prefix..right.len() - suffix].iter().cloned().map(|(kind, value)| (kind, value, "endRange")));
    parts.extend(left[left.len() - suffix..].iter().cloned().map(|(kind, value)| (kind, value, "shared")));
    parts
}

fn intl_date_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(&this)?;
    let object = i.object_value();
    for (key, value) in [
        ("locale", &state.locale), ("calendar", &state.calendar),
        ("numberingSystem", &state.numbering_system), ("timeZone", &state.time_zone),
        ("hourCycle", &state.hour_cycle),
    ] { i.set_prop(&object, key, JsValue::str(value))?; }
    i.set_prop(&object, "hour12", JsValue::Bool(["h11", "h12"].contains(&state.hour_cycle.as_str())))?;
    for (key, value) in [
        ("dateStyle", &state.date_style), ("timeStyle", &state.time_style),
        ("weekday", &state.weekday), ("era", &state.era), ("year", &state.year),
        ("month", &state.month), ("day", &state.day), ("dayPeriod", &state.day_period),
        ("hour", &state.hour), ("minute", &state.minute), ("second", &state.second),
        ("timeZoneName", &state.time_zone_name),
    ] {
        if let Some(value) = value { i.set_prop(&object, key, JsValue::str(value))?; }
    }
    if let Some(value) = state.fractional_second_digits {
        i.set_prop(&object, "fractionalSecondDigits", JsValue::num(value as f64))?;
    }
    Ok(object)
}

#[cfg(test)]
mod intl_date_tests {
    use super::*;

    #[test]
    fn gregorian_styles_and_fixed_offsets() {
        let mut i = Interpreter::new();
        let value = i.run_script("let a=new Intl.DateTimeFormat('en',{dateStyle:'short',timeZone:'UTC'}).format(0); let b=new Intl.DateTimeFormat('en',{timeStyle:'short',timeZone:'+05:30'}).format(0); a+'|'+b").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "1/1/70|5:30 AM");
    }

    #[test]
    fn iana_zone_observes_dst_transition() {
        let mut i = Interpreter::new();
        let value = i.run_script("let d=new Intl.DateTimeFormat('en',{timeZone:'America/New_York',hour:'numeric',minute:'2-digit',hourCycle:'h23'}); d.format(1710053940000)+'|'+d.format(1710054000000)").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "1:59|3:00");
    }

    #[test]
    fn range_parts_mark_shared_and_distinct_fields() {
        let mut i = Interpreter::new();
        let value = i.run_script("let d=new Intl.DateTimeFormat('en',{timeZone:'UTC',year:'numeric',month:'short',day:'numeric'}); d.formatRangeToParts(1546473600000,1546646400000).map(p=>p.type+':'+p.source).join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "month:shared|literal:shared|day:startRange|literal:shared|day:endRange|literal:shared|year:shared");
    }
}
