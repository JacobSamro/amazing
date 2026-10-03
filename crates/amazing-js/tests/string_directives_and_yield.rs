use amazing_js::{interp::ExecutionBackend, value::JsValue, Interpreter};

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let value = i
            .run_script_with_backend(source, backend)
            .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
        assert_eq!(value, JsValue::Bool(true), "{backend:?}: {source}");
    }
}

#[test]
fn escaped_directives_do_not_enable_strict_but_keep_the_prologue_open() {
    for directive in [
        r#"'use\x20strict'"#,
        r#"'use\u0020strict'"#,
        "'use \\\nstrict'",
        "'use \\\r\nstrict'",
    ] {
        check_both(&format!("function f(){{{directive};return this===globalThis}};f.call(undefined)&&Function({directive:?}+';return this===globalThis').call(undefined);"));
        check_both(&format!(
            "function f(){{{directive};'use strict';return this===undefined}};f.call(undefined);"
        ));
        assert!(amazing_js::parser::parse_script(&format!(
            "function f(){{{directive};'use strict';with({{}}){{}}}}"
        ))
        .is_err());
    }
}

#[test]
fn octal_and_decimal_escapes_consume_the_specified_number_of_digits() {
    check_both(
        r#"var cases=[['\0',0],['\00',0],['\07',7],['\377',255],['\400',32],['\777',63],['\8',56],['\9',57],['\08',0]];cases.every(function(p){return p[0].charCodeAt(0)===p[1]})&&'\400'===' 0'&&'\777'==='?7'&&'\08'==='\x008';"#,
    );
    for escape in ["\\00", "\\07", "\\377", "\\8", "\\9", "\\08"] {
        for prefix in [
            "'use strict';",
            "class C { m(){",
            "function f(){'use strict';",
        ] {
            let suffix = if prefix.starts_with("class") {
                "}}"
            } else if prefix.starts_with("function") {
                "}"
            } else {
                ""
            };
            assert!(
                amazing_js::parser::parse_script(&format!("{prefix}'{escape}';{suffix}")).is_err()
            );
        }
    }
}

#[test]
fn all_line_continuations_have_empty_cooked_content() {
    for terminator in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        check_both(&format!(
            "'a\\{terminator}b'==='ab'&&\"a\\{terminator}b\"==='ab'&&`a\\{terminator}b`==='ab';"
        ));
    }
    check_both(
        r#"var raw,cooked;function tag(s){raw=s.raw[0];cooked=s[0]};tag`\07`;cooked===undefined&&raw==='\\07'&&'\uD800'.charCodeAt(0)===0xD800;"#,
    );
}

#[test]
fn delegated_yield_allows_line_terminators_after_the_star() {
    for separator in ["\n", "\r\n", "\u{2028}", "\u{2029}", "/*\n*/"] {
        for function in [
            format!("function* g(){{yield*{separator}[1,2]}}"),
            format!("var g=function*(){{yield*{separator}[1,2]}}"),
            format!("var o={{*g(){{yield*{separator}[1,2]}}}};var g=()=>o.g()"),
            format!("class C{{*g(){{yield*{separator}[1,2]}}}};var g=()=>new C().g()"),
        ] {
            check_both(&format!(
                "{function};var it=g();it.next().value===1&&it.next().value===2&&it.next().done;"
            ));
        }
        assert!(amazing_js::parser::parse_script(&format!(
            "function* g(){{yield{separator}*[1]}}"
        ))
        .is_err());
    }
    for source in [
        "function* g(){yield*;}",
        "function* g(){yield*}",
        "async function* g(){yield*;}",
    ] {
        assert!(amazing_js::parser::parse_script(source).is_err());
    }
    check_both("function* g(){yield\n1;}var it=g();it.next().value===undefined&&it.next().done;");
}

#[test]
fn escaped_directives_keep_non_simple_parameters_sloppy() {
    check_both(
        r#"var f=(a=1)=>{'use\x20strict';return a};function g(a=2){'use\u0020strict';return a};var o={m(a=3){'use\x20strict';return a}};f()===1&&g()===2&&o.m()===3;"#,
    );
    assert!(
        amazing_js::parser::parse_script("function f(a=1){'use\\x20strict';'use strict';}")
            .is_err()
    );
}
