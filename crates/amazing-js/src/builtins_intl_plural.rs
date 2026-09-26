// Intl.PluralRules using rules generated verbatim from pinned CLDR JSON.

#[derive(Clone)]
struct IntlPluralState {
    locale: String,
    kind: String,
    min_fraction: usize,
    max_fraction: usize,
    min_significant: Option<usize>,
    max_significant: Option<usize>,
    rounding_mode: String,
}

impl Interpreter {
    fn install_intl_plural(&mut self, namespace: &Obj) {
        let constructor = Self::native("PluralRules", intl_plural_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "PluralRules", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        intl_method(self, &prototype, "select", 1, intl_plural_select);
        intl_method(self, &prototype, "selectRange", 2, intl_plural_select_range);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_plural_resolved_options);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.PluralRules")));
        let _ = namespace.borrow_mut().define("PluralRules", binary_writable_property(constructor));
    }
}

fn intl_plural_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("PluralRules requires new")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let locale = intl_lookup(&requested);
    let options = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)).cloned()
        .unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null PluralRules options")); }
    let _matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let kind = intl_string_option(i, &options, "type", &["cardinal", "ordinal"], "cardinal")?;
    let min_fraction = intl_integer_option(i, &options, "minimumFractionDigits", 0, 100, 0)?;
    let max_fraction = intl_integer_option(i, &options, "maximumFractionDigits", 0, 100, 3.max(min_fraction))?;
    if min_fraction > max_fraction { return Err(Throw::range_error("fraction digits range")); }
    let min_sig_value = i.get_prop(&options, "minimumSignificantDigits")?;
    let max_sig_value = i.get_prop(&options, "maximumSignificantDigits")?;
    let min_significant = if min_sig_value.is_undefined() { None }
        else { Some(intl_integer_option(i, &options, "minimumSignificantDigits", 1, 21, 1)?) };
    let max_significant = if max_sig_value.is_undefined() { None }
        else { Some(intl_integer_option(i, &options, "maximumSignificantDigits", 1, 21, 21)?) };
    let rounding_mode = intl_string_option(i, &options, "roundingMode",
        &["ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfExpand", "halfTrunc", "halfEven"], "halfExpand")?;
    intl_register(&object, IntlState::Plural(IntlPluralState {
        locale, kind, min_fraction, max_fraction, min_significant, max_significant, rounding_mode,
    }));
    Ok(JsValue::Object(object))
}

fn intl_plural_state(this: &JsValue) -> Result<IntlPluralState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible PluralRules receiver")); };
    match intl_state(object) {
        Some(IntlState::Plural(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible PluralRules receiver")),
    }
}

fn intl_plural_operands(state: &IntlPluralState, value: f64) -> [f64; 7] {
    if !value.is_finite() { return [f64::NAN; 7]; }
    let absolute = value.abs();
    let fraction = if let Some(max_sig) = state.max_significant {
        let before = if absolute == 0.0 { 1 } else { absolute.log10().floor() as i32 + 1 };
        (max_sig as i32 - before).clamp(0, 100) as usize
    } else { state.max_fraction };
    let scale = 10f64.powi(fraction.min(100) as i32);
    let rounded = intl_round_scaled(absolute * scale, &state.rounding_mode) / scale;
    let mut text = format!("{:.*}", fraction.min(100), rounded);
    if let Some(dot) = text.find('.') {
        while text.len() > dot + 1 + state.min_fraction && text.ends_with('0') { text.pop(); }
        if text.ends_with('.') { text.pop(); }
    }
    let (integer, fractional) = text.split_once('.').unwrap_or((&text, ""));
    let visible = fractional.len();
    let fraction_int = fractional.parse::<u64>().unwrap_or(0);
    let trimmed = fractional.trim_end_matches('0');
    let trimmed_int = trimmed.parse::<u64>().unwrap_or(0);
    [
        rounded, integer.parse::<f64>().unwrap_or(0.0), visible as f64,
        visible as f64, fraction_int as f64, trimmed.len() as f64, trimmed_int as f64,
    ]
}

fn intl_plural_atom(atom: &str, operands: &[f64; 7]) -> bool {
    let words: Vec<&str> = atom.split_whitespace().collect();
    if words.len() < 3 { return false; }
    let operand = match words[0] {
        "n" => operands[0], "i" => operands[1], "v" => operands[2],
        "w" => operands[5], "f" => operands[4], "t" => operands[6],
        "e" | "c" => 0.0, _ => return false,
    };
    let (value, operator_index) = if words.get(1) == Some(&"%") {
        let divisor = words.get(2).and_then(|part| part.parse::<f64>().ok()).unwrap_or(1.0);
        (operand % divisor, 3)
    } else { (operand, 1) };
    let op = words.get(operator_index).copied().unwrap_or("");
    let right = words.get(operator_index + 1).copied().unwrap_or("");
    let matched = right.split(',').any(|range| {
        if let Some((lower, upper)) = range.split_once("..") {
            let lower = lower.parse::<f64>().unwrap_or(f64::NAN);
            let upper = upper.parse::<f64>().unwrap_or(f64::NAN);
            lower <= value && value <= upper
        } else {
            range.parse::<f64>().is_ok_and(|target| value == target)
        }
    });
    if op == "!=" { !matched } else { matched }
}

fn intl_plural_matches(rule: &str, operands: &[f64; 7]) -> bool {
    rule.split(" or ").any(|branch| branch.split(" and ").all(|atom| intl_plural_atom(atom, operands)))
}

fn intl_plural_category(state: &IntlPluralState, value: f64) -> &'static str {
    let operands = intl_plural_operands(state, value);
    if !value.is_finite() { return "other"; }
    for rule in INTL_PLURAL_RULES.iter().filter(|rule| rule.locale == state.locale && rule.kind == state.kind) {
        if intl_plural_matches(rule.rule, &operands) { return rule.category; }
    }
    "other"
}

fn intl_plural_select(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_plural_state(&this)?;
    let number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::str(intl_plural_category(&state, number)))
}

fn intl_plural_select_range(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_plural_state(&this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("selectRange requires two numbers"));
    }
    let start = i.to_number(&args[0])?;
    let end = i.to_number(&args[1])?;
    if start.is_nan() || end.is_nan() { return Err(Throw::range_error("NaN range endpoint")); }
    let left = intl_plural_category(&state, start);
    let right = intl_plural_category(&state, end);
    Ok(JsValue::str(if left == right { left } else { "other" }))
}

fn intl_plural_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_plural_state(&this)?;
    let object = i.object_value();
    i.set_prop(&object, "locale", JsValue::str(&state.locale))?;
    i.set_prop(&object, "type", JsValue::str(&state.kind))?;
    i.set_prop(&object, "minimumIntegerDigits", JsValue::num(1.0))?;
    i.set_prop(&object, "minimumFractionDigits", JsValue::num(state.min_fraction as f64))?;
    i.set_prop(&object, "maximumFractionDigits", JsValue::num(state.max_fraction as f64))?;
    if let Some(value) = state.min_significant {
        i.set_prop(&object, "minimumSignificantDigits", JsValue::num(value as f64))?;
    }
    if let Some(value) = state.max_significant {
        i.set_prop(&object, "maximumSignificantDigits", JsValue::num(value as f64))?;
    }
    i.set_prop(&object, "roundingMode", JsValue::str(&state.rounding_mode))?;
    let mut categories = vec!["other"];
    for rule in INTL_PLURAL_RULES.iter().filter(|rule| rule.locale == state.locale && rule.kind == state.kind) {
        categories.push(rule.category);
    }
    categories.sort();
    let categories = intl_array(i, categories.into_iter().map(JsValue::str))?;
    i.set_prop(&object, "pluralCategories", categories)?;
    Ok(object)
}

#[cfg(test)]
mod intl_plural_tests {
    use super::*;

    #[test]
    fn cldr_cardinal_and_ordinal_rules() {
        let mut i = Interpreter::new();
        let value = i.run_script("let en=new Intl.PluralRules('en'); let ru=new Intl.PluralRules('ru'); let ord=new Intl.PluralRules('en',{type:'ordinal'}); [en.select(1),en.select(2),ru.select(2),ru.select(5),ord.select(3)].join(',')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "one,other,few,many,few");
    }
}
