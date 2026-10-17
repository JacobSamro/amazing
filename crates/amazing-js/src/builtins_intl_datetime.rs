// Portions ported from lucid-softworks/lumen (crates/lumen/src/intl/datetimeformat.rs, non-Gregorian calendar fields, era names,
// chinese/dangi related-year cluster) @ a0f0314de7835f76eaa913995489945aabebf345, MIT.
// Gregorian Intl.DateTimeFormat with UTC and fixed-offset time zones.

#[derive(Clone)]
struct IntlDateState {
    locale: String,
    resolved_locale: String,
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
    // True when year/month/day were synthesized by the constructor's "no
    // component requested at all" fallback rather than asked for by the
    // caller. A Temporal kind that can't supply year/month/day (PlainTime)
    // drops them either way, but only an *explicit* request with no
    // remaining overlap is a TypeError -- see intl_temporal_adapt_state.
    ymd_defaulted: bool,
    // Plain Temporal values have no zone: a timeStyle's zone name is dropped from the pattern.
    omit_time_zone: bool,
}

fn intl_optional_string(i: &mut Interpreter, options: &JsValue, key: &str, allowed: &[&str]) -> Result<Option<String>, Throw> {
    let value = i.get_prop(options, key)?;
    if matches!(value, JsValue::Undefined) { return Ok(None); }
    let value = i.to_string(&value)?;
    if !allowed.contains(&value.as_str()) { return Err(Throw::range_error(format!("invalid {key} option"))); }
    Ok(Some(value))
}

fn intl_date_state(i: &mut Interpreter, this: &JsValue) -> Result<IntlDateState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible DateTimeFormat receiver")); };
    match intl_unwrap_state(i, object, "DateTimeFormat")? {
        Some(IntlState::DateTime(state)) | Some(IntlState::BoundDateTime(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible DateTimeFormat receiver")),
    }
}

fn intl_now_milliseconds() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as f64).unwrap_or(0.0)
}

fn intl_timezone(text: &str) -> Result<String, Throw> {
    tz_canonicalize(text).ok_or_else(|| Throw::range_error("invalid time zone"))
}

impl Interpreter {
    fn install_intl_datetime(&mut self, namespace: &Obj) {
        let constructor = Self::native_constructor("DateTimeFormat", intl_date_ctor, &self.function_proto);
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
        intl_register_intrinsic(self, "DateTimeFormat", function, &prototype);
        let _ = namespace.borrow_mut().define("DateTimeFormat", binary_writable_property(constructor));
    }
}

fn intl_calendar_supported(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let canonical: &str = match lower.as_str() {
        "gregorian" => "gregory", "islamicc" => "islamic-civil",
        "ethiopic-amete-alem" => "ethioaa", other => other,
    };
    INTL_KNOWN_CALENDARS.contains(&canonical)
}

thread_local! {
    // Date.prototype.toLocale{,Date,Time}String hand their ToDateTimeOptions
    // (required, defaults) pair to the DateTimeFormat intrinsic through this slot.
    static INTL_DATE_MODE: std::cell::Cell<Option<(&'static str, &'static str)>> = const { std::cell::Cell::new(None) };
}

fn intl_date_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mode = INTL_DATE_MODE.with(|slot| slot.take());
    let state = intl_date_create_state(i, args.first().unwrap_or(&JsValue::Undefined),
        args.get(1).unwrap_or(&JsValue::Undefined), mode)?;
    intl_chain_construct(i, this, "DateTimeFormat", IntlState::DateTime(state))
}

// CreateDateTimeFormat. `mode` is the Temporal `toLocaleString` (required,
// defaults) pair ("date" | "time" | "any", and "date" | "time" | "all" |
// "year-month" | "month-day"); None is the plain Intl.DateTimeFormat
// constructor, whose (any, date) defaulting keeps its own historical rule.
fn intl_date_create_state(i: &mut Interpreter, locales: &JsValue, options_arg: &JsValue,
    mode: Option<(&str, &str)>) -> Result<IntlDateState, Throw> {
    let requested = intl_canonical_locale_list(i, locales)?;
    let options = Some(options_arg).filter(|value| !matches!(value, JsValue::Undefined)).cloned()
        .unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null DateTimeFormat options")); }
    let matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let calendar_raw = i.get_prop(&options, "calendar")?;
    let calendar_present = !calendar_raw.is_undefined();
    let mut calendar_text: Option<String> = None;
    if calendar_present {
        let value = i.to_string(&calendar_raw)?;
        calendar_text = Some(value.clone());
        if value.split('-').any(|part| !(3..=8).contains(&part.len())
            || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
            return Err(Throw::range_error("invalid calendar"));
        }
    }
    // A well-formed but unavailable calendar is ignored; the requested -u-ca
    // extension or the locale default is used instead.
    let ca_extension = requested.first().and_then(|tag| intl_unicode_keyword(tag, "ca"));
    let ca_option = calendar_text;
    let (ca_retained, calendar_effective) = intl_resolve_keyword(
        ca_extension, ca_option, intl_calendar_supported, "gregory");
    let calendar = match calendar_effective.to_ascii_lowercase().as_str() {
        "gregorian" | "gregory" => "gregory", "islamicc" | "islamic-civil" => "islamic-civil",
        "ethiopic-amete-alem" | "ethioaa" => "ethioaa", other => other,
    }.to_string();
    // intl_lookup has no observable side effects, so resolving it here
    // (after localeMatcher is read, once its bool is known) rather than at
    // the top does not change getter-call order for any other option.
    let locale = intl_lookup(&requested, matcher == "best fit");
    let mut numbering_default = intl_data(&locale).numbering_system;
    if let Some(tag) = requested.first() {
        // CLDR: Arabic regions other than the Maghreb/UAE default to Arabic-Indic digits.
        let mut subtags = tag.split('-');
        if subtags.next() == Some("ar") {
            if let Some(region) = subtags.find(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_uppercase())) {
                if !["AE", "DZ", "EH", "LY", "MA", "TN"].contains(&region) { numbering_default = "arab"; }
            }
        }
    }
    let numbering_raw = i.get_prop(&options, "numberingSystem")?;
    let numbering_option = if numbering_raw.is_undefined() { None } else { Some(i.to_string(&numbering_raw)?) };
    if let Some(value) = numbering_option.as_ref() {
        if !intl_is_well_formed_type(value) {
            return Err(Throw::range_error("invalid numbering system"));
        }
    }
    let nu_extension = requested.first().and_then(|tag| intl_unicode_keyword(tag, "nu"));
    let (nu_retained, numbering_system) =
        intl_resolve_keyword(nu_extension, numbering_option, intl_supported_numbering_system, numbering_default);
    let hour12 = i.get_prop(&options, "hour12")?;
    let hour_cycle_option = intl_optional_string(i, &options, "hourCycle", &["h11", "h12", "h23", "h24"])?;
    let time_zone_option = intl_string_option(i, &options, "timeZone", &[], "UTC")?;
    let time_zone = intl_timezone(&time_zone_option)?;
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
        else { Some(intl_integer_from_value(i, &fractional_second_value, "fractionalSecondDigits", 1, 3, 1)?) };
    let time_zone_name = intl_optional_string(i, &options, "timeZoneName",
        &["short", "long", "shortOffset", "longOffset", "shortGeneric", "longGeneric"])?;
    let _format_matcher = intl_string_option(i, &options, "formatMatcher", &["basic", "best fit"], "best fit")?;
    let date_style = intl_optional_string(i, &options, "dateStyle", &["full", "long", "medium", "short"])?;
    let time_style = intl_optional_string(i, &options, "timeStyle", &["full", "long", "medium", "short"])?;
    if (date_style.is_some() || time_style.is_some())
        && [weekday.as_ref(), era.as_ref(), year.as_ref(), month.as_ref(), day.as_ref(), day_period.as_ref(),
            hour.as_ref(), minute.as_ref(), second.as_ref(), time_zone_name.as_ref()].iter().any(|field| field.is_some())
            || (date_style.is_some() || time_style.is_some()) && fractional_second_digits.is_some()
    {
        return Err(Throw::type_error("dateStyle/timeStyle conflicts with components"));
    }
    // Unlike the conflict check above, timeZoneName is deliberately excluded
    // here: requesting timeZoneName alone (no dateStyle/timeStyle, no other
    // component) still gets the year/month/day numeric default, it doesn't
    // count as "something else was requested" -- see the intl402/
    // DateTimeFormat/prototype/*/temporal-plain*-formatting-timezonename.js
    // tests, which construct exactly that and expect a non-throwing result.
    let mut hour = hour;
    let mut minute = minute;
    let mut second = second;
    let ymd_defaulted = match mode {
        None => date_style.is_none() && time_style.is_none()
            && [weekday.as_ref(), year.as_ref(), month.as_ref(), day.as_ref(), day_period.as_ref(),
                hour.as_ref(), minute.as_ref(), second.as_ref()].iter().all(|field| field.is_none())
            && fractional_second_digits.is_none(),
        Some((required, defaults)) => {
            if (required == "date" && time_style.is_some()) || (required == "time" && date_style.is_some()) {
                return Err(Throw::type_error("dateStyle/timeStyle does not apply to this Temporal type"));
            }
            let mut need = date_style.is_none() && time_style.is_none();
            if matches!(required, "date" | "any")
                && [weekday.as_ref(), year.as_ref(), month.as_ref(), day.as_ref()].iter().any(|f| f.is_some()) {
                need = false;
            }
            if matches!(required, "time" | "any")
                && ([day_period.as_ref(), hour.as_ref(), minute.as_ref(), second.as_ref()].iter().any(|f| f.is_some())
                    || fractional_second_digits.is_some()) {
                need = false;
            }
            if need {
                let numeric = || Some("numeric".to_string());
                if matches!(defaults, "date" | "all") { year = numeric(); month = numeric(); day = numeric(); }
                if defaults == "year-month" { year = numeric(); month = numeric(); }
                if defaults == "month-day" { month = numeric(); day = numeric(); }
                if matches!(defaults, "time" | "all") { hour = numeric(); minute = numeric(); second = numeric(); }
            }
            need
        }
    };
    if mode.is_none() && ymd_defaulted {
        year = Some("numeric".into());
        month = Some("numeric".into());
        day = Some("numeric".into());
    }
    let hc_extension = requested.first().and_then(|tag| intl_unicode_keyword(tag, "hc"))
        .filter(|value| ["h11", "h12", "h23", "h24"].contains(&value.as_str()));
    let default_cycle = if ["en", "ar", "ko", "zh-Hant"].contains(&locale.as_str()) { "h12" } else { "h23" };
    // An explicit hour12 always overrides (and drops) the hc extension.
    let (hc_retained, hour_cycle) = if !hour12.is_undefined() {
        (None, if hour12.to_boolean() { if locale == "ja" { "h11" } else { "h12" } } else { "h23" }.to_string())
    } else {
        intl_resolve_keyword(hc_extension, hour_cycle_option,
            |value| ["h11", "h12", "h23", "h24"].contains(&value), default_cycle)
    };
    let mut retained: Vec<(&str, String)> = Vec::new();
    if let Some(value) = ca_retained { retained.push(("ca", value)); }
    if let Some(value) = hc_retained { retained.push(("hc", value)); }
    if let Some(value) = nu_retained { retained.push(("nu", value)); }
    let resolved_locale = intl_locale_with_extensions(&locale, &retained);
    Ok(IntlDateState {
        locale, resolved_locale, calendar, numbering_system, time_zone,
        date_style, time_style, weekday, era, year, month, day, day_period, hour, minute,
        second, fractional_second_digits, time_zone_name, hour_cycle, ymd_defaulted, omit_time_zone: false,
    })
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

// Removes the zone field (and the brackets/parentheses/spaces around it) from a CLDR time pattern.
fn intl_strip_zone(pattern: &str) -> String {
    let mut out = String::new();
    for c in pattern.chars() { if c != 'z' { out.push(c); } }
    let out = out.replace("()", "").replace("[]", "");
    out.trim_matches(|c: char| c == ' ' || c == '\u{202f}').to_string()
}

// Rewrites a CLDR time pattern written for the locale's default hour cycle into the requested one.
fn intl_adapt_hour_cycle(pattern: &str, cycle: &str) -> String {
    let is_24 = matches!(cycle, "h23" | "h24");
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    if is_24 && pattern.contains('h') {
        while index < chars.len() {
            let c = chars[index];
            if c == 'h' {
                while index < chars.len() && chars[index] == 'h' { index += 1; }
                out.push_str("HH");
                continue;
            }
            if c == 'a' {
                if matches!(out.chars().last(), Some(' ') | Some('\u{202f}')) { out.pop(); }
                else if matches!(chars.get(index + 1), Some(' ') | Some('\u{202f}')) { index += 1; }
                index += 1;
                continue;
            }
            out.push(c);
            index += 1;
        }
        out
    } else if !is_24 && pattern.contains('H') && !pattern.contains('a') {
        let mut last_digits = 0;
        while index < chars.len() {
            let c = chars[index];
            if c == 'H' {
                while index < chars.len() && chars[index] == 'H' { index += 1; }
                out.push('h');
                continue;
            }
            out.push(c);
            index += 1;
            if c == 's' || (c == 'm' && !pattern.contains('s')) {
                while index < chars.len() && chars[index] == c { out.push(c); index += 1; }
                last_digits = out.len();
            }
        }
        if last_digits > 0 { out.insert_str(last_digits, "\u{202f}a"); } else { out.push_str("\u{202f}a"); }
        out
    } else {
        pattern.to_string()
    }
}

fn intl_date_pattern(state: &IntlDateState) -> String {
    let data = intl_date_data(&state.locale);
    let style_index = |style: &str| match style { "full" => 0, "long" => 1, "medium" => 2, _ => 3 };
    let date = state.date_style.as_ref().map(|style| data.date[style_index(style)].to_string());
    let time = state.time_style.as_ref().map(|style| {
        let pattern = intl_adapt_hour_cycle(data.time[style_index(style)], &state.hour_cycle);
        if state.omit_time_zone { intl_strip_zone(&pattern) } else { pattern }
    });
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
    let fields: Vec<&str> = if ["de", "fr", "es", "ru", "sr"].contains(&state.locale.as_str()) {
        [day, month, year].into_iter().filter(|part| !part.is_empty()).collect()
    } else if ["ja", "zh", "zh-Hans", "zh-Hant", "ko"].contains(&state.locale.as_str()) {
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
        date.push_str(&fields.join(if ["de", "fr", "es", "ru", "sr"].contains(&state.locale.as_str()) { "." } else { "/" }));
    }
    if let Some(era) = &state.era {
        if state.calendar != "chinese" && state.calendar != "dangi" {
            let token = match era.as_str() { "long" => "GGGG", "narrow" => "GGGGG", _ => "G" };
            if ["ja", "zh", "zh-Hans", "zh-Hant", "ko"].contains(&state.locale.as_str()) {
                date = format!("{token}{date}");
            } else if date.is_empty() {
                date.push_str(token);
            } else {
                date.push(' ');
                date.push_str(token);
            }
        }
    }
    let mut time = String::new();
    if let Some(hour) = &state.hour {
        // CLDR 24-hour skeletons ("Hms", "Hm") use a two-digit hour in every
        // locale this table covers except Japanese.
        let two_digit_24h = ["h23", "h24"].contains(&state.hour_cycle.as_str()) && state.locale != "ja";
        time.push_str(if hour == "2-digit" || two_digit_24h { "HH" } else { "H" });
    }
    if let Some(minute) = &state.minute {
        if !time.is_empty() { time.push(':'); }
        // ECMA-402 keeps minute and second two-digit whenever they are part of
        // a time pattern, regardless of the requested "numeric" width.
        time.push_str("mm");
        let _ = minute;
    }
    if let Some(second) = &state.second {
        if !time.is_empty() { time.push(':'); }
        time.push_str("ss");
        let _ = second;
    }
    if let Some(digits) = state.fractional_second_digits {
        time.push(if matches!(state.numbering_system.as_str(), "arab" | "arabext") { '\u{66b}' } else { '.' });
        time.push_str(&"S".repeat(digits));
    }
    // CLDR's "hh" skeleton keeps an ordinary space before the day period; the others use U+202F.
    let period_space = if ["en", "en-GB", "hi"].contains(&state.locale.as_str()) && state.hour.as_deref() != Some("2-digit") { '\u{202f}' } else { ' ' };
    if state.day_period.is_some() {
        if !time.is_empty() { time.push(' '); }
        time.push('B');
    } else if state.hour.is_some() && ["h11", "h12"].contains(&state.hour_cycle.as_str()) {
        time.push(period_space);
        time.push('a');
    }
    // timeZoneName can never combine with dateStyle/timeStyle (rejected at
    // construction), so it only needs adding to the manually-built pattern
    // here; any 'z' letter renders identically at the char-loop below
    // regardless of the requested width.
    if state.time_zone_name.is_some() {
        if !time.is_empty() { time.push(' '); time.push('z'); }
        else { if !date.is_empty() { date.push(' '); } date.push('z'); }
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
    intl_render_civil(state, date, offset_seconds)
}

// Days from 1970-01-01 in the proleptic Gregorian calendar; the exact
// inverse of intl_civil_date's civil-from-days decoding (same era/yoe/doy
// constants), so it round-trips correctly at the extreme Temporal PlainDate
// bounds (-271821-04-19 .. +275760-09-13), which fall outside the range a
// legacy Date/millisecond value can represent -- see
// temporal-objects-no-time-clip-weekday.js.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// Which Temporal Plain* type a value was recognised as; drives which
// DateTimeFormat fields apply (see intl_temporal_adapt_state) and which
// fields intl_civil_date_from_temporal reads out of the shared 10-field
// layout ([[year, month, day, hour, minute, second, ms, us, ns, refYear]]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum IntlTemporalKind { Date, Time, DateTime, YearMonth, MonthDay }

fn intl_civil_date_from_temporal(kind: IntlTemporalKind, fields: &[i64; 10]) -> IntlCivilDate {
    let (year, month, day, hour, minute, second, millisecond) = match kind {
        IntlTemporalKind::Date => (fields[0], fields[1], fields[2], 0, 0, 0, 0),
        IntlTemporalKind::Time => (1970, 1, 1, fields[3], fields[4], fields[5], fields[6]),
        IntlTemporalKind::DateTime => (fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6]),
        // YearMonth/MonthDay carry a placeholder reference day/year (never
        // rendered, since intl_temporal_adapt_state always drops the field
        // the kind doesn't have).
        IntlTemporalKind::YearMonth => (fields[0], fields[1], fields[2].max(1), 0, 0, 0, 0),
        IntlTemporalKind::MonthDay => (1972, fields[1], fields[2], 0, 0, 0, 0),
    };
    let days = days_from_civil(year, month, day);
    IntlCivilDate {
        year: year as i32, month: month as u32, day: day as u32,
        weekday: (days + 4).rem_euclid(7) as usize,
        hour: hour as u32, minute: minute as u32, second: second as u32, millisecond: millisecond as u32,
    }
}

// HandleDateTimeValue's internal-slot dispatch: try each Temporal Plain*
// kind's exact internal slot (mirroring "x has an [[InitializedTemporalX]]
// internal slot") without ever invoking a prototype getter. Instant and
// ZonedDateTime are not implemented yet (see .agents/js-intl/questions.md),
// so those values fall through to the legacy ToNumber path below, same as
// any other non-Date object -- correct for ZonedDateTime (format() must
// throw TypeError for it) but not yet reachable for Instant.
fn intl_temporal_lookup(value: &JsValue) -> Option<(IntlTemporalKind, IntlCivilDate, String)> {
    for (std_kind, kind) in [
        (StdTemporalKind::PlainDateTime, IntlTemporalKind::DateTime),
        (StdTemporalKind::PlainDate, IntlTemporalKind::Date),
        (StdTemporalKind::PlainTime, IntlTemporalKind::Time),
        (StdTemporalKind::PlainYearMonth, IntlTemporalKind::YearMonth),
        (StdTemporalKind::PlainMonthDay, IntlTemporalKind::MonthDay),
    ] {
        if let Ok(record) = std_temporal_record(value, Some(std_kind)) {
            return Some((kind, intl_civil_date_from_temporal(kind, &record.fields), record.calendar.clone()));
        }
    }
    None
}

// Drops every DateTimeFormat field the Temporal kind has no data for (a
// PlainDate has no hour, a PlainYearMonth has no day, no Plain* type has a
// time zone since [[IsPlain]] is true for all of them) and throws TypeError
// if nothing requested survives -- e.g. a timeStyle-only formatter has no
// field a PlainDate can supply. Matches
// temporal-objects-not-overlapping-options.js's per-field "no overlap"
// behaviour (drop, don't throw, as long as *something* still overlaps).
fn intl_temporal_adapt_state(state: &IntlDateState, kind: IntlTemporalKind) -> Result<IntlDateState, Throw> {
    let has_date = !matches!(kind, IntlTemporalKind::Time);
    let has_time = matches!(kind, IntlTemporalKind::Time | IntlTemporalKind::DateTime);
    let has_weekday = matches!(kind, IntlTemporalKind::Date | IntlTemporalKind::DateTime);
    let has_era_year = matches!(kind, IntlTemporalKind::Date | IntlTemporalKind::DateTime | IntlTemporalKind::YearMonth);
    let has_month = has_date;
    let has_day = matches!(kind, IntlTemporalKind::Date | IntlTemporalKind::DateTime | IntlTemporalKind::MonthDay);
    let mut adapted = state.clone();
    if state.ymd_defaulted {
        // Nothing was asked for explicitly: each Temporal type gets its own
        // defaults (PlainDate: date, PlainTime: time, PlainDateTime: both,
        // PlainYearMonth: year+month, PlainMonthDay: month+day) rather than
        // the Date-oriented year/month/day the constructor synthesized.
        for field in [&mut adapted.year, &mut adapted.month, &mut adapted.day,
            &mut adapted.hour, &mut adapted.minute, &mut adapted.second] { *field = None; }
        let numeric = || Some("numeric".to_string());
        match kind {
            IntlTemporalKind::Date => { adapted.year = numeric(); adapted.month = numeric(); adapted.day = numeric(); }
            IntlTemporalKind::Time => { adapted.hour = numeric(); adapted.minute = numeric(); adapted.second = numeric(); }
            IntlTemporalKind::DateTime => {
                adapted.year = numeric(); adapted.month = numeric(); adapted.day = numeric();
                adapted.hour = numeric(); adapted.minute = numeric(); adapted.second = numeric();
            }
            IntlTemporalKind::YearMonth => { adapted.year = numeric(); adapted.month = numeric(); }
            IntlTemporalKind::MonthDay => { adapted.month = numeric(); adapted.day = numeric(); }
        }
    }
    adapted.time_zone_name = None;
    adapted.omit_time_zone = true;
    if matches!(kind, IntlTemporalKind::YearMonth | IntlTemporalKind::MonthDay) {
        // A dateStyle on a partial date expands to the matching year-month / month-day skeleton.
        if let Some(style) = adapted.date_style.take() {
            let month = match style.as_str() { "full" | "long" => "long", "medium" => "short", _ => "numeric" };
            adapted.month = Some(month.to_string());
            if kind == IntlTemporalKind::YearMonth {
                adapted.year = Some(if style == "short" { "2-digit" } else { "numeric" }.to_string());
            } else {
                adapted.day = Some("numeric".to_string());
            }
        }
    }
    if !has_date { adapted.date_style = None; }
    if !has_time { adapted.time_style = None; }
    if !has_weekday { adapted.weekday = None; }
    if !has_era_year { adapted.era = None; adapted.year = None; }
    if !has_month { adapted.month = None; }
    if !has_day { adapted.day = None; }
    if !has_time {
        adapted.hour = None; adapted.minute = None; adapted.second = None;
        adapted.day_period = None; adapted.fractional_second_digits = None;
    }
    let has_any = adapted.date_style.is_some() || adapted.time_style.is_some()
        || adapted.weekday.is_some() || adapted.era.is_some() || adapted.year.is_some()
        || adapted.month.is_some() || adapted.day.is_some() || adapted.day_period.is_some()
        || adapted.hour.is_some() || adapted.minute.is_some() || adapted.second.is_some()
        || adapted.fractional_second_digits.is_some();
    if !has_any {
        // The user never explicitly asked for anything this kind lacks --
        // year/month/day were only present because the constructor
        // synthesizes them when nothing else is requested. Render whatever
        // is left (possibly nothing) instead of throwing; only an
        // *explicit* request with no overlap is a TypeError. See
        // temporal-plain{time,date,datetime}-formatting-timezonename.js,
        // which request only timeZoneName (always dropped for Plain*
        // kinds) and must not throw.
        if state.ymd_defaulted { return Ok(adapted); }
        return Err(Throw::type_error("no overlap between DateTimeFormat options and the Temporal object"));
    }
    Ok(adapted)
}

// A value handed to format/formatToParts/formatRange/formatRangeToParts:
// either the legacy Date/number/undefined("now") path (unchanged), or one
// of the five implemented Temporal Plain* kinds, carrying its own
// already-computed civil date (never routed through millisecond TimeClip,
// so it works past the legacy Date range -- see days_from_civil above).
enum IntlDateInput { Legacy(f64), Instant(f64), Temporal(IntlTemporalKind, IntlCivilDate, String), Zoned }

fn intl_date_input(i: &mut Interpreter, value: Option<&JsValue>) -> Result<IntlDateInput, Throw> {
    if let Some(value @ JsValue::Object(_)) = value {
        // HandleDateTimeValue: a Temporal.Instant formats as its epoch time
        // (ZonedDateTime deliberately falls through to the valueOf TypeError).
        if let Ok(record) = std_epoch_record(value, Some(StdEpochKind::Instant)) {
            return Ok(IntlDateInput::Instant(record.epoch_ns.div_euclid(1_000_000) as f64));
        }
        if let Some((kind, date, calendar)) = intl_temporal_lookup(value) {
            return Ok(IntlDateInput::Temporal(kind, date, calendar));
        }
    }
    intl_date_value(i, value).map(IntlDateInput::Legacy)
}

// formatRange's endpoints: HandleDateTimeValue defers TimeClip until both argument kinds are known to match.
fn intl_date_input_range(i: &mut Interpreter, value: Option<&JsValue>) -> Result<IntlDateInput, Throw> {
    if let Some(value @ JsValue::Object(_)) = value {
        // A ZonedDateTime is only rejected once both endpoints have been converted.
        if std_epoch_record(value, Some(StdEpochKind::ZonedDateTime)).is_ok() { return Ok(IntlDateInput::Zoned); }
        if let Ok(record) = std_epoch_record(value, Some(StdEpochKind::Instant)) {
            return Ok(IntlDateInput::Instant(record.epoch_ns.div_euclid(1_000_000) as f64));
        }
        if let Some((kind, date, calendar)) = intl_temporal_lookup(value) {
            return Ok(IntlDateInput::Temporal(kind, date, calendar));
        }
    }
    intl_date_value_raw(i, value).map(IntlDateInput::Legacy)
}

fn intl_render_civil(state: &IntlDateState, date: IntlCivilDate, offset_seconds: i64) -> Vec<(&'static str, String)> {
    let pattern = intl_date_pattern(state);
    let data = intl_date_data(&state.locale);
    let mut parts: Vec<(&'static str, String)> = Vec::new();
    let mut chars = pattern.chars().peekable();
    let mut literal = String::new();
    let mut quoted = false;
    let calendar_fields = intl_calendar_fields(&state.calendar, date.year as i64, date.month as i64, date.day as i64);
    let foreign = intl_calendar_is_foreign(&state.calendar);
    let era_shown = state.era.is_some();
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
        if ch == 'y' && calendar_fields.related_year.is_some() {
            let related = calendar_fields.related_year.unwrap_or(0);
            let name = intl_cycle_year_name(related, &state.locale);
            let chinese = intl_cldr_locale(&state.locale).starts_with("zh");
            parts.push(("relatedYear", intl_localize_digits(&related.to_string(), &state.numbering_system)));
            if chinese {
                parts.push(("yearName", name));
                if chars.peek() != Some(&'年') { parts.push(("literal", "年".into())); }
            } else {
                parts.push(("literal", "(".into()));
                parts.push(("yearName", name));
                parts.push(("literal", ")".into()));
            }
            continue;
        }
        let display_year = if foreign { calendar_fields.year }
            else if era_shown && date.year <= 0 { 1 - date.year as i64 } else { date.year as i64 };
        let named_month = if (count >= 3 && foreign) || state.calendar == "hebrew" {
            let width = match count { 3 => "short", 4 => "long", _ => "narrow" };
            let width = if state.calendar == "hebrew" { "long" } else { width };
            if calendar_fields.adar_ii && intl_cldr_locale(&state.locale) == "en" { Some("Adar II") }
            else { intl_cldr_month_name(&state.locale, &state.calendar, width, calendar_fields.month) }
        } else { None };
        let numeric_month = if foreign { calendar_fields.month } else { date.month as i64 };
        let numeric_day = if foreign { calendar_fields.day } else { date.day as i64 };
        let (kind, value) = match ch {
            'y' => ("year", if count == 2 { format!("{:02}", display_year.rem_euclid(100)) } else { display_year.to_string() }),
            'M' | 'L' if named_month.is_some() => ("month", named_month.unwrap_or("").to_string()),
            'M' | 'L' if foreign => ("month", {
                let bis = if calendar_fields.leap_month { "bis" } else { "" };
                if count == 2 { format!("{:02}{bis}", numeric_month) } else { format!("{numeric_month}{bis}") }
            }),
            'M' | 'L' => ("month", if count >= 4 {
                data.months_wide[(date.month - 1) as usize].to_string()
            } else if count == 3 {
                data.months_abbr[(date.month - 1) as usize].to_string()
            } else if count == 2 { format!("{:02}", date.month) } else { date.month.to_string() }),
            'd' => ("day", if count == 2 { format!("{:02}", numeric_day) } else { numeric_day.to_string() }),
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
            'G' => ("era", {
                let width = match count { 4 => "long", 5 => "narrow", _ => "short" };
                let fallback = if date.year > 0 { "AD" } else { "BC" };
                match &calendar_fields.era {
                    Some((key, era)) if foreign || state.calendar == "gregory" =>
                        intl_cldr_era_name(&state.locale, key, width, era).unwrap_or(fallback).to_string(),
                    _ => fallback.to_string(),
                }
            }),
            'z' | 'Z' | 'O' | 'v' | 'V' | 'X' | 'x' => ("timeZoneName", {
                let minutes = offset_seconds.unsigned_abs() / 60;
                let sign = if offset_seconds < 0 { '-' } else { '+' };
                let name = state.time_zone_name.as_deref();
                let utc_zone = matches!(state.time_zone.as_str(), "UTC" | "Etc/UTC" | "Etc/GMT" | "GMT");
                if name == Some("shortOffset") {
                    if offset_seconds == 0 { "GMT".to_string() }
                    else if minutes % 60 == 0 { format!("GMT{sign}{}", minutes / 60) }
                    else { format!("GMT{sign}{}:{:02}", minutes / 60, minutes % 60) }
                } else if name == Some("longOffset") {
                    if offset_seconds == 0 { "GMT".to_string() }
                    else { format!("GMT{sign}{:02}:{:02}", minutes / 60, minutes % 60) }
                } else if let Some(named) = (!utc_zone && tz_fixed_offset_seconds(&state.time_zone).is_none()).then(|| {
                    let style = match name {
                        Some(style @ ("short" | "long" | "shortGeneric" | "longGeneric")) => style,
                        _ => if count >= 4 { "long" } else { "short" },
                    };
                    let january = tz_offset_ns(&state.time_zone, days_from_civil(date.year as i64, 1, 1) as i128 * 86_400_000_000_000);
                    let july = tz_offset_ns(&state.time_zone, days_from_civil(date.year as i64, 7, 1) as i128 * 86_400_000_000_000);
                    let daylight = january != july && offset_seconds as i128 * 1_000_000_000 > january.min(july);
                    intl_tz_display_name(&state.time_zone, style, daylight)
                }).flatten() {
                    named
                } else if offset_seconds == 0 && utc_zone {
                    if count >= 4 || name == Some("long") { "Coordinated Universal Time".to_string() } else { "UTC".to_string() }
                } else if offset_seconds == 0 {
                    if tz_fixed_offset_seconds(&state.time_zone).is_some() { "GMT".to_string() } else { "UTC".to_string() }
                } else if count >= 4 || matches!(name, Some("long")) {
                    format!("GMT{sign}{:02}:{:02}", minutes / 60, minutes % 60)
                } else if minutes % 60 == 0 { format!("GMT{sign}{}", minutes / 60) }
                else { format!("GMT{sign}{}:{:02}", minutes / 60, minutes % 60) }
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

fn intl_date_value_raw(i: &mut Interpreter, value: Option<&JsValue>) -> Result<f64, Throw> {
    Ok(match value {
        Some(JsValue::Object(object)) if object.borrow().get_own("[[DateValue]]").is_some() => {
            let stored = object.borrow().get_own("[[DateValue]]").unwrap();
            i.to_number(&stored)?
        }
        Some(JsValue::Undefined) | None => intl_now_milliseconds(),
        Some(value) => i.to_number(value)?,
    })
}

fn intl_date_value(i: &mut Interpreter, value: Option<&JsValue>) -> Result<f64, Throw> {
    let milliseconds = intl_date_value_raw(i, value)?;
    // PartitionDateTimePattern applies TimeClip: non-finite values and values
    // outside the ECMAScript time range are a RangeError; in-range finite
    // values are truncated toward zero.
    if !milliseconds.is_finite() || milliseconds.abs() > 8.64e15 {
        return Err(Throw::range_error("invalid time value"));
    }
    Ok(milliseconds.trunc())
}

fn intl_date_format_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(i, &this)?;
    let function = Interpreter::native_arity("", intl_date_bound_format, &i.function_proto, 1);
    let JsValue::Object(object) = &function else { unreachable!() };
    object.borrow_mut().constructible = false;
    object.borrow_mut().delete_own("prototype");
    intl_register(object, IntlState::BoundDateTime(state));
    Ok(function)
}

// A Temporal.Instant formats with both date and time defaults (required "any",
// defaults "all"), unlike a Date, whose bare format is date-only.
fn intl_instant_state(state: &IntlDateState) -> IntlDateState {
    let mut instant = state.clone();
    if instant.ymd_defaulted {
        let numeric = || Some("numeric".to_string());
        instant.hour = numeric(); instant.minute = numeric(); instant.second = numeric();
    }
    instant
}

fn intl_render_input(state: &IntlDateState, input: IntlDateInput) -> Result<Vec<(&'static str, String)>, Throw> {
    match input {
        IntlDateInput::Legacy(milliseconds) => Ok(intl_render_date(state, milliseconds)),
        IntlDateInput::Instant(milliseconds) => Ok(intl_render_date(&intl_instant_state(state), milliseconds)),
        IntlDateInput::Temporal(kind, date, _calendar) => Ok(intl_render_civil(&intl_temporal_adapt_state(state, kind)?, date, 0)),
        IntlDateInput::Zoned => Err(Throw::type_error("ZonedDateTime cannot be formatted")),
    }
}

// formatRange/formatRangeToParts requires both endpoints to be the "same
// kind" of value: two legacy Date/number values, or two Temporal objects of
// the identical Plain* kind. Any other pairing (including one Temporal, one
// legacy) is a TypeError -- see fails-on-distinct-temporal-types.js.
fn intl_date_range_input(
    state: &IntlDateState, left: IntlDateInput, right: IntlDateInput,
) -> Result<Vec<(&'static str, String, &'static str)>, Throw> {
    match (left, right) {
        (IntlDateInput::Instant(start), IntlDateInput::Instant(end)) => {
            Ok(intl_date_range_parts(&intl_instant_state(state), start, end))
        }
        (IntlDateInput::Legacy(start), IntlDateInput::Legacy(end)) => {
            if [start, end].iter().any(|value| !value.is_finite() || value.abs() > 8.64e15) {
                return Err(Throw::range_error("invalid date range"));
            }
            Ok(intl_date_range_parts(state, start.trunc(), end.trunc()))
        }
        (IntlDateInput::Temporal(left_kind, start, left_calendar), IntlDateInput::Temporal(right_kind, end, right_calendar))
            if left_kind == right_kind => {
            // CalendarEquals (spec): formatRange/formatRangeToParts require
            // both endpoints to share a calendar, same as since/until --
            // confirmed by temporal-objects-throws-with-different-
            // calendars.js.
            if left_calendar != right_calendar {
                return Err(Throw::range_error("formatRange requires the same calendar on both dates"));
            }
            intl_date_range_parts_temporal(state, left_kind, start, end)
        }
        _ => Err(Throw::type_error("formatRange requires two values of the same type")),
    }
}

fn intl_date_bound_format(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let function = i.native_stack.last().ok_or_else(|| Throw::type_error("missing bound DateTimeFormat"))?;
    let state = match intl_state(function) {
        Some(IntlState::BoundDateTime(state)) => state,
        _ => return Err(Throw::type_error("invalid bound DateTimeFormat")),
    };
    let input = intl_date_input(i, args.first())?;
    let parts = intl_render_input(&state, input)?;
    Ok(JsValue::str(parts.iter().map(|(_, value)| value.as_str()).collect::<String>()))
}

fn intl_date_format_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(i, &this)?;
    let input = intl_date_input(i, args.first())?;
    intl_parts_array(i, intl_render_input(&state, input)?)
}

fn intl_date_format_range(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(i, &this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRange requires two dates"));
    }
    let start = intl_date_input_range(i, args.first())?;
    let end = intl_date_input_range(i, args.get(1))?;
    let parts = intl_date_range_input(&state, start, end)?;
    Ok(JsValue::str(parts.iter().map(|(_, value, _)| value.as_str()).collect::<String>()))
}

fn intl_date_format_range_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_date_state(i, &this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRangeToParts requires two dates"));
    }
    let start = intl_date_input_range(i, args.first())?;
    let end = intl_date_input_range(i, args.get(1))?;
    let mut values = Vec::new();
    for (kind, value, source) in intl_date_range_input(&state, start, end)? {
        let part = i.object_value();
        i.set_prop(&part, "type", JsValue::str(kind))?;
        i.set_prop(&part, "value", JsValue::str(value))?;
        i.set_prop(&part, "source", JsValue::str(source))?;
        values.push(part);
    }
    intl_array(i, values)
}

fn intl_date_range_parts(state: &IntlDateState, start: f64, end: f64) -> Vec<(&'static str, String, &'static str)> {
    intl_date_range_parts_from(state, intl_render_date(state, start), intl_render_date(state, end))
}

// Both Temporal range endpoints have already been checked to share the same
// IntlTemporalKind (fails-on-distinct-temporal-types.js), so the state is
// adapted once and both sides render through the same field set.
fn intl_date_range_parts_temporal(
    state: &IntlDateState, kind: IntlTemporalKind, start: IntlCivilDate, end: IntlCivilDate,
) -> Result<Vec<(&'static str, String, &'static str)>, Throw> {
    let adapted = intl_temporal_adapt_state(state, kind)?;
    let left = intl_render_civil(&adapted, start, 0);
    let right = intl_render_civil(&adapted, end, 0);
    Ok(intl_date_range_parts_from(&adapted, left, right))
}

fn intl_date_range_parts_from(
    state: &IntlDateState, left: Vec<(&'static str, String)>, right: Vec<(&'static str, String)>,
) -> Vec<(&'static str, String, &'static str)> {
    if left == right {
        return left.into_iter().map(|(kind, value)| (kind, value, "shared")).collect();
    }
    let textual_month = ["long", "short", "narrow"].contains(&state.month.as_deref().unwrap_or(""));
    let is_date = |kind: &str| matches!(kind, "year" | "month" | "day" | "weekday" | "era" | "relatedYear" | "yearName");
    let value_of = |parts: &[(&'static str, String)], kind: &str| {
        parts.iter().find(|(part, _)| *part == kind).map(|(_, text)| text.clone())
    };
    let date_equal = ["year", "month", "day", "weekday", "era", "relatedYear"]
        .iter().all(|kind| value_of(&left, kind) == value_of(&right, kind));
    let has_time = left.iter().any(|(kind, _)| *kind != "literal" && !is_date(kind));
    let mut prefix = 0;
    let mut suffix = 0;
    if date_equal && has_time {
        // Same day, different times: the date is shared, only the time part is a range.
        prefix = left.iter().position(|(kind, _)| *kind != "literal" && !is_date(kind)).unwrap_or(0);
    } else if !has_time && textual_month && value_of(&left, "year") == value_of(&right, "year") {
        while prefix < left.len().min(right.len()) && left[prefix] == right[prefix] { prefix += 1; }
        while suffix < left.len().min(right.len()) - prefix
            && left[left.len() - suffix - 1] == right[right.len() - suffix - 1] { suffix += 1; }
        // A month that differs leaves nothing shared in front of it.
        if value_of(&left, "month") != value_of(&right, "month") { prefix = 0; }
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
    let state = intl_date_state(i, &this)?;
    let object = i.object_value();
    for (key, value) in [
        ("locale", &state.resolved_locale), ("calendar", &state.calendar),
        ("numberingSystem", &state.numbering_system), ("timeZone", &state.time_zone),
    ] { i.set_prop(&object, key, JsValue::str(value))?; }
    // hourCycle and hour12 are only part of the resolved options when the
    // requested pattern actually contains a time component.
    if state.hour.is_some() || state.time_style.is_some() {
        i.set_prop(&object, "hourCycle", JsValue::str(state.hour_cycle.as_str()))?;
        i.set_prop(&object, "hour12", JsValue::Bool(["h11", "h12"].contains(&state.hour_cycle.as_str())))?;
    }
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
        assert_eq!(i.to_string(&value).unwrap(), "01:59|03:00");
    }

    #[test]
    fn range_parts_mark_shared_and_distinct_fields() {
        let mut i = Interpreter::new();
        let value = i.run_script("let d=new Intl.DateTimeFormat('en',{timeZone:'UTC',year:'numeric',month:'short',day:'numeric'}); d.formatRangeToParts(1546473600000,1546646400000).map(p=>p.type+':'+p.source).join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "month:shared|literal:shared|day:startRange|literal:shared|day:endRange|literal:shared|year:shared");
    }
}

// Temporal.*.prototype.toLocaleString (locales, options): CreateDateTimeFormat
// with the type's (required, defaults) pair, the calendar compatibility check,
// then the same adapted rendering DateTimeFormat.prototype.format applies to a
// Temporal value. `zone` is the ZonedDateTime's own time zone, if any.
fn intl_temporal_to_locale_string(i: &mut Interpreter, value: &JsValue, locales: &JsValue, options: &JsValue) -> Result<JsValue, Throw> {
    let plain = intl_temporal_lookup(value);
    let instant = std_epoch_record(value, Some(StdEpochKind::Instant)).ok();
    let zoned = std_epoch_record(value, Some(StdEpochKind::ZonedDateTime)).ok();
    let mode = match plain.as_ref().map(|(kind, ..)| *kind) {
        Some(IntlTemporalKind::Date) => ("date", "date"),
        Some(IntlTemporalKind::Time) => ("time", "time"),
        Some(IntlTemporalKind::YearMonth) => ("date", "year-month"),
        Some(IntlTemporalKind::MonthDay) => ("date", "month-day"),
        _ => ("any", "all"),
    };
    if zoned.is_some() && !options.is_undefined() && options.is_object() {
        let zone = i.get_prop(options, "timeZone")?;
        if !zone.is_undefined() {
            return Err(Throw::type_error("ZonedDateTime.toLocaleString does not accept a timeZone option"));
        }
    }
    let mut state = intl_date_create_state(i, locales, options, Some(mode))?;
    if let Some(data) = &zoned {
        state.time_zone = data.zone_id.clone();
        if state.time_zone_name.is_none() && state.date_style.is_none() && state.time_style.is_none() && state.ymd_defaulted {
            state.time_zone_name = Some("short".to_string());
        }
    }
    if let Some((kind, date, calendar)) = plain {
        // PlainYearMonth/PlainMonthDay have no calendar-independent reading,
        // so even the ISO calendar must match the formatter's own.
        let strict = matches!(kind, IntlTemporalKind::YearMonth | IntlTemporalKind::MonthDay);
        if (calendar != "iso8601" || strict) && calendar != state.calendar {
            return Err(Throw::range_error("Temporal calendar does not match the DateTimeFormat calendar"));
        }
        let parts = intl_render_input(&state, IntlDateInput::Temporal(kind, date, calendar))?;
        return Ok(JsValue::str(parts.iter().map(|(_, text)| text.as_str()).collect::<String>()));
    }
    let epoch_ns = if let Some(data) = instant.as_ref().or(zoned.as_ref()) { data.epoch_ns } else {
        return Err(Throw::type_error("incompatible Temporal receiver"));
    };
    if let Some(data) = &zoned {
        if data.calendar != "iso8601" && data.calendar != state.calendar {
            return Err(Throw::range_error("Temporal calendar does not match the DateTimeFormat calendar"));
        }
    }
    let parts = intl_render_input(&state, IntlDateInput::Instant(epoch_ns.div_euclid(1_000_000) as f64))?;
    Ok(JsValue::str(parts.iter().map(|(_, text)| text.as_str()).collect::<String>()))
}
