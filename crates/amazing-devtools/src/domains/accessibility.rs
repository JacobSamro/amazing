//! The `Accessibility` domain: the CDP accessibility tree.
//!
//! The engine builds its own accessibility tree (for this project, the a11y
//! lane's `amazing-a11y::AxTree`); this crate stays independent of that crate
//! and only defines the CDP-shaped [`crate::host::AxNode`] seam, matching the
//! pattern used by every other domain here.

use super::object;
use crate::host::{AxNode, EngineNodeId};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles an `Accessibility.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Accessibility.enable" => {
            session.enabled.accessibility = true;
            session
                .dev
                .accessibility_enabled(true)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Accessibility.disable" => {
            session.enabled.accessibility = false;
            session
                .dev
                .accessibility_enabled(false)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Accessibility.getFullAXTree" => {
            let depth = optional_depth(request)?;
            let nodes = session.dev.get_full_ax_tree(depth).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(nodes_result(&nodes)))
        }
        "Accessibility.getPartialAXTree" => {
            let node = resolve_node(session, request)?;
            let fetch_relatives = request.optional_bool("fetchRelatives", true)?;
            let nodes = session
                .dev
                .get_partial_ax_tree(node, fetch_relatives)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(nodes_result(&nodes)))
        }
        "Accessibility.getChildAXNodes" => {
            let id = request.string_param("id")?;
            let nodes = session
                .dev
                .get_child_ax_nodes(id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(nodes_result(&nodes)))
        }
        "Accessibility.queryAXTree" => {
            let node = optional_resolve_node(session, request)?;
            let accessible_name = request.optional_string("accessibleName")?;
            let role = request.optional_string("role")?;
            let nodes = session
                .dev
                .query_ax_tree(node, accessible_name, role)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(nodes_result(&nodes)))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Serializes a CDP `Accessibility.AXValue` for a plain string value.
fn ax_value(kind: &str, value: &str) -> Json {
    object(vec![
        ("type", Json::String(kind.into())),
        ("value", Json::String(value.into())),
    ])
}

/// Serializes one `Accessibility.AXNode`.
fn ax_node_json(node: &AxNode) -> Json {
    let mut fields = vec![
        ("nodeId", Json::String(node.ax_node_id.clone())),
        ("ignored", Json::Bool(node.ignored)),
    ];
    if let Some(role) = &node.role {
        fields.push(("role", ax_value("role", role)));
    }
    if let Some(name) = &node.name {
        fields.push(("name", ax_value("computedString", name)));
    }
    if let Some(description) = &node.description {
        fields.push(("description", ax_value("computedString", description)));
    }
    if let Some(value) = &node.value {
        fields.push(("value", ax_value("computedString", value)));
    }
    if !node.child_ids.is_empty() {
        fields.push((
            "childIds",
            Json::Array(node.child_ids.iter().cloned().map(Json::String).collect()),
        ));
    }
    if let Some(engine_node) = node.node {
        fields.push(("backendDOMNodeId", Json::number(engine_node.0 as f64)));
    }
    object(fields)
}

/// Serializes a list of `AXNode`s as the `nodes` result field.
fn nodes_result(nodes: &[AxNode]) -> Json {
    object(vec![(
        "nodes",
        Json::Array(nodes.iter().map(ax_node_json).collect()),
    )])
}

/// Builds the `Accessibility.nodesUpdated` event.
pub fn nodes_updated(nodes: &[AxNode]) -> Json {
    crate::protocol::event(
        "Accessibility.nodesUpdated",
        object(vec![(
            "nodes",
            Json::Array(nodes.iter().map(ax_node_json).collect()),
        )]),
    )
}

/// Reads the CDP `depth` parameter: `None`/absent/negative means the whole
/// tree, matching `Accessibility.getFullAXTree`'s `-1` convention.
fn optional_depth(request: &CdpRequest) -> Result<Option<u32>, CdpError> {
    match request.params.get("depth") {
        None | Some(Json::Null) => Ok(None),
        Some(value) => {
            let n = value
                .as_i64()
                .ok_or_else(|| CdpError::invalid_params("depth must be an integer"))?;
            Ok(if n < 0 { None } else { Some(n as u32) })
        }
    }
}

/// Resolves a required `nodeId`/`backendNodeId` pair to engine identity.
fn resolve_node(session: &mut SessionState, request: &CdpRequest) -> Result<EngineNodeId, CdpError> {
    optional_resolve_node(session, request)?.ok_or_else(|| {
        CdpError::invalid_params("Either nodeId or backendNodeId must be specified")
    })
}

/// Resolves an optional `nodeId`/`backendNodeId` pair to engine identity.
fn optional_resolve_node(
    session: &mut SessionState,
    request: &CdpRequest,
) -> Result<Option<EngineNodeId>, CdpError> {
    if let Some(node_id) = request.optional_u64("nodeId")? {
        if node_id == 0 {
            return Err(CdpError::server("Could not find node with given id"));
        }
        return session
            .nodes
            .engine(node_id)
            .map(Some)
            .ok_or_else(|| CdpError::server("Could not find node with given id"));
    }
    if let Some(backend) = request.optional_u64("backendNodeId")? {
        return session
            .dev
            .resolve_node(backend)
            .map(Some)
            .map_err(CdpError::from);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Tree;

    impl DevToolsSession for Tree {
        fn resolve_node(&mut self, backend: u64) -> Result<EngineNodeId, HostError> {
            Ok(EngineNodeId(backend))
        }
        fn get_full_ax_tree(&mut self, _depth: Option<u32>) -> Result<Vec<AxNode>, HostError> {
            Ok(vec![AxNode {
                ax_node_id: "ax-1".into(),
                role: Some("WebArea".into()),
                name: Some("Test".into()),
                node: Some(EngineNodeId(1)),
                child_ids: vec!["ax-2".into()],
                ..AxNode::default()
            }])
        }
        fn get_partial_ax_tree(
            &mut self,
            node: EngineNodeId,
            _fetch_relatives: bool,
        ) -> Result<Vec<AxNode>, HostError> {
            Ok(vec![AxNode {
                ax_node_id: "ax-2".into(),
                role: Some("generic".into()),
                node: Some(node),
                ..AxNode::default()
            }])
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
    fn enable_sets_the_flag_and_calls_the_host() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Accessibility.enable"}"#).unwrap();
        dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.accessibility);
    }

    #[test]
    fn get_full_ax_tree_serializes_role_name_and_children() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"Accessibility.getFullAXTree"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let nodes = outcome.result.get("nodes").unwrap().as_array().unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].get("nodeId").unwrap().as_str(), Some("ax-1"));
        assert_eq!(
            nodes[0].get("role").unwrap().get("value").unwrap().as_str(),
            Some("WebArea")
        );
        assert_eq!(
            nodes[0].get("name").unwrap().get("value").unwrap().as_str(),
            Some("Test")
        );
        assert_eq!(
            nodes[0].get("backendDOMNodeId").unwrap().as_f64(),
            Some(1.0)
        );
        assert_eq!(
            nodes[0].get("childIds").unwrap().as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn get_partial_ax_tree_resolves_backend_node_id() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Accessibility.getPartialAXTree","params":{"backendNodeId":7}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let nodes = outcome.result.get("nodes").unwrap().as_array().unwrap();
        assert_eq!(
            nodes[0].get("backendDOMNodeId").unwrap().as_f64(),
            Some(7.0)
        );
    }

    #[test]
    fn get_partial_ax_tree_without_a_node_is_invalid_params() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"Accessibility.getPartialAXTree"}"#).unwrap();
        let error = dispatch(&mut session, &request).unwrap_err();
        assert_eq!(error.code, crate::protocol::INVALID_PARAMS);
    }
}
