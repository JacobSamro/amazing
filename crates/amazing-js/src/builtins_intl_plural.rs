// Intl.PluralRules using rules generated verbatim from pinned CLDR JSON.

#[derive(Clone)]
struct IntlPluralState {
    locale: String,
    kind: String,
    notation: String,
    compact_display: Option<String>,
    min_integer: usize,
    min_fraction: usize,
    max_fraction: usize,
    min_significant: Option<usize>,
    max_significant: Option<usize>,
    rounding_increment: usize,
    rounding_mode: String,
    rounding_priority: String,
    trailing_zero_display: String,
}

impl Interpreter {
    fn install_intl_plural(&mut self, namespace: &Obj) {
        let constructor = Self::native_constructor("PluralRules", intl_plural_ctor, &self.function_proto);
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
        intl_register_intrinsic(self, "PluralRules", function, &prototype);
        let _ = namespace.borrow_mut().define("PluralRules", binary_writable_property(constructor));
    }
}

fn intl_plural_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("PluralRules requires new")); }
    let JsValue::Object(object) = this else { return Err(Throw::type_error("PluralRules requires an object")); };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let options = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)).cloned()
        .unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null PluralRules options")); }
    let matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let locale = intl_lookup(&requested, matcher == "best fit");
    let kind = intl_string_option(i, &options, "type", &["cardinal", "ordinal"], "cardinal")?;
    let notation = intl_string_option(i, &options, "notation",
        &["standard", "compact", "scientific", "engineering"], "standard")?;
    let compact_display = if notation == "compact" {
        Some(intl_string_option(i, &options, "compactDisplay", &["short", "long"], "short")?)
    } else {
        // The option is still read (and validated) even when notation is not compact.
        let _ = intl_string_option(i, &options, "compactDisplay", &["short", "long"], "short")?;
        None
    };
    let min_integer = intl_integer_option(i, &options, "minimumIntegerDigits", 1, 21, 1)?;
    let min_fraction = intl_integer_option(i, &options, "minimumFractionDigits", 0, 100, 0)?;
    let max_fraction = intl_integer_option(i, &options, "maximumFractionDigits", 0, 100, 3.max(min_fraction))?;
    if min_fraction > max_fraction { return Err(Throw::range_error("fraction digits range")); }
    // Read each significant-digits getter exactly once (min before max).
    let min_sig_value = i.get_prop(&options, "minimumSignificantDigits")?;
    let max_sig_value = i.get_prop(&options, "maximumSignificantDigits")?;
    let min_significant = if min_sig_value.is_undefined() { None }
        else { Some(intl_integer_from_value(i, &min_sig_value, "minimumSignificantDigits", 1, 21, 1)?) };
    let max_significant = if max_sig_value.is_undefined() { None }
        else { Some(intl_integer_from_value(i, &max_sig_value, "maximumSignificantDigits", 1, 21, 21)?) };
    let rounding_increment = intl_integer_option(i, &options, "roundingIncrement", 1, 5000, 1)?;
    if ![1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000].contains(&rounding_increment) {
        return Err(Throw::range_error("invalid rounding increment"));
    }
    let rounding_mode = intl_string_option(i, &options, "roundingMode",
        &["ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfExpand", "halfTrunc", "halfEven"], "halfExpand")?;
    let rounding_priority = intl_string_option(i, &options, "roundingPriority",
        &["auto", "morePrecision", "lessPrecision"], "auto")?;
    let trailing_zero_display = intl_string_option(i, &options, "trailingZeroDisplay",
        &["auto", "stripIfInteger"], "auto")?;
    intl_register(&object, IntlState::Plural(IntlPluralState {
        locale, kind, notation, compact_display, min_integer, min_fraction, max_fraction,
        min_significant, max_significant, rounding_increment, rounding_mode,
        rounding_priority, trailing_zero_display,
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

fn intl_plural_operands(state: &IntlPluralState, value: f64) -> [f64; 8] {
    if !value.is_finite() { return [f64::NAN; 8]; }
    let mut absolute = value.abs();
    // Compact notation (CLDR plural operands e/c): the number is shown scaled by 10^e
    // ("1.5M" is e = 6, i = 1, f = 5); one fraction digit survives only below 10.
    let mut exponent = 0i32;
    let mut compact_fraction = None;
    if state.notation == "compact" && absolute >= 1000.0 {
        exponent = ((absolute.log10().floor() as i32) / 3 * 3).min(15);
        absolute /= 10f64.powi(exponent);
        compact_fraction = Some(if absolute < 10.0 { 1 } else { 0 });
    }
    let fraction = if let Some(digits) = compact_fraction { digits } else if let Some(max_sig) = state.max_significant {
        let before = if absolute == 0.0 { 1 } else { absolute.log10().floor() as i32 + 1 };
        (max_sig as i32 - before).clamp(0, 100) as usize
    } else { state.max_fraction };
    // `rounding_increment` (default 1, a no-op) rounds to the nearest
    // multiple of increment/scale rather than 1/scale.
    let scale = 10f64.powi(fraction.min(100) as i32) / state.rounding_increment as f64;
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
        exponent as f64,
    ]
}

fn intl_plural_atom(atom: &str, operands: &[f64; 8]) -> bool {
    let words: Vec<&str> = atom.split_whitespace().collect();
    if words.len() < 3 { return false; }
    let operand = match words[0] {
        "n" => operands[0], "i" => operands[1], "v" => operands[2],
        "w" => operands[5], "f" => operands[4], "t" => operands[6],
        "e" | "c" => operands[7], _ => return false,
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

fn intl_plural_matches(rule: &str, operands: &[f64; 8]) -> bool {
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
    i.set_prop(&object, "notation", JsValue::str(&state.notation))?;
    if let Some(display) = &state.compact_display {
        i.set_prop(&object, "compactDisplay", JsValue::str(display))?;
    }
    i.set_prop(&object, "minimumIntegerDigits", JsValue::num(state.min_integer as f64))?;
    i.set_prop(&object, "minimumFractionDigits", JsValue::num(state.min_fraction as f64))?;
    i.set_prop(&object, "maximumFractionDigits", JsValue::num(state.max_fraction as f64))?;
    if let Some(value) = state.min_significant {
        i.set_prop(&object, "minimumSignificantDigits", JsValue::num(value as f64))?;
    }
    if let Some(value) = state.max_significant {
        i.set_prop(&object, "maximumSignificantDigits", JsValue::num(value as f64))?;
    }
    // pluralCategories uses the canonical ECMA-402 order, not sorted order.
    const ORDER: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
    let mut categories = Vec::new();
    for category in ORDER {
        if category == "other"
            || INTL_PLURAL_RULES.iter().any(|rule| rule.locale == state.locale && rule.kind == state.kind && rule.category == category)
        {
            categories.push(category);
        }
    }
    let categories = intl_array(i, categories.into_iter().map(JsValue::str))?;
    i.set_prop(&object, "pluralCategories", categories)?;
    i.set_prop(&object, "roundingIncrement", JsValue::num(state.rounding_increment as f64))?;
    i.set_prop(&object, "roundingMode", JsValue::str(&state.rounding_mode))?;
    i.set_prop(&object, "roundingPriority", JsValue::str(&state.rounding_priority))?;
    i.set_prop(&object, "trailingZeroDisplay", JsValue::str(&state.trailing_zero_display))?;
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

    #[test]
    fn plural_defaults_ignore_object_prototype_and_expose_notation() {
        let mut i = Interpreter::new();
        let value = i.run_script(
            "Object.prototype.type='ordinal'; Object.prototype.notation='compact'; \
             let p=new Intl.PluralRules('en'); let r=p.resolvedOptions(); \
             [r.type, r.notation, r.pluralCategories.join(','), Object.getOwnPropertyNames(r).join('>')].join('|')").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "cardinal|standard|one,other|locale>type>notation>minimumIntegerDigits>minimumFractionDigits>maximumFractionDigits>pluralCategories>roundingIncrement>roundingMode>roundingPriority>trailingZeroDisplay");
    }
}
