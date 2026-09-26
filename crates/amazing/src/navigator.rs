//! Browser-side navigation boundary shared by the shell and headless WPT runner.
//! A controller belongs to one tab; the process implementation is opt-in.
use crate::runtime_host_ipc::{self, Driver};
use amazing_js::SerializedValue;
use amazing_process::history::{Eligibility, HistorySnapshot};

pub(crate) mod state_wire {
    use amazing_ipc::codec::{self, ByteBuf, Decoder, Encoder, Error, Wire};
    use amazing_js::{
        SerializedAtom as Atom, SerializedBoxKind as BoxKind, SerializedHostValue,
        SerializedNode as Node, SerializedValue,
    };

    fn size(value: usize) -> Result<u32, Error> {
        u32::try_from(value).map_err(|_| Error::Limit)
    }
    fn index(value: u32) -> usize {
        value as usize
    }
    fn number(value: f64, e: &mut Encoder) -> Result<(), Error> {
        value.to_bits().encode(e)
    }
    fn read_number(d: &mut Decoder<'_>) -> Result<f64, Error> {
        Ok(f64::from_bits(u64::decode(d)?))
    }
    fn atom(value: &Atom, e: &mut Encoder) -> Result<(), Error> {
        match value {
            Atom::Undefined => 0u8.encode(e),
            Atom::Null => 1u8.encode(e),
            Atom::Boolean(v) => {
                2u8.encode(e)?;
                v.encode(e)
            }
            Atom::Number(v) => {
                3u8.encode(e)?;
                number(*v, e)
            }
            Atom::BigInt { negative, limbs } => {
                4u8.encode(e)?;
                negative.encode(e)?;
                limbs.encode(e)
            }
            Atom::String(v) => {
                5u8.encode(e)?;
                v.encode(e)
            }
            Atom::Reference(v) => {
                6u8.encode(e)?;
                size(*v)?.encode(e)
            }
        }
    }
    fn read_atom(d: &mut Decoder<'_>) -> Result<Atom, Error> {
        Ok(match u8::decode(d)? {
            0 => Atom::Undefined,
            1 => Atom::Null,
            2 => Atom::Boolean(bool::decode(d)?),
            3 => Atom::Number(read_number(d)?),
            4 => Atom::BigInt {
                negative: bool::decode(d)?,
                limbs: Vec::<u32>::decode(d)?,
            },
            5 => Atom::String(Vec::<u16>::decode(d)?),
            6 => Atom::Reference(index(u32::decode(d)?)),
            _ => return Err(Error::Invalid),
        })
    }
    fn atoms(values: &[Atom], e: &mut Encoder) -> Result<(), Error> {
        size(values.len())?.encode(e)?;
        for value in values {
            atom(value, e)?;
        }
        Ok(())
    }
    fn read_atoms(d: &mut Decoder<'_>) -> Result<Vec<Atom>, Error> {
        let len = u32::decode(d)? as usize;
        if len > codec::MAX_ITEMS || len > d.remaining() {
            return Err(Error::Limit);
        }
        (0..len).map(|_| read_atom(d)).collect()
    }
    fn props(values: &[(Vec<u16>, Atom)], e: &mut Encoder) -> Result<(), Error> {
        size(values.len())?.encode(e)?;
        for (key, value) in values {
            key.encode(e)?;
            atom(value, e)?;
        }
        Ok(())
    }
    fn read_props(d: &mut Decoder<'_>) -> Result<Vec<(Vec<u16>, Atom)>, Error> {
        let len = u32::decode(d)? as usize;
        if len > codec::MAX_ITEMS || len > d.remaining() / 2 {
            return Err(Error::Limit);
        }
        (0..len)
            .map(|_| Ok((Vec::<u16>::decode(d)?, read_atom(d)?)))
            .collect()
    }
    fn node(value: &Node, e: &mut Encoder) -> Result<(), Error> {
        match value {
            Node::Object { properties } => {
                0u8.encode(e)?;
                props(properties, e)
            }
            Node::Array { length, properties } => {
                1u8.encode(e)?;
                size(*length)?.encode(e)?;
                props(properties, e)
            }
            Node::Boxed { kind, value } => {
                2u8.encode(e)?;
                (match kind {
                    BoxKind::Boolean => 0u8,
                    BoxKind::Number => 1,
                    BoxKind::BigInt => 2,
                    BoxKind::String => 3,
                })
                .encode(e)?;
                atom(value, e)
            }
            Node::Date(value) => {
                3u8.encode(e)?;
                number(*value, e)
            }
            Node::RegExp { pattern, flags } => {
                4u8.encode(e)?;
                pattern.encode(e)?;
                flags.encode(e)
            }
            Node::ArrayBuffer {
                bytes,
                max_byte_length,
                resizable,
            } => {
                5u8.encode(e)?;
                ByteBuf(bytes.clone()).encode(e)?;
                size(*max_byte_length)?.encode(e)?;
                resizable.encode(e)
            }
            Node::ArrayBufferView {
                buffer,
                byte_offset,
                byte_length,
                kind,
            } => {
                6u8.encode(e)?;
                size(*buffer)?.encode(e)?;
                size(*byte_offset)?.encode(e)?;
                byte_length.map(size).transpose()?.encode(e)?;
                kind.encode(e)
            }
            Node::Map(entries) => {
                7u8.encode(e)?;
                size(entries.len())?.encode(e)?;
                for (key, value) in entries {
                    atom(key, e)?;
                    atom(value, e)?;
                }
                Ok(())
            }
            Node::Set(entries) => {
                8u8.encode(e)?;
                atoms(entries, e)
            }
            Node::Error {
                name,
                message,
                cause,
            } => {
                9u8.encode(e)?;
                name.encode(e)?;
                message.encode(e)?;
                match cause {
                    Some(v) => {
                        true.encode(e)?;
                        atom(v, e)
                    }
                    None => false.encode(e),
                }
            }
            Node::Host(value) => {
                10u8.encode(e)?;
                value.type_name.encode(e)?;
                ByteBuf(value.bytes.clone()).encode(e)
            }
        }
    }
    fn read_node(d: &mut Decoder<'_>) -> Result<Node, Error> {
        Ok(match u8::decode(d)? {
            0 => Node::Object {
                properties: read_props(d)?,
            },
            1 => Node::Array {
                length: index(u32::decode(d)?),
                properties: read_props(d)?,
            },
            2 => Node::Boxed {
                kind: match u8::decode(d)? {
                    0 => BoxKind::Boolean,
                    1 => BoxKind::Number,
                    2 => BoxKind::BigInt,
                    3 => BoxKind::String,
                    _ => return Err(Error::Invalid),
                },
                value: read_atom(d)?,
            },
            3 => Node::Date(read_number(d)?),
            4 => Node::RegExp {
                pattern: Vec::<u16>::decode(d)?,
                flags: String::decode(d)?,
            },
            5 => Node::ArrayBuffer {
                bytes: ByteBuf::decode(d)?.0,
                max_byte_length: index(u32::decode(d)?),
                resizable: bool::decode(d)?,
            },
            6 => Node::ArrayBufferView {
                buffer: index(u32::decode(d)?),
                byte_offset: index(u32::decode(d)?),
                byte_length: Option::<u32>::decode(d)?.map(index),
                kind: Option::<String>::decode(d)?,
            },
            7 => {
                let len = u32::decode(d)? as usize;
                if len > codec::MAX_ITEMS || len > d.remaining() / 2 {
                    return Err(Error::Limit);
                }
                Node::Map(
                    (0..len)
                        .map(|_| Ok((read_atom(d)?, read_atom(d)?)))
                        .collect::<Result<_, Error>>()?,
                )
            }
            8 => Node::Set(read_atoms(d)?),
            9 => {
                let name = String::decode(d)?;
                let message = Option::<Vec<u16>>::decode(d)?;
                let cause = if bool::decode(d)? {
                    Some(read_atom(d)?)
                } else {
                    None
                };
                Node::Error {
                    name,
                    message,
                    cause,
                }
            }
            10 => Node::Host(SerializedHostValue {
                type_name: String::decode(d)?,
                bytes: ByteBuf::decode(d)?.0,
            }),
            _ => return Err(Error::Invalid),
        })
    }
    fn valid_atom(atom: &Atom, count: usize) -> bool {
        !matches!(atom, Atom::Reference(index) if *index >= count)
    }
    pub fn encode(value: &SerializedValue) -> Result<Vec<u8>, String> {
        let mut e = Encoder::new();
        1u16.encode(&mut e).map_err(|e| e.to_string())?;
        atom(&value.root, &mut e).map_err(|e| e.to_string())?;
        size(value.nodes.len())
            .map_err(|e| e.to_string())?
            .encode(&mut e)
            .map_err(|e| e.to_string())?;
        for value in &value.nodes {
            node(value, &mut e).map_err(|e| e.to_string())?;
        }
        let bytes = e.finish();
        if bytes.len() > amazing_process::history::MAX_STATE_BYTES {
            return Err("history.state limit".into());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<SerializedValue, String> {
        if bytes.len() > amazing_process::history::MAX_STATE_BYTES {
            return Err("history.state limit".into());
        }
        let mut d = Decoder::new(bytes).map_err(|e| e.to_string())?;
        if u16::decode(&mut d).map_err(|e| e.to_string())? != 1 {
            return Err("history.state version".into());
        }
        let root = read_atom(&mut d).map_err(|e| e.to_string())?;
        let len = u32::decode(&mut d).map_err(|e| e.to_string())? as usize;
        if len > codec::MAX_ITEMS || len > d.remaining() {
            return Err("history.state node limit".into());
        }
        let nodes = (0..len)
            .map(|_| read_node(&mut d))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        d.finish().map_err(|e| e.to_string())?;
        if !valid_atom(&root, len) {
            return Err("history.state reference".into());
        }
        for node in &nodes {
            let valid = match node {
                Node::Object { properties } | Node::Array { properties, .. } => {
                    properties.iter().all(|(_, v)| valid_atom(v, len))
                }
                Node::Boxed { value, .. } => valid_atom(value, len),
                Node::Map(entries) => entries
                    .iter()
                    .all(|(k, v)| valid_atom(k, len) && valid_atom(v, len)),
                Node::Set(entries) => entries.iter().all(|v| valid_atom(v, len)),
                Node::Error { cause, .. } => cause.as_ref().is_none_or(|v| valid_atom(v, len)),
                Node::ArrayBufferView { buffer, .. } => *buffer < len,
                _ => true,
            };
            if !valid {
                return Err("history.state reference".into());
            }
        }
        Ok(SerializedValue { root, nodes })
    }
}

const ROOT_NAVIGABLE: u64 = 1;

#[derive(Clone)]
struct Entry {
    url: String,
    scroll_y: i32,
    history_state: Option<SerializedValue>,
    process_step: Option<u64>,
    document: u64,
}

pub struct NavigationResult<T> {
    pub rendered: T,
    pub url: String,
    pub scroll_y: i32,
    pub history_index: usize,
    pub history_length: usize,
    pub history: Vec<String>,
    pub history_state: Option<SerializedValue>,
}

/// Owns one tab's browser history, process tokens, and child navigables.
pub struct Navigator {
    tab_id: u64,
    process: Option<Driver>,
    entries: Vec<Entry>,
    index: Option<usize>,
    next_direct_child: u64,
    next_direct_document: u64,
}

impl Navigator {
    pub fn new(tab_id: u64) -> Self {
        Self {
            tab_id,
            process: None,
            entries: Vec::new(),
            index: None,
            next_direct_child: ROOT_NAVIGABLE + 1,
            next_direct_document: 1,
        }
    }

    pub fn current_url(&self) -> Option<&str> {
        self.index
            .and_then(|index| self.entries.get(index))
            .map(|entry| entry.url.as_str())
    }

    pub fn history_length(&self) -> usize {
        self.entries.len()
    }

    pub fn history_index(&self) -> Option<usize> {
        self.index
    }

    fn current_uses_process(&self) -> bool {
        self.index
            .and_then(|index| self.entries.get(index))
            .is_some_and(|entry| entry.process_step.is_some())
    }

    pub fn urls(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.url.clone()).collect()
    }

    pub fn target(&self, delta: i32) -> Option<(usize, &str)> {
        let current = i64::try_from(self.index?).ok()?;
        let next = current.checked_add(i64::from(delta))?;
        let index = usize::try_from(next).ok()?;
        self.entries
            .get(index)
            .map(|entry| (index, entry.url.as_str()))
    }

    pub fn same_document_target(&self, target: usize) -> bool {
        self.index
            .and_then(|index| self.entries.get(index))
            .zip(self.entries.get(target))
            .is_some_and(|(current, next)| {
                current.document == next.document
                    && current.process_step.is_some() == next.process_step.is_some()
            })
    }

    pub fn traverse_same_document(
        &mut self,
        target: usize,
        outgoing_scroll_y: i32,
    ) -> Result<(String, i32, Option<SerializedValue>), String> {
        if !self.same_document_target(target) {
            return Err("traversal crosses documents".into());
        }
        let current = self.index.unwrap();
        if self.current_uses_process()
            && let Some(driver) = self.process.as_mut()
        {
            let step = self.entries[target]
                .process_step
                .ok_or("target lacks process step")?;
            driver.traverse_same_document(
                self.tab_id,
                step,
                HistorySnapshot {
                    history_state: self.entries[current]
                        .history_state.as_ref().map(state_wire::encode).transpose()?,
                    scroll: vec![(ROOT_NAVIGABLE, 0, outgoing_scroll_y)],
                    ..HistorySnapshot::default()
                },
            )?;
        }
        self.entries[current].scroll_y = outgoing_scroll_y;
        self.index = Some(target);
        let entry = &self.entries[target];
        Ok((
            entry.url.clone(),
            entry.scroll_y,
            entry.history_state.clone(),
        ))
    }

    fn process(&mut self, url: &str) -> Result<Option<&mut Driver>, String> {
        if !runtime_host_ipc::enabled()
            || !(url.starts_with("http://") || url.starts_with("https://"))
        {
            return Ok(None);
        }
        if self.process.is_none() {
            self.process = Some(Driver::new(runtime_host_ipc::child_executable()?));
        }
        Ok(self.process.as_mut())
    }

    /// `target` selects a traversal. None with `record=false` requests reload.
    /// The caller receives the saved scroll position when a step is restored.
    pub fn navigate<T>(
        &mut self,
        url: &str,
        record: bool,
        target: Option<usize>,
        outgoing_scroll_y: i32,
        render: impl FnOnce(&str, &[u8]) -> Result<T, String>,
    ) -> Result<NavigationResult<T>, String> {
        let target = if !record && target == self.index {
            None
        } else {
            target
        };
        if !record && target.is_some_and(|index| index >= self.entries.len()) {
            return Err("history target is outside this tab".into());
        }
        let tab_id = self.tab_id;
        let target_step = target
            .and_then(|index| self.entries.get(index))
            .and_then(|entry| entry.process_step);
        let direct_restored_state = target
            .or_else(|| (!record).then_some(self.index).flatten())
            .and_then(|index| self.entries.get(index))
            .and_then(|entry| entry.history_state.clone());
        let snapshot = HistorySnapshot {
            history_state: self
                .index
                .and_then(|index| self.entries.get(index))
                .and_then(|entry| entry.history_state.as_ref())
                .map(state_wire::encode)
                .transpose()?,
            scroll: vec![(ROOT_NAVIGABLE, 0, outgoing_scroll_y)],
            ..HistorySnapshot::default()
        };
        let process = self.process(url)?;
        let (rendered, final_url, process_step, restored_scroll, restored_state, document) =
            if let Some(driver) = process {
                let step_index = target_step
                    .and_then(|step| {
                        driver
                            .history(tab_id)?
                            .steps()
                            .iter()
                            .position(|candidate| candidate.id == step)
                    })
                    .unwrap_or(usize::MAX);
                let rendered = driver.navigate(
                    tab_id,
                    url,
                    record,
                    step_index,
                    snapshot,
                    Eligibility {
                        // Until the renderer reports quiescence, discard conservatively.
                        outstanding_ipc: true,
                        ..Eligibility::default()
                    },
                    render,
                )?;
                let history = driver.history(tab_id).ok_or("missing tab history")?;
                let active = history
                    .active_entry(ROOT_NAVIGABLE)
                    .ok_or("navigation has no active root")?;
                let scroll = active
                    .snapshot
                    .scroll
                    .iter()
                    .find(|(id, _, _)| *id == ROOT_NAVIGABLE)
                    .map_or(0, |(_, _, y)| *y);
                (
                    rendered,
                    active.url.clone(),
                    Some(active.step),
                    scroll,
                    active
                        .snapshot
                        .history_state
                        .as_deref()
                        .map(state_wire::decode)
                        .transpose()?,
                    active.document,
                )
            } else {
                let rendered = render(url, &[])?;
                let scroll = target
                    .and_then(|index| self.entries.get(index))
                    .map_or(0, |entry| entry.scroll_y);
                let document = target
                    .and_then(|index| self.entries.get(index))
                    .map(|entry| entry.document)
                    .unwrap_or_else(|| {
                        let id = self.next_direct_document;
                        self.next_direct_document = id.saturating_add(1);
                        id
                    });
                (
                    rendered,
                    url.to_owned(),
                    None,
                    scroll,
                    direct_restored_state,
                    document,
                )
            };
        if let Some(index) = self.index
            && let Some(entry) = self.entries.get_mut(index)
        {
            entry.scroll_y = outgoing_scroll_y;
        }
        let entry = Entry {
            url: final_url.clone(),
            scroll_y: restored_scroll,
            history_state: restored_state.clone(),
            process_step,
            document,
        };
        let index = if record || self.index.is_none() {
            self.entries
                .truncate(self.index.map_or(0, |index| index + 1));
            self.entries.push(entry);
            self.entries.len() - 1
        } else if let Some(target) = target {
            self.entries[target].url = entry.url;
            self.entries[target].process_step = entry.process_step;
            self.entries[target].document = entry.document;
            self.entries[target].scroll_y = entry.scroll_y;
            target
        } else {
            let index = self.index.unwrap();
            self.entries[index] = entry;
            index
        };
        self.index = Some(index);
        Ok(NavigationResult {
            rendered,
            url: final_url,
            scroll_y: restored_scroll,
            history_index: index,
            history_length: self.entries.len(),
            history: self.urls(),
            history_state: restored_state,
        })
    }

    pub fn same_document(
        &mut self,
        url: String,
        replace: bool,
        state: Option<SerializedValue>,
        scroll_y: i32,
    ) -> Result<(), String> {
        let index = self.index.ok_or("same-document navigation has no page")?;
        if url.len() > 8192 {
            return Err("history URL limit".into());
        }
        let current_url = &self.entries[index].url;
        if current_url.starts_with("http://") || current_url.starts_with("https://") {
            let current = amazing_fetch::HttpUrl::parse(current_url)
                .map_err(|_| "invalid current history URL")?;
            let proposed =
                amazing_fetch::HttpUrl::parse(&url).map_err(|_| "invalid same-document URL")?;
            if current.origin() != proposed.origin() {
                return Err("same-document URL changes origin".into());
            }
        }
        let snapshot = HistorySnapshot {
            history_state: state.as_ref().map(state_wire::encode).transpose()?,
            scroll: vec![(ROOT_NAVIGABLE, 0, scroll_y)],
            ..HistorySnapshot::default()
        };
        snapshot
            .validate()
            .map_err(|error| format!("history snapshot: {error:?}"))?;
        let step = if self.current_uses_process() {
            let driver = self.process.as_mut().ok_or("missing process history")?;
            Some(
                driver
                    .same_document(self.tab_id, ROOT_NAVIGABLE, url.clone(), replace, snapshot)?
                    .step,
            )
        } else {
            None
        };
        let entry = Entry {
            url,
            scroll_y,
            history_state: state,
            process_step: step,
            document: self.entries[index].document,
        };
        if replace {
            self.entries[index] = entry;
        } else {
            self.entries.truncate(index + 1);
            self.entries.push(entry);
            self.index = Some(self.entries.len() - 1);
        }
        Ok(())
    }

    pub fn create_child(&mut self, parent: u64) -> Result<u64, String> {
        if self.current_uses_process()
            && let Some(driver) = self.process.as_mut()
        {
            driver.create_child(self.tab_id, parent)
        } else {
            let child = self.next_direct_child;
            self.next_direct_child = child.checked_add(1).ok_or("child IDs exhausted")?;
            Ok(child)
        }
    }

    pub fn navigate_child<T>(
        &mut self,
        child: u64,
        url: &str,
        render: impl FnOnce(&str, &[u8]) -> Result<T, String>,
    ) -> Result<T, String> {
        let tab_id = self.tab_id;
        if self.current_uses_process()
            && let Some(driver) = self.process(url)?
        {
            let output = driver.navigate_child(
                tab_id,
                child,
                url,
                HistorySnapshot::default(),
                Eligibility {
                    outstanding_ipc: true,
                    ..Eligibility::default()
                },
                render,
            )?;
            let step = driver
                .history(tab_id)
                .and_then(|history| history.current_step());
            if let Some(index) = self.index {
                self.entries[index].process_step = step;
            }
            Ok(output)
        } else {
            render(url, &[])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_js::{SerializedAtom, SerializedNode};

    #[test]
    fn history_state_crosses_wire_into_new_realm() {
        let mut source = amazing_js::Interpreter::new();
        let value = source
            .run_script("var x = { count: 4 }; x.self = x; x")
            .unwrap();
        let state = source.structured_serialize(&value).unwrap();
        let bytes = state_wire::encode(&state).unwrap();
        let restored = state_wire::decode(&bytes).unwrap();
        let mut target = amazing_js::Interpreter::new();
        let value = target.structured_deserialize(&restored).unwrap();
        target
            .global_obj
            .borrow_mut()
            .set_own("wire", value.clone());
        target.global_env.declare_var("wire", value).unwrap();
        assert_eq!(
            target
                .run_script("wire.self === wire && wire.count === 4")
                .unwrap(),
            amazing_js::value::JsValue::Bool(true)
        );
    }

    #[test]
    fn history_state_wire_preserves_cycles_and_rejects_bad_references() {
        let value = SerializedValue {
            root: SerializedAtom::Reference(0),
            nodes: vec![SerializedNode::Object {
                properties: vec![(
                    "self".encode_utf16().collect(),
                    SerializedAtom::Reference(0),
                )],
            }],
        };
        let bytes = state_wire::encode(&value).unwrap();
        let decoded = state_wire::decode(&bytes).unwrap();
        assert!(matches!(decoded.root, SerializedAtom::Reference(0)));
        assert!(
            matches!(&decoded.nodes[0], SerializedNode::Object { properties } if matches!(properties[0].1, SerializedAtom::Reference(0)))
        );
        let invalid = SerializedValue {
            root: SerializedAtom::Reference(1),
            nodes: value.nodes,
        };
        assert!(state_wire::decode(&state_wire::encode(&invalid).unwrap()).is_err());
    }

    #[test]
    fn one_tab_restores_same_and_cross_document_scroll() {
        let mut navigator = Navigator::new(7);
        let first = navigator
            .navigate("file:///first.html", true, None, 0, |url, _| {
                Ok(url.to_owned())
            })
            .unwrap();
        assert_eq!(first.history_length, 1);
        navigator
            .same_document(
                "file:///first.html#state".into(),
                false,
                Some(SerializedValue {
                    root: SerializedAtom::Number(7.0),
                    nodes: Vec::new(),
                }),
                12,
            )
            .unwrap();
        assert!(navigator.same_document_target(0));
        let (url, scroll, state) = navigator.traverse_same_document(0, 23).unwrap();
        assert_eq!((url.as_str(), scroll), ("file:///first.html", 0));
        assert!(state.is_none());
        let (url, _, state) = navigator.traverse_same_document(1, 0).unwrap();
        assert_eq!(url, "file:///first.html#state");
        assert!(matches!(state.unwrap().root, SerializedAtom::Number(7.0)));
        let second = navigator
            .navigate("file:///second.html", true, None, 31, |url, _| {
                Ok(url.to_owned())
            })
            .unwrap();
        assert_eq!(
            second.history,
            vec![
                "file:///first.html".to_owned(),
                "file:///first.html#state".to_owned(),
                "file:///second.html".to_owned()
            ]
        );
        let restored = navigator
            .navigate("file:///first.html#state", false, Some(1), 44, |url, _| {
                Ok(url.to_owned())
            })
            .unwrap();
        assert_eq!(restored.scroll_y, 31);
        assert_eq!(restored.history_index, 1);
        assert!(matches!(
            restored.history_state.unwrap().root,
            SerializedAtom::Number(7.0)
        ));
    }
}
