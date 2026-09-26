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
fn regexp_match_with(
    i: &mut Interpreter,
    this: &JsValue,
    flags: amazing_regex::Flags,
    matcher: &mut amazing_regex::Matcher<'_>,
) -> Result<Option<amazing_regex::Match>, Throw> {
    let value = i.get_prop(this, "lastIndex")?;
    let number = i.to_number(&value)?;
    let index = if number.is_nan() || number <= 0.0 {
        0
    } else {
        number.floor().min(9_007_199_254_740_991.0) as usize
    };
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
) -> Result<JsValue, Throw> {
    let source = source.into();
    let units = regexp_units(&source)?;
    let mut values = Vec::new();
    for capture in matched.captures {
        values.push(match capture {
            Some(r) => JsValue::str_units(units[r].to_vec()),
            None => JsValue::Undefined,
        });
    }
    let array = array_values(i, values)?;
    i.set_prop(&array, "index", JsValue::num(matched.range.start as f64))?;
    i.set_prop(&array, "input", JsValue::str(source))?;
    i.set_prop(&array, "groups", JsValue::Undefined)?;
    Ok(array)
}
// RegExpExec(R, S) (21.2.5.2.1): reads "exec" and calls it when callable --
// which for a plain RegExp is RegExp.prototype.exec (native_regexp_exec)
// itself, so this has no effect on the common case beyond one indirect call
// -- falling back to RegExpBuiltinExec only when "exec" is not callable. The
// @@match/@@replace/@@matchAll algorithms all dispatch through this, not
// through the internal matcher directly, so an overridden `exec`, a
// non-writable `lastIndex`, or an abrupt `groups`/`0` getter on a custom
// result all take effect exactly where the spec puts them.
// True when `exec` is exactly the unmodified RegExp.prototype.exec (the
// overwhelmingly common case): callers matching in a loop over one very long
// string skip materializing a JS result object per match and re-reading it
// back out, and go straight to the internal matcher instead. `Get(this,
// "exec")` still runs every iteration either way, so an override installed
// mid-loop by a getter side effect is still observed on the next match.
fn regexp_exec_default(exec: &JsValue) -> bool {
    matches!(callable_function(exec), Some(crate::value::JsFunction { body: crate::value::FunctionBody::Native(f), .. })
        if std::ptr::fn_addr_eq(f, native_regexp_exec as crate::value::NativeFn))
}
fn regexp_exec_call(
    i: &mut Interpreter,
    this: &JsValue,
    source: &crate::value::JsString,
    exec: &JsValue,
) -> Result<JsValue, Throw> {
    if callable_function(exec).is_some() {
        let result = i.call_value(exec, this.clone(), vec![JsValue::str(source.clone())])?;
        if !result.is_object() && !result.is_null() {
            return Err(Throw::type_error("RegExp exec must return an object or null"));
        }
        return Ok(result);
    }
    if regexp_data(this).is_none() {
        return Err(Throw::type_error("incompatible RegExp receiver"));
    }
    match regexp_match(i, this, source.clone())? {
        Some(m) => regexp_result(i, source.clone(), m),
        None => Ok(JsValue::Null),
    }
}
// RegExpExec(R, S) (21.2.5.2.1): reads "exec" and calls it when callable --
// which for a plain RegExp is RegExp.prototype.exec (native_regexp_exec)
// itself, so this has no effect on the common case beyond one indirect call
// -- falling back to RegExpBuiltinExec only when "exec" is not callable. The
// @@match/@@replace/@@matchAll algorithms all dispatch through this, not
// through the internal matcher directly, so an overridden `exec`, a
// non-writable `lastIndex`, or an abrupt `groups`/`0` getter on a custom
// result all take effect exactly where the spec puts them.
fn regexp_exec_dispatch(
    i: &mut Interpreter,
    this: &JsValue,
    source: &crate::value::JsString,
) -> Result<JsValue, Throw> {
    let exec = i.get_prop(this, "exec")?;
    regexp_exec_call(i, this, source, &exec)
}
fn native_regexp_exec(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if regexp_data(&this).is_none() {
        return Err(Throw::type_error("incompatible RegExp receiver"));
    }
    let source = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match regexp_match(i, &this, &source)? {
        Some(m) => regexp_result(i, &source, m),
        None => Ok(JsValue::Null),
    }
}
fn native_regexp_test(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let source = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    // RegExpExec delegates to an overridden exec before falling back to slots.
    let exec = i.get_prop(&this, "exec")?;
    if callable_function(&exec).is_some() {
        let result = i.call_value(&exec, this, vec![JsValue::str(source)])?;
        if !result.is_object() && !result.is_null() {
            return Err(Throw::type_error(
                "RegExp exec must return an object or null",
            ));
        }
        return Ok(JsValue::Bool(!result.is_null()));
    }
    Ok(JsValue::Bool(regexp_match(i, &this, &source)?.is_some()))
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
        return regexp_exec_dispatch(i, &regexp, &source);
    }
    i.set_prop(&regexp, "lastIndex", JsValue::num(0.0))?;
    let units = regexp_units(&source)?;
    let mut matches = Vec::new();
    loop {
        let exec = i.get_prop(&regexp, "exec")?;
        let match_str = if regexp_exec_default(&exec) {
            match regexp_match(i, &regexp, &source)? {
                None => break,
                Some(m) => crate::value::JsString::from_units(units[m.range].to_vec()),
            }
        } else {
            let result = regexp_exec_call(i, &regexp, &source, &exec)?;
            if result.is_null() { break; }
            let match_value = i.get_prop(&result, "0")?;
            i.to_js_string(&match_value)?
        };
        if matches.len() >= 100_000 {
            return Err(Throw::range_error("RegExp match collection limit"));
        }
        matches.push(JsValue::str(match_str.clone()));
        if match_str.is_empty() {
            let last_index = i.get_prop(&regexp, "lastIndex")?;
            let last = i.to_number(&last_index)?;
            let next = amazing_regex::advance_index(
                &units,
                if last.is_nan() { 0 } else { last.max(0.0) as usize },
                regex.flags().unicode,
            );
            i.set_prop(&regexp, "lastIndex", JsValue::num(next as f64))?;
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
struct RegexpReplaceMatch {
    matched: crate::value::JsString,
    position: usize,
    captures: Vec<Option<crate::value::JsString>>,
    named: Option<JsValue>,
}
fn regexp_replace(i: &mut Interpreter, source: &crate::value::JsString, args: &[JsValue]) -> Result<JsValue, Throw> {
    let regexp = &args[0];
    let regex = regexp_data(regexp).unwrap();
    let global = regex.flags().global;
    let unicode = regex.flags().unicode;
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
    if global {
        i.set_prop(regexp, "lastIndex", JsValue::num(0.0))?;
    }
    let units = regexp_units(source)?;
    let mut matches = Vec::new();
    let mut slots = 0;
    loop {
        // RegExpExec, not the internal matcher directly: an overridden `exec`,
        // a `0`/`groups` getter with side effects, or a non-writable
        // `lastIndex` must all be observed exactly where the spec puts them.
        // The unmodified-`exec` case skips materializing and re-reading a JS
        // result object per match -- it matters when matching over one very
        // long string produces many thousands of matches in a single call.
        let exec = i.get_prop(regexp, "exec")?;
        let (matched, position, captures, named) = if regexp_exec_default(&exec) {
            match regexp_match(i, regexp, source)? {
                None => break,
                Some(m) => {
                    let matched = crate::value::JsString::from_units(units[m.range.clone()].to_vec());
                    let captures = std::iter::once(Some(matched.clone()))
                        .chain(m.captures.iter().skip(1).map(|r| r.clone()
                            .map(|r| crate::value::JsString::from_units(units[r].to_vec()))))
                        .collect::<Vec<_>>();
                    (matched, m.range.start, captures, JsValue::Undefined)
                }
            }
        } else {
            let result = regexp_exec_call(i, regexp, source, &exec)?;
            if result.is_null() { break; }
            let length_value = i.get_prop(&result, "length")?;
            let length_number = i.to_number(&length_value)?;
            let length = if length_number.is_nan() || length_number <= 0.0 { 0 }
                else { length_number.min(9_007_199_254_740_991.0) as usize };
            let n_captures = length.saturating_sub(1);
            slots += n_captures;
            if matches.len() >= 100_000 || slots > 1_048_576 {
                return Err(Throw::range_error("RegExp replacement collection limit"));
            }
            let matched_value = i.get_prop(&result, "0")?;
            let matched = i.to_js_string(&matched_value)?;
            let position_value = i.get_prop(&result, "index")?;
            let position_number = i.to_number(&position_value)?;
            let position = if position_number.is_nan() || position_number <= 0.0 { 0 }
                else { (position_number as usize).min(units.len()) };
            // `regexp_substitution`'s $&/$N indexing treats slot 0 as the
            // whole match and slot N (N >= 1) as capture group N, matching
            // amazing_regex::Match.captures's own convention -- so slot 0 is
            // seeded with `matched` up front rather than left for group 1.
            let mut captures = Vec::with_capacity(n_captures + 1);
            captures.push(Some(matched.clone()));
            for index in 1..=n_captures {
                let capture = i.get_prop(&result, &index.to_string())?;
                captures.push(if capture.is_undefined() { None } else { Some(i.to_js_string(&capture)?) });
            }
            // Get(result, "groups") happens unconditionally per match, before
            // the functional/string-replacement branch decides what to do
            // with it; the string branch's ToObject(namedCaptures) is where a
            // null/primitive groups value throws (e.g. `groups: null` on a
            // custom exec result).
            let named = i.get_prop(&result, "groups")?;
            (matched, position, captures, named)
        };
        let named = if named.is_undefined() { None } else if !callable {
            Some(std_legacy_to_object(i, named)?)
        } else { Some(named) };
        let empty = matched.is_empty();
        matches.push(RegexpReplaceMatch { matched, position, captures, named });
        if !global { break; }
        if empty {
            let last_index = i.get_prop(regexp, "lastIndex")?;
            let last = i.to_number(&last_index)?;
            let next = amazing_regex::advance_index(
                &units,
                if last.is_nan() { 0 } else { last.max(0.0) as usize },
                unicode,
            );
            i.set_prop(regexp, "lastIndex", JsValue::num(next as f64))?;
        }
    }
    let mut out = Vec::<u16>::new();
    let mut end = 0usize;
    for m in matches {
        // A match whose reported position precedes the end of the previous
        // replacement (possible only with a custom `exec`) is skipped whole,
        // per spec step 14's "if position >= nextSourcePosition" guard.
        if m.position < end { continue; }
        out.extend_from_slice(&units[end..m.position]);
        if callable {
            // m.captures[0] is the synthetic whole-match slot (see above);
            // the call arguments are matched, then groups 1..=n, so skip it.
            let mut call_args: Vec<_> = m.captures[1..].iter()
                .map(|c| c.clone().map(JsValue::str).unwrap_or(JsValue::Undefined))
                .collect();
            call_args.insert(0, JsValue::str(m.matched.clone()));
            call_args.push(JsValue::num(m.position as f64));
            call_args.push(JsValue::str(source));
            if let Some(named) = &m.named { call_args.push(named.clone()); }
            let value = i.call_value(&replacement, JsValue::Undefined, call_args)?;
            out.extend_from_slice(i.to_js_string(&value)?.units());
        } else {
            let range = m.position..(m.position + m.matched.units().len()).min(units.len());
            out.extend_from_slice(&regexp_substitution(
                &text,
                &m.captures,
                &units,
                range,
            )?);
        }
        end = (m.position + m.matched.units().len()).min(units.len());
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
