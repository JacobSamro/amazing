// RegExp bindings. All pattern machinery stays behind amazing_regex::Regex;
// the interpreter owns JS objects, coercion, lastIndex and replacement callbacks.
fn regexp_error(error: amazing_regex::Error) -> Throw {
    match error {
        amazing_regex::Error::Syntax(message) => Throw::syntax_error(message),
        amazing_regex::Error::Limit(message) => {
            Throw::range_error(format!("RegExp resource limit: {message}"))
        }
    }
}
fn regexp_units(source: impl Into<crate::value::JsString>) -> Result<Vec<u16>, Throw> {
    const MAX_UNITS: usize = 1_048_576;
    let source = source.into();
    let units = source.units().to_vec();
    if units.len() > MAX_UNITS {
        return Err(Throw::range_error(
            "RegExp resource limit: input code units",
        ));
    }
    Ok(units)
}
fn regexp_data(value: &JsValue) -> Option<Rc<amazing_regex::Regex>> {
    match value {
        JsValue::Object(o) => match &o.borrow().exotic {
            Exotic::RegExp(r) => Some(r.clone()),
            _ => None,
        },
        _ => None,
    }
}
fn regexp_source(pattern: &[u16]) -> crate::value::JsString {
    if pattern.is_empty() {
        return "(?:)".into();
    }
    let mut out = Vec::new();
    let mut escaped = false;
    for &c in pattern {
        match c {
            0x2f if !escaped => out.extend("\\/".encode_utf16()),
            0x0a => out.extend("\\n".encode_utf16()),
            0x0d => out.extend("\\r".encode_utf16()),
            0x2028 => out.extend("\\u2028".encode_utf16()),
            0x2029 => out.extend("\\u2029".encode_utf16()),
            _ => out.push(c),
        }
        escaped = c == 0x5c && !escaped;
    }
    crate::value::JsString::from_units(out)
}
fn regexp_property(value: JsValue, writable: bool) -> crate::value::Property {
    crate::value::Property {
        value,
        writable,
        enumerable: false,
        configurable: true,
        getter: None,
        setter: None,
    }
}
impl Interpreter {
    fn make_regexp(&mut self, regex: Rc<amazing_regex::Regex>) -> Result<JsValue, Throw> {
        let mut object = JsObject::ordinary();
        object.proto = Some(self.regexp_proto.clone());
        object.exotic = Exotic::RegExp(regex);
        let mut last = regexp_property(JsValue::num(0.0), true);
        last.configurable = false;
        object
            .define("lastIndex", last)
            .map_err(|_| Throw::type_error("cannot initialize lastIndex"))?;
        Ok(JsValue::object(object))
    }
    fn install_regexp(&mut self) {
        let constructor = Self::native("RegExp", native_regexp, &self.function_proto);
        if let JsValue::Object(object) = &constructor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.regexp_proto.clone()));
            let _ = object
                .borrow_mut()
                .define("length", regexp_property(JsValue::num(2.0), false));
            let escape = Self::native("escape", native_regexp_escape, &self.function_proto);
            if let JsValue::Object(o) = &escape {
                o.borrow_mut().constructible = false;
                let _ = o
                    .borrow_mut()
                    .define("length", regexp_property(JsValue::num(1.0), false));
            }
            let _ = object
                .borrow_mut()
                .define("escape", regexp_property(escape, true));
        }
        let _ = self
            .regexp_proto
            .borrow_mut()
            .define("constructor", regexp_property(constructor.clone(), true));
        for (name, fun, length) in [
            ("exec", native_regexp_exec as crate::value::NativeFn, 1),
            ("test", native_regexp_test, 1),
            ("toString", native_regexp_to_string, 0),
        ] {
            let method = Self::native(name, fun, &self.function_proto);
            if let JsValue::Object(o) = &method {
                o.borrow_mut().constructible = false;
                let _ = o.borrow_mut().define(
                    "length",
                    regexp_property(JsValue::num(length as f64), false),
                );
            }
            let _ = self
                .regexp_proto
                .borrow_mut()
                .define(name, regexp_property(method, true));
        }
        for name in [
            "source",
            "flags",
            "hasIndices",
            "global",
            "ignoreCase",
            "multiline",
            "dotAll",
            "unicode",
            "unicodeSets",
            "sticky",
        ] {
            let getter = Self::native(
                &format!("get {name}"),
                native_regexp_getter,
                &self.function_proto,
            );
            let mut prop = regexp_property(JsValue::Undefined, false);
            prop.getter = Some(getter);
            let _ = self.regexp_proto.borrow_mut().define(name, prop);
        }
        for (symbol, tag_name, fun) in [
            (
                WellKnownSymbol::Match,
                "[Symbol.match]",
                native_regexp_symbol_match as crate::value::NativeFn,
            ),
            (
                WellKnownSymbol::Search,
                "[Symbol.search]",
                native_regexp_symbol_search,
            ),
            (
                WellKnownSymbol::MatchAll,
                "[Symbol.matchAll]",
                native_regexp_symbol_match_all,
            ),
            (
                WellKnownSymbol::Split,
                "[Symbol.split]",
                native_regexp_symbol_split,
            ),
        ] {
            let method = Self::native(tag_name, fun, &self.function_proto);
            if let JsValue::Object(o) = &method {
                o.borrow_mut().constructible = false;
                let length = if symbol == WellKnownSymbol::Split { 2.0 } else { 1.0 };
                let _ = o
                    .borrow_mut()
                    .define("length", regexp_property(JsValue::num(length), false));
            }
            let _ = self
                .regexp_proto
                .borrow_mut()
                .define(symbol.symbol(), regexp_property(method, true));
        }
        self.define_global("RegExp", constructor);
    }
}
fn native_regexp(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let pattern = args.first().cloned().unwrap_or(JsValue::Undefined);
    let flag_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if let Some(original) = regexp_data(&pattern) {
        let constructing = matches!(&this, JsValue::Object(o) if o.borrow().proto.as_ref().is_some_and(|p|Rc::ptr_eq(&p.0,&i.regexp_proto.0)));
        if flag_value.is_undefined() && !constructing {
            return Ok(pattern);
        }
        let flags = if flag_value.is_undefined() {
            original.flags().as_string()
        } else {
            i.to_string(&flag_value)?
        };
        return i.make_regexp(Rc::new(
            amazing_regex::Regex::new_utf16(original.pattern_utf16(), &flags).map_err(regexp_error)?,
        ));
    }
    let pattern = if pattern.is_undefined() {
        crate::value::JsString::from("")
    } else {
        i.to_js_string(&pattern)?
    };
    let flags = if flag_value.is_undefined() {
        String::new()
    } else {
        i.to_string(&flag_value)?
    };
    i.make_regexp(Rc::new(
        amazing_regex::Regex::new_utf16(pattern.units(), &flags).map_err(regexp_error)?,
    ))
}
fn native_regexp_getter(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let name = i
        .native_stack
        .last()
        .and_then(|o| o.borrow().func.as_ref().map(|f| f.name.clone()))
        .unwrap_or_default();
    let key = name.strip_prefix("get ").unwrap_or("");
    if key == "flags" {
        if !this.is_object() {
            return Err(Throw::type_error("RegExp flags receiver must be an object"));
        }
        let mut result = String::new();
        for (property, flag) in [
            ("hasIndices", 'd'),
            ("global", 'g'),
            ("ignoreCase", 'i'),
            ("multiline", 'm'),
            ("dotAll", 's'),
            ("unicode", 'u'),
            ("unicodeSets", 'v'),
            ("sticky", 'y'),
        ] {
            if i.get_prop(&this, property)?.to_boolean() {
                result.push(flag);
            }
        }
        return Ok(JsValue::str(result));
    }
    let regex = match regexp_data(&this) {
        Some(r) => r,
        None if matches!(&this,JsValue::Object(o) if Rc::ptr_eq(&o.0,&i.regexp_proto.0)) => {
            return Ok(if key == "source" {
                JsValue::str("(?:)")
            } else {
                JsValue::Undefined
            });
        }
        None => return Err(Throw::type_error("incompatible RegExp receiver")),
    };
    let flags = regex.flags();
    Ok(match key {
        "source" => JsValue::Str(regexp_source(regex.pattern_utf16())),
        "hasIndices" => JsValue::Bool(flags.has_indices),
        "global" => JsValue::Bool(flags.global),
        "ignoreCase" => JsValue::Bool(flags.ignore_case),
        "multiline" => JsValue::Bool(flags.multiline),
        "dotAll" => JsValue::Bool(flags.dot_all),
        "unicode" => JsValue::Bool(flags.unicode),
        "unicodeSets" => JsValue::Bool(flags.unicode_sets),
        "sticky" => JsValue::Bool(flags.sticky),
        _ => JsValue::Undefined,
    })
}
fn native_regexp_to_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error(
            "RegExp toString receiver must be an object",
        ));
    }
    let source = i.get_prop(&this, "source")?;
    let source = i.to_string(&source)?;
    let flags = i.get_prop(&this, "flags")?;
    let flags = i.to_string(&flags)?;
    Ok(JsValue::str(format!("/{source}/{flags}")))
}
/// EncodeForRegExpEscape ( c ): appends the escaped form of one code point.
fn encode_for_regexp_escape(c: u32, out: &mut Vec<u16>) {
    const CONTROL_ESCAPES: &[(u32, u16)] = &[
        (9, b't' as u16),
        (10, b'n' as u16),
        (11, b'v' as u16),
        (12, b'f' as u16),
        (13, b'r' as u16),
    ];
    let is_syntax_or_solidus = matches!(
        char::from_u32(c),
        Some('^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/')
    );
    if is_syntax_or_solidus {
        out.push(0x5c);
        out.extend(char_utf16(c));
        return;
    }
    if let Some(&(_, escape)) = CONTROL_ESCAPES.iter().find(|&&(code, _)| code == c) {
        out.push(0x5c);
        out.push(escape);
        return;
    }
    // otherPunctuators, plus WhiteSpace/LineTerminator (ECMA-262 regex `\s`
    // class, which is exactly WhiteSpace union LineTerminator), plus lone
    // surrogates.
    let is_other_punctuator = matches!(
        char::from_u32(c),
        Some(',' | '-' | '=' | '<' | '>' | '#' | '&' | '!' | '%' | ':' | ';' | '@' | '~' | '\'' | '`' | '"')
    );
    let is_whitespace_or_line_terminator = matches!(
        c,
        9..=13 | 32 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    );
    let is_lone_surrogate = (0xD800..=0xDFFF).contains(&c);
    if is_other_punctuator || is_whitespace_or_line_terminator || is_lone_surrogate {
        if c <= 0xFF {
            out.extend(format!("\\x{c:02x}").encode_utf16());
        } else {
            for unit in char_utf16(c) {
                out.extend(format!("\\u{unit:04x}").encode_utf16());
            }
        }
        return;
    }
    out.extend(char_utf16(c));
}
/// StringToCodePoints: decodes UTF-16 code units into Unicode code points,
/// combining valid surrogate pairs and passing lone surrogates through as
/// their own numeric value (never substituting U+FFFD).
fn units_to_code_points(units: &[u16]) -> Vec<u32> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..=0xDBFF).contains(&unit) {
            if let Some(&low) = units.get(i + 1) {
                if (0xDC00..=0xDFFF).contains(&low) {
                    out.push(0x10000 + ((unit as u32 - 0xD800) << 10) + (low as u32 - 0xDC00));
                    i += 2;
                    continue;
                }
            }
        }
        out.push(unit as u32);
        i += 1;
    }
    out
}
/// UTF16EncodeCodePoint: a valid code point becomes 1-2 UTF-16 units; a
/// lone surrogate value (not a real code point) is passed through as-is.
fn char_utf16(c: u32) -> Vec<u16> {
    match char::from_u32(c) {
        Some(ch) => {
            let mut buf = [0u16; 2];
            ch.encode_utf16(&mut buf)[..].to_vec()
        }
        None => vec![c as u16],
    }
}
fn native_regexp_escape(
    _i: &mut Interpreter,
    _this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let JsValue::Str(s) = &value else {
        return Err(Throw::type_error("RegExp.escape argument must be a string"));
    };
    let code_points = units_to_code_points(s.units());
    let mut out = Vec::new();
    for (index, c) in code_points.into_iter().enumerate() {
        let is_decimal_digit = (0x30..=0x39).contains(&c);
        let is_ascii_letter = (0x41..=0x5A).contains(&c) || (0x61..=0x7A).contains(&c);
        if index == 0 && (is_decimal_digit || is_ascii_letter) {
            out.extend(format!("\\x{c:02x}").encode_utf16());
        } else {
            encode_for_regexp_escape(c, &mut out);
        }
    }
    Ok(JsValue::str_units(out))
}
fn regexp_match(
    i: &mut Interpreter,
    this: &JsValue,
    source: impl Into<crate::value::JsString>,
) -> Result<Option<amazing_regex::Match>, Throw> {
    let regex =
        regexp_data(this).ok_or_else(|| Throw::type_error("incompatible RegExp receiver"))?;
    let units = regexp_units(source)?;
    let mut matcher = regex.matcher_utf16(&units).map_err(regexp_error)?;
    regexp_match_with(i, this, regex.flags(), &mut matcher)
}
/// ToLength(? Get(rx, "lastIndex")), clamped exactly as RegExpBuiltinExec
/// requires (NaN or negative becomes 0; huge values saturate at 2**53-1).
fn regexp_last_index(i: &mut Interpreter, this: &JsValue) -> Result<usize, Throw> {
    let value = i.get_prop(this, "lastIndex")?;
    let number = i.to_number(&value)?;
    Ok(if number.is_nan() || number <= 0.0 {
        0
    } else {
        number.floor().min(9_007_199_254_740_991.0) as usize
    })
}
fn regexp_match_with(
    i: &mut Interpreter,
    this: &JsValue,
    flags: amazing_regex::Flags,
    matcher: &mut amazing_regex::Matcher<'_>,
) -> Result<Option<amazing_regex::Match>, Throw> {
    let index = regexp_last_index(i, this)?;
    let stateful = flags.global || flags.sticky;
    let result = matcher
        .exec(if stateful { index } else { 0 })
        .map_err(regexp_error)?;
    if stateful {
        i.set_prop(
            this,
            "lastIndex",
            JsValue::num(result.as_ref().map(|m| m.range.end).unwrap_or(0) as f64),
        )?;
    }
    Ok(result)
}
fn regexp_result(
    i: &mut Interpreter,
    source: impl Into<crate::value::JsString>,
    matched: amazing_regex::Match,
    group_names: &[(Box<str>, usize)],
    has_indices: bool,
) -> Result<JsValue, Throw> {
    let source = source.into();
    let units = regexp_units(&source)?;
    let mut values = Vec::new();
    for capture in &matched.captures {
        values.push(match capture {
            Some(r) => JsValue::str_units(units[r.clone()].to_vec()),
            None => JsValue::Undefined,
        });
    }
    let array = array_values(i, values)?;
    i.set_prop(&array, "index", JsValue::num(matched.range.start as f64))?;
    i.set_prop(&array, "input", JsValue::str(source))?;
    let groups = if group_names.is_empty() {
        JsValue::Undefined
    } else {
        let mut object = JsObject::ordinary();
        object.proto = None;
        let groups = JsValue::object(object);
        // A name may be declared by more than one `(?<name>...)` in
        // different alternatives (only one can ever participate at once):
        // pick whichever of its declarations actually matched.
        let mut seen: Vec<&str> = Vec::new();
        for (name, _) in group_names {
            if seen.contains(&name.as_ref()) {
                continue;
            }
            seen.push(name.as_ref());
            let value = group_names
                .iter()
                .filter(|(n, _)| n.as_ref() == name.as_ref())
                .find_map(|(_, idx)| matched.captures.get(*idx).and_then(|c| c.clone()))
                .map(|r| JsValue::str_units(units[r].to_vec()))
                .unwrap_or(JsValue::Undefined);
            i.set_prop(&groups, name.as_ref(), value)?;
        }
        groups
    };
    i.set_prop(&array, "groups", groups)?;
    if has_indices {
        let indices = regexp_indices_array(i, &matched, group_names)?;
        i.set_prop(&array, "indices", indices)?;
    }
    Ok(array)
}
/// MakeIndicesArray ( S, indices, groupNames, hasGroups ): a match array's
/// `.indices` property, parallel to the match array itself, with a `[start,
/// end]` UTF-16 code-unit pair per participating capture (`undefined` for
/// non-participating ones) and its own `.groups` object when named groups
/// are present.
fn regexp_indices_array(
    i: &mut Interpreter,
    matched: &amazing_regex::Match,
    group_names: &[(Box<str>, usize)],
) -> Result<JsValue, Throw> {
    let mut entries = Vec::with_capacity(matched.captures.len());
    for capture in &matched.captures {
        entries.push(match capture {
            Some(r) => array_values(
                i,
                vec![JsValue::num(r.start as f64), JsValue::num(r.end as f64)],
            )?,
            None => JsValue::Undefined,
        });
    }
    let array = array_values(i, entries.clone())?;
    let groups = if group_names.is_empty() {
        JsValue::Undefined
    } else {
        let mut object = JsObject::ordinary();
        object.proto = None;
        let groups = JsValue::object(object);
        let mut seen: Vec<&str> = Vec::new();
        for (name, _) in group_names {
            if seen.contains(&name.as_ref()) {
                continue;
            }
            seen.push(name.as_ref());
            let value = group_names
                .iter()
                .filter(|(n, _)| n.as_ref() == name.as_ref())
                .find_map(|(_, idx)| {
                    entries.get(*idx).cloned().filter(|v| !v.is_undefined())
                })
                .unwrap_or(JsValue::Undefined);
            i.set_prop(&groups, name.as_ref(), value)?;
        }
        groups
    };
    i.set_prop(&array, "groups", groups)?;
    Ok(array)
}
fn native_regexp_exec(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let Some(regex) = regexp_data(&this) else {
        return Err(Throw::type_error("incompatible RegExp receiver"));
    };
    let source = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match regexp_match(i, &this, &source)? {
        Some(m) => regexp_result(i, &source, m, regex.group_names(), regex.flags().has_indices),
        None => Ok(JsValue::Null),
    }
}
fn native_regexp_test(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let result = regexp_exec_abstract(i, &this, &source)?;
    Ok(JsValue::Bool(!result.is_null()))
}
/// RegExpExec ( R, S ): call an overridden `exec` if one is callable,
/// validating its return shape, else fall back to the built-in matching
/// algorithm. Shared by `test`, `Symbol.match`, `Symbol.search` and the
/// `Symbol.matchAll` iterator's `next`.
fn regexp_exec_abstract(
    i: &mut Interpreter,
    regexp: &JsValue,
    source: &crate::value::JsString,
) -> Result<JsValue, Throw> {
    if !regexp.is_object() {
        return Err(Throw::type_error("RegExp receiver must be an object"));
    }
    let exec = i.get_prop(regexp, "exec")?;
    if callable_function(&exec).is_some() {
        let result = i.call_value(&exec, regexp.clone(), vec![JsValue::str(source.clone())])?;
        if !result.is_object() && !result.is_null() {
            return Err(Throw::type_error(
                "RegExp exec must return an object or null",
            ));
        }
        return Ok(result);
    }
    let regex =
        regexp_data(regexp).ok_or_else(|| Throw::type_error("incompatible RegExp receiver"))?;
    match regexp_match(i, regexp, source.clone())? {
        Some(m) => regexp_result(i, source.clone(), m, regex.group_names(), regex.flags().has_indices),
        None => Ok(JsValue::Null),
    }
}
/// SpeciesConstructor(R, %RegExp%): the constructor `matchAll` builds its
/// clone from, honoring an overridden `constructor` or `[Symbol.species]`.
fn regexp_species_constructor(i: &mut Interpreter, this: &JsValue) -> Result<JsValue, Throw> {
    let default_ctor = i
        .regexp_proto
        .borrow()
        .get_own("constructor")
        .unwrap_or(JsValue::Undefined);
    if !this.is_object() {
        return Err(Throw::type_error("RegExp receiver must be an object"));
    }
    let ctor = i.get_prop(this, "constructor")?;
    if ctor.is_undefined() {
        return Ok(default_ctor);
    }
    if !ctor.is_object() {
        return Err(Throw::type_error("RegExp constructor must be an object"));
    }
    let species = i.get_prop(&ctor, WellKnownSymbol::Species.symbol())?;
    if species.is_undefined() || species.is_null() {
        return Ok(default_ctor);
    }
    if callable_function(&species).is_none() {
        return Err(Throw::type_error("[Symbol.species] must be a constructor"));
    }
    Ok(species)
}
/// RegExp.prototype[Symbol.match] ( string ) (§22.2.6.8).
fn native_regexp_symbol_match(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error(
            "RegExp.prototype[Symbol.match] receiver must be an object",
        ));
    }
    let s = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let global = i.get_prop(&this, "global")?.to_boolean();
    if !global {
        return regexp_exec_abstract(i, &this, &s);
    }
    let full_unicode =
        i.get_prop(&this, "unicode")?.to_boolean() || i.get_prop(&this, "unicodeSets")?.to_boolean();
    i.set_prop(&this, "lastIndex", JsValue::num(0.0))?;
    let mut results = Vec::new();
    loop {
        let result = regexp_exec_abstract(i, &this, &s)?;
        if result.is_null() {
            break;
        }
        let match_0 = i.get_prop(&result, "0")?;
        let match_str = i.to_js_string(&match_0)?;
        if results.len() >= 100_000 {
            return Err(Throw::range_error("RegExp match collection limit"));
        }
        let empty = match_str.units().is_empty();
        results.push(JsValue::str(match_str));
        if empty {
            let last_index = regexp_last_index(i, &this)?;
            let advanced = amazing_regex::advance_index(s.units(), last_index, full_unicode);
            i.set_prop(&this, "lastIndex", JsValue::num(advanced as f64))?;
        }
    }
    if results.is_empty() {
        Ok(JsValue::Null)
    } else {
        array_values(i, results)
    }
}
/// RegExp.prototype[Symbol.search] ( string ) (§22.2.6.9).
fn native_regexp_symbol_search(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error(
            "RegExp.prototype[Symbol.search] receiver must be an object",
        ));
    }
    let s = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let previous_last_index = i.get_prop(&this, "lastIndex")?;
    if !i.binding_same_value(&previous_last_index, &JsValue::num(0.0)) {
        i.set_prop(&this, "lastIndex", JsValue::num(0.0))?;
    }
    let result = regexp_exec_abstract(i, &this, &s)?;
    let current_last_index = i.get_prop(&this, "lastIndex")?;
    if !i.binding_same_value(&current_last_index, &previous_last_index) {
        i.set_prop(&this, "lastIndex", previous_last_index)?;
    }
    if result.is_null() {
        return Ok(JsValue::num(-1.0));
    }
    i.get_prop(&result, "index")
}
/// Internal state keys for a `%RegExpStringIteratorPrototype%`-shaped object.
/// Non-enumerable/non-configurable own properties, since this crate models
/// iterator internal slots as instance properties rather than a new `Exotic`
/// variant (see the Map/Set iterators in interp_symbol.rs for the same
/// convention).
const RSI_MATCHER: &str = "%RegExpStringIterator.matcher%";
const RSI_STRING: &str = "%RegExpStringIterator.string%";
const RSI_GLOBAL: &str = "%RegExpStringIterator.global%";
const RSI_UNICODE: &str = "%RegExpStringIterator.unicode%";
const RSI_DONE: &str = "%RegExpStringIterator.done%";
fn regexp_iter_state(value: JsValue) -> crate::value::Property {
    crate::value::Property {
        value,
        writable: true,
        enumerable: false,
        configurable: false,
        getter: None,
        setter: None,
    }
}
fn regexp_string_iterator(
    i: &Interpreter,
    matcher: JsValue,
    string: crate::value::JsString,
    global: bool,
    unicode: bool,
) -> JsValue {
    let mut object = JsObject::ordinary();
    object.proto = Some(i.object_proto.clone());
    let _ = object.define(RSI_MATCHER, regexp_iter_state(matcher));
    let _ = object.define(RSI_STRING, regexp_iter_state(JsValue::str(string)));
    let _ = object.define(RSI_GLOBAL, regexp_iter_state(JsValue::Bool(global)));
    let _ = object.define(RSI_UNICODE, regexp_iter_state(JsValue::Bool(unicode)));
    let _ = object.define(RSI_DONE, regexp_iter_state(JsValue::Bool(false)));
    let next = Interpreter::native("next", native_regexp_string_iterator_next, &i.function_proto);
    if let JsValue::Object(o) = &next {
        o.borrow_mut().constructible = false;
        o.borrow_mut().delete_own("prototype");
    }
    let _ = object.define("next", regexp_property(next, true));
    let self_iter = Interpreter::native(
        "[Symbol.iterator]",
        native_iterator_self_placeholder,
        &i.function_proto,
    );
    if let JsValue::Object(o) = &self_iter {
        o.borrow_mut().constructible = false;
        o.borrow_mut().delete_own("prototype");
    }
    let _ = object.define(WellKnownSymbol::Iterator.symbol(), regexp_property(self_iter, true));
    let _ = object.define(
        WellKnownSymbol::ToStringTag.symbol(),
        regexp_property(JsValue::str("RegExp String Iterator"), false),
    );
    JsValue::object(object)
}
fn native_iterator_self_placeholder(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(this)
}
fn regexp_iter_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut object = JsObject::ordinary();
    object.proto = Some(i.object_proto.clone());
    object.set_own("value", value);
    object.set_own("done", JsValue::Bool(done));
    JsValue::object(object)
}
fn native_regexp_string_iterator_next(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else {
        return Err(Throw::type_error(
            "incompatible RegExp String Iterator receiver",
        ));
    };
    let Some(matcher) = object.borrow().get_own(RSI_MATCHER) else {
        return Err(Throw::type_error(
            "incompatible RegExp String Iterator receiver",
        ));
    };
    if object
        .borrow()
        .get_own(RSI_DONE)
        .is_some_and(|d| d.to_boolean())
    {
        return Ok(regexp_iter_result(i, JsValue::Undefined, true));
    }
    let string = object.borrow().get_own(RSI_STRING).unwrap_or(JsValue::Undefined);
    let string = i.to_js_string(&string)?;
    let global = object
        .borrow()
        .get_own(RSI_GLOBAL)
        .is_some_and(|v| v.to_boolean());
    let full_unicode = object
        .borrow()
        .get_own(RSI_UNICODE)
        .is_some_and(|v| v.to_boolean());
    let result = regexp_exec_abstract(i, &matcher, &string)?;
    if result.is_null() {
        object.borrow_mut().set_own(RSI_DONE, JsValue::Bool(true));
        return Ok(regexp_iter_result(i, JsValue::Undefined, true));
    }
    if !global {
        object.borrow_mut().set_own(RSI_DONE, JsValue::Bool(true));
        return Ok(regexp_iter_result(i, result, false));
    }
    let match_0 = i.get_prop(&result, "0")?;
    let match_str = i.to_js_string(&match_0)?;
    if match_str.units().is_empty() {
        let last_index = regexp_last_index(i, &matcher)?;
        let advanced = amazing_regex::advance_index(string.units(), last_index, full_unicode);
        i.set_prop(&matcher, "lastIndex", JsValue::num(advanced as f64))?;
    }
    Ok(regexp_iter_result(i, result, false))
}
/// RegExp.prototype[Symbol.matchAll] ( string ) (§22.2.6.7).
fn native_regexp_symbol_match_all(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error(
            "RegExp.prototype[Symbol.matchAll] receiver must be an object",
        ));
    }
    let s = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let ctor = regexp_species_constructor(i, &this)?;
    let flags_value = i.get_prop(&this, "flags")?;
    let flags = i.to_js_string(&flags_value)?;
    let matcher = i.construct(&ctor, vec![this.clone(), JsValue::str(flags.clone())])?;
    let last_index = regexp_last_index(i, &this)?;
    i.set_prop(&matcher, "lastIndex", JsValue::num(last_index as f64))?;
    let global = flags.units().contains(&(b'g' as u16));
    let full_unicode = flags.units().contains(&(b'u' as u16)) || flags.units().contains(&(b'v' as u16));
    Ok(regexp_string_iterator(i, matcher, s, global, full_unicode))
}
/// RegExp.prototype[Symbol.split] ( string, limit )
fn native_regexp_symbol_split(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !this.is_object() {
        return Err(Throw::type_error(
            "RegExp.prototype[Symbol.split] receiver must be an object",
        ));
    }
    let s = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let limit_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let ctor = regexp_species_constructor(i, &this)?;
    let flags_value = i.get_prop(&this, "flags")?;
    let flags = i.to_js_string(&flags_value)?;
    let flag_units = flags.units();
    let unicode_matching =
        flag_units.contains(&(b'u' as u16)) || flag_units.contains(&(b'v' as u16));
    let sticky = flag_units.contains(&(b'y' as u16));
    let new_flags = if sticky {
        flags.clone()
    } else {
        let mut units = flags.units().to_vec();
        units.push(b'y' as u16);
        crate::value::JsString::from_units(units)
    };
    let splitter = i.construct(&ctor, vec![this.clone(), JsValue::str(new_flags)])?;
    let units = s.units().to_vec();
    let size = units.len();
    let lim = if limit_value.is_undefined() {
        u32::MAX as usize
    } else {
        i.to_uint32(&limit_value)? as usize
    };
    if lim == 0 {
        return array_values(i, Vec::new());
    }
    if size == 0 {
        let z = regexp_exec_abstract(i, &splitter, &s)?;
        if !z.is_null() {
            return array_values(i, Vec::new());
        }
        return array_values(i, vec![JsValue::str(s)]);
    }
    let mut results = Vec::new();
    let mut p = 0usize;
    let mut q = 0usize;
    while q < size {
        i.set_prop(&splitter, "lastIndex", JsValue::num(q as f64))?;
        let z = regexp_exec_abstract(i, &splitter, &s)?;
        if z.is_null() {
            q = amazing_regex::advance_index(&units, q, unicode_matching);
            continue;
        }
        let last_index = regexp_last_index(i, &splitter)?;
        let e = last_index.min(size);
        if e == p {
            q = amazing_regex::advance_index(&units, q, unicode_matching);
            continue;
        }
        results.push(JsValue::str_units(units[p..q].to_vec()));
        if results.len() == lim {
            return array_values(i, results);
        }
        p = e;
        let length_value = i.get_prop(&z, "length")?;
        let length = i.to_number(&length_value)?;
        let number_of_captures = if length.is_finite() && length > 1.0 {
            (length as usize) - 1
        } else {
            0
        };
        for idx in 1..=number_of_captures {
            let capture = i.get_prop(&z, idx.to_string())?;
            results.push(capture);
            if results.len() == lim {
                return array_values(i, results);
            }
        }
        q = p;
    }
    results.push(JsValue::str_units(units[p..size].to_vec()));
    array_values(i, results)
}
fn native_string_match(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let regexp = if regexp_data(&value).is_some() {
        value
    } else {
        native_regexp(i, JsValue::Undefined, vec![value])?
    };
    let regex = regexp_data(&regexp).unwrap();
    if !regex.flags().global {
        return match regexp_match(i, &regexp, &source)? {
            Some(m) => regexp_result(i, &source, m, regex.group_names(), regex.flags().has_indices),
            None => Ok(JsValue::Null),
        };
    }
    i.set_prop(&regexp, "lastIndex", JsValue::num(0.0))?;
    let units = regexp_units(&source)?;
    let mut matches = Vec::new();
    let mut matcher = regex.matcher_utf16(&units).map_err(regexp_error)?;
    while let Some(m) = regexp_match_with(i, &regexp, regex.flags(), &mut matcher)? {
        if matches.len() >= 100_000 {
            return Err(Throw::range_error("RegExp match collection limit"));
        }
        matches.push(JsValue::str_units(units[m.range.clone()].to_vec()));
        if m.range.is_empty() {
            i.set_prop(
                &regexp,
                "lastIndex",
                JsValue::num(amazing_regex::advance_index(
                    &units,
                    m.range.end,
                    regex.flags().unicode,
                ) as f64),
            )?;
        }
    }
    if matches.is_empty() {
        Ok(JsValue::Null)
    } else {
        array_values(i, matches)
    }
}
fn native_string_search(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = string_this_units(i, this)?;
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let regexp = if regexp_data(&value).is_some() {
        value
    } else {
        native_regexp(i, JsValue::Undefined, vec![value])?
    };
    let saved = i.get_prop(&regexp, "lastIndex")?;
    i.set_prop(&regexp, "lastIndex", JsValue::num(0.0))?;
    let result = regexp_match(i, &regexp, &source);
    i.set_prop(&regexp, "lastIndex", saved)?;
    Ok(JsValue::num(
        result?.map(|m| m.range.start as f64).unwrap_or(-1.0),
    ))
}
fn regexp_replace(i: &mut Interpreter, source: &crate::value::JsString, args: &[JsValue]) -> Result<JsValue, Throw> {
    let regexp = &args[0];
    let regex = regexp_data(regexp).unwrap();
    let replacement = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let callable = callable_function(&replacement).is_some();
    let text = if callable {
        crate::value::JsString::from("")
    } else {
        i.to_js_string(&replacement)?
    };
    if text.len() > 16_777_216 {
        return Err(Throw::range_error("RegExp replacement text limit"));
    }
    if regex.flags().global {
        i.set_prop(regexp, "lastIndex", JsValue::num(0.0))?;
    }
    let units = regexp_units(source)?;
    let mut matches = Vec::new();
    let mut slots = 0;
    let mut matcher = regex.matcher_utf16(&units).map_err(regexp_error)?;
    while let Some(m) = regexp_match_with(i, regexp, regex.flags(), &mut matcher)? {
        slots += m.captures.len();
        if matches.len() >= 100_000 || slots > 1_048_576 {
            return Err(Throw::range_error("RegExp replacement collection limit"));
        }
        let end = m.range.end;
        let empty = m.range.is_empty();
        matches.push(m);
        if !regex.flags().global {
            break;
        }
        if empty {
            i.set_prop(
                regexp,
                "lastIndex",
                JsValue::num(
                    amazing_regex::advance_index(&units, end, regex.flags().unicode) as f64,
                ),
            )?;
        }
    }
    let mut out = Vec::<u16>::new();
    let mut end = 0;
    for m in matches {
        out.extend_from_slice(&units[end..m.range.start]);
        let captures = m
            .captures
            .iter()
            .map(|r| r.clone().map(|r| crate::value::JsString::from_units(units[r].to_vec())))
            .collect::<Vec<_>>();
        if callable {
            let mut args: Vec<_> = captures
                .iter()
                .map(|c| c.clone().map(JsValue::str).unwrap_or(JsValue::Undefined))
                .collect();
            args.push(JsValue::num(m.range.start as f64));
            args.push(JsValue::str(source));
            if !regex.group_names().is_empty() {
                let mut object = JsObject::ordinary();
                object.proto = None;
                let groups = JsValue::object(object);
                for (name, index) in regex.group_names() {
                    let value = captures
                        .get(*index)
                        .cloned()
                        .flatten()
                        .map(JsValue::str)
                        .unwrap_or(JsValue::Undefined);
                    i.set_prop(&groups, name.as_ref(), value)?;
                }
                args.push(groups);
            }
            let value = i.call_value(&replacement, JsValue::Undefined, args)?;
            out.extend_from_slice(i.to_js_string(&value)?.units());
        } else {
            out.extend_from_slice(&regexp_substitution(
                &text,
                &captures,
                &units,
                m.range.clone(),
                regex.group_names(),
            )?);
        }
        end = m.range.end;
        if out.len() > 16_777_216 {
            return Err(Throw::range_error("RegExp replacement output limit"));
        }
    }
    out.extend_from_slice(&units[end..]);
    if out.len() > 16_777_216 {
        return Err(Throw::range_error("RegExp replacement output limit"));
    }
    Ok(JsValue::str_units(out))
}
fn regexp_substitution(
    text: &crate::value::JsString,
    captures: &[Option<crate::value::JsString>],
    input: &[u16],
    range: std::ops::Range<usize>,
    group_names: &[(Box<str>, usize)],
) -> Result<Vec<u16>, Throw> {
    let mut out = Vec::new();
    let mut chars = text.units().iter().copied().peekable();
    while let Some(c) = chars.next() {
        if out.len() > 16_777_216 {
            return Err(Throw::range_error("RegExp replacement output limit"));
        }
        if c != b'$' as u16 {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some(0x24) => {
                chars.next();
                out.push(0x24);
            }
            Some(0x26) => {
                chars.next();
                if let Some(capture) = &captures[0] { out.extend_from_slice(capture.units()); }
            }
            Some(0x60) => {
                chars.next();
                out.extend_from_slice(&input[..range.start]);
            }
            Some(0x27) => {
                chars.next();
                out.extend_from_slice(&input[range.end..]);
            }
            Some(0x3c) if !group_names.is_empty() => {
                let mut look = chars.clone();
                look.next();
                let mut name_units = Vec::new();
                let mut closed = false;
                while let Some(&u) = look.peek() {
                    if u == 0x3e {
                        closed = true;
                        break;
                    }
                    name_units.push(u);
                    look.next();
                }
                if closed {
                    let name = String::from_utf16_lossy(&name_units);
                    chars.next();
                    for _ in 0..name_units.len() {
                        chars.next();
                    }
                    chars.next();
                    if let Some((_, index)) = group_names.iter().find(|(n, _)| n.as_ref() == name)
                    {
                        if let Some(capture) = &captures[*index] {
                            out.extend_from_slice(capture.units());
                        }
                    }
                } else {
                    out.push(0x24);
                }
            }
            Some(d @ 0x30..=0x39) => {
                let one = (d - 0x30) as usize;
                let mut look = chars.clone();
                look.next();
                let two = look
                    .peek()
                    .filter(|c| (0x30u16..=0x39u16).contains(*c))
                    .map(|n| one * 10 + (*n - 0x30) as usize);
                let group = two
                    .filter(|n| *n > 0 && *n < captures.len())
                    .or_else(|| (one > 0 && one < captures.len()).then_some(one));
                if let Some(n) = group {
                    chars.next();
                    if Some(n) == two {
                        chars.next();
                    }
                    if let Some(capture) = &captures[n] { out.extend_from_slice(capture.units()); }
                } else {
                    out.push(0x24);
                }
            }
            _ => out.push(0x24),
        }
    }
    if out.len() > 16_777_216 {
        return Err(Throw::range_error("RegExp replacement output limit"));
    }
    Ok(out)
}
