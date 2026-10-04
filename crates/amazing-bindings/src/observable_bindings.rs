//! WHATWG/WICG `Observable` (dom/observable/tentative) plus `EventTarget.when()`.
//! The native model (amazing_dom::observable) is JS-free: it stores only
//! opaque `CallbackId` tokens. This module is the driving loop that
//! dereferences them, applies operator transforms and delivers to observers,
//! entirely outside any `Dom` borrow.
use crate::host::{AbortHost, BindingHost, CallbackId, HostCallPolicy, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{
    abort::AbortSignalId,
    events::ListenerOptions,
    observable::{ObservableId, Producer, RelayKind, RemoveOutcome, Sink, Teardown, Terminal, TerminalKind},
};

pub(crate) fn handles(op: O) -> bool {
    matches!(op,
        O::ObservableConstructor | O::ObservableSubscribe | O::ObservableMap | O::ObservableFilter
        | O::ObservableTake | O::ObservableDrop | O::ObservableFlatMap | O::ObservableSwitchMap
        | O::ObservableTakeUntil | O::ObservableForEach | O::ObservableToArray | O::ObservableFirst
        | O::ObservableLast | O::ObservableFind | O::ObservableSome | O::ObservableEvery
        | O::ObservableReduce | O::ObservableFrom | O::SubscriberNext | O::SubscriberError
        | O::SubscriberComplete | O::SubscriberAddTeardown | O::SubscriberActive | O::SubscriberSignal
        | O::EventTargetWhen | O::ObservableInternalThunk(_))
}

// ---------------------------------------------------------------- identity --
fn arena<H: BindingHost>(host: &H) -> u64 { host.node_platform_key(host.current_document()).arena }
fn observable_key<H: BindingHost>(host: &H, id: ObservableId) -> PlatformKey {
    PlatformKey { arena: arena(host), slot: id.0 as u64, generation: 1 }
}
/// `platform_object` requires `generation == 1` for every brand (a driver-wide
/// liveness invariant, not brand-specific state), so the run generation a
/// Subscriber belongs to is packed into `slot` alongside the `ObservableId`
/// instead. This lets a reference from a torn-down run detect staleness
/// instead of touching a later, unrelated run that reused the same id.
fn pack_subscriber_slot(id: ObservableId, generation: u64) -> u64 {
    (id.0 as u64) << 24 | (generation & 0xFF_FFFF)
}
fn unpack_subscriber_slot(slot: u64) -> (ObservableId, u64) {
    (ObservableId((slot >> 24) as usize), slot & 0xFF_FFFF)
}
fn subscriber_key<H: BindingHost>(host: &H, id: ObservableId, generation: u64) -> PlatformKey {
    PlatformKey { arena: arena(host), slot: pack_subscriber_slot(id, generation), generation: 1 }
}
fn observable_of<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<ObservableId, H::Error> {
    let key = host.platform_object(value, InterfaceBrand::OBSERVABLE)
        .ok_or_else(|| host.type_error("Expected an Observable"))?;
    if key.arena != arena(host) || key.generation != 1 { return Err(host.type_error("Invalid Observable")); }
    let id = ObservableId(key.slot as usize);
    host.with_dom(|dom| dom.observable_checked(id))?;
    Ok(id)
}
fn subscriber_of<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<(ObservableId, u64), H::Error> {
    let key = host.platform_object(value, InterfaceBrand::SUBSCRIBER)
        .ok_or_else(|| host.type_error("Expected a Subscriber"))?;
    if key.arena != arena(host) { return Err(host.type_error("Invalid Subscriber")); }
    let (id, generation) = unpack_subscriber_slot(key.slot);
    host.with_dom(|dom| dom.observable_checked(id))?;
    Ok((id, generation))
}
fn wrap_observable<H: BindingHost>(host: &mut H, id: ObservableId) -> Result<H::Value, H::Error> {
    host.wrap_platform_object(observable_key(host, id), InterfaceBrand::OBSERVABLE)
}
fn wrap_subscriber<H: BindingHost>(host: &mut H, id: ObservableId, generation: u64) -> Result<H::Value, H::Error> {
    host.wrap_platform_object(subscriber_key(host, id, generation), InterfaceBrand::SUBSCRIBER)
}

// -------------------------------------------------- AbortAlgorithm tokens --
// Self-describing: (ObservableId, call_id) packed directly, so no side table
// is needed. Bit 63 stays clear; a future subsystem (e.g. net's fetch/XHR
// abort algorithms) should reserve it for its own, non-colliding tokens.
fn pack_unsub(id: ObservableId, call_id: u64) -> u64 {
    ((id.0 as u64) & 0x7FFF_FFFF) << 32 | (call_id & 0xFFFF_FFFF)
}
fn unpack_unsub(token: u64) -> (ObservableId, u64) {
    (ObservableId(((token >> 32) & 0x7FFF_FFFF) as usize), token & 0xFFFF_FFFF)
}

// ------------------------------------------- JS-callable thunk tokens --
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThunkKind { EventNext, PromiseResolve, PromiseReject }
fn pack_thunk(kind: ThunkKind, id: ObservableId, generation: u64) -> u64 {
    let tag: u64 = match kind { ThunkKind::EventNext => 0, ThunkKind::PromiseResolve => 1, ThunkKind::PromiseReject => 2 };
    tag << 62 | ((id.0 as u64) & 0x3FFF_FFFF) << 32 | (generation & 0xFFFF_FFFF)
}
fn unpack_thunk(token: u64) -> (ThunkKind, ObservableId, u64) {
    let kind = match token >> 62 { 0 => ThunkKind::EventNext, 1 => ThunkKind::PromiseResolve, _ => ThunkKind::PromiseReject };
    (kind, ObservableId(((token >> 32) & 0x3FFF_FFFF) as usize), token & 0xFFFF_FFFF)
}

// -------------------------------------------------------------- delivery --
enum RelayEvent<V> { Next(V), Error(V), Complete }

/// Called from the driver's `run_abort_algorithm` when an individual
/// subscribe call's own signal (a user-supplied `options.signal`, or one of
/// our own internal relay/inner-subscription signals) fires. `reason` is the
/// signal's abort reason, reused to settle a `Terminal` sink's promise: an
/// `options.signal` abort on a terminal operator (`first()`, `toArray()`, ...)
/// must reject with that reason, not just silently drop the call.
pub fn handle_unsubscribe<H: AbortHost>(host: &mut H, token: u64, reason: &H::Value) -> Result<(), H::Error> {
    let (id, call_id) = unpack_unsub(token);
    let outcome = host.with_dom(|dom| dom.observable_remove_observer(id, call_id))?;
    let (sink, teardowns) = match outcome {
        RemoveOutcome::StillActive(sink) => (Some(sink), Vec::new()),
        RemoveOutcome::TornDown(sink, teardowns) => (Some(sink), teardowns),
        RemoveOutcome::AlreadyInactive => (None, Vec::new()),
    };
    if let Some(Sink::Terminal(term)) = sink {
        if let Some(promise) = host.root_value(term.promise) {
            host.reject_promise(&promise, reason.clone())?;
        }
    }
    if !teardowns.is_empty() {
        run_teardowns(host, teardowns)?;
    }
    Ok(())
}

fn run_teardowns<H: AbortHost>(host: &mut H, mut teardowns: Vec<Teardown>) -> Result<(), H::Error> {
    while let Some(teardown) = teardowns.pop() {
        match teardown {
            Teardown::Js(cb) => {
                if let Some(function) = host.root_value(cb) {
                    if host.is_callable(&function) {
                        let this = host.undefined();
                        host.call(&function, &this, &[])?;
                    }
                    host.release_value(cb);
                }
            }
            Teardown::AbortSignal(signal) => {
                let reason = host.undefined();
                crate::abort_bindings::propagate(host, signal, reason)?;
            }
        }
    }
    Ok(())
}

fn deliver_js<H: BindingHost>(host: &mut H, root: CallbackId, event: RelayEvent<H::Value>) -> Result<(), H::Error> {
    let Some(observer) = host.root_value(root) else { return Ok(()) };
    let (name, arg) = match event {
        RelayEvent::Next(v) => ("next", Some(v)),
        RelayEvent::Error(v) => ("error", Some(v)),
        RelayEvent::Complete => ("complete", None),
    };
    let function = if host.is_callable(&observer) {
        if name != "next" { return Ok(()); }
        observer
    } else {
        let f = host.get_property(&observer, name)?;
        if !host.is_callable(&f) { return Ok(()); }
        f
    };
    let this = host.undefined();
    let args: Vec<H::Value> = arg.into_iter().collect();
    host.call(&function, &this, &args)?;
    Ok(())
}

fn forward<H: AbortHost>(host: &mut H, owner: ObservableId, call_id: u64, sink: Sink, event: RelayEvent<H::Value>)
    -> Result<(), H::Error> {
    match sink {
        Sink::Js(root) => deliver_js(host, root, event),
        Sink::Relay(target, kind) => relay_forward(host, target, kind, event),
        Sink::Terminal(_) => terminal_forward(host, owner, call_id, event),
    }
}

/// `next()`'s snapshot iteration rule: a subscribe during delivery must not
/// receive the value already in flight.
fn notify_next<H: AbortHost>(host: &mut H, id: ObservableId, value: H::Value) -> Result<(), H::Error> {
    let snapshot = host.with_dom(|dom| dom.observable_observers_snapshot(id))?;
    for entry in snapshot {
        forward(host, id, entry.call_id, entry.sink, RelayEvent::Next(value.clone()))?;
    }
    Ok(())
}

/// `error()`/`complete()`: per WICG Observable, step 3 of both methods is
/// "close a subscription" -- set inactive (done by `observable_terminate`
/// above), signal-abort the subscription controller with the reason, then
/// run teardown callbacks in reverse insertion order -- and only steps 4-5
/// *after that* forward the event to the observers registered at the time.
/// A no-op once inactive. Verified against the live spec text (SS4's
/// `error(error)`/`complete()` algorithms and "close a subscription") after
/// a WPT observable-takeUntil.any.js test asserted a teardown ran before the
/// forwarded callback it superficially looks unrelated to -- this ordering,
/// not just the takeUntil-specific mapping below, was backwards.
fn notify_terminate<H: AbortHost>(host: &mut H, id: ObservableId, is_error: bool, value: Option<H::Value>)
    -> Result<(), H::Error> {
    let Some(termination) = host.with_dom(|dom| dom.observable_terminate(id))? else { return Ok(()) };
    let generation = host.with_dom(|dom| dom.observable_current_generation(id))?;
    let signal = host.with_dom(|dom| dom.observable_signal_for_generation(id, generation)).ok();
    if let Some(signal) = signal {
        let reason = value.clone().unwrap_or_else(|| host.undefined());
        crate::abort_bindings::propagate(host, signal, reason)?;
    }
    run_teardowns(host, termination.teardowns)?;
    for entry in termination.observers {
        let event = if is_error {
            RelayEvent::Error(value.clone().unwrap_or_else(|| host.undefined()))
        } else {
            RelayEvent::Complete
        };
        forward(host, id, entry.call_id, entry.sink, event)?;
    }
    Ok(())
}

fn relay_forward<H: AbortHost>(host: &mut H, target: ObservableId, kind: RelayKind, event: RelayEvent<H::Value>)
    -> Result<(), H::Error> {
    match (kind, event) {
        (RelayKind::Map(mapper), RelayEvent::Next(v)) => {
            let Some(f) = host.root_value(mapper) else { return Ok(()) };
            let this = host.undefined();
            let mapped = host.call(&f, &this, &[v])?;
            notify_next(host, target, mapped)
        }
        (RelayKind::Filter(pred), RelayEvent::Next(v)) => {
            let Some(f) = host.root_value(pred) else { return Ok(()) };
            let this = host.undefined();
            let keep = host.call(&f, &this, &[v.clone()])?;
            if host.to_boolean(&keep) { notify_next(host, target, v) } else { Ok(()) }
        }
        (RelayKind::Map(_) | RelayKind::Filter(_), RelayEvent::Error(e)) => notify_terminate(host, target, true, Some(e)),
        (RelayKind::Map(_) | RelayKind::Filter(_), RelayEvent::Complete) => notify_terminate(host, target, false, None),
        (RelayKind::Take, RelayEvent::Next(v)) => {
            let remaining = host.with_dom(|dom| dom.observable_remaining(target))?;
            if remaining == 0 { return Ok(()); }
            let after = host.with_dom(|dom| dom.observable_decrement_remaining(target))?;
            notify_next(host, target, v)?;
            if after == 0 { notify_terminate(host, target, false, None)?; }
            Ok(())
        }
        (RelayKind::Drop, RelayEvent::Next(v)) => {
            let remaining = host.with_dom(|dom| dom.observable_remaining(target))?;
            if remaining > 0 { host.with_dom(|dom| dom.observable_decrement_remaining(target))?; Ok(()) }
            else { notify_next(host, target, v) }
        }
        (RelayKind::Take | RelayKind::Drop, RelayEvent::Error(e)) => notify_terminate(host, target, true, Some(e)),
        (RelayKind::Take | RelayKind::Drop, RelayEvent::Complete) => notify_terminate(host, target, false, None),
        (RelayKind::FlatMapOuter(mapper), RelayEvent::Next(v)) => flat_map_spawn(host, target, mapper, v, false),
        (RelayKind::SwitchMapOuter(mapper), RelayEvent::Next(v)) => flat_map_spawn(host, target, mapper, v, true),
        (RelayKind::FlatMapOuter(_) | RelayKind::SwitchMapOuter(_), RelayEvent::Error(e)) =>
            notify_terminate(host, target, true, Some(e)),
        (RelayKind::FlatMapOuter(_) | RelayKind::SwitchMapOuter(_), RelayEvent::Complete) => {
            host.with_dom(|dom| dom.observable_set_outer_complete(target, true))?;
            if host.with_dom(|dom| dom.observable_outer_finished(target))? {
                notify_terminate(host, target, false, None)?;
            }
            Ok(())
        }
        (RelayKind::FlatMapInner | RelayKind::SwitchMapInner, RelayEvent::Next(v)) => notify_next(host, target, v),
        (RelayKind::FlatMapInner | RelayKind::SwitchMapInner, RelayEvent::Error(e)) =>
            notify_terminate(host, target, true, Some(e)),
        (RelayKind::FlatMapInner | RelayKind::SwitchMapInner, RelayEvent::Complete) => {
            host.with_dom(|dom| dom.observable_open_inner(target, -1))?;
            if host.with_dom(|dom| dom.observable_outer_finished(target))? {
                notify_terminate(host, target, false, None)?;
            }
            Ok(())
        }
        (RelayKind::TakeUntilSource, RelayEvent::Next(v)) => notify_next(host, target, v),
        (RelayKind::TakeUntilSource, RelayEvent::Error(e)) => notify_terminate(host, target, true, Some(e)),
        (RelayKind::TakeUntilSource, RelayEvent::Complete) => notify_terminate(host, target, false, None),
        // WICG Observable takeUntil(): the notifier's internal observer has
        // *next steps* and *error steps* that both run the outer Subscriber's
        // complete() method (not next()/error() -- confirmed against the live
        // spec text, and against observable-takeUntil.any.js's own asserted
        // "takeUntil() complete callback" result for both a notifier error()
        // call and a notifier throw()). It has no *complete steps* at all --
        // the spec leaves the notifier completing itself unspecified, i.e. a
        // no-op: "the observable will continue to mirror sourceObservable
        // uninterrupted". Previously this arm treated notifier Error as an
        // outer *error* and notifier Complete as an outer *complete*, both
        // backwards from spec.
        (RelayKind::TakeUntilNotifier, RelayEvent::Next(_) | RelayEvent::Error(_)) =>
            notify_terminate(host, target, false, None),
        (RelayKind::TakeUntilNotifier, RelayEvent::Complete) => Ok(()),
    }
}

fn flat_map_spawn<H: AbortHost>(host: &mut H, target: ObservableId, mapper: CallbackId, value: H::Value, is_switch: bool)
    -> Result<(), H::Error> {
    let Some(f) = host.root_value(mapper) else { return Ok(()) };
    let this = host.undefined();
    let mapped = host.call(&f, &this, &[value])?;
    let inner_id = from_value(host, &mapped)?;
    if is_switch {
        let previous = host.with_dom(|dom| dom.observable_swap_switch_inner(target, None))?;
        if let Some(previous) = previous {
            let reason = host.undefined();
            crate::abort_bindings::propagate(host, previous, reason)?;
        }
    }
    let inner_signal = host.with_dom(|dom| Ok(dom.create_abort_signal()))?;
    host.abort_signal_created(inner_signal)?;
    host.with_dom(|dom| dom.observable_push_teardown(target, Teardown::AbortSignal(inner_signal)))?;
    if is_switch {
        host.with_dom(|dom| dom.observable_swap_switch_inner(target, Some(inner_signal)))?;
    }
    host.with_dom(|dom| dom.observable_open_inner(target, 1))?;
    let kind = if is_switch { RelayKind::SwitchMapInner } else { RelayKind::FlatMapInner };
    subscribe_internal(host, inner_id, Sink::Relay(target, kind), Some(inner_signal))
}

fn terminal_forward<H: AbortHost>(host: &mut H, owner: ObservableId, call_id: u64, event: RelayEvent<H::Value>)
    -> Result<(), H::Error> {
    let term = host.with_dom(|dom| Ok(dom.observable_terminal_mut(owner, call_id)?.clone()))?;
    let promise = host.root_value(term.promise);
    match (term.kind, event) {
        (_, RelayEvent::Error(e)) => {
            if let Some(promise) = promise { host.reject_promise(&promise, e)?; }
        }
        (TerminalKind::ForEach(cb), RelayEvent::Next(v)) => {
            if let Some(f) = host.root_value(cb) {
                let index = host.number(term.index as f64);
                host.with_dom(|dom| { dom.observable_terminal_mut(owner, call_id)?.index += 1; Ok(()) })?;
                let this = host.undefined();
                host.call(&f, &this, &[v, index])?;
            }
        }
        (TerminalKind::ForEach(_), RelayEvent::Complete) => {
            if let Some(promise) = promise { let value = host.undefined(); host.resolve_promise(&promise, value)?; }
        }
        (TerminalKind::ToArray, RelayEvent::Next(v)) => {
            let root = host.retain_value(v);
            host.with_dom(|dom| { dom.observable_terminal_mut(owner, call_id)?.acc.push(root); Ok(()) })?;
        }
        (TerminalKind::ToArray, RelayEvent::Complete) => {
            let values: Vec<H::Value> = term.acc.iter().filter_map(|root| host.root_value(*root)).collect();
            for root in &term.acc { host.release_value(*root); }
            let array = host.create_array(values)?;
            if let Some(promise) = promise { host.resolve_promise(&promise, array)?; }
        }
        (TerminalKind::First, RelayEvent::Next(v)) => {
            if let Some(promise) = promise { host.resolve_promise(&promise, v)?; }
            notify_terminate(host, owner, false, None)?;
        }
        (TerminalKind::First, RelayEvent::Complete) => {
            if let Some(promise) = promise {
                let error = host.range_error_value("Observable completed without emitting any values")?;
                host.reject_promise(&promise, error)?;
            }
        }
        (TerminalKind::Last, RelayEvent::Next(v)) => {
            let root = host.retain_value(v);
            let old = host.with_dom(|dom| {
                let entry = dom.observable_terminal_mut(owner, call_id)?;
                Ok(std::mem::replace(&mut entry.acc, vec![root]))
            })?;
            for root in old { host.release_value(root); }
        }
        (TerminalKind::Last, RelayEvent::Complete) => {
            if let Some(promise) = promise {
                if let Some(root) = term.acc.first().copied() {
                    if let Some(value) = host.root_value(root) { host.resolve_promise(&promise, value)?; }
                    host.release_value(root);
                } else {
                    let error = host.range_error_value("Observable completed without emitting any values")?;
                    host.reject_promise(&promise, error)?;
                }
            }
        }
        (TerminalKind::Find(pred), RelayEvent::Next(v)) => {
            if let Some(f) = host.root_value(pred) {
                let index = host.number(term.index as f64);
                host.with_dom(|dom| { dom.observable_terminal_mut(owner, call_id)?.index += 1; Ok(()) })?;
                let this = host.undefined();
                let matched = host.call(&f, &this, &[v.clone(), index])?;
                if host.to_boolean(&matched) {
                    if let Some(promise) = promise { host.resolve_promise(&promise, v)?; }
                    notify_terminate(host, owner, false, None)?;
                }
            }
        }
        (TerminalKind::Find(_), RelayEvent::Complete) => {
            if let Some(promise) = promise { let value = host.undefined(); host.resolve_promise(&promise, value)?; }
        }
        (TerminalKind::Some(pred), RelayEvent::Next(v)) => {
            if let Some(f) = host.root_value(pred) {
                let index = host.number(term.index as f64);
                host.with_dom(|dom| { dom.observable_terminal_mut(owner, call_id)?.index += 1; Ok(()) })?;
                let this = host.undefined();
                let matched = host.call(&f, &this, &[v, index])?;
                if host.to_boolean(&matched) {
                    if let Some(promise) = promise { let value = host.boolean(true); host.resolve_promise(&promise, value)?; }
                    notify_terminate(host, owner, false, None)?;
                }
            }
        }
        (TerminalKind::Some(_), RelayEvent::Complete) => {
            if let Some(promise) = promise { let value = host.boolean(false); host.resolve_promise(&promise, value)?; }
        }
        (TerminalKind::Every(pred), RelayEvent::Next(v)) => {
            if let Some(f) = host.root_value(pred) {
                let index = host.number(term.index as f64);
                host.with_dom(|dom| { dom.observable_terminal_mut(owner, call_id)?.index += 1; Ok(()) })?;
                let this = host.undefined();
                let matched = host.call(&f, &this, &[v, index])?;
                if !host.to_boolean(&matched) {
                    if let Some(promise) = promise { let value = host.boolean(false); host.resolve_promise(&promise, value)?; }
                    notify_terminate(host, owner, false, None)?;
                }
            }
        }
        (TerminalKind::Every(_), RelayEvent::Complete) => {
            if let Some(promise) = promise { let value = host.boolean(true); host.resolve_promise(&promise, value)?; }
        }
        (TerminalKind::Reduce(reducer, _), RelayEvent::Next(v)) => {
            if let Some(f) = host.root_value(reducer) {
                if term.acc.is_empty() {
                    let root = host.retain_value(v);
                    host.with_dom(|dom| {
                        let entry = dom.observable_terminal_mut(owner, call_id)?;
                        entry.acc.push(root); entry.index += 1; Ok(())
                    })?;
                } else {
                    let acc_root = term.acc[0];
                    let acc_value = host.root_value(acc_root).unwrap_or_else(|| host.undefined());
                    let index = host.number(term.index as f64);
                    let this = host.undefined();
                    let next_acc = host.call(&f, &this, &[acc_value, v, index])?;
                    let new_root = host.retain_value(next_acc);
                    host.with_dom(|dom| {
                        let entry = dom.observable_terminal_mut(owner, call_id)?;
                        entry.acc = vec![new_root]; entry.index += 1; Ok(())
                    })?;
                    host.release_value(acc_root);
                }
            }
        }
        (TerminalKind::Reduce(_, has_initial), RelayEvent::Complete) => {
            if let Some(promise) = promise {
                if let Some(root) = term.acc.first().copied() {
                    let value = host.root_value(root).unwrap_or_else(|| host.undefined());
                    host.resolve_promise(&promise, value)?;
                } else if has_initial {
                    let value = host.undefined();
                    host.resolve_promise(&promise, value)?;
                } else {
                    let error = host.range_error_value("Reduce of empty Observable with no initial value")?;
                    host.reject_promise(&promise, error)?;
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------- subscribe --
fn subscribe_internal<H: AbortHost>(host: &mut H, id: ObservableId, sink: Sink, own_signal: Option<AbortSignalId>)
    -> Result<(), H::Error> {
    if let Some(signal) = own_signal {
        if let Some(reason_root) = host.with_dom(|dom| dom.abort_signal_reason(signal))? {
            let reason = host.root_value(reason_root).unwrap_or_else(|| host.undefined());
            return match sink {
                Sink::Js(root) => deliver_js(host, root, RelayEvent::Error(reason)),
                Sink::Terminal(term) => {
                    if let Some(promise) = host.root_value(term.promise) {
                        host.reject_promise(&promise, reason)?;
                    }
                    Ok(())
                }
                Sink::Relay(..) => Ok(()),
            };
        }
    }
    let (is_first, call_id, run_signal, generation) = host.with_dom(|dom| dom.observable_subscribe(id, sink))?;
    if is_first { host.abort_signal_created(run_signal)?; }
    if let Some(signal) = own_signal {
        let token = pack_unsub(id, call_id);
        host.with_dom(|dom| dom.add_abort_algorithm(signal, token))?;
    }
    if is_first {
        start_producer(host, id, generation, run_signal)?;
    }
    Ok(())
}

fn start_relay<H: AbortHost>(host: &mut H, id: ObservableId, source: ObservableId, kind: RelayKind) -> Result<(), H::Error> {
    let relay_signal = host.with_dom(|dom| Ok(dom.create_abort_signal()))?;
    host.abort_signal_created(relay_signal)?;
    host.with_dom(|dom| dom.observable_push_teardown(id, Teardown::AbortSignal(relay_signal)))?;
    subscribe_internal(host, source, Sink::Relay(id, kind), Some(relay_signal))
}

fn start_producer<H: AbortHost>(host: &mut H, id: ObservableId, generation: u64, run_signal: AbortSignalId)
    -> Result<(), H::Error> {
    let producer = host.with_dom(|dom| dom.observable_producer(id))?;
    match producer {
        Producer::Js(init) => {
            let Some(init_fn) = host.root_value(init) else { return Ok(()) };
            let subscriber = wrap_subscriber(host, id, generation)?;
            let this = host.undefined();
            host.call(&init_fn, &this, &[subscriber])?;
            Ok(())
        }
        Producer::Map(source, mapper) => start_relay(host, id, source, RelayKind::Map(mapper)),
        Producer::Filter(source, pred) => start_relay(host, id, source, RelayKind::Filter(pred)),
        Producer::Take(source, count) => {
            host.with_dom(|dom| dom.observable_set_remaining(id, count))?;
            if count == 0 { return notify_terminate(host, id, false, None); }
            start_relay(host, id, source, RelayKind::Take)
        }
        Producer::Drop(source, count) => {
            host.with_dom(|dom| dom.observable_set_remaining(id, count))?;
            start_relay(host, id, source, RelayKind::Drop)
        }
        Producer::FlatMap(source, mapper) => start_relay(host, id, source, RelayKind::FlatMapOuter(mapper)),
        Producer::SwitchMap(source, mapper) => start_relay(host, id, source, RelayKind::SwitchMapOuter(mapper)),
        Producer::TakeUntil(source, notifier) => {
            // The notifier subscribes first: if it synchronously emits a
            // value/error or completes, the source must never be subscribed
            // to at all (WPT observable-takeUntil.any.js).
            start_relay(host, id, notifier, RelayKind::TakeUntilNotifier)?;
            if host.with_dom(|dom| dom.observable_run_active(id))? {
                start_relay(host, id, source, RelayKind::TakeUntilSource)?;
            }
            Ok(())
        }
        Producer::FromValues(values) => {
            for root in &values {
                if !host.with_dom(|dom| dom.observable_run_active(id))? { break; }
                let Some(value) = host.root_value(*root) else { continue };
                notify_next(host, id, value)?;
            }
            if host.with_dom(|dom| dom.observable_run_active(id))? {
                notify_terminate(host, id, false, None)?;
            }
            for root in &values { host.release_value(*root); }
            Ok(())
        }
        Producer::FromPromise(promise_cb) => {
            let Some(promise) = host.root_value(promise_cb) else { return Ok(()) };
            let then = host.get_property(&promise, "then")?;
            let resolve = host.create_native_function("", 1,
                O::ObservableInternalThunk(pack_thunk(ThunkKind::PromiseResolve, id, generation)), HostCallPolicy::CallOnly)?;
            let reject = host.create_native_function("", 1,
                O::ObservableInternalThunk(pack_thunk(ThunkKind::PromiseReject, id, generation)), HostCallPolicy::CallOnly)?;
            host.call(&then, &promise, &[resolve, reject])?;
            Ok(())
        }
        Producer::EventTargetWhen { target, event_type, capture } => {
            let listener = host.create_native_function("", 1,
                O::ObservableInternalThunk(pack_thunk(ThunkKind::EventNext, id, generation)), HostCallPolicy::CallOnly)?;
            let cb = host.retain_value(listener);
            let options = ListenerOptions { capture, once: false, passive: None };
            let result = host.with_dom(|dom|
                dom.add_abortable_event_listener_utf16(target, &event_type, Some(cb), options, Some(run_signal)));
            if !matches!(result, Ok(Some(_))) { host.release_value(cb); }
            result?;
            Ok(())
        }
    }
}

/// `Observable.from()`'s coercion, reused by `flatMap`/`switchMap` for their
/// mapper's result. An Observable passes through unchanged (no wrapping); a
/// thenable is consumed via `.then`; anything else falls back to an
/// array-like read (`.length` + indexed properties) -- not the full iterator
/// protocol, so a custom iterable or async generator is not walked.
fn from_value<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<ObservableId, H::Error> {
    if let Some(key) = host.platform_object(value, InterfaceBrand::OBSERVABLE) {
        if key.arena == arena(host) && key.generation == 1 {
            return Ok(ObservableId(key.slot as usize));
        }
    }
    if host.is_object_value(value)? || host.is_callable(value) {
        let then = host.get_property(value, "then")?;
        if host.is_callable(&then) {
            let root = host.retain_value(value.clone());
            return host.with_dom(|dom| Ok(dom.create_observable(Producer::FromPromise(root))));
        }
    }
    match host.sequence_values(value) {
        Ok(values) => {
            let roots: Vec<CallbackId> = values.into_iter().map(|v| host.retain_value(v)).collect();
            host.with_dom(|dom| Ok(dom.create_observable(Producer::FromValues(roots))))
        }
        Err(_) => Err(host.type_error("Value is not observable, thenable, or array-like")),
    }
}

fn normalize_observer<H: BindingHost>(host: &mut H, arg: Option<&H::Value>) -> Result<CallbackId, H::Error> {
    let value = match arg {
        Some(v) if !host.is_undefined(v) => v.clone(),
        _ => host.create_object(None)?,
    };
    if !host.is_callable(&value) && !host.is_object_value(&value)? {
        return Err(host.type_error("Observer must be a function or an object"));
    }
    Ok(host.retain_value(value))
}

fn options_signal<H: BindingHost>(host: &mut H, options: Option<&H::Value>) -> Result<Option<AbortSignalId>, H::Error> {
    let Some(options) = options else { return Ok(None) };
    if !host.is_object_value(options)? { return Ok(None); }
    let value = host.get_property(options, "signal")?;
    if host.is_undefined(&value) { return Ok(None); }
    Ok(Some(crate::abort_bindings::signal(host, &value, InterfaceBrand::ABORT_SIGNAL)?))
}

fn start_terminal<H: AbortHost>(host: &mut H, id: ObservableId, term: Terminal, signal: Option<AbortSignalId>)
    -> Result<H::Value, H::Error> {
    let promise = host.root_value(term.promise).ok_or_else(|| host.type_error("Lost promise root"))?;
    subscribe_internal(host, id, Sink::Terminal(term), signal)?;
    Ok(promise)
}

// --------------------------------------------------------------- invoke --
pub(crate) fn invoke<H: AbortHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    match op {
        O::ObservableConstructor => {
            let init = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(init) { return Err(host.type_error("Observable constructor requires a function")); }
            let root = host.retain_value(init.clone());
            let id = host.with_dom(|dom| Ok(dom.create_observable(Producer::Js(root))))?;
            wrap_observable(host, id)
        }
        O::ObservableFrom => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let id = from_value(host, value)?;
            wrap_observable(host, id)
        }
        O::ObservableSubscribe => {
            let id = observable_of(host, &receiver)?;
            let observer_root = normalize_observer(host, args.first())?;
            let signal = options_signal(host, args.get(1))?;
            subscribe_internal(host, id, Sink::Js(observer_root), signal)?;
            Ok(host.undefined())
        }
        O::ObservableMap | O::ObservableFilter | O::ObservableFlatMap | O::ObservableSwitchMap => {
            let id = observable_of(host, &receiver)?;
            let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(callback) { return Err(host.type_error("Callback must be a function")); }
            let cb = host.retain_value(callback.clone());
            let producer = match op {
                O::ObservableMap => Producer::Map(id, cb),
                O::ObservableFilter => Producer::Filter(id, cb),
                O::ObservableFlatMap => Producer::FlatMap(id, cb),
                _ => Producer::SwitchMap(id, cb),
            };
            let new_id = host.with_dom(|dom| Ok(dom.create_observable(producer)))?;
            wrap_observable(host, new_id)
        }
        O::ObservableTake | O::ObservableDrop => {
            let id = observable_of(host, &receiver)?;
            let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let number = host.to_number(arg)?;
            let count = if !number.is_finite() || number < 0. { 0 } else { number.trunc() as u64 };
            let producer = if op == O::ObservableTake { Producer::Take(id, count) } else { Producer::Drop(id, count) };
            let new_id = host.with_dom(|dom| Ok(dom.create_observable(producer)))?;
            wrap_observable(host, new_id)
        }
        O::ObservableTakeUntil => {
            let id = observable_of(host, &receiver)?;
            let notifier_value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let notifier = from_value(host, notifier_value)?;
            let new_id = host.with_dom(|dom| Ok(dom.create_observable(Producer::TakeUntil(id, notifier))))?;
            wrap_observable(host, new_id)
        }
        O::ObservableForEach | O::ObservableFind | O::ObservableSome | O::ObservableEvery => {
            let id = observable_of(host, &receiver)?;
            let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(callback) { return Err(host.type_error("Callback must be a function")); }
            let cb = host.retain_value(callback.clone());
            let kind = match op {
                O::ObservableForEach => TerminalKind::ForEach(cb),
                O::ObservableFind => TerminalKind::Find(cb),
                O::ObservableSome => TerminalKind::Some(cb),
                _ => TerminalKind::Every(cb),
            };
            let signal = options_signal(host, args.get(1))?;
            let promise = host.new_promise()?;
            let promise_root = host.retain_value(promise);
            let term = Terminal { kind, promise: promise_root, index: 0, acc: Vec::new() };
            start_terminal(host, id, term, signal)
        }
        O::ObservableReduce => {
            let id = observable_of(host, &receiver)?;
            let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(callback) { return Err(host.type_error("Callback must be a function")); }
            let cb = host.retain_value(callback.clone());
            let has_initial = args.len() > 1;
            let signal = options_signal(host, args.get(2))?;
            let promise = host.new_promise()?;
            let promise_root = host.retain_value(promise);
            let mut acc = Vec::new();
            if has_initial { acc.push(host.retain_value(args[1].clone())); }
            let term = Terminal { kind: TerminalKind::Reduce(cb, has_initial), promise: promise_root, index: 0, acc };
            start_terminal(host, id, term, signal)
        }
        O::ObservableToArray | O::ObservableFirst | O::ObservableLast => {
            let id = observable_of(host, &receiver)?;
            let kind = match op {
                O::ObservableToArray => TerminalKind::ToArray,
                O::ObservableFirst => TerminalKind::First,
                _ => TerminalKind::Last,
            };
            let signal = options_signal(host, args.first())?;
            let promise = host.new_promise()?;
            let promise_root = host.retain_value(promise);
            let term = Terminal { kind, promise: promise_root, index: 0, acc: Vec::new() };
            start_terminal(host, id, term, signal)
        }
        O::SubscriberNext => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            if host.with_dom(|dom| dom.observable_generation_active(id, generation))? {
                let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                notify_next(host, id, value)?;
            }
            Ok(host.undefined())
        }
        O::SubscriberError => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            if host.with_dom(|dom| dom.observable_generation_active(id, generation))? {
                let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                notify_terminate(host, id, true, Some(value))?;
            }
            Ok(host.undefined())
        }
        O::SubscriberComplete => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            if host.with_dom(|dom| dom.observable_generation_active(id, generation))? {
                notify_terminate(host, id, false, None)?;
            }
            Ok(host.undefined())
        }
        O::SubscriberAddTeardown => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            let callback = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if !host.is_callable(callback) { return Ok(host.undefined()); }
            let active = host.with_dom(|dom| dom.observable_generation_active(id, generation))?;
            let cb = host.retain_value(callback.clone());
            if active {
                host.with_dom(|dom| dom.observable_push_teardown(id, Teardown::Js(cb)))?;
            } else {
                // Already inactive: run immediately (FIFO across multiple such
                // calls), distinct from the LIFO order at real teardown time.
                let this = host.undefined();
                host.call(callback, &this, &[])?;
                host.release_value(cb);
            }
            Ok(host.undefined())
        }
        O::SubscriberActive => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            let active = host.with_dom(|dom| dom.observable_generation_active(id, generation))?;
            Ok(host.boolean(active))
        }
        O::SubscriberSignal => {
            let (id, generation) = subscriber_of(host, &receiver)?;
            let signal_id = host.with_dom(|dom| dom.observable_signal_for_generation(id, generation))?;
            let key = PlatformKey { arena: arena(host), slot: signal_id.0 as u64, generation: 1 };
            host.wrap_platform_object(key, InterfaceBrand::ABORT_SIGNAL)
        }
        O::EventTargetWhen => {
            let target = crate::event_bindings::target(host, &receiver)?;
            let event_type_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let event_type = host.to_dom_string(event_type_arg)?;
            let mut capture = false;
            if let Some(options) = args.get(1) {
                if host.is_object_value(options)? {
                    let value = host.get_property(options, "capture")?;
                    capture = host.to_boolean(&value);
                } else {
                    capture = host.to_boolean(options);
                }
            }
            let producer = Producer::EventTargetWhen { target, event_type, capture };
            let id = host.with_dom(|dom| Ok(dom.create_observable(producer)))?;
            wrap_observable(host, id)
        }
        O::ObservableInternalThunk(token) => {
            let (kind, id, generation) = unpack_thunk(token);
            if host.with_dom(|dom| dom.observable_generation_active(id, generation))? {
                match kind {
                    ThunkKind::EventNext => {
                        let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                        notify_next(host, id, value)?;
                    }
                    ThunkKind::PromiseResolve => {
                        let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                        notify_next(host, id, value)?;
                        notify_terminate(host, id, false, None)?;
                    }
                    ThunkKind::PromiseReject => {
                        let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                        notify_terminate(host, id, true, Some(value))?;
                    }
                }
            }
            Ok(host.undefined())
        }
        _ => unreachable!("operation routed by handles()"),
    }
}
