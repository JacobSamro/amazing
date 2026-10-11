// ZonedDateTime-aware relative Duration arithmetic: ToRelativeTemporalObject
// results that keep their time zone, AddZonedDateTime, DifferenceZonedDateTime,
// RoundRelativeDuration (NudgeToCalendarUnit / NudgeToZonedTime) and
// BubbleRelativeDuration, used by Duration.prototype.{round,total},
// Duration.compare and ZonedDateTime.prototype.{since,until}.
//
// Structure ported from boa-dev/temporal (temporal_rs)
// @ eadf8039d2577e4375493a0eb3f46a66f25616ec,
// src/builtins/core/duration.rs (round_relative_duration, nudge_calendar_unit,
// nudge_to_zoned_time, bubble_relative_duration) and
// src/builtins/core/zoneddatetime.rs (difference_zoned_date_time), Apache-2.0,
// rewritten onto this engine's own types. See THIRD_PARTY_NOTICES.md.

#[derive(Clone)]
struct StdZonedRel {
    epoch_ns: i128,
    zone: String,
    calendar: String,
}

#[derive(Clone)]
struct StdRelative {
    // Local date of the relativeTo value (time-of-day zeroed).
    fields: [i64; 10],
    zoned: Option<StdZonedRel>,
}

struct StdNudge {
    date: [i64; 10],
    time_ns: i128,
    nudged_ns: i128,
    expanded: bool,
    // Exact total as numerator/denominator (only meaningful for calendar nudges).
    total: Option<(i128, i128)>,
}

fn zr_local(zone: &str, ns: i128) -> Result<[i64; 10], Throw> {
    let mut fields = [0i64; 10];
    std_temporal_set_epoch_ns(&mut fields, ns + tz_offset_ns(zone, ns))?;
    Ok(fields)
}

fn zr_check_date(fields: &[i64; 10]) -> Result<(), Throw> {
    let mut date = *fields;
    date[3..9].fill(0);
    std_temporal_validate(StdTemporalKind::PlainDate, &date)
}

// AddZonedDateTime with a date part (years, months, weeks, days in slots 0..4)
// and an exact clock part.
fn zr_add(rel: &StdZonedRel, ns: i128, date: &[i64; 10], time_ns: i128) -> Result<i128, Throw> {
    if date[..4].iter().all(|v| *v == 0) {
        return std_epoch_check_range(ns + time_ns);
    }
    let local = zr_local(&rel.zone, ns)?;
    let mut step = [0i64; 10];
    step[..4].copy_from_slice(&date[..4]);
    let shifted = std_temporal_add_calendar_cal(&rel.calendar, &local, &step, 1, false)?;
    zr_check_date(&shifted)?;
    let intermediate = std_epoch_disambiguate(&rel.zone, std_temporal_epoch_ns(&shifted), "compatible")?;
    std_epoch_check_range(std_epoch_check_range(intermediate)? + time_ns)
}

// Instant at `start_local`'s time-of-day on the date reached by adding the
// date duration `duration` to `start_local`'s date.
fn zr_resolve(rel: &StdZonedRel, _start_local: &[i64; 10], duration: &[i64; 10]) -> Result<i128, Throw> {
    zr_add(rel, rel.epoch_ns, duration, 0)
}

// DifferenceZonedDateTime: calendar part plus an exact sub-day remainder.
fn zr_difference(rel: &StdZonedRel, ns1: i128, ns2: i128, largest: &str) -> Result<([i64; 10], i128), Throw> {
    if ns1 == ns2 { return Ok(([0; 10], 0)); }
    let start = zr_local(&rel.zone, ns1)?;
    let end = zr_local(&rel.zone, ns2)?;
    if start[..3] == end[..3] { return Ok(([0; 10], ns2 - ns1)); }
    let sign: i128 = if ns2 < ns1 { -1 } else { 1 };
    let start_time = std_temporal_time_ns(&start);
    let mut time_diff = std_temporal_time_ns(&end) - start_time;
    let max_correction: i128 = if sign == 1 { 2 } else { 1 };
    let mut correction: i128 = if time_diff.signum() == -sign { 1 } else { 0 };
    let end_day = std_temporal_epoch_day(end[0], end[1], end[2]);
    let mut intermediate = [0i64; 10];
    let mut success = false;
    while correction <= max_correction {
        let day = end_day - (correction * sign) as i64;
        let (y, m, d) = std_temporal_civil_from_epoch_day(day);
        let mut candidate = [0i64; 10];
        candidate[0] = y; candidate[1] = m; candidate[2] = d;
        std_temporal_set_time_ns(&mut candidate, start_time);
        let candidate_ns = std_epoch_disambiguate(&rel.zone, std_temporal_epoch_ns(&candidate), "compatible")?;
        time_diff = ns2 - candidate_ns;
        intermediate = candidate;
        if time_diff.signum() != -sign { success = true; break; }
        correction += 1;
    }
    if !success { return Err(Throw::range_error("ZonedDateTime difference did not converge")); }
    let date_largest = if matches!(largest, "year" | "month" | "week") { largest } else { "day" };
    let mut first = [0i64; 10];
    first[..3].copy_from_slice(&start[..3]);
    let mut second = [0i64; 10];
    second[..3].copy_from_slice(&intermediate[..3]);
    let date = std_temporal_calendar_difference_cal(&rel.calendar, &first, &second, date_largest)?;
    Ok((date, time_diff))
}

fn zr_decide_up(sign: i64, r1: i64, inc: i64, num: i128, denom: i128, mode: &str) -> bool {
    let (n, d) = (num.abs(), denom.abs());
    if n == 0 { return false; }
    if n == d { return true; }
    let twice = n * 2;
    match mode {
        "ceil" => sign > 0,
        "floor" => sign < 0,
        "trunc" => false,
        "expand" => true,
        "halfExpand" => twice >= d,
        "halfTrunc" => twice > d,
        "halfCeil" => twice > d || (twice == d && sign > 0),
        "halfFloor" => twice > d || (twice == d && sign < 0),
        _ => twice > d || (twice == d && (r1.abs() / inc) % 2 == 1), // halfEven
    }
}

// NudgeToCalendarUnit for a zoned relativeTo (also used for unit "day").
fn zr_nudge_calendar(rel: &StdZonedRel, sign: i64, date: &[i64; 10], dest_ns: i128, start_local: &[i64; 10],
    inc: i64, unit: &str, mode: &str) -> Result<StdNudge, Throw> {
    let index = std_temporal_unit_rank(unit).filter(|r| *r <= 3).ok_or_else(|| Throw::range_error("invalid unit"))?;
    // Weeks absorb whole weeks hiding in the day count (CalendarDateUntil
    // with largestUnit week from the year/month-shifted start).
    let current = if index == 2 { date[2] + date[3] / 7 } else { date[index] };
    let r1 = (current / inc) * inc;
    let r2 = r1 + inc * sign;
    let mut start_d = [0i64; 10];
    let mut end_d = [0i64; 10];
    for k in 0..index { start_d[k] = date[k]; end_d[k] = date[k]; }
    start_d[index] = r1;
    end_d[index] = r2;
    let start_ns = zr_resolve(rel, start_local, &start_d)?;
    let end_ns = zr_resolve(rel, start_local, &end_d)?;
    let in_order = if sign > 0 { start_ns <= dest_ns && dest_ns <= end_ns } else { end_ns <= dest_ns && dest_ns <= start_ns };
    if !in_order || start_ns == end_ns {
        return Err(Throw::range_error("rounded duration is outside the calendar unit bracket"));
    }
    let num = dest_ns - start_ns;
    let denom = end_ns - start_ns;
    let up = zr_decide_up(sign, r1, inc, num, denom, mode);
    let total = Some((r1 as i128 * denom.abs() + sign as i128 * inc as i128 * num.abs(), denom.abs()));
    Ok(if up {
        StdNudge { date: end_d, time_ns: 0, nudged_ns: end_ns, expanded: true, total }
    } else {
        StdNudge { date: start_d, time_ns: 0, nudged_ns: start_ns, expanded: false, total }
    })
}

// NudgeToZonedTime: round the time part against the real length of the
// (possibly DST-shifted) day that follows the date part.
fn zr_nudge_time(rel: &StdZonedRel, sign: i64, date: &[i64; 10], time_ns: i128, start_local: &[i64; 10],
    inc: i64, unit: &str, mode: &str) -> Result<StdNudge, Throw> {
    let start_ns = zr_resolve(rel, start_local, date)?;
    let mut next = *date;
    next[3] += sign;
    let end_ns = zr_resolve(rel, start_local, &next)?;
    let day_span = end_ns - start_ns;
    if day_span.signum() != sign as i128 {
        return Err(Throw::range_error("day span has the wrong sign"));
    }
    let step = std_temporal_unit_ns(unit).ok_or_else(|| Throw::range_error("invalid smallestUnit"))? * inc as i128;
    let mut rounded = std_temporal_round_integer(time_ns, step, mode)?;
    let beyond = rounded - day_span;
    let mut result_date = *date;
    let nudged_ns;
    let expanded;
    if beyond.signum() != -(sign as i128) {
        expanded = true;
        result_date[3] += sign;
        rounded = std_temporal_round_integer(beyond, step, mode)?;
        nudged_ns = end_ns + rounded;
    } else {
        expanded = false;
        nudged_ns = start_ns + rounded;
    }
    Ok(StdNudge { date: result_date, time_ns: rounded, nudged_ns, expanded, total: None })
}

// BubbleRelativeDuration: after a calendar unit was rounded up, carry the
// increment into larger units while the nudged instant is at or past them.
fn zr_bubble(rel: &StdZonedRel, sign: i64, mut date: [i64; 10], mut time_ns: i128, nudged_ns: i128,
    start_local: &[i64; 10], largest: &str, start_unit: &str) -> Result<([i64; 10], i128), Throw> {
    if start_unit == largest { return Ok((date, time_ns)); }
    let largest_index = std_temporal_unit_rank(largest).unwrap_or(0) as i64;
    let mut unit_index = std_temporal_unit_rank(start_unit).unwrap_or(3) as i64 - 1;
    while unit_index >= largest_index {
        let index = unit_index as usize;
        if index != 2 || largest == "week" {
            let mut end_d = [0i64; 10];
            for k in 0..index { end_d[k] = date[k]; }
            end_d[index] = date[index] + sign;
            let end_ns = zr_resolve(rel, start_local, &end_d)?;
            let beyond = nudged_ns - end_ns;
            if beyond.signum() != -(sign as i128) {
                date = end_d;
                time_ns = 0;
            } else {
                break;
            }
        }
        unit_index -= 1;
    }
    Ok((date, time_ns))
}

fn zr_sign(date: &[i64; 10], time_ns: i128) -> i64 {
    for field in &date[..4] {
        if *field != 0 { return field.signum(); }
    }
    if time_ns != 0 { time_ns.signum() as i64 } else { 1 }
}

// RoundRelativeDuration with a time zone.
fn zr_round_relative(rel: &StdZonedRel, date: [i64; 10], time_ns: i128, dest_ns: i128, start_local: &[i64; 10],
    largest: &str, inc: i64, smallest: &str, mode: &str) -> Result<([i64; 10], i128), Throw> {
    let sign = zr_sign(&date, time_ns);
    let nudge = if matches!(smallest, "year" | "month" | "week" | "day") {
        zr_nudge_calendar(rel, sign, &date, dest_ns, start_local, inc, smallest, mode)?
    } else {
        zr_nudge_time(rel, sign, &date, time_ns, start_local, inc, smallest, mode)?
    };
    let (mut out_date, mut out_time) = (nudge.date, nudge.time_ns);
    if nudge.expanded && smallest != "week" {
        let start_unit = std_temporal_larger_unit(smallest, "day");
        (out_date, out_time) = zr_bubble(rel, sign, out_date, out_time, nudge.nudged_ns, start_local, largest, &start_unit)?;
    }
    Ok((out_date, out_time))
}

// DifferenceZonedDateTimeWithRounding.
fn zr_difference_rounded(rel: &StdZonedRel, ns1: i128, ns2: i128, largest: &str, inc: i64, smallest: &str,
    mode: &str) -> Result<([i64; 10], i128), Throw> {
    if std_temporal_unit_rank(largest).is_some_and(|r| r >= 4) {
        let step = std_temporal_unit_ns(smallest).ok_or_else(|| Throw::range_error("invalid smallestUnit"))? * inc as i128;
        return Ok(([0; 10], std_temporal_round_integer(ns2 - ns1, step, mode)?));
    }
    let (date, time) = zr_difference(rel, ns1, ns2, largest)?;
    if smallest == "nanosecond" && inc == 1 { return Ok((date, time)); }
    let start_local = zr_local(&rel.zone, ns1)?;
    zr_round_relative(rel, date, time, ns2, &start_local, largest, inc, smallest, mode)
}

// TemporalDurationFromInternal: balance the exact part to hours (or the
// requested time unit) and keep the date part.
fn zr_duration_fields(date: &[i64; 10], time_ns: i128, largest: &str) -> StdDuration {
    let rank = std_temporal_unit_rank(largest).unwrap_or(4);
    let mut wide = std_temporal_split_duration_ns_wide(time_ns, rank.max(4));
    for k in 0..4 { wide[k] = date[k] as i128; }
    wide
}

// Duration.prototype.total with a ZonedDateTime relativeTo.
fn zr_total(rel: &StdZonedRel, date: &[i64; 10], clock_ns: i128, unit: &str) -> Result<f64, Throw> {
    let target = zr_add(rel, rel.epoch_ns, date, clock_ns)?;
    if std_temporal_unit_rank(unit).is_some_and(|r| r >= 4) {
        let unit_ns = std_temporal_unit_ns(unit).ok_or_else(|| Throw::range_error("invalid unit"))?;
        return Ok(std_temporal_exact_ratio(target - rel.epoch_ns, unit_ns));
    }
    let (diff_date, diff_time) = zr_difference(rel, rel.epoch_ns, target, unit)?;
    let start_local = zr_local(&rel.zone, rel.epoch_ns)?;
    let sign = zr_sign(&diff_date, diff_time);
    let nudge = zr_nudge_calendar(rel, sign, &diff_date, target, &start_local, 1, unit, "trunc")?;
    let (n, d) = nudge.total.unwrap_or((0, 1));
    Ok(std_temporal_exact_ratio(n, d))
}

// Snapshot of a ToRelativeTemporalObject property bag: reads `calendar` then
// the calendar fields in alphabetical order (PrepareCalendarFields), applying
// each field's own conversion at its read position, into a plain object the
// shared resolvers can re-read without further observable effects.
fn std_temporal_relative_snapshot(i: &mut Interpreter, bag: &JsValue) -> Result<JsValue, Throw> {
    let out = i.object_value();
    let calendar = i.get_prop(bag, "calendar")?;
    let calendar_id = std_temporal_resolve_calendar_id(i, &calendar)?;
    if !calendar.is_undefined() { i.set_prop(&out, "calendar", calendar)?; }
    let uses_era = std_calendar_uses_era(&calendar_id);
    for name in ["day", "era", "eraYear", "hour", "microsecond", "millisecond", "minute", "month", "monthCode",
        "nanosecond", "offset", "second", "timeZone", "year"] {
        if matches!(name, "era" | "eraYear") && !uses_era { continue; }
        let value = i.get_prop(bag, name)?;
        if value.is_undefined() { continue; }
        let converted = match name {
            "era" => JsValue::str(i.to_string(&value)?),
            "monthCode" => {
                let text = std_epoch_string_field(i, value)?;
                std_temporal_validate_month_code_syntax(&text)?;
                JsValue::str(text)
            }
            "offset" => {
                let text = std_epoch_string_field(i, value)?;
                std_epoch_parse_offset(&text)?;
                JsValue::str(text)
            }
            "timeZone" => {
                let zone = std_epoch_zone_value(i, &value)?;
                JsValue::str(zone)
            }
            _ => JsValue::num(std_temporal_number(i, value, None)? as f64),
        };
        i.set_prop(&out, name, converted)?;
    }
    Ok(out)
}

// Comparison of two durations under a ZonedDateTime relativeTo.
fn zr_compare(rel: &StdZonedRel, left: &[i64; 10], right: &[i64; 10]) -> Result<std::cmp::Ordering, Throw> {
    let clock = |d: &[i64; 10]| std_temporal_duration_subday_ns(d);
    let after = |d: &[i64; 10]| zr_add(rel, rel.epoch_ns, d, clock(d));
    Ok(after(left)?.cmp(&after(right)?))
}
