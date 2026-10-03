//! Persistent script-created Document input streams (HTML §8.4).
use super::*;
use amazing_dom::events::{EventInit, EventTarget};

#[derive(Default)]
pub(super) struct DocumentWriteSession {
    generation: u64,
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

    pub(in super::super) fn document_open(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        self.check_write_document(document)?;
        self.check_document_open_origin(document)?;
        let ignore = {
            let state = self.state.borrow();
            state.frames.document_write_sessions.get(&document)
                .map_or_else(|| state.active_parsers.contains_key(&document)
                    || state.write_insertion_points.contains_key(&document), |session| session.script_depth > 0)
        };
        if !ignore { self.reopen_document(document)?; }
        Ok(receiver)
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
        self.check_write_document(document)?;
        let mut text = String::new();
        for arg in args { text.push_str(&String::from_utf16_lossy(&self.to_dom_string(arg)?)); }
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
    }

    fn run_written_script(&mut self, document: NodeIdx, source_node: NodeIdx) -> Result<(), Throw> {
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
        let base = self.dom_document_base_url(document).map_err(|error| throw_dom_error(self, error))?;
        let script_url = match script.src.as_deref() {
            Some(src) => ParsedUrl::parse(&base).and_then(|base| base.resolve(src))
                .map(|url| url.serialize()).unwrap_or_else(|_| base.clone()),
            None => base.clone(),
        };
        let bindings = RuntimeBindings { state: self.state.clone() };
        let resolver = self.state.borrow().frames.resolver.clone();
        if let Some(session) = self.state.borrow_mut().frames.document_write_sessions.get_mut(&document) {
            session.script_depth += 1;
        }
        let realm = {
            let state = self.state.borrow();
            state.frames.contexts.iter().find(|(realm, context)| context.document == document
                && state.frames.live_context(context, **realm)).map(|(realm, _)| *realm)
        };
        let execute = |js: &mut Interpreter| {
            let text = match script.src.as_deref() {
                None => Some(script.body.clone()),
                Some(src) => if script.kind == crate::cmd::js::ScriptKind::Module {
                    bindings.load_module_source(js, &script_url).ok()
                } else { fetch_child_script(src, &base, resolver.as_ref()) },
            };
            match text {
                Some(text) => bindings.execute_document_script(js, &script, &text, &script_url),
                None if script.kind == crate::cmd::js::ScriptKind::Module =>
                    bindings.finish_module_script(js, &script, None, &script_url, false),
                None => Ok(()),
            }
        };
        let result = match realm {
            Some(realm) => self.js.binding_with_realm(realm, execute),
            None => execute(self.js),
        };
        if let Some(session) = self.state.borrow_mut().frames.document_write_sessions.get_mut(&document) {
            session.script_depth -= 1;
        }
        result
    }

    fn write_into_active_parser(&mut self, document: NodeIdx,
        parser: &Rc<RefCell<amazing_html::ResumableParser>>, text: &str) -> Result<(), Throw> {
        parser.borrow_mut().begin_write(text);
        loop {
            if let Err(error) = self.graft_document_parser(document, parser, true) {
                parser.borrow_mut().end_write();
                return Err(error);
            }
            let script = parser.borrow_mut().run_write();
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
        let source_realm = self.js.binding_incumbent_realm();
        let pair = {
            let state = self.state.borrow();
            state.frames.contexts.get(&source_realm).cloned().zip(state.frames.contexts.iter()
                .find(|(realm, context)| context.document == document && state.frames.live_context(context, **realm))
                .map(|(_, context)| context.clone()))
        };
        if let Some((source, target)) = pair {
            if source.fetch_origin != target.fetch_origin {
                let error = self.js.binding_with_realm(source_realm, |js| {
                    let global = js.binding_global_object();
                    let constructor = js.binding_get_property(&global, "DOMException")?;
                    js.binding_construct(&constructor, vec![JsValue::str("Blocked cross-origin Document.open"),
                        JsValue::str("SecurityError")])
                })?;
                return Err(self.js.binding_throw(error));
            }
        }
        Ok(())
    }

    fn update_open_document_url(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let source_realm = self.js.binding_incumbent_realm();
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
        let (generation, scripting_enabled, window_slot) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            (host.dom.node_generation(document).ok_or_else(|| Throw::type_error("Stale Document"))?,
                host.dom.document_scripting_enabled(document).unwrap_or(false),
                state.frames.contexts.iter().find(|(realm, context)| context.document == document
                    && state.frames.live_context(context, **realm))
                    .and_then(|(_, context)| self.dom_window_event_target_slot(&context.window)))
        };
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
        let parser = Rc::new(RefCell::new(parser));
        register_active_parser_state(&self.state, document, parser);
        let mut state = self.state.borrow_mut();
        state.frames.parser_imports.insert(document,
            Rc::new(RefCell::new(amazing_dom::markup::IncrementalDocumentImport::new(document))));
        state.frames.document_write_sessions.insert(document,
            DocumentWriteSession { generation, ..Default::default() });
        Ok(())
    }

    fn finish_document_stream(&mut self, document: NodeIdx) -> Result<(), Throw> {
        let parser = self.state.borrow().active_parsers.get(&document).cloned();
        let Some(parser) = parser else { return Ok(()); };
        let valid = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            state.frames.document_write_sessions.get(&document)
                .is_some_and(|session| host.dom.node_generation(document) == Some(session.generation))
        };
        if !valid { return Err(Throw::type_error("Stale Document parser")); }
        loop {
            self.graft_document_parser(document, &parser, true)?;
            let signal = parser.borrow_mut().run();
            self.graft_document_parser(document, &parser, false)?;
            match signal {
                amazing_html::ParseSignal::Eof => break,
                amazing_html::ParseSignal::Script(node) => self.run_written_script(document, node)?,
            }
        }
        // The tree builder's deferred ids belong to its private arena. Capture
        // actual imported nodes before retiring this parser, then run them with
        // no parser script-nesting level: deferred/module scripts can reopen.
        let deferred = parser.borrow_mut().take_deferred_or_async();
        let deferred: Vec<_> = {
            let state = self.state.borrow();
            deferred.into_iter().filter_map(|(node, _)| state.frames.parser_imports.get(&document)
                .and_then(|import| import.borrow().imported(node))).collect()
        };
        unregister_active_parser_state(&self.state, document);
        {
            let mut state = self.state.borrow_mut();
            state.frames.parser_imports.remove(&document);
            state.frames.document_write_sessions.remove(&document);
        }
        self.with_dom(|dom| dom.set_document_ready_state(document, ReadyState::Interactive))?;
        for node in deferred {
            self.run_written_script(document, node)?;
            if self.state.borrow().frames.document_write_sessions.contains_key(&document) { return Ok(()); }
        }
        let init = EventInit { bubbles: false, cancelable: false, composed: false };
        amazing_bindings::dispatch_user_agent_event(self, EventTarget::Node(document), "readystatechange", init)?;
        if self.state.borrow().frames.document_write_sessions.contains_key(&document) { return Ok(()); }
        amazing_bindings::dispatch_user_agent_event(self, EventTarget::Node(document), "DOMContentLoaded",
            EventInit { bubbles: true, ..init })?;
        if self.state.borrow().frames.document_write_sessions.contains_key(&document) { return Ok(()); }
        self.with_dom(|dom| dom.set_document_ready_state(document, ReadyState::Complete))?;
        amazing_bindings::dispatch_user_agent_event(self, EventTarget::Node(document), "readystatechange", init)?;
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
        "#).unwrap(), JsValue::str("true|html|CSS1Compat|A&B|a&b|complete|comment|x\ny"));
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
            document.open(); document.write('<p id="discarded">one');
            document.open(); document.write('<!doctype html><p id="kept">two'); document.close();
            old.dispatchEvent(new Event('x')); document.dispatchEvent(new Event('x')); window.dispatchEvent(new Event('x'));
            var states=[];
            document.open(); document.write('<p id="final">three');
            document.addEventListener('readystatechange',()=>states.push(document.readyState));
            document.addEventListener('DOMContentLoaded',()=>states.push('dom'));
            document.close(); document.close();
            [seen,document.getElementById('discarded')===null,document.getElementById('kept')===null,
             document.getElementById('final').textContent,document.compatMode,states.join(',')].join('|')
        "#).unwrap(), JsValue::str("0|true|true|three|BackCompat|interactive,dom,complete"));
    }

    #[test]
    fn nested_written_scripts_keep_input_order_and_defer_explicit_close() {
        let mut scheduler = scheduler();
        assert_eq!(scheduler.run_script(r#"
            var order=[]; document.open();
            document.write('<!doctype html><body><script>order.push("outer");document.open();document.write("<b id=inner>inner</b>");document.close();order.push("done");<\/script><i id=tail>tail</i>');
            [order.join(','),document.getElementById('inner').textContent,document.getElementById('inner').nextSibling.id,
             document.readyState,document.doctype.name].join('|')
        "#).unwrap(), JsValue::str("outer,done|inner|tail|complete|html"));
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
        "#).unwrap(), JsValue::str("42|true|complete|tail"));
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
            child.open(); child.write('<!doctype html><script>window.childRan=document===parent.frame.contentDocument<\/script><p id=c>child');child.close();
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
