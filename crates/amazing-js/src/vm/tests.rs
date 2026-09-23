use super::*;
use crate::interp::ExecutionBackend;
use crate::parse_script;

fn same_value(left: &JsValue, right: &JsValue) -> bool {
    match (left, right) {
        (JsValue::Num(a), JsValue::Num(b)) => {
            (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
        }
        _ => left == right,
    }
}

fn differential(source: &str, names: &[&str]) {
    let mut tree = Interpreter::new();
    let mut vm = Interpreter::new();
    let expected = tree.run_script(source);
    let actual = vm.run_script_with_backend(source, ExecutionBackend::Bytecode);
    match (expected, actual) {
        (Ok(a), Ok(b)) => assert!(same_value(&a, &b), "{source}: {a:?} != {b:?}"),
        (Err(a), Err(b)) => assert_eq!(a.describe(), b.describe(), "{source}"),
        (a, b) => panic!("{source}: {a:?} != {b:?}"),
    }
    for name in names {
        let a = tree.global_env.resolve(name);
        let b = vm.global_env.resolve(name);
        match (a, b) {
            (Some(a), Some(b)) => assert!(same_value(&a, &b), "{source}: binding {name}"),
            (None, None) => {}
            (a, b) => panic!("{source}: binding {name}: {a:?} != {b:?}"),
        }
    }
}

#[test]
fn primitive_and_operator_differential_corpus() {
    for source in [
        "",
        "; debugger;",
        "this",
        "null",
        "true",
        "'hello' + ' world'",
        "1 + 2 * 3 ** 2",
        "0xff + 0b11 + 0o7",
        "1_000 + 1",
        "-0",
        "0 / 0",
        "1 / 0",
        "-1 / 0",
        "'3' + 2",
        "'3' - 2",
        "!0",
        "~12",
        "+'42'",
        "void 5",
        "typeof absent",
        "typeof (absent)",
        "typeof ((absent))",
        "typeof null",
        "typeof 3",
        "typeof this",
        "'10' < '2'",
        "'10' > 2",
        "2 <= 2",
        "4 >= 2",
        "null == undefined",
        "1 != '1'",
        "1 === '1'",
        "1 !== '1'",
        "8 << 2",
        "-8 >> 1",
        "-8 >>> 1",
        "15 & 33",
        "15 | 33",
        "15 ^ 33",
        "5 % 2",
        "(1, 2, 3)",
        "false ? missing : 42",
        "true ? 42 : missing",
        "0 && missing",
        "'yes' || missing",
        "null ?? 7",
        "0 ?? missing",
        "undefined ?? 9",
        "'x' in 1",
        "1 instanceof 2",
        "missing",
    ] {
        differential(source, &[]);
    }
}

#[test]
fn bindings_branches_loops_and_completion_match() {
    for source in [
        "var sum = 0; var i = 0; while (i < 100) { sum = sum + i; i++; } sum;",
        "let x = 5; { let x = 9; x++; } x;",
        "const x = 5; { const x = 6; x; } x;",
        "let x = 0; if (true) { x = 4; } else { x = 8; } x;",
        "var x = 5; if (false) { var y = 3; } y;",
        "var x = 1; var x; x;",
        "x; var x = 7; x;",
        "let x = 2; x = (x = 3) + x; x;",
        "var x = '2'; var y = x++; y + ':' + x;",
        "var x = 3; var y = --x; y + x;",
        "var x = 0; false && (x = 1); true || (x = 2); 0 ?? (x = 3); x;",
        "var x = 0; true && (x = 1); false || (x = 2); null ?? (x = 3); x;",
        "var x = 4; x += 3; x *= 2; x;",
        "var x = 0; x ||= 7; x &&= 5; x ??= 9; x;",
        "let x = 0; var y = x++ + ++x; y;",
        "var x = 4; while (x > 0) { let y = x; x--; } x;",
        "var x = 0; do { x++; } while (x < 3); x;",
        "var sum = 0; for (var j = 0; j < 4; j++) { sum = sum + j; } sum;",
        "x = 12; x;",
        "1; var x;",
        "1; {}",
        "1; if (false) 2;",
        "1; while (false) 2;",
        "1; while (true) { throw 7; }",
        "const x = 1; x = 2;",
        "let x = 1; let x = 2;",
        "const x = 1; var x = 2;",
        "throw 'oops'",
    ] {
        differential(source, &["x", "y", "sum", "i"]);
    }
}

#[test]
fn unsupported_program_is_rejected_before_hoisting_or_side_effects() {
    for source in [
        "var changed = 1; function f() { [1]; }",
        "changed = 1; if (false) { print('no'); }",
        "changed = 1; [1, 2];",
        "changed = 1; ({a: 2});",
        "changed = 1; for (;;) { break; }",
        "changed = 1; while (true) { break; }",
        "changed = 1; try {} catch (e) {}",
        "changed = 1; let [a] = [2];",
        "changed = 1; changed += [2];",
        "changed = 1; delete changed;",
        "changed = 1; 2n;",
    ] {
        let mut interpreter = Interpreter::new();
        let result = interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode);
        assert!(result.is_err(), "{source}");
        assert!(
            interpreter.global_env.resolve("changed").is_none(),
            "{source}"
        );
        assert!(interpreter.output.borrow().is_empty());
    }
}

#[test]
fn tree_walker_is_still_default_and_explicitly_selectable() {
    assert_eq!(ExecutionBackend::default(), ExecutionBackend::TreeWalk);
    let source = "function twice(x) { return x * 2; } twice(21);";
    assert_eq!(
        Interpreter::new().run_script(source).unwrap(),
        JsValue::num(42.0)
    );
    assert_eq!(
        Interpreter::new()
            .run_script_with_backend(source, ExecutionBackend::TreeWalk)
            .unwrap(),
        JsValue::num(42.0)
    );
    assert_eq!(
        Interpreter::new()
            .run_script_with_backend(source, ExecutionBackend::Bytecode)
            .unwrap(),
        JsValue::num(42.0)
    );
}

#[test]
fn bytecode_functions_and_closures() {
    for source in [
        "function twice(x) { return x * 2; } twice(21);",
        "function outer(x) { function inner(y) { return x + y; } return inner; } outer(19)(23);",
        "function outer(x) { return function(y) { return x + y; }; } outer(20)(22);",
        "function f() { var x = 1; x++; return x; } f();",
        "function f() { return; } f();",
        "((x) => x + 1)(41);",
        "((x) => ((y) => x + y))(20)(22);",
    ] {
        differential(source, &[]);
    }
}

#[test]
fn compiled_program_can_outlive_ast_and_be_reused() {
    let code = compile(&parse_script("counter = counter + 1; counter;").unwrap()).unwrap();
    assert!(code.instruction_count() > 1);
    let mut interpreter = Interpreter::new();
    interpreter.run_script("var counter = 40;").unwrap();
    assert_eq!(code.run(&mut interpreter, 100).unwrap(), JsValue::num(41.0));
    assert_eq!(code.run(&mut interpreter, 100).unwrap(), JsValue::num(42.0));
}

#[test]
fn fuel_stops_backedges_and_vm_state_is_clean_after_error() {
    let mut interpreter = Interpreter::new();
    interpreter.completion = JsValue::str("saved");
    let code =
        compile(&parse_script("let x = 0; while (true) { let local = 3; x++; }").unwrap()).unwrap();
    let error = code.run(&mut interpreter, 100).unwrap_err();
    assert!(error.describe().contains("instruction limit"));
    assert_eq!(interpreter.completion, JsValue::str("saved"));
    assert!(interpreter.global_env.resolve("local").is_none());
    let result = interpreter
        .run_script_with_backend("2 + 3", ExecutionBackend::Bytecode)
        .unwrap();
    assert_eq!(result, JsValue::num(5.0));
    assert_eq!(interpreter.completion, JsValue::str("saved"));
}

#[test]
fn zero_budget_does_not_mutate_bindings() {
    let code = compile(&parse_script("var x = 1;").unwrap()).unwrap();
    let mut interpreter = Interpreter::new();
    assert!(code.run(&mut interpreter, 0).is_err());
    assert!(interpreter.global_env.resolve("x").is_none());
}

#[test]
fn global_object_reference_is_resolved_before_rhs_changes_the_binding() {
    // A RHS assignment to a global property cannot retarget the saved LHS.
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.global_obj
            .borrow_mut()
            .set_own("external", JsValue::num(4.0));
        let result = i
            .run_script_with_backend("external = (external = 6) + external; external;", backend)
            .unwrap();
        assert_eq!(result, JsValue::num(12.0));
        assert!(i.global_env.resolve("external").is_none());
    }
}

#[test]
fn compiler_limits_deep_manually_constructed_ast() {
    use crate::ast::{Expr, ExprKind, Stmt, StmtKind};
    let mut ast = parse_script("1;").unwrap();
    let mut expression = match ast.body.remove(0).kind {
        StmtKind::Expression(expression) => expression,
        _ => unreachable!(),
    };
    for _ in 0..300 {
        expression = Expr::new(ExprKind::Paren(Box::new(expression)), ast.span);
    }
    ast.body
        .push(Stmt::new(StmtKind::Expression(expression), ast.span));
    assert!(compile(&ast).unwrap_err().message.contains("limit"));
}

#[test]
fn assignment_preserves_legacy_function_name_inference() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let anonymous = i.run_script("(function () {});").unwrap();
        i.global_env.declare_var("anonymous", anonymous).unwrap();
        i.run_script_with_backend("named = anonymous;", backend)
            .unwrap();
        assert_eq!(i.run_script("named.name;").unwrap(), JsValue::str("named"));
    }
}

#[test]
fn shared_coercion_preserves_callback_order_and_exceptions() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.run_script("var calls = ''; var a = { valueOf: function () { calls = calls + 'a'; return 2; } }; var b = { valueOf: function () { calls = calls + 'b'; return 3; } };").unwrap();
        assert_eq!(
            i.run_script_with_backend("a + b;", backend).unwrap(),
            JsValue::num(5.0)
        );
        assert_eq!(i.global_env.resolve("calls"), Some(JsValue::str("ab")));
        i.run_script("a.valueOf = function () { throw 'coercion'; };")
            .unwrap();
        assert_eq!(
            i.run_script_with_backend("a + b;", backend)
                .unwrap_err()
                .describe(),
            "coercion"
        );
    }
}
