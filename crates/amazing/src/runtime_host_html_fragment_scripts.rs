//! Ordinary markup setters make their actual imported scripts inert before insertion.
//! The temporary parser arena and connected-tree differences cannot identify
//! scripts later adopted from detached elements or template contents.
use super::*;

impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn html_mark_markup_fragment_scripts(
        &mut self,
        fragment: NodeIdx,
    ) -> Result<(), DomError> {
        let scripts = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            if host.dom.node_generation(fragment).is_none()
                || !matches!(&host.dom.get(fragment).kind, NodeKind::DocumentFragment)
            {
                return Err(DomError::TypeError);
            }
            let mut pending = vec![fragment];
            let mut visited = HashSet::new();
            let mut scripts = Vec::new();
            while let Some(node) = pending.pop() {
                if !visited.insert(node) { continue; }
                if matches!(host.dom.namespace(node), Some(Namespace::Html) | Some(Namespace::Svg))
                    && host.dom.local_name(node) == Some("script")
                {
                    scripts.push(node);
                }
                pending.extend(host.dom.children(node));
                if let Some(contents) = host.dom.template_contents(node) {
                    pending.push(contents);
                }
            }
            scripts
        };
        // No DOM borrow survives this existing execution-policy update.
        self.state.borrow_mut().frames.already_started_scripts.extend(scripts);
        Ok(())
    }
}
