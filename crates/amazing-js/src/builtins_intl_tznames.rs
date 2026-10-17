// Portions ported from lucid-softworks/lumen (crates/lumen/src/intl/datetimeformat.rs `tz_display_name`)
// @ a0f0314de7835f76eaa913995489945aabebf345, MIT; the zone list is extended with the other common
// CLDR (en) metazone names and their daylight-time variants.
//
// English display names of IANA zones: (zones, standard long, daylight long, standard short, daylight short).
// An empty daylight name means the metazone has no daylight variant.

struct IntlTzName {
    zones: &'static [&'static str],
    long: (&'static str, &'static str),
    short: (&'static str, &'static str),
}

const INTL_TZ_NAMES: &[IntlTzName] = &[
    IntlTzName { zones: &["Europe/Vienna", "Europe/Berlin", "Europe/Paris", "Europe/Rome", "Europe/Madrid",
        "Europe/Amsterdam", "Europe/Brussels", "Europe/Prague", "Europe/Warsaw", "Europe/Budapest",
        "Europe/Stockholm", "Europe/Oslo", "Europe/Copenhagen", "Europe/Zurich", "Europe/Belgrade",
        "Europe/Zagreb", "Europe/Bratislava", "Europe/Ljubljana", "Europe/Luxembourg", "Europe/Malta",
        "Europe/Monaco", "Europe/Vaduz", "Europe/Andorra", "Europe/Tirane", "Europe/Sarajevo",
        "Europe/Skopje", "Europe/Podgorica", "Europe/San_Marino", "Europe/Vatican", "Europe/Gibraltar",
        "Africa/Tunis", "Africa/Algiers", "Africa/Ceuta"],
        long: ("Central European Standard Time", "Central European Summer Time"), short: ("", "") },
    IntlTzName { zones: &["Europe/Athens", "Europe/Helsinki", "Europe/Kyiv", "Europe/Kiev", "Europe/Bucharest",
        "Europe/Sofia", "Europe/Riga", "Europe/Tallinn", "Europe/Vilnius", "Europe/Chisinau", "Asia/Nicosia",
        "Africa/Cairo", "Africa/Tripoli"],
        long: ("Eastern European Standard Time", "Eastern European Summer Time"), short: ("", "") },
    IntlTzName { zones: &["Europe/Lisbon", "Atlantic/Madeira", "Atlantic/Canary", "Atlantic/Faroe"],
        long: ("Western European Standard Time", "Western European Summer Time"), short: ("", "") },
    IntlTzName { zones: &["Europe/London", "Europe/Belfast", "Europe/Jersey", "Europe/Guernsey", "Europe/Isle_of_Man"],
        long: ("Greenwich Mean Time", "British Summer Time"), short: ("GMT", "") },
    IntlTzName { zones: &["Europe/Dublin"], long: ("Greenwich Mean Time", "Irish Standard Time"), short: ("GMT", "") },
    IntlTzName { zones: &["Europe/Moscow"], long: ("Moscow Standard Time", "Moscow Summer Time"), short: ("", "") },
    IntlTzName { zones: &["America/New_York", "America/Toronto", "America/Detroit", "America/Nassau",
        "America/Indiana/Indianapolis", "America/Kentucky/Louisville"],
        long: ("Eastern Standard Time", "Eastern Daylight Time"), short: ("EST", "EDT") },
    IntlTzName { zones: &["America/Chicago", "America/Winnipeg", "America/Menominee", "America/Indiana/Knox"],
        long: ("Central Standard Time", "Central Daylight Time"), short: ("CST", "CDT") },
    IntlTzName { zones: &["America/Mexico_City", "America/Regina", "America/Costa_Rica", "America/Guatemala"],
        long: ("Central Standard Time", "Central Daylight Time"), short: ("CST", "CDT") },
    IntlTzName { zones: &["America/Denver", "America/Edmonton", "America/Boise", "America/Phoenix"],
        long: ("Mountain Standard Time", "Mountain Daylight Time"), short: ("MST", "MDT") },
    IntlTzName { zones: &["America/Los_Angeles", "America/Vancouver", "America/Tijuana"],
        long: ("Pacific Standard Time", "Pacific Daylight Time"), short: ("PST", "PDT") },
    IntlTzName { zones: &["America/Anchorage", "America/Juneau", "America/Nome"],
        long: ("Alaska Standard Time", "Alaska Daylight Time"), short: ("AKST", "AKDT") },
    IntlTzName { zones: &["Pacific/Honolulu"], long: ("Hawaii-Aleutian Standard Time", "Hawaii-Aleutian Daylight Time"),
        short: ("HST", "") },
    IntlTzName { zones: &["America/Halifax", "America/Puerto_Rico", "Atlantic/Bermuda"],
        long: ("Atlantic Standard Time", "Atlantic Daylight Time"), short: ("AST", "ADT") },
    IntlTzName { zones: &["America/St_Johns"], long: ("Newfoundland Standard Time", "Newfoundland Daylight Time"),
        short: ("", "") },
    IntlTzName { zones: &["America/Sao_Paulo"], long: ("Brasilia Standard Time", "Brasilia Summer Time"), short: ("", "") },
    IntlTzName { zones: &["America/Argentina/Buenos_Aires"], long: ("Argentina Standard Time", "Argentina Summer Time"),
        short: ("", "") },
    IntlTzName { zones: &["Asia/Tokyo"], long: ("Japan Standard Time", "Japan Daylight Time"), short: ("", "") },
    IntlTzName { zones: &["Asia/Seoul"], long: ("Korean Standard Time", "Korean Daylight Time"), short: ("", "") },
    IntlTzName { zones: &["Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin"],
        long: ("China Standard Time", "China Daylight Time"), short: ("", "") },
    IntlTzName { zones: &["Asia/Hong_Kong"], long: ("Hong Kong Standard Time", "Hong Kong Summer Time"), short: ("", "") },
    IntlTzName { zones: &["Asia/Kolkata", "Asia/Calcutta"], long: ("India Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Asia/Dubai", "Asia/Muscat"], long: ("Gulf Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Asia/Singapore"], long: ("Singapore Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Asia/Jerusalem", "Asia/Tel_Aviv"], long: ("Israel Standard Time", "Israel Daylight Time"),
        short: ("", "") },
    IntlTzName { zones: &["Australia/Sydney", "Australia/Melbourne", "Australia/Hobart", "Australia/Canberra"],
        long: ("Australian Eastern Standard Time", "Australian Eastern Daylight Time"), short: ("", "") },
    IntlTzName { zones: &["Australia/Brisbane"], long: ("Australian Eastern Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Australia/Perth"], long: ("Australian Western Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Pacific/Auckland"], long: ("New Zealand Standard Time", "New Zealand Daylight Time"),
        short: ("", "") },
    IntlTzName { zones: &["Africa/Johannesburg"], long: ("South Africa Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Africa/Lagos"], long: ("West Africa Standard Time", ""), short: ("", "") },
    IntlTzName { zones: &["Africa/Nairobi"], long: ("East Africa Time", ""), short: ("", "") },
];

/// A localized English zone name for `style`, or None when the zone has no known name (the caller then
/// falls back to a GMT offset). `daylight` selects the daylight-time variant.
fn intl_tz_display_name(zone: &str, style: &str, daylight: bool) -> Option<String> {
    let entry = INTL_TZ_NAMES.iter().find(|entry| entry.zones.contains(&zone))?;
    let pick = |names: (&'static str, &'static str)| {
        let name = if daylight { names.1 } else { names.0 };
        if name.is_empty() { None } else { Some(name) }
    };
    match style {
        "long" => pick(entry.long).map(str::to_string),
        "short" => pick(entry.short).map(str::to_string),
        "longGeneric" => {
            let name = entry.long.0;
            let generic = name.replace(" Standard Time", " Time").replace(" Time Time", " Time");
            Some(if generic == name { name.to_string() } else { generic })
        }
        "shortGeneric" => {
            let short = entry.short.0;
            if short.len() == 3 && short.ends_with("ST") { Some(format!("{}T", &short[..1])) } else { None }
        }
        _ => None,
    }
}
