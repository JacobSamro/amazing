//! The `CSS` domain: computed styles and matched rules.

use super::{empty_array, object, style_origin};
use crate::host::{MatchedRule, MediaQuery};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `CSS.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "CSS.enable" => {
            session.enabled.css = true;
            Ok(DispatchOutcome::empty())
        }
        "CSS.disable" => {
            session.enabled.css = false;
            Ok(DispatchOutcome::empty())
        }
        "CSS.getComputedStyleForNode" => {
            let node = node_from_request(session, request)?;
            let properties = session
                .dev
                .get_computed_style_for_node(node)
                .map_err(CdpError::from)?;
            let computed: Vec<Json> = properties
                .iter()
                .map(|(name, value)| {
                    object(vec![
                        ("name", Json::String(name.clone())),
                        ("value", Json::String(value.clone())),
                    ])
                })
                .collect();
            Ok(DispatchOutcome::result(object(vec![(
                "computedStyle",
                Json::Array(computed),
            )])))
        }
        "CSS.getMatchedStylesForNode" => {
            let node = node_from_request(session, request)?;
            let rules = session
                .dev
                .get_matched_styles_for_node(node)
                .map_err(CdpError::from)?;
            let matched: Vec<Json> = rules.iter().map(matched_rule).collect();
            Ok(DispatchOutcome::result(object(vec![
                ("matchedCSSRules", Json::Array(matched)),
                ("inherited", empty_array()),
                ("pseudoElements", empty_array()),
            ])))
        }
        "CSS.getInlineStylesForNode" => {
            let node = node_from_request(session, request)?;
            let rules = session
                .dev
                .get_matched_styles_for_node(node)
                .map_err(CdpError::from)?;
            let inline: Vec<Json> = rules
                .iter()
                .filter(|rule| rule.origin == crate::host::StyleOrigin::Inspector)
                .flat_map(|rule| rule.properties.iter())
                .map(|(name, value)| {
                    object(vec![
                        ("name", Json::String(name.clone())),
                        ("value", Json::String(value.clone())),
                        ("disabled", Json::Bool(false)),
                        ("implicit", Json::Bool(false)),
                    ])
                })
                .collect();
            Ok(DispatchOutcome::result(object(vec![
                (
                    "inlineStyle",
                    object(vec![
                        ("cssProperties", Json::Array(inline)),
                        ("shorthandEntries", empty_array()),
                    ]),
                ),
                (
                    "attributesStyle",
                    object(vec![
                        ("cssProperties", empty_array()),
                        ("shorthandEntries", empty_array()),
                    ]),
                ),
            ])))
        }
        "CSS.getStyleSheetText" => {
            let style_sheet_id = request.string_param("styleSheetId")?;
            let text = session
                .dev
                .get_style_sheet_text(style_sheet_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "text",
                Json::String(text.unwrap_or_default()),
            )])))
        }
        "CSS.setStyleSheetText" => {
            let style_sheet_id = request.string_param("styleSheetId")?;
            let text = request.string_param("text")?;
            session
                .dev
                .set_style_sheet_text(style_sheet_id, text)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "text",
                Json::String(text.to_string()),
            )])))
        }
        "CSS.getMediaQueries" => {
            let queries = session.dev.get_media_queries().map_err(CdpError::from)?;
            let medias: Vec<Json> = queries.iter().map(media_query).collect();
            Ok(DispatchOutcome::result(object(vec![(
                "medias",
                Json::Array(medias),
            )])))
        }
        "CSS.getBackgroundColors" => {
            let node = node_from_request(session, request)?;
            let colors = session
                .dev
                .get_background_colors(node)
                .map_err(CdpError::from)?;
            let colors: Vec<Json> = colors
                .into_iter()
                .map(|color| match color {
                    Some(value) => Json::String(value),
                    None => Json::Null,
                })
                .collect();
            Ok(DispatchOutcome::result(object(vec![
                ("colors", Json::Array(colors)),
                ("computedFontSize", Json::String(String::new())),
                ("computedFontWeight", Json::String(String::new())),
            ])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn media_query(query: &MediaQuery) -> Json {
    let mut fields = vec![
        ("text", Json::String(query.text.clone())),
        ("source", Json::String(query.source.clone())),
    ];
    if let Some(url) = &query.source_url {
        fields.push(("sourceURL", Json::String(url.clone())));
    }
    object(fields)
}

fn node_from_request(
    session: &SessionState,
    request: &CdpRequest,
) -> Result<crate::host::EngineNodeId, CdpError> {
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

fn matched_rule(rule: &MatchedRule) -> Json {
    let properties: Vec<Json> = rule
        .properties
        .iter()
        .map(|(name, value)| {
            object(vec![
                ("name", Json::String(name.clone())),
                ("value", Json::String(value.clone())),
                ("disabled", Json::Bool(false)),
                ("implicit", Json::Bool(false)),
            ])
        })
        .collect();
    object(vec![
        (
            "rule",
            object(vec![
                (
                    "selectorList",
                    object(vec![
                        (
                            "selectors",
                            Json::Array(vec![object(vec![(
                                "text",
                                Json::String(rule.selector.clone()),
                            )])]),
                        ),
                        ("text", Json::String(rule.selector.clone())),
                    ]),
                ),
                ("origin", Json::String(style_origin(rule.origin).to_string())),
                (
                    "style",
                    object(vec![
                        ("cssProperties", Json::Array(properties)),
                        ("shorthandEntries", empty_array()),
                    ]),
                ),
            ]),
        ),
        ("matchingSelectors", Json::Array(vec![Json::number(0.0)])),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, EngineNodeId, HostError, MatchedRule, StyleOrigin};
    use crate::session::SessionState;

    struct Styles;

    impl DevToolsSession for Styles {
        fn get_document(&mut self, _depth: u32) -> Result<crate::host::NodeDescription, HostError> {
            Ok(crate::host::NodeDescription::element(EngineNodeId(1), "div"))
        }
        fn get_computed_style_for_node(
            &mut self,
            _node: EngineNodeId,
        ) -> Result<Vec<(String, String)>, HostError> {
            Ok(vec![("color".into(), "rgb(1, 2, 3)".into())])
        }
        fn get_matched_styles_for_node(
            &mut self,
            _node: EngineNodeId,
        ) -> Result<Vec<MatchedRule>, HostError> {
            Ok(vec![MatchedRule {
                selector: "div".into(),
                origin: StyleOrigin::Author,
                properties: vec![("color".into(), "red".into())],
            }])
        }
    }

    fn session_with_document() -> SessionState {
        let mut session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Styles),
        );
        session.nodes.id(EngineNodeId(1));
        session
    }

    #[test]
    fn computes_and_matches_styles() {
        let mut session = session_with_document();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"CSS.getComputedStyleForNode","params":{"nodeId":1}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let first = &outcome.result.get("computedStyle").unwrap().as_array().unwrap()[0];
        assert_eq!(first.get("value").unwrap().as_str(), Some("rgb(1, 2, 3)"));

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"CSS.getMatchedStylesForNode","params":{"nodeId":1}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let rule = &outcome.result.get("matchedCSSRules").unwrap().as_array().unwrap()[0];
        assert_eq!(
            rule.get("rule").unwrap().get("origin").unwrap().as_str(),
            Some("author")
        );
    }
}
