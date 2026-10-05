use super::*;

fn literal_body(i: &mut Interpreter, name: &str, ast: bool) {
    let JsValue::Object(f)=i.run_script(name).unwrap() else {panic!("function")};
    let f=f.borrow();
    match &f.func.as_ref().unwrap().body {
        FunctionBody::Bytecode(code) if !ast=>{
            assert!(!code.0.program.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))),"literal delegates: {name}");
            assert!(code.0.program.code.iter().any(|op|matches!(op,Instruction::Constant(_,JsValue::BigInt(_)))),"missing BigInt constant: {name}");
        },
        FunctionBody::Block(_) if ast=>{},
        body=>panic!("unexpected AST={ast}: {body:?}"),
    }
}

fn drive(kind: &str) -> &'static str {
    match kind {
        "ordinary"=>"try{out=home.m();done=true}catch(e){error=e;done=true}",
        "generator"=>"try{var r=home.m().next();out=r.value;done=r.done}catch(e){error=e;done=true}",
        "async"=>"home.m().then(v=>{out=v;done=true},e=>{error=e;done=true})",
        _=>"home.m().next().then(r=>{out=r.value;done=r.done},e=>{error=e;done=true})",
    }
}

#[test]
fn bigint_literal_whole_scalar_matrix_is_native_in_all_callable_kinds() {
    let cases=[
        ("0n","out.toString()==='0'",""),("1n","out.toString()==='1'",""),
        ("0xFFn","out.toString()==='255'",""),("0Xffn","out.toString()==='255'",""),
        ("0o777n","out.toString()==='511'",""),("0O10n","out.toString()==='8'",""),
        ("0b101010n","out.toString()==='42'",""),("0B1111n","out.toString()==='15'",""),
        ("1_000_000n","out.toString()==='1000000'",""),("0xAB_CD_EFn","out.toString()==='11259375'",""),
        ("0o7_7_7n","out.toString()==='511'",""),("0b1_0_1_0n","out.toString()==='10'",""),
        ("-0n","out.toString()==='0'",""),("-123n","out.toString()==='-123'",""),
        ("~0n","out.toString()==='-1'",""),("!0n","out===true",""),
        ("typeof 1n","out==='bigint'",""),("1n+2n","out.toString()==='3'",""),
        ("9n-12n","out.toString()==='-3'",""),("7n*6n","out.toString()==='42'",""),
        ("-7n/2n","out.toString()==='-3'",""),("-7n%2n","out.toString()==='-1'",""),
        ("2n**80n","out.toString()==='1208925819614629174706176'",""),
        ("1n<<65n","out.toString()==='36893488147419103232'",""),
        ("-8n>>2n","out.toString()==='-2'",""),("0xFFn&0xAAn","out.toString()==='170'",""),
        ("5n|2n","out.toString()==='7'",""),("5n^3n","out.toString()==='6'",""),
        ("0n||9n","out.toString()==='9'",""),("1n&&7n","out.toString()==='7'",""),
        ("0n??9n","out.toString()==='0'",""),("true?23n:5n","out.toString()==='23'",""),
        ("9007199254740993n===9007199254740993n","out===true",""),
        ("9007199254740993n>9007199254740992","out===true",""),
        ("23n==23","out===true",""),("23n===23","out===false",""),
        ("+1n","true","type"),("1n+1","true","type"),("1n/0n","true","range"),
        ("2n**-1n","true","range"),("1n>>>1n","true","type"),
        ("({[23n]:7})[23n]","out===7",""),("[1n,2n,3n][1]","out.toString()==='2'",""),
        ("`${123n}`","out==='123'",""),
        ("((x)=>x+1n)(7n)","out.toString()==='8'",""),
        ("(1n,2n,3n)","out.toString()==='3'",""),
        ("{...{[23n]:19}}[23n]","out===19",""),
    ];
    let mut failures=Vec::new();
    for ast in [false,true] {for kind in ["ordinary","generator","async","async-generator"] {for (expression,check,error_kind) in cases {
        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
            let mut i=Interpreter::new();i.force_ast_callables=ast;
            i.run_script(&format!("var out,done=false,error;var home={{{prefix}{star}m(){{return {expression}}}}};",prefix=if matches!(kind,"ordinary"|"generator"){""}else{"async "},star=if matches!(kind,"generator"|"async-generator"){"*"}else{""})).unwrap();
            literal_body(&mut i,"home.m",ast);i.run_script(drive(kind)).unwrap();i.drain_microtasks().unwrap();
            let error_check=match error_kind {"type"=>"error instanceof TypeError","range"=>"error instanceof RangeError",_=>"error===undefined"};
            assert_eq!(i.run_script(&format!("done&&({error_check})&&({check})")).unwrap(),JsValue::Bool(true),"{:?}",i.run_script("JSON.stringify([done,String(error),String(out)])").unwrap());
        }));if result.is_err(){failures.push(format!("AST={ast} {kind} {expression}"));}
    }}}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn bigint_literals_keep_arbitrary_precision_without_number_roundtrip() {
    for ast in [false,true] {for digits in ["18446744073709551617".to_string(),"9".repeat(512),format!("1{}7","0".repeat(1024))] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(&format!("function f(){{return {digits}n}};var expected='{digits}'")).unwrap();literal_body(&mut i,"f",ast);
        assert_eq!(i.run_script("f().toString()===expected&&(-f()).toString()==='-'+expected&&f()===f()").unwrap(),JsValue::Bool(true));
    }}
}

#[test]
fn bigint_literal_defaults_destructuring_closures_and_update_keep_value_semantics() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            function f(x=23n,{v=9n}={}){let a=x;let previous=a++;let nested=()=>a+1n;let {[23n]:p=7n}={};return [previous,a,nested(),v,p,23n]}
            function explicit(x){return x===0n?1n:2n}
        "#).unwrap();literal_body(&mut i,"f",ast);literal_body(&mut i,"explicit",ast);
        assert_eq!(i.run_script("f().map(String).join(',')==='23,24,25,9,7,23'&&f(99n,{v:3n}).map(String).join(',')==='99,100,101,3,7,23'&&explicit(0n)===1n").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn bigint_class_keys_fields_static_initialization_and_native_methods_remain_coherent() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            class C { [23n]=7n;#v=9n;static [5n]=11n;static {this.seen=13n} m(){return this[23n]+this.#v+1n} *g(){return 17n} async a(){return 19n} async *ag(){return 29n} }
            var c=new C,out,done=false,error;
        "#).unwrap();
        for name in ["C.prototype.m","C.prototype.g","C.prototype.a","C.prototype.ag"]{literal_body(&mut i,name,ast);}
        assert_eq!(i.run_script("c.m()===17n&&c.g().next().value===17n&&C[5n]===11n&&C.seen===13n").unwrap(),JsValue::Bool(true));
        i.run_script("Promise.all([c.a(),c.ag().next().then(r=>r.value)]).then(v=>{out=v;done=true},e=>{error=e;done=true})").unwrap();i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("done&&error===undefined&&out[0]===19n&&out[1]===29n").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn bigint_literal_registers_survive_yield_await_and_abrupt_cleanup() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        i.run_script(r#"
            var log=[],release,marker={},out,done=false,error;
            var pending=new Promise(r=>release=r);
            var home={*g(){using x={[Symbol.dispose](){log.push('dispose')}};try{let n=18446744073709551617n;yield n;return n+3n}finally{log.push('finally')}},
                async a(){let n=18446744073709551617n;await pending;return n+5n},
                async *ag(){let n=18446744073709551617n;yield n;return n+7n}};
        "#).unwrap();for name in ["home.g","home.a","home.ag"]{literal_body(&mut i,name,ast);}
        assert_eq!(i.run_script("var g=home.g();g.next().value===18446744073709551617n&&g.next().value===18446744073709551620n&&log.join(',')==='finally,dispose'").unwrap(),JsValue::Bool(true));
        i.run_script("home.a().then(v=>{out=v;done=true},e=>{error=e;done=true})").unwrap();assert_eq!(i.run_script("done===false").unwrap(),JsValue::Bool(true));
        i.run_script("release(0)").unwrap();i.drain_microtasks().unwrap();assert_eq!(i.run_script("done&&error===undefined&&out===18446744073709551622n").unwrap(),JsValue::Bool(true));
        i.run_script("var ag=home.ag(),values=[];ag.next().then(r=>{values.push(r.value);return ag.next()}).then(r=>values.push(r.value))").unwrap();i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("values[0]===18446744073709551617n&&values[1]===18446744073709551624n").unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script("log=[];g=home.g();g.next();try{g.throw(marker)}catch(e){error=e};error===marker&&log.join(',')==='finally,dispose'").unwrap(),JsValue::Bool(true));
    }
}

#[test]
fn bigint_constants_are_realm_independent_but_operator_errors_restore_callee_context() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;let parent=i.binding_current_realm();let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let f=i.binding_eval_in_realm(child,"function f(){return 18446744073709551617n};f").unwrap();
        let bad=i.binding_eval_in_realm(child,"function bad(){return 1n+1};bad").unwrap();
        i.binding_with_realm(child,|i|{literal_body(i,"f",ast);literal_body(i,"bad",ast);Ok(())}).unwrap();
        let value=i.binding_call_in_realm(parent,&f,JsValue::Undefined,vec![]).unwrap();let JsValue::BigInt(value)=value else {panic!("bigint")};assert_eq!(value.to_radix_string(10),"18446744073709551617");
        let error=i.binding_call_in_realm(parent,&bad,JsValue::Undefined,vec![]).unwrap_err();let error=i.normalize_throw(error).0;
        i.binding_with_realm(child,|i|{i.global_obj.borrow_mut().set_own("error",error.clone());assert_eq!(i.run_script("error instanceof TypeError").unwrap(),JsValue::Bool(true));Ok(())}).unwrap();
        assert_eq!(i.binding_current_realm(),parent);assert!(i.frames.is_empty());
    }
}

#[test]
fn malformed_manual_bigint_ast_keeps_runtime_syntax_error_adapter_in_actual_realm() {
    for digits in ["","12x","-1","1_0"] {
        let mut ast=crate::parse_script("1n;").unwrap();let crate::ast::StmtKind::Expression(expression)=&mut ast.body[0].kind else {panic!("expression")};expression.kind=crate::ast::ExprKind::BigInt(digits.into());
        let code=compile(&ast,false).unwrap();assert!(code.code.iter().any(|op|matches!(op,Instruction::TreeEval(..))));assert!(!code.code.iter().any(|op|matches!(op,Instruction::Constant(_,JsValue::BigInt(_)))));
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        for realm in [parent,child] {i.binding_with_realm(realm,|i|{let error=code.run(i,100).unwrap_err();let error=i.normalize_throw(error).0;i.global_obj.borrow_mut().set_own("error",error);assert_eq!(i.run_script("error instanceof SyntaxError").unwrap(),JsValue::Bool(true));Ok(())}).unwrap();}
    }
}

#[test]
fn bigint_constant_strong_roots_outlive_ast_and_quantums_and_budget_errors_restore_context() {
    let digits="9".repeat(512);let code=compile(&crate::parse_script(&format!("{digits}n;")).unwrap(),false).unwrap();
    let mut i=Interpreter::new();let a=code.run(&mut i,100).unwrap();let b=code.run(&mut i,100).unwrap();let (JsValue::BigInt(a),JsValue::BigInt(b))=(a,b) else {panic!("bigints")};assert!(Rc::ptr_eq(&a,&b));assert_eq!(a.to_radix_string(10),digits);
    let depth=i.frames.len();let realm=i.binding_current_realm();let strict=i.strict_eval_context;
    let env=i.global_env.clone();let mut frame=code.start_frame(&mut i,env,100).unwrap();let value=loop{match code.resume_frame(&mut i,&mut frame,1).unwrap(){FrameOutcome::Suspended=>{},FrameOutcome::Completed(v)=>break v,_=>panic!("scalar outcome")}};
    let JsValue::BigInt(value)=value else {panic!("bigint")};assert!(Rc::ptr_eq(&a,&value));
    let error=code.run(&mut i,0).unwrap_err();let error=i.normalize_throw(error).0;i.global_obj.borrow_mut().set_own("error",error);
    assert_eq!(i.run_script("error instanceof RangeError").unwrap(),JsValue::Bool(true));assert_eq!(i.frames.len(),depth);assert_eq!(i.binding_current_realm(),realm);assert_eq!(i.strict_eval_context,strict);
}
