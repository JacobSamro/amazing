use super::*;

fn code(source: &str) -> FunctionCode {
    let parsed = crate::parse_script(source).unwrap();
    let crate::ast::StmtKind::Function(function) = &parsed.body[0].kind else {
        panic!("declaration")
    };
    compiler::compile_function_item(function, function.body.span, false).unwrap()
}

#[test]
fn native_tail_opcode_eligibility_covers_entire_expression_and_statement_boundary() {
    for body in [
        "return g()",
        "return (g())",
        "return (0,g())",
        "return true?g():g()",
        "return true&&g()",
        "return false||g()",
        "return null??g()",
        "return g?.()",
        "return o?.g()",
        "return g`x`",
        "try{}catch(e){return g()}",
        "try{}finally{return g()}",
        "while(true)return g()",
        "for(var x in {})return g()",
    ] {
        let code = code(&format!("function f(){{'use strict';{body}}}"));
        assert!(
            code.0
                .program
                .code
                .iter()
                .any(|i| matches!(i, Instruction::TailCall(..))),
            "{body}"
        );
        assert!(
            !code
                .0
                .program
                .code
                .iter()
                .any(|i| matches!(i, Instruction::TreeEval(..))),
            "{body}"
        );
    }
    for body in [
        "return 1+g()",
        "return (g(),0)",
        "return g()&&0",
        "return g()||0",
        "try{return g()}catch(e){}",
        "try{return g()}finally{}",
        "try{}catch(e){return g()}finally{}",
        "for(var x of [])return g()",
        "using x=null;return g()",
        "for(using x=null;;)return g()",
        "switch(1){case 1:{using x=null;return g()}}",
    ] {
        let code = code(&format!("function f(){{'use strict';{body}}}"));
        assert!(
            !code
                .0
                .program
                .code
                .iter()
                .any(|i| matches!(i, Instruction::TailCall(..))),
            "{body}"
        );
    }
    for body in [
        "switch(1){case 1:using x=null;return g()}",
        "switch(1){default:using x=null;return g()}",
    ] {
        assert!(
            crate::parse_script(&format!("function f(){{'use strict';{body}}}")).is_err(),
            "{body}"
        );
    }
    for prefix in ["function*", "async function", "async function*"] {
        let code = code(&format!("{prefix} f(){{'use strict';return g()}}"));
        assert!(!code
            .0
            .program
            .code
            .iter()
            .any(|i| matches!(i, Instruction::TailCall(..))));
    }
    assert!(!code("function f(){return g()}")
        .0
        .program
        .code
        .iter()
        .any(|i| matches!(i, Instruction::TailCall(..))));
}

#[test]
fn quantum_tail_outcome_retains_only_captured_values_and_restores_caller_context() {
    let code = code("function f(){'use strict';return target(argument())}");
    let mut i = Interpreter::new();
    i.run_script(
        "var count=0;function argument(){count++;return {x:7}}function target(v){return v.x}",
    )
    .unwrap();
    let env = i.global_env.clone();
    let original_frames = i.frames.len();
    let mut frame = code
        .0
        .program
        .start_function_frame(&mut i, env, 1000, true)
        .unwrap();
    let record = loop {
        match code.0.program.resume_frame(&mut i, &mut frame, 1).unwrap() {
            FrameOutcome::Suspended => assert_eq!(i.frames.len(), original_frames),
            FrameOutcome::Tail(record) => break record,
            _ => panic!("native tail opcode did not return its owned outcome"),
        }
    };
    drop(frame);
    assert_eq!(i.frames.len(), original_frames);
    assert_eq!(i.run_script("count").unwrap(), JsValue::num(1.0));
    let mut slots = 0;
    let mut environments = 0;
    record.visit_strong_slots(|_| slots += 1, |_| environments += 1);
    assert_eq!((slots, environments), (3, 0));
    assert_eq!(
        i.finish_detached_tail_call(record).unwrap(),
        JsValue::num(7.0)
    );
}

#[test]
fn direct_eval_tail_instruction_executes_in_original_lexical_context_without_transfer() {
    let code = code("function f(){'use strict';let x=7;return eval('x')}");
    let mut i = Interpreter::new();
    let parent = i.global_env.clone();
    assert_eq!(
        code.call(
            &mut i,
            parent,
            false,
            JsValue::Undefined,
            Vec::new(),
            None,
            None,
            None
        )
        .unwrap(),
        JsValue::num(7.0)
    );
}
