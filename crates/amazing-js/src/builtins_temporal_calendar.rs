// Portions ported from lucid-softworks/lumen (crates/lumen/src/temporal.rs) @
// f6c9860d44fa52249cfd647122edea184d32c863, MIT license. Ported: the non-ISO
// calendar math (calendar id canonicalization, era tables, the 13-month
// coptic/ethiopic/ethioaa calendar, the tabular islamic/islamic-civil/
// islamic-tbla calendar, the indian (Saka) calendar, the persian (arithmetic
// Solar Hijri) calendar, the hebrew (lunisolar) calendar, and the chinese/
// dangi (astronomical lunisolar) calendar, plus the generic per-calendar
// dispatch (`std_calendar_fields`, `std_calendar_to_iso`, `std_calendar_add`,
// era/year/monthCode resolution) -- adapted onto this engine's own
// `std_temporal_epoch_day`/`std_temporal_civil_from_epoch_day`/
// `std_temporal_days_in_month`/`std_temporal_leap` (rather than lumen's
// equivalent `days_from_civil`/`civil_from_days`/`days_in_month`/`is_leap`,
// which compute the same values) and this engine's own `[i64; 10]` calendar
// fields / `Throw` error type in place of lumen's `IsoDate`/`Value`. No
// ICU4X or other external calendar library is used or depended on -- the
// astronomical constants (solar longitude series, new-moon series, ΔT
// polynomials) are the standard Meeus/Calendrical-Calculations formulas
// lumen itself implements from scratch.
//
// Umm al-Qura's year-start and month-length tables are ported separately in
// builtins_temporal_umalqura.rs; outside their AH1300–1600 coverage the
// calendar uses the tabular civil fallback, matching the upstream behavior.

mod std_calendar_umalqura { include!("builtins_temporal_umalqura.rs"); }

/// The calendar identifiers Temporal accepts (BCP-47 `ca` values), canonical
/// spelling. `check_calendar`'s CLDR aliases below map onto this list.
const STD_CALENDAR_IDS: &[&str] = &[
    "iso8601",
    "gregory",
    "buddhist",
    "chinese",
    "coptic",
    "dangi",
    "ethioaa",
    "ethiopic",
    "hebrew",
    "indian",
    "islamic-umalqura",
    "islamic-tbla",
    "islamic-civil",
    "japanese",
    "persian",
    "roc",
];

/// Canonicalizes a calendar identifier (case-insensitively), resolving the
/// documented CLDR aliases ("islamicc" -> "islamic-civil", "gregorian" ->
/// "gregory", "ethiopic-amete-alem" -> "ethioaa"). Returns `None` for
/// anything not in `STD_CALENDAR_IDS`.
fn std_calendar_canonicalize(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "islamicc" => "islamic-civil",
        "ethiopic-amete-alem" => "ethioaa",
        "gregorian" => "gregory",
        other => other,
    };
    STD_CALENDAR_IDS.iter().copied().find(|k| *k == mapped)
}

/// Whether a calendar numbers years via eras (so an `era`/`eraYear` pair, not
/// just `year`, can resolve the year during construction).
fn std_calendar_uses_era(cal: &str) -> bool {
    matches!(cal, "gregory" | "japanese" | "buddhist" | "roc")
        || std_calendar_is_13month(cal)
        || std_calendar_is_islamic(cal)
        || matches!(cal, "indian" | "hebrew" | "persian")
}

/// Whether `era` (already lowercased) is one this calendar defines.
fn std_calendar_valid_era(cal: &str, era: &str) -> bool {
    match cal {
        "gregory" => matches!(era, "ce" | "bce" | "gregory" | "gregory-inverse" | "ad" | "bc"),
        "japanese" => matches!(era, "ce" | "bce" | "gregory" | "gregory-inverse" | "ad" | "bc"
            | "reiwa" | "heisei" | "showa" | "taisho" | "meiji"),
        "buddhist" => era == "be",
        "roc" => matches!(era, "roc" | "roc-inverse" | "broc" | "minguo" | "before-roc"),
        "coptic" => era == "am",
        "ethiopic" => matches!(era, "am" | "aa" | "mundi" | "incar"),
        "ethioaa" => matches!(era, "aa" | "mundi"),
        _ if std_calendar_is_islamic(cal) => matches!(era, "ah" | "bh"),
        "indian" => era == "shaka",
        "hebrew" => era == "am",
        "persian" => era == "ap",
        _ => false,
    }
}

/// Back-converts an already-validated (era, eraYear) pair to the calendar-
/// NATIVE year -- the same numbering `std_calendar_year_num`/
/// `std_calendar_fields` report, and what `std_temporal_bag`/`std_temporal_
/// with` store into `fields[0]` before any further year-to-ISO conversion
/// (`std_calendar_year_to_iso` for the ISO-grid family,
/// `std_calendar_to_iso` for the true month-structure calendars). `era` must
/// already be lowercased and validated via `std_calendar_valid_era` --
/// this returns `None` only for an era this function doesn't yet have a
/// formula for (should not happen for a pre-validated era).
fn std_calendar_era_to_native_year(cal: &str, era: &str, era_year: i64) -> Option<i64> {
    match cal {
        "gregory" | "japanese" => match era {
            "ce" | "gregory" | "ad" => Some(era_year),
            "bce" | "gregory-inverse" | "bc" => Some(1 - era_year),
            "reiwa" => Some(2019 + era_year - 1),
            "heisei" => Some(1989 + era_year - 1),
            "showa" => Some(1926 + era_year - 1),
            "taisho" => Some(1912 + era_year - 1),
            "meiji" => Some(1868 + era_year - 1),
            _ => None,
        },
        "buddhist" if era == "be" => Some(era_year),
        "roc" => match era {
            "roc" | "minguo" => Some(era_year),
            "broc" | "before-roc" => Some(1 - era_year),
            _ => None,
        },
        "coptic" if era == "am" => Some(era_year),
        "ethiopic" => match era {
            "am" | "incar" => Some(era_year),
            "aa" | "mundi" => Some(era_year - 5500),
            _ => None,
        },
        "ethioaa" if matches!(era, "aa" | "mundi") => Some(era_year),
        _ if std_calendar_is_islamic(cal) => match era {
            "ah" => Some(era_year),
            "bh" => Some(1 - era_year),
            _ => None,
        },
        "indian" if era == "shaka" => Some(era_year),
        "hebrew" if era == "am" => Some(era_year),
        "persian" if era == "ap" => Some(era_year),
        _ => None,
    }
}

/// Back-converts a calendar-native `year` field to the ISO year, for the
/// calendars that share the ISO year/month/day grid.
fn std_calendar_year_to_iso(cal: &str, year: i64) -> i64 {
    match cal {
        "buddhist" => year - 543,
        "roc" => year + 1911,
        _ => year,
    }
}

/// The (era code, era year) Temporal reports for a calendar at an ISO date.
/// `(None, None)` for iso8601 (no era) and for chinese/dangi (also no era).
fn std_calendar_era(cal: &str, year: i64, month: i64, day: i64) -> (Option<&'static str>, Option<i64>) {
    match cal {
        "gregory" => if year >= 1 { (Some("ce"), Some(year)) } else { (Some("bce"), Some(1 - year)) },
        "japanese" => std_calendar_japanese_era(year, month, day),
        "buddhist" => (Some("be"), Some(year + 543)),
        "roc" => {
            let y = year - 1911;
            if y >= 1 { (Some("roc"), Some(y)) } else { (Some("broc"), Some(1 - y)) }
        }
        "coptic" => (Some("am"), Some(std_calendar_from_13month(year, month, day, STD_CALENDAR_COPTIC_EPOCH).0)),
        "ethiopic" => {
            let am = std_calendar_from_13month(year, month, day, STD_CALENDAR_ETHIOPIC_EPOCH).0;
            if am >= 1 { (Some("am"), Some(am)) } else { (Some("aa"), Some(am + 5500)) }
        }
        "ethioaa" => (Some("aa"), Some(std_calendar_from_13month(year, month, day, STD_CALENDAR_ETHIOPIC_EPOCH).0 + 5500)),
        _ if std_calendar_is_islamic(cal) => {
            let y = std_calendar_islamic_from_iso(cal, year, month, day).0;
            if y >= 1 { (Some("ah"), Some(y)) } else { (Some("bh"), Some(1 - y)) }
        }
        "indian" => (Some("shaka"), Some(std_calendar_from_indian(year, month, day).0)),
        "hebrew" => (Some("am"), Some(std_calendar_hebrew_from_iso(year, month, day).0)),
        "persian" => (Some("ap"), Some(std_calendar_from_persian(year, month, day).0)),
        _ => (None, None),
    }
}

/// The Japanese era in effect on an ISO date (1-based era year); pre-Meiji
/// falls back to the proleptic gregorian ce/bce eras, matching ICU/CLDR.
fn std_calendar_japanese_era(year: i64, month: i64, day: i64) -> (Option<&'static str>, Option<i64>) {
    let e = std_temporal_epoch_day(year, month, day);
    let start = std_temporal_epoch_day;
    if e >= start(2019, 5, 1) { (Some("reiwa"), Some(year - 2019 + 1)) }
    else if e >= start(1989, 1, 8) { (Some("heisei"), Some(year - 1989 + 1)) }
    else if e >= start(1926, 12, 25) { (Some("showa"), Some(year - 1926 + 1)) }
    else if e >= start(1912, 7, 30) { (Some("taisho"), Some(year - 1912 + 1)) }
    else if e >= start(1873, 1, 1) { (Some("meiji"), Some(year - 1868 + 1)) }
    else if year >= 1 { (Some("ce"), Some(year)) }
    else { (Some("bce"), Some(1 - year)) }
}

// ===== 13-month calendars (coptic, ethiopic, ethioaa) =========================================
// epoch_days (from 1970-01-01, proleptic Gregorian) of each calendar's year 1-01-01.
const STD_CALENDAR_COPTIC_EPOCH: i64 = -615558; // Coptic 1-01-01 = Julian 284-08-29
const STD_CALENDAR_ETHIOPIC_EPOCH: i64 = -716367; // Ethiopic (amete mihret) 1-01-01 = Julian 8-08-29

fn std_calendar_is_13month(cal: &str) -> bool { matches!(cal, "coptic" | "ethiopic" | "ethioaa") }
fn std_calendar_13month_epoch(cal: &str) -> i64 {
    if cal == "coptic" { STD_CALENDAR_COPTIC_EPOCH } else { STD_CALENDAR_ETHIOPIC_EPOCH }
}

/// A 13-month (12x30 days + a 5/6-day epagomenal month) calendar date from an
/// ISO date: (year, month, day).
fn std_calendar_from_13month(year: i64, month: i64, day: i64, epoch: i64) -> (i64, i64, i64) {
    let ed = std_temporal_epoch_day(year, month, day);
    let d0 = ed - epoch;
    let cal_year = (4 * d0 + 1463).div_euclid(1461);
    let year_start = epoch + 365 * (cal_year - 1) + cal_year.div_euclid(4);
    let doy = ed - year_start;
    (cal_year, doy.div_euclid(30) + 1, doy.rem_euclid(30) + 1)
}
/// The ISO (year, month, day) for a 13-month calendar's (year, month, day).
fn std_calendar_to_13month(year: i64, month: i64, day: i64, epoch: i64) -> (i64, i64, i64) {
    let ed = epoch + 365 * (year - 1) + year.div_euclid(4) + 30 * (month - 1) + (day - 1);
    std_temporal_civil_from_epoch_day(ed)
}

// ===== Tabular Islamic (islamic-civil / islamic-tbla) ==========================================
// epoch_days of AH 1-01-01: civil = Julian 622-07-16, astronomical (tbla) = 622-07-15.
// Umm al-Qura uses its observational calendar tables within their coverage.
const STD_CALENDAR_ISLAMIC_EPOCH: i64 = -492148;
fn std_calendar_is_islamic(cal: &str) -> bool { cal.starts_with("islamic") }
fn std_calendar_islamic_epoch(cal: &str) -> i64 {
    if cal == "islamic-tbla" { STD_CALENDAR_ISLAMIC_EPOCH - 1 } else { STD_CALENDAR_ISLAMIC_EPOCH }
}
fn std_calendar_islamic_leap(y: i64) -> bool { (14 + 11 * y).rem_euclid(30) < 11 }
fn std_calendar_islamic_month_len(m: i64, leap: bool) -> i64 {
    if m % 2 == 1 || (m == 12 && leap) { 30 } else { 29 }
}
fn std_calendar_from_islamic(year: i64, month: i64, day: i64, epoch: i64) -> (i64, i64, i64) {
    let fixed = std_temporal_epoch_day(year, month, day);
    let cal_year = (30 * (fixed - epoch) + 10646).div_euclid(10631);
    let year_start = epoch + (cal_year - 1) * 354 + (3 + 11 * cal_year).div_euclid(30);
    let mut rem = fixed - year_start;
    let leap = std_calendar_islamic_leap(cal_year);
    let mut m = 1;
    while m < 12 {
        let ml = std_calendar_islamic_month_len(m, leap);
        if rem < ml { break; }
        rem -= ml;
        m += 1;
    }
    (cal_year, m, rem + 1)
}
fn std_calendar_to_islamic(y: i64, m: i64, d: i64, epoch: i64) -> (i64, i64, i64) {
    let fixed = epoch + (y - 1) * 354 + (3 + 11 * y).div_euclid(30) + (m - 1) * 29 + m.div_euclid(2) + (d - 1);
    std_temporal_civil_from_epoch_day(fixed)
}
fn std_calendar_umalqura_index(year: i64) -> Option<usize> {
    if (std_calendar_umalqura::YEAR_START as i64..=std_calendar_umalqura::YEAR_END as i64).contains(&year) {
        Some((year - std_calendar_umalqura::YEAR_START as i64) as usize)
    } else { None }
}
fn std_calendar_umalqura_month_len(index: usize, month: i64) -> i64 {
    29 + ((std_calendar_umalqura::MONTH_LENGTHS[index] >> (12 - month)) & 1) as i64
}
/// Calendar-aware Islamic conversions. The Umm al-Qura table's end limit
/// includes every day of its final year; outside the table use civil math.
fn std_calendar_islamic_from_iso(cal: &str, year: i64, month: i64, day: i64) -> (i64, i64, i64) {
    if cal == "islamic-umalqura" {
        let since_origin = std_temporal_epoch_day(year, month, day) - std_calendar_umalqura::EPOCH_OFFSET;
        let starts = &std_calendar_umalqura::YEAR_STARTS;
        let end = starts[starts.len() - 1]
            + 348 + std_calendar_umalqura::MONTH_LENGTHS[starts.len() - 1].count_ones() as i64;
        if since_origin >= starts[0] && since_origin < end {
            let index = starts.partition_point(|start| *start <= since_origin) - 1;
            let mut remaining = since_origin - starts[index];
            let mut native_month = 1;
            while native_month < 12 {
                let length = std_calendar_umalqura_month_len(index, native_month);
                if remaining < length { break; }
                remaining -= length;
                native_month += 1;
            }
            return (std_calendar_umalqura::YEAR_START as i64 + index as i64, native_month, remaining + 1);
        }
    }
    std_calendar_from_islamic(year, month, day, std_calendar_islamic_epoch(cal))
}
fn std_calendar_islamic_to_iso(cal: &str, y: i64, m: i64, d: i64) -> (i64, i64, i64) {
    if cal == "islamic-umalqura" && (1..=12).contains(&m) {
        if let Some(index) = std_calendar_umalqura_index(y) {
            let preceding: i64 = (1..m).map(|month| std_calendar_umalqura_month_len(index, month)).sum();
            let epoch = std_calendar_umalqura::EPOCH_OFFSET + std_calendar_umalqura::YEAR_STARTS[index]
                + preceding + d - 1;
            return std_temporal_civil_from_epoch_day(epoch);
        }
    }
    std_calendar_to_islamic(y, m, d, std_calendar_islamic_epoch(cal))
}
fn std_calendar_islamic_month_len_for(cal: &str, y: i64, m: i64) -> i64 {
    if cal == "islamic-umalqura" && (1..=12).contains(&m) {
        if let Some(index) = std_calendar_umalqura_index(y) {
            return std_calendar_umalqura_month_len(index, m);
        }
    }
    std_calendar_islamic_month_len(m, std_calendar_islamic_leap(y))
}

// ===== Indian national (Saka) calendar ==========================================================
// A solar calendar whose year begins on Chaitra 1 (Gregorian Mar 22, or Mar 21 in a Gregorian
// leap year). Saka year = Gregorian year - 78 (or -79 before that year's Chaitra 1).
fn std_calendar_indian_month_len(m: i64, leap: bool) -> i64 {
    if m == 1 { if leap { 31 } else { 30 } } else if m <= 6 { 31 } else { 30 }
}
fn std_calendar_from_indian(year: i64, month: i64, day: i64) -> (i64, i64, i64) {
    let g = year;
    let fixed = std_temporal_epoch_day(year, month, day);
    let chaitra1 = |gy: i64| std_temporal_epoch_day(gy, 3, if std_temporal_leap(gy) { 21 } else { 22 });
    let (s, start) = if fixed >= chaitra1(g) { (g - 78, chaitra1(g)) } else { (g - 79, chaitra1(g - 1)) };
    let leap = std_temporal_leap(s + 78);
    let mut rem = fixed - start;
    let mut m = 1;
    while m < 12 {
        let ml = std_calendar_indian_month_len(m, leap);
        if rem < ml { break; }
        rem -= ml;
        m += 1;
    }
    (s, m, rem + 1)
}
fn std_calendar_to_indian(s: i64, m: i64, d: i64) -> (i64, i64, i64) {
    let gy = s + 78;
    let leap = std_temporal_leap(gy);
    let chaitra1 = std_temporal_epoch_day(gy, 3, if leap { 21 } else { 22 });
    let doy: i64 = (1..m).map(|mm| std_calendar_indian_month_len(mm, leap)).sum::<i64>() + (d - 1);
    std_temporal_civil_from_epoch_day(chaitra1 + doy)
}

// ===== Persian (Solar Hijri, arithmetic) ========================================================
// ICU's arithmetic Persian calendar (a 33-year leap cycle, exact at any year, matching CLDR).
fn std_calendar_persian_new_year(py: i64) -> i64 {
    const PERSIAN_EPOCH_ED: i64 = 1_948_320 - 2_440_588; // Julian Day 1948320 -> our own epoch day
    PERSIAN_EPOCH_ED + 365 * (py - 1) + (8 * py + 21).div_euclid(33)
}
fn std_calendar_persian_leap(py: i64) -> bool {
    std_calendar_persian_new_year(py + 1) - std_calendar_persian_new_year(py) == 366
}
fn std_calendar_persian_month_len(py: i64, m: i64) -> i64 {
    if m <= 6 { 31 } else if m <= 11 || std_calendar_persian_leap(py) { 30 } else { 29 }
}
fn std_calendar_from_persian(year: i64, month: i64, day: i64) -> (i64, i64, i64) {
    let ed = std_temporal_epoch_day(year, month, day);
    let mut py = year - 621;
    while std_calendar_persian_new_year(py) > ed { py -= 1; }
    while std_calendar_persian_new_year(py + 1) <= ed { py += 1; }
    let doy = ed - std_calendar_persian_new_year(py);
    let (m, mstart) = if doy < 186 { (doy / 31 + 1, (doy / 31) * 31) }
        else { let m = (doy - 186) / 30 + 7; (m, 186 + (m - 7) * 30) };
    (py, m, doy - mstart + 1)
}
fn std_calendar_to_persian(py: i64, m: i64, d: i64) -> (i64, i64, i64) {
    let offset = if m <= 7 { 31 * (m - 1) } else { 30 * (m - 1) + 6 };
    std_temporal_civil_from_epoch_day(std_calendar_persian_new_year(py) + offset + (d - 1))
}

// ===== Hebrew (lunisolar) calendar ==============================================================
// Reingold/Dershowitz arithmetic. Works in R.D. (days from proleptic-Gregorian 0001-01-01);
// R.D. = epoch_day + 719163 (epoch_day of 0001-01-01 is -719162).
const STD_CALENDAR_RD_OFFSET: i64 = 719163;
const STD_CALENDAR_HEBREW_EPOCH: i64 = -1373427; // R.D. of Hebrew 1-01-01 (Tishri 1, year 1)
fn std_calendar_hebrew_leap(y: i64) -> bool { (7 * y + 1).rem_euclid(19) < 7 }
fn std_calendar_hebrew_months_in_year(y: i64) -> i64 { if std_calendar_hebrew_leap(y) { 13 } else { 12 } }
fn std_calendar_hebrew_elapsed_days(y: i64) -> i64 {
    let months = (235 * y - 234).div_euclid(19);
    let parts = 12084 + 13753 * months;
    let day = 29 * months + parts.div_euclid(25920);
    if (3 * (day + 1)).rem_euclid(7) < 3 { day + 1 } else { day }
}
fn std_calendar_hebrew_new_year(y: i64) -> i64 {
    let ed = std_calendar_hebrew_elapsed_days(y);
    let corr = {
        let ny0 = std_calendar_hebrew_elapsed_days(y - 1);
        let ny2 = std_calendar_hebrew_elapsed_days(y + 1);
        if ny2 - ed == 356 { 2 } else if ed - ny0 == 382 { 1 } else { 0 }
    };
    STD_CALENDAR_HEBREW_EPOCH + ed + corr
}
fn std_calendar_hebrew_year_len(y: i64) -> i64 {
    std_calendar_hebrew_new_year(y + 1) - std_calendar_hebrew_new_year(y)
}
/// Length of the `ordinal`-th Hebrew month (Temporal numbering: 1=Tishri) in year `y`.
fn std_calendar_hebrew_month_len(y: i64, ordinal: i64) -> i64 {
    let leap = std_calendar_hebrew_leap(y);
    let yl = std_calendar_hebrew_year_len(y);
    match ordinal {
        1 => 30,
        2 => if yl % 10 == 5 { 30 } else { 29 },
        3 => if yl % 10 == 3 { 29 } else { 30 },
        4 => 29,
        5 => 30,
        6 => if leap { 30 } else { 29 },
        7 => if leap { 29 } else { 30 },
        n => {
            let m = if leap { n - 1 } else { n };
            if m % 2 == 1 { 30 } else { 29 }
        }
    }
}
fn std_calendar_hebrew_from_iso(year: i64, month: i64, day: i64) -> (i64, i64, i64) {
    let fixed = std_temporal_epoch_day(year, month, day) + STD_CALENDAR_RD_OFFSET;
    let mut y = ((fixed - STD_CALENDAR_HEBREW_EPOCH) as f64 / 365.25).floor() as i64 + 1;
    while std_calendar_hebrew_new_year(y) > fixed { y -= 1; }
    while std_calendar_hebrew_new_year(y + 1) <= fixed { y += 1; }
    let mut rem = fixed - std_calendar_hebrew_new_year(y);
    let mut m = 1;
    while m < std_calendar_hebrew_months_in_year(y) {
        let ml = std_calendar_hebrew_month_len(y, m);
        if rem < ml { break; }
        rem -= ml;
        m += 1;
    }
    (y, m, rem + 1)
}
fn std_calendar_hebrew_to_iso(y: i64, ordinal: i64, day: i64) -> (i64, i64, i64) {
    let mut fixed = std_calendar_hebrew_new_year(y) + (day - 1);
    for m in 1..ordinal { fixed += std_calendar_hebrew_month_len(y, m); }
    std_temporal_civil_from_epoch_day(fixed - STD_CALENDAR_RD_OFFSET)
}
/// The Hebrew monthCode for an ordinal month: the leap month (Adar I, ordinal
/// 6 in a leap year) is "M05L"; later leap-year months shift back by one so
/// codes stay stable across years.
fn std_calendar_hebrew_month_code(y: i64, ordinal: i64) -> String {
    if std_calendar_hebrew_leap(y) {
        if ordinal <= 5 { format!("M{ordinal:02}") }
        else if ordinal == 6 { "M05L".to_string() }
        else { format!("M{:02}", ordinal - 1) }
    } else {
        format!("M{ordinal:02}")
    }
}
/// The ordinal Hebrew month for a monthCode in year `y` (`None` if not valid that year).
fn std_calendar_hebrew_ord_from_code(y: i64, code: &str) -> Option<i64> {
    let leap = std_calendar_hebrew_leap(y);
    if code == "M05L" { return if leap { Some(6) } else { None }; }
    let n: i64 = code.strip_prefix('M')?.parse().ok()?;
    if !(1..=12).contains(&n) { return None; }
    Some(if leap && n >= 6 { n + 1 } else { n })
}

// ===== Astronomy (from scratch) for the Chinese/Dangi lunisolar calendars =======================
// Universal-time moments are real numbers of days from 1970-01-01T00:00Z (an "epoch-day moment").
const STD_CALENDAR_MEAN_TROPICAL_YEAR: f64 = 365.242189;
const STD_CALENDAR_MEAN_SYNODIC_MONTH: f64 = 29.530588861;

/// ΔT (TT - UT) in seconds, Espenak-Meeus polynomials by era.
fn std_calendar_delta_t_seconds(year: i64) -> f64 {
    let y = year as f64;
    if (2005..=2050).contains(&year) {
        let t = y - 2000.0;
        62.92 + 0.32217 * t + 0.005589 * t * t
    } else if (1986..2005).contains(&year) {
        let t = y - 2000.0;
        63.86 + 0.3345 * t - 0.060374 * t.powi(2) + 0.0017275 * t.powi(3)
            + 0.000651814 * t.powi(4) + 0.00002373599 * t.powi(5)
    } else if (1961..1986).contains(&year) {
        let t = y - 1975.0;
        45.45 + 1.067 * t - t * t / 260.0 - t.powi(3) / 718.0
    } else if (1941..1961).contains(&year) {
        let t = y - 1950.0;
        29.07 + 0.407 * t - t * t / 233.0 + t.powi(3) / 2547.0
    } else if (1920..1941).contains(&year) {
        let t = y - 1920.0;
        21.20 + 0.84493 * t - 0.076100 * t * t + 0.0020936 * t.powi(3)
    } else if (1900..1920).contains(&year) {
        let t = y - 1900.0;
        -2.79 + 1.494119 * t - 0.0598939 * t * t + 0.0061966 * t.powi(3) - 0.000197 * t.powi(4)
    } else if (1860..1900).contains(&year) {
        let t = y - 1860.0;
        7.62 + 0.5737 * t - 0.251754 * t * t + 0.01680668 * t.powi(3) - 0.0004473624 * t.powi(4)
            + t.powi(5) / 233174.0
    } else if (1800..1860).contains(&year) {
        let t = y - 1800.0;
        13.72 - 0.332447 * t + 0.0068612 * t * t + 0.0041116 * t.powi(3) - 0.00037436 * t.powi(4)
            + 0.0000121272 * t.powi(5) - 0.0000001699 * t.powi(6) + 0.000000000875 * t.powi(7)
    } else {
        let u = (y - 1820.0) / 100.0;
        -20.0 + 32.0 * u * u
    }
}
fn std_calendar_julian_centuries_tt(ed: f64) -> f64 {
    let (gy, _, _) = std_temporal_civil_from_epoch_day(ed.floor() as i64);
    let jd_tt = ed + 2440587.5 + std_calendar_delta_t_seconds(gy) / 86400.0;
    (jd_tt - 2451545.0) / 36525.0
}
/// The sun's apparent longitude (degrees) at a universal-time moment (Meeus/Calendrical-Calc series).
fn std_calendar_solar_longitude(ed: f64) -> f64 {
    let c = std_calendar_julian_centuries_tt(ed);
    const COEF: [f64; 49] = [
        403406., 195207., 119433., 112392., 3891., 2819., 1721., 660., 350., 334., 314., 268.,
        242., 234., 158., 132., 129., 114., 99., 93., 86., 78., 72., 68., 64., 46., 38., 37., 32.,
        29., 28., 27., 27., 25., 24., 21., 21., 20., 18., 17., 14., 13., 13., 13., 12., 10., 10.,
        10., 10.,
    ];
    const MULT: [f64; 49] = [
        0.9287892, 35999.1376958, 35999.4089666, 35998.7287385, 71998.20261, 71998.4403,
        36000.35726, 71997.4812, 32964.4678, -19.4410, 445267.1117, 45036.8840, 3.1008,
        22518.4434, -19.9739, 65928.9345, 9038.0293, 3034.7684, 33718.148, 3034.448, -2280.773,
        29929.992, 31556.493, 149.588, 9037.750, 107997.405, -4444.176, 151.771, 67555.316,
        31556.080, -4561.540, 107996.706, 1221.655, 62894.167, 31437.369, 14578.298, -31931.757,
        34777.243, 1221.999, 62894.511, -4442.039, 107997.909, 119.066, 16859.071, -4.578,
        26895.292, -39.127, 12297.536, 90073.778,
    ];
    const ADD: [f64; 49] = [
        270.54861, 340.19128, 63.91854, 331.26220, 317.843, 86.631, 240.052, 310.26, 247.23,
        260.87, 297.82, 343.14, 166.79, 81.53, 3.50, 132.75, 182.95, 162.03, 29.8, 266.4, 249.2,
        157.6, 257.8, 185.1, 69.9, 8.0, 197.1, 250.4, 65.3, 162.7, 341.5, 291.6, 98.5, 146.7,
        110.0, 5.2, 342.6, 230.9, 256.1, 45.3, 242.9, 115.2, 151.8, 285.3, 53.3, 126.6, 205.7,
        85.9, 146.1,
    ];
    let mut sum = 0.0;
    for i in 0..49 {
        sum += COEF[i] * (ADD[i] + MULT[i] * c).to_radians().sin();
    }
    let lambda = 282.7771834 + 36000.76953744 * c + 0.000005729577951308232 * sum;
    let aberration = 0.0000974 * (177.63 + 35999.01848 * c).to_radians().cos() - 0.005575;
    let a = 124.90 - 1934.134 * c + 0.002063 * c * c;
    let b = 201.11 + 72001.5377 * c + 0.00057 * c * c;
    let nutation = -0.004778 * a.to_radians().sin() - 0.0003667 * b.to_radians().sin();
    (lambda + aberration + nutation).rem_euclid(360.0)
}
/// The universal-time moment of the `n`th new moon after the epoch new moon (Meeus, ch. 49).
fn std_calendar_nth_new_moon(n: i64) -> f64 {
    let k = n as f64;
    let t = k / 1236.85;
    let jde = 2451550.09766 + STD_CALENDAR_MEAN_SYNODIC_MONTH * k + 0.00015437 * t * t
        - 0.000000150 * t.powi(3) + 0.00000000073 * t.powi(4);
    let e = 1.0 - 0.002516 * t - 0.0000074 * t * t;
    let m = (2.5534 + 29.10535670 * k - 0.0000014 * t * t - 0.00000011 * t.powi(3)).to_radians();
    let mp = (201.5643 + 385.81693528 * k + 0.0107582 * t * t + 0.00001238 * t.powi(3)
        - 0.000000058 * t.powi(4)).to_radians();
    let f = (160.7108 + 390.67050284 * k - 0.0016118 * t * t - 0.00000227 * t.powi(3)
        + 0.000000011 * t.powi(4)).to_radians();
    let om = (124.7746 - 1.56375588 * k + 0.0020672 * t * t + 0.00000215 * t.powi(3)).to_radians();
    let corr = -0.40720 * mp.sin() + 0.17241 * e * m.sin() + 0.01608 * (2.0 * mp).sin()
        + 0.01039 * (2.0 * f).sin() + 0.00739 * e * (mp - m).sin() - 0.00514 * e * (mp + m).sin()
        + 0.00208 * e * e * (2.0 * m).sin() - 0.00111 * (mp - 2.0 * f).sin()
        - 0.00057 * (mp + 2.0 * f).sin() + 0.00056 * e * (2.0 * mp + m).sin()
        - 0.00042 * (3.0 * mp).sin() + 0.00042 * e * (m + 2.0 * f).sin()
        + 0.00038 * e * (m - 2.0 * f).sin() - 0.00024 * e * (2.0 * mp - m).sin()
        - 0.00017 * om.sin() - 0.00007 * (mp + 2.0 * m).sin()
        + 0.00004 * (2.0 * mp - 2.0 * f).sin() + 0.00004 * (3.0 * m).sin()
        + 0.00003 * (mp + m - 2.0 * f).sin() + 0.00003 * (2.0 * mp + 2.0 * f).sin()
        - 0.00003 * (mp + m + 2.0 * f).sin() + 0.00003 * (mp - m + 2.0 * f).sin()
        - 0.00002 * (mp - m - 2.0 * f).sin() - 0.00002 * (3.0 * mp + m).sin()
        + 0.00002 * (4.0 * mp).sin();
    const ADDL: [(f64, f64, f64); 14] = [
        (0.000325, 299.77, 0.107408), (0.000165, 251.88, 0.016321), (0.000164, 251.83, 26.651886),
        (0.000126, 349.42, 36.412478), (0.000110, 84.66, 18.206239), (0.000062, 141.74, 53.303771),
        (0.000060, 207.14, 2.453732), (0.000056, 154.84, 7.306860), (0.000047, 34.52, 27.261239),
        (0.000042, 207.19, 0.121824), (0.000040, 291.34, 1.844379), (0.000037, 161.72, 24.198154),
        (0.000035, 239.56, 25.513099), (0.000023, 331.55, 3.592518),
    ];
    let mut add = 0.0;
    for (idx, (amp, c0, c1)) in ADDL.iter().enumerate() {
        let arg = if idx == 0 { c0 + c1 * k - 0.009173 * t * t } else { c0 + c1 * k };
        add += amp * arg.to_radians().sin();
    }
    let jde_new = jde + corr + add;
    let approx_ed = jde_new - 2440587.5;
    let (gy, _, _) = std_temporal_civil_from_epoch_day(approx_ed.floor() as i64);
    jde_new - std_calendar_delta_t_seconds(gy) / 86400.0 - 2440587.5
}
fn std_calendar_new_moon_before(ed: f64) -> f64 {
    let n0 = ((ed + 2440587.5 - 2451550.09766) / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64;
    let mut n = n0 + 2;
    while std_calendar_nth_new_moon(n) >= ed { n -= 1; }
    std_calendar_nth_new_moon(n)
}
fn std_calendar_new_moon_at_or_after(ed: f64) -> f64 {
    let n0 = ((ed + 2440587.5 - 2451550.09766) / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64;
    let mut n = n0 - 2;
    while std_calendar_nth_new_moon(n) < ed { n += 1; }
    std_calendar_nth_new_moon(n)
}
fn std_calendar_estimate_prior_solar_longitude(lambda: f64, t: f64) -> f64 {
    let rate = STD_CALENDAR_MEAN_TROPICAL_YEAR / 360.0;
    let tau = t - rate * (std_calendar_solar_longitude(t) - lambda).rem_euclid(360.0);
    let delta = (std_calendar_solar_longitude(tau) - lambda + 180.0).rem_euclid(360.0) - 180.0;
    t.min(tau - rate * delta)
}
/// UTC offset (fraction of a day) for the calendar's reference meridian in a Gregorian year.
fn std_calendar_china_zone(cal: &str, gy: i64) -> f64 {
    if cal == "dangi" {
        if gy < 1908 { 3809.0 / 450.0 / 24.0 } else { 9.0 / 24.0 }
    } else if gy < 1929 { 1397.0 / 180.0 / 24.0 } else { 8.0 / 24.0 }
}
fn std_calendar_midnight_china(cal: &str, day: i64) -> f64 {
    let (gy, _, _) = std_temporal_civil_from_epoch_day(day);
    day as f64 - std_calendar_china_zone(cal, gy)
}
fn std_calendar_china_day_of(cal: &str, ut: f64) -> i64 {
    let (gy, _, _) = std_temporal_civil_from_epoch_day(ut.floor() as i64);
    (ut + std_calendar_china_zone(cal, gy)).floor() as i64
}
fn std_calendar_current_major_solar_term(cal: &str, day: i64) -> i64 {
    let s = std_calendar_solar_longitude(std_calendar_midnight_china(cal, day));
    (2 + (s / 30.0).floor() as i64 - 1).rem_euclid(12) + 1
}
fn std_calendar_china_new_moon_before(cal: &str, day: i64) -> i64 {
    std_calendar_china_day_of(cal, std_calendar_new_moon_before(std_calendar_midnight_china(cal, day)))
}
fn std_calendar_china_new_moon_on_or_after(cal: &str, day: i64) -> i64 {
    std_calendar_china_day_of(cal, std_calendar_new_moon_at_or_after(std_calendar_midnight_china(cal, day)))
}
fn std_calendar_china_no_major_solar_term(cal: &str, m: i64) -> bool {
    std_calendar_current_major_solar_term(cal, m)
        == std_calendar_current_major_solar_term(cal, std_calendar_china_new_moon_on_or_after(cal, m + 1))
}
fn std_calendar_china_winter_solstice_before(cal: &str, day: i64) -> i64 {
    let approx = std_calendar_estimate_prior_solar_longitude(270.0, std_calendar_midnight_china(cal, day + 1));
    let mut d = approx.floor() as i64 - 1;
    while std_calendar_solar_longitude(std_calendar_midnight_china(cal, d + 1)) <= 270.0 { d += 1; }
    d
}
fn std_calendar_china_prior_leap_month(cal: &str, m_prime: i64, m: i64) -> bool {
    m >= m_prime
        && (std_calendar_china_no_major_solar_term(cal, m)
            || std_calendar_china_prior_leap_month(cal, m_prime, std_calendar_china_new_moon_before(cal, m)))
}
fn std_calendar_china_new_year_in_sui(cal: &str, day: i64) -> i64 {
    let s1 = std_calendar_china_winter_solstice_before(cal, day);
    let s2 = std_calendar_china_winter_solstice_before(cal, s1 + 370);
    let m12 = std_calendar_china_new_moon_on_or_after(cal, s1 + 1);
    let m13 = std_calendar_china_new_moon_on_or_after(cal, m12 + 1);
    let next_m11 = std_calendar_china_new_moon_before(cal, s2 + 1);
    if ((next_m11 - m12) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64 == 12
        && (std_calendar_china_no_major_solar_term(cal, m12) || std_calendar_china_no_major_solar_term(cal, m13))
    {
        std_calendar_china_new_moon_on_or_after(cal, m13 + 1)
    } else { m13 }
}
fn std_calendar_china_new_year_before(cal: &str, day: i64) -> i64 {
    let ny = std_calendar_china_new_year_in_sui(cal, day);
    if day >= ny { ny } else { std_calendar_china_new_year_in_sui(cal, day - 180) }
}
/// (year, ordinal-month [1..13], month-number [1..12], leap-month?, day) for
/// an ISO date in Chinese/Dangi.
fn std_calendar_china_fields(cal: &str, year: i64, month: i64, day: i64) -> (i64, i64, i64, bool, i64) {
    let d = std_temporal_epoch_day(year, month, day);
    let s1 = std_calendar_china_winter_solstice_before(cal, d);
    let s2 = std_calendar_china_winter_solstice_before(cal, s1 + 370);
    let m12 = std_calendar_china_new_moon_on_or_after(cal, s1 + 1);
    let next_m11 = std_calendar_china_new_moon_before(cal, s2 + 1);
    let leap_year = ((next_m11 - m12) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64 == 12;
    let m = std_calendar_china_new_moon_before(cal, d + 1);
    let elapsed = ((m - m12) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64;
    let adj = if leap_year && std_calendar_china_prior_leap_month(cal, m12, m) { 1 } else { 0 };
    let month_num = (elapsed - adj - 1).rem_euclid(12) + 1;
    let leap_month = leap_year
        && std_calendar_china_no_major_solar_term(cal, m)
        && !std_calendar_china_prior_leap_month(cal, m12, std_calendar_china_new_moon_before(cal, m));
    let ny = std_calendar_china_new_year_before(cal, d);
    let ordinal = ((m - ny) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64 + 1;
    let (cyear, _, _) = std_temporal_civil_from_epoch_day(ny);
    (cyear, ordinal, month_num, leap_month, d - m + 1)
}
fn std_calendar_china_new_year_of(cal: &str, year: i64) -> i64 {
    std_calendar_china_new_year_before(cal, std_temporal_epoch_day(year, 12, 31))
}
fn std_calendar_china_months_in_year(cal: &str, year: i64) -> i64 {
    let ny = std_calendar_china_new_year_of(cal, year);
    let ny2 = std_calendar_china_new_year_of(cal, year + 1);
    ((ny2 - ny) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64
}
fn std_calendar_china_month_start(cal: &str, year: i64, num: i64, leap: bool) -> Option<i64> {
    let mut m = std_calendar_china_new_year_of(cal, year);
    for _ in 0..14 {
        let (y, mo, d) = std_temporal_civil_from_epoch_day(m);
        let f = std_calendar_china_fields(cal, y, mo, d);
        if f.2 == num && f.3 == leap { return Some(m); }
        m = std_calendar_china_new_moon_on_or_after(cal, m + 1);
    }
    None
}
fn std_calendar_china_month_len_at(cal: &str, start: i64) -> i64 {
    std_calendar_china_new_moon_on_or_after(cal, start + 1) - start
}
fn std_calendar_china_ord_start(cal: &str, year: i64, ordinal: i64) -> i64 {
    let mut m = std_calendar_china_new_year_of(cal, year);
    for _ in 1..ordinal { m = std_calendar_china_new_moon_on_or_after(cal, m + 1); }
    m
}
fn std_calendar_china_ord_to_iso(cal: &str, year: i64, ordinal: i64, day: i64) -> (i64, i64, i64) {
    std_temporal_civil_from_epoch_day(std_calendar_china_ord_start(cal, year, ordinal) + day - 1)
}
fn std_calendar_china_ord_month_len(cal: &str, year: i64, ordinal: i64) -> i64 {
    std_calendar_china_month_len_at(cal, std_calendar_china_ord_start(cal, year, ordinal))
}
fn std_calendar_china_ord_of_start(cal: &str, year: i64, start: i64) -> i64 {
    let ny = std_calendar_china_new_year_of(cal, year);
    ((start - ny) as f64 / STD_CALENDAR_MEAN_SYNODIC_MONTH).round() as i64 + 1
}
/// The ordinal (1-based) of the month (month-number, leap?) in `year`,
/// constraining a missing leap month to its plain counterpart.
/// Returns (leap-existed?, ordinal).
fn std_calendar_china_resolve_month_ord(cal: &str, year: i64, num: i64, leap: bool) -> (bool, i64) {
    if let Some(s) = std_calendar_china_month_start(cal, year, num, leap) {
        (true, std_calendar_china_ord_of_start(cal, year, s))
    } else {
        let s = std_calendar_china_month_start(cal, year, num, false).unwrap();
        (false, std_calendar_china_ord_of_start(cal, year, s))
    }
}
/// The Chinese monthCode for a (month-number, leap?): "M04" or "M04L".
fn std_calendar_china_month_code(num: i64, leap: bool) -> String {
    if leap { format!("M{num:02}L") } else { format!("M{num:02}") }
}

// ===== Generic per-calendar dispatch =============================================================

/// Whether a calendar has its own month structure (not the Gregorian
/// months), so date arithmetic and field resolution must work in the
/// calendar's own terms rather than passing through the ISO month/day.
fn std_calendar_is_month_structure(cal: &str) -> bool {
    std_calendar_is_13month(cal)
        || std_calendar_is_islamic(cal)
        || matches!(cal, "indian" | "hebrew" | "persian" | "chinese" | "dangi")
}

/// Whether a calendar's month numbering can include a leap month (a
/// monthCode with an "L" suffix): hebrew (Adar I) and chinese/dangi.
fn std_calendar_has_leap_months(cal: &str) -> bool { matches!(cal, "hebrew" | "chinese" | "dangi") }

/// The ISO (year, month, day) for a month-structure calendar's own (year,
/// month, day) [ordinal month for hebrew/chinese/dangi].
fn std_calendar_to_iso(cal: &str, y: i64, m: i64, d: i64) -> (i64, i64, i64) {
    if std_calendar_is_13month(cal) { std_calendar_to_13month(if cal == "ethioaa" { y - 5500 } else { y }, m, d, std_calendar_13month_epoch(cal)) }
    else if std_calendar_is_islamic(cal) { std_calendar_islamic_to_iso(cal, y, m, d) }
    else if cal == "hebrew" { std_calendar_hebrew_to_iso(y, m, d) }
    else if cal == "persian" { std_calendar_to_persian(y, m, d) }
    else if cal == "chinese" || cal == "dangi" { std_calendar_china_ord_to_iso(cal, y, m, d) }
    else { std_calendar_to_indian(y, m, d) }
}
/// The length of month `m` in year `y` of a month-structure calendar.
fn std_calendar_month_len(cal: &str, y: i64, m: i64) -> i64 {
    if std_calendar_is_13month(cal) {
        if m <= 12 { 30 } else if y.rem_euclid(4) == 3 { 6 } else { 5 }
    } else if std_calendar_is_islamic(cal) { std_calendar_islamic_month_len_for(cal, y, m) }
    else if cal == "hebrew" { std_calendar_hebrew_month_len(y, m) }
    else if cal == "persian" { std_calendar_persian_month_len(y, m) }
    else if cal == "chinese" || cal == "dangi" { std_calendar_china_ord_month_len(cal, y, m) }
    else { std_calendar_indian_month_len(m, std_temporal_leap(y + 78)) }
}
/// Months in year `y` of a month-structure calendar (13 for coptic/
/// ethiopic/ethioaa, 12 or 13 for hebrew/chinese/dangi, else 12).
fn std_calendar_months_in_year(cal: &str, y: i64) -> i64 {
    if cal == "hebrew" { std_calendar_hebrew_months_in_year(y) }
    else if cal == "chinese" || cal == "dangi" { std_calendar_china_months_in_year(cal, y) }
    else if std_calendar_is_13month(cal) { 13 }
    else { 12 }
}
/// The calendar-year number a receiver's `.year` reports (iso8601/gregory/
/// japanese/roc-share the ISO year's numbering rules already folded into
/// `std_calendar_year_to_iso`'s inverse below; the others resolve their own
/// native year here).
fn std_calendar_year_num(cal: &str, year: i64, month: i64, day: i64) -> i64 {
    match cal {
        "buddhist" => year + 543,
        "roc" => year - 1911,
        "coptic" | "ethiopic" => std_calendar_from_13month(year, month, day, std_calendar_13month_epoch(cal)).0,
        "ethioaa" => std_calendar_from_13month(year, month, day, STD_CALENDAR_ETHIOPIC_EPOCH).0 + 5500,
        _ if std_calendar_is_islamic(cal) => std_calendar_islamic_from_iso(cal, year, month, day).0,
        "indian" => std_calendar_from_indian(year, month, day).0,
        "hebrew" => std_calendar_hebrew_from_iso(year, month, day).0,
        "persian" => std_calendar_from_persian(year, month, day).0,
        "chinese" | "dangi" => std_calendar_china_fields(cal, year, month, day).0,
        _ => year,
    }
}
/// The monthCode string for a date. Most calendars derive it from the
/// ordinal month; hebrew/chinese/dangi can report a leap-month code.
fn std_calendar_month_code(cal: &str, year: i64, month: i64, day: i64) -> String {
    if cal == "hebrew" {
        let (y, m, _) = std_calendar_hebrew_from_iso(year, month, day);
        std_calendar_hebrew_month_code(y, m)
    } else if cal == "chinese" || cal == "dangi" {
        let (_, _, num, leap, _) = std_calendar_china_fields(cal, year, month, day);
        std_calendar_china_month_code(num, leap)
    } else {
        format!("M{:02}", std_calendar_fields(cal, year, month, day).1)
    }
}
/// Full calendar fields for an ISO date: (year, month, day, daysInMonth,
/// monthsInYear, dayOfYear, daysInYear, inLeapYear). iso8601/gregory/
/// japanese/roc/buddhist keep the ISO month/day structure; the rest use
/// their own month-structure conversion.
fn std_calendar_fields(cal: &str, year: i64, month: i64, day: i64) -> (i64, i64, i64, i64, i64, i64, i64, bool) {
    if std_calendar_is_13month(cal) {
        let (y, m, d) = std_calendar_from_13month(year, month, day, std_calendar_13month_epoch(cal));
        let leap = y.rem_euclid(4) == 3;
        let dim = if m <= 12 { 30 } else if leap { 6 } else { 5 };
        let doy = 30 * (m - 1) + d;
        (if cal == "ethioaa" { y + 5500 } else { y }, m, d, dim, 13, doy, if leap { 366 } else { 365 }, leap)
    } else if std_calendar_is_islamic(cal) {
        let (y, m, d) = std_calendar_islamic_from_iso(cal, year, month, day);
        let dim = std_calendar_islamic_month_len_for(cal, y, m);
        let doy = (1..m).map(|mm| std_calendar_islamic_month_len_for(cal, y, mm)).sum::<i64>() + d;
        let year_len = (1..=12).map(|month| std_calendar_islamic_month_len_for(cal, y, month)).sum::<i64>();
        (y, m, d, dim, 12, doy, year_len, year_len == 355)
    } else if cal == "indian" {
        let (y, m, d) = std_calendar_from_indian(year, month, day);
        let leap = std_temporal_leap(y + 78);
        let dim = std_calendar_indian_month_len(m, leap);
        let doy = (1..m).map(|mm| std_calendar_indian_month_len(mm, leap)).sum::<i64>() + d;
        (y, m, d, dim, 12, doy, if leap { 366 } else { 365 }, leap)
    } else if cal == "persian" {
        let (y, m, d) = std_calendar_from_persian(year, month, day);
        let dim = std_calendar_persian_month_len(y, m);
        let leap = std_calendar_persian_leap(y);
        let doy = (1..m).map(|mm| std_calendar_persian_month_len(y, mm)).sum::<i64>() + d;
        (y, m, d, dim, 12, doy, if leap { 366 } else { 365 }, leap)
    } else if cal == "hebrew" {
        let (y, m, d) = std_calendar_hebrew_from_iso(year, month, day);
        let mpy = std_calendar_hebrew_months_in_year(y);
        let dim = std_calendar_hebrew_month_len(y, m);
        let doy = (1..m).map(|mm| std_calendar_hebrew_month_len(y, mm)).sum::<i64>() + d;
        (y, m, d, dim, mpy, doy, std_calendar_hebrew_year_len(y), std_calendar_hebrew_leap(y))
    } else if cal == "chinese" || cal == "dangi" {
        let (y, ord, _num, _leap, d) = std_calendar_china_fields(cal, year, month, day);
        let day_epoch = std_temporal_epoch_day(year, month, day);
        let start = std_calendar_china_new_moon_before(cal, day_epoch + 1);
        let dim = std_calendar_china_month_len_at(cal, start);
        let mpy = std_calendar_china_months_in_year(cal, y);
        let ny = std_calendar_china_new_year_of(cal, y);
        let doy = day_epoch - ny + 1;
        let diy = std_calendar_china_new_year_of(cal, y + 1) - ny;
        (y, ord, d, dim, mpy, doy, diy, mpy == 13)
    } else {
        let dim = std_temporal_days_in_month(year, month);
        let doy = std_temporal_epoch_day(year, month, day) - std_temporal_epoch_day(year, 1, 1) + 1;
        (std_calendar_year_num(cal, year, month, day), month, day, dim, 12, doy,
            if std_temporal_leap(year) { 366 } else { 365 }, std_temporal_leap(year))
    }
}

/// Resolve a monthCode to an ordinal month in a leap-month calendar (hebrew/
/// chinese/dangi), constraining an absent leap month to its plain
/// counterpart. Returns (ordinal, leap-existed?).
fn std_calendar_resolve_code_ord(cal: &str, year: i64, code: &str) -> Option<(i64, bool)> {
    if cal == "hebrew" {
        if let Some(o) = std_calendar_hebrew_ord_from_code(year, code) { return Some((o, true)); }
        // Only "M05L" (Adar I) is a real Hebrew monthCode that can be absent
        // in a particular (non-leap) year -- everything else that failed
        // above (out-of-range "M13", any OTHER "*L"-suffixed code, a
        // malformed shape) is never a real Hebrew monthCode in ANY year, so
        // it is a hard invalid code with no constrain-mode fallback.
        // Confirmed by invalid-month-codes-hebrew.js, which requires "M13"
        // and every "*L" code besides "M05L" to throw unconditionally in
        // both leap and common years.
        if code != "M05L" { return None; }
        let fallback = std_calendar_hebrew_ord_from_code(year, "M06")?;
        Some((fallback, false))
    } else {
        let body = code.strip_prefix('M')?;
        let (digits, leap) = match body.strip_suffix('L') { Some(d) => (d, true), None => (body, false) };
        if digits.len() != 2 || !digits.bytes().all(|b| b.is_ascii_digit()) { return None; }
        let num: i64 = digits.parse().ok()?;
        if !(1..=12).contains(&num) { return None; }
        let (ok, ord) = std_calendar_china_resolve_month_ord(cal, year, num, leap);
        Some((ord, ok))
    }
}
/// Advance an ordinal month by a signed `k` across years of a
/// varying-length (leap-month) calendar.
fn std_calendar_advance_ord(cal: &str, year: i64, ord: i64, k: i64) -> (i64, i64) {
    let (mut y, mut m) = (year, ord);
    if k >= 0 {
        for _ in 0..k {
            m += 1;
            if m > std_calendar_months_in_year(cal, y) { m = 1; y += 1; }
        }
    } else {
        for _ in 0..(-k) {
            m -= 1;
            if m < 1 { y -= 1; m = std_calendar_months_in_year(cal, y); }
        }
    }
    (y, m)
}

/// Renormalize a (year, possibly out-of-range month) pair into range,
/// walking year boundaries with the CALENDAR'S OWN months-per-year count
/// (varying for hebrew/chinese/dangi) rather than a fixed 12 -- the
/// month-structure generalization of std_temporal_balance_year_month
/// (builtins_temporal_arithmetic.rs), used by the since/until calendar-unit
/// bracket search (JSS-calendars item 2, std_temporal_calendar_difference_cal).
fn std_calendar_balance_year_month(cal: &str, year: i64, month: i64) -> (i64, i64) {
    if !std_calendar_is_month_structure(cal) { return std_temporal_balance_year_month(year, month); }
    let mut y = year;
    let mut m = month;
    while m < 1 {
        y -= 1;
        m += std_calendar_months_in_year(cal, y);
    }
    loop {
        let months_in_year = std_calendar_months_in_year(cal, y);
        if m <= months_in_year { break; }
        m -= months_in_year;
        y += 1;
    }
    (y, m)
}

/// Total signed month count from `from_year` to `to_year` (exclusive of
/// `to_year` itself, matching `(to_year - from_year) * 12`'s ISO-grid
/// meaning) -- the varying-months-per-year generalization needed to fold a
/// bracket-searched year count into a month count for hebrew/chinese/dangi,
/// where a plain `* 12` is wrong.
fn std_calendar_months_between(cal: &str, from_year: i64, to_year: i64) -> i64 {
    if !std_calendar_is_month_structure(cal) { return (to_year - from_year) * 12; }
    let mut total = 0i64;
    if to_year >= from_year {
        for y in from_year..to_year { total += std_calendar_months_in_year(cal, y); }
    } else {
        for y in to_year..from_year { total -= std_calendar_months_in_year(cal, y); }
    }
    total
}
