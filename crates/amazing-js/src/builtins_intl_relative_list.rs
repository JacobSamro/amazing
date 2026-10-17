// RelativeTimeFormat and ListFormat use generated CLDR phrases and patterns.

#[derive(Clone)]
struct IntlRelativeState { locale: String, resolved: String, style: String, numeric: String, numbering_system: String }

#[derive(Clone)]
struct IntlListState { locale: String, resolved: String, style: String, kind: String }

impl Interpreter {
    fn install_intl_relative_and_list(&mut self, namespace: &Obj) {
        let relative = Self::native_constructor("RelativeTimeFormat", intl_relative_ctor, &self.function_proto);
        let JsValue::Object(function) = &relative else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "RelativeTimeFormat", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(relative.clone()));
        intl_method(self, &prototype, "format", 2, intl_relative_format);
        intl_method(self, &prototype, "formatToParts", 2, intl_relative_format_to_parts);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_relative_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.RelativeTimeFormat")));
        intl_register_intrinsic(self, "RelativeTimeFormat", function, &prototype);
        let _ = namespace.borrow_mut().define("RelativeTimeFormat", binary_writable_property(relative));

        let list = Self::native_constructor("ListFormat", intl_list_ctor, &self.function_proto);
        let JsValue::Object(function) = &list else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "ListFormat", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(list.clone()));
        intl_method(self, &prototype, "format", 1, intl_list_format);
        intl_method(self, &prototype, "formatToParts", 1, intl_list_format_to_parts);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_list_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.ListFormat")));
        intl_register_intrinsic(self, "ListFormat", function, &prototype);
        let _ = namespace.borrow_mut().define("ListFormat", binary_writable_property(list));
    }
}

fn intl_relative_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("RelativeTimeFormat requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("RelativeTimeFormat requires an object")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let options = args.get(1).filter(|value| !value.is_undefined()).cloned().unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null RelativeTimeFormat options")); }
    let matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let locale = intl_lookup(&requested, matcher == "best fit");
    let numbering_default = intl_data(&locale).numbering_system;
    let numbering_raw = i.get_prop(&options, "numberingSystem")?;
    let numbering_option = if numbering_raw.is_undefined() { None } else { Some(i.to_string(&numbering_raw)?) };
    if let Some(value) = numbering_option.as_ref() {
        if !intl_is_well_formed_type(value) {
            return Err(Throw::range_error("invalid numbering system"));
        }
    }
    let nu_extension = requested.first().and_then(|tag| intl_unicode_keyword(tag, "nu"));
    let (nu_retained, numbering_system) =
        intl_resolve_keyword(nu_extension, numbering_option, intl_supported_numbering_system, numbering_default);
    let style = intl_string_option(i, &options, "style", &["long", "short", "narrow"], "long")?;
    let numeric = intl_string_option(i, &options, "numeric", &["always", "auto"], "always")?;
    let mut retained: Vec<(&str, String)> = Vec::new();
    if let Some(value) = nu_retained { retained.push(("nu", value)); }
    let resolved = intl_locale_with_extensions(&intl_resolved_locale(&requested, &locale), &retained);
    intl_register(&object, IntlState::Relative(IntlRelativeState { locale, resolved, style, numeric, numbering_system }));
    Ok(JsValue::Object(object))
}

fn intl_relative_state(this: &JsValue) -> Result<IntlRelativeState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible RelativeTimeFormat receiver")); };
    match intl_state(object) {
        Some(IntlState::Relative(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible RelativeTimeFormat receiver")),
    }
}

fn intl_relative_number_state(state: &IntlRelativeState) -> IntlNumberState {
    IntlNumberState {
        locale: state.locale.clone(), resolved_locale: state.locale.clone(),
        numbering_system: state.numbering_system.clone(),
        style: "decimal".into(), currency: None, currency_display: "symbol".into(),
        currency_sign: "standard".into(), unit: None, unit_display: "short".into(),
        notation: "standard".into(), compact_display: "short".into(), sign_display: "auto".into(),
        use_grouping: "auto".into(), min_integer: 1, min_fraction: 0, max_fraction: 3,
        min_significant: None, max_significant: None, rounding_priority: "auto".into(),
        rounding_mode: "halfExpand".into(), rounding_increment: 1, trailing_zero_display: "auto".into(),
    }
}

include!("builtins_intl_relative_pl.rs");

fn intl_relative_parts(state: &IntlRelativeState, value: f64, unit: &str) -> Result<Vec<(&'static str, String, bool)>, Throw> {
    if !value.is_finite() { return Err(Throw::range_error("non-finite relative time")); }
    let unit = unit.strip_suffix('s').unwrap_or(unit);
    if state.locale == "pl" { return intl_relative_pl_parts(state, value, unit); }
    let data = INTL_RELATIVE_DATA.iter().find(|data| data.locale == state.locale && data.unit == unit && data.style == state.style)
        .ok_or_else(|| Throw::range_error("invalid relative time unit"))?;
    let negative = value < 0.0 || (value == 0.0 && value.is_sign_negative());
    if state.numeric == "auto" {
        // Zero has no sign for the "current" text: +0 and -0 both read as
        // "now"/"today"/etc, even though -0 still formats as "ago" (negative)
        // once it falls through to the general plural-pattern path below.
        let text = if value == 0.0 { data.current }
            else if !negative && value == 1.0 { data.next }
            else if negative && value == -1.0 { data.previous } else { "" };
        if !text.is_empty() { return Ok(vec![("literal", text.to_string(), false)]); }
    }
    let plural = IntlPluralState {
        locale: state.locale.clone(), kind: "cardinal".into(), notation: "standard".into(),
        compact_display: None, min_integer: 1, min_fraction: 0, max_fraction: 3,
        min_significant: None, max_significant: None, rounding_increment: 1,
        rounding_mode: "halfExpand".into(), rounding_priority: "auto".into(),
        trailing_zero_display: "auto".into(),
    };
    let category = intl_plural_category(&plural, value.abs());
    let pattern = match (negative, category == "one") {
        (true, true) => data.past_one, (true, false) => data.past_other,
        (false, true) => data.future_one, (false, false) => data.future_other,
    };
    let number_state = intl_relative_number_state(state);
    let (_, number_parts) = intl_format_number(&number_state, value.abs());
    let Some((before, after)) = pattern.split_once("{0}") else {
        return Ok(vec![("literal", pattern.to_string(), false)]);
    };
    let mut parts = Vec::new();
    if !before.is_empty() { parts.push(("literal", before.to_string(), false)); }
    parts.extend(number_parts.into_iter().map(|(kind, text)| (kind, text, true)));
    if !after.is_empty() { parts.push(("literal", after.to_string(), false)); }
    Ok(parts)
}

fn intl_relative_arguments(i: &mut Interpreter, args: &[JsValue]) -> Result<(f64, String), Throw> {
    let value = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let unit = i.to_string(args.get(1).unwrap_or(&JsValue::Undefined))?;
    Ok((value, unit))
}

fn intl_relative_format(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_relative_state(&this)?;
    let (value, unit) = intl_relative_arguments(i, &args)?;
    Ok(JsValue::str(intl_relative_parts(&state, value, &unit)?.into_iter().map(|(_, text, _)| text).collect::<String>()))
}

fn intl_relative_format_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_relative_state(&this)?;
    let (value, unit) = intl_relative_arguments(i, &args)?;
    let unit_name = unit.strip_suffix('s').unwrap_or(&unit).to_string();
    let parts = intl_relative_parts(&state, value, &unit)?;
    let mut values = Vec::new();
    for (kind, text, carries_unit) in parts {
        let part = i.object_value();
        i.set_prop(&part, "type", JsValue::str(kind))?;
        i.set_prop(&part, "value", JsValue::str(text))?;
        if carries_unit { i.set_prop(&part, "unit", JsValue::str(unit_name.clone()))?; }
        values.push(part);
    }
    intl_array(i, values)
}

fn intl_relative_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_relative_state(&this)?;
    let object = i.object_value();
    i.set_prop(&object, "locale", JsValue::str(state.resolved.as_str()))?;
    i.set_prop(&object, "style", JsValue::str(state.style))?;
    i.set_prop(&object, "numeric", JsValue::str(state.numeric))?;
    i.set_prop(&object, "numberingSystem", JsValue::str(state.numbering_system.as_str()))?;
    Ok(object)
}

fn intl_list_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("ListFormat requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("ListFormat requires an object")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let options = match args.get(1) {
        None | Some(JsValue::Undefined) => intl_empty_options(),
        Some(JsValue::Object(_)) => args[1].clone(),
        // InitializeListFormat requires an object options argument.
        Some(_) => return Err(Throw::type_error("ListFormat options must be an object")),
    };
    let matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let locale = intl_lookup(&requested, matcher == "best fit");
    let kind = intl_string_option(i, &options, "type", &["conjunction", "disjunction", "unit"], "conjunction")?;
    let style = intl_string_option(i, &options, "style", &["long", "short", "narrow"], "long")?;
    let resolved = intl_resolved_locale(&requested, &locale);
    intl_register(&object, IntlState::List(IntlListState { locale, resolved, style, kind }));
    Ok(JsValue::Object(object))
}

fn intl_list_state(this: &JsValue) -> Result<IntlListState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible ListFormat receiver")); };
    match intl_state(object) {
        Some(IntlState::List(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible ListFormat receiver")),
    }
}

fn intl_list_pair(pattern: &str, left: Vec<(&'static str, String)>, right: Vec<(&'static str, String)>) -> Vec<(&'static str, String)> {
    let mut output = Vec::new();
    let mut remaining = pattern;
    while !remaining.is_empty() {
        let left_at = remaining.find("{0}");
        let right_at = remaining.find("{1}");
        let choice = match (left_at, right_at) {
            (Some(a), Some(b)) => Some((a.min(b), a < b)),
            (Some(a), None) => Some((a, true)),
            (None, Some(b)) => Some((b, false)),
            (None, None) => None,
        };
        let Some((at, is_left)) = choice else {
            output.push(("literal", remaining.to_string()));
            break;
        };
        if at > 0 { output.push(("literal", remaining[..at].to_string())); }
        output.extend(if is_left { left.clone() } else { right.clone() });
        remaining = &remaining[at + 3..];
    }
    output
}

fn intl_list_parts(state: &IntlListState, values: Vec<String>) -> Vec<(&'static str, String)> {
    let data = INTL_LIST_DATA.iter().find(|data| data.locale == state.locale && data.kind == state.kind && data.style == state.style)
        .unwrap_or(&INTL_LIST_DATA[0]);
    let mut values = values.into_iter();
    let Some(first) = values.next() else { return Vec::new() };
    let mut parts = vec![("element", first)];
    let Some(second) = values.next() else { return parts };
    let remaining: Vec<String> = values.collect();
    if remaining.is_empty() { return intl_list_pair(data.pair, parts, vec![("element", second)]); }
    parts = intl_list_pair(data.start, parts, vec![("element", second)]);
    for (index, item) in remaining.iter().enumerate() {
        let pattern = if index + 1 == remaining.len() { data.end } else { data.middle };
        parts = intl_list_pair(pattern, parts, vec![("element", item.clone())]);
    }
    parts
}

fn intl_list_arguments(i: &mut Interpreter, args: &[JsValue]) -> Result<Vec<String>, Throw> {
    let input = args.first().unwrap_or(&JsValue::Undefined);
    if input.is_undefined() { return Ok(Vec::new()); }
    // GetIterator: StringListFromIterable advances the iterator one element at
    // a time and closes it as soon as a non-string value appears.
    let iterator = {
        let method = i.get_prop(input, WellKnownSymbol::Iterator.symbol())?;
        i.call_value(&method, input.clone(), Vec::new())?
    };
    if i.get_prop(&iterator, "next")?.is_undefined() {
        return Err(Throw::type_error("value is not iterable"));
    }
    let next = i.get_prop(&iterator, "next")?;
    let mut values = Vec::new();
    loop {
        let step = i.call_value(&next, iterator.clone(), Vec::new())?;
        let JsValue::Object(result) = step else {
            let _ = i.close_iterator(&iterator);
            return Err(Throw::type_error("iterator result is not an object"));
        };
        let done = result.borrow().get_own("done").map(|value| value.to_boolean()).unwrap_or(false);
        if done { return Ok(values); }
        let value = i.get_prop(&JsValue::Object(result), "value")?;
        if !matches!(value, JsValue::Str(_)) {
            let _ = i.close_iterator(&iterator);
            return Err(Throw::type_error("ListFormat elements must be strings"));
        }
        values.push(i.to_string(&value)?);
    }
}

fn intl_list_format(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_list_state(&this)?;
    let values = intl_list_arguments(i, &args)?;
    Ok(JsValue::str(intl_list_parts(&state, values).into_iter().map(|(_, text)| text).collect::<String>()))
}

fn intl_list_format_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_list_state(&this)?;
    let values = intl_list_arguments(i, &args)?;
    intl_parts_array(i, intl_list_parts(&state, values))
}

fn intl_list_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_list_state(&this)?;
    let object = i.object_value();
    i.set_prop(&object, "locale", JsValue::str(state.resolved.as_str()))?;
    i.set_prop(&object, "type", JsValue::str(state.kind))?;
    i.set_prop(&object, "style", JsValue::str(state.style))?;
    Ok(object)
}

#[cfg(test)]
mod intl_relative_list_tests {
    use super::*;

    #[test]
    fn cldr_relative_and_list_patterns() {
        let mut i = Interpreter::new();
        let value = i.run_script("let r=new Intl.RelativeTimeFormat('en',{numeric:'auto'}); let l=new Intl.ListFormat('en'); [r.format(-1,'day'),r.format(2,'day'),l.format(['a','b','c'])].join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "yesterday|in 2 days|a, b, and c");
    }
}
