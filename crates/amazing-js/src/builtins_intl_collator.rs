// Intl.Collator root ordering with the supported locale's common letter forms.

#[derive(Clone)]
struct IntlCollatorState {
    locale: String, usage: String, sensitivity: String, collation: String,
    numeric: bool, case_first: String, ignore_punctuation: bool,
}

impl Interpreter {
    fn install_intl_collator(&mut self, namespace: &Obj) {
        let constructor = Self::native("Collator", intl_collator_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "Collator", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        binary_getter(self, &prototype, "compare", intl_collator_compare_getter);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_collator_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.Collator")));
        let _ = namespace.borrow_mut().define("Collator", binary_writable_property(constructor));
    }
}

fn intl_collator_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("Collator requires new")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let locale = intl_lookup(&requested);
    let options = args.get(1).filter(|value| !value.is_undefined()).cloned().unwrap_or_else(|| i.object_value());
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null Collator options")); }
    let usage = intl_string_option(i, &options, "usage", &["sort", "search"], "sort")?;
    let _matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let sensitivity = intl_string_option(i, &options, "sensitivity", &["base", "accent", "case", "variant"], "variant")?;
    let collation = intl_string_option(i, &options, "collation", &[], "default")?;
    if !collation.bytes().all(|b| b.is_ascii_alphanumeric()) || !(3..=8).contains(&collation.len()) && collation != "default" {
        return Err(Throw::range_error("invalid collation"));
    }
    let numeric_value = i.get_prop(&options, "numeric")?;
    let numeric = if numeric_value.is_undefined() { false } else { numeric_value.to_boolean() };
    let case_first = intl_string_option(i, &options, "caseFirst", &["upper", "lower", "false"], "false")?;
    let ignore_value = i.get_prop(&options, "ignorePunctuation")?;
    let ignore_punctuation = if ignore_value.is_undefined() { false } else { ignore_value.to_boolean() };
    intl_register(&object, IntlState::Collator(IntlCollatorState {
        locale, usage, sensitivity, collation, numeric, case_first, ignore_punctuation,
    }));
    Ok(JsValue::Object(object))
}

fn intl_collator_state(this: &JsValue) -> Result<IntlCollatorState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible Collator receiver")); };
    match intl_state(object) {
        Some(IntlState::Collator(state)) | Some(IntlState::BoundCollator(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible Collator receiver")),
    }
}

fn intl_collator_compare_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_collator_state(&this)?;
    let function = Interpreter::native("compare", intl_collator_bound_compare, &i.function_proto);
    let JsValue::Object(object) = &function else { unreachable!() };
    object.borrow_mut().constructible = false;
    object.borrow_mut().delete_own("prototype");
    intl_register(object, IntlState::BoundCollator(state));
    Ok(function)
}

fn intl_collator_bound_compare(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let function = i.native_stack.last().ok_or_else(|| Throw::type_error("missing bound Collator"))?;
    let state = match intl_state(function) {
        Some(IntlState::BoundCollator(state)) => state,
        _ => return Err(Throw::type_error("invalid bound Collator")),
    };
    let left = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let right = i.to_string(args.get(1).unwrap_or(&JsValue::Undefined))?;
    let result = intl_collate(&state, &left, &right);
    Ok(JsValue::num(match result {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}

fn intl_collator_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_collator_state(&this)?;
    let object = i.object_value();
    for (key, value) in [
        ("locale", state.locale), ("usage", state.usage), ("sensitivity", state.sensitivity),
        ("collation", state.collation), ("caseFirst", state.case_first),
    ] { i.set_prop(&object, key, JsValue::str(value))?; }
    i.set_prop(&object, "numeric", JsValue::Bool(state.numeric))?;
    i.set_prop(&object, "ignorePunctuation", JsValue::Bool(state.ignore_punctuation))?;
    Ok(object)
}

fn intl_base_char(ch: char) -> &'static str {
    match ch {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ā' => "a",
        'æ' | 'Æ' => "ae", 'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => "c",
        'é' | 'è' | 'ê' | 'ë' | 'ē' | 'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => "e",
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => "i",
        'ñ' | 'ń' | 'Ñ' | 'Ń' => "n",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => "o",
        'œ' | 'Œ' => "oe", 'ß' => "ss",
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => "u",
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => "y",
        _ => "",
    }
}

fn intl_primary_key(state: &IntlCollatorState, text: &str) -> String {
    let mut output = String::new();
    for ch in text.chars() {
        if state.ignore_punctuation && (ch.is_ascii_punctuation() || ['“', '”', '‘', '’', '—', '–'].contains(&ch)) {
            continue;
        }
        let mapped = intl_base_char(ch);
        if !mapped.is_empty() { output.push_str(mapped); }
        else { output.extend(ch.to_lowercase()); }
    }
    output
}

fn intl_compare_numeric(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut a = left.chars().peekable();
    let mut b = right.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut ax = String::new();
                let mut by = String::new();
                while a.peek().is_some_and(|ch| ch.is_ascii_digit()) { ax.push(a.next().unwrap()); }
                while b.peek().is_some_and(|ch| ch.is_ascii_digit()) { by.push(b.next().unwrap()); }
                let ax = ax.trim_start_matches('0');
                let by = by.trim_start_matches('0');
                let result = ax.len().cmp(&by.len()).then_with(|| ax.cmp(by));
                if result != Ordering::Equal { return result; }
            }
            (Some(x), Some(y)) => {
                a.next(); b.next();
                let result = x.cmp(&y);
                if result != Ordering::Equal { return result; }
            }
        }
    }
}

fn intl_collate(state: &IntlCollatorState, left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let left_primary = intl_primary_key(state, left);
    let right_primary = intl_primary_key(state, right);
    let primary = if state.numeric { intl_compare_numeric(&left_primary, &right_primary) }
        else { left_primary.cmp(&right_primary) };
    if primary != Ordering::Equal { return primary; }
    if ["accent", "variant"].contains(&state.sensitivity.as_str()) {
        let accents = left.to_lowercase().cmp(&right.to_lowercase());
        if accents != Ordering::Equal { return accents; }
    }
    if ["case", "variant"].contains(&state.sensitivity.as_str()) {
        let cases = match state.case_first.as_str() {
            "upper" => right.cmp(left),
            _ => left.cmp(right),
        };
        if cases != Ordering::Equal { return cases; }
    }
    Ordering::Equal
}

#[cfg(test)]
mod intl_collator_tests {
    use super::*;

    #[test]
    fn numeric_and_base_sensitivity() {
        let mut i = Interpreter::new();
        let value = i.run_script("let n=new Intl.Collator('en',{numeric:true}).compare; let b=new Intl.Collator('de',{sensitivity:'base'}).compare; [n('a2','a10'),b('ä','a')].join(',')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "-1,0");
    }
}
