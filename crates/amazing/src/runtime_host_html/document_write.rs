//! Persistent script-created Document input streams (HTML §8.4).
use super::*;
use amazing_dom::events::{EventInit, EventTarget};

#[derive(Default)]
pub(super) struct DocumentWriteSession {
    generation: u64,
    lifecycle: Option<HtmlParserSession>,
    script_depth: usize,
    close_pending: bool,
    trailing_cr: bool,
}

impl RuntimeBindingHost<'_> {
    fn document_receiver_node(&self, receiver: &JsValue) -> Result<NodeIdx, Throw> {
        let key = self.platform_object(receiver, InterfaceBrand::NODE)
            .ok_or_else(|| Throw::type_error("Invalid Document receiver"))?;
        let node = self.node_from_platform_key(key)
            .ok_or_else(|| Throw::type_error("Stale Document receiver"))?;
        let state = self.state.borrow();
        let host = state.host.borrow();
        if !matches!(host.dom.get(node).kind, NodeKind::Document) {
            return Err(Throw::type_error("Invalid Document receiver"));
        }
        Ok(node)
    }

    fn check_write_document(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let xml = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let content_type = host.dom.document_metadata(document).ok().flatten()
                .map(|metadata| metadata.content_type.as_str())
                .or_else(|| (document == NodeIdx(0)).then_some(state.document_content_type.as_deref()).flatten());
            content_type.is_some_and(|mime| matches!(mime, "application/xml" | "text/xml") || mime.ends_with("+xml"))
        };
        if xml { return Err(throw_dom_error(self, DomError::InvalidStateError)); }
        Ok(())
    }

    fn check_dynamic_markup_insertion(&mut self, document: NodeIdx) -> Result<(), Throw> {
        if self.html_parser_creation_is_active(document) {
            return Err(self.document_exception("InvalidStateError", "Parser constructor prohibits dynamic markup insertion")?);
        }
        Ok(())
    }

    pub(in super::super) fn document_open(&mut self, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        for value in args.iter().take(if args.len() >= 3 { 3 } else { 2 }) {
            self.to_dom_string(value)?;
        }
        if args.len() >= 3 {
            if self.html_document_context(document).is_none() {
                return Err(self.document_exception("InvalidAccessError", "Document is not fully active")?);
            }
            // This overload opens a Window; it must never erase the current
            // Document while the browsing-context creation seam is absent.
            return Err(self.document_exception("NotSupportedError", "Document window-opening overload is unavailable")?);
        }
        self.check_write_document(document)?;
        self.check_dynamic_markup_insertion(document)?;
        self.check_document_open_origin(document)?;
        let ignore = RuntimeBindings { state: self.state.clone() }.parser_script_nesting(document) > 0;
        if !ignore { self.reopen_document(document)?; }
        Ok(receiver)
    }

    fn document_exception(&mut self, name: &str, message: &str) -> Result<Throw, Throw> {
        let global = self.js.binding_global_object();
        let constructor = self.js.binding_get_property(&global, "DOMException")?;
        let error = self.js.binding_construct(&constructor, vec![JsValue::str(message), JsValue::str(name)])?;
        Ok(self.js.binding_throw(error))
    }

    pub(in super::super) fn document_close(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        self.check_write_document(document)?;
        let finish = {
            let mut state = self.state.borrow_mut();
            match state.frames.document_write_sessions.get_mut(&document) {
                Some(session) => { session.close_pending = true; session.script_depth == 0 }
                None => false,
            }
        };
        if finish { self.finish_document_stream(document)?; }
        Ok(JsValue::Undefined)
    }

    pub(in super::super) fn document_write(&mut self, receiver: JsValue, args: &[JsValue],
        newline: bool) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        let mut text = String::new();
        for arg in args { text.push_str(&String::from_utf16_lossy(&self.to_dom_string(arg)?)); }
        // WebIDL argument conversion precedes the HTML method algorithm,
        // including its XML-document InvalidStateError check.
        self.check_write_document(document)?;
        self.check_dynamic_markup_insertion(document)?;
        if newline { text.push('\n'); }
        if !self.state.borrow().active_parsers.contains_key(&document) {
            self.reopen_document(document)?;
        }
        let parser = self.state.borrow().active_parsers.get(&document).cloned()
            .ok_or_else(|| Throw::type_error("Missing Document parser"))?;
        // Input preprocessing spans author write calls, including a CR/LF
        // pair split across two strings. Empty writes preserve the boundary.
        if !text.is_empty() {
            let mut state = self.state.borrow_mut();
            if let Some(session) = state.frames.document_write_sessions.get_mut(&document) {
                let starts_lf = text.starts_with('\n');
                let trailing_cr = text.ends_with('\r');
                if session.trailing_cr && starts_lf { text.remove(0); }
                session.trailing_cr = trailing_cr;
            }
        }
        self.write_into_active_parser(document, &parser, &text)?;
        let finish = self.state.borrow().frames.document_write_sessions.get(&document)
            .is_some_and(|session| session.close_pending && session.script_depth == 0);
        if finish { self.finish_document_stream(document)?; }
        Ok(JsValue::Undefined)
    }

    fn graft_document_parser(&mut self, document: NodeIdx,
        parser: &Rc<RefCell<amazing_html::ResumableParser>>, reverse: bool) -> Result<(), Throw> {
        let graft = (|| {
            let state = self.state.borrow();
            let mut host = state.host.borrow_mut();
            let mut parser = parser.borrow_mut();
            if let Some(import) = state.frames.parser_imports.get(&document) {
                if reverse {
                    import.borrow_mut().reverse_sync(parser.dom_mut(), &host.dom)
                        .map_err(|error| Throw::type_error(format!("Document parser reverse graft: {error:?}")))?;
                } else {
                    import.borrow_mut().sync(&mut host.dom, parser.dom())
                        .map_err(|error| Throw::type_error(format!("Document parser graft: {error:?}")))?;
                    let mode = parser.dom().document_quirks_mode(NodeIdx(0))
                        .map_err(|error| Throw::type_error(format!("Document parser mode: {error:?}")))?;
                    host.dom.set_document_quirks_mode(document, mode)
                        .map_err(|error| Throw::type_error(format!("Document mode: {error:?}")))?;
                }
            } else { parser.swap_dom(&mut host.dom); }
            Ok(())
        })();
        // Import can commit mutations before reporting an error. Admit their
        // notifications after all native borrows end, before author execution.
        let emitted = RuntimeBindings { state: self.state.clone() }
            .emit_native_selection_tasks(self.js);
        graft.and(emitted)
    }

    fn run_written_script(&mut self, document: NodeIdx, source_node: NodeIdx) -> Result<(), Throw> {
        self.run_written_script_with_mode(document, source_node, amazing_html::ScriptMode::Blocking)
    }
    fn run_written_script_with_mode(&mut self, document: NodeIdx, source_node: NodeIdx,
        mode: amazing_html::ScriptMode) -> Result<(), Throw> {
        let (node, script) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let node = match state.frames.parser_imports.get(&document) {
                Some(import) => import.borrow().imported(source_node), None => Some(source_node),
            };
            (node, node.and_then(|node| crate::cmd::js::script_source_from_node(&host.dom, node)))
        };
        if let Some(node) = node { self.state.borrow_mut().frames.already_started_scripts.insert(node); }
        let scripting_enabled = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            host.dom.document_scripting_enabled(document).unwrap_or(false)
        };
        if !scripting_enabled { return Ok(()); }
        let Some(script) = script else { return Ok(()); };
        let Some(node) = node else { return Ok(()); };
        let base = self.dom_document_base_url(document).map_err(|error| throw_dom_error(self, error))?;
        let script_url = match script.src.as_deref() {
            Some(src) => ParsedUrl::parse(&base).and_then(|base| base.resolve(src))
                .map(|url| url.serialize()).unwrap_or_else(|_| base.clone()),
            None => base.clone(),
        };
        let bindings = RuntimeBindings { state: self.state.clone() };
        let lifecycle = bindings.parser_session(document)
            .ok_or_else(|| Throw::type_error("Written script has no actual parser session"))?;
        let resolver = self.state.borrow().frames.resolver.clone();
        let entered_session = {
            let mut state = self.state.borrow_mut();
            state.frames.document_write_sessions.get_mut(&document).map(|session| {
                session.script_depth += 1;
                (session.generation, session.lifecycle)
            })
        };
        let realm = {
            let state = self.state.borrow();
            state.frames.contexts.iter().find(|(realm, context)| context.document == document
                && state.frames.live_context(context, **realm)).map(|(realm, _)| *realm)
        };
        let execute = |js: &mut Interpreter| {
            let payload = match script.src.as_deref() {
                None => Some((script.body.clone(), script_url.clone())),
                Some(src) => if script.kind == crate::cmd::js::ScriptKind::Module {
                    bindings.load_module_source(js, &script_url).ok().map(|text| (text, script_url.clone()))
                } else { bindings.fetch_document_classic_script(js, document, node, src, &base, resolver.as_ref()) },
            };
            if !bindings.parser_session_current(lifecycle) { return Ok(()); }
            match payload {
                Some((text, final_url)) if mode == amazing_html::ScriptMode::Async => {
                    bindings.queue_ready_parser_script(crate::cmd::js::HtmlParserScript {
                        script: script.clone(), mode, session: lifecycle }, &text, &final_url)
                }
                Some((text, final_url)) => {
                    let _scope = if mode == amazing_html::ScriptMode::Blocking {
                        Some(bindings.enter_parser_blocking_script(lifecycle)?)
                    } else { None };
                    bindings.execute_document_script(js, &script, &text, &final_url)
                },
                None if script.kind == crate::cmd::js::ScriptKind::Module =>
                    bindings.finish_module_script(js, &script, None, &script_url, false),
                None => Ok(()),
            }
        };
        let result = match realm {
            Some(realm) => self.js.binding_with_realm(realm, execute),
            None => execute(self.js),
        };
        if let Some(entered) = entered_session
            && let Some(session) = self.state.borrow_mut().frames.document_write_sessions.get_mut(&document)
            && (session.generation, session.lifecycle) == entered {
            session.script_depth -= 1;
        }
        result
    }

    fn write_into_active_parser(&mut self, document: NodeIdx,
        parser: &Rc<RefCell<amazing_html::ResumableParser>>, text: &str) -> Result<(), Throw> {
        parser.borrow_mut().begin_write(text);
        loop {
            // Validate the original associated realm while the native arena
            // is still installed; a root graft physically hands that arena to
            // the parser, leaving no Document in the temporary host shell.
            // A Document with no browsing context (a clone, or one made by
            // DOMParser) has no realm, so no custom element definition can
            // be looked up: it parses with the plain native tree builder.
            let (realm, import) = {
                let state = self.state.borrow();
                let realm = state.realm_documents.iter().find_map(|(realm, (node, generation))|
                    (*node == document && state.host.borrow().dom.node_generation(document) == Some(*generation)).then_some(*realm));
                (realm, state.frames.parser_imports.get(&document).cloned())
            };
            if let Err(error) = self.graft_document_parser(document, parser, true) {
                parser.borrow_mut().end_write();
                return Err(error);
            }
            let script = if let Some(realm) = realm {
                let bindings = RuntimeBindings { state: self.state.clone() };
                let mut creation = super::html_constructor::HtmlParserCreationHost { js: self.js, bindings, realm, import, parser: parser.clone() };
                match amazing_html::ResumableParser::run_write_with_creation_host(parser, &mut creation) {
                    Ok(script) => script,
                    Err(error) => return Err(throw_dom_error(self, error)),
                }
            } else {
                parser.borrow_mut().run_write()
            };
            if parser.borrow().is_aborted() { return Ok(()); }
            if let Err(error) = self.graft_document_parser(document, parser, false) {
                if script.is_some() { parser.borrow_mut().end_write(); }
                return Err(error);
            }
            RuntimeBindings { state: self.state.clone() }.navigate_iframes(self.js);
            let Some(node) = script else { return Ok(()); };
            if let Err(error) = self.run_written_script(document, node) {
                parser.borrow_mut().end_write();
                return Err(error);
            }
        }
    }

    fn check_document_open_origin(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let source_realm = self.js.binding_entry_realm();
        let pair = {
            let state = self.state.borrow();
            state.frames.contexts.get(&source_realm).cloned().zip(state.frames.contexts.iter()
                .find(|(realm, context)| context.document == document && state.frames.live_context(context, **realm))
                .map(|(_, context)| context.clone()))
        };
        if let Some((source, target)) = pair {
            if source.fetch_origin != target.fetch_origin {
                return Err(self.document_exception("SecurityError", "Blocked cross-origin Document.open")?);
            }
        }
        Ok(())
    }

    fn update_open_document_url(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let source_realm = self.js.binding_entry_realm();
        let pair = {
            let state = self.state.borrow();
            state.frames.contexts.get(&source_realm).cloned().zip(state.frames.contexts.iter()
                .find(|(realm, context)| context.document == document && state.frames.live_context(context, **realm))
                .map(|(realm, context)| (*realm, context.clone())))
        };
        let Some((source, (realm, target))) = pair else { return Ok(()); };
        let mut url = source.document_url.clone();
        if source.document != document { url = url.split('#').next().unwrap_or(&url).to_owned(); }
        let fallback = if ParsedUrl::parse(&url).is_ok_and(|url| matches_about_url(&url, "blank") || matches_about_url(&url, "srcdoc")) {
            Some(self.dom_document_base_url(source.document).map_err(|error| throw_dom_error(self, error))?)
        } else { None };
        {
            let mut state = self.state.borrow_mut();
            if let Some(context) = state.frames.contexts.get_mut(&realm) { context.document_url = url.clone(); }
            if let Some(frame) = target.frame {
                if let Some(entry) = state.frames.documents.get_mut(&frame) { entry.url = url.clone(); }
            }
            if let Some(base) = fallback { state.frames.about_base_urls.insert(document, (target.document_generation, base)); }
            let mut host = state.host.borrow_mut();
            if let Some(mut metadata) = host.dom.document_metadata(document).ok().flatten().cloned() {
                metadata.url = url.clone();
                host.dom.set_document_metadata(document, metadata)
                    .map_err(|error| Throw::type_error(format!("Document URL: {error:?}")))?;
            }
        }
        self.js.binding_with_realm(realm, |js| { js.set_location_url(url); Ok(()) })
    }

    fn reopen_document(&mut self, document: NodeIdx) -> Result<(), Throw> {
        self.check_document_open_origin(document)?;
        self.update_open_document_url(document)?;
        let mut parser = amazing_html::ResumableParser::new(Dom::new());
        let (generation, scripting_enabled, allow_shadow_roots, window_slot) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            (host.dom.node_generation(document).ok_or_else(|| Throw::type_error("Stale Document"))?,
                host.dom.document_scripting_enabled(document).unwrap_or(false),
                host.dom.document_allows_declarative_shadow_roots(document)
                    .map_err(|error| Throw::type_error(format!("Document shadow policy: {error:?}")))?,
                state.frames.contexts.iter().find(|(realm, context)| context.document == document
                    && state.frames.live_context(context, **realm))
                    .and_then(|(_, context)| self.dom_window_event_target_slot(&context.window)))
        };
        parser.set_allow_declarative_shadow_roots(allow_shadow_roots);
        if !scripting_enabled { parser.set_scripting_mode(amazing_html::ScriptingMode::Disabled); }
        self.with_dom(|dom| {
            for node in dom.shadow_including_tree_order(document) {
                dom.events_mut().remove_all_listeners(EventTarget::Node(node));
            }
            if let Some(slot) = window_slot { dom.events_mut().remove_all_listeners(EventTarget::External(slot)); }
            dom.replace_all(document, None)?;
            dom.set_document_quirks_mode(document, amazing_dom::QuirksMode::NoQuirks)?;
            dom.set_document_ready_state(document, ReadyState::Loading)?;
            Ok(())
        })?;
        let lifecycle = RuntimeBindings { state: self.state.clone() }.begin_parser_session(self.js, document)?;
        let parser = Rc::new(RefCell::new(parser));
        register_active_parser_state(&self.state, document, parser);
        let mut state = self.state.borrow_mut();
        state.frames.parser_imports.insert(document,
            Rc::new(RefCell::new(amazing_dom::markup::IncrementalDocumentImport::new(document))));
        state.frames.document_write_sessions.insert(document,
            DocumentWriteSession { generation, lifecycle: Some(lifecycle), ..Default::default() });
        Ok(())
    }

    fn finish_document_stream(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let parser = self.state.borrow().active_parsers.get(&document).cloned();
        let Some(parser) = parser else { return Ok(()); };
        let lifecycle = self.state.borrow().frames.document_write_sessions.get(&document)
            .and_then(|session| session.lifecycle);
        let Some(lifecycle) = lifecycle else { return Ok(()); };
        let valid = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            state.frames.document_write_sessions.get(&document)
                .is_some_and(|session| host.dom.node_generation(document) == Some(session.generation))
        };
        if !valid { return Err(Throw::type_error("Stale Document parser")); }
        // A constructor owns the tree frame until it returns. close() records
        // EOF now; the outer written-input entry will consume it after restore.
        if !parser.borrow().execution_frame_available() { return Ok(()); }
        let bindings = RuntimeBindings { state: self.state.clone() };
        loop {
            if !bindings.parser_session_current(lifecycle) { return Ok(()); }
            self.graft_document_parser(document, &parser, true)?;
            let import = self.state.borrow().frames.parser_imports.get(&document).cloned();
            let signal = {
                let mut creation = super::html_constructor::HtmlParserCreationHost {
                    js: self.js, bindings: bindings.clone(), realm: lifecycle.realm, import, parser: parser.clone(),
                };
                amazing_html::ResumableParser::run_with_creation_host(&parser, &mut creation)
            }.map_err(|error| throw_dom_error(self, error))?;
            if signal == amazing_html::ParseSignal::Aborted { return Ok(()); }
            self.graft_document_parser(document, &parser, false)?;
            match signal {
                amazing_html::ParseSignal::Aborted => return Ok(()),
                amazing_html::ParseSignal::Eof => break,
                amazing_html::ParseSignal::Script(node) => self.run_written_script(document, node)?,
            }
        }
        if !bindings.parser_session_current(lifecycle) { return Ok(()); }
        // The tree builder's deferred ids belong to its private arena. Capture
        // actual imported nodes before retiring this parser, then run them with
        // no parser script-nesting level: deferred/module scripts can reopen.
        let deferred = parser.borrow_mut().take_deferred_or_async();
        let deferred: Vec<_> = {
            let state = self.state.borrow();
            deferred.into_iter().filter_map(|(node, mode)| state.frames.parser_imports.get(&document)
                .and_then(|import| import.borrow().imported(node)).map(|node| (node, mode))).collect()
        };
        unregister_active_parser_state(&self.state, document);
        {
            let mut state = self.state.borrow_mut();
            state.frames.parser_imports.remove(&document);
            state.frames.document_write_sessions.remove(&document);
        }
        if !bindings.parser_session_current(lifecycle) { return Ok(()); }
        bindings.parser_eof(self.js, document)?;
        for (node, mode) in deferred {
            if !bindings.parser_session_current(lifecycle) { return Ok(()); }
            self.run_written_script_with_mode(document, node, mode)?;
        }
        if bindings.parser_session_current(lifecycle) { bindings.finish_parser_document(document)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

    fn scheduler() -> DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><body><p id=old>old</p></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html#fragment");
        scheduler
    }

    #[test]
    fn reopened_stream_preserves_partial_tokens_doctype_rawtext_and_document_identity() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            var original=document;
            document.open();
            if(document.documentElement!==null || document.readyState!=='loading') throw new Error('not empty');
            document.write('<!doc'); document.write('type html><html><head><title>A&am');
            document.write('p;B</title></head><body><!--com'); document.write('ment--><p id="wri');
            document.write('tten">a&amp;'); document.writeln('b</p>');
            document.write('<pre id=lines>x\r');document.write('');document.write('\ny</pre>'); document.close();
            [document===original,document.doctype.name,document.compatMode,document.title,
             document.getElementById('written').textContent,document.readyState,document.body.firstChild.data,
             document.getElementById('lines').textContent].join('|')
        "#).unwrap(), JsValue::str("true|html|CSS1Compat|A&B|a&b|interactive|comment|x\ny"));
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("[document===original,document.doctype.name,document.compatMode,document.title,document.getElementById('written').textContent,document.readyState,document.body.firstChild.data,document.getElementById('lines').textContent].join('|')").unwrap(),
            JsValue::str("true|html|CSS1Compat|A&B|a&b|complete|comment|x\ny"));
    }

    #[test]
    fn repeated_open_replaces_session_and_close_only_finishes_script_created_parser() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            document.close();
            if(!document.getElementById('old')) throw new Error('close changed original');
            var old=document.getElementById('old'), seen=0;
            document.addEventListener('x',()=>seen++); old.addEventListener('x',()=>seen++);
            window.addEventListener('x',()=>seen++);
            old.onclick=()=>seen++;
            document.open(); document.write('<p id="discarded">one');
            document.open(); document.write('<!doctype html><p id="kept">two'); document.close();
            old.dispatchEvent(new Event('x')); document.dispatchEvent(new Event('x')); window.dispatchEvent(new Event('x'));
            old.dispatchEvent(new Event('click'));
            if(old.onclick!==null) throw new Error('handler survived open');
            var states=[];
            document.open(); document.write('<p id="final">three');
            document.addEventListener('readystatechange',()=>states.push(document.readyState));
            document.addEventListener('DOMContentLoaded',()=>states.push('dom'));
            document.close(); document.close();
            [seen,document.getElementById('discarded')===null,document.getElementById('kept')===null,
             document.getElementById('final').textContent,document.compatMode,states.join(',')].join('|')
        "#).unwrap(), JsValue::str("0|true|true|three|BackCompat|interactive"));
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("states.join(',')").unwrap(), JsValue::str("interactive,dom,complete"));
    }

    #[test]
    fn nested_written_scripts_keep_input_order_and_defer_explicit_close() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            var order=[]; document.open();
            document.write('<!doctype html><body><script>order.push("outer");document.open();document.write("<b id=inner>inner</b>");document.close();order.push("done");<\/script><i id=tail>tail</i>');
            [order.join(','),document.getElementById('inner').textContent,document.getElementById('inner').nextSibling.id,
             document.readyState,document.doctype.name].join('|')
        "#).unwrap(), JsValue::str("outer,done|inner|tail|interactive|html"));
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("[order.join(','),document.getElementById('inner').textContent,document.getElementById('inner').nextSibling.id,document.readyState,document.doctype.name].join('|')").unwrap(),
            JsValue::str("outer,done|inner|tail|complete|html"));
    }

    #[test]
    fn reopened_stream_modules_use_module_registry_and_current_script_policy() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            document.open();
            document.write('<!doctype html><script type=module>window.moduleValue=41+1;window.moduleCurrent=document.currentScript;<\/script><p>tail');
            if(typeof moduleValue!=='undefined') throw new Error('module ran before close');
            document.close();
            [moduleValue,moduleCurrent===null,document.readyState,document.body.lastChild.textContent].join('|')
        "#).unwrap(), JsValue::str("42|true|interactive|tail"));
        scheduler.pump_at(Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("[moduleValue,moduleCurrent===null,document.readyState,document.body.lastChild.textContent].join('|')").unwrap(),
            JsValue::str("42|true|complete|tail"));
    }

    #[test]
    fn open_converts_arguments_and_keeps_window_overload_outside_document_replacement() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            var element=document.documentElement,seen=[];
            try {document.open(Symbol('x'),'unused')}catch(e){seen.push(e.name)}
            try {document.open({toString(){throw new Error('conversion')}},'unused')}catch(e){seen.push(e.message)}
            try {document.open('/child','named','',Symbol('ignored'))}catch(e){seen.push(e.name)}
            var inert=document.implementation.createHTMLDocument('inert'),inertElement=inert.documentElement;
            try {inert.open('/child','named','')}catch(e){seen.push(e.name)}
            var xml=new DOMParser().parseFromString('<root/>','application/xml');
            try {xml.open(Symbol('x'),'unused')}catch(e){seen.push(e.name)}
            if(document.documentElement!==element || inert.documentElement!==inertElement) throw new Error('overload erased Document');
            var result=document.open({toString(){seen.push('first');return 'unused'}},
                                     {toString(){seen.push('second');return 'unused'}});
            document.write('<!doctype html><p id=p>opened');document.close();
            [seen.join(','),result===document,document.getElementById('p').textContent].join('|')
        "#).unwrap(), JsValue::str("TypeError,conversion,NotSupportedError,InvalidAccessError,TypeError,first,second|true|opened"));
    }

    #[test]
    fn reopened_child_url_uses_entry_document_across_borrowed_child_functions() {
        let mut scheduler = scheduler();
        scheduler.run_script(r#"
            var frame=document.createElement('iframe');
            frame.srcdoc='<script>window.openAgain=function(){return document.open()};<\/script>';
            document.body.appendChild(frame);
        "#).unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(r#"
            var child=frame.contentDocument,win=frame.contentWindow;
            var returned=win.openAgain();
            child.write('<!doctype html><p>opened');child.close();
            [returned===child,child.URL,document.URL,child.defaultView===win].join('|')
        "#).unwrap(), JsValue::str("true|https://example.test/main.html|https://example.test/main.html#fragment|true"));
    }

    #[test]
    fn stream_argument_conversion_precedes_xml_algorithm_and_checks_genuine_receivers() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            var xml=new DOMParser().parseFromString('<root/>','application/xml'), seen=[];
            var argument={toString(){seen.push('convert');throw new Error('conversion')}};
            for(var name of ['write','writeln']) {
                try {xml[name](argument)} catch(e){seen.push(e.message)}
                try {xml[name](Symbol('x'))} catch(e){seen.push(e.name)}
                try {Document.prototype[name].call({},argument)} catch(e){seen.push(e.name)}
                try {xml[name]('x')} catch(e){seen.push(e.name)}
            }
            [seen.join(','),xml.documentElement.localName,!!document.getElementById('old')].join('|')
        "#).unwrap(), JsValue::str("convert,conversion,TypeError,TypeError,InvalidStateError,convert,conversion,TypeError,TypeError,InvalidStateError|root|true"));
    }

    #[test]
    fn child_and_inert_streams_use_actual_scripting_context_and_xml_rejects_operations() {
        let mut scheduler = scheduler();
        scheduler.run_script("var frame=document.createElement('iframe');document.body.appendChild(frame)").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(r#"
            var checkpoint="child-open";
            try {
            var child=frame.contentDocument, win=frame.contentWindow;
            child.open(); child.write('<!doctype html><script>window.childRan=document===parent.document.querySelector("iframe").contentDocument<\/script><p id=c>child');child.close();
            checkpoint="inert-open";
            var inert=document.implementation.createHTMLDocument('inert');
            inert.open();inert.write('<script>parent.inertRan=true<\/script><noscript><p id=n>inert</p></noscript>');inert.close();
            checkpoint="xml";
            var xml=new DOMParser().parseFromString('<root/>','application/xml'), errors=[];
            for(var name of ['open','write','close']) { try {xml[name]('x');errors.push('missing')}catch(e){errors.push(e.name)} }
            checkpoint="observations";
            [frame.contentDocument===child,frame.contentWindow===win,win.childRan,
             child.getElementById('c').textContent,typeof inertRan,inert.getElementById('n').textContent,errors.join(',')].join('|')
            } catch(error) { checkpoint+"|"+error.name+"|"+error.message; }
        "#).unwrap(), JsValue::str("true|true|true|child|undefined|inert|InvalidStateError,InvalidStateError,InvalidStateError"));
    }
}

#[cfg(test)]
mod selection_checkpoints {
    use super::*;
    use amazing_js::interp::SchedulingRequest;

    fn setup() -> (Interpreter, RuntimeBindings, Rc<RefCell<BrowserDomHost>>) {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><p>abc</p></body>"), native_forwarded: false,
        }));
        let mut js = Interpreter::new_with_external_scheduler(64).unwrap();
        js.set_dom_host(host.clone());
        let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/stream-selection");
        (js, bindings, host)
    }
    fn clear_requests(js: &mut Interpreter, bindings: &RuntimeBindings) {
        for request in js.take_scheduling_requests() {
            if let SchedulingRequest::NativeTask { token } = request {
                bindings.discard_native_selection_task(token).unwrap();
            }
        }
    }
    fn order(js: &mut Interpreter) -> Vec<&'static str> {
        js.take_scheduling_requests().into_iter().filter_map(|request| match request {
            SchedulingRequest::NativeTask { .. } => Some("selection"),
            SchedulingRequest::Timer { .. } => Some("timer"),
            _ => None,
        }).collect()
    }
    #[test]
    fn open_and_reentrant_written_script_emit_before_author_continuation() {
        for timer_first in [false, true] {
            let (mut js, bindings, host) = setup();
            js.run_script("getSelection().setPosition(document.querySelector('p').firstChild,3)").unwrap();
            clear_requests(&mut js, &bindings);
            let generation = host.borrow().dom.node_generation(NodeIdx(0)).unwrap();
            let old = bindings.begin_parser_session(&mut js, NodeIdx(0)).unwrap();
            // No active parser registration: this is a real reopening, not the
            // normative ignored open() from inside a parser-blocking script.
            let timer = "setTimeout(()=>{},0);";
            let open = "document.open();document.write('<body><p>abc</p><scr'+'ipt>getSelection().setPosition(document.querySelector(\"p\").firstChild,1)</scr'+'ipt>');document.close();";
            js.run_script(&if timer_first { format!("{timer}{open}") } else { format!("{open}{timer}") }).unwrap();
            assert_eq!(order(&mut js), if timer_first { vec!["timer", "selection"] } else { vec!["selection", "timer"] });
            let new = bindings.parser_session(NodeIdx(0)).unwrap();
            assert_ne!(new, old);
            assert_eq!(host.borrow().dom.node_generation(NodeIdx(0)), Some(generation));
            assert!(!bindings.parser_session_current(old));
            assert_eq!(js.run_script("getSelection().anchorOffset===1").unwrap(), JsValue::Bool(true));
        }
    }
    #[test]
    fn forward_graft_range_adjustment_enters_same_fifo_before_next_author_timer() {
        for timer_first in [false, true] {
            let (mut js, bindings, host) = setup();
            js.run_script("document.open();document.write('<body><p>abc</p>');getSelection().setPosition(document.querySelector('p').firstChild,3)").unwrap();
            clear_requests(&mut js, &bindings);
            if timer_first { js.run_script("setTimeout(()=>{},0)").unwrap(); }
            let parser = bindings.state.borrow().active_parsers[&NodeIdx(0)].clone();
            let text = parser.borrow().dom().tree_order(NodeIdx(0)).into_iter()
                .find(|node| matches!(parser.borrow().dom().get(*node).kind, NodeKind::Text { .. })).unwrap();
            parser.borrow_mut().dom_mut().set_character_data(text, "a".into()).unwrap();
            let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
            adapter.graft_document_parser(NodeIdx(0), &parser, false).unwrap();
            assert!(host.borrow().dom.pending_selection_change_tasks().is_empty());
            if !timer_first { js.run_script("setTimeout(()=>{},0)").unwrap(); }
            assert_eq!(order(&mut js), if timer_first { vec!["timer", "selection"] } else { vec!["selection", "timer"] });
            assert_eq!(js.run_script("getSelection().anchorOffset===1").unwrap(), JsValue::Bool(true));
        }
    }
}
