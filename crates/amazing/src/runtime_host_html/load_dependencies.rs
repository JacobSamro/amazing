//! HTML iframe attribute processing and owner Document load dependencies.
//! Current navigation's lazy flag is immutable; viewport fetch deferral is
//! not implemented here. HTML §4.8.5 and §2.5.7, reviewed 2026-10-04.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct PendingFrameLoad {
    pub(crate) frame: NodeIdx,
    frame_generation: u64,
    owner: NodeIdx,
    owner_generation: u64,
    pub(crate) document: Option<NodeIdx>,
    document_generation: Option<u64>,
    pub(crate) fire_load_events: bool,
    lazy: bool,
}

impl PendingFrameLoad {
    pub(super) fn blocks_owner(&self, dom: &Dom, frames: &FrameRuntime, owner: NodeIdx) -> bool {
        self.owner == owner && self.fire_load_events && !self.lazy && frames.frame_load_is_current(dom, self)
    }
}

impl FrameRuntime {
    pub(super) fn enqueue_frame_load(&mut self, dom: &Dom, frame: NodeIdx,
        document: Option<NodeIdx>, fire_load_events: bool, lazy: bool) {
        let Some(frame_generation) = dom.node_generation(frame) else { return; };
        let Some(owner) = dom.owner_document(frame) else { return; };
        let Some(owner_generation) = dom.node_generation(owner) else { return; };
        let document_generation = document.and_then(|doc| dom.node_generation(doc));
        self.pending_loads.push_back(PendingFrameLoad { frame, frame_generation,
            owner, owner_generation, document, document_generation, fire_load_events, lazy });
    }
    fn frame_load_is_current(&self, dom: &Dom, load: &PendingFrameLoad) -> bool {
        dom.node_generation(load.frame) == Some(load.frame_generation)
            && dom.node_generation(load.owner) == Some(load.owner_generation)
            && dom.owner_document(load.frame) == Some(load.owner)
            && dom.is_connected(load.frame)
            && self.has_document(load.owner)
            && load.document.and_then(|doc| dom.node_generation(doc)) == load.document_generation
            && self.documents.get(&load.frame).is_some_and(|entry|
                entry.document == load.document
                    && (!load.fire_load_events || entry.current_navigation_was_lazy_loaded == load.lazy))
    }
}

impl RuntimeBindings {
    pub(crate) fn iframe_load_is_current(&self, load: &PendingFrameLoad) -> bool {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.frame_load_is_current(&host.dom, load)
    }
    pub(crate) fn take_next_iframe_load(&self, owner: Option<NodeIdx>) -> Option<PendingFrameLoad> {
        let mut state = self.state.borrow_mut();
        let host = state.host.clone(); let host = host.borrow();
        // Select from the still-owned queue, rather than draining a whole batch:
        // newly discovered nested eager frames must complete before their parent.
        loop {
            let index = state.frames.pending_loads.iter().position(|load|
                owner.is_none_or(|owner| load.owner == owner && !load.lazy))?;
            let load = state.frames.pending_loads.remove(index)?;
            if state.frames.frame_load_is_current(&host.dom, &load) { return Some(load); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

    #[test]
    fn captured_lazy_navigation_does_not_block_owner_window_load() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><iframe id=e srcdoc='<p>eager</p>'></iframe><iframe id=l loading=LaZy sandbox srcdoc='<p>lazy</p>'></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root");
        scheduler.run_script("var trace=[]; window.addEventListener('load',()=>trace.push('root')); document.getElementById('e').addEventListener('load',()=>trace.push('eager')); document.getElementById('l').addEventListener('load',()=>trace.push('lazy')); document.getElementById('l').setAttribute('loading','eager');").unwrap();
        scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        scheduler.parser_eof(NodeIdx(0)).unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.test_pump_admitted_at(std::time::Duration::ZERO).unwrap();
        scheduler.run_script("if(trace.join(',')!=='eager,root') throw Error(trace.join(','));").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.test_pump_admitted_at(std::time::Duration::ZERO).unwrap();
        scheduler.run_script("if(trace.join(',')!=='eager,root,lazy') throw Error(trace.join(','));").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.run_script("if(trace.length!==3) throw Error('duplicate');").unwrap();
    }

    #[test]
    fn production_pump_preserves_admission_order_for_resumed_lazy_navigation() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><iframe id=e srcdoc='<p>eager</p>'></iframe><iframe id=l loading=lazy srcdoc='<p>resumed</p>'></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root");
        scheduler.run_script("var trace=[]; window.addEventListener('load',()=>trace.push('root')); document.getElementById('e').addEventListener('load',()=>trace.push('eager')); document.getElementById('l').addEventListener('load',()=>trace.push('lazy')); document.getElementById('l').setAttribute('loading','eager');").unwrap();
        scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        scheduler.parser_eof(NodeIdx(0)).unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        // Both child completion tasks are now prepared by the actual general
        // pump. Lazy does not block the owner, but its already-admitted task
        // is not reordered behind the newly released owner completion.
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        scheduler.run_script("if(trace.join(',')!=='eager,lazy,root') throw Error(trace.join(','));").unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        scheduler.run_script("if(trace.length!==3) throw Error('duplicate');").unwrap();
    }

    #[test]
    fn scripting_disabled_and_initial_blank_are_not_lazy_dependencies() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><iframe id=b loading=lazy src='about:blank'></iframe><iframe id=i loading=' lazy' srcdoc='<p>invalid keyword</p>'></iframe><iframe id=n loading=lazy srcdoc='<p>no script</p>'></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        host.borrow_mut().dom.set_document_scripting_enabled(NodeIdx(0), false).unwrap();
        scheduler.set_location_url("https://example.test/root");
        let state = scheduler.test_bindings().state.clone();
        assert!(state.borrow().frames.pending_loads.iter().all(|load| !load.lazy));
        scheduler.dispatch_load_blocking_iframe_loads(NodeIdx(0)).unwrap();
        assert!(state.borrow().frames.pending_loads.is_empty());
    }
    #[test]
    fn nested_eager_dependencies_precede_parent_but_lazy_and_stale_navigations_do_not() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            r#"<!doctype html><iframe id=p srcdoc="<iframe id=child srcdoc='<p>nested</p>'></iframe><iframe loading=lazy srcdoc='<p>late</p>'></iframe>"></iframe><iframe id=l loading=lazy srcdoc='<p>first</p>'></iframe>"#), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root");
        let state = scheduler.test_bindings().state.clone();
        let stale = state.borrow().frames.pending_loads.iter().find(|load| load.lazy).unwrap().clone();
        scheduler.run_script("document.getElementById('l').srcdoc='<p>second</p>';").unwrap();
        assert!(!scheduler.test_bindings().iframe_load_is_current(&stale));
        scheduler.run_script("var trace=[]; document.getElementById('p').addEventListener('load',()=>trace.push('parent')); window.addEventListener('load',()=>trace.push('root'));").unwrap();
        scheduler.dispatch_load_blocking_iframe_loads(NodeIdx(0)).unwrap();
        {
            let state = state.borrow(); let dom = &host.borrow().dom;
            let parent = state.frames.contexts.values().find(|context| context.frame.is_some_and(|frame| dom.get_attribute_ns(frame,None,"id")==Some("p"))).unwrap();
            assert!(state.frames.pending_loads.iter().any(|load| load.owner == parent.document && load.lazy));
            assert!(!state.frames.pending_loads.iter().any(|load| load.owner == parent.document && !load.lazy));
        }
        scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        scheduler.parser_eof(NodeIdx(0)).unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.test_pump_admitted_at(std::time::Duration::ZERO).unwrap();
        scheduler.run_script("if(trace.join(',')!=='parent,root') throw Error(trace.join(',')); document.getElementById('l').remove();").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert!(state.borrow().frames.pending_loads.is_empty());
    }

}
