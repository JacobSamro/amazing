//! Trusted lower tasks retain driver custody until the actual callback starts.
//! Opaque scalar tokens cross the interpreter FIFO; no JS callback roots are added.
use std::collections::{BTreeMap, HashMap};
use amazing_dom::{NodeKind, selection::SelectionChangeTask};
use amazing_js::{interp::{Interpreter, NativeTaskToken}, value::Throw};
use super::RuntimeBindings;

#[derive(Default)]
pub(super) struct NativeTaskRuntime {
    next_slot: u64,
    pending: BTreeMap<u64, Entry>,
    by_task: HashMap<SelectionChangeTask, u64>,
}
struct Entry { task: SelectionChangeTask, admitted: bool }

impl RuntimeBindings {
    /// Also called at checkpoints for mutations made outside a JS binding call.
    /// Disabled external scheduling keeps the old lower outbox unchanged.
    pub(crate) fn emit_native_selection_tasks(&self, js: &mut Interpreter) -> Result<(), Throw> {
        if !js.external_scheduler_enabled() { return Ok(()); }
        let host = self.state.borrow().host.clone();
        let tasks = host.borrow().dom.pending_selection_change_tasks().to_vec();
        for task in tasks {
            let token = {
                let mut state = self.state.borrow_mut();
                let arena = state.arena;
                let runtime = &mut state.native_tasks;
                let slot = if let Some(slot) = runtime.by_task.get(&task) { *slot } else {
                    let slot = runtime.next_slot.checked_add(1)
                        .ok_or_else(|| Throw::range_error("native task tokens exhausted"))?;
                    runtime.next_slot = slot;
                    runtime.pending.insert(slot, Entry { task, admitted: false });
                    runtime.by_task.insert(task, slot);
                    slot
                };
                NativeTaskToken::new(arena, slot)
            };
            // Retain lower and driver custody on failure. A successful append
            // acknowledges only this exact task, never a later coalesced event.
            js.enqueue_native_task(token)?;
            host.borrow_mut().dom.acknowledge_selection_change_task(task);
        }
        Ok(())
    }

    /// Pure admission peek. Missing/foreign/already-admitted tokens are invalid;
    /// a retired Document generation is a stale task, not a different document.
    pub(crate) fn native_selection_task(&self, token: NativeTaskToken)
        -> Result<Option<SelectionChangeTask>, Throw> {
        let state = self.state.borrow();
        if token.owner() != state.arena { return Err(Throw::type_error("foreign native task owner")); }
        let entry = state.native_tasks.pending.get(&token.slot())
            .ok_or_else(|| Throw::type_error("unknown native task token"))?;
        if entry.admitted { return Err(Throw::type_error("native task already admitted")); }
        let task = entry.task;
        let host = state.host.borrow();
        let dom = &host.dom;
        Ok((dom.node_generation(task.document) == Some(task.document_generation)
            && matches!(dom.get(task.document).kind, NodeKind::Document)).then_some(task))
    }

    /// Called only after queue_task succeeds, without any intervening JS entry.
    pub(crate) fn native_selection_admitted(&self, token: NativeTaskToken) {
        let mut state = self.state.borrow_mut();
        assert_eq!(token.owner(), state.arena);
        let entry = state.native_tasks.pending.get_mut(&token.slot()).expect("validated pending task");
        assert!(!entry.admitted);
        entry.admitted = true;
    }

    /// Release the pending-only map before dispatch can run author callbacks.
    /// Stale task dispatch still clears its own lower flag through begin_selection_change.
    pub(crate) fn release_native_selection_task(&self, token: NativeTaskToken)
        -> Result<SelectionChangeTask, Throw> {
        let mut state = self.state.borrow_mut();
        if token.owner() != state.arena { return Err(Throw::type_error("foreign native task owner")); }
        let entry = state.native_tasks.pending.remove(&token.slot())
            .ok_or_else(|| Throw::type_error("unknown native task token"))?;
        state.native_tasks.by_task.remove(&entry.task);
        Ok(entry.task)
    }

    /// Scheduler/global teardown can leave BindingState alive through held
    /// wrappers. Retire only this arena's owned tasks, without author callbacks.
    pub(crate) fn retire_native_selection_tasks(&self) {
        let (host, tasks) = {
            let mut state = self.state.borrow_mut();
            let tasks: Vec<_> = std::mem::take(&mut state.native_tasks.pending)
                .into_values().map(|entry| entry.task).collect();
            state.native_tasks.by_task.clear();
            (state.host.clone(), tasks)
        };
        let mut host = host.borrow_mut();
        for task in tasks {
            host.dom.acknowledge_selection_change_task(task);
            host.dom.begin_selection_change(task);
        }
    }

    pub(crate) fn discard_native_selection_task(&self, token: NativeTaskToken) -> Result<(), Throw> {
        let task = self.release_native_selection_task(token)?;
        let host = self.state.borrow().host.clone();
        host.borrow_mut().dom.begin_selection_change(task);
        Ok(())
    }
}

#[cfg(test)]
impl RuntimeBindings {
    pub(crate) fn test_native_tasks_empty(&self) -> bool {
        self.state.borrow().native_tasks.pending.is_empty()
    }
    /// Exercise the driver's stale captured-generation branch, without claiming
    /// a collector actually recycled a node (the lower crate tests that path).
    pub(crate) fn test_stale_native_generation(&self, token: NativeTaskToken) -> SelectionChangeTask {
        let mut state = self.state.borrow_mut();
        assert_eq!(token.owner(), state.arena);
        let runtime = &mut state.native_tasks;
        let entry = runtime.pending.get_mut(&token.slot()).unwrap();
        let original = entry.task;
        entry.task.document_generation = original.document_generation.checked_add(1).unwrap();
        let stale = entry.task;
        runtime.by_task.remove(&original);
        runtime.by_task.insert(stale, token.slot());
        original
    }
    pub(crate) fn test_partial_selection_mutation(&self, js: &mut Interpreter) -> Result<(), Throw> {
        use amazing_bindings::host::BindingHost;
        let mut adapter = super::RuntimeBindingHost { js, state: self.state.clone() };
        adapter.with_dom(|dom| {
            dom.selection_remove_all_ranges(amazing_dom::NodeIdx(0))?;
            Err(amazing_dom::DomError::HierarchyRequestError)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use amazing_dom::{NodeIdx, range::BoundaryPoint};
    use crate::dom_host::BrowserDomHost;

    #[test]
    fn disabled_external_scheduler_preserves_legacy_lower_outbox() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut js = Interpreter::new();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        let task = {
            let mut host = host.borrow_mut();
            let body = host.dom.shadow_including_tree_order(NodeIdx(0)).into_iter()
                .find(|node| host.dom.local_name(*node) == Some("body")).unwrap();
            let text = host.dom.create_text(body, "x");
            host.dom.selection_collapse(NodeIdx(0), Some(BoundaryPoint::new(text, 0))).unwrap();
            host.dom.pending_selection_change_tasks()[0]
        };
        bindings.emit_native_selection_tasks(&mut js).unwrap();
        assert_eq!(host.borrow().dom.pending_selection_change_tasks(), &[task]);
        assert!(bindings.state.borrow().native_tasks.pending.is_empty());
        assert!(js.take_scheduling_requests().is_empty());
    }

    #[test]
    fn exhausted_host_token_retains_exact_lower_task_without_acknowledgement() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut js = Interpreter::new_with_external_scheduler(1).unwrap();
        js.set_dom_host(host.clone());
        let bindings = super::super::install(&mut js, host.clone()).unwrap();
        let task = {
            let mut host = host.borrow_mut();
            let body = host.dom.shadow_including_tree_order(NodeIdx(0)).into_iter()
                .find(|node| host.dom.local_name(*node) == Some("body")).unwrap();
            let text = host.dom.create_text(body, "x");
            host.dom.selection_collapse(NodeIdx(0), Some(BoundaryPoint::new(text, 0))).unwrap();
            host.dom.pending_selection_change_tasks()[0]
        };
        bindings.state.borrow_mut().native_tasks.next_slot = u64::MAX;
        assert!(bindings.emit_native_selection_tasks(&mut js).is_err());
        assert_eq!(host.borrow().dom.pending_selection_change_tasks(), &[task]);
        assert!(bindings.state.borrow().native_tasks.pending.is_empty());
        assert!(js.take_scheduling_requests().is_empty());
    }
}
