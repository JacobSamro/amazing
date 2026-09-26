// Intl.NumberFormat. The locale data is generated from pinned CLDR JSON.

#[derive(Clone)]
struct IntlNumberState {
    locale: String,
    numbering_system: String,
    style: String,
    currency: Option<String>,
    currency_display: String,
    currency_sign: String,
    unit: Option<String>,
    unit_display: String,
    notation: String,
    compact_display: String,
    sign_display: String,
    use_grouping: String,
    min_integer: usize,
    min_fraction: usize,
    max_fraction: usize,
    min_significant: Option<usize>,
    max_significant: Option<usize>,
    rounding_priority: String,
    rounding_mode: String,
    rounding_increment: usize,
    trailing_zero_display: String,
}

fn intl_string_option(i: &mut Interpreter, options: &JsValue, key: &str, allowed: &[&str], default: &str) -> Result<String, Throw> {
    let value = i.get_prop(options, key)?;
    if matches!(value, JsValue::Undefined) { return Ok(default.to_string()); }
    let text = i.to_string(&value)?;
    if !allowed.is_empty() && !allowed.contains(&text.as_str()) {
        return Err(Throw::range_error(format!("invalid {key} option")));
    }
    Ok(text)
}

fn intl_integer_option(i: &mut Interpreter, options: &JsValue, key: &str, min: usize, max: usize, default: usize) -> Result<usize, Throw> {
    let value = i.get_prop(options, key)?;
    if matches!(value, JsValue::Undefined) { return Ok(default); }
    let number = i.to_number(&value)?;
    if !number.is_finite() || number < min as f64 || number > max as f64 {
        return Err(Throw::range_error(format!("invalid {key} option")));
    }
    Ok(number.trunc() as usize)
}

fn intl_number_state(this: &JsValue) -> Result<IntlNumberState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible NumberFormat receiver")); };
    match intl_state(object) {
        Some(IntlState::Number(state)) | Some(IntlState::BoundNumber(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible NumberFormat receiver")),
    }
}

impl Interpreter {
    fn install_intl_number(&mut self, namespace: &Obj) {
        let constructor = Self::native("NumberFormat", intl_number_ctor, &self.function_proto);
        let JsValue::Object(function) = &constructor else { unreachable!() };
        let JsValue::Object(prototype) = function.borrow().get_own("prototype").unwrap() else { unreachable!() };
        prototype.borrow_mut().proto = Some(self.object_proto.clone());
        binary_configure_constructor(function, "NumberFormat", 0, &prototype);
        let _ = prototype.borrow_mut().define("constructor", binary_writable_property(constructor.clone()));
        binary_getter(self, &prototype, "format", intl_number_format_getter);
        intl_method(self, &prototype, "resolvedOptions", 0, intl_number_resolved_options);
        intl_method(self, &prototype, "formatToParts", 1, intl_number_format_to_parts);
        intl_method(self, &prototype, "formatRange", 2, intl_number_format_range);
        intl_method(self, &prototype, "formatRangeToParts", 2, intl_number_format_range_to_parts);
        intl_method(self, function, "supportedLocalesOf", 1, intl_number_supported_locales);
        let _ = prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl.NumberFormat")));
        let _ = namespace.borrow_mut().define("NumberFormat", binary_writable_property(constructor));
    }
}

fn intl_number_supported_locales(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let locales = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    if let Some(options) = args.get(1).filter(|value| !value.is_undefined()) {
        if matches!(options, JsValue::Null) { return Err(Throw::type_error("null supportedLocalesOf options")); }
        let _matcher = intl_string_option(i, options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    }
    let supported = locales.into_iter().filter(|tag| {
        let base = tag.split("-u-").next().unwrap_or(tag);
        let mut candidate = base;
        loop {
            if INTL_NUMBER_DATA.iter().any(|data| data.locale.eq_ignore_ascii_case(candidate)) { return true; }
            let Some((parent, _)) = candidate.rsplit_once('-') else { return false };
            candidate = parent;
        }
    });
    intl_array(i, supported.map(JsValue::str))
}

fn intl_number_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let locale = intl_lookup(&requested);
    let options = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)).cloned()
        .unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null NumberFormat options")); }
    let _matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    let numbering_system = intl_string_option(i, &options, "numberingSystem", &[], intl_data(&locale).numbering_system)?;
    if !numbering_system.bytes().all(|b| b.is_ascii_alphanumeric()) || !(3..=8).contains(&numbering_system.len()) {
        return Err(Throw::range_error("invalid numbering system"));
    }
    let style = intl_string_option(i, &options, "style", &["decimal", "percent", "currency", "unit"], "decimal")?;
    let currency_value = i.get_prop(&options, "currency")?;
    let currency = if matches!(currency_value, JsValue::Undefined) { None }
        else { Some(i.to_string(&currency_value)?.to_ascii_uppercase()) };
    if currency.as_ref().is_some_and(|code| code.len() != 3 || !code.bytes().all(|b| b.is_ascii_alphabetic())) {
        return Err(Throw::range_error("invalid currency code"));
    }
    if style == "currency" && currency.is_none() { return Err(Throw::type_error("currency is required")); }
    let currency_display = intl_string_option(i, &options, "currencyDisplay",
        &["code", "symbol", "narrowSymbol", "name"], "symbol")?;
    let currency_sign = intl_string_option(i, &options, "currencySign", &["standard", "accounting"], "standard")?;
    let unit_value = i.get_prop(&options, "unit")?;
    let unit = if matches!(unit_value, JsValue::Undefined) { None } else { Some(i.to_string(&unit_value)?) };
    if style == "unit" && unit.is_none() { return Err(Throw::type_error("unit is required")); }
    if unit.as_ref().is_some_and(|unit| !intl_valid_unit(unit)) { return Err(Throw::range_error("invalid unit")); }
    let unit_display = intl_string_option(i, &options, "unitDisplay", &["short", "narrow", "long"], "short")?;
    let notation = intl_string_option(i, &options, "notation",
        &["standard", "scientific", "engineering", "compact"], "standard")?;
    let compact_display = intl_string_option(i, &options, "compactDisplay", &["short", "long"], "short")?;
    let sign_display = intl_string_option(i, &options, "signDisplay",
        &["auto", "never", "always", "exceptZero", "negative"], "auto")?;
    let grouping_value = i.get_prop(&options, "useGrouping")?;
    let use_grouping = match grouping_value {
        JsValue::Undefined => if notation == "compact" { "min2" } else { "auto" }.to_string(),
        JsValue::Bool(false) => "false".to_string(),
        JsValue::Str(_) => {
            let value = i.to_string(&grouping_value)?;
            if ["min2", "auto", "always"].contains(&value.as_str()) { value } else { "auto".into() }
        }
        _ => "auto".into(),
    };
    let min_integer = intl_integer_option(i, &options, "minimumIntegerDigits", 1, 21, 1)?;
    let currency_digits = currency.as_ref().and_then(|code| INTL_CURRENCY_DATA.iter()
        .find(|entry| entry.code == code)).map_or(2, |entry| entry.digits);
    let default_fraction = if notation == "compact" { 0 } else if style == "currency" { currency_digits } else { 0 };
    let default_max = if notation == "compact" { 0 } else if style == "currency" { currency_digits } else if style == "percent" { 0 } else { 3 };
    let minimum_option = i.get_prop(&options, "minimumFractionDigits")?;
    let minimum_option = if minimum_option.is_undefined() { None } else {
        let value = i.to_number(&minimum_option)?;
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(Throw::range_error("invalid minimumFractionDigits option"));
        }
        Some(value.trunc() as usize)
    };
    let maximum_option = i.get_prop(&options, "maximumFractionDigits")?;
    let maximum_option = if maximum_option.is_undefined() { None } else {
        let value = i.to_number(&maximum_option)?;
        if !value.is_finite() || !(0.0..=100.0).contains(&value) {
            return Err(Throw::range_error("invalid maximumFractionDigits option"));
        }
        Some(value.trunc() as usize)
    };
    let min_fraction = minimum_option.unwrap_or_else(|| default_fraction.min(maximum_option.unwrap_or(100)));
    let max_fraction = maximum_option.unwrap_or_else(|| default_max.max(min_fraction));
    if min_fraction > max_fraction { return Err(Throw::range_error("minimumFractionDigits exceeds maximumFractionDigits")); }
    let min_sig_value = i.get_prop(&options, "minimumSignificantDigits")?;
    let max_sig_value = i.get_prop(&options, "maximumSignificantDigits")?;
    let has_significant = !min_sig_value.is_undefined() || !max_sig_value.is_undefined();
    let min_significant = if has_significant {
        Some(if min_sig_value.is_undefined() { 1 } else {
            intl_integer_option(i, &options, "minimumSignificantDigits", 1, 21, 1)?
        })
    } else { None };
    let max_significant = if has_significant {
        Some(if max_sig_value.is_undefined() { 21 } else {
            intl_integer_option(i, &options, "maximumSignificantDigits", 1, 21, 21)?
        })
    } else { None };
    if min_significant.zip(max_significant).is_some_and(|(min, max)| min > max) {
        return Err(Throw::range_error("significant digits range"));
    }
    let rounding_mode = intl_string_option(i, &options, "roundingMode",
        &["ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfExpand", "halfTrunc", "halfEven"], "halfExpand")?;
    let rounding_increment = intl_integer_option(i, &options, "roundingIncrement", 1, 5000, 1)?;
    if ![1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000].contains(&rounding_increment) {
        return Err(Throw::range_error("invalid rounding increment"));
    }
    let rounding_priority = intl_string_option(i, &options, "roundingPriority",
        &["auto", "morePrecision", "lessPrecision"], "auto")?;
    let trailing_zero_display = intl_string_option(i, &options, "trailingZeroDisplay",
        &["auto", "stripIfInteger"], "auto")?;
    if rounding_increment != 1 {
        if has_significant || rounding_priority != "auto" || notation == "compact" {
            return Err(Throw::type_error("roundingIncrement requires fraction digits"));
        }
        if min_fraction != max_fraction {
            return Err(Throw::range_error("roundingIncrement requires equal fraction digits"));
        }
    }
    let compact_defaults = notation == "compact" && rounding_priority == "auto"
        && !has_significant && min_fraction == 0 && max_fraction == 0;
    let rounding_priority = if compact_defaults { "morePrecision".into() } else { rounding_priority };
    let min_significant = if compact_defaults { Some(1) } else { min_significant };
    let max_significant = if compact_defaults { Some(2) } else { max_significant };
    let state = IntlNumberState {
        locale, numbering_system, style, currency, currency_display, currency_sign, unit, unit_display,
        notation, compact_display, sign_display, use_grouping, min_integer,
        min_fraction, max_fraction, min_significant, max_significant, rounding_priority, rounding_mode,
        rounding_increment, trailing_zero_display,
    };
    let object = match this {
        JsValue::Object(object) if i.current_new_target().is_some() => object,
        _ => {
            let intl = i.global_env.resolve("Intl").unwrap();
            let constructor = i.get_prop(&intl, "NumberFormat")?;
            let JsValue::Object(proto) = i.get_prop(&constructor, "prototype")? else { unreachable!() };
            let mut raw = JsObject::ordinary();
            raw.proto = Some(proto);
            Obj(Rc::new(RefCell::new(raw)))
        }
    };
    intl_register(&object, IntlState::Number(state));
    Ok(JsValue::Object(object))
}

const INTL_SIMPLE_UNITS: &[&str] = &["acre", "bit", "byte", "celsius", "centimeter", "day", "degree",
        "fahrenheit", "fluid-ounce", "foot", "gallon", "gigabit", "gigabyte", "gram", "hectare",
        "hour", "inch", "kilobit", "kilobyte", "kilogram", "kilometer", "liter", "megabit",
        "megabyte", "meter", "microsecond", "mile", "mile-scandinavian", "milliliter",
        "millimeter", "millisecond", "minute", "month", "nanosecond", "ounce", "percent",
        "petabyte", "pound", "second", "stone", "terabit", "terabyte", "week", "yard", "year"];

fn intl_valid_unit(unit: &str) -> bool {
    INTL_SIMPLE_UNITS.contains(&unit) || unit.split_once("-per-").is_some_and(|(a, b)|
        INTL_SIMPLE_UNITS.contains(&a) && INTL_SIMPLE_UNITS.contains(&b))
}

fn intl_number_format_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_number_state(&this)?;
    let function = Interpreter::native_arity("", intl_number_bound_format, &i.function_proto, 1);
    let JsValue::Object(object) = &function else { unreachable!() };
    object.borrow_mut().constructible = false;
    object.borrow_mut().delete_own("prototype");
    intl_register(object, IntlState::BoundNumber(state));
    Ok(function)
}

fn intl_number_bound_format(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let function = i.native_stack.last().ok_or_else(|| Throw::type_error("missing bound NumberFormat"))?;
    let state = match intl_state(function) {
        Some(IntlState::BoundNumber(state)) => state,
        _ => return Err(Throw::type_error("invalid bound NumberFormat")),
    };
    if let Some(JsValue::BigInt(bigint)) = args.first() {
        return Ok(JsValue::str(intl_format_bigint(&state, &bigint.to_radix_string(10))));
    }
    if let Some(JsValue::Str(_)) = args.first() {
        let text = i.to_string(args.first().unwrap())?;
        if let Some(parts) = intl_format_decimal_string(&state, &text) {
            return Ok(JsValue::str(parts.into_iter().map(|(_, value)| value).collect::<String>()));
        }
    }
    let number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::str(intl_format_number(&state, number).0))
}

fn intl_number_resolved_options(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_number_state(&this)?;
    let object = i.object_value();
    // Property insertion order per ECMA-402 16.5.5 Intl.NumberFormat.prototype.resolvedOptions.
    for (name, value) in [
        ("locale", state.locale.as_str()), ("numberingSystem", state.numbering_system.as_str()),
        ("style", state.style.as_str()),
    ] { i.set_prop(&object, name, JsValue::str(value))?; }
    if state.style == "currency" {
        if let Some(currency) = state.currency.as_ref() { i.set_prop(&object, "currency", JsValue::str(currency))?; }
        i.set_prop(&object, "currencyDisplay", JsValue::str(state.currency_display))?;
        i.set_prop(&object, "currencySign", JsValue::str(state.currency_sign))?;
    }
    if state.style == "unit" {
        if let Some(unit) = state.unit.as_ref() { i.set_prop(&object, "unit", JsValue::str(unit))?; }
        i.set_prop(&object, "unitDisplay", JsValue::str(state.unit_display))?;
    }
    for (name, value) in [
        ("minimumIntegerDigits", state.min_integer), ("minimumFractionDigits", state.min_fraction),
        ("maximumFractionDigits", state.max_fraction),
    ] { i.set_prop(&object, name, JsValue::num(value as f64))?; }
    if let Some(value) = state.min_significant { i.set_prop(&object, "minimumSignificantDigits", JsValue::num(value as f64))?; }
    if let Some(value) = state.max_significant { i.set_prop(&object, "maximumSignificantDigits", JsValue::num(value as f64))?; }
    i.set_prop(&object, "useGrouping", JsValue::str(state.use_grouping.as_str()))?;
    i.set_prop(&object, "notation", JsValue::str(state.notation.as_str()))?;
    if state.notation == "compact" { i.set_prop(&object, "compactDisplay", JsValue::str(state.compact_display))?; }
    i.set_prop(&object, "signDisplay", JsValue::str(state.sign_display.as_str()))?;
    i.set_prop(&object, "roundingIncrement", JsValue::num(state.rounding_increment as f64))?;
    i.set_prop(&object, "roundingMode", JsValue::str(state.rounding_mode.as_str()))?;
    i.set_prop(&object, "roundingPriority", JsValue::str(state.rounding_priority.as_str()))?;
    i.set_prop(&object, "trailingZeroDisplay", JsValue::str(state.trailing_zero_display.as_str()))?;
    Ok(object)
}

fn intl_number_format_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_number_state(&this)?;
    if let Some(JsValue::BigInt(bigint)) = args.first() {
        return intl_parts_array(i, vec![("integer", intl_format_bigint(&state, &bigint.to_radix_string(10)))]);
    }
    if let Some(JsValue::Str(_)) = args.first() {
        let text = i.to_string(args.first().unwrap())?;
        if let Some(parts) = intl_format_decimal_string(&state, &text) {
            return intl_parts_array(i, parts);
        }
    }
    let number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let (_, parts) = intl_format_number(&state, number);
    intl_parts_array(i, parts)
}

fn intl_parts_array(i: &mut Interpreter, parts: Vec<(&'static str, String)>) -> Result<JsValue, Throw> {
    let mut values = Vec::new();
    for (kind, value) in parts {
        let part = i.object_value();
        i.set_prop(&part, "type", JsValue::str(kind))?;
        i.set_prop(&part, "value", JsValue::str(value))?;
        values.push(part);
    }
    intl_array(i, values)
}

fn intl_number_range_endpoint(i: &mut Interpreter, state: &IntlNumberState, value: &JsValue) -> Result<(String, bool), Throw> {
    if let JsValue::BigInt(bigint) = value {
        return Ok((intl_format_bigint(state, &bigint.to_radix_string(10)), true));
    }
    if let JsValue::Str(_) = value {
        let text = i.to_string(value)?;
        let digits = text.strip_prefix('-').unwrap_or(&text);
        if digits.len() > 15 && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok((intl_format_bigint(state, &text), true));
        }
    }
    let number = i.to_number(value)?;
    if number.is_nan() { return Err(Throw::range_error("NaN range endpoint")); }
    Ok((intl_format_number(state, number).0, false))
}

fn intl_number_format_range(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_number_state(&this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRange requires two numbers"));
    }
    let (left, left_exact) = intl_number_range_endpoint(i, &state, &args[0])?;
    let (right, right_exact) = intl_number_range_endpoint(i, &state, &args[1])?;
    if left == right { return Ok(JsValue::str(format!("~{left}"))); }
    if state.style == "currency" && state.sign_display == "always" {
        let shared = left.chars().zip(right.chars())
            .take_while(|(a, b)| a == b && !a.is_ascii_digit())
            .map(|(ch, _)| ch.len_utf8()).sum::<usize>();
        if shared > 0 { return Ok(JsValue::str(format!("{left}–{}", &right[shared..]))); }
    }
    Ok(JsValue::str(if left_exact && right_exact { format!("{left}–{right}") }
        else { format!("{left} – {right}") }))
}

fn intl_number_endpoint_parts(i: &mut Interpreter, state: &IntlNumberState, value: &JsValue) -> Result<(String, Vec<(&'static str, String)>), Throw> {
    if let JsValue::BigInt(bigint) = value {
        let text = intl_format_bigint(state, &bigint.to_radix_string(10));
        return Ok((text.clone(), vec![("integer", text)]));
    }
    let number = i.to_number(value)?;
    if number.is_nan() { return Err(Throw::range_error("NaN range endpoint")); }
    Ok(intl_format_number(state, number))
}

fn intl_number_format_range_to_parts(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = intl_number_state(&this)?;
    if args.len() < 2 || args[0].is_undefined() || args[1].is_undefined() {
        return Err(Throw::type_error("formatRangeToParts requires two numbers"));
    }
    let (left, left_parts) = intl_number_endpoint_parts(i, &state, &args[0])?;
    let (right, right_parts) = intl_number_endpoint_parts(i, &state, &args[1])?;
    let mut parts: Vec<(&'static str, String, &'static str)> = Vec::new();
    if left == right {
        parts.push(("approximatelySign", "~".into(), "shared"));
        parts.extend(left_parts.into_iter().map(|(kind, value)| (kind, value, "shared")));
    } else {
        parts.extend(left_parts.into_iter().map(|(kind, value)| (kind, value, "startRange")));
        parts.push(("literal", " – ".into(), "shared"));
        parts.extend(right_parts.into_iter().map(|(kind, value)| (kind, value, "endRange")));
    }
    let mut values = Vec::new();
    for (kind, value, source) in parts {
        let part = i.object_value();
        i.set_prop(&part, "type", JsValue::str(kind))?;
        i.set_prop(&part, "value", JsValue::str(value))?;
        i.set_prop(&part, "source", JsValue::str(source))?;
        values.push(part);
    }
    intl_array(i, values)
}

fn intl_round_scaled(value: f64, mode: &str) -> f64 {
    if !value.is_finite() { return value; }
    let floor = value.floor();
    let fraction = value - floor;
    if fraction == 0.0 { return value; }
    let negative = value.is_sign_negative();
    match mode {
        "ceil" => value.ceil(), "floor" => floor,
        "expand" => if negative { floor } else { value.ceil() },
        "trunc" => value.trunc(),
        _ if fraction < 0.5 => floor,
        _ if fraction > 0.5 => value.ceil(),
        "halfEven" => if floor.rem_euclid(2.0) == 0.0 { floor } else { value.ceil() },
        "halfTrunc" => if negative { value.ceil() } else { floor },
        "halfFloor" => floor,
        "halfCeil" => value.ceil(),
        _ => if negative { floor } else { value.ceil() },
    }
}

// ECMA-402 rounds the decimal representation of a Number. Multiplying the
// binary float by a power of ten first changes ties such as 1.015 into 1.01499.
fn intl_round_decimal(value: f64, negative: bool, fraction_digits: i32,
    increment: usize, mode: &str) -> Option<String> {
    let decimal = value.to_string();
    let (whole, fraction) = decimal.split_once('.').unwrap_or((&decimal, ""));
    if !whole.bytes().chain(fraction.bytes()).all(|byte| byte.is_ascii_digit()) { return None; }
    let coefficient = whole.bytes().chain(fraction.bytes()).try_fold(0u128, |number, byte| {
        number.checked_mul(10)?.checked_add((byte - b'0') as u128)
    })?;
    // `d` is the number of decimal places between the exact value and the
    // requested rounding position. It is negative when rounding to a place
    // left of the decimal point (significant digits above the leading digit).
    let d: i64 = fraction.len() as i64 - fraction_digits as i64;
    let (numerator, denominator) = if d >= 0 {
        (coefficient, (increment as u128).checked_mul(10u128.checked_pow(d as u32)?)?)
    } else {
        (coefficient.checked_mul(10u128.checked_pow((-d) as u32)?)?, increment as u128)
    };
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let above_half = remainder > denominator - remainder;
    let tie = remainder == denominator - remainder;
    let increase = match mode {
        "ceil" => !negative && remainder != 0,
        "floor" => negative && remainder != 0,
        "expand" => remainder != 0,
        "trunc" => false,
        "halfCeil" => above_half || (tie && !negative),
        "halfFloor" => above_half || (tie && negative),
        "halfTrunc" => above_half,
        "halfEven" => above_half || (tie && quotient % 2 == 1),
        _ => above_half || tie,
    };
    let units = quotient.checked_add(u128::from(increase))?.checked_mul(increment as u128)?;
    if fraction_digits > 0 {
        let scale = 10u128.checked_pow(fraction_digits as u32)?;
        Some(format!("{}.{:0width$}", units / scale, units % scale, width = fraction_digits as usize))
    } else if fraction_digits == 0 {
        Some(units.to_string())
    } else {
        let scale = 10u128.checked_pow((-fraction_digits) as u32)?;
        Some(units.checked_mul(scale)?.to_string())
    }
}

fn intl_localize_digits(source: &str, numbering: &str) -> String {
    let zero = match numbering { "arab" => '٠', "deva" => '०', _ => return source.to_string() };
    source.chars().map(|ch| if let Some(digit) = ch.to_digit(10) {
        char::from_u32(zero as u32 + digit).unwrap()
    } else { ch }).collect()
}

/// Unit prefix/suffix and the unit-name segment for one locale and style.
fn intl_unit_for(locale: &str, unit: &str, style: &str) -> Option<&'static IntlUnitData> {
    INTL_UNIT_DATA.iter().find(|entry| entry.locale == locale && entry.unit == unit && entry.style == style)
}

/// Split a unit affix into its literal and unit parts, marking the unit name.
fn intl_unit_pattern_parts(text: &str, unit_name: &str) -> Vec<(&'static str, String)> {
    let mut parts = Vec::new();
    if let Some(position) = text.find(unit_name) {
        if position > 0 { parts.push(("literal", text[..position].to_string())); }
        parts.push(("unit", unit_name.to_string()));
        let after = &text[position + unit_name.len()..];
        if !after.is_empty() { parts.push(("literal", after.to_string())); }
    } else if !text.is_empty() {
        parts.push(("literal", text.to_string()));
    }
    parts
}

/// Exact decimal formatting for String arguments (ECMA-402 ToIntlMathematicalValue).
/// Returns `None` for shapes the f64 path must handle (non-decimal strings,
/// currency/unit/percent styles, significant digits, increments, other notations).
fn intl_format_decimal_string(state: &IntlNumberState, text: &str) -> Option<Vec<(&'static str, String)>> {
    if state.style != "decimal" || state.notation != "standard"
        || state.min_significant.is_some() || state.max_significant.is_some()
        || state.rounding_increment != 1
    {
        return None;
    }
    let (negative, body) = if let Some(rest) = text.strip_prefix('-') { (true, rest) }
        else if let Some(rest) = text.strip_prefix('+') { (false, rest) } else { (false, text) };
    if body.is_empty() || body.matches('.').count() > 1 { return None; }
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    if whole.is_empty() && fraction.is_empty() { return None; }
    if !whole.bytes().all(|byte| byte.is_ascii_digit()) || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut whole_digits: Vec<u8> = whole.bytes().map(|byte| byte - b'0').collect();
    if whole_digits.is_empty() { whole_digits.push(0); }
    let mut fraction_digits: Vec<u8> = fraction.bytes().map(|byte| byte - b'0').collect();
    if fraction_digits.len() > state.max_fraction {
        let keep = state.max_fraction;
        let next = fraction_digits[keep];
        let rest_nonzero = fraction_digits[keep + 1..].iter().any(|digit| *digit != 0);
        let last_odd = keep > 0 && fraction_digits[keep - 1] % 2 == 1;
        let round_up = match state.rounding_mode.as_str() {
            "trunc" | "floor" => false,
            "halfTrunc" => next > 5,
            "halfEven" => next > 5 || (next == 5 && (rest_nonzero || last_odd)),
            _ => next >= 5,
        };
        fraction_digits.truncate(keep);
        if round_up {
            let mut carry = true;
            for digit in fraction_digits.iter_mut().rev() {
                if *digit == 9 { *digit = 0; } else { *digit += 1; carry = false; break; }
            }
            if carry {
                for digit in whole_digits.iter_mut().rev() {
                    if *digit == 9 { *digit = 0; } else { *digit += 1; carry = false; break; }
                }
                if carry { whole_digits.insert(0, 1); }
            }
        }
    }
    while fraction_digits.len() < state.min_fraction { fraction_digits.push(0); }
    while fraction_digits.len() > state.min_fraction && fraction_digits.last() == Some(&0) {
        fraction_digits.pop();
    }
    let whole_text: String = whole_digits.iter().map(|digit| (b'0' + *digit) as char).collect();
    let fraction_text: String = fraction_digits.iter().map(|digit| (b'0' + *digit) as char).collect();
    let data = intl_data(&state.locale);
    let mut parts: Vec<(&'static str, String)> = Vec::new();
    let rounded_zero = whole_digits.iter().all(|digit| *digit == 0) && fraction_digits.iter().all(|digit| *digit == 0);
    let sign = match state.sign_display.as_str() {
        "never" => None,
        "always" => Some(if negative { ("minusSign", data.minus) } else { ("plusSign", data.plus) }),
        "exceptZero" if rounded_zero => None,
        "exceptZero" => Some(if negative { ("minusSign", data.minus) } else { ("plusSign", data.plus) }),
        "negative" if rounded_zero => None,
        _ if negative => Some(("minusSign", data.minus)),
        _ => None,
    };
    if let Some((kind, value)) = sign { parts.push((kind, value.to_string())); }
    let padded = format!("{:0>width$}", whole_text, width = state.min_integer);
    let grouped = state.use_grouping != "false" && padded.len() >= 4
        && (state.use_grouping != "min2" || padded.len() >= 5);
    if grouped {
        let indian = data.decimal_pattern.contains("#,##,##");
        let mut groups = Vec::new();
        let mut rest = padded.as_str();
        let mut size = 3;
        while rest.len() > size {
            groups.push(rest[rest.len() - size..].to_string());
            rest = &rest[..rest.len() - size];
            if indian { size = 2; }
        }
        groups.push(rest.to_string());
        groups.reverse();
        for (index, group) in groups.into_iter().enumerate() {
            if index > 0 { parts.push(("group", data.group.to_string())); }
            parts.push(("integer", intl_localize_digits(&group, &state.numbering_system)));
        }
    } else {
        parts.push(("integer", intl_localize_digits(&padded, &state.numbering_system)));
    }
    if !fraction_text.is_empty() {
        parts.push(("decimal", data.decimal.to_string()));
        parts.push(("fraction", intl_localize_digits(&fraction_text, &state.numbering_system)));
    }
    Some(parts)
}

/// Compact notation scale and suffix for the largest generated magnitude that
/// does not exceed the value's decimal exponent. A magnitude whose CLDR
/// pattern has no suffix (for example German short thousands) yields `None`,
/// which falls back to ordinary decimal formatting.
fn intl_compact_for(locale: &str, style: &str, exponent: i32) -> Option<(f64, &'static str)> {
    let mut best: Option<&IntlCompactData> = None;
    for entry in INTL_COMPACT_DATA.iter()
        .filter(|entry| entry.locale == locale && entry.style == style)
    {
        if (entry.magnitude as i32) <= exponent
            && best.map_or(true, |current| entry.magnitude > current.magnitude)
        {
            best = Some(entry);
        }
    }
    best.filter(|entry| !entry.suffix.is_empty())
        .map(|entry| (10f64.powi(entry.divisor as i32), entry.suffix))
}

fn intl_format_number(state: &IntlNumberState, input: f64) -> (String, Vec<(&'static str, String)>) {
    let data = intl_data(&state.locale);
    let mut parts = Vec::new();
    let negative = input.is_sign_negative();
    let mut rounded_zero = input == 0.0;
    let mut value = input.abs();
    if state.style == "percent" { value *= 100.0; }
    let mut exponent = 0i32;
    if (state.notation == "scientific" || state.notation == "engineering") && value.is_finite() && value != 0.0 {
        let order = value.log10().floor() as i32;
        exponent = if state.notation == "engineering" { order.div_euclid(3) * 3 } else { order };
        value = if exponent < -308 {
            value * 1e308 * 10f64.powi(-exponent - 308)
        } else { value / 10f64.powi(exponent) };
    }
    let mut compact_suffix = "";
    if state.notation == "compact" && value.is_finite() && value != 0.0 {
        if let Some((scale, suffix)) = intl_compact_for(&state.locale, &state.compact_display, value.log10().floor() as i32) {
            value /= scale;
            compact_suffix = suffix;
        }
    }
    let mut number_parts = Vec::new();
    if value.is_nan() {
        number_parts.push(("nan", data.nan.to_string()));
    } else if value.is_infinite() {
        number_parts.push(("infinity", data.infinity.to_string()));
    } else {
        let digits_before = if value == 0.0 { 1 } else { value.log10().floor() as i32 + 1 };
        let sig_min = (state.min_significant.unwrap_or(1) as i32 - digits_before).max(0);
        let sig_max = state.max_significant.unwrap_or(21) as i32 - digits_before;
        let use_significant = match state.rounding_priority.as_str() {
            "morePrecision" | "lessPrecision" => {
                let fixed_more_precise = -(state.max_fraction as i32)
                    < digits_before - state.max_significant.unwrap_or(21) as i32;
                (state.rounding_priority == "morePrecision" && !fixed_more_precise)
                    || (state.rounding_priority == "lessPrecision" && fixed_more_precise)
            }
            _ => state.max_significant.is_some(),
        };
        let (min_fraction, max_fraction) = if use_significant { (sig_min, sig_max) }
            else { (state.min_fraction as i32, state.max_fraction as i32) };
        let scale = 10f64.powi(max_fraction.clamp(0, 100));
        let increment = state.rounding_increment as f64;
        let signed = if negative { -value } else { value };
        let rounded = intl_round_scaled(signed * scale / increment, &state.rounding_mode).abs() * increment / scale;
        let mut digits = intl_round_decimal(value, negative, max_fraction, state.rounding_increment,
            &state.rounding_mode).unwrap_or_else(|| format!("{:.*}", max_fraction.clamp(0, 100) as usize, rounded));
        if let Some(dot) = digits.find('.') {
            while digits.len() > dot + 1 + min_fraction as usize && digits.ends_with('0') { digits.pop(); }
            if digits.ends_with('.') { digits.pop(); }
        }
        rounded_zero = digits.bytes().all(|byte| byte == b'0' || byte == b'.');
        if state.trailing_zero_display == "stripIfInteger" && rounded.fract() == 0.0 {
            digits.truncate(digits.find('.').unwrap_or(digits.len()));
        }
        let (integer, fraction) = digits.split_once('.').unwrap_or((&digits, ""));
        let padded = format!("{:0>width$}", integer, width = state.min_integer);
        let grouped = state.notation == "standard" || state.notation == "compact";
        let grouped = grouped && state.use_grouping != "false" && padded.len() >= 4
            && (state.use_grouping != "min2" || padded.len() >= 5);
        if grouped {
            let indian = data.decimal_pattern.contains("#,##,##");
            let mut groups = Vec::new();
            let mut rest = padded.as_str();
            let mut size = 3;
            while rest.len() > size {
                groups.push(rest[rest.len() - size..].to_string());
                rest = &rest[..rest.len() - size];
                if indian { size = 2; }
            }
            groups.push(rest.to_string());
            groups.reverse();
            for (index, group) in groups.into_iter().enumerate() {
                if index > 0 { number_parts.push(("group", data.group.to_string())); }
                number_parts.push(("integer", intl_localize_digits(&group, &state.numbering_system)));
            }
        } else {
            number_parts.push(("integer", intl_localize_digits(&padded, &state.numbering_system)));
        }
        if !fraction.is_empty() {
            number_parts.push(("decimal", data.decimal.to_string()));
            number_parts.push(("fraction", intl_localize_digits(fraction, &state.numbering_system)));
        }
        if !compact_suffix.is_empty() {
            let trimmed = compact_suffix.trim_start_matches(|ch: char| !ch.is_alphanumeric());
            let lead = compact_suffix.len() - trimmed.len();
            if lead > 0 { number_parts.push(("literal", compact_suffix[..lead].to_string())); }
            if !trimmed.is_empty() { number_parts.push(("compact", trimmed.to_string())); }
        }
        if state.notation == "scientific" || state.notation == "engineering" {
            number_parts.push(("exponentSeparator", "E".into()));
            if exponent < 0 { number_parts.push(("exponentMinusSign", data.minus.into())); }
            number_parts.push(("exponentInteger", intl_localize_digits(&exponent.unsigned_abs().to_string(), &state.numbering_system)));
        }
    }
    let sign = match state.sign_display.as_str() {
        "never" => None,
        "always" => Some(if negative { ("minusSign", data.minus) } else { ("plusSign", data.plus) }),
        "exceptZero" if rounded_zero || value.is_nan() => None,
        "exceptZero" => Some(if negative { ("minusSign", data.minus) } else { ("plusSign", data.plus) }),
        "negative" if rounded_zero => None,
        _ if negative => Some(("minusSign", data.minus)),
        _ => None,
    };
    let accounting = state.style == "currency" && state.currency_sign == "accounting"
        && data.currency_accounting_pattern.split(';').nth(1).is_some_and(|pattern| pattern.contains('('))
        && sign.is_some_and(|(kind, _)| kind == "minusSign");
    if accounting { parts.push(("literal", "(".into())); }
    else if let Some((kind, value)) = sign { parts.push((kind, value.to_string())); }
    let currency = state.currency.as_ref().and_then(|code| INTL_CURRENCY_DATA.iter().find(|entry| entry.code == code));
    let currency_text = state.currency.as_ref().map(|code| {
        let index = INTL_NUMBER_DATA.iter().position(|data| data.locale == state.locale).unwrap_or(0);
        match state.currency_display.as_str() {
            "code" => code.clone(),
            "name" => currency.map_or(code.clone(), |entry| entry.names[index].to_string()),
            "narrowSymbol" => currency.map_or(code.clone(), |entry| entry.narrow_symbols[index].to_string()),
            _ => currency.map_or(code.clone(), |entry| entry.symbols[index].to_string()),
        }
    });
    let currency_before = data.currency_pattern.find('¤').unwrap_or(0) < data.currency_pattern.find('0').unwrap_or(0);
    if state.style == "currency" && currency_before {
        parts.push(("currency", currency_text.clone().unwrap_or_default()));
        if data.currency_pattern.contains("¤ ") || data.currency_pattern.contains("¤ ") { parts.push(("literal", " ".into())); }
    }
    parts.extend(number_parts);
    if state.style == "percent" {
        if data.percent_pattern.find('%').unwrap_or(usize::MAX) < data.percent_pattern.find('0').unwrap_or(0) {
            parts.insert(0, ("percentSign", data.percent.to_string()));
        } else {
            if data.percent_pattern.contains(" %") || data.percent_pattern.contains(" %") { parts.push(("literal", " ".into())); }
            parts.push(("percentSign", data.percent.to_string()));
        }
    }
    if state.style == "currency" && !currency_before {
        if data.currency_pattern.contains(" ¤") || data.currency_pattern.contains(" ¤") { parts.push(("literal", " ".into())); }
        parts.push(("currency", currency_text.unwrap_or_default()));
    }
    if state.style == "unit" {
        let unit_id = state.unit.clone().unwrap_or_default();
        match intl_unit_for(&state.locale, &unit_id, &state.unit_display) {
            Some(entry) => {
                if !entry.prefix.is_empty() {
                    let prefix_parts = intl_unit_pattern_parts(entry.prefix, entry.unit_name);
                    for (offset, part) in prefix_parts.into_iter().enumerate() {
                        parts.insert(offset, part);
                    }
                }
                parts.extend(intl_unit_pattern_parts(entry.suffix, entry.unit_name));
            }
            None => {
                parts.push(("literal", " ".into()));
                parts.push(("unit", unit_id));
            }
        }
    }
    if accounting { parts.push(("literal", ")".into())); }
    let text = parts.iter().map(|(_, value)| value.as_str()).collect();
    (text, parts)
}

fn intl_round_bigint_digits(digits: &str, max_significant: usize, mode: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.len() <= max_significant || max_significant == 0 {
        return if trimmed.is_empty() { "0".into() } else { trimmed.to_string() };
    }
    let mut kept: Vec<u8> = trimmed.as_bytes()[..max_significant].iter().map(|byte| byte - b'0').collect();
    let next = trimmed.as_bytes()[max_significant] - b'0';
    let rest_nonzero = trimmed.as_bytes()[max_significant + 1..].iter().any(|byte| *byte != b'0');
    let last_odd = kept.last().is_some_and(|digit| digit % 2 == 1);
    let round_up = match mode {
        "trunc" | "floor" => false,
        "expand" => next != 0 || rest_nonzero,
        "halfTrunc" => next > 5,
        "halfEven" => next > 5 || (next == 5 && (rest_nonzero || last_odd)),
        _ => next >= 5,
    };
    let mut carry = round_up;
    for index in (0..kept.len()).rev() {
        if !carry { break; }
        if kept[index] == 9 { kept[index] = 0; } else { kept[index] += 1; carry = false; }
    }
    let trailing = trimmed.len() - max_significant;
    if carry {
        let mut result = String::from("1");
        result.push_str(&"0".repeat(trailing + 1));
        result
    } else {
        let mut result: String = kept.iter().map(|digit| (b'0' + *digit) as char).collect();
        result.push_str(&"0".repeat(trailing));
        result
    }
}

fn intl_format_bigint(state: &IntlNumberState, input: &str) -> String {
    let data = intl_data(&state.locale);
    let negative = input.starts_with('-');
    let mut digits = input.trim_start_matches('-').to_string();
    if state.style == "percent" { digits.push_str("00"); }
    if let Some(max_significant) = state.max_significant {
        digits = intl_round_bigint_digits(&digits, max_significant, &state.rounding_mode);
    }
    let padded = format!("{:0>width$}", digits, width = state.min_integer);
    let mut groups = Vec::new();
    if state.use_grouping != "false" && padded.len() >= 4 {
        let mut rest = padded.as_str();
        let mut size = 3;
        while rest.len() > size {
            groups.push(rest[rest.len() - size..].to_string());
            rest = &rest[..rest.len() - size];
            if data.decimal_pattern.contains("#,##,##") { size = 2; }
        }
        groups.push(rest.to_string());
        groups.reverse();
    } else { groups.push(padded); }
    let mut result = intl_localize_digits(&groups.join(data.group), &state.numbering_system);
    if state.min_fraction > 0 {
        result.push_str(data.decimal);
        result.push_str(&"0".repeat(state.min_fraction));
    }
    if state.style == "percent" {
        if data.percent_pattern.contains('\u{a0}') || data.percent_pattern.contains(' ') {
            result.push('\u{a0}');
        }
        result.push_str(data.percent);
    }
    if state.style == "currency" {
        let code = state.currency.as_deref().unwrap_or("");
        let index = INTL_NUMBER_DATA.iter().position(|entry| entry.locale == state.locale).unwrap_or(0);
        let currency = INTL_CURRENCY_DATA.iter().find(|entry| entry.code == code);
        let symbol = match state.currency_display.as_str() {
            "code" => code.to_string(),
            "name" => currency.map_or(code.to_string(), |entry| entry.names[index].to_string()),
            "narrowSymbol" => currency.map_or(code.to_string(), |entry| entry.narrow_symbols[index].to_string()),
            _ => currency.map_or(code.to_string(), |entry| entry.symbols[index].to_string()),
        };
        if data.currency_pattern.find('¤').unwrap_or(0) < data.currency_pattern.find('0').unwrap_or(0) {
            result = format!("{symbol}{result}");
        } else if data.currency_pattern.contains("¤\u{a0}") || data.currency_pattern.contains("¤ ") {
            result = format!("{result}\u{a0}{symbol}");
        } else { result = format!("{result} {symbol}"); }
    }
    if negative && state.sign_display != "never" { result = format!("{}{result}", data.minus); }
    else if !negative && state.sign_display == "always" { result = format!("{}{result}", data.plus); }
    result
}

#[cfg(test)]
mod intl_number_tests {
    use super::*;

    fn eval(source: &str) -> String {
        let mut i = Interpreter::new();
        let result = i.run_script(source).unwrap_or_else(|error| panic!("Intl.NumberFormat error: {error:?}"));
        i.to_string(&result).unwrap()
    }

    #[test]
    fn locale_grouping_and_decimal_symbols() {
        assert_eq!(eval("new Intl.NumberFormat('de-DE').format(12345.67)"), "12.345,67");
        assert_eq!(eval("new Intl.NumberFormat('en-US').format(12345.67)"), "12,345.67");
        assert_eq!(eval("new Intl.NumberFormat('hi-IN').format(1234567.89)"), "12,34,567.89");
    }

    #[test]
    fn percent_and_currency_use_cldr_patterns() {
        assert_eq!(eval("new Intl.NumberFormat('en', {style:'percent'}).format(0.25)"), "25%");
        assert_eq!(eval("new Intl.NumberFormat('en', {style:'currency', currency:'USD'}).format(12.5)"), "$12.50");
        assert_eq!(eval("new Intl.NumberFormat('de', {style:'currency', currency:'EUR'}).format(12.5)"), "12,50 €");
    }

    #[test]
    fn bound_format_and_parts() {
        assert_eq!(eval("let f = new Intl.NumberFormat('en', {minimumFractionDigits:2}).format; f(1234)"), "1,234.00");
        assert_eq!(eval("new Intl.NumberFormat('en').formatToParts(1234.5).map(p=>p.type+':'+p.value).join('|')"),
            "integer:1|group:,|integer:234|decimal:.|fraction:5");
    }

    #[test]
    fn decimal_ties_and_rounding_increments() {
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,maximumFractionDigits:2}).format(1.015)"), "1.02");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,minimumFractionDigits:2,maximumFractionDigits:2,roundingIncrement:10}).format(1.15)"), "1.20");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,minimumSignificantDigits:3,maximumSignificantDigits:5}).format(0)"), "0.00");
        assert_eq!(eval("new Intl.NumberFormat('en',{notation:'engineering'}).format(0.000345)"), "345E-6");
        assert_eq!(eval("new Intl.NumberFormat('en',{notation:'scientific'}).format(543211.1)"), "5.432E5");
    }

    #[test]
    fn rounding_priority_selects_precision() {
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,roundingPriority:'morePrecision',maximumSignificantDigits:2,maximumFractionDigits:2}).format(1.23)"), "1.23");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,roundingPriority:'lessPrecision',maximumSignificantDigits:2,maximumFractionDigits:2}).format(1.23)"), "1.2");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,roundingPriority:'morePrecision',minimumSignificantDigits:3,minimumFractionDigits:2}).format(1)"), "1.00");
    }

    #[test]
    fn sign_display_uses_rounded_result_and_accounting_pattern() {
        assert_eq!(eval("new Intl.NumberFormat('en',{signDisplay:'exceptZero'}).format(-0.0001)"), "0");
        assert_eq!(eval("new Intl.NumberFormat('en',{signDisplay:'negative'}).format(-0)"), "0");
        assert_eq!(eval("new Intl.NumberFormat('en',{style:'currency',currency:'USD',currencySign:'accounting'}).format(-987)"), "($987.00)");
        assert_eq!(eval("new Intl.NumberFormat('en',{signDisplay:'exceptZero'}).format(987)"), "+987");
        assert_eq!(eval("new Intl.NumberFormat('en',{signDisplay:'exceptZero'}).format(NaN)"), "NaN");
    }

    #[test]
    fn significant_digits_round_above_the_decimal_point() {
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,minimumSignificantDigits:3,maximumSignificantDigits:5}).format(123445.01)"), "123450");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,minimumSignificantDigits:3,maximumSignificantDigits:5}).format(1.2)"), "1.20");
        assert_eq!(eval("new Intl.NumberFormat('en',{useGrouping:false,minimumSignificantDigits:3,maximumSignificantDigits:5}).format(0.001234)"), "0.001234");
    }

    #[test]
    fn compact_notation_uses_generated_cldr_patterns() {
        assert_eq!(eval("new Intl.NumberFormat('en',{notation:'compact'}).format(987654321)"), "988M");
        assert_eq!(eval("new Intl.NumberFormat('en',{notation:'compact'}).format(98765)"), "99K");
        assert_eq!(eval("new Intl.NumberFormat('de',{notation:'compact'}).format(98765)"), "98.765");
        assert_eq!(eval("new Intl.NumberFormat('de',{notation:'compact'}).format(98765432)"), "99\u{a0}Mio.");
        assert_eq!(eval("new Intl.NumberFormat('de',{notation:'compact',compactDisplay:'long'}).format(98765)"), "99 Tausend");
    }

    #[test]
    fn unit_display_uses_generated_cldr_patterns() {
        assert_eq!(eval("new Intl.NumberFormat('en',{style:'unit',unit:'kilometer-per-hour'}).format(987)"), "987 km/h");
        assert_eq!(eval("new Intl.NumberFormat('en',{style:'unit',unit:'kilometer-per-hour',unitDisplay:'narrow'}).format(987)"), "987km/h");
        assert_eq!(eval("new Intl.NumberFormat('en',{style:'unit',unit:'kilometer-per-hour',unitDisplay:'long'}).format(987)"), "987 kilometers per hour");
        assert_eq!(eval("new Intl.NumberFormat('de',{style:'unit',unit:'kilometer-per-hour'}).format(987)"), "987 km/h");
    }

    #[test]
    fn string_arguments_keep_exact_decimal_precision() {
        assert_eq!(eval("let nf=new Intl.NumberFormat('en-US',{maximumFractionDigits:20}); nf.format('1.0000000000000001')"), "1.0000000000000001");
        assert_eq!(eval("new Intl.NumberFormat('en-US').format('987654321987654321')"), "987,654,321,987,654,321");
        assert_eq!(eval("new Intl.NumberFormat('en-US').format('42')"), "42");
        assert_eq!(eval("new Intl.NumberFormat('en-US').format('foo')"), "NaN");
    }

    #[test]
    fn bound_format_is_anonymous_with_length_then_name() {
        assert_eq!(eval("Object.getOwnPropertyNames(new Intl.NumberFormat().format).join(',')"), "length,name");
        assert_eq!(eval("new Intl.NumberFormat().format.name"), "");
    }

    #[test]
    fn bigint_significant_digits_round_and_group() {
        assert_eq!(eval("88776655n.toLocaleString('de',{maximumSignificantDigits:4})"), "88.780.000");
        assert_eq!(eval("88776655n.toLocaleString('de',{minimumFractionDigits:3})"), "88.776.655,000");
    }
}
