//! Canonical storage for the driver's existing inline-handler compiler subset.
//!
//! This preserves the previous function(event) compiler. HTML's additional
//! Element/Form/Document lexical environments and CSP inline-handler checks
//! remain separate compiler work; this adapter does not claim those semantics.
use super::*;

impl RuntimeBindings {
    pub(crate) fn install_canonical_inline_handler(&self, js: &mut Interpreter,
        node: u32, event: &str, body: &str) -> Result<(), Throw> {
        let node = NodeIdx(node);
        let (generation, document, realm, context) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let generation = host.dom.node_generation(node)
                .ok_or_else(|| Throw::type_error("Invalid inline handler node"))?;
            let document = host.dom.owner_document(node)
                .ok_or_else(|| Throw::type_error("Inline handler has no owner Document"))?;
            let context = state.frames.document_context(&host.dom, document)
                .ok_or_else(|| Throw::type_error("Inline handler Document is not active"))?;
            let realm = state.frames.document_realm(&host.dom, document)
                .ok_or_else(|| Throw::type_error("Inline handler Document has no realm"))?;
            if !host.dom.document_scripting_enabled(document).unwrap_or(false) { return Ok(()); }
            (generation, document, realm, context)
        };
        let source = format!("(function(event){{{body}}})");
        let callback = js.binding_with_realm(realm, |js|
            js.run_script_with_source_context(&source, &context.document_url))?;
        if !js.binding_is_callable(&callback) {
            return Err(Throw::type_error("Inline event handler is not callable"));
        }
        // Compilation and realm entry run without any arena/state borrow.
        // Recheck identity before publishing the rooted callback.
        {
            let state = self.state.borrow();
            let host = state.host.borrow();
            if host.dom.node_generation(node) != Some(generation)
                || host.dom.owner_document(node) != Some(document)
                || state.frames.document_realm(&host.dom, document) != Some(realm) {
                return Err(Throw::type_error("Inline handler owner changed during compilation"));
            }
        }
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        let root = adapter.retain_value(callback);
        let old = self.state.borrow_mut().host.borrow_mut().dom.events_mut()
            .set_event_handler(amazing_dom::events::EventTarget::Node(node), event, Some(root));
        if let Some(old) = old { adapter.release_value(old); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};
    #[test]
    fn canonical_inline_handler_replacement_preserves_order_and_cancellation() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><input id=check type=checkbox>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/handler/page.html");
        let node = host.borrow().dom.children(host.borrow().dom.document_body(NodeIdx(0)).unwrap().unwrap()).next().unwrap();
        scheduler.run_script("var trace=[];document.getElementById('check').addEventListener('click',()=>trace.push('before'))").unwrap();
        scheduler.install_inline_handler(node.0, "click", "trace.push(this.id+':'+event.isTrusted);return false").unwrap();
        scheduler.run_script("document.getElementById('check').addEventListener('click',()=>trace.push('after'))").unwrap();
        let event = || {
            let mut event = amazing_dom::events::Event::new("click", amazing_dom::events::EventInit {
                bubbles:true, cancelable:true, composed:true }, Duration::ZERO);
            event.set_class(amazing_dom::events::EventClass::Mouse); event
        };
        assert!(!scheduler.dispatch_user_agent_payload(amazing_dom::events::EventTarget::Node(node), event()).unwrap());
        assert_eq!(scheduler.run_script("trace.join('|')+';'+document.getElementById('check').checked").unwrap(),
            JsValue::str("before|check:true|after;false"));
        scheduler.install_inline_handler(node.0, "click", "trace.push('replacement')").unwrap();
        assert!(scheduler.dispatch_user_agent_payload(amazing_dom::events::EventTarget::Node(node), event()).unwrap());
        assert_eq!(scheduler.run_script("trace.join('|')+';'+document.getElementById('check').checked").unwrap(),
            JsValue::str("before|check:true|after|before|replacement|after;true"));
        let state = scheduler.test_bindings().state.borrow();
        assert_eq!(state.roots.iter().filter(|value|value.is_none()).count(), 1,
            "replacing the handler releases its previous callback root");
    }
}
