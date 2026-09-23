// Epoch nanoseconds are retained exactly in i128; the public boundary uses BigInt.

#[derive(Clone, Copy, PartialEq, Eq)]
enum StdEpochKind { Instant, ZonedDateTime }

#[derive(Clone)]
struct StdEpochData {
    owner: Weak<RefCell<JsObject>>,
    kind: StdEpochKind,
    epoch_ns: i128,
    offset_minutes: i64,
}

struct StdEpochPrototypes {
    owner: Weak<RefCell<JsObject>>,
    instant: Obj,
    zoned: Obj,
}

thread_local! {
    static STD_EPOCH_DATA: RefCell<HashMap<usize, StdEpochData>> = RefCell::new(HashMap::new());
    static STD_EPOCH_PROTOS: RefCell<HashMap<usize, StdEpochPrototypes>> = RefCell::new(HashMap::new());
}

impl Interpreter {
    fn install_temporal_epoch(&mut self, namespace: &Obj) {
        let mut prototypes = Vec::new();
        for (kind, name, constructor, length) in [
            (StdEpochKind::Instant, "Instant", std_epoch_instant_constructor as crate::value::NativeFn, 1),
            (StdEpochKind::ZonedDateTime, "ZonedDateTime", std_epoch_zoned_constructor as crate::value::NativeFn, 2),
        ] {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(self.object_proto.clone());
            let prototype = Obj(Rc::new(RefCell::new(prototype)));
            let function = Self::native(name, constructor, &self.function_proto);
            if let JsValue::Object(object) = &function {
                object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                object.borrow_mut().define("length", symbol_data(JsValue::num(length as f64), false, true)).unwrap();
                object.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
                for (method, count, native) in [
                    ("from", 1, std_epoch_from as crate::value::NativeFn),
                    ("compare", 2, std_epoch_compare as crate::value::NativeFn),
                ] {
                    let value = symbol_method(self, method, native);
                    symbol_method_length(&value, count);
                    object.borrow_mut().define(method, symbol_data(value, true, true)).unwrap();
                }
                if kind == StdEpochKind::Instant {
                    for (method, native) in [
                        ("fromEpochMilliseconds", std_epoch_from_milliseconds as crate::value::NativeFn),
                        ("fromEpochNanoseconds", std_epoch_from_nanoseconds as crate::value::NativeFn),
                    ] {
                        let value = symbol_method(self, method, native);
                        symbol_method_length(&value, 1);
                        object.borrow_mut().define(method, symbol_data(value, true, true)).unwrap();
                    }
                }
            }
            prototype.borrow_mut().define("constructor", symbol_data(function.clone(), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                symbol_data(JsValue::str(format!("Temporal.{name}")), false, true)).unwrap();
            let fields: &[&str] = if kind == StdEpochKind::Instant {
                &["epochMilliseconds", "epochNanoseconds"]
            } else {
                &["calendarId", "timeZoneId", "offset", "offsetNanoseconds", "epochMilliseconds",
                  "epochNanoseconds", "year", "month", "monthCode", "day", "dayOfWeek", "dayOfYear",
                  "weekOfYear", "yearOfWeek", "daysInWeek", "daysInMonth", "daysInYear",
                  "monthsInYear", "inLeapYear", "hour", "minute", "second", "millisecond",
                  "microsecond", "nanosecond", "hoursInDay", "era", "eraYear"]
            };
            for field in fields {
                let getter = symbol_method(self, &format!("get {field}"), std_epoch_getter);
                let property = Property { value: JsValue::Undefined, writable: false,
                    enumerable: false, configurable: true,
                    getter: callable_function(&getter).map(Box::new), setter: None };
                prototype.borrow_mut().define(*field, property).unwrap();
            }
            let mut methods: Vec<(&str, u32, crate::value::NativeFn)> = vec![
                ("toString", 0, std_epoch_to_string), ("toJSON", 0, std_epoch_to_json),
                ("toLocaleString", 0, std_epoch_to_string), ("valueOf", 0, std_epoch_value_of),
                ("equals", 1, std_epoch_equals), ("add", 1, std_epoch_add),
                ("subtract", 1, std_epoch_subtract), ("until", 1, std_epoch_until),
                ("since", 1, std_epoch_since), ("round", 1, std_epoch_round),
            ];
            if kind == StdEpochKind::Instant {
                methods.push(("toZonedDateTimeISO", 1, std_epoch_to_zoned));
            } else {
                methods.extend_from_slice(&[
                    ("toInstant", 0, std_epoch_to_instant),
                    ("toPlainDate", 0, std_epoch_to_plain),
                    ("toPlainTime", 0, std_epoch_to_plain),
                    ("toPlainDateTime", 0, std_epoch_to_plain),
                    ("withTimeZone", 1, std_epoch_with_zone),
                    ("withCalendar", 1, std_epoch_with_calendar),
                    ("with", 1, std_epoch_with),
                    ("withPlainTime", 0, std_epoch_with_plain_time),
                    ("getTimeZoneTransition", 1, std_epoch_transition),
                    ("startOfDay", 0, std_epoch_start_of_day),
                ]);
            }
            for (method, length, native) in methods {
                let value = symbol_method(self, method, native);
                symbol_method_length(&value, length);
                prototype.borrow_mut().define(method, symbol_data(value, true, true)).unwrap();
            }
            namespace.borrow_mut().define(name, symbol_data(function, true, true)).unwrap();
            prototypes.push(prototype);
        }
        STD_EPOCH_PROTOS.with(|registry| registry.borrow_mut().insert(std_weak_id(&self.global_obj),
            StdEpochPrototypes { owner: Rc::downgrade(&self.global_obj.0),
                instant: prototypes[0].clone(), zoned: prototypes[1].clone() }));
    }
}

fn std_epoch_make(i: &Interpreter, kind: StdEpochKind, epoch_ns: i128, offset_minutes: i64) -> Result<JsValue, Throw> {
    if !(-8_640_000_000_000_000_000_000i128..=8_640_000_000_000_000_000_000i128).contains(&epoch_ns) {
        return Err(Throw::range_error("instant outside Temporal range"));
    }
    let prototype = STD_EPOCH_PROTOS.with(|registry| registry.borrow().get(&std_weak_id(&i.global_obj))
        .filter(|entry| std_weak_same(&entry.owner, &i.global_obj))
        .map(|entry| if kind == StdEpochKind::Instant { entry.instant.clone() } else { entry.zoned.clone() }))
        .ok_or_else(|| Throw::type_error("Temporal realm is unavailable"))?;
    let mut object = JsObject::ordinary();
    object.proto = Some(prototype);
    let object = Obj(Rc::new(RefCell::new(object)));
    STD_EPOCH_DATA.with(|registry| registry.borrow_mut().insert(std_weak_id(&object),
        StdEpochData { owner: Rc::downgrade(&object.0), kind, epoch_ns, offset_minutes }));
    Ok(JsValue::Object(object))
}

fn std_epoch_record(value: &JsValue, expected: Option<StdEpochKind>) -> Result<StdEpochData, Throw> {
    let JsValue::Object(object) = value else { return Err(Throw::type_error("incompatible Temporal receiver")); };
    STD_EPOCH_DATA.with(|registry| registry.borrow().get(&std_weak_id(object))
        .filter(|data| std_weak_same(&data.owner, object) && expected.is_none_or(|kind| kind == data.kind))
        .cloned().ok_or_else(|| Throw::type_error("incompatible Temporal receiver")))
}

fn std_epoch_zone_text(minutes: i64) -> String {
    if minutes == 0 { return "UTC".to_string(); }
    format!("{}{:02}:{:02}", if minutes < 0 { '-' } else { '+' }, minutes.abs() / 60, minutes.abs() % 60)
}

fn std_epoch_offset_text(minutes: i64) -> String {
    format!("{}{:02}:{:02}", if minutes < 0 { '-' } else { '+' }, minutes.abs() / 60, minutes.abs() % 60)
}

fn std_epoch_parse(text: &str, kind: StdEpochKind) -> Result<(i128, i64), Throw> {
    let invalid = || Throw::range_error("invalid Temporal instant string");
    let (body, bracket) = if let Some(start) = text.find('[') {
        (&text[..start], Some(&text[start..]))
    } else { (text, None) };
    let zone = if kind == StdEpochKind::ZonedDateTime {
        let bracket = bracket.ok_or_else(invalid)?;
        let end = bracket.find(']').ok_or_else(invalid)?;
        std_temporal_fixed_offset(&bracket[1..end])?
    } else { 0 };
    let time_start = body.find('T').or_else(|| body.find('t')).or_else(|| body.find(' ')).ok_or_else(invalid)?;
    let time = &body[time_start + 1..];
    let (clock, offset) = if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        (clock, 0)
    } else {
        let start = time.char_indices().find(|(index, c)| *index > 0 && (*c == '+' || *c == '-'))
            .map(|(index, _)| index).ok_or_else(invalid)?;
        (&time[..start], std_temporal_fixed_offset(&time[start..])?)
    };
    if kind == StdEpochKind::ZonedDateTime && offset != zone { return Err(invalid()); }
    let date = &body[..time_start];
    let (year, month, day, tail) = std_temporal_parse_date(date, true)?;
    if !tail.is_empty() { return Err(invalid()); }
    let mut fields = [0; 10];
    fields[0] = year; fields[1] = month; fields[2] = day;
    std_temporal_parse_time(clock, &mut fields)?;
    std_temporal_validate(StdTemporalKind::PlainDateTime, &fields)?;
    Ok((std_temporal_epoch_ns(&fields) - offset as i128 * 60 * STD_TEMPORAL_NS_SECOND, zone))
}

fn std_epoch_from_value(i: &mut Interpreter, value: JsValue, kind: StdEpochKind) -> Result<(i128, i64), Throw> {
    if let Ok(data) = std_epoch_record(&value, Some(kind)) { return Ok((data.epoch_ns, data.offset_minutes)); }
    match value {
        JsValue::Str(text) => std_epoch_parse(&text.to_utf8_lossy(), kind),
        JsValue::Object(_) if kind == StdEpochKind::ZonedDateTime => {
            let zone_value = i.get_prop(&value, "timeZone")?;
            let zone = std_temporal_fixed_offset(&i.to_string(&zone_value)?)?;
            let fields = std_temporal_bag(i, StdTemporalKind::PlainDateTime, &value)?;
            Ok((std_temporal_epoch_ns(&fields) - zone as i128 * 60 * STD_TEMPORAL_NS_SECOND, zone))
        }
        _ => Err(Throw::type_error("invalid Temporal epoch input")),
    }
}

fn std_epoch_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, kind: StdEpochKind) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Temporal constructor requires new")); };
    if !object.borrow().constructed { return Err(Throw::type_error("Temporal constructor requires new")); }
    let epoch_ns = std_epoch_to_nanoseconds(i, &std_reflect_arg(&args, 0))?;
    let zone = if kind == StdEpochKind::ZonedDateTime {
        let zone = std_temporal_fixed_offset(&i.to_string(&std_reflect_arg(&args, 1))?)?;
        let calendar = std_reflect_arg(&args, 2);
        if !calendar.is_undefined() && !matches!(calendar, JsValue::Str(_)) {
            return Err(Throw::type_error("Temporal calendar must be a string"));
        }
        if !calendar.is_undefined() && !i.to_string(&calendar)?.eq_ignore_ascii_case("iso8601") {
            return Err(Throw::range_error("unsupported Temporal calendar"));
        }
        zone
    } else { 0 };
    std_epoch_make(i, kind, epoch_ns, zone)
}
fn std_epoch_to_nanoseconds(i: &mut Interpreter, value: &JsValue) -> Result<i128, Throw> {
    let bigint = i.to_bigint(value)?;
    bigint.to_radix_string(10).parse::<i128>()
        .map_err(|_| Throw::range_error("instant outside Temporal range"))
}
fn std_epoch_bigint(epoch_ns: i128) -> JsValue {
    Interpreter::bigint_value(BinaryBigInt::parse_string(&epoch_ns.to_string()).unwrap())
}
fn std_epoch_from_nanoseconds(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let epoch_ns = std_epoch_to_nanoseconds(i, &std_reflect_arg(&args, 0))?;
    std_epoch_make(i, StdEpochKind::Instant, epoch_ns, 0)
}
fn std_epoch_from_milliseconds(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let milliseconds = i.to_number(&std_reflect_arg(&args, 0))?;
    if !milliseconds.is_finite() || milliseconds.fract() != 0.0 || milliseconds.abs() > 8_640_000_000_000_000.0 {
        return Err(Throw::range_error("invalid epoch milliseconds"));
    }
    std_epoch_make(i, StdEpochKind::Instant, milliseconds as i128 * 1_000_000, 0)
}
fn std_epoch_instant_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_constructor(i, this, args, StdEpochKind::Instant)
}
fn std_epoch_zoned_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_constructor(i, this, args, StdEpochKind::ZonedDateTime)
}
fn std_epoch_kind_from_callee(i: &Interpreter) -> StdEpochKind {
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let _ = name;
    StdEpochKind::Instant
}
fn std_epoch_from(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let kind = if let JsValue::Object(object) = &this {
        let name = object.borrow().func.as_ref().map(|function| function.name.clone()).unwrap_or_default();
        if name == "ZonedDateTime" { StdEpochKind::ZonedDateTime } else { StdEpochKind::Instant }
    } else { std_epoch_kind_from_callee(i) };
    let (ns, zone) = std_epoch_from_value(i, std_reflect_arg(&args, 0), kind)?;
    std_epoch_make(i, kind, ns, zone)
}
fn std_epoch_compare(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let kind = if let JsValue::Object(object) = &this {
        if object.borrow().func.as_ref().is_some_and(|function| function.name == "ZonedDateTime") {
            StdEpochKind::ZonedDateTime
        } else { StdEpochKind::Instant }
    } else { StdEpochKind::Instant };
    let a = std_epoch_from_value(i, std_reflect_arg(&args, 0), kind)?.0;
    let b = std_epoch_from_value(i, std_reflect_arg(&args, 1), kind)?.0;
    Ok(JsValue::num(a.cmp(&b) as i32 as f64))
}

fn std_epoch_local_fields(data: &StdEpochData) -> Result<[i64; 10], Throw> {
    let mut fields = [0; 10];
    std_temporal_set_epoch_ns(&mut fields,
        data.epoch_ns + data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND)?;
    Ok(fields)
}

fn std_epoch_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let field = name.strip_prefix("get ").unwrap_or("");
    match field {
        "epochNanoseconds" => return Ok(std_epoch_bigint(data.epoch_ns)),
        "epochMilliseconds" => return Ok(JsValue::num(data.epoch_ns.div_euclid(1_000_000) as f64)),
        "timeZoneId" => return Ok(JsValue::str(std_epoch_zone_text(data.offset_minutes))),
        "calendarId" => return Ok(JsValue::str("iso8601")),
        "offset" => return Ok(JsValue::str(std_epoch_offset_text(data.offset_minutes))),
        "offsetNanoseconds" => return Ok(JsValue::num((data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND) as f64)),
        "era" | "eraYear" => return Ok(JsValue::Undefined),
        _ => {}
    }
    let v = std_epoch_local_fields(&data)?;
    let day = std_temporal_epoch_day(v[0], v[1], v[2]);
    let result = match field {
        "monthCode" => return Ok(JsValue::str(format!("M{:02}", v[1]))),
        "inLeapYear" => return Ok(JsValue::Bool(std_temporal_leap(v[0]))),
        "year" => v[0], "month" => v[1], "day" => v[2],
        "hour" => v[3], "minute" => v[4], "second" => v[5],
        "millisecond" => v[6], "microsecond" => v[7], "nanosecond" => v[8],
        "dayOfWeek" => std_temporal_day_of_week(day),
        "dayOfYear" => day - std_temporal_epoch_day(v[0], 1, 1) + 1,
        "daysInWeek" => 7, "daysInMonth" => std_temporal_days_in_month(v[0], v[1]),
        "daysInYear" => if std_temporal_leap(v[0]) { 366 } else { 365 },
        "monthsInYear" => 12, "hoursInDay" => 24,
        "weekOfYear" | "yearOfWeek" => {
            let year = if day < std_temporal_week_start(v[0]) { v[0] - 1 }
                else if day >= std_temporal_week_start(v[0] + 1) { v[0] + 1 } else { v[0] };
            if field == "yearOfWeek" { year } else { (day - std_temporal_week_start(year)) / 7 + 1 }
        }
        _ => return Ok(JsValue::Undefined),
    };
    Ok(JsValue::num(result as f64))
}

fn std_epoch_text(data: &StdEpochData) -> Result<String, Throw> {
    let fields = std_epoch_local_fields(data)?;
    let date = std_temporal_date_text(fields[0], fields[1], fields[2]);
    let time = std_temporal_time_text(&fields);
    if data.kind == StdEpochKind::Instant { Ok(format!("{date}T{time}Z")) }
    else { Ok(format!("{date}T{time}{}[{}]", std_epoch_offset_text(data.offset_minutes),
        std_epoch_zone_text(data.offset_minutes))) }
}
fn std_epoch_to_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut data = std_epoch_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if !option.is_undefined() && !option.is_object() {
        return Err(Throw::type_error("Temporal options must be an object"));
    }
    let (digits, minute_only, step, mode) = std_temporal_string_precision(i, &option)?;
    let calendar = std_temporal_option_string(i, &option, "calendarName")?
        .unwrap_or_else(|| "auto".to_string());
    let offset = std_temporal_option_string(i, &option, "offset")?
        .unwrap_or_else(|| "auto".to_string());
    let time_zone_name = std_temporal_option_string(i, &option, "timeZoneName")?
        .unwrap_or_else(|| "auto".to_string());
    if !matches!(calendar.as_str(), "auto" | "always" | "never" | "critical")
        || !matches!(offset.as_str(), "auto" | "never")
        || !matches!(time_zone_name.as_str(), "auto" | "never" | "critical") {
        return Err(Throw::range_error("invalid Temporal string option"));
    }
    if data.kind == StdEpochKind::Instant {
        let zone = std_temporal_option_value(i, &option, "timeZone")?;
        if !zone.is_undefined() { data.offset_minutes = std_temporal_fixed_offset(&i.to_string(&zone)?)?; }
    }
    if let Some(step) = step {
        let local = data.epoch_ns + data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND;
        data.epoch_ns = std_temporal_round_integer(local, step, &mode)?
            - data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND;
    }
    let fields = std_epoch_local_fields(&data)?;
    let mut text = format!("{}T{}", std_temporal_date_text(fields[0], fields[1], fields[2]),
        std_temporal_time_text_options(&fields, digits, minute_only));
    if data.kind == StdEpochKind::Instant {
        if data.offset_minutes == 0 { text.push('Z'); }
        else { text.push_str(&std_epoch_offset_text(data.offset_minutes)); }
    } else {
        if offset == "auto" { text.push_str(&std_epoch_offset_text(data.offset_minutes)); }
        if time_zone_name != "never" {
            text.push('[');
            if time_zone_name == "critical" { text.push('!'); }
            text.push_str(&std_epoch_zone_text(data.offset_minutes));
            text.push(']');
        }
        if matches!(calendar.as_str(), "always" | "critical") {
            text.push_str(if calendar == "critical" { "[!u-ca=iso8601]" } else { "[u-ca=iso8601]" });
        }
    }
    Ok(JsValue::str(text))
}
fn std_epoch_to_json(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_to_string(i, this, vec![])
}
fn std_epoch_value_of(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_record(&this, None)?;
    Err(Throw::type_error("Temporal values cannot be converted to primitives"))
}
fn std_epoch_equals(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let (other, zone) = std_epoch_from_value(i, std_reflect_arg(&args, 0), data.kind)?;
    Ok(JsValue::Bool(data.epoch_ns == other && (data.kind == StdEpochKind::Instant || data.offset_minutes == zone)))
}

fn std_epoch_to_plain(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let kind = match name.as_str() {
        "toPlainDate" => StdTemporalKind::PlainDate,
        "toPlainTime" => StdTemporalKind::PlainTime,
        _ => StdTemporalKind::PlainDateTime,
    };
    std_temporal_make(i, kind, std_epoch_local_fields(&data)?)
}
fn std_epoch_to_instant(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    std_epoch_make(i, StdEpochKind::Instant, data.epoch_ns, 0)
}
fn std_epoch_to_zoned(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::Instant))?;
    let zone = std_temporal_fixed_offset(&i.to_string(&std_reflect_arg(&args, 0))?)?;
    std_epoch_make(i, StdEpochKind::ZonedDateTime, data.epoch_ns, zone)
}
fn std_epoch_with_zone(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let zone = std_temporal_fixed_offset(&i.to_string(&std_reflect_arg(&args, 0))?)?;
    std_epoch_make(i, data.kind, data.epoch_ns, zone)
}
fn std_epoch_with_calendar(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    std_temporal_iso_calendar(i, &std_reflect_arg(&args, 0))?;
    std_epoch_make(i, data.kind, data.epoch_ns, data.offset_minutes)
}
fn std_epoch_with(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let local = std_temporal_make(i, StdTemporalKind::PlainDateTime, std_epoch_local_fields(&data)?)?;
    let changed = std_temporal_with(i, local, args)?;
    let fields = std_temporal_record(&changed, Some(StdTemporalKind::PlainDateTime))?.fields;
    std_epoch_make(i, data.kind, std_temporal_epoch_ns(&fields)
        - data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND, data.offset_minutes)
}
fn std_epoch_with_plain_time(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let mut fields = std_epoch_local_fields(&data)?;
    let time = std_reflect_arg(&args, 0);
    if time.is_undefined() { fields[3..9].fill(0); }
    else {
        let parts = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time)?;
        fields[3..9].copy_from_slice(&parts[3..9]);
    }
    std_epoch_make(i, data.kind, std_temporal_epoch_ns(&fields)
        - data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND, data.offset_minutes)
}
fn std_epoch_transition(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let option = std_reflect_arg(&args, 0);
    let direction = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "direction")?
            .ok_or_else(|| Throw::range_error("direction is required"))? };
    if !matches!(direction.as_str(), "next" | "previous") {
        return Err(Throw::range_error("invalid direction"));
    }
    Ok(JsValue::Null)
}
fn std_epoch_start_of_day(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let mut fields = std_epoch_local_fields(&data)?;
    fields[3..9].fill(0);
    std_epoch_make(i, data.kind, std_temporal_epoch_ns(&fields)
        - data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND, data.offset_minutes)
}

fn std_temporal_plain_to_zoned(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let input = std_reflect_arg(&args, 0);
    let zone_value = if data.kind == StdTemporalKind::PlainDate {
        if !input.is_object() { return Err(Throw::type_error("timeZone property bag required")); }
        i.get_prop(&input, "timeZone")?
    } else { input.clone() };
    let zone = std_temporal_fixed_offset(&i.to_string(&zone_value)?)?;
    let mut fields = data.fields;
    if data.kind == StdTemporalKind::PlainDate {
        let time = i.get_prop(&input, "plainTime")?;
        if !time.is_undefined() {
            let parsed = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time)?;
            fields[3..9].copy_from_slice(&parsed[3..9]);
        }
    }
    std_epoch_make(i, StdEpochKind::ZonedDateTime,
        std_temporal_epoch_ns(&fields) - zone as i128 * 60 * STD_TEMPORAL_NS_SECOND, zone)
}

fn std_temporal_now_instant(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_make(i, StdEpochKind::Instant, std_temporal_epoch_ns(&std_temporal_now_fields()), 0)
}
fn std_temporal_now_zoned(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let zone_value = std_reflect_arg(&args, 0);
    let zone = if zone_value.is_undefined() { 0 } else { std_temporal_fixed_offset(&i.to_string(&zone_value)?)? };
    std_epoch_make(i, StdEpochKind::ZonedDateTime, std_temporal_epoch_ns(&std_temporal_now_fields()), zone)
}

fn std_epoch_add_values(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, sign: i64) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let duration = std_temporal_from_fields(i, StdTemporalKind::Duration, std_reflect_arg(&args, 0))?;
    if data.kind == StdEpochKind::Instant && duration[..4].iter().any(|part| *part != 0) {
        return Err(Throw::range_error("date units are not valid for Instant"));
    }
    let result = if data.kind == StdEpochKind::Instant {
        data.epoch_ns + sign as i128 * std_temporal_duration_time_ns(&duration)
    } else {
        let local = std_epoch_local_fields(&data)?;
        let shifted = std_temporal_add_calendar(&local, &duration, sign, false)?;
        let time_ns = std_temporal_duration_time_ns(&duration)
            - (duration[2] as i128 * 7 + duration[3] as i128) * STD_TEMPORAL_NS_DAY;
        std_temporal_epoch_ns(&shifted) + sign as i128 * time_ns
            - data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND
    };
    std_epoch_make(i, data.kind, result, data.offset_minutes)
}
fn std_epoch_add(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_add_values(i, this, args, 1)
}
fn std_epoch_subtract(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_add_values(i, this, args, -1)
}
fn std_epoch_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, since: bool) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let (other, _) = std_epoch_from_value(i, std_reflect_arg(&args, 0), data.kind)?;
    let options = std_reflect_arg(&args, 1);
    let default = if data.kind == StdEpochKind::Instant { "second" } else { "hour" };
    let largest = std_temporal_largest_unit(i, &options, default)?;
    let (smallest, increment, mode) = std_temporal_rounding_options(i, &options, "nanosecond", "trunc")?;
    if data.kind == StdEpochKind::ZonedDateTime && matches!(largest.as_str(), "year" | "month") {
        let first = std_epoch_local_fields(&data)?;
        let mut second = [0; 10];
        std_temporal_set_epoch_ns(&mut second, other + data.offset_minutes as i128 * 60 * STD_TEMPORAL_NS_SECOND)?;
        let mut fields = std_temporal_calendar_difference(&first, &second, &largest)?;
        if since { for field in &mut fields { *field = -*field; } }
        return std_temporal_make(i, StdTemporalKind::Duration, fields);
    }
    let index = match largest.as_str() {
        "week" if data.kind == StdEpochKind::ZonedDateTime => 2,
        "day" if data.kind == StdEpochKind::ZonedDateTime => 3,
        "hour" => 4, "minute" => 5, "second" => 6,
        "millisecond" => 7, "microsecond" => 8, "nanosecond" => 9,
        _ => return Err(Throw::range_error("invalid largestUnit")),
    };
    let step = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
    let delta = if since { data.epoch_ns - other } else { other - data.epoch_ns };
    let rounded = std_temporal_round_integer(delta, step * increment as i128, &mode)?;
    std_temporal_make(i, StdTemporalKind::Duration, std_temporal_split_duration_ns(rounded, index))
}
fn std_epoch_until(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_difference(i, this, args, false)
}
fn std_epoch_since(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_difference(i, this, args, true)
}
fn std_epoch_round(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if option.is_undefined() { return Err(Throw::type_error("round options are required")); }
    let unit = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "smallestUnit")?
            .ok_or_else(|| Throw::range_error("smallestUnit is required"))? };
    let step = std_temporal_unit_ns(&unit).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
    let mode = if option.is_object() {
        std_temporal_option_string(i, &option, "roundingMode")?
            .unwrap_or_else(|| "halfExpand".to_string())
    } else { "halfExpand".to_string() };
    let increment = if option.is_object() { std_temporal_option_value(i, &option, "roundingIncrement")? }
        else { JsValue::Undefined };
    let increment = std_temporal_number(i, increment, Some(1))?;
    if increment < 1 { return Err(Throw::range_error("invalid roundingIncrement")); }
    let rounded = std_temporal_round_integer(data.epoch_ns, step * increment as i128, &mode)?;
    std_epoch_make(i, data.kind, rounded, data.offset_minutes)
}
