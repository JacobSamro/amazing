// ECMA-262 Date: full constructor, statics, prototype accessors and setters,
// ISO 8601 and legacy parsing, string formats, and Annex B extensions.
//
// The internal [[DateValue]] slot is stored as a non-enumerable own property
// keyed "[[DateValue]]", matching the boxed-primitive convention already used
// by [[NumberData]]/[[StringData]]. Local time is the engine's local zone,
// which is UTC (see date_local_tza_ms); Test262 runs with UTC local time.

const DATE_MS_PER_DAY: f64 = 86_400_000.0;
const DATE_MS_PER_HOUR: f64 = 3_600_000.0;
const DATE_MS_PER_MINUTE: f64 = 60_000.0;
const DATE_MS_PER_SECOND: f64 = 1_000.0;
const DATE_MAX_TIME: f64 = 8_640_000_000_000_000.0;
const DATE_SLOT: &str = "[[DateValue]]";

const DATE_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const DATE_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

// ---------------------------------------------------------------------------
// [[DateValue]] storage
// ---------------------------------------------------------------------------

fn date_this_value(this: &JsValue) -> Result<f64, Throw> {
    if let JsValue::Object(object) = this {
        if let Some(JsValue::Num(value)) = object.borrow().get_own(DATE_SLOT) {
            return Ok(value);
        }
    }
    Err(Throw::type_error("this is not a Date object"))
}

fn date_store(object: &Obj, value: f64) -> f64 {
    let _ = object
        .borrow_mut()
        .define(DATE_SLOT, symbol_data(JsValue::Num(value), true, true));
    value
}

// ---------------------------------------------------------------------------
// Time value arithmetic (ECMA-262 §21.4.1)
// ---------------------------------------------------------------------------

fn date_to_integer(value: f64) -> f64 {
    if value.is_nan() {
        f64::NAN
    } else {
        value.trunc()
    }
}

fn date_time_clip(time: f64) -> f64 {
    if !time.is_finite() || time.abs() > DATE_MAX_TIME {
        return f64::NAN;
    }
    let clipped = time.trunc();
    if clipped == 0.0 {
        0.0
    } else {
        clipped
    }
}

fn date_day(t: f64) -> f64 {
    (t / DATE_MS_PER_DAY).floor()
}

fn date_time_within_day(t: f64) -> f64 {
    t - date_day(t) * DATE_MS_PER_DAY
}

fn date_hour(t: f64) -> f64 {
    ((t / DATE_MS_PER_HOUR).floor()).rem_euclid(24.0) + 0.0
}

fn date_minutes(t: f64) -> f64 {
    ((t / DATE_MS_PER_MINUTE).floor()).rem_euclid(60.0) + 0.0
}

fn date_seconds(t: f64) -> f64 {
    ((t / DATE_MS_PER_SECOND).floor()).rem_euclid(60.0) + 0.0
}

fn date_ms_from_time(t: f64) -> f64 {
    t.rem_euclid(DATE_MS_PER_SECOND) + 0.0
}

fn date_week_day(t: f64) -> f64 {
    let days = date_day(t) as i64;
    (days + 4).rem_euclid(7) as f64
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant).
fn date_days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `date_days_from_civil`.
fn date_civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (YearFromTime, MonthFromTime (0-based), DateFromTime) of a time value.
fn date_ymd(t: f64) -> (f64, f64, f64) {
    let (y, m, d) = date_civil_from_days(date_day(t) as i64);
    (y as f64, (m - 1) as f64, d as f64)
}

fn date_year_from_time(t: f64) -> f64 {
    date_ymd(t).0
}

fn date_month_from_time(t: f64) -> f64 {
    date_ymd(t).1
}

fn date_date_from_time(t: f64) -> f64 {
    date_ymd(t).2
}

/// MakeTime: `(h * msPerHour + m * msPerMinute) + s * msPerSecond + milli`,
/// keeping JavaScript's left-to-right IEEE-754 association.
fn date_make_time(hour: f64, min: f64, sec: f64, ms: f64) -> f64 {
    if !hour.is_finite() || !min.is_finite() || !sec.is_finite() || !ms.is_finite() {
        return f64::NAN;
    }
    let h = date_to_integer(hour);
    let m = date_to_integer(min);
    let s = date_to_integer(sec);
    let milli = date_to_integer(ms);
    h * DATE_MS_PER_HOUR + m * DATE_MS_PER_MINUTE + s * DATE_MS_PER_SECOND + milli
}

fn date_make_day(year: f64, month: f64, date: f64) -> f64 {
    if !year.is_finite() || !month.is_finite() || !date.is_finite() {
        return f64::NAN;
    }
    let y = date_to_integer(year);
    let m = date_to_integer(month);
    let dt = date_to_integer(date);
    let ym = y + (m / 12.0).floor();
    let mn = m.rem_euclid(12.0);
    if !ym.is_finite() || !(-271_821.0..=275_760.0).contains(&ym) {
        return f64::NAN;
    }
    let days = date_days_from_civil(ym as i64, mn as i64 + 1, 1);
    days as f64 + dt - 1.0
}

fn date_make_date(day: f64, time: f64) -> f64 {
    if !day.is_finite() || !time.is_finite() {
        return f64::NAN;
    }
    day * DATE_MS_PER_DAY + time
}

// The engine's local zone is UTC; the Intl default time zone is UTC and the
// Test262 host runs with UTC local time. Routing through tz_offset_ns keeps the
// seam explicit for a future host-provided zone.
fn date_local_tza_ms(t: f64) -> f64 {
    if t.is_nan() {
        return 0.0;
    }
    let ns = (t * 1_000_000.0) as i128;
    (tz_offset_ns("UTC", ns) / 1_000_000) as f64
}

fn date_local_time(t: f64) -> f64 {
    if t.is_nan() {
        t
    } else {
        t + date_local_tza_ms(t)
    }
}

fn date_utc_time(t: f64) -> f64 {
    if t.is_nan() {
        t
    } else {
        t - date_local_tza_ms(t)
    }
}

fn date_now_ms() -> f64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as f64,
        Err(_) => 0.0,
    }
}

// ---------------------------------------------------------------------------
// String formats
// ---------------------------------------------------------------------------

fn date_format_year(year: i64) -> String {
    if year < 0 {
        format!("-{:04}", year.unsigned_abs())
    } else if year > 9_999 {
        format!("{year}")
    } else {
        format!("{year:04}")
    }
}

fn date_format_iso_year(year: i64) -> String {
    if (0..=9_999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", year.unsigned_abs())
    } else {
        format!("+{year:06}")
    }
}

fn date_date_string(t: f64) -> String {
    let (y, m, d) = date_ymd(t);
    let weekday = date_week_day(t) as usize;
    format!(
        "{} {} {:02} {}",
        DATE_WEEKDAYS[weekday],
        DATE_MONTHS[m as usize],
        d as i64,
        date_format_year(y as i64)
    )
}

fn date_utc_date_string(t: f64) -> String {
    let (y, m, d) = date_ymd(t);
    let weekday = date_week_day(t) as usize;
    format!(
        "{}, {:02} {} {}",
        DATE_WEEKDAYS[weekday],
        d as i64,
        DATE_MONTHS[m as usize],
        date_format_year(y as i64)
    )
}

fn date_time_string(t: f64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        date_hour(t) as i64,
        date_minutes(t) as i64,
        date_seconds(t) as i64
    )
}

fn date_time_zone_string(t: f64) -> String {
    let offset = date_local_tza_ms(t);
    let sign = if offset < 0.0 { '-' } else { '+' };
    let magnitude = offset.abs();
    let hours = (magnitude / DATE_MS_PER_HOUR).floor() as i64;
    let minutes = ((magnitude / DATE_MS_PER_MINUTE).floor() as i64) % 60;
    format!("GMT{sign}{hours:02}{minutes:02} (Coordinated Universal Time)")
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn date_parse_digits(bytes: &[u8], pos: &mut usize, count: usize) -> Option<i64> {
    if *pos + count > bytes.len() {
        return None;
    }
    let mut value = 0i64;
    for offset in 0..count {
        let byte = bytes[*pos + offset];
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (byte - b'0') as i64;
    }
    *pos += count;
    Some(value)
}

/// The ECMAScript Date Time String Format. Returns `None` for non-ISO text.
fn date_parse_iso(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut pos = 0usize;
    let year = match bytes[0] {
        b'+' | b'-' => {
            let negative = bytes[0] == b'-';
            pos += 1;
            let value = date_parse_digits(bytes, &mut pos, 6)?;
            if negative && value == 0 {
                return None;
            }
            if negative {
                -value
            } else {
                value
            }
        }
        _ => date_parse_digits(bytes, &mut pos, 4)?,
    };
    let mut month = 1i64;
    let mut day = 1i64;
    let mut hour = 0i64;
    let mut minute = 0i64;
    let mut second = 0i64;
    let mut milli = 0i64;
    let mut offset_minutes: Option<i64> = None;
    let mut has_time = false;

    if pos < bytes.len() {
        if bytes[pos] != b'-' {
            return None;
        }
        pos += 1;
        month = date_parse_digits(bytes, &mut pos, 2)?;
        if !(1..=12).contains(&month) {
            return None;
        }
        if pos < bytes.len() {
            if bytes[pos] != b'-' {
                return None;
            }
            pos += 1;
            day = date_parse_digits(bytes, &mut pos, 2)?;
            if !(1..=31).contains(&day) {
                return None;
            }
        }
    }
    if pos < bytes.len() {
        if bytes[pos] != b'T' && bytes[pos] != b't' {
            return None;
        }
        has_time = true;
        pos += 1;
        hour = date_parse_digits(bytes, &mut pos, 2)?;
        if hour > 24 {
            return None;
        }
        if pos >= bytes.len() || bytes[pos] != b':' {
            return None;
        }
        pos += 1;
        minute = date_parse_digits(bytes, &mut pos, 2)?;
        if minute > 59 {
            return None;
        }
        if pos < bytes.len() && bytes[pos] == b':' {
            pos += 1;
            second = date_parse_digits(bytes, &mut pos, 2)?;
            if second > 59 {
                return None;
            }
            if pos < bytes.len() && bytes[pos] == b'.' {
                pos += 1;
                let start = pos;
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                let digits = pos - start;
                if digits == 0 || digits > 9 {
                    return None;
                }
                let mut value = 0i64;
                for index in 0..3 {
                    let digit = if index < digits {
                        (bytes[start + index] - b'0') as i64
                    } else {
                        0
                    };
                    value = value * 10 + digit;
                }
                milli = value;
            }
        }
        if pos < bytes.len() {
            match bytes[pos] {
                b'Z' | b'z' => {
                    pos += 1;
                    offset_minutes = Some(0);
                }
                b'+' | b'-' => {
                    let sign = if bytes[pos] == b'-' { -1 } else { 1 };
                    pos += 1;
                    let hours = date_parse_digits(bytes, &mut pos, 2)?;
                    if hours > 23 {
                        return None;
                    }
                    let minutes = if pos < bytes.len() && bytes[pos] == b':' {
                        pos += 1;
                        let minutes = date_parse_digits(bytes, &mut pos, 2)?;
                        if minutes > 59 {
                            return None;
                        }
                        minutes
                    } else if pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        let minutes = date_parse_digits(bytes, &mut pos, 2)?;
                        if minutes > 59 {
                            return None;
                        }
                        minutes
                    } else {
                        // Date Time String Format offsets are ±HH:mm; hours alone are invalid.
                        return None;
                    };
                    offset_minutes = Some(sign * (hours * 60 + minutes));
                }
                _ => return None,
            }
        }
    }
    if pos != bytes.len() {
        return None;
    }

    let day_value = date_make_day(year as f64, (month - 1) as f64, day as f64);
    let time_value = date_make_time(hour as f64, minute as f64, second as f64, milli as f64);
    let mut t = date_make_date(day_value, time_value);
    if !t.is_finite() {
        return None;
    }
    // Date-only forms are UTC; date-time forms without an offset are local
    // (UTC in this engine). An explicit offset is subtracted.
    if let Some(offset) = offset_minutes {
        t -= offset as f64 * DATE_MS_PER_MINUTE;
    } else if has_time {
        t = date_utc_time(t);
    }
    Some(date_time_clip(t))
}

fn date_month_index(token: &str) -> Option<i64> {
    if token.len() < 3 {
        return None;
    }
    let lower = token[..3].to_ascii_lowercase();
    DATE_MONTHS
        .iter()
        .position(|month| month.to_ascii_lowercase() == lower)
        .map(|index| index as i64)
}

fn date_is_weekday(token: &str) -> bool {
    if token.len() < 3 {
        return false;
    }
    let lower = token[..3].to_ascii_lowercase();
    DATE_WEEKDAYS
        .iter()
        .any(|day| day.to_ascii_lowercase() == lower)
}

fn date_parse_time_token(token: &str) -> Option<(i64, i64, i64, i64)> {
    let parts: Vec<&str> = token.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let hour: i64 = parts[0].parse().ok()?;
    let minute: i64 = parts[1].parse().ok()?;
    let (second, milli) = if parts.len() == 3 {
        let seconds = parts[2];
        match seconds.find('.') {
            Some(dot) => {
                let whole: i64 = seconds[..dot].parse().ok()?;
                let fraction = &seconds[dot + 1..];
                if fraction.is_empty() || fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let mut value = 0i64;
                for index in 0..3 {
                    let digit = if index < fraction.len() {
                        (fraction.as_bytes()[index] - b'0') as i64
                    } else {
                        0
                    };
                    value = value * 10 + digit;
                }
                (whole, value)
            }
            None => (seconds.parse().ok()?, 0),
        }
    } else {
        (0, 0)
    };
    Some((hour, minute, second, milli))
}

fn date_parse_offset(text: &str) -> Option<i64> {
    let (sign, body) = if let Some(rest) = text.strip_prefix('+') {
        (1i64, rest)
    } else if let Some(rest) = text.strip_prefix('-') {
        (-1i64, rest)
    } else {
        return None;
    };
    let digits: String = body.chars().filter(|c| c.is_ascii_digit()).collect();
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i64>().ok()?, 0),
        4 => (
            digits[..2].parse::<i64>().ok()?,
            digits[2..].parse::<i64>().ok()?,
        ),
        _ => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

/// Legacy formats: `Www Mmm DD YYYY HH:MM:SS GMT±HHMM (Zone)` and the
/// `toUTCString` form. Only the shapes this engine emits are needed by the
/// corpus, so the scanner is deliberately permissive.
fn date_parse_legacy(text: &str) -> Option<f64> {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            ',' | '(' | ')' => ' ',
            other => other,
        })
        .collect();
    let mut month: Option<i64> = None;
    let mut day: Option<i64> = None;
    let mut year: Option<i64> = None;
    let mut hour = 0i64;
    let mut minute = 0i64;
    let mut second = 0i64;
    let mut milli = 0i64;
    let mut offset = 0i64;
    let mut numbers: Vec<(i64, usize)> = Vec::new();
    let mut slashed = false;

    for token in cleaned.split_whitespace() {
        if token.eq_ignore_ascii_case("GMT") || token.eq_ignore_ascii_case("UTC") {
            continue;
        }
        if let Some(index) = date_month_index(token) {
            month = Some(index);
            continue;
        }
        if date_is_weekday(token) {
            continue;
        }
        if let Some((h, m, s, ms)) = date_parse_time_token(token) {
            hour = h;
            minute = m;
            second = s;
            milli = ms;
            continue;
        }
        if token.len() > 3
            && (token[..3].eq_ignore_ascii_case("GMT") || token[..3].eq_ignore_ascii_case("UTC"))
        {
            offset = date_parse_offset(&token[3..])?;
            continue;
        }
        if (token.starts_with('+') || token.starts_with('-'))
            && token[1..].bytes().all(|b| b.is_ascii_digit())
        {
            // A signed all-digit token is a negative/extended year unless the
            // year was already seen, in which case it is a bare UTC offset.
            if year.is_none() && !token[1..].is_empty() {
                year = token.parse::<i64>().ok();
            } else {
                offset = date_parse_offset(token)?;
            }
            continue;
        }
        if token.contains('/') {
            // M/D/Y, or Y/M/D when the first field cannot be a month or day (> 31).
            let fields: Vec<&str> = token.split('/').collect();
            if fields.len() != 3 || !fields.iter().all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit())) {
                return None;
            }
            let values: Vec<i64> = fields.iter().map(|f| f.parse().ok()).collect::<Option<_>>()?;
            let (m, d, y) = if values[0] > 31 || fields[0].len() >= 3 {
                (values[1], values[2], date_legacy_year(values[0], fields[0].len()))
            } else {
                (values[0], values[1], date_legacy_year(values[2], fields[2].len()))
            };
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return None; }
            month = Some(m - 1);
            day = Some(d);
            year = Some(y);
            slashed = true;
            continue;
        }
        if token.bytes().all(|b| b.is_ascii_digit()) {
            numbers.push((token.parse().ok()?, token.len()));
            continue;
        }
        // Zone name words ("Coordinated", "Universal", "Time", ...) are ignored.
    }
    if !slashed {
        // With a month name the remaining numbers are a day and a year in either order;
        // a first number above 31 (or three digits long) can only be the year.
        match (year, numbers.as_slice()) {
            (Some(_), [(d, _)]) => day = Some(*d),
            (None, [(a, alen), (b, blen)]) => {
                if *a > 31 || *alen >= 3 {
                    year = Some(date_legacy_year(*a, *alen));
                    day = Some(*b);
                } else {
                    day = Some(*a);
                    year = Some(date_legacy_year(*b, *blen));
                }
            }
            _ => return None,
        }
        if !(1..=31).contains(&day?) { return None; }
    }
    let (month, day, year) = (month?, day?, year?);
    let t = date_make_date(
        date_make_day(year as f64, month as f64, day as f64),
        date_make_time(hour as f64, minute as f64, second as f64, milli as f64),
    );
    if !t.is_finite() {
        return None;
    }
    Some(date_time_clip(t - offset as f64 * DATE_MS_PER_MINUTE))
}

/// Two-digit (or shorter) years follow the SpiderMonkey/V8 pivot: 0-49 are 20xx, 50-99 are 19xx.
fn date_legacy_year(value: i64, digits: usize) -> i64 {
    if digits >= 3 { value } else if value < 50 { 2000 + value } else { 1900 + value }
}

/// `YYYY-M-D[ H:M[:S[.f]]][Z|±HH[[:]MM]]` with one- or two-digit fields and a space (not `T`)
/// before the time, interpreted as local time when no offset is given.
fn date_parse_relaxed_iso(text: &str) -> Option<f64> {
    let (date_text, time_text) = match text.split_once(' ') {
        Some((date, time)) => (date, Some(time.trim())),
        None => (text, None),
    };
    let (sign, body) = match date_text.as_bytes().first()? {
        b'+' => (1i64, &date_text[1..]),
        b'-' => (-1i64, &date_text[1..]),
        _ => (1, date_text),
    };
    let fields: Vec<&str> = body.split('-').collect();
    if fields.len() != 3 || fields.iter().any(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) { return None; }
    let year_digits = if date_text.starts_with(['+', '-']) { 6 } else { 4 };
    if fields[0].len() != year_digits || fields[1].len() > 2 || fields[2].len() > 2 { return None; }
    let year = sign * fields[0].parse::<i64>().ok()?;
    let month: i64 = fields[1].parse().ok()?;
    let day: i64 = fields[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) { return None; }
    let (mut hour, mut minute, mut second, mut milli) = (0i64, 0i64, 0i64, 0i64);
    let mut offset: Option<i64> = None;
    let has_time = time_text.is_some();
    if let Some(time) = time_text {
        let (clock, zone) = match time.find(['Z', '+', '-']) {
            Some(at) => (&time[..at], Some(&time[at..])),
            None => (time, None),
        };
        let parts: Vec<&str> = clock.split(':').collect();
        if !(2..=3).contains(&parts.len()) { return None; }
        let digits = |f: &str, max: usize| -> Option<i64> {
            if f.is_empty() || f.len() > max || !f.bytes().all(|b| b.is_ascii_digit()) { None } else { f.parse().ok() }
        };
        hour = digits(parts[0], 2)?;
        minute = digits(parts[1], 2)?;
        if let Some(seconds) = parts.get(2) {
            let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
            second = digits(whole, 2)?;
            if seconds.contains('.') {
                if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) { return None; }
                let mut padded = fraction.to_string();
                padded.push_str("000");
                milli = padded[..3].parse().ok()?;
            }
        }
        if hour > 23 || minute > 59 || second > 59 { return None; }
        if let Some(zone) = zone {
            offset = Some(if zone == "Z" { 0 } else {
                let zsign = if zone.starts_with('-') { -1 } else { 1 };
                let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
                if !zone[1..].bytes().all(|b| b.is_ascii_digit() || b == b':') { return None; }
                let (zh, zm) = match digits.len() {
                    2 => (digits.parse::<i64>().ok()?, 0),
                    4 => (digits[..2].parse::<i64>().ok()?, digits[2..].parse::<i64>().ok()?),
                    _ => return None,
                };
                if zh > 23 || zm > 59 { return None; }
                zsign * (zh * 60 + zm)
            });
        }
    }
    let mut t = date_make_date(
        date_make_day(year as f64, (month - 1) as f64, day as f64),
        date_make_time(hour as f64, minute as f64, second as f64, milli as f64),
    );
    if !t.is_finite() { return None; }
    if let Some(offset) = offset { t -= offset as f64 * DATE_MS_PER_MINUTE; }
    else if has_time { t = date_utc_time(t); }
    Some(date_time_clip(t))
}

fn date_parse_string(text: &str) -> f64 {
    if let Some(value) = date_parse_iso(text) {
        return value;
    }
    if let Some(value) = date_parse_relaxed_iso(text) {
        return value;
    }
    date_parse_legacy(text).unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------
// Prototype accessors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum DateGetField {
    Time,
    Date,
    Day,
    FullYear,
    Hours,
    Milliseconds,
    Minutes,
    Month,
    Seconds,
    TimezoneOffset,
    Year,
}

fn date_get(
    _i: &mut Interpreter,
    this: JsValue,
    utc: bool,
    field: DateGetField,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if let DateGetField::Time = field {
        return Ok(JsValue::num(t));
    }
    if t.is_nan() {
        return Ok(JsValue::num(f64::NAN));
    }
    if let DateGetField::TimezoneOffset = field {
        return Ok(JsValue::num(
            (t - date_local_time(t)) / DATE_MS_PER_MINUTE,
        ));
    }
    let t = if utc { t } else { date_local_time(t) };
    let (year, month, date) = date_ymd(t);
    let value = match field {
        DateGetField::Date => date,
        DateGetField::Day => date_week_day(t),
        DateGetField::FullYear => year,
        DateGetField::Hours => date_hour(t),
        DateGetField::Milliseconds => date_ms_from_time(t),
        DateGetField::Minutes => date_minutes(t),
        DateGetField::Month => month,
        DateGetField::Seconds => date_seconds(t),
        DateGetField::Year => year - 1900.0,
        DateGetField::Time | DateGetField::TimezoneOffset => unreachable!(),
    };
    Ok(JsValue::num(value))
}

macro_rules! date_getter {
    ($name:ident, $utc:expr, $field:ident) => {
        fn $name(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
            date_get(i, this, $utc, DateGetField::$field)
        }
    };
}

date_getter!(date_proto_get_time, false, Time);
date_getter!(date_proto_get_date, false, Date);
date_getter!(date_proto_get_day, false, Day);
date_getter!(date_proto_get_full_year, false, FullYear);
date_getter!(date_proto_get_hours, false, Hours);
date_getter!(date_proto_get_milliseconds, false, Milliseconds);
date_getter!(date_proto_get_minutes, false, Minutes);
date_getter!(date_proto_get_month, false, Month);
date_getter!(date_proto_get_seconds, false, Seconds);
date_getter!(date_proto_get_timezone_offset, false, TimezoneOffset);
date_getter!(date_proto_get_year, false, Year);
date_getter!(date_proto_get_utc_date, true, Date);
date_getter!(date_proto_get_utc_day, true, Day);
date_getter!(date_proto_get_utc_full_year, true, FullYear);
date_getter!(date_proto_get_utc_hours, true, Hours);
date_getter!(date_proto_get_utc_milliseconds, true, Milliseconds);
date_getter!(date_proto_get_utc_minutes, true, Minutes);
date_getter!(date_proto_get_utc_month, true, Month);
date_getter!(date_proto_get_utc_seconds, true, Seconds);

// ---------------------------------------------------------------------------
// Prototype mutators
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum DateSetField {
    Time,
    Date,
    FullYear,
    Hours,
    Milliseconds,
    Minutes,
    Month,
    Seconds,
    Year,
}

fn date_required_number(i: &mut Interpreter, args: &[JsValue], index: usize) -> Result<f64, Throw> {
    match args.get(index) {
        Some(value) => i.to_number(value),
        None => i.to_number(&JsValue::Undefined),
    }
}

fn date_optional_number(
    i: &mut Interpreter,
    args: &[JsValue],
    index: usize,
) -> Result<Option<f64>, Throw> {
    match args.get(index) {
        Some(value) => Ok(Some(i.to_number(value)?)),
        None => Ok(None),
    }
}

fn date_set(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
    utc: bool,
    field: DateSetField,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else {
        return Err(Throw::type_error("this is not a Date object"));
    };
    let object = object.clone();
    let t0 = date_this_value(&this)?;

    match field {
        DateSetField::Time => {
            let t = date_required_number(i, &args, 0)?;
            let value = date_time_clip(t);
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Date => {
            let dt = date_required_number(i, &args, 0)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let new_date = date_make_date(
                date_make_day(date_year_from_time(t), date_month_from_time(t), dt),
                date_time_within_day(t),
            );
            let value = date_time_clip(date_utc(new_date, utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Month => {
            let month = date_required_number(i, &args, 0)?;
            let dt = date_optional_number(i, &args, 1)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let dt = dt.unwrap_or_else(|| date_date_from_time(t));
            let new_date = date_make_date(
                date_make_day(date_year_from_time(t), month, dt),
                date_time_within_day(t),
            );
            let value = date_time_clip(date_utc(new_date, utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::FullYear | DateSetField::Year => {
            let year = date_required_number(i, &args, 0)?;
            let (month, date) = if let DateSetField::FullYear = field {
                let month = date_optional_number(i, &args, 1)?;
                let date = date_optional_number(i, &args, 2)?;
                (month, date)
            } else {
                (None, None)
            };
            if let DateSetField::Year = field {
                if year.is_nan() {
                    return Ok(JsValue::num(date_store(&object, f64::NAN)));
                }
            }
            let t = if t0.is_nan() {
                0.0
            } else {
                date_local(t0, utc)
            };
            let y = if let DateSetField::Year = field {
                let integer = date_to_integer(year);
                if (0.0..=99.0).contains(&integer) {
                    integer + 1900.0
                } else {
                    year
                }
            } else {
                year
            };
            let m = month.unwrap_or_else(|| date_month_from_time(t));
            let dt = date.unwrap_or_else(|| date_date_from_time(t));
            let new_date =
                date_make_date(date_make_day(y, m, dt), date_time_within_day(t));
            let value = date_time_clip(date_utc(new_date, utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Hours => {
            let hour = date_required_number(i, &args, 0)?;
            let minute = date_optional_number(i, &args, 1)?;
            let second = date_optional_number(i, &args, 2)?;
            let milli = date_optional_number(i, &args, 3)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let time = date_make_time(
                hour,
                minute.unwrap_or_else(|| date_minutes(t)),
                second.unwrap_or_else(|| date_seconds(t)),
                milli.unwrap_or_else(|| date_ms_from_time(t)),
            );
            let value = date_time_clip(date_utc(date_make_date(date_day(t), time), utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Minutes => {
            let minute = date_required_number(i, &args, 0)?;
            let second = date_optional_number(i, &args, 1)?;
            let milli = date_optional_number(i, &args, 2)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let time = date_make_time(
                date_hour(t),
                minute,
                second.unwrap_or_else(|| date_seconds(t)),
                milli.unwrap_or_else(|| date_ms_from_time(t)),
            );
            let value = date_time_clip(date_utc(date_make_date(date_day(t), time), utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Seconds => {
            let second = date_required_number(i, &args, 0)?;
            let milli = date_optional_number(i, &args, 1)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let time = date_make_time(
                date_hour(t),
                date_minutes(t),
                second,
                milli.unwrap_or_else(|| date_ms_from_time(t)),
            );
            let value = date_time_clip(date_utc(date_make_date(date_day(t), time), utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
        DateSetField::Milliseconds => {
            let milli = date_required_number(i, &args, 0)?;
            if t0.is_nan() {
                return Ok(JsValue::num(f64::NAN));
            }
            let t = date_local(t0, utc);
            let time = date_make_time(
                date_hour(t),
                date_minutes(t),
                date_seconds(t),
                milli,
            );
            let value = date_time_clip(date_utc(date_make_date(date_day(t), time), utc));
            return Ok(JsValue::num(date_store(&object, value)));
        }
    }
}

fn date_local(t: f64, utc: bool) -> f64 {
    if utc {
        t
    } else {
        date_local_time(t)
    }
}

fn date_utc(t: f64, utc: bool) -> f64 {
    if utc {
        t
    } else {
        date_utc_time(t)
    }
}

macro_rules! date_setter {
    ($name:ident, $utc:expr, $field:ident) => {
        fn $name(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
            date_set(i, this, args, $utc, DateSetField::$field)
        }
    };
}

date_setter!(date_proto_set_time, false, Time);
date_setter!(date_proto_set_date, false, Date);
date_setter!(date_proto_set_full_year, false, FullYear);
date_setter!(date_proto_set_hours, false, Hours);
date_setter!(date_proto_set_milliseconds, false, Milliseconds);
date_setter!(date_proto_set_minutes, false, Minutes);
date_setter!(date_proto_set_month, false, Month);
date_setter!(date_proto_set_seconds, false, Seconds);
date_setter!(date_proto_set_year, false, Year);
date_setter!(date_proto_set_utc_date, true, Date);
date_setter!(date_proto_set_utc_full_year, true, FullYear);
date_setter!(date_proto_set_utc_hours, true, Hours);
date_setter!(date_proto_set_utc_milliseconds, true, Milliseconds);
date_setter!(date_proto_set_utc_minutes, true, Minutes);
date_setter!(date_proto_set_utc_month, true, Month);
date_setter!(date_proto_set_utc_seconds, true, Seconds);

// ---------------------------------------------------------------------------
// String conversion and JSON
// ---------------------------------------------------------------------------

fn date_proto_to_string(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Ok(JsValue::str("Invalid Date"));
    }
    let t = date_local_time(t);
    Ok(JsValue::str(format!(
        "{} {} {}",
        date_date_string(t),
        date_time_string(t),
        date_time_zone_string(t)
    )))
}

fn date_proto_to_date_string(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Ok(JsValue::str("Invalid Date"));
    }
    Ok(JsValue::str(date_date_string(date_local_time(t))))
}

fn date_proto_to_time_string(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Ok(JsValue::str("Invalid Date"));
    }
    let t = date_local_time(t);
    Ok(JsValue::str(format!(
        "{} {}",
        date_time_string(t),
        date_time_zone_string(t)
    )))
}

fn date_proto_to_utc_string(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Ok(JsValue::str("Invalid Date"));
    }
    Ok(JsValue::str(format!(
        "{} {} GMT",
        date_utc_date_string(t),
        date_time_string(t)
    )))
}

fn date_proto_to_iso_string(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Err(Throw::range_error("Invalid time value"));
    }
    let (year, month, date) = date_ymd(t);
    Ok(JsValue::str(format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        date_format_iso_year(year as i64),
        month as i64 + 1,
        date as i64,
        date_hour(t) as i64,
        date_minutes(t) as i64,
        date_seconds(t) as i64,
        date_ms_from_time(t) as i64
    )))
}

fn date_value_of(
    _i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(date_this_value(&this)?))
}

fn date_ordinary_to_primitive(
    i: &mut Interpreter,
    value: &JsValue,
    try_first_string: bool,
) -> Result<JsValue, Throw> {
    let names: [&str; 2] = if try_first_string {
        ["toString", "valueOf"]
    } else {
        ["valueOf", "toString"]
    };
    for name in names {
        let method = i.get_prop(value, name)?;
        if callable_function(&method).is_some() {
            let result = i.call_value(&method, value.clone(), Vec::new())?;
            if !result.is_object() {
                return Ok(result);
            }
        }
    }
    Err(Throw::type_error("cannot convert object to primitive"))
}

fn date_proto_to_primitive(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error("Date.prototype[Symbol.toPrimitive] called on non-object"));
    }
    let try_first_string = match args.first() {
        Some(JsValue::Str(hint)) => match hint.to_utf8_lossy().as_str() {
            "string" | "default" => true,
            "number" => false,
            _ => return Err(Throw::type_error("invalid hint")),
        },
        _ => return Err(Throw::type_error("invalid hint")),
    };
    date_ordinary_to_primitive(i, &this, try_first_string)
}

fn date_proto_to_json(
    i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if matches!(this, JsValue::Undefined | JsValue::Null) {
        return Err(Throw::type_error("toJSON called on null or undefined"));
    }
    let object = native_object(i, JsValue::Undefined, vec![this])?;
    // ToPrimitive(O, number): unlike `primitive` (default hint), the number hint
    // must be passed through to an exotic @@toPrimitive method.
    let tv = i.primitive_number_hint(object.clone())?;
    if let JsValue::Num(number) = tv {
        if !number.is_finite() {
            return Ok(JsValue::Null);
        }
    }
    let method = i.get_prop(&object, "toISOString")?;
    i.call_value(&method, object, Vec::new())
}

fn date_proto_to_temporal_instant(
    i: &mut Interpreter,
    this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let t = date_this_value(&this)?;
    if t.is_nan() {
        return Err(Throw::range_error("invalid Date value"));
    }
    let nanoseconds = (t as i128) * 1_000_000;
    std_epoch_make(i, StdEpochKind::Instant, nanoseconds, "UTC".into())
}

// ---------------------------------------------------------------------------
// Constructor and static methods
// ---------------------------------------------------------------------------

fn date_value_to_number(i: &mut Interpreter, value: &JsValue) -> Result<f64, Throw> {
    match value {
        JsValue::Str(text) => Ok(date_parse_string(&text.to_utf8_lossy())),
        _ => i.to_number(value),
    }
}

fn date_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() {
        let now = date_local_time(date_now_ms());
        return Ok(JsValue::str(format!(
            "{} {} {}",
            date_date_string(now),
            date_time_string(now),
            date_time_zone_string(now)
        )));
    }
    let _ = &this;
    let value = if args.is_empty() {
        date_time_clip(date_now_ms())
    } else if args.len() == 1 {
        let value = args[0].clone();
        let t = if let JsValue::Object(candidate) = &value {
            let existing = candidate.borrow().get_own(DATE_SLOT);
            if let Some(JsValue::Num(existing)) = existing {
                existing
            } else {
                let primitive = i.primitive_with_hint(value.clone(), false)?;
                date_value_to_number(i, &primitive)?
            }
        } else {
            date_value_to_number(i, &value)?
        };
        date_time_clip(t)
    } else {
        let year = i.to_number(&args[0])?;
        let month = i.to_number(&args[1])?;
        let date = match args.get(2) {
            Some(value) => i.to_number(value)?,
            None => 1.0,
        };
        let hour = match args.get(3) {
            Some(value) => i.to_number(value)?,
            None => 0.0,
        };
        let minute = match args.get(4) {
            Some(value) => i.to_number(value)?,
            None => 0.0,
        };
        let second = match args.get(5) {
            Some(value) => i.to_number(value)?,
            None => 0.0,
        };
        let milli = match args.get(6) {
            Some(value) => i.to_number(value)?,
            None => 0.0,
        };
        let year = if year.is_nan() {
            year
        } else {
            let integer = date_to_integer(year);
            if (0.0..=99.0).contains(&integer) {
                integer + 1900.0
            } else {
                year
            }
        };
        let local = date_make_date(
            date_make_day(year, month, date),
            date_make_time(hour, minute, second, milli),
        );
        date_time_clip(date_utc_time(local))
    };
    // DateConstructor owns allocation: the time value is computed first, then NewTarget.prototype.
    let prototype = std_collection_constructor_prototype(i)?;
    let mut raw = JsObject::ordinary();
    raw.proto = Some(prototype);
    raw.constructed = true;
    let object = Obj(Rc::new(RefCell::new(raw)));
    date_store(&object, value);
    Ok(JsValue::Object(object))
}

fn date_static_now(
    _i: &mut Interpreter,
    _this: JsValue,
    _args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::num(date_now_ms()))
}

fn date_static_parse(
    i: &mut Interpreter,
    _this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let text = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::num(date_parse_string(&text)))
}

fn date_static_utc(
    i: &mut Interpreter,
    _this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let year = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    // `month` is optional in Date.UTC; an absent month is +0.
    let month = if args.len() > 1 {
        i.to_number(&args[1])?
    } else {
        0.0
    };
    let date = match args.get(2) {
        Some(value) => i.to_number(value)?,
        None => 1.0,
    };
    let hour = match args.get(3) {
        Some(value) => i.to_number(value)?,
        None => 0.0,
    };
    let minute = match args.get(4) {
        Some(value) => i.to_number(value)?,
        None => 0.0,
    };
    let second = match args.get(5) {
        Some(value) => i.to_number(value)?,
        None => 0.0,
    };
    let milli = match args.get(6) {
        Some(value) => i.to_number(value)?,
        None => 0.0,
    };
    let year = if year.is_nan() {
        year
    } else {
        let integer = date_to_integer(year);
        if (0.0..=99.0).contains(&integer) {
            integer + 1900.0
        } else {
            year
        }
    };
    let t = date_make_date(
        date_make_day(year, month, date),
        date_make_time(hour, minute, second, milli),
    );
    Ok(JsValue::num(date_time_clip(t)))
}

// ---------------------------------------------------------------------------
// Installation
// ---------------------------------------------------------------------------

impl Interpreter {
    fn install_date(&mut self) {
        let constructor = Self::native_constructor("Date", date_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else {
            unreachable!("Date constructor is an object");
        };
        binary_configure_constructor(function, "Date", 7, &self.date_proto);
        let proto = &self.date_proto;
        let _ = proto.borrow_mut().define(
            "constructor",
            binary_writable_property(constructor.clone()),
        );

        for (name, native, length) in [
            ("getTime", date_proto_get_time as crate::value::NativeFn, 0),
            ("getDate", date_proto_get_date as crate::value::NativeFn, 0),
            ("getDay", date_proto_get_day as crate::value::NativeFn, 0),
            ("getFullYear", date_proto_get_full_year as crate::value::NativeFn, 0),
            ("getHours", date_proto_get_hours as crate::value::NativeFn, 0),
            ("getMilliseconds", date_proto_get_milliseconds as crate::value::NativeFn, 0),
            ("getMinutes", date_proto_get_minutes as crate::value::NativeFn, 0),
            ("getMonth", date_proto_get_month as crate::value::NativeFn, 0),
            ("getSeconds", date_proto_get_seconds as crate::value::NativeFn, 0),
            ("getTimezoneOffset", date_proto_get_timezone_offset as crate::value::NativeFn, 0),
            ("getUTCDate", date_proto_get_utc_date as crate::value::NativeFn, 0),
            ("getUTCDay", date_proto_get_utc_day as crate::value::NativeFn, 0),
            ("getUTCFullYear", date_proto_get_utc_full_year as crate::value::NativeFn, 0),
            ("getUTCHours", date_proto_get_utc_hours as crate::value::NativeFn, 0),
            ("getUTCMilliseconds", date_proto_get_utc_milliseconds as crate::value::NativeFn, 0),
            ("getUTCMinutes", date_proto_get_utc_minutes as crate::value::NativeFn, 0),
            ("getUTCMonth", date_proto_get_utc_month as crate::value::NativeFn, 0),
            ("getUTCSeconds", date_proto_get_utc_seconds as crate::value::NativeFn, 0),
            ("setTime", date_proto_set_time as crate::value::NativeFn, 1),
            ("setDate", date_proto_set_date as crate::value::NativeFn, 1),
            ("setFullYear", date_proto_set_full_year as crate::value::NativeFn, 3),
            ("setHours", date_proto_set_hours as crate::value::NativeFn, 4),
            ("setMilliseconds", date_proto_set_milliseconds as crate::value::NativeFn, 1),
            ("setMinutes", date_proto_set_minutes as crate::value::NativeFn, 3),
            ("setMonth", date_proto_set_month as crate::value::NativeFn, 2),
            ("setSeconds", date_proto_set_seconds as crate::value::NativeFn, 2),
            ("setUTCDate", date_proto_set_utc_date as crate::value::NativeFn, 1),
            ("setUTCFullYear", date_proto_set_utc_full_year as crate::value::NativeFn, 3),
            ("setUTCHours", date_proto_set_utc_hours as crate::value::NativeFn, 4),
            ("setUTCMilliseconds", date_proto_set_utc_milliseconds as crate::value::NativeFn, 1),
            ("setUTCMinutes", date_proto_set_utc_minutes as crate::value::NativeFn, 3),
            ("setUTCMonth", date_proto_set_utc_month as crate::value::NativeFn, 2),
            ("setUTCSeconds", date_proto_set_utc_seconds as crate::value::NativeFn, 2),
            ("toString", date_proto_to_string as crate::value::NativeFn, 0),
            ("toDateString", date_proto_to_date_string as crate::value::NativeFn, 0),
            ("toTimeString", date_proto_to_time_string as crate::value::NativeFn, 0),
            ("toUTCString", date_proto_to_utc_string as crate::value::NativeFn, 0),
            ("toISOString", date_proto_to_iso_string as crate::value::NativeFn, 0),
            ("toJSON", date_proto_to_json as crate::value::NativeFn, 1),
            ("valueOf", date_value_of as crate::value::NativeFn, 0),
            ("toTemporalInstant", date_proto_to_temporal_instant as crate::value::NativeFn, 0),
            ("getYear", date_proto_get_year as crate::value::NativeFn, 0),
            ("setYear", date_proto_set_year as crate::value::NativeFn, 1),
        ] {
            binary_method(self, proto, name, native, length);
        }

        // Annex B: toGMTString is the very same function object as toUTCString.
        let to_utc_string = proto.borrow().get_own("toUTCString");
        if let Some(to_utc_string) = to_utc_string {
            let _ = proto
                .borrow_mut()
                .define("toGMTString", binary_writable_property(to_utc_string));
        }

        let to_primitive = Self::native(
            "[Symbol.toPrimitive]",
            date_proto_to_primitive,
            &self.function_proto,
        );
        if let JsValue::Object(callable) = &to_primitive {
            callable.borrow_mut().constructible = false;
            callable.borrow_mut().delete_own("prototype");
            let _ = callable
                .borrow_mut()
                .define("length", binary_property(JsValue::num(1.0)));
            let _ = callable.borrow_mut().define(
                "name",
                binary_property(JsValue::str("[Symbol.toPrimitive]")),
            );
        }
        let _ = proto.borrow_mut().define(
            WellKnownSymbol::ToPrimitive.symbol(),
            binary_property(to_primitive),
        );

        for (name, native, length) in [
            ("now", date_static_now as crate::value::NativeFn, 0),
            ("parse", date_static_parse as crate::value::NativeFn, 1),
            ("UTC", date_static_utc as crate::value::NativeFn, 7),
        ] {
            binary_method(self, function, name, native, length);
        }

        self.define_global("Date", constructor);
    }
}

#[cfg(test)]
mod date_tag_metadata_tests {
    use super::*;
    #[test]
    fn initial_date_prototype_has_no_explicit_tag_and_proxy_date_is_unbranded() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            let source=r#"
              var valid=Object.getOwnPropertyDescriptor(Date.prototype,Symbol.toStringTag)===undefined && !Reflect.ownKeys(Date.prototype).includes(Symbol.toStringTag);
              valid=valid && Object.prototype.toString.call(Date.prototype)==='[object Object]';
              var date=new Date(0),proxy=new Proxy(date,{});date.join=undefined;
              valid=valid && Object.prototype.toString.call(date)==='[object Date]' && Object.prototype.toString.call(proxy)==='[object Object]';
              valid=valid && Array.prototype.toString.call(date)==='[object Date]' && Array.prototype.toString.call(proxy)==='[object Object]';valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn user_date_tags_preserve_receiver_override_and_abrupt_observation() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            let source=r#"
              var valid=true,date=new Date(0),proxy=new Proxy(date,{}),seen;
              Object.defineProperty(date,Symbol.toStringTag,{get:function(){seen=this;return'Custom';},configurable:true});
              valid=valid && Object.prototype.toString.call(proxy)==='[object Custom]' && seen===proxy;
              valid=valid && Object.prototype.toString.call(date)==='[object Custom]' && seen===date;
              delete date[Symbol.toStringTag];Date.prototype[Symbol.toStringTag]='User';
              valid=valid && Object.prototype.toString.call(proxy)==='[object User]' && Object.prototype.toString.call(Date.prototype)==='[object User]';
              delete Date.prototype[Symbol.toStringTag];date[Symbol.toStringTag]=7;
              valid=valid && Object.prototype.toString.call(date)==='[object Date]' && Object.prototype.toString.call(proxy)==='[object Object]';
              var error={};Object.defineProperty(date,Symbol.toStringTag,{get:function(){throw error;},configurable:true});
              try{Object.prototype.toString.call(proxy);valid=false;}catch(e){valid=valid && e===error;}
              var revoked=Proxy.revocable(date,{});revoked.revoke();try{Object.prototype.toString.call(revoked.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn foreign_date_tags_do_not_leak_a_brand_through_proxy_or_prototype() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let global=i.binding_create_object(None).unwrap();let realm=i.binding_create_realm(global).unwrap();
            for(name,source) in [("foreignDate","new Date(0)"),("foreignDatePrototype","Date.prototype"),("foreignTag","Object.prototype.toString"),("foreignArrayTag","Array.prototype.toString"),("foreignType","TypeError.prototype")] {
                let value=i.binding_eval_in_realm(realm,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            let source=r#"
              var valid=Object.getOwnPropertyDescriptor(foreignDatePrototype,Symbol.toStringTag)===undefined;
              valid=valid && foreignTag.call(foreignDatePrototype)==='[object Object]' && Object.prototype.toString.call(foreignDate)==='[object Date]';
              var proxy=new Proxy(foreignDate,{});foreignDate.join=undefined;
              valid=valid && foreignTag.call(proxy)==='[object Object]' && foreignArrayTag.call(proxy)==='[object Object]';
              var seen;Object.defineProperty(foreignDate,Symbol.toStringTag,{get:function(){seen=this;return'ForeignUser';}});
              valid=valid && foreignTag.call(proxy)==='[object ForeignUser]' && seen===proxy;
              var revoked=Proxy.revocable(foreignDate,{});revoked.revoke();try{foreignTag.call(revoked.proxy);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }
}
