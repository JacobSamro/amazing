// Intl.Locale information methods use pinned CLDR supplemental tables.
include!("builtins_intl_locale_data.rs");
include!("builtins_intl_likely_data_0.rs");
include!("builtins_intl_likely_data_1.rs");
include!("builtins_intl_likely_data_2.rs");

fn intl_locale_explicit_region(tag: &str) -> Option<String> {
    let base = tag.split("-u-").next().unwrap_or(tag);
    base.split('-').skip(1).find(|part| {
        (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
    }).map(str::to_ascii_uppercase)
}

fn intl_locale_region(tag: &str) -> &'static str {
    if let Some(override_region) = intl_unicode_keyword(tag, "rg") {
        let region = override_region.trim_end_matches("zzzz").to_ascii_uppercase();
        if let Some(data) = INTL_LOCALE_REGIONS.iter().find(|data| data.region == region) {
            return data.region;
        }
    }
    if let Some(region) = intl_locale_explicit_region(tag) {
        if let Some(data) = INTL_LOCALE_REGIONS.iter().find(|data| data.region == region) {
            return data.region;
        }
    }
    if let Some(subdivision) = intl_unicode_keyword(tag, "sd") {
        let region = subdivision.get(..2).unwrap_or("").to_ascii_uppercase();
        if let Some(data) = INTL_LOCALE_REGIONS.iter().find(|data| data.region == region) {
            return data.region;
        }
    }
    let language = tag.split('-').next().unwrap_or("und");
    if let Some(expanded) = intl_likely_tag(language) {
        return expanded.rsplit('-').next().unwrap_or("001");
    }
    "001"
}

fn intl_likely_tag(tag: &str) -> Option<&'static str> {
    for table in [INTL_LIKELY_REGIONS_0, INTL_LIKELY_REGIONS_1, INTL_LIKELY_REGIONS_2] {
        if let Some((_, expanded)) = table.iter().find(|(candidate, _)| *candidate == tag) {
            return Some(expanded);
        }
    }
    None
}

fn intl_locale_maximized_base(tag: &str) -> String {
    let base = tag.split("-u-").next().unwrap_or(tag);
    let parts: Vec<&str> = base.split('-').collect();
    let language = parts[0];
    let script = parts.iter().skip(1).find(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())).copied();
    let region = intl_locale_explicit_region(base);
    let mut keys = Vec::new();
    if let (Some(script), Some(region)) = (script, region.as_ref()) {
        keys.push(format!("{language}-{script}-{region}"));
    }
    if let Some(script) = script { keys.push(format!("{language}-{script}")); }
    if let Some(region) = region.as_ref() { keys.push(format!("{language}-{region}")); }
    keys.push(language.into());
    if let Some(script) = script { keys.push(format!("und-{script}")); }
    if let Some(region) = region.as_ref() { keys.push(format!("und-{region}")); }
    keys.push("und".into());
    let likely = keys.iter().find_map(|key| intl_likely_tag(key)).unwrap_or("en-Latn-US");
    let likely_parts: Vec<&str> = likely.split('-').collect();
    let out_language = if language == "und" { likely_parts[0] } else { language };
    let out_script = script.unwrap_or_else(|| likely_parts.get(1).copied().unwrap_or("Latn"));
    let out_region = region.as_deref().unwrap_or_else(|| likely_parts.last().copied().unwrap_or("US"));
    let mut output = format!("{out_language}-{out_script}-{out_region}");
    let mut index = 1;
    if parts.get(index).is_some_and(|part| part.len() == 4) { index += 1; }
    if parts.get(index).is_some_and(|part| part.len() == 2 || part.len() == 3) { index += 1; }
    for variant in &parts[index..] { output.push('-'); output.push_str(variant); }
    output
}

fn intl_locale_region_data(tag: &str) -> &'static IntlLocaleRegionData {
    let region = intl_locale_region(tag);
    INTL_LOCALE_REGIONS.iter().find(|data| data.region == region).unwrap_or(&INTL_LOCALE_REGIONS[0])
}

fn intl_locale_day(keyword: &str) -> Option<u8> {
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].iter()
        .position(|day| *day == keyword).map(|index| index as u8 + 1)
}

fn intl_locale_first_day_of_week(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    Ok(intl_unicode_keyword(&tag, "fw").map(JsValue::str).unwrap_or(JsValue::Undefined))
}

fn intl_locale_variants(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let mut parts = tag.split('-').skip(1).peekable();
    if parts.peek().is_some_and(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())) { parts.next(); }
    if parts.peek().is_some_and(|part| (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
        || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))) { parts.next(); }
    let variants = parts.take_while(|part| part.len() > 1).collect::<Vec<_>>();
    if variants.is_empty() { Ok(JsValue::Undefined) }
    else { Ok(JsValue::str(variants.join("-"))) }
}

fn intl_locale_get_calendars(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    if let Some(value) = intl_unicode_keyword(&tag, "ca") {
        return intl_array(i, [JsValue::str(value)]);
    }
    intl_array(i, intl_locale_region_data(&tag).calendars.iter().map(|value| JsValue::str(*value)))
}

fn intl_locale_get_collations(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    if let Some(collation) = intl_unicode_keyword(&tag, "co").filter(|value| !value.is_empty()) {
        return intl_array(i, [JsValue::str(collation)]);
    }
    let language = tag.split('-').next().unwrap_or("und");
    if language == "und" || !INTL_NUMBER_DATA.iter().any(|data| data.locale == language) {
        return intl_array(i, [JsValue::str("emoji"), JsValue::str("eor")]);
    }
    intl_array(i, [JsValue::str("emoji")])
}

fn intl_locale_get_hour_cycles(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    if let Some(value) = intl_unicode_keyword(&tag, "hc") {
        return intl_array(i, [JsValue::str(value)]);
    }
    let language = tag.split('-').next().unwrap_or("und");
    let region = intl_locale_region(&tag);
    let key = format!("{language}-{region}");
    let hours = INTL_LOCALE_HOUR_CYCLES.iter().find(|(locale, _)| *locale == key.as_str())
        .map(|(_, values)| *values).unwrap_or(intl_locale_region_data(&tag).hour_cycles);
    intl_array(i, hours.iter().map(|value| JsValue::str(*value)))
}

fn intl_locale_get_numbering_systems(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    if let Some(value) = intl_unicode_keyword(&tag, "nu") {
        return intl_array(i, [JsValue::str(value)]);
    }
    let locale = intl_lookup(&[tag], false);
    intl_array(i, [JsValue::str(intl_data(&locale).numbering_system)])
}

fn intl_locale_get_time_zones(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    if intl_locale_explicit_region(&tag).is_none() { return Ok(JsValue::Undefined); }
    let zones = intl_locale_region_data(&tag).time_zones.iter()
        .filter_map(|zone| tz_canonicalize(zone)).collect::<std::collections::BTreeSet<_>>();
    intl_array(i, zones.into_iter().map(JsValue::str))
}

fn intl_locale_get_text_info(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let locale = intl_lookup(&[tag], false);
    let direction = INTL_LOCALE_PREFERENCES.iter().find(|(name, _, _)| *name == locale.as_str())
        .map_or("ltr", |(_, _, direction)| *direction);
    let object = i.object_value();
    i.set_prop(&object, "direction", JsValue::str(direction))?;
    Ok(object)
}

fn intl_locale_get_week_info(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let data = intl_locale_region_data(&tag);
    let first = intl_unicode_keyword(&tag, "fw").as_deref().and_then(intl_locale_day)
        .unwrap_or(data.first_day);
    let mut weekend = Vec::new();
    let mut day = data.weekend_start;
    loop {
        weekend.push(day);
        if day == data.weekend_end { break; }
        day = day % 7 + 1;
    }
    weekend.sort_unstable();
    let object = i.object_value();
    i.set_prop(&object, "firstDay", JsValue::num(first as f64))?;
    let weekend = intl_array(i, weekend.into_iter().map(|day| JsValue::num(day as f64)))?;
    i.set_prop(&object, "weekend", weekend)?;
    Ok(object)
}
