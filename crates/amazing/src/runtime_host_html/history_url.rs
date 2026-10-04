//! Projection of an already validated same-document session-history URL.
//! This performs no conversion, navigation, origin change or author callback.
use super::*;

impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn html_commit_history_url(&mut self, document: NodeIdx,
        generation: u64, url: &ParsedUrl) -> Result<(), Throw> {
        commit_history_url(self.js, self.state.clone(), document, generation, url, |_| Ok(()))
    }
}

fn commit_history_url(js: &mut Interpreter, state: Rc<RefCell<BindingState>>, document: NodeIdx,
    generation: u64, url: &ParsedUrl,
    stage: impl FnOnce(&mut Interpreter) -> Result<(), Throw>) -> Result<(), Throw> {
    let (context, mut metadata) = {
        let state = state.borrow(); let host = state.host.borrow();
        let context = state.frames.history_context(&host.dom, document)
            .filter(|context| context.document.document_generation == generation)
            .ok_or_else(|| Throw::type_error("History document is not fully active"))?;
        let metadata = host.dom.document_metadata(document).map_err(|_| Throw::type_error("Invalid History document"))?
            .cloned().or_else(|| {
                // Active root metadata is supplied by the embedding's selected
                // response decoder. Never infer MIME or encoding from the URL.
                state.frames.contexts.get(&context.realm).filter(|c| c.frame.is_none()).and_then(|_|
                    Some(amazing_dom::document_surface::DocumentMetadata {
                        url: context.document.document_url.clone(),
                        content_type: state.document_content_type.clone()?,
                        character_set: state.document_character_set.clone()?,
                    }))
            }).ok_or_else(|| Throw::type_error("History document response metadata is unavailable"))?;
        (context, metadata)
    };
    let serialized = url.serialize();
    metadata.url = serialized.clone();
    // Actual realm entry can fail. It happens before any native URL, metadata
    // or Location mutation. The production stage cannot invoke author code.
    js.binding_with_realm(context.realm, |js| {
        stage(js)?;
        let mut state = state.borrow_mut();
        let host = state.host.clone(); let mut host = host.borrow_mut();
        // Nothing can change these identities between validation and this
        // native-only commit: no callbacks, conversion or event dispatch occur.
        host.dom.set_document_metadata(document, metadata).expect("validated native History Document");
        state.frames.contexts.get_mut(&context.realm).expect("validated actual History realm")
            .document_url = serialized.clone();
        js.set_location_url(serialized);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_url_projection_preserves_identity_origin_policy_and_pending_navigation() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe></iframe>"), native_forwarded: false }));
        let mut js = Interpreter::new(); js.set_dom_host(host.clone());
        let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
        bindings.set_document_metadata("windows-1252", "text/html", "");
        bindings.set_document_url(&mut js, "https://example.test/root");
        let state = bindings.state.clone();
        let original = state.borrow().frames.history_context(&host.borrow().dom, NodeIdx(0)).unwrap();
        let root = state.borrow().frames.contexts[&original.realm].clone();
        js.binding_location_navigate("https://example.test/pending", false);
        let url = ParsedUrl::parse("https://example.test/root?query#fragment").unwrap();
        commit_history_url(&mut js, state.clone(), root.document, root.document_generation, &url, |_| Ok(())).unwrap();
        let after = state.borrow().frames.history_context(&host.borrow().dom, root.document).unwrap();
        assert_eq!(after.document.document_url, url.serialize());
        assert_eq!(after.document.origin, original.document.origin);
        assert_eq!(after.document.fallback_base_url, original.document.fallback_base_url);
        assert_eq!(after.navigable, original.navigable);
        assert_eq!(after.realm, original.realm);
        let metadata = host.borrow().dom.document_metadata(root.document).unwrap().cloned().unwrap();
        assert_eq!(metadata.url, url.serialize());
        assert_eq!(metadata.character_set, "windows-1252"); assert_eq!(metadata.content_type, "text/html");
        assert_eq!(js.binding_location_href(), url.serialize());
        assert_eq!(js.take_navigation(), Some("https://example.test/pending".into()));
        let child = state.borrow().frames.contexts.values().find(|context| context.frame.is_some()).unwrap().clone();
        let child_before = state.borrow().frames.history_context(&host.borrow().dom, child.document).unwrap();
        let child_url = ParsedUrl::parse("about:blank#history").unwrap();
        commit_history_url(&mut js, state.clone(), child.document, child.document_generation,
            &child_url, |_| Ok(())).unwrap();
        let child_after = state.borrow().frames.history_context(&host.borrow().dom, child.document).unwrap();
        assert_eq!(child_after.document.document_url, child_url.serialize());
        assert_eq!(child_after.document.origin, child_before.document.origin);
        assert_eq!(child_after.document.fallback_base_url, child_before.document.fallback_base_url);
        assert_eq!(child_after.navigable, child_before.navigable);
        assert_eq!(js.binding_location_href(), url.serialize());
        assert_eq!(js.binding_with_realm(child_after.realm, |js| Ok(js.binding_location_href().to_owned())).unwrap(), child_url.serialize());
        assert!(js.take_navigation().is_none());
        let failed_url = ParsedUrl::parse("https://example.test/failed").unwrap();
        assert!(commit_history_url(&mut js, state.clone(), root.document, root.document_generation,
            &failed_url, |_| Err(Throw::type_error("injected precommit failure"))).is_err());
        assert_eq!(js.binding_location_href(), url.serialize());
        assert_eq!(host.borrow().dom.document_metadata(root.document).unwrap(), Some(&metadata));
        assert_eq!(state.borrow().frames.contexts[&original.realm].document_url, url.serialize());
        assert!(commit_history_url(&mut js, state.clone(), root.document, root.document_generation + 1,
            &failed_url, |_| Ok(())).is_err());
        assert_eq!(js.binding_location_href(), url.serialize());
        let foreign = Interpreter::new();
        let foreign_realm = foreign.binding_current_realm();
        let saved = state.borrow_mut().frames.contexts.remove(&original.realm).unwrap();
        state.borrow_mut().frames.contexts.insert(foreign_realm, saved);
        state.borrow_mut().frames.record_proxy_target(&root.window, foreign_realm);
        assert!(commit_history_url(&mut js, state.clone(), root.document, root.document_generation,
            &failed_url, |_| Ok(())).is_err());
        assert_eq!(js.binding_location_href(), url.serialize());
        assert_eq!(host.borrow().dom.document_metadata(root.document).unwrap(), Some(&metadata));
        assert_eq!(state.borrow().frames.contexts[&foreign_realm].document_url, url.serialize());
    }
}
