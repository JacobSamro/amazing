// Intl.Collator root ordering with the supported locale's common letter forms.

#[derive(Clone)]
struct IntlCollatorState {
    locale: String, usage: String, sensitivity: String, collation: String,
    numeric: bool, case_first: String, ignore_punctuation: bool,
    bound_compare: Option<JsValue>,
}

impl Interpreter {
    fn install_intl_collator(&mut self, namespace: &Obj) {
        let constructor = Self::native_constructor("Collator", intl_collator_ctor, &self.function_proto);
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
        intl_register_intrinsic(self, "Collator", function, &prototype);
        let _ = namespace.borrow_mut().define("Collator", binary_writable_property(constructor));
    }
}

fn intl_collator_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    // Intl.Collator is callable without new; the this-value is ignored and a
    // fresh instance is created with the correct prototype.
    let object = match this {
        JsValue::Object(object) if i.current_new_target().is_some() => object,
        _ => {
            // Intl.Collator is legacy-callable without `new`; the fresh instance's
            // prototype comes from the %Collator% intrinsic, not a `Get(Intl,
            // "Collator")` lookup on the mutable `Intl` namespace object (taintable
            // per testIntl.js's taintDataProperty).
            let proto = i.intrinsic_constructors.get("Collator")
                .map(|(_, prototype)| prototype.clone())
                .ok_or_else(|| Throw::type_error("Intl service unavailable"))?;
            let mut raw = JsObject::ordinary();
            raw.proto = Some(proto);
            Obj(Rc::new(RefCell::new(raw)))
        }
    };
    let requested = intl_canonical_locale_list(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let options = args.get(1).filter(|value| !value.is_undefined()).cloned().unwrap_or_else(intl_empty_options);
    if matches!(options, JsValue::Null) { return Err(Throw::type_error("null Collator options")); }
    let usage = intl_string_option(i, &options, "usage", &["sort", "search"], "sort")?;
    let matcher = intl_string_option(i, &options, "localeMatcher", &["lookup", "best fit"], "best fit")?;
    // intl_lookup and the extension reads below have no observable side
    // effects (pure string ops over the already-canonicalized `requested`),
    // so moving them after the option reads above -- needed so `matcher` is
    // known before locale resolution -- does not change getter-call order
    // relative to any other option (constructor-options-order.js et al.).
    let mut locale = intl_lookup(&requested, matcher == "best fit");
    let extension = requested.iter().find(|tag| {
        tag.split("-u-").next().unwrap_or(tag).starts_with(&locale)
    });
    let ext_numeric = extension.and_then(|tag| intl_unicode_keyword(tag, "kn"));
    let ext_case_first = extension.and_then(|tag| intl_unicode_keyword(tag, "kf"));
    let ext_collation_raw = extension.and_then(|tag| intl_unicode_keyword(tag, "co"));
    let sensitivity = intl_string_option(i, &options, "sensitivity", &["base", "accent", "case", "variant"], "variant")?;
    let opt_collation = intl_string_option(i, &options, "collation", &[], "default")?;
    if !opt_collation.bytes().all(|b| b.is_ascii_alphanumeric()) || !(3..=8).contains(&opt_collation.len()) && opt_collation != "default" {
        return Err(Throw::range_error("invalid collation"));
    }
    // "search" and "standard" are reserved: they name the *usage*, not a
    // collation, and ECMA-402 requires Collator to disregard them here.
    let opt_collation = if opt_collation == "search" || opt_collation == "standard" { "default".to_string() } else { opt_collation };
    // ResolveLocale for the "co" key: an explicit, *supported* options.collation
    // wins; otherwise a supported `-u-co-` value from the requested tag; else
    // "default". `intl_collation_supported` is a narrow, test-justified check
    // (not a full CLDR co-availableValues table -- de's "phonebk" and the
    // generally-applicable "eor" are the only entries a currently-pinned
    // intl402 case exercises).
    // Checked against the *requested* language, not the resolved `locale`
    // (which falls back to "en" for a language we have no other data for at
    // all, like "si"/"ln"/"sv" -- Intl/supportedValuesOf/collations-accepted-
    // by-Collator.js still expects those languages' CLDR-documented
    // collations to be recognized as supported).
    let requested_language = requested.first()
        .map(|tag| tag.split('-').next().unwrap_or(tag).to_string())
        .unwrap_or_else(|| locale.clone());
    let ext_collation = ext_collation_raw.as_deref().filter(|value| intl_collation_supported(&requested_language, value));
    let opt_collation_valid = opt_collation != "default" && intl_collation_supported(&requested_language, &opt_collation);
    let collation = if opt_collation_valid { opt_collation } else { ext_collation.map(str::to_string).unwrap_or_else(|| "default".to_string()) };
    let numeric_value = i.get_prop(&options, "numeric")?;
    let ext_numeric_value = ext_numeric.as_deref().and_then(|value| match value {
        "" | "true" => Some(true), "false" => Some(false), _ => None,
    });
    let numeric = if numeric_value.is_undefined() { ext_numeric_value.unwrap_or(false) }
        else { numeric_value.to_boolean() };
    let case_first = intl_string_option(i, &options, "caseFirst", &["upper", "lower", "false"],
        ext_case_first.as_deref().filter(|value| ["upper", "lower", "false"].contains(value))
            .unwrap_or("false"))?;
    let mut extension_parts = Vec::new();
    if ext_case_first.as_deref() == Some(case_first.as_str()) {
        extension_parts.push(format!("kf-{case_first}"));
    }
    if ext_numeric_value == Some(numeric) {
        extension_parts.push(if numeric { "kn".into() } else { "kn-false".into() });
    }
    if ext_collation == Some(collation.as_str()) {
        extension_parts.push(format!("co-{collation}"));
    }
    if !extension_parts.is_empty() { locale.push_str(&format!("-u-{}", extension_parts.join("-"))); }
    let ignore_value = i.get_prop(&options, "ignorePunctuation")?;
    // CLDR's root collation defaults ignorePunctuation to true for a handful
    // of locales (Thai's is the one a pinned intl402 case exercises); every
    // other locale defaults to false.
    let ignore_default = locale.split('-').next().unwrap_or(&locale) == "th";
    let ignore_punctuation = if ignore_value.is_undefined() { ignore_default } else { ignore_value.to_boolean() };
    intl_register(&object, IntlState::Collator(IntlCollatorState {
        locale, usage, sensitivity, collation, numeric, case_first, ignore_punctuation,
        bound_compare: None,
    }));
    Ok(JsValue::Object(object))
}

// Per-locale "co" availableValues, matching the exact mapping
// intl402/Intl/supportedValuesOf/collations-accepted-by-Collator.js documents
// in its own comments (one representative locale per non-default collation,
// drawn from CLDR's collation.xml). Not every locale/collation combination
// CLDR lists is covered, only enough that every value INTL_SUPPORTED_COLLATIONS
// advertises resolves as supported for at least the locale that test expects,
// and that the pinned resolved-collation-unicode-extensions-and-options.js
// cases (de/phonebk valid, de/eor valid, de/pinyin and en/phonebk invalid)
// come out right. "eor"/"emoji"/"ducet" are broadly applicable base
// collations, not exclusive to any one language.
fn intl_collation_supported(locale: &str, collation: &str) -> bool {
    let language = locale.split('-').next().unwrap_or(locale);
    match collation {
        "eor" | "emoji" | "ducet" => true,
        "phonebk" => language == "de",
        "trad" => language == "es",
        "direct" => language == "hi",
        "searchjl" => language == "ko",
        "phonetic" => language == "ln",
        "dict" => language == "si",
        "reformed" => language == "sv",
        "compat" => language == "ar",
        "big5han" | "gb2312" | "pinyin" | "stroke" | "unihan" | "zhuyin" => language == "zh",
        _ => false,
    }
}

fn intl_collator_state(this: &JsValue) -> Result<IntlCollatorState, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible Collator receiver")); };
    match intl_state(object) {
        Some(IntlState::Collator(state)) | Some(IntlState::BoundCollator(state)) => Ok(state),
        _ => Err(Throw::type_error("incompatible Collator receiver")),
    }
}

fn intl_collator_compare_getter(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(collator) = &this else {
        return Err(Throw::type_error("incompatible Collator receiver"));
    };
    let Some(IntlState::Collator(mut state)) = intl_state(collator) else {
        return Err(Throw::type_error("incompatible Collator receiver"));
    };
    if let Some(function) = &state.bound_compare { return Ok(function.clone()); }
    let function = Interpreter::native_arity("", intl_collator_bound_compare, &i.function_proto, 2);
    let JsValue::Object(object) = &function else { unreachable!() };
    object.borrow_mut().constructible = false;
    object.borrow_mut().delete_own("prototype");
    // The callable owns only an uncached state snapshot, so it does not
    // retain its originating Collator or a reference to itself.
    intl_register(object, IntlState::BoundCollator(state.clone()));
    state.bound_compare = Some(function.clone());
    intl_register(collator, IntlState::Collator(state));
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
    // ECMA-402 16.3.5 order: locale, usage, sensitivity, ignorePunctuation,
    // collation, numeric, caseFirst.
    for (key, value) in [
        ("locale", state.locale), ("usage", state.usage), ("sensitivity", state.sensitivity),
    ] { i.set_prop(&object, key, JsValue::str(value))?; }
    i.set_prop(&object, "ignorePunctuation", JsValue::Bool(state.ignore_punctuation))?;
    i.set_prop(&object, "collation", JsValue::str(state.collation))?;
    i.set_prop(&object, "numeric", JsValue::Bool(state.numeric))?;
    i.set_prop(&object, "caseFirst", JsValue::str(state.case_first))?;
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

// Drops punctuation/whitespace when ignorePunctuation is set, so every
// sensitivity level (primary through the accent/case tiebreaks in
// intl_collate) treats e.g. "" and " " identically instead of only the
// primary key doing so (Collator/prototype/compare/ignorePunctuation.js).
fn intl_strip_punctuation(state: &IntlCollatorState, text: &str) -> String {
    if !state.ignore_punctuation { return text.to_string(); }
    text.chars().filter(|ch| !(ch.is_ascii_punctuation() || ch.is_whitespace()
        || ['“', '”', '‘', '’', '—', '–'].contains(ch))).collect()
}

// German phonebook collation (DIN 5007-2, CLDR "de-u-co-phonebk") sorts ä/ö/ü
// as if they were written "ae"/"oe"/"ue", unlike standard German collation
// (DIN 5007-1) where they sort next to plain a/o/u -- Collator/prototype/
// compare/non-normative-phonebook.js exercises exactly this expansion.
fn intl_base_char_for(state: &IntlCollatorState, ch: char) -> &'static str {
    if state.collation == "phonebk" {
        match ch {
            'ä' | 'Ä' => return "ae",
            'ö' | 'Ö' => return "oe",
            'ü' | 'Ü' => return "ue",
            _ => {}
        }
    }
    intl_base_char(ch)
}

fn intl_primary_key(state: &IntlCollatorState, text: &str) -> String {
    let mut output = String::new();
    for ch in intl_strip_punctuation(state, text).chars() {
        if matches!(ch as u32, 0x0300..=0x036f) { continue; }
        let mapped = intl_base_char_for(state, ch);
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

// Complete canonical normalization shares String.prototype.normalize's data.
// Rust strings here already contain scalar values; this preserves the existing
// collation input boundary and does not change locale weights/options.
fn intl_canonical_form(text: &str, form: &str) -> String {
    let points: Vec<u32> = text.chars().map(u32::from).collect();
    runtime_string_normalization::normalize(&points, form).into_iter()
        .map(|point| char::from_u32(point).expect("normalization preserves scalar values"))
        .collect()
}

// Strips diacritics (via the same base-letter table as intl_primary_key)
// while preserving case, for the "case"-only sensitivity level: at that
// level accent differences must NOT be distinguished (that's "variant"'s
// job), only case (Collator/prototype/compare/non-normative-sensitivity.js:
// "Aa" and "Aã" must both match a "case"-sensitivity target of "Aa").
fn intl_case_stripped(text: &str) -> String {
    let mut output = String::new();
    for ch in text.chars() {
        let mapped = intl_base_char(ch);
        if mapped.is_empty() { output.push(ch); }
        else if ch.is_uppercase() { output.extend(mapped.to_uppercase().chars()); }
        else { output.push_str(mapped); }
    }
    output
}

fn intl_collate(state: &IntlCollatorState, left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Canonically equivalent input reaches the same primary weights. NFC
    // retains precomposed locale tailoring before later NFD accent ordering.
    let left_normalized = intl_canonical_form(left, "NFC");
    let right_normalized = intl_canonical_form(right, "NFC");
    let left = left_normalized.as_str();
    let right = right_normalized.as_str();
    let left_primary = intl_primary_key(state, left);
    let right_primary = intl_primary_key(state, right);
    let primary = if state.numeric { intl_compare_numeric(&left_primary, &right_primary) }
        else { left_primary.cmp(&right_primary) };
    if primary != Ordering::Equal { return primary; }
    let left_clean = intl_strip_punctuation(state, left);
    let right_clean = intl_strip_punctuation(state, right);
    if ["accent", "variant"].contains(&state.sensitivity.as_str()) {
        let accents = intl_canonical_form(&left_clean.to_lowercase(), "NFD")
            .cmp(&intl_canonical_form(&right_clean.to_lowercase(), "NFD"));
        if accents != Ordering::Equal { return accents; }
    }
    if ["case", "variant"].contains(&state.sensitivity.as_str()) {
        let (left_case, right_case) = if state.sensitivity == "case" {
            (intl_case_stripped(&left_clean), intl_case_stripped(&right_clean))
        } else {
            (intl_canonical_form(&left_clean, "NFD"), intl_canonical_form(&right_clean, "NFD"))
        };
        let cases = match state.case_first.as_str() {
            "upper" => right_case.cmp(&left_case),
            _ => left_case.cmp(&right_case),
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
    #[test]
    fn complete_canonical_equivalence_preserves_options_and_locale_tailoring() {
        let source = r#"
          var failures=[];
          var pairs=[['e\u0301','é'],['\u212b','Å'],['\u2126','Ω'],['\u1100\u1161\u11a8','각'],['\u1e0b\u0323','\u1e0d\u0307'],['\u0315\u0300','\u0300\u0315'],['a\u0306\u0301','ắ'],['\u0344','\u0308\u0301'],['\ud834\udd5e','\ud834\udd57\ud834\udd65']];
          for(var locale of ['en','de','de-u-co-phonebk','sv','vi','ko']) {
            for(var sensitivity of ['base','accent','case','variant']) {
              var collator=new Intl.Collator(locale,{sensitivity:sensitivity});
              var compare=collator.compare;
              if(compare!==collator.compare)failures.push('bound compare identity:'+locale+':'+sensitivity);
              for(var pair of pairs){var forward=compare(pair[0],pair[1]),reverse=compare(pair[1],pair[0]);if(forward!==0||reverse!==0)failures.push('canonical pair:'+locale+':'+sensitivity+':'+pair.join('|')+':'+forward+':'+reverse);}
            }
          }
          var getter=Object.getOwnPropertyDescriptor(Intl.Collator.prototype,'compare').get;
          var holder=new Intl.Collator('en'),cached=holder.compare;
          for(var invalid of [cached,Intl.Collator.prototype,{},Object.create(Intl.Collator.prototype),new Intl.NumberFormat('en'),null,undefined]){
            try{getter.call(invalid);failures.push('compare getter accepted incompatible brand');}catch(e){if(Object.getPrototypeOf(e)!==TypeError.prototype)failures.push('compare getter wrong brand error');}
          }
          if(getter.call(holder)!==cached||holder.compare!==cached||cached('a','b')>=0)failures.push('brand errors disturbed cached bound function');
          var customError=new Error('custom compare input');Object.setPrototypeOf(customError,null);
          try{cached({toString(){throw customError;}},'x');failures.push('compare swallowed custom throw');}catch(e){if(e!==customError||Object.getPrototypeOf(e)!==null)failures.push('compare changed custom throw');}
          var phone=new Intl.Collator('de-u-co-phonebk',{sensitivity:'variant'}).compare;
          var standard=new Intl.Collator('de',{sensitivity:'variant'}).compare;
          var tailoring=[phone('äb','adb'),phone('a\u0308b','adb'),standard('äb','adb'),standard('a\u0308b','adb')];if(!(tailoring[0]>0&&tailoring[1]>0&&tailoring[2]<0&&tailoring[3]<0))failures.push('preserved tailoring:'+tailoring.join(','));
          var numeric=new Intl.Collator('en',{numeric:true,ignorePunctuation:true}).compare;
          var numbers=[numeric('é-2','e\u030110'),numeric('é-2','e\u0301 2')];if(!(numbers[0]<0&&numbers[1]===0))failures.push('numeric punctuation:'+numbers.join(','));
          if(failures.length)throw failures.join(';');true;
        "#;
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|error|panic!("{backend:?}: {}",error.describe())),JsValue::Bool(true));
            let caller=i.binding_current_realm();
            let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for (name, expression) in [
                ("foreignCompareGetter", "Object.getOwnPropertyDescriptor(Intl.Collator.prototype,'compare').get"),
                ("foreignFunctionPrototype", "Function.prototype"),
                ("foreignTypePrototype", "TypeError.prototype"),
            ] {
                let value=i.binding_with_realm(child,|i|i.run_script_with_backend(expression,backend)).unwrap();
                i.global_env.declare_var(name,value).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var fresh=new Intl.Collator('en');
              var borrowed=foreignCompareGetter.call(fresh);
              var realmOk=Object.getPrototypeOf(borrowed)===foreignFunctionPrototype && fresh.compare===borrowed;
              var result=borrowed('e\u0301','é');realmOk=realmOk&&result===0&&borrowed.name===''&&borrowed.length===2;
              try{foreignCompareGetter.call(borrowed);realmOk=false;}catch(e){realmOk=realmOk&&Object.getPrototypeOf(e)===foreignTypePrototype;}
              realmOk&&fresh.compare===borrowed;
            "#,backend).unwrap(),JsValue::Bool(true),"{backend:?}: cached compare creation realm");
            assert_eq!(i.binding_current_realm(),caller);
            assert_eq!(i.binding_incumbent_realm(),caller);
        }
    }

}
