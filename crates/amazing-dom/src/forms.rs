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
pub enum SelectionDirection {
    #[default]
    None,
    Forward,
    Backward,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueMode {
    Value,
    Default,
    DefaultOn,
    Filename,
}
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
    pub fn valid(&self) -> bool {
        self == &Self::default()
    }
}
fn units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
fn ascii_lower(value: &str) -> String {
    value.to_ascii_lowercase()
}
fn strip_newlines(value: &[u16]) -> Vec<u16> {
    value
        .iter()
        .copied()
        .filter(|u| *u != 10 && *u != 13)
        .collect()
}
fn trim_ascii_ws(value: &[u16]) -> &[u16] {
    let start = value
        .iter()
        .position(|u| !matches!(*u, 9 | 10 | 12 | 13 | 32))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|u| !matches!(*u, 9 | 10 | 12 | 13 | 32))
        .map_or(start, |i| i + 1);
    &value[start..end]
}
fn normalize_newlines(value: &[u16]) -> Vec<u16> {
    let mut result = Vec::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        if value[i] == 13 {
            result.push(10);
            if value.get(i + 1) == Some(&10) {
                i += 1;
            }
        } else {
            result.push(value[i]);
        }
        i += 1;
    }
    result
}
fn valid_number(value: &str) -> Option<f64> {
    if value.is_empty() || value.trim() != value || value.starts_with('+') {
        return None;
    }
    let n: f64 = value.parse().ok()?;
    n.is_finite().then_some(n)
}
fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let shifted = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
fn date_parts(value: &str) -> Option<(i64, i64, i64)> {
    let mut parts = value.split('-');
    let y = parts.next()?;
    let m = parts.next()?;
    let d = parts.next()?;
    if parts.next().is_some()
        || y.len() < 4
        || m.len() != 2
        || d.len() != 2
        || ![y, m, d]
            .iter()
            .all(|s| s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let year: i64 = y.parse().ok()?;
    let month: i64 = m.parse().ok()?;
    let day: i64 = d.parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    (civil_from_days(days) == (year, month, day)).then_some((year, month, day))
}
fn time_millis(value: &str) -> Option<i64> {
    let (main, fraction) = value
        .split_once('.')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    let parts: Vec<_> = main.split(':').collect();
    if !(parts.len() == 2 || parts.len() == 3)
        || parts
            .iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let hour: i64 = parts[0].parse().ok()?;
    let minute: i64 = parts[1].parse().ok()?;
    let second: i64 = parts.get(2).map_or(Some(0), |v| v.parse().ok())?;
    if hour >= 24 || minute >= 60 || second >= 60 || (fraction.is_some() && parts.len() != 3) {
        return None;
    }
    let milli = if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 3
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        format!("{fraction:0<3}").parse::<i64>().ok()?
    } else {
        0
    };
    Some(((hour * 60 + minute) * 60 + second) * 1000 + milli)
}
fn format_date(year: i64, month: i64, day: i64) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}
fn format_time(ms: i64) -> String {
    let hour = ms / 3_600_000;
    let minute = ms / 60_000 % 60;
    let second = ms / 1000 % 60;
    let milli = ms % 1000;
    if milli != 0 {
        format!("{hour:02}:{minute:02}:{second:02}.{milli:03}")
    } else if second != 0 {
        format!("{hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{hour:02}:{minute:02}")
    }
}
fn temporal_number(kind: &str, value: &str) -> Option<f64> {
    match kind {
        "date" => {
            let (y, m, d) = date_parts(value)?;
            Some((days_from_civil(y, m, d) * 86_400_000) as f64)
        }
        "month" => {
            let (y, m, _) = date_parts(&format!("{value}-01"))?;
            Some(((y - 1970) * 12 + m - 1) as f64)
        }
        "time" => Some(time_millis(value)? as f64),
        "datetime-local" => {
            let (date, time) = value.split_once(['T', ' '])?;
            let (y, m, d) = date_parts(date)?;
            Some((days_from_civil(y, m, d) * 86_400_000 + time_millis(time)?) as f64)
        }
        "week" => {
            let (year, week) = value.split_once("-W")?;
            let year: i64 = year.parse().ok()?;
            let week: i64 = week.parse().ok()?;
            if year < 1 || !(1..=53).contains(&week) {
                return None;
            }
            let jan4 = days_from_civil(year, 1, 4);
            let monday = jan4 - (jan4 + 3).rem_euclid(7);
            Some(((monday + (week - 1) * 7) * 86_400_000) as f64)
        }
        _ => valid_number(value),
    }
}
fn temporal_string(kind: &str, number: f64) -> Option<String> {
    if !number.is_finite() || number.abs() > 8.64e15 {
        return None;
    }
    let n = number.trunc() as i64;
    match kind {
        "date" => {
            let (y, m, d) = civil_from_days(n.div_euclid(86_400_000));
            (y >= 1).then(|| format_date(y, m, d))
        }
        "month" => {
            let months = n + 1970 * 12;
            let y = months.div_euclid(12);
            let m = months.rem_euclid(12) + 1;
            (y >= 1).then(|| format!("{y:04}-{m:02}"))
        }
        "time" => Some(format_time(n.rem_euclid(86_400_000))),
        "datetime-local" => {
            let (y, m, d) = civil_from_days(n.div_euclid(86_400_000));
            (y >= 1).then(|| {
                format!(
                    "{}T{}",
                    format_date(y, m, d),
                    format_time(n.rem_euclid(86_400_000))
                )
            })
        }
        "week" => {
            let days = n.div_euclid(86_400_000);
            let thursday = days + 3 - (days + 3).rem_euclid(7);
            let (year, _, _) = civil_from_days(thursday);
            let jan4 = days_from_civil(year, 1, 4);
            let monday = jan4 - (jan4 + 3).rem_euclid(7);
            Some(format!(
                "{year:04}-W{:02}",
                (days - monday).div_euclid(7) + 1
            ))
        }
        _ => Some(number.to_string()),
    }
}
fn date_shape(value: &str, kind: &str) -> bool {
    match kind {
        "date" => date_parts(value).is_some(),
        "month" | "week" | "time" | "datetime-local" => temporal_number(kind, value).is_some(),
        _ => false,
    }
}
pub fn sanitize_input(
    kind: &str,
    value: &[u16],
    min: Option<&str>,
    max: Option<&str>,
    step: Option<&str>,
) -> Vec<u16> {
    let cleaned = match kind {
        "text" | "search" | "tel" | "url" | "email" | "password" => strip_newlines(value),
        _ => value.to_vec(),
    };
    let s = String::from_utf16_lossy(&cleaned);
    match kind {
        "email" => {
            let trimmed = trim_ascii_ws(&cleaned);
            let mut result = Vec::new();
            for (index, part) in trimmed.split(|u| *u == 44).enumerate() {
                if index != 0 {
                    result.push(44);
                }
                result.extend_from_slice(trim_ascii_ws(part));
            }
            result
        }
        "url" => trim_ascii_ws(&cleaned).to_vec(),
        "number" if valid_number(&s).is_none() => Vec::new(),
        "range" => {
            let lo = min.and_then(valid_number).unwrap_or(0.);
            let hi = max.and_then(valid_number).unwrap_or(100.).max(lo);
            let default = (lo + hi) / 2.;
            let mut number = valid_number(&s).unwrap_or(default).clamp(lo, hi);
            let step_value = if step == Some("any") {
                None
            } else {
                Some(
                    step.and_then(valid_number)
                        .filter(|s| *s > 0.)
                        .unwrap_or(1.),
                )
            };
            if let Some(step) = step_value {
                let lower = lo + ((number - lo) / step).floor() * step;
                let upper = lower + step;
                number = if upper <= hi && upper - number <= number - lower {
                    upper
                } else {
                    lower
                };
            }
            units(&number.to_string())
        }
        "color" => {
            if s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit()) {
                units(&ascii_lower(&s))
            } else {
                units("#000000")
            }
        }
        "date" | "month" | "week" | "time" | "datetime-local" if !date_shape(&s, kind) => {
            Vec::new()
        }
        "time" => units(&format_time(time_millis(&s).unwrap_or(0))),
        "datetime-local" => {
            let (date, time) = s.split_once(['T', ' ']).unwrap();
            units(&format!(
                "{date}T{}",
                format_time(time_millis(time).unwrap_or(0))
            ))
        }
        _ => cleaned,
    }
}
impl Dom {
    pub(crate) fn form_tree_inserted(&mut self, root: NodeIdx) {
        let descendants = self.tree_order(root);
        for node in descendants {
            if self.html_tag(node, "input")
                && self.input_type(node).ok() == Some("radio")
                && self.input_checked(node).unwrap_or(false)
            {
                let dirty = self
                    .form_state(node)
                    .is_some_and(|state| state.dirty_checked);
                let _ = self.set_input_checked(node, true);
                self.form_state_mut(node).dirty_checked = dirty;
            }
            if self.html_tag(node, "option") && self.option_selected(node).unwrap_or(false) {
                let dirty = self
                    .form_state(node)
                    .is_some_and(|state| state.dirty_selected);
                let _ = self.set_option_selected(node, true);
                self.form_state_mut(node).dirty_selected = dirty;
            }
        }
    }
    pub(crate) fn clone_form_state(&mut self, from: NodeIdx, to: NodeIdx) {
        if let Some(state) = self.form_state(from).cloned() {
            *self.form_state_mut(to) = state;
        }
    }
    fn form_state(&self, node: NodeIdx) -> Option<&ControlState> {
        self.form_states
            .get(node.0 as usize)
            .and_then(Option::as_ref)
    }
    fn form_state_mut(&mut self, node: NodeIdx) -> &mut ControlState {
        let index = node.0 as usize;
        if self.form_states.len() <= index {
            self.form_states.resize_with(index + 1, || None);
        }
        self.form_states[index].get_or_insert_with(ControlState::default)
    }
    pub fn html_tag(&self, node: NodeIdx, tag: &str) -> bool {
        matches!(&self.get(node).kind, NodeKind::Element { name, namespace: Namespace::Html, .. } if name.eq_ignore_ascii_case(tag))
    }
    fn require_tag(&self, node: NodeIdx, tags: &[&str]) -> Result<(), DomError> {
        self.check_node(node)?;
        if tags.iter().any(|tag| self.html_tag(node, tag)) {
            Ok(())
        } else {
            Err(DomError::TypeError)
        }
    }
    fn attr(&self, node: NodeIdx, name: &str) -> Option<&str> {
        self.get_attribute_ns(node, None, name)
    }
    fn has(&self, node: NodeIdx, name: &str) -> bool {
        self.attr(node, name).is_some()
    }
    pub fn input_type(&self, node: NodeIdx) -> Result<&'static str, DomError> {
        self.require_tag(node, &["input"])?;
        const TYPES: &[&str] = &[
            "hidden",
            "text",
            "search",
            "tel",
            "url",
            "email",
            "password",
            "date",
            "month",
            "week",
            "time",
            "datetime-local",
            "number",
            "range",
            "color",
            "checkbox",
            "radio",
            "file",
            "submit",
            "image",
            "reset",
            "button",
        ];
        Ok(TYPES
            .iter()
            .copied()
            .find(|t| {
                self.attr(node, "type")
                    .is_some_and(|v| v.eq_ignore_ascii_case(t))
            })
            .unwrap_or("text"))
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
            ValueMode::DefaultOn => self
                .attribute_utf16(node, "value")
                .unwrap_or_else(|| units("on")),
            ValueMode::Filename => Vec::new(),
            ValueMode::Value => sanitize_input(
                kind,
                self.form_state(node)
                    .and_then(|s| s.value.as_deref())
                    .unwrap_or(&attr),
                self.attr(node, "min"),
                self.attr(node, "max"),
                self.attr(node, "step"),
            ),
        })
    }
    pub fn set_input_value(&mut self, node: NodeIdx, value: Vec<u16>) -> Result<(), DomError> {
        match self.input_value_mode(node)? {
            ValueMode::Default | ValueMode::DefaultOn => {
                if !self.has(node, "value") {
                    self.set_attribute(node, "value", "")?;
                }
                let attr = self
                    .get_attribute_node(node, None, "value")
                    .ok_or(DomError::InvalidStateError)?;
                self.set_attr_value(attr, value)
            }
            ValueMode::Filename if !value.is_empty() => Err(DomError::InvalidStateError),
            ValueMode::Filename => Ok(()),
            ValueMode::Value => {
                let kind = self.input_type(node)?;
                let sanitized = sanitize_input(
                    kind,
                    &value,
                    self.attr(node, "min"),
                    self.attr(node, "max"),
                    self.attr(node, "step"),
                );
                let state = self.form_state_mut(node);
                state.value = Some(sanitized);
                state.dirty_value = true;
                state.selection = None;
                self.record_render_full(node, "input value state");
                Ok(())
            }
        }
    }
    pub fn input_value_as_number(&self, node: NodeIdx) -> Result<f64, DomError> {
        let kind = self.input_type(node)?;
        if !matches!(
            kind,
            "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
        ) {
            return Ok(f64::NAN);
        }
        let value = String::from_utf16_lossy(&self.input_value(node)?);
        Ok(temporal_number(kind, &value).unwrap_or(f64::NAN))
    }
    pub fn set_input_value_as_number(
        &mut self,
        node: NodeIdx,
        number: f64,
    ) -> Result<(), DomError> {
        let kind = self.input_type(node)?;
        if !matches!(
            kind,
            "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
        ) {
            return Err(DomError::InvalidStateError);
        }
        let value = temporal_string(kind, number).ok_or(DomError::TypeError)?;
        self.set_input_value(node, units(&value))
    }
    pub fn input_value_as_date_ms(&self, node: NodeIdx) -> Result<Option<f64>, DomError> {
        let kind = self.input_type(node)?;
        if !matches!(kind, "date" | "month" | "week" | "time") {
            return Ok(None);
        }
        let number = self.input_value_as_number(node)?;
        if !number.is_finite() {
            return Ok(None);
        }
        if kind == "month" {
            let value = String::from_utf16_lossy(&self.input_value(node)?);
            let (y, m, _) =
                date_parts(&format!("{value}-01")).ok_or(DomError::InvalidStateError)?;
            return Ok(Some((days_from_civil(y, m, 1) * 86_400_000) as f64));
        }
        Ok(Some(number))
    }
    pub fn set_input_value_as_date_ms(
        &mut self,
        node: NodeIdx,
        date: Option<f64>,
    ) -> Result<(), DomError> {
        let kind = self.input_type(node)?;
        if !matches!(kind, "date" | "month" | "week" | "time") {
            return Err(DomError::InvalidStateError);
        }
        let Some(date) = date else {
            return self.set_input_value(node, Vec::new());
        };
        if !date.is_finite() {
            return self.set_input_value(node, Vec::new());
        }
        let number = if kind == "month" {
            let (y, m, _) = civil_from_days((date.trunc() as i64).div_euclid(86_400_000));
            ((y - 1970) * 12 + m - 1) as f64
        } else {
            date
        };
        self.set_input_value_as_number(node, number)
    }
    pub fn input_checked(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["input"])?;
        Ok(self
            .form_state(node)
            .and_then(|s| s.checked)
            .unwrap_or_else(|| self.has(node, "checked")))
    }
    pub(crate) fn form_attribute_changed(&mut self, node: NodeIdx, name: &str) {
        if self.html_tag(node, "option") && name == "selected" {
            let dirty = self.form_state(node).is_some_and(|s| s.dirty_selected);
            if !dirty {
                if let Some(state) = self
                    .form_states
                    .get_mut(node.0 as usize)
                    .and_then(Option::as_mut)
                {
                    state.selected = None;
                }
                if self.has(node, "selected") {
                    let _ = self.set_option_selected(node, true);
                    self.form_state_mut(node).dirty_selected = false;
                }
            }
            return;
        }
        if !self.html_tag(node, "input") {
            return;
        }
        if name == "checked" {
            if let Some(state) = self
                .form_states
                .get_mut(node.0 as usize)
                .and_then(Option::as_mut)
            {
                if !state.dirty_checked {
                    state.checked = None;
                }
            }
        }
        if matches!(name, "checked" | "name" | "form" | "type")
            && self.input_type(node).ok() == Some("radio")
            && self.input_checked(node).unwrap_or(false)
        {
            let dirty = self.form_state(node).is_some_and(|s| s.dirty_checked);
            let _ = self.set_input_checked(node, true);
            self.form_state_mut(node).dirty_checked = dirty;
        }
    }
    pub fn set_input_checked(&mut self, node: NodeIdx, checked: bool) -> Result<(), DomError> {
        self.require_tag(node, &["input"])?;
        let old_checked = self.input_checked(node)?;
        let state = self.form_state_mut(node);
        state.checked = Some(checked);
        state.dirty_checked = true;
        self.record_render_state(node, "checked", old_checked, checked);
        if checked && self.input_type(node)? == "radio" {
            let name = self.attr(node, "name").unwrap_or("").to_owned();
            let owner = self.form_owner(node);
            let document = self.owner_document(node).unwrap_or(node);
            let root = self.root(node);
            let peers: Vec<_> = (0..self.node_count())
                .map(|i| NodeIdx(i as u32))
                .filter(|peer| {
                    *peer != node
                        && self.html_tag(*peer, "input")
                        && self.input_type(*peer).ok() == Some("radio")
                        && self.attr(*peer, "name").unwrap_or("") == name
                        && self.form_owner(*peer) == owner
                        && self.owner_document(*peer) == Some(document)
                        && self.root(*peer) == root
                })
                .collect();
            for peer in peers {
                let old_peer = self.input_checked(peer)?;
                self.form_state_mut(peer).checked = Some(false);
                self.record_render_state(peer, "checked", old_peer, false);
            }
        }
        Ok(())
    }
    /// Form owner is live: a `form` attribute may point outside the ancestor tree.
    pub fn form_owner(&self, node: NodeIdx) -> Option<NodeIdx> {
        if ![
            "input", "button", "select", "textarea", "fieldset", "output", "object",
        ]
        .iter()
        .any(|tag| self.html_tag(node, tag))
        {
            return None;
        }
        if let Some(id) = self.attr(node, "form") {
            if self.is_connected(node) {
                let document = self.owner_document(node)?;
                let first = self
                    .tree_order(document)
                    .into_iter()
                    .find(|candidate| self.attr(*candidate, "id") == Some(id));
                if first.is_some_and(|candidate| self.html_tag(candidate, "form")) {
                    return first;
                }
            }
        }
        let mut parent = self.get(node).parent;
        while let Some(current) = parent {
            if self.html_tag(current, "form") {
                return Some(current);
            }
            parent = self.get(current).parent;
        }
        None
    }
    pub fn form_controls(&self, form: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.require_tag(form, &["form"])?;
        let root = if self.is_connected(form) {
            self.owner_document(form).unwrap_or(NodeIdx(0))
        } else {
            self.root(form)
        };
        Ok(self
            .tree_order(root)
            .into_iter()
            .filter(|node| {
                self.form_owner(*node) == Some(form)
                    && [
                        "input", "button", "select", "textarea", "fieldset", "output", "object",
                    ]
                    .iter()
                    .any(|tag| self.html_tag(*node, tag))
            })
            .collect())
    }
    pub fn radio_list(&mut self, form: NodeIdx, name: Vec<u16>) -> Result<usize, DomError> {
        self.require_tag(form, &["form"])?;
        if let Some(index) = self
            .radio_lists
            .iter()
            .position(|entry| entry.0 == form && entry.1 == name)
        {
            return Ok(index);
        }
        let index = self.radio_lists.len();
        self.radio_lists.push((form, name));
        Ok(index)
    }
    pub fn radio_list_items(&self, index: usize) -> Result<Vec<NodeIdx>, DomError> {
        let (form, name) = self
            .radio_lists
            .get(index)
            .ok_or(DomError::InvalidStateError)?;
        Ok(self
            .form_controls(*form)?
            .into_iter()
            .filter(|node| {
                self.attribute_utf16(*node, "id").as_deref() == Some(name)
                    || self.attribute_utf16(*node, "name").as_deref() == Some(name)
            })
            .collect())
    }
    pub fn textarea_value(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["textarea"])?;
        if let Some(value) = self.form_state(node).and_then(|s| s.value.as_ref()) {
            return Ok(normalize_newlines(value));
        }
        let raw: Vec<u16> = self
            .tree_order(node)
            .into_iter()
            .skip(1)
            .filter_map(|child| self.character_data_utf16(child))
            .flatten()
            .collect();
        Ok(normalize_newlines(&raw))
    }
    pub fn set_textarea_value(&mut self, node: NodeIdx, value: Vec<u16>) -> Result<(), DomError> {
        self.require_tag(node, &["textarea"])?;
        let state = self.form_state_mut(node);
        state.value = Some(value);
        state.dirty_value = true;
        self.record_render_full(node, "textarea value state");
        Ok(())
    }
    pub fn option_text(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["option"])?;
        let text: Vec<u16> = self
            .tree_order(node)
            .into_iter()
            .skip(1)
            .filter_map(|child| self.character_data_utf16(child))
            .flatten()
            .collect();
        let compact = String::from_utf16_lossy(&text)
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Ok(units(&compact))
    }
    pub fn option_value(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(node, &["option"])?;
        if let Some(value) = self.attribute_utf16(node, "value") {
            return Ok(value);
        }
        self.option_text(node)
    }
    fn raw_option_selected(&self, node: NodeIdx) -> bool {
        self.form_state(node)
            .and_then(|s| s.selected)
            .unwrap_or_else(|| self.has(node, "selected"))
    }
    pub fn option_selected(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["option"])?;
        if self.raw_option_selected(node) {
            return Ok(true);
        }
        let mut parent = self.get(node).parent;
        while let Some(candidate) = parent {
            if self.html_tag(candidate, "select") {
                return Ok(self.selected_index(candidate)? >= 0
                    && self
                        .select_options(candidate)?
                        .get(self.selected_index(candidate)? as usize)
                        == Some(&node));
            }
            parent = self.get(candidate).parent;
        }
        Ok(false)
    }
    pub fn select_options(&self, select: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.require_tag(select, &["select"])?;
        Ok(self
            .tree_order(select)
            .into_iter()
            .skip(1)
            .filter(|n| self.html_tag(*n, "option"))
            .collect())
    }
    pub fn selected_index(&self, select: NodeIdx) -> Result<i32, DomError> {
        let options = self.select_options(select)?;
        let selected = if self.has(select, "multiple") {
            options.iter().position(|n| self.raw_option_selected(*n))
        } else {
            options.iter().rposition(|n| self.raw_option_selected(*n))
        };
        if let Some(index) = selected {
            return Ok(index as i32);
        }
        if self.has(select, "multiple") || self.has(select, "size") {
            Ok(-1)
        } else {
            Ok(options
                .iter()
                .position(|n| !self.has(*n, "disabled"))
                .map_or(-1, |n| n as i32))
        }
    }
    pub fn set_option_selected(&mut self, option: NodeIdx, selected: bool) -> Result<(), DomError> {
        self.require_tag(option, &["option"])?;
        let old_selected = self.option_selected(option)?;
        let state = self.form_state_mut(option);
        state.selected = Some(selected);
        state.dirty_selected = true;
        self.record_render_state(option, "selected", old_selected, selected);
        if selected {
            let mut parent = self.get(option).parent;
            while let Some(node) = parent {
                if self.html_tag(node, "select") {
                    if !self.has(node, "multiple") {
                        for peer in self
                            .select_options(node)?
                            .into_iter()
                            .filter(|n| *n != option)
                        {
                            let old_peer = self.option_selected(peer)?;
                            self.form_state_mut(peer).selected = Some(false);
                            self.record_render_state(peer, "selected", old_peer, false);
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
        for (i, option) in options.into_iter().enumerate() {
            self.set_option_selected(option, i as i32 == index)?;
        }
        Ok(())
    }
    pub fn select_value(&self, select: NodeIdx) -> Result<Vec<u16>, DomError> {
        let index = self.selected_index(select)?;
        if index < 0 {
            return Ok(Vec::new());
        }
        self.option_value(self.select_options(select)?[index as usize])
    }
    pub fn set_select_value(&mut self, select: NodeIdx, value: &[u16]) -> Result<(), DomError> {
        let options = self.select_options(select)?;
        let index = options
            .iter()
            .position(|n| self.option_value(*n).ok().as_deref() == Some(value));
        self.set_selected_index(select, index.map_or(-1, |i| i as i32))
    }
    pub fn selection(&self, node: NodeIdx) -> Result<(u32, u32, SelectionDirection), DomError> {
        self.require_tag(node, &["input", "textarea"])?;
        if !self.selection_applicable(node)? {
            return Err(DomError::InvalidStateError);
        }
        let len = if self.html_tag(node, "input") {
            self.input_value(node)?.len()
        } else {
            self.textarea_value(node)?.len()
        } as u32;
        Ok(self.form_state(node).and_then(|s| s.selection).unwrap_or((
            len,
            len,
            SelectionDirection::None,
        )))
    }
    pub fn set_selection(
        &mut self,
        node: NodeIdx,
        start: u32,
        end: u32,
        direction: SelectionDirection,
    ) -> Result<(), DomError> {
        self.require_tag(node, &["input", "textarea"])?;
        if !self.selection_applicable(node)? {
            return Err(DomError::InvalidStateError);
        }
        let len = if self.html_tag(node, "input") {
            self.input_value(node)?.len()
        } else {
            self.textarea_value(node)?.len()
        } as u32;
        self.form_state_mut(node).selection =
            Some((start.min(end).min(len), end.min(len), direction));
        self.record_render_full(node, "selection state");
        Ok(())
    }
    pub fn selection_applicable(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(node, &["input", "textarea"])?;
        Ok(self.html_tag(node, "textarea")
            || matches!(
                self.input_type(node)?,
                "text" | "search" | "tel" | "url" | "password"
            ))
    }
    pub fn set_custom_validity(
        &mut self,
        node: NodeIdx,
        message: Vec<u16>,
    ) -> Result<(), DomError> {
        self.require_tag(
            node,
            &[
                "input", "select", "textarea", "button", "fieldset", "output",
            ],
        )?;
        self.form_state_mut(node).custom_error = message;
        self.record_render_full(node, "form validity state");
        Ok(())
    }
    pub fn validation_message(&self, node: NodeIdx) -> Result<Vec<u16>, DomError> {
        self.require_tag(
            node,
            &[
                "input", "select", "textarea", "button", "fieldset", "output",
            ],
        )?;
        Ok(self
            .form_state(node)
            .map(|s| s.custom_error.clone())
            .unwrap_or_default())
    }
    pub fn will_validate(&self, node: NodeIdx) -> Result<bool, DomError> {
        self.require_tag(
            node,
            &[
                "input", "select", "textarea", "button", "fieldset", "output",
            ],
        )?;
        if self.has(node, "disabled")
            || self.html_tag(node, "fieldset")
            || self.html_tag(node, "output")
        {
            return Ok(false);
        }
        if self.has(node, "readonly")
            && (self.html_tag(node, "input") || self.html_tag(node, "textarea"))
        {
            return Ok(false);
        }
        let mut ancestor = self.get(node).parent;
        while let Some(parent) = ancestor {
            if self.html_tag(parent, "datalist") {
                return Ok(false);
            }
            if self.html_tag(parent, "fieldset") && self.has(parent, "disabled") {
                let legend = self
                    .children(parent)
                    .find(|child| self.html_tag(*child, "legend"));
                if !legend.is_some_and(|legend| self.contains(legend, Some(node))) {
                    return Ok(false);
                }
            }
            ancestor = self.get(parent).parent;
        }
        if self.html_tag(node, "input")
            && matches!(
                self.input_type(node)?,
                "hidden" | "button" | "reset" | "submit" | "image"
            )
        {
            return Ok(false);
        }
        if self.html_tag(node, "button")
            && self.attr(node, "type").is_some_and(|v| {
                v.eq_ignore_ascii_case("button") || v.eq_ignore_ascii_case("reset")
            })
        {
            return Ok(false);
        }
        Ok(true)
    }
    pub fn validity(
        &self,
        node: NodeIdx,
        pattern_matches: impl FnOnce(&str, &str) -> Option<bool>,
    ) -> Result<ValidityState, DomError> {
        if !self.will_validate(node)? {
            return Ok(ValidityState::default());
        }
        let mut v = ValidityState::default();
        let kind = if self.html_tag(node, "input") {
            self.input_type(node)?
        } else {
            ""
        };
        let value = if self.html_tag(node, "input") {
            self.input_value(node)?
        } else if self.html_tag(node, "textarea") {
            self.textarea_value(node)?
        } else if self.html_tag(node, "select") {
            self.select_value(node)?
        } else {
            Vec::new()
        };
        let s = String::from_utf16_lossy(&value);
        v.custom_error = self
            .form_state(node)
            .is_some_and(|state| !state.custom_error.is_empty());
        v.value_missing = if kind == "checkbox" {
            self.has(node, "required") && !self.input_checked(node)?
        } else if kind == "radio" {
            let name = self.attr(node, "name").unwrap_or("");
            let owner = self.form_owner(node);
            let root = self.root(node);
            let group: Vec<_> = (0..self.node_count())
                .map(|i| NodeIdx(i as u32))
                .filter(|peer| {
                    self.html_tag(*peer, "input")
                        && self.input_type(*peer).ok() == Some("radio")
                        && self.attr(*peer, "name").unwrap_or("") == name
                        && self.form_owner(*peer) == owner
                        && self.root(*peer) == root
                })
                .collect();
            group.iter().any(|peer| self.has(*peer, "required"))
                && !group
                    .iter()
                    .any(|peer| self.input_checked(*peer).unwrap_or(false))
        } else if self.html_tag(node, "select") && self.has(node, "required") {
            let options = self.select_options(node)?;
            let index = self.selected_index(node)?;
            if index < 0 {
                true
            } else {
                let selected = options[index as usize];
                let first = options.first().copied();
                !self.has(node, "multiple")
                    && self
                        .attr(node, "size")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(1)
                        == 1
                    && first == Some(selected)
                    && self.get(selected).parent == Some(node)
                    && self.option_value(selected)?.is_empty()
            }
        } else {
            self.has(node, "required") && s.is_empty()
        };
        if !s.is_empty() {
            v.type_mismatch = match kind {
                "email" => {
                    let addresses: Vec<_> = if self.has(node, "multiple") {
                        s.split(',').collect()
                    } else {
                        vec![s.as_str()]
                    };
                    !addresses.iter().all(|item| {
                        let item = item.trim();
                        let mut parts = item.split('@');
                        parts
                            .next()
                            .is_some_and(|v| !v.is_empty() && !v.contains([' ', ',']))
                            && parts.next().is_some_and(|v| {
                                !v.is_empty()
                                    && !v.contains([' ', ','])
                                    && v.split('.').all(|label| {
                                        !label.is_empty()
                                            && !label.starts_with('-')
                                            && !label.ends_with('-')
                                    })
                            })
                            && parts.next().is_none()
                    })
                }
                "url" => !(s.contains(':') && !s.contains(' ')),
                _ => false,
            };
            if matches!(
                kind,
                "text" | "search" | "tel" | "url" | "email" | "password"
            ) {
                if let Some(pattern) = self.attr(node, "pattern") {
                    v.pattern_mismatch = pattern_matches(pattern, &s) == Some(false);
                }
            }
            let length = value.len();
            if self.form_state(node).is_some_and(|state| state.dirty_value) {
                v.too_long = self
                    .attr(node, "maxlength")
                    .and_then(|v| v.parse::<usize>().ok())
                    .is_some_and(|max| length > max);
                v.too_short = self
                    .attr(node, "minlength")
                    .and_then(|v| v.parse::<usize>().ok())
                    .is_some_and(|min| length < min);
            }
            if matches!(
                kind,
                "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
            ) {
                if let Some(n) = temporal_number(kind, &s) {
                    let min = self
                        .attr(node, "min")
                        .and_then(|v| temporal_number(kind, v));
                    let max = self
                        .attr(node, "max")
                        .and_then(|v| temporal_number(kind, v));
                    v.range_underflow = min.is_some_and(|min| n < min);
                    v.range_overflow = max.is_some_and(|max| n > max);
                    if self.attr(node, "step") != Some("any") {
                        let scale = match kind {
                            "date" => 86_400_000.,
                            "week" => 604_800_000.,
                            "time" | "datetime-local" => 1000.,
                            _ => 1.,
                        };
                        let default = if matches!(kind, "time" | "datetime-local") {
                            60.
                        } else {
                            1.
                        };
                        let step = self
                            .attr(node, "step")
                            .and_then(valid_number)
                            .filter(|v| *v > 0.)
                            .unwrap_or(default)
                            * scale;
                        let base = min
                            .or_else(|| {
                                self.attr(node, "value")
                                    .and_then(|v| temporal_number(kind, v))
                            })
                            .unwrap_or(if kind == "week" { -259_200_000. } else { 0. });
                        v.step_mismatch =
                            ((n - base) / step - ((n - base) / step).round()).abs() > 1e-8;
                    }
                }
            }
        }
        Ok(v)
    }
    pub fn reset_form(&mut self, form: NodeIdx) -> Result<(), DomError> {
        let mut controls = self.form_controls(form)?;
        let options: Vec<_> = controls
            .iter()
            .copied()
            .filter(|node| self.html_tag(*node, "select"))
            .flat_map(|select| self.select_options(select).unwrap_or_default())
            .collect();
        controls.extend(options);
        for node in controls {
            let state = self.form_state_mut(node);
            state.value = None;
            state.dirty_value = false;
            state.checked = None;
            state.dirty_checked = false;
            state.selected = None;
            state.dirty_selected = false;
            self.record_render_full(node, "form reset state");
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
        assert_eq!(
            dom.set_input_value(input, units("secret")),
            Err(DomError::InvalidStateError)
        );
        dom.set_attribute(input, "type", "color").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("#000000"));
        dom.set_input_value(input, units("#A1B2C3")).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("#a1b2c3"));
        dom.set_attribute(input, "type", "number").unwrap();
        dom.set_input_value(input, units("xyz")).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), Vec::<u16>::new());
    }
    #[test]
    fn range_clamps_and_rounds_without_inverted_bounds() {
        let mut dom = Dom::new();
        let input = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(input, "type", "range").unwrap();
        dom.set_attribute(input, "min", "200").unwrap();
        dom.set_attribute(input, "max", "100").unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("200"));
        dom.set_attribute(input, "max", "210").unwrap();
        dom.set_attribute(input, "step", "2").unwrap();
        dom.set_input_value(input, units("205")).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("206"));
    }
    #[test]
    fn textarea_keeps_raw_children_and_normalizes_api_value() {
        let mut dom = Dom::new();
        let textarea = dom.create_element(NodeIdx(0), "textarea");
        dom.create_text(textarea, "\r\nfirst\rsecond");
        assert_eq!(
            dom.textarea_value(textarea).unwrap(),
            units("\nfirst\nsecond")
        );
        dom.set_textarea_value(textarea, units("other\r\nline"))
            .unwrap();
        assert_eq!(dom.textarea_value(textarea).unwrap(), units("other\nline"));
        let form = dom.create_element(NodeIdx(0), "form");
        dom.pre_insert(textarea, form, None).unwrap();
        dom.reset_form(form).unwrap();
        assert_eq!(
            dom.textarea_value(textarea).unwrap(),
            units("\nfirst\nsecond")
        );
    }
    #[test]
    fn cloning_preserves_dirty_form_state() {
        let mut dom = Dom::new();
        let input = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(input, "value", "default").unwrap();
        dom.set_input_value(input, units("edited")).unwrap();
        dom.set_input_checked(input, true).unwrap();
        let copy = dom.clone_node(input, false).unwrap();
        assert_eq!(dom.input_value(copy).unwrap(), units("edited"));
        assert!(dom.input_checked(copy).unwrap());
        dom.set_attribute(copy, "value", "changed default").unwrap();
        assert_eq!(dom.input_value(copy).unwrap(), units("edited"));
    }
    #[test]
    fn radio_groups_follow_form_owner_and_reparenting() {
        let mut dom = Dom::new();
        let first_form = dom.create_element(NodeIdx(0), "form");
        dom.set_attribute(first_form, "id", "f").unwrap();
        let a = dom.create_element(first_form, "input");
        let b = dom.create_element(NodeIdx(0), "input");
        for node in [a, b] {
            dom.set_attribute(node, "type", "radio").unwrap();
            dom.set_attribute(node, "name", "choice").unwrap();
        }
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
    fn checked_content_attribute_reconciles_radio_group_without_dirtying_default() {
        let mut dom = Dom::new();
        let form = dom.create_element(NodeIdx(0), "form");
        let a = dom.create_element(form, "input");
        let b = dom.create_element(form, "input");
        for node in [a, b] {
            dom.set_attribute(node, "type", "radio").unwrap();
            dom.set_attribute(node, "name", "g").unwrap();
        }
        dom.set_attribute(a, "checked", "").unwrap();
        dom.set_attribute(b, "checked", "").unwrap();
        assert!(!dom.input_checked(a).unwrap());
        assert!(dom.input_checked(b).unwrap());
        dom.remove_attribute(a, "checked").unwrap();
        assert!(!dom.input_checked(a).unwrap());
        dom.reset_form(form).unwrap();
        assert!(dom.input_checked(b).unwrap());
    }
    #[test]
    fn form_owner_uses_first_matching_id_then_ancestor_fallback() {
        let mut dom = Dom::new();
        let blocker = dom.create_element(NodeIdx(0), "div");
        dom.set_attribute(blocker, "id", "target").unwrap();
        let form = dom.create_element(NodeIdx(0), "form");
        dom.set_attribute(form, "id", "target").unwrap();
        let input = dom.create_element(form, "input");
        dom.set_attribute(input, "form", "target").unwrap();
        assert_eq!(dom.form_owner(input), Some(form));
        dom.detach(input);
        assert_eq!(dom.form_owner(input), None);
        dom.pre_insert(input, form, None).unwrap();
        dom.set_attribute(blocker, "id", "other").unwrap();
        assert_eq!(dom.form_owner(input), Some(form));
    }
    #[test]
    fn reparenting_a_checked_radio_reconciles_its_new_group() {
        let mut dom = Dom::new();
        let form = dom.create_element(NodeIdx(0), "form");
        let outside = dom.create_element(NodeIdx(0), "div");
        let existing = dom.create_element(form, "input");
        let moved = dom.create_element(outside, "input");
        for node in [existing, moved] {
            dom.set_attribute(node, "type", "radio").unwrap();
            dom.set_attribute(node, "name", "same").unwrap();
            dom.set_input_checked(node, true).unwrap();
        }
        assert!(dom.input_checked(existing).unwrap());
        dom.pre_insert(moved, form, None).unwrap();
        assert!(!dom.input_checked(existing).unwrap());
        assert!(dom.input_checked(moved).unwrap());
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
    // WPT html/semantics/forms/constraints/form-validation-validity-valueMissing.html
    // and form-validation-willValidate.html at pin 53bc812da2ed81179ebd1ba76b2d197cc4747d12.
    #[test]
    fn required_select_placeholder_and_disabled_fieldset_rules() {
        let mut dom = Dom::new();
        let form = dom.create_element(NodeIdx(0), "form");
        let select = dom.create_element(form, "select");
        dom.set_attribute(select, "required", "").unwrap();
        let placeholder = dom.create_element(select, "option");
        dom.set_attribute(placeholder, "value", "").unwrap();
        let real = dom.create_element(select, "option");
        dom.set_attribute(real, "value", "good").unwrap();
        assert!(dom.validity(select, |_, _| None).unwrap().value_missing);
        dom.set_option_selected(real, true).unwrap();
        assert!(!dom.validity(select, |_, _| None).unwrap().value_missing);
        let fieldset = dom.create_element(form, "fieldset");
        dom.set_attribute(fieldset, "disabled", "").unwrap();
        let input = dom.create_element(fieldset, "input");
        assert!(!dom.will_validate(input).unwrap());
        let legend = dom.create_element(fieldset, "legend");
        let protected = dom.create_element(legend, "input");
        assert!(dom.will_validate(protected).unwrap());
        let datalist = dom.create_element(form, "datalist");
        let hidden = dom.create_element(datalist, "input");
        assert!(!dom.will_validate(hidden).unwrap());
        dom.set_attribute(select, "readonly", "").unwrap();
        assert!(dom.will_validate(select).unwrap());
    }
}

// Assertion ports: WPT pin 53bc812da2ed81179ebd1ba76b2d197cc4747d12,
// html/semantics/forms/the-input-element/input-valueasnumber.html and
// html/semantics/forms/the-input-element/input-valueasdate.html.
#[cfg(test)]
mod temporal_tests {
    use super::*;
    #[test]
    fn date_time_month_and_number_conversions() {
        let mut dom = Dom::new();
        let input = dom.create_element(NodeIdx(0), "input");
        dom.set_attribute(input, "type", "date").unwrap();
        dom.set_input_value(input, units("1970-01-02")).unwrap();
        assert_eq!(dom.input_value_as_number(input).unwrap(), 86_400_000.);
        assert_eq!(
            dom.input_value_as_date_ms(input).unwrap(),
            Some(86_400_000.)
        );
        dom.set_input_value_as_number(input, 0.).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("1970-01-01"));
        dom.set_attribute(input, "type", "month").unwrap();
        dom.set_input_value(input, units("1970-02")).unwrap();
        assert_eq!(dom.input_value_as_number(input).unwrap(), 1.);
        assert_eq!(
            dom.input_value_as_date_ms(input).unwrap(),
            Some(31. * 86_400_000.)
        );
        dom.set_attribute(input, "type", "time").unwrap();
        dom.set_input_value(input, units("12:34:56.789")).unwrap();
        assert_eq!(dom.input_value_as_number(input).unwrap(), 45_296_789.);
        dom.set_input_value_as_number(input, 60_000.).unwrap();
        assert_eq!(dom.input_value(input).unwrap(), units("00:01"));
        dom.set_attribute(input, "type", "text").unwrap();
        assert!(dom.input_value_as_number(input).unwrap().is_nan());
        assert_eq!(
            dom.set_input_value_as_number(input, 1.),
            Err(DomError::InvalidStateError)
        );
    }
}
