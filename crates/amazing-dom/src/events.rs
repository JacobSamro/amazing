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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventClass { Event, Custom, Ui, Focus, Mouse, Wheel, Keyboard, Composition, Message }

#[derive(Clone, Debug)]
pub enum EventExtra { Boolean(bool), Number(f64), String(Vec<u16>), Root(Option<CallbackId>) }

/// Message payload roots belong to the embedding, not the native DOM arena.
#[derive(Clone, Debug, Default)]
pub struct MessageEventData {
    pub data: Option<CallbackId>,
    pub origin: Vec<u16>,
    pub last_event_id: Vec<u16>,
    pub source: Option<CallbackId>,
    pub ports: Option<CallbackId>,
}

#[derive(Debug, Clone)]
pub struct Event {
    event_type: String,
    initialized: bool,
    class: EventClass,
    extras: std::collections::HashMap<&'static str, EventExtra>,
    message: Option<MessageEventData>,
    custom: bool,
    custom_detail: Option<CallbackId>,
    init: EventInit,
    timestamp: Duration,
    trusted: bool,
    target: Option<EventTarget>,
    related_target: Option<EventTarget>,
    current_target: Option<EventTarget>,
    phase: EventPhase,
    dispatching: bool,
    stop: bool,
    immediate: bool,
    canceled: bool,
    passive: bool,
    path: Vec<EventTarget>,
    path_closed_roots: Vec<Vec<NodeIdx>>,
}
impl Event {
    /// Timestamp is monotonic time since the embedding's time origin. The DOM
    /// model does not invent a scheduler or use wall-clock/SystemTime.
    pub fn new(event_type: impl Into<String>, init: EventInit, timestamp: Duration) -> Self {
        Self {
            event_type: event_type.into(),
            initialized: true,
            class: EventClass::Event,
            extras: std::collections::HashMap::new(),
            message: None,
            custom: false,
            custom_detail: None,
            init,
            timestamp,
            trusted: false,
            target: None,
            related_target: None,
            current_target: None,
            phase: EventPhase::None,
            dispatching: false,
            stop: false,
            immediate: false,
            canceled: false,
            passive: false,
            path: vec![],
            path_closed_roots: vec![],
        }
    }
    pub fn event_type(&self) -> &str {
        &self.event_type
    }
    pub fn class(&self) -> EventClass { self.class }
    pub fn set_class(&mut self, class: EventClass) { self.class = class; }
    pub fn extra(&self, name: &'static str) -> Option<&EventExtra> { self.extras.get(name) }
    pub fn set_extra(&mut self, name: &'static str, value: EventExtra) { self.extras.insert(name, value); }
    /// Legacy `Document.createEvent()` leaves this flag unset until initEvent.
    pub fn mark_uninitialized(&mut self) { self.initialized = false; }
    /// A CustomEvent keeps only an opaque driver-owned value root.
    pub fn new_custom(event_type: impl Into<String>, init: EventInit, timestamp: Duration, detail: Option<CallbackId>) -> Self {
        let mut event = Self::new(event_type, init, timestamp);
        event.custom = true;
        event.class = EventClass::Custom;
        event.custom_detail = detail;
        event
    }
    pub fn new_message(event_type: impl Into<String>, init: EventInit,
        timestamp: Duration, data: MessageEventData) -> Self {
        let mut event = Self::new(event_type, init, timestamp);
        event.class = EventClass::Message;
        event.message = Some(data);
        event
    }
    pub fn message_data(&self) -> Option<&MessageEventData> { self.message.as_ref() }
    /// Returns old roots for the embedding to release; no change while dispatching.
    pub fn init_message_event(&mut self, event_type: impl Into<String>, bubbles: bool,
        cancelable: bool, data: MessageEventData) -> Option<MessageEventData> {
        if self.dispatching || self.message.is_none() { return None; }
        self.init_event(event_type, bubbles, cancelable);
        self.message.replace(data)
    }
    pub fn set_message_ports_root(&mut self, root: CallbackId) {
        if let Some(message) = &mut self.message { message.ports = Some(root); }
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
    pub fn related_target(&self) -> Option<EventTarget> { self.related_target }
    pub fn set_related_target(&mut self, target: Option<EventTarget>) {
        if !self.dispatching { self.related_target = target; }
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
    /// The path and its closed-root membership are frozen before callbacks.
    /// A listener outside a closed root cannot observe entries inside it.
    pub fn composed_path(&self) -> Vec<EventTarget> {
        let Some(current) = self.current_target else { return vec![] };
        let Some(index) = self.path.iter().position(|target| *target == current) else { return vec![] };
        let visible = self.path_closed_roots.get(index).map(Vec::as_slice).unwrap_or(&[]);
        self.path.iter().enumerate().filter(|(index, _)| self.path_closed_roots.get(*index)
            .is_none_or(|roots| roots.iter().all(|root| visible.contains(root))))
            .map(|(_, target)| *target).collect()
    }
    pub(crate) fn raw_path(&self) -> &[EventTarget] { &self.path }
    pub(crate) fn set_closed_path_roots(&mut self, roots: Vec<Vec<NodeIdx>>) { self.path_closed_roots = roots; }
    pub fn init_event(&mut self, event_type: impl Into<String>, bubbles: bool, cancelable: bool) {
        if self.dispatching {
            return;
        }
        self.event_type = event_type.into();
        self.initialized = true;
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
/// The path and shadow membership are captured before any callback runs.
pub trait EventTree {
    fn event_parent(&self, target: EventTarget, event: &Event) -> Option<EventTarget>;
    fn retarget(&self, original: EventTarget, _against: EventTarget) -> EventTarget {
        original
    }
    fn is_in_shadow_tree(&self, _target: EventTarget) -> bool { false }
    fn default_passive(&self, _target: EventTarget, _event_type: &str) -> bool {
        false
    }
}
impl EventTree for Dom {
    fn event_parent(&self, target: EventTarget, event: &Event) -> Option<EventTarget> {
        match target {
            EventTarget::Node(node) => {
                if let Some(slot) = self.find_slot(node, false) { return Some(EventTarget::Node(slot)); }
                if let Some(parent) = self.get(node).parent { return Some(EventTarget::Node(parent)); }
                if matches!(self.get(node).kind, NodeKind::Document) && node == crate::NodeIdx(0) {
                    return self.window_event_target().map(EventTarget::External);
                }
                let root = self.shadow_data(node)?;
                let originated_inside = event.target().is_some_and(|target|
                    matches!(target, EventTarget::Node(original) if self.root(original) == node));
                if event.composed() || !originated_inside { Some(EventTarget::Node(root.host)) }
                else { None }
            }
            EventTarget::External(_) => None,
        }
    }
    fn retarget(&self, original: EventTarget, against: EventTarget) -> EventTarget {
        let EventTarget::Node(mut node) = original else { return original };
        loop {
            let root = self.root(node);
            let Some(data) = self.shadow_data(root) else { return EventTarget::Node(node) };
            if matches!(against, EventTarget::Node(reference)
                if self.shadow_including_ancestors(reference).contains(&root)) { return EventTarget::Node(node); }
            node = data.host;
        }
    }
    fn is_in_shadow_tree(&self, target: EventTarget) -> bool {
        matches!(target, EventTarget::Node(node) if self.shadow_data(self.root(node)).is_some())
    }
    fn default_passive(&self, target: EventTarget, event_type: &str) -> bool {
        if !["touchstart", "touchmove", "wheel", "mousewheel"].contains(&event_type) {
            return false;
        }
        let node = match target {
            EventTarget::Node(node) => node,
            EventTarget::External(slot) => return self.window_event_target() == Some(slot),
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
impl Dom {
    /// Whether a cancelable input event could meet a non-passive listener on
    /// this target's composed dispatch path, including Document and Window.
    /// Callers must also invalidate their cached path after tree mutations.
    pub fn has_blocking_listener(&self, target: EventTarget, types: &[&str]) -> bool {
        let mut event = Event::new("", EventInit { composed: true, ..EventInit::default() }, Duration::ZERO);
        event.target = Some(target);
        let mut current = Some(target);
        let mut seen = Vec::new();
        while let Some(at) = current {
            if seen.contains(&at) { break; }
            seen.push(at);
            if self.events.has_blocking_listener(at, types) { return true; }
            current = self.event_parent(at, &event);
        }
        false
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
    listener_generation: u64,
    events: Vec<Event>,
}
#[derive(Clone, Debug)]
struct PathEntry {
    invocation: EventTarget,
    adjusted_target: EventTarget,
    adjusted_related: Option<EventTarget>,
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
    clear_targets: bool,
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
    pub related_target: Option<EventTarget>,
    pub phase: EventPhase,
    /// The listener has been removed from the native registry before invocation.
    pub once: bool,
}
impl EventTargets {
    /// Changes whenever the effective listener set changes. A compositor can
    /// cache scroll-blocking answers until this value or the DOM path changes.
    pub fn listener_generation(&self) -> u64 { self.listener_generation }

    pub fn has_blocking_listener(&self, target: EventTarget, types: &[&str]) -> bool {
        self.listeners.iter().any(|listener| listener.target == target
            && !listener.passive && types.contains(&listener.event_type.as_str()))
    }

    pub fn listener_id(&self, target: EventTarget, event_type: &str, callback: CallbackId,
        capture: bool) -> Option<ListenerId> {
        self.listeners.iter().find(|listener| listener.target == target
            && listener.event_type == event_type && listener.callback == callback
            && listener.capture == capture).map(|listener| listener.id)
    }

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
        self.listener_generation = self.listener_generation.wrapping_add(1);
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
        if !removed.is_empty() {
            self.listener_generation = self.listener_generation.wrapping_add(1);
        }
        removed
    }
    pub fn remove_listener_id(&mut self, id: ListenerId) -> Option<CallbackId> {
        let callback = self.listeners.iter().find(|listener| listener.id == id).map(|listener| listener.callback);
        let before = self.listeners.len();
        self.listeners.retain(|l| l.id != id);
        if self.listeners.len() != before {
            self.listener_generation = self.listener_generation.wrapping_add(1);
        }
        callback
    }
    pub fn remove_all_listeners(&mut self, target: EventTarget) {
        let before = self.listeners.len();
        self.listeners.retain(|l| l.target != target);
        if self.listeners.len() != before {
            self.listener_generation = self.listener_generation.wrapping_add(1);
        }
    }
    pub fn begin_dispatch(
        &mut self,
        tree: &impl EventTree,
        target: EventTarget,
        id: EventId,
        origin: DispatchOrigin,
    ) -> Result<Dispatch, DomError> {
        let mut event = self.events.get(id.0).ok_or(DomError::InvalidStateError)?.clone();
        if event.dispatching || !event.initialized {
            return Err(DomError::InvalidStateError);
        }
        // EventTree needs the original target when deciding whether a
        // non-composed event began inside this shadow root or at a slottable.
        event.target = Some(target);
        let mut path = vec![];
        let related = event.related_target.map(|related| tree.retarget(related, target));
        let mut current = if related == Some(target) && event.related_target != Some(target) {
            None
        } else { Some(target) };
        let mut previous_adjusted = None;
        while let Some(invocation) = current {
            if path.iter().any(|p: &PathEntry| p.invocation == invocation) {
                return Err(DomError::InvalidStateError);
            }
            let adjusted_target = tree.retarget(target, invocation);
            let adjusted_related = event.related_target.map(|related| tree.retarget(related, invocation));
            let at_target = previous_adjusted != Some(adjusted_target);
            if !path.is_empty() && at_target && adjusted_related == Some(invocation) { break; }
            path.push(PathEntry {
                invocation,
                adjusted_target,
                adjusted_related,
                at_target,
            });
            previous_adjusted = Some(adjusted_target);
            current = tree.event_parent(invocation, &event);
        }
        let clear_targets = path.iter().rev().find(|entry| entry.at_target).is_some_and(|entry|
            tree.is_in_shadow_tree(entry.adjusted_target)
                || entry.adjusted_related.is_some_and(|related| tree.is_in_shadow_tree(related)));
        let event = self.event_mut(id);
        event.dispatching = true;
        event.trusted = origin == DispatchOrigin::UserAgent;
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
            clear_targets,
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
                        let _ = self.remove_listener_id(id);
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
                        related_target: entry.adjusted_related,
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
            if !capturing && !entry.at_target && !event.bubbles() {
                continue;
            }
            event.target = Some(entry.adjusted_target);
            event.related_target = entry.adjusted_related;
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
        if dispatch.clear_targets {
            event.target = None;
            event.related_target = None;
        }
        event.path.clear();
        event.path_closed_roots.clear();
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
        if result.is_ok() {
            let roots = events.event(event).raw_path().iter().map(|target| match target {
                EventTarget::Node(node) => self.shadow_including_ancestors(*node).into_iter()
                    .filter(|id| self.shadow_data(*id).is_some_and(|data|
                        data.options.mode == crate::shadow::ShadowMode::Closed)).collect(),
                EventTarget::External(_) => vec![],
            }).collect();
            events.event_mut(event).set_closed_path_roots(roots);
        }
        self.events = events;
        result
    }
}
#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
