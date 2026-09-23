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
                    getter: callable_function(&getter).map(Box::new), setter: None };
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
    let minutes = if zone.is_undefined() { 0 }
        else { std_temporal_fixed_offset(&i.to_string(&zone)?)? };
    let mut fields = std_temporal_now_fields();
    let shifted = std_temporal_epoch_ns(&fields) + minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND;
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
    let name = i.to_string(value)?;
    if name.eq_ignore_ascii_case("iso8601") { Ok(()) }
    else { Err(Throw::range_error("only iso8601 calendar is supported")) }
}

fn std_temporal_week_start(year: i64) -> i64 {
    let jan4 = std_temporal_epoch_day(year, 1, 4);
    jan4 - std_temporal_day_of_week(jan4) + 1
}

fn std_temporal_validate(kind: StdTemporalKind, values: &[i64; 10]) -> Result<(), Throw> {
    if kind == StdTemporalKind::Duration {
        let sign = values.iter().find(|v| **v != 0).map(|v| v.signum()).unwrap_or(0);
        if values.iter().any(|v| *v != 0 && v.signum() != sign)
            || values[..4].iter().any(|v| v.abs() > 4_294_967_295)
            || values[4..].iter().any(|v| v.abs() > 9_007_199_254_740_991) {
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

fn std_temporal_bag(i: &mut Interpreter, kind: StdTemporalKind, value: &JsValue) -> Result<[i64; 10], Throw> {
    let mut fields = [0; 10];
    if !matches!(kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        let calendar = i.get_prop(value, "calendar")?;
        std_temporal_iso_calendar(i, &calendar)?;
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
    std_temporal_validate(kind, &fields)?;
    Ok(fields)
}

fn std_temporal_parse_date(text: &str, require_day: bool) -> Result<(i64, i64, i64, &str), Throw> {
    let year_end = if text.starts_with('+') || text.starts_with('-') { 7 } else { 4 };
    let invalid = || Throw::range_error("invalid ISO date string");
    if text.len() < year_end + 3 || text.as_bytes().get(year_end) != Some(&b'-') {
        return Err(invalid());
    }
    if text.starts_with("-000000") { return Err(invalid()); }
    let year = text.get(..year_end).and_then(|part| part.parse::<i64>().ok()).ok_or_else(invalid)?;
    let month = text.get(year_end + 1..year_end + 3).and_then(|part| part.parse::<i64>().ok()).ok_or_else(invalid)?;
    let mut tail = text.get(year_end + 3..).ok_or_else(invalid)?;
    let day = if let Some(rest) = tail.strip_prefix('-') {
        let value = rest.get(..2).and_then(|part| part.parse::<i64>().ok()).ok_or_else(invalid)?;
        tail = rest.get(2..).ok_or_else(invalid)?;
        value
    } else if require_day { return Err(invalid()); } else { 1 };
    Ok((year, month, day, tail))
}

fn std_temporal_parse_time(text: &str, fields: &mut [i64; 10]) -> Result<(), Throw> {
    let invalid = || Throw::range_error("invalid ISO time string");
    let text = text.strip_prefix('T').unwrap_or(text);
    let (hour, rest) = text.split_once(':').ok_or_else(invalid)?;
    fields[3] = hour.parse().map_err(|_| invalid())?;
    let minute = rest.get(..2).ok_or_else(invalid)?;
    fields[4] = minute.parse().map_err(|_| invalid())?;
    let rest = rest.get(2..).ok_or_else(invalid)?;
    if let Some(rest) = rest.strip_prefix(':') {
        let seconds = rest.get(..2).ok_or_else(invalid)?;
        let second = seconds.parse::<i64>().map_err(|_| invalid())?;
        if second > 60 { return Err(invalid()); }
        fields[5] = second.min(59);
        let tail = rest.get(2..).ok_or_else(invalid)?;
        if let Some(fraction) = tail.strip_prefix('.').or_else(|| tail.strip_prefix(',')) {
            if fraction.is_empty() || fraction.len() > 9 || !fraction.bytes().all(|c| c.is_ascii_digit()) {
                return Err(invalid());
            }
            let mut digits = fraction.to_string();
            while digits.len() < 9 { digits.push('0'); }
            let nanoseconds = digits.parse::<i64>().map_err(|_| invalid())?;
            fields[6] = nanoseconds / 1_000_000;
            fields[7] = (nanoseconds / 1_000) % 1_000;
            fields[8] = nanoseconds % 1_000;
        } else if !tail.is_empty() { return Err(invalid()); }
    } else if !rest.is_empty() { return Err(invalid()); }
    Ok(())
}

fn std_temporal_parse_annotations(text: &str) -> Result<(&str, bool), Throw> {
    let invalid = || Throw::range_error("invalid Temporal annotation");
    let Some(first) = text.find('[') else { return Ok((text, false)); };
    let body = &text[..first];
    let mut tail = &text[first..];
    let mut calendar = false;
    let mut time_zone = false;
    while !tail.is_empty() {
        let rest = tail.strip_prefix('[').ok_or_else(invalid)?;
        let end = rest.find(']').ok_or_else(invalid)?;
        let value = &rest[..end];
        tail = &rest[end + 1..];
        let annotation = value.strip_prefix('!').unwrap_or(value);
        if let Some(name) = annotation.strip_prefix("u-ca=") {
            if !name.eq_ignore_ascii_case("iso8601") || calendar {
                return Err(invalid());
            }
            calendar = true;
        } else if annotation.contains('=') {
            if value.starts_with('!') || annotation.bytes().any(|c| c.is_ascii_uppercase()) {
                return Err(invalid());
            }
        } else if time_zone || value.starts_with('!') || value.is_empty() {
            return Err(invalid());
        } else { time_zone = true; }
    }
    Ok((body, calendar))
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
    if kind == StdTemporalKind::Duration { return std_temporal_parse_duration(text); }
    let invalid = || Throw::range_error("invalid Temporal string");
    let (text, _calendar) = std_temporal_parse_annotations(text)?;
    let mut values = [0; 10];
    match kind {
        StdTemporalKind::PlainTime => {
            let time = if let Some((_, time)) = text.split_once('T').or_else(|| text.split_once('t')).or_else(|| text.split_once(' ')) {
                time
            } else { text };
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
            values[0] = year; values[1] = month; values[2] = day;
            if kind == StdTemporalKind::PlainMonthDay { values[0] = 0; values[9] = 1972; }
            if kind == StdTemporalKind::PlainDateTime {
                let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                std_temporal_parse_time(std_temporal_time_prefix(time), &mut values)?;
            } else if !tail.is_empty() && !matches!(kind, StdTemporalKind::PlainDate) {
                if !tail.starts_with('T') && !tail.starts_with('t') && !tail.starts_with(' ') { return Err(invalid()); }
                let mut ignored = [0; 10];
                std_temporal_parse_time(std_temporal_time_prefix(&tail[1..]), &mut ignored)?;
            } else if !tail.is_empty() {
                let time = tail.strip_prefix('T').or_else(|| tail.strip_prefix('t')).or_else(|| tail.strip_prefix(' ')).ok_or_else(invalid)?;
                let mut ignored = [0; 10];
                std_temporal_parse_time(std_temporal_time_prefix(time), &mut ignored)?;
            }
        }
    }
    std_temporal_validate(kind, &values)?;
    Ok(values)
}

fn std_temporal_from_fields(i: &mut Interpreter, kind: StdTemporalKind, value: JsValue) -> Result<[i64; 10], Throw> {
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
        JsValue::Object(_) => std_temporal_bag(i, kind, &value),
        _ => Err(Throw::type_error("invalid Temporal input")),
    }
}

fn std_temporal_from(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let fields = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 0))?;
    std_temporal_make(i, kind, fields)
}

fn std_temporal_compare(i: &mut Interpreter, args: Vec<JsValue>, kind: StdTemporalKind) -> Result<JsValue, Throw> {
    let left = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 0))?;
    let right = std_temporal_from_fields(i, kind, std_reflect_arg(&args, 1))?;
    let ordering = if kind == StdTemporalKind::Duration {
        let options = std_reflect_arg(&args, 2);
        let relative = std_temporal_option_value(i, &options, "relativeTo")?;
        let calendar = left[0..2].iter().chain(right[0..2].iter()).any(|v| *v != 0);
        if calendar && relative.is_undefined() {
            return Err(Throw::range_error("calendar units require relativeTo"));
        }
        if calendar {
            let base = std_temporal_from_fields(i, StdTemporalKind::PlainDate, relative)?;
            let total = |duration: &[i64; 10]| -> Result<i128, Throw> {
                let target = std_temporal_add_calendar(&base, duration, 1, false)?;
                let days = std_temporal_epoch_day(target[0], target[1], target[2])
                    - std_temporal_epoch_day(base[0], base[1], base[2]);
                Ok(days as i128 * STD_TEMPORAL_NS_DAY
                    + std_temporal_duration_time_ns(duration)
                    - (duration[2] as i128 * 7 + duration[3] as i128) * STD_TEMPORAL_NS_DAY)
            };
            total(&left)?.cmp(&total(&right)?)
        } else { std_temporal_duration_time_ns(&left).cmp(&std_temporal_duration_time_ns(&right)) }
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
    let other = std_temporal_from_fields(i, data.kind, std_reflect_arg(&args, 0))?;
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
    if i.to_string(&std_reflect_arg(&args, 0))? != "iso8601" {
        return Err(Throw::range_error("only iso8601 calendar is supported"));
    }
    std_temporal_make(i, data.kind, data.fields)
}

fn std_temporal_date_to_datetime(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut fields = std_temporal_record(&this, Some(StdTemporalKind::PlainDate))?.fields;
    let time = std_reflect_arg(&args, 0);
    if !time.is_undefined() {
        let parts = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time)?;
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

fn std_temporal_string_precision(i: &mut Interpreter, option: &JsValue) -> Result<(Option<usize>, bool, Option<i128>, String), Throw> {
    if !option.is_object() { return Ok((None, false, None, "trunc".to_string())); }
    let digits_value = i.get_prop(option, "fractionalSecondDigits")?;
    let mut digits = if digits_value.is_undefined() { None }
        else if let JsValue::Str(text) = &digits_value {
            if text.to_utf8_lossy() == "auto" { None }
            else { return Err(Throw::range_error("invalid fractionalSecondDigits")); }
        } else {
            let number = i.to_number(&digits_value)?;
            if !number.is_finite() || number.fract() != 0.0 || !(0.0..=9.0).contains(&number) {
                return Err(Throw::range_error("invalid fractionalSecondDigits"));
            }
            Some(number as usize)
        };
    let mode = std_temporal_option_string(i, option, "roundingMode")?
        .unwrap_or_else(|| "trunc".to_string());
    let unit = std_temporal_option_string(i, option, "smallestUnit")?;
    let minute_only = unit.as_deref().is_some_and(|name| name.strip_suffix('s').unwrap_or(name) == "minute");
    let step = if let Some(unit) = unit {
        let unit = unit.strip_suffix('s').unwrap_or(&unit);
        digits = match unit {
            "minute" => Some(0), "second" => Some(0), "millisecond" => Some(3),
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
    let calendar_name = if option.is_undefined() { "auto".to_string() }
        else if option.is_object() {
            let value = i.get_prop(&option, "calendarName")?;
            if value.is_undefined() { "auto".to_string() } else { i.to_string(&value)? }
        } else { return Err(Throw::type_error("Temporal options must be an object")); };
    if !matches!(calendar_name.as_str(), "auto" | "always" | "never" | "critical") {
        return Err(Throw::range_error("invalid calendarName"));
    }
    let (digits, minute_only, step, mode) = std_temporal_string_precision(i, &option)?;
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
        StdTemporalKind::Duration => std_temporal_duration_text(&v),
    };
    if show_calendar && !matches!(data.kind, StdTemporalKind::Duration | StdTemporalKind::PlainTime) {
        if calendar_name == "critical" { text.push_str("[!u-ca=iso8601]"); }
        else { text.push_str("[u-ca=iso8601]"); }
    }
    Ok(JsValue::str(text))
}

fn std_temporal_duration_text(fields: &[i64; 10]) -> String {
    let negative = fields.iter().any(|field| *field < 0);
    let mut text = if negative { String::from("-P") } else { String::from("P") };
    for (index, suffix) in [(0, "Y"), (1, "M"), (2, "W"), (3, "D")] {
        if fields[index] != 0 { text.push_str(&format!("{}{suffix}", fields[index].abs())); }
    }
    if fields[4..].iter().any(|field| *field != 0) || text == "P" || text == "-P" {
        text.push('T');
        for (index, suffix) in [(4, "H"), (5, "M")] {
            if fields[index] != 0 { text.push_str(&format!("{}{suffix}", fields[index].abs())); }
        }
        let fraction = fields[7].abs() * 1_000_000 + fields[8].abs() * 1_000 + fields[9].abs();
        if fields[6] != 0 || fraction != 0 || text.ends_with('T') {
            text.push_str(&fields[6].abs().to_string());
            if fraction != 0 { text.push_str(&format!(".{fraction:09}").trim_end_matches('0')); }
            text.push('S');
        }
    }
    text
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
