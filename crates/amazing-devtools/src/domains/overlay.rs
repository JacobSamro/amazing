//! The `Overlay` domain: inspector node/rect/quad highlighting.
//!
//! The crate parses CDP's `HighlightConfig` and hit-test node references; the
//! engine renders the highlight. The `setShow*` toggles are accepted and
//! validated but are engine hints, not protocol state this crate owns.

use crate::host::{EngineNodeId, HighlightConfig, Quad, Rect, Rgba};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles an `Overlay.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Overlay.enable" => {
            session.dev.overlay_enabled(true).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.disable" => {
            session.dev.overlay_enabled(false).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.setInspectMode" => {
            let mode = request.string_param("mode")?;
            let config = optional_config(request, "highlightOptions")?;
            session
                .dev
                .set_inspect_mode(mode, &config)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.highlightNode" => {
            let config = required_config(request)?;
            let node = resolve_node(session, request)?;
            session
                .dev
                .highlight_node(node, &config)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.highlightRect" => {
            let rect = Rect {
                x: request.number_param("x")?,
                y: request.number_param("y")?,
                width: request.number_param("width")?,
                height: request.number_param("height")?,
                scale: 1.0,
            };
            let config = optional_config(request, "highlightConfig")?;
            session
                .dev
                .highlight_rect(rect, &config)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.highlightQuad" => {
            let quad = parse_quad(request)?;
            let config = optional_config(request, "highlightConfig")?;
            session
                .dev
                .highlight_quad(quad, &config)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.hideHighlight" => {
            session.dev.hide_highlight().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Overlay.setShowDebugBorders"
        | "Overlay.setShowFPSCounter"
        | "Overlay.setShowPaintRects"
        | "Overlay.setShowScrollBottleneckRects"
        | "Overlay.setShowHitTestBorders"
        | "Overlay.setShowAdHighlights"
        | "Overlay.setShowWebVitals"
        | "Overlay.setShowViewportSizeOnResize" => {
            // Validate the flag; the value is an engine hint this crate does
            // not retain. `setShowPaintRects` uses `result`, the rest `show`.
            let _ = request.optional_bool("show", false)?;
            let _ = request.optional_bool("result", false)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

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

fn required_config(request: &CdpRequest) -> Result<HighlightConfig, CdpError> {
    let value = request
        .params
        .get("highlightConfig")
        .ok_or_else(|| CdpError::invalid_params("missing highlightConfig"))?;
    parse_config(value)
}

fn optional_config(request: &CdpRequest, name: &str) -> Result<HighlightConfig, CdpError> {
    match request.params.get(name) {
        Some(value) if value != &Json::Null => parse_config(value),
        _ => Ok(HighlightConfig::default()),
    }
}

fn parse_config(value: &Json) -> Result<HighlightConfig, CdpError> {
    Ok(HighlightConfig {
        content_color: color(value, "contentColor")?,
        padding_color: color(value, "paddingColor")?,
        border_color: color(value, "borderColor")?,
        margin_color: color(value, "marginColor")?,
        event_target_color: color(value, "eventTargetColor")?,
        shape_color: color(value, "shapeColor")?,
        show_info: bool_field(value, "showInfo")?,
        show_styles: bool_field(value, "showStyles")?,
        show_rulers: bool_field(value, "showRulers")?,
        show_extension_lines: bool_field(value, "showExtensionLines")?,
    })
}

fn bool_field(value: &Json, name: &str) -> Result<bool, CdpError> {
    match value.get(name) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(CdpError::invalid_params(format!(
            "highlightConfig.{name} must be a boolean"
        ))),
    }
}

fn color(value: &Json, name: &str) -> Result<Option<Rgba>, CdpError> {
    let Some(raw) = value.get(name) else {
        return Ok(None);
    };
    if raw == &Json::Null {
        return Ok(None);
    }
    let channel = |field: &str| -> Result<u8, CdpError> {
        raw.get(field)
            .and_then(Json::as_i64)
            .map(|value| value.clamp(0, 255) as u8)
            .ok_or_else(|| CdpError::invalid_params(format!("{name}.{field} must be an integer")))
    };
    let alpha = raw.get("a").and_then(Json::as_f64).unwrap_or(1.0);
    Ok(Some(Rgba {
        r: channel("r")?,
        g: channel("g")?,
        b: channel("b")?,
        alpha,
    }))
}

fn parse_quad(request: &CdpRequest) -> Result<Quad, CdpError> {
    let raw = request
        .params
        .get("quad")
        .and_then(Json::as_array)
        .ok_or_else(|| CdpError::invalid_params("quad must be an array"))?;
    if raw.len() != 8 {
        return Err(CdpError::invalid_params("quad must contain 8 numbers"));
    }
    let mut values = [0.0f64; 8];
    for (index, item) in raw.iter().enumerate() {
        values[index] = item
            .as_f64()
            .ok_or_else(|| CdpError::invalid_params("quad entries must be numbers"))?;
    }
    Ok(Quad {
        points: [
            (values[0], values[1]),
            (values[2], values[3]),
            (values[4], values[5]),
            (values[6], values[7]),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        enabled: Option<bool>,
        mode: Option<String>,
        node: Option<EngineNodeId>,
        rect: Option<Rect>,
        quad: Option<Quad>,
        config: Option<HighlightConfig>,
        hidden: Option<bool>,
    }

    struct Overlay(Arc<Mutex<Recording>>);

    impl DevToolsSession for Overlay {
        fn overlay_enabled(&mut self, enabled: bool) -> Result<(), HostError> {
            self.0.lock().unwrap().enabled = Some(enabled);
            Ok(())
        }
        fn set_inspect_mode(
            &mut self,
            mode: &str,
            _config: &HighlightConfig,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().mode = Some(mode.to_string());
            Ok(())
        }
        fn highlight_node(
            &mut self,
            node: EngineNodeId,
            config: &HighlightConfig,
        ) -> Result<(), HostError> {
            let mut recording = self.0.lock().unwrap();
            recording.node = Some(node);
            recording.config = Some(config.clone());
            Ok(())
        }
        fn highlight_rect(
            &mut self,
            rect: Rect,
            _config: &HighlightConfig,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().rect = Some(rect);
            Ok(())
        }
        fn highlight_quad(
            &mut self,
            quad: Quad,
            _config: &HighlightConfig,
        ) -> Result<(), HostError> {
            self.0.lock().unwrap().quad = Some(quad);
            Ok(())
        }
        fn hide_highlight(&mut self) -> Result<(), HostError> {
            self.0.lock().unwrap().hidden = Some(true);
            Ok(())
        }
    }

    fn session() -> (SessionState, Arc<Mutex<Recording>>) {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let mut session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Overlay(Arc::clone(&recording))),
        );
        session.nodes.id(EngineNodeId(4));
        (session, recording)
    }

    #[test]
    fn highlights_nodes_rects_and_quads() {
        let (mut session, recording) = session();

        let request = CdpRequest::parse(r#"{"id":1,"method":"Overlay.enable"}"#).unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"Overlay.highlightNode","params":{"nodeId":1,"highlightConfig":{"contentColor":{"r":1,"g":2,"b":3,"a":0.5},"showInfo":true}}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":3,"method":"Overlay.highlightRect","params":{"x":1,"y":2,"width":30,"height":40}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(
            r#"{"id":4,"method":"Overlay.highlightQuad","params":{"quad":[0,0,10,0,10,10,0,10]}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let request = CdpRequest::parse(r#"{"id":5,"method":"Overlay.hideHighlight"}"#).unwrap();
        dispatch(&mut session, &request).unwrap();

        let recording = recording.lock().unwrap();
        assert_eq!(recording.enabled, Some(true));
        assert_eq!(recording.node, Some(EngineNodeId(4)));
        let config = recording.config.as_ref().unwrap();
        assert_eq!(
            config.content_color,
            Some(Rgba {
                r: 1,
                g: 2,
                b: 3,
                alpha: 0.5
            })
        );
        assert!(config.show_info);
        assert_eq!(recording.rect.unwrap().width, 30.0);
        assert_eq!(recording.quad.unwrap().points[2], (10.0, 10.0));
        assert_eq!(recording.hidden, Some(true));
    }

    #[test]
    fn malformed_quad_is_invalid_params() {
        let (mut session, _) = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Overlay.highlightQuad","params":{"quad":[0,0,1,1]}}"#,
        )
        .unwrap();
        assert_eq!(
            dispatch(&mut session, &request).unwrap_err().code,
            crate::protocol::INVALID_PARAMS
        );
    }
}
