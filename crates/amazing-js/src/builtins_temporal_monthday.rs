// CalendarMonthDayFromFields for every supported calendar: resolves a
// (monthCode | month, day [, year]) triple to the ISO reference date a
// PlainMonthDay stores. The reference date is the latest ISO date on or
// before 1972-12-31 whose calendar month code and day match; an explicit year
// only decides whether the month/day exist (overflow constrain or reject), it
// never becomes the reference year. Month lengths and leap months come from
// the per-calendar tables in builtins_temporal_calendar.rs.

const STD_MD_LAST_ISO: (i64, i64, i64) = (1972, 12, 31);

struct StdMdCode { num: i64, leap: bool }

fn std_md_parse_code(code: &str) -> Option<StdMdCode> {
    let body = code.strip_prefix('M')?;
    let (digits, leap) = match body.strip_suffix('L') { Some(d) => (d, true), None => (body, false) };
    if digits.len() != 2 || !digits.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let num: i64 = digits.parse().ok()?;
    if num < 1 { return None; }
    Some(StdMdCode { num, leap })
}

fn std_md_text(code: &StdMdCode) -> String {
    format!("M{:02}{}", code.num, if code.leap { "L" } else { "" })
}

// Ordinal month and whether the exact code exists in calendar year `y`.
// `None` when the code can never be valid for this calendar.
fn std_md_ord(cal: &str, y: i64, code: &StdMdCode) -> Option<(i64, bool)> {
    if !std_calendar_is_month_structure(cal) {
        // ISO-grid calendars (iso8601, gregory, japanese, roc, buddhist).
        if code.leap || code.num > 12 { return None; }
        return Some((code.num, true));
    }
    if std_calendar_has_leap_months(cal) {
        std_calendar_resolve_code_ord(cal, y, &std_md_text(code))
    } else {
        let most = if std_calendar_is_13month(cal) { 13 } else { 12 };
        if code.leap || code.num > most { return None; }
        Some((code.num, code.num <= std_calendar_months_in_year(cal, y)))
    }
}

fn std_md_month_len(cal: &str, y: i64, ord: i64) -> i64 {
    if !std_calendar_is_month_structure(cal) { std_temporal_days_in_month(y, ord) }
    else { std_calendar_month_len(cal, y, ord) }
}

fn std_md_to_iso(cal: &str, y: i64, ord: i64, day: i64) -> (i64, i64, i64) {
    if !std_calendar_is_month_structure(cal) { (y, ord, day) } else { std_calendar_to_iso(cal, y, ord, day) }
}

fn std_md_months_in_year(cal: &str, y: i64) -> i64 {
    if !std_calendar_is_month_structure(cal) { 12 } else { std_calendar_months_in_year(cal, y) }
}

// Latest reference date for a month code and day, together with the longest
// matching month seen (to constrain an overlong day).
fn std_md_search(cal: &str, code: &StdMdCode, day: i64) -> (Option<(i64, i64, i64)>, i64) {
    let grid = !std_calendar_is_month_structure(cal);
    let start = if grid { STD_MD_LAST_ISO.0 } else { std_calendar_fields(cal, STD_MD_LAST_ISO.0, STD_MD_LAST_ISO.1, STD_MD_LAST_ISO.2).0 };
    let window: i64 = if matches!(cal, "chinese" | "dangi") { 76 } else { 120 };
    let mut longest = 0;
    for step in 0..window {
        let y = start - step;
        let Some((ord, exists)) = std_md_ord(cal, y, code) else { return (None, 0) };
        if !exists { continue; }
        let len = std_md_month_len(cal, y, ord);
        longest = longest.max(len);
        if len >= day {
            let iso = std_md_to_iso(cal, y, ord, day);
            if iso <= STD_MD_LAST_ISO { return (Some(iso), longest); }
        }
    }
    // Months that do not occur in the table range before the reference date
    // (rare Chinese/Dangi leap months) use their first occurrence afterwards.
    if matches!(cal, "chinese" | "dangi") {
        for step in 1..=63 {
            let y = start + step;
            let Some((ord, exists)) = std_md_ord(cal, y, code) else { break };
            if !exists { continue; }
            let len = std_md_month_len(cal, y, ord);
            if len >= day {
                let iso = std_md_to_iso(cal, y, ord, day);
                // The reference table ends with ISO year 2034.
                if iso.0 > 2034 { break; }
                return (Some(iso), longest.max(len));
            }
            longest = longest.max(len);
        }
    }
    (None, longest)
}

// `year` is the calendar-native year when the caller supplied one (year or
// era+eraYear); `month` the ordinal month and `code` the monthCode text.
fn std_calendar_monthday_reference(cal: &str, year: Option<i64>, month: Option<i64>, code: Option<&str>,
    day: i64, reject: bool) -> Result<(i64, i64, i64), Throw> {
    let grid = !std_calendar_is_month_structure(cal);
    let leap_calendar = std_calendar_has_leap_months(cal);
    let _ = leap_calendar;
    if day < 1 { return Err(Throw::range_error("day must be a positive integer")); }
    let parsed = match code {
        Some(text) => Some(std_md_parse_code(text).ok_or_else(|| Throw::range_error("invalid monthCode"))?),
        None => None,
    };
    // A month number alone needs a year in any non-ISO calendar, and a
    // month/monthCode pair needs one wherever the two can disagree.
    if cal != "iso8601" && year.is_none() && (parsed.is_none() || month.is_some()) {
        return Err(Throw::type_error("year is required to resolve the month"));
    }
    let iso_year = year.map(|y| if grid { std_calendar_year_to_iso(cal, y) } else { y });
    let mut day = day;
    let code = match (parsed, month, iso_year) {
        (Some(code), month, y) => {
            if std_md_ord(cal, y.unwrap_or(STD_MD_LAST_ISO.0), &code).is_none() {
                return Err(Throw::range_error("invalid monthCode"));
            }
            if let Some(month) = month {
                let (ord, _) = std_md_ord(cal, y.unwrap_or(STD_MD_LAST_ISO.0), &code).unwrap();
                if month != ord { return Err(Throw::range_error("month and monthCode disagree")); }
            }
            code
        }
        (None, Some(month), Some(y)) => {
            let months = std_md_months_in_year(cal, y);
            if month < 1 { return Err(Throw::range_error("month must be a positive integer")); }
            if month > months && reject { return Err(Throw::range_error("month is out of range")); }
            let ord = month.min(months);
            if leap_calendar {
                let iso = std_md_to_iso(cal, y, ord, 1);
                let text = std_calendar_month_code(cal, iso.0, iso.1, iso.2);
                std_md_parse_code(&text).ok_or_else(|| Throw::range_error("invalid monthCode"))?
            } else { StdMdCode { num: ord, leap: false } }
        }
        (None, Some(month), None) => {
            // iso8601 only: the ordinal month is the month number.
            if month < 1 { return Err(Throw::range_error("month must be a positive integer")); }
            if month > 12 && reject { return Err(Throw::range_error("month is out of range")); }
            StdMdCode { num: month.min(12), leap: false }
        }
        (None, None, _) => return Err(Throw::type_error("month is required")),
    };
    let mut code = code;
    // With an explicit year the month/day must exist in that year.
    if let Some(y) = iso_year {
        let (_, exists) = std_md_ord(cal, y, &code).ok_or_else(|| Throw::range_error("invalid monthCode"))?;
        if !exists {
            if reject { return Err(Throw::range_error("month does not exist in this year")); }
            // A leap month absent from the year falls back to the common month
            // (Hebrew Adar I to Adar, Chinese/Dangi leap Mxx to Mxx).
            code = StdMdCode { num: if cal == "hebrew" && code.leap { 6 } else { code.num }, leap: false };
        }
        let (ord, _) = std_md_ord(cal, y, &code).ok_or_else(|| Throw::range_error("invalid monthCode"))?;
        let len = std_md_month_len(cal, y, ord);
        if day > len {
            if reject { return Err(Throw::range_error("day is out of range for this month")); }
            day = len;
        }
    }
    let (found, longest) = std_md_search(cal, &code, day);
    if let Some(iso) = found { return Ok(iso); }
    if reject { return Err(Throw::range_error("month/day does not exist in the calendar")); }
    if code.leap {
        // A leap month constrains the day to the calendar's longest month
        // (30); when even that day never occurs for this leap month, the
        // month falls back to its common sibling with the requested day.
        let cap = day.min(30);
        if cap != day {
            if let (Some(iso), _) = std_md_search(cal, &code, cap) { return Ok(iso); }
        }
        let sibling = StdMdCode { num: code.num, leap: false };
        let (found, longest) = std_md_search(cal, &sibling, day);
        if let Some(iso) = found { return Ok(iso); }
        if longest > 0 {
            return std_md_search(cal, &sibling, longest).0
                .ok_or_else(|| Throw::range_error("month/day does not exist in the calendar"));
        }
        return Err(Throw::range_error("month/day does not exist in the calendar"));
    }
    if longest > 0 {
        let (found, _) = std_md_search(cal, &code, longest);
        return found.ok_or_else(|| Throw::range_error("month/day does not exist in the calendar"));
    }
    Err(Throw::range_error("month/day does not exist in the calendar"))
}
