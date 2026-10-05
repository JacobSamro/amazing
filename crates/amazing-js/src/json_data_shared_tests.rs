#[cfg(test)]
mod json_data_shared_tests {
    use super::*;

    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn materialization_has_real_intrinsics_own_data_and_duplicate_order() {
        both(r#"var calls=0;Object.defineProperty(Array.prototype,'0',{set(){calls++},configurable:true});
            Object.defineProperty(Object.prototype,'poison',{set(){calls++},configurable:true});
            var a=JSON.parse('[1,2]'),o=JSON.parse('{"__proto__":{"x":1},"poison":3,"b":1,"a":1,"b":2}');
            delete Array.prototype[0];delete Object.prototype.poison;
            var d=Object.getOwnPropertyDescriptor(o,'__proto__'),ad=Object.getOwnPropertyDescriptor(a,'0');
            calls===0&&Object.getPrototypeOf(a)===Array.prototype&&Object.getPrototypeOf(o)===Object.prototype&&
            a.length===2&&a[0]===1&&ad.writable&&ad.enumerable&&ad.configurable&&
            d.value.x===1&&d.writable&&d.enumerable&&d.configurable&&o.poison===3&&o.b===2&&
            Object.keys(o).join(',')==='__proto__,poison,b,a';"#);
    }

    #[test]
    fn coercion_once_abrupt_noncallable_reviver_and_no_script_execution() {
        both(r#"var calls=0,x=0,seen;var source={toString(){calls++;return '[1]'}};
            var a=JSON.parse(source,{}),e={};try{JSON.parse({toString(){throw e}})}catch(v){seen=v}
            var bad=['(x=1)','{a:1}','[...[]]','{get x(){return 1}}','function(){}','1+2'];
            var ok=true;for(var s of bad){try{JSON.parse(s);ok=false}catch(e){ok=ok&&e instanceof SyntaxError}}
            calls===1&&a[0]===1&&seen===e&&x===0&&ok;"#);
    }

    #[test]
    fn reviver_postorder_root_source_and_latest_duplicate_span() {
        both(r#"var keys=[],sources=[],holders=[];
            var result=JSON.parse(' {"a":1,"a":2.0,"b":[-0,1e400]} ',function(k,v,c){
                keys.push(k);holders.push(this);sources.push(c.source);if(k==='0')return undefined;return v;});
            keys.join(',')==='a,0,1,b,'&&sources[0]==='2.0'&&sources[1]==='-0'&&sources[2]==='1e400'&&
            sources[3]===undefined&&sources[4]===undefined&&!(0 in result.b)&&result.b.length===2&&
            result.b[1]===Infinity&&holders[0]===result&&holders[4]['']===result;"#);
    }

    #[test]
    fn reviver_mutation_proxy_abrupt_and_failed_definitions_preserve_algorithm() {
        both(r#"var log='',source,ok=true;
            var value=JSON.parse('{"a":0,"b":1,"c":{"x":1}}',function(k,v,c){
                if(k==='a'){this.b=7;Object.defineProperty(this,'a',{value:8,writable:false,configurable:false})}
                if(k==='b')source=c.source;if(k==='x'){Object.preventExtensions(this);return 2}return v;});
            ok=ok&&value.a===8&&value.b===7&&source===undefined&&value.c.x===2;
            var token={};try{JSON.parse('[1,2]',function(k,v){if(k==='0')throw token;return v});ok=false}catch(e){ok=ok&&e===token}
            var out=JSON.parse('{"a":0,"b":[1]}',function(k,v,c){if(k==='a')this.b=new Proxy([2],{
                get(t,k,r){if(k==='length')log+='L';return Reflect.get(t,k,r)},
                defineProperty(t,k,d){log+='D';return Reflect.defineProperty(t,k,d)}});return v});
            ok&&log==='LD'&&out.b[0]===2;"#);
    }


}
