// Epoch nanoseconds are retained exactly in i128; the public boundary uses BigInt.

#[derive(Clone, Copy, PartialEq, Eq)]
enum StdEpochKind { Instant, ZonedDateTime }

#[derive(Clone)]
struct StdEpochData {
    owner: Weak<RefCell<JsObject>>,
    kind: StdEpochKind,
    epoch_ns: i128,
    zone_id: String,
    // Instant has no calendar concept at all and this is always "iso8601"
    // for it; ZonedDateTime's calendar is a VIEW-selecting tag exactly like
    // StdTemporalData's own (see builtins_temporal_calendar.rs's file
    // header): the object's local date/time is always computed from
    // epoch_ns/zone_id in proleptic-Gregorian (ISO) terms, and a non-iso8601
    // calendar's era/eraYear/year/month/monthCode/day/daysInMonth/
    // daysInYear/monthsInYear/inLeapYear are views computed on demand from
    // that local ISO date, matching JSS-calendars' PlainDate/PlainDateTime
    // architecture. Added by JSS-calendars item 2 (ZonedDateTime).
    calendar: String,
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
            let function = Self::native_constructor(name, constructor, &self.function_proto);
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
                    getter: Some(getter), setter: None };
                prototype.borrow_mut().define(*field, property).unwrap();
            }
            let mut methods: Vec<(&str, u32, crate::value::NativeFn)> = vec![
                ("toString", 0, std_epoch_to_string), ("toJSON", 0, std_epoch_to_json),
                ("toLocaleString", 0, std_epoch_to_locale_string), ("valueOf", 0, std_epoch_value_of),
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

fn std_epoch_make(i: &Interpreter, kind: StdEpochKind, epoch_ns: i128, zone_id: String) -> Result<JsValue, Throw> {
    std_epoch_make_cal(i, kind, epoch_ns, zone_id, "iso8601".to_string())
}

fn std_epoch_make_cal(i: &Interpreter, kind: StdEpochKind, epoch_ns: i128, zone_id: String,
    calendar: String) -> Result<JsValue, Throw> {
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
        StdEpochData { owner: Rc::downgrade(&object.0), kind, epoch_ns, zone_id, calendar }));
    Ok(JsValue::Object(object))
}

fn std_epoch_record(value: &JsValue, expected: Option<StdEpochKind>) -> Result<StdEpochData, Throw> {
    let JsValue::Object(object) = value else { return Err(Throw::type_error("incompatible Temporal receiver")); };
    STD_EPOCH_DATA.with(|registry| registry.borrow().get(&std_weak_id(object))
        .filter(|data| std_weak_same(&data.owner, object) && expected.is_none_or(|kind| kind == data.kind))
        .cloned().ok_or_else(|| Throw::type_error("incompatible Temporal receiver")))
}

fn std_epoch_zone(text: &str) -> Result<String, Throw> {
    tz_canonicalize(text).ok_or_else(|| Throw::range_error("invalid Temporal time zone"))
}

// The Temporal.ZonedDateTime constructor's `timeZone` parameter is resolved
// via ParseTimeZoneIdentifier, not ToTemporalTimeZoneIdentifier -- a much
// narrower grammar (a bare TimeZoneIdentifier only: an IANA name or a
// minute-precision numeric offset) than every other Temporal operation that
// accepts a time zone (`.from()`, `toZonedDateTime()`, relativeTo, ...),
// which additionally accept a full ISO date-time string (parsed for its
// bracket annotation or trailing offset) and a ZonedDateTime instance (reused
// via its own zone). The constructor accepts neither: a non-string --
// including a ZonedDateTime instance -- is a TypeError, and a syntactically
// full date-time string is a RangeError even though the same string is valid
// input to `.from()`/`toZonedDateTime()`. Confirmed by
// ZonedDateTime/timezone-iso-string.js.
fn std_epoch_zone_value_strict(value: &JsValue) -> Result<String, Throw> {
    match value {
        JsValue::Str(text) => std_epoch_zone(&text.to_utf8_lossy()),
        _ => Err(Throw::type_error("time zone must be a string")),
    }
}

// ToTemporalTimeZoneIdentifier: unlike a generic ToString coercion, a
// non-string, non-ZonedDateTime-instance value is a TypeError, not converted
// through its own toString/valueOf and then rejected as a malformed zone
// string (RangeError) -- so `undefined` (no `timeZone` property at all), a
// Symbol, a plain object, or any other Temporal instance all throw
// TypeError, while only an actual STRING that fails to parse as a zone
// identifier throws RangeError. Confirmed by ZonedDateTime.from's own
// `argument-propertybag-timezone-wrong-type.js` (an empty bag's `timeZone`
// is `undefined`, matching `argument-propertybag-empty-throws.js`'s/
// `argument-object.js`'s "empty object throws TypeError" too).
fn std_epoch_zone_value(i: &mut Interpreter, value: &JsValue) -> Result<String, Throw> {
    if let Ok(data) = std_epoch_record(value, Some(StdEpochKind::ZonedDateTime)) {
        return Ok(data.zone_id);
    }
    match value {
        JsValue::Str(text) => std_epoch_zone_from_string(&text.to_utf8_lossy()),
        _ => Err(Throw::type_error("time zone must be a string or a ZonedDateTime instance")),
    }
}

// GetTemporalOffsetStringFor (PrepareCalendarFields' handling of a
// string-valued field like `offset`): a String or a non-null Object is
// still coerced via ToString (an Object's own toString/valueOf can still
// produce a garbage string, which then fails as a malformed offset and
// throws RangeError, not TypeError) -- but null, a boolean, a number, or a
// BigInt throw TypeError immediately, without ever attempting ToString,
// even though plain ToString(null)/(true)/(1000n) would not themselves
// throw. Confirmed by ZonedDateTime.from's own
// `argument-propertybag-invalid-offset-string.js`.
fn std_epoch_string_field_value(i: &mut Interpreter, value: &JsValue) -> Result<String, Throw> {
    if matches!(value, JsValue::Str(_)) || value.is_object() {
        i.to_string(value)
    } else {
        Err(Throw::type_error("value must be a string or an object"))
    }
}

// ParseTemporalTimeZoneString: a bare TimeZoneIdentifier (IANA name, "UTC",
// or a minute-precision numeric offset) is tried first. Otherwise the text
// must parse as an ISO date-time; a bracketed time zone annotation wins over
// the date-time's own `Z`/offset, `Z` alone means UTC, and a bare numeric
// offset with no annotation becomes the zone itself -- but, unlike an
// Instant/ZonedDateTime exact offset, only at minute precision (no seconds
// or sub-second digits; TimeZoneNumericUTCOffset forbids them where
// TimeZoneUTCOffset, used for exact time, allows them).
fn std_epoch_zone_from_string(text: &str) -> Result<String, Throw> {
    let invalid = || Throw::range_error("invalid Temporal time zone");
    if let Some(zone) = tz_canonicalize(text) { return Ok(zone); }
    let (body, annotation, _calendar) = std_epoch_validate_annotations(text, false)?;
    // The date-time is validated in full regardless of whether an
    // annotation ultimately supplies the zone -- an annotation only picks
    // which zone id wins, it does not exempt the rest of the string from
    // ParseISODateTime.
    let time_start = body.find('T').or_else(|| body.find('t')).or_else(|| body.find(' ')).ok_or_else(invalid)?;
    let date = &body[..time_start];
    let (_, _, _, tail) = std_temporal_parse_date(date, true)?;
    if !tail.is_empty() { return Err(invalid()); }
    let time = &body[time_start + 1..];
    let mut fields = [0; 10];
    if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        std_temporal_parse_time(clock, &mut fields)?;
        return match annotation {
            Some(zone) => std_epoch_zone(zone),
            None => Ok("UTC".into()),
        };
    }
    match time.char_indices().find(|(index, c)| *index > 0 && (*c == '+' || *c == '-')).map(|(index, _)| index) {
        Some(start) => {
            std_temporal_parse_time(&time[..start], &mut fields)?;
            let offset_text = &time[start..];
            if let Some(zone) = annotation {
                std_epoch_parse_offset(offset_text)?;
                return std_epoch_zone(zone);
            }
            let ns = std_epoch_parse_offset(offset_text)?;
            // TimeZoneNumericUTCOffset (a bare numeric offset used AS the
            // zone identifier itself, with no bracket annotation) forbids a
            // seconds/sub-second component syntactically -- "+01:00:00" is
            // invalid here even though it is numerically identical to
            // "+01:00" (unlike TimeZoneUTCOffset, the date-time's own exact
            // offset, which allows arbitrary sub-second precision).
            // Confirmed by timezone-string-datetime.js's exhaustive
            // sub-minute-offset sweep, which rejects every zero-second and
            // zero-fraction variant too, not just non-zero ones.
            let digits = offset_text.trim_start_matches(['+', '-']);
            let whole = digits.split(['.', ',']).next().unwrap_or(digits);
            let has_seconds_syntax = digits.contains(['.', ',']) || whole.len() == 6 || whole.matches(':').count() == 2;
            if ns % (60 * STD_TEMPORAL_NS_SECOND) != 0 || has_seconds_syntax { return Err(invalid()); }
            let sign = if ns < 0 { '-' } else { '+' };
            let minutes = (ns.unsigned_abs() / (60 * STD_TEMPORAL_NS_SECOND as u128)) as i64;
            Ok(format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60))
        }
        None => {
            // No `Z` and no explicit numeric offset -- the date-time's own
            // local time carries no zone information of its own, so only a
            // bracketed annotation can supply the zone (a bare date-time
            // string with neither is not a time zone identifier at all).
            // Confirmed by timezone-string-datetime.js: "2021-08-19T17:30"
            // alone throws, while "2021-08-19T17:30[UTC]" resolves via the
            // annotation.
            match annotation {
                Some(zone) => { std_temporal_parse_time(time, &mut fields)?; std_epoch_zone(zone) }
                None => Err(invalid()),
            }
        }
    }
}

fn std_epoch_offset(data: &StdEpochData) -> i128 {
    tz_offset_ns(&data.zone_id, data.epoch_ns)
}

fn std_epoch_offset_text(offset_ns: i128) -> String {
    let seconds = offset_ns / STD_TEMPORAL_NS_SECOND;
    let absolute = seconds.abs();
    let mut text = format!("{}{:02}:{:02}", if seconds < 0 { '-' } else { '+' },
        absolute / 3600, absolute / 60 % 60);
    if absolute % 60 != 0 || offset_ns % STD_TEMPORAL_NS_SECOND != 0 {
        text.push_str(&format!(":{:02}", absolute % 60));
        if offset_ns % STD_TEMPORAL_NS_SECOND != 0 {
            text.push_str(&format!(".{:09}", offset_ns.abs() % STD_TEMPORAL_NS_SECOND));
            while text.ends_with('0') { text.pop(); }
        }
    }
    text
}

fn std_epoch_parse_offset(text: &str) -> Result<i128, Throw> {
    let invalid = || Throw::range_error("invalid Temporal offset");
    let (sign, digits) = if let Some(rest) = text.strip_prefix('+') { (1, rest) }
        else if let Some(rest) = text.strip_prefix('-') { (-1, rest) } else { return Err(invalid()); };
    let (whole, fraction) = digits.split_once(['.', ',']).unwrap_or((digits, ""));
    let parts: Vec<&str> = whole.split(':').collect();
    let (hour, minute, second) = match parts.as_slice() {
        [hour] if matches!(hour.len(), 2 | 4 | 6) =>
            (&hour[..2], if hour.len() >= 4 { &hour[2..4] } else { "00" },
                if hour.len() == 6 { &hour[4..6] } else { "00" }),
        [hour, minute] if hour.len() == 2 && minute.len() == 2 => (*hour, *minute, "00"),
        [hour, minute, second] if hour.len() == 2 && minute.len() == 2 && second.len() == 2 =>
            (*hour, *minute, *second),
        _ => return Err(invalid()),
    };
    let has_seconds = (parts.len() == 1 && whole.len() == 6) || parts.len() == 3;
    let has_fraction = digits.contains(['.', ',']);
    if fraction.len() > 9 || (has_fraction && (fraction.is_empty() || !has_seconds))
        || !whole.bytes().filter(|c| *c != b':').chain(fraction.bytes()).all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    let hour = hour.parse::<i128>().map_err(|_| invalid())?;
    let minute = minute.parse::<i128>().map_err(|_| invalid())?;
    let second = second.parse::<i128>().map_err(|_| invalid())?;
    if hour > 23 || minute > 59 || second > 59 { return Err(invalid()); }
    let fraction = format!("{fraction:0<9}").parse::<i128>().map_err(|_| invalid())?;
    Ok(sign * (((hour * 60 + minute) * 60 + second) * STD_TEMPORAL_NS_SECOND + fraction))
}

fn std_epoch_validate_disambiguation(mode: &str) -> Result<(), Throw> {
    if !matches!(mode, "compatible" | "earlier" | "later" | "reject") {
        return Err(Throw::range_error("invalid disambiguation"));
    }
    Ok(())
}

fn std_epoch_disambiguate(zone: &str, local_ns: i128, mode: &str) -> Result<i128, Throw> {
    std_epoch_validate_disambiguation(mode)?;
    let possible = tz_possible_epoch_ns(zone, local_ns);
    match possible.as_slice() {
        [one] => Ok(*one),
        [earlier, later] => match mode {
            "earlier" | "compatible" => Ok(*earlier),
            "later" => Ok(*later),
            _ => Err(Throw::range_error("ambiguous Temporal local time")),
        },
        [] => {
            if mode == "reject" { return Err(Throw::range_error("nonexistent Temporal local time")); }
            let before = tz_offset_ns(zone, local_ns - STD_TEMPORAL_NS_DAY);
            let after = tz_offset_ns(zone, local_ns + STD_TEMPORAL_NS_DAY);
            let first = local_ns - before;
            let second = local_ns - after;
            if mode == "earlier" { Ok(first.min(second)) } else { Ok(first.max(second)) }
        }
        _ => Err(Throw::range_error("invalid Temporal local time")),
    }
}

// Syntactic validation of ISO annotations for Instant/ZonedDateTime strings.
// Zone and calendar names are returned opaquely: Instant ignores them, while
// ZonedDateTime canonicalizes the zone itself. Multiple time zone annotations,
// a repeated critical calendar annotation, uppercase unknown keys, critical
// unknown annotations, and trailing junk are syntax errors.
fn std_epoch_annotation_offset(text: &str) -> bool {
    let Some(digits) = text.strip_prefix(['+', '-']) else { return false; };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit() || c == b':') { return false; }
    match digits.split(':').collect::<Vec<_>>().as_slice() {
        [hour] => matches!(hour.len(), 2 | 4),
        [hour, minute] => hour.len() == 2 && minute.len() == 2,
        _ => false,
    }
}

fn std_epoch_annotation_zone(text: &str) -> bool {
    if text.starts_with(['+', '-']) { return std_epoch_annotation_offset(text); }
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(c) if c.is_ascii_alphabetic())
        && text.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'_' | b'-' | b'+' | b'.'))
}

// Returns (body, zone annotation, calendar annotation). Only the FIRST
// u-ca= annotation is ever consulted (a second one is still grammar-checked
// -- including its own/the first's "!" making the pair illegal -- but never
// overwrites the first), matching std_temporal_parse_annotations' identical
// rule for PlainDate/PlainDateTime/PlainYearMonth/PlainMonthDay (JSS-
// calendars item 1). `require_known_calendar` gates whether the name must
// canonicalize via std_calendar_canonicalize: true for ZonedDateTime (which,
// like PlainDate/PlainDateTime, accepts any of the 16 supported calendars in
// its own string form), false for Instant and for a bare time-zone-value
// string (std_epoch_zone_from_string) -- neither has a calendar slot at all,
// so any syntactically valid id (including an outright unknown one) is
// accepted and simply discarded, exactly like PlainTime's own calendar
// annotation. Confirmed by Instant's own argument-string-calendar-
// annotation.js, which uses an unrecognized calendar name on purpose.
fn std_epoch_validate_annotations(text: &str, require_known_calendar: bool) -> Result<(&str, Option<&str>, Option<&str>), Throw> {
    let invalid = || Throw::range_error("invalid Temporal annotation");
    let Some(first) = text.find('[') else { return Ok((text, None, None)); };
    let body = &text[..first];
    let mut tail = &text[first..];
    let mut calendar_name: Option<&str> = None;
    let mut critical_calendar = false;
    let mut time_zone: Option<&str> = None;
    while !tail.is_empty() {
        let rest = tail.strip_prefix('[').ok_or_else(invalid)?;
        let end = rest.find(']').ok_or_else(invalid)?;
        let value = &rest[..end];
        tail = &rest[end + 1..];
        let critical = value.starts_with('!');
        let annotation = value.strip_prefix('!').unwrap_or(value);
        if let Some(name) = annotation.strip_prefix("u-ca=") {
            if name.is_empty()
                || name.bytes().any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')) {
                return Err(invalid());
            }
            if calendar_name.is_none() {
                if require_known_calendar && std_calendar_canonicalize(name).is_none() { return Err(invalid()); }
                calendar_name = Some(name);
                critical_calendar = critical;
            } else if critical_calendar || critical {
                return Err(invalid());
            }
        } else if let Some((key, _)) = annotation.split_once('=') {
            if critical || key.is_empty() || key.bytes().any(|c| c.is_ascii_uppercase()) {
                return Err(invalid());
            }
        } else {
            if time_zone.is_some() || annotation.is_empty() || !std_epoch_annotation_zone(annotation) {
                return Err(invalid());
            }
            time_zone = Some(annotation);
        }
    }
    Ok((body, time_zone, calendar_name))
}

// The string parse is split into a pure-syntax phase (`std_epoch_parse_syntax`)
// and an options-dependent resolution phase (`std_epoch_resolve_parsed`) so
// that callers can surface a malformed string's RangeError before ever
// reading (and possibly rejecting the type of) `options` -- ToTemporalZoned
// DateTime/ToTemporalInstant parse the string as part of resolving `item`
// ahead of validating `options`.
struct StdEpochParsed {
    fields: [i64; 10],
    zone: String,
    offset_absent: bool,
    exact: bool,
    offset: i128,
    minute_precision: bool,
    calendar: String,
}

fn std_epoch_parse_syntax(text: &str, kind: StdEpochKind) -> Result<StdEpochParsed, Throw> {
    let invalid = || Throw::range_error("invalid Temporal instant string");
    // Annotations are validated syntactically for every epoch type. Instant
    // ignores the zone and calendar (its own calendar is always "iso8601");
    // ZonedDateTime requires a zone it can canonicalize and canonicalizes
    // its own calendar annotation the same way PlainDate/PlainDateTime do
    // (JSS-calendars item 2).
    let (body, zone_annotation, calendar_annotation) = std_epoch_validate_annotations(text, kind == StdEpochKind::ZonedDateTime)?;
    let zone = if kind == StdEpochKind::ZonedDateTime {
        std_epoch_zone(zone_annotation.ok_or_else(invalid)?)?
    } else {
        "UTC".into()
    };
    let calendar = if kind == StdEpochKind::ZonedDateTime {
        calendar_annotation.and_then(std_calendar_canonicalize).unwrap_or("iso8601").to_string()
    } else {
        "iso8601".to_string()
    };
    let time_start = body.find('T').or_else(|| body.find('t')).or_else(|| body.find(' '));
    // A TemporalZonedDateTimeString may be just `Date TimeZoneAnnotation`,
    // with no time-of-day at all -- "2000-01-01[UTC]" is valid and means
    // wall-clock midnight in that zone, resolved through disambiguation like
    // any other offset-absent wall-clock time. Every other kind (Instant,
    // and ZonedDateTime strings that do carry a time) still requires the
    // `T`/`t`/` ` separator.
    if time_start.is_none() && kind != StdEpochKind::ZonedDateTime { return Err(invalid()); }
    let time = time_start.map_or("", |start| &body[start + 1..]);
    // A ZonedDateTime string may omit both `Z` and a numeric UTC offset when a
    // bracketed time zone annotation supplies the zone (OffsetBehaviour
    // "wall" in the spec's ParseISODateTime): the wall-clock local time is
    // then resolved through disambiguation alone. Instant strings have no
    // annotation fallback and must always carry `Z` or a numeric offset.
    let (clock, offset, exact, offset_absent) = if time.is_empty() {
        ("", 0, false, true)
    } else if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        (clock, 0, true, false)
    } else if let Some(start) = time.char_indices().find(|(index, c)| *index > 0 && (*c == '+' || *c == '-')).map(|(index, _)| index) {
        (&time[..start], std_epoch_parse_offset(&time[start..])?, false, false)
    } else if kind == StdEpochKind::ZonedDateTime {
        (time, 0, false, true)
    } else {
        return Err(invalid());
    };
    let date = &body[..time_start.unwrap_or(body.len())];
    let (year, month, day, tail) = std_temporal_parse_date(date, true)?;
    if !tail.is_empty() { return Err(invalid()); }
    if kind == StdEpochKind::ZonedDateTime && !exact && !offset_absent {
        // InterpretISODateTimeOffset's ~option~/~reject~ branch (an explicit
        // non-"Z" numeric offset, resolved against a zone) runs
        // CheckISODaysRange on the *unadjusted local* ISO date before ever
        // computing an epoch instant -- rejecting a local date more than
        // 10^8 days from the epoch even when the offset would shift the
        // final instant back into Instant's own +-nsMaxInstant range.
        // "-271821-04-19T23:00-01:00[-01:00]" resolves to exactly
        // nsMinInstant (valid on its own), but its *local* date
        // "-271821-04-19" is one day past the 10^8-day boundary and must
        // still throw -- confirmed by relativeto-string-limits.js.
        if std_temporal_epoch_day(year, month, day).abs() > 100_000_000 { return Err(invalid()); }
    }
    let mut fields = [0; 10];
    fields[0] = year; fields[1] = month; fields[2] = day;
    if !time.is_empty() { std_temporal_parse_time(clock, &mut fields)?; }
    std_temporal_validate(StdTemporalKind::PlainDateTime, &fields)?;
    let minute_precision = !time.is_empty() && time[clock.len()..].matches(':').count() == 1;
    Ok(StdEpochParsed { fields, zone, offset_absent, exact, offset, minute_precision, calendar })
}

fn std_epoch_check_range(epoch_ns: i128) -> Result<i128, Throw> {
    if !(-8_640_000_000_000_000_000_000i128..=8_640_000_000_000_000_000_000i128).contains(&epoch_ns) {
        return Err(Throw::range_error("instant outside Temporal range"));
    }
    Ok(epoch_ns)
}

fn std_epoch_resolve_parsed(parsed: StdEpochParsed, kind: StdEpochKind, offset_option: &str,
    disambiguation: &str) -> Result<(i128, String, String), Throw> {
    let invalid = || Throw::range_error("invalid Temporal instant string");
    let StdEpochParsed { fields, zone, offset_absent, exact, offset, minute_precision, calendar } = parsed;
    if kind == StdEpochKind::ZonedDateTime && offset_absent {
        let epoch_ns = std_epoch_disambiguate(&zone, std_temporal_epoch_ns(&fields), disambiguation)?;
        return Ok((std_epoch_check_range(epoch_ns)?, zone, calendar));
    }
    let epoch_ns = std_epoch_check_range(std_temporal_epoch_ns(&fields) - offset)?;
    if kind == StdEpochKind::ZonedDateTime && !exact {
        let actual_offset = tz_offset_ns(&zone, epoch_ns);
        let matches = actual_offset == offset || (minute_precision
            && std_temporal_round_integer(actual_offset, 60 * STD_TEMPORAL_NS_SECOND, "halfExpand")? == offset);
        return match offset_option {
            "use" => Ok((epoch_ns, zone, calendar)),
            "ignore" => Ok((std_epoch_check_range(std_epoch_disambiguate(&zone, std_temporal_epoch_ns(&fields), disambiguation)?)?, zone, calendar)),
            "prefer" if !matches => Ok((std_epoch_check_range(std_epoch_disambiguate(&zone, std_temporal_epoch_ns(&fields), disambiguation)?)?, zone, calendar)),
            _ if matches => Ok((epoch_ns, zone, calendar)),
            _ => Err(invalid()),
        };
    }
    Ok((epoch_ns, zone, calendar))
}

fn std_epoch_parse(text: &str, kind: StdEpochKind, offset_option: &str,
    disambiguation: &str) -> Result<(i128, String, String), Throw> {
    let parsed = std_epoch_parse_syntax(text, kind)?;
    std_epoch_resolve_parsed(parsed, kind, offset_option, disambiguation)
}

fn std_epoch_from_value(i: &mut Interpreter, value: JsValue, kind: StdEpochKind) -> Result<(i128, String, String), Throw> {
    std_epoch_from_value_with_options(i, value, kind, JsValue::Undefined)
}

fn std_epoch_from_value_with_options(i: &mut Interpreter, value: JsValue, kind: StdEpochKind,
    options: JsValue) -> Result<(i128, String, String), Throw> {
    if let Ok(data) = std_epoch_record(&value, Some(kind)) {
        // `item` is already the target kind: ZonedDateTime.from clones it
        // without applying overflow/disambiguation/offset, but per spec
        // still reads and validates them (surfacing e.g. a wrong-type
        // TypeError) before returning the clone.
        if kind == StdEpochKind::ZonedDateTime {
            std_temporal_overflow(i, options.clone())?;
            let disambiguation = std_temporal_option_string(i, &options, "disambiguation")?
                .unwrap_or_else(|| "compatible".into());
            std_epoch_validate_disambiguation(&disambiguation)?;
            let offset_option = std_temporal_option_string(i, &options, "offset")?
                .unwrap_or_else(|| "reject".into());
            if !matches!(offset_option.as_str(), "reject" | "use" | "ignore" | "prefer") {
                return Err(Throw::range_error("invalid Temporal offset option"));
            }
        }
        return Ok((data.epoch_ns, data.zone_id.clone(), data.calendar.clone()));
    }
    // ToTemporalInstant also accepts a ZonedDateTime instance; Instant has
    // no calendar of its own, so the source's calendar is discarded (this
    // return's third element is never consulted by an Instant caller).
    if kind == StdEpochKind::Instant {
        if let Ok(data) = std_epoch_record(&value, Some(StdEpochKind::ZonedDateTime)) {
            return Ok((data.epoch_ns, data.zone_id.clone(), "iso8601".to_string()));
        }
    }
    // A string argument is parsed -- and any syntax error surfaced -- before
    // its options are read: ToTemporalZonedDateTime/ToTemporalInstant parse
    // the string while resolving `item`, ahead of validating `options`.
    if let JsValue::Str(text) = &value {
        let parsed = std_epoch_parse_syntax(&text.to_utf8_lossy(), kind)?;
        let (disambiguation, offset_option) = if kind == StdEpochKind::ZonedDateTime {
            (std_temporal_option_string(i, &options, "disambiguation")?
                .unwrap_or_else(|| "compatible".into()),
             std_temporal_option_string(i, &options, "offset")?
                .unwrap_or_else(|| "reject".into()))
        } else { ("compatible".into(), "reject".into()) };
        if !matches!(offset_option.as_str(), "reject" | "use" | "ignore" | "prefer") {
            return Err(Throw::range_error("invalid Temporal offset option"));
        }
        if kind == StdEpochKind::ZonedDateTime {
            std_epoch_validate_disambiguation(&disambiguation)?;
            std_temporal_overflow(i, options.clone())?;
        }
        return std_epoch_resolve_parsed(parsed, kind, &offset_option, &disambiguation);
    }
    let (disambiguation, offset_option) = if kind == StdEpochKind::ZonedDateTime {
        (std_temporal_option_string(i, &options, "disambiguation")?
            .unwrap_or_else(|| "compatible".into()),
         std_temporal_option_string(i, &options, "offset")?
            .unwrap_or_else(|| "reject".into()))
    } else { ("compatible".into(), "reject".into()) };
    if !matches!(offset_option.as_str(), "reject" | "use" | "ignore" | "prefer") {
        return Err(Throw::range_error("invalid Temporal offset option"));
    }
    let reject = kind == StdEpochKind::ZonedDateTime && std_temporal_overflow(i, options.clone())?;
    match value {
        JsValue::Object(_) if kind == StdEpochKind::ZonedDateTime => {
            // Calendar resolution (RangeError for an unsupported calendar id,
            // e.g. "islamic") happens before PrepareCalendarFields' field
            // reads, which is in turn before timeZone is read/validated --
            // std_temporal_bag validates the calendar as its first action.
            // The resolved calendar id is then used (JSS-calendars item 2)
            // to convert the bag's still calendar-native year/month/day into
            // this object's always-ISO internal fields, exactly as the
            // Plain* kinds' own bag path does (std_temporal_bag_resolve_calendar).
            let (mut fields, month, code, calendar) = std_temporal_bag(i, StdTemporalKind::PlainDateTime, &value)?;
            std_temporal_bag_resolve_calendar(StdTemporalKind::PlainDateTime, &calendar, &mut fields, month, code, reject)?;
            let zone_value = i.get_prop(&value, "timeZone")?;
            let zone = std_epoch_zone_value(i, &zone_value)?;
            std_temporal_regulate(StdTemporalKind::PlainDateTime, &mut fields, reject);
            std_temporal_validate(StdTemporalKind::PlainDateTime, &fields)?;
            let local_ns = std_temporal_epoch_ns(&fields);
            let given_offset = i.get_prop(&value, "offset")?;
            // The offset field's syntax is validated (via
            // std_epoch_parse_offset, RangeError on a malformed string) as
            // soon as it is read, regardless of the offset *option* -- even
            // "ignore", which discards the parsed value, still requires it
            // to have been well-formed.
            let offset_ns = if given_offset.is_undefined() {
                None
            } else {
                Some(std_epoch_parse_offset(&std_epoch_string_field_value(i, &given_offset)?)?)
            };
            let epoch_ns = if offset_ns.is_none() || offset_option == "ignore" {
                std_epoch_disambiguate(&zone, local_ns, &disambiguation)?
            } else {
                let offset_ns = offset_ns.unwrap();
                let candidate = local_ns - offset_ns;
                let matches = tz_offset_ns(&zone, candidate) == offset_ns;
                match offset_option.as_str() {
                    "use" => candidate,
                    "prefer" if !matches => std_epoch_disambiguate(&zone, local_ns, &disambiguation)?,
                    _ if matches => candidate,
                    _ => return Err(Throw::range_error("Temporal offset does not match time zone")),
                }
            };
            Ok((std_epoch_check_range(epoch_ns)?, zone, calendar))
        }
        // Any other object is coerced to its string form, as ToTemporalInstant does.
        JsValue::Object(_) => {
            let text = i.to_js_string(&value)?;
            std_epoch_parse(&text.to_utf8_lossy(), kind, &offset_option, &disambiguation)
        }
        _ => Err(Throw::type_error("invalid Temporal epoch input")),
    }
}

fn std_epoch_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, kind: StdEpochKind) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Temporal constructor requires new")); };
    if !object.borrow().constructed { return Err(Throw::type_error("Temporal constructor requires new")); }
    let epoch_ns = std_epoch_to_nanoseconds(i, &std_reflect_arg(&args, 0))?;
    let (zone, calendar_id) = if kind == StdEpochKind::ZonedDateTime {
        let zone = std_epoch_zone_value_strict(&std_reflect_arg(&args, 1))?;
        // Strict resolution, matching every Plain* constructor's own
        // calendar argument (std_temporal_resolve_calendar_id_strict):
        // rejects a full ISO-string-with-annotation form that with()/
        // withCalendar()/a bag's own `calendar` property all accept.
        // Confirmed by ZonedDateTime's own calendar-invalid-iso-string.js.
        let calendar = std_temporal_resolve_calendar_id_strict(i, &std_reflect_arg(&args, 2))?;
        (zone, calendar)
    } else { ("UTC".into(), "iso8601".to_string()) };
    std_epoch_make_cal(i, kind, epoch_ns, zone, calendar_id)
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
    std_epoch_make(i, StdEpochKind::Instant, epoch_ns, "UTC".into())
}
fn std_epoch_from_milliseconds(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let milliseconds = i.to_number(&std_reflect_arg(&args, 0))?;
    if !milliseconds.is_finite() || milliseconds.fract() != 0.0 || milliseconds.abs() > 8_640_000_000_000_000.0 {
        return Err(Throw::range_error("invalid epoch milliseconds"));
    }
    std_epoch_make(i, StdEpochKind::Instant, milliseconds as i128 * 1_000_000, "UTC".into())
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
    let (ns, zone, calendar) = std_epoch_from_value_with_options(i, std_reflect_arg(&args, 0), kind,
        std_reflect_arg(&args, 1))?;
    std_epoch_make_cal(i, kind, ns, zone, calendar)
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
        data.epoch_ns + std_epoch_offset(data))?;
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
        "timeZoneId" => return Ok(JsValue::str(data.zone_id.clone())),
        "calendarId" => return Ok(JsValue::str(data.calendar.clone())),
        "offset" => return Ok(JsValue::str(std_epoch_offset_text(std_epoch_offset(&data)))),
        "offsetNanoseconds" => return Ok(JsValue::num(std_epoch_offset(&data) as f64)),
        _ => {}
    }
    let v = std_epoch_local_fields(&data)?;
    let day = std_temporal_epoch_day(v[0], v[1], v[2]);
    // Non-iso8601 calendars are VIEWS computed on demand from the object's
    // always-ISO local date, exactly like PlainDate/PlainDateTime's own
    // getter (std_temporal_getter, builtins_temporal.rs) -- JSS-calendars
    // item 2. (2026-09-27: confirmed required for every calendar, including
    // chinese/dangi -- toPlainDate()/toPlainDateTime()/toPlainYearMonth()
    // share this object's calendar tag with a Plain* object whose OWN
    // getters are already unconditionally calendar-real (JSS-calendars item
    // 1, not touched by this ticket), so narrowing ZDT's own getters to ISO
    // for chinese/dangi just moves the self-consistency break to the
    // ZDT-getter-vs-toPlainDateTime()-getter boundary instead of fixing it
    // -- confirmed by direct measurement. The actual gap is since()/until()
    // for chinese/dangi only; see its call site below and questions.md.)
    let view_cal: &str = &data.calendar;
    let calendar_fields = if view_cal != "iso8601" {
        Some(std_calendar_fields(view_cal, v[0], v[1], v[2]))
    } else { None };
    match field {
        "era" => {
            if view_cal == "iso8601" { return Ok(JsValue::Undefined); }
            let (era, _) = std_calendar_era(view_cal, v[0], v[1], v[2]);
            return Ok(era.map(JsValue::str).unwrap_or(JsValue::Undefined));
        }
        "eraYear" => {
            if view_cal == "iso8601" { return Ok(JsValue::Undefined); }
            let (_, era_year) = std_calendar_era(view_cal, v[0], v[1], v[2]);
            return Ok(era_year.map(|value| JsValue::num(value as f64)).unwrap_or(JsValue::Undefined));
        }
        "monthCode" => {
            let code = if view_cal == "iso8601" { format!("M{:02}", v[1]) }
                else { std_calendar_month_code(view_cal, v[0], v[1], v[2]) };
            return Ok(JsValue::str(code));
        }
        _ => {}
    }
    let result = match field {
        "inLeapYear" => return Ok(JsValue::Bool(calendar_fields.map(|f| f.7).unwrap_or_else(|| std_temporal_leap(v[0])))),
        "year" => calendar_fields.map(|f| f.0).unwrap_or(v[0]),
        "month" => calendar_fields.map(|f| f.1).unwrap_or(v[1]),
        "day" => calendar_fields.map(|f| f.2).unwrap_or(v[2]),
        "hour" => v[3], "minute" => v[4], "second" => v[5],
        "millisecond" => v[6], "microsecond" => v[7], "nanosecond" => v[8],
        "dayOfWeek" => std_temporal_day_of_week(day),
        "dayOfYear" => calendar_fields.map(|f| f.5)
            .unwrap_or_else(|| day - std_temporal_epoch_day(v[0], 1, 1) + 1),
        "daysInWeek" => 7, "daysInMonth" => calendar_fields.map(|f| f.3).unwrap_or_else(|| std_temporal_days_in_month(v[0], v[1])),
        "daysInYear" => calendar_fields.map(|f| f.6).unwrap_or_else(|| if std_temporal_leap(v[0]) { 366 } else { 365 }),
        "monthsInYear" => calendar_fields.map(|f| f.4).unwrap_or(12),
        "hoursInDay" => {
            let mut midnight = v;
            midnight[3..9].fill(0);
            let local_midnight = std_temporal_epoch_ns(&midnight);
            let start = std_epoch_disambiguate(&data.zone_id, local_midnight, "compatible")?;
            std_temporal_set_epoch_ns(&mut midnight, local_midnight + STD_TEMPORAL_NS_DAY)?;
            let end = std_epoch_disambiguate(&data.zone_id, std_temporal_epoch_ns(&midnight), "compatible")?;
            return Ok(JsValue::num((end - start) as f64 / STD_TEMPORAL_NS_SECOND as f64 / 3600.0));
        }
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
    else { Ok(format!("{date}T{time}{}[{}]", std_epoch_offset_text(std_epoch_offset(data)),
        data.zone_id)) }
}
// toLocaleString(locale, options) takes an Intl.DateTimeFormat-style options
// bag (dateStyle/timeStyle/...), NOT toString's own (calendarName/offset/
// timeZoneName/...) options -- passing it straight through to
// std_epoch_to_string would misinterpret it (or reject a non-object locale
// argument as an invalid Temporal option). No locale-sensitive formatting is
// implemented; this only has to return *a* string, per the spec's minimal
// requirement absent a full Intl.DateTimeFormat.
fn std_epoch_to_locale_string(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_to_string(i, this, vec![])
}

fn std_epoch_to_string(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut data = std_epoch_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if !option.is_undefined() && !option.is_object() {
        return Err(Throw::type_error("Temporal options must be an object"));
    }
    let (digits, minute_only, step, mode) = std_temporal_string_precision(i, &option, true)?;
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
        if !zone.is_undefined() { data.zone_id = std_epoch_zone_value(i, &zone)?; }
    }
    if let Some(step) = step {
        let local = data.epoch_ns + std_epoch_offset(&data);
        data.epoch_ns = std_temporal_round_integer(local, step, &mode)?
            - std_epoch_offset(&data);
    }
    let fields = std_epoch_local_fields(&data)?;
    let mut text = format!("{}T{}", std_temporal_date_text(fields[0], fields[1], fields[2]),
        std_temporal_time_text_options(&fields, digits, minute_only));
    if data.kind == StdEpochKind::Instant {
        if std_epoch_offset(&data) == 0 { text.push('Z'); }
        else { text.push_str(&std_epoch_offset_text(std_epoch_offset(&data))); }
    } else {
        if offset == "auto" { text.push_str(&std_epoch_offset_text(std_epoch_offset(&data))); }
        if time_zone_name != "never" {
            text.push('[');
            if time_zone_name == "critical" { text.push('!'); }
            text.push_str(&data.zone_id);
            text.push(']');
        }
        // "auto" shows the calendar annotation only when it is not iso8601
        // (matching Plain*'s own toString), "never" never shows it
        // regardless of the calendar, and "always"/"critical" always show
        // it (optionally critical-flagged).
        if calendar == "always" || calendar == "critical" || (calendar == "auto" && data.calendar != "iso8601") {
            text.push('[');
            if calendar == "critical" { text.push('!'); }
            text.push_str("u-ca=");
            text.push_str(&data.calendar);
            text.push(']');
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
    let (other, zone, other_calendar) = std_epoch_from_value(i, std_reflect_arg(&args, 0), data.kind)?;
    // Instant.equals only ever compares epoch nanoseconds. ZonedDateTime.
    // equals additionally requires the time zone identifier AND the
    // calendar identifier to both match (CalendarEquals, JSS-calendars item
    // 2) -- confirmed by ZonedDateTime/prototype/equals' own family of
    // calendar-mismatch tests.
    Ok(JsValue::Bool(data.epoch_ns == other && (data.kind == StdEpochKind::Instant
        || (data.zone_id == zone && data.calendar == other_calendar))))
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
    // The receiver's own calendar carries forward into the Plain* result
    // (JSS-calendars item 2); PlainTime has no calendar concept, so its
    // conversion path is unaffected either way.
    std_temporal_make_cal(i, kind, std_epoch_local_fields(&data)?, data.calendar)
}
fn std_epoch_to_instant(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    std_epoch_make(i, StdEpochKind::Instant, data.epoch_ns, "UTC".into())
}
fn std_epoch_to_zoned(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::Instant))?;
    let zone = std_epoch_zone_value(i, &std_reflect_arg(&args, 0))?;
    // toZonedDateTimeISO: always iso8601, per its own name -- Instant has no
    // calendar of its own to carry forward.
    std_epoch_make(i, StdEpochKind::ZonedDateTime, data.epoch_ns, zone)
}
fn std_epoch_with_zone(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let zone = std_epoch_zone_value(i, &std_reflect_arg(&args, 0))?;
    std_epoch_make_cal(i, data.kind, data.epoch_ns, zone, data.calendar)
}
fn std_epoch_with_calendar(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    // Permissive resolution, matching every other calendar-argument site
    // except the constructor (std_temporal_resolve_calendar_id, not
    // std_temporal_resolve_calendar_id_strict) -- confirmed accepting a
    // full ISO-string-with-annotation form by withCalendar's own
    // calendar-iso-string.js.
    let calendar = std_temporal_resolve_calendar_id(i, &std_reflect_arg(&args, 0))?;
    std_epoch_make_cal(i, data.kind, data.epoch_ns, data.zone_id.clone(), calendar)
}
// PrepareTemporalFields's "string"-typed fields (monthCode, offset) read via
// ToPrimitive with a string hint, then require the result to already be a
// String -- NOT the ordinary ToString coercion used for e.g. option names,
// which would happily turn a bare number/boolean/bigint into a (wrongly
// formatted but well-typed) string and mask what must be a TypeError as a
// RangeError instead.
fn std_epoch_string_field(i: &mut Interpreter, value: JsValue) -> Result<String, Throw> {
    let primitive = if value.is_object() { i.primitive_with_hint(value, true)? } else { value };
    match primitive {
        JsValue::Str(text) => Ok(text.to_utf8_lossy()),
        _ => Err(Throw::type_error("Temporal field must be a string")),
    }
}

fn std_epoch_with(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let view_cal: &str = &data.calendar;
    let like = std_reflect_arg(&args, 0);
    if !like.is_object() { return Err(Throw::type_error("with() argument must be an object")); }
    // RejectObjectWithCalendarOrTimeZone: `calendar`/`timeZone` are always
    // read (even on a recognized Temporal object, where they're simply
    // undefined since those getters are named calendarId/timeZoneId
    // instead), then rejected if either came back defined OR the argument is
    // itself a Temporal date/time/zoned object (which carries an internal
    // calendar/time-zone concept regardless of what properties it exposes).
    let calendar = i.get_prop(&like, "calendar")?;
    let time_zone = i.get_prop(&like, "timeZone")?;
    let is_temporal_object = std_temporal_record(&like, None).is_ok_and(|d| d.kind != StdTemporalKind::Duration)
        || std_epoch_record(&like, None).is_ok();
    if !calendar.is_undefined() || !time_zone.is_undefined() || is_temporal_object {
        return Err(Throw::type_error("with() argument must be a plain property bag"));
    }
    // PrepareTemporalFields reads the bag's own fields in alphabetical
    // order, extended with `offset` (defaulting to the receiver's current
    // offset when the bag doesn't name one), and merges them onto the
    // receiver's own current local date-time.
    let mut merged = std_epoch_local_fields(&data)?;
    // Non-iso8601 calendars merge in CALENDAR-NATIVE year/month/day terms
    // (CalendarMergeFields + CalendarDateFromFields, JSS-calendars item 2),
    // not ISO ones -- cal_year/cal_month/cal_day start as the receiver's own
    // calendar-native date (std_calendar_fields for a month-structure
    // calendar; for the ISO-grid family (gregory/japanese/roc/buddhist)
    // month/day are already the ISO ones and only the year differs).
    // Without this, a supplied `day`/`month`/`monthCode` would silently be
    // interpreted as an ISO field instead, which for a month-structure
    // calendar is a different date entirely.
    let is_month_structure = std_calendar_is_month_structure(view_cal);
    let (mut cal_year, mut cal_month, mut cal_day) = if view_cal == "iso8601" {
        (merged[0], merged[1], merged[2])
    } else {
        let (y, m, d, ..) = std_calendar_fields(view_cal, merged[0], merged[1], merged[2]);
        (y, m, d)
    };
    let mut month_seen: Option<i64> = None;
    let mut month_code_text: Option<String> = None;
    let mut era_text: Option<String> = None;
    let mut era_year: Option<i64> = None;
    let mut year_seen: Option<i64> = None;
    let mut offset_field = std_epoch_offset_text(std_epoch_offset(&data));
    let mut present = false;
    // Each field's own lower bound (day/month >= 1, every time field >= 0) is
    // enforced immediately as it's read -- unconditionally, before the
    // options argument's type is even inspected (a wrong options type is a
    // TypeError, but an out-of-range field value throwing first is itself
    // observable behavior, per options-wrong-type.js). The upper bound (day
    // <= days in that month, month <= 12, hour <= 23, ...) is overflow-
    // dependent instead (clamped under "constrain", rejected under "reject")
    // and is checked below, once overflow is known -- so hour: 29 must NOT
    // fail here, only day: -1 does.
    let number_field = |i: &mut Interpreter, raw: JsValue, minimum: i64| -> Result<i64, Throw> {
        let value = std_temporal_number(i, raw, None)?;
        if value < minimum { return Err(Throw::range_error("Temporal field out of range")); }
        Ok(value)
    };
    for key in ["day", "era", "eraYear", "hour", "microsecond", "millisecond", "minute", "month", "monthCode",
        "nanosecond", "offset", "second", "year"] {
        if matches!(key, "era" | "eraYear") && !std_calendar_uses_era(view_cal) { continue; }
        let raw = i.get_prop(&like, key)?;
        if raw.is_undefined() { continue; }
        match key {
            "day" => { cal_day = number_field(i, raw, 1)?; present = true; }
            "era" => { era_text = Some(i.to_string(&raw)?.to_ascii_lowercase()); present = true; }
            "eraYear" => { era_year = Some(std_temporal_number(i, raw, None)?); present = true; }
            "hour" => { merged[3] = number_field(i, raw, 0)?; present = true; }
            "microsecond" => { merged[7] = number_field(i, raw, 0)?; present = true; }
            "millisecond" => { merged[6] = number_field(i, raw, 0)?; present = true; }
            "minute" => { merged[4] = number_field(i, raw, 0)?; present = true; }
            "month" => { let value = number_field(i, raw, 1)?; cal_month = value; month_seen = Some(value); present = true; }
            // Only the field's own type is validated here (must be a string
            // at all); resolving that string against the calendar -- and
            // rejecting month/monthCode disagreement -- is a
            // calendar-dependent step deferred to InterpretTemporalDateTimeFields,
            // after options has been fully read (options-read-before-
            // algorithmic-validation.js).
            "monthCode" => { month_code_text = Some(std_epoch_string_field(i, raw)?); present = true; }
            "nanosecond" => { merged[8] = number_field(i, raw, 0)?; present = true; }
            "offset" => { offset_field = std_epoch_string_field(i, raw)?; present = true; }
            "second" => { merged[5] = number_field(i, raw, 0)?; present = true; }
            "year" => { cal_year = std_temporal_number(i, raw, None)?; year_seen = Some(cal_year); present = true; }
            _ => unreachable!(),
        }
    }
    if !present { return Err(Throw::type_error("no Temporal fields provided")); }
    let options = std_reflect_arg(&args, 1);
    if !options.is_undefined() && !options.is_object() {
        return Err(Throw::type_error("Temporal options must be an object"));
    }
    let disambiguation = std_temporal_option_string(i, &options, "disambiguation")?.unwrap_or_else(|| "compatible".into());
    let offset_option = std_temporal_option_string(i, &options, "offset")?.unwrap_or_else(|| "prefer".into());
    let reject_overflow = std_temporal_overflow(i, options)?;
    if !matches!(disambiguation.as_str(), "compatible" | "earlier" | "later" | "reject") {
        return Err(Throw::range_error("invalid disambiguation"));
    }
    if !matches!(offset_option.as_str(), "prefer" | "use" | "ignore" | "reject") {
        return Err(Throw::range_error("invalid offset option"));
    }
    // Supplied era fields replace the receiver's year and era pair together;
    // a partial pair cannot inherit its missing half from the receiver.
    if era_text.is_some() || era_year.is_some() {
        cal_year = std_temporal_resolve_era_year(view_cal, &era_text, era_year, year_seen)?
            .ok_or_else(|| Throw::type_error("year is required"))?;
    }
    // Changing only the year preserves the receiver's monthCode, rather
    // than its ordinal position, across years with different leap months.
    if month_seen.is_none() && month_code_text.is_none() {
        month_code_text = Some(std_calendar_month_code(view_cal, merged[0], merged[1], merged[2]));
    }
    // InterpretTemporalDateTimeFields's calendar-dependent step: resolve
    // monthCode against the receiver's own calendar (iso8601 and every
    // non-leap-month calendar have no "M##L" leap-month code at all; hebrew/
    // chinese/dangi do) now that options has been read in full.
    if let Some(text) = month_code_text {
        if is_month_structure && std_calendar_has_leap_months(view_cal) {
            let (ordinal, existed) = std_calendar_resolve_code_ord(view_cal, cal_year, &text)
                .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
            if reject_overflow && !existed { return Err(Throw::range_error("leap month is absent in this year")); }
            // 2026-09-27: see the matching fix and comment at
            // std_temporal_bag_resolve_calendar's identical check
            // (builtins_temporal.rs) -- the disagreement check must not be
            // gated on `_existed`.
            if month_seen.is_some_and(|value| value != ordinal) {
                return Err(Throw::range_error("month and monthCode disagree"));
            }
            cal_month = ordinal;
        } else {
            let month = text.strip_prefix('M').and_then(|digits| digits.parse::<i64>().ok())
                .filter(|month| text.len() == 3 && *month >= 1)
                .ok_or_else(|| Throw::range_error("invalid monthCode"))?;
            if month_seen.is_some_and(|value| value != month) {
                return Err(Throw::range_error("month and monthCode disagree"));
            }
            cal_month = month;
        }
    }
    // Upper bounds are overflow-dependent: "constrain" (the default) clamps
    // each field into range, "reject" throws once any of them is out of
    // range (checked together, matching Duration/Plain*'s own with()). A
    // month-structure calendar's own month/day bounds
    // (std_calendar_months_in_year/std_calendar_month_len) replace the
    // ISO-grid ones (std_temporal_days_in_month, always 12 months); the
    // ISO-grid family (gregory/japanese/roc/buddhist) still uses
    // std_temporal_days_in_month, but against the ISO year the calendar-
    // native year maps to (std_calendar_year_to_iso), since a leap Feb 29 is
    // an ISO-calendar concept.
    let months_in_year = if is_month_structure { std_calendar_months_in_year(view_cal, cal_year) } else { 12 };
    if reject_overflow {
        let max_day = if is_month_structure { std_calendar_month_len(view_cal, cal_year, cal_month) }
            else { std_temporal_days_in_month(std_calendar_year_to_iso(view_cal, cal_year), cal_month) };
        if cal_month > months_in_year || cal_day > max_day
            || merged[3] > 23 || merged[4] > 59 || merged[5] > 59
            || merged[6..9].iter().any(|value| *value > 999) {
            return Err(Throw::range_error("Temporal field out of range"));
        }
    } else {
        cal_month = cal_month.clamp(1, months_in_year);
        let max_day = if is_month_structure { std_calendar_month_len(view_cal, cal_year, cal_month) }
            else { std_temporal_days_in_month(std_calendar_year_to_iso(view_cal, cal_year), cal_month) };
        cal_day = cal_day.clamp(1, max_day);
        merged[3] = merged[3].clamp(0, 23);
        merged[4] = merged[4].clamp(0, 59);
        merged[5] = merged[5].clamp(0, 59);
        for field in &mut merged[6..9] { *field = (*field).clamp(0, 999); }
    }
    (merged[0], merged[1], merged[2]) = if is_month_structure {
        std_calendar_to_iso(view_cal, cal_year, cal_month, cal_day)
    } else {
        (std_calendar_year_to_iso(view_cal, cal_year), cal_month, cal_day)
    };
    std_temporal_validate(StdTemporalKind::PlainDateTime, &merged)?;
    let local_ns = std_temporal_epoch_ns(&merged);
    let offset_ns = std_epoch_parse_offset(&offset_field)?;
    let epoch_ns = if offset_option == "ignore" {
        std_epoch_disambiguate(&data.zone_id, local_ns, &disambiguation)?
    } else {
        match offset_option.as_str() {
            "use" => local_ns - offset_ns,
            "reject" => tz_possible_epoch_ns(&data.zone_id, local_ns).into_iter()
                .find(|&candidate| tz_offset_ns(&data.zone_id, candidate) == offset_ns)
                .ok_or_else(|| Throw::range_error("offset does not match time zone"))?,
            _ => {
                // "prefer" (the default): keep the given offset if it names
                // one of the local time's actual possible offsets (resolving
                // an ambiguous local time in its favor), else fall back to
                // ordinary disambiguation.
                match tz_possible_epoch_ns(&data.zone_id, local_ns).into_iter()
                    .find(|&candidate| tz_offset_ns(&data.zone_id, candidate) == offset_ns) {
                    Some(candidate) => candidate,
                    None => std_epoch_disambiguate(&data.zone_id, local_ns, &disambiguation)?,
                }
            }
        }
    };
    std_epoch_make_cal(i, data.kind, epoch_ns, data.zone_id, data.calendar)
}
fn std_epoch_with_plain_time(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let mut fields = std_epoch_local_fields(&data)?;
    let time = std_reflect_arg(&args, 0);
    if time.is_undefined() { fields[3..9].fill(0); }
    else {
        let parts = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time, false)?.0;
        fields[3..9].copy_from_slice(&parts[3..9]);
    }
    let epoch_ns = std_epoch_disambiguate(&data.zone_id, std_temporal_epoch_ns(&fields), "compatible")?;
    std_epoch_make_cal(i, data.kind, epoch_ns, data.zone_id, data.calendar)
}
fn std_epoch_transition(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let option = std_reflect_arg(&args, 0);
    let direction = if let JsValue::Str(value) = &option { value.to_utf8_lossy() }
        else { std_temporal_option_string(i, &option, "direction")?
            .ok_or_else(|| Throw::range_error("direction is required"))? };
    if !matches!(direction.as_str(), "next" | "previous") {
        return Err(Throw::range_error("invalid direction"));
    }
    let transition = if matches!(data.zone_id.as_str(), "UTC" | "Etc/UTC" | "Etc/GMT") {
        None
    } else if direction == "next" { tz_next_transition(&data.zone_id, data.epoch_ns) }
        else { tz_previous_transition(&data.zone_id, data.epoch_ns) };
    match transition {
        Some(epoch_ns) => std_epoch_make_cal(i, data.kind, epoch_ns, data.zone_id, data.calendar),
        None => Ok(JsValue::Null),
    }
}
fn std_epoch_start_of_day(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, Some(StdEpochKind::ZonedDateTime))?;
    let fields = std_epoch_local_fields(&data)?;
    let epoch_ns = std_epoch_start_of_day_ns(&data.zone_id, &fields)?;
    std_epoch_make_cal(i, data.kind, epoch_ns, data.zone_id, data.calendar)
}

fn std_temporal_plain_to_zoned(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_temporal_record(&this, None)?;
    let input = std_reflect_arg(&args, 0);
    // PlainDate.prototype.toZonedDateTime(item): item may be a bare
    // TimeZoneLike value (string or ZonedDateTime instance) directly, OR an
    // object with a `timeZone` property (plus an optional `plainTime`). Per
    // spec, an object whose `timeZone` property is undefined falls back to
    // treating the WHOLE object as the TimeZoneLike value itself (rather
    // than throwing) -- confirmed by basic.js/timezone-string.js/
    // timezone-case-insensitive.js/timezone-wrong-type.js/
    // no-observable-array-iteration.js/get-start-of-day-throws.js, all of
    // which pass a bare string (or, for the object form, a `{timeZone,
    // plainTime}` bag). PlainDateTime.prototype.toZonedDateTime(timeZoneLike,
    // options) always takes the bare form plus a separate disambiguation
    // options bag.
    let (zone_value, time_value) = if data.kind == StdTemporalKind::PlainDate {
        if input.is_object() {
            let tz_like = i.get_prop(&input, "timeZone")?;
            if tz_like.is_undefined() {
                (input.clone(), JsValue::Undefined)
            } else {
                (tz_like, i.get_prop(&input, "plainTime")?)
            }
        } else {
            (input.clone(), JsValue::Undefined)
        }
    } else { (input.clone(), JsValue::Undefined) };
    let zone = std_epoch_zone_value(i, &zone_value)?;
    let mut fields = data.fields;
    let has_time = data.kind == StdTemporalKind::PlainDate && !time_value.is_undefined();
    if has_time {
        let parsed = std_temporal_from_fields(i, StdTemporalKind::PlainTime, time_value, false)?.0;
        fields[3..9].copy_from_slice(&parsed[3..9]);
        // ISODateTimeWithinLimits is checked on the COMBINED date+time (the
        // date alone, or the time alone, may each be within their own
        // narrower limits without the combination being valid) --
        // confirmed by PlainDate/prototype/toZonedDateTime's own family of
        // `*-outside-valid-iso-range.js`-style tests elsewhere in this
        // ticket (PlainDateTime.with/withPlainTime).
        std_temporal_validate(StdTemporalKind::PlainDateTime, &fields)?;
    }
    let options = if data.kind == StdTemporalKind::PlainDate { JsValue::Undefined }
        else { std_reflect_arg(&args, 1) };
    let disambiguation = std_temporal_option_string(i, &options, "disambiguation")?
        .unwrap_or_else(|| "compatible".into());
    let epoch_ns = std_epoch_disambiguate(&zone, std_temporal_epoch_ns(&fields), &disambiguation)?;
    // The source PlainDate/PlainDateTime's own calendar carries forward
    // (JSS-calendars item 2), matching ToTemporalZonedDateTime's use of the
    // receiver's already-resolved calendar.
    std_epoch_make_cal(i, StdEpochKind::ZonedDateTime, epoch_ns, zone, data.calendar)
}

fn std_temporal_now_instant(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_make(i, StdEpochKind::Instant, std_temporal_epoch_ns(&std_temporal_now_fields()), "UTC".into())
}
fn std_temporal_now_zoned(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let zone_value = std_reflect_arg(&args, 0);
    let zone = if zone_value.is_undefined() { "UTC".into() } else { std_epoch_zone_value(i, &zone_value)? };
    std_epoch_make(i, StdEpochKind::ZonedDateTime, std_temporal_epoch_ns(&std_temporal_now_fields()), zone)
}

fn std_epoch_add_values(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, sign: i64) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let duration = std_temporal_from_fields(i, StdTemporalKind::Duration, std_reflect_arg(&args, 0), false)?.0;
    if data.kind == StdEpochKind::Instant && duration[..4].iter().any(|part| *part != 0) {
        return Err(Throw::range_error("date units are not valid for Instant"));
    }
    let result = if data.kind == StdEpochKind::Instant {
        // Temporal.Instant.prototype.add/subtract take no options parameter
        // at all (AddDurationToOrSubtractDurationFromInstant): args[1], if
        // any, is simply never read.
        data.epoch_ns + sign as i128 * std_temporal_duration_time_ns(&duration)
    } else {
        // 2026-09-27: this used to hardcode `reject_overflow: false`
        // (constrain), silently ignoring a real `overflow` option in
        // args[1] -- AddZonedDateTime reads GetTemporalOverflowOption just
        // like every Plain* kind's own add()/subtract() does. Fixed by
        // reading it the same way (std_temporal_overflow), confirmed
        // required by ZonedDateTime/prototype/{add,subtract}/constrain-day-
        // */leap-year-*.js's own `{ overflow: "reject" }` cases across every
        // calendar (including the ISO-grid family -- this was never
        // calendar-specific).
        let reject = std_temporal_overflow(i, std_reflect_arg(&args, 1))?;
        let local = std_epoch_local_fields(&data)?;
        // (2026-09-27: confirmed required, not parkable -- add()/subtract()
        // must stay calendar-native here to match the calendar-real getter
        // views above for month-structure calendars, including chinese/
        // dangi (narrowing just them to ISO here was tried and reverted --
        // see the getter comment above); only since()/until() (std_temporal_
        // epoch_breakdown_cal, below) is parked, since its bracket search
        // has an unresolved bug for chinese/dangi.)
        let shifted = std_temporal_add_calendar_cal(&data.calendar, &local, &duration, sign, reject)?;
        let time_ns = std_temporal_duration_time_ns(&duration)
            - (duration[2] as i128 * 7 + duration[3] as i128) * STD_TEMPORAL_NS_DAY;
        std_epoch_disambiguate(&data.zone_id, std_temporal_epoch_ns(&shifted), "compatible")?
            + sign as i128 * time_ns
    };
    std_epoch_make_cal(i, data.kind, result, data.zone_id.clone(), data.calendar.clone())
}
fn std_epoch_add(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_add_values(i, this, args, 1)
}
fn std_epoch_subtract(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_add_values(i, this, args, -1)
}
fn std_epoch_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, since: bool) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let (other, _, other_calendar) = std_epoch_from_value(i, std_reflect_arg(&args, 0), data.kind)?;
    // CalendarEquals (spec): since/until require the receiver and argument
    // to share a calendar -- mirrors std_temporal_difference's identical
    // check for PlainDate/PlainDateTime/PlainYearMonth/PlainMonthDay
    // (JSS-calendars item 1). Instant has no calendar concept at all
    // (its `other_calendar` is always the "iso8601" placeholder), so this
    // is a no-op for it.
    if data.kind == StdEpochKind::ZonedDateTime && data.calendar != other_calendar {
        return Err(Throw::range_error("since/until require the same calendar"));
    }
    let options = std_reflect_arg(&args, 1);
    // All four option properties (largestUnit, roundingIncrement,
    // roundingMode, smallestUnit) are read here, in that spec order, before
    // any default-resolution or cross-field validation below is allowed to
    // throw -- see `std_temporal_difference_reads` (builtins_temporal_
    // arithmetic.rs), shared with PlainDate/PlainDateTime's since/until.
    // `since_for_fixed_units` distinguishes the two fixed-unit paths below
    // (Instant always, ZonedDateTime's non-calendar largestUnit) from the
    // calendar-anchored one: the fixed-unit paths compute the signed delta
    // directly in the caller's requested direction and round it as-is (no
    // mode swap, no final negation -- `since_for_fixed_units` is passed
    // `false` to `std_temporal_difference_reads` so the mode comes back
    // un-swapped), while the calendar path always anchors at the receiver
    // and swaps+negates like PlainDate/PlainDateTime (`since` passed
    // through), because a calendar unit's length depends on which side
    // anchors it.
    let (largest_raw, increment, mode_for_fixed, smallest_raw) =
        std_temporal_difference_reads(i, &options, "trunc", false)?;
    let smallest = smallest_raw.unwrap_or_else(|| "nanosecond".to_string());
    // The default largestUnit is the coarser of the kind's own default and the
    // (possibly explicit, possibly coarser) smallestUnit -- e.g. smallestUnit:
    // "years" with no largestUnit given must default largestUnit to "years"
    // too, not to the kind's usual "hour"/"second".
    let kind_default = if data.kind == StdEpochKind::Instant { "second" } else { "hour" };
    let default = std_temporal_larger_unit(kind_default, &smallest);
    let largest = std_temporal_resolve_largest(&largest_raw, &default);
    if std_temporal_unit_rank(&largest).zip(std_temporal_unit_rank(&smallest))
        .is_some_and(|(l, s)| l > s) {
        return Err(Throw::range_error("largestUnit is smaller than smallestUnit"));
    }
    std_temporal_validate_rounding_increment(&smallest, increment)?;
    if data.kind == StdEpochKind::ZonedDateTime && matches!(largest.as_str(), "year" | "month" | "week") {
        // Rounds relative to the receiver: the calendar bracket search is
        // always anchored at `first` (the receiver's own local fields), for
        // both since and until; since negates the whole result at the end.
        // Unlike the fixed-unit paths below, this needs the directional
        // roundingMode swap for `since` (ceil/floor, halfCeil/halfFloor),
        // matching PlainDate/PlainDateTime's calendar branch -- confirmed by
        // ZonedDateTime's own `since`/`until` roundingmode-ceil/floor tests,
        // which were previously wrong here because this branch reused the
        // fixed-unit direct-delta model's un-swapped mode.
        let mode = if since { std_temporal_swap_directional_mode(&mode_for_fixed) } else { mode_for_fixed };
        let first = std_epoch_local_fields(&data)?;
        let mut second = [0; 10];
        std_temporal_set_epoch_ns(&mut second, other + tz_offset_ns(&data.zone_id, other))?;
        let target_ns = std_temporal_epoch_ns(&second);
        let smallest_rank = std_temporal_unit_rank(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?;
        let mut fields = if smallest_rank <= 2 {
            std_temporal_round_to_calendar_unit(&first, target_ns, &smallest, &largest, increment, &mode)?
        } else {
            let total_ns = target_ns - std_temporal_epoch_ns(&first);
            let step = std_temporal_unit_ns(&smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))?
                * increment as i128;
            let rounded_ns = std_temporal_round_integer(total_ns, step, &mode)?;
            std_temporal_epoch_breakdown_cal(&data.calendar, &first, std_temporal_epoch_ns(&first) + rounded_ns, &largest)?
        };
        std_temporal_validate(StdTemporalKind::Duration, &fields)?;
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
    if data.kind == StdEpochKind::Instant
        && !matches!(smallest.as_str(), "hour" | "minute" | "second" | "millisecond" | "microsecond" | "nanosecond") {
        return Err(Throw::range_error("invalid smallestUnit"));
    }
    let delta = if since { data.epoch_ns - other } else { other - data.epoch_ns };
    let rounded = std_temporal_round_integer(delta, step * increment as i128, &mode_for_fixed)?;
    std_temporal_make(i, StdTemporalKind::Duration, std_temporal_split_duration_ns(rounded, index))
}
fn std_epoch_until(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_difference(i, this, args, false)
}
fn std_epoch_since(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_epoch_difference(i, this, args, true)
}
// Epoch nanoseconds of local midnight for `date` (only fields 0..2 are used)
// in `zone_id`, i.e. GetStartOfDay. Disambiguates a nonexistent local
// midnight (a DST "spring forward" that skips it) the same way the
// constructor and `startOfDay` do, and enforces the representable Instant
// range itself, since callers reuse this both for `startOfDay` and for
// `round`'s day-bracket search, where landing one day past the maximum (or
// before the minimum) representable date must throw here rather than
// producing a silently out-of-range value.
fn std_epoch_start_of_day_ns(zone_id: &str, date: &[i64; 10]) -> Result<i128, Throw> {
    let mut fields = *date;
    fields[3..9].fill(0);
    let epoch_ns = std_epoch_disambiguate(zone_id, std_temporal_epoch_ns(&fields), "compatible")?;
    if !(-8_640_000_000_000_000_000_000i128..=8_640_000_000_000_000_000_000i128).contains(&epoch_ns) {
        return Err(Throw::range_error("start of day outside Temporal range"));
    }
    Ok(epoch_ns)
}

fn std_epoch_round(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = std_epoch_record(&this, None)?;
    let option = std_reflect_arg(&args, 0);
    if option.is_undefined() { return Err(Throw::type_error("round options are required")); }
    // Every option value is read and coerced before any algorithmic
    // validation of it runs, in this exact order: roundingIncrement (ToNumber
    // via GetOption, ToTemporalRoundingIncrement's own finite/safe-integer
    // check happening as part of that same read), roundingMode (ToString),
    // then smallestUnit (ToString, required). A bare string short-circuits to
    // just smallestUnit, matching Duration.prototype.round's own options.
    let (increment, mode, unit_raw) = if let JsValue::Str(value) = &option {
        (1i64, "halfExpand".to_string(), value.to_utf8_lossy())
    } else {
        if !option.is_object() { return Err(Throw::type_error("Temporal options must be an object")); }
        let increment_value = i.get_prop(&option, "roundingIncrement")?;
        let increment = std_temporal_number(i, increment_value, Some(1))?;
        let mode_value = i.get_prop(&option, "roundingMode")?;
        let mode = if mode_value.is_undefined() { "halfExpand".to_string() } else { i.to_string(&mode_value)? };
        let unit_value = i.get_prop(&option, "smallestUnit")?;
        if unit_value.is_undefined() { return Err(Throw::range_error("smallestUnit is required")); }
        (increment, mode, i.to_string(&unit_value)?)
    };
    // Algorithmic validation, deferred until every option has been read:
    // smallestUnit must be one of day..nanosecond (year/month/week are valid
    // Duration units but not valid here), Instant additionally excludes day
    // (it has no zone to resolve a calendar day against), roundingMode must
    // be a recognized name, and roundingIncrement must be in range.
    let unit = std_temporal_singular_unit(&unit_raw);
    if !matches!(unit.as_str(), "day" | "hour" | "minute" | "second" | "millisecond" | "microsecond" | "nanosecond") {
        return Err(Throw::range_error("invalid smallestUnit"));
    }
    if data.kind == StdEpochKind::Instant && unit == "day" {
        return Err(Throw::range_error("invalid smallestUnit"));
    }
    if !std_temporal_round_mode_valid(&mode) { return Err(Throw::range_error("invalid roundingMode")); }
    if !(1..=1_000_000_000).contains(&increment) {
        return Err(Throw::range_error("invalid roundingIncrement"));
    }
    if data.kind == StdEpochKind::Instant {
        // Instant has no local calendar day at all -- a day is always
        // exactly 86,400 SI seconds, so (unlike ZonedDateTime, where an hour
        // increment of 24 might not land on an actual local day boundary
        // under DST) any increment that evenly divides a full day is valid,
        // up to and including the full day itself (hour:24, minute:1440,
        // second:86400, ... are all accepted).
        let step = std_temporal_unit_ns(&unit).unwrap() * increment as i128;
        if STD_TEMPORAL_NS_DAY % step != 0 {
            return Err(Throw::range_error("invalid roundingIncrement"));
        }
        let rounded = std_temporal_round_integer(data.epoch_ns, step, &mode)?;
        return std_epoch_make_cal(i, data.kind, rounded, data.zone_id.clone(), data.calendar.clone());
    }
    if unit == "day" {
        if increment != 1 { return Err(Throw::range_error("invalid roundingIncrement")); }
    } else {
        let maximum = match unit.as_str() {
            "hour" => 24, "minute" | "second" => 60,
            _ => 1_000,
        };
        if increment >= maximum || maximum % increment != 0 {
            return Err(Throw::range_error("invalid roundingIncrement"));
        }
    }
    // ZonedDateTime: round the LOCAL wall-clock date-time (always a
    // non-negative time-of-day, regardless of how far in the past/future the
    // overall instant is -- rounding the raw, possibly enormous-negative
    // epoch nanoseconds directly, as Instant.round does, gets the boundary
    // wrong), then reinterpret the rounded local date-time back into an
    // instant the same way the constructor would.
    let local = std_epoch_local_fields(&data)?;
    let epoch_ns = if unit == "day" {
        // Variable-length local day (DST can make it shorter/longer than 24h):
        // bracket epoch_ns between the start of the current local day and the
        // start of the next, and round its position within that bracket.
        let start_ns = std_epoch_start_of_day_ns(&data.zone_id, &local)?;
        let next_day = std_temporal_epoch_day(local[0], local[1], local[2]) + 1;
        let mut next_date = local;
        (next_date[0], next_date[1], next_date[2]) = std_temporal_civil_from_epoch_day(next_day);
        let end_ns = std_epoch_start_of_day_ns(&data.zone_id, &next_date)?;
        let day_length_ns = end_ns - start_ns;
        if day_length_ns <= 0 { return Err(Throw::range_error("invalid Temporal day length")); }
        let offset_in_day = data.epoch_ns - start_ns;
        let rounded = std_temporal_round_integer(offset_in_day, day_length_ns, &mode)?;
        start_ns + rounded
    } else {
        let step = std_temporal_unit_ns(&unit).unwrap() * increment as i128;
        let time_ns = std_temporal_time_ns(&local);
        let rounded_time = std_temporal_round_integer(time_ns, step, &mode)?;
        let mut new_local = local;
        let day_delta = rounded_time.div_euclid(STD_TEMPORAL_NS_DAY);
        if day_delta != 0 {
            let day = std_temporal_epoch_day(local[0], local[1], local[2]) + day_delta as i64;
            (new_local[0], new_local[1], new_local[2]) = std_temporal_civil_from_epoch_day(day);
        }
        std_temporal_set_time_ns(&mut new_local, rounded_time);
        std_epoch_disambiguate(&data.zone_id, std_temporal_epoch_ns(&new_local), "compatible")?
    };
    std_epoch_make_cal(i, data.kind, epoch_ns, data.zone_id.clone(), data.calendar.clone())
}
