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
            // `RegExp.prototype` (§22.2.4): unlike an ordinary function's
            // own `prototype` (writable, non-configurable), this one is
            // `{ writable: false, enumerable: false, configurable: false }`.
            // `set_own`'s default (all-true, `Property::data`) was
            // overwriting whatever `Self::native` had set with the wrong
            // attributes -- built-ins/RegExp/prototype/S15.10.5.1_A2/A3/A4.js
            // check exactly this triple via `propertyHelper.js`.
            let _ = object.borrow_mut().define(
                "prototype",
                crate::value::Property {
                    value: JsValue::Object(self.regexp_proto.clone()),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    getter: None,
                    setter: None,
                },
            );
            let _ = object
                .borrow_mut()
                .define("length", regexp_property(JsValue::num(2.0), false));
            let escape = Self::native("escape", native_regexp_escape, &self.function_proto);
            if let JsValue::Object(o) = &escape {
                o.borrow_mut().constructible = false;
                o.borrow_mut().delete_own("prototype");
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
                o.borrow_mut().delete_own("prototype");
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
        // Annex B.2.5.1 `RegExp.prototype.compile ( pattern, flags )`: a
        // real, own method (test262's `B.RegExp.prototype.compile.js` and
        // the `length`/`name` descriptor tests need it to actually exist,
        // not just be dispatched by name).
        let compile = Self::native_arity("compile", native_regexp_compile, &self.function_proto, 2);
        if let JsValue::Object(o) = &compile {
            o.borrow_mut().constructible = false;
            o.borrow_mut().delete_own("prototype");
        }
        let _ = self
            .regexp_proto
            .borrow_mut()
            .define("compile", regexp_property(compile, true));
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
                o.borrow_mut().delete_own("prototype");
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
        // Annex B.2.5 "legacy RegExp static properties" ($1-$9, input/$_,
        // lastMatch/$&, lastParen/$+, leftContext/$`, rightContext/$'):
        // accessor properties on the *constructor* object itself (not its
        // prototype -- test262 looks these up via
        // `Object.getOwnPropertyDescriptor(RegExp, "...")`). `legacy_regexp_store`
        // lazily creates the realm-scoped backing values (a hidden data
        // property on `regexp_proto`, not a new Interpreter/RealmState
        // field, so this stays entirely inside js-date's own grant) the
        // first time any of these is touched.
        if let JsValue::Object(object) = &constructor {
            for (name, alias, has_setter) in [
                ("input", Some("$_"), true),
                ("lastMatch", Some("$&"), false),
                ("lastParen", Some("$+"), false),
                ("leftContext", Some("$`"), false),
                ("rightContext", Some("$'"), false),
            ] {
                for key in std::iter::once(name).chain(alias) {
                    let getter = Self::native(
                        &format!("get {key}"),
                        native_regexp_legacy_getter,
                        &self.function_proto,
                    );
                    let mut prop = regexp_property(JsValue::Undefined, false);
                    prop.getter = Some(getter);
                    if has_setter {
                        prop.setter = Some(Self::native_arity(
                            &format!("set {key}"),
                            native_regexp_legacy_setter,
                            &self.function_proto,
                            1,
                        ));
                    }
                    let _ = object.borrow_mut().define(key, prop);
                }
            }
            for n in 1..=9 {
                let key = format!("${n}");
                let getter = Self::native(
                    &format!("get {key}"),
                    native_regexp_legacy_getter,
                    &self.function_proto,
                );
                let mut prop = regexp_property(JsValue::Undefined, false);
                prop.getter = Some(getter);
                let _ = object.borrow_mut().define(key, prop);
            }
        }
        // `RegExp[Symbol.species]` (§22.2.5.2): a `get` accessor returning
        // the receiver unchanged, the same idiom `native_array_species`/
        // `symbol_method` already establish for Array -- reused verbatim
        // (both live in the same flat `include!`d module as this file, see
        // interp.rs) so the accessor function's own `name`/`length`
        // descriptors match what `built-ins/RegExp/Symbol.species/*.js`
        // checks without duplicating that bookkeeping here.
        if let JsValue::Object(object) = &constructor {
            let getter = symbol_method(self, "get [Symbol.species]", native_regexp_species);
            let _ = object.borrow_mut().define(
                WellKnownSymbol::Species.symbol(),
                crate::value::Property {
                    value: JsValue::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    getter: Some(getter),
                    setter: None,
                },
            );
        }
        self.install_regexp_string_iterator_proto();
        self.define_global("RegExp", constructor);
    }
    /// `%RegExpStringIteratorPrototype%` (§22.2.7.6), installed once per
    /// realm rather than redefined on every iterator instance -- test262's
    /// `built-ins/RegExpStringIteratorPrototype/next/*` tests look up the
    /// shared prototype via `Object.getPrototypeOf` on a live iterator and
    /// inspect `next`'s own property descriptor there, which only exists if
    /// every iterator instance shares one prototype object.
    fn install_regexp_string_iterator_proto(&mut self) {
        let next = Self::native("next", native_regexp_string_iterator_next, &self.function_proto);
        if let JsValue::Object(o) = &next {
            o.borrow_mut().constructible = false;
            o.borrow_mut().delete_own("prototype");
        }
        let _ = self
            .regexp_string_iterator_proto
            .borrow_mut()
            .define("next", crate::value::Property { value: next, writable: true, enumerable: false, configurable: true, getter: None, setter: None });
        let self_iter = Self::native(
            "[Symbol.iterator]",
            native_iterator_self_placeholder,
            &self.function_proto,
        );
        if let JsValue::Object(o) = &self_iter {
            o.borrow_mut().constructible = false;
            o.borrow_mut().delete_own("prototype");
        }
        let _ = self.regexp_string_iterator_proto.borrow_mut().define(
            WellKnownSymbol::Iterator.symbol(),
            crate::value::Property { value: self_iter, writable: true, enumerable: false, configurable: true, getter: None, setter: None },
        );
        let _ = self.regexp_string_iterator_proto.borrow_mut().define(
            WellKnownSymbol::ToStringTag.symbol(),
            crate::value::Property { value: JsValue::str("RegExp String Iterator"), writable: false, enumerable: false, configurable: true, getter: None, setter: None },
        );
    }
}
// Annex B.2.5's legacy RegExp static properties need a mutable, realm-scoped
// place to live. Rather than adding a field to `Interpreter`/`RealmState`
// (owned outside this lane's grant -- see AGENTS.md's LANE_PATHS), the
// values are kept as an ordinary hidden data property directly on
// `regexp_proto`, which is itself already realm-scoped and swapped on
// cross-realm entry. The key is a string no JS source can spell as an
// identifier or an ordinary property-access string (`RegExp["\0..."]` is
// technically possible but not something any test262 test does), so it
// can't collide with a real property and doesn't need a new internal-slot
// mechanism in `value.rs`.
const LEGACY_STATICS_KEY: &str = "\u{0}legacyRegExpStatics";
const LEGACY_STATICS_FIELDS: [&str; 14] = [
    "input", "lastMatch", "lastParen", "leftContext", "rightContext",
    "paren1", "paren2", "paren3", "paren4", "paren5", "paren6", "paren7", "paren8", "paren9",
];
fn legacy_regexp_store(i: &mut Interpreter) -> JsValue {
    if let Some(existing) = i.regexp_proto.borrow().get_own(LEGACY_STATICS_KEY) {
        return existing;
    }
    let mut store = JsObject::ordinary();
    store.proto = None;
    for key in LEGACY_STATICS_FIELDS {
        let _ = store.define(key, regexp_property(JsValue::str(""), true));
    }
    let value = JsValue::object(store);
    let _ = i
        .regexp_proto
        .borrow_mut()
        .define(LEGACY_STATICS_KEY, regexp_property(value.clone(), true));
    value
}
/// Maps a legacy accessor's own name (its canonical form or its `$`-sigil
/// alias) to the backing field `legacy_regexp_store` uses for it.
fn legacy_regexp_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "input" | "$_" => "input",
        "lastMatch" | "$&" => "lastMatch",
        "lastParen" | "$+" => "lastParen",
        "leftContext" | "$`" => "leftContext",
        "rightContext" | "$'" => "rightContext",
        "$1" => "paren1",
        "$2" => "paren2",
        "$3" => "paren3",
        "$4" => "paren4",
        "$5" => "paren5",
        "$6" => "paren6",
        "$7" => "paren7",
        "$8" => "paren8",
        "$9" => "paren9",
        _ => return None,
    })
}
/// `GetLegacyRegExpStaticProperty`/`SetLegacyRegExpStaticProperty`'s shared
/// receiver check (Annex B.2.5): every legacy accessor throws unless `this`
/// is *exactly* the current realm's own `%RegExp%` intrinsic -- not a
/// subclass, not a cross-realm `%RegExp%`, not `RegExp.prototype`. Read via
/// `regexp_proto`'s own "constructor" property (set in `install_regexp`)
/// rather than a new Interpreter field, for the same lane-grant reason as
/// `legacy_regexp_store` above.
fn legacy_regexp_receiver_ok(i: &Interpreter, this: &JsValue) -> bool {
    let Some(JsValue::Object(expected)) = i.regexp_proto.borrow().get_own("constructor") else {
        return false;
    };
    matches!(this, JsValue::Object(actual) if Rc::ptr_eq(&actual.0, &expected.0))
}
fn native_regexp_legacy_getter(
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
    if !legacy_regexp_receiver_ok(i, &this) {
        return Err(Throw::type_error(
            "legacy RegExp static property receiver must be the RegExp constructor",
        ));
    }
    let field = legacy_regexp_field(key).unwrap_or("input");
    let store = legacy_regexp_store(i);
    i.get_prop(&store, field)
}
/// Only `RegExp.input`/`RegExp.$_` has a setter (Annex B.2.5.1); every other
/// legacy accessor is get-only (`[[Set]]: undefined`), so this doesn't need
/// the getter's name-sniffing trick.
fn native_regexp_legacy_setter(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if !legacy_regexp_receiver_ok(i, &this) {
        return Err(Throw::type_error(
            "legacy RegExp static property receiver must be the RegExp constructor",
        ));
    }
    let value = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let store = legacy_regexp_store(i);
    i.set_prop(&store, "input", JsValue::Str(value))?;
    Ok(JsValue::Undefined)
}
/// Annex B.2.5.2 `UpdateLegacyRegExpStaticProperties`, called from every
/// successful built-in match (both `RegExp.prototype.exec` itself and the
/// `RegExpExec` fallback path other RegExp algorithms use when `exec` isn't
/// overridden -- see the two call sites below). `matched.captures[0]` is
/// `amazing_regex::Match`'s own convention for the whole-match range;
/// `captures[1..]` are the capturing groups in declaration order.
fn update_legacy_regexp_statics(
    i: &mut Interpreter,
    source: &crate::value::JsString,
    matched: &amazing_regex::Match,
) -> Result<(), Throw> {
    let units = regexp_units(source.clone())?;
    let text = |r: std::ops::Range<usize>| JsValue::str_units(units[r].to_vec());
    let store = legacy_regexp_store(i);
    i.set_prop(&store, "input", JsValue::Str(source.clone()))?;
    i.set_prop(&store, "lastMatch", text(matched.range.clone()))?;
    i.set_prop(&store, "leftContext", text(0..matched.range.start))?;
    i.set_prop(&store, "rightContext", text(matched.range.end..units.len()))?;
    let last_paren = matched
        .captures
        .iter()
        .skip(1)
        .rev()
        .find_map(|c| c.clone())
        .map(text)
        .unwrap_or_else(|| JsValue::str(""));
    i.set_prop(&store, "lastParen", last_paren)?;
    for n in 1..=9usize {
        let value = matched
            .captures
            .get(n)
            .and_then(|c| c.clone())
            .map(text)
            .unwrap_or_else(|| JsValue::str(""));
        i.set_prop(&store, &format!("paren{n}"), value)?;
    }
    Ok(())
}
/// Annex B.2.5.1 `RegExp.prototype.compile ( pattern, flags )`: re-runs
/// `RegExpInitialize` on the *same* object (unlike `new RegExp(...)`, which
/// always allocates a fresh one), including resetting `lastIndex` to 0.
fn native_regexp_compile(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else {
        return Err(Throw::type_error(
            "RegExp.prototype.compile called on non-object receiver",
        ));
    };
    if regexp_data(&this).is_none() {
        return Err(Throw::type_error(
            "RegExp.prototype.compile called on incompatible receiver",
        ));
    }
    let pattern_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
    let flags_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let (pattern_units, flags_string) = if let Some(other) = regexp_data(&pattern_arg) {
        if !flags_arg.is_undefined() {
            return Err(Throw::type_error(
                "cannot supply flags when recompiling RegExp.prototype.compile from a RegExp",
            ));
        }
        (other.pattern_utf16().to_vec(), other.flags().as_string())
    } else {
        let pattern_units = if pattern_arg.is_undefined() {
            Vec::new()
        } else {
            i.to_js_string(&pattern_arg)?.units().to_vec()
        };
        let flags_string = if flags_arg.is_undefined() {
            String::new()
        } else {
            i.to_string(&flags_arg)?
        };
        (pattern_units, flags_string)
    };
    let regex = amazing_regex::Regex::new_utf16(&pattern_units, &flags_string).map_err(regexp_error)?;
    object.borrow_mut().exotic = Exotic::RegExp(Rc::new(regex));
    let mut last_index = regexp_property(JsValue::num(0.0), true);
    last_index.configurable = false;
    let _ = object.borrow_mut().define("lastIndex", last_index);
    Ok(this)
}
/// `IsRegExp ( argument )` (§21.2.3.2.1): an actual RegExp instance, or any
/// other object exposing a truthy `@@match` -- and, notably, NOT a real
/// RegExp instance whose `@@match` has been overridden to something falsy
/// (`built-ins/RegExp/call_with_regexp_match_falsy.js`).
fn is_regexp_like(i: &mut Interpreter, value: &JsValue) -> Result<bool, Throw> {
    if !value.is_object() {
        return Ok(false);
    }
    let matcher = i.get_prop(value, WellKnownSymbol::Match.symbol())?;
    Ok(if matcher.is_undefined() {
        regexp_data(value).is_some()
    } else {
        matcher.to_boolean()
    })
}
fn native_regexp(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let pattern = args.first().cloned().unwrap_or(JsValue::Undefined);
    let flag_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let pattern_is_regexp = is_regexp_like(i, &pattern)?;
    // This engine's `NativeFn` calling convention has no NewTarget, so
    // "was this invoked via `new`" is approximated the same way the
    // pre-existing code already did: a freshly `[[Construct]]`-allocated
    // `this` carries `regexp_proto` as its prototype, which a plain
    // `RegExp(...)` call's `this` (the global object, or undefined) does
    // not.
    let constructing = matches!(&this, JsValue::Object(o) if o.borrow().proto.as_ref().is_some_and(|p| Rc::ptr_eq(&p.0, &i.regexp_proto.0)));
    if pattern_is_regexp && flag_value.is_undefined() && !constructing {
        // Step 4b: only short-circuits to the *same* pattern object when
        // its own "constructor" is exactly this realm's active %RegExp%
        // (`from-regexp-like-short-circuit.js` requires the check;
        // `call_with_regexp_not_same_constructor.js` requires it to
        // otherwise fall through and build a distinct, equal-content
        // object instead).
        let pattern_ctor = i.get_prop(&pattern, "constructor")?;
        let active_ctor = i.regexp_proto.borrow().get_own("constructor");
        let same = matches!(
            (&pattern_ctor, &active_ctor),
            (JsValue::Object(a), Some(JsValue::Object(b))) if Rc::ptr_eq(&a.0, &b.0)
        );
        if same {
            return Ok(pattern);
        }
    }
    // Steps 6-9: a RegExp-like pattern's source/flags come from property
    // GETS -- observing overrides and forwarding thrown errors exactly
    // where the spec puts them, e.g. `from-regexp-like-flag-override.js`
    // requires "flags" to go entirely unread once an explicit `flags`
    // argument is given -- never from ToString-ing the whole object (which
    // is what a plain, non-regexp-like pattern gets instead).
    let (pattern_units, flags_string) = if pattern_is_regexp {
        let source = i.get_prop(&pattern, "source")?;
        let source_units = i.to_js_string(&source)?.units().to_vec();
        let flags_string = if flag_value.is_undefined() {
            let flags = i.get_prop(&pattern, "flags")?;
            i.to_string(&flags)?
        } else {
            i.to_string(&flag_value)?
        };
        (source_units, flags_string)
    } else {
        let pattern_units = if pattern.is_undefined() {
            Vec::new()
        } else {
            i.to_js_string(&pattern)?.units().to_vec()
        };
        let flags_string = if flag_value.is_undefined() {
            String::new()
        } else {
            i.to_string(&flag_value)?
        };
        (pattern_units, flags_string)
    };
    i.make_regexp(Rc::new(
        amazing_regex::Regex::new_utf16(&pattern_units, &flags_string).map_err(regexp_error)?,
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
/// Builds a `groups` object from captures already materialized as
/// `JsString`s (the default-`exec` fast path in `regexp_replace`, which
/// skips building a full match-result object per match). Same dedup rule as
/// `regexp_result`'s `groups`: a name declared on more than one
/// mutually-exclusive alternative picks whichever declaration's capture
/// actually participated.
fn regexp_named_captures(
    i: &mut Interpreter,
    group_names: &[(Box<str>, usize)],
    captures: &[Option<crate::value::JsString>],
) -> Result<JsValue, Throw> {
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
            .find_map(|(_, idx)| captures.get(*idx).and_then(|c| c.clone()))
            .map(JsValue::str)
            .unwrap_or(JsValue::Undefined);
        i.set_prop(&groups, name.as_ref(), value)?;
    }
    Ok(groups)
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
    let Some(regex) = regexp_data(this) else {
        return Err(Throw::type_error("incompatible RegExp receiver"));
    };
    match regexp_match(i, this, source.clone())? {
        Some(m) => {
            update_legacy_regexp_statics(i, &source, &m)?;
            regexp_result(i, source.clone(), m, regex.group_names(), regex.flags().has_indices)
        }
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
    let Some(regex) = regexp_data(&this) else {
        return Err(Throw::type_error("incompatible RegExp receiver"));
    };
    let source = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    match regexp_match(i, &this, &source)? {
        Some(m) => {
            update_legacy_regexp_statics(i, &source, &m)?;
            regexp_result(i, &source, m, regex.group_names(), regex.flags().has_indices)
        }
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
/// `get RegExp[Symbol.species]` (§22.2.5.2): returns the receiver
/// unchanged, so a subclass that doesn't override it derives itself rather
/// than the base `RegExp`.
fn native_regexp_species(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
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
    // §22.2.6.8 step 4 reads the *composed* "flags" string exactly once,
    // even on a non-global regexp that returns right after -- which is why
    // an overridden "unicode" accessor must still be observed for a
    // non-global receiver too (`Symbol.match/get-unicode-error.js`: both
    // `nonGlobalRe` and `globalRe` must throw). Reading "global"/"unicode"
    // as separate, conditionally-skipped property gets (the previous
    // approach) can never do that.
    let flags_value = i.get_prop(&this, "flags")?;
    let flags = i.to_js_string(&flags_value)?;
    if !flags.units().contains(&(b'g' as u16)) {
        return regexp_exec_abstract(i, &this, &s);
    }
    let full_unicode =
        flags.units().contains(&(b'u' as u16)) || flags.units().contains(&(b'v' as u16));
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
    object.proto = Some(i.regexp_string_iterator_proto.clone());
    let _ = object.define(RSI_MATCHER, regexp_iter_state(matcher));
    let _ = object.define(RSI_STRING, regexp_iter_state(JsValue::str(string)));
    let _ = object.define(RSI_GLOBAL, regexp_iter_state(JsValue::Bool(global)));
    let _ = object.define(RSI_UNICODE, regexp_iter_state(JsValue::Bool(unicode)));
    let _ = object.define(RSI_DONE, regexp_iter_state(JsValue::Bool(false)));
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
    // RegExp.prototype[Symbol.replace] (§22.2.6.11) reads the *composed*
    // "flags" string exactly once (step 6.a.i for a string replacement,
    // and unconditionally for a functional one too), not "global"/
    // "unicode" as separate own-property reads -- reading "flags" is what
    // transitively touches every individual flag accessor (including ones
    // this function never otherwise looks at, like "unicode" on a
    // non-global regexp), so an override on any of them must be observed
    // here (`Symbol.replace/get-global-err.js`, `get-unicode-error.js`,
    // `get-flags-err.js`), which reading straight off `regex.flags()`
    // (bypassing property access entirely) could never do.
    let flags_value = i.get_prop(regexp, "flags")?;
    let flags = i.to_js_string(&flags_value)?;
    let global = flags.units().contains(&(b'g' as u16));
    let unicode = flags.units().contains(&(b'u' as u16)) || flags.units().contains(&(b'v' as u16));
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
                    let named = if regex.group_names().is_empty() {
                        JsValue::Undefined
                    } else {
                        regexp_named_captures(i, regex.group_names(), &captures)?
                    };
                    (matched, m.range.start, captures, named)
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
                i,
                &text,
                &m.captures,
                &units,
                range,
                m.named.as_ref(),
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
    i: &mut Interpreter,
    text: &crate::value::JsString,
    captures: &[Option<crate::value::JsString>],
    input: &[u16],
    range: std::ops::Range<usize>,
    named: Option<&JsValue>,
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
            Some(0x3c) if named.is_some() => {
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
                    // Get(namedCaptures, groupName): undefined (missing name,
                    // or a name absent from a custom `exec`'s own `groups`
                    // object) contributes the empty string, not an error.
                    if let Some(named) = named {
                        let value = i.get_prop(named, &name)?;
                        if !value.is_undefined() {
                            out.extend_from_slice(i.to_js_string(&value)?.units());
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
