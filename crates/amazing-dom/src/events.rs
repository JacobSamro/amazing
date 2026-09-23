//! DOM Standard §2.7–2.9. The core yields callback ids; no arena borrow or JS
//! value crosses a callback. An embedding must resume or abort each dispatch.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventTarget {
    Node(NodeIdx),
    External(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CallbackId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventId(usize);
impl EventId {
    pub fn slot(self) -> usize { self.0 }
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerId(u64);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListenerOptions {
    pub capture: bool,
    pub once: bool,
    pub passive: Option<bool>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventInit {
    pub bubbles: bool,
    pub cancelable: bool,
    pub composed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPhase {
    None = 0,
    Capturing = 1,
    AtTarget = 2,
    Bubbling = 3,
}
/// Bindings expose Script dispatch only. UserAgent is for embedding-generated
/// events and does not imply a user gesture or user-activation permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchOrigin {
    Script,
    UserAgent,
}
#[derive(Debug)]
pub struct Event {
    event_type: String,
    custom: bool,
    custom_detail: Option<CallbackId>,
    init: EventInit,
    timestamp: Duration,
    trusted: bool,
    target: Option<EventTarget>,
    current_target: Option<EventTarget>,
    phase: EventPhase,
    dispatching: bool,
    stop: bool,
    immediate: bool,
    canceled: bool,
    passive: bool,
    path: Vec<EventTarget>,
}
impl Event {
    /// Timestamp is monotonic time since the embedding's time origin. The DOM
    /// model does not invent a scheduler or use wall-clock/SystemTime.
    pub fn new(event_type: impl Into<String>, init: EventInit, timestamp: Duration) -> Self {
        Self {
            event_type: event_type.into(),
            custom: false,
            custom_detail: None,
            init,
            timestamp,
            trusted: false,
            target: None,
            current_target: None,
            phase: EventPhase::None,
            dispatching: false,
            stop: false,
            immediate: false,
            canceled: false,
            passive: false,
            path: vec![],
        }
    }
    pub fn event_type(&self) -> &str {
        &self.event_type
    }
    /// A CustomEvent keeps only an opaque driver-owned value root.
    pub fn new_custom(event_type: impl Into<String>, init: EventInit, timestamp: Duration, detail: Option<CallbackId>) -> Self {
        let mut event = Self::new(event_type, init, timestamp);
        event.custom = true;
        event.custom_detail = detail;
        event
    }
    pub fn is_custom(&self) -> bool { self.custom }
    pub fn custom_detail(&self) -> Option<CallbackId> { self.custom_detail }
    /// None means dispatch is active and the initialization had no effect.
    /// Otherwise returns the previous root for the binding driver to release.
    pub fn init_custom_event(&mut self, event_type: impl Into<String>, bubbles: bool, cancelable: bool, detail: Option<CallbackId>) -> Option<Option<CallbackId>> {
        if self.dispatching || !self.custom { return None; }
        self.init_event(event_type, bubbles, cancelable);
        Some(std::mem::replace(&mut self.custom_detail, detail))
    }
    pub fn bubbles(&self) -> bool {
        self.init.bubbles
    }
    pub fn cancelable(&self) -> bool {
        self.init.cancelable
    }
    pub fn composed(&self) -> bool {
        self.init.composed
    }
    pub fn timestamp(&self) -> Duration {
        self.timestamp
    }
    pub fn is_trusted(&self) -> bool {
        self.trusted
    }
    pub fn target(&self) -> Option<EventTarget> {
        self.target
    }
    pub fn current_target(&self) -> Option<EventTarget> {
        self.current_target
    }
    pub fn phase(&self) -> EventPhase {
        self.phase
    }
    pub fn default_prevented(&self) -> bool {
        self.canceled
    }
    pub fn stop_propagation(&mut self) {
        self.stop = true;
    }
    pub fn stop_immediate_propagation(&mut self) {
        self.stop = true;
        self.immediate = true;
    }
    pub fn prevent_default(&mut self) {
        if self.init.cancelable && !self.passive {
            self.canceled = true;
        }
    }
    pub fn cancel_bubble(&self) -> bool {
        self.stop
    }
    pub fn set_cancel_bubble(&mut self, value: bool) {
        if value {
            self.stop = true;
        }
    }
    pub fn return_value(&self) -> bool {
        !self.canceled
    }
    pub fn set_return_value(&mut self, value: bool) {
        if !value {
            self.prevent_default();
        }
    }
    /// Light-tree path. A future Shadow DOM adapter must add closed-tree
    /// filtering before exposing composedPath to JS across shadow boundaries.
    pub fn composed_path(&self) -> Vec<EventTarget> {
        self.path.clone()
    }
    pub fn init_event(&mut self, event_type: impl Into<String>, bubbles: bool, cancelable: bool) {
        if self.dispatching {
            return;
        }
        self.event_type = event_type.into();
        self.init.bubbles = bubbles;
        self.init.cancelable = cancelable;
        self.stop = false;
        self.immediate = false;
        self.canceled = false;
        self.trusted = false;
        self.target = None;
    }
}

/// Pure topology hooks, evaluated before callbacks. Shadow DOM can supply
/// composed-dependent parents and adjusted targets without changing listeners.
/// Closed-root composedPath filtering and activation behavior are later seams.
pub trait EventTree {
    fn event_parent(&self, target: EventTarget, event: &Event) -> Option<EventTarget>;
    fn retarget(&self, original: EventTarget, _against: EventTarget) -> EventTarget {
        original
    }
    fn default_passive(&self, _target: EventTarget, _event_type: &str) -> bool {
        false
    }
}
impl EventTree for Dom {
    fn event_parent(&self, target: EventTarget, _event: &Event) -> Option<EventTarget> {
        match target {
            EventTarget::Node(node) => self.get(node).parent.map(EventTarget::Node),
            EventTarget::External(_) => None,
        }
    }
    fn default_passive(&self, target: EventTarget, event_type: &str) -> bool {
        if !["touchstart", "touchmove", "wheel", "mousewheel"].contains(&event_type) {
            return false;
        }
        let EventTarget::Node(node) = target else {
            return false;
        };
        if matches!(self.get(node).kind, NodeKind::Document) {
            return true;
        }
        let document = self.root(node);
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return false;
        }
        let element = self.children(document).find(|n| self.name(*n).is_some());
        if element == Some(node) {
            return true;
        }
        element.is_some_and(|e| {
            self.children(e).find(|n| {
                self.name(*n) == Some("body") && self.namespace(*n) == Some(Namespace::Html)
            }) == Some(node)
        })
    }
}
#[derive(Debug)]
struct Listener {
    id: ListenerId,
    target: EventTarget,
    event_type: String,
    callback: CallbackId,
    capture: bool,
    once: bool,
    passive: bool,
}
#[derive(Debug, Default)]
pub struct EventTargets {
    listeners: Vec<Listener>,
    next_listener: u64,
    events: Vec<Event>,
}
#[derive(Clone, Debug)]
struct PathEntry {
    invocation: EventTarget,
    adjusted_target: EventTarget,
    at_target: bool,
}
#[derive(Debug)]
#[must_use = "resume with next_listener until completion, or call abort_dispatch"]
pub struct Dispatch {
    event: EventId,
    path: Vec<PathEntry>,
    phase_step: usize,
    snapshot: Vec<ListenerId>,
    cursor: usize,
    capturing: bool,
    active_entry: Option<PathEntry>,
    finished: bool,
    exceptions: Vec<String>,
}
impl Dispatch {
    pub fn event(&self) -> EventId {
        self.event
    }
    /// Bindings report thrown callback exceptions here and continue dispatch.
    pub fn report_exception(&mut self, message: String) {
        self.exceptions.push(message);
    }
    pub fn exceptions(&self) -> &[String] {
        &self.exceptions
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventInvocation {
    pub listener: ListenerId,
    pub callback: CallbackId,
    pub event: EventId,
    pub current_target: EventTarget,
    pub target: EventTarget,
    pub phase: EventPhase,
    /// The listener has been removed from the native registry before invocation.
    pub once: bool,
}
impl EventTargets {
    pub fn listener_callbacks(&self, target: EventTarget, event_type: &str, capture: bool) -> Vec<CallbackId> {
        self.listeners.iter().filter(|listener| listener.target == target
            && listener.event_type == event_type && listener.capture == capture)
            .map(|listener| listener.callback).collect()
    }
    pub fn create_event(&mut self, event: Event) -> EventId {
        let id = EventId(self.events.len());
        self.events.push(event);
        id
    }
    pub fn event(&self, id: EventId) -> &Event {
        &self.events[id.0]
    }
    pub fn event_checked(&self, id: EventId) -> Option<&Event> {
        self.events.get(id.0)
    }
    pub fn event_mut(&mut self, id: EventId) -> &mut Event {
        &mut self.events[id.0]
    }
    pub fn add_listener(
        &mut self,
        tree: &impl EventTree,
        target: EventTarget,
        event_type: &str,
        callback: Option<CallbackId>,
        options: ListenerOptions,
    ) -> Option<ListenerId> {
        let callback = callback?;
        if let Some(existing) = self.listeners.iter().find(|l| {
            l.target == target
                && l.event_type == event_type
                && l.callback == callback
                && l.capture == options.capture
        }) {
            return Some(existing.id);
        }
        let id = ListenerId(self.next_listener);
        self.next_listener += 1;
        self.listeners.push(Listener {
            id,
            target,
            event_type: event_type.into(),
            callback,
            capture: options.capture,
            once: options.once,
            passive: options
                .passive
                .unwrap_or_else(|| tree.default_passive(target, event_type)),
        });
        Some(id)
    }
    pub fn remove_listener(
        &mut self,
        target: EventTarget,
        event_type: &str,
        callback: Option<CallbackId>,
        capture: bool,
    ) -> Vec<CallbackId> {
        let mut removed = Vec::new();
        self.listeners.retain(|l| {
            let matches = l.target == target
                && l.event_type == event_type
                && Some(l.callback) == callback
                && l.capture == capture;
            if matches { removed.push(l.callback); }
            !matches
        });
        removed
    }
    pub fn remove_listener_id(&mut self, id: ListenerId) {
        self.listeners.retain(|l| l.id != id);
    }
    pub fn remove_all_listeners(&mut self, target: EventTarget) {
        self.listeners.retain(|l| l.target != target);
    }
    pub fn begin_dispatch(
        &mut self,
        tree: &impl EventTree,
        target: EventTarget,
        id: EventId,
        origin: DispatchOrigin,
    ) -> Result<Dispatch, DomError> {
        let event = self.events.get(id.0).ok_or(DomError::InvalidStateError)?;
        if event.dispatching {
            return Err(DomError::InvalidStateError);
        }
        let mut path = vec![];
        let mut current = Some(target);
        let mut previous_adjusted = None;
        while let Some(invocation) = current {
            if path.iter().any(|p: &PathEntry| p.invocation == invocation) {
                return Err(DomError::InvalidStateError);
            }
            let adjusted_target = tree.retarget(target, invocation);
            let at_target = previous_adjusted != Some(adjusted_target);
            path.push(PathEntry {
                invocation,
                adjusted_target,
                at_target,
            });
            previous_adjusted = Some(adjusted_target);
            current = tree.event_parent(invocation, event);
        }
        let event = self.event_mut(id);
        event.dispatching = true;
        event.trusted = origin == DispatchOrigin::UserAgent;
        event.target = Some(target);
        event.path = path.iter().map(|p| p.invocation).collect();
        Ok(Dispatch {
            event: id,
            path,
            phase_step: 0,
            snapshot: vec![],
            cursor: 0,
            capturing: true,
            active_entry: None,
            finished: false,
            exceptions: vec![],
        })
    }
    /// Resume after the previous invocation returns. Listener mutations and
    /// nested dispatches are permitted between calls; the event path is frozen.
    pub fn next_listener(&mut self, dispatch: &mut Dispatch) -> Option<EventInvocation> {
        if dispatch.finished {
            return None;
        }
        self.event_mut(dispatch.event).passive = false;
        loop {
            if !self.event(dispatch.event).immediate {
                while dispatch.cursor < dispatch.snapshot.len() {
                    let id = dispatch.snapshot[dispatch.cursor];
                    dispatch.cursor += 1;
                    let Some(listener) = self.listeners.iter().find(|l| l.id == id) else {
                        continue;
                    };
                    if listener.capture != dispatch.capturing
                        || listener.event_type != self.event(dispatch.event).event_type
                    {
                        continue;
                    }
                    let callback = listener.callback;
                    let once = listener.once;
                    let passive = listener.passive;
                    if once {
                        self.remove_listener_id(id);
                    }
                    let event = self.event_mut(dispatch.event);
                    event.passive = passive;
                    let entry = dispatch.active_entry.as_ref().unwrap();
                    return Some(EventInvocation {
                        listener: id,
                        callback,
                        event: dispatch.event,
                        current_target: entry.invocation,
                        target: entry.adjusted_target,
                        phase: event.phase,
                        once,
                    });
                }
            }
            if dispatch.phase_step == dispatch.path.len() * 2 {
                self.finish_dispatch(dispatch);
                return None;
            }
            let step = dispatch.phase_step;
            dispatch.phase_step += 1;
            let capturing = step < dispatch.path.len();
            let index = if capturing {
                dispatch.path.len() - step - 1
            } else {
                step - dispatch.path.len()
            };
            let entry = dispatch.path[index].clone();
            let event = self.event_mut(dispatch.event);
            event.target = Some(entry.adjusted_target);
            if !capturing && !entry.at_target && !event.bubbles() {
                continue;
            }
            event.phase = if entry.at_target {
                EventPhase::AtTarget
            } else if capturing {
                EventPhase::Capturing
            } else {
                EventPhase::Bubbling
            };
            if event.stop {
                continue;
            }
            event.current_target = Some(entry.invocation);
            dispatch.snapshot = self
                .listeners
                .iter()
                .filter(|l| l.target == entry.invocation)
                .map(|l| l.id)
                .collect();
            dispatch.cursor = 0;
            dispatch.capturing = capturing;
            dispatch.active_entry = Some(entry);
        }
    }
    fn finish_dispatch(&mut self, dispatch: &mut Dispatch) {
        if dispatch.finished {
            return;
        }
        let event = self.event_mut(dispatch.event);
        event.phase = EventPhase::None;
        event.current_target = None;
        event.path.clear();
        event.dispatching = false;
        event.stop = false;
        event.immediate = false;
        event.passive = false;
        dispatch.finished = true;
    }
    pub fn abort_dispatch(&mut self, mut dispatch: Dispatch) {
        self.finish_dispatch(&mut dispatch);
    }
    pub fn dispatch_result(&self, id: EventId) -> Result<bool, DomError> {
        let event = self.event(id);
        if event.dispatching {
            Err(DomError::InvalidStateError)
        } else {
            Ok(!event.default_prevented())
        }
    }
}
impl Dom {
    /// Temporarily move the registry to satisfy the tree/registry borrows.
    /// The resumable dispatch returns before script runs.
    pub fn add_event_listener(
        &mut self, target: EventTarget, event_type: &str,
        callback: Option<CallbackId>, options: ListenerOptions,
    ) -> Option<ListenerId> {
        let mut events = std::mem::take(&mut self.events);
        let result = events.add_listener(self, target, event_type, callback, options);
        self.events = events;
        result
    }
    pub fn begin_event_dispatch(
        &mut self, target: EventTarget, event: EventId, origin: DispatchOrigin,
    ) -> Result<Dispatch, DomError> {
        let mut events = std::mem::take(&mut self.events);
        let result = events.begin_dispatch(self, target, event, origin);
        self.events = events;
        result
    }
}
#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
