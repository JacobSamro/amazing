//! HTML Standard §4.10 form state. State belongs to arena nodes; association
//! and collection membership are computed from the current tree.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Debug, Default)]
pub struct ControlState {
    value: Option<Vec<u16>>,
    dirty_value: bool,
    checked: Option<bool>,
    dirty_checked: bool,
    selected: Option<bool>,
    dirty_selected: bool,
    custom_error: Vec<u16>,
    selection: Option<(u32, u32, SelectionDirection)>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionDirection { #[default] None, Forward, Backward }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueMode { Value, Default, DefaultOn, Filename }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidityState {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
}
impl ValidityState {
    pub fn valid(&self) -> bool { self == &Self::default() }
}
fn units(value: &str) -> Vec<u16> { value.encode_utf16().collect() }
fn ascii_lower(value: &str) -> String { value.to_ascii_lowercase() }
fn strip_newlines(value: &[u16]) -> Vec<u16> {
    value.iter().copied().filter(|u| *u != 10 && *u != 13).collect()
}
fn normalize_newlines(value: &[u16]) -> Vec<u16> {
    let mut result = Vec::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        if value[i] == 13 { result.push(10); if value.get(i + 1) == Some(&10) { i += 1; } }
        else { result.push(value[i]); }
        i += 1;
    }
    result
}
fn valid_number(value: &str) -> Option<f64> {
    if value.is_empty() || value.trim() != value || value.starts_with('+') { return None; }
    let n: f64 = value.parse().ok()?;
    n.is_finite().then_some(n)
}
fn date_shape(value: &str, kind: &str) -> bool {
    let b = value.as_bytes();
    let digits = |a: usize, z: usize| b.get(a..z).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let date = || b.len() == 10 && digits(0, 4) && b[4] == b'-' && digits(5, 7) && b[7] == b'-' && digits(8, 10)
        && (1..=12).contains(&value[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&value[8..10].parse::<u8>().unwrap_or(0));
    let time = |offset: usize| b.len() >= offset + 5 && digits(offset, offset+2) && b[offset+2] == b':' && digits(offset+3, offset+5)
        && value[offset..offset+2].parse::<u8>().unwrap_or(99) < 24
        && value[offset+3..offset+5].parse::<u8>().unwrap_or(99) < 60;
    match kind {
        "date" => date(),
        "month" => b.len() == 7 && digits(0, 4) && b[4] == b'-' && digits(5, 7) && (1..=12).contains(&value[5..7].parse::<u8>().unwrap_or(0)),
        "week" => b.len() == 8 && digits(0, 4) && b[4..6] == *b"-W" && digits(6, 8) && (1..=53).contains(&value[6..8].parse::<u8>().unwrap_or(0)),
        "time" => (b.len() == 5 || b.len() == 8 || b.len() > 9) && time(0),
        "datetime-local" => b.len() >= 16 && b.is_ascii() && date_shape(&value[..10], "date") && b[10] == b'T' && time(11),
        _ => false,
    }
}
pub fn sanitize_input(kind: &str, value: &[u16], min: Option<&str>, max: Option<&str>) -> Vec<u16> {
    let cleaned = match kind {
        "text" | "search" | "tel" | "url" | "email" | "password" => strip_newlines(value),
        _ => value.to_vec(),
    };
    let s = String::from_utf16_lossy(&cleaned);
    match kind {
        "number" if valid_number(&s).is_none() => Vec::new(),
        "range" => {
            let lo = min.and_then(valid_number).unwrap_or(0.);
            let hi = max.and_then(valid_number).filter(|n| *n >= lo).unwrap_or(100.);
            let default = if hi < lo { lo } else { (lo + hi) / 2. };
            units(&valid_number(&s).unwrap_or(default).clamp(lo, hi).to_string())
        }
        "color" => {
            if s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) { units(&ascii_lower(&s)) }
            else { units("#000000") }
        }
        "date" | "month" | "week" | "time" | "datetime-local" if !date_shape(&s, kind) => Vec::new(),
        _ => cleaned,
    }
}
impl Dom {
    fn form_state(&self, node: NodeIdx) -> Option<&ControlState> {
        self.form_states.get(node.0 as usize).and_then(Option::as_ref)
    }
    fn form_state_mut(&mut self, node: NodeIdx) -> &mut ControlState {
        let index = node.0 as usize;
        if self.form_states.len() <= index { self.form_states.resize_with(index + 1, || None); }
        self.form_states[index].get_or_insert_with(ControlState::default)
    }
    pub fn html_tag(&self, node: NodeIdx, tag: &str) -> bool {
        matches!(&self.get(node).kind, NodeKind::Element { name, namespace: Namespace::Html, .. } if name.eq_ignore_ascii_case(tag))
    }
    fn require_tag(&self, node: NodeIdx, tags: &[&str]) -> Result<(), DomError> {
        self.check_node(node)?;
        if tags.iter().any(|tag| self.html_tag(node, tag)) { Ok(()) } else { Err(DomError::TypeError) }
    }
    fn attr(&self, node: NodeIdx, name: &str) -> Option<&str> { self.get_attribute_ns(node, None, name) }
    fn has(&self, node: NodeIdx, name: &str) -> bool { self.attr(node, name).is_some() }
    pub fn input_type(&self, node: NodeIdx) -> Result<&'static str, DomError> {
        self.require_tag(node, &["input"])?;
        const TYPES: &[&str] = &["hidden", "text", "search", "tel", "url", "email", "password", "date", "month", "week", "time", "datetime-local", "number", "range", "color", "checkbox", "radio", "file", "submit", "image", "reset", "button"];
        Ok(TYPES.iter().copied().find(|t| self.attr(node, "type").is_some_and(|v| v.eq_ignore_ascii_case(t))).unwrap_or("text"))
    }
    pub fn input_value_mode(&self, node: NodeIdx) -> Result<ValueMode, DomError> {
        Ok(match self.input_type(node)? {
            "hidden" | "submit" | "image" | "reset" | "button" => ValueMode::Default,
            "checkbox" | "radio" => ValueMode::DefaultOn,
            "file" => ValueMode::Filename,
            _ => ValueMode::Value,
        })
    }
    pub fn input_value(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        let kind = self.input_type(node)?;
        let attr = self.attribute_utf16(node, "value").unwrap_or_default();
        Ok(match self.input_value_mode(node)? {
            ValueMode::Default => attr,
            ValueMode::DefaultOn => self.attribute_utf16(node, "value").unwrap_or_else(|| units("on")),
            ValueMode::Filename => Vec::new(),
            ValueMode::Value => sanitize_input(kind, self.form_state(node).and_then(|s| s.value.as_deref()).unwrap_or(&attr), self.attr(node, "min"), self.attr(node, "max")),
        })
    }
    pub fn set_input_value(&mut self, node: NodeIdx, value: Vec<u16>) -> Result<(), DomError> {
        match self.input_value_mode(node)? {
            ValueMode::Default | ValueMode::DefaultOn => self.set_attribute(node, "value", &String::from_utf16_lossy(&value)),
            ValueMode::Filename if !value.is_empty() => Err(DomError::InvalidStateError),
            ValueMode::Filename => Ok(()),
            ValueMode::Value => {
                let kind = self.input_type(node)?;
                let sanitized = sanitize_input(kind, &value, self.attr(node, "min"), self.attr(node, "max"));
                let state = self.form_state_mut(node);
                state.value = Some(sanitized);
                state.dirty_value = true;
                state.selection = None;
                Ok(())
            }
        }
    }
    pub fn input_checked(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["input"])?;
        Ok(self.form_state(node).and_then(|s| s.checked).unwrap_or_else(|| self.has(node, "checked")))
    }
    pub fn set_input_checked(&mut self, node: NodeIdx, checked: bool) -> Result<(), DomError> {
        self.require_tag(node, &["input"])?;
        let state = self.form_state_mut(node);
        state.checked = Some(checked); state.dirty_checked = true;
        if checked && self.input_type(node)? == "radio" {
            let name = self.attr(node, "name").unwrap_or("").to_owned();
            let owner = self.form_owner(node);
            let document = self.owner_document(node).unwrap_or(node);
            let peers: Vec<_> = (0..self.node_count()).map(|i| NodeIdx(i as u32)).filter(|peer| *peer != node && self.html_tag(*peer, "input")
                && self.input_type(*peer).ok() == Some("radio")
                && self.attr(*peer, "name").unwrap_or("") == name
                && self.form_owner(*peer) == owner && self.owner_document(*peer) == Some(document)).collect();
            for peer in peers { self.form_state_mut(peer).checked = Some(false); }
        }
        Ok(())
    }
    /// Form owner is live: a `form` attribute may point outside the ancestor tree.
    pub fn form_owner(&self, node: NodeIdx) -> Option<NodeIdx> {
        if !["input", "button", "select", "textarea", "fieldset", "output", "object"].iter().any(|tag| self.html_tag(node, tag)) { return None; }
        if let Some(id) = self.attr(node, "form") {
            let document = self.owner_document(node)?;
            return self.tree_order(document).into_iter().find(|candidate| self.html_tag(*candidate, "form") && self.attr(*candidate, "id") == Some(id));
        }
        let mut parent = self.get(node).parent;
        while let Some(current) = parent {
            if self.html_tag(current, "form") { return Some(current); }
            parent = self.get(current).parent;
        }
        None
    }
    pub fn form_controls(&self, form: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.require_tag(form, &["form"])?;
        let document = self.owner_document(form).unwrap_or(NodeIdx(0));
        Ok(self.tree_order(document).into_iter().filter(|node| self.form_owner(*node) == Some(form)
            && ["input", "button", "select", "textarea", "fieldset", "output", "object"].iter().any(|tag| self.html_tag(*node, tag))).collect())
    }
    pub fn radio_list(&mut self, form: NodeIdx, name: Vec<u16>) -> Result<usize, DomError> {
        self.require_tag(form, &["form"])?;
        if let Some(index) = self.radio_lists.iter().position(|entry| entry.0 == form && entry.1 == name) { return Ok(index); }
        let index = self.radio_lists.len();
        self.radio_lists.push((form, name));
        Ok(index)
    }
    pub fn radio_list_items(&self, index: usize) -> Result<Vec<NodeIdx>, DomError> {
        let (form, name) = self.radio_lists.get(index).ok_or(DomError::InvalidStateError)?;
        Ok(self.form_controls(*form)?.into_iter().filter(|node| {
            self.attribute_utf16(*node, "id").as_deref() == Some(name) ||
            self.attribute_utf16(*node, "name").as_deref() == Some(name)
        }).collect())
    }
    pub fn textarea_value(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["textarea"])?;
        if let Some(value) = self.form_state(node).and_then(|s| s.value.as_ref()) { return Ok(normalize_newlines(value)); }
        let mut raw: Vec<u16> = self.tree_order(node).into_iter().skip(1).filter_map(|child| self.character_data_utf16(child)).flatten().collect();
        if raw.first() == Some(&10) { raw.remove(0); }
        Ok(normalize_newlines(&raw))
    }
    pub fn set_textarea_value(&mut self, node: NodeIdx, value: Vec<u16>) -> Result<(), DomError> {
        self.require_tag(node, &["textarea"])?;
        let state = self.form_state_mut(node);
        state.value = Some(normalize_newlines(&value)); state.dirty_value = true;
        Ok(())
    }
    pub fn option_value(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["option"])?;
        if let Some(value) = self.attribute_utf16(node, "value") { return Ok(value); }
        let text: Vec<u16> = self.tree_order(node).into_iter().skip(1).filter_map(|child| self.character_data_utf16(child)).flatten().collect();
        let compact = String::from_utf16_lossy(&text).split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        Ok(units(&compact))
    }
    pub fn option_selected(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["option"])?;
        Ok(self.form_state(node).and_then(|s| s.selected).unwrap_or_else(|| self.has(node, "selected")))
    }
    pub fn select_options(&self, select: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.require_tag(select, &["select"])?;
        Ok(self.tree_order(select).into_iter().skip(1).filter(|n| self.html_tag(*n, "option")).collect())
    }
    pub fn selected_index(&self, select: NodeIdx) -> Result<i32, DomError> {
        let options = self.select_options(select)?;
        if let Some(index) = options.iter().position(|n| self.option_selected(*n).unwrap_or(false)) { return Ok(index as i32); }
        if self.has(select, "multiple") || self.has(select, "size") { Ok(-1) }
        else { Ok(options.iter().position(|n| !self.has(*n, "disabled")).map_or(-1, |n| n as i32)) }
    }
    pub fn set_option_selected(&mut self, option: NodeIdx, selected: bool) -> Result<(), DomError> {
        self.require_tag(option, &["option"])?;
        let state = self.form_state_mut(option);
        state.selected = Some(selected); state.dirty_selected = true;
        if selected {
            let mut parent = self.get(option).parent;
            while let Some(node) = parent {
                if self.html_tag(node, "select") {
                    if !self.has(node, "multiple") {
                        for peer in self.select_options(node)?.into_iter().filter(|n| *n != option) {
                            self.form_state_mut(peer).selected = Some(false);
                        }
                    }
                    break;
                }
                parent = self.get(node).parent;
            }
        }
        Ok(())
    }
    pub fn set_selected_index(&mut self, select: NodeIdx, index: i32) -> Result<(), DomError> {
        let options = self.select_options(select)?;
        for (i, option) in options.into_iter().enumerate() { self.set_option_selected(option, i as i32 == index)?; }
        Ok(())
    }
    pub fn select_value(&self, select: NodeIdx) -> Result<Vec<u16>, DomError> {
        let index = self.selected_index(select)?;
        if index < 0 { return Ok(Vec::new()); }
        self.option_value(self.select_options(select)?[index as usize])
    }
    pub fn set_select_value(&mut self, select: NodeIdx, value: &[u16]) -> Result<(), DomError> {
        let options = self.select_options(select)?;
        let index = options.iter().position(|n| self.option_value(*n).ok().as_deref() == Some(value));
        self.set_selected_index(select, index.map_or(-1, |i| i as i32))
    }
    pub fn selection(&self, node: NodeIdx) -> Result<(u32, u32, SelectionDirection), DomError> {
        self.require_tag(node, &["input", "textarea"])?;
        let len = if self.html_tag(node, "input") { self.input_value(node)?.len() } else { self.textarea_value(node)?.len() } as u32;
        Ok(self.form_state(node).and_then(|s| s.selection).unwrap_or((len, len, SelectionDirection::None)))
    }
    pub fn set_selection(&mut self, node: NodeIdx, start: u32, end: u32, direction: SelectionDirection) -> Result<(), DomError> {
        self.require_tag(node, &["input", "textarea"])?;
        let len = if self.html_tag(node, "input") { self.input_value(node)?.len() } else { self.textarea_value(node)?.len() } as u32;
        self.form_state_mut(node).selection = Some((start.min(end).min(len), end.min(len), direction));
        Ok(())
    }
    pub fn set_custom_validity(&mut self, node: NodeIdx, message: Vec<u16>) -> Result<(), DomError> {
        self.require_tag(node, &["input", "select", "textarea", "button", "fieldset", "output"])?;
        self.form_state_mut(node).custom_error = message;
        Ok(())
    }
    pub fn validation_message(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["input", "select", "textarea", "button", "fieldset", "output"])?;
        Ok(self.form_state(node).map(|s| s.custom_error.clone()).unwrap_or_default())
    }
    pub fn will_validate(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["input", "select", "textarea", "button", "fieldset", "output"])?;
        if self.has(node, "disabled") || self.has(node, "readonly") || self.html_tag(node, "fieldset") || self.html_tag(node, "output") { return Ok(false); }
        if self.html_tag(node, "input") && matches!(self.input_type(node)?, "hidden" | "button" | "reset" | "submit" | "image") { return Ok(false); }
        if self.html_tag(node, "button") && self.attr(node, "type").is_some_and(|v| v.eq_ignore_ascii_case("button") || v.eq_ignore_ascii_case("reset")) { return Ok(false); }
        Ok(true)
    }
    pub fn validity(&self, node: NodeIdx, pattern_matches: impl FnOnce(&str, &str) -> Option<bool>) -> Result<ValidityState, DomError> {
        if !self.will_validate(node)? { return Ok(ValidityState::default()); }
        let mut v = ValidityState::default();
        let kind = if self.html_tag(node, "input") { self.input_type(node)? } else { "" };
        let value = if self.html_tag(node, "input") { self.input_value(node)? }
            else if self.html_tag(node, "textarea") { self.textarea_value(node)? }
            else if self.html_tag(node, "select") { self.select_value(node)? } else { Vec::new() };
        let s = String::from_utf16_lossy(&value);
        v.custom_error = self.form_state(node).is_some_and(|state| !state.custom_error.is_empty());
        v.value_missing = self.has(node, "required") && (s.is_empty() || (kind == "checkbox" && !self.input_checked(node)?)
            || (kind == "radio" && !self.input_checked(node)?));
        if !s.is_empty() {
            v.type_mismatch = match kind {
                "email" => !s.split(',').all(|item| { let item = item.trim(); let mut parts = item.split('@');
                    parts.next().is_some_and(|v| !v.is_empty()) && parts.next().is_some_and(|v| v.contains('.') && !v.contains(' ')) && parts.next().is_none() }),
                "url" => !(s.contains(':') && !s.contains(' ')),
                _ => false,
            };
            if let Some(pattern) = self.attr(node, "pattern") { v.pattern_mismatch = pattern_matches(pattern, &s) == Some(false); }
            let length = value.len();
            if self.form_state(node).is_some_and(|state| state.dirty_value) {
                v.too_long = self.attr(node, "maxlength").and_then(|v| v.parse::<usize>().ok()).is_some_and(|max| length > max);
                v.too_short = self.attr(node, "minlength").and_then(|v| v.parse::<usize>().ok()).is_some_and(|min| length < min);
            }
            if let Some(n) = valid_number(&s) {
                v.range_underflow = self.attr(node, "min").and_then(valid_number).is_some_and(|min| n < min);
                v.range_overflow = self.attr(node, "max").and_then(valid_number).is_some_and(|max| n > max);
                if let Some(step) = self.attr(node, "step").filter(|v| *v != "any").and_then(valid_number).filter(|v| *v > 0.) {
                    let base = self.attr(node, "min").and_then(valid_number).unwrap_or(0.);
                    v.step_mismatch = ((n - base) / step - ((n - base) / step).round()).abs() > 1e-8;
                }
            }
        }
        Ok(v)
    }
    pub fn reset_form(&mut self, form: NodeIdx) -> Result<(), DomError> {
        let mut controls = self.form_controls(form)?;
        let options: Vec<_> = controls.iter().copied().filter(|node| self.html_tag(*node, "select"))
            .flat_map(|select| self.select_options(select).unwrap_or_default()).collect();
        controls.extend(options);
        for node in controls {
            let state = self.form_state_mut(node);
            state.value = None; state.dirty_value = false;
            state.checked = None; state.dirty_checked = false;
            state.selected = None; state.dirty_selected = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_value_modes_dirty_flags_and_sanitization() {
        let mut dom = Dom::new();
        let input = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(input, "value", "a\rb\nc").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("abc"));
        dom.set_input_value(input, units("script")).unwrap();
        dom.set_attribute(input, "value", "default").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("script"));
        dom.set_attribute(input, "type", "checkbox").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("default"));
        dom.remove_attribute(input, "value").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("on"));
        dom.set_attribute(input, "type", "file").unwrap();
        assert_eq!(dom.set_input_value(input, units("secret")), Err(DomError::InvalidStateError));
        dom.set_attribute(input, "type", "color").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("#000000"));
        dom.set_input_value(input, units("#A1B2C3")).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("#a1b2c3"));
        dom.set_attribute(input, "type", "number").unwrap();
        dom.set_input_value(input, units("xyz")).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), Vec::<u16>::new());
    }
    #[test]
    fn radio_groups_follow_form_owner_and_reparenting() {
        let mut dom = Dom::new();
        let first_form = dom.create_element(NodeIdx(0), "form");
        dom.set_attribute(first_form, "id", "f").unwrap();
        let a = dom.create_element(first_form, "input");
        let b = dom.create_element(NodeIdx(0), "input");
        for node in [a, b] { dom.set_attribute(node, "type", "radio").unwrap(); dom.set_attribute(node, "name", "choice").unwrap(); }
        dom.set_attribute(b, "form", "f").unwrap();
        dom.set_input_checked(a, true).unwrap();
        dom.set_input_checked(b, true).unwrap();
        assert!(!dom.input_checked(a).unwrap());
        assert!(dom.input_checked(b).unwrap());
        dom.remove_attribute(b, "form").unwrap();
        dom.set_input_checked(a, true).unwrap();
        assert!(dom.input_checked(b).unwrap());
        assert_eq!(dom.form_controls(first_form).unwrap(), vec![a]);
    }
    #[test]
    fn option_selection_and_form_reset() {
        let mut dom = Dom::new();
        let form = dom.create_element(NodeIdx(0), "form");
        let select = dom.create_element(form, "select");
        let a = dom.create_element(select, "option");
        dom.create_text(a, " alpha  beta ");
        let b = dom.create_element(select, "option");
        dom.set_attribute(b, "value", "B").unwrap();
        assert_eq!(dom.selected_index(select).unwrap(), 0);
        assert_eq!(dom.select_value(select).unwrap(), units("alpha beta"));
        dom.set_select_value(select, &units("B")).unwrap();
        assert_eq!(dom.selected_index(select).unwrap(), 1);
        assert!(!dom.option_selected(a).unwrap());
        assert!(dom.option_selected(b).unwrap());
        dom.reset_form(form).unwrap();
        assert_eq!(dom.selected_index(select).unwrap(), 0);
    }
    #[test]
    fn validation_flags_and_custom_error() {
        let mut dom = Dom::new();
        let input = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(input, "required", "").unwrap();
        assert!(dom.validity(input, |_, _| None).unwrap().value_missing);
        dom.set_attribute(input, "type", "number").unwrap();
        dom.set_attribute(input, "min", "2").unwrap();
        dom.set_attribute(input, "max", "8").unwrap();
        dom.set_attribute(input, "step", "2").unwrap();
        dom.set_input_value(input, units("3")).unwrap();
        let validity = dom.validity(input, |_, _| None).unwrap();
        assert!(validity.step_mismatch);
        assert!(!validity.range_underflow);
        dom.set_custom_validity(input, units("no")).unwrap();
        assert!(dom.validity(input, |_, _| None).unwrap().custom_error);
        dom.set_attribute(input, "disabled", "").unwrap();
        assert!(!dom.will_validate(input).unwrap());
        assert!(dom.validity(input, |_, _| None).unwrap().valid());
    }
}
