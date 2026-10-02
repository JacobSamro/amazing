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
        (Ok(a), Ok(b)) if source == "this" => {
            assert_eq!(a, JsValue::Object(tree.global_obj.clone()));
            assert_eq!(b, JsValue::Object(vm.global_obj.clone()));
        }
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
fn inline_cache_tracks_shape_and_descriptor_changes() {
    for source in [
        "var o={x:1}; var n=o.x; o.x=2; n+o.x",
        "var o={x:1}; o.x; Object.defineProperty(o,'x',{value:7,writable:false}); o.x",
        "var o={x:1}; o.x; Reflect.deleteProperty(o,'x'); o.x=8; o.x",
        "var o={x:1}; o.x; Object.defineProperty(o,'x',{get:function(){return 9}}); o.x",
        "var o={x:1}; o.x; o.x='abc'; o.x",
        "var o={id:7}; o.id; o['[[DomNode]]']=1; o.id",
    ] {
        differential(source, &[]);
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
        "(function(){ return this===globalThis })()",
        "(function(){ 'use strict'; return this===undefined })()",
        "(function(){ return Object.getPrototypeOf(this)===Number.prototype }).call(5)",
        "function outer(a,b){ return [0].map(() => arguments.length)[0] }; outer(1,2)",
        "var inner; function outer(x){ inner=()=>arguments[0] }; outer(42); inner()",

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
fn loose_equality_matches_the_tree_walker_and_spec_cases() {
    for (source, expected) in [
        ("301 == '301'", true),
        ("' 0x10 ' == 16", true),
        ("0 == ''", true),
        ("0 == 'not a number'", false),
        ("0 / 0 == 'NaN'", false),
        ("null == undefined", true),
        ("null == 0", false),
        ("false == '0'", true),
        ("true == BigInt(1)", true),
        ("BigInt(1) == '1'", true),
        ("BigInt(1) == '1.0'", false),
        ("BigInt('9007199254740993') == 9007199254740992", false),
        ("BigInt(1) == 1.5", false),
        ("Symbol('x') == 'Symbol(x)'", false),
        ("({valueOf:function(){return 301;}}) == '301'", true),
        ("[301] == 301", true),
        ("var x = {}; x == x", true),
        ("({}) == ({})", false),
        ("301 != '301'", false),
    ] {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let actual = Interpreter::new().run_script_with_backend(source, backend).unwrap();
            assert_eq!(actual, JsValue::Bool(expected), "{source} with {backend:?}");
        }
    }
}

#[test]
fn getv_uses_intrinsic_prototypes_and_primitive_receivers() {
    for source in [
        "(5).toString(16) === '5'",
        "Number = undefined; (31).toString(16) === '1f'",
        "Boolean = undefined; true.toString() === 'true' && false.valueOf() === false",
        "var b = BigInt(15); BigInt = undefined; b.toString(16) === 'f'",
        "var s = Symbol('x'); Symbol = undefined; s.description === 'x'",
        "Object.defineProperty(Number.prototype, 'self', {get:function(){return this.valueOf();}}); (5).self === 5",
    ] {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let actual = Interpreter::new().run_script_with_backend(source, backend);
            assert_eq!(actual.unwrap(), JsValue::Bool(true), "{source} with {backend:?}");
        }
    }
}

#[test]
fn classic_scripts_preserve_var_and_function_bindings_on_redeclaration() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut interpreter = Interpreter::new();
        interpreter.run_script_with_backend("var dbname = 'first'; function openDb() { return 1; }", backend).unwrap();
        interpreter.run_script_with_backend("var dbname; var openDb;", backend).unwrap();
        assert_eq!(interpreter.run_script_with_backend("dbname + ':' + openDb()", backend).unwrap(), JsValue::str("first:1"));
        interpreter.run_script_with_backend("var dbname = 'second';", backend).unwrap();
        assert_eq!(interpreter.run_script_with_backend("dbname", backend).unwrap(), JsValue::str("second"));
        interpreter.run_script_with_backend("let lexicalName = 1;", backend).unwrap();
        assert!(interpreter.run_script_with_backend("var lexicalName;", backend).is_err());
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
fn jsvm10_break_continue_switch_for_in_for_of_labels_match() {
    for source in [
        // Plain break/continue in every loop shape.
        "var sum=0,i=0; while(true){ i++; if(i>5) break; sum+=i; } sum+':'+i;",
        "var sum=0,i=0; do { i++; if(i%2===0) continue; sum+=i; } while(i<6); sum+':'+i;",
        "var sum=0; for(var i=0;i<10;i++){ if(i===7) break; if(i%2) continue; sum+=i; } sum+':'+i;",
        "var out=''; for(var i=0;i<3;i++){ for(var j=0;j<3;j++){ if(j===1) continue; if(i===2) break; out+=i+','+j+';'; } } out;",
        // Labeled break/continue across nested loops.
        "var out=''; outer: for(var i=0;i<3;i++){ for(var j=0;j<3;j++){ if(j===1) continue outer; out+=i+','+j+';'; } } out;",
        "var out=''; outer: for(var i=0;i<3;i++){ for(var j=0;j<3;j++){ if(i===1&&j===1) break outer; out+=i+','+j+';'; } } out;",
        "var out=''; a: b: for(var i=0;i<3;i++){ if(i===1) continue b; out+=i+';'; } out;",
        "l: { var x=1; break l; x=2; } x;",
        // switch: fallthrough, default anywhere, break, no-default no-match.
        "var out=''; for(var i=0;i<4;i++){ switch(i){ case 0: out+='a'; case 1: out+='b'; break; case 2: out+='c'; break; default: out+='d'; } } out;",
        "var out=''; switch(2){ default: out+='d'; case 0: out+='a'; break; case 1: out+='b'; } out;",
        "var out='x'; switch(99){ case 0: out+='a'; } out;",
        "var calls=0; function f(){ calls++; return 1; } switch(f()){ case 0: break; case f(): break; } calls;",
        "var out=''; outer: for(var i=0;i<3;i++){ switch(i){ case 1: continue outer; } out+=i+';'; } out;",
        "var out=''; loop: for(var i=0;i<3;i++){ switch(i){ case 1: break loop; } out+=i+';'; } out;",
        // for-in: own+inherited enumerable keys, order, skips non-enumerable.
        "var out=''; for(var k in {a:1,b:2,c:3}) out+=k; out;",
        "var out=''; function P(){} P.prototype.p=1; var o=new P(); o.own=2; for(var k in o) out+=k; out;",
        "var out=''; for(var k in [10,20,30]) out+=k; out;",
        "var out=''; for(var k in null) out+=k; for(var k2 in undefined) out+=k2; out+'done';",
        "var out=''; outer: for(var i=0;i<2;i++){ for(var k in {a:1,b:2}){ if(k==='b') continue outer; out+=i+k+';'; } } out;",
        // for-of: arrays, strings, custom iterators, break closes early.
        "var out=''; for(var v of [1,2,3]) out+=v; out;",
        "var out=''; for(var v of 'abc') out+=v; out;",
        "var out=''; for(var v of [1,2,3,4]){ if(v===3) break; out+=v; } out;",
        "var out=''; for(var [a,b] of [[1,2],[3,4]]) out+=a+':'+b+';'; out;",
        "var closed=false; var it={ [Symbol.iterator](){ return { i:0, next(){ return {value:this.i++, done:this.i>5}; }, return(v){ closed=true; return {value:v,done:true}; } }; } };\
         for(var v of it){ if(v===2) break; } closed;",
        "var out=''; try { for(var v of [1,2,3]){ if(v===2) throw 'x'; out+=v; } } catch(e){ out+=e; } out;",
        // continue/break through try/finally must still run finally.
        "var out=''; for(var i=0;i<3;i++){ try { if(i===1) continue; out+=i; } finally { out+='f'; } } out;",
        "var out=''; outer: for(var i=0;i<3;i++){ try { if(i===1) break outer; out+=i; } finally { out+='f'; } } out;",
        "var out=''; while(true){ try { out+='a'; break; } finally { out+='b'; } } out;",
        // break/continue pop every scope they leave (else the next `let`
        // lands in a stale block/iteration scope and redeclares).
        "for (let x of [1,2]) { continue; } let x = 7; x;",
        "for (let x in {a:1}) { break; } let x = 8; x;",
        "while (true) { let y = 1; { let z = 2; break; } } let y = 2; let z = 3; y + z;",
        "switch (1) { case 1: let q = 1; break; } let q = 2; q;",
        "l: { let w = 1; { let v = 2; break l; } } let w = 3; let v = 4; w + v;",
        "var out=''; for (var i = 0; i < 3; i++) { let j = i; { let k = j; if (k === 1) continue; out += k; } } let k = 9; out + k;",
        "var out=''; for (const a of [1,2,3]) { try { let t = a; if (t === 2) continue; out += t; } finally { out += 'f'; } } let t = 0; out + t;",
    ] {
        differential(source, &["out", "sum", "i", "calls", "closed"]);
    }
}

#[test]
fn jsvm10_private_fields_and_methods_compile_in_generator_and_async_bodies() {
    use crate::ast::{self, ClassMember, StmtKind};

    fn class_method<'a>(class: &'a ast::ClassItem, name: &str) -> &'a ast::ClassMethod {
        class.members.iter().find_map(|member| match member {
            ClassMember::Method(method) => match &method.key {
                ast::PropKey::Ident(id) | ast::PropKey::Private(id) if id.name == name => {
                    Some(method)
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| panic!("no method {name} on class"))
    }

    let program = parse_script(
        "class C {            #x = 1;            #getX() { return this.#x; }            *gen() { this.#x++; yield this.#x; this.#x += 10; yield this.#getX(); yield (#x in this); }            async run() { this.#x++; await 1; this.#x += 10; return [this.#x, this.#getX(), (#x in this)].join(','); }            async *agen() { this.#x++; yield this.#x; await 1; this.#x += 10; yield this.#getX(); }          }",
    )
    .unwrap();
    let class = match &program.body[0].kind {
        StmtKind::Class(class) => class,
        other => panic!("expected class declaration: {other:?}"),
    };

    compile_class_method(class_method(class, "gen"), true)
        .expect("generator method with private field/method/`in` access should compile");
    compile_class_method(class_method(class, "run"), true)
        .expect("async method with private field/method/`in` access should compile");
    compile_class_method(class_method(class, "agen"), true)
        .expect("async generator method with private field/method access should compile");
}

#[test]
fn jsvm10_private_fields_and_methods_in_generator_and_async_bodies_match() {
    let mut interpreter = Interpreter::new();
    let source = "
        class C {
            #x = 1;
            #getX() { return this.#x; }
            *gen() {
                this.#x++;
                yield this.#x;
                this.#x += 10;
                yield this.#getX();
                yield (#x in this);
            }
        }
        var g = new C().gen();
        var out = [g.next().value, g.next().value, g.next().value].join(',');
    ";
    interpreter.run_script(source).unwrap();
    assert_eq!(interpreter.global_env.resolve("out"), Some(JsValue::str("2,12,true")));

    let mut interpreter = Interpreter::new();
    let source = "
        class D {
            #y = 2;
            static #tag = 'd';
            #getY() { return this.#y; }
            static #describe() { return D.#tag; }
            async run() {
                this.#y++;
                await 1;
                this.#y += 10;
                await 1;
                return [this.#y, this.#getY(), (#y in this), D.#describe()].join(',');
            }
        }
        var out = '';
        new D().run().then(function(v) { out = v; });
    ";
    interpreter.run_script(source).unwrap();
    assert_eq!(interpreter.global_env.resolve("out"), Some(JsValue::str("13,13,true,d")));

    let mut interpreter = Interpreter::new();
    let source = "
        class E {
            #z = 5;
            async *agen() {
                var self = this;
                var read = function() { return self.#z; };
                this.#z += read();
                yield this.#z;
            }
        }
        var out = '';
        var it = new E().agen();
        it.next().then(function(r) { out = r.value; });
    ";
    interpreter.run_script(source).unwrap();
    assert_eq!(interpreter.global_env.resolve("out"), Some(JsValue::num(10.0)));

    // Plain `=` to a setter-only private accessor must not read it first.
    let mut interpreter = Interpreter::new();
    let source = "
        class S {
            set #s(v) { this.log = v; }
            get #g() { return 1; }
            *gen() {
                this.#s = 5;
                yield this.log;
                try { this.#g = 2; } catch (e) { yield e instanceof TypeError; }
            }
        }
        var g = new S().gen();
        var out = [g.next().value, g.next().value].join(',');
    ";
    interpreter.run_script(source).unwrap();
    assert_eq!(interpreter.global_env.resolve("out"), Some(JsValue::str("5,true")));
}

#[test]
fn jsvm11_suspending_for_heads_and_using_declarations_compile() {
    for source in [
        "function* g() { var x; for ([x = yield] of [[]]) {} }",
        "function* g() { var x; for ({a: x = yield} of [{}]) {} }",
        "function* g() { for (var [y = yield] of [[]]) {} }",
        "async function f() { var o; for ({[await 0]: o} of [{}]) {} }",
        "async function f() { for (const [C = class extends (await 0) {}] of [[]]) {} }",
        "async function f() { var x; for ([x = await 0] in {a: 1}) {} }",
        "async function f() { for (await using x of []) {} }",
        "async function f() { for (let i in [0]) { await using x = null; } }",
        "function f() { using x = null; }",
    ] {
        let program = parse_script(source).unwrap();
        let function = match &program.body[0].kind {
            crate::ast::StmtKind::Function(function) => function,
            other => panic!("expected function: {other:?}"),
        };
        assert!(compile_function_item(function, program.body[0].span, false).is_ok(), "{source}");
    }
    // Plain destructuring heads without a suspension point still compile.
    for source in [
        "function* g() { var x; for ([x = 1] of [[]]) { yield x; } }",
        "async function f() { for (const {a = 2} of [{}]) { await a; } }",
        "async function f() { for await (const [C = class { static x = 1; }] of [[]]) {} }",
        "function* g() { var o; for ({a: o} of [{a: 1}]) { yield o; } }",
    ] {
        let program = parse_script(source).unwrap();
        let function = match &program.body[0].kind {
            crate::ast::StmtKind::Function(function) => function,
            other => panic!("expected function: {other:?}"),
        };
        assert!(compile_function_item(function, program.body[0].span, false).is_ok(), "{source}");
    }
}

#[test]
fn jsvm11_destructuring_preserves_iterator_across_yield_and_return() {
    let source = r#"
        var log = '', x;
        var values = {[Symbol.iterator]() { return {
            next() { log += 'n'; return {done:false, value:undefined}; },
            return() { log += 'c'; return {}; }
        }; }};
        function* g() { for ([x = yield 'default'] of [values]) { log += x; } }
        var it = g(); var first = it.next().value;
        var before = log; it.return(9); var after = log;
        first + ':' + before + ':' + after;
    "#;
    let result = Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str("default:n:nc"));
    let source = "function* g(){ let [x = yield 1, ...rest] = [undefined, 3, 4]; yield x + rest[1]; } var it=g(); it.next(); it.next(38).value;";
    assert_eq!(Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap(), JsValue::num(42.0));
    let source = r#"
        var log='', o={};
        var iterable={[Symbol.iterator](){return {
            next(){log+='next;'; return {done:false,value:9}},
            return(){log+='close;';return {}}
        }}};
        function* g(){[o[yield 'key']] = iterable;}
        var it=g(); it.next(); var before=log; it.next('x');
        before+':'+log+':'+o.x;
    "#;
    assert_eq!(Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap(), JsValue::str(":next;close;:9"));
    let source = r#"
        var touched=0;
        var iterable={[Symbol.iterator](){return {
            next(){return {done:false,get value(){touched++;throw 1}}},
            return(){return {}}
        }}};
        let [,]=iterable;
        var f; [f]=[function(){}];
        touched+':'+f.name;
    "#;
    assert_eq!(Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap(), JsValue::str("0:"));
}

#[test]
fn jsvm11_resource_disposal_unwinds_and_suppresses_errors() {
    for source in [
        "var out=''; function f(){ using a = {[Symbol.dispose](){out+='a'}}; using b = {[Symbol.dispose](){out+='b'}}; return 1; } f(); out;",
        "var out=''; for(using x of [{[Symbol.dispose](){out+='a'}}, {[Symbol.dispose](){out+='b'}}]) {out+='x'; continue;} out;",
        "var out=''; function* g(){using x={[Symbol.dispose](){out+='d'}}; yield 1;} var it=g(); it.next(); it.return(); out;",
    ] { differential(source, &["out"]); }
    let source = "var out=''; try {using x={[Symbol.dispose](){throw 'dispose'}}; throw 'body';} catch(e){out=e.error+':'+e.suppressed;} out;";
    assert_eq!(Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap(), JsValue::str("dispose:body"));
}

#[test]
fn jsvm11_async_disposal_parks_until_settlement_and_runs_remaining_resources() {
    let source = r#"
        var log='', resolve;
        var pending=new Promise(function(r){resolve=r});
        async function work(){
            using a={[Symbol.dispose](){log+='a'}};
            await using b={[Symbol.asyncDispose](){log+='b'; return pending;}};
            return 42;
        }
        var result=0;
        work().then(function(v){result=v});
        var before=log;
        resolve();
        before+':'+log+':'+result;
    "#;
    let mut interpreter = Interpreter::new();
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("before"), Some(JsValue::str("b")));
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("ba")));
    assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::num(42.0)));
    let source = r#"
        var errors='';
        async function f(){
            using a={[Symbol.dispose](){throw 'a'}};
            await using b={[Symbol.asyncDispose](){return Promise.reject('b')}};
            throw 'body';
        }
        f().catch(function(e){errors=e.error+':'+e.suppressed.error+':'+e.suppressed.suppressed});
    "#;
    let mut interpreter = Interpreter::new();
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("errors"), Some(JsValue::str("a:b:body")));
    let source = r#"
        var log='';
        async function f(){
            using a={[Symbol.dispose](){log+='a'}};
            await using b={[Symbol.dispose](){log+='b';throw 'error'}};
        }
        f().catch(function(e){log+=e});
        var before=log;
    "#;
    let mut interpreter = Interpreter::new();
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("before"), Some(JsValue::str("b")));
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("baerror")));
}

#[test]
fn jsvm12_generator_fdi_is_eager_and_parameter_eval_compiles() {
    let source = "function* g(){let arguments=42;yield arguments} g().next().value;";
    assert_eq!(Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap(), JsValue::num(42.0));
    for declaration in ["function* g", "async function* g"] {
        for lexical in ["let x;", "const x=2;", "class x {}", "using x=null;"] {
            let source = format!("{declaration}(a=eval('var a=1')){{{lexical} yield 1;}} var caught=false; try{{g()}}catch(e){{caught=e instanceof SyntaxError}} caught;");
            let mut interpreter = Interpreter::new();
            assert_eq!(interpreter.run_script_with_backend(&source, ExecutionBackend::Bytecode).unwrap(), JsValue::Bool(true), "{source}");
            let program = parse_script(&source).unwrap();
            let crate::ast::StmtKind::Function(function) = &program.body[0].kind else { unreachable!() };
            assert!(compile_function_item(function, program.body[0].span, false).is_ok(), "{source}");
        }
    }
    let source = r#"
        var ran=0;
        function* g(){
            var x=41;
            function f(){return 42}
            ran++; yield x;
        }
        var it=g();
    "#;
    let mut interpreter = Interpreter::new();
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    let state = generator_state(interpreter.global_env.resolve("it").unwrap()).unwrap();
    let state = state.borrow();
    assert!(!state.started);
    assert!(state.frame.is_some());
    assert_eq!(state.env.resolve("x"), Some(JsValue::Undefined));
    assert!(matches!(state.env.resolve("f"), Some(JsValue::Object(_))));
    assert_eq!(interpreter.global_env.resolve("ran"), Some(JsValue::num(0.0)));
    drop(state);
    assert_eq!(interpreter.run_script("it.next().value").unwrap(), JsValue::num(41.0));
    for action in ["it.return(9)", "try {it.throw(9)} catch(e){}"] {
        let source = format!("var ran=0; function* g(a=eval('var x=1')){{ran++;yield x}} var it=g(); {action}; it.next().done && ran===0;");
        assert_eq!(Interpreter::new().run_script_with_backend(&source, ExecutionBackend::Bytecode).unwrap(), JsValue::Bool(true));
    }
    let source = r#"
        var ran=0, done=false;
        async function* g(a=eval('var x=1')){ran++;yield x}
        var it=g();
        it.return(9).then(function(){return it.next()}).then(function(r){done=r.done});
    "#;
    let mut interpreter = Interpreter::new();
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("ran"), Some(JsValue::num(0.0)));
    assert_eq!(interpreter.global_env.resolve("done"), Some(JsValue::Bool(true)));
}

#[test]
fn jsvm10_suspension_free_unsupported_expressions_run_through_the_tree_walker() {
    for source in [
        "var o = {a: 1}; delete o.a; 'a' in o;",
        "var a, b; [a, b] = [1, 2]; a + b;",
        "function t(s, ...v) { return s.raw.join('|') + v.join(','); } t`x${1}y${2}`;",
        "var o = null; String(o?.a) + String(o?.b.c) + String(o?.());",
        "var calls = 0; function f() { calls++; return 'k'; } var o = null; \
         String(o?.b[f()]) + String(o?.b.c()) + calls;",
        "var o = {m() { return this === o; }}; String(o?.m()) + String(o.n?.());",
        "function sum() { var t = 0; for (var i = 0; i < arguments.length; i++) t += arguments[i]; return t; } \
         var xs = [1, 2]; sum(...xs, 3) + [...xs, ...'ab'].join('');",
        "typeof 10n + String(10n + 5n);",
        "var x = 1; eval('x + 1');",
        "function f() { var y = 2; eval('var z = y * 3'); return z; } f();",
        "function f(a) { eval('a = 5'); return arguments[0] + a; } f(1);",
        "function f() { 'use strict'; eval('var q = 1'); return typeof q; } f();",
        "function* g() { var n = yield 1; yield eval('n * 2'); } var it = g(); it.next(); it.next(21).value;",
        "var out = ''; async function h() { await 0; eval('var w = 7'); out = String(w); } h(); 0;",
        "function* g() { var a, b; var x = yield 1; [a, b] = [x, 2]; yield a + b; } \
         var it = g(); it.next(); var out = it.next(5).value; out;",
        "var out = ''; async function h() { var p = {q: {r: 3}}; await 0; out = String(p?.q?.r) + t`z`; } \
         function t(s) { return s[0]; } h(); 0;",
    ] {
        differential(source, &["out", "a"]);
    }
    // Assignment patterns containing yield now compile to resumable instructions.
    let program = parse_script("function* g() { var a; [a] = [yield 1]; }").unwrap();
    let function = match &program.body[0].kind {
        crate::ast::StmtKind::Function(function) => function,
        other => panic!("expected function: {other:?}"),
    };
    assert!(compile_function_item(function, program.body[0].span, false).is_ok());
}

#[test]
fn jsvm10_lexical_for_heads_match() {
    for source in [
        "var fs = []; for (let i = 0; i < 3; i++) fs.push(() => i); fs.map(f => f()).join(',');",
        "var fs = []; for (let i = 0; i < 3; i++) { fs.push(() => i); i += 0; } fs.map(f => f()).join(',');",
        "var out = ''; for (let i = 0, j = 10; i < 3; i++, j--) out += i + ':' + j + ';'; out;",
        "var out = ''; for (const x = 5; out.length < 3;) out += x; out;",
        "var out = ''; for (let i = 0; i < 5; i++) { if (i === 1) continue; if (i === 3) break; out += i; } let i = 'after'; out + i;",
        "var fs = []; for (let i = 0; i < 4; i++) { if (i % 2) continue; fs.push(() => i); } fs.map(f => f()).join(',');",
        "var out = ''; outer: for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) { if (j === 1) continue outer; out += i + '' + j; } out;",
        "var f; for (let i = 0; f = () => i, i < 2; i++) {} f();",
        "var out = ''; for (let i = 0; i < 3; i++) { try { if (i === 1) continue; out += i; } finally { out += 'f'; } } out;",
    ] {
        differential(source, &["out"]);
    }
}

#[test]
fn unsupported_program_is_rejected_before_hoisting_or_side_effects() {
    for source in [
        "changed = 1; function* g(){class A extends (yield 1) {x=7}}",
        "changed = 1; function* g(){class A {[yield 1]=7; static {1;}}}",
        // for/while/for-of/for-await-of/labeled break/continue are all VM-
        // compilable as of JSVM-10; ordinary class fields/static blocks
        // compile as of JSVM-14. These three cover constructs the
        // compiler still declines, keeping this test's actual point (atomic
        // whole-program rejection before hoisting or side effects) real.
        "changed = 1; with ({}) { 1; }",
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
    let code = compile(&parse_script("counter = counter + 1; counter;").unwrap(), false).unwrap();
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
        false,
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
        false,
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
fn generator_instance_is_iterable_via_symbol_iterator() {
    // `for...of` (and spread, destructuring, etc.) reach a generator's
    // `next` only through a real `GetIterator` call on the actual
    // `Symbol.iterator` well-known symbol, never through a lookalike named
    // property -- a generator instance's `[Symbol.iterator]` method must be
    // reachable by that real symbol key.
    let source = "
        function* g() { yield 1; yield 2; }
        var it = g();
        var iterFn = it[Symbol.iterator];
        var same = iterFn.call(it) === it;
        var first = it.next().value;
        same + ':' + first;
    ";
    let result = Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str("true:1"));
}

#[test]
fn async_generator_interleaves_yield_and_await_in_order() {
    let mut interpreter = Interpreter::new();
    let source = "
        var resolve;
        var pending = new Promise(function(r) { resolve = r; });
        var log = '';
        async function* g() {
            log += 'a';
            yield 1;
            log += 'b';
            var value = await pending;
            log += 'c';
            yield value + 1;
            log += 'd';
        }
        var it = g();
        var results = [];
        it.next().then(function(r) { results.push(r.value + ':' + r.done); log += 'n1'; });
        var beforeAwait = log;
        it.next().then(function(r) { results.push(r.value + ':' + r.done); log += 'n2'; });
        resolve(9);
        it.next().then(function(r) { results.push(r.value + ':' + r.done); log += 'n3'; });
        beforeAwait;
    ";
    let before_await = interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    // The first `.next()` call ran the body synchronously up to (and
    // including) the first `yield`, settling its own promise before
    // `.then()` is even attached; the `.then()` reaction itself is a
    // microtask and has not run yet at this point in the script.
    assert_eq!(before_await, JsValue::str("a"));
    // `resolve(9)` unblocks the second `.next()`'s await; its continuation
    // (a microtask) resumes to the second `yield`, settles that call's own
    // promise (queuing its `.then()` as a further microtask), and -- since
    // the generator is idle again -- immediately drains the third, already
    // queued `.next()` too, running it to completion within that same
    // microtask before any `.then()` reaction gets to run.
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("abn1cdn2n3")));
    let JsValue::Object(results) = interpreter.global_env.resolve("results").unwrap() else {
        panic!("results is not an array");
    };
    assert_eq!(results.borrow().get_own("0"), Some(JsValue::str("1:false")));
    assert_eq!(results.borrow().get_own("1"), Some(JsValue::str("10:false")));
    assert_eq!(results.borrow().get_own("2"), Some(JsValue::str("undefined:true")));
}

#[test]
fn async_generator_return_and_throw_before_start_complete_without_running_body() {
    // Mirrors the sync generator's own pre-start `.return`/`.throw`
    // short-circuit (`generator_resume`): per spec, calling either before
    // the body has ever run completes the generator directly, without
    // executing any of its code.
    let mut interpreter = Interpreter::new();
    let source = "
        var log = '';
        async function* g() { log += 'ran'; yield 1; }
        var it = g();
        var results = [];
        it.return(7).then(function(r) { results.push(r.value + ':' + r.done); });
        var beforeMicrotasks = log;
        beforeMicrotasks;
    ";
    let before = interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(before, JsValue::str(""));
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("")));
    let JsValue::Object(results) = interpreter.global_env.resolve("results").unwrap() else {
        panic!("results is not an array");
    };
    assert_eq!(results.borrow().get_own("0"), Some(JsValue::str("7:true")));
}

#[test]
fn async_generator_throw_settles_the_calls_own_promise() {
    let mut interpreter = Interpreter::new();
    let source = "
        async function* g() { yield 1; }
        var it = g();
        var caught = null;
        it.next().then(function() {
            it.throw('boom').catch(function(e) { caught = e; });
        });
        1;
    ";
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(interpreter.global_env.resolve("caught"), Some(JsValue::str("boom")));
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
fn compile_async_body_suspends_and_resumes_with_prebound_env() {
    use crate::ast::StmtKind;
    use crate::env::{EnvHandle, EnvKind};

    let mut interpreter = Interpreter::new();
    // The runtime bridge binds parameters and the surrounding script before
    // compiling only the async body; set up the same global shape here.
    interpreter
        .run_script("var resolve; var pending = new Promise(function(r){ resolve = r; }); var log = '';")
        .unwrap();

    let program = parse_script(
        "async function work() { log += 'a'; var value = await pending; log += 'b'; return value + 1; }",
    )
    .unwrap();
    let body = match &program.body[0].kind {
        StmtKind::Function(function) => function.body.clone(),
        other => panic!("expected function declaration: {other:?}"),
    };

    let code = compile_async_body(&body, false).unwrap();
    let env = EnvHandle::new(EnvKind::Function, Some(interpreter.global_env.clone()));
    env.bind_this(JsValue::Undefined);

    let promise = code.call_async_with_env(&mut interpreter, env).unwrap();
    // The body suspended at the pending await: only the prefix ran.
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("a")));
    assert!(matches!(promise, JsValue::Object(_)));

    // Settling the pending promise resumes the suspended frame to completion.
    interpreter.run_script("resolve(41)").unwrap();
    assert_eq!(interpreter.global_env.resolve("log"), Some(JsValue::str("ab")));
}

#[test]
fn async_resume_restores_strict_function_context() {
    // A strict async body must still run strict after resuming from a
    // suspended await. The probe observes the interpreter's strict-eval
    // context, which the branching strictness (`CompiledFunction::strict`)
    // must impose on the resumed slice, not on the entry slice alone.
    fn is_strict(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
        Ok(JsValue::Bool(i.strict_eval_context))
    }

    let mut interpreter = Interpreter::new();
    let probe = Interpreter::native("__is_strict", is_strict, &interpreter.function_proto);
    interpreter.global_obj.borrow_mut().set_own("__is_strict", probe);

    let source = "
        var resolve;
        var pending = new Promise(function(r) { resolve = r; });
        var log = '';
        var receiver = {async method() {
            'use strict';
            log += __is_strict();
            await pending;
            log += ',' + __is_strict();
        }};
        receiver.method();
        resolve();
    ";
    interpreter
        .run_script_with_backend(source, ExecutionBackend::Bytecode)
        .unwrap();
    assert_eq!(
        interpreter.global_env.resolve("log"),
        Some(JsValue::str("true,true")),
        "resumed slice must keep the compiled strictness"
    );
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
fn jsvm15_object_spread_operands_suspend_without_losing_literal_state() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let source = r#"
            var calls=0, log='', symbol=Symbol();
            var proto={set a(v){calls++}};
            var source={get a(){log+='g';return 7},[symbol]:8};
            Object.defineProperty(source,'__proto__',{value:42,enumerable:true});
            function* g(){yield {__proto__:proto,...(yield source)}}
            var it=g();
            var first=it.next();
            var o=it.next(first.value).value;
            o.a===7 && o[symbol]===8 && o.__proto__===42 &&
                Object.getPrototypeOf(o)===proto && calls===0 && log==='g';
        "#;
        assert_eq!(Interpreter::new().run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true));
        let source = r#"
            function* g(){yield {...yield yield}}
            var it=g();
            it.next(); it.next({a:42});
            it.next({a:42}).value.a;
        "#;
        assert_eq!(Interpreter::new().run_script_with_backend(source, backend).unwrap(), JsValue::num(42.0));
        let source = r#"
            var log='', result;
            var source={get a(){log+='g';return 42}};
            async function f(){return {before:(log+='b',1),...(await source),after:(log+='a',2)}}
            f().then(function(o){result=[o.before,o.a,o.after,log].join(',')});
        "#;
        let mut interpreter = Interpreter::new();
        interpreter.run_script_with_backend(source, backend).unwrap();
        assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::str("1,42,2,bga")));
        let source = r#"
            var result;
            async function* f(){yield {...(await null),...(await undefined),...(await 'xy')}}
            f().next().then(function(r){result=Object.keys(r.value).join(',')+':'+r.value[0]+r.value[1]});
        "#;
        let mut interpreter = Interpreter::new();
        interpreter.run_script_with_backend(source, backend).unwrap();
        assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::str("0,1:xy")));
    }
    for source in ["function* g(){yield {...yield yield}}", "async function f(){return {...await 0}}"] {
        let program = parse_script(source).unwrap();
        let crate::ast::StmtKind::Function(function) = &program.body[0].kind else { unreachable!() };
        assert!(compile_function_item(function, program.body[0].span, false).is_ok());
    }
}

#[test]
fn jsvm16_await_uses_internal_reactions_across_all_driver_boundaries() {
    let forms = [
        ("async function f(){try{return await p}catch(e){return 'error:'+e}} var output=f();", false, true, false),
        ("async function* f(){try{yield await p}catch(e){yield 'error:'+e}} var output=f().next();", true, true, false),
        ("async function* f(){yield 1;throw 99} var it=f();it.next();var output=it.return(p);", true, false, true),
        ("async function* f(){throw 99} var output=f().return(p);", true, false, true),
        ("async function* f(){} var it=f();it.next();var output=it.return(p);", true, false, true),
    ];
    let inputs = [
        ("var p=Promise.resolve(7);", "", false),
        ("var p=Promise.reject(9);", "", true),
        ("var release;var p=new Promise(function(r){release=r});", "release(7);", false),
        ("var release;var p=new Promise(function(r,j){release=j});", "release(9);", true),
    ];
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for (form, iterator, catches, done) in forms {
            for (input, release, rejects) in inputs {
                for poison_then in [false, true] {
                    let poison = if poison_then {
                        "Object.defineProperty(p,'then',{get:function(){thenGets++;throw 97}});"
                    } else { "" };
                    let source = format!(r#"
                        var thenGets=0,speciesGets=0,constructorGets=0;
                        {input}
                        Object.defineProperty(p,'constructor',{{get:function(){{constructorGets++;return Promise}}}});
                        {poison}
                        Object.defineProperty(Promise,Symbol.species,{{get:function(){{speciesGets++;throw 98}},configurable:true}});
                        {form}
                        {release}
                    "#);
                    let mut i = Interpreter::new();
                    i.run_script_with_backend(&source, backend).unwrap_or_else(|error|
                        panic!("{backend:?}: {form}: {input}: {poison_then}: {error:?}"));
                    for name in ["thenGets", "speciesGets"] {
                        assert_eq!(i.global_env.resolve(name), Some(JsValue::num(0.0)), "{form}: {name}");
                    }
                    assert_eq!(i.global_env.resolve("constructorGets"), Some(JsValue::num(1.0)), "{form}");
                    let Some(JsValue::Object(output)) = i.global_env.resolve("output") else { unreachable!() };
                    let fate = match &output.borrow().exotic {
                        Exotic::Promise(state) => state.state.clone(),
                        _ => panic!("async result must be a Promise"),
                    };
                    if rejects && !catches {
                        assert!(matches!(fate, PromiseFate::Rejected(JsValue::Num(9.0))), "{form}: {fate:?}");
                    } else {
                        let PromiseFate::Fulfilled(value) = fate else { panic!("{form}: {fate:?}") };
                        let expected = if rejects { JsValue::str("error:9") } else { JsValue::num(7.0) };
                        if iterator {
                            assert_eq!(i.get_prop(&value, "value").unwrap(), expected, "{form}");
                            assert_eq!(i.get_prop(&value, "done").unwrap(), JsValue::Bool(done), "{form}");
                        } else {
                            assert_eq!(value, expected, "{form}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn jsvm14_class_declarations_share_atomic_expression_lowering() {
    let sources = [
        r#"function* g(){class A {x=1; #p=2; get(){return this.x+this.#p} static n=4; static {this.n++}} yield [new A().get(),A.n,A.name].join(',')} g().next().value;"#,
        r#"function* g(){class Base {x=2} class A extends Base {y=3; static nameSeen=A.name} yield [new A().x,new A().y,A.nameSeen].join(',')} g().next().value;"#,
        r#"function* g(){var caught=false;try{class A {static x=A; static {throw 9}}}catch(e){caught=e===9} yield caught} g().next().value;"#,
    ];
    let expected = [JsValue::str("3,5,A"), JsValue::str("2,3,A"), JsValue::Bool(true)];
    for (source, expected) in sources.into_iter().zip(expected) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            assert_eq!(Interpreter::new().run_script_with_backend(source, backend).unwrap(), expected);
        }
        let program = parse_script(source).unwrap();
        let crate::ast::StmtKind::Function(function) = &program.body[0].kind else { unreachable!() };
        assert!(compile_function_item(function, program.body[0].span, false).is_ok());
    }
    for declaration in ["async function f", "async function* f"] {
        let return_or_yield = if declaration.contains('*') { "yield" } else { "return" };
        let invoke = if declaration.contains('*') { "f().next().then(function(r){result=r.value})" } else { "f().then(function(r){result=r})" };
        let source = format!("var result; {declaration}(){{class A{{x=42;static{{this.ok=true}}}} await 0; {return_or_yield} new A().x}} {invoke};");
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut interpreter = Interpreter::new();
            interpreter.run_script_with_backend(&source, backend).unwrap();
            assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::num(42.0)));
        }
    }
    let program = parse_script("function* g(){class A extends (yield 1){x=2}}").unwrap();
    let crate::ast::StmtKind::Function(function) = &program.body[0].kind else { unreachable!() };
    assert!(compile_function_item(function, program.body[0].span, false).is_err());
}

#[test]
fn jsvm13_async_resolution_observes_then_and_keeps_the_first_resolution() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for body in [
            "async function f(){return value} f().then(ok,bad);",
            "async function f(){return await value} f().then(ok,bad);",
            "async function* f(){yield value} f().next().then(function(r){ok(r.value)},bad);",
            "async function* f(){return value} f().next().then(function(r){ok(r.value)},bad);",
        ] {
            let source = format!(r#"
                var release, result='pending';
                var pending=new Promise(function(resolve){{release=resolve}});
                var value={{then:function(resolve,reject){{resolve(pending);reject(9);throw 10}}}};
                function ok(v){{result='ok:'+v}}
                function bad(v){{result='bad:'+v}}
                {body}
                release(7);
            "#);
            let mut interpreter = Interpreter::new();
            interpreter.run_script_with_backend(&source, backend).unwrap();
            assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::str("ok:7")), "{body}");
        }
        let source = r#"
            var gets=0, result='pending';
            var value=Promise.resolve(7);
            Object.defineProperty(value,'then',{get:function(){gets++;throw 9}});
            async function f(){return value}
            f().then(function(){result='fulfilled'},function(e){result='error:'+e});
        "#;
        let mut interpreter = Interpreter::new();
        interpreter.run_script_with_backend(source, backend).unwrap();
        assert_eq!(interpreter.global_env.resolve("gets"), Some(JsValue::num(1.0)));
        assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::str("error:9")));
        let source = r#"
            var gets=0, result='pending';
            var value=Promise.resolve(7);
            function Other(){}
            Other.prototype=Promise.prototype;
            value.constructor=Other;
            Object.defineProperty(value,'then',{get:function(){gets++;return function(resolve){resolve(42)}}});
            async function f(){return await value}
            f().then(function(v){result=v});
        "#;
        let mut interpreter = Interpreter::new();
        interpreter.run_script_with_backend(source, backend).unwrap();
        assert_eq!(interpreter.global_env.resolve("gets"), Some(JsValue::num(1.0)));
        assert_eq!(interpreter.global_env.resolve("result"), Some(JsValue::num(42.0)));
    }
}

#[test]
fn async_jobs_interleave_with_promise_reactions() {
    let mut interpreter = Interpreter::new();
    let source = "
        var log = [];
        async function first(value) { log.push('await:' + value); }
        async function both() { await first(1); await first(2); }
        both();
        new Promise(function(resolve) { log.push('promise:1'); resolve(); })
          .then(function() { log.push('promise:2'); });
    ";
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    let result = interpreter.run_script("log.join(',')").unwrap();
    assert_eq!(result, JsValue::str("await:1,promise:1,await:2,promise:2"));

    let mut interpreter = Interpreter::new();
    let source = "
        var log = [];
        var thenable = { then: function(resolve) { resolve(1); } };
        async function work() { log.push('await:' + await thenable); }
        work();
        new Promise(function(resolve) { log.push('promise:1'); resolve(); })
          .then(function() { log.push('promise:2'); });
    ";
    interpreter.run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    let result = interpreter.run_script("log.join(',')").unwrap();
    assert_eq!(result, JsValue::str("promise:1,promise:2,await:1"));
}

#[test]
fn fuel_stops_backedges_and_vm_state_is_clean_after_error() {
    let mut interpreter = Interpreter::new();
    interpreter.completion = JsValue::str("saved");
    let code =
        compile(&parse_script("let x = 0; while (true) { let local = 3; x++; }").unwrap(), false).unwrap();
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
    let code = compile(&parse_script("var x = 1;").unwrap(), false).unwrap();
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
    assert!(compile(&ast, false).unwrap_err().message.contains("limit"));
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
fn ordinary_constructor_and_member_access_gaps_match_tree_walker() {
    // These are not derived-class features, but the VM only became eligible
    // to run these programs once classes with `extends` stopped forcing a
    // fallback: they surfaced as VM-vs-interpreter regressions in the JSVM-6
    // Test262 gate and were fixed alongside it.
    for source in [
        // An ordinary VM-compiled function's `prototype.constructor` must
        // link back to it (MakeConstructor), or `new F()` instances resolve
        // `.constructor` through Object.prototype instead of F.
        "function F() {} (new F()).constructor === F",
        // A class constructor throws TypeError when called without `new`.
        "class C {} (function () { try { C(); return 'no throw'; } \
         catch (e) { return e instanceof TypeError; } })()",
        // Arrow functions lexically inherit `new.target` from the enclosing
        // (possibly derived) constructor, set at the arrow's *creation* time.
        "function F() { var capture = () => new.target; return capture(); } \
         new F() === F",
        // Arrow `super()`/`super.prop` inside a derived constructor must
        // resolve through the same lexical inheritance.
        "class A { constructor(x) { this.x = x; } } \
         class B extends A { constructor(x) { (() => super(x))(); this.y = x + 1; } } \
         var b = new B(1); b.x + b.y",
        // `__proto__: null` in an object literal sets [[Prototype]] instead
        // of creating an own property; `__proto__: <object>` sets it too.
        "var o = { __proto__: null }; [Object.getPrototypeOf(o) === null, \
         Object.getOwnPropertyDescriptor(o, '__proto__') === undefined].join(',')",
        "var proto = { tag: 1 }; var o = { __proto__: proto }; Object.getPrototypeOf(o) === proto",
        // Sloppy-mode duplicate simple parameter names: the last one wins,
        // and no `arguments` object is created when a param is named that.
        "function f(a, a) { return a; } f(1, 2)",
        "function f(arguments) { return arguments; } f(9)",
        // Member-access evaluation order on a computed key: the key
        // expression's own side effect (a call) still runs even though the
        // base is nullish, but converting the *result* to a property key
        // must not run before the nullish check on the base.
        "(function () { \
           var base = null; var prop = { toString() { throw new TypeError('should not run'); } }; \
           try { base[prop] = 1; return 'no throw'; } catch (e) { return e instanceof TypeError && e.message !== 'should not run'; } \
         })()",
        "(function () { \
           var seen = false; var base = null; var prop = function () { seen = true; throw new RangeError('key'); }; \
           try { base[prop()] = 1; return 'no throw'; } catch (e) { return seen && e instanceof RangeError; } \
         })()",
    ] {
        differential(source, &[]);
    }
}

#[test]
fn derived_classes_match_tree_walker() {
    for source in [
        // Explicit super() call, own field assignment after it.
        "class A { constructor(x) { this.x = x; } } \
         class B extends A { constructor(x) { super(x); this.y = x + 1; } } \
         var b = new B(1); b.x + b.y",
        // Implicit constructor forwards its arguments to super().
        "class A { constructor(x, y) { this.x = x; this.y = y; } } \
         class B extends A {} \
         var b = new B(2, 3); b.x + b.y",
        // super.method() call: property lookup on the prototype chain, `this` receiver.
        "class A { greet() { return 'A' + this.tag; } } \
         class B extends A { constructor() { super(); this.tag = '!'; } greet() { return super.greet() + 'B'; } } \
         (new B()).greet()",
        // super.prop plain read (not a call).
        "class A { get value() { return 1; } } \
         class B extends A { constructor() { super(); } read() { return super.value; } } \
         (new B()).read()",
        // instanceof through the prototype chain, and constructor-chain
        // (static) inheritance from ClassDefinitionEvaluation.
        "class A {} class A2 extends A {} class B extends A2 {} var b = new B(); \
         [b instanceof A, b instanceof A2, Object.getPrototypeOf(B) === A2].join(',')",
        // new.target through an implicit super() call.
        "class A { constructor() { this.nt = new.target === B; } } \
         class B extends A {} (new B()).nt",
        // super() spread arguments.
        "class A { constructor(x, y) { this.sum = x + y; } } \
         class B extends A { constructor(xs) { super(...xs); } } \
         (new B([2, 5])).sum",
        // Explicit object return from a derived constructor wins over `this`.
        "class A {} class B extends A { constructor() { super(); return { tag: 'override' }; } } \
         (new B()).tag",
        // this used before super() throws a ReferenceError, matching the tree walker.
        "class A {} class B extends A { constructor() { this.x = 1; super(); } } \
         (function () { try { new B(); return 'no throw'; } catch (e) { return e instanceof ReferenceError; } })()",
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
    assert!(compile(&program, false).is_err());
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
                                    Ok(ast) => match compile(&ast, false) {
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

#[test]
fn merged_class_accessors_and_arrow_arguments() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for source in [
            "class C { get x() { return this.value; } set x(v) { this.value=v; } } var a=Object.getOwnPropertyDescriptor(C.prototype,'x'); var b=Object.getOwnPropertyDescriptor(C.prototype,'x'); var c=new C(); c.x=7; a.get===b.get && a.set===b.set && a.get.name==='get x' && a.set.length===1 && c.x===7",
            "class C { method() {} } Object.getOwnPropertyDescriptor(C.prototype,'method').enumerable===false && C.prototype.method.name==='method'",
            "function f(value) { return (({x}={x:3}) => arguments[0]+x)(); } f(7)===10",
        ] {
            let mut interpreter = Interpreter::new();
            let result = interpreter.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {source}: {}", error.describe()));
            assert_eq!(result, JsValue::Bool(true), "{backend:?}: {source}");
        }
    }
}

#[test]
fn compiled_method_and_generator_boundaries_preserve_eval_strictness_and_names() {
    for source in [
        "function* g() {} var d=Object.getOwnPropertyDescriptor(g,'name'); d.value==='g' && !d.writable && !d.enumerable && d.configurable",
        "var o={m(){ let x; eval('var x'); }}; try { o.m(); false; } catch(e) { e instanceof SyntaxError; }",
        // Parameter eval declarations and body lexical declarations are in
        // separate environments when the parameter list contains expressions.
        "var o={*m(p=eval('var arguments')){let arguments;yield arguments;}}; var g=o.m(); g.next().value===undefined && g.next().done",
        "'use strict'; var o={m(){return function(){return typeof this;};}}; o.m().call('x')==='string'",
    ] {
        let mut interpreter=Interpreter::new();
        assert_eq!(interpreter.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
    }
}

#[test]
fn async_iterator_close_preserves_body_throw_over_every_close_failure() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for close in [
            "get return() { throw closeError; }",
            "return: true",
            "return() { throw closeError; }",
            "return() { return Promise.reject(closeError); }",
            "return() { return Promise.resolve(1); }",
        ] {
            for completion in ["throw bodyError", "return 1"] {
                let mut interpreter = Interpreter::new();
                let source = format!(r#"
                    var bodyError = {{origin: 'body'}}, closeError = {{origin: 'close'}};
                    var observed, settled = false, iterations = 0;
                    var iterable = {{[Symbol.asyncIterator]() {{
                        return {{next() {{ return {{value: 1, done: false}}; }}, {close}}};
                    }}}};
                    async function work() {{
                        for await (const value of iterable) {{ iterations++; {completion}; }}
                    }}
                    work().then(function(value) {{ observed = value; settled = true; }},
                        function(error) {{ observed = error; settled = true; }});
                "#);
                interpreter.run_script_with_backend(&source, backend).unwrap();
                let expected = if completion.starts_with("throw") {
                    "settled && iterations === 1 && observed === bodyError"
                } else if close == "return: true" || close.contains("Promise.resolve(1)") {
                    "settled && iterations === 1 && observed instanceof TypeError"
                } else {
                    "settled && iterations === 1 && observed === closeError"
                };
                assert_eq!(interpreter.run_script(expected).unwrap(), JsValue::Bool(true),
                    "{backend:?}: {completion}: {close}");
            }
        }
    }
}

#[test]
fn class_static_methods_and_accessors_install_on_the_constructor() {
    // Found while isolating JSVM-7's generator/async-generator Test262 delta
    // from unrelated noise: `compile_class` never read `ClassMethod::is_static`,
    // so every static method/accessor was compiled and installed exactly like
    // an instance one -- onto `prototype`, with `prototype` as its
    // [[HomeObject]] -- leaving the constructor's own static members
    // permanently `undefined`. A class with only static methods (no fields,
    // no static blocks, both of which already force tree-walker fallback)
    // was otherwise fully VM-eligible, so this was live, not masked.
    let source = "
        class Base { static tag() { return 'base'; } }
        class C extends Base {
            static m(a, b) { return arguments.length + ':' + (a + b); }
            static get g() { return 42; }
            static set g(v) { C.captured = v; }
            static parent() { return super.tag(); }
        }
        var proto = C.prototype;
        (typeof proto.m) + ':' + (typeof C.m) + ':' + C.m(1, 2) + ':'
            + C.g + ':' + (C.g = 7, C.captured) + ':' + C.parent();
    ";
    let result = Interpreter::new().run_script_with_backend(source, ExecutionBackend::Bytecode).unwrap();
    assert_eq!(result, JsValue::str("undefined:function:2:3:42:7:base"));
}
