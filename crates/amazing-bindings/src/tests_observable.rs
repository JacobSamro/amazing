//! Crate-level coverage for `observable_bindings.rs`'s `notify_terminate`
//! ordering and `takeUntil()`'s notifier-event mapping. Before this file, no
//! Observable unit test existed anywhere in this crate -- the only signal on
//! this code was the WPT corpus, which is exactly how an initializer-throw
//! fix attempt regressed nine unrelated files without either gate noticing
//! (see `docs/lanes/dom-bindings_TODO.md`, "observable-takeUntil.any.js:
//! investigated..."). These tests drive `Host::call` with `V::Script`
//! closures that perform the same side effects a real JS callback would
//! (calling `subscriber.next()`/`.error()`/`.complete()`/`.addTeardown()`
//! back on the exact `Subscriber` value they were invoked with), so an
//! initializer's body can be expressed directly in Rust without a JS engine.
use super::*;

/// `Subscriber#complete()`'s step 3 ("close a subscription") runs the
/// teardown callbacks *before* the method's steps 4-5 forward the event to
/// registered observers (WICG Observable spec, "close a subscription": set
/// inactive, signal-abort, run teardowns LIFO -- all before `error()`/
/// `complete()` iterate `internal observers copy`). `h.calls` is an ordered
/// log of every plain `V::Callable` invocation, so the relative order of the
/// teardown (id 1) and the outer observer's `complete` callback (id 2)
/// directly proves which side `notify_terminate` runs first.
#[test]
fn complete_runs_teardowns_before_forwarding_to_observers() {
    let mut h = Host::default();
    let init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberAddTeardown, subscriber.clone(), &[V::Callable(1)]);
        call(host, O::SubscriberComplete, subscriber, &[]);
        Ok(V::Undefined)
    });
    let observable = call(&mut h, O::ObservableConstructor, V::Undefined, &[init]);
    let observer = V::Dictionary(vec![("complete", V::Callable(2))]);
    call(&mut h, O::ObservableSubscribe, observable, &[observer]);
    assert_eq!(h.calls, vec![1, 2],
        "teardown (1) must run before the outer complete() callback (2)");
}

/// Same ordering requirement, through `error()` instead of `complete()`.
#[test]
fn error_runs_teardowns_before_forwarding_to_observers() {
    let mut h = Host::default();
    let init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberAddTeardown, subscriber.clone(), &[V::Callable(1)]);
        call(host, O::SubscriberError, subscriber, &[str_value("boom")]);
        Ok(V::Undefined)
    });
    let observable = call(&mut h, O::ObservableConstructor, V::Undefined, &[init]);
    let observer = V::Dictionary(vec![("error", V::Callable(2))]);
    call(&mut h, O::ObservableSubscribe, observable, &[observer]);
    assert_eq!(h.calls, vec![1, 2],
        "teardown (1) must run before the outer error() callback (2)");
}

/// WICG Observable `takeUntil(notifier)`: the notifier's internal observer's
/// *error steps* run the outer Subscriber's `complete()` method, not
/// `error()` -- confirmed against the live spec text, and against
/// `observable-takeUntil.any.js`'s own asserted `'takeUntil() complete
/// callback'` result for this exact scenario (both a direct
/// `subscriber.error()` call and an equivalent synchronous `throw` in the
/// notifier -- this test covers the direct call; the crate has no JS engine
/// to drive a real `throw` from a mock callback).
#[test]
fn take_until_notifier_error_completes_the_outer_instead_of_erroring() {
    let mut h = Host::default();
    let source_init = h.script(|_host, _this, _args| Ok(V::Undefined));
    let source = call(&mut h, O::ObservableConstructor, V::Undefined, &[source_init]);
    let notifier_init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberError, subscriber, &[str_value("notifier error")]);
        Ok(V::Undefined)
    });
    let notifier = call(&mut h, O::ObservableConstructor, V::Undefined, &[notifier_init]);
    let composite = call(&mut h, O::ObservableTakeUntil, source, &[notifier]);
    let observer = V::Dictionary(vec![
        ("next", V::Callable(1)), ("error", V::Callable(2)), ("complete", V::Callable(3)),
    ]);
    call(&mut h, O::ObservableSubscribe, composite, &[observer]);
    assert_eq!(h.calls, vec![3],
        "notifier error() must run the outer's complete() (3), not next() (1) or error() (2)");
}

/// WICG Observable `takeUntil(notifier)`: the notifier completing *itself*
/// has no specified complete steps -- a no-op. The outer Observable must
/// keep mirroring the source uninterrupted (confirmed against the spec's own
/// note: "we do not specify complete steps, because if the notifier
/// Observable completes itself, we do not need to complete the subscriber
/// associated with the observable returned from this method").
#[test]
fn take_until_notifier_completing_itself_does_not_terminate_the_outer() {
    let mut h = Host::default();
    let source_init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberNext, subscriber.clone(), &[V::Number(1.)]);
        call(host, O::SubscriberComplete, subscriber, &[]);
        Ok(V::Undefined)
    });
    let source = call(&mut h, O::ObservableConstructor, V::Undefined, &[source_init]);
    let notifier_init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberComplete, subscriber, &[]);
        Ok(V::Undefined)
    });
    let notifier = call(&mut h, O::ObservableConstructor, V::Undefined, &[notifier_init]);
    let composite = call(&mut h, O::ObservableTakeUntil, source, &[notifier]);
    let observer = V::Dictionary(vec![("next", V::Callable(1)), ("complete", V::Callable(2))]);
    call(&mut h, O::ObservableSubscribe, composite, &[observer]);
    assert_eq!(h.calls, vec![1, 2],
        "the notifier completing itself is a no-op; the source's own next()/complete() must still reach the outer uninterrupted");
}

/// Guard against regressing the one notifier-event mapping that was already
/// correct: the notifier synchronously emitting a *value* completes the
/// outer and the source must never be subscribed to at all (WICG Observable
/// takeUntil() note 1: "notifier starts emitting values... we unsubscribe
/// from sourceObservable, because it no longer needs to produce values").
/// `source_init` logs marker id 99 through `host.call` directly (bypassing
/// `invoke`) if it is ever run at all, so this also proves source is never
/// subscribed to, not just that its emissions don't reach the outer.
#[test]
fn take_until_notifier_next_completes_the_outer_and_never_subscribes_source() {
    let mut h = Host::default();
    let source_init = h.script(|host, _this, _args| host.call(&V::Callable(99), &V::Undefined, &[]));
    let source = call(&mut h, O::ObservableConstructor, V::Undefined, &[source_init]);
    let notifier_init = h.script(|host, _this, args| {
        let subscriber = args[0].clone();
        call(host, O::SubscriberNext, subscriber, &[str_value("value")]);
        Ok(V::Undefined)
    });
    let notifier = call(&mut h, O::ObservableConstructor, V::Undefined, &[notifier_init]);
    let composite = call(&mut h, O::ObservableTakeUntil, source, &[notifier]);
    let observer = V::Dictionary(vec![("next", V::Callable(1)), ("complete", V::Callable(2))]);
    call(&mut h, O::ObservableSubscribe, composite, &[observer]);
    assert_eq!(h.calls, vec![2],
        "notifier emitting synchronously before source subscribes must complete the outer without ever subscribing to source");
}
