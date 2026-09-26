//! The `DOMSnapshot` domain: a flat, string-interned snapshot of every
//! document (`DOMSnapshot.captureSnapshot`).
//!
//! CDP's wire format references node names/values/attributes by index into a
//! shared `strings` array, to keep large trees compact. The engine seam
//! ([`crate::host::DomSnapshotDocument`]/[`crate::host::DomSnapshotNode`])
//! stays in plain strings; this domain performs the interning and the
//! parent-index inversion (the seam builds `children` top-down, which is the
//! natural direction for a tree walk; the wire format wants `parentIndex`
//! per node) when it serializes.

use super::object;
use crate::host::{DomSnapshotDocument, DomSnapshotNode};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};
use std::collections::HashMap;

/// Handles a `DOMSnapshot.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "DOMSnapshot.captureSnapshot" => {
            let computed_styles = string_array(request, "computedStyles")?;
            let documents = session
                .dev
                .capture_dom_snapshot(&computed_styles)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(capture_result(&documents)))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn string_array(request: &CdpRequest, name: &str) -> Result<Vec<String>, CdpError> {
    match request.params.get(name) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    CdpError::invalid_params(format!("{name} must be an array of strings"))
                })
            })
            .collect(),
        Some(_) => Err(CdpError::invalid_params(format!("{name} must be an array"))),
    }
}

/// Deduplicating string table for the wire format's shared `strings` array.
struct StringTable {
    strings: Vec<String>,
    index: HashMap<String, i64>,
}

impl StringTable {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn intern(&mut self, value: &str) -> i64 {
        if let Some(&id) = self.index.get(value) {
            return id;
        }
        let id = self.strings.len() as i64;
        self.strings.push(value.to_string());
        self.index.insert(value.to_string(), id);
        id
    }
}

/// Serializes every document, then the shared `strings` array they interned
/// into.
fn capture_result(documents: &[DomSnapshotDocument]) -> Json {
    let mut table = StringTable::new();
    let documents_json: Vec<Json> = documents
        .iter()
        .map(|document| document_json(document, &mut table))
        .collect();
    object(vec![
        ("documents", Json::Array(documents_json)),
        (
            "strings",
            Json::Array(table.strings.into_iter().map(Json::String).collect()),
        ),
    ])
}

/// Serializes one `DOMSnapshot.DocumentSnapshot`.
fn document_json(document: &DomSnapshotDocument, table: &mut StringTable) -> Json {
    let document_url = table.intern(&document.document_url);
    let base_url = table.intern(&document.base_url);

    let mut parent_index = vec![-1i64; document.nodes.len()];
    for (index, node) in document.nodes.iter().enumerate() {
        for &child in &node.children {
            if let Some(slot) = parent_index.get_mut(child as usize) {
                *slot = index as i64;
            }
        }
    }

    let mut node_type = Vec::with_capacity(document.nodes.len());
    let mut node_name = Vec::with_capacity(document.nodes.len());
    let mut node_value = Vec::with_capacity(document.nodes.len());
    let mut backend_node_id = Vec::with_capacity(document.nodes.len());
    let mut attributes = Vec::with_capacity(document.nodes.len());
    for node in &document.nodes {
        node_type.push(Json::number(node.node_type as f64));
        node_name.push(Json::number(table.intern(&node.node_name) as f64));
        node_value.push(Json::number(table.intern(&node.node_value) as f64));
        backend_node_id.push(Json::number(node.backend_node_id.0 as f64));
        attributes.push(attributes_json(node, table));
    }

    object(vec![
        ("documentURL", Json::number(document_url as f64)),
        ("baseURL", Json::number(base_url as f64)),
        (
            "nodes",
            object(vec![
                (
                    "parentIndex",
                    Json::Array(parent_index.into_iter().map(|v| Json::number(v as f64)).collect()),
                ),
                ("nodeType", Json::Array(node_type)),
                ("nodeName", Json::Array(node_name)),
                ("nodeValue", Json::Array(node_value)),
                ("backendNodeId", Json::Array(backend_node_id)),
                ("attributes", Json::Array(attributes)),
            ]),
        ),
    ])
}

/// Serializes one node's `attributes` as a flat `[nameIdx, valueIdx, ...]`
/// array (CDP's `ArrayOfStrings`).
fn attributes_json(node: &DomSnapshotNode, table: &mut StringTable) -> Json {
    let mut flat = Vec::with_capacity(node.attributes.len() * 2);
    for (name, value) in &node.attributes {
        flat.push(Json::number(table.intern(name) as f64));
        flat.push(Json::number(table.intern(value) as f64));
    }
    Json::Array(flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, EngineNodeId, HostError};

    struct Tree;

    impl DevToolsSession for Tree {
        fn capture_dom_snapshot(
            &mut self,
            _computed_styles: &[String],
        ) -> Result<Vec<DomSnapshotDocument>, HostError> {
            Ok(vec![DomSnapshotDocument {
                document_url: "about:blank".into(),
                base_url: "about:blank".into(),
                nodes: vec![
                    DomSnapshotNode {
                        node_type: 1,
                        node_name: "DIV".into(),
                        node_value: String::new(),
                        backend_node_id: EngineNodeId(4),
                        attributes: vec![("id".into(), "main".into())],
                        children: vec![1],
                    },
                    DomSnapshotNode {
                        node_type: 3,
                        node_name: "#text".into(),
                        node_value: "Hello".into(),
                        backend_node_id: EngineNodeId(6),
                        attributes: Vec::new(),
                        children: Vec::new(),
                    },
                ],
            }])
        }
    }

    fn session() -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", "A"),
            Box::new(Tree),
        )
    }

    #[test]
    fn capture_snapshot_interns_strings_and_computes_parent_index() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"DOMSnapshot.captureSnapshot"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let documents = outcome.result.get("documents").unwrap().as_array().unwrap();
        assert_eq!(documents.len(), 1);
        let nodes = documents[0].get("nodes").unwrap();
        let parent_index = nodes.get("parentIndex").unwrap().as_array().unwrap();
        assert_eq!(parent_index[0].as_f64(), Some(-1.0));
        assert_eq!(parent_index[1].as_f64(), Some(0.0));

        let strings = outcome.result.get("strings").unwrap().as_array().unwrap();
        let node_name = nodes.get("nodeName").unwrap().as_array().unwrap();
        let div_name_index = node_name[0].as_f64().unwrap() as usize;
        assert_eq!(strings[div_name_index].as_str(), Some("DIV"));

        let attributes = nodes.get("attributes").unwrap().as_array().unwrap();
        let div_attrs = attributes[0].as_array().unwrap();
        assert_eq!(div_attrs.len(), 2);
        let name_index = div_attrs[0].as_f64().unwrap() as usize;
        let value_index = div_attrs[1].as_f64().unwrap() as usize;
        assert_eq!(strings[name_index].as_str(), Some("id"));
        assert_eq!(strings[value_index].as_str(), Some("main"));

        let backend = nodes.get("backendNodeId").unwrap().as_array().unwrap();
        assert_eq!(backend[0].as_f64(), Some(4.0));
        assert_eq!(backend[1].as_f64(), Some(6.0));
    }
}
