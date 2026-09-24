// ECMA-402 locale negotiation and Intl service installation. Locale tables
// come from the pinned CLDR JSON generator, not from the host OS.

#[derive(Clone)]
enum IntlState {
    Locale(String),
    Number(IntlNumberState),
    BoundNumber(IntlNumberState),
    Plural(IntlPluralState),
    DateTime(IntlDateState),
    BoundDateTime(IntlDateState),
    Relative(IntlRelativeState),
    List(IntlListState),
    Collator(IntlCollatorState),
    BoundCollator(IntlCollatorState),
    Segmenter(IntlSegmenterState),
    Segments(IntlSegmentsState),
    SegmentIterator(IntlSegmentIteratorState),
    DisplayNames(IntlDisplayState),
}

thread_local! {
    static INTL_OBJECTS: RefCell<HashMap<usize, (Weak<RefCell<JsObject>>, IntlState)>> =
        RefCell::new(HashMap::new());
}

fn intl_register(object: &Obj, state: IntlState) {
    INTL_OBJECTS.with(|entries| {
        entries.borrow_mut().insert(Rc::as_ptr(&object.0) as usize, (Rc::downgrade(&object.0), state));
    });
}

fn intl_state(object: &Obj) -> Option<IntlState> {
    INTL_OBJECTS.with(|entries| {
        let entries = entries.borrow();
        let (weak, state) = entries.get(&(Rc::as_ptr(&object.0) as usize))?;
        weak.upgrade().filter(|live| Rc::ptr_eq(live, &object.0)).map(|_| state.clone())
    })
}

fn intl_data(locale: &str) -> &'static IntlNumberData {
    INTL_NUMBER_DATA.iter().find(|data| data.locale == locale).unwrap_or(&INTL_NUMBER_DATA[0])
}

fn intl_empty_options() -> JsValue {
    // ECMA-402 uses ObjectCreate(null) when the options argument is absent.
    JsValue::object(JsObject::ordinary())
}

fn intl_unicode_keyword(tag: &str, wanted: &str) -> Option<String> {
    let extension = tag.split("-u-").nth(1)?;
    let subtags: Vec<&str> = extension.split('-').take_while(|part| part.len() != 1).collect();
    let mut index = 0;
    while index < subtags.len() {
        if subtags[index].len() != 2 { index += 1; continue; }
        let key = subtags[index];
        index += 1;
        let start = index;
        while index < subtags.len() && subtags[index].len() > 2 { index += 1; }
        if key == wanted { return Some(subtags[start..index].join("-")); }
    }
    None
}

fn intl_alias(aliases: &'static [(&'static str, &'static str)], input: &str) -> Option<&'static str> {
    aliases.iter().find(|(old, _)| old.eq_ignore_ascii_case(input)).map(|(_, new)| *new)
}

fn intl_title(input: &str) -> String {
    let mut chars = input.chars();
    chars.next().map(|first| first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()).unwrap_or_default()
}

fn intl_canonical_tag(input: &str) -> Result<String, Throw> {
    if input.is_empty() || !input.is_ascii() || input.contains('_') {
        return Err(Throw::range_error("invalid language tag"));
    }
    let parts: Vec<&str> = input.split('-').collect();
    if parts.iter().any(|part| part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric())) {
        return Err(Throw::range_error("invalid language tag"));
    }
    let language = parts[0];
    if !(2..=8).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(Throw::range_error("invalid language tag"));
    }
    let mut canonical = Vec::new();
    let mut language = language.to_ascii_lowercase();
    if let Some(alias) = intl_alias(INTL_LANGUAGE_ALIASES, &language) {
        language = alias.to_string();
    }
    canonical.push(language);
    let mut i = 1;
    if i < parts.len() && parts[i].len() == 4 && parts[i].bytes().all(|b| b.is_ascii_alphabetic()) {
        let script = intl_title(parts[i]);
        canonical.push(intl_alias(INTL_SCRIPT_ALIASES, &script).unwrap_or(&script).to_string());
        i += 1;
    }
    if i < parts.len() && ((parts[i].len() == 2 && parts[i].bytes().all(|b| b.is_ascii_alphabetic()))
        || (parts[i].len() == 3 && parts[i].bytes().all(|b| b.is_ascii_digit()))) {
        let region = parts[i].to_ascii_uppercase();
        canonical.push(intl_alias(INTL_REGION_ALIASES, &region).unwrap_or(&region).to_string());
        i += 1;
    }
    let mut variants = Vec::new();
    while i < parts.len() && ((5..=8).contains(&parts[i].len())
        || (parts[i].len() == 4 && parts[i].as_bytes()[0].is_ascii_digit())) {
        let variant = parts[i].to_ascii_lowercase();
        let variant = intl_alias(INTL_VARIANT_ALIASES, &variant).unwrap_or(&variant).to_string();
        if variants.contains(&variant) { return Err(Throw::range_error("duplicate language variant")); }
        variants.push(variant);
        i += 1;
    }
    variants.sort();
    canonical.extend(variants);
    let mut extensions = Vec::new();
    while i < parts.len() {
        let singleton = parts[i].to_ascii_lowercase();
        if singleton.len() != 1 || !singleton.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Throw::range_error("invalid language extension"));
        }
        i += 1;
        let start = i;
        let min = if singleton == "x" { 1 } else { 2 };
        while i < parts.len() && parts[i].len() >= min {
            i += 1;
        }
        if i == start { return Err(Throw::range_error("empty language extension")); }
        if extensions.iter().any(|(key, _): &(String, Vec<String>)| key == &singleton) {
            return Err(Throw::range_error("duplicate language extension"));
        }
        let mut subtags: Vec<String> = parts[start..i].iter().map(|part| part.to_ascii_lowercase()).collect();
        if singleton == "u" {
            let mut attributes = Vec::new();
            let mut keywords = Vec::new();
            let mut j = 0;
            while j < subtags.len() && subtags[j].len() > 2 {
                attributes.push(subtags[j].clone());
                j += 1;
            }
            attributes.sort();
            while j < subtags.len() {
                if subtags[j].len() != 2 { return Err(Throw::range_error("invalid Unicode locale extension")); }
                let key = subtags[j].clone();
                j += 1;
                let value_start = j;
                while j < subtags.len() && subtags[j].len() > 2 { j += 1; }
                if !keywords.iter().any(|(old, _): &(String, Vec<String>)| old == &key) {
                    keywords.push((key, subtags[value_start..j].to_vec()));
                }
            }
            keywords.sort_by(|a, b| a.0.cmp(&b.0));
            subtags = attributes;
            for (key, values) in keywords { subtags.push(key); subtags.extend(values); }
        }
        extensions.push((singleton.clone(), subtags));
        if singleton == "x" && i < parts.len() { return Err(Throw::range_error("private use must be last")); }
    }
    extensions.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, values) in extensions {
        canonical.push(key);
        canonical.extend(values);
    }
    Ok(canonical.join("-"))
}

fn intl_canonical_locale_list(i: &mut Interpreter, value: &JsValue) -> Result<Vec<String>, Throw> {
    if matches!(value, JsValue::Undefined) { return Ok(Vec::new()); }
    let candidates = match value {
        JsValue::Str(_) => vec![value.clone()],
        JsValue::Object(object) if matches!(intl_state(object), Some(IntlState::Locale(_))) => vec![value.clone()],
        JsValue::Object(_) => {
            let length = i.get_prop(value, "length")?;
            let length = i.to_number(&length)?.max(0.0).min(100_000.0) as usize;
            let mut values = Vec::new();
            for index in 0..length {
                let key = index.to_string();
                if let JsValue::Object(object) = value {
                    if object.borrow().has_property(&key) { values.push(i.get_prop(value, &key)?); }
                }
            }
            values
        }
        JsValue::Null => return Err(Throw::type_error("locales is null")),
        _ => Vec::new(),
    };
    let mut output = Vec::new();
    for candidate in candidates {
        let tag = match &candidate {
            JsValue::Str(_) => i.to_string(&candidate)?,
            JsValue::Object(object) => match intl_state(object) {
                Some(IntlState::Locale(tag)) => tag,
                _ => i.to_string(&candidate)?,
            },
            _ => return Err(Throw::type_error("locale must be a string or object")),
        };
        let tag = intl_canonical_tag(&tag)?;
        if !output.contains(&tag) { output.push(tag); }
    }
    Ok(output)
}

fn intl_lookup(requested: &[String]) -> String {
    for tag in requested {
        let no_extension = tag.split("-u-").next().unwrap_or(tag);
        let mut candidate = no_extension.to_string();
        loop {
            if INTL_NUMBER_DATA.iter().any(|data| data.locale.eq_ignore_ascii_case(&candidate)) {
                return INTL_NUMBER_DATA.iter().find(|data| data.locale.eq_ignore_ascii_case(&candidate)).unwrap().locale.to_string();
            }
            let Some(dash) = candidate.rfind('-') else { break };
            candidate.truncate(dash);
        }
    }
    "en".to_string()
}

fn intl_array(i: &mut Interpreter, values: impl IntoIterator<Item = JsValue>) -> Result<JsValue, Throw> {
    let mut raw = JsObject::array();
    raw.proto = Some(i.array_proto.clone());
    let array = JsValue::object(raw);
    let mut length = 0;
    for value in values {
        i.set_prop(&array, length.to_string(), value)?;
        length += 1;
    }
    i.set_prop(&array, "length", JsValue::num(length as f64))?;
    Ok(array)
}

fn intl_method(i: &Interpreter, owner: &Obj, name: &str, arity: usize, native: crate::value::NativeFn) {
    let method = Interpreter::native(name, native, &i.function_proto);
    if let JsValue::Object(function) = &method {
        function.borrow_mut().constructible = false;
        function.borrow_mut().delete_own("prototype");
        let _ = function.borrow_mut().define("length", Property {
            value: JsValue::num(arity as f64), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
    }
    let _ = owner.borrow_mut().define(name, Property {
        value: method, writable: true, enumerable: false,
        configurable: true, getter: None, setter: None,
    });
}

impl Interpreter {
    fn install_intl_builtins(&mut self) {
        let intl = self.object_value();
        let JsValue::Object(namespace) = &intl else { unreachable!() };
        intl_method(self, namespace, "getCanonicalLocales", 1, intl_get_canonical_locales);
        intl_method(self, namespace, "supportedValuesOf", 1, intl_supported_values_of);
        let locale = Self::native("Locale", intl_locale_ctor, &self.function_proto);
        if let JsValue::Object(ctor) = &locale {
            let JsValue::Object(proto) = ctor.borrow().get_own("prototype").unwrap() else { unreachable!() };
            proto.borrow_mut().proto = Some(self.object_proto.clone());
            binary_configure_constructor(ctor, "Locale", 1, &proto);
            let _ = proto.borrow_mut().define("constructor", binary_writable_property(locale.clone()));
            intl_method(self, &proto, "toString", 0, intl_locale_to_string);
            intl_method(self, &proto, "maximize", 0, intl_locale_maximize);
            intl_method(self, &proto, "minimize", 0, intl_locale_minimize);
            for name in ["baseName", "language", "script", "region", "calendar",
                "collation", "hourCycle", "numberingSystem", "numeric", "caseFirst"] {
                binary_getter(self, &proto, name, intl_locale_getter);
            }
            let _ = proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                binary_property(JsValue::str("Intl.Locale")));
            let _ = namespace.borrow_mut().define("Locale", binary_writable_property(locale));
        }
        self.install_intl_number(&namespace);
        self.install_intl_plural(&namespace);
        self.install_intl_datetime(&namespace);
        self.install_intl_relative_and_list(&namespace);
        self.install_intl_collator(&namespace);
        self.install_intl_segmenter(&namespace);
        self.install_intl_display_names(&namespace);
        let _ = namespace.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            binary_property(JsValue::str("Intl")));
        self.define_global("Intl", intl);
        self.install_intl_locale_methods();
    }
}

fn intl_get_canonical_locales(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let locales = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    intl_array(i, locales.into_iter().map(JsValue::str))
}

fn intl_supported_values_of(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let key = i.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let values: Vec<&str> = match key.as_str() {
        "calendar" => vec!["gregory"],
        "collation" => vec![],
        "currency" => INTL_SUPPORTED_CURRENCIES.to_vec(),
        "numberingSystem" => vec!["arab", "deva", "latn"],
        // Expand this list when Temporal.ZonedDateTime accepts the shared IANA
        // resolver too: Test262 constructs a ZonedDateTime for every listed ID.
        "timeZone" => vec!["UTC"],
        "unit" => vec!["bit", "byte", "celsius", "day", "degree", "foot", "gallon", "gram",
            "hour", "kilogram", "kilometer", "liter", "meter", "mile", "millimeter",
            "minute", "month", "percent", "second", "week", "year"],
        _ => return Err(Throw::range_error("unsupported Intl value key")),
    };
    intl_array(i, values.into_iter().map(JsValue::str))
}

fn intl_locale_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !matches!(this, JsValue::Object(_)) { return Err(Throw::type_error("Intl.Locale requires new")); }
    let first = args.first().unwrap_or(&JsValue::Undefined);
    let raw = match first {
        JsValue::Str(_) => i.to_string(first)?,
        JsValue::Object(object) => match intl_state(object) {
            Some(IntlState::Locale(tag)) => tag,
            _ => i.to_string(first)?,
        },
        _ => return Err(Throw::type_error("invalid Intl.Locale tag")),
    };
    let mut tag = intl_canonical_tag(&raw)?;
    if let Some(options) = args.get(1).filter(|value| !matches!(value, JsValue::Undefined)) {
        if matches!(options, JsValue::Null) { return Err(Throw::type_error("null locale options")); }
        for (property, prefix) in [("language", 0), ("script", 1), ("region", 2)] {
            let value = i.get_prop(options, property)?;
            if !matches!(value, JsValue::Undefined) {
                let value = i.to_string(&value)?;
                let mut subtags: Vec<String> = tag.split('-').map(str::to_string).collect();
                if prefix == 0 { subtags[0] = value; }
                else {
                    let position = if prefix == 1 { 1 } else if subtags.get(1).is_some_and(|part| part.len() == 4) { 2 } else { 1 };
                    if subtags.get(position).is_some_and(|part| part.len() == value.len()) { subtags[position] = value; }
                    else { subtags.insert(position, value); }
                }
                tag = intl_canonical_tag(&subtags.join("-"))?;
            }
        }
        let mut keywords = std::collections::BTreeMap::<String, String>::new();
        if let Some(extension) = tag.split("-u-").nth(1) {
            let subtags: Vec<&str> = extension.split('-').collect();
            let mut index = 0;
            while index < subtags.len() {
                if subtags[index].len() != 2 { index += 1; continue; }
                let key = subtags[index].to_string();
                index += 1;
                let begin = index;
                while index < subtags.len() && subtags[index].len() > 2 { index += 1; }
                keywords.insert(key, subtags[begin..index].join("-"));
            }
        }
        for (option, key) in [("calendar", "ca"), ("collation", "co"), ("hourCycle", "hc"),
            ("caseFirst", "kf"), ("numberingSystem", "nu")] {
            let value = i.get_prop(options, option)?;
            if value.is_undefined() { continue; }
            let text = i.to_string(&value)?.to_ascii_lowercase();
            if text.is_empty() || text.split('-').any(|part| !(3..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
                return Err(Throw::range_error(format!("invalid {option} option")));
            }
            keywords.insert(key.to_string(), text);
        }
        let numeric = i.get_prop(options, "numeric")?;
        if !numeric.is_undefined() { keywords.insert("kn".into(), if numeric.to_boolean() { "true" } else { "false" }.into()); }
        if !keywords.is_empty() {
            let base = tag.split("-u-").next().unwrap_or(&tag);
            let suffix = keywords.into_iter().map(|(key, value)| if value == "true" { key }
                else { format!("{key}-{value}") }).collect::<Vec<_>>().join("-");
            tag = intl_canonical_tag(&format!("{base}-u-{suffix}"))?;
        }
    }
    let JsValue::Object(object) = &this else { unreachable!() };
    intl_register(object, IntlState::Locale(tag));
    Ok(this)
}

fn intl_locale_tag(this: &JsValue) -> Result<String, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible Intl.Locale receiver")); };
    match intl_state(object) {
        Some(IntlState::Locale(tag)) => Ok(tag),
        _ => Err(Throw::type_error("incompatible Intl.Locale receiver")),
    }
}

fn intl_locale_to_string(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str(intl_locale_tag(&this)?))
}

fn intl_locale_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let name = binary_current_name(i);
    let base = tag.split("-u-").next().unwrap_or(&tag);
    let parts: Vec<&str> = base.split('-').collect();
    let result = match name.as_str() {
        "baseName" => Some(base.to_string()),
        "language" => parts.first().map(|value| (*value).to_string()),
        "script" => parts.iter().skip(1).find(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())).map(|s| (*s).to_string()),
        "region" => parts.iter().skip(1).find(|part| (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))).map(|s| (*s).to_string()),
        "calendar" | "collation" | "hourCycle" | "numberingSystem" | "numeric" | "caseFirst" => {
            let key = match name.as_str() {
                "calendar" => "ca", "collation" => "co", "hourCycle" => "hc",
                "numberingSystem" => "nu", "numeric" => "kn", _ => "kf",
            };
            let mut values = tag.split("-u-").nth(1).unwrap_or("").split('-').peekable();
            let mut result = None;
            while let Some(part) = values.next() {
                if part.len() != 2 { continue; }
                let mut subtags = Vec::new();
                while values.peek().is_some_and(|next| next.len() != 2) {
                    subtags.push(values.next().unwrap());
                }
                if part == key {
                    result = Some(if subtags.is_empty() { "true".to_string() } else { subtags.join("-") });
                    break;
                }
            }
            result
        }
        _ => None,
    };
    if name == "numeric" { return Ok(JsValue::Bool(result.is_some_and(|value| value != "false"))); }
    Ok(result.map(JsValue::str).unwrap_or(JsValue::Undefined))
}

fn intl_locale_maximize(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let language = tag.split('-').next().unwrap_or("en");
    let suffix = match language {
        "en" => "Latn-US", "de" => "Latn-DE", "fr" => "Latn-FR", "es" => "Latn-ES",
        "ja" => "Jpan-JP", "zh" => "Hans-CN", "ar" => "Arab-EG", "hi" => "Deva-IN",
        "ru" => "Cyrl-RU", _ => "Latn-US",
    };
    let expanded = if tag.split('-').count() == 1 { format!("{tag}-{suffix}") } else { tag };
    let constructor = i.global_env.resolve("Intl").ok_or_else(|| Throw::type_error("Intl unavailable"))?;
    let locale_ctor = i.get_prop(&constructor, "Locale")?;
    i.construct(&locale_ctor, vec![JsValue::str(expanded)])
}

fn intl_locale_minimize(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let constructor = i.global_env.resolve("Intl").ok_or_else(|| Throw::type_error("Intl unavailable"))?;
    let locale_ctor = i.get_prop(&constructor, "Locale")?;
    i.construct(&locale_ctor, vec![JsValue::str(tag.split('-').next().unwrap_or("en"))])
}

#[cfg(test)]
mod intl_tests {
    use super::*;

    #[test]
    fn canonical_locales_deduplicate_and_apply_cldr_aliases() {
        let mut i = Interpreter::new();
        let result = i.run_script("Intl.getCanonicalLocales(['EN-us', 'iw-IL', 'en-US']).join(',')")
            .unwrap_or_else(|error| panic!("locale error: {:?}", match error.0 {
                JsValue::Object(object) => object.borrow().get_own("message"),
                value => Some(value),
            }));
        assert_eq!(i.to_string(&result).unwrap(), "en-US,he-IL");
    }

    #[test]
    fn locale_exposes_canonical_base_and_unicode_extension() {
        let mut i = Interpreter::new();
        let result = i.run_script("let l = new Intl.Locale('DE-de-u-nu-latn-ca-gregory'); [l.toString(),l.baseName,l.language,l.region,l.calendar,l.numberingSystem].join('|')").unwrap();
        assert_eq!(i.to_string(&result).unwrap(), "de-DE-u-ca-gregory-nu-latn|de-DE|de|DE|gregory|latn");
    }

    #[test]
    fn invalid_locale_is_range_error() {
        let mut i = Interpreter::new();
        let result = i.run_script("try { Intl.getCanonicalLocales('en_US') } catch (e) { e.constructor.name }").unwrap();
        assert_eq!(i.to_string(&result).unwrap(), "RangeError");
    }

    #[test]
    fn locale_options_update_unicode_extension() {
        let mut i = Interpreter::new();
        let result = i.run_script("new Intl.Locale('en-US-u-nu-latn', {calendar:'gregory',numeric:true}).toString()").unwrap();
        assert_eq!(i.to_string(&result).unwrap(), "en-US-u-ca-gregory-kn-nu-latn");
    }
}
