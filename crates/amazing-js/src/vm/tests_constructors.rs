use super::*;

fn constructor_native(i: &mut Interpreter, name: &str, ast: bool) {
    let JsValue::Object(function) = i.run_script(name).unwrap() else { panic!("constructor") };
    let default_builtin = super::super::is_default_class_constructor(&function);
    let function = function.borrow();
    match &function.func.as_ref().unwrap().body {
        FunctionBody::Bytecode(_) if !ast => {},
        FunctionBody::Block(_) if ast => {},
        FunctionBody::Native(_) if default_builtin => {},
        body => panic!("unexpected constructor AST={ast}: {body:?}"),
    }
}

fn constructor_check(source: &str, check: &str, name: &str) {
    for ast in [false, true] {
        let mut i = Interpreter::new(); i.force_ast_callables = ast;
        i.run_script(source).unwrap(); constructor_native(&mut i, name, ast);
        assert_eq!(i.run_script(check).unwrap(), JsValue::Bool(true), "AST={ast} {source}");
    }
}

#[test]
fn constructors_read_live_parent_for_explicit_implicit_defaults_body_eval_and_arrows() {
    let bodies = [
        "constructor(){super()}",
        "constructor(x=Object.setPrototypeOf(C,B)){super()}",
        "constructor(){Object.setPrototypeOf(C,B);super()}",
        "constructor(){var invoke=()=>super();Object.setPrototypeOf(C,B);invoke()}",
        "constructor(){var invoke=()=>()=>super();Object.setPrototypeOf(C,B);invoke()()}",
        "constructor(){eval('Object.setPrototypeOf(C,B);super()')}",
        "constructor(){var invoke=()=>eval('super()');Object.setPrototypeOf(C,B);invoke()}",
        "",
    ];
    for body in bodies {
        constructor_check(&format!("class A{{constructor(){{this.parent='A'}}}};class B{{constructor(){{this.parent='B'}}}};class C extends A{{{body}}};Object.setPrototypeOf(C,B);var value=new C;"), "value.parent==='B'&&value instanceof C", "C");
    }
}

#[test]
fn super_parent_capture_precedes_argument_mutation_and_iterator_expansion() {
    for argv in ["Object.setPrototypeOf(C,B)", "...{[Symbol.iterator](){Object.setPrototypeOf(C,B);return [1][Symbol.iterator]()}}"] {
        constructor_check(&format!("class A{{constructor(){{this.parent='A'}}}};class B{{constructor(){{this.parent='B'}}}};class C extends A{{constructor(){{super({argv})}}}};var first=new C,second=new C;"), "first.parent==='A'&&second.parent==='B'", "C");
    }
}

#[test]
fn null_and_nonconstructible_super_check_follows_all_arguments_and_original_throw() {
    for parent in ["null", "{}", "()=>{}", "function*(){}"] {
        constructor_check(&format!(r#"
            var log=[],token={{}},caught=false;class A{{}};
            class C extends A{{constructor(throwing){{super((log.push('argv'),throwing?(()=>{{throw token}})():1))}}}};
            Object.setPrototypeOf(C,{parent});
            try{{new C(false)}}catch(e){{log.push(e instanceof TypeError)}};
            try{{new C(true)}}catch(e){{caught=e===token}};
        "#), "caught&&log.join(',')==='argv,true,argv'", "C");
    }
}

#[test]
fn lexical_super_initializes_original_public_private_and_decorated_class_elements() {
    let bodies = ["super()", "(()=>super())()", "(()=>()=>super())()()", "eval('super()')", "(()=>eval('super()'))()"];
    for body in bodies {
        constructor_check(&format!(r#"
            var log=[];class A{{constructor(){{log.push('parent')}}}};
            class C extends A{{field=(log.push('field'),31);#value=37;read(){{return this.#value}};constructor(){{{body}}}}};
            var value=new C;
        "#), "value.field===31&&value.read()===37&&log.join(',')==='parent,field'", "C");
    }
}

#[test]
fn lexical_super_replacement_object_receives_original_private_brand_and_fields() {
    constructor_check(r#"
        var result={};class A{constructor(){return result}};
        class C extends A{field=41;#value=43;read(){return this.#value};constructor(){(()=>super())()}};
        var value=new C;
    "#, "value===result&&value.field===41&&C.prototype.read.call(value)===43", "C");
}

#[test]
fn repeat_and_nested_super_construct_before_binding_error_fields_only_once() {
    for body in ["super();super(log.push('argv'))", "super(super())"] {
        let expected = if body.contains("argv") { "parent,field,argv,parent,ReferenceError" } else { "parent,field,parent,ReferenceError" };
        constructor_check(&format!("var log=[];class A{{constructor(){{log.push('parent')}}}};class C extends A{{field=(log.push('field'),1);constructor(){{{body}}}}};try{{new C}}catch(e){{log.push(e.name)}}"), &format!("log.join(',')==='{expected}'"), "C");
    }
}

#[test]
fn proxy_super_receives_actual_newtarget_and_implicit_forwarding_skips_iterator() {
    constructor_check(r#"
        var log=[],target;class A{};
        var P=new Proxy(A,{construct(t,args,newTarget){log.push(args[0]+':'+(newTarget===target));return {}}});
        class C extends A{field=1;constructor(value){super(value)}};
        class D extends A{field=2};Object.setPrototypeOf(C,P);Object.setPrototypeOf(D,P);
        target=C;var c=Reflect.construct(C,[7]);target=D;var d=new D(9);
    "#, "c.field===1&&d.field===2&&log.join(',')==='7:true,9:true'", "C");
}

#[test]
fn base_fields_precede_parameters_and_object_returns_preserve_completion() {
    constructor_check(r#"
        var log=[],replacement={};class C{field=(log.push('field'),1);#p=2;
            constructor(x=(log.push('parameter'),this.#p)){log.push('body:'+x);return replacement}}
        var value=new C;
    "#, "value===replacement&&log.join(',')==='field,parameter,body:2'", "C");
    constructor_check("class C{constructor(){return 3}};var value=new C", "value instanceof C", "C");
}

#[test]
fn derived_completion_covers_objects_undefined_primitives_uninitialized_this_and_body_throw() {
    for body in ["return {}", "super();return undefined", "return 1", "return null", "return undefined", "throw marker"] {
        let check = match body {
            "return {}" => "value!==undefined&&Object.getPrototypeOf(value)===Object.prototype&&error===undefined",
            "super();return undefined" => "value instanceof C&&error===undefined",
            "return 1" | "return null" => "error instanceof TypeError",
            "return undefined" => "error instanceof ReferenceError",
            _ => "error===marker",
        };
        constructor_check(&format!("var marker={{}},value,error;class A{{}};class C extends A{{constructor(){{{body}}}}};try{{value=new C}}catch(e){{error=e}}"), check, "C");
    }
}

#[test]
fn borrowed_foreign_constructors_complete_in_caller_realm_body_errors_stay_callee() {
    for ast in [false,true] {
        let mut i=Interpreter::new();i.force_ast_callables=ast;
        let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let exported=i.binding_eval_in_realm(child,r#"
            class A{};class Primitive extends A{constructor(){return 1}};
            class Missing extends A{constructor(){}};
            class Body extends A{constructor(){throw new TypeError('body')}};
            [Primitive,Missing,Body,TypeError,ReferenceError]
        "#).unwrap();
        i.global_env.declare_var("exports",exported).unwrap();
        constructor_native(&mut i,"exports[0]",ast);
        assert_eq!(i.run_script(r#"
            var result=[];for(var n=0;n<3;n++){try{new exports[n]}catch(e){
                result.push(Object.getPrototypeOf(e)===(n===1?ReferenceError:TypeError).prototype);
                result.push(Object.getPrototypeOf(e)===exports[n===1?4:3].prototype)}};
            result.join(',')==='true,false,true,false,false,true'
        "#).unwrap(),JsValue::Bool(true),"AST={ast}");
    }
}

#[test]
fn escaped_lexical_super_retains_original_constructor_and_live_parent_after_abrupt_body() {
    constructor_check(r#"
        var escaped,log=[];class A{constructor(){log.push('A')}};
        class B{constructor(){log.push('B')}};
        class C extends A{field=(log.push('field'),1);constructor(){escaped=()=>super();throw 9}};
        try{new C}catch(e){};Object.setPrototypeOf(C,B);var value=escaped();
    "#, "value.field===1&&log.join(',')==='B,field'", "C");
    constructor_check(r#"
        var escaped,log=[];class A{constructor(){log.push('A')}};
        class B{constructor(){log.push('B')}};
        class C extends A{constructor(){escaped=()=>super();super()}};
        new C;Object.setPrototypeOf(C,B);try{escaped()}catch(e){log.push(e.name)}
    "#, "log.join(',')==='A,B,ReferenceError'", "C");
}

#[test]
fn explicit_constructors_preserve_strict_private_eval_try_finally_and_recursive_calls() {
    constructor_check(r#"
        var log=[],marker={};class A{constructor(v){this.value=v}};
        class C extends A{#p=5;constructor(v){try{if(v>0)new C(v-1);super(v);eval('this.#p=7');
            try{absent=1}catch(e){log.push(e instanceof ReferenceError)}}finally{log.push('finally')}}read(){return this.#p}};
        var value=new C(2);
    "#, "value.value===2&&value.read()===7&&log.join(',')==='true,finally,true,finally,true,finally'", "C");
}

#[test]
fn constructor_decorated_elements_keep_extra_initializers_with_lexical_super() {
    constructor_check(r#"
        var log=[];
        function field(value,context){context.addInitializer(function(){log.push('extra:'+this.value)});return function(v){return v+2}}
        class A{constructor(){log.push('parent')}};
        class C extends A{@field value=3;constructor(){(()=>super())();log.push('body')}};
        var value=new C;
    "#, "value.value===5&&log.join(',')==='parent,extra:5,body'", "C");
}

#[test]
fn rooted_super_constructor_survives_one_opcode_quantums_and_budget_cleanup() {
    for exhaust in [false,true] {
        let mut i=Interpreter::new();i.run_script(r#"
            var log=[];class A{constructor(){log.push('A')}};
            class B{constructor(){log.push('B')}};
            class C extends A{field=(log.push('field'),1);constructor(){super(7);this.ok=11}};
        "#).unwrap();constructor_native(&mut i,"C",false);
        let JsValue::Object(function)=i.run_script("C").unwrap() else {panic!("constructor")};
        let (code,parent,home)={let f=function.borrow();let fun=f.func.as_ref().unwrap();
            let FunctionBody::Bytecode(code)=&fun.body else {panic!("native")};(code.clone(),fun.env.clone(),fun.home.clone())};
        let environment=EnvHandle::new(EnvKind::Function,Some(parent));environment.mark_this_uninitialized();
        i.frames.push(super::super::CallFrame {context_id:super::super::CallFrame::next_context_id(),this:JsValue::Undefined,
            home,super_constructor:Some(JsValue::Object(function.clone())),new_target:Some(JsValue::Object(function.clone())),own_function:Some(function)});
        let depth=i.frames.len();let outer_strict=i.strict_eval_context;
        i.strict_eval_context=true;
        let mut frame=code.0.program.start_frame(&mut i,environment.clone(),10000).unwrap();i.frames.pop();i.strict_eval_context=outer_strict;
        let mut changed=false;
        let outcome=loop {
            if !changed&&matches!(code.0.program.code.get(frame.pc),Some(Instruction::SuperCall(_,_,_))) {
                i.run_script("Object.setPrototypeOf(C,B)").unwrap();changed=true;if exhaust {frame.fuel=0}
            }
            match code.0.program.resume_frame(&mut i,&mut frame,1) {
                Ok(FrameOutcome::Suspended)=>{},result=>break result,
            }
        };
        assert!(changed);assert_eq!(i.frames.len(),depth-1);assert_eq!(i.strict_eval_context,outer_strict);
        if exhaust {
            let error=i.normalize_throw(outcome.err().expect("fuel")).0;i.global_obj.borrow_mut().set_own("error",error);
            assert_eq!(i.run_script("error instanceof RangeError&&log.length===0").unwrap(),JsValue::Bool(true));
        } else {
            assert!(matches!(outcome,Ok(FrameOutcome::Completed(_))));
            i.global_obj.borrow_mut().set_own("value",environment.resolve_this_checked().unwrap());
            assert_eq!(i.run_script("value.ok===11&&value.field===1&&log.join(',')==='A,field'").unwrap(),JsValue::Bool(true));
        }
    }
}

#[test]
fn explicit_constructor_identity_name_length_proto_and_class_call_contract_are_retained() {
    constructor_check(r#"
        class A{};class C extends A{constructor(first,second=2,...rest){super();this.sum=first+second+rest.length}};
        var value=new C(3,4,5),descriptor=Object.getOwnPropertyDescriptor(C,'prototype'),classError=false;
        try{C(1)}catch(e){classError=e instanceof TypeError}
    "#, "value.sum===8&&C.name==='C'&&C.length===1&&Object.getPrototypeOf(C)===A&&C.prototype.constructor===C&&!descriptor.writable&&!descriptor.configurable&&classError", "C");
}
