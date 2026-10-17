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
    // These fields are ALWAYS in the proleptic-Gregorian (ISO) calendar
    // regardless of `calendar` below -- a non-iso8601 calendar's own
    // year/month/day/era/monthCode/etc. are VIEWS computed on demand from
    // these ISO fields (see builtins_temporal_calendar.rs). `calendar` is
    // unused (always "iso8601") for Duration/PlainTime, which have no
    // calendar concept.
    fields: [i64; 10],
    calendar: String,
    // Exact Duration fields (see builtins_temporal_duration.rs); zero for every other kind.
    wide: [i128; 10],
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
            let constructor = Self::native_constructor(name, std_temporal_constructor(kind), &self.function_proto);
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
                let property = Property { accessor: true, value: JsValue::Undefined, writable: false,
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
    std_temporal_make_cal(i, kind, fields, "iso8601".to_string())
}

fn std_temporal_make_cal(i: &Interpreter, kind: StdTemporalKind, mut fields: [i64; 10], calendar: String) -> Result<JsValue, Throw> {
    // Every Duration Record field is spec-constrained to a float64-
    // representable integer (Number, not an exact big integer), so the
    // internal representation must be canonicalized here -- the single
    // choke point every Duration passes through on creation (constructor,
    // add/subtract/round/negated/abs/with/from) -- not just at the getter.
    // Confirmed by float64-representable-integer.js: an add() result that
    // overflows 2**53 loses precision, and a LATER add()/toString()/compare
    // must see that already-lossy value, not a hidden exact one.
    if kind == StdTemporalKind::Duration {
        return std_temporal_make_duration(i, std_temporal_duration_widen(&fields));
    }
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
        StdTemporalData { owner: Rc::downgrade(&object.0), kind, fields, calendar, wide: [0; 10] }));
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
            StdTemporalKind::PlainYearMonth, StdTemporalKind::PlainMonthDay, StdTemporalKind::PlainTime]
            .iter().any(|kind| std_temporal_parse(*kind, &name.to_utf8_lossy()).is_ok())
            || std_epoch_parse_syntax(&name.to_utf8_lossy(), StdEpochKind::Instant).is_ok() {
            return Ok(());
        }
    }
    std_temporal_iso_calendar(i, value)
}

// ToTemporalCalendarIdentifier (spec), for the Plain* kinds: unlike
// std_temporal_iso_calendar/std_temporal_calendar_from_value above (kept
// iso8601-only and used only by ZonedDateTime's calendar gate -- ADR 0013's
// JSS-calendars port has not yet extended ZonedDateTime), this resolves and
// returns the canonical calendar id for any of the calendars
// builtins_temporal_calendar.rs supports. A bare calendar id string
// (case-insensitively, with its documented CLDR aliases) is the common case;
// an existing Temporal date-like object carries its own stored calendar
// forward; a full ISO string bearing a non-iso8601 [u-ca=...] annotation as
// the CALENDAR ARGUMENT ITSELF (not the value being converted -- see
// std_temporal_parse for that far more common case) is a rare form left
// iso8601-only, matching this function's prior behavior for strings that
// don't parse as a bare calendar id.
fn std_temporal_resolve_calendar_id(i: &mut Interpreter, value: &JsValue) -> Result<String, Throw> {
    std_temporal_resolve_calendar_id_ex(i, value, true)
}

// The constructor's own calendar argument is stricter than every other
// calendar-argument site (with(), withCalendar(), a bag's own "calendar"
// property): those fall back to accepting a full ISO string carrying an
// iso8601 annotation, e.g. "2020-01-01[u-ca=iso8601]" (confirmed valid by
// argument-propertybag-calendar-iso-string.js and withCalendar's own
// calendar-iso-string.js), but `new Temporal.PlainDate(y, m, d, calendarArg)`
// rejects that exact same form -- confirmed by calendar-invalid-iso-
// string.js's "1997-12-04[u-ca=iso8601]" case, which must throw RangeError.
fn std_temporal_resolve_calendar_id_strict(i: &mut Interpreter, value: &JsValue) -> Result<String, Throw> {
    std_temporal_resolve_calendar_id_ex(i, value, false)
}

fn std_temporal_resolve_calendar_id_ex(i: &mut Interpreter, value: &JsValue, allow_iso_string_fallback: bool) -> Result<String, Throw> {
    if value.is_undefined() { return Ok("iso8601".to_string()); }
    if let Ok(record) = std_temporal_record(value, None) {
        if matches!(record.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime
            | StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay) {
            return Ok(record.calendar.clone());
        }
    }
    if let Ok(data) = std_epoch_record(value, Some(StdEpochKind::ZonedDateTime)) {
        return Ok(data.calendar.clone());
    }
    if !matches!(value, JsValue::Str(_)) {
        return Err(Throw::type_error("Temporal calendar must be a string or Temporal object"));
    }
    let name = i.to_string(value)?;
    if let Some(canonical) = std_calendar_canonicalize(&name) { return Ok(canonical.to_string()); }
    if allow_iso_string_fallback && std_temporal_calendar_from_value(i, value).is_ok() {
        // The calendar of an annotated ISO string is its [u-ca=...] value.
        if let Some(start) = name.find("[u-ca=").or_else(|| name.find("[!u-ca=")) {
            let rest = &name[start..];
            let rest = &rest[rest.find('=').unwrap_or(0) + 1..];
            if let Some(canonical) = rest.find(']').and_then(|end| std_calendar_canonicalize(&rest[..end])) {
                return Ok(canonical.to_string());
            }
        }
        return Ok("iso8601".to_string());
    }
    Err(Throw::range_error("invalid Temporal calendar"))
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
        // Days-through-nanoseconds are bounded only by the total, not
        // individually: a days-only (or, after `since`/`until` collapses a
        // full-precision span into a single requested largestUnit, a
        // nanoseconds-only) span can run far past 2**53-1 in that one field
        // while the exact total expressed in seconds is still safely in
        // range -- e.g. Duration/max.js's "max days" case, and
        // PlainDateTime's own since/until `largestunit-plurals-accepted.js`/
        // `float64-representable-integer.js`/`units-changed.js` at
        // `largestUnit: "nanoseconds"`. `Duration.prototype.round`'s own
        // narrower rule -- that the SPECIFIC field its BalanceTimeDuration
        // conversion produces must itself be float64-safe, confirmed by
        // `round/out-of-range-when-converting-from-normalized-duration.js`
        // -- is a property of that one conversion algorithm, not of Duration
        // validity in general, so it is checked locally in
        // `std_temporal_round_duration` instead of here.
        if values.iter().any(|v| *v != 0 && v.signum() != sign)
            || values[..3].iter().any(|v| v.abs() > 4_294_967_295)
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
        if matches!(kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime | StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay) {
            // ISODateTimeWithinLimits (spec): PlainDateTime can represent a
            // point in time within 24h (nsPerDay) of Instant's own
            // +-nsMaxInstant boundary on either side -- a plain day-count
            // check is not equivalent, since a date exactly one day past
            // Instant's boundary is valid at some times of day and not
            // others (e.g. year -271821-04-19T00:00 is invalid, but the
            // same date at any nanosecond after midnight is valid).
            // PlainDate has no time-of-day of its own, so its own
            // ISODateWithinLimits defers to this same check with hour 12 (a
            // neutral reference time comfortably inside the padding at
            // either extreme) rather than actual midnight. PlainYearMonth
            // (ISOYearMonthWithinLimits) represents the WHOLE month, not a
            // single day, so it defers to this same check at whichever of
            // day 1 or the month's last day is closer to being in range --
            // a coarse year-only bound is not tight enough at the extremes,
            // but neither is checking day 1 alone: e.g. -271821-04 (April)
            // is a VALID minimum year-month even though its day 1 (April 1)
            // falls before the ISO limit (April 19), because its LAST day
            // (April 30) does not -- confirmed by `PlainYearMonth/from/
            // limits.js`'s "min object"/"max object" cases. Conversely a
            // calendar's earliest representable year-month can still fail
            // this finer, day-precise check at BOTH its first and last day
            // even though the bare year number is in range (confirmed by
            // every calendar's `extreme-unsupported-dates.js`, e.g. buddhist
            // year -271278 month 3 -- ISO year -271821 month 3 (March),
            // entirely before the April 19 limit at either endpoint).
            const NS_PER_DAY: i128 = 86_400_000_000_000;
            const NS_MAX_INSTANT: i128 = 8_640_000_000_000_000_000_000;
            let time_ns: i128 = if kind == StdTemporalKind::PlainDateTime {
                hour as i128 * 3_600_000_000_000
                    + minute as i128 * 60_000_000_000
                    + second as i128 * 1_000_000_000
                    + millisecond as i128 * 1_000_000
                    + microsecond as i128 * 1_000
                    + nanosecond as i128
            } else {
                12 * 3_600_000_000_000
            };
            let in_range = |d: i64| {
                let total_ns = std_temporal_epoch_day(actual_year, month, d) as i128 * NS_PER_DAY + time_ns;
                total_ns > -NS_MAX_INSTANT - NS_PER_DAY && total_ns < NS_MAX_INSTANT + NS_PER_DAY
            };
            let within = if kind == StdTemporalKind::PlainYearMonth {
                in_range(1) || in_range(std_temporal_days_in_month(actual_year, month))
            } else {
                in_range(day)
            };
            if !within {
                return Err(Throw::range_error("ISO date outside Temporal range"));
            }
        }
        // ISOYearMonthWithinLimits (spec): PlainYearMonth can represent any
        // month that contains a day PlainDate can represent -- coarser than
        // ISODateWithinLimits's day-precise boundary, since it drops the
        // day/time-of-day components entirely and checks only year+month at
        // the two extreme years.
        if kind == StdTemporalKind::PlainYearMonth
            && ((actual_year == -271_821 && month < 4) || (actual_year == 275_760 && month > 9)) {
            return Err(Throw::range_error("ISO year-month outside Temporal range"));
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
    if kind == StdTemporalKind::Duration {
        let mut wide = [0i128; 10];
        for index in 0..10 {
            wide[index] = std_temporal_duration_number_wide(i, std_reflect_arg(&args, index), Some(0))?;
        }
        std_temporal_validate_duration(&wide)?;
        STD_TEMPORAL_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(object),
            StdTemporalData { owner: Rc::downgrade(&object.0), kind, fields: std_temporal_duration_fit(&wide),
                calendar: "iso8601".to_string(), wide }));
        return Ok(this);
    }
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
        values[destination] = std_temporal_number(i, std_reflect_arg(&args, index), (index >= required).then_some(0))?;
    }
    if kind == StdTemporalKind::PlainYearMonth { values[2] = 1; }
    if kind == StdTemporalKind::PlainMonthDay { values[9] = 1972; }
    // Per each constructor's own spec steps (PlainDate/PlainYearMonth/
    // PlainMonthDay alike), the calendar argument's own type-check and
    // canonicalization runs BEFORE IsValidISODate's final range validation
    // -- and, for PlainYearMonth/PlainMonthDay specifically, also BEFORE the
    // trailing referenceISODay/referenceISOYear argument is even read.
    // Confirmed by PlainYearMonth's own calendar-invalid.js: an invalid
    // calendar with a Symbol referenceISODay must throw RangeError from the
    // calendar check, never reaching (and never coercing) that trailing arg.
    let mut calendar_id = "iso8601".to_string();
    if kind != StdTemporalKind::Duration && kind != StdTemporalKind::PlainTime {
        let calendar_arg = match kind {
            StdTemporalKind::PlainDate => 3,
            StdTemporalKind::PlainDateTime => 9,
            StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay => 2,
            _ => unreachable!(),
        };
        let calendar = std_reflect_arg(&args, calendar_arg);
        calendar_id = std_temporal_resolve_calendar_id_strict(i, &calendar)?;
    }
    if kind == StdTemporalKind::PlainMonthDay { values[9] = std_temporal_number(i, std_reflect_arg(&args, 3), Some(1972))?; }
    if kind == StdTemporalKind::PlainYearMonth {
        values[2] = std_temporal_number(i, std_reflect_arg(&args, 3), Some(1))?;
    }
    std_temporal_validate(kind, &values)?;
    STD_TEMPORAL_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(object),
        StdTemporalData { owner: Rc::downgrade(&object.0), kind, fields: values, calendar: calendar_id, wide: [0; 10] }));
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
fn std_temporal_bag(i: &mut Interpreter, kind: StdTemporalKind, value: &JsValue) -> Result<([i64; 10], Option<i64>, Option<String>, String), Throw> {
    let mut fields = [0; 10];
    let mut calendar_id = "iso8601".to_string();
    if !matches!(kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        let calendar = i.get_prop(value, "calendar")?;
        calendar_id = std_temporal_resolve_calendar_id(i, &calendar)?;
    }
    if kind == StdTemporalKind::Duration {
        let wide = std_temporal_duration_bag_wide(i, value)?;
        return Ok((std_temporal_duration_fit(&wide), None, None, calendar_id));
    } else if kind == StdTemporalKind::PlainTime {
        // Reads hour..nanosecond in alphabetical order (hour, microsecond,
        // millisecond, minute, nanosecond, second), not the natural
        // hour->nanosecond order used here before this port -- confirmed
        // wrong by PlainTime's own `since`/`until` `order-of-operations.js`.
        let read = std_temporal_read_calendar_fields(i, kind, &calendar_id, value)?;
        if !read.any_present { return Err(Throw::type_error("time-like object has no fields")); }
        fields[3..9].copy_from_slice(&read.time);
        return Ok((fields, None, None, calendar_id));
    } else {
        // Reads year/month/monthCode/day(/hour..nanosecond for
        // PlainDateTime) in the exact alphabetical order
        // PrepareCalendarFields specifies -- not year, month, monthCode, day
        // as this used to read them, which is only alphabetical by
        // coincidence for that particular subset and was never alphabetical
        // once hour..nanosecond joined for PlainDateTime. Each field's own
        // immediate validation (month/day < 1) happens inside the read, at
        // its own alphabetical position, matching
        // `options-read-before-algorithmic-validation.js`'s distinction
        // between a field's own malformed-value throw (immediate) and
        // cross-field validation (deferred, here: month vs monthCode
        // disagreement, and each field being present at all).
        let read = std_temporal_read_calendar_fields(i, kind, &calendar_id, value)?;
        // era/eraYear (a RangeError for an unrecognized era name, or a
        // mismatch against an explicit `year`) resolve into the same
        // calendar-native year `year` would otherwise supply -- this must
        // run for every kind, INCLUDING PlainMonthDay, even though
        // PlainMonthDay's own fields[0] is always hardcoded to 0 below: an
        // eraYear/year (or monthCode/month) inconsistency must still throw
        // (confirmed by PlainMonthDay's own fields-overspecified.js).
        let resolved_year = std_temporal_resolve_era_year_checked(&calendar_id, &read.era_text, read.era_year, read.year)?;
        // PlainMonthDay stores no calendar-native year at all (fields[0] is
        // always hardcoded to 0 below) and its ISO reference year (1972) is
        // fixed regardless of what's given -- but a wildly out-of-range
        // `year`/`era`+`eraYear` must still throw immediately, rather than
        // silently ignoring it and going on to resolve monthCode against the
        // fixed reference year. Confirmed by dont-calculate-month-info-for-
        // out-of-range-year.js's year: Β±999999 cases (every real calendar's
        // representable range, checked precisely for PlainYearMonth/PlainDate
        // via std_temporal_validate, is comfortably inside Β±300_000).
        if kind == StdTemporalKind::PlainMonthDay && calendar_id != "iso8601" {
            if let Some(y) = resolved_year {
                if !(-300_000..=300_000).contains(&y) {
                    return Err(Throw::range_error("year is outside the representable range"));
                }
            }
        }
        // CalendarResolveFields: every required field's PRESENCE (a
        // TypeError, since a field is simply missing, not malformed) is
        // checked before any monthCode SUITABILITY validation (a
        // RangeError, deferred here) runs -- "month is required" only needs
        // to know THAT monthCode was supplied, not yet whether its value is
        // suitable. Confirmed by calendarresolvefields-error-ordering.js,
        // where an invalid/conflicting monthCode must never preempt a
        // missing year/month/day's TypeError.
        // PlainMonthDay carries its (optional) calendar-native year in
        // fields[0], flagged by fields[3], until the month-day reference
        // resolver consumes it.
        fields[0] = if kind == StdTemporalKind::PlainMonthDay {
            fields[3] = resolved_year.is_some() as i64;
            resolved_year.unwrap_or(0)
        } else {
            resolved_year.ok_or_else(|| Throw::type_error("year is required"))?
        };
        if read.month.is_none() && read.month_code_text.is_none() {
            return Err(Throw::type_error("month is required"));
        }
        fields[2] = match kind {
            StdTemporalKind::PlainYearMonth => 1,
            _ => read.day.ok_or_else(|| Throw::type_error("day is required"))?,
        };
        if kind == StdTemporalKind::PlainMonthDay { fields[9] = 1972; }
        if kind == StdTemporalKind::PlainDateTime { fields[3..9].copy_from_slice(&read.time); }
        // monthCode SUITABILITY (a RangeError: does it name a real month,
        // and does it agree with an explicit `month`?) is deferred to the
        // caller, which may need to read/coerce an `overflow` option in
        // between -- CalendarYearMonthFromFields/CalendarDateFromFields
        // read GetTemporalOverflowOption before running CalendarResolveFields'
        // own suitability check. Confirmed by `.from()`'s own
        // options-read-before-algorithmic-validation.js.
        return Ok((fields, read.month, read.month_code_text, calendar_id));
    }
}

fn std_temporal_bag_resolve_month(fields: &mut [i64; 10], month: Option<i64>, code_text: Option<String>) -> Result<(), Throw> {
    // Duration/PlainTime's own std_temporal_bag branches always return
    // (None, None) here (they have no month/monthCode concept at all) --
    // every caller invokes this unconditionally after std_temporal_bag
    // regardless of kind, so this must be a safe no-op for them rather
    // than unwrapping two Nones.
    if month.is_none() && code_text.is_none() { return Ok(()); }
    let code_month = if let Some(code) = &code_text {
        let number = code.strip_prefix('M').and_then(|digits| digits.parse::<i64>().ok())
            .filter(|number| code.len() == 3 && (1..=12).contains(number))
            .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
        Some(number)
    } else { None };
    if month.zip(code_month).is_some_and(|(a, b)| a != b) {
        return Err(Throw::range_error("month and monthCode disagree"));
    }
    fields[1] = month.or(code_month).unwrap();
    Ok(())
}

// Resolves the month (from `month`/`monthCode`, as read by std_temporal_bag)
// and, for a non-iso8601 calendar, converts the still calendar-native
// year/day (as read into fields[0]/fields[2] by std_temporal_bag) into the
// object's actual internal ISO year/month/day -- original integration glue
// (not ported); the per-calendar tables it calls into
// (builtins_temporal_calendar.rs) are the ported part. PlainMonthDay is left
// iso8601-only here (a documented gap: see std_temporal_monthday_to_date) --
// it has no calendar-native year to convert from at all in this bag path
// (std_temporal_bag always sets its fields[0] to 0), so its month/day are
// simply taken as already-ISO, matching this function's behavior before the
// JSS-calendars port.
fn std_temporal_bag_resolve_calendar(kind: StdTemporalKind, calendar: &str, fields: &mut [i64; 10],
    month: Option<i64>, code_text: Option<String>, reject_overflow: bool) -> Result<(), Throw> {
    if kind == StdTemporalKind::PlainMonthDay {
        let year = if fields[3] == 1 { Some(fields[0]) } else { None };
        let (iso_year, iso_month, iso_day) = std_calendar_monthday_reference(calendar, year, month,
            code_text.as_deref(), fields[2], reject_overflow)?;
        *fields = [0; 10];
        fields[1] = iso_month;
        fields[2] = iso_day;
        fields[9] = iso_year;
        return Ok(());
    }
    if calendar == "iso8601" {
        return std_temporal_bag_resolve_month(fields, month, code_text);
    }
    if month.is_none() && code_text.is_none() { return Ok(()); }
    let native_year = fields[0];
    let native_day = fields[2];
    if !std_calendar_is_month_structure(calendar) {
        // gregory/japanese/roc/buddhist: same ISO month/day grid, only the
        // year numbering differs.
        let m = std_calendar_bag_resolve_plain_month(calendar, month, code_text)?;
        fields[0] = std_calendar_year_to_iso(calendar, native_year);
        fields[1] = m;
        fields[2] = native_day;
        return Ok(());
    }
    let months_in_year = std_calendar_months_in_year(calendar, native_year);
    let m = if std_calendar_has_leap_months(calendar) {
        if let Some(code) = &code_text {
            let (ordinal, existed) = std_calendar_resolve_code_ord(calendar, native_year, code)
                .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
            if reject_overflow && !existed { return Err(Throw::range_error("leap month is absent in this year")); }
            // 2026-09-27 (calendarresolvefields-error-ordering-chinese.js):
            // a month/monthCode disagreement is a RangeError unconditionally
            // once both are given -- `_existed` only distinguishes a
            // genuinely-nonexistent leap month (e.g. "M05L" outside a leap
            // year, which resolve_code_ord already turned into a fallback
            // ordinal) from a real one; it must never suppress this check,
            // or an explicit month that disagrees with a valid monthCode
              // silently wins instead of throwing.
            if month.is_some_and(|value| value != ordinal) {
                return Err(Throw::range_error("month and monthCode disagree"));
            }
            ordinal
        } else {
            month.ok_or_else(|| Throw::type_error("month is required"))?
        }
    } else {
        std_calendar_bag_resolve_plain_month(calendar, month, code_text)?
    };
    // 2026-09-27: month/day bounds for a month-structure calendar are
    // overflow-dependent just like the ISO grid's own RegulateISODate
    // (std_temporal_regulate) -- "reject" throws RangeError instead of
    // clamping. Previously this always clamped regardless of the overflow
    // option, so {overflow:"reject"} silently accepted an out-of-range
    // calendar-native day/month instead of throwing.
    let m = if reject_overflow {
        if m > months_in_year { return Err(Throw::range_error("month is invalid in this calendar")); }
        m
    } else { m.clamp(1, months_in_year) };
    let max_day = std_calendar_month_len(calendar, native_year, m);
    let native_day = if reject_overflow {
        if native_day > max_day { return Err(Throw::range_error("day is invalid in this calendar month")); }
        native_day
    } else { native_day.clamp(1, max_day) };
    let (iso_year, iso_month, iso_day) = std_calendar_to_iso(calendar, native_year, m, native_day);
    fields[0] = iso_year;
    fields[1] = iso_month;
    fields[2] = iso_day;
    Ok(())
}

// month/monthCode resolution shared by every non-leap-month calendar
// (gregory/japanese/roc/buddhist/coptic/ethiopic/ethioaa/islamic-family/
// indian/persian): a plain 1-based month number, with no "L" (leap) suffix
// ever valid on the monthCode.
fn std_calendar_bag_resolve_plain_month(calendar: &str, month: Option<i64>, code_text: Option<String>) -> Result<i64, Throw> {
    let _ = calendar;
    let code_month = if let Some(code) = &code_text {
        if code.ends_with('L') { return Err(Throw::range_error("calendar has no leap months")); }
        let digits = code.strip_prefix('M').ok_or_else(|| Throw::range_error("invalid monthCode"))?;
        Some(digits.parse::<i64>().map_err(|_| Throw::range_error("invalid monthCode"))?)
    } else { None };
    if month.zip(code_month).is_some_and(|(a, b)| a != b) {
        return Err(Throw::range_error("month and monthCode disagree"));
    }
    month.or(code_month).ok_or_else(|| Throw::type_error("month is required"))
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

fn std_temporal_parse_annotations(text: &str, require_known_calendar: bool) -> Result<(&str, Option<&str>, Option<&str>), Throw> {
    let invalid = || Throw::range_error("invalid Temporal annotation");
    let Some(first) = text.find('[') else { return Ok((text, None, None)); };
    let body = &text[..first];
    let mut tail = &text[first..];
    let mut calendar_name: Option<&str> = None;
    let mut critical_calendar = false;
    let mut time_zone = None;
    while !tail.is_empty() {
        let rest = tail.strip_prefix('[').ok_or_else(invalid)?;
        let end = rest.find(']').ok_or_else(invalid)?;
        let value = &rest[..end];
        tail = &rest[end + 1..];
        let annotation = value.strip_prefix('!').unwrap_or(value);
        if let Some(name) = annotation.strip_prefix("u-ca=") {
            // Only the FIRST calendar annotation is ever consulted (later
            // ones are still grammar-checked but otherwise ignored, per
            // "second annotation ignored" in both PlainDate's and
            // PlainTime's own argument-string-calendar-annotation.js). A
            // calendar-bearing kind (PlainDate/PlainDateTime/PlainYearMonth/
            // PlainMonthDay/ZonedDateTime) must reject any id
            // builtins_temporal_calendar.rs's std_calendar_canonicalize
            // does not recognize -- but Temporal.PlainTime has no calendar
            // slot at all and never canonicalizes this value, so any
            // syntactically valid calendar ID (including an outright
            // unknown one) is accepted there and simply discarded.
            if calendar_name.is_none() {
                if require_known_calendar && std_calendar_canonicalize(name).is_none() {
                    return Err(invalid());
                }
                calendar_name = Some(name);
                critical_calendar = value.starts_with('!');
            } else if critical_calendar || value.starts_with('!') {
                // A second annotation is grammar-checked (it must still be
                // well-formed) but never overwrites the first, consulted
                // one -- only whether ITS OWN "!" or the first's "!" makes
                // the pair illegal is checked here.
                return Err(invalid());
            }
        } else if let Some((key, _)) = annotation.split_once('=') {
            if value.starts_with('!') || key.bytes().any(|c| c.is_ascii_uppercase()) {
                return Err(invalid());
            }
        } else if time_zone.is_some() || annotation.is_empty() {
            return Err(invalid());
        } else { time_zone = Some(annotation); }
    }
    Ok((body, calendar_name, time_zone))
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
    Ok(std_temporal_duration_fit(&std_temporal_parse_duration_wide(text)?))
}

fn std_temporal_parse_duration_wide(text: &str) -> Result<StdDuration, Throw> {
    let upper = text.to_ascii_uppercase();
    let text = upper.as_str();
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
            values[index] = whole.parse::<i128>().map_err(|_| invalid())?;
            let mut digits = fraction.to_string();
            while digits.len() < 9 { digits.push('0'); }
            let nanos = digits.parse::<i128>().map_err(|_| invalid())?
                * match index { 4 => 3_600, 5 => 60, _ => 1 };
            let pieces = std_temporal_split_duration_ns_wide(nanos, index + 1);
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

fn std_temporal_time_string_ambiguous(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let number = |part: &str| part.parse::<i64>().unwrap_or(0);
    let bytes = text.as_bytes();
    match bytes.len() {
        6 if digits(text) => (1..=12).contains(&number(&text[4..])),
        7 if bytes[4] == b'-' && digits(&text[..4]) && digits(&text[5..]) => (1..=12).contains(&number(&text[5..])),
        5 if bytes[2] == b'-' && digits(&text[..2]) && digits(&text[3..]) => {
            let (month, day) = (number(&text[..2]), number(&text[3..]));
            (1..=12).contains(&month) && (1..=std_temporal_days_in_month(1972, month)).contains(&day)
        }
        4 if digits(text) => {
            let (month, day) = (number(&text[..2]), number(&text[2..]));
            (1..=12).contains(&month) && (1..=std_temporal_days_in_month(1972, month)).contains(&day)
        }
        _ => false,
    }
}

fn std_temporal_parse(kind: StdTemporalKind, text: &str) -> Result<([i64; 10], String), Throw> {
    if kind == StdTemporalKind::Duration {
        // Unlike every other kind (validated below, after the match), a
        // Duration string returns early here and must run the same sign/range
        // check itself -- the property-bag path (std_temporal_bag) already
        // validates unconditionally, so a string-form "P4294967296Y" or a
        // ISO duration string mixing a positive and negative field skipped it
        // entirely before this.
        let wide = std_temporal_parse_duration_wide(text)?;
        std_temporal_validate_duration(&wide)?;
        return Ok((std_temporal_duration_fit(&wide), "iso8601".to_string()));
    }
    let invalid = || Throw::range_error("invalid Temporal string");
    let (text, calendar_name, _zone) = std_temporal_parse_annotations(text, kind != StdTemporalKind::PlainTime)?;
    // A non-ISO year-month string needs a complete reference ISO date;
    // the short YYYY-MM spelling cannot identify its calendar month.
    let year_end = if text.starts_with(['+', '-']) { 7 } else { 4 };
    let complete_yearmonth_date = text.as_bytes().get(year_end + 3) == Some(&b'-');
    if matches!(kind, StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay)
        && ((kind == StdTemporalKind::PlainMonthDay && (text.starts_with("--") || text.len() == 5 || (text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit())))) || (kind == StdTemporalKind::PlainYearMonth && !complete_yearmonth_date))
        && calendar_name.is_some_and(|name| !name.eq_ignore_ascii_case("iso8601")) {
        return Err(invalid());
    }
    // A calendar annotation on a PlainTime string is grammar-checked above
    // but never canonicalized/kept (PlainTime has no calendar slot); every
    // other kind's calendar id defaults to iso8601 with no annotation, or
    // canonicalizes the (already-validated-known) annotated name.
    let calendar_id = if kind == StdTemporalKind::PlainTime { "iso8601".to_string() }
        else { calendar_name.and_then(std_calendar_canonicalize).unwrap_or("iso8601").to_string() };
    if text.ends_with('Z') || text.ends_with('z') { return Err(invalid()); }
    let mut values = [0; 10];
    match kind {
        StdTemporalKind::PlainTime => {
            let separated = text.split_once('T').or_else(|| text.split_once('t')).or_else(|| text.split_once(' '));
            if let Some((date, _)) = separated {
                // A space is not a substitute for the T designator on a
                // bare time: it needs a date before it.
                if date.is_empty() && !text.starts_with(['T', 't']) { return Err(invalid()); }
            } else if std_temporal_time_string_ambiguous(text) {
                // "2021-12", "1214", "12-14" also parse as a year-month or
                // month-day, so a time must carry the T prefix.
                return Err(invalid());
            }
            let time = if let Some((date, time)) = separated {
                // The date prefix is discarded (PlainTime keeps only the
                // time-of-day) but is still validated in full when present
                // -- a syntactically-invalid or out-of-range discarded date
                // (an extended year of -000000, an out-of-range month/day, a
                // non-ASCII minus sign, ...) must still throw. Confirmed by
                // withPlainTime's own year-zero.js and
                // argument-string-minus-sign.js. An explicit "T" designator
                // with NO date at all (just "T15:23:30") splits to an empty
                // date prefix, which is not itself a validation failure --
                // confirmed by PlainTime.from's own argument-string.js/
                // argument-string-with-time-designator.js.
                if !date.is_empty() {
                    std_temporal_parse_date(date, true)?;
                }
                time
            } else { text };
            std_temporal_validate_time_offset(time)?;
            std_temporal_parse_time(std_temporal_time_prefix(time), &mut values)?;
        }
        StdTemporalKind::PlainMonthDay if text.starts_with("--") || text.len() == 5
            || (text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit())) => {
            let text = text.strip_prefix("--").unwrap_or(text);
            let (month, day) = match text.split_once('-') {
                Some(pair) => pair,
                None if text.len() == 4 => text.split_at(2),
                None => return Err(invalid()),
            };
            if month.len() != 2 || day.len() != 2 || !month.bytes().chain(day.bytes()).all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
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
            if kind == StdTemporalKind::PlainYearMonth && calendar_id == "iso8601" { values[2] = 1; }
            if kind == StdTemporalKind::PlainMonthDay {
                if calendar_id == "iso8601" { values[0] = 0; values[9] = 1972; }
                else {
                    // The ISO date names a calendar month/day; the stored
                    // reference date is re-derived from it.
                    let mut probe = [0i64; 10];
                    probe[0] = year; probe[1] = month; probe[2] = day;
                    std_temporal_validate(StdTemporalKind::PlainDate, &probe)?;
                    let (_, _, native_day, ..) = std_calendar_fields(&calendar_id, year, month, day);
                    let code = std_calendar_month_code(&calendar_id, year, month, day);
                    let (iso_year, iso_month, iso_day) =
                        std_calendar_monthday_reference(&calendar_id, None, None, Some(&code), native_day, false)?;
                    values = [0; 10];
                    values[1] = iso_month; values[2] = iso_day; values[9] = iso_year;
                }
            }
            if kind == StdTemporalKind::PlainDateTime {
                // A TemporalDateTimeString's time-of-day is optional -- a
                // bare date string ("-271821-04-20") defaults to midnight,
                // it is not required to carry a T/t/space time separator at
                // all. Confirmed by from/argument-string-limits.js's valid
                // boundary dates, which are bare dates with no time part.
                if !tail.is_empty() {
                    let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                    std_temporal_validate_time_offset(time)?;
                    std_temporal_parse_time(std_temporal_time_prefix(time), &mut values)?;
                }
            } else if !tail.is_empty() && !matches!(kind, StdTemporalKind::PlainDate) {
                if !tail.starts_with('T') && !tail.starts_with('t') && !tail.starts_with(' ') { return Err(invalid()); }
                let mut ignored = [0; 10];
                std_temporal_validate_time_offset(&tail[1..])?;
                std_temporal_parse_time(std_temporal_time_prefix(&tail[1..]), &mut ignored)?;
                // A discarded time part is still validated (hour <= 23,
                // minute <= 59, ...) -- std_temporal_parse_time itself only
                // checks digit shape/count, not range, and the range check
                // below this match only ever sees `values`, never `ignored`.
                // Confirmed by PlainDate's own since/until
                // `argument-string-invalid.js`, where a discarded
                // "T25:00:00" must still throw.
                std_temporal_validate(StdTemporalKind::PlainTime, &ignored)?;
            } else if !tail.is_empty() {
                let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                let mut ignored = [0; 10];
                std_temporal_validate_time_offset(time)?;
                std_temporal_parse_time(std_temporal_time_prefix(time), &mut ignored)?;
                std_temporal_validate(StdTemporalKind::PlainTime, &ignored)?;
            }
        }
    }
    if kind == StdTemporalKind::PlainYearMonth && calendar_id != "iso8601" {
        values = std_temporal_yearmonth_anchor(&calendar_id, &values)?;
    }
    std_temporal_validate(kind, &values)?;
    Ok((values, calendar_id))
}

fn std_temporal_from_fields(i: &mut Interpreter, kind: StdTemporalKind, value: JsValue, reject: bool) -> Result<([i64; 10], String), Throw> {
    if kind == StdTemporalKind::Duration {
        let wide = std_temporal_to_duration_wide(i, value)?;
        return Ok((std_temporal_duration_fit(&wide), "iso8601".to_string()));
    }
    if let Ok(data) = std_temporal_record(&value, Some(kind)) { return Ok((data.fields, data.calendar)); }
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
                StdTemporalKind::PlainYearMonth => fields = std_temporal_yearmonth_anchor(&data.calendar, &fields)?,
                StdTemporalKind::PlainMonthDay => fields = std_temporal_monthday_of_date(&data.calendar, &fields, reject)?,
                _ => {}
            }
            return Ok((fields, data.calendar));
        }
    }
    if let Ok(data) = std_epoch_record(&value, Some(StdEpochKind::ZonedDateTime)) {
        if matches!(kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainTime | StdTemporalKind::PlainDateTime) {
            let mut fields = std_epoch_local_fields(&data)?;
            // std_epoch_local_fields always returns the full local
            // date+time; PlainDate/PlainTime keep only their own half,
            // matching the zeroing convention every other conversion path
            // above uses (a bare PlainTime's own fields[0..3] are always
            // 0) -- confirmed by PlainTime's own
            // argument-zoneddatetime-negative-epochnanoseconds.js/
            // argument-zoneddatetime-balance-negative-time-units.js, where
            // compare()/equals() against a ZonedDateTime instance compared
            // the receiver's zeroed date fields against the ZDT's real
            // local date and never matched.
            match kind {
                StdTemporalKind::PlainDate => fields[3..9].fill(0),
                StdTemporalKind::PlainTime => fields[0..3].fill(0),
                _ => {}
            }
            // ZonedDateTime carries its own calendar (JSS-calendars item 2)
            // forward into the converted Plain* result, matching
            // ToTemporalDate/ToTemporalDateTime/ToTemporalTime.
            return Ok((fields, data.calendar.clone()));
        }
    }
    match &value {
        JsValue::Str(text) => std_temporal_parse(kind, &text.to_utf8_lossy()),
        JsValue::Object(_) => {
            let (mut fields, month, code, calendar) = std_temporal_bag(i, kind, &value)?;
            std_temporal_bag_resolve_calendar(kind, &calendar, &mut fields, month, code, reject)?;
            std_temporal_regulate(kind, &mut fields, reject);
            std_temporal_validate(kind, &fields)?;
            Ok((fields, calendar))
        }
        _ => Err(Throw::type_error("invalid Temporal input")),
    }
}

fn std_temporal_from(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let item = std_reflect_arg(&args, 0);
    if kind == StdTemporalKind::Duration {
        let wide = std_temporal_to_duration_wide(i, item)?;
        return std_temporal_make_duration(i, wide);
    }
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
    let (fields, calendar) = if is_bag {
        let (mut fields, month, code, calendar) = std_temporal_bag(i, kind, &item)?;
        let reject = std_temporal_overflow(i, std_reflect_arg(&args, 1))?;
        // monthCode suitability (CalendarResolveFields, folded into
        // CalendarYearMonthFromFields/CalendarDateFromFields) is checked
        // AFTER GetTemporalOverflowOption reads/coerces `overflow` here --
        // confirmed by options-read-before-algorithmic-validation.js.
        std_temporal_bag_resolve_calendar(kind, &calendar, &mut fields, month, code, reject)?;
        std_temporal_regulate(kind, &mut fields, reject);
        std_temporal_validate(kind, &fields)?;
        (fields, calendar)
    } else if matches!(item, JsValue::Str(_)) {
        let result = std_temporal_from_fields(i, kind, item, false)?;
        if kind != StdTemporalKind::Duration { std_temporal_overflow(i, std_reflect_arg(&args, 1))?; }
        result
    } else if std_temporal_record(&item, None).is_ok()
        || std_epoch_record(&item, Some(StdEpochKind::ZonedDateTime)).is_ok() {
        // A recognized Temporal-like value (an existing record of some
        // kind, or a ZonedDateTime instance): GetTemporalOverflowOption is
        // validated (and its VALUE discarded, since none of
        // std_temporal_from_fields' non-bag branches consume `reject`)
        // before the already-fully-specified conversion runs.
        let reject = if kind == StdTemporalKind::Duration { false }
            else { std_temporal_overflow(i, std_reflect_arg(&args, 1))? };
        std_temporal_from_fields(i, kind, item, reject)?
    } else {
        // Neither a bag, a string, nor any recognized Temporal-like value
        // (e.g. a bare Number or Boolean) -- this throws TypeError inside
        // std_temporal_from_fields without ever touching `options`.
        // Confirmed by observable-get-overflow-argument-primitive.js's
        // `from(7, options)` case, which asserts zero property access on a
        // failing non-bag, non-string, non-Temporal argument.
        std_temporal_from_fields(i, kind, item, false)?
    };
    std_temporal_make_cal(i, kind, fields, calendar)
}

fn std_temporal_compare(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let left = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 0), false)?.0;
    let right = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 1), false)?.0;
    let ordering = if kind == StdTemporalKind::Duration {
        // GetOptionsObject + ToRelativeTemporalObject (spec steps 3-4) run
        // unconditionally -- a wrong options type or a malformed relativeTo
        // string throws even when the two durations turn out identical and
        // the parsed relativeTo would otherwise go unused.
        let options = std_reflect_arg(&args, 2);
        let relative = std_temporal_option_value(i, &options, "relativeTo")?;
        let relative_full = std_temporal_resolve_relative_full(i, &relative)?;
        let zoned = relative_full.as_ref().and_then(|r| r.zoned.clone());
        let relative_base = relative_full.map(|r| r.fields);
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
                _ if zoned.is_some() && (calendar || left[3] != 0 || right[3] != 0) => {
                    zr_compare(zoned.as_ref().unwrap(), &left, &right)?
                }
                (true, None) => return Err(Throw::range_error("calendar units require relativeTo")),
                (true, Some(base)) => {
                    let total = |duration: &[i64; 10]| -> Result<i128, Throw> {
                        let target = std_temporal_add_calendar(&base, duration, 1, false)?;
                        let days = std_temporal_epoch_day(target[0], target[1], target[2])
                            - std_temporal_epoch_day(base[0], base[1], base[2]);
                        // Add24HourDaysToTimeDuration: the whole day span plus
                        // the clock part must stay a valid time duration.
                        let span = days as i128 * STD_TEMPORAL_NS_DAY
                            + std_temporal_duration_time_ns(duration)
                            - (duration[2] as i128 * 7 + duration[3] as i128) * STD_TEMPORAL_NS_DAY;
                        if span.unsigned_abs() > STD_TEMPORAL_DURATION_MAX_NS as u128 {
                            return Err(Throw::range_error("duration is out of range for relativeTo"));
                        }
                        Ok(span)
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
    let (other, other_calendar) = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0), false)?;
    Ok(JsValue::Bool(data.fields == other && data.calendar == other_calendar))
}

fn std_temporal_duration_abs(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut wide = std_temporal_record(&this, Some(StdTemporalKind::Duration))?.wide;
    for field in &mut wide { *field = field.abs(); }
    std_temporal_make_duration(i, wide)
}

fn std_temporal_duration_negated(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut wide = std_temporal_record(&this, Some(StdTemporalKind::Duration))?.wide;
    for field in &mut wide { *field = -*field; }
    std_temporal_make_duration(i, wide)
}

fn std_temporal_with_calendar(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    if !matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime |
        StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay) {
        return Err(Throw::type_error("incompatible Temporal receiver"));
    }
    if std_reflect_arg(&args, 0).is_undefined() { return Err(Throw::type_error("withCalendar requires a calendar")); }
    let calendar = std_temporal_resolve_calendar_id(i, &std_reflect_arg(&args, 0))?;
    std_temporal_make_cal(i, data.kind, data.fields, calendar)
}

fn std_temporal_date_to_datetime(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?;
    let mut fields = data.fields;
    let time = std_reflect_arg(&args, 0);
    if !time.is_undefined() {
        let (parts, _) = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time, false)?;
        fields[3..9].copy_from_slice(&parts[3..9]);
    }
    std_temporal_make_cal(i, StdTemporalKind::PlainDateTime, fields, data.calendar)
}

fn std_temporal_date_to_yearmonth(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?;
    let fields = std_temporal_yearmonth_anchor(&data.calendar, &data.fields)?;
    std_temporal_make_cal(i, StdTemporalKind::PlainYearMonth, fields, data.calendar)
}

// The stored reference date for the month-day of an ISO date in a calendar.
fn std_temporal_monthday_of_date(calendar: &str, iso: &[i64; 10], reject: bool) -> Result<[i64; 10], Throw> {
    let mut fields = *iso;
    if calendar == "iso8601" {
        fields[9] = 1972;
        fields[0] = 0;
        return Ok(fields);
    }
    let (_, _, native_day, ..) = std_calendar_fields(calendar, iso[0], iso[1], iso[2]);
    let code = std_calendar_month_code(calendar, iso[0], iso[1], iso[2]);
    let (iso_year, iso_month, iso_day) = std_calendar_monthday_reference(calendar, None, None, Some(&code), native_day, reject)?;
    Ok([0, iso_month, iso_day, 0, 0, 0, 0, 0, 0, iso_year])
}

fn std_temporal_date_to_monthday(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?;
    let fields = std_temporal_monthday_of_date(&data.calendar, &data.fields, false)?;
    std_temporal_make_cal(i, StdTemporalKind::PlainMonthDay, fields, data.calendar)
}

fn std_temporal_datetime_to_date(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?;
    let mut fields = data.fields;
    fields[3..9].fill(0);
    std_temporal_make_cal(i, StdTemporalKind::PlainDate, fields, data.calendar)
}

fn std_temporal_datetime_to_time(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDateTime))?.fields;
    fields[0..3].fill(0);
    std_temporal_make(i, StdTemporalKind::PlainTime, fields)
}

fn std_temporal_yearmonth_to_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainYearMonth))?;
    let mut fields = data.fields;
    let bag = std_reflect_arg(&args, 0);
    if !bag.is_object() { return Err(Throw::type_error("day property bag required")); }
    let day = std_temporal_property(i, &bag, "day")?
        .ok_or_else(|| Throw::type_error("day is required"))?;
    // toPlainDate (spec): CalendarDateFromFields is always called with
    // ~constrain~ overflow here -- there is no overflow option to read --
    // so a day past the target month's end (e.g. day 29 in a common-year
    // February, or day 31 in a 30-day month) is clamped, never rejected.
    // Confirmed by default-overflow-behaviour.js. For a non-iso8601
    // month-structure calendar, `day` is calendar-native (the object's own
    // fields[0]/fields[1] are ISO, so they are first converted to that
    // calendar's own (year, month) before the day bound is computed).
    if std_calendar_is_month_structure(&data.calendar) {
        let (cal_year, cal_month, ..) = std_calendar_fields(&data.calendar, fields[0], fields[1], fields[2]);
        let clamped_day = day.clamp(1, std_calendar_month_len(&data.calendar, cal_year, cal_month));
        let (iso_year, iso_month, iso_day) = std_calendar_to_iso(&data.calendar, cal_year, cal_month, clamped_day);
        fields[0] = iso_year; fields[1] = iso_month; fields[2] = iso_day;
        return std_temporal_make_cal(i, StdTemporalKind::PlainDate, fields, data.calendar);
    }
    fields[2] = day.min(std_temporal_days_in_month(fields[0], fields[1])).max(1);
    std_temporal_make_cal(i, StdTemporalKind::PlainDate, fields, data.calendar)
}

fn std_temporal_monthday_to_date(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, Some(StdTemporalKind::PlainMonthDay))?;
    let mut fields = data.fields;
    let bag = std_reflect_arg(&args, 0);
    if !bag.is_object() { return Err(Throw::type_error("year property bag required")); }
    if data.calendar == "iso8601" {
        fields[0] = std_temporal_property(i, &bag, "year")?
            .ok_or_else(|| Throw::type_error("year is required"))?;
        fields[9] = 0;
        fields[2] = fields[2].min(std_temporal_days_in_month(fields[0], fields[1])).max(1);
        std_temporal_validate(StdTemporalKind::PlainDate, &fields)?;
        return std_temporal_make_cal(i, StdTemporalKind::PlainDate, fields, data.calendar);
    }
    // CalendarMergeFields(monthCode, day of the month-day; year, era, eraYear
    // of the argument) then CalendarDateFromFields with constrain.
    let uses_era = std_calendar_uses_era(&data.calendar);
    let mut era_text = None;
    let mut era_year = None;
    if uses_era {
        let era = i.get_prop(&bag, "era")?;
        if !era.is_undefined() { era_text = Some(i.to_string(&era)?.to_ascii_lowercase()); }
        let value = i.get_prop(&bag, "eraYear")?;
        if !value.is_undefined() { era_year = Some(std_temporal_number(i, value, None)?); }
    }
    let year_value = i.get_prop(&bag, "year")?;
    let year = if year_value.is_undefined() { None } else { Some(std_temporal_number(i, year_value, None)?) };
    let native_year = std_temporal_resolve_era_year_checked(&data.calendar, &era_text, era_year, year)?
        .ok_or_else(|| Throw::type_error("year is required"))?;
    let reference = (data.fields[9], data.fields[1], data.fields[2]);
    let (_, _, native_day, ..) = std_calendar_fields(&data.calendar, reference.0, reference.1, reference.2);
    let code = std_calendar_month_code(&data.calendar, reference.0, reference.1, reference.2);
    let mut date = [0i64; 10];
    date[0] = native_year;
    date[2] = native_day;
    std_temporal_bag_resolve_calendar(StdTemporalKind::PlainDate, &data.calendar, &mut date, None, Some(code), false)?;
    std_temporal_regulate(StdTemporalKind::PlainDate, &mut date, false);
    std_temporal_validate(StdTemporalKind::PlainDate, &date)?;
    std_temporal_make_cal(i, StdTemporalKind::PlainDate, date, data.calendar)
}

fn std_temporal_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let field = name.strip_prefix("get ").unwrap_or("");
    if data.kind == StdTemporalKind::Duration {
        let w = data.wide;
        return Ok(match field {
            "years" => JsValue::num(w[0] as f64), "months" => JsValue::num(w[1] as f64),
            "weeks" => JsValue::num(w[2] as f64), "days" => JsValue::num(w[3] as f64),
            "hours" => JsValue::num(w[4] as f64), "minutes" => JsValue::num(w[5] as f64),
            "seconds" => JsValue::num(w[6] as f64), "milliseconds" => JsValue::num(w[7] as f64),
            "microseconds" => JsValue::num(w[8] as f64), "nanoseconds" => JsValue::num(w[9] as f64),
            "sign" => JsValue::num(w.iter().find(|part| **part != 0).map(|part| part.signum()).unwrap_or(0) as f64),
            "blank" => JsValue::Bool(w.iter().all(|part| *part == 0)),
            _ => JsValue::Undefined,
        });
    }
    let v = data.fields;
    // Non-iso8601 calendars are VIEWS computed on demand from the object's
    // always-ISO internal fields (see builtins_temporal_calendar.rs's file
    // header): a Duration/PlainTime never has one, and PlainMonthDay's own
    // calendar-relevant date is its reference ISO year (fields[9]), not
    // fields[0] (always 0 for PlainMonthDay).
    let calendar_relevant = matches!(data.kind, StdTemporalKind::PlainDate | StdTemporalKind::PlainDateTime
        | StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay);
    let iso_triple = if data.kind == StdTemporalKind::PlainMonthDay { (v[9], v[1], v[2]) } else { (v[0], v[1], v[2]) };
    let calendar_fields = if calendar_relevant && data.calendar != "iso8601" {
        Some(std_calendar_fields(&data.calendar, iso_triple.0, iso_triple.1, iso_triple.2))
    } else { None };
    if calendar_relevant {
        match field {
            "calendarId" => return Ok(JsValue::str(data.calendar.clone())),
            "era" => {
                if data.calendar == "iso8601" { return Ok(JsValue::Undefined); }
                let (era, _) = std_calendar_era(&data.calendar, iso_triple.0, iso_triple.1, iso_triple.2);
                return Ok(era.map(JsValue::str).unwrap_or(JsValue::Undefined));
            }
            "eraYear" => {
                if data.calendar == "iso8601" { return Ok(JsValue::Undefined); }
                let (_, era_year) = std_calendar_era(&data.calendar, iso_triple.0, iso_triple.1, iso_triple.2);
                return Ok(era_year.map(|value| JsValue::num(value as f64)).unwrap_or(JsValue::Undefined));
            }
            "monthCode" => {
                let code = if data.calendar == "iso8601" { format!("M{:02}", v[1]) }
                    else { std_calendar_month_code(&data.calendar, iso_triple.0, iso_triple.1, iso_triple.2) };
                return Ok(JsValue::str(code));
            }
            _ => {}
        }
    }
    let result = match field {
        "era" | "eraYear" => return Ok(JsValue::Undefined),
        "monthCode" => return Ok(JsValue::str(format!("M{:02}", v[1]))),
        "inLeapYear" => return Ok(JsValue::Bool(calendar_fields.map(|f| f.7).unwrap_or_else(|| std_temporal_leap(v[0])))),
        "blank" => return Ok(JsValue::Bool(v.iter().all(|part| *part == 0))),
        "sign" => v.iter().find(|part| **part != 0).map(|part| part.signum()).unwrap_or(0),
        "daysInMonth" => calendar_fields.map(|f| f.3).unwrap_or_else(|| std_temporal_days_in_month(v[0], v[1])),
        "daysInYear" => calendar_fields.map(|f| f.6).unwrap_or_else(|| if std_temporal_leap(v[0]) { 366 } else { 365 }),
        "monthsInYear" => calendar_fields.map(|f| f.4).unwrap_or(12),
        "daysInWeek" => 7,
        "dayOfWeek" => std_temporal_day_of_week(std_temporal_epoch_day(v[0], v[1], v[2])),
        "dayOfYear" => calendar_fields.map(|f| f.5)
            .unwrap_or_else(|| std_temporal_epoch_day(v[0], v[1], v[2]) - std_temporal_epoch_day(v[0], 1, 1) + 1),
        "weekOfYear" | "yearOfWeek" if data.calendar != "iso8601" => return Ok(JsValue::Undefined),
        "weekOfYear" | "yearOfWeek" => {
            let day = std_temporal_epoch_day(v[0], v[1], v[2]);
            let week_year = if day < std_temporal_week_start(v[0]) { v[0] - 1 }
                else if day >= std_temporal_week_start(v[0] + 1) { v[0] + 1 }
                else { v[0] };
            if field == "yearOfWeek" { week_year }
            else { (day - std_temporal_week_start(week_year)) / 7 + 1 }
        }
        "year" | "years" => calendar_fields.map(|f| f.0).unwrap_or(v[0]),
        "month" | "months" => calendar_fields.map(|f| f.1).unwrap_or(v[1]),
        "day" => calendar_fields.map(|f| f.2).unwrap_or(v[2]),
        "weeks" => v[2],
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
    std_temporal_string_precision_ex(i, option, allow_minute, None, None)
}

// `offset_slot`: ZonedDateTime.prototype.toString reads its `offset` option
// between fractionalSecondDigits and roundingMode (alphabetical order).
fn std_temporal_string_precision_ex(i: &mut Interpreter, option: &JsValue, allow_minute: bool,
    mut offset_slot: Option<&mut String>,
    after_reads: Option<&mut dyn FnMut(&mut Interpreter) -> Result<(), Throw>>) -> Result<(Option<usize>, bool, Option<i128>, String), Throw> {
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
    if let Some(slot) = offset_slot.as_deref_mut() {
        *slot = std_temporal_option_string(i, option, "offset")?.unwrap_or_else(|| "auto".to_string());
    }
    let mode = std_temporal_option_string(i, option, "roundingMode")?
        .unwrap_or_else(|| "trunc".to_string());
    let unit = std_temporal_option_string(i, option, "smallestUnit")?;
    // Every option is read before any algorithmic validation runs.
    if let Some(reads) = after_reads { reads(i)?; }
    if !std_temporal_round_mode_valid(&mode) {
        return Err(Throw::range_error("invalid roundingMode"));
    }
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
    // Duration.prototype.toString/PlainTime.prototype.toString have no
    // calendarName option (neither Durations nor a bare time-of-day carry a
    // calendar) -- reading it at all is observable (and wrong) via a
    // Proxy-like options bag, confirmed by
    // Duration/prototype/toString/order-of-operations.js and PlainTime's own
    // options-read-before-algorithmic-validation.js, whose expected
    // property-access logs never include "get options.calendarName".
    let calendar_name = if matches!(data.kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) { "auto".to_string() }
        else if option.is_undefined() { "auto".to_string() }
        else {
            let value = i.get_prop(&option, "calendarName")?;
            if value.is_undefined() { "auto".to_string() } else { i.to_string(&value)? }
        };
    if !matches!(calendar_name.as_str(), "auto" | "always" | "never" | "critical") {
        return Err(Throw::range_error("invalid calendarName"));
    }
    // PlainDate/PlainYearMonth/PlainMonthDay have no time-of-day at all --
    // TemporalDateToString/TemporalYearMonthToString/
    // TemporalMonthDayToString never call GetTemporalFractionalSecondDigits
    // Option/GetRoundingModeOption/GetTemporalUnitValuedOption, only
    // GetTemporalShowCalendarNameOption above. Reading those three options
    // regardless (even to find them absent) is observable via a Proxy
    // options bag and wrong -- confirmed by PlainYearMonth's own
    // toString/order-of-operations.js.
    let (digits, minute_only, step, mode) = if matches!(data.kind,
        StdTemporalKind::PlainDate | StdTemporalKind::PlainYearMonth | StdTemporalKind::PlainMonthDay) {
        (None, false, None, "trunc".to_string())
    } else {
        std_temporal_string_precision(i, &option, data.kind != StdTemporalKind::Duration)?
    };
    if let Some(step) = step {
        if data.kind == StdTemporalKind::PlainTime {
            let rounded = std_temporal_round_integer(std_temporal_time_ns(&v), step, &mode)?;
            std_temporal_set_time_ns(&mut v, rounded);
        } else if data.kind == StdTemporalKind::PlainDateTime {
            let rounded = std_temporal_round_as_if_positive(std_temporal_epoch_ns(&v), step, &mode)?;
            std_temporal_set_epoch_ns(&mut v, rounded)?;
            std_temporal_validate(StdTemporalKind::PlainDateTime, &v)?;
        }
    }
    // ShowCalendar (spec): "auto" shows the calendar annotation whenever the
    // calendar is anything other than iso8601 -- this engine was iso8601-only
    // when this always/critical-only condition was written, so "auto" never
    // needed to show anything on its own; now that non-iso8601 calendars
    // exist, "auto" must show them too, but "never" still never does.
    let show_calendar = matches!(calendar_name.as_str(), "always" | "critical")
        || (calendar_name == "auto" && data.calendar != "iso8601");
    let mut text = match data.kind {
        StdTemporalKind::PlainDate => std_temporal_date_text(v[0], v[1], v[2]),
        StdTemporalKind::PlainTime => std_temporal_time_text_options(&v, digits, minute_only),
        StdTemporalKind::PlainDateTime => format!("{}T{}", std_temporal_date_text(v[0], v[1], v[2]),
            std_temporal_time_text_options(&v, digits, minute_only)),
        StdTemporalKind::PlainYearMonth => if show_calendar || data.calendar != "iso8601" {
            std_temporal_date_text(v[0], v[1], v[2])
        } else { format!("{}-{:02}", std_temporal_year_text(v[0]), v[1]) },
        StdTemporalKind::PlainMonthDay => if show_calendar || data.calendar != "iso8601" {
            std_temporal_date_text(v[9], v[1], v[2])
        } else { format!("{:02}-{:02}", v[1], v[2]) },
        StdTemporalKind::Duration => std_temporal_duration_to_string(&data.wide, digits, step, &mode)?,
    };
    if show_calendar && !matches!(data.kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        if calendar_name == "critical" { text.push_str(&format!("[!u-ca={}]", data.calendar)); }
        else { text.push_str(&format!("[u-ca={}]", data.calendar)); }
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
    fields: &[i128; 10],
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
        let total_ns: i128 = (balance_rank..10).map(|rank| f[rank] * unit_ns(rank)).sum();
        let rounded = std_temporal_round_integer(total_ns, step, mode)?;
        let sign: i128 = if rounded < 0 { -1 } else { 1 };
        let mut remainder = rounded.unsigned_abs() as i128;
        f[balance_rank] = sign * (remainder / unit_ns(balance_rank));
        remainder %= unit_ns(balance_rank);
        for rank in (balance_rank + 1)..=9 {
            f[rank] = sign * (remainder / unit_ns(rank));
            remainder %= unit_ns(rank);
        }
        std_temporal_validate_duration(&f)?;
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

fn std_temporal_to_locale_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    // Duration.prototype.toLocaleString formats through Intl.DurationFormat.
    if data.kind == StdTemporalKind::Duration {
        let formatter = intl_construct_service(i, "DurationFormat", std_reflect_arg(&args, 0), std_reflect_arg(&args, 1))?;
        let format = i.get_prop(&formatter, "format")?;
        return i.call_value(&format, formatter, vec![this]);
    }
    intl_temporal_to_locale_string(i, &this, &std_reflect_arg(&args, 0), &std_reflect_arg(&args, 1))
}

fn std_temporal_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_temporal_record(&this, None)?;
    Err(Throw::type_error("Temporal values cannot be converted to primitives"))
}
