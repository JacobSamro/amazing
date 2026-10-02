// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// PerformPromiseThen adapted to Amazing's native reaction lists and scheduler.

fn perform_promise_then(
    i: &mut Interpreter,
    promise: &Obj,
    on_fulfilled: Option<JsValue>,
    on_rejected: Option<JsValue>,
    result_capability: Option<Obj>,
) -> Result<(), Throw> {
    let fulfilled = PendingReaction {
        handler: on_fulfilled.filter(|value| callable_function(value).is_some()),
        capability: result_capability.clone(),
        is_reject_handler: false,
    };
    let rejected = PendingReaction {
        handler: on_rejected.filter(|value| callable_function(value).is_some()),
        capability: result_capability,
        is_reject_handler: true,
    };
    let fate = {
        let mut object = promise.borrow_mut();
        let Exotic::Promise(state) = &mut object.exotic else {
            return Err(Throw::type_error("internal reaction requires a Promise"));
        };
        if matches!(state.state, PromiseFate::Pending) {
            state.fulfill_reactions.push(fulfilled);
            state.reject_reactions.push(rejected);
            return Ok(());
        }
        state.state.clone()
    };
    let reaction = if matches!(fate, PromiseFate::Fulfilled(_)) { fulfilled } else { rejected };
    i.enqueue_promise_job(PromiseJob { reaction, fate })
}

#[cfg(test)]
mod internal_promise_reaction_tests {
    use super::*;

    #[test]
    fn internal_reactions_ignore_observable_then_and_species_and_preserve_order() {
        for fate in [PromiseFate::Pending, PromiseFate::Fulfilled(JsValue::num(7.0)),
            PromiseFate::Rejected(JsValue::num(7.0))] {
            let mut i = Interpreter::new();
            let value = new_promise(&i, fate.clone());
            let promise = promise_object(&value).unwrap();
            i.global_env.declare_var("p", value).unwrap();
            i.run_script("var seen=[];Object.defineProperty(p,'then',{get(){throw 99}});Object.defineProperty(p,'constructor',{get(){throw 98}});").unwrap();
            let handlers = ["v=>{seen.push('a'+v)}", "v=>{seen.push('b'+v)}"]
                .map(|source| i.run_script(source).unwrap());
            for handler in handlers {
                perform_promise_then(&mut i, &promise, Some(handler.clone()), Some(handler), None).unwrap();
            }
            let seen = i.global_env.resolve("seen").unwrap();
            assert_eq!(i.get_prop(&seen, "length").unwrap(), JsValue::num(0.0));
            if matches!(fate, PromiseFate::Pending) {
                let state = promise.borrow();
                let Exotic::Promise(state) = &state.exotic else { unreachable!() };
                assert!(state.fulfill_reactions.iter().all(|reaction| reaction.capability.is_none()));
            }
            if matches!(fate, PromiseFate::Pending) {
                settle_promise(&promise, PromiseFate::Fulfilled(JsValue::num(7.0)), &mut i).unwrap();
            }
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("seen.join(',')").unwrap(), JsValue::str("a7,b7"));
        }
    }

    #[test]
    fn internal_reaction_does_not_assimilate_handler_return_values() {
        let mut i = Interpreter::new();
        let value = new_promise(&i, PromiseFate::Fulfilled(JsValue::Undefined));
        let promise = promise_object(&value).unwrap();
        i.run_script("var reads=0;var returned={get then(){reads++;throw 99}};").unwrap();
        let handler = i.run_script("()=>returned").unwrap();
        perform_promise_then(&mut i, &promise, Some(handler), None, None).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("reads").unwrap(), JsValue::num(0.0));
    }
}
