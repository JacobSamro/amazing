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
    Duration(IntlDurationState),
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

thread_local! {
    static INTL_FALLBACK_SYMBOL: RefCell<Option<JsSymbol>> = RefCell::new(None);
}

// %Intl%.[[FallbackSymbol]]: a Symbol with description "IntlLegacyConstructedSymbol",
// created once per realm and shared by ChainDateTimeFormat/ChainNumberFormat below.
// One process runs one JS realm in this host, so a thread-local stands in for "per
// realm"; a genuine $262.createRealm() still won't hand back a second distinct one.
fn intl_fallback_symbol() -> JsSymbol {
    INTL_FALLBACK_SYMBOL.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            slot.replace(JsSymbol::new(Some(crate::value::JsString::from("IntlLegacyConstructedSymbol"))));
        }
        slot.clone().unwrap()
    })
}

// Registers a service constructor (Collator, NumberFormat, DateTimeFormat, ...)
// as an %X%-style intrinsic, the same table `capture_intrinsic_constructors`
// (interp_realms.rs) builds for ordinary globals like Array/RegExp. Each Intl
// service constructor lives under the `Intl` namespace object rather than as a
// direct global property, so that generic top-level scan never finds it; every
// `install_intl_*` function below calls this once so two things that consult
// this same table start working for Intl too: (1) `constructor_fallback_prototype`,
// which GetPrototypeFromConstructor uses when `Reflect.construct(Intl.X, [],
// newTarget)` is given a newTarget whose own `.prototype` isn't an object
// (falls back to newTarget's realm's %XPrototype%, not %ObjectPrototype%), and
// (2) `intl_construct_service` below, which uses it to reach the intrinsic
// constructor directly instead of a mutable `Intl.X` property lookup.
fn intl_register_intrinsic(i: &mut Interpreter, name: &str, function: &Obj, prototype: &Obj) {
    i.intrinsic_constructors.insert(name.to_string(), (function.clone(), prototype.clone()));
}

// ChainDateTimeFormat / ChainNumberFormat (ECMA-402 sec-Intl.DateTimeFormat /
// sec-Intl.NumberFormat "Abstract Operations"): build the internal state-bearing
// object and either hand it back directly (the `new` case, and the plain
// function-call case where `this` isn't already an instance) or, when called as
// a function on a receiver that already is one (the pre-Reflect.construct
// subclassing idiom, e.g. `Intl.DateTimeFormat.call(this, ...)`), tag `this`
// with %Intl%.[[FallbackSymbol]] and return `this` unchanged. `ctor_key` names
// the constructor under `Intl` ("DateTimeFormat" or "NumberFormat").
fn intl_chain_construct(i: &mut Interpreter, this: JsValue, ctor_key: &str, state: IntlState) -> Result<JsValue, Throw> {
    if i.current_new_target().is_some() {
        let JsValue::Object(object) = this else {
            unreachable!("`this` under `new` is always the freshly allocated instance")
        };
        intl_register(&object, state);
        return Ok(JsValue::Object(object));
    }
    // ECMA-402's Chain* abstract operations name the %DateTimeFormat%/%NumberFormat%
    // intrinsic directly here, not a `Get(Intl, ctor_key)` lookup on the (mutable,
    // taint-able) `Intl` namespace object.
    let (constructor, proto) = i.intrinsic_constructors.get(ctor_key)
        .map(|(constructor, prototype)| (constructor.clone(), prototype.clone()))
        .ok_or_else(|| Throw::type_error("Intl service unavailable"))?;
    let constructor = JsValue::Object(constructor);
    let mut raw = JsObject::ordinary();
    raw.proto = Some(proto);
    let internal = Obj(Rc::new(RefCell::new(raw)));
    intl_register(&internal, state);
    if i.ordinary_has_instance(&this, &constructor)?.to_boolean() {
        if let JsValue::Object(receiver) = &this {
            let _ = receiver.borrow_mut().define(intl_fallback_symbol(),
                binary_locked_property(JsValue::Object(internal)));
            return Ok(this);
        }
    }
    Ok(JsValue::Object(internal))
}

// UnwrapDateTimeFormat / UnwrapNumberFormat: if `object` itself carries no
// registered state, and it is (per OrdinaryHasInstance, not the full
// `instanceof` operator with its @@hasInstance lookup) an instance of the
// constructor named by `ctor_key`, follow %Intl%.[[FallbackSymbol]] to the
// internal object ChainDateTimeFormat/ChainNumberFormat stashed there. The
// property read goes through `Interpreter::get_prop` so a Proxy `this` still
// sees its `get` trap fire, matching the on-unwrap conformance tests.
fn intl_unwrap_state(i: &mut Interpreter, object: &Obj, ctor_key: &str) -> Result<Option<IntlState>, Throw> {
    if let Some(state) = intl_state(object) { return Ok(Some(state)); }
    let intl = i.global_env.resolve("Intl").unwrap();
    let constructor = i.get_prop(&intl, ctor_key)?;
    let this = JsValue::Object(object.clone());
    if !i.ordinary_has_instance(&this, &constructor)?.to_boolean() { return Ok(None); }
    match i.get_prop(&this, intl_fallback_symbol())? {
        JsValue::Object(inner) => Ok(intl_state(&inner)),
        _ => Ok(None),
    }
}

// Calendars for which Intl.DateTimeFormat provides (Gregorian-backed)
// functionality. ECMA-402 AvailableCalendars; the list is sorted and unique.
const INTL_KNOWN_CALENDARS: &[&str] = &[
    "buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory",
    "hebrew", "indian", "islamic-civil", "islamic-tbla", "islamic-umalqura",
    "iso8601", "japanese", "persian", "roc",
];

fn intl_data(locale: &str) -> &'static IntlNumberData {
    INTL_NUMBER_DATA.iter().find(|data| data.locale == locale).unwrap_or(&INTL_NUMBER_DATA[0])
}

fn intl_empty_options() -> JsValue {
    // ECMA-402 uses ObjectCreate(null) when the options argument is absent.
    JsValue::object(JsObject::ordinary())
}

// UTS35 "type" grammar, shared by the numberingSystem/calendar/collation option
// values across constructors: one or more hyphen-separated alphanumeric
// subtags, each 3-8 characters. Well-formed but unrecognized values are
// accepted here and simply ignored later (kept at the locale/style default).
fn intl_is_well_formed_type(value: &str) -> bool {
    !value.is_empty() && value.split('-').all(|part|
        (3..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

fn intl_unicode_keyword(tag: &str, wanted: &str) -> Option<String> {
    // A private-use singleton ("-x-...") always comes last and everything
    // after it is opaque private-use subtags, never a real "-u-" extension --
    // "de-x-u-co-phonebk" has no Unicode extension at all, just a private-use
    // sequence that happens to contain the substring "u-co-phonebk". Truncate
    // there first so a naive substring search doesn't reach into it
    // (Collator/unicode-ext-seq-in-private-tag.js).
    let before_private_use = tag.split("-x-").next().unwrap_or(tag);
    let extension = before_private_use.split("-u-").nth(1)?;
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

// General multi-candidate territoryAlias disambiguation (UTS 35 §3.2.1): when
// a region's replacement lists more than one candidate, the likely region for
// the tag's own (already-canonicalized) language and script picks among
// them, falling back to the first candidate. `script` is the tag's script
// subtag if one is present (in Title case, as stored in `canonical`).
fn intl_region_replacement(region: &str, language: &str, script: Option<&str>) -> String {
    let Some(candidates) = intl_alias(INTL_REGION_ALIASES, region) else { return region.to_string() };
    let mut iter = candidates.split(' ');
    let first = iter.next().unwrap_or(region);
    if iter.next().is_none() { return first.to_string(); }
    let prefix = match script {
        Some(script) => format!("{language}-{script}"),
        None => language.to_string(),
    };
    if let Some(expanded) = intl_likely_tag(&prefix) {
        if let Some(likely_region) = expanded.rsplit('-').next() {
            if candidates.split(' ').any(|candidate| candidate == likely_region) {
                return likely_region.to_string();
            }
        }
    }
    first.to_string()
}

// Resolve a Unicode extension ("u") or transformed extension ("t") tvalue/type
// against the BCP-47 keyword registry (calendar/collation/measure/timezone/
// number/segmentation/variant/currency for "u"; the transform-* files for
// "t"), then, only for "u", drop a value that canonicalizes to the literal
// "true" (UTS 35 §3.2.1: "any type or tfield value 'true' is removed" --
// transformed-ext-canonical.js notes this does NOT apply to "t" tvalues,
// since a tkey must always be followed by a tvalue).
fn intl_bcp47_type_alias(key: &str, value: &str) -> Option<&'static str> {
    INTL_BCP47_KEY_TYPE_ALIASES.iter()
        .find(|(k, alias, _)| *k == key && alias.eq_ignore_ascii_case(value))
        .map(|(_, _, canonical)| *canonical)
}

// Shared by the primary unicode_language_id and by a "t" extension's tlang:
// language ["-" script] ["-" region] *("-" variant), with alias replacement
// (including the CLDR compound "language-variant"/"language-region" keys
// used for e.g. "zh-guoyu"->"zh", "aa-saaho"->"ssy" and "sgn-GR"->"gss", and
// the generic "und-"-prefixed variant deprecations) and conditional merging
// of any script/region an aliased *plain* language subtag's replacement adds
// (only when the tag doesn't already supply that component itself). Returns
// canonicalized (language, script, region, sorted variants); the caller
// lowercases everything for a tlang or applies primary-tag casing itself.
fn intl_canonicalize_language_id(parts: &[String]) -> Result<(String, Option<String>, Option<String>, Vec<String>), Throw> {
    let mut language = parts[0].to_ascii_lowercase();
    let mut pending_script: Option<String> = None;
    let mut pending_region: Option<String> = None;
    if let Some(replacement) = intl_alias(INTL_LANGUAGE_ALIASES, &language) {
        let mut pieces = replacement.split('-');
        language = pieces.next().unwrap_or(replacement).to_string();
        for piece in pieces {
            if piece.len() == 4 && piece.bytes().all(|b| b.is_ascii_alphabetic()) {
                pending_script = Some(piece.to_string());
            } else {
                pending_region = Some(piece.to_string());
            }
        }
    }
    let mut i = 1;
    let mut script = None;
    if i < parts.len() && parts[i].len() == 4 && parts[i].bytes().all(|b| b.is_ascii_alphabetic()) {
        script = Some(intl_title(&parts[i]));
        i += 1;
    } else if let Some(replacement) = pending_script.take() {
        script = Some(intl_title(&replacement));
    }
    if let Some(script_value) = &script {
        script = Some(intl_alias(INTL_SCRIPT_ALIASES, script_value).map(str::to_string).unwrap_or_else(|| script_value.clone()));
    }
    let mut region = None;
    if i < parts.len() && ((parts[i].len() == 2 && parts[i].bytes().all(|b| b.is_ascii_alphabetic()))
        || (parts[i].len() == 3 && parts[i].bytes().all(|b| b.is_ascii_digit()))) {
        region = Some(parts[i].to_ascii_uppercase());
        i += 1;
    } else if let Some(replacement) = pending_region.take() {
        region = Some(replacement.to_ascii_uppercase());
    }
    if let Some(region_value) = &region {
        // A compound "language-region" alias (the CLDR sign-language codes,
        // e.g. "sgn-GR" -> "gss") replaces the whole language id; it takes
        // priority over the generic multi-candidate territoryAlias table.
        let compound = format!("{language}-{region_value}");
        if let Some(replacement) = intl_alias(INTL_LANGUAGE_ALIASES, &compound) {
            if !replacement.contains('-') {
                language = replacement.to_string();
                region = None;
            }
        }
    }
    if let Some(region_value) = &region {
        region = Some(intl_region_replacement(region_value, &language, script.as_deref()));
    }
    // Variant subtags are collected, sorted, and only then alias-resolved
    // (UTS 35's "bring the tag into canonical syntax first" ordering): a
    // language-specific compound key ("zh-guoyu", "art-lojban", "aa-saaho")
    // is tried before the generic "und-"-prefixed deprecation, longest
    // contiguous window first, so a multi-subtag replacement like
    // "und-hepburn-heploc"->"und-alalc97" still matches.
    let mut raw_variants = Vec::new();
    while i < parts.len() && ((5..=8).contains(&parts[i].len())
        || (parts[i].len() == 4 && parts[i].as_bytes()[0].is_ascii_digit())) {
        let variant = parts[i].to_ascii_lowercase();
        if raw_variants.contains(&variant) { return Err(Throw::range_error("duplicate language variant")); }
        raw_variants.push(variant);
        i += 1;
    }
    raw_variants.sort();
    let mut variants = Vec::new();
    let mut pos = 0;
    while pos < raw_variants.len() {
        let mut matched = false;
        for len in (1..=raw_variants.len() - pos).rev() {
            let window = raw_variants[pos..pos + len].join("-");
            let specific = format!("{language}-{window}");
            let generic = format!("und-{window}");
            if let Some(replacement) = intl_alias(INTL_LANGUAGE_ALIASES, &specific)
                .or_else(|| intl_alias(INTL_LANGUAGE_ALIASES, &generic)) {
                if replacement == "und" {
                    // Generic deprecation: drop the variant, tag unchanged.
                } else if let Some(suffix) = replacement.strip_prefix("und-") {
                    variants.extend(suffix.split('-').map(str::to_string));
                } else {
                    language = replacement.to_string();
                }
                pos += len;
                matched = true;
                break;
            }
            // CLDR's plain variantAlias table (e.g. "heploc" -> "alalc97" on
            // its own, unpaired with "hepburn") only ever replaces a single
            // variant subtag with another, so it's only tried at len == 1,
            // after the more specific language-conditioned compounds above.
            if len == 1 {
                if let Some(replacement) = intl_alias(INTL_VARIANT_ALIASES, &window) {
                    variants.push(replacement.to_string());
                    pos += 1;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            variants.push(raw_variants[pos].clone());
            pos += 1;
        }
    }
    variants.sort();
    variants.dedup();
    Ok((language, script, region, variants))
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
    // How many of `parts` the primary unicode_language_id consumes: language,
    // optional script, optional region, then a run of variants. Recomputed
    // structurally (independent of aliasing) so extension parsing resumes at
    // the right index regardless of what intl_canonicalize_language_id did.
    let mut consumed = 1;
    if parts.get(consumed).is_some_and(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())) {
        consumed += 1;
    }
    if parts.get(consumed).is_some_and(|part| (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
        || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))) {
        consumed += 1;
    }
    while parts.get(consumed).is_some_and(|part| (5..=8).contains(&part.len())
        || (part.len() == 4 && part.as_bytes()[0].is_ascii_digit())) {
        consumed += 1;
    }
    let language_id_parts: Vec<String> = parts[..consumed].iter().map(|part| part.to_string()).collect();
    let (language, script, region, variants) = intl_canonicalize_language_id(&language_id_parts)?;
    let mut canonical = vec![language];
    if let Some(script) = script { canonical.push(script); }
    if let Some(region) = region { canonical.push(region); }
    canonical.extend(variants);
    let mut i = consumed;
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
        if singleton == "t" {
            subtags = intl_canonicalize_transformed_extension(&subtags)?;
        }
        if singleton == "u" {
            subtags = intl_canonicalize_unicode_extension(&subtags)?;
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

// tlang = unicode_language_subtag ["-" unicode_script_subtag]
//         ["-" unicode_region_subtag] *("-" unicode_variant_subtag)
// tfield = tkey 1*("-" tvalue) ; tkey = alpha digit ; tvalue = 3*8alphanum
// UTS 35 3.7.1 / RFC 6497. Unlike the primary language id, an invalid or
// incomplete structure here is a hard error (transformed-ext-invalid.js),
// tlang keeps lowercase casing throughout (transformed-ext-canonical.js:
// "en-t-en-latn-ca" is already canonical), and "true" tvalues are kept.
fn intl_canonicalize_transformed_extension(subtags: &[String]) -> Result<Vec<String>, Throw> {
    let mut idx = 0;
    if subtags.first().is_some_and(|part| ((2..=3).contains(&part.len()) || (5..=8).contains(&part.len()))
        && part.bytes().all(|b| b.is_ascii_alphabetic())) {
        idx = 1;
        if subtags.get(idx).is_some_and(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())) {
            idx += 1;
        }
        if subtags.get(idx).is_some_and(|part| (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
            || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))) {
            idx += 1;
        }
        let mut seen = std::collections::HashSet::new();
        while idx < subtags.len() {
            let part = &subtags[idx];
            let is_variant = (5..=8).contains(&part.len()) || (part.len() == 4 && part.as_bytes()[0].is_ascii_digit());
            if !is_variant { break; }
            if !seen.insert(part.clone()) { return Err(Throw::range_error("duplicate transformed-language variant")); }
            idx += 1;
        }
        if idx < subtags.len() && subtags[idx].len() != 2 {
            // Anything left over that isn't tkey-shaped means tlang itself
            // didn't fully match the grammar (e.g. an over-length variant,
            // an extlang-shaped subtag, or a malformed region).
            return Err(Throw::range_error("invalid transformed language"));
        }
    }
    // Otherwise the first subtag isn't tlang-shaped (e.g. it's already a
    // "d0"-style tkey) -- there's no tlang, and idx stays 0 so the tfield
    // loop below parses it, erroring itself if it isn't a valid tkey either.
    let language_id_parts: Vec<String> = subtags[..idx].to_vec();
    let mut tlang = Vec::new();
    if !language_id_parts.is_empty() {
        let (language, script, region, variants) = intl_canonicalize_language_id(&language_id_parts)?;
        tlang.push(language.to_ascii_lowercase());
        if let Some(script) = script { tlang.push(script.to_ascii_lowercase()); }
        if let Some(region) = region { tlang.push(region.to_ascii_lowercase()); }
        tlang.extend(variants);
    }
    let mut fields: Vec<(String, Vec<String>)> = Vec::new();
    let mut j = idx;
    while j < subtags.len() {
        let key = subtags[j].clone();
        if key.len() != 2 || !key.as_bytes()[0].is_ascii_alphabetic() || !key.as_bytes()[1].is_ascii_digit() {
            return Err(Throw::range_error("invalid transform extension key"));
        }
        j += 1;
        let value_start = j;
        while j < subtags.len() && subtags[j].len() >= 3 { j += 1; }
        if j == value_start { return Err(Throw::range_error("transform key without value")); }
        let values = subtags[value_start..j].iter()
            .map(|value| intl_bcp47_type_alias(&key, value).map(str::to_string).unwrap_or_else(|| value.clone()))
            .collect();
        fields.push((key, values));
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    let mut result = tlang;
    for (key, values) in fields { result.push(key); result.extend(values); }
    Ok(result)
}

// Unicode extension ("u") subtags: attributes (3-8 alphanum, sorted), then
// key-value pairs (key = alphanum + alpha; value = one or more 3-8 alphanum
// subtags), sorted by key. Each value is resolved against the BCP-47 keyword
// registry and a value that canonicalizes to "true" is dropped; "sd"/"rg"
// values additionally go through the CLDR subdivisionAlias table.
fn intl_canonicalize_unicode_extension(subtags: &[String]) -> Result<Vec<String>, Throw> {
    let mut attributes = Vec::new();
    let mut j = 0;
    while j < subtags.len() && subtags[j].len() > 2 {
        attributes.push(subtags[j].clone());
        j += 1;
    }
    attributes.sort();
    let mut keywords: Vec<(String, Vec<String>)> = Vec::new();
    while j < subtags.len() {
        if subtags[j].len() != 2 || !subtags[j].as_bytes()[1].is_ascii_alphabetic() {
            return Err(Throw::range_error("invalid Unicode locale extension key"));
        }
        let key = subtags[j].clone();
        j += 1;
        let value_start = j;
        while j < subtags.len() && subtags[j].len() > 2 { j += 1; }
        if keywords.iter().any(|(old, _)| old == &key) { continue; }
        let mut values = subtags[value_start..j].to_vec();
        if key == "sd" || key == "rg" {
            if let [value] = &mut values[..] {
                if let Some(replacement) = intl_alias(INTL_SUBDIVISION_ALIASES, value) {
                    *value = replacement.to_string();
                }
            }
        }
        // A registry alias can span the whole (possibly multi-subtag) value,
        // e.g. "ca" "ethiopic-amete-alem" -> "ethioaa" or "ca" "islamicc" ->
        // "islamic-civil" (itself two subtags); try that before resolving
        // subtag-by-subtag (e.g. "co" "phonebook" -> "phonebk" alone).
        let joined = values.join("-");
        if let Some(replacement) = intl_bcp47_type_alias(&key, &joined) {
            values = replacement.split('-').map(str::to_string).collect();
        } else {
            for value in values.iter_mut() {
                if let Some(replacement) = intl_bcp47_type_alias(&key, value) {
                    *value = replacement.to_string();
                }
            }
        }
        if values.len() == 1 && values[0] == "true" { values.clear(); }
        keywords.push((key, values));
    }
    keywords.sort_by(|a, b| a.0.cmp(&b.0));
    let mut result = attributes;
    for (key, values) in keywords { result.push(key); result.extend(values); }
    Ok(result)
}

fn intl_canonical_locale_list(i: &mut Interpreter, value: &JsValue) -> Result<Vec<String>, Throw> {
    if matches!(value, JsValue::Undefined) { return Ok(Vec::new()); }
    let mut output = Vec::new();
    let mut push = |i: &mut Interpreter, candidate: &JsValue| -> Result<(), Throw> {
        let tag = match candidate {
            JsValue::Str(_) => i.to_string(candidate)?,
            JsValue::Object(object) => match intl_state(object) {
                Some(IntlState::Locale(tag)) => tag,
                _ => i.to_string(candidate)?,
            },
            _ => return Err(Throw::type_error("locale must be a string or object")),
        };
        let tag = intl_canonical_tag(&tag)?;
        if !output.contains(&tag) { output.push(tag); }
        Ok(())
    };
    match value {
        JsValue::Str(_) => push(i, value)?,
        JsValue::Object(object) if matches!(intl_state(object), Some(IntlState::Locale(_))) => push(i, value)?,
        JsValue::Null => return Err(Throw::type_error("locales is null")),
        _ => {
            // ToObject followed by LengthOfArrayLike. Plain objects use their
            // own prototype chain, primitives use the matching wrapper
            // prototype so inherited length/index properties are observed.
            let prototype = match value {
                JsValue::Object(object) => object.clone(),
                JsValue::Num(_) => i.number_proto.clone(),
                JsValue::Bool(_) => i.boolean_proto.clone(),
                JsValue::Symbol(_) => i.symbol_proto.clone(),
                JsValue::BigInt(_) => i.bigint_proto.clone(),
                _ => return Ok(Vec::new()),
            };
            let length = i.get_prop(value, "length")?;
            let length = i.to_number(&length)?.max(0.0).min(100_000.0) as usize;
            for index in 0..length {
                let key = index.to_string();
                if prototype.borrow().has_property(&key) {
                    let candidate = i.get_prop(value, &key)?;
                    push(i, &candidate)?;
                }
            }
        }
    }
    Ok(output)
}

/// The BCP-47 Lookup algorithm against `INTL_NUMBER_DATA`'s locale inventory,
/// for one already extension-stripped tag: truncate trailing subtags until an
/// exact data locale is found, then -- if that exhausts down to the bare
/// primary-language subtag with no exact hit -- match any data locale that
/// carries this language under a different script (for example "zh" or
/// "zh-TW" against the scripted "zh-Hans": the only Chinese data this engine
/// carries). Shared by `intl_lookup` and `intl_number_supported_locales` so
/// resolvedOptions().locale and supportedLocalesOf() stay consistent; see
/// intl402/fallback-locales-are-supported.js and
/// intl402/supportedLocalesOf-consistent-with-resolvedOptions.js.
/// `best_fit == false` is the BCP-47 Lookup algorithm: pure subtag
/// truncation, no script inference, so resolvedOptions().locale always stays
/// a literal string prefix of the request -- required by
/// intl402/supportedLocalesOf-consistent-with-resolvedOptions.js, which
/// classifies a request as "unsupported" the moment that isn't true. `best_fit
/// == true` additionally consults the CLDR likely-subtags table (already
/// generated for `Intl.Locale`'s own methods, `intl_likely_tag` in
/// builtins_intl_locale_info.rs) for an explicit script hint before
/// truncating a region away: this engine carries both "zh" (Hans) and
/// "zh-Hant" as literal rows, and plain truncation alone would resolve
/// "zh-TW"/"zh-HK"/"zh-US" down to bare "zh" even though CLDR says those
/// regions are Traditional by default. This is a no-op for every language
/// this engine has only one script for: the expanded tag's language-script
/// pair only ever matches a literal row for Chinese, since no other data
/// locale here keys on script. See intl402/NumberFormat/prototype/format/
/// {signDisplay,unit,notation-compact,engineering-scientific}-zh-TW.js.
fn intl_lookup_one(tag: &str, best_fit: bool) -> Option<&'static str> {
    if best_fit && tag.contains('-') {
        if let Some(expanded) = intl_likely_tag(tag) {
            let mut parts = expanded.split('-');
            let language = parts.next().unwrap_or("");
            if let Some(script) = parts.next().filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphabetic())) {
                let scripted = format!("{language}-{script}");
                if let Some(data) = INTL_NUMBER_DATA.iter().find(|data| data.locale.eq_ignore_ascii_case(&scripted)) {
                    return Some(data.locale);
                }
            }
        }
    }
    let mut candidate = tag;
    loop {
        if let Some(data) = INTL_NUMBER_DATA.iter().find(|data| data.locale.eq_ignore_ascii_case(candidate)) {
            return Some(data.locale);
        }
        let Some(dash) = candidate.rfind('-') else { break };
        candidate = &candidate[..dash];
    }
    INTL_NUMBER_DATA.iter()
        .find(|data| data.locale.split('-').next().unwrap().eq_ignore_ascii_case(candidate))
        .map(|data| data.locale)
}

fn intl_lookup(requested: &[String], best_fit: bool) -> String {
    for tag in requested {
        let no_extension = tag.split("-u-").next().unwrap_or(tag);
        if let Some(locale) = intl_lookup_one(no_extension, best_fit) {
            return locale.to_string();
        }
    }
    "en".to_string()
}

/// The BCP-47 locale string reported by `resolvedOptions()`. It keeps the
/// requested region for supported languages and falls back to the default
/// locale (en-US) when no requested tag matches the data locale.
fn intl_resolved_locale(requested: &[String], data_locale: &str) -> String {
    let data_language = data_locale.split('-').next().unwrap_or(data_locale);
    // Only echo the request's own region/script verbatim when its bare
    // language itself has a literal data row (e.g. "en" -> keep "en-GB").
    // When the data locale is reachable only through intl_lookup_one's
    // relaxed same-language-prefix fallback -- a bare language with no row
    // of its own, only a more specific one (e.g. "zh" has no row, only
    // "zh-Hans") -- echoing the bare request would make resolvedOptions()
    // report it as fully "supported" while supportedLocalesOf (which never
    // echoes, and matches by strict truncation only) disagrees. Returning
    // the real data locale instead keeps both consistent. See
    // intl402/supportedLocalesOf-consistent-with-resolvedOptions.js.
    let data_language_has_own_row = INTL_NUMBER_DATA.iter()
        .any(|data| data.locale.eq_ignore_ascii_case(data_language));
    for tag in requested {
        let base = tag.split("-u-").next().unwrap_or(tag);
        if base.split('-').next() == Some(data_language) {
            return if data_language_has_own_row { base.to_string() } else { data_locale.to_string() };
        }
    }
    "en-US".to_string()
}

/// Resolve one Unicode extension keyword against its option value per
/// ECMA-402 ResolveLocale. Returns the value to keep in the resolved locale (if
/// any) and the effective value used for formatting. An option value that the
/// implementation does not support is ignored in favour of the requested
/// extension value; a supported option that differs from the extension drops
/// the extension from the resolved locale.
fn intl_resolve_keyword(extension: Option<String>, option: Option<String>,
    supported: impl Fn(&str) -> bool, fallback: &str) -> (Option<String>, String) {
    if let Some(value) = option {
        if supported(&value) {
            let keep = extension.as_deref() == Some(value.as_str());
            return (if keep { Some(value.clone()) } else { None }, value);
        }
    }
    if let Some(value) = extension {
        if supported(&value) { return (Some(value.clone()), value); }
    }
    (None, fallback.to_string())
}

/// Append the retained Unicode extension keywords to a resolved base locale.
fn intl_locale_with_extensions(base: &str, retained: &[(&str, String)]) -> String {
    let mut pairs: Vec<(&str, String)> = retained.iter()
        .filter(|entry| !entry.1.is_empty())
        .map(|(key, value)| (*key, value.clone())).collect();
    if pairs.is_empty() { return base.to_string(); }
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    let mut subtags: Vec<String> = Vec::new();
    for (key, value) in pairs {
        subtags.push(key.to_string());
        for part in value.split('-') { if !part.is_empty() { subtags.push(part.to_string()); } }
    }
    format!("{base}-u-{}", subtags.join("-"))
}

/// The numbering systems the implementation advertises and can render native
/// digits for (CLDR simple digit mappings). An option value outside this list
/// is ignored per ECMA-402, and `INTL_SUPPORTED_NUMBERING_SYSTEMS` is generated
/// to match so `supportedValuesOf` never names an unrenderable system.
fn intl_supported_numbering_system(value: &str) -> bool {
    INTL_SUPPORTED_NUMBERING_SYSTEMS.split(' ').any(|entry| entry == value)
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
    let method = Interpreter::native_arity(name, native, &i.function_proto, arity);
    if let JsValue::Object(function) = &method {
        function.borrow_mut().constructible = false;
        function.borrow_mut().delete_own("prototype");
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
            binary_getter(self, &proto, "firstDayOfWeek", intl_locale_first_day_of_week);
            binary_getter(self, &proto, "variants", intl_locale_variants);
            for (name, method) in [
                ("getCalendars", intl_locale_get_calendars as crate::value::NativeFn),
                ("getCollations", intl_locale_get_collations),
                ("getHourCycles", intl_locale_get_hour_cycles),
                ("getNumberingSystems", intl_locale_get_numbering_systems),
                ("getTimeZones", intl_locale_get_time_zones),
                ("getTextInfo", intl_locale_get_text_info),
                ("getWeekInfo", intl_locale_get_week_info),
            ] { intl_method(self, &proto, name, 0, method); }
            let _ = proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
                binary_property(JsValue::str("Intl.Locale")));
            intl_register_intrinsic(self, "Locale", ctor, &proto);
            let _ = namespace.borrow_mut().define("Locale", binary_writable_property(locale));
        }
        self.install_intl_number(&namespace);
        self.install_intl_plural(&namespace);
        self.install_intl_datetime(&namespace);
        self.install_intl_relative_and_list(&namespace);
        self.install_intl_collator(&namespace);
        self.install_intl_segmenter(&namespace);
        self.install_intl_display_names(&namespace);
        self.install_intl_duration(&namespace);
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
    if key == "timeZone" {
        // AvailableTimeZones is canonicalised and de-duplicated: keep only
        // fixed points of the resolver so every listed name round-trips.
        let mut ids: Vec<String> = tz_supported_ids().into_iter()
            .filter_map(tz_canonicalize).collect();
        ids.sort_unstable();
        ids.dedup();
        return intl_array(i, ids.into_iter().map(JsValue::str));
    }
    let values: Vec<&str> = match key.as_str() {
        "calendar" => INTL_KNOWN_CALENDARS.to_vec(),
        "collation" => INTL_SUPPORTED_COLLATIONS.split(' ').filter(|value| !value.is_empty()).collect(),
        "currency" => INTL_ALL_CURRENCIES.split(' ').filter(|value| !value.is_empty()).collect(),
        "numberingSystem" => INTL_SUPPORTED_NUMBERING_SYSTEMS.split(' ').collect(),
        "unit" => INTL_SIMPLE_UNITS.to_vec(),
        _ => return Err(Throw::range_error("unsupported Intl value key")),
    };
    intl_array(i, values.into_iter().map(JsValue::str))
}

fn intl_locale_ctor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("Intl.Locale requires new")); }
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
                let valid = match property {
                    "language" => ((2..=3).contains(&value.len()) || (5..=8).contains(&value.len()))
                        && value.bytes().all(|byte| byte.is_ascii_alphabetic()),
                    "script" => value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_alphabetic()),
                    _ => (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic()))
                        || (value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit())),
                };
                if !valid { return Err(Throw::range_error(format!("invalid {property} option"))); }
                let mut subtags: Vec<String> = tag.split('-').map(str::to_string).collect();
                if prefix == 0 { subtags[0] = value; }
                else {
                    let position = if prefix == 1 { 1 } else if subtags.get(1).is_some_and(|part| part.len() == 4) { 2 } else { 1 };
                    let replaces = if prefix == 1 {
                        subtags.get(position).is_some_and(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()))
                    } else {
                        subtags.get(position).is_some_and(|part|
                            (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
                                || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit())))
                    };
                    if replaces { subtags[position] = value; }
                    else { subtags.insert(position, value); }
                }
                tag = intl_canonical_tag(&subtags.join("-"))?;
            }
        }
        let variants = i.get_prop(options, "variants")?;
        if !variants.is_undefined() {
            let text = i.to_string(&variants)?;
            let mut requested = Vec::new();
            for variant in text.split('-') {
                let valid = ((5..=8).contains(&variant.len())
                    || (variant.len() == 4 && variant.as_bytes()[0].is_ascii_digit()))
                    && variant.bytes().all(|byte| byte.is_ascii_alphanumeric());
                if !valid { return Err(Throw::range_error("invalid variants option")); }
                let variant = variant.to_ascii_lowercase();
                if requested.contains(&variant) { return Err(Throw::range_error("duplicate variants option")); }
                requested.push(variant);
            }
            requested.sort();
            let parts: Vec<&str> = tag.split('-').collect();
            let mut base_end = 1;
            if parts.get(base_end).is_some_and(|part| part.len() == 4) { base_end += 1; }
            if parts.get(base_end).is_some_and(|part| part.len() == 2 || part.len() == 3) { base_end += 1; }
            let mut extension_start = base_end;
            while parts.get(extension_start).is_some_and(|part| part.len() > 1) { extension_start += 1; }
            let mut updated: Vec<String> = parts[..base_end].iter().map(|part| (*part).into()).collect();
            updated.extend(requested);
            updated.extend(parts[extension_start..].iter().map(|part| (*part).into()));
            tag = intl_canonical_tag(&updated.join("-"))?;
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
            ("caseFirst", "kf")] {
            let value = i.get_prop(options, option)?;
            if value.is_undefined() { continue; }
            let text = i.to_string(&value)?;
            if (option == "hourCycle" && !["h11", "h12", "h23", "h24"].contains(&text.as_str()))
                || (option == "caseFirst" && !["upper", "lower", "false"].contains(&text.as_str())) {
                return Err(Throw::range_error(format!("invalid {option} option")));
            }
            let text = text.to_ascii_lowercase();
            if text.is_empty() || text.split('-').any(|part| !(3..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
                return Err(Throw::range_error(format!("invalid {option} option")));
            }
            keywords.insert(key.to_string(), if key == "ca" && text == "ethiopic-amete-alem" {
                "ethioaa".into()
            } else if key == "ca" && text == "islamicc" {
                "islamic-civil".into()
            } else { text });
        }
        let numeric = i.get_prop(options, "numeric")?;
        if !numeric.is_undefined() { keywords.insert("kn".into(), if numeric.to_boolean() { "true" } else { "false" }.into()); }
        let numbering_system = i.get_prop(options, "numberingSystem")?;
        if !numbering_system.is_undefined() {
            let text = i.to_string(&numbering_system)?;
            if text.is_empty() || text.split('-').any(|part| !(3..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
                return Err(Throw::range_error("invalid numberingSystem option"));
            }
            keywords.insert("nu".into(), text.to_ascii_lowercase());
        }
        let first_day = i.get_prop(options, "firstDayOfWeek")?;
        if !first_day.is_undefined() {
            let value = i.to_string(&first_day)?;
            let day = match value.as_str() {
                "1" => "mon", "2" => "tue", "3" => "wed", "4" => "thu",
                "5" => "fri", "6" => "sat", "7" | "0" => "sun",
                _ => value.as_str(),
            };
            if day.split('-').any(|part| !(3..=8).contains(&part.len())
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())) {
                return Err(Throw::range_error("invalid firstDayOfWeek option"));
            }
            keywords.insert("fw".into(), day.into());
        }
        if !keywords.is_empty() {
            let base = tag.split("-u-").next().unwrap_or(&tag);
            let suffix = keywords.into_iter().map(|(key, value)| if value.is_empty() || value == "true" { key }
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
    let mut expanded = intl_locale_maximized_base(&tag);
    if let Some((_, extension)) = tag.split_once("-u-") {
        expanded.push_str("-u-"); expanded.push_str(extension);
    }
    let constructor = i.global_env.resolve("Intl").ok_or_else(|| Throw::type_error("Intl unavailable"))?;
    let locale_ctor = i.get_prop(&constructor, "Locale")?;
    i.construct(&locale_ctor, vec![JsValue::str(expanded)])
}

fn intl_locale_minimize(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let tag = intl_locale_tag(&this)?;
    let maximal = intl_locale_maximized_base(&tag);
    let pieces: Vec<&str> = maximal.split('-').collect();
    let language = pieces[0];
    let script = pieces.get(1).copied().unwrap_or("Latn");
    let region = pieces.get(2).copied().unwrap_or("US");
    let maximal_base = format!("{language}-{script}-{region}");
    let shortened = if intl_locale_maximized_base(language) == maximal_base { language.to_string() }
        else if intl_locale_maximized_base(&format!("{language}-{region}")) == maximal_base {
            format!("{language}-{region}")
        } else if intl_locale_maximized_base(&format!("{language}-{script}")) == maximal_base {
            format!("{language}-{script}")
        } else { format!("{language}-{script}-{region}") };
    let mut shortened = shortened;
    let base = tag.split("-u-").next().unwrap_or(&tag);
    let parts: Vec<&str> = base.split('-').collect();
    let mut variant_start = 1;
    if parts.get(variant_start).is_some_and(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())) {
        variant_start += 1;
    }
    if parts.get(variant_start).is_some_and(|part| (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
        || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))) {
        variant_start += 1;
    }
    for variant in &parts[variant_start..] {
        shortened.push('-');
        shortened.push_str(variant);
    }
    if let Some((_, extension)) = tag.split_once("-u-") {
        shortened.push_str("-u-"); shortened.push_str(extension);
    }
    let constructor = i.global_env.resolve("Intl").ok_or_else(|| Throw::type_error("Intl unavailable"))?;
    let locale_ctor = i.get_prop(&constructor, "Locale")?;
    i.construct(&locale_ctor, vec![JsValue::str(shortened)])
}

include!("builtins_intl_locale_info.rs");

#[cfg(test)]
mod intl_tests {
    use super::*;

    #[test]
    fn canonical_locale_list_boxes_primitives_and_reads_length_each_step() {
        let mut i = Interpreter::new();
        let value = i.run_script(
            "Number.prototype[0]='en-US'; Number.prototype.length=1; \
             let boxed = Intl.getCanonicalLocales(NaN).join(','); \
             Number.prototype.length=0; \
             let locales = { '0': { toString: function(){ locales[1]='pt-BR'; return 'en-US'; } }, length: 2 }; \
             let sequenced = Intl.getCanonicalLocales(locales).join(','); \
             boxed + '|' + sequenced").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "en-US|en-US,pt-BR");
    }

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
    fn deprecated_und_variants_are_removed() {
        let mut i = Interpreter::new();
        let result = i.run_script("Intl.getCanonicalLocales('hy-arevela').join(',')").unwrap();
        assert_eq!(i.to_string(&result).unwrap(), "hy");
        let duplicate = i.run_script(
            "try { new Intl.Locale('de-t-en-emodeng-emodeng'); } catch (e) { e.constructor.name; }").unwrap();
        assert_eq!(i.to_string(&duplicate).unwrap(), "RangeError");
    }

    #[test]
    fn locale_options_update_unicode_extension() {
        let mut i = Interpreter::new();
        let result = i.run_script("new Intl.Locale('en-US-u-nu-latn', {calendar:'gregory',numeric:true}).toString()").unwrap();
        assert_eq!(i.to_string(&result).unwrap(), "en-US-u-ca-gregory-kn-nu-latn");
    }

    #[test]
    fn user_thrown_custom_errors_keep_identity() {
        let mut i = Interpreter::new();
        let result = i.run_script(
            "function CustomError() {} \
             let caught; \
             try { ({ get x() { throw new CustomError(); } }).x; } catch (e) { caught = e; } \
             caught.constructor === CustomError;").unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "unexpected: {result:?}");
    }

    #[test]
    fn custom_errors_from_intl_option_getters_propagate() {
        let mut i = Interpreter::new();
        let result = i.run_script(
            "function CustomError() {} \
             let caught; \
             try { new Intl.RelativeTimeFormat('en', { get localeMatcher() { throw new CustomError(); } }); } catch (e) { caught = e; } \
             caught && caught.constructor === CustomError;").unwrap();
        assert!(matches!(result, JsValue::Bool(true)), "unexpected: {result:?}");
    }
}
