// ISO-calendar Temporal values. The slots live outside JsObject so ownKeys and
// property descriptors cannot observe them. Epoch-based types follow JSB-2.

#[derive(Clone, Copy, PartialEq, Eq)]
enum StdTemporalKind { Duration, PlainDate, PlainTime, PlainDateTime, PlainYearMonth, PlainMonthDay }

#[derive(Clone)]
struct StdTemporalData {
    owner: Weak<RefCell<JsObject>>,
    kind: StdTemporalKind,
    // Calendar values: year, month, day, hour, minute, second, ms, us, ns,
    // reference year. Duration values use the ten fields in spec order.
    fields: [i64; 10],
}

struct StdTemporalPrototypes {
    owner: Weak<RefCell<JsObject>>,
    entries: Vec<(StdTemporalKind, Obj)>,
}

thread_local! {
    static STD_TEMPORAL_DATA: RefCell<HashMap<usize, StdTemporalData>> = RefCell::new(HashMap::new());
    static STD_TEMPORAL_PROTOS: RefCell<HashMap<usize, StdTemporalPrototypes>> = RefCell::new(HashMap::new());
}

fn std_temporal_name(kind: StdTemporalKind) -> &'static str {
    match kind {
        StdTemporalKind::Duration => "Duration",
        StdTemporalKind::PlainDate => "PlainDate",
        StdTemporalKind::PlainTime => "PlainTime",
        StdTemporalKind::PlainDateTime => "PlainDateTime",
        StdTemporalKind::PlainYearMonth => "PlainYearMonth",
        StdTemporalKind::PlainMonthDay => "PlainMonthDay",
    }
}

fn std_temporal_constructor(kind: StdTemporalKind) -> crate::value::NativeFn {
    match kind {
        StdTemporalKind::Duration => std_temporal_duration_constructor,
        StdTemporalKind::PlainDate => std_temporal_date_constructor,
        StdTemporalKind::PlainTime => std_temporal_time_constructor,
        StdTemporalKind::PlainDateTime => std_temporal_datetime_constructor,
        StdTemporalKind::PlainYearMonth => std_temporal_yearmonth_constructor,
        StdTemporalKind::PlainMonthDay => std_temporal_monthday_constructor,
    }
}

fn std_temporal_length(kind: StdTemporalKind) -> u32 {
    match kind {
        StdTemporalKind::Duration => 0,
        StdTemporalKind::PlainDate => 3,
        StdTemporalKind::PlainTime => 0,
        StdTemporalKind::PlainDateTime => 3,
        StdTemporalKind::PlainYearMonth => 2,
        StdTemporalKind::PlainMonthDay => 2,
    }
}

fn std_temporal_from_method(kind: StdTemporalKind) -> crate::value::NativeFn {
    match kind {
        StdTemporalKind::Duration => std_temporal_duration_from,
        StdTemporalKind::PlainDate => std_temporal_date_from,
        StdTemporalKind::PlainTime => std_temporal_time_from,
        StdTemporalKind::PlainDateTime => std_temporal_datetime_from,
        StdTemporalKind::PlainYearMonth => std_temporal_yearmonth_from,
        StdTemporalKind::PlainMonthDay => std_temporal_monthday_from,
    }
}

fn std_temporal_compare_method(kind: StdTemporalKind) -> crate::value::NativeFn {
    match kind {
        StdTemporalKind::Duration => std_temporal_duration_compare,
        StdTemporalKind::PlainDate => std_temporal_date_compare,
        StdTemporalKind::PlainTime => std_temporal_time_compare,
        StdTemporalKind::PlainDateTime => std_temporal_datetime_compare,
        StdTemporalKind::PlainYearMonth => std_temporal_yearmonth_compare,
        StdTemporalKind::PlainMonthDay => std_temporal_monthday_compare,
    }
}

fn std_temporal_extra_methods(kind: StdTemporalKind) -> &'static [(&'static str, u32, crate::value::NativeFn)] {
    match kind {
        StdTemporalKind::Duration => &[
            ("abs", 0, std_temporal_duration_abs),
            ("negated", 0, std_temporal_duration_negated),
            ("add", 1, std_temporal_add),
            ("subtract", 1, std_temporal_subtract),
            ("with", 1, std_temporal_with),
            ("total", 1, std_temporal_duration_total),
            ("round", 1, std_temporal_round),
        ],
        StdTemporalKind::PlainDate => &[
            ("add", 1, std_temporal_add),
            ("subtract", 1, std_temporal_subtract),
            ("until", 1, std_temporal_until),
            ("since", 1, std_temporal_since),
            ("with", 1, std_temporal_with),
            ("toPlainDateTime", 0, std_temporal_date_to_datetime),
            ("toPlainYearMonth", 0, std_temporal_date_to_yearmonth),
            ("toPlainMonthDay", 0, std_temporal_date_to_monthday),
            ("toZonedDateTime", 1, std_temporal_plain_to_zoned),
            ("withCalendar", 1, std_temporal_with_calendar),
        ],
        StdTemporalKind::PlainTime => &[
            ("add", 1, std_temporal_add),
            ("subtract", 1, std_temporal_subtract),
            ("until", 1, std_temporal_until),
            ("since", 1, std_temporal_since),
            ("with", 1, std_temporal_with),
            ("round", 1, std_temporal_round),
        ],
        StdTemporalKind::PlainDateTime => &[
            ("add", 1, std_temporal_add),
            ("subtract", 1, std_temporal_subtract),
            ("until", 1, std_temporal_until),
            ("since", 1, std_temporal_since),
            ("with", 1, std_temporal_with),
            ("withPlainTime", 0, std_temporal_datetime_with_time),
            ("round", 1, std_temporal_round),
            ("toPlainDate", 0, std_temporal_datetime_to_date),
            ("toPlainTime", 0, std_temporal_datetime_to_time),
            ("toZonedDateTime", 1, std_temporal_plain_to_zoned),
            ("withCalendar", 1, std_temporal_with_calendar),
        ],
        StdTemporalKind::PlainYearMonth => &[
            ("add", 1, std_temporal_add),
            ("subtract", 1, std_temporal_subtract),
            ("until", 1, std_temporal_until),
            ("since", 1, std_temporal_since),
            ("with", 1, std_temporal_with),
            ("toPlainDate", 1, std_temporal_yearmonth_to_date),
            ("withCalendar", 1, std_temporal_with_calendar),
        ],
        StdTemporalKind::PlainMonthDay => &[
            ("with", 1, std_temporal_with),
            ("toPlainDate", 1, std_temporal_monthday_to_date),
            ("withCalendar", 1, std_temporal_with_calendar),
        ],
    }
}

fn std_temporal_fields(kind: StdTemporalKind) -> &'static [&'static str] {
    match kind {
        StdTemporalKind::Duration => &["years", "months", "weeks", "days", "hours", "minutes", "seconds", "milliseconds", "microseconds", "nanoseconds", "sign", "blank"],
        StdTemporalKind::PlainDate => &["calendarId", "era", "eraYear", "year", "month", "monthCode", "day", "dayOfWeek", "dayOfYear", "weekOfYear", "yearOfWeek", "daysInWeek", "daysInMonth", "daysInYear", "monthsInYear", "inLeapYear"],
        StdTemporalKind::PlainTime => &["hour", "minute", "second", "millisecond", "microsecond", "nanosecond"],
        StdTemporalKind::PlainDateTime => &["calendarId", "era", "eraYear", "year", "month", "monthCode", "day", "dayOfWeek", "dayOfYear", "weekOfYear", "yearOfWeek", "daysInWeek", "daysInMonth", "daysInYear", "monthsInYear", "inLeapYear", "hour", "minute", "second", "millisecond", "microsecond", "nanosecond"],
        StdTemporalKind::PlainYearMonth => &["calendarId", "era", "eraYear", "year", "month", "monthCode", "daysInMonth", "daysInYear", "monthsInYear", "inLeapYear"],
        StdTemporalKind::PlainMonthDay => &["calendarId", "monthCode", "day"],
    }
}

impl Interpreter {
    fn install_temporal(&mut self) {
        let temporal = self.object_value();
        let JsValue::Object(namespace) = &temporal else { unreachable!() };
        let mut entries = Vec::new();
        for kind in [StdTemporalKind::Duration, StdTemporalKind::PlainDate,
            StdTemporalKind::PlainTime, StdTemporalKind::PlainDateTime,
            StdTemporalKind::PlainYearMonth, StdTemporalKind::PlainMonthDay] {
            let name = std_temporal_name(kind);
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(self.object_proto.clone());
            let prototype = Obj(Rc::new(RefCell::new(prototype)));
            let constructor = Self::native(name, std_temporal_constructor(kind), &self.function_proto);
            if let JsValue::Object(function) = &constructor {
                function.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                function.borrow_mut().define("length", symbol_data(JsValue::num(std_temporal_length(kind) as f64), false, true)).unwrap();
                function.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
                for (method, length, native) in [
                    ("from", 1, std_temporal_from_method(kind)),
                    ("compare", 2, std_temporal_compare_method(kind)),
                ] {
                    let value = symbol_method(self, method, native);
                    symbol_method_length(&value, length);
                    function.borrow_mut().define(method, symbol_data(value, true, true)).unwrap();
                }
            }
            prototype.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                symbol_data(JsValue::str(format!("Temporal.{name}")), false, true)).unwrap();
            for field in std_temporal_fields(kind) {
                let getter = symbol_method(self, &format!("get {field}"), std_temporal_getter);
                let property = Property { value: JsValue::Undefined, writable: false,
                    enumerable: false, configurable: true,
                    getter: Some(getter), setter: None };
                prototype.borrow_mut().define(*field, property).unwrap();
            }
            for (method, length, native) in [
                ("toString", 0, std_temporal_to_string as crate::value::NativeFn),
                ("toJSON", 0, std_temporal_to_json as crate::value::NativeFn),
                ("toLocaleString", 0, std_temporal_to_locale_string as crate::value::NativeFn),
                ("valueOf", 0, std_temporal_value_of as crate::value::NativeFn),
                ("equals", 1, std_temporal_equals as crate::value::NativeFn),
            ] {
                let function = symbol_method(self, method, native);
                symbol_method_length(&function, length);
                prototype.borrow_mut().define(method, symbol_data(function, true, true)).unwrap();
            }
            for (method, length, native) in std_temporal_extra_methods(kind) {
                let function = symbol_method(self, method, *native);
                symbol_method_length(&function, *length);
                prototype.borrow_mut().define(*method, symbol_data(function, true, true)).unwrap();
            }
            namespace.borrow_mut().define(name, symbol_data(constructor, true, true)).unwrap();
            entries.push((kind, prototype));
        }
        let now = self.object_value();
        if let JsValue::Object(object) = &now {
            for (name, native) in [
                ("timeZoneId", std_temporal_now_zone as crate::value::NativeFn),
                ("plainDateISO", std_temporal_now_date as crate::value::NativeFn),
                ("plainTimeISO", std_temporal_now_time as crate::value::NativeFn),
                ("plainDateTimeISO", std_temporal_now_datetime as crate::value::NativeFn),
                ("instant", std_temporal_now_instant as crate::value::NativeFn),
                ("zonedDateTimeISO", std_temporal_now_zoned as crate::value::NativeFn),
            ] {
                object.borrow_mut().define(name,
                    symbol_data(symbol_method(self, name, native), true, true)).unwrap();
            }
            object.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                symbol_data(JsValue::str("Temporal.Now"), false, true)).unwrap();
        }
        namespace.borrow_mut().define("Now", symbol_data(now, true, true)).unwrap();
        self.install_temporal_epoch(namespace);
        namespace.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            symbol_data(JsValue::str("Temporal"), false, true)).unwrap();
        STD_TEMPORAL_PROTOS.with(|registry| registry.borrow_mut().insert(std_weak_id(&self.global_obj),
            StdTemporalPrototypes { owner: Rc::downgrade(&self.global_obj.0), entries }));
        self.define_global("Temporal", temporal.clone());
        self.global_obj.borrow_mut().define("Temporal", symbol_data(temporal, true, true)).unwrap();
    }
}

fn std_temporal_civil_from_epoch_day(epoch_day: i64) -> (i64, i64, i64) {
    let z = epoch_day + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn std_temporal_now_fields() -> [i64; 10] {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = elapsed.as_secs() as i64;
    let epoch_day = seconds.div_euclid(86_400);
    let within_day = seconds.rem_euclid(86_400);
    let (year, month, day) = std_temporal_civil_from_epoch_day(epoch_day);
    let nanos = elapsed.subsec_nanos() as i64;
    [year, month, day, within_day / 3_600, (within_day / 60) % 60,
        within_day % 60, nanos / 1_000_000, (nanos / 1_000) % 1_000,
        nanos % 1_000, 0]
}

fn std_temporal_now_zone(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str("UTC"))
}

fn std_temporal_fixed_offset(text: &str) -> Result<i64, Throw> {
    if text.eq_ignore_ascii_case("UTC") { return Ok(0); }
    let (sign, rest) = if let Some(rest) = text.strip_prefix('+') { (1, rest) }
        else if let Some(rest) = text.strip_prefix('-') { (-1, rest) }
        else { return Err(Throw::range_error("invalid time zone")); };
    let (hour, minute) = rest.split_once(':').ok_or_else(|| Throw::range_error("invalid time zone"))?;
    if hour.len() != 2 || minute.len() != 2 || !hour.bytes().chain(minute.bytes()).all(|c| c.is_ascii_digit()) {
        return Err(Throw::range_error("invalid time zone"));
    }
    let hour = hour.parse::<i64>().map_err(|_| Throw::range_error("invalid time zone"))?;
    let minute = minute.parse::<i64>().map_err(|_| Throw::range_error("invalid time zone"))?;
    if hour > 23 || minute > 59 { return Err(Throw::range_error("invalid time zone")); }
    Ok(sign * (hour * 60 + minute))
}

fn std_temporal_now_in_zone(i: &mut Interpreter, args: Vec<JsValue>) -> Result<[i64; 10], Throw> {
    let zone = std_reflect_arg(&args, 0);
    let zone = if zone.is_undefined() { "UTC".to_string() }
        else { std_epoch_zone_value(i, &zone)? };
    let mut fields = std_temporal_now_fields();
    let epoch_ns = std_temporal_epoch_ns(&fields);
    let shifted = epoch_ns + tz_offset_ns(&zone, epoch_ns);
    std_temporal_set_epoch_ns(&mut fields, shifted)?;
    Ok(fields)
}

fn std_temporal_now_date(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let fields = std_temporal_now_in_zone(i, args)?;
    std_temporal_make(i, StdTemporalKind::PlainDate, fields)
}

fn std_temporal_now_time(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let fields = std_temporal_now_in_zone(i, args)?;
    std_temporal_make(i, StdTemporalKind::PlainTime, fields)
}

fn std_temporal_now_datetime(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let fields = std_temporal_now_in_zone(i, args)?;
    std_temporal_make(i, StdTemporalKind::PlainDateTime, fields)
}

fn std_temporal_make(i: &Interpreter, kind: StdTemporalKind, fields: [i64; 10]) -> Result<JsValue, Throw> {
    std_temporal_validate(kind, &fields)?;
    let prototype = STD_TEMPORAL_PROTOS.with(|registry| registry.borrow().get(&std_weak_id(&i.global_obj))
        .filter(|entry| std_weak_same(&entry.owner, &i.global_obj))
        .and_then(|entry| entry.entries.iter().find(|(candidate, _)| *candidate == kind))
        .map(|(_, prototype)| prototype.clone()))
        .ok_or_else(|| Throw::type_error("Temporal realm is unavailable"))?;
    let mut object = JsObject::ordinary();
    object.proto = Some(prototype);
    let object = Obj(Rc::new(RefCell::new(object)));
    STD_TEMPORAL_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(&object),
        StdTemporalData { owner: Rc::downgrade(&object.0), kind, fields }));
    Ok(JsValue::Object(object))
}

fn std_temporal_record(value: &JsValue, expected: Option<StdTemporalKind>) -> Result<StdTemporalData, Throw> {
    let JsValue::Object(object) = value else { return Err(Throw::type_error("incompatible Temporal receiver")); };
    STD_TEMPORAL_DATA.with(|registry| registry.borrow().get(&std_weak_id(object))
        .filter(|data| std_weak_same(&data.owner, object) && expected.is_none_or(|kind| kind == data.kind))
        .cloned().ok_or_else(|| Throw::type_error("incompatible Temporal receiver")))
}

fn std_temporal_number(i: &mut Interpreter, value: JsValue, default: Option<i64>) -> Result<i64, Throw> {
    if value.is_undefined() {
        if let Some(default) = default { return Ok(default); }
    }
    let number = i.to_number(&value)?;
    if !number.is_finite() || number.abs() > 9_007_199_254_740_991.0 {
        return Err(Throw::range_error("Temporal field must be an integer"));
    }
    Ok(number.trunc() as i64)
}

fn std_temporal_duration_number(i: &mut Interpreter, value: JsValue, default: Option<i64>) -> Result<i64, Throw> {
    if value.is_undefined() {
        if let Some(default) = default { return Ok(default); }
    }
    let number = i.to_number(&value)?;
    if !number.is_finite() || number.fract() != 0.0 || number.abs() > 9_007_199_254_740_991.0 {
        return Err(Throw::range_error("duration field must be an integer"));
    }
    Ok(number as i64)
}

fn std_temporal_leap(year: i64) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }

fn std_temporal_days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 => if std_temporal_leap(year) { 29 } else { 28 },
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Days from 1970-01-01 in the proleptic Gregorian calendar.
fn std_temporal_epoch_day(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let year_of_era_day = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + year_of_era_day - 719_468
}

fn std_temporal_day_of_week(epoch_day: i64) -> i64 { (epoch_day + 3).rem_euclid(7) + 1 }

fn std_temporal_iso_calendar(i: &mut Interpreter, value: &JsValue) -> Result<(), Throw> {
    if value.is_undefined() { return Ok(()); }
    if std_temporal_record(value, None).is_ok_and(|record| matches!(record.kind,
        StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime
        | StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay))
        || std_epoch_record(value, Some(StdEpochKind::ZonedDateTime)).is_ok() {
        return Ok(());
    }
    if !matches!(value, JsValue::Str(_)) {
        return Err(Throw::type_error("Temporal calendar must be a string or Temporal object"));
    }
    let name = i.to_string(value)?;
    if name.eq_ignore_ascii_case("iso8601") { Ok(()) }
    else { Err(Throw::range_error("only iso8601 calendar is supported")) }
}

fn std_temporal_calendar_from_value(i: &mut Interpreter, value: &JsValue) -> Result<(), Throw> {
    if std_temporal_iso_calendar(i, value).is_ok() { return Ok(()); }
    if let JsValue::Str(name) = value {
        if [StdTemporalKind::PlainDate, StdTemporalKind::PlainDateTime,
            StdTemporalKind::PlainYearMonth, StdTemporalKind::PlainMonthDay]
            .iter().any(|kind| std_temporal_parse(*kind, &name.to_utf8_lossy()).is_ok()) {
            return Ok(());
        }
    }
    std_temporal_iso_calendar(i, value)
}

fn std_temporal_week_start(year: i64) -> i64 {
    let jan4 = std_temporal_epoch_day(year, 1, 4);
    jan4 - std_temporal_day_of_week(jan4) + 1
}

fn std_temporal_validate(kind: StdTemporalKind, values: &[i64; 10]) -> Result<(), Throw> {
    if kind == StdTemporalKind::Duration {
        let sign = values.iter().find(|v| **v != 0).map(|v| v.signum()).unwrap_or(0);
        // years/months/weeks are each independently capped at 2**32-1 (no
        // relationship to the time part). days-through-nanoseconds are NOT
        // capped per field -- each can individually run into the
        // quadrillions once expressed in a fine-enough unit -- instead the
        // spec caps the *exact total* of that whole day..ns span, expressed
        // in seconds, to the float64-safe-integer range (with up to
        // .999999999 of a further second, since nanosecond precision is
        // kept exactly). Confirmed by Duration/max.js (the largest legal
        // days/hours/minutes/seconds cases, none of which are individually
        // under 2**32) and Duration/out-of-range.js (one unit step past
        // each threshold, including a finer unit balancing into the next).
        const STD_TEMPORAL_DURATION_MAX_NS: i128 = 9_007_199_254_740_991 * 1_000_000_000 + 999_999_999;
        let total_ns: i128 = values[3] as i128 * STD_TEMPORAL_NS_DAY
            + values[4] as i128 * 3_600 * STD_TEMPORAL_NS_SECOND
            + values[5] as i128 * 60 * STD_TEMPORAL_NS_SECOND
            + values[6] as i128 * STD_TEMPORAL_NS_SECOND
            + values[7] as i128 * 1_000_000
            + values[8] as i128 * 1_000
            + values[9] as i128;
        // Days is bounded only by the total (a days-only span can already
        // run past 2**32-1 while its total in seconds is still safely in
        // range, e.g. Duration/max.js's "max days" case), but
        // hours-through-nanoseconds ALSO each keep an individual
        // float64-safe-integer cap on top of the total: once BalanceDuration
        // collapses everything into a single requested largestUnit (e.g.
        // `.round({largestUnit: "nanoseconds"})`), that one surviving field
        // must itself be representable, even though the exact total
        // expressed in seconds is still within the total-based bound --
        // confirmed by
        // round/out-of-range-when-converting-from-normalized-duration.js.
        if values.iter().any(|v| *v != 0 && v.signum() != sign)
            || values[..3].iter().any(|v| v.abs() > 4_294_967_295)
            || values[4..].iter().any(|v| v.abs() > 9_007_199_254_740_991)
            || total_ns.abs() > STD_TEMPORAL_DURATION_MAX_NS {
            return Err(Throw::range_error("invalid Temporal duration"));
        }
        return Ok(());
    }
    let [year, month, day, hour, minute, second, millisecond, microsecond, nanosecond, reference_year] = *values;
    if kind != StdTemporalKind::PlainTime {
        let actual_year = if kind == StdTemporalKind::PlainMonthDay { reference_year } else { year };
        if !(-271_821..=275_760).contains(&actual_year) || !(1..=12).contains(&month)
            || !(1..=std_temporal_days_in_month(actual_year, month)).contains(&day) {
            return Err(Throw::range_error("invalid ISO date"));
        }
        if matches!(kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime)
            && !(-100_000_001..=100_000_000).contains(&std_temporal_epoch_day(actual_year, month, day)) {
            return Err(Throw::range_error("ISO date outside Temporal range"));
        }
    }
    if matches!(kind, StdTemporalKind::PlainTime | StdTemporalKind::PlainDateTime)
        && (hour > 23 || minute > 59 || second > 59 || millisecond > 999
            || microsecond > 999 || nanosecond > 999
            || [hour, minute, second, millisecond, microsecond, nanosecond].iter().any(|v| *v < 0)) {
        return Err(Throw::range_error("invalid ISO time"));
    }
    Ok(())
}

fn std_temporal_construct(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Temporal constructor requires new")); };
    if !object.borrow().constructed { return Err(Throw::type_error("Temporal constructor requires new")); }
    let mut values = [0i64; 10];
    let required = match kind {
        StdTemporalKind::Duration | StdTemporalKind::PlainTime => 0,
        StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime => 3,
        StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay => 2,
    };
    let count = match kind {
        StdTemporalKind::Duration => 10,
        StdTemporalKind::PlainDate => 3,
        StdTemporalKind::PlainTime => 6,
        StdTemporalKind::PlainDateTime => 9,
        StdTemporalKind::PlainYearMonth => 2,
        StdTemporalKind::PlainMonthDay => 2,
    };
    for index in 0..count {
        let destination = match kind {
            StdTemporalKind::PlainMonthDay => index + 1,
            StdTemporalKind::PlainTime => index + 3,
            _ => index,
        };
        values[destination] = if kind == StdTemporalKind::Duration {
            std_temporal_duration_number(i, std_reflect_arg(&args, index), Some(0))?
        } else {
            std_temporal_number(i, std_reflect_arg(&args, index), (index >= required).then_some(0))?
        };
    }
    if kind == StdTemporalKind::PlainYearMonth { values[2] = 1; }
    if kind == StdTemporalKind::PlainMonthDay { values[9] = std_temporal_number(i, std_reflect_arg(&args, 3), Some(1972))?; }
    if kind == StdTemporalKind::PlainYearMonth {
        values[2] = std_temporal_number(i, std_reflect_arg(&args, 3), Some(1))?;
    }
    std_temporal_validate(kind, &values)?;
    if kind != StdTemporalKind::Duration && kind != StdTemporalKind::PlainTime {
        let calendar_arg = match kind {
            StdTemporalKind::PlainDate => 3,
            StdTemporalKind::PlainDateTime => 9,
            StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay => 2,
            _ => unreachable!(),
        };
        let calendar = std_reflect_arg(&args, calendar_arg);
        std_temporal_iso_calendar(i, &calendar)?;
    }
    STD_TEMPORAL_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(object),
        StdTemporalData { owner: Rc::downgrade(&object.0), kind, fields: values }));
    Ok(this)
}

fn std_temporal_duration_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::Duration) }
fn std_temporal_date_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::PlainDate) }
fn std_temporal_time_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::PlainTime) }
fn std_temporal_datetime_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::PlainDateTime) }
fn std_temporal_yearmonth_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::PlainYearMonth) }
fn std_temporal_monthday_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_temporal_construct(i, this, args, StdTemporalKind::PlainMonthDay) }

fn std_temporal_property(i: &mut Interpreter, value: &JsValue, name: &str) -> Result<Option<i64>, Throw> {
    let property = i.get_prop(value, name)?;
    if property.is_undefined() { Ok(None) }
    else { std_temporal_number(i, property, None).map(Some) }
}

// Reads the property bag's fields (and validates their raw types) but does
// NOT regulate out-of-range values or run the final ISO-validity check --
// that half depends on the overflow option, which for Temporal.*.from() must
// be read from a *different* object (options) only after these gets on
// `value` complete (Proxy-observable order). std_temporal_regulate below
// applies the overflow-dependent half once the caller knows it.
fn std_temporal_bag(i: &mut Interpreter, kind: StdTemporalKind, value: &JsValue) -> Result<[i64; 10], Throw> {
    let mut fields = [0; 10];
    if !matches!(kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        let calendar = i.get_prop(value, "calendar")?;
        std_temporal_calendar_from_value(i, &calendar)?;
    }
    if kind == StdTemporalKind::Duration {
        let mut present = false;
        for (index, name) in ["years", "months", "weeks", "days", "hours", "minutes",
            "seconds", "milliseconds", "microseconds", "nanoseconds"].iter().enumerate() {
            let property = i.get_prop(value, *name)?;
            if !property.is_undefined() {
                fields[index] = std_temporal_duration_number(i, property, None)?;
                present = true;
            }
        }
        if !present { return Err(Throw::type_error("duration-like object has no fields")); }
        // Duration never takes an overflow option; its range check is
        // unconditional, so it runs here rather than in std_temporal_regulate.
        std_temporal_validate(kind, &fields)?;
        return Ok(fields);
    } else if kind == StdTemporalKind::PlainTime {
        let mut present = false;
        for (index, name) in ["hour", "minute", "second", "millisecond", "microsecond", "nanosecond"].iter().enumerate() {
            if let Some(part) = std_temporal_property(i, value, name)? {
                fields[index + 3] = part;
                present = true;
            }
        }
        if !present { return Err(Throw::type_error("time-like object has no fields")); }
    } else {
        let year = std_temporal_property(i, value, "year")?;
        let month = std_temporal_property(i, value, "month")?;
        // PrepareCalendarFields reads "month" and "day" via
        // ToPositiveIntegerWithTruncation: a value less than 1 throws a
        // RangeError immediately, unconditionally of the overflow option --
        // unlike the *upper* bound (month > 12, day > days-in-month), which
        // std_temporal_regulate below applies only once overflow is known.
        if month.is_some_and(|value| value < 1) {
            return Err(Throw::range_error("month must be a positive integer"));
        }
        let month_code = i.get_prop(value, "monthCode")?;
        let code_month = if month_code.is_undefined() { None } else {
            let code = i.to_string(&month_code)?;
            let number = code.strip_prefix('M').and_then(|digits| digits.parse::<i64>().ok())
                .filter(|number| code.len() == 3 && (1..=12).contains(number))
                .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
            Some(number)
        };
        if month.zip(code_month).is_some_and(|(a, b)| a != b) {
            return Err(Throw::range_error("month and monthCode disagree"));
        }
        let day = std_temporal_property(i, value, "day")?;
        if day.is_some_and(|value| value < 1) && kind != StdTemporalKind::PlainYearMonth {
            return Err(Throw::range_error("day must be a positive integer"));
        }
        fields[0] = if matches!(kind, StdTemporalKind::PlainMonthDay | StdTemporalKind::PlainTime) { 0 } else {
            year.ok_or_else(|| Throw::type_error("year is required"))?
        };
        fields[1] = match kind {
            StdTemporalKind::PlainTime => 0,
            _ => month.or(code_month).ok_or_else(|| Throw::type_error("month is required"))?,
        };
        fields[2] = match kind {
            StdTemporalKind::PlainYearMonth => 1,
            StdTemporalKind::PlainTime => 0,
            _ => day.ok_or_else(|| Throw::type_error("day is required"))?,
        };
        if kind == StdTemporalKind::PlainMonthDay { fields[9] = 1972; }
        if kind == StdTemporalKind::PlainDateTime {
            for (index, name) in ["hour", "minute", "second", "millisecond", "microsecond", "nanosecond"].iter().enumerate() {
                if let Some(part) = std_temporal_property(i, value, name)? {
                    fields[index + 3] = part;
                }
            }
        }
    }
    Ok(fields)
}

// RegulateISODate / RegulateISOYearMonth / RegulateISOMonthDay / RegulateTime:
// under "constrain" (reject == false), out-of-range month/day/time fields
// clamp into range; under "reject" this is a no-op and the unconditional ISO
// validity check in std_temporal_validate (called by every caller right after
// this) throws instead. Split out of std_temporal_bag so a caller that needs
// to read the overflow option from a *different* object first (Temporal.*.
// from(item, options)) can do so between extracting item's fields and
// regulating them, matching spec (and Proxy-observable) property-read order.
fn std_temporal_regulate(kind: StdTemporalKind, fields: &mut [i64; 10], reject: bool) {
    if reject || kind == StdTemporalKind::Duration { return; }
    if kind == StdTemporalKind::PlainTime {
        fields[3] = fields[3].clamp(0, 23);
        fields[4] = fields[4].clamp(0, 59);
        fields[5] = fields[5].clamp(0, 59);
        for field in &mut fields[6..9] { *field = (*field).clamp(0, 999); }
        return;
    }
    fields[1] = fields[1].clamp(1, 12);
    let actual_year = if kind == StdTemporalKind::PlainMonthDay { fields[9] } else { fields[0] };
    fields[2] = fields[2].clamp(1, std_temporal_days_in_month(actual_year, fields[1]));
    if kind == StdTemporalKind::PlainDateTime {
        fields[3] = fields[3].clamp(0, 23);
        fields[4] = fields[4].clamp(0, 59);
        fields[5] = fields[5].clamp(0, 59);
        for field in &mut fields[6..9] { *field = (*field).clamp(0, 999); }
    }
}

fn std_temporal_parse_date(text: &str, require_day: bool) -> Result<(i64, i64, i64, &str), Throw> {
    let year_end = if text.starts_with('+') || text.starts_with('-') { 7 } else { 4 };
    let invalid = || Throw::range_error("invalid ISO date string");
    if text.len() < year_end + 2 { return Err(invalid()); }
    if text.starts_with("-000000") { return Err(invalid()); }
    let year_text = text.get(..year_end).ok_or_else(invalid)?;
    if !year_text.trim_start_matches(['+', '-']).bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let year = year_text.parse::<i64>().map_err(|_| invalid())?;
    let rest = text.get(year_end..).ok_or_else(invalid)?;
    let (rest, extended) = if let Some(rest) = rest.strip_prefix('-') { (rest, true) }
        else { (rest, false) };
    let month_text = rest.get(..2).ok_or_else(invalid)?;
    if !month_text.bytes().all(|byte| byte.is_ascii_digit()) { return Err(invalid()); }
    let month = month_text.parse::<i64>().map_err(|_| invalid())?;
    let mut tail = rest.get(2..).ok_or_else(invalid)?;
    let day_text = if extended { tail.strip_prefix('-').and_then(|rest| rest.get(..2)) }
        else if tail.as_bytes().first().is_some_and(u8::is_ascii_digit) { tail.get(..2) }
        else { None };
    let day = if let Some(day_text) = day_text {
        if !day_text.bytes().all(|byte| byte.is_ascii_digit()) { return Err(invalid()); }
        tail = &tail[if extended { 3 } else { 2 }..];
        day_text.parse::<i64>().map_err(|_| invalid())?
    } else if require_day { return Err(invalid()); } else { 1 };
    Ok((year, month, day, tail))
}

fn std_temporal_parse_time(text: &str, fields: &mut [i64; 10]) -> Result<(), Throw> {
    let invalid = || Throw::range_error("invalid ISO time string");
    let text = text.strip_prefix('T').unwrap_or(text);
    let hour = text.get(..2).ok_or_else(invalid)?;
    if !hour.bytes().all(|b| b.is_ascii_digit()) { return Err(invalid()); }
    fields[3] = hour.parse().map_err(|_| invalid())?;
    let mut rest = text.get(2..).ok_or_else(invalid)?;
    if rest.is_empty() { return Ok(()); }
    let separated = rest.starts_with(':');
    if separated { rest = &rest[1..]; }
    let minute = rest.get(..2).ok_or_else(invalid)?;
    if !minute.bytes().all(|b| b.is_ascii_digit()) { return Err(invalid()); }
    fields[4] = minute.parse().map_err(|_| invalid())?;
    rest = rest.get(2..).ok_or_else(invalid)?;
    if rest.is_empty() { return Ok(()); }
    if separated {
        rest = rest.strip_prefix(':').ok_or_else(invalid)?;
    } else if rest.starts_with(':') { return Err(invalid()); }
    let second = rest.get(..2).ok_or_else(invalid)?;
    if !second.bytes().all(|b| b.is_ascii_digit()) { return Err(invalid()); }
    let second = second.parse::<i64>().map_err(|_| invalid())?;
    if second > 60 { return Err(invalid()); }
    fields[5] = second.min(59);
    rest = rest.get(2..).ok_or_else(invalid)?;
    if let Some(fraction) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(',')) {
        if fraction.is_empty() || fraction.len() > 9 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        let mut digits = fraction.to_string();
        while digits.len() < 9 { digits.push('0'); }
        let nanoseconds = digits.parse::<i64>().map_err(|_| invalid())?;
        fields[6] = nanoseconds / 1_000_000;
        fields[7] = (nanoseconds / 1_000) % 1_000;
        fields[8] = nanoseconds % 1_000;
    } else if !rest.is_empty() { return Err(invalid()); }
    Ok(())
}

fn std_temporal_parse_annotations(text: &str) -> Result<(&str, bool, Option<&str>), Throw> {
    let invalid = || Throw::range_error("invalid Temporal annotation");
    let Some(first) = text.find('[') else { return Ok((text, false, None)); };
    let body = &text[..first];
    let mut tail = &text[first..];
    let mut calendar = false;
    let mut critical_calendar = false;
    let mut time_zone = None;
    while !tail.is_empty() {
        let rest = tail.strip_prefix('[').ok_or_else(invalid)?;
        let end = rest.find(']').ok_or_else(invalid)?;
        let value = &rest[..end];
        tail = &rest[end + 1..];
        let annotation = value.strip_prefix('!').unwrap_or(value);
        if let Some(name) = annotation.strip_prefix("u-ca=") {
            if (!calendar && !name.eq_ignore_ascii_case("iso8601"))
                || (calendar && (critical_calendar || value.starts_with('!'))) {
                return Err(invalid());
            }
            calendar = true;
            critical_calendar = value.starts_with('!');
        } else if let Some((key, _)) = annotation.split_once('=') {
            if value.starts_with('!') || key.bytes().any(|c| c.is_ascii_uppercase()) {
                return Err(invalid());
            }
        } else if time_zone.is_some() || annotation.is_empty() {
            return Err(invalid());
        } else { time_zone = Some(annotation); }
    }
    Ok((body, calendar, time_zone))
}

fn std_temporal_time_prefix(text: &str) -> &str {
    let mut end = text.len();
    for (index, character) in text.char_indices() {
        if character == 'Z' || character == 'z' || (index > 0 && (character == '+' || character == '-')) {
            end = index;
            break;
        }
    }
    &text[..end]
}

fn std_temporal_validate_time_offset(text: &str) -> Result<(), Throw> {
    let suffix = &text[std_temporal_time_prefix(text).len()..];
    if suffix.is_empty() { return Ok(()); }
    if suffix.eq_ignore_ascii_case("z") { return Ok(()); }
    std_epoch_parse_offset(suffix)?;
    Ok(())
}

fn std_temporal_parse_duration(text: &str) -> Result<[i64; 10], Throw> {
    let invalid = || Throw::range_error("invalid ISO duration string");
    let (sign, text) = if let Some(rest) = text.strip_prefix('-') { (-1, rest) }
        else if let Some(rest) = text.strip_prefix('+') { (1, rest) } else { (1, text) };
    let text = text.strip_prefix('P').ok_or_else(invalid)?;
    if text.is_empty() { return Err(invalid()); }
    let mut values = [0; 10];
    let mut time = false;
    let mut number = String::new();
    let mut seen = false;
    let mut previous = None;
    let mut fractional = false;
    for character in text.chars() {
        if character == 'T' && !time && number.is_empty() && !fractional { time = true; continue; }
        if character.is_ascii_digit() || character == '.' || character == ',' {
            number.push(if character == ',' { '.' } else { character });
            continue;
        }
        let index = match (time, character) {
            (false, 'Y') => 0, (false, 'M') => 1, (false, 'W') => 2, (false, 'D') => 3,
            (true, 'H') => 4, (true, 'M') => 5, (true, 'S') => 6,
            _ => return Err(invalid()),
        };
        if number.is_empty() { return Err(invalid()); }
        if previous.is_some_and(|prior| index <= prior) || fractional { return Err(invalid()); }
        if let Some((whole, fraction)) = number.split_once('.') {
            if !(4..=6).contains(&index) || fraction.is_empty() || fraction.len() > 9 { return Err(invalid()); }
            values[index] = whole.parse::<i64>().map_err(|_| invalid())?;
            let mut digits = fraction.to_string();
            while digits.len() < 9 { digits.push('0'); }
            let nanos = digits.parse::<i128>().map_err(|_| invalid())?
                * match index { 4 => 3_600, 5 => 60, _ => 1 };
            let pieces = std_temporal_split_duration_ns(nanos, index + 1);
            for next in index + 1..10 { values[next] = pieces[next]; }
            fractional = true;
        } else { values[index] = number.parse().map_err(|_| invalid())?; }
        number.clear();
        seen = true;
        previous = Some(index);
    }
    if !number.is_empty() || !seen { return Err(invalid()); }
    for value in &mut values { *value *= sign; }
    Ok(values)
}

fn std_temporal_parse(kind: StdTemporalKind, text: &str) -> Result<[i64; 10], Throw> {
    if kind == StdTemporalKind::Duration {
        // Unlike every other kind (validated below, after the match), a
        // Duration string returns early here and must run the same sign/range
        // check itself -- the property-bag path (std_temporal_bag) already
        // validates unconditionally, so a string-form "P4294967296Y" or a
        // ISO duration string mixing a positive and negative field skipped it
        // entirely before this.
        let values = std_temporal_parse_duration(text)?;
        std_temporal_validate(StdTemporalKind::Duration, &values)?;
        return Ok(values);
    }
    let invalid = || Throw::range_error("invalid Temporal string");
    let (text, _calendar, _zone) = std_temporal_parse_annotations(text)?;
    if text.ends_with('Z') || text.ends_with('z') { return Err(invalid()); }
    let mut values = [0; 10];
    match kind {
        StdTemporalKind::PlainTime => {
            let time = if let Some((_, time)) = text.split_once('T').or_else(|| text.split_once('t')).or_else(|| text.split_once(' ')) {
                time
            } else { text };
            std_temporal_validate_time_offset(time)?;
            std_temporal_parse_time(std_temporal_time_prefix(time), &mut values)?;
        }
        StdTemporalKind::PlainMonthDay if text.starts_with("--") || text.len() == 5 => {
            let text = text.strip_prefix("--").unwrap_or(text);
            let (month, day) = text.split_once('-').ok_or_else(invalid)?;
            values[1] = month.parse().map_err(|_| invalid())?;
            values[2] = day.parse().map_err(|_| invalid())?;
            values[9] = 1972;
        }
        _ => {
            let (year, month, day, tail) = std_temporal_parse_date(text,
                !matches!(kind, StdTemporalKind::PlainYearMonth))?;
            if kind == StdTemporalKind::PlainYearMonth && !(1..=std_temporal_days_in_month(year, month)).contains(&day) {
                return Err(invalid());
            }
            values[0] = year; values[1] = month; values[2] = day;
            if kind == StdTemporalKind::PlainYearMonth { values[2] = 1; }
            if kind == StdTemporalKind::PlainMonthDay { values[0] = 0; values[9] = 1972; }
            if kind == StdTemporalKind::PlainDateTime {
                let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                std_temporal_validate_time_offset(time)?;
                std_temporal_parse_time(std_temporal_time_prefix(time), &mut values)?;
            } else if !tail.is_empty() && !matches!(kind, StdTemporalKind::PlainDate) {
                if !tail.starts_with('T') && !tail.starts_with('t') && !tail.starts_with(' ') { return Err(invalid()); }
                let mut ignored = [0; 10];
                std_temporal_validate_time_offset(&tail[1..])?;
                std_temporal_parse_time(std_temporal_time_prefix(&tail[1..]), &mut ignored)?;
            } else if !tail.is_empty() {
                let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                let mut ignored = [0; 10];
                std_temporal_validate_time_offset(time)?;
                std_temporal_parse_time(std_temporal_time_prefix(time), &mut ignored)?;
            }
        }
    }
    std_temporal_validate(kind, &values)?;
    Ok(values)
}

// ToRelativeTemporalObject (used only by Duration.prototype.{round,total,compare}'s
// relativeTo handling) additionally requires a string relativeTo carrying both a
// bracketed time zone annotation and an explicit numeric UTC offset to be
// internally consistent, even though the plain date/time fields end up all that
// gets kept. General Temporal string parsing (Plain*.from, .compare, etc.) must
// NOT run this check: a mismatched offset there is simply ignored per spec, and
// applying it unconditionally in std_temporal_parse regressed a wide swath of
// otherwise-valid Plain-type parsing (e.g. PlainDate.from with a UTC-offset or
// zone-annotation suffix).
fn std_temporal_validate_relative_to_offset(text: &str) -> Result<(), Throw> {
    let original = text;
    let (text, _calendar, zone) = std_temporal_parse_annotations(text)?;
    if zone.is_none() || text.ends_with('Z') || text.ends_with('z') { return Ok(()); }
    let has_offset = text.find('T').or_else(|| text.find('t')).or_else(|| text.find(' '))
        .is_some_and(|time_start| text[time_start + 1..].char_indices()
            .any(|(index, c)| index > 0 && (c == '+' || c == '-')));
    if has_offset {
        std_epoch_parse(original, StdEpochKind::ZonedDateTime, "reject", "compatible")?;
    }
    Ok(())
}

fn std_temporal_from_fields(i: &mut Interpreter, kind: StdTemporalKind, value: JsValue, reject: bool) -> Result<[i64; 10], Throw> {
    if let Ok(data) = std_temporal_record(&value, Some(kind)) { return Ok(data.fields); }
    if let Ok(data) = std_temporal_record(&value, None) {
        let compatible = matches!((kind, data.kind),
            (StdTemporalKind::PlainDate, StdTemporalKind::PlainDateTime)
            | (StdTemporalKind::PlainTime, StdTemporalKind::PlainDateTime)
            | (StdTemporalKind::PlainDateTime, StdTemporalKind::PlainDate)
            | (StdTemporalKind::PlainYearMonth, StdTemporalKind::PlainDate)
            | (StdTemporalKind::PlainYearMonth, StdTemporalKind::PlainDateTime)
            | (StdTemporalKind::PlainMonthDay, StdTemporalKind::PlainDate)
            | (StdTemporalKind::PlainMonthDay, StdTemporalKind::PlainDateTime));
        if compatible {
            let mut fields = data.fields;
            match kind {
                StdTemporalKind::PlainDate => fields[3..9].fill(0),
                StdTemporalKind::PlainTime => fields[0..3].fill(0),
                StdTemporalKind::PlainDateTime if data.kind == StdTemporalKind::PlainDate => fields[3..9].fill(0),
                StdTemporalKind::PlainYearMonth => fields[2] = 1,
                StdTemporalKind::PlainMonthDay => { fields[0] = 0; fields[9] = 1972; },
                _ => {}
            }
            return Ok(fields);
        }
    }
    if let Ok(data) = std_epoch_record(&value, Some(StdEpochKind::ZonedDateTime)) {
        if matches!(kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainTime | StdTemporalKind::PlainDateTime) {
            return std_epoch_local_fields(&data);
        }
    }
    match &value {
        JsValue::Str(text) => std_temporal_parse(kind, &text.to_utf8_lossy()),
        JsValue::Object(_) => {
            let mut fields = std_temporal_bag(i, kind, &value)?;
            std_temporal_regulate(kind, &mut fields, reject);
            std_temporal_validate(kind, &fields)?;
            Ok(fields)
        }
        _ => Err(Throw::type_error("invalid Temporal input")),
    }
}

fn std_temporal_from(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let item = std_reflect_arg(&args, 0);
    // Three input shapes read `item` and `options` in different, spec-fixed
    // (and Proxy-observable) orders:
    //  - a plain property bag: PrepareCalendarFields reads item's own
    //    properties BEFORE GetTemporalOverflowOption touches options;
    //  - a string: ParseISODate(item) runs (and can throw RangeError for bad
    //    syntax) BEFORE options is examined at all;
    //  - anything else (an existing Temporal record, or a ZonedDateTime being
    //    converted): GetTemporalOverflowOption(options) is validated first,
    //    and only then is the already-fully-specified result produced/cloned.
    // Only the bag case actually lets overflow's *value* affect the result;
    // the other two validate-and-discard it, matching the original behavior.
    let is_bag = kind != StdTemporalKind::Duration && item.is_object()
        && std_temporal_record(&item, None).is_err()
        && std_epoch_record(&item, Some(StdEpochKind::ZonedDateTime)).is_err();
    let fields = if is_bag {
        let mut fields = std_temporal_bag(i, kind, &item)?;
        let reject = std_temporal_overflow(i, std_reflect_arg(&args, 1))?;
        std_temporal_regulate(kind, &mut fields, reject);
        std_temporal_validate(kind, &fields)?;
        fields
    } else if matches!(item, JsValue::Str(_)) {
        let fields = std_temporal_from_fields(i, kind, item, false)?;
        if kind != StdTemporalKind::Duration { std_temporal_overflow(i, std_reflect_arg(&args, 1))?; }
        fields
    } else {
        let reject = if kind == StdTemporalKind::Duration { false }
            else { std_temporal_overflow(i, std_reflect_arg(&args, 1))? };
        std_temporal_from_fields(i, kind, item, reject)?
    };
    std_temporal_make(i, kind, fields)
}

fn std_temporal_compare(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let left = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 0), false)?;
    let right = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 1), false)?;
    let ordering = if kind == StdTemporalKind::Duration {
        // GetOptionsObject + ToRelativeTemporalObject (spec steps 3-4) run
        // unconditionally -- a wrong options type or a malformed relativeTo
        // string throws even when the two durations turn out identical and
        // the parsed relativeTo would otherwise go unused.
        let options = std_reflect_arg(&args, 2);
        let relative = std_temporal_option_value(i, &options, "relativeTo")?;
        // Same two relativeTo checks as Duration.prototype.round/total: a
        // property bag's `timeZone` must still be resolved (and can still
        // throw) even though only the local date is kept, and a relativeTo
        // string carrying both a bracketed zone annotation and an explicit
        // numeric UTC offset must be internally consistent.
        if relative.is_object() {
            let time_zone = i.get_prop(&relative, "timeZone")?;
            if !time_zone.is_undefined() { std_epoch_zone_value(i, &time_zone)?; }
        }
        if let JsValue::Str(text) = &relative {
            std_temporal_validate_relative_to_offset(&text.to_utf8_lossy())?;
        }
        let relative_base = if relative.is_undefined() { None } else {
            Some(std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative, false)?)
        };
        if left == right {
            // Identical Duration Records (spec step 5): equal even when a
            // calendar unit is non-zero and relativeTo was never resolved.
            std::cmp::Ordering::Equal
        } else {
            // Years, months, AND weeks all need relativeTo (index 2 is
            // weeks, not part of a [0..2) years/months-only slice) -- days
            // (index 3) alone never does, since a day is always exactly
            // 86,400 seconds in ISO.
            let calendar = left[0..3].iter().chain(right[0..3].iter()).any(|v| *v != 0);
            match (calendar, relative_base) {
                (true, None) => return Err(Throw::range_error("calendar units require relativeTo")),
                (true, Some(base)) => {
                    let total = |duration: &[i64; 10]| -> Result<i128, Throw> {
                        let target = std_temporal_add_calendar(&base, duration, 1, false)?;
                        let days = std_temporal_epoch_day(target[0], target[1], target[2])
                            - std_temporal_epoch_day(base[0], base[1], base[2]);
                        Ok(days as i128 * STD_TEMPORAL_NS_DAY
                            + std_temporal_duration_time_ns(duration)
                            - (duration[2] as i128 * 7 + duration[3] as i128) * STD_TEMPORAL_NS_DAY)
                    };
                    total(&left)?.cmp(&total(&right)?)
                }
                (false, _) => std_temporal_duration_time_ns(&left).cmp(&std_temporal_duration_time_ns(&right)),
            }
        }
    } else if kind == StdTemporalKind::PlainMonthDay {
        (left[9], left[1], left[2]).cmp(&(right[9], right[1], right[2]))
    } else { left.cmp(&right) };
    Ok(JsValue::num(match ordering {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}

macro_rules! temporal_statics {
    ($from:ident, $compare:ident, $kind:expr) => {
        fn $from(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
            std_temporal_from(i, args, $kind)
        }
        fn $compare(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
            std_temporal_compare(i, args, $kind)
        }
    };
}
temporal_statics!(std_temporal_duration_from, std_temporal_duration_compare, StdTemporalKind::Duration);
temporal_statics!(std_temporal_date_from, std_temporal_date_compare, StdTemporalKind::PlainDate);
temporal_statics!(std_temporal_time_from, std_temporal_time_compare, StdTemporalKind::PlainTime);
temporal_statics!(std_temporal_datetime_from, std_temporal_datetime_compare, StdTemporalKind::PlainDateTime);
temporal_statics!(std_temporal_yearmonth_from, std_temporal_yearmonth_compare, StdTemporalKind::PlainYearMonth);
temporal_statics!(std_temporal_monthday_from, std_temporal_monthday_compare, StdTemporalKind::PlainMonthDay);

fn std_temporal_equals(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let other = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0), false)?;
    Ok(JsValue::Bool(data.fields == other))
}

fn std_temporal_duration_abs(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::Duration))?.fields;
    for field in &mut fields { *field = field.abs(); }
    std_temporal_make(i, StdTemporalKind::Duration, fields)
}

fn std_temporal_duration_negated(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::Duration))?.fields;
    for field in &mut fields { *field = -*field; }
    std_temporal_make(i, StdTemporalKind::Duration, fields)
}

fn std_temporal_with_calendar(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    if !matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime |
        StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay) {
        return Err(Throw::type_error("incompatible Temporal receiver"));
    }
    std_temporal_calendar_from_value(i, &std_reflect_arg(&args, 0))?;
    std_temporal_make(i, data.kind, data.fields)
}

fn std_temporal_date_to_datetime(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?.fields;
    let time = std_reflect_arg(&args, 0);
    if !time.is_undefined() {
        let parts = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time, false)?;
        fields[3..9].copy_from_slice(&parts[3..9]);
    }
    std_temporal_make(i, StdTemporalKind::PlainDateTime, fields)
}

fn std_temporal_date_to_yearmonth(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?.fields;
    fields[2] = 1;
    std_temporal_make(i, StdTemporalKind::PlainYearMonth, fields)
}

fn std_temporal_date_to_monthday(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?.fields;
    fields[9] = 1972;
    fields[0] = 0;
    std_temporal_make(i, StdTemporalKind::PlainMonthDay, fields)
}

fn std_temporal_datetime_to_date(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?.fields;
    fields[3..9].fill(0);
    std_temporal_make(i, StdTemporalKind::PlainDate, fields)
}

fn std_temporal_datetime_to_time(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?.fields;
    fields[0..3].fill(0);
    std_temporal_make(i, StdTemporalKind::PlainTime, fields)
}

fn std_temporal_yearmonth_to_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainYearMonth))?.fields;
    let bag = std_reflect_arg(&args, 0);
    if !bag.is_object() { return Err(Throw::type_error("day property bag required")); }
    fields[2] = std_temporal_property(i, &bag, "day")?
        .ok_or_else(|| Throw::type_error("day is required"))?;
    std_temporal_make(i, StdTemporalKind::PlainDate, fields)
}

fn std_temporal_monthday_to_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainMonthDay))?.fields;
    let bag = std_reflect_arg(&args, 0);
    if !bag.is_object() { return Err(Throw::type_error("year property bag required")); }
    fields[0] = std_temporal_property(i, &bag, "year")?
        .ok_or_else(|| Throw::type_error("year is required"))?;
    std_temporal_make(i, StdTemporalKind::PlainDate, fields)
}

fn std_temporal_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let field = name.strip_prefix("get ").unwrap_or("");
    let v = data.fields;
    let result = match field {
        "calendarId" => return Ok(JsValue::str("iso8601")),
        "era" | "eraYear" => return Ok(JsValue::Undefined),
        "monthCode" => return Ok(JsValue::str(format!("M{:02}", v[1]))),
        "inLeapYear" => return Ok(JsValue::Bool(std_temporal_leap(v[0]))),
        "blank" => return Ok(JsValue::Bool(v.iter().all(|part| *part == 0))),
        "sign" => v.iter().find(|part| **part != 0).map(|part| part.signum()).unwrap_or(0),
        "daysInMonth" => std_temporal_days_in_month(v[0], v[1]),
        "daysInYear" => if std_temporal_leap(v[0]) { 366 } else { 365 },
        "monthsInYear" => 12,
        "daysInWeek" => 7,
        "dayOfWeek" => std_temporal_day_of_week(std_temporal_epoch_day(v[0], v[1], v[2])),
        "dayOfYear" => std_temporal_epoch_day(v[0], v[1], v[2]) - std_temporal_epoch_day(v[0], 1, 1) + 1,
        "weekOfYear" | "yearOfWeek" => {
            let day = std_temporal_epoch_day(v[0], v[1], v[2]);
            let week_year = if day < std_temporal_week_start(v[0]) { v[0] - 1 }
                else if day >= std_temporal_week_start(v[0] + 1) { v[0] + 1 }
                else { v[0] };
            if field == "yearOfWeek" { week_year }
            else { (day - std_temporal_week_start(week_year)) / 7 + 1 }
        }
        "year" | "years" => v[0],
        "month" | "months" => v[1],
        "day" | "weeks" => if field == "day" { v[2] } else { v[2] },
        "days" => v[3],
        "hour" => v[3],
        "minute" => v[4],
        "second" => v[5],
        "millisecond" => v[6],
        "microsecond" => v[7],
        "nanosecond" => v[8],
        "hours" => v[4],
        "minutes" => v[5],
        "seconds" => v[6],
        "milliseconds" => v[7],
        "microseconds" => v[8],
        "nanoseconds" => v[9],
        _ => return Ok(JsValue::Undefined),
    };
    Ok(JsValue::num(result as f64))
}

fn std_temporal_year_text(year: i64) -> String {
    if (0..=9999).contains(&year) { format!("{year:04}") }
        else if year < 0 { format!("-{:06}", -year) }
        else { format!("+{year:06}") }
}

fn std_temporal_date_text(year: i64, month: i64, day: i64) -> String {
    format!("{}-{month:02}-{day:02}", std_temporal_year_text(year))
}

fn std_temporal_time_text(fields: &[i64; 10]) -> String {
    let mut result = format!("{:02}:{:02}:{:02}", fields[3], fields[4], fields[5]);
    let fraction = fields[6] * 1_000_000 + fields[7] * 1_000 + fields[8];
    if fraction != 0 { result.push_str(&format!(".{fraction:09}").trim_end_matches('0')); }
    result
}

fn std_temporal_time_text_options(fields: &[i64; 10], digits: Option<usize>, minute_only: bool) -> String {
    if minute_only { return format!("{:02}:{:02}", fields[3], fields[4]); }
    let mut result = format!("{:02}:{:02}:{:02}", fields[3], fields[4], fields[5]);
    let fraction = fields[6] * 1_000_000 + fields[7] * 1_000 + fields[8];
    if let Some(count) = digits {
        if count != 0 { result.push_str(&format!(".{fraction:09}")[..count + 1]); }
    } else if fraction != 0 {
        result.push_str(&format!(".{fraction:09}").trim_end_matches('0'));
    }
    result
}

fn std_temporal_string_precision(i: &mut Interpreter, option: &JsValue, allow_minute: bool) -> Result<(Option<usize>, bool, Option<i128>, String), Throw> {
    if !option.is_object() { return Ok((None, false, None, "trunc".to_string())); }
    let digits_value = i.get_prop(option, "fractionalSecondDigits")?;
    // GetOption dispatches on the ACTUAL type of the value: only a primitive
    // Number takes the ToNumber branch (then floors, per
    // GetStringOrNumberOption step 3.b); every other type -- including
    // Boolean, Null, BigInt, Object and a boxed Number -- goes through
    // ToString instead (which itself throws TypeError for a Symbol), and
    // any resulting string other than "auto" is invalid.
    let mut digits = if digits_value.is_undefined() { None }
        else if matches!(digits_value, JsValue::Num(_)) {
            let number = i.to_number(&digits_value)?;
            if !number.is_finite() { return Err(Throw::range_error("invalid fractionalSecondDigits")); }
            let floored = number.floor();
            if !(0.0..=9.0).contains(&floored) {
                return Err(Throw::range_error("invalid fractionalSecondDigits"));
            }
            Some(floored as usize)
        } else {
            let text = i.to_string(&digits_value)?;
            if text == "auto" { None }
            else { return Err(Throw::range_error("invalid fractionalSecondDigits")); }
        };
    let mode = std_temporal_option_string(i, option, "roundingMode")?
        .unwrap_or_else(|| "trunc".to_string());
    if !std_temporal_round_mode_valid(&mode) {
        return Err(Throw::range_error("invalid roundingMode"));
    }
    let unit = std_temporal_option_string(i, option, "smallestUnit")?;
    let minute_only = allow_minute && unit.as_deref().is_some_and(|name| name.strip_suffix('s').unwrap_or(name) == "minute");
    let step = if let Some(unit) = unit {
        let unit = unit.strip_suffix('s').unwrap_or(&unit);
        digits = match unit {
            "minute" if allow_minute => Some(0), "second" => Some(0), "millisecond" => Some(3),
            "microsecond" => Some(6), "nanosecond" => Some(9),
            _ => return Err(Throw::range_error("invalid smallestUnit")),
        };
        Some(std_temporal_unit_ns(unit).unwrap())
    } else { digits.map(|count| 10i128.pow((9 - count) as u32)) };
    Ok((digits, minute_only, step, mode))
}

fn std_temporal_to_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let mut v = data.fields;
    let option = std_reflect_arg(&args, 0);
    if !option.is_undefined() && !option.is_object() {
        return Err(Throw::type_error("Temporal options must be an object"));
    }
    // Duration.prototype.toString has no calendarName option (Durations
    // carry no calendar) -- reading it at all is observable (and wrong)
    // via a Proxy-like options bag, confirmed by
    // Duration/prototype/toString/order-of-operations.js, whose expected
    // property-access log never includes "get options.calendarName".
    let calendar_name = if data.kind == StdTemporalKind::Duration { "auto".to_string() }
        else if option.is_undefined() { "auto".to_string() }
        else {
            let value = i.get_prop(&option, "calendarName")?;
            if value.is_undefined() { "auto".to_string() } else { i.to_string(&value)? }
        };
    if !matches!(calendar_name.as_str(), "auto" | "always" | "never" | "critical") {
        return Err(Throw::range_error("invalid calendarName"));
    }
    let (digits, minute_only, step, mode) =
        std_temporal_string_precision(i, &option, data.kind != StdTemporalKind::Duration)?;
    if let Some(step) = step {
        if data.kind == StdTemporalKind::PlainTime {
            let rounded = std_temporal_round_integer(std_temporal_time_ns(&v), step, &mode)?;
            std_temporal_set_time_ns(&mut v, rounded);
        } else if data.kind == StdTemporalKind::PlainDateTime {
            let rounded = std_temporal_round_integer(std_temporal_epoch_ns(&v), step, &mode)?;
            std_temporal_set_epoch_ns(&mut v, rounded)?;
        }
    }
    let show_calendar = matches!(calendar_name.as_str(), "always" | "critical");
    let mut text = match data.kind {
        StdTemporalKind::PlainDate => std_temporal_date_text(v[0], v[1], v[2]),
        StdTemporalKind::PlainTime => std_temporal_time_text_options(&v, digits, minute_only),
        StdTemporalKind::PlainDateTime => format!("{}T{}", std_temporal_date_text(v[0], v[1], v[2]),
            std_temporal_time_text_options(&v, digits, minute_only)),
        StdTemporalKind::PlainYearMonth => if show_calendar {
            std_temporal_date_text(v[0], v[1], v[2])
        } else { format!("{}-{:02}", std_temporal_year_text(v[0]), v[1]) },
        StdTemporalKind::PlainMonthDay => if show_calendar {
            std_temporal_date_text(v[9], v[1], v[2])
        } else { format!("{:02}-{:02}", v[1], v[2]) },
        StdTemporalKind::Duration => std_temporal_duration_to_string(&v, digits, step, &mode)?,
    };
    if show_calendar && !matches!(data.kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        if calendar_name == "critical" { text.push_str("[!u-ca=iso8601]"); }
        else { text.push_str("[u-ca=iso8601]"); }
    }
    Ok(JsValue::str(text))
}

// Duration.prototype.toString (TemporalDurationToString in the spec).
//
// Two independent mechanisms feed the same day..nanosecond fields:
//
//  (A) When an explicit precision was requested (`smallestUnit` or a numeric
//      `fractionalSecondDigits`, i.e. `step.is_some()`), the whole
//      day-through-nanosecond span is summed to exact nanoseconds, rounded
//      to the requested increment/mode, then re-split starting at
//      `balance_rank`: the rank of the highest-ranked nonzero field, clamped
//      up to at least "day" (years/months/weeks can't be redistributed
//      without a `relativeTo`, so they are clamped away from the rounding
//      but otherwise left untouched). Ranks below "day" are never promoted
//      further on their own — a duration whose only populated field is
//      "seconds" stays capped at "second" even when the rounded value
//      exceeds 60, since balance_rank there is already 6 ("second").
//  (B) Otherwise (`step.is_none()`, "auto" precision), only a mandatory,
//      much smaller ns -> us -> ms -> seconds overflow cascade runs, and
//      minutes/hours/days are never touched.
//
// `std_temporal_validate` re-runs the Duration safe-integer/consistent-sign
// checks afterward, since rounding under mechanism (A) can overflow a field
// out of the ECMA-262 safe-integer range (float64-representable) even when
// the original duration was in range.
fn std_temporal_duration_to_string(
    fields: &[i64; 10],
    digits: Option<usize>,
    step: Option<i128>,
    mode: &str,
) -> Result<String, Throw> {
    let mut f = *fields;
    let precision_auto = step.is_none();
    if let Some(step) = step {
        // `balance_rank` is the rank (0=year..9=nanosecond) that absorbs the
        // rounded total: the highest-ranked nonzero field, but never
        // coarser than "day" (years/months/weeks can't be redistributed
        // without a relativeTo, so a duration with any of those nonzero
        // still only rebalances day-and-finer -- confirmed by
        // round-cross-unit-boundary.js, where 1Y11M30D23:59:59.999999999
        // rounds up into 31D, not into the months). Only ranks
        // `balance_rank..=9` are summed and rewritten; everything coarser
        // (years/months/weeks when balance_rank is "day", or nothing, when
        // balance_rank is coarser than "day" because minutes/hours/days are
        // themselves all zero) is left exactly as it was -- confirmed by
        // fractionalseconddigits-number.js, where weeks/days stay
        // untouched while only the seconds fraction rounds, and by
        // round-cross-unit-boundary.js's "no balancing if smallest unit is
        // largest unit" case, where a bare `seconds` field rounds past 60
        // without ever promoting into minutes.
        let unit_ns = |rank: usize| -> i128 {
            match rank {
                3 => STD_TEMPORAL_NS_DAY,
                4 => 3_600 * STD_TEMPORAL_NS_SECOND,
                5 => 60 * STD_TEMPORAL_NS_SECOND,
                6 => STD_TEMPORAL_NS_SECOND,
                7 => 1_000_000,
                8 => 1_000,
                _ => 1,
            }
        };
        let largest_rank = (0..10).find(|&idx| f[idx] != 0).unwrap_or(9);
        let balance_rank = largest_rank.max(3);
        let total_ns: i128 = (balance_rank..10).map(|rank| f[rank] as i128 * unit_ns(rank)).sum();
        let rounded = std_temporal_round_integer(total_ns, step, mode)?;
        let sign: i64 = if rounded < 0 { -1 } else { 1 };
        let mut remainder = rounded.unsigned_abs() as i128;
        f[balance_rank] = sign * ((remainder / unit_ns(balance_rank)) as i64);
        remainder %= unit_ns(balance_rank);
        for rank in (balance_rank + 1)..=9 {
            f[rank] = sign * ((remainder / unit_ns(rank)) as i64);
            remainder %= unit_ns(rank);
        }
        std_temporal_validate(StdTemporalKind::Duration, &f)?;
    } else {
        f[8] += f[9] / 1_000; f[9] %= 1_000;
        f[7] += f[8] / 1_000; f[8] %= 1_000;
        f[6] += f[7] / 1_000; f[7] %= 1_000;
    }
    let negative = f.iter().any(|value| *value < 0);
    let mut text = if negative { String::from("-P") } else { String::from("P") };
    for (index, suffix) in [(0, "Y"), (1, "M"), (2, "W"), (3, "D")] {
        if f[index] != 0 { text.push_str(&format!("{}{suffix}", f[index].abs())); }
    }
    // sec-temporaldurationtostring step 21: show the seconds (+fraction)
    // part when it is itself nonzero, or when the duration has no other
    // populated component, or whenever a precision was explicitly
    // requested (even a zero result must still be printed, e.g. "PT0S").
    let show_seconds = f[6] != 0 || f[7] != 0 || f[8] != 0 || f[9] != 0
        || f[0..6].iter().all(|value| *value == 0)
        || !precision_auto;
    if f[4] != 0 || f[5] != 0 || show_seconds {
        text.push('T');
        for (index, suffix) in [(4, "H"), (5, "M")] {
            if f[index] != 0 { text.push_str(&format!("{}{suffix}", f[index].abs())); }
        }
        if show_seconds {
            text.push_str(&f[6].abs().to_string());
            let fraction = f[7].abs() * 1_000_000 + f[8].abs() * 1_000 + f[9].abs();
            if let Some(count) = digits {
                if count != 0 { text.push_str(&format!(".{fraction:09}")[..count + 1]); }
            } else if fraction != 0 {
                text.push_str(&format!(".{fraction:09}").trim_end_matches('0'));
            }
            text.push('S');
        }
    }
    Ok(text)
}

fn std_temporal_to_json(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_to_string(i, this, vec![])
}

fn std_temporal_to_locale_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_to_string(i, this, vec![])
}

fn std_temporal_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_record(&this, None)?;
    Err(Throw::type_error("Temporal values cannot be converted to primitives"))
}
