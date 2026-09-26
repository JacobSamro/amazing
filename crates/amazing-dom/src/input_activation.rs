//! Checkbox/radio legacy activation state, kept outside callback execution.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivationKind { Checkbox, Radio, Other }

/// The radio button that was checked in the group before pre-click activation,
/// snapshotted by its OWN identity (name/form/root), not the clicked node's.
/// Validity at restoration time is judged against this snapshot, never by
/// re-deriving the clicked node's (possibly since-morphed) current group.
#[derive(Clone, Debug)]
struct RadioGroupPeer {
    node: NodeIdx,
    generation: u64,
    name: Option<Vec<u16>>,
    form_owner: Option<NodeIdx>,
    root: NodeIdx,
}

#[derive(Clone, Debug)]
pub struct InputActivation {
    node: NodeIdx,
    generation: u64,
    kind: ActivationKind,
    checked: bool,
    indeterminate: bool,
    previous_radio: Option<RadioGroupPeer>,
}
impl Dom {
    pub fn input_indeterminate(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["input"])?;
        Ok(self.form_state(node).is_some_and(|state| state.indeterminate))
    }
    pub fn set_input_indeterminate(&mut self, node: NodeIdx, value: bool) -> Result<(), DomError> {
        let old = self.input_indeterminate(node)?;
        self.form_state_mut(node).indeterminate = value;
        self.record_render_state(node, "indeterminate", old, value);
        Ok(())
    }
    pub fn same_radio_group(&self, left: NodeIdx, right: NodeIdx) -> bool {
        if self.input_type(left).ok() != Some("radio") || self.input_type(right).ok() != Some("radio") {
            return false;
        }
        let name = self.attribute_utf16(left, "name");
        name.as_ref().is_some_and(|name| !name.is_empty())
            && name == self.attribute_utf16(right, "name")
            && self.form_owner(left) == self.form_owner(right)
            && self.root(left) == self.root(right)
    }
    pub fn form_control_disabled(&self, node: NodeIdx) -> bool {
        if !["input", "button", "select", "textarea", "fieldset", "option", "optgroup"]
            .iter().any(|tag| self.html_tag(node, tag)) { return false; }
        if self.has(node, "disabled") { return true; }
        let mut ancestor = self.get(node).parent;
        while let Some(parent) = ancestor {
            if self.html_tag(parent, "fieldset") && self.has(parent, "disabled") {
                let legend = self.children(parent).find(|child| self.html_tag(*child, "legend"));
                if !legend.is_some_and(|legend| self.contains(legend, Some(node))) { return true; }
            }
            ancestor = self.get(parent).parent;
        }
        false
    }
    pub fn begin_html_click(&mut self, node: NodeIdx) -> Result<bool, DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { namespace: Namespace::Html, .. }) {
            return Err(DomError::TypeError);
        }
        if self.form_control_disabled(node) || self.form_state(node).is_some_and(|state| state.click_in_progress) {
            return Ok(false);
        }
        self.form_state_mut(node).click_in_progress = true;
        Ok(true)
    }
    pub fn end_html_click(&mut self, node: NodeIdx) {
        self.form_state_mut(node).click_in_progress = false;
    }
    pub(crate) fn has_html_activation_behavior(&self, node: NodeIdx) -> bool {
        ["input", "button", "label", "summary"].iter().any(|tag| self.html_tag(node, tag))
            || (["a", "area"].iter().any(|tag| self.html_tag(node, tag)) && self.has(node, "href"))
    }
    pub(crate) fn prepare_input_activation(&mut self, node: NodeIdx) -> Option<InputActivation> {
        if self.html_tag(node, "label") {
            // A label's own activation behavior is to run its labeled
            // control's activation behavior: forward to the resolved
            // control (if any -- and if it is itself activation-bearing,
            // i.e. an `<input>`; `label_control` never resolves back to a
            // `<label>`, so this cannot recurse indefinitely). A disabled
            // control's activation behavior does not run at all (WPT
            // Event-dispatch-click.tentative.html: clicking the label of a
            // disabled checkbox/radio must not check it), so that guard is
            // checked here rather than left to the recursive call, which
            // would otherwise still prepare activation for a plain
            // (non-label) disabled `<input>` click too -- a behavior change
            // outside this fix's scope.
            let control = self.label_control(node).ok()??;
            if self.form_control_disabled(control) { return None; }
            return self.prepare_input_activation(control);
        }
        if !self.html_tag(node, "input") { return None; }
        let checked = self.input_checked(node).ok()?;
        let indeterminate = self.input_indeterminate(node).ok()?;
        // The activation kind is fixed to whatever the control's type was when
        // pre-activation ran. A handler that morphs `type` mid-dispatch must
        // not change which restoration (or lack of one) later applies: HTML's
        // checkbox/radio pre-click and canceled-activation steps are captured
        // once, not re-derived from the (possibly now different) current type.
        let kind = match self.input_type(node).ok()? {
            "checkbox" => ActivationKind::Checkbox,
            "radio" => ActivationKind::Radio,
            _ => ActivationKind::Other,
        };
        let previous_radio = if kind == ActivationKind::Radio {
            (0..self.node_count()).map(|i| NodeIdx(i as u32))
                .find(|peer| (*peer == node || self.same_radio_group(node, *peer)) && self.input_checked(*peer).unwrap_or(false))
                .and_then(|peer| self.node_generation(peer).map(|generation| RadioGroupPeer {
                    node: peer, generation,
                    name: self.attribute_utf16(peer, "name"),
                    form_owner: self.form_owner(peer),
                    root: self.root(peer),
                }))
        } else { None };
        match kind {
            ActivationKind::Checkbox => {
                self.set_input_checked(node, !checked).ok()?;
                self.set_input_indeterminate(node, false).ok()?;
            }
            ActivationKind::Radio => self.set_input_checked(node, true).ok()?,
            ActivationKind::Other => {},
        }
        Some(InputActivation { node, generation: self.node_generation(node)?, kind, checked, indeterminate, previous_radio })
    }
    pub(crate) fn complete_input_activation(&mut self, state: InputActivation, canceled: bool) -> Option<NodeIdx> {
        let node = state.node;
        if self.node_generation(node) != Some(state.generation) { return None; }
        if canceled {
            match state.kind {
                ActivationKind::Checkbox => {
                    self.set_input_checked(node, state.checked).ok()?;
                    self.set_input_indeterminate(node, state.indeterminate).ok()?;
                }
                ActivationKind::Radio => {
                    // Always revert the clicked control itself first: whether
                    // or not the previously-checked peer can still be
                    // restored, `node`'s pre-activation checkedness (usually
                    // false) must win, even if `node` no longer qualifies as
                    // a radio (a handler morphed its `type`).
                    self.set_input_checked(node, state.checked).ok()?;
                    if let Some(peer) = state.previous_radio.filter(|peer| peer.node != node
                        && self.node_generation(peer.node) == Some(peer.generation)
                        && self.input_type(peer.node).ok() == Some("radio")
                        && self.attribute_utf16(peer.node, "name") == peer.name
                        && self.form_owner(peer.node) == peer.form_owner
                        && self.root(peer.node) == peer.root) {
                        self.set_input_checked(peer.node, true).ok()?;
                    }
                }
                ActivationKind::Other => {},
            }
            return None;
        }
        if !self.is_connected(node) { return None; }
        matches!(state.kind, ActivationKind::Checkbox | ActivationKind::Radio).then_some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DispatchOrigin, Event, EventClass, EventInit, EventTarget};
    use std::time::Duration;
    fn click(dom: &mut Dom, node: NodeIdx) -> crate::events::Dispatch {
        let mut event = Event::new("click", EventInit { bubbles: true, cancelable: true, composed: true }, Duration::ZERO);
        event.set_class(EventClass::Mouse);
        let id = dom.events_mut().create_event(event);
        dom.begin_event_dispatch(EventTarget::Node(node), id, DispatchOrigin::Script).unwrap()
    }
    fn finish(dom: &mut Dom, dispatch: &mut crate::events::Dispatch) -> Option<NodeIdx> {
        while dom.events_mut().next_listener(dispatch).is_some() {}
        dom.finish_input_activation(dispatch)
    }
    #[test]
    fn canceled_checkbox_restores_checked_and_indeterminate_after_dispatch_cleanup() {
        let mut dom = Dom::new();
        let node = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(node, "type", "checkbox").unwrap();
        dom.set_input_indeterminate(node, true).unwrap();
        let mut dispatch = click(&mut dom, node);
        assert!(dom.input_checked(node).unwrap());
        assert!(!dom.input_indeterminate(node).unwrap());
        dom.events_mut().event_mut(dispatch.event()).prevent_default();
        assert_eq!(finish(&mut dom, &mut dispatch), None);
        assert!(!dom.input_checked(node).unwrap());
        assert!(dom.input_indeterminate(node).unwrap());
        assert_eq!(finish(&mut dom, &mut dispatch), None);
    }
    #[test]
    fn radio_rollback_rechecks_the_current_group_and_unnamed_radios_stay_independent() {
        let mut dom = Dom::new();
        let a = dom.create_element(NodeIdx(0), "input");
        let b = dom.create_element(NodeIdx(0), "input");
        for node in [a, b] { dom.set_attribute(node, "type", "radio").unwrap(); dom.set_input_checked(node, true).unwrap(); }
        assert!(dom.input_checked(a).unwrap() && dom.input_checked(b).unwrap());
        for node in [a, b] { dom.set_attribute(node, "name", "g").unwrap(); }
        dom.set_input_checked(a, true).unwrap();
        let mut dispatch = click(&mut dom, b);
        assert!(!dom.input_checked(a).unwrap() && dom.input_checked(b).unwrap());
        dom.set_attribute(a, "name", "other").unwrap();
        dom.events_mut().event_mut(dispatch.event()).prevent_default();
        assert_eq!(finish(&mut dom, &mut dispatch), None);
        assert!(!dom.input_checked(a).unwrap() && !dom.input_checked(b).unwrap());
    }
    #[test]
    fn disabled_click_guard_does_not_suppress_script_dispatch_activation() {
        let mut dom = Dom::new();
        let fieldset = dom.create_element(NodeIdx(0), "fieldset");
        dom.set_attribute(fieldset, "disabled", "").unwrap();
        let node = dom.create_element(fieldset, "input");
        dom.set_attribute(node, "type", "checkbox").unwrap();
        assert!(!dom.begin_html_click(node).unwrap());
        let mut dispatch = click(&mut dom, node);
        assert!(dom.input_checked(node).unwrap());
        assert_eq!(finish(&mut dom, &mut dispatch), Some(node));
        dom.remove_attribute(fieldset, "disabled").unwrap();
        assert!(dom.begin_html_click(node).unwrap());
        assert!(!dom.begin_html_click(node).unwrap());
        dom.end_html_click(node);
        assert!(dom.begin_html_click(node).unwrap());
        dom.end_html_click(node);
    }
    #[test]
    fn canceled_activation_restores_by_the_type_captured_at_prepare_time_even_after_a_type_morph() {
        // WPT dom/events/Event-dispatch-click.tentative.html: a handler that
        // changes `type` before calling preventDefault() must not escape the
        // checkbox/radio restoration that pre-activation already queued.
        let mut dom = Dom::new();
        let checkbox = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(checkbox, "type", "checkbox").unwrap();
        let mut dispatch = click(&mut dom, checkbox);
        assert!(dom.input_checked(checkbox).unwrap());
        dom.set_attribute(checkbox, "type", "date").unwrap();
        dom.events_mut().event_mut(dispatch.event()).prevent_default();
        assert_eq!(finish(&mut dom, &mut dispatch), None);
        assert!(!dom.input_checked(checkbox).unwrap());

        let radio1 = dom.create_element(NodeIdx(0), "input");
        let radio2 = dom.create_element(NodeIdx(0), "input");
        for node in [radio1, radio2] { dom.set_attribute(node, "type", "radio").unwrap(); dom.set_attribute(node, "name", "foo").unwrap(); }
        dom.set_input_checked(radio2, true).unwrap();
        let mut dispatch = click(&mut dom, radio1);
        assert!(dom.input_checked(radio1).unwrap() && !dom.input_checked(radio2).unwrap());
        dom.set_attribute(radio1, "type", "date").unwrap();
        dom.events_mut().event_mut(dispatch.event()).prevent_default();
        assert_eq!(finish(&mut dom, &mut dispatch), None);
        assert!(!dom.input_checked(radio1).unwrap() && dom.input_checked(radio2).unwrap());
    }
}
