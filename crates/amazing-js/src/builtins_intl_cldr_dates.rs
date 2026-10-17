// Portions ported from lucid-softworks/lumen (crates/lumen/src/cldr_dates.rs, crates/lumen/src/intl/datetimeformat.rs)
// @ a0f0314de7835f76eaa913995489945aabebf345, MIT.
// CLDR month and era names for the non-Gregorian calendars of Intl.DateTimeFormat, plus the calendar-field
// conversion that feeds them. The name tables live in builtins_intl_cldr_{months,eras}_N.rs.

include!("builtins_intl_cldr_months_0.rs");
include!("builtins_intl_cldr_months_1.rs");
include!("builtins_intl_cldr_eras_0.rs");
include!("builtins_intl_cldr_eras_1.rs");
include!("builtins_intl_cldr_eras_2.rs");

type IntlCldrMonthIndex = std::collections::HashMap<(&'static str, &'static str, &'static str, u8), &'static str>;
type IntlCldrEraIndex = std::collections::HashMap<(&'static str, &'static str, &'static str, &'static str), &'static str>;

fn intl_cldr_month_index() -> &'static IntlCldrMonthIndex {
    static INDEX: std::sync::OnceLock<IntlCldrMonthIndex> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = IntlCldrMonthIndex::new();
        for table in [&INTL_CLDR_MONTHS_0[..], &INTL_CLDR_MONTHS_1[..]] {
            for (locale, calendar, width, month, name) in table {
                map.entry((*locale, *calendar, *width, *month)).or_insert(*name);
            }
        }
        map
    })
}

fn intl_cldr_era_index() -> &'static IntlCldrEraIndex {
    static INDEX: std::sync::OnceLock<IntlCldrEraIndex> = std::sync::OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = IntlCldrEraIndex::new();
        for table in [&INTL_CLDR_ERAS_0[..], &INTL_CLDR_ERAS_1[..], &INTL_CLDR_ERAS_2[..]] {
            for (locale, calendar, width, era, name) in table {
                map.entry((*locale, *calendar, *width, *era)).or_insert(*name);
            }
        }
        map
    })
}

/// The CLDR locale key whose name tables serve a resolved locale (zh splits by script).
fn intl_cldr_locale(locale: &str) -> &'static str {
    let mut parts = locale.split('-');
    let language = parts.next().unwrap_or("en");
    let hant = locale.contains("Hant") || locale.ends_with("-TW") || locale.ends_with("-HK") || locale.ends_with("-MO");
    match language {
        "de" => "de", "es" => "es", "fr" => "fr", "ja" => "ja", "ko" => "ko", "ru" => "ru", "ar" => "ar",
        "it" => "it", "nl" => "nl", "pt" => "pt",
        "zh" => if hant { "zh-Hant" } else { "zh-Hans" },
        _ => "en",
    }
}

fn intl_cldr_month_name(locale: &str, calendar: &str, width: &str, month: i64) -> Option<&'static str> {
    let index = intl_cldr_month_index();
    let key = intl_cldr_locale(locale);
    index.get(&(key, calendar, width, month as u8)).or_else(|| index.get(&("en", calendar, width, month as u8))).copied()
}

fn intl_cldr_era_name(locale: &str, calendar: &str, width: &str, era: &str) -> Option<&'static str> {
    let index = intl_cldr_era_index();
    let key = intl_cldr_locale(locale);
    index.get(&(key, calendar, width, era)).or_else(|| index.get(&("en", calendar, width, era))).copied()
}

/// A civil date expressed in a display calendar.
struct IntlCalendarFields {
    year: i64,
    /// CLDR month index (hebrew skips 6 in common years; chinese/dangi use the month number).
    month: i64,
    day: i64,
    leap_month: bool,
    /// The calendar key and CLDR era key used for the era name, when the calendar has eras.
    era: Option<(&'static str, String)>,
    /// Related Gregorian year of a chinese/dangi year.
    related_year: Option<i64>,
    /// Second Adar of a Hebrew leap year (CLDR's leap-year variant of month 7).
    adar_ii: bool,
}

fn intl_calendar_is_foreign(calendar: &str) -> bool {
    !matches!(calendar, "" | "gregory" | "iso8601")
}

fn intl_era_cldr_key(calendar: &str, code: &str) -> (&'static str, String) {
    let gregorian = |code: &str| ("gregory", if code == "bce" { "0" } else { "1" }.to_string());
    match calendar {
        "gregory" => gregorian(code),
        "japanese" => match code {
            "meiji" => ("japanese", "232".into()), "taisho" => ("japanese", "233".into()),
            "showa" => ("japanese", "234".into()), "heisei" => ("japanese", "235".into()),
            "reiwa" => ("japanese", "236".into()),
            _ => gregorian(code),
        },
        "buddhist" => ("buddhist", "0".into()),
        "roc" => ("roc", if code == "broc" { "0" } else { "1" }.into()),
        "coptic" => ("coptic", "1".into()),
        "ethiopic" => ("ethiopic", if code == "aa" { "0" } else { "1" }.into()),
        "ethioaa" => ("ethioaa", "0".into()),
        "hebrew" => ("hebrew", "0".into()),
        "indian" => ("indian", "0".into()),
        "persian" => ("persian", "0".into()),
        other => (
            match other {
                "islamic" => "islamic", "islamic-civil" => "islamic-civil",
                "islamic-tbla" => "islamic-tbla", _ => "islamic-umalqura",
            },
            if code == "bh" { "1" } else { "0" }.into(),
        ),
    }
}

/// Calendar fields for an ISO civil date; the Gregorian calendar reports its era only.
fn intl_calendar_fields(calendar: &str, iso_year: i64, iso_month: i64, iso_day: i64) -> IntlCalendarFields {
    if !intl_calendar_is_foreign(calendar) {
        let era = std_calendar_era("gregory", iso_year, iso_month, iso_day);
        return IntlCalendarFields {
            year: iso_year, month: iso_month, day: iso_day, leap_month: false,
            era: era.0.map(|code| intl_era_cldr_key("gregory", code)), related_year: None, adar_ii: false,
        };
    }
    let fields = std_calendar_fields(calendar, iso_year, iso_month, iso_day);
    let (mut year, mut month, day) = (fields.0, fields.1, fields.2);
    let mut leap_month = false;
    let mut related_year = None;
    let mut era = None;
    let mut adar_ii = false;
    if calendar == "chinese" || calendar == "dangi" {
        let china = std_calendar_china_fields(calendar, iso_year, iso_month, iso_day);
        month = china.2;
        leap_month = china.3;
        related_year = Some(china.0);
        year = china.0;
    } else {
        let (code, era_year) = std_calendar_era(calendar, iso_year, iso_month, iso_day);
        if let Some(code) = code { era = Some(intl_era_cldr_key(calendar, code)); }
        if let Some(era_year) = era_year { year = era_year; }
        if calendar == "hebrew" {
            if !std_calendar_hebrew_leap(year) { if month >= 6 { month += 1; } } else if month == 7 { adar_ii = true; }
        }
    }
    IntlCalendarFields { year, month, day, leap_month, era, related_year, adar_ii }
}

/// The sexagenary cycle name of a chinese/dangi related year.
fn intl_cycle_year_name(related_year: i64, locale: &str) -> String {
    const STEMS: [&str; 10] = ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"];
    const BRANCHES: [&str; 12] = ["子", "丑", "寅", "卯", "辰", "巳", "午", "未", "申", "酉", "戌", "亥"];
    const PINYIN_STEMS: [&str; 10] = ["jia", "yi", "bing", "ding", "wu", "ji", "geng", "xin", "ren", "gui"];
    const PINYIN_BRANCHES: [&str; 12] = ["zi", "chou", "yin", "mao", "chen", "si", "wu", "wei", "shen", "you", "xu", "hai"];
    let stem = (related_year - 4).rem_euclid(10) as usize;
    let branch = (related_year - 4).rem_euclid(12) as usize;
    if intl_cldr_locale(locale).starts_with("zh") {
        format!("{}{}", STEMS[stem], BRANCHES[branch])
    } else {
        format!("{}-{}", PINYIN_STEMS[stem], PINYIN_BRANCHES[branch])
    }
}
