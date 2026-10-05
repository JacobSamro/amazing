// Whole runtime source/constructor matrices use real public eval/call APIs.
#[cfg(test)]
mod dynamic_function_source_protocol_tests {
    use super::*;
    fn matrix(source: &str) {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            assert_eq!(i.run_script_with_backend(source,backend)
                .unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())),JsValue::Bool(true));
        }
    }
    #[test]
    fn all_four_dynamic_kinds_retain_prescribed_source_and_metadata() {
        matrix(r#"
          function check(v,l){if(!v)throw l;}
          var kinds=[[Function,'function'],[(function*(){}).constructor,'function*'],
            [(async function(){}).constructor,'async function'],[(async function*(){}).constructor,'async function*']];
          for(var [C,prefix] of kinds){
            for(var create of [()=>C('a','b=1','return a+b;'),()=>new C('a','b=1','return a+b;'),
              ()=>Reflect.construct(C,['a','b=1','return a+b;']),()=>C.bind(null,'a')('b=1','return a+b;'),
              ()=>new Proxy(C,{})('a','b=1','return a+b;'),
              ()=>new (C.bind(null,'a'))('b=1','return a+b;'),
              ()=>Reflect.apply(new Proxy(C,{}),null,['a','b=1','return a+b;'])]){
              var f=create();check(f.toString()===prefix+' anonymous(a,b=1\n) {\nreturn a+b;\n}','prescribed source');
              check(f.name==='anonymous'&&f.length===1,'name/length');
              Object.defineProperty(f,'name',{value:'changed'});
              check(f.toString()===prefix+' anonymous(a,b=1\n) {\nreturn a+b;\n}','source ignores renamed metadata');
            }
            check(C().toString()===prefix+' anonymous(\n) {\n\n}','empty source');
            check(C(undefined).toString()===prefix+' anonymous(\n) {\nundefined\n}','explicit undefined body');
          } true
        "#);
    }
    #[test]
    fn three_real_grammar_goals_preserve_comments_and_forbid_boundary_crossing() {
        matrix(r#"
          function check(v,l){if(!v)throw l;}
          var kinds=[Function,(function*(){}).constructor,(async function(){}).constructor,(async function*(){}).constructor];
          for(var C of kinds){
            for(var parameter of ['a// eof','a/*ok*/','<!-- comment\na','a\n--> eof'])
              check(typeof C(parameter,'return a;')==='function','independent parameter EOF comments');
            for(var args of [['/*','*/ ) {'],['a) { globalThis.escaped=true; } function other(','return 1;'],
              ['a','} globalThis.escaped=true; {'],['a=1','"use strict";return a;'],['a','a','"use strict";'],['super.x'],['super()']]){
              var rejected=false;try{C.apply(null,args);}catch(e){rejected=e instanceof SyntaxError;}
              check(rejected,'whole grammar rejection');
            }
            check(globalThis.escaped===undefined,'no body escape');
          }
          check(typeof Function('a','a','return a')==='function','ordinary sloppy duplicate permitted');
          check(Function('yield','await','return yield+await;')(2,3)===5,'ordinary parameter goal');
          for(var [C,word] of [[kinds[1],'yield'],[kinds[2],'await'],[kinds[3],'yield'],[kinds[3],'await']]){
            var rejected=false;try{C(word,'return 1;');}catch(e){rejected=e instanceof SyntaxError;}
            check(rejected,'kind parameter goal');
          }
          function strictCaller(){'use strict';return Function('return this;');}
          check(strictCaller()()===globalThis,'dynamic caller strictness excluded');
          var named=Function('return typeof anonymous;');
          check(named()==='undefined','no synthesized name lexical binding');
          // Current primary FormalParameters permits simple sloppy duplicates
          // for declarations, expressions and dynamic functions in all kinds.
          var prefixes=['function','function*','async function','async function*'];
          for(var k=0;k<kinds.length;k++){
            var C=kinds[k],prefix=prefixes[k];
            check(typeof C('a','a','return a;')==='function','all-kind sloppy dynamic duplicate');
            check(typeof (0,eval)(prefix+' declared(a,a){return a;};declared')==='function','all-kind sloppy declaration duplicate');
            check(typeof (0,eval)('('+prefix+' named(a,a){return a;})')==='function','all-kind sloppy expression duplicate');
            for(var args of [['a','a','"use strict";return a;'],['a=1','a','return a;'],
              ['{a}','a','return a;'],['a','...a','return a;']]){
              var rejected=false;try{C.apply(null,args);}catch(e){rejected=e instanceof SyntaxError;}
              check(rejected,'strict or non-simple dynamic duplicates rejected');
            }
            for(var text of [prefix+' strictDecl(a,a){"use strict";}',
              '('+prefix+' strictExpr(a,a){"use strict";})',
              prefix+' nonSimple(a=1,a){}']){
              var rejected=false;try{(0,eval)(text);}catch(e){rejected=e instanceof SyntaxError;}
              check(rejected,'strict or non-simple declaration/expression duplicates rejected');
            }
          }
          for(var text of ['({m(a,a){}})','({*m(a,a){}})','({async m(a,a){}})',
            '({async *m(a,a){}})','(class{m(a,a){}})','(class{*m(a,a){}})',
            '(class{async m(a,a){}})','(class{async *m(a,a){}})',
            '(a,a)=>a','async(a,a)=>a']){
            var rejected=false;try{(0,eval)(text);}catch(e){rejected=e instanceof SyntaxError;}
            check(rejected,'method and arrow unique parameter protocol');
          } true
        "#);
    }
    #[test]
    fn argument_coercion_and_all_parse_errors_precede_newtarget_prototype_once() {
        matrix(r#"
          function check(v,l){if(!v)throw l;}
          var kinds=[Function,(function*(){}).constructor,(async function(){}).constructor,(async function*(){}).constructor];
          for(var C of kinds){
            var log=[],proto={},sentinel={};
            function text(label,value,fail){return{toString(){log.push(label);if(fail)throw sentinel;return value;}};}
            var N=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){log.push('prototype');return proto;}return Reflect.get(t,k,r);}});
            var f=Reflect.construct(C,[text('p1','a'),text('p2','b'),text('body','return a+b;')],N);
            check(log.join(',')==='p1,p2,body,prototype'&&Object.getPrototypeOf(f)===proto,'coercion before prototype');
            for(var failure of ['first','second','body','params','bodySyntax']){
              log=[];var same=false,args=[text('p1',failure==='params'?'a)':'a',failure==='first'),
                text('p2','b',failure==='second'),text('body',failure==='bodySyntax'?'}':'return 1;',failure==='body')];
              try{Reflect.construct(C,args,N);}catch(e){same=['params','bodySyntax'].includes(failure)?e instanceof SyntaxError:e===sentinel;}
              var expected=failure==='first'?'p1':failure==='second'?'p1,p2':'p1,p2,body';
              check(same&&log.join(',')===expected,'abrupt/error order');
            }
            var throwingN=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){log.push('prototype');throw sentinel;}return Reflect.get(t,k,r);}});
            log=[];var same=false;try{Reflect.construct(C,[text('body','return 1;')],throwingN);}catch(e){same=e===sentinel;}
            check(same&&log.join(',')==='body,prototype','prototype abrupt identity');
          } true
        "#);
    }
    #[test]
    fn actual_source_spelling_survives_function_forms_and_utf16_eval() {
        matrix(r#"
          function check(v,l){if(!v)throw l;}
          var sources=['function /*a*/ named ( a ) {\r\n return a; }','function* /*g*/ named(){yield 1}',
            'async function /*a*/ named(){return 1}','async function* /*ag*/ named(){yield 1}',
            '(a /*b*/) => /*c*/ a','async (a) => a','class /*c*/ A { constructor /*d*/ () {} m(){} }',
            'class A {}'];
          for(var source of sources){var f=(0,eval)('('+source+')');check(f.toString()===source,'actual spelling');}
          for(var text of ['\ud800','\udc00','\ud800X\udc00','\ud800\udc00','\u0000','\ud801\udc00']){
            var source='function /*'+text+'*/ f(){return "'+text+'";}';
            var f=(0,eval)('('+source+')');check(f.toString()===source&&f()===text,'exact units and retained eval source');
            for(var C of [Function,(function*(){}).constructor,(async function(){}).constructor,(async function*(){}).constructor]){
              var f=C('a="'+text+'"','return a;');
              check(f.toString().includes('a="'+text+'"\n) {\nreturn a;\n}'),'dynamic exact units');
            }
          }
          var key={toString(){return'actual'}};var o={ /*lead*/ [key] /*a*/ (x) {return x;}, get value /*b*/ () {return 1;} };
          check(o.actual.toString()==='[key] /*a*/ (x) {return x;}','computed method syntax');
          check(Object.getOwnPropertyDescriptor(o,'value').get.toString()==='get value /*b*/ () {return 1;}','getter syntax');
          true
        "#);
    }
    #[test]
    fn native_bound_and_proxy_fallback_reads_no_author_properties() {
        matrix(r#"
          function check(v,l){if(!v)throw l;}
          var f=function /*source*/ f(){return 1;};
          Object.defineProperty(f,'name',{get(){throw'author name getter'}});
          check(Function.prototype.toString.call(f)==='function /*source*/ f(){return 1;}','genuine source');
          var count=0,p=new Proxy(f,{get(){count++;throw'proxy get'},getPrototypeOf(){count++;throw'proxy prototype'}});
          check(Function.prototype.toString.call(p).includes('[native code]')&&count===0,'proxy opaque fallback');
          var revocable=Proxy.revocable(f,{});revocable.revoke();
          check(Function.prototype.toString.call(revocable.proxy).includes('[native code]'),'revoked callable fallback');
          var b=(function ordinary(){}).bind(null);Object.defineProperty(b,'name',{get(){throw'bound name getter'}});
          check(Function.prototype.toString.call(b).includes('[native code]'),'bound fallback');
          check(Function.prototype.toString.call(Function.prototype.call).includes('[native code]'),'intrinsic fallback');
          var rejected=false;try{Function.prototype.toString.call({get name(){throw'noncallable name'}});}catch(e){rejected=e instanceof TypeError;}
          check(rejected,'noncallable brand');true
        "#);
    }
    #[test]
    fn real_foreign_ctor_and_newtarget_intrinsics_scope_and_async_execution() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            let caller=i.binding_current_realm();
            let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            i.binding_eval_in_realm(child,"var realmValue=73;function ForeignNewTarget(){};ForeignNewTarget.prototype=null;").unwrap();
            for (name,source) in [("foreignNewTarget","ForeignNewTarget"),
                ("foreignFunction","Function"),("foreignGenerator","(function*(){}).constructor"),
                ("foreignAsync","(async function(){}).constructor"),("foreignAsyncGenerator","(async function*(){}).constructor"),
                ("foreignFunctionPrototype","Function.prototype"),
                ("foreignGeneratorPrototype","Object.getPrototypeOf(function*(){})"),
                ("foreignAsyncPrototype","Object.getPrototypeOf(async function(){})"),
                ("foreignAsyncGeneratorPrototype","Object.getPrototypeOf(async function*(){})"),
                ("foreignSyntaxErrorPrototype","SyntaxError.prototype")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            i.binding_eval_in_realm(child,"Function=function(){throw 'poisoned Function'};SyntaxError=function(){throw 'poisoned SyntaxError'}").unwrap();
            assert_eq!(i.run_script_with_backend(r#"
              function check(v,l){if(!v)throw l;}
              var realmValue=19,observed=0,asyncFailures=0;
              var local=[Function,(function*(){}).constructor,(async function(){}).constructor,(async function*(){}).constructor];
              var foreign=[foreignFunction,foreignGenerator,foreignAsync,foreignAsyncGenerator];
              var expected=[foreignFunctionPrototype,foreignGeneratorPrototype,foreignAsyncPrototype,foreignAsyncGeneratorPrototype];
              var source=['return realmValue;','yield realmValue;','return realmValue;','yield realmValue;'];
              var rootN=function(){};rootN.prototype=null;
              for(var k=0;k<4;k++){
                for(var N of [foreignNewTarget,foreignNewTarget.bind(null),new Proxy(foreignNewTarget,{})]){
                  var f=Reflect.construct(local[k],[source[k]],N);
                  check(Object.getPrototypeOf(f)===expected[k],'fallback NewTarget realm/kind');
                }
                var f=Reflect.construct(foreign[k],[source[k]],rootN);
                check(Object.getPrototypeOf(f)===local[k].prototype,'callee kind/root NewTarget fallback');
                if(k===0)check(f()===73,'callee global scope');
                else if(k===1)check(f().next().value===73,'generator callee scope');
                else if(k===2)f().then(v=>{if(v===73)observed++;else asyncFailures++;},()=>asyncFailures++);
                else f().next().then(v=>{if(v.value===73)observed++;else asyncFailures++;},()=>asyncFailures++);
                var correct=false;try{foreign[k](')');}catch(e){correct=Object.getPrototypeOf(e)===foreignSyntaxErrorPrototype;}
                check(correct,'foreign syntax error immutable intrinsic');
              } true
            "#,backend).unwrap(),JsValue::Bool(true));
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("observed===2 && asyncFailures===0").unwrap(),JsValue::Bool(true));
            assert_eq!(i.binding_current_realm(),caller);
        }
    }

}
