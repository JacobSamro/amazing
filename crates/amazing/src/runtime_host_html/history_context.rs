//! Pure identity and fully-active checks for the browser's History adapter.
//! HTML document-sequences §7.3.3; no JS callbacks or session-history IDs.
use super::*;

/// Same-agent browsing-context identity, retained across actual proxy retargets.
/// Browser session history owns its IDs and cross-runtime/tab associations.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HtmlNavigableHandle(JsValue);
impl Eq for HtmlNavigableHandle {}

#[derive(Clone, Debug)]
pub(crate) struct HtmlHistoryContext {
    pub(crate) document: HtmlDocumentContext,
    pub(crate) realm: RealmHandle,
    pub(crate) navigable: HtmlNavigableHandle,
    pub(crate) parent_navigable: Option<HtmlNavigableHandle>,
}

impl FrameRuntime {
    fn history_context_is_current(&self, dom: &Dom, realm: RealmHandle, context: &WindowContext) -> bool {
        dom.node_generation(context.document) == Some(context.document_generation)
            && matches!(&dom.get(context.document).kind, NodeKind::Document)
            && dom.document_window_event_target(context.document) == Some(context.event_target_slot)
            && self.proxy_targets.iter().any(|(proxy, target)| *proxy == context.window && *target == realm)
            && self.live_context(context, realm)
    }

    /// Fully active means this actual current Document and every actual
    /// container Document are active. A saved old global is insufficient.
    pub(crate) fn history_context(&self, dom: &Dom, document: NodeIdx) -> Option<HtmlHistoryContext> {
        let (realm, context) = self.contexts.iter().find(|(realm, context)|
            context.document == document && self.history_context_is_current(dom, **realm, context))?;
        let mut seen = HashSet::from([*realm]);
        let mut current = context;
        let mut parent_navigable = None;
        loop {
            let Some(frame) = current.frame else {
                if current.frame_generation.is_some() || current.parent != current.window
                    || current.top != current.window || current.window != context.top { return None; }
                break;
            };
            if dom.node_generation(frame) != current.frame_generation || !dom.is_connected(frame)
                || !matches!((dom.namespace(frame), dom.local_name(frame)),
                    (Some(Namespace::Html), Some("iframe" | "frame"))) { return None; }
            let (parent_realm, parent) = self.contexts.iter().find(|(realm, candidate)|
                candidate.window == current.parent && self.history_context_is_current(dom, **realm, candidate))?;
            if dom.owner_document(frame) != Some(parent.document)
                || dom.shadow_including_root(frame) != parent.document
                || !seen.insert(*parent_realm) { return None; }
            if parent_navigable.is_none() { parent_navigable = Some(HtmlNavigableHandle(parent.window.clone())); }
            current = parent;
        }
        Some(HtmlHistoryContext { document: self.document_context(dom, document)?, realm: *realm,
            navigable: HtmlNavigableHandle(context.window.clone()), parent_navigable })
    }
}

impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn html_history_context(&self, document: NodeIdx) -> Option<HtmlHistoryContext> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        state.frames.history_context(&host.dom, document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};

    #[test]
    fn history_context_preserves_actual_navigable_identity_and_rejects_retired_documents() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><iframe id=a></iframe><iframe id=b sandbox='allow-scripts'></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let state = scheduler.test_bindings().state.clone();
        let (frame, old, original) = {
            let state = state.borrow(); let host = host.borrow();
            let root = state.frames.history_context(&host.dom, NodeIdx(0)).unwrap();
            assert!(root.parent_navigable.is_none());
            assert!(state.frames.history_context(&host.dom, NodeIdx(u32::MAX)).is_none());
            let frame = host.dom.tree_order(NodeIdx(0)).into_iter().find(|node|
                host.dom.get_attribute_ns(*node, None, "id") == Some("a")).unwrap();
            let realm = state.frames.windows[&frame].0;
            let old = state.frames.contexts[&realm].clone();
            let original = state.frames.history_context(&host.dom, old.document).unwrap();
            assert_eq!(original.realm, realm);
            assert_eq!(original.parent_navigable, Some(root.navigable));
            assert_eq!(original.document.origin, root.document.origin);
            let sandbox = state.frames.contexts.values().find(|c| c.frame.is_some_and(|f|
                host.dom.get_attribute_ns(f, None, "id") == Some("b"))).unwrap();
            let opaque = state.frames.history_context(&host.dom, sandbox.document).unwrap();
            assert_eq!(opaque.document.origin, sandbox.fetch_origin);
            assert_ne!(opaque.document.origin, original.document.origin);
            (frame, old, original)
        };
        scheduler.run_script("document.getElementById('a').srcdoc='<p>new</p>'").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        let current = {
            let state = state.borrow(); let host = host.borrow();
            assert!(state.frames.history_context(&host.dom, old.document).is_none());
            let current = state.frames.contexts[&state.frames.windows[&frame].0].clone();
            let live = state.frames.history_context(&host.dom, current.document).unwrap();
            assert_eq!(live.navigable, original.navigable);
            assert_ne!(live.realm, original.realm);
            current
        };
        host.borrow_mut().dom.detach(frame);
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
        // Exact container-generation and native association guards, independent
        // of whether the old frame map has already been swept by lifecycle.
        host.borrow_mut().dom.attach_last(NodeIdx(0), frame);
        let realm = state.borrow().frames.windows[&frame].0;
        let uncommitted = state.borrow_mut().frames.documents.remove(&frame).unwrap();
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
        state.borrow_mut().frames.documents.insert(frame, uncommitted);
        let inert = host.borrow_mut().dom.create_document();
        host.borrow_mut().dom.detach(frame);
        host.borrow_mut().dom.attach_last(inert, frame);
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
        host.borrow_mut().dom.detach(frame);
        host.borrow_mut().dom.attach_last(NodeIdx(0), frame);
        state.borrow_mut().frames.contexts.get_mut(&realm).unwrap().frame_generation =
            Some(current.frame_generation.unwrap() + 1);
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
        state.borrow_mut().frames.contexts.get_mut(&realm).unwrap().frame_generation = current.frame_generation;
        host.borrow_mut().dom.dissociate_document_window_event_target(
            current.document, current.document_generation, current.event_target_slot);
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
        // Root frame=None does not bypass actual current Window association.
        let root = state.borrow().frames.contexts.values().find(|c| c.frame.is_none()).unwrap().clone();
        host.borrow_mut().dom.dissociate_document_window_event_target(
            root.document, root.document_generation, root.event_target_slot);
        assert!(state.borrow().frames.history_context(&host.borrow().dom, root.document).is_none());
        assert!(state.borrow().frames.history_context(&host.borrow().dom, current.document).is_none());
    }
}
