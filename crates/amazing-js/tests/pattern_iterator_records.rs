use amazing_js::interp::ExecutionBackend;
use amazing_js::value::JsValue;
use amazing_js::Interpreter;

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut interpreter = Interpreter::new();
        let result = interpreter
            .run_script_with_backend(source, backend)
            .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
        assert_eq!(result, JsValue::Bool(true), "{backend:?}");
    }
}

#[test]
fn empty_patterns_read_next_without_callability_validation_and_still_close() {
    check_both(
        r#"
        var marker={},closed=0,reads=0,body=false;
        var source={[Symbol.iterator](){return {
            get next(){reads++;return 7;},return(){closed++;throw marker;}
        };}};
        var a,b,c;try{([]=source);}catch(e){a=e===marker;}
        try{var []=source;}catch(e){b=e===marker;}
        function parameter([]){body=true;}
        try{parameter(source);}catch(e){c=e===marker;}
        a&&b&&c&&reads===3&&closed===3&&!body;
    "#,
    );
}

#[test]
fn iterator_object_validation_precedes_next_get_and_open_errors_do_not_close() {
    check_both(
        r#"
        var reads=0,marker={},closes=0;
        Object.defineProperty(Number.prototype,'next',{get(){reads++;throw marker;},configurable:true});
        var bad={[Symbol.iterator](){return 7;}},a,b;
        try{var []=bad;}catch(e){a=e instanceof TypeError;}
        var openFailure={[Symbol.iterator](){return {
            get next(){throw marker;},return(){closes++;return {};}
        };}};
        try{var []=openFailure;}catch(e){b=e===marker;}
        a&&b&&reads===0&&closes===0;
    "#,
    );
}

#[test]
fn cached_next_and_elisions_preserve_receiver_and_skip_value_getters() {
    check_both(
        r#"
        var nextReads=0,valueReads=0,steps=0,closes=0,correct=true,iterator;
        var source={[Symbol.iterator](){iterator={
            get next(){nextReads++;return function(){
                correct=correct&&this===iterator&&arguments.length===0;
                steps++;return {done:false,get value(){valueReads++;return steps;}};
            };},return(){correct=correct&&this===iterator&&arguments.length===0;closes++;return {};}
        };return iterator;}};
        var [,picked]=source;
        nextReads===1&&steps===2&&valueReads===1&&closes===1&&correct&&picked===2;
    "#,
    );
}

#[test]
fn iterator_step_abrupt_completions_mark_done_and_do_not_close() {
    check_both(
        r#"
        var marker={},closeReads=0,results=[];
        function make(mode){return {[Symbol.iterator](){return {
            next(){
                if(mode==='next')throw marker;
                if(mode==='primitive')return 1;
                return {get done(){if(mode==='done')throw marker;return false;},
                    get value(){throw marker;}};
            },get return(){closeReads++;return ()=>({});}
        };}};}
        for(var mode of ['next','done','value','primitive']){
            try{var [picked]=make(mode);}catch(e){results.push(mode==='primitive'?e instanceof TypeError:e===marker);}
        }
        results.length===4&&results.every(v=>v)&&closeReads===0;
    "#,
    );
}

#[test]
fn close_getmethod_and_result_validation_follow_normal_completion_rules() {
    check_both(
        r#"
        var marker={},seen=[];
        function source(ret){return {[Symbol.iterator](){return {next(){return {done:false,value:1};},return:ret};}};}
        for(var ret of [undefined,null,7,{},()=>0,()=>({})]){
            var success=false,error=false;
            try{var [v]=source(ret);success=true;}catch(e){error=e instanceof TypeError;}
            seen.push(ret===undefined||ret===null||typeof ret==='function'&&ret() instanceof Object?success:error);
        }
        var custom={[Symbol.iterator](){return {next(){return {done:false,value:1};},get return(){throw marker;}};}},same=false;
        try{var [value]=custom;}catch(e){same=e===marker;}
        seen.length===6&&seen.every(v=>v)&&same;
    "#,
    );
}

#[test]
fn original_abrupt_binding_completion_wins_while_nested_iterators_close_in_order() {
    check_both(
        r#"
        var marker={},closingError={},events=[],caught;
        var inner={[Symbol.iterator](){return {next(){return {done:false,value:undefined};},
            return(){events.push('inner');throw closingError;}};}};
        var outer={[Symbol.iterator](){return {next(){return {done:false,value:inner};},
            return(){events.push('outer');return {};}};}};
        function fail(){throw marker;}
        try{var [[value=fail()]]=outer;}catch(e){caught=e;}
        caught===marker&&events.join(',')==='inner,outer';
    "#,
    );
}

#[test]
fn native_array_iterator_observes_length_mutation_and_prototype_return() {
    check_both(
        r#"
        var grows=[undefined],shrinks=[undefined,2];
        var [first=(grows.push(2),1),added]=grows;
        var [other=(shrinks.length=1,1),removed=3]=shrinks;
        var prototype=Object.getPrototypeOf([][Symbol.iterator]()),closed=0,remaining;
        Object.defineProperty(prototype,'return',{configurable:true,value:function(){
            closed++;remaining=this.next().value;return {};
        }});
        var [picked]=[4,5];
        added===2&&removed===3&&picked===4&&closed===1&&remaining===5;
    "#,
    );
}

#[test]
fn native_array_next_getter_and_string_iterator_override_are_observable() {
    check_both(
        r#"
        var prototype=Object.getPrototypeOf([][Symbol.iterator]()),original=prototype.next,reads=0;
        Object.defineProperty(prototype,'next',{configurable:true,get(){reads++;return original;}});
        var [a,b]=[1,2],nativeReads=reads;
        Object.defineProperty(String.prototype,Symbol.iterator,{configurable:true,value:function(){
            return {next(){return {done:false,value:99};},return(){return {};}};
        }});
        var [value]='x';
        a===1&&b===2&&nativeReads===1&&value===99;
    "#,
    );
}

#[test]
fn suspended_assignment_targets_close_before_attempting_missing_next() {
    check_both(
        r#"
        var marker={},closes=0,reads=0;
        var source={[Symbol.iterator](){return {
            get next(){reads++;return undefined},
            return(){closes++;throw marker}
        }}};
        function* element(){[{}[yield 'element']]=source}
        function* rest(){[...{}[yield 'rest']]=source}
        var a=element(),b=rest(),caught=0;
        if(a.next().value!=='element'||b.next().value!=='rest')throw 1;
        try{a.return(1)}catch(e){if(e===marker)caught++}
        try{b.return(2)}catch(e){if(e===marker)caught++}
        caught===2&&closes===2&&reads===2;
    "#,
    );
}
