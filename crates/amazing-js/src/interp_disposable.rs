// Portions ported from lucid-softworks/lumen (crates/lumen/src/builtins/disposable.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//
// `DisposableStack` / `AsyncDisposableStack` (explicit resource management, spec §11.4).
// Resources are stored as `DisposableStackEntry` records on `Exotic::DisposableStack` and run
// LIFO on `dispose()` / `disposeAsync()`, folding multiple disposal errors into a `SuppressedError`
// chain via the same rule `Interpreter::dispose_frame` uses for `using`/`await using` statements.
// Design ported from lumen, adapted onto our `Exotic`-variant internal-slot convention (rather
// than string-keyed properties) and simplified: our `await_value` already passes a non-promise
// value through untouched, so a sync `@@dispose` fallback used by an `AsyncDisposableStack` needs
// no wrapper closure to make its result inert before awaiting it.
//
// `use`/`move` are hint- (i.e. method-, not receiver-) determined per spec (a `DisposableStack`'s
// `use` always resolves `@@dispose` only, and its `move` always creates a fresh `DisposableStack`,
// even if invoked on/for an `AsyncDisposableStack` receiver via `Function.prototype.call`), so each
// gets a distinct native function per stack kind rather than branching on the receiver's own state.
// `adopt`/`defer`/`disposed` only need the generic `[[DisposableState]]`-slot brand check, which
// either kind of receiver satisfies (RequireInternalSlot in the spec has no kind-specific brand).

fn require_disposable_state(this: &JsValue) -> Result<Obj, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("receiver does not have a [[DisposableState]] internal slot"));
    };
    if !matches!(&object.borrow().exotic, Exotic::DisposableStack(_)) {
        return Err(Throw::type_error("receiver does not have a [[DisposableState]] internal slot"));
    }
    Ok(object.clone())
}

fn disposable_stack_disposed(object: &Obj) -> bool {
    matches!(&object.borrow().exotic, Exotic::DisposableStack(state) if state.disposed)
}

impl Interpreter {
    /// AddDisposableResource for a stack's `use(value)`: resolve `value`'s dispose method (async
    /// falling back to sync for `AsyncDisposableStack`, else sync-only) and push it. A
    /// null/undefined `value` is simply not disposable-tracked, matching the statement-level
    /// `add_disposable` (see interp.rs) for the same reason: without real coroutines, the spec's
    /// async-hint sentinel-await has no observable effect in this interpreter.
    fn disposable_stack_push_use(&mut self, object: &Obj, value: JsValue, is_async: bool) -> Result<(), Throw> {
        if matches!(value, JsValue::Null | JsValue::Undefined) {
            return Ok(());
        }
        let (method, _) = self.dispose_method(&value, is_async)?;
        if let Exotic::DisposableStack(state) = &mut object.borrow_mut().exotic {
            state.resources.push(DisposableStackEntry { method, this_arg: value, arg: None });
        }
        Ok(())
    }

    fn disposable_stack_push_callback(&mut self, object: &Obj, on_dispose: JsValue, arg: Option<JsValue>) {
        if let Exotic::DisposableStack(state) = &mut object.borrow_mut().exotic {
            state.resources.push(DisposableStackEntry { method: on_dispose, this_arg: JsValue::Undefined, arg });
        }
    }

    /// DisposeResources over a stack's own resource list: every entry runs, in reverse
    /// registration order; `await_each` awaits every call's result (an `AsyncDisposableStack`'s
    /// `disposeAsync`), never only some of them — unlike the per-declaration `method_is_async`
    /// distinction `dispose_frame` makes for `using`/`await using` statements, an
    /// AsyncDisposableStack's hint is uniformly async-dispose for its whole resource list.
    fn dispose_stack_entries(&mut self, entries: Vec<DisposableStackEntry>, await_each: bool) -> Result<(), Throw> {
        let mut completion: Result<(), Throw> = Ok(());
        for entry in entries.into_iter().rev() {
            let args = entry.arg.map(|a| vec![a]).unwrap_or_default();
            let disposed = self
                .call_value(&entry.method, entry.this_arg, args)
                .and_then(|v| if await_each { self.await_value(v).map(|_| ()) } else { Ok(()) });
            if let Err(new_err) = disposed {
                completion = match completion {
                    Err(prev) => Err(Throw(self.make_suppressed_error(new_err.0, prev.0))),
                    Ok(()) => Err(new_err),
                };
            }
        }
        completion
    }

    fn install_disposable_stack(&mut self) {
        for is_async in [false, true] {
            let name = if is_async { "AsyncDisposableStack" } else { "DisposableStack" };
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(self.object_proto.clone());
            let prototype = Obj(Rc::new(RefCell::new(prototype)));

            let constructor: crate::value::NativeFn = if is_async {
                native_async_disposable_stack_constructor
            } else {
                native_disposable_stack_constructor
            };
            let constructor = Self::native(name, constructor, &self.function_proto);
            if let JsValue::Object(object) = &constructor {
                object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                object.borrow_mut().define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
                object.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
            }
            prototype.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(name), false, true)).unwrap();

            let (use_fn, move_fn, dispose_name, dispose_fn): (crate::value::NativeFn, crate::value::NativeFn, &str, crate::value::NativeFn) = if is_async {
                (native_disposable_stack_use_async, native_disposable_stack_move_async, "disposeAsync", native_async_disposable_stack_dispose)
            } else {
                (native_disposable_stack_use_sync, native_disposable_stack_move_sync, "dispose", native_disposable_stack_dispose)
            };
            let methods: Vec<(&str, u32, crate::value::NativeFn)> = vec![
                ("use", 1, use_fn),
                ("adopt", 2, native_disposable_stack_adopt),
                ("defer", 1, native_disposable_stack_defer),
                (dispose_name, 0, dispose_fn),
                ("move", 0, move_fn),
            ];
            for (method_name, length, native) in methods {
                prototype.borrow_mut().define(method_name,
                    symbol_data(std_collection_method(self, method_name, length, native), true, true)).unwrap();
            }
            let disposed_getter = std_collection_method(self, "get disposed", 0, native_disposable_stack_disposed_getter);
            prototype.borrow_mut().define("disposed", Property {
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: Some(disposed_getter), setter: None,
            }).unwrap();
            // `[Symbol.dispose]`/`[Symbol.asyncDispose]` alias the primary dispose method (same
            // function object, per spec, so `stack[Symbol.dispose] === stack.dispose`).
            let alias_symbol = if is_async { WellKnownSymbol::AsyncDispose.symbol() } else { WellKnownSymbol::Dispose.symbol() };
            let primary = prototype.borrow().get_own(dispose_name);
            if let Some(primary) = primary {
                prototype.borrow_mut().define(alias_symbol, symbol_data(primary, true, true)).unwrap();
            }

            self.define_global(name, constructor);
            if let Some(value) = self.global_env.resolve(name) {
                self.global_obj.borrow_mut().define(name, symbol_data(value, true, true)).unwrap();
            }
        }
    }
}

fn native_disposable_stack_constructor(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Constructor DisposableStack requires 'new'")); };
    object.borrow_mut().exotic = Exotic::DisposableStack(DisposableStackState { is_async: false, disposed: false, resources: Vec::new() });
    Ok(this)
}

fn native_async_disposable_stack_constructor(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Constructor AsyncDisposableStack requires 'new'")); };
    object.borrow_mut().exotic = Exotic::DisposableStack(DisposableStackState { is_async: true, disposed: false, resources: Vec::new() });
    Ok(this)
}

fn disposable_stack_use_impl(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, is_async: bool) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    if disposable_stack_disposed(&object) {
        return Err(Throw::reference_error("resource stack is already disposed"));
    }
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    i.disposable_stack_push_use(&object, value.clone(), is_async)?;
    Ok(value)
}
fn native_disposable_stack_use_sync(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    disposable_stack_use_impl(i, this, args, false)
}
fn native_disposable_stack_use_async(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    disposable_stack_use_impl(i, this, args, true)
}

fn native_disposable_stack_adopt(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    if disposable_stack_disposed(&object) {
        return Err(Throw::reference_error("resource stack is already disposed"));
    }
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let on_dispose = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !i.binding_is_callable(&on_dispose) {
        return Err(Throw::type_error("onDispose is not callable"));
    }
    i.disposable_stack_push_callback(&object, on_dispose, Some(value.clone()));
    Ok(value)
}

fn native_disposable_stack_defer(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    if disposable_stack_disposed(&object) {
        return Err(Throw::reference_error("resource stack is already disposed"));
    }
    let on_dispose = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !i.binding_is_callable(&on_dispose) {
        return Err(Throw::type_error("onDispose is not callable"));
    }
    i.disposable_stack_push_callback(&object, on_dispose, None);
    Ok(JsValue::Undefined)
}

fn native_disposable_stack_dispose(i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    if disposable_stack_disposed(&object) {
        return Ok(JsValue::Undefined);
    }
    let entries = match &mut object.borrow_mut().exotic {
        Exotic::DisposableStack(state) => { state.disposed = true; std::mem::take(&mut state.resources) }
        _ => Vec::new(),
    };
    i.dispose_stack_entries(entries, false)?;
    Ok(JsValue::Undefined)
}

fn native_async_disposable_stack_dispose(i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    // `disposeAsync` never throws synchronously: even a brand failure rejects the returned
    // promise (spec wraps the whole body as an async abstract operation).
    let object = match require_disposable_state(&this) {
        Ok(object) => object,
        Err(Throw(error)) => return Ok(new_promise(i, PromiseFate::Rejected(error))),
    };
    if disposable_stack_disposed(&object) {
        return Ok(new_promise(i, PromiseFate::Fulfilled(JsValue::Undefined)));
    }
    let entries = match &mut object.borrow_mut().exotic {
        Exotic::DisposableStack(state) => { state.disposed = true; std::mem::take(&mut state.resources) }
        _ => Vec::new(),
    };
    Ok(match i.dispose_stack_entries(entries, true) {
        Ok(()) => new_promise(i, PromiseFate::Fulfilled(JsValue::Undefined)),
        Err(Throw(error)) => new_promise(i, PromiseFate::Rejected(error)),
    })
}

fn disposable_stack_move_impl(i: &mut Interpreter, this: JsValue, is_async: bool, proto_name: &str) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    if disposable_stack_disposed(&object) {
        return Err(Throw::reference_error("resource stack is already disposed"));
    }
    let entries = match &mut object.borrow_mut().exotic {
        Exotic::DisposableStack(state) => std::mem::take(&mut state.resources),
        _ => Vec::new(),
    };
    if let Exotic::DisposableStack(state) = &mut object.borrow_mut().exotic {
        state.disposed = true;
    }
    let proto = i.intrinsic_constructors.get(proto_name).map(|(_, proto)| proto.clone()).unwrap_or_else(|| i.object_proto.clone());
    let mut fresh = JsObject::ordinary();
    fresh.proto = Some(proto);
    fresh.exotic = Exotic::DisposableStack(DisposableStackState { is_async, disposed: false, resources: entries });
    Ok(JsValue::Object(Obj(Rc::new(RefCell::new(fresh)))))
}
fn native_disposable_stack_move_sync(i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    disposable_stack_move_impl(i, this, false, "DisposableStack")
}
fn native_disposable_stack_move_async(i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    disposable_stack_move_impl(i, this, true, "AsyncDisposableStack")
}

fn native_disposable_stack_disposed_getter(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = require_disposable_state(&this)?;
    Ok(JsValue::Bool(disposable_stack_disposed(&object)))
}
