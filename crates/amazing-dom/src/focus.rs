//! Document focus state and the programmatically focusable-area predicate.
use crate::{Dom, Namespace, NodeIdx, NodeKind};

impl Dom {
    pub fn active_element(&self, document: NodeIdx) -> Option<NodeIdx> {
        self.focused_elements.iter().find(|(owner, _, _)| *owner == document)
            .map(|(_, node, _)| *node)
            .filter(|node| self.node_generation(*node).is_some() && self.is_connected(*node))
    }

    pub fn is_focused(&self, node: NodeIdx) -> bool {
        self.owner_document(node).is_some_and(|document| self.active_element(document) == Some(node))
    }

    pub fn is_focus_visible(&self, node: NodeIdx) -> bool {
        self.focused_elements.iter().any(|(_, focused, visible)| *focused == node && *visible)
            && self.is_focused(node)
    }

    pub fn is_focus_within(&self, node: NodeIdx) -> bool {
        self.owner_document(node).and_then(|document| self.active_element(document))
            .is_some_and(|active| self.contains(node, Some(active)))
    }

    pub fn is_focusable_area(&self, node: NodeIdx) -> bool {
        if self.node_generation(node).is_none() || !self.is_connected(node)
            || !matches!(self.get(node).kind, NodeKind::Element { .. }) { return false; }
        let mut ancestor = Some(node);
        while let Some(current) = ancestor {
            if self.attribute_utf16(current, "inert").is_some()
                || self.attribute_utf16(current, "hidden").is_some() { return false; }
            ancestor = self.get(current).parent;
        }
        let name = self.local_name(node).unwrap_or("");
        let html = self.namespace(node) == Some(Namespace::Html);
        if html && matches!(name, "button" | "input" | "select" | "textarea" | "fieldset" | "optgroup" | "option")
            && self.attribute_utf16(node, "disabled").is_some() { return false; }
        // HTML's actually-disabled predicate includes ancestor fieldsets, but
        // exempts descendants of each fieldset's first HTML legend child.
        // This must precede tabindex: tabindex cannot enable disabled controls.
        if html && matches!(name, "button" | "input" | "select" | "textarea" | "fieldset") {
            let mut ancestor = self.get(node).parent;
            while let Some(parent) = ancestor {
                if self.namespace(parent) == Some(Namespace::Html)
                    && self.local_name(parent) == Some("fieldset")
                    && self.attribute_utf16(parent, "disabled").is_some()
                {
                    let legend = self.children(parent).find(|child|
                        self.namespace(*child) == Some(Namespace::Html)
                            && self.local_name(*child) == Some("legend"));
                    if !legend.is_some_and(|legend| self.contains(legend, Some(node))) {
                        return false;
                    }
                }
                ancestor = self.get(parent).parent;
            }
        }
        if html && self.is_form_associated_custom_element(node) && self.is_actually_disabled(node) { return false; }
        if html && name == "input" && self.attribute_utf16(node, "type")
            .is_some_and(|value| String::from_utf16_lossy(&value).eq_ignore_ascii_case("hidden")) { return false; }
        if self.attribute_utf16(node, "tabindex").is_some_and(|value| {
            let text = String::from_utf16_lossy(&value);
            text.trim().parse::<i32>().is_ok()
        }) { return true; }
        if html && matches!(name, "input" | "button" | "select" | "textarea" | "iframe" | "object") { return true; }
        if html && matches!(name, "a" | "area") && self.attribute_utf16(node, "href").is_some() { return true; }
        if html && name == "summary" && self.get(node).parent.is_some_and(|parent|
            self.local_name(parent) == Some("details")
                && self.children(parent).find(|child| self.local_name(*child) == Some("summary")) == Some(node)) { return true; }
        if html && self.attribute_utf16(node, "contenteditable").is_some_and(|value| {
            let value = String::from_utf16_lossy(&value);
            value.is_empty() || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("plaintext-only")
        }) { return true; }
        false
    }

    /// Returns the old focused element when the focus actually changes.
    pub fn set_active_element(&mut self, document: NodeIdx, new: Option<NodeIdx>, visible: bool)
        -> Option<Option<NodeIdx>> {
        let old = self.active_element(document);
        if old == new { return None; }
        let old_visible = old.is_some_and(|node| self.is_focus_visible(node));
        if let Some(index) = self.focused_elements.iter().position(|(owner, _, _)| *owner == document) {
            self.focused_elements.remove(index);
        }
        if let Some(new) = new { self.focused_elements.push((document, new, visible)); }
        if let Some(old) = old {
            self.record_render_state(old, "focus", true, false);
            self.record_render_state(old, "focus-visible", old_visible, false);
        }
        if let Some(new) = new {
            self.record_render_state(new, "focus", false, true);
            self.record_render_state(new, "focus-visible", false, visible);
        }
        Some(old)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Dom, NodeIdx};

    #[test]
    fn focusability_and_document_state_follow_connected_elements() {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let html = dom.create_element(document, "html");
        let body = dom.create_element(html, "body");
        let input = dom.create_element(body, "input");
        let div = dom.create_element(body, "div");
        assert!(dom.is_focusable_area(input));
        assert!(!dom.is_focusable_area(div));
        dom.set_attribute(div, "tabindex", "-1").unwrap();
        assert!(dom.is_focusable_area(div));
        assert_eq!(dom.set_active_element(document, Some(input), true), Some(None));
        assert!(dom.is_focused(input));
        assert!(dom.is_focus_visible(input));
        assert!(dom.is_focus_within(body));
        assert_eq!(dom.set_active_element(document, Some(div), false), Some(Some(input)));
        assert!(!dom.is_focused(input));
        assert!(dom.is_focused(div));
        assert!(!dom.is_focus_visible(div));
        dom.set_attribute(body, "inert", "").unwrap();
        assert!(!dom.is_focusable_area(input));
        assert!(!dom.is_focusable_area(div));
    }

    #[test]
    fn disabled_fieldsets_prevent_focus_except_in_the_first_legend() {
        let mut dom = Dom::new();
        let body = dom.create_element(NodeIdx(0), "body");
        let fieldset = dom.create_element(body, "fieldset");
        dom.set_attribute(fieldset, "disabled", "").unwrap();
        let first = dom.create_element(fieldset, "legend");
        let allowed = dom.create_element(first, "input");
        let second = dom.create_element(fieldset, "legend");
        let blocked = dom.create_element(second, "input");
        assert!(dom.is_focusable_area(allowed));
        assert!(!dom.is_focusable_area(blocked));
        for tag in ["input", "button", "select", "textarea", "fieldset"] {
            let control = dom.create_element(fieldset, tag);
            dom.set_attribute(control, "tabindex", "0").unwrap();
            assert!(!dom.is_focusable_area(control), "{tag}");
        }
        let link = dom.create_element(fieldset, "a");
        dom.set_attribute(link, "href", "#").unwrap();
        assert!(dom.is_focusable_area(link));
        // An inner first legend does not exempt its controls from the outer
        // disabled fieldset when the inner fieldset is outside its legend.
        let inner = dom.create_element(fieldset, "fieldset");
        dom.set_attribute(inner, "disabled", "").unwrap();
        let inner_legend = dom.create_element(inner, "legend");
        let nested = dom.create_element(inner_legend, "input");
        assert!(!dom.is_focusable_area(nested));
        dom.remove_attribute(fieldset, "disabled").unwrap();
        assert!(dom.is_focusable_area(nested));
        assert!(dom.is_focusable_area(blocked));
    }

}
