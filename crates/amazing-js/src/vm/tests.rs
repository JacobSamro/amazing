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
        "changed = 1; class A {}",
        "changed = 1; for (;;) { break; }",
        "changed = 1; while (true) { break; }",
        "changed = 1; let [a] = [2];",
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
fn heap_frame_suspends_and_resumes_at_instruction_boundaries() {
    let code = compile(
        &parse_script(
            "var counter = 0; while (counter < 3) { let local = counter; counter++; } counter;",
        )
        .unwrap(),
    )
    .unwrap();
    let mut interpreter = Interpreter::new();
    let env = interpreter.global_env.clone();
    let mut frame = code.start_frame(&mut interpreter, env, 1000).unwrap();
    let address = &*frame as *const VmFrame;
    let mut suspensions = 0;
    loop {
        match code.resume_frame(&mut interpreter, &mut frame, 3).unwrap() {
            FrameOutcome::Suspended => {
                suspensions += 1;
                assert_eq!(&*frame as *const VmFrame, address);
                assert!(frame.pc > 0);
            }
            FrameOutcome::Completed(value) => {
                assert_eq!(value, JsValue::num(3.0));
                break;
            }
            FrameOutcome::Yielded(_) => panic!("ordinary frame yielded"),
            FrameOutcome::Awaited(_) => panic!("ordinary frame awaited"),
        }
    }
    assert!(suspensions > 1);
    assert!(frame.scopes.is_empty());
    assert!(frame.try_stack.is_empty());
    assert_eq!(
        interpreter.global_env.resolve("counter"),
        Some(JsValue::num(3.0))
    );
    assert!(interpreter.global_env.resolve("local").is_none());
    assert!(matches!(
        code.resume_frame(&mut interpreter, &mut frame, 1).unwrap(),
        FrameOutcome::Completed(JsValue::Num(_))
    ));
}

#[test]
fn try_catch_finally_matches_tree_walker() {
    for source in [
        "var x = 0; try { x = 1; } catch (e) { x = 7; } x",
        "var x = 0; try { throw 3; } catch (e) { x = e + 4; } x",
        "var x = 0; try { throw 3; } catch { x = 4; } x",
        "var x = 0; try { x = 1; } finally { x += 2; } x",
        "var x = 0; try { throw 3; } catch (e) { x = e; } finally { x += 2; } x",
        "try { throw 'a'; } catch (e) { throw e + 'b'; }",
        "try { missing; } catch (e) { 42; }",
        "try { missing *= 1; } catch (e) { e instanceof ReferenceError; }",
        "var side = 0; try { missing *= (side = 1); } catch (e) {} side",
        "try { throw 1; } catch (e) {} typeof e",
        "function f() { try { return 1; } finally { var x = 2; } } f()",
        "function f() { try { return 1; } finally { return 2; } } f()",
        "function f() { try { throw 1; } finally { return 2; } } f()",
        "function f() { try { return 1; } catch (e) { return 2; } } f()",
        "function f() { try { throw 1; } catch (e) { return e + 1; } finally { 3; } } f()",
        "try { try { throw 1; } finally { 2; } } catch (e) { e; }",
        "var x = 0; while (x < 3) { try { x++; if (x === 2) throw 5; } catch (e) { x += e; } finally { x++; } } x",
    ] {
        differential(source, &["x"]);
    }
}

#[test]
fn heap_frame_resumes_through_catch_and_finally() {
    let code = compile(
        &parse_script(
            "var x = 0; try { x = 1; throw 7; } catch (e) { x += e; } finally { x++; } x",
        )
        .unwrap(),
    )
    .unwrap();
    let mut interpreter = Interpreter::new();
    let env = interpreter.global_env.clone();
    let mut frame = code.start_frame(&mut interpreter, env, 1000).unwrap();
    let mut suspended_in_handler = false;
    loop {
        match code.resume_frame(&mut interpreter, &mut frame, 1).unwrap() {
            FrameOutcome::Suspended => suspended_in_handler |= !frame.try_stack.is_empty(),
            FrameOutcome::Completed(value) => {
                assert_eq!(value, JsValue::num(9.0));
                break;
            }
            FrameOutcome::Yielded(_) => panic!("ordinary frame yielded"),
            FrameOutcome::Awaited(_) => panic!("ordinary frame awaited"),
        }
    }
    assert!(suspended_in_handler);
    assert!(frame.try_stack.is_empty());
    assert!(frame.scopes.is_empty());
    assert!(interpreter.global_env.resolve("e").is_none());
}

#[test]
fn generator_resume_and_finally_observe_completion() {
    let mut interpreter = Interpreter::new();
    let source = "
        var log = '';
        function* g() {
            log += 'a';
            try { var input = yield 3; log += input; }
            finally { log += 'f'; yield 8; log += 'z'; }
            return 9;
        }
        var it = g();
        var before = log;
        var first = it.next(99);
        var second = it.next('b');
        var third = it.next();
        var fourth = it.next();
        before + ':' + first.value + ':' + second.value + ':' + third.value + ':' + fourth.done + ':' + log;
    ";
    let result = interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str(":3:8:9:true:abfz"));

    let source = "
        function* g() { try { yield 1; } finally { yield 2; } }
        var it = g(); it.next();
        var a = it.return(7); var b = it.next();
        a.value + ':' + a.done + ':' + b.value + ':' + b.done;
    ";
    let result = Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str("2:false:7:true"));
}

#[test]
fn generator_delegation_forwards_values_and_returns() {
    let source = "
        function* inner() { var v = yield 1; return v + 2; }
        function* outer() { return yield* inner(); }
        var it = outer(); var a = it.next(); var b = it.next(40);
        a.value + ':' + a.done + ':' + b.value + ':' + b.done;
    ";
    let result = Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str("1:false:42:true"));
}

#[test]
fn async_frame_resumes_from_pending_promise() {
    let mut interpreter = Interpreter::new();
    let source = "
        var resolve;
        var pending = new Promise(function(r) { resolve = r; });
        var log = '';
        async function work() { log += 'a'; var value = await pending; log += 'b'; return value + 1; }
        var result = 0;
        var promise = work();
        var before = log;
        promise.then(function(value) { result = value; });
        resolve(41);
        before;
    ";
    let before = interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(before, JsValue::str("a"));
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("ab")));
    assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::num(42.0)));
}

#[test]
fn async_await_rejection_runs_catch_and_finally() {
    let mut interpreter = Interpreter::new();
    let source = "
        var reject;
        var pending = new Promise(function(resolve, fail) { reject = fail; });
        var log = '';
        async function work() {
            try { await pending; return 0; }
            catch (error) { log += error; return 7; }
            finally { log += 'f'; }
        }
        var result = 0;
        work().then(function(value) { result = value; });
        reject('e');
    ";
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("ef")));
    assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::num(7.0)));
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

#[test]
fn shared_calls_and_construction_match_tree_walker() {
    for source in [
        "Number('12') + 3",
        "String(42) + '!'",
        "(function (x) { return x * 2; })(4)",
        "function twice(x) { return x + x; } twice(5)",
        "typeof new Object()",
        "typeof new Array(2)",
    ] {
        differential(source, &[]);
    }

    let mut tree = Interpreter::new();
    let mut vm = Interpreter::new();
    tree.run_script("function prior(x) { return x + 1; }")
        .unwrap();
    vm.run_script("function prior(x) { return x + 1; }")
        .unwrap();
    assert_eq!(
        tree.run_script("prior(3)").unwrap(),
        vm.run_script_with_backend("prior(3)", ExecutionBackend::Bytecode)
            .unwrap()
    );
}

#[test]
fn shared_members_arrays_and_objects_match_tree_walker() {
    for source in [
        "({ a: 1 }).a",
        "var o = { a: 1 }; o.a = 3; o.a",
        "var o = { a: 1 }; o['a'] += 2; o.a",
        "var o = { a: 4, m: function () { return this.a; } }; o.m()",
        "var a = [1, , 3]; a.length",
        "var a = [1, 2]; a[0] + a[1]",
        "var a = [1]; a.push(2); a.length",
        "var s = ''; var o = { [(s = s + 'k')]: (s = s + 'v') }; s",
        "var key = 'x'; ({ [key]: 7 }).x",
        "var o = { 2: 9 }; o[2]",
    ] {
        differential(source, &[]);
    }
}

#[test]
fn long_finite_loop_finishes_with_default_fuel() {
    let source = "var count = 0; for (var index = 0; index < 65536; index++) { count++; } count";
    differential(source, &[]);
}

#[test]
fn deep_binary_expression_rejects_before_execution() {
    let source = format!("changed = 1; {}", vec!["'verse'"; 100].join(" + "));
    let program = parse_script(&source).unwrap();
    assert!(compile(&program).is_err());
    let mut interpreter = Interpreter::new();
    assert!(interpreter
        .run_program_with_backend(&program, ExecutionBackend::Bytecode)
        .is_err());
    assert!(interpreter.global_env.resolve("changed").is_none());
}

/// Explicit evidence tool: emit the first atomic compiler blocker per Test262
/// file. The companion report filters these rows to eligible runtime files.
#[test]
#[ignore = "requires a pinned Test262 checkout and VM_RANK_OUTPUT"]
fn rank_test262_first_blockers() {
    use std::path::{Path, PathBuf};

    fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "js") {
                files.push(path);
            }
        }
    }

    let root = PathBuf::from(std::env::var_os("TEST262_ROOT").expect("TEST262_ROOT"));
    let output = PathBuf::from(std::env::var_os("VM_RANK_OUTPUT").expect("VM_RANK_OUTPUT"));
    let test_root = root.join("test");
    let mut files = Vec::new();
    visit(&test_root, &mut files);
    files.sort();
    let chunk_size = files.len().div_ceil(8);
    let rows = std::thread::scope(|scope| {
        let jobs: Vec<_> = files
            .chunks(chunk_size)
            .enumerate()
            .map(|(worker, chunk)| {
                let test_root = &test_root;
                std::thread::Builder::new()
                    .stack_size(32 * 1024 * 1024)
                    .spawn_scoped(scope, move || {
                        let mut rows = String::new();
                        for (index, path) in chunk.iter().enumerate() {
                            let relative = path.strip_prefix(test_root).unwrap().to_string_lossy();
                            let source = std::fs::read_to_string(path).unwrap();
                            let reason = if source.len() > 100_000 {
                                "large source"
                            } else {
                                match parse_script(&source) {
                                    Ok(ast) => match compile(&ast) {
                                        Ok(_) => "compiled",
                                        Err(error) => error.message,
                                    },
                                    Err(_) => "parse",
                                }
                            };
                            rows.push_str(&relative.replace('\\', "/"));
                            rows.push('\t');
                            rows.push_str(reason);
                            rows.push('\n');
                            if (index + 1) % 1_000 == 0 {
                                eprintln!("ranking worker {worker}: {}/{}", index + 1, chunk.len());
                            }
                        }
                        rows
                    })
                    .unwrap()
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
            .concat()
    });
    std::fs::write(output, rows).unwrap();
}
