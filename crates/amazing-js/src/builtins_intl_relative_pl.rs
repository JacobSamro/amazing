// Portions ported from lucid-softworks/lumen (crates/lumen/src/intl/relativetimeformat.rs,
// `unit_word_pl`) @ a0f0314de7835f76eaa913995489945aabebf345, MIT. Polish relative-time
// phrases (CLDR 4-category cardinal plurals); numeric "auto" phrases added from CLDR.

/// (one, few, many, other) unit words for Polish `style` (long, short, narrow).
fn intl_relative_pl_words(unit: &str, style: &str) -> Option<[&'static str; 4]> {
    Some(match (style, unit) {
        ("long", "second") => ["sekundę", "sekundy", "sekund", "sekundy"],
        ("long", "minute") => ["minutę", "minuty", "minut", "minuty"],
        ("long", "hour") => ["godzinę", "godziny", "godzin", "godziny"],
        ("long", "day") => ["dzień", "dni", "dni", "dnia"],
        ("long", "week") => ["tydzień", "tygodnie", "tygodni", "tygodnia"],
        ("long", "month") => ["miesiąc", "miesiące", "miesięcy", "miesiąca"],
        ("long", "quarter") => ["kwartał", "kwartały", "kwartałów", "kwartału"],
        ("long", "year") => ["rok", "lata", "lat", "roku"],
        ("short", "second") => ["sek."; 4],
        ("short", "minute") => ["min"; 4],
        ("short", "hour") => ["godz."; 4],
        ("short", "day") => ["dzień", "dni", "dni", "dnia"],
        ("short", "week") => ["tydz.", "tyg.", "tyg.", "tyg."],
        ("short", "month") => ["mies."; 4],
        ("short", "quarter") => ["kw."; 4],
        ("short", "year") => ["rok", "lata", "lat", "roku"],
        ("narrow", "second") => ["s"; 4],
        ("narrow", "minute") => ["min"; 4],
        ("narrow", "hour") => ["g."; 4],
        ("narrow", "day") => ["dzień", "dni", "dni", "dnia"],
        ("narrow", "week") => ["tydz.", "tyg.", "tyg.", "tyg."],
        ("narrow", "month") => ["mies."; 4],
        ("narrow", "quarter") => ["kw."; 4],
        ("narrow", "year") => ["rok", "lata", "lat", "roku"],
        _ => return None,
    })
}

/// numeric:"auto" phrases (previous, current, next) for a Polish unit.
fn intl_relative_pl_auto(unit: &str) -> Option<[&'static str; 3]> {
    Some(match unit {
        "year" => ["w zeszłym roku", "w tym roku", "w przyszłym roku"],
        "quarter" => ["w zeszłym kwartale", "w tym kwartale", "w przyszłym kwartale"],
        "month" => ["w zeszłym miesiącu", "w tym miesiącu", "w przyszłym miesiącu"],
        "week" => ["w zeszłym tygodniu", "w tym tygodniu", "w przyszłym tygodniu"],
        "day" => ["wczoraj", "dzisiaj", "jutro"],
        "hour" => ["", "ta godzina", ""],
        "minute" => ["", "ta minuta", ""],
        "second" => ["", "teraz", ""],
        _ => return None,
    })
}

fn intl_relative_pl_parts(state: &IntlRelativeState, value: f64, unit: &str) -> Result<Vec<(&'static str, String, bool)>, Throw> {
    let words = intl_relative_pl_words(unit, &state.style)
        .ok_or_else(|| Throw::range_error("invalid relative time unit"))?;
    let negative = value < 0.0 || (value == 0.0 && value.is_sign_negative());
    if state.numeric == "auto" {
        if let Some([previous, current, next]) = intl_relative_pl_auto(unit) {
            let text = if value == 0.0 { current } else if value == 1.0 { next } else if value == -1.0 { previous } else { "" };
            if !text.is_empty() { return Ok(vec![("literal", text.to_string(), false)]); }
        }
    }
    let plural = IntlPluralState {
        locale: "pl".into(), kind: "cardinal".into(), notation: "standard".into(),
        compact_display: None, min_integer: 1, min_fraction: 0, max_fraction: 3,
        min_significant: None, max_significant: None, rounding_increment: 1,
        rounding_mode: "halfExpand".into(), rounding_priority: "auto".into(),
        trailing_zero_display: "auto".into(),
    };
    let word = match intl_plural_category(&plural, value.abs()) {
        "one" => words[0], "few" => words[1], "many" => words[2], _ => words[3],
    };
    let number_state = intl_relative_number_state(state);
    let (_, number_parts) = intl_format_number(&number_state, value.abs());
    let (before, after) = if negative { ("", format!(" {word} temu")) } else { ("za ", format!(" {word}")) };
    let mut parts = Vec::new();
    if !before.is_empty() { parts.push(("literal", before.to_string(), false)); }
    parts.extend(number_parts.into_iter().map(|(kind, text)| (kind, text, true)));
    parts.push(("literal", after, false));
    Ok(parts)
}
