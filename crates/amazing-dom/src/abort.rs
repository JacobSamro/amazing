//! DOM abort signal state. JS reasons and subscriber callbacks stay in the host.
use crate::{events::{CallbackId, EventTarget, ListenerId, ListenerOptions}, Dom, DomError};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbortSignalId(pub usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbortAlgorithm {
    Host(u64),
    RemoveEventListener(ListenerId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbortDelivery {
    pub signal: AbortSignalId,
    pub algorithms: Vec<AbortAlgorithm>,
}

#[derive(Debug)]
struct Signal {
    target: u64,
    reason: Option<CallbackId>,
    sources: Vec<AbortSignalId>,
    dependent: bool,
    dependents: Vec<AbortSignalId>,
    algorithms: Vec<AbortAlgorithm>,
    onabort: Option<CallbackId>,
}

#[derive(Debug, Default)]
pub struct AbortSignals { signals: Vec<Signal> }

impl Dom {
    pub fn create_abort_signal(&mut self) -> AbortSignalId {
        let id = AbortSignalId(self.abort_signals.signals.len());
        let target = self.create_event_target();
        self.abort_signals.signals.push(Signal {
            target, reason: None, sources: Vec::new(), dependent: false,
            dependents: Vec::new(), algorithms: Vec::new(), onabort: None,
        });
        id
    }

    /// A dependent signal is already aborted without firing an event when an
    /// input was aborted before `any()` was called.
    pub fn create_dependent_abort_signal(&mut self, sources: &[AbortSignalId])
        -> Result<AbortSignalId, DomError> {
        for source in sources { self.abort_signal_checked(*source)?; }
        let id = self.create_abort_signal();
        if let Some(reason) = sources.iter().find_map(|source| self.abort_signals.signals[source.0].reason) {
            self.abort_signals.signals[id.0].reason = Some(reason);
        } else {
            self.abort_signals.signals[id.0].dependent = true;
            for source in sources {
                let roots = if self.abort_signals.signals[source.0].dependent {
                    self.abort_signals.signals[source.0].sources.clone()
                } else { vec![*source] };
                for root in roots {
                    if !self.abort_signals.signals[id.0].sources.contains(&root) {
                        self.abort_signals.signals[id.0].sources.push(root);
                    }
                    let dependents = &mut self.abort_signals.signals[root.0].dependents;
                    if !dependents.contains(&id) { dependents.push(id); }
                }
            }
        }
        Ok(id)
    }

    pub fn abort_signal_checked(&self, id: AbortSignalId) -> Result<(), DomError> {
        if self.abort_signals.signals.get(id.0).is_some() { Ok(()) }
        else { Err(DomError::InvalidStateError) }
    }
    pub fn abort_signal_reason(&self, id: AbortSignalId) -> Result<Option<CallbackId>, DomError> {
        Ok(self.abort_signals.signals.get(id.0).ok_or(DomError::InvalidStateError)?.reason)
    }
    pub fn abort_signal_target(&self, id: AbortSignalId) -> Result<u64, DomError> {
        Ok(self.abort_signals.signals.get(id.0).ok_or(DomError::InvalidStateError)?.target)
    }
    pub fn abort_event_handler(&self, id: AbortSignalId) -> Result<Option<CallbackId>, DomError> {
        Ok(self.abort_signals.signals.get(id.0).ok_or(DomError::InvalidStateError)?.onabort)
    }
    pub fn set_abort_event_handler(&mut self, id: AbortSignalId, handler: Option<CallbackId>)
        -> Result<Option<CallbackId>, DomError> {
        let signal = self.abort_signals.signals.get_mut(id.0).ok_or(DomError::InvalidStateError)?;
        Ok(std::mem::replace(&mut signal.onabort, handler))
    }
    pub fn abort_signal_for_target(&self, target: u64) -> Option<AbortSignalId> {
        self.abort_signals.signals.iter().position(|signal| signal.target == target).map(AbortSignalId)
    }

    /// Subscriber IDs are embedding-owned. An already-aborted signal returns
    /// its reason so the subscriber can fail immediately instead of being lost.
    pub fn add_abort_algorithm(&mut self, id: AbortSignalId, algorithm: u64)
        -> Result<Option<CallbackId>, DomError> {
        let signal = self.abort_signals.signals.get_mut(id.0).ok_or(DomError::InvalidStateError)?;
        if let Some(reason) = signal.reason { Ok(Some(reason)) }
        else { signal.algorithms.push(AbortAlgorithm::Host(algorithm)); Ok(None) }
    }
    pub fn remove_abort_algorithm(&mut self, id: AbortSignalId, algorithm: u64) -> Result<(), DomError> {
        let signal = self.abort_signals.signals.get_mut(id.0).ok_or(DomError::InvalidStateError)?;
        signal.algorithms.retain(|entry| *entry != AbortAlgorithm::Host(algorithm));
        Ok(())
    }

    /// Register a signal removal step only for a newly installed listener.
    /// Duplicate registrations keep the original signal and options.
    pub fn add_abortable_event_listener(&mut self, target: EventTarget, event_type: &str,
        callback: Option<CallbackId>, options: ListenerOptions, signal: Option<AbortSignalId>)
        -> Result<Option<ListenerId>, DomError> {
        if let Some(signal) = signal {
            if self.abort_signal_reason(signal)?.is_some() { return Ok(None); }
        }
        let Some(callback) = callback else { return Ok(None) };
        if let Some(existing) = self.events().listener_id(target, event_type, callback, options.capture) {
            return Ok(Some(existing));
        }
        let listener = self.add_event_listener(target, event_type, Some(callback), options);
        if let (Some(signal), Some(listener)) = (signal, listener) {
            self.abort_signals.signals[signal.0].algorithms.push(AbortAlgorithm::RemoveEventListener(listener));
        }
        Ok(listener)
    }

    /// Sets all dependent reasons before yielding the ordered abort steps.
    /// The embedding runs each subscriber, then fires that signal's event.
    pub fn abort_signal(&mut self, id: AbortSignalId, reason: CallbackId)
        -> Result<Vec<AbortDelivery>, DomError> {
        self.abort_signal_checked(id)?;
        if self.abort_signals.signals[id.0].reason.is_some() { return Ok(Vec::new()); }
        let mut queue = VecDeque::from([id]);
        let mut ordered = Vec::new();
        while let Some(next) = queue.pop_front() {
            let signal = &mut self.abort_signals.signals[next.0];
            if signal.reason.is_some() { continue; }
            signal.reason = Some(reason);
            ordered.push(next);
            queue.extend(signal.dependents.iter().copied());
        }
        Ok(ordered.into_iter().map(|signal| AbortDelivery {
            signal, algorithms: std::mem::take(&mut self.abort_signals.signals[signal.0].algorithms),
        }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listener_abort_steps_share_subscriber_order_and_duplicates_keep_first_signal() {
        let mut dom = Dom::new();
        let a = dom.create_abort_signal();
        let b = dom.create_abort_signal();
        let target = EventTarget::External(dom.create_event_target());
        dom.add_abort_algorithm(a, 1).unwrap();
        let listener = dom.add_abortable_event_listener(target, "x", Some(CallbackId(7)),
            ListenerOptions::default(), Some(a)).unwrap().unwrap();
        dom.add_abort_algorithm(a, 2).unwrap();
        assert_eq!(dom.add_abortable_event_listener(target, "x", Some(CallbackId(7)),
            ListenerOptions::default(), Some(b)).unwrap(), Some(listener));
        assert!(dom.abort_signal(b, CallbackId(9)).unwrap()[0].algorithms.is_empty());
        let deliveries = dom.abort_signal(a, CallbackId(10)).unwrap();
        assert_eq!(deliveries[0].algorithms, [AbortAlgorithm::Host(1),
            AbortAlgorithm::RemoveEventListener(listener), AbortAlgorithm::Host(2)]);
        assert_eq!(dom.events_mut().remove_listener_id(listener), Some(CallbackId(7)));
        assert_eq!(dom.events_mut().remove_listener_id(listener), None);
    }

    #[test]
    fn abort_any_propagates_once_in_source_then_dependent_order() {
        let mut dom = Dom::new();
        let a = dom.create_abort_signal();
        let b = dom.create_abort_signal();
        let any = dom.create_dependent_abort_signal(&[a, b]).unwrap();
        let nested = dom.create_dependent_abort_signal(&[any]).unwrap();
        dom.add_abort_algorithm(a, 1).unwrap();
        dom.add_abort_algorithm(any, 2).unwrap();
        let reason = CallbackId(9);
        assert_eq!(dom.abort_signal(a, reason).unwrap(), vec![
            AbortDelivery { signal: a, algorithms: vec![AbortAlgorithm::Host(1)] },
            AbortDelivery { signal: any, algorithms: vec![AbortAlgorithm::Host(2)] },
            AbortDelivery { signal: nested, algorithms: vec![] },
        ]);
        assert_eq!(dom.abort_signal(b, CallbackId(10)).unwrap(), vec![AbortDelivery { signal: b, algorithms: vec![] }]);
        assert_eq!(dom.abort_signal(any, reason).unwrap(), Vec::new());
        assert_eq!(dom.abort_signal_reason(nested).unwrap(), Some(reason));
        assert_eq!(dom.create_dependent_abort_signal(&[a]).and_then(|id| dom.abort_signal_reason(id)), Ok(Some(reason)));
    }
}
