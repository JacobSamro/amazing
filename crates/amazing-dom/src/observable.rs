//! WHATWG/WICG Observable native state (dom/observable/tentative). A shared,
//! ref-counted multicast producer: the first `subscribe()` call for an
//! Observable starts its producer; later concurrent calls just add another
//! observer to the same run. Teardown (LIFO) runs once the observer list
//! empties (via `complete()`/`error()`, or the last individual unsubscribe).
//! This crate never calls JS: `CallbackId` fields are opaque rooted-value
//! tokens dereferenced only by the host, outside any `Dom` borrow.
use crate::abort::AbortSignalId;
use crate::events::CallbackId;
use crate::DomError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObservableId(pub usize);

/// How a producer is started. Operators reference their upstream by id;
/// the host applies transforms while relaying, since that needs JS calls.
#[derive(Clone, Debug)]
pub enum Producer {
    /// `new Observable(initializer)`.
    Js(CallbackId),
    Map(ObservableId, CallbackId),
    Filter(ObservableId, CallbackId),
    Take(ObservableId, u64),
    Drop(ObservableId, u64),
    FlatMap(ObservableId, CallbackId),
    SwitchMap(ObservableId, CallbackId),
    TakeUntil(ObservableId, ObservableId),
    /// `Observable.from(arrayLike)`: rooted values replayed fresh per run.
    FromValues(Vec<CallbackId>),
    /// `Observable.from(thenable)`.
    FromPromise(CallbackId),
    /// `EventTarget.prototype.when(type, options)`.
    EventTargetWhen { target: crate::events::EventTarget, event_type: Vec<u16>, capture: bool },
}

/// Tag consulted by the host when relaying a value through an operator's
/// internal upstream subscription; the transform itself needs a JS call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayKind {
    Map(CallbackId),
    Filter(CallbackId),
    /// Count lives on the run's `remaining` scratch field.
    Take,
    Drop,
    FlatMapOuter(CallbackId),
    SwitchMapOuter(CallbackId),
    /// An inner subscription's own completion never completes the outer.
    FlatMapInner,
    SwitchMapInner,
    TakeUntilSource,
    /// Any value (or completion) from the notifier completes the outer.
    TakeUntilNotifier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalKind {
    ForEach(CallbackId),
    ToArray,
    First,
    Last,
    Find(CallbackId),
    Some(CallbackId),
    Every(CallbackId),
    /// Reducer callback, and whether an initial value was supplied.
    Reduce(CallbackId, bool),
}

#[derive(Clone, Debug)]
pub struct Terminal {
    pub kind: TerminalKind,
    pub promise: CallbackId,
    pub index: u64,
    /// Reduce's running accumulator (`acc[0]`) or ToArray's growing list.
    pub acc: Vec<CallbackId>,
}

#[derive(Clone, Debug)]
pub enum Sink {
    /// A normalized `{next?, error?, complete?}` observer object.
    Js(CallbackId),
    Relay(ObservableId, RelayKind),
    Terminal(Terminal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Teardown {
    Js(CallbackId),
    /// Cascades unsubscription to an internally opened upstream/inner
    /// subscription when this run ends.
    AbortSignal(AbortSignalId),
}

#[derive(Clone, Debug)]
pub struct ObserverEntry {
    pub call_id: u64,
    pub sink: Sink,
}

#[derive(Debug)]
pub struct Run {
    pub signal: AbortSignalId,
    pub observers: Vec<ObserverEntry>,
    pub teardowns: Vec<Teardown>,
    next_call_id: u64,
    /// Take/Drop: values still to let through/skip on this run's one
    /// upstream subscription.
    pub remaining: u64,
    /// SwitchMap: the currently open inner subscription's signal, aborted
    /// when a new outer value arrives or the outer run ends.
    pub switch_current_inner: Option<AbortSignalId>,
    /// FlatMap/SwitchMap: outstanding inner subscriptions; the outer only
    /// completes once its source has completed and this reaches zero.
    pub open_inner_count: u64,
    pub outer_complete: bool,
}

impl Run {
    fn new(signal: AbortSignalId) -> Self {
        Run {
            signal, observers: Vec::new(), teardowns: Vec::new(), next_call_id: 0,
            remaining: 0, switch_current_inner: None, open_inner_count: 0, outer_complete: false,
        }
    }
}

#[derive(Debug)]
struct ObservableEntry {
    producer: Producer,
    run: Option<Run>,
    /// Bumped each time a fresh run starts; lets a Subscriber wrapper from a
    /// torn-down run detect staleness and no-op instead of touching a later,
    /// unrelated run that reused the same `ObservableId`.
    generation: u64,
    /// Kept after teardown so `Subscriber.signal` keeps returning the same
    /// (now aborted) signal instead of panicking.
    last_signal: Option<AbortSignalId>,
}

#[derive(Debug, Default)]
pub struct Observables { entries: Vec<ObservableEntry> }

/// What happened to a run when one observer was removed from it. Both active
/// outcomes hand back the removed entry's `Sink` so the caller can react when
/// it was a `Terminal` (its promise needs settling on this removal, e.g. an
/// `options.signal` abort) even though the shared run lives on for others.
pub enum RemoveOutcome {
    StillActive(Sink),
    AlreadyInactive,
    /// The run's observers just emptied: run this teardown list (LIFO,
    /// outside any `Dom` borrow) then treat the producer as fully stopped.
    TornDown(Sink, Vec<Teardown>),
}

/// Returned by `terminate` (an explicit `complete()`/`error()` call): the
/// snapshot of observers to notify, and the teardown list to run after.
pub struct Termination { pub observers: Vec<ObserverEntry>, pub teardowns: Vec<Teardown> }

impl crate::Dom {
    pub fn create_observable(&mut self, producer: Producer) -> ObservableId {
        let id = ObservableId(self.observables.entries.len());
        self.observables.entries.push(ObservableEntry { producer, run: None, generation: 0, last_signal: None });
        id
    }
    pub fn observable_producer(&self, id: ObservableId) -> Result<Producer, DomError> {
        Ok(self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?.producer.clone())
    }
    pub fn observable_checked(&self, id: ObservableId) -> Result<(), DomError> {
        if self.observables.entries.get(id.0).is_some() { Ok(()) } else { Err(DomError::InvalidStateError) }
    }
    pub fn observable_run_active(&self, id: ObservableId) -> Result<bool, DomError> {
        Ok(self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?.run.is_some())
    }
    /// A Subscriber's generation and signal never change after creation, so
    /// they are read once by the wrapper's `PlatformKey`, not re-derived.
    pub fn observable_current_generation(&self, id: ObservableId) -> Result<u64, DomError> {
        Ok(self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?.generation)
    }
    pub fn observable_generation_active(&self, id: ObservableId, generation: u64) -> Result<bool, DomError> {
        let entry = self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?;
        Ok(entry.generation == generation && entry.run.is_some())
    }
    pub fn observable_signal_for_generation(&self, id: ObservableId, generation: u64) -> Result<AbortSignalId, DomError> {
        let entry = self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?;
        if entry.generation != generation { return Err(DomError::InvalidStateError); }
        entry.last_signal.ok_or(DomError::InvalidStateError)
    }
    /// Adds one observer, starting a fresh run first if none is active.
    /// Returns whether this call started the run (caller must then drive
    /// producer startup), this call's id (for later removal) and the run's
    /// internal signal (this run's `Subscriber.signal`, and this call's
    /// generation for building the Subscriber's `PlatformKey`).
    pub fn observable_subscribe(&mut self, id: ObservableId, sink: Sink)
        -> Result<(bool, u64, AbortSignalId, u64), DomError> {
        if self.observables.entries.get(id.0).is_none() { return Err(DomError::InvalidStateError); }
        let is_first = self.observables.entries[id.0].run.is_none();
        if is_first {
            let signal = self.create_abort_signal();
            let entry = &mut self.observables.entries[id.0];
            entry.run = Some(Run::new(signal));
            entry.generation += 1;
            entry.last_signal = Some(signal);
        }
        let entry = &mut self.observables.entries[id.0];
        let generation = entry.generation;
        let run = entry.run.as_mut().unwrap();
        let call_id = run.next_call_id;
        run.next_call_id += 1;
        let signal = run.signal;
        run.observers.push(ObserverEntry { call_id, sink });
        Ok((is_first, call_id, signal, generation))
    }
    pub fn observable_remove_observer(&mut self, id: ObservableId, call_id: u64) -> Result<RemoveOutcome, DomError> {
        let entry = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?;
        let Some(run) = entry.run.as_mut() else { return Ok(RemoveOutcome::AlreadyInactive) };
        let Some(position) = run.observers.iter().position(|o| o.call_id == call_id) else {
            return Ok(RemoveOutcome::AlreadyInactive);
        };
        let removed = run.observers.remove(position);
        if run.observers.is_empty() {
            let run = entry.run.take().unwrap();
            Ok(RemoveOutcome::TornDown(removed.sink, run.teardowns))
        } else {
            Ok(RemoveOutcome::StillActive(removed.sink))
        }
    }
    /// `complete()`/`error()`: a no-op once already inactive.
    pub fn observable_terminate(&mut self, id: ObservableId) -> Result<Option<Termination>, DomError> {
        let entry = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?;
        let Some(run) = entry.run.take() else { return Ok(None) };
        Ok(Some(Termination { observers: run.observers, teardowns: run.teardowns }))
    }
    pub fn observable_push_teardown(&mut self, id: ObservableId, teardown: Teardown) -> Result<(), DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        run.teardowns.push(teardown);
        Ok(())
    }
    pub fn observable_pop_teardown(&mut self, id: ObservableId) -> Result<Option<Teardown>, DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        Ok(run.teardowns.pop())
    }
    /// A snapshot for `next()`'s iteration rule: a subscribe during delivery
    /// must not receive the in-flight value.
    pub fn observable_observers_snapshot(&self, id: ObservableId) -> Result<Vec<ObserverEntry>, DomError> {
        let entry = self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?;
        Ok(entry.run.as_ref().map(|run| run.observers.clone()).unwrap_or_default())
    }
    pub fn observable_set_remaining(&mut self, id: ObservableId, remaining: u64) -> Result<(), DomError> {
        self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?.remaining = remaining;
        Ok(())
    }
    pub fn observable_remaining(&self, id: ObservableId) -> Result<u64, DomError> {
        Ok(self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_ref().ok_or(DomError::InvalidStateError)?.remaining)
    }
    pub fn observable_decrement_remaining(&mut self, id: ObservableId) -> Result<u64, DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        run.remaining = run.remaining.saturating_sub(1);
        Ok(run.remaining)
    }
    pub fn observable_swap_switch_inner(&mut self, id: ObservableId, inner: Option<AbortSignalId>)
        -> Result<Option<AbortSignalId>, DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        Ok(std::mem::replace(&mut run.switch_current_inner, inner))
    }
    pub fn observable_open_inner(&mut self, id: ObservableId, delta: i64) -> Result<u64, DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        run.open_inner_count = (run.open_inner_count as i64 + delta).max(0) as u64;
        Ok(run.open_inner_count)
    }
    pub fn observable_set_outer_complete(&mut self, id: ObservableId, value: bool) -> Result<(), DomError> {
        self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?.outer_complete = value;
        Ok(())
    }
    /// Whether the source+inners have both finished (flatMap/switchMap).
    pub fn observable_outer_finished(&self, id: ObservableId) -> Result<bool, DomError> {
        let run = self.observables.entries.get(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_ref().ok_or(DomError::InvalidStateError)?;
        Ok(run.outer_complete && run.open_inner_count == 0)
    }
    pub fn observable_terminal_mut(&mut self, id: ObservableId, call_id: u64)
        -> Result<&mut Terminal, DomError> {
        let run = self.observables.entries.get_mut(id.0).ok_or(DomError::InvalidStateError)?
            .run.as_mut().ok_or(DomError::InvalidStateError)?;
        run.observers.iter_mut().find(|o| o.call_id == call_id)
            .and_then(|o| match &mut o.sink { Sink::Terminal(terminal) => Some(terminal), _ => None })
            .ok_or(DomError::InvalidStateError)
    }
}
