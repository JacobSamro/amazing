//! The `DOM` domain: document tree, queries, serialization and attributes.

use super::{object, remote_object};
use crate::host::{BoxModel, EngineNodeId, NodeDescription, Quad, RemoteObject};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `DOM.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "DOM.enable" => {
            session.enabled.dom = true;
            Ok(DispatchOutcome::empty())
        }
        "DOM.disable" => {
            session.enabled.dom = false;
            Ok(DispatchOutcome::empty())
        }
        "DOM.getDocument" => {
            let depth = depth_param(request, 1)?;
            let root = session.dev.get_document(depth).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "root",
                node_to_json(session, &root),
            )])))
        }
        "DOM.querySelector" => {
            let node = require_node(session, request)?;
            let selector = request.string_param("selector")?;
            let found = session
                .dev
                .query_selector(node, selector)
                .map_err(CdpError::from)?;
            let node_id = found.map(|engine| session.nodes.id(engine)).unwrap_or(0);
            Ok(DispatchOutcome::result(object(vec![(
                "nodeId",
                Json::number(node_id as f64),
            )])))
        }
        "DOM.querySelectorAll" => {
            let node = require_node(session, request)?;
            let selector = request.string_param("selector")?;
            let found = session
                .dev
                .query_selector_all(node, selector)
                .map_err(CdpError::from)?;
            let ids: Vec<Json> = found
                .into_iter()
                .map(|engine| Json::number(session.nodes.id(engine) as f64))
                .collect();
            Ok(DispatchOutcome::result(object(vec![(
                "nodeIds",
                Json::Array(ids),
            )])))
        }
        "DOM.getOuterHTML" => {
            let node = resolve_node(session, request)?;
            let html = session.dev.get_outer_html(node).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "outerHTML",
                Json::String(html),
            )])))
        }
        "DOM.describeNode" => {
            let node = resolve_node(session, request)?;
            let depth = depth_param(request, 0)?;
            let described = session
                .dev
                .describe_node(node, depth)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "node",
                node_to_json(session, &described),
            )])))
        }
        "DOM.requestChildNodes" => {
            let node = require_node(session, request)?;
            let depth = depth_param(request, 1)?;
            let described = session
                .dev
                .describe_node(node, depth)
                .map_err(CdpError::from)?;
            let parent_id = session.nodes.id(described.node);
            let nodes: Vec<Json> = described
                .children
                .iter()
                .map(|child| node_to_json(session, child))
                .collect();
            let event = crate::protocol::event(
                "DOM.setChildNodes",
                object(vec![
                    ("parentId", Json::number(parent_id as f64)),
                    ("nodes", Json::Array(nodes)),
                ]),
            );
            Ok(DispatchOutcome::empty().with_events(vec![event]))
        }
        "DOM.getAttributes" => {
            let node = require_node(session, request)?;
            let described = session.dev.describe_node(node, 0).map_err(CdpError::from)?;
            let mut attributes = Vec::new();
            for (name, value) in &described.attributes {
                attributes.push(Json::String(name.clone()));
                attributes.push(Json::String(value.clone()));
            }
            Ok(DispatchOutcome::result(object(vec![(
                "attributes",
                Json::Array(attributes),
            )])))
        }
        "DOM.setAttributeValue" => {
            let node = require_node(session, request)?;
            let name = request.string_param("name")?;
            let value = request.string_param("value")?;
            session
                .dev
                .set_attribute_value(node, name, Some(value))
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "DOM.removeAttribute" => {
            let node = require_node(session, request)?;
            let name = request.string_param("name")?;
            session
                .dev
                .set_attribute_value(node, name, None)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "DOM.resolveNode" => {
            let node = resolve_node(session, request)?;
            let described = session.dev.describe_node(node, 0).map_err(CdpError::from)?;
            let remote = RemoteObject {
                object_type: "object".into(),
                subtype: Some("node".into()),
                class_name: Some(described.node_name.clone()),
                description: Some(describe_label(&described)),
                object_id: Some(node_object_id(node)),
                ..RemoteObject::undefined()
            };
            Ok(DispatchOutcome::result(object(vec![(
                "object",
                remote_object(&remote),
            )])))
        }
        "DOM.setNodeValue" => {
            let node = require_node(session, request)?;
            let value = request.string_param("value")?;
            session
                .dev
                .set_node_value(node, value)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "DOM.getBoxModel" => {
            let node = resolve_node(session, request)?;
            let model = session.dev.get_box_model(node).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "model",
                box_model_json(&model),
            )])))
        }
        "DOM.getContentQuads" => {
            let node = resolve_node(session, request)?;
            let quads = session
                .dev
                .get_content_quads(node)
                .map_err(CdpError::from)?;
            let quads: Vec<Json> = quads.iter().map(|model| quad_json(&model.content)).collect();
            Ok(DispatchOutcome::result(object(vec![(
                "quads",
                Json::Array(quads),
            )])))
        }
        "DOM.performSearch" => {
            let query = request.string_param("query")?;
            let _include_shadow = request.optional_bool("includeUserAgentShadowDOM", false)?;
            let found = session.dev.perform_search(query).map_err(CdpError::from)?;
            let count = found.len();
            let search_id = session.store_search(found);
            Ok(DispatchOutcome::result(object(vec![
                ("searchId", Json::String(search_id)),
                ("resultCount", Json::number(count as f64)),
            ])))
        }
        "DOM.getSearchResults" => {
            let search_id = request.string_param("searchId")?;
            let from = request.optional_u64("fromIndex")?.unwrap_or(0) as usize;
            let to = request.optional_u64("toIndex")?.unwrap_or(u64::MAX) as usize;
            let results = session.searches.get(search_id).cloned().unwrap_or_default();
            let end = to.min(results.len());
            let start = from.min(end);
            let ids: Vec<Json> = results[start..end]
                .iter()
                .map(|engine| Json::number(session.nodes.id(*engine) as f64))
                .collect();
            Ok(DispatchOutcome::result(object(vec![(
                "nodeIds",
                Json::Array(ids),
            )])))
        }
        "DOM.discardSearchResults" => {
            let search_id = request.string_param("searchId")?;
            session.searches.remove(search_id);
            Ok(DispatchOutcome::empty())
        }
        "DOM.pushNodesByBackendIdsToFrontend" => {
            let raw = request
                .params
                .get("backendNodeIds")
                .and_then(Json::as_array)
                .ok_or_else(|| CdpError::invalid_params("backendNodeIds must be an array"))?;
            let mut backend = Vec::with_capacity(raw.len());
            for item in raw {
                backend.push(item.as_u64().ok_or_else(|| {
                    CdpError::invalid_params("backendNodeIds entries must be integers")
                })?);
            }
            let nodes = session.dev.push_nodes(&backend).map_err(CdpError::from)?;
            let ids: Vec<Json> = nodes
                .into_iter()
                .map(|engine| Json::number(session.nodes.id(engine) as f64))
                .collect();
            Ok(DispatchOutcome::result(object(vec![(
                "nodeIds",
                Json::Array(ids),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Serializes a quad as CDP's flat `[x1,y1,x2,y2,x3,y3,x4,y4]`.
fn quad_json(quad: &Quad) -> Json {
    let mut values = Vec::with_capacity(8);
    for (x, y) in quad.points {
        values.push(Json::number(x));
        values.push(Json::number(y));
    }
    Json::Array(values)
}

/// Serializes a `DOM.BoxModel`.
fn box_model_json(model: &BoxModel) -> Json {
    object(vec![
        ("content", quad_json(&model.content)),
        ("padding", quad_json(&model.padding)),
        ("border", quad_json(&model.border)),
        ("margin", quad_json(&model.margin)),
        ("width", Json::number(model.width as f64)),
        ("height", Json::number(model.height as f64)),
    ])
}

/// The opaque object handle used by `DOM.resolveNode`.
pub fn node_object_id(node: EngineNodeId) -> String {
    format!("node:{}", node.0)
}

/// Serializes one `DOM.Node`, assigning and remembering CDP node ids.
pub fn node_to_json(session: &mut SessionState, described: &NodeDescription) -> Json {
    let node_id = session.nodes.id(described.node);
    let mut fields = vec![
        ("nodeId", Json::number(node_id as f64)),
        ("backendNodeId", Json::number(described.backend_node_id as f64)),
        ("nodeType", Json::number(described.node_type as f64)),
        ("nodeName", Json::String(described.node_name.clone())),
        ("localName", Json::String(described.local_name.clone())),
        ("nodeValue", Json::String(described.node_value.clone())),
    ];
    if !described.attributes.is_empty() {
        let mut attributes = Vec::new();
        for (name, value) in &described.attributes {
            attributes.push(Json::String(name.clone()));
            attributes.push(Json::String(value.clone()));
        }
        fields.push(("attributes", Json::Array(attributes)));
    }
    if let Some(parent) = described.parent {
        let parent_id = session.nodes.id(parent);
        fields.push(("parentId", Json::number(parent_id as f64)));
    }
    let child_count = described.child_node_count.max(described.children.len());
    if child_count > 0 {
        fields.push(("childNodeCount", Json::number(child_count as f64)));
    }
    if !described.children.is_empty() {
        let children: Vec<Json> = described
            .children
            .iter()
            .map(|child| node_to_json(session, child))
            .collect();
        fields.push(("children", Json::Array(children)));
    }
    object(fields)
}

fn describe_label(described: &NodeDescription) -> String {
    let mut label = described.local_name.clone();
    for (name, value) in &described.attributes {
        if name == "id" {
            label.push('#');
            label.push_str(value);
        } else if name == "class" {
            label.push('.');
            label.push_str(&value.replace(' ', "."));
        }
    }
    label
}

/// Resolves `nodeId` or `backendNodeId` from the request to engine identity.
fn resolve_node(session: &mut SessionState, request: &CdpRequest) -> Result<EngineNodeId, CdpError> {
    if let Some(node_id) = request.optional_u64("nodeId")? {
        if node_id == 0 {
            return Err(CdpError::server("Could not find node with given id"));
        }
        return session
            .nodes
            .engine(node_id)
            .ok_or_else(|| CdpError::server("Could not find node with given id"));
    }
    if let Some(backend) = request.optional_u64("backendNodeId")? {
        return session.dev.resolve_node(backend).map_err(CdpError::from);
    }
    Err(CdpError::invalid_params(
        "Either nodeId or backendNodeId must be specified",
    ))
}

fn require_node(session: &mut SessionState, request: &CdpRequest) -> Result<EngineNodeId, CdpError> {
    let node_id = request
        .optional_u64("nodeId")?
        .ok_or_else(|| CdpError::invalid_params("missing nodeId"))?;
    if node_id == 0 {
        return Err(CdpError::server("Could not find node with given id"));
    }
    session
        .nodes
        .engine(node_id)
        .ok_or_else(|| CdpError::server("Could not find node with given id"))
}

/// Reads the CDP `depth` parameter: default `default`, `-1` means the whole
/// subtree (clamped to a safe maximum).
fn depth_param(request: &CdpRequest, default: u32) -> Result<u32, CdpError> {
    match request.params.get("depth") {
        None | Some(Json::Null) => Ok(default),
        Some(value) => match value.as_i64() {
            Some(-1) => Ok(64),
            Some(n) if n < 0 => Ok(0),
            Some(n) => Ok((n as u32).min(64)),
            None => Err(CdpError::invalid_params("depth must be an integer")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError, NodeDescription};
    use crate::session::SessionState;

    struct Tree;

    fn document() -> NodeDescription {
        let div = NodeDescription::element(EngineNodeId(2), "div")
            .with_attributes(vec![("id".into(), "a".into())])
            .with_children(vec![NodeDescription::text(EngineNodeId(3), "hi")]);
        NodeDescription::element(EngineNodeId(1), "html")
            .with_children(vec![NodeDescription::element(EngineNodeId(4), "body").with_children(vec![div])])
    }

    impl DevToolsSession for Tree {
        fn get_document(&mut self, _depth: u32) -> Result<NodeDescription, HostError> {
            Ok(document())
        }
        fn query_selector(
            &mut self,
            _node: EngineNodeId,
            selector: &str,
        ) -> Result<Option<EngineNodeId>, HostError> {
            Ok((selector == "div").then_some(EngineNodeId(2)))
        }
        fn query_selector_all(
            &mut self,
            _node: EngineNodeId,
            selector: &str,
        ) -> Result<Vec<EngineNodeId>, HostError> {
            Ok((selector == "div").then_some(EngineNodeId(2)).into_iter().collect())
        }
        fn get_outer_html(&mut self, node: EngineNodeId) -> Result<String, HostError> {
            Ok(format!("<e{}>", node.0))
        }
        fn describe_node(
            &mut self,
            node: EngineNodeId,
            _depth: u32,
        ) -> Result<NodeDescription, HostError> {
            let mut found = None;
            fn walk(node: &NodeDescription, target: EngineNodeId, out: &mut Option<NodeDescription>) {
                if node.node == target {
                    *out = Some(node.clone());
                }
                for child in &node.children {
                    walk(child, target, out);
                }
            }
            walk(&document(), node, &mut found);
            found.ok_or(HostError::NotFound)
        }
        fn resolve_node(&mut self, backend: u64) -> Result<EngineNodeId, HostError> {
            Ok(EngineNodeId(backend))
        }
        fn set_attribute_value(
            &mut self,
            _node: EngineNodeId,
            _name: &str,
            _value: Option<&str>,
        ) -> Result<(), HostError> {
            Ok(())
        }
    }

    fn session() -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/", "A"),
            Box::new(Tree),
        )
    }

    #[test]
    fn get_document_assigns_dense_node_ids() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"DOM.getDocument","params":{"depth":-1}}"#)
            .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let root = outcome.result.get("root").unwrap();
        assert_eq!(root.get("nodeId").unwrap().as_u64(), Some(1));
        assert_eq!(root.get("nodeName").unwrap().as_str(), Some("HTML"));
        let body = root.get("children").unwrap().as_array().unwrap()[0].get("children").unwrap();
        assert!(body.as_array().unwrap()[0].get("nodeId").unwrap().as_u64().is_some());
    }

    #[test]
    fn query_selector_returns_zero_when_missing() {
        let mut session = session();
        dispatch(
            &mut session,
            &CdpRequest::parse(r#"{"id":1,"method":"DOM.getDocument"}"#).unwrap(),
        )
        .unwrap();
        let root_id = 1;
        let request = CdpRequest::parse(&format!(
            r#"{{"id":2,"method":"DOM.querySelector","params":{{"nodeId":{root_id},"selector":"span"}}}}"#
        ))
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.result.get("nodeId").unwrap().as_u64(), Some(0));
    }

    #[test]
    fn get_outer_html_and_backend_resolution() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"DOM.getOuterHTML","params":{"backendNodeId":2}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.result.get("outerHTML").unwrap().as_str(), Some("<e2>"));
    }

    #[test]
    fn missing_node_is_a_server_error() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"DOM.getOuterHTML","params":{"nodeId":9}}"#)
                .unwrap();
        assert_eq!(
            dispatch(&mut session, &request).unwrap_err().code,
            crate::protocol::SERVER_ERROR
        );
    }
}
