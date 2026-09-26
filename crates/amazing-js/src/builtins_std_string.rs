// ECMA-262 String constructor and prototype additions. All positions and
// slices use UTF-16 code units, including isolated surrogates.

fn std_core_define(i: &Interpreter, target: &Obj, name: &str, length: u32,
    native: crate::value::NativeFn) -> JsValue {
    let method = symbol_method(i, name, native);
    symbol_method_length(&method, length);
    target.borrow_mut().define(name, symbol_data(method.clone(), true, true)).unwrap();
    method
}

impl Interpreter {
    fn install_std_string(&mut self) {
        let prototype = self.string_proto.clone();
        for (name, length, native) in [
            ("toLowerCase", 0, std_string_lower as crate::value::NativeFn),
            ("toUpperCase", 0, std_string_upper as crate::value::NativeFn),
            ("toLocaleLowerCase", 0, std_string_locale_lower as crate::value::NativeFn),
            ("toLocaleUpperCase", 0, std_string_locale_upper as crate::value::NativeFn),
            ("trimStart", 0, std_string_trim_start as crate::value::NativeFn),
            ("trimEnd", 0, std_string_trim_end as crate::value::NativeFn),
            ("padStart", 1, std_string_pad_start as crate::value::NativeFn),
            ("padEnd", 1, std_string_pad_end as crate::value::NativeFn),
            ("repeat", 1, std_string_repeat as crate::value::NativeFn),
            ("concat", 1, std_string_concat as crate::value::NativeFn),
            ("at", 1, std_string_at as crate::value::NativeFn),
            ("lastIndexOf", 1, std_string_last_index_of as crate::value::NativeFn),
            ("replaceAll", 2, std_string_replace_all as crate::value::NativeFn),
            ("matchAll", 1, std_string_match_all as crate::value::NativeFn),
            ("substr", 2, std_string_substr as crate::value::NativeFn),
            ("anchor", 1, std_string_html as crate::value::NativeFn),
            ("big", 0, std_string_html as crate::value::NativeFn),
            ("blink", 0, std_string_html as crate::value::NativeFn),
            ("bold", 0, std_string_html as crate::value::NativeFn),
            ("fixed", 0, std_string_html as crate::value::NativeFn),
            ("fontcolor", 1, std_string_html as crate::value::NativeFn),
            ("fontsize", 1, std_string_html as crate::value::NativeFn),
            ("italics", 0, std_string_html as crate::value::NativeFn),
            ("link", 1, std_string_html as crate::value::NativeFn),
            ("small", 0, std_string_html as crate::value::NativeFn),
            ("strike", 0, std_string_html as crate::value::NativeFn),
            ("sub", 0, std_string_html as crate::value::NativeFn),
            ("sup", 0, std_string_html as crate::value::NativeFn),
        ] {
            std_core_define(self, &prototype, name, length, native);
        }
        let start = prototype.borrow().get_own("trimStart").unwrap();
        let end = prototype.borrow().get_own("trimEnd").unwrap();
        prototype.borrow_mut().define("trimLeft", symbol_data(start, true, true)).unwrap();
        prototype.borrow_mut().define("trimRight", symbol_data(end, true, true)).unwrap();
        if let Some(JsValue::Object(constructor)) = self.global_env.resolve("String") {
            std_core_define(self, &constructor, "fromCodePoint", 1, std_string_from_code_point);
            std_core_define(self, &constructor, "raw", 1, std_string_raw);
        }
    }
}

fn std_string_receiver(i: &mut Interpreter, this: JsValue) -> Result<crate::value::JsString, Throw> {
    if this.is_null() || this.is_undefined() { return Err(Throw::type_error("String receiver is null or undefined")); }
    string_this_units(i, this)
}

fn std_string_map_case(source: &crate::value::JsString, upper: bool, turkic: bool) -> crate::value::JsString {
    let mut out = Vec::new();
    let units = source.units();
    let mut at = 0;
    while at < units.len() {
        let start = at;
        while at < units.len() {
            let unit = units[at];
            if (0xd800..=0xdbff).contains(&unit) {
                if at + 1 < units.len() && (0xdc00..=0xdfff).contains(&units[at + 1]) { at += 2; continue; }
                break;
            }
            if (0xdc00..=0xdfff).contains(&unit) { break; }
            at += 1;
        }
        if at > start {
            let text = String::from_utf16(&units[start..at]).unwrap_or_default();
            let mapped = if turkic {
                if upper { text.replace('i', "İ").replace('ı', "I").to_uppercase() }
                else { text.replace("I\u{307}", "i").replace('I', "ı").replace('İ', "i").to_lowercase() }
            } else if upper { text.to_uppercase() } else { text.to_lowercase() };
            out.extend(mapped.encode_utf16());
        }
        if at < units.len() { out.push(units[at]); at += 1; }
    }
    crate::value::JsString::from_units(out)
}

fn std_string_case(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, upper: bool, locale: bool) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let turkic = if locale {
        let locale = std_reflect_arg(&args, 0);
        if locale.is_undefined() { false }
        else {
            let locale = i.to_string(&locale)?.to_ascii_lowercase();
            locale == "tr" || locale.starts_with("tr-") || locale == "az" || locale.starts_with("az-")
        }
    } else { false };
    Ok(JsValue::str(std_string_map_case(&source, upper, turkic)))
}
fn std_string_lower(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_case(i, this, args, false, false) }
fn std_string_upper(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_case(i, this, args, true, false) }
fn std_string_locale_lower(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_case(i, this, args, false, true) }
fn std_string_locale_upper(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_case(i, this, args, true, true) }

fn std_string_space(unit: u16) -> bool {
    matches!(unit, 0x0009..=0x000d | 0x0020 | 0x00a0 | 0x1680 | 0x2000..=0x200a
        | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}
fn std_string_trim_start(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let start = source.units().iter().position(|unit| !std_string_space(*unit)).unwrap_or(source.len());
    Ok(JsValue::str_units(source.units()[start..].to_vec()))
}
fn std_string_trim_end(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let end = source.units().iter().rposition(|unit| !std_string_space(*unit)).map_or(0, |n| n + 1);
    Ok(JsValue::str_units(source.units()[..end].to_vec()))
}

fn std_string_length(i: &mut Interpreter, value: JsValue) -> Result<usize, Throw> {
    let number = i.to_number(&value)?;
    if number.is_nan() || number <= 0.0 { return Ok(0); }
    if number.is_infinite() { return Ok(usize::MAX); }
    Ok(number.trunc().min(9_007_199_254_740_991.0) as usize)
}
fn std_string_pad(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, start: bool) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let target = std_string_length(i, std_reflect_arg(&args, 0))?;
    if target <= source.len() { return Ok(JsValue::str(source)); }
    if target > 16_777_216 { return Err(Throw::range_error("string length exceeds implementation limit")); }
    let fill = if args.get(1).is_none_or(JsValue::is_undefined) { crate::value::JsString::from(" ") }
        else { i.to_js_string(&args[1])? };
    if fill.is_empty() { return Ok(JsValue::str(source)); }
    let needed = target - source.len();
    let mut padding = Vec::with_capacity(needed);
    while padding.len() < needed {
        let count = (needed - padding.len()).min(fill.len());
        padding.extend_from_slice(&fill.units()[..count]);
    }
    let mut out = Vec::with_capacity(target);
    if start { out.extend(padding); out.extend_from_slice(source.units()); }
    else { out.extend_from_slice(source.units()); out.extend(padding); }
    Ok(JsValue::str_units(out))
}
fn std_string_pad_start(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_pad(i, this, args, true) }
fn std_string_pad_end(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> { std_string_pad(i, this, args, false) }
fn std_string_repeat(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let count = i.to_number(&std_reflect_arg(&args, 0))?;
    if count < 0.0 || count.is_infinite() { return Err(Throw::range_error("invalid repeat count")); }
    let count = if count.is_nan() { 0 } else { count.trunc() as usize };
    let size = source.len().checked_mul(count).ok_or_else(|| Throw::range_error("string length overflow"))?;
    if size > 16_777_216 { return Err(Throw::range_error("string length exceeds implementation limit")); }
    Ok(JsValue::str_units(source.units().repeat(count)))
}
fn std_string_concat(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let mut out = source.units().to_vec();
    for arg in args {
        let part = i.to_js_string(&arg)?;
        if out.len().saturating_add(part.len()) > 16_777_216 { return Err(Throw::range_error("string length exceeds implementation limit")); }
        out.extend_from_slice(part.units());
    }
    Ok(JsValue::str_units(out))
}
fn std_string_at(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let number = i.to_number(&std_reflect_arg(&args, 0))?;
    let index = if number.is_nan() { 0.0 } else { number.trunc() };
    let index = if index < 0.0 { source.len() as f64 + index } else { index };
    if !(0.0..source.len() as f64).contains(&index) { return Ok(JsValue::Undefined); }
    Ok(JsValue::str_units(vec![source.units()[index as usize]]))
}
fn std_string_last_index_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let search = i.to_js_string(&std_reflect_arg(&args, 0))?;
    let position = args.get(1).filter(|arg| !arg.is_undefined());
    let n = if let Some(value) = position { i.to_number(value)? } else { f64::INFINITY };
    let max = if n.is_nan() { f64::INFINITY } else { n.trunc() }.clamp(0.0, source.len() as f64) as usize;
    let result = if search.is_empty() { Some(max) }
        else if search.len() > source.len() { None }
        else { source.units()[..max.min(source.len() - search.len()) + search.len()]
            .windows(search.len()).rposition(|part| part == search.units()) };
    Ok(JsValue::num(result.map_or(-1.0, |n| n as f64)))
}

fn std_string_substr(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let len = source.len() as f64;
    let start = i.to_number(&std_reflect_arg(&args, 0))?;
    let start = if start.is_nan() { 0.0 } else { start.trunc() };
    let start = if start < 0.0 { (len + start).max(0.0) } else { start.min(len) } as usize;
    let count = if let Some(value) = args.get(1) {
        if value.is_undefined() { len as usize } else { std_string_length(i, value.clone())? }
    } else { len as usize };
    let end = start.saturating_add(count).min(source.len());
    Ok(JsValue::str_units(source.units()[start..end].to_vec()))
}

fn std_string_replacement(i: &mut Interpreter, replacement: &JsValue,
    source: &crate::value::JsString, search: &crate::value::JsString, position: usize) -> Result<Vec<u16>, Throw> {
    if callable_function(replacement).is_some() {
        let value = i.call_value(replacement, JsValue::Undefined, vec![JsValue::str(search.clone()),
            JsValue::num(position as f64), JsValue::str(source.clone())])?;
        return Ok(i.to_js_string(&value)?.units().to_vec());
    }
    let template = i.to_js_string(replacement)?;
    let mut out = Vec::new();
    let mut at = 0;
    while at < template.len() {
        if template.units()[at] == b'$' as u16 && at + 1 < template.len() {
            match template.units()[at + 1] {
                0x24 => out.push(0x24),
                0x26 => out.extend_from_slice(search.units()),
                0x60 => out.extend_from_slice(&source.units()[..position]),
                0x27 => out.extend_from_slice(&source.units()[position + search.len()..]),
                _ => { out.push(0x24); at += 1; continue; }
            }
            at += 2;
        } else { out.push(template.units()[at]); at += 1; }
    }
    Ok(out)
}
fn std_is_regexp(i: &mut Interpreter, value: &JsValue) -> Result<bool, Throw> {
    if !value.is_object() { return Ok(false); }
    let matcher = i.get_prop(value, WellKnownSymbol::Match.symbol())?;
    if !matcher.is_undefined() { return Ok(matcher.to_boolean()); }
    Ok(regexp_data(value).is_some())
}

fn std_string_replace_all(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() { return Err(Throw::type_error("String receiver is null or undefined")); }
    let search_value = std_reflect_arg(&args, 0);
    // IsRegExp reads @@match before this is coerced to a string, so an abrupt
    // @@match getter wins over a throwing receiver toString.
    let is_regexp = if search_value.is_null() || search_value.is_undefined() {
        false
    } else {
        std_is_regexp(i, &search_value)?
    };
    let internal_regexp = regexp_data(&search_value);
    // Step 2.b validates flags and globality before step 3 coerces this.
    if is_regexp {
        let flags = i.get_prop(&search_value, "flags")?;
        if flags.is_null() || flags.is_undefined() {
            return Err(Throw::type_error("replaceAll requires RegExp flags"));
        }
        if !i.to_string(&flags)?.contains('g')
            || internal_regexp.as_ref().is_some_and(|regex| !regex.flags().global)
        {
            return Err(Throw::type_error("replaceAll requires a global RegExp"));
        }
    }
    let source = std_string_receiver(i, this)?;
    if is_regexp {
        if internal_regexp.is_some() {
            return regexp_replace(i, &source, &args);
        }
        let replacer = i.get_prop(&search_value, WellKnownSymbol::Replace.symbol())?;
        if callable_function(&replacer).is_none() {
            return Err(Throw::type_error("replaceAll requires a global RegExp"));
        }
        return i.call_value(&replacer, search_value, vec![JsValue::str(source), std_reflect_arg(&args, 1)]);
    }
    let search = i.to_js_string(&search_value)?;
    let replacement = std_reflect_arg(&args, 1);
    let mut positions = Vec::new();
    if search.is_empty() { positions.extend(0..=source.len()); }
    else {
        let mut at = 0;
        while at + search.len() <= source.len() {
            if source.units()[at..at + search.len()] == *search.units() {
                positions.push(at); at += search.len();
            } else { at += 1; }
        }
    }
    let mut out = Vec::new();
    let mut end = 0;
    for position in positions {
        out.extend_from_slice(&source.units()[end..position]);
        out.extend(std_string_replacement(i, &replacement, &source, &search, position)?);
        end = position + search.len();
        if out.len() > 16_777_216 { return Err(Throw::range_error("string length exceeds implementation limit")); }
    }
    out.extend_from_slice(&source.units()[end..]);
    Ok(JsValue::str_units(out))
}
fn std_string_match_all(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() { return Err(Throw::type_error("String receiver is null or undefined")); }
    let value = std_reflect_arg(&args, 0);
    if value.is_object() {
        if std_is_regexp(i, &value)? {
            let flags = i.get_prop(&value, "flags")?;
            if flags.is_null() || flags.is_undefined() {
                return Err(Throw::type_error("matchAll requires RegExp flags"));
            }
            if !i.to_string(&flags)?.contains('g') {
                return Err(Throw::type_error("matchAll requires a global RegExp"));
            }
        }
        let matcher = i.get_prop(&value, WellKnownSymbol::MatchAll.symbol())?;
        if !matcher.is_null() && !matcher.is_undefined() {
            if callable_function(&matcher).is_none() {
                return Err(Throw::type_error("@@matchAll is not a function"));
            }
            let source = std_string_receiver(i, this)?;
            return i.call_value(&matcher, value, vec![JsValue::str(source)]);
        }
    }
    let source = std_string_receiver(i, this)?;
    let regex = if regexp_data(&value).is_some() {
        value
    } else {
        native_regexp(i, JsValue::Undefined, vec![value, JsValue::str("g")])?
    };
    // Collect RegExp match records ([captures…, index, input, groups]) rather
    // than the matched substrings, and advance over empty matches so the
    // global lastIndex cannot stall.
    let units = regexp_units(&source)?;
    let regex_inner = regexp_data(&regex).unwrap();
    let mut values = Vec::new();
    loop {
        let matched = regexp_match(i, &regex, &source)?;
        let Some(matched) = matched else { break };
        if values.len() >= 100_000 { return Err(Throw::range_error("matchAll collection limit")); }
        let empty = matched.range.is_empty();
        values.push(regexp_result(i, &source, matched)?);
        if empty {
            let last_token = i.get_prop(&regex, "lastIndex")?;
            let last = i.to_number(&last_token)?;
            let next = amazing_regex::advance_index(&units, last.max(0.0) as usize, regex_inner.flags().unicode);
            i.set_prop(&regex, "lastIndex", JsValue::num(next as f64))?;
        }
    }
    let array = array_values(i, values)?;
    native_array_iterator_values(i, array, vec![])
}

fn std_string_html(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_string_receiver(i, this)?;
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().map(|function| function.name.clone()))
        .unwrap_or_default();
    let (tag, attribute) = match name.as_str() {
        "anchor" => ("a", Some("name")), "link" => ("a", Some("href")),
        "fontcolor" => ("font", Some("color")), "fontsize" => ("font", Some("size")),
        "italics" => ("i", None), "bold" => ("b", None), "fixed" => ("tt", None),
        "strike" => ("strike", None), "small" => ("small", None), "big" => ("big", None),
        "blink" => ("blink", None), "sub" => ("sub", None), "sup" => ("sup", None),
        _ => return Err(Throw::type_error("invalid String HTML method")),
    };
    let mut text = format!("<{tag}").encode_utf16().collect::<Vec<_>>();
    if let Some(attribute) = attribute {
        text.extend(format!(" {attribute}=\"").encode_utf16());
        let value = i.to_js_string(&std_reflect_arg(&args, 0))?;
        for &unit in value.units() {
            if unit == b'"' as u16 { text.extend("&quot;".encode_utf16()); }
            else { text.push(unit); }
        }
        text.push(b'"' as u16);
    }
    text.push(b'>' as u16);
    text.extend_from_slice(source.units());
    text.extend(format!("</{tag}>").encode_utf16());
    Ok(JsValue::str_units(text))
}

fn std_string_from_code_point(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut units = Vec::new();
    for arg in args {
        let number = i.to_number(&arg)?;
        if !number.is_finite() || number.fract() != 0.0 || !(0.0..=0x10ffff as f64).contains(&number) {
            return Err(Throw::range_error("invalid code point"));
        }
        let point = number as u32;
        if point <= 0xffff { units.push(point as u16); }
        else {
            let point = point - 0x10000;
            units.push((0xd800 + (point >> 10)) as u16);
            units.push((0xdc00 + (point & 0x3ff)) as u16);
        }
    }
    Ok(JsValue::str_units(units))
}
fn std_string_raw(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let template = std_reflect_arg(&args, 0);
    if template.is_null() || template.is_undefined() { return Err(Throw::type_error("template is null or undefined")); }
    let raw = i.get_prop(&template, "raw")?;
    if raw.is_null() || raw.is_undefined() { return Err(Throw::type_error("raw template is null or undefined")); }
    let length = i.get_prop(&raw, "length")?;
    let count = std_string_length(i, length)?;
    if count > 100_000 { return Err(Throw::range_error("raw template length exceeds implementation limit")); }
    let mut out = Vec::new();
    for index in 0..count {
        let value = i.get_prop(&raw, &index.to_string())?;
        let part = i.to_js_string(&value)?;
        out.extend_from_slice(part.units());
        if index + 1 < count {
            if let Some(value) = args.get(index + 1) {
                let part = i.to_js_string(value)?;
                out.extend_from_slice(part.units());
            }
        }
        if out.len() > 16_777_216 { return Err(Throw::range_error("string length exceeds implementation limit")); }
    }
    Ok(JsValue::str_units(out))
}
