#[cfg(test)]
mod iterable_list_tests {
    use super::*;

    fn both(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            let result = i.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn eager_iterator_requires_object_results_and_an_actual_iterator_method() {
        both(r#"
            function read(x){return [...x];}
            function call(x){return function(){}(...x);}
            function make(x){return new (function(){})(...x);}
            var values=[undefined,null,false,7,'text',Symbol('s')],ok=true;
            for(var value of values) for(var consume of [read,call,make]) {
                var source={[Symbol.iterator](){return value;}};
                try{consume(source);ok=false;}catch(e){ok=ok&&e instanceof TypeError;}
            }
            for(var source of [{next(){return {done:true};}},{[Symbol.iterator]:null},
                    {[Symbol.iterator]:7},null,undefined]) {
                try{read(source);ok=false;}catch(e){ok=ok&&e instanceof TypeError;}
            }
            ok;
        "#);
    }

    #[test]
    fn eager_iterator_captures_next_without_reentering_iterator_method() {
        both(r#"
            var log=[],count=0,iterator={
                get [Symbol.iterator](){throw 'must not reenter';},
                get next(){log.push('next-get');return function(){
                    log.push(this===iterator?'next':'bad-this');
                    Object.defineProperty(iterator,'next',{value(){throw 'replacement';}});
                    return count++<2?{done:false,value:count}:{done:true};
                };}};
            var source={get [Symbol.iterator](){log.push('method-get');return function(){
                log.push(this===source?'method':'bad-source');return iterator;};}};
            var values=[...source];
            values.length===2&&values[0]===1&&values[1]===2&&
                log.join(',')==='method-get,method,next-get,next,next,next';
        "#);
    }

    #[test]
    fn eager_iterator_propagates_step_abrupt_without_closing_or_later_arguments() {
        both(r#"
            var ok=true;
            for(var phase of ['method','next-get','next-call','result','done','value']) {
                var token={},closed=0,later=0;
                var iterator={get return(){closed++;throw 'return';},get next(){
                    if(phase==='next-get')throw token;
                    return function(){if(phase==='next-call')throw token;
                        if(phase==='result')return 1;
                        return {get done(){if(phase==='done')throw token;return false;},
                            get value(){throw token;}};};}};
                var source={[Symbol.iterator](){if(phase==='method')throw token;return iterator;}};
                try{(function(){})(...source,++later);ok=false;}
                catch(e){ok=ok&&(phase==='result'?e instanceof TypeError:e===token);}
                ok=ok&&closed===0&&later===0;
            }
            ok;
        "#);
    }

    #[test]
    fn eager_iterator_done_precedes_value_and_completion_does_not_close() {
        both(r#"
            var values=0,closes=0,steps=0,iterator={
                next(){steps++;return {get done(){return {};},get value(){values++;throw 'value';}};},
                return(){closes++;throw 'close';}};
            var result=[...{[Symbol.iterator](){return iterator;}}];
            result.length===0&&values===0&&closes===0&&steps===1;
        "#);
    }

    #[test]
    fn eager_iterator_observes_array_iterator_next_and_live_length_mutations() {
        both(r#"
            var array=[1,2],saved=Object.getPrototypeOf(array.values()).next,reads=0;
            var proto=Object.getPrototypeOf(array.values());
            Object.defineProperty(array,'0',{get(){array.push(3);return 1;}});
            Object.defineProperty(proto,'next',{configurable:true,get(){reads++;return saved;}});
            var result=[...array];
            Object.defineProperty(proto,'next',{configurable:true,writable:true,value:saved});
            result.length===3&&result[2]===3&&reads===1;
        "#);
    }

    #[test]
    fn eager_iterator_honors_primitive_prototype_overrides_and_utf16_default() {
        both(r#"
            var natural=[...'A\uD83D\uDE00\uD800'],ok=natural.length===3&&
                natural[1].length===2&&natural[2].charCodeAt(0)===0xD800;
            var saved=String.prototype[Symbol.iterator];
            String.prototype[Symbol.iterator]=function(){return [this.length].values();};
            var changed=[...'abcd'];String.prototype[Symbol.iterator]=saved;
            Number.prototype[Symbol.iterator]=function(){return [Number(this)+1].values();};
            Boolean.prototype[Symbol.iterator]=function(){return [Boolean(this.valueOf())].values();};
            var number=[...6],boolean=[...false];
            delete Number.prototype[Symbol.iterator];delete Boolean.prototype[Symbol.iterator];
            ok&&changed.length===1&&changed[0]===4&&number[0]===7&&boolean[0]===false;
        "#);
    }

    #[test]
    fn eager_iterator_spread_order_receiver_construct_and_tail_calls_remain_real() {
        both(r#"
            var log=[],receiver={f(a,b,c){return this===receiver&&[a,b,c].join(',')==='1,2,3';}};
            function source(){log.push('source');var n=0;return {[Symbol.iterator](){
                log.push('iterator');return {next(){log.push('next');return ++n<3?{value:n}:{done:true};}};}};}
            function last(){log.push('last');return 3;}
            var call=receiver.f(...source(),last());
            function C(...args){this.args=args;this.target=new.target;}
            var instance=new C(...[4,5]);
            function tail(source){'use strict';return receiver.f(...source,3);}
            call&&log.join(',')==='source,iterator,next,next,next,last'&&
                instance.args.join(',')==='4,5'&&instance.target===C&&tail([1,2]);
        "#);
    }

    #[test]
    fn eager_iterator_suspension_and_array_generator_consumers_preserve_protocol() {
        both(r#"
            var gets=0;
            var source={get [Symbol.iterator](){gets++;return function*(){yield 5;yield 6;};}};
            function* outer(){yield 1;return [...source];}
            var it=outer(),first=it.next(),last=it.next();
            first.value===1&&!first.done&&last.done&&last.value.join(',')==='5,6'&&gets===1;
        "#);
    }

    #[test]
    fn eager_iterator_collections_and_typed_source_capture_original_method_once() {
        both(r#"
            var gets=0,source={get [Symbol.iterator](){gets++;return function(){return [1,2].values();};}};
            var typed=new Uint8Array(source),typedGets=gets;
            var set=new Set(source),setGets=gets;
            var map=new Map({[Symbol.iterator](){return [['x',3]].values();}});
            typed[0]===1&&typed[1]===2&&typedGets===1&&setGets===2&&set.has(2)&&map.get('x')===3;
        "#);
    }
}
