//! Actual WindowGlobal EventTarget identities, independent of proxy retargeting.
use super::*;

impl FrameRuntime {
    pub(super) fn record_proxy_target(&mut self, proxy: &JsValue, realm: RealmHandle) {
        if let Some((_, target)) = self.proxy_targets.iter_mut().find(|(value, _)| value == proxy) {
            *target = realm;
        } else { self.proxy_targets.push((proxy.clone(), realm)); }
    }
    fn window_event_target_slot(&self, value: &JsValue) -> Option<u64> {
        // Saved underlying globals keep their immutable identities after a
        // navigation. A proxy consults only its actual latest target record.
        if let Some(context) = self.contexts.values().find(|context| context.global == *value) {
            return Some(context.event_target_slot);
        }
        let realm = self.proxy_targets.iter().find(|(proxy, _)| proxy == value)?.1;
        self.contexts.get(&realm).map(|context| context.event_target_slot)
    }
    fn window_event_target_value(&self, slot: u64) -> Option<JsValue> {
        let (realm, context) = self.contexts.iter().find(|(_, context)| context.event_target_slot == slot)?;
        let current = self.proxy_targets.iter().any(|(proxy, target)| *target == *realm && *proxy == context.window);
        Some(if current { context.window.clone() } else { context.global.clone() })
    }
}
impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn html_window_event_target_slot(&self, value: &JsValue) -> Option<u64> {
        self.state.borrow().frames.window_event_target_slot(value)
    }
    pub(in crate::runtime_binding_host) fn html_window_event_target_value(&self, slot: u64) -> Option<JsValue> {
        self.state.borrow().frames.window_event_target_value(slot)
    }
}
impl RuntimeBindings {
    pub(super) fn retire_window_event_association(&self, realm: RealmHandle) {
        let (host, context) = {
            let state = self.state.borrow();
            (state.host.clone(), state.frames.contexts.get(&realm).cloned())
        };
        if let Some(context) = context {
            host.borrow_mut().dom.dissociate_document_window_event_target(
                context.document, context.document_generation, context.event_target_slot);
        }
    }
    pub(super) fn retire_disconnected_window_event_associations(&self) {
        let (host, retired) = {
            let state = self.state.borrow();
            let retired: Vec<_> = state.frames.contexts.iter().filter(|(realm, context)|
                !state.frames.live_context(context, **realm)).map(|(_, context)| context.clone()).collect();
            (state.host.clone(), retired)
        };
        let mut host = host.borrow_mut();
        for context in retired {
            host.dom.dissociate_document_window_event_target(
                context.document, context.document_generation, context.event_target_slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
    #[test]
    fn actual_window_targets_survive_retarget_and_retire_exact_document_paths() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><iframe id='a'></iframe><iframe id='b'></iframe>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root.html");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let state = scheduler.test_bindings().state.clone();
        let (root_slot, frame, old_realm, old) = {
            let state = state.borrow();
            let root_slot = state.window_event_target_slot;
            let frame = host.borrow().dom.tree_order(NodeIdx(0)).into_iter().find(|node|
                host.borrow().dom.get_attribute_ns(*node, None, "id") == Some("a")).unwrap();
            let old_realm = state.frames.windows[&frame].0;
            let old = state.frames.contexts[&old_realm].clone();
            let slots: HashSet<_> = state.frames.contexts.values().map(|context| context.event_target_slot).collect();
            assert_eq!(slots.len(), 3);
            assert_ne!(old.event_target_slot, root_slot);
            for context in state.frames.contexts.values() {
                assert_eq!(state.frames.window_event_target_slot(&context.global), Some(context.event_target_slot));
                assert_eq!(state.frames.window_event_target_slot(&context.window), Some(context.event_target_slot));
                assert_eq!(host.borrow().dom.document_window_event_target(context.document), Some(context.event_target_slot));
            }
            (root_slot, frame, old_realm, old)
        };
        scheduler.run_script("document.getElementById('a').srcdoc='<p>new</p>'").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let current = {
            let state = state.borrow();
            let new_realm = state.frames.windows[&frame].0;
            assert_ne!(new_realm, old_realm);
            let current = state.frames.contexts[&new_realm].clone();
            assert_eq!(current.window, old.window);
            assert_ne!(current.event_target_slot, old.event_target_slot);
            assert_eq!(state.frames.window_event_target_slot(&old.global), Some(old.event_target_slot));
            assert_eq!(state.frames.window_event_target_slot(&old.window), Some(current.event_target_slot));
            assert_eq!(state.frames.window_event_target_value(old.event_target_slot), Some(old.global.clone()));
            assert_eq!(state.frames.window_event_target_value(current.event_target_slot), Some(current.window.clone()));
            current
        };
        assert_eq!(host.borrow().dom.document_window_event_target(old.document), None);
        assert!(host.borrow().dom.is_window_event_target(old.event_target_slot));
        scheduler.run_script("document.getElementById('a').remove()").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let state = state.borrow();
        assert_eq!(host.borrow().dom.document_window_event_target(current.document), None);
        assert_eq!(host.borrow().dom.document_window_event_target(NodeIdx(0)), Some(root_slot));
        assert_eq!(state.frames.window_event_target_slot(&current.window), Some(current.event_target_slot));
        assert_eq!(state.frames.window_event_target_value(current.event_target_slot), Some(current.window));
        assert_eq!(state.frames.window_event_target_slot(&JsValue::Null), None);
        assert_eq!(state.frames.window_event_target_slot(&JsValue::str("window")), None);
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
    #[test]
    fn window_load_uses_document_target_waits_for_children_and_reflects_body_handlers() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(r###"<!doctype html><body><iframe id=child srcdoc="&lt;!doctype html&gt;&lt;script&gt;window.addEventListener(&quot;load&quot;,function(e){parent.trace.push(&quot;childWindow:&quot;+(this===window)+&quot;:&quot;+(e.target===document)+&quot;:&quot;+e.bubbles)})&lt;/script&gt;&lt;body onload=&quot;parent.trace.push(&#x27;childBody:&#x27;+(this===window))&quot;&gt;&lt;iframe srcdoc=&quot;&amp;lt;!doctype html&amp;gt;&amp;lt;body onload=&amp;quot;parent.parent.trace.push(&amp;#x27;nestedBody:&amp;#x27;+(this===window)+&amp;#x27;:&amp;#x27;+(event.target===document))&amp;quot;&amp;gt;nested&quot;&gt;&lt;/iframe&gt;"></iframe>"###), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/load/root.html");
        scheduler.run_script(r#"
            var trace=[], documentLoads=0, rootLoads=0;
            document.addEventListener('DOMContentLoaded',function(e){trace.push('dcl:'+(e.target===document)+':'+e.bubbles)});
            document.addEventListener('load',()=>documentLoads++);
            document.getElementById('child').addEventListener('load',function(e){trace.push('frame:'+(e.target===this)+':'+e.bubbles)});
            window.addEventListener('load',function(e){rootLoads++;trace.push('rootWindow:'+(this===window)+':'+(e.target===document)+':'+e.isTrusted+':'+e.bubbles+':'+e.cancelable)});
        "#).unwrap();
        let body = host.borrow().dom.document_body(NodeIdx(0)).unwrap().unwrap();
        scheduler.install_inline_handler(body.0,"load","trace.push('rootBody:'+(this===window)+':'+(event.target===document))").unwrap();
        scheduler.dispatch_dom_event(0,"DOMContentLoaded").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.dispatch_document_load(NodeIdx(0)).unwrap();
        // Draining again must not manufacture a second child or iframe load.
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("trace.join('|')").unwrap(),JsValue::str(
            "dcl:true:true|nestedBody:true:true|childWindow:true:true:false|childBody:true|frame:true:false|rootWindow:true:true:true:false:false|rootBody:true:true"));
        assert_eq!(scheduler.run_script("documentLoads+':'+rootLoads").unwrap(),JsValue::str("0:1"));
        let inert = host.borrow_mut().dom.create_document();
        scheduler.dispatch_document_load(inert).unwrap();
        assert_eq!(scheduler.run_script("rootLoads").unwrap(),JsValue::num(1.0));
        // Public Document dispatch remains a normal Document event and cannot
        // acquire HTML's internal legacy Window target override.
        scheduler.run_script("document.dispatchEvent(new Event('load'));documentLoads").map(|v| assert_eq!(v,JsValue::num(1.0))).unwrap();
    }
}
