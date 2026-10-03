use amazing_js::{
    Interpreter,
    ast::{ExprKind, StmtKind},
    interp::ExecutionBackend,
    value::{FunctionBody, JsValue},
};

// Exercise the real native frame and the real pooled-worker consumer independently
// of top-level compilation policy. Only the body representation is replaced.
fn matrix(cases: &[(&str, &str, &str)]) {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            for (setup, factory, drive) in cases {
                let mut i = Interpreter::new();
                i.run_script_with_backend(setup, backend).unwrap();
                i.run_script_with_backend(&format!("var g={factory}"), backend)
                    .unwrap();
                let JsValue::Object(function) = i.run_script("g").unwrap() else {
                    panic!("factory")
                };
                assert!(
                    matches!(
                        function.borrow().func.as_ref().unwrap().body,
                        FunctionBody::Bytecode(_)
                    ),
                    "native factory: {factory}"
                );
                if actor {
                    let parsed = amazing_js::parse_script(factory).unwrap();
                    let StmtKind::Expression(expression) = &parsed.body[0].kind else {
                        panic!("expression")
                    };
                    let ExprKind::Paren(inner) = &expression.kind else {
                        panic!("group")
                    };
                    let ExprKind::Function(item) = &inner.kind else {
                        panic!("function")
                    };
                    function.borrow_mut().func.as_mut().unwrap().body =
                        FunctionBody::Block(item.body.clone());
                }
                let result = i
                    .run_script_with_backend(drive, backend)
                    .and_then(|_| i.drain_microtasks())
                    .and_then(|_| i.run_script("ok"));
                if !matches!(result, Ok(JsValue::Bool(true))) {
                    failures.push(format!(
                        "{backend:?} actor={actor}: {factory}; {drive} -> {result:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn legacy_octal_and_non_octal_decimal_values_keep_their_grammar() {
    matrix(&[
        (
            "var ok=false;",
            "(function(){return 077===63&&0010===8&&000===0&&08===8&&009===9&&078===78})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){return 08.1_2===8.12&&09e1_0===9e10&&0.1_2===0.12&&0e1_0===0})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){return 077.toString()==='63'&&08..toString()==='8'})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn numeric_property_names_use_exact_bigint_and_number_string_keys() {
    matrix(&[
        (
            "var ok=false;",
            "(function(){var o={999999999999999999999999999999999999n:1,0xFFFF_FFFF_FFFF_FFFFn:2,077:3,08:4};return o['999999999999999999999999999999999999']===1&&o['18446744073709551615']===2&&o['63']===3&&o['8']===4})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){var o={get 0x10n(){return 4},set 0b10000n(v){this.value=v},1n(){return 5}};o['16']=7;return o['16']===4&&o.value===7&&o['1']()===5&&o['1'].name==='1'})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){class C{0x10n(){return 4}static 0b10001n(){return 5}18n=6;static 19n=7}var c=new C();return c['16']()===4&&C['17']()===5&&c['18']===6&&C['19']===7})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){var {0x10n:a,999999999999999999999n:b,...rest}={'16':4,'999999999999999999999':5,x:6};var c;({0b10000n:c}={'16':7});return a===4&&b===5&&c===7&&rest.x===6})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn explicit_octal_leading_zero_digits_are_valid_in_strict_contexts() {
    matrix(&[
        (
            "var ok=false;",
            "(function(){'use strict';var o={0o007:7};return 0o0007===7&&0x000F===15&&0b001===1&&o['7']===7})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){class C{0o007(){return 7}static 0o0007=8}return new C()['7']()===7&&C['7']===8})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn decimal_bigint_literals_and_radix_bigints_preserve_arbitrary_precision() {
    matrix(&[
        (
            "var ok=false;",
            "(function(){return 0n===0x0n&&0o0000n===0n&&0b0001n===1n&&1_234_567_890_123_456_789_012_345_678_901n===BigInt('1234567890123456789012345678901')})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){var o={0n:1,1e0:2,0b10:3,0o3:4,0x4:5};return Object.keys(o).join(',')==='0,1,2,3,4'})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn radix_number_literals_round_beyond_u128_without_nan() {
    let hex = format!("0x{}", "f".repeat(40));
    let huge = format!("0x1{}", "0".repeat(300));
    let binary = format!("0b1{}1", "0".repeat(130));
    let factory = format!(
        "(function(){{return {hex}===Number(BigInt('{hex}'))&&{binary}===Number(BigInt('{binary}'))&&{huge}===Infinity&&0x20000000000001===9007199254740992&&0x20000000000003===9007199254740996}})"
    );
    matrix(&[("var ok=false;", &factory, "ok=g()")]);
}
#[test]
fn numeric_early_errors_cover_strict_property_function_and_class_contexts() {
    let bad = [
        "077",
        "08",
        "009.1",
        "08e1",
        "({077:1})",
        "({08(){}})",
        "({get 077(){return 1}})",
        "var {077:x}={}",
    ];
    let mut failures = Vec::new();
    for body in bad {
        for source in [
            format!("'use strict';{body}"),
            format!("function f(){{'use strict';{body}}}"),
            ["class C{m(){", body, "}}"].concat(),
            format!("'use strict'; function f(x=({body})){{}}"),
        ] {
            if amazing_js::parse_script(&source).is_ok() {
                failures.push(source.clone())
            }
        }
        if amazing_js::parse_module(body).is_ok() {
            failures.push(format!("module:{body}"))
        }
    }
    for source in [
        "class C{077(){}}",
        "class C{08=1}",
        "class C{static 077(){}}",
        "class C{get 08(){}}",
    ] {
        if amazing_js::parse_script(source).is_ok() {
            failures.push(source.to_string())
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn numeric_separator_suffix_and_legacy_fraction_boundaries_reject() {
    let bad = [
        "00.1", "077.1", "00e1", "077e1", "00n", "077n", "08n", "0_0", "00_1", "08_1", "0_1n",
        "0x_1", "0o8", "0b2", "0x1_n", "1__0", "1_.0", "1._0", "1e_1", "1e+_1", "1n0", "1nf",
        "0x1n0", "1\\u0061",
    ];
    let failures: Vec<_> = bad
        .iter()
        .filter(|source| amazing_js::parse_script(source).is_ok())
        .collect();
    assert!(failures.is_empty(), "accepted: {failures:?}");
}
#[test]
fn numeric_scanner_retains_actual_legacy_radix_and_raw_digits() {
    use amazing_js::tokens::TokenKind;
    let tokens = amazing_js::lexer::tokenize("077 078 0o007 0x000F 1_000n").unwrap();
    let expected = [
        ("077", 8, false),
        ("078", 10, false),
        ("007", 8, false),
        ("000F", 16, false),
        ("1000", 10, true),
    ];
    for (token, (raw, radix, bigint)) in tokens.iter().zip(expected) {
        let TokenKind::Numeric(n) = &token.kind else {
            panic!("numeric")
        };
        assert_eq!((n.raw.as_str(), n.radix, n.bigint), (raw, radix, bigint));
    }
}
