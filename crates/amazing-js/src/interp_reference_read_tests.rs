use super::*;

fn matrix(body: &str) {
    for actor in [false, true] {
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        i.run_script(&format!("var tested=function(){{{body}}};")).unwrap();
        let JsValue::Object(function) = i.run_script("tested").unwrap() else { panic!("function") };
        assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !actor);
        assert_eq!(i.run_script("tested()").unwrap_or_else(|e| panic!("AST={actor}: {}", e.describe())), JsValue::Bool(true), "AST={actor}");
    }
}

#[test]
fn nullish_reads_evaluate_raw_keys_but_reject_before_key_coercion() {
    for base in ["null", "undefined"] {
        for read in ["base[key()]", "(base)[key()]", "((base[key()]))", "typeof base[key()]", "base[key()].x"] {
            matrix(&format!(r#"
                var base={base},log=[],marker={{}},coercion={{[Symbol.toPrimitive](hint){{log.push(hint);throw marker}}}};
                function key(){{log.push('raw');return coercion}}
                var caught=false;try{{{read};}}catch(e){{caught=e instanceof TypeError&&e!==marker}}
                return caught&&log.join(',')==='raw';
            "#));
        }
    }
}

#[test]
fn raw_expression_and_base_abrupts_keep_their_original_identity() {
    matrix(r#"
        var log=[],marker={},caught=0;
        function base(){log.push('base');throw marker}
        function key(){log.push('raw');throw marker}
        try{base()[key()]}catch(e){if(e===marker)caught++}
        try{null[key()]}catch(e){if(e===marker)caught++}
        var holder={get key(){log.push('getter');throw marker}};
        try{undefined[holder.key]}catch(e){if(e===marker)caught++}
        var k={[Symbol.toPrimitive](hint){log.push(hint);throw marker}};
        try{({})[k]}catch(e){if(e===marker)caught++}
        return caught===4&&log.join(',')==='base,raw,getter,string';
    "#);
}

#[test]
fn primitive_symbol_utf16_and_proxy_reads_preserve_keys_and_receiver() {
    matrix(r#"
        var symbol=Symbol('s'),log=[],object={x:7,[symbol]:8,'\ud800':9};
        var p=new Proxy(object,{get(target,key,receiver){log.push(key===symbol?'symbol':key);return receiver===p?Reflect.get(target,key,receiver):-1}});
        var key={[Symbol.toPrimitive](hint){log.push(hint);return symbol}};
        var s=p[key],x=(p).x,u=p['\ud800'];
        Object.defineProperty(Number.prototype,'referenceProbe',{configurable:true,get:function(){'use strict';return this}});
        var n=(3).referenceProbe;delete Number.prototype.referenceProbe;
        return s===8&&x===7&&u===9&&log.join(',')==='string,symbol,x,\ud800'&&
            'abc'[1]==='b'&&(true).valueOf()===true&&n===3;
    "#);
}

#[test]
fn method_and_tag_reference_consumers_capture_receiver_before_arguments() {
    matrix(r#"
        var log=[],object={id:7,get m(){log.push('get');return function(v){'use strict';log.push('call');return this===object&&v===3}}};
        var key={toString(){log.push('key');return 'm'}};
        function raw(){log.push('raw');return key}function arg(){log.push('arg');return 3}
        var a=(object[raw()])(arg()),b=((object.m))(arg());
        var c=object[raw()]`x${arg()}`;
        var bare={m:function(){'use strict';return this===undefined}};
        return a&&b&&!c&&(0,bare.m)()&&(true?bare.m:bare.m)()&&
            log.join(',')==='raw,key,get,arg,call,get,arg,call,raw,key,get,arg,call';
    "#);
}

#[test]
fn nested_and_optional_reference_boundaries_preserve_short_circuit_work() {
    matrix(r#"
        var log=[],nil=null,marker={},key={toString(){log.push('key');throw marker}};
        function raw(){log.push('raw');return key}
        var a=nil?.[raw()].x,b=nil?.x[raw()],caught=0;
        try{(nil?.x)[raw()]}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        try{({get child(){log.push('child');return null}}).child[raw()]}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        class C{#x=11;read(o){return o.#x}maybe(o){return o?.#x}}
        var c=new C;try{c.read(null)}catch(e){if(e instanceof TypeError)caught++}
        return a===undefined&&b===undefined&&c.read(c)===11&&c.maybe(null)===undefined&&caught===3&&log.join(',')==='raw,child,raw';
    "#);
}

#[test]
fn write_delete_compound_update_and_pattern_consumers_keep_their_own_order() {
    matrix(r#"
        var log=[],marker={},base=null,key={toString(){log.push('key');throw marker}};
        function raw(){log.push('raw');return key}function rhs(){log.push('rhs');return 1}
        var caught=0;
        try{base[raw()]=rhs()}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        try{base[raw()]+=rhs()}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        try{base[raw()]++}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        try{delete base[raw()]}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        try{[base[raw()]=rhs()]=[undefined]}catch(e){if(e instanceof TypeError&&e!==marker)caught++}
        var o={x:2},once=0,canonical={toString(){once++;return 'x'}};
        o[canonical]+=3;o[canonical]++;
        return caught===5&&once===2&&o.x===6&&log.join(',')==='raw,rhs,raw,raw,raw,raw,rhs';
    "#);
}

#[test]
fn suspended_reads_keep_original_base_and_raw_key_error_order() {
    let cases = [
        ("return original[(PAUSE,key)]", "out===7&&log.join(',')==='key,get'"),
        ("try{return nil[(PAUSE,key)]}catch(e){return e instanceof TypeError}", "out===true&&log.length===0"),
        ("try{return nil[(PAUSE,throwRaw())]}catch(e){return e===marker}", "out===true&&log.join(',')==='raw'"),
        ("return (original[(PAUSE,'m')])((PAUSE,3))", "out===10&&log.join(',')==='call'"),
        ("return original?.[(PAUSE,key)]", "out===7&&log.join(',')==='key,get'"),
        ("return nil?.[(PAUSE,key)]", "out===undefined&&log.length===0"),
        ("try{return (nil?.x)[(PAUSE,key)]}catch(e){return e instanceof TypeError}", "out===true&&log.length===0"),
    ];
    let mut failures=Vec::new();
    for actor in [false,true] {for kind in ["generator","async","async-generator"] {
        for (body,expected) in cases {
            let body=body.replace("PAUSE",if kind=="async"{"await 0"}else{"yield 0"});
            let source=format!(r#"
                var log=[],nil=null,marker={{}},out,done=false,failed=false;
                var original={{value:7,get x(){{log.push('get');return this.value}},m(v){{log.push('call');return this.value+v}}}};
                var key={{toString(){{log.push('key');return 'x'}}}};
                function throwRaw(){{log.push('raw');throw marker}}
                var f={prefix}function{star}(){{{body}}};
            "#,prefix=if kind=="generator"{""}else{"async "},star=if kind=="async"{""}else{"*"});
            let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                let mut i=Interpreter::new();i.test_force_ast_callables(actor);i.run_script(&source).unwrap();
                let JsValue::Object(f)=i.run_script("f").unwrap() else {panic!("function")};
                assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!actor);
                i.run_script(match kind {
                    "generator"=>"var g=f(),r;do{r=g.next()}while(!r.done);out=r.value;done=true;",
                    "async-generator"=>"var g=f();async function drive(){var r;do{r=await g.next()}while(!r.done);return r.value}drive().then(v=>{out=v;done=true},e=>{out=e;failed=true;done=true});",
                    _=>"f().then(v=>{out=v;done=true},e=>{out=e;failed=true;done=true});",
                }).unwrap();i.drain_microtasks().unwrap();
                assert_eq!(i.run_script(&format!("done&&!failed&&({expected})")).unwrap(),JsValue::Bool(true));
            }));if outcome.is_err(){failures.push(format!("AST={actor} {kind}: {body}"));}
        }
    }}
    assert!(failures.is_empty(),"{}",failures.join("\n"));
}

#[test]
fn parked_computed_expression_retains_base_identity_before_replacement() {
    for actor in [false,true] {
        let mut i=Interpreter::new();i.test_force_ast_callables(actor);
        i.run_script(r#"
            var old={x:7},base=old,log=[],key={toString(){log.push('key');return 'x'}};
            var f=function*(){return base[yield 'raw']};var g=f(),a=g.next();base={x:99};var b=g.next(key);
        "#).unwrap();
        let JsValue::Object(f)=i.run_script("f").unwrap() else {panic!("function")};
        assert_eq!(matches!(f.borrow().func.as_ref().unwrap().body,FunctionBody::Bytecode(_)),!actor);
        assert_eq!(i.run_script("a.value==='raw'&&!a.done&&b.done&&b.value===7&&log.join(',')==='key'").unwrap(),JsValue::Bool(true));
    }
}
