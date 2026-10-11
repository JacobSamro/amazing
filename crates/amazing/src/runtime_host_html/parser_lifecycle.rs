//! Actual parser sessions and queued HTML end-of-parsing phases.
//! HTML §13.2.7: relevant-global association is distinct from event target.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HtmlParserSession {
    pub(crate) document: NodeIdx,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) realm: RealmHandle,
    pub(crate) global_document: NodeIdx,
    pub(crate) global_generation: u64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum HtmlParserPhase { DomContentLoaded, WindowLoad }
#[derive(Clone, Copy, Debug)]
pub(crate) struct HtmlParserTask {
    pub(crate) session: HtmlParserSession,
    pub(crate) phase: HtmlParserPhase,
}
#[derive(Default)]
pub(super) struct ParserLifecycle {
    next_epoch: u64,
    sessions: HashMap<NodeIdx, ParserCompletion>,
    tasks: VecDeque<HtmlParserTask>,
    scripts: VecDeque<HtmlReadyParserScript>,
}
struct ParserCompletion {
    parsed: bool,
    session: HtmlParserSession,
    eof: bool,
    dcl: bool,
    load_queued: bool,
    loaded: bool,
    asynchronous: usize,
    script_nesting: usize,
    embedding: Option<PendingFrameLoad>,
}
/// An executing parser-blocking script owns this exact session epoch. A
/// replacement parser cannot inherit or be decremented by its old script.
pub(crate) struct ParserScriptScope {
    state: Rc<RefCell<BindingState>>,
    session: HtmlParserSession,
}
impl Drop for ParserScriptScope {
    fn drop(&mut self) {
        if let Some(entry) = self.state.borrow_mut().frames.parser_lifecycle.sessions.get_mut(&self.session.document)
            && entry.session == self.session {
            entry.script_nesting -= 1;
        }
    }
}
impl FrameRuntime {
    fn parser_session_current(&self, dom: &Dom, session: HtmlParserSession) -> bool {
        dom.node_generation(session.document) == Some(session.generation)
            && matches!(dom.get(session.document).kind, NodeKind::Document)
            && self.parser_lifecycle.sessions.get(&session.document)
                .is_some_and(|entry| entry.session == session)
    }
    fn parser_load_blocked(&self, dom: &Dom, document: NodeIdx) -> bool {
        self.pending_loads.iter().any(|load| load.blocks_owner(dom, self, document))
            || self.parser_lifecycle.sessions.values().any(|entry| !entry.loaded
                && entry.embedding.as_ref().is_some_and(|load|
                    load.blocks_owner(dom, self, document)))
    }
    fn queue_ready_parser_loads(&mut self, dom: &Dom) {
        let ready: Vec<_> = self.parser_lifecycle.sessions.values().filter(|entry|
            entry.eof && entry.dcl && !entry.loaded && !entry.load_queued
                && entry.asynchronous == 0 && self.parser_session_current(dom, entry.session)
                && !self.parser_load_blocked(dom, entry.session.document))
            .map(|entry| entry.session).collect();
        for session in ready {
            self.parser_lifecycle.sessions.get_mut(&session.document).unwrap().load_queued = true;
            self.parser_lifecycle.tasks.push_back(HtmlParserTask { session, phase: HtmlParserPhase::WindowLoad });
        }
    }
}
impl RuntimeBindings {
    /// Begin while the complete installed arena is still in the host.
    pub(crate) fn begin_parser_session(&self, js: &mut Interpreter, document: NodeIdx)
        -> Result<HtmlParserSession, Throw> {
        let adapter = RuntimeBindingHost { js, state: self.state.clone() };
        let realm = adapter.dom_node_realm(document)?;
        let mut state = self.state.borrow_mut();
        let host = state.host.clone(); let host = host.borrow();
        let generation = host.dom.node_generation(document)
            .ok_or_else(|| Throw::type_error("Parser Document is stale"))?;
        if !matches!(host.dom.get(document).kind, NodeKind::Document) {
            return Err(Throw::type_error("Parser requires a Document"));
        }
        // Only authoritative actual WindowGlobal registration associates a global.
        let context = state.frames.contexts.get(&realm)
            .ok_or_else(|| Throw::type_error("Parser relevant global is not registered"))?;
        let (global_document, global_generation) = (context.document, context.document_generation);
        if state.realm_documents.get(&realm) != Some(&(global_document, global_generation))
            || host.dom.node_generation(global_document) != Some(global_generation) {
            return Err(Throw::type_error("Parser relevant global Document is stale"));
        }
        let lifecycle = &mut state.frames.parser_lifecycle;
        lifecycle.next_epoch = lifecycle.next_epoch.checked_add(1)
            .ok_or_else(|| Throw::range_error("Parser session epochs exhausted"))?;
        let session = HtmlParserSession { document, generation, epoch: lifecycle.next_epoch,
            realm, global_document, global_generation };
        let embedding = lifecycle.sessions.get(&document).filter(|entry| entry.session.generation == generation)
            .and_then(|entry| entry.embedding.clone());
        lifecycle.sessions.insert(document, ParserCompletion { session, parsed: false, eof: false,
            dcl: false, load_queued: false, loaded: false, asynchronous: 0, script_nesting: 0, embedding });
        Ok(session)
    }
    pub(crate) fn enter_parser_blocking_script(&self, session: HtmlParserSession) -> Result<ParserScriptScope, Throw> {
        if !self.parser_session_current(session) { return Err(Throw::type_error("Stale parser script session")); }
        let mut state = self.state.borrow_mut();
        let entry = state.frames.parser_lifecycle.sessions.get_mut(&session.document).unwrap();
        entry.script_nesting = entry.script_nesting.checked_add(1)
            .ok_or_else(|| Throw::range_error("Parser script nesting exhausted"))?;
        Ok(ParserScriptScope { state: self.state.clone(), session })
    }
    pub(super) fn parser_script_nesting(&self, document: NodeIdx) -> usize {
        self.state.borrow().frames.parser_lifecycle.sessions.get(&document)
            .map_or(0, |entry| entry.script_nesting)
    }
    pub(crate) fn parser_session(&self, document: NodeIdx) -> Option<HtmlParserSession> {
        self.state.borrow().frames.parser_lifecycle.sessions.get(&document).map(|entry| entry.session)
    }
    pub(crate) fn parser_session_current(&self, session: HtmlParserSession) -> bool {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.parser_session_current(&host.dom, session)
    }
    pub(crate) fn parser_eof(&self, js: &mut Interpreter, document: NodeIdx) -> Result<(), Throw> {
        let session = {
            let mut state = self.state.borrow_mut(); let host = state.host.clone(); let mut host = host.borrow_mut();
            let Some(session) = state.frames.parser_lifecycle.sessions.get(&document).map(|entry| entry.session) else {
                return Err(Throw::type_error("Parser EOF has no actual session"));
            };
            if !state.frames.parser_session_current(&host.dom, session) { return Ok(()); }
            let entry = state.frames.parser_lifecycle.sessions.get_mut(&document).unwrap();
            if entry.parsed { return Ok(()); }
            entry.parsed = true;
            host.dom.set_document_ready_state(document, ReadyState::Interactive)
                .map_err(|_| Throw::type_error("Parser Document readiness is stale"))?;
            session
        };
        js.binding_with_realm(session.realm, |js| self.dispatch_user_agent_payload(js,
            amazing_dom::events::EventTarget::Node(document),
            amazing_dom::events::Event::new("readystatechange", amazing_dom::events::EventInit::default(), Duration::ZERO)).map(|_| ()))
    }
    /// Caller has completed its actual finished-parsing script list.
    pub(crate) fn finish_parser_document(&self, document: NodeIdx) -> Result<(), Throw> {
        let mut state = self.state.borrow_mut(); let host = state.host.clone(); let host = host.borrow();
        let Some(session) = state.frames.parser_lifecycle.sessions.get(&document).map(|entry| entry.session) else {
            return Err(Throw::type_error("Parser completion has no actual session"));
        };
        if !state.frames.parser_session_current(&host.dom, session) { return Ok(()); }
        let entry = state.frames.parser_lifecycle.sessions.get_mut(&document).unwrap();
        if !entry.parsed || entry.eof { return Ok(()); }
        entry.eof = true;
        state.frames.parser_lifecycle.tasks.push_back(HtmlParserTask { session, phase: HtmlParserPhase::DomContentLoaded });
        Ok(())
    }
    pub(crate) fn set_parser_embedding(&self, document: NodeIdx, load: PendingFrameLoad) {
        if let Some(entry) = self.state.borrow_mut().frames.parser_lifecycle.sessions.get_mut(&document) {
            entry.embedding = Some(load);
        }
    }
    pub(crate) fn take_parser_tasks(&self) -> Vec<HtmlParserTask> {
        self.state.borrow_mut().frames.parser_lifecycle.tasks.drain(..).collect()
    }
    /// Registry candidates come exclusively from genuine immutable WindowGlobals.
    pub(crate) fn parser_global_associations(&self) -> Vec<(RealmHandle, NodeIdx, u64, bool)> {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.contexts.iter().filter(|(realm, context)|
            state.realm_documents.get(realm) == Some(&(context.document, context.document_generation)))
            .map(|(realm, context)| (*realm, context.document, context.document_generation,
                state.frames.history_context(&host.dom, context.document)
                    .is_some_and(|active| active.realm == *realm))).collect()
    }
    pub(crate) fn run_parser_task(&self, js: &mut Interpreter, task: HtmlParserTask) -> Result<(), Throw> {
        if !self.parser_session_current(task.session) { return Ok(()); }
        let document = task.session.document;
        let state = self.state.clone();
        let delivered = (|| -> Result<(), Throw> {
        match task.phase {
            HtmlParserPhase::DomContentLoaded => {
                {
                    let mut state = state.borrow_mut();
                    let entry = state.frames.parser_lifecycle.sessions.get_mut(&document).unwrap();
                    if entry.dcl { return Ok(()); }
                    entry.dcl = true;
                }
                js.binding_with_realm(task.session.realm, |js| {
                    self.dispatch_event(js, document.0, "DOMContentLoaded")
                })?;
            }
            HtmlParserPhase::WindowLoad => {
                let embedding = {
                    let mut state = state.borrow_mut(); let host = state.host.clone(); let mut host = host.borrow_mut();
                    let blocked = state.frames.parser_load_blocked(&host.dom, document);
                    let entry = state.frames.parser_lifecycle.sessions.get_mut(&document).unwrap();
                    entry.load_queued = false;
                    if entry.loaded || blocked || entry.asynchronous != 0 { return Ok(()); }
                    entry.loaded = true;
                    host.dom.set_document_ready_state(document, ReadyState::Complete)
                        .map_err(|_| Throw::type_error("Parser Document is stale"))?;
                    entry.embedding.clone()
                };
                js.binding_with_realm(task.session.realm, |js| {
                    self.dispatch_user_agent_payload(js, amazing_dom::events::EventTarget::Node(document),
                        amazing_dom::events::Event::new("readystatechange", amazing_dom::events::EventInit::default(), Duration::ZERO))?;
                    if self.parser_session_current(task.session) { self.dispatch_document_load(js, document)?; }
                    Ok(())
                })?;
                if self.parser_session_current(task.session)
                    && let Some(load) = embedding.filter(|load| load.fire_load_events && self.iframe_load_is_current(load)) {
                    self.dispatch_user_agent_payload(js, amazing_dom::events::EventTarget::Node(load.frame),
                        amazing_dom::events::Event::new("load", amazing_dom::events::EventInit::default(), Duration::ZERO))?;
                }
            }
        }
        Ok(())
        })();
        if self.parser_session_current(task.session) {
            let mut state = state.borrow_mut(); let host = state.host.clone(); let host = host.borrow();
            state.frames.queue_ready_parser_loads(&host.dom);
        }
        delivered
    }
}

#[derive(Clone)]
pub(crate) struct HtmlReadyParserScript {
    pub(crate) record: crate::cmd::js::HtmlParserScript,
    pub(crate) source: amazing_js::value::JsString,
    pub(crate) url: String,
}
impl RuntimeBindings {
    pub(crate) fn queue_ready_parser_script(&self, record: crate::cmd::js::HtmlParserScript,
        source: impl Into<amazing_js::value::JsString>, url: &str) -> Result<(), Throw> {
        if !self.parser_session_current(record.session) { return Ok(()); }
        let mut state = self.state.borrow_mut();
        let lifecycle = &mut state.frames.parser_lifecycle;
        let entry = lifecycle.sessions.get_mut(&record.session.document).unwrap();
        entry.asynchronous = entry.asynchronous.checked_add(1)
            .ok_or_else(|| Throw::range_error("Async parser script count exhausted"))?;
        lifecycle.scripts.push_back(HtmlReadyParserScript { record,
            source: source.into(), url: url.to_owned() });
        Ok(())
    }
    pub(crate) fn take_ready_parser_scripts(&self) -> Vec<HtmlReadyParserScript> {
        self.state.borrow_mut().frames.parser_lifecycle.scripts.drain(..).collect()
    }
    pub(crate) fn run_ready_parser_script(&self, js: &mut Interpreter, task: HtmlReadyParserScript)
        -> Result<(), Throw> {
        let session = task.record.session;
        if !self.parser_session_current(session) { return Ok(()); }
        let current = {
            let state = self.state.borrow(); let host = state.host.borrow();
            task.record.node_generation == host.dom.node_generation(task.record.node)
        };
        let result = if current { js.binding_with_realm(session.realm, |js|
            self.execute_document_script(js, &task.record, &task.source, &task.url)) } else { Ok(()) };
        // Completion accounting must survive a JS exception, but never modify
        // the replacement session created by a callback's document.open().
        if self.parser_session_current(session) {
            let mut state = self.state.borrow_mut(); let host = state.host.clone(); let host = host.borrow();
            let entry = state.frames.parser_lifecycle.sessions.get_mut(&session.document).unwrap();
            entry.asynchronous = entry.asynchronous.saturating_sub(1);
            state.frames.queue_ready_parser_loads(&host.dom);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    fn scheduler() -> (Rc<RefCell<BrowserDomHost>>, DriverScheduler) {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/parser");
        (host, scheduler)
    }
    #[test]
    fn defer_timer_tasks_dcl_and_load_have_actual_session_order() {
        let (host, mut scheduler) = scheduler();
        let scripts = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><script>var trace=[];document.addEventListener('DOMContentLoaded',()=>trace.push('dcl'));window.addEventListener('load',()=>trace.push('load'));</script><script defer src='defer.js'></script>",
            |scheduler, script| scheduler.run_page_script(script, &script.body, "https://example.test/parser")
                .map(|_| ()).map_err(|error| error.describe())).unwrap();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].mode, amazing_html::ScriptMode::Deferred);
        assert_eq!(host.borrow().dom.document_ready_state(NodeIdx(0)).unwrap(), ReadyState::Interactive);
        scheduler.run_parser_script(&scripts[0], "trace.push('defer');setTimeout(()=>{trace.push('first:'+trace.includes('dcl'));setTimeout(()=>trace.push('second:'+trace.includes('dcl')),0)},0)", "https://example.test/defer.js").unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("defer"));
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("defer,first:false,dcl,second:true,load"));
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.length").unwrap(), JsValue::num(5.0));
    }
    #[test]
    fn actual_async_ready_script_is_distinct_and_load_accounting_survives_throw() {
        let (host, mut scheduler) = scheduler();
        let scripts = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><script>var trace=[];document.addEventListener('DOMContentLoaded',()=>trace.push('dcl'));window.addEventListener('load',()=>trace.push('load'));</script><script async src='async.js'></script><script defer src='defer.js'></script>",
            |scheduler, script| scheduler.run_page_script(script, &script.body, "https://example.test/parser")
                .map(|_| ()).map_err(|error| error.describe())).unwrap();
        assert_eq!(scripts[0].mode, amazing_html::ScriptMode::Async);
        assert_eq!(scripts[1].mode, amazing_html::ScriptMode::Deferred);
        scheduler.run_parser_script(&scripts[0], "trace.push('async');throw Error('observed async failure')", "https://example.test/async.js").unwrap();
        scheduler.run_parser_script(&scripts[1], "trace.push('defer')", "https://example.test/defer.js").unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        assert!(scheduler.pump_at(Duration::ZERO, false).is_err());
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("defer,async,dcl,load"));
        assert_eq!(host.borrow().dom.document_ready_state(NodeIdx(0)).unwrap(), ReadyState::Complete);
    }
    #[test]
    fn same_generation_document_open_replaces_epoch_without_old_completion() {
        let (host, mut scheduler) = scheduler();
        let old = scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        scheduler.parser_eof(NodeIdx(0)).unwrap();
        scheduler.run_script("var trace=[];document.addEventListener('DOMContentLoaded',()=>{trace.push('old');document.open();document.write('<p>replacement</p>');document.addEventListener('DOMContentLoaded',()=>trace.push('new'));window.addEventListener('load',()=>trace.push('load'));document.close()})").unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        let state = scheduler.test_bindings().state.clone();
        let new = scheduler.test_bindings().parser_session(NodeIdx(0)).unwrap();
        assert_eq!(old.generation, new.generation);
        assert_ne!(old.epoch, new.epoch);
        assert!(!state.borrow().frames.parser_session_current(&host.borrow().dom, old));
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("old,new,load"));
    }
    #[test]
    fn inert_event_target_uses_real_creation_global_and_history_does_not_cancel() {
        let (host, mut scheduler) = scheduler();
        scheduler.run_script("var trace=[];var inert=document.implementation.createHTMLDocument('inert');inert.open();inert.write('<p>inert</p>');inert.addEventListener('DOMContentLoaded',()=>trace.push('inert'));inert.close()").unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("inert"));
        let bindings = scheduler.test_bindings();
        let state = bindings.state.borrow();
        let (document, entry) = state.frames.parser_lifecycle.sessions.iter().find(|(doc, _)| **doc != NodeIdx(0)).unwrap();
        assert_eq!(entry.session.global_document, NodeIdx(0));
        assert_ne!(*document, entry.session.global_document);
        assert_eq!(host.borrow().dom.document_window_event_target(*document), None);
        drop(state);
        scheduler.set_document_metadata("UTF-8", "text/html", "");
        let session = scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        scheduler.parser_eof(NodeIdx(0)).unwrap();
        let url = ParsedUrl::parse("https://example.test/parser#same").unwrap();
        let state = scheduler.test_bindings().state.clone();
        let mut adapter = RuntimeBindingHost { js: &mut scheduler.js, state };
        adapter.html_commit_history_url(session.document, session.generation, &url).unwrap();
        assert!(scheduler.test_bindings().parser_session_current(session));
    }
    #[test]
    fn child_global_ids_are_native_and_retired_original_global_stays_inactive() {
        let (host, mut scheduler) = scheduler();
        let _ = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><iframe id=c sandbox='allow-scripts' srcdoc='<p>child</p>'></iframe>", |_, _| Ok(())).unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        let state = scheduler.test_bindings().state.clone();
        let child = state.borrow().frames.contexts.iter().find(|(_, context)| context.frame.is_some()).map(|(realm, context)| (*realm, context.document)).unwrap();
        assert!(scheduler.test_parser_global(child.0).is_some());
        assert!(state.borrow().frames.document_context_for_realm(&host.borrow().dom, child.0).is_some());
        scheduler.run_script("document.getElementById('c').remove()").unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert!(scheduler.test_parser_global(child.0).is_some());
        assert!(state.borrow().frames.history_context(&host.borrow().dom, child.1).is_none());
        assert_ne!(scheduler.test_parser_global(child.0).unwrap().0, crate::js_scheduler::GLOBAL);
    }
}

#[cfg(test)]
mod stale_task_tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    #[test]
    fn stale_task_generation_epoch_and_removed_child_do_not_complete_a_new_owner() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/stale");
        let _ = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><script>var trace=[];window.addEventListener('load',()=>trace.push('root'));</script><iframe id=c srcdoc='<script>document.addEventListener(\"DOMContentLoaded\",()=>parent.trace.push(\"child\"))</script>'></iframe>",
            |scheduler, script| scheduler.run_page_script(script, &script.body, "https://example.test/stale")
                .map(|_| ()).map_err(|error| error.describe())).unwrap();
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        let bindings = scheduler.test_bindings().clone();
        let actual = bindings.parser_session(NodeIdx(0)).unwrap();
        let mut stale = actual; stale.generation += 1;
        bindings.run_parser_task(&mut scheduler.js, HtmlParserTask { session: stale, phase: HtmlParserPhase::WindowLoad }).unwrap();
        stale = actual; stale.epoch += 1;
        bindings.run_parser_task(&mut scheduler.js, HtmlParserTask { session: stale, phase: HtmlParserPhase::WindowLoad }).unwrap();
        assert_eq!(scheduler.run_script("trace.length").unwrap(), JsValue::num(0.0));
        scheduler.run_script("document.getElementById('c').remove()").unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("root"));
    }
}

#[cfg(test)]
mod written_defer_tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    #[test]
    fn written_defer_can_reopen_without_decrementing_the_replacement_nesting_level() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><body>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/written");
        let path = std::env::temp_dir().join(format!("amazing-written-defer-{}.js", std::process::id()));
        std::fs::write(&path, "document.open();document.write('<p id=replacement>done</p>');document.addEventListener('DOMContentLoaded',()=>trace.push('new'));window.addEventListener('load',()=>trace.push('load'));document.close()").unwrap();
        let script_path = path.to_string_lossy().into_owned();
        scheduler.set_frame_resource_resolver(Rc::new(move |url| url.ends_with("reopen.js").then(|| script_path.clone())));
        scheduler.run_script(r#"var trace=[];document.open();document.write('<script defer src="reopen.js"><\/script>');document.close()"#).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("new,load"));
        assert_eq!(host.borrow().dom.document_ready_state(NodeIdx(0)).unwrap(), ReadyState::Complete);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod root_reentrancy_tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    #[test]
    fn blocking_open_is_ignored_and_eof_callback_preserves_replacement_parser() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root-reentry");
        let scripts = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><p id=before>before</p><script>document.open();document.write('<p id=written>written</p>')</script><p id=after>after</p>",
            |scheduler, script| scheduler.run_page_script(script, &script.body, "https://example.test/root-reentry")
                .map(|_| ()).map_err(|error| error.describe())).unwrap();
        assert!(scripts.is_empty());
        assert_eq!(scheduler.run_script("!!document.getElementById('before')&&!!document.getElementById('written')&&!!document.getElementById('after')").unwrap(), JsValue::Bool(true));
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        // A second fresh installed arena isolates the EOF reentrancy case;
        // the document parser requires a fresh input document, not a second
        // full parse appended into the first case's completed root.
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root-eof-reentry");
        scheduler.run_script("var trace=[];document.addEventListener('readystatechange',()=>{if(document.readyState==='interactive'){document.open();document.write('<p id=replacement>new</p>');document.addEventListener('DOMContentLoaded',()=>trace.push('new'));window.addEventListener('load',()=>trace.push('load'))}})").unwrap();
        let scripts = crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<p id=retired>retired</p><script defer src='retired.js'></script>", |_, _| Ok(())).unwrap();
        assert!(scripts.is_empty());
        assert_eq!(host.borrow().dom.document_ready_state(NodeIdx(0)).unwrap(), ReadyState::Loading);
        assert_eq!(scheduler.run_script("!!document.getElementById('replacement')&&!document.getElementById('retired')").unwrap(), JsValue::Bool(true));
        scheduler.run_script("document.close()").unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("new,load"));
    }
}

#[cfg(test)]
mod initial_blank_tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    #[test]
    fn initial_blank_is_complete_without_parser_events_and_retarget_has_own_lifecycle() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/blank");
        crate::cmd::js::run_interleaved_parse(&host, &mut scheduler,
            "<!doctype html><iframe id=c></iframe><script>var trace=[];var c=document.getElementById('c');var initial=c.contentDocument;initial.addEventListener('DOMContentLoaded',()=>trace.push('unexpected-dcl'));c.contentWindow.addEventListener('load',()=>trace.push('unexpected-window'));c.addEventListener('load',()=>trace.push('container'));window.addEventListener('load',()=>trace.push('root'))</script>",
            |scheduler, script| scheduler.run_page_script(script, &script.body, "https://example.test/blank")
                .map(|_| ()).map_err(|error| error.describe())).unwrap();
        assert_eq!(scheduler.run_script("initial.readyState").unwrap(), JsValue::str("complete"));
        scheduler.finish_parser_document(NodeIdx(0)).unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("root"));
        assert_eq!(scheduler.run_script("initial===c.contentDocument").unwrap(), JsValue::Bool(true));
        scheduler.run_script("c.contentWindow.location.href='about:blank'").unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("root,container"));
        assert_eq!(scheduler.run_script("initial!==c.contentDocument&&c.contentDocument.readyState==='complete'").unwrap(), JsValue::Bool(true));
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("trace.join(',')").unwrap(), JsValue::str("root,container"));
    }
}
