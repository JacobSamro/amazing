use amazing_js::{ast::{ExprKind, StmtKind}, interp::ExecutionBackend,
    parser::{parse_eval_utf16, EvalContext}, parse_module_utf16, parse_script, parse_script_utf16,
    value::{FunctionBody, JsValue}, Interpreter};

fn source(prefix: &str, units: &[u16], suffix: &str) -> Vec<u16> {
    prefix.encode_utf16().chain(units.iter().copied()).chain(suffix.encode_utf16()).collect()
}

/// Execute the actual parsed units through both entry drivers and the actual
/// native factory, or replace only its body with the original parsed AST.
fn matrix(body: &[u16], drive: &str) {
    let program = parse_script_utf16(&source("function make(){", body, "}")).unwrap();
    let StmtKind::Function(item) = &program.body[0].kind else { panic!("function source") };
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
            let mut i = Interpreter::new();
            i.run_program_with_backend(&program, backend).unwrap();
            let JsValue::Object(function) = i.run_script("make").unwrap() else { panic!("callable") };
            assert!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)),
                "actual native factory for {body:?}");
            if actor { function.borrow_mut().func.as_mut().unwrap().body = FunctionBody::Block(item.body.clone()); }
            let result = i.run_script_with_backend(drive, backend);
            if !matches!(result, Ok(JsValue::Bool(true))) { failures.push(format!("{backend:?} actor={actor} {body:?}: {result:?}")); }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn literal_units_distinguish_surrogates_replacement_and_private_use_code_points() {
    let units = [0, 0xd800, 65, 0xdc00, 0xd83d, 0xde00, 0xfffd, 0xe000, 0xf8ff, 0xdb80, 0xdc00];
    let expected = format!("[{}]", units.iter().map(u16::to_string).collect::<Vec<_>>().join(","));
    matrix(&source("return '", &units, "'"), &format!("var r=make(),expected={expected};r.length===expected.length&&expected.every((u,n)=>r.charCodeAt(n)===u)"));
    for unit in [0xd800, 0xdbff, 0xdc00, 0xdfff] {
        matrix(&source("return '\\", &[unit], "'"), &format!("make().length===1&&make().charCodeAt(0)==={unit}"));
    }
}

#[test]
fn escaped_and_literal_units_are_equivalent_without_rewriting_the_source() {
    for units in [vec![0xd800], vec![0xdc00], vec![0xd800,65,0xdc00], vec![0xd83d,0xde00]] {
        let escaped = units.iter().map(|u|format!("\\u{u:04X}")).collect::<String>();
        let body = source("return '", &units, &format!("'==='{}'", escaped));
        matrix(&body, "make()===true");
    }
    let ordinary = "var x='\u{fffd}\u{e000}😀';x;";
    assert_eq!(parse_script(ordinary).unwrap(), parse_script_utf16(&ordinary.encode_utf16().collect::<Vec<_>>()).unwrap());
}

#[test]
fn template_raw_and_cooked_units_normalize_only_literal_line_terminators() {
    let units = [0xd800,13,10,0xdc00,13,65,0xfffd,0xe000];
    matrix(&source("return ((parts)=>[parts[0],parts.raw[0]])`", &units, "`"),
        "var r=make(),expected=[0xd800,10,0xdc00,10,65,0xfffd,0xe000];r.every(s=>s.length===expected.length&&expected.every((u,n)=>s.charCodeAt(n)===u))");
    matrix(&source("return ((p)=>[p[0],p.raw[0]])`\\uD800", &[0xdc00], "\\u{110000}`"),
        "var r=make();r[0]===undefined&&r[1].slice(0,6)==='\\\\uD800'&&r[1].charCodeAt(6)===0xdc00&&r[1].slice(7)==='\\\\u{110000}'");
    matrix(&source("return `", &[0xd800,13,10,0xdc00], "${'A'}\\uD800`"),
        "var r=make();r.length===5&&r.charCodeAt(0)===0xd800&&r.charCodeAt(1)===10&&r.charCodeAt(2)===0xdc00&&r[3]==='A'&&r.charCodeAt(4)===0xd800");
}

#[test]
fn regex_goal_rewind_compiles_exact_raw_and_escaped_pattern_units() {
    for unit in [0xd800,0xdbff,0xdc00,0xdfff,0xfffd,0xe000] {
        for prefix in ["return /", "return (true ? /", "var a=1; a/=1;return /"] {
            let suffix = if prefix.contains('?') { "/ : /a/).source" } else { "/.source" };
            matrix(&source(prefix,&[unit],suffix),&format!("var r=make();r.length===1&&r.charCodeAt(0)==={unit}"));
        }
        matrix(&source("return /a\\",&[unit],"/.source"),&format!("var r=make();r.length===3&&r[0]==='a'&&r[1]==='\\\\'&&r.charCodeAt(2)==={unit}"));
    }
    matrix(&source("return /", &[0xd83d,0xde00], "/u.test('\\u{1F600}')"), "make()===true");
    matrix(&source("return /[", &[0xd800], "]/.test('\\uD800')"), "make()===true");
    for units in [vec![0xd800], vec![0xdbff], vec![0xdc00], vec![0xdfff],
        vec![0xd83d,0xde00], vec![0xdbff,0xdfff]] {
        for flags in ["", "u", "v"] {
            for (prefix,suffix) in [("/",format!("/{flags}")),("/[",format!("]/{flags}"))] {
                assert!(parse_script_utf16(&source(prefix,&units,&suffix)).is_ok());
                let escaped = format!("{prefix}\\");
                assert_eq!(parse_script_utf16(&source(&escaped,&units,&suffix)).is_ok(),flags.is_empty(),
                    "raw identity escape units {units:?}, flags {flags}");
            }
        }
    }
}

#[test]
fn comments_hashbang_and_asi_accept_real_source_characters_and_keep_lines() {
    for prefix in ["#!", "//", "/*"] {
        let suffix = if prefix=="/*" { "*/\r\n41+1" } else { "\r\n41+1" };
        let p=parse_script_utf16(&source(prefix,&[0xd800,0xfffd,0xe000,0xdc00],suffix)).unwrap();
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            assert_eq!(Interpreter::new().run_program_with_backend(&p,backend).unwrap(),JsValue::Num(42.0));
        }
        assert_eq!(p.body[0].span.start.line,2);
        assert_eq!(p.body[0].span.start.column,1);
    }
    for line in ["\r","\r\n","\n","\u{2028}","\u{2029}"] {
        matrix(&format!("return{line}'unreachable'").encode_utf16().collect::<Vec<_>>(), "make()===undefined");
        let p=parse_script_utf16(&format!("/*😀*/{line}1").encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(p.body[0].span.start.line,2);
        assert_eq!(p.body[0].span.start.column,1);
    }
}

#[test]
fn string_line_continuations_differ_from_raw_terminators_and_template_normalization() {
    for line in ["\r","\r\n","\n","\u{2028}","\u{2029}"] {
        matrix(&source("return 'A\\",&line.encode_utf16().collect::<Vec<_>>(),"B'"),"make()==='AB'");
    }
    for line in ["\r","\r\n","\n"] {
        assert!(parse_script_utf16(&source("'A",&line.encode_utf16().collect::<Vec<_>>(),"B'")).is_err());
    }
    for line in ["\u{2028}","\u{2029}"] {
        matrix(&source("return 'A",&line.encode_utf16().collect::<Vec<_>>(),"B'"),"make().length===3");
    }
}

#[test]
fn non_scalar_grammar_characters_reject_without_eof_or_identifier_substitution() {
    for unit in [0xd800,0xdbff,0xdc00,0xdfff] {
        for (prefix,suffix) in [("",""),("var ","=1"),("var a","=1"),("class C{#",";}") ,("/a/","")] {
            assert!(parse_script_utf16(&source(prefix,&[unit],suffix)).is_err(),"{prefix} {unit:x} {suffix}");
        }
        assert!(parse_script_utf16(&source("/*",&[unit],"")).is_err(),"unterminated comment is not EOF success");
        assert!(parse_script_utf16(&source("'",&[unit],"")).is_err());
        assert!(parse_script_utf16(&source("`",&[unit],"")).is_err());
        assert!(parse_script_utf16(&source("/",&[unit],"")).is_err());
    }
    for source in ["var \\uD800=1","var a\\uDC00=1","class C{#\\uD800;}"] {
        assert!(parse_script_utf16(&source.encode_utf16().collect::<Vec<_>>()).is_err());
    }
}

#[test]
fn module_and_eval_goals_keep_strictness_private_names_and_lexical_permissions() {
    let raw=source("'",&[0xd800],"';new.target;super.x;this.#value");
    let context=EvalContext{allow_new_target:true,allow_super_property:true,visible_private_names:vec!["#value".into()],..EvalContext::default()};
    assert!(parse_eval_utf16(&raw,&context).is_ok());
    assert!(parse_eval_utf16(&raw,&EvalContext::default()).is_err());
    let strict=source("export const value='",&[0xdc00],"';");
    assert!(parse_module_utf16(&strict).is_ok());
    assert!(parse_script_utf16(&strict).is_err());
    assert!(parse_module_utf16(&source("'",&[0xd800],"';010")).is_err());
    assert!(parse_eval_utf16(&source("'use strict';'",&[0xd800],"';010"),&EvalContext::default()).is_err());
    for code in ["return 1","yield 1","await 1"] {assert!(parse_eval_utf16(&code.encode_utf16().collect::<Vec<_>>(),&context).is_err());}
}

#[test]
fn cloned_template_sites_keep_identity_while_fresh_parses_and_realms_are_distinct() {
    let script=source("function make(){return (p=>p)`",&[0xd800,0xfffd,0xe000],"`}");
    let first=parse_script_utf16(&script).unwrap();let cloned=first.clone();let fresh=parse_script_utf16(&script).unwrap();
    assert_eq!(first,fresh,"semantic AST equality ignores site identity");
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=Interpreter::new();
        i.run_program_with_backend(&first,backend).unwrap();i.run_script("var first=make()").unwrap();
        i.run_program_with_backend(&cloned,backend).unwrap();assert_eq!(i.run_script("make()===first").unwrap(),JsValue::Bool(true));
        i.run_program_with_backend(&fresh,backend).unwrap();assert_eq!(i.run_script("make()!==first").unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script("Object.isFrozen(first)&&Object.isFrozen(first.raw)&&first[0].charCodeAt(0)===0xd800&&first.raw[0].charCodeAt(0)===0xd800").unwrap(),JsValue::Bool(true));
    }
    let p=parse_script_utf16(&source("'",&[0xd800],"'")).unwrap();
    let StmtKind::Expression(expression)=&p.body[0].kind else{panic!("literal")};
    assert!(matches!(expression.kind,ExprKind::Str(_)));assert_eq!(expression.string_utf16,Some(vec![0xd800]));
}

#[test]
fn coroutine_workers_keep_exact_literal_and_template_site_units() {
    for (kind, body, drive) in [
        ("function*", "yield ((p)=>p)`", "var a=make(),b=make(),x=a.next().value,y=b.next().value;ok=x===y&&x[0].charCodeAt(0)===0xd800&&x.raw[0].charCodeAt(0)===0xd800"),
        ("async function", "await Promise.resolve();return ((p)=>p)`", "Promise.all([make(),make()]).then(r=>ok=r[0]===r[1]&&r[0][0].charCodeAt(0)===0xd800)"),
        ("async function*", "await Promise.resolve();yield ((p)=>p)`", "Promise.all([make().next(),make().next()]).then(r=>ok=r[0].value===r[1].value&&r[0].value[0].charCodeAt(0)===0xd800)")
    ] {
        let input=source(&format!("{kind} make(){{{body}"), &[0xd800], "`}");
        let program=parse_script_utf16(&input).unwrap();
        let StmtKind::Function(item)=&program.body[0].kind else{panic!("function")};
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            for actor in [false,true] {
                let mut i=Interpreter::new();i.run_script("var ok=false").unwrap();
                i.run_program_with_backend(&program,backend).unwrap();
                let JsValue::Object(function)=i.run_script("make").unwrap() else{panic!("function")};
                assert!(matches!(function.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)));
                if actor {function.borrow_mut().func.as_mut().unwrap().body=FunctionBody::Block(item.body.clone());}
                i.run_script_with_backend(drive,backend).unwrap();i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("ok").unwrap(),JsValue::Bool(true),"{kind} {backend:?} actor={actor}");
            }
        }
    }
}

#[test]
fn same_parsed_utf16_template_has_distinct_actual_realm_objects() {
    let program=parse_script_utf16(&source("function make(){return (p=>p)`",&[0xd800,0xdc00,0xfffd,0xe000],"`}")).unwrap();
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=Interpreter::new();i.run_program_with_backend(&program,backend).unwrap();
        let parent=i.run_script("make()").unwrap();
        let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let other=i.binding_with_realm(child,|i|{i.run_program_with_backend(&program,backend)?;i.run_script("make()")}).unwrap();
        assert_ne!(parent,other,"realm cache identity");
        let again=i.binding_with_realm(child,|i|{i.run_program_with_backend(&program.clone(),backend)?;i.run_script("make()")}).unwrap();
        assert_eq!(again,other,"same source/site and child realm identity");
        let global=i.binding_global_object();i.binding_set_property(&global,"foreign",other).unwrap();
        assert_eq!(i.run_script("foreign[0].charCodeAt(0)===0xd800&&foreign[0].charCodeAt(1)===0xdc00&&foreign[0].charCodeAt(2)===0xfffd&&foreign[0].charCodeAt(3)===0xe000&&Object.isFrozen(foreign)&&Object.isFrozen(foreign.raw)&&Object.getPrototypeOf(foreign)!==Array.prototype").unwrap(),JsValue::Bool(true));
    }
}
