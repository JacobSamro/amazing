// Intl.DisplayNames for the bounded CLDR locale set.

#[derive(Clone)]
struct IntlDisplayState {
    locale: String, resolved: String, kind: String, style: String, fallback: String, language_display: String,
}

impl Interpreter {
    fn install_intl_display_names(&mut self, namespace: &Obj) {
        let constructor = Self::native("DisplayNames", intl_display_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "DisplayNames", 2, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        intl_method(self, &prototype, "of", 1, intl_display_of);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_display_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.DisplayNames")));
        let _ = namespace.borrow_mut().define("DisplayNames", binary_writable_property(constructor));
    }
}

fn intl_display_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("DisplayNames requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("DisplayNames requires an object")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let Some(options) = args.get(1).filter(|value| !value.is_undefined()) else {
        return Err(Throw::type_error("DisplayNames options are required"));
    };
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null DisplayNames options")); }
    let matcher = intl_string_option(i, options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let locale = intl_lookup(&requested, matcher == "best fit");
    let style = intl_string_option(i, options, "style", &["long", "short", "narrow"], "long")?;
    let type_value = i.get_prop(options, "type")?;
    if type_value.is_undefined() { return Err(Throw::type_error("DisplayNames type is required")); }
    let kind = i.to_string(&type_value)?;
    if !["language", "region", "script", "currency", "calendar", "dateTimeField"].contains(&kind.as_str()) {
        return Err(Throw::range_error("invalid DisplayNames type"));
    }
    let fallback = intl_string_option(i, options, "fallback", &["code", "none"], "code")?;
    let language_display = intl_string_option(i, options, "languageDisplay", &["dialect", "standard"], "dialect")?;
    let resolved = intl_resolved_locale(&requested, &locale);
    intl_register(&object, IntlState::DisplayNames(IntlDisplayState {
        locale, resolved, kind, style, fallback, language_display,
    }));
    Ok(JsValue::Object(object))
}

fn intl_display_state(this: &JsValue) -> Result<IntlDisplayState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible DisplayNames receiver")); };
    match intl_state(object) {
        Some(IntlState::DisplayNames(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible DisplayNames receiver")),
    }
}

fn intl_display_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_display_state(&this)?;
    let value = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let code = match state.kind.as_str() {
        "language" => intl_canonical_tag(&value)?,
        "region" if (value.len() == 2 && value.bytes().all(|b| b.is_ascii_alphabetic()))
            || (value.len() == 3 && value.bytes().all(|b| b.is_ascii_digit())) => value.to_ascii_uppercase(),
        "script" if value.len() == 4 && value.bytes().all(|b| b.is_ascii_alphabetic()) => intl_title(&value),
        "currency" if value.len() == 3 && value.bytes().all(|b| b.is_ascii_alphabetic()) => value.to_ascii_uppercase(),
        "calendar" if value.split('-').all(|part| (3..=8).contains(&part.len())
            && part.bytes().all(|b| b.is_ascii_alphanumeric())) => value.to_ascii_lowercase(),
        "dateTimeField" if ["era", "year", "quarter", "month", "weekOfYear", "weekday", "day",
            "dayPeriod", "hour", "minute", "second", "timeZoneName"].contains(&value.as_str()) => value,
        _ => return Err(Throw::range_error("invalid DisplayNames code")),
    };
    let data = INTL_DISPLAY_DATA.iter().find(|(locale, _)| *locale == state.locale).map(|(_, data)| *data)
        .unwrap_or(INTL_DISPLAY_DATA[0].1);
    let mut name = data.iter().find(|entry| entry.kind == state.kind && entry.code == code).map(|entry| entry.name.to_string());
    if name.is_none() && state.kind == "calendar" {
        // The generated calendar-name table keys some entries by their CLDR
        // "type" attribute rather than the BCP47 -u-ca- subtag AvailableCalendars
        // reports (ethioaa/gregory); look those up under their table alias too.
        let alias = match code.as_str() { "ethioaa" => "ethiopic-amete-alem", "gregory" => "gregorian", _ => "" };
        if !alias.is_empty() {
            name = data.iter().find(|entry| entry.kind == "calendar" && entry.code == alias).map(|entry| entry.name.to_string());
        }
    }
    if state.kind == "currency" {
        let index = INTL_NUMBER_DATA.iter().position(|data| data.locale == state.locale).unwrap_or(0);
        name = INTL_CURRENCY_DATA.iter().find(|entry| entry.code == code).map(|entry| entry.names[index].to_string());
    }
    if name.is_none() && state.fallback == "none" { Ok(JsValue::Undefined) }
    else { Ok(JsValue::str(name.unwrap_or(code))) }
}

fn intl_display_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_display_state(&this)?;
    let object = i.object_value();
    for (key, value) in [
        ("locale", state.resolved), ("style", state.style), ("type", state.kind),
        ("fallback", state.fallback), ("languageDisplay", state.language_display),
    ] { i.set_prop(&object, key, JsValue::str(value))?; }
    Ok(object)
}

#[cfg(test)]
mod intl_display_tests {
    use super::*;

    #[test]
    fn cldr_display_names_and_fallback() {
        let mut i = Interpreter::new();
        let value = i.run_script("let d=new Intl.DisplayNames('fr',{type:'region'}); let e=new Intl.DisplayNames('en',{type:'language'}); [d.of('US'),e.of('de')].join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "États-Unis|German");
    }
}
