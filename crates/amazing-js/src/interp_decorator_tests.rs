#[cfg(test)]
mod decorator_tests {
    use super::*;
    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut interpreter = Interpreter::new();
            let result = interpreter.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {}\n{source}", error.describe()));
            assert_eq!(result, JsValue::Bool(true), "{backend:?}: {source}");
        }
    }

    #[test]
    fn decorator_expressions_evaluate_once_before_reverse_application_with_receivers() {
        both(r#"
            var log='', receiver={
                get first(){log+='A';return function(v,c){log+=this===receiver?'a':'!';};},
                get second(){log+='B';return function(v,c){log+=this===receiver?'b':'!';};}
            };
            function field(v,c){log+='f';return x=>x+1;}
            @receiver.first @receiver.second class C {
                @field [(log+='K','value')]=2;
            }
            var x=new C();
            log==='ABKfba' && x.value===3;
        "#);
    }

    #[test]
    fn decorator_initializers_follow_method_field_static_and_class_order() {
        both(r#"
            var log='';
            function dec(tag){return function(v,c){
                c.addInitializer(function(){log+=tag;});
                if(c.kind==='field')return function(x){return x+1;};
            };}
            @dec('C') class C {
                @dec('F') field=(log+='f',3);
                @dec('M') method(){}
                @dec('S') static method(){}
                @dec('T') static field=(log+='t',4);
                static {log+='B';}
            }
            var instance=new C();
            log==='StTBCMfF' && C.field===5 && instance.field===4;
        "#);
    }

    #[test]
    fn field_transforms_run_outer_to_inner_and_closed_initializers_reject_calls() {
        both(r#"
            var saved, log='';
            function outer(v,c){saved=c.addInitializer;return function(x){log+='o';return x*2;};}
            function inner(v,c){c.addInitializer(function(){log+='e';});return function(x){log+='i';return x+3;};}
            class C {@outer @inner value=4;}
            var x=new C(),closed=false,bad=false;
            try {saved(()=>{});}catch(e){closed=e instanceof TypeError;}
            try {@((v,c)=>{c.addInitializer(1);}) class Bad{}}catch(e){bad=e instanceof TypeError;}
            x.value===11 && log==='oie' && closed && bad;
        "#);
    }

    #[test]
    fn public_symbol_and_private_decorator_access_preserve_brands_and_gateway_behavior() {
        both(r#"
            var access,name,staticAccess,key=Symbol('key');
            function dec(v,c){access=c.access;name=c.name;}
            class C {@dec #value=7; check(){return this.#value;} }
            var instance=new C(), branded=access.has(instance), absent=!access.has({}), failed=false;
            access.set(instance,9);
            try {access.get({});}catch(e){failed=e instanceof TypeError;}
            var privateName=name==='#value',privateGood=instance.check()===9;
            class D {@dec [key]=3;}
            var d=new D(); access.set(d,4);
            var symbolGood=name===key && access.has(d) && access.get(d)===4;
            var malformed=false; try {access.has(1);}catch(e){malformed=e instanceof TypeError;}
            class E { static before=this.#later(); @((v,c)=>{staticAccess=c.access;}) static #later(){return 2;} }
            branded && absent && failed && privateName && privateGood && symbolGood && malformed && E.before===2 && staticAccess.has(E);
        "#);
    }

    #[test]
    fn decorated_auto_accessors_use_private_storage_and_validate_replacements() {
        both(r#"
            var access;
            function dec(pair,c){access=c.access;return {
                get(){return pair.get.call(this)*2;},set(v){pair.set.call(this,v+1);},init(v){return v+3;}
            };}
            class C {@dec accessor value=4;}
            var x=new C(),initial=x.value===14,own=!Object.hasOwn(x,'value');
            access.set(x,8);
            var after=access.get(x)===18, rejected=false;
            try {C.prototype.value;}catch(e){rejected=e instanceof TypeError;}
            var invalid=false;
            try {class D {@((v,c)=>({get:3})) accessor value;}}catch(e){invalid=e instanceof TypeError;}
            initial && own && after && rejected && invalid;
        "#);
    }

    #[test]
    fn class_replacements_receive_static_initialization_and_private_methods_precede_fields() {
        both(r#"
            var original,replacement,seen;
            function dec(value,context){original=value; replacement=function(){};
                context.addInitializer(function(){seen=this;});return replacement;}
            @dec class C { static value=7; static {this.self=C;} }
            class D { value=this.#method(); #method(){return 5;} }
            C===replacement && original!==C && C.value===7 && C.self===C && seen===C && new D().value===5;
        "#);
    }
    #[test]
    fn decorator_context_shape_and_abrupt_completion_close_initializer_functions() {
        both(r#"
            var context,saved,closed=false,shape=false;
            function dec(value,c){context=c;
                shape=Object.keys(c).join(',')==='kind,access,static,private,name,addInitializer' &&
                      Object.keys(c.access).join(',')==='get,set,has' && c.access.get.length===1 &&
                      c.access.set.length===2 && c.access.has.length===1 && !Object.hasOwn(c.addInitializer,'prototype');
            }
            class C {@dec accessor value=1;}
            try {@((v,c)=>{saved=c.addInitializer;throw 0;}) class Bad{}}catch(e){}
            try {saved(()=>{});}catch(e){closed=e instanceof TypeError;}
            shape && closed && context.kind==='accessor' && context.private===false && context.static===false;
        "#);
    }

    #[test]
    fn replacement_static_initializers_keep_original_super_home_and_final_this() {
        both(r#"
            class Base {static get marker(){return this.kind;}}
            function replace(value,c){var replacement=function(){};
                Object.setPrototypeOf(replacement,{marker:'wrong'});return replacement;}
            @replace class C extends Base {static kind='final';static value=super.marker;}
            C.value==='final';
        "#);
    }

    #[test]
    fn method_and_accessor_decorators_replace_values_without_renaming_replacements() {
        both(r#"
            var methodContext,getContext,setContext;
            function method(value,c){methodContext=c;return function replacement(){return value.call(this)+1;};}
            function getter(value,c){getContext=c;return function(){return value.call(this)*2;};}
            function setter(value,c){setContext=c;return function(v){value.call(this,v+3);};}
            class C {value=4;@method method(){return this.value;}
                @getter get accessor(){return this.value;} @setter set accessor(v){this.value=v;}}
            var x=new C();x.accessor=5;
            var shape=methodContext.kind==='method' && Object.keys(methodContext.access).join(',')==='get,has' &&
                Object.keys(getContext.access).join(',')==='get,has' && Object.keys(setContext.access).join(',')==='set,has';
            shape && x.value===8 && x.accessor===16 && x.method()===9 && x.method.name==='replacement';
        "#);
    }

    #[test]
    fn class_heritage_rejects_nonconstructors_before_observable_prototype_reads() {
        both(r#"
            var arrow=()=>{},calls=0,rejected=0;
            var bound=arrow.bind(null),proxy=new Proxy(arrow,{get(){calls++;throw 1;}});
            Object.defineProperty(arrow,'prototype',{get(){calls++;throw 1;}});
            Object.defineProperty(bound,'prototype',{get(){calls++;throw 1;}});
            for(var value of [arrow,bound,proxy]){
                try {class C extends value{}}catch(e){if(e instanceof TypeError)rejected++;}
            }
            calls===0 && rejected===3;
        "#);
    }

}
