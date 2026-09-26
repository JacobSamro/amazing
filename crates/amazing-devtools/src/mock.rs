//! An in-memory, scriptable [`DevToolsHost`] for tests and driver prototyping.
//!
//! `MockHost` implements the full engine seam over a small fixed DOM and a set
//! of caller-controlled event queues. A driver author can point a
//! [`crate::server::CdpServer`] at it and drive a real CDP client end to end
//! before any engine wiring exists, and the crate's own integration tests use
//! it as the scripted engine.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::host::{
    BoxModel, ConsoleEvent, DeviceMetrics, DevToolsHost, DevToolsSession, EngineNodeId,
    EvaluateParams, EvaluateResult, ExceptionDetails, FetchDecision, FetchPattern, FrameInfo,
    FrameResourceTree, FrameTree, HighlightConfig, HostError, HostInfo, InterceptedRequest,
    KeyEvent, LayoutMetrics, LifecycleEvent, LogEntry, MatchedRule, MediaFeature, MediaQuery,
    MouseEvent, NetworkFinishedEvent, NetworkRequestEvent, NetworkResponseEvent, NodeDescription,
    PropertyDescriptor, Quad, Rect, RemoteObject, ResourceContent, ResourceInfo, Screenshot,
    ScreenshotParams, SecurityState, TargetEvent, TargetEventKind, TargetInfo, TouchEvent,
    Viewport,
};
use crate::json::Json;

/// Mutable script shared by a [`MockHost`] and every session it hands out.
pub struct MockScript {
    /// Targets returned by `Target.getTargets` and `/json/list`.
    pub targets: Vec<TargetInfo>,
    /// Value returned by `Runtime.evaluate` when no exception is scripted.
    pub eval_value: Json,
    /// When set, `Runtime.evaluate` returns this exception.
    pub eval_exception: Option<ExceptionDetails>,
    /// Bytes returned by `Page.captureScreenshot`.
    pub screenshot: Vec<u8>,
    /// Computed style returned by `CSS.getComputedStyleForNode`.
    pub computed_style: Vec<(String, String)>,
    /// Matched rules returned by `CSS.getMatchedStylesForNode`.
    pub matched_rules: Vec<MatchedRule>,
    /// Whether `Page.navigate`/`Page.reload` queue lifecycle events.
    pub auto_lifecycle: bool,
    /// Every URL passed to `Page.navigate`, in order.
    pub navigations: Vec<String>,
    /// Every expression passed to `Runtime.evaluate`, in order.
    pub evaluations: Vec<String>,
    /// Whether `reload` was asked to ignore the cache, in order.
    pub reloads: Vec<bool>,
    /// Attribute writes recorded by `DOM.setAttributeValue`.
    pub mutations: Vec<(EngineNodeId, String, Option<String>)>,
    /// Text-node writes recorded by `DOM.setNodeValue`.
    pub node_values: Vec<(EngineNodeId, String)>,
    /// Mouse events recorded by `Input.dispatchMouseEvent`.
    pub mouse_events: Vec<MouseEvent>,
    /// Keyboard events recorded by `Input.dispatchKeyEvent`.
    pub key_events: Vec<KeyEvent>,
    /// Touch events recorded by `Input.dispatchTouchEvent`.
    pub touch_events: Vec<TouchEvent>,
    /// Text inserted by `Input.insertText`.
    pub inserted_text: Vec<String>,
    /// Last `Input.setIgnoreInputEvents` value.
    pub ignore_input: Option<bool>,
    /// Node returned by `DOM.getNodeForLocation`.
    pub hit_test: Option<EngineNodeId>,
    /// Fetch interception state from the last `set_fetch_enabled`.
    pub fetch_enabled: bool,
    /// Fetch patterns from the last `set_fetch_enabled`.
    pub fetch_patterns: Vec<FetchPattern>,
    /// Whether the client asked to handle auth challenges.
    pub fetch_handle_auth: bool,
    /// Requests paused for Fetch interception.
    pub intercepted: VecDeque<InterceptedRequest>,
    /// Decisions recorded by `resolve_intercepted_request`.
    pub fetch_decisions: Vec<(String, FetchDecision)>,
    /// Response bodies for `getResponseBody`: (request_id, bytes, mime).
    pub response_bodies: Vec<(String, Vec<u8>, String)>,
    /// Request post data for `getRequestPostData`: (request_id, text).
    pub request_post_data: Vec<(String, String)>,
    /// Last `Emulation.setDeviceMetricsOverride` value.
    pub device_metrics: Option<DeviceMetrics>,
    /// Last `Emulation.setUserAgentOverride` value.
    pub user_agent_override: Option<String>,
    /// Last `Emulation.setGeolocationOverride` (`Some(None)` clears it).
    pub geolocation_override: Option<Option<(f64, f64, f64)>>,
    /// Last `Emulation.setTimezoneOverride` value.
    pub timezone_override: Option<String>,
    /// Last `Emulation.setLocaleOverride` (`Some(None)` clears it).
    pub locale_override: Option<Option<String>>,
    /// Last `Emulation.setEmulatedMedia` value.
    pub emulated_media: Option<(Option<String>, Vec<MediaFeature>)>,
    /// Last `Emulation.setScrollbarsHidden` value.
    pub scrollbars_hidden: Option<bool>,
    /// Last `Emulation.setTouchEmulationEnabled` value.
    pub touch_emulation: Option<(bool, Option<u32>)>,
    /// Last `Emulation.setCPUThrottlingRate` value.
    pub cpu_throttling_rate: Option<f64>,
    /// Last `Emulation.setScriptExecutionDisabled` value.
    pub script_execution_disabled: Option<bool>,
    /// Last `Overlay.enable`/`disable` state.
    pub overlay_enabled: Option<bool>,
    /// Last `Overlay.setInspectMode` value.
    pub inspect_mode: Option<String>,
    /// Last `Overlay.highlightNode` node.
    pub highlighted_node: Option<EngineNodeId>,
    /// Last `Overlay.highlightRect` rectangle.
    pub highlighted_rect: Option<Rect>,
    /// Last `Overlay.highlightQuad` quad.
    pub highlighted_quad: Option<Quad>,
    /// Last `Overlay.highlight*` configuration.
    pub highlight_config: Option<HighlightConfig>,
    /// Set by `Overlay.hideHighlight`.
    pub highlight_hidden: Option<bool>,
    /// Metrics returned by `Performance.getMetrics`.
    pub performance_metrics: Vec<(String, f64)>,
    /// The target's current security state.
    pub current_security_state: SecurityState,
    /// Pending `Security.securityStateChanged` changes.
    pub security_states: VecDeque<SecurityState>,
    /// Last `Security.setIgnoreCertificateErrors` value.
    pub ignore_certificate_errors: Option<bool>,
    /// `Security.handleCertificateError` calls: (event_id, action).
    pub certificate_errors: Vec<(i64, i32)>,
    /// Layout metrics returned by `Page.getLayoutMetrics`.
    pub layout_metrics: LayoutMetrics,
    /// Box model returned by `DOM.getBoxModel` / `DOM.getContentQuads`.
    pub box_model: BoxModel,
    /// Style sheet text returned by `CSS.getStyleSheetText`.
    pub style_sheet_text: Option<String>,
    /// Media queries returned by `CSS.getMediaQueries`.
    pub media_queries: Vec<MediaQuery>,
    /// Background colors returned by `CSS.getBackgroundColors`.
    pub background_colors: Vec<Option<String>>,
    /// Resources advertised in `Page.getResourceTree`.
    pub resources: Vec<ResourceInfo>,
    /// Resource bodies for `Page.getResourceContent`: (url, bytes, mime).
    pub resource_bodies: Vec<(String, Vec<u8>, String)>,

    /// Pending `Page.lifecycleEvent` events.
    pub lifecycle: VecDeque<LifecycleEvent>,
    /// Pending `Runtime.consoleAPICalled` events.
    pub console: VecDeque<ConsoleEvent>,
    /// Pending `Network.requestWillBeSent` events.
    pub requests: VecDeque<NetworkRequestEvent>,
    /// Pending `Network.responseReceived` events.
    pub responses: VecDeque<NetworkResponseEvent>,
    /// Pending `Network.loadingFinished` events.
    pub finished: VecDeque<NetworkFinishedEvent>,
    /// Pending `Log.entryAdded` events.
    pub logs: VecDeque<LogEntry>,
    /// Pending `Target.*` lifecycle events, polled by the browser endpoint.
    pub target_events: VecDeque<TargetEvent>,
}

impl Default for MockScript {
    fn default() -> Self {
        Self {
            targets: vec![TargetInfo::page("page-1", "about:blank", "Mock Page")],
            eval_value: Json::Number(42.0),
            eval_exception: None,
            screenshot: b"mock-png".to_vec(),
            computed_style: vec![
                ("display".into(), "block".into()),
                ("color".into(), "rgb(0, 0, 0)".into()),
            ],
            matched_rules: Vec::new(),
            auto_lifecycle: true,
            navigations: Vec::new(),
            evaluations: Vec::new(),
            reloads: Vec::new(),
            mutations: Vec::new(),
            node_values: Vec::new(),
            mouse_events: Vec::new(),
            key_events: Vec::new(),
            touch_events: Vec::new(),
            inserted_text: Vec::new(),
            ignore_input: None,
            hit_test: Some(EngineNodeId(4)),
            fetch_enabled: false,
            fetch_patterns: Vec::new(),
            fetch_handle_auth: false,
            intercepted: VecDeque::new(),
            fetch_decisions: Vec::new(),
            response_bodies: Vec::new(),
            request_post_data: Vec::new(),
            device_metrics: None,
            user_agent_override: None,
            geolocation_override: None,
            timezone_override: None,
            locale_override: None,
            emulated_media: None,
            scrollbars_hidden: None,
            touch_emulation: None,
            cpu_throttling_rate: None,
            script_execution_disabled: None,
            overlay_enabled: None,
            inspect_mode: None,
            highlighted_node: None,
            highlighted_rect: None,
            highlighted_quad: None,
            highlight_config: None,
            highlight_hidden: None,
            performance_metrics: vec![("Frames".into(), 12.0), ("Nodes".into(), 340.0)],
            current_security_state: SecurityState::default(),
            security_states: VecDeque::new(),
            ignore_certificate_errors: None,
            certificate_errors: Vec::new(),
            layout_metrics: LayoutMetrics {
                layout_viewport: Viewport {
                    page_x: 0.0,
                    page_y: 0.0,
                    client_width: 800.0,
                    client_height: 600.0,
                },
                visual_viewport: Viewport {
                    page_x: 0.0,
                    page_y: 0.0,
                    client_width: 800.0,
                    client_height: 600.0,
                },
                content_width: 800.0,
                content_height: 1200.0,
            },
            box_model: BoxModel::from_rect(0.0, 0.0, 100.0, 50.0),
            style_sheet_text: Some("body { color: red }".into()),
            media_queries: vec![MediaQuery {
                text: "(min-width: 600px)".into(),
                source: "mediaRule".into(),
                source_url: None,
            }],
            background_colors: vec![Some("rgb(255, 255, 255)".into()), None],
            resources: vec![ResourceInfo {
                url: "https://example.com/".into(),
                mime_type: "text/html".into(),
                resource_type: "Document".into(),
                content_size: Some(5),
                last_modified: None,
                cached: false,
            }],
            resource_bodies: vec![(
                "https://example.com/".into(),
                b"<html>".to_vec(),
                "text/html".into(),
            )],
            lifecycle: VecDeque::new(),
            console: VecDeque::new(),
            requests: VecDeque::new(),
            responses: VecDeque::new(),
            finished: VecDeque::new(),
            logs: VecDeque::new(),
            target_events: VecDeque::new(),
        }
    }
}

impl MockScript {
    /// Queues a `Page.lifecycleEvent`.
    pub fn push_lifecycle(&mut self, frame_id: &str, name: &str) {
        self.lifecycle.push_back(LifecycleEvent {
            frame_id: frame_id.to_string(),
            name: name.to_string(),
            loader_id: String::new(),
            timestamp: 1.0,
        });
    }

    /// Queues a console message.
    pub fn push_console(&mut self, kind: &str, args: Vec<RemoteObject>) {
        self.console.push_back(ConsoleEvent {
            kind: kind.to_string(),
            args,
            execution_context_id: 1,
            timestamp: 1.0,
        });
    }

    /// Queues a network request/response/finished triple with a shared id.
    pub fn push_network(&mut self, request_id: &str, url: &str, status: u16) {
        self.requests.push_back(NetworkRequestEvent {
            request_id: request_id.to_string(),
            url: url.to_string(),
            method: "GET".into(),
            headers: vec![("Accept".into(), "*/*".into())],
            post_data: None,
            resource_type: "Document".into(),
            frame_id: "F1".into(),
            loader_id: String::new(),
            timestamp: 1.0,
            wall_time: 2.0,
        });
        self.responses.push_back(NetworkResponseEvent {
            request_id: request_id.to_string(),
            url: url.to_string(),
            status,
            status_text: if status == 200 { "OK" } else { "" }.into(),
            mime_type: "text/html".into(),
            headers: vec![("content-type".into(), "text/html".into())],
            encoded_data_length: 128,
            from_disk_cache: false,
            resource_type: "Document".into(),
            timestamp: 1.1,
        });
        self.finished.push_back(NetworkFinishedEvent {
            request_id: request_id.to_string(),
            encoded_data_length: 128,
            decoded_body_length: 200,
            timestamp: 1.2,
        });
    }

    /// Queues a log entry.
    pub fn push_log(&mut self, source: &str, level: &str, text: &str) {
        self.logs.push_back(LogEntry {
            source: source.to_string(),
            level: level.to_string(),
            text: text.to_string(),
            timestamp: 1.0,
            url: None,
            line_number: None,
            stack_trace: None,
        });
    }

    /// Queues a `Target.*` lifecycle event, polled by the browser endpoint.
    pub fn push_target_event(&mut self, kind: TargetEventKind, target: TargetInfo) {
        self.target_events.push_back(TargetEvent {
            kind,
            target,
            session_id: None,
        });
    }

    /// Queues `Target.targetCreated` and adds the target to the host's list.
    pub fn push_target_created(&mut self, id: &str, url: &str, title: &str) {
        let target = TargetInfo::page(id, url, title);
        self.targets.push(target.clone());
        self.push_target_event(TargetEventKind::Created, target);
    }

    /// Queues `Target.targetDestroyed` and removes the target from the list.
    pub fn push_target_destroyed(&mut self, id: &str) {
        if let Some(index) = self.targets.iter().position(|target| target.id == id) {
            let target = self.targets.remove(index);
            self.push_target_event(TargetEventKind::Destroyed, target);
        }
    }

    /// Queues a request paused for Fetch interception.
    pub fn push_intercepted(&mut self, item: InterceptedRequest) {
        self.intercepted.push_back(item);
    }

    /// Registers a response body for `Network.getResponseBody`.
    pub fn set_response_body(&mut self, request_id: &str, bytes: Vec<u8>, mime: &str) {
        self.response_bodies
            .push((request_id.to_string(), bytes, mime.to_string()));
    }

    /// Registers request post data for `Network.getRequestPostData`.
    pub fn set_request_post_data(&mut self, request_id: &str, text: &str) {
        self.request_post_data
            .push((request_id.to_string(), text.to_string()));
    }

    /// Sets the current security state without queueing an event.
    pub fn set_current_security_state(&mut self, state: SecurityState) {
        self.current_security_state = state;
    }

    /// Queues a `Security.securityStateChanged` change.
    pub fn push_security_state(&mut self, state: SecurityState) {
        self.security_states.push_back(state);
    }
}

/// A [`DevToolsHost`] backed by a [`MockScript`].
#[derive(Clone)]
pub struct MockHost {
    script: Arc<Mutex<MockScript>>,
    info: Arc<Mutex<HostInfo>>,
}

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHost {
    /// Creates a host with one page target and default responses.
    pub fn new() -> Self {
        Self {
            script: Arc::new(Mutex::new(MockScript::default())),
            info: Arc::new(Mutex::new(HostInfo::default())),
        }
    }

    /// Creates a host with explicit targets.
    pub fn with_targets(targets: Vec<TargetInfo>) -> Self {
        let host = Self::new();
        host.script.lock().unwrap().targets = targets;
        host
    }

    /// The shared script, for queueing events and changing responses.
    pub fn script(&self) -> Arc<Mutex<MockScript>> {
        Arc::clone(&self.script)
    }

    /// Replaces the advertised product/version strings.
    pub fn set_info(&self, info: HostInfo) {
        *self.info.lock().unwrap() = info;
    }
}

impl DevToolsHost for MockHost {
    fn info(&self) -> HostInfo {
        self.info.lock().unwrap().clone()
    }

    fn list_targets(&self) -> Vec<TargetInfo> {
        self.script.lock().unwrap().targets.clone()
    }

    fn attach(&self, target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        let target = self
            .script
            .lock()
            .unwrap()
            .targets
            .iter()
            .find(|target| target.id == target_id)
            .cloned()
            .ok_or(HostError::NotFound)?;
        Ok(Box::new(MockSession {
            script: Arc::clone(&self.script),
            target,
        }))
    }

    fn drain_target_events(&self) -> Vec<TargetEvent> {
        self.script
            .lock()
            .unwrap()
            .target_events
            .drain(..)
            .collect()
    }
}

/// One attached mock page.
pub struct MockSession {
    script: Arc<Mutex<MockScript>>,
    target: TargetInfo,
}

impl MockSession {
    fn frame(&self, url: &str) -> FrameInfo {
        FrameInfo {
            id: "F1".into(),
            parent_id: None,
            url: url.to_string(),
            name: String::new(),
            security_origin: crate::domains::runtime::origin_of(url),
            mime_type: "text/html".into(),
        }
    }
}

/// Builds the fixed mock document used for DOM queries.
pub fn mock_document() -> NodeDescription {
    let text = NodeDescription::text(EngineNodeId(6), "Hello");
    let span = NodeDescription::element(EngineNodeId(5), "span").with_children(vec![text]);
    let main = NodeDescription::element(EngineNodeId(4), "div")
        .with_attributes(vec![
            ("id".into(), "main".into()),
            ("class".into(), "box".into()),
        ])
        .with_children(vec![span]);
    let body = NodeDescription::element(EngineNodeId(3), "body").with_children(vec![main]);
    let head = NodeDescription::element(EngineNodeId(2), "head");
    NodeDescription::element(EngineNodeId(1), "html").with_children(vec![head, body])
}

impl DevToolsSession for MockSession {
    fn target_id(&self) -> &str {
        &self.target.id
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        self.target.url = url.to_string();
        let frame = self.frame(url);
        let mut script = self.script.lock().unwrap();
        script.navigations.push(url.to_string());
        script.targets.iter_mut().for_each(|target| {
            if target.id == self.target.id {
                target.url = url.to_string();
            }
        });
        if script.auto_lifecycle {
            script.push_lifecycle(&frame.id, "DOMContentLoaded");
            script.push_lifecycle(&frame.id, "load");
        }
        Ok(frame)
    }

    fn reload(&mut self, ignore_cache: bool) -> Result<(), HostError> {
        let frame_id = "F1".to_string();
        let mut script = self.script.lock().unwrap();
        script.reloads.push(ignore_cache);
        if script.auto_lifecycle {
            script.push_lifecycle(&frame_id, "load");
        }
        Ok(())
    }

    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        let url = self.target.url.clone();
        Ok(FrameTree {
            frame: self.frame(&url),
            child_frames: Vec::new(),
        })
    }

    fn capture_screenshot(&mut self, _params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        let script = self.script.lock().unwrap();
        Ok(Screenshot {
            data: script.screenshot.clone(),
            format: crate::host::ScreenshotFormat::Png,
        })
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        let mut script = self.script.lock().unwrap();
        script.evaluations.push(params.expression.clone());
        if let Some(exception) = script.eval_exception.clone() {
            return Ok(EvaluateResult {
                result: RemoteObject::undefined(),
                exception: Some(exception),
            });
        }
        Ok(EvaluateResult::value(RemoteObject::object(
            script.eval_value.clone(),
        )))
    }

    fn get_properties(
        &mut self,
        object_id: &str,
        _own_only: bool,
    ) -> Result<Vec<PropertyDescriptor>, HostError> {
        if object_id.is_empty() {
            return Err(HostError::InvalidParams("empty objectId".into()));
        }
        Ok(vec![PropertyDescriptor::own(
            "answer",
            RemoteObject::number(42.0),
        )])
    }

    fn get_document(&mut self, depth: u32) -> Result<NodeDescription, HostError> {
        Ok(truncate(&mock_document(), depth))
    }

    fn query_selector(
        &mut self,
        _node: EngineNodeId,
        selector: &str,
    ) -> Result<Option<EngineNodeId>, HostError> {
        Ok(find_all(&mock_document(), selector).first().copied())
    }

    fn query_selector_all(
        &mut self,
        _node: EngineNodeId,
        selector: &str,
    ) -> Result<Vec<EngineNodeId>, HostError> {
        Ok(find_all(&mock_document(), selector))
    }

    fn get_outer_html(&mut self, node: EngineNodeId) -> Result<String, HostError> {
        let found = find_by_id(&mock_document(), node).ok_or(HostError::NotFound)?;
        Ok(outer_html(&found))
    }

    fn describe_node(&mut self, node: EngineNodeId, depth: u32) -> Result<NodeDescription, HostError> {
        let found = find_by_id(&mock_document(), node).ok_or(HostError::NotFound)?;
        Ok(truncate(&found, depth))
    }

    fn set_attribute_value(
        &mut self,
        node: EngineNodeId,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), HostError> {
        self.script
            .lock()
            .unwrap()
            .mutations
            .push((node, name.to_string(), value.map(str::to_string)));
        Ok(())
    }

    fn resolve_node(&mut self, backend_node_id: u64) -> Result<EngineNodeId, HostError> {
        let node = EngineNodeId(backend_node_id);
        if find_by_id(&mock_document(), node).is_some() {
            Ok(node)
        } else {
            Err(HostError::NotFound)
        }
    }

    fn get_computed_style_for_node(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<(String, String)>, HostError> {
        Ok(self.script.lock().unwrap().computed_style.clone())
    }

    fn get_matched_styles_for_node(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<MatchedRule>, HostError> {
        Ok(self.script.lock().unwrap().matched_rules.clone())
    }

    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        Ok(self.script.lock().unwrap().layout_metrics)
    }

    fn get_resource_tree(&mut self) -> Result<FrameResourceTree, HostError> {
        let url = self.target.url.clone();
        let script = self.script.lock().unwrap();
        Ok(FrameResourceTree {
            frame: self.frame(&url),
            resources: script.resources.clone(),
            child_frames: Vec::new(),
        })
    }

    fn get_resource_content(
        &mut self,
        _frame_id: &str,
        url: &str,
    ) -> Result<ResourceContent, HostError> {
        let script = self.script.lock().unwrap();
        script
            .resource_bodies
            .iter()
            .find(|(candidate, _, _)| candidate == url)
            .map(|(_, bytes, mime)| ResourceContent {
                mime_type: mime.clone(),
                bytes: bytes.clone(),
            })
            .ok_or(HostError::NotFound)
    }

    fn get_box_model(&mut self, _node: EngineNodeId) -> Result<BoxModel, HostError> {
        Ok(self.script.lock().unwrap().box_model)
    }

    fn get_content_quads(&mut self, _node: EngineNodeId) -> Result<Vec<BoxModel>, HostError> {
        Ok(vec![self.script.lock().unwrap().box_model])
    }

    fn set_node_value(&mut self, node: EngineNodeId, value: &str) -> Result<(), HostError> {
        self.script
            .lock()
            .unwrap()
            .node_values
            .push((node, value.to_string()));
        Ok(())
    }

    fn perform_search(&mut self, query: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let mut found = Vec::new();
        search_document(&mock_document(), query, &mut found);
        Ok(found)
    }

    fn push_nodes(&mut self, backend_node_ids: &[u64]) -> Result<Vec<EngineNodeId>, HostError> {
        let mut nodes = Vec::with_capacity(backend_node_ids.len());
        for backend in backend_node_ids {
            nodes.push(self.resolve_node(*backend)?);
        }
        Ok(nodes)
    }

    fn get_style_sheet_text(
        &mut self,
        _style_sheet_id: &str,
    ) -> Result<Option<String>, HostError> {
        Ok(self.script.lock().unwrap().style_sheet_text.clone())
    }

    fn set_style_sheet_text(
        &mut self,
        _style_sheet_id: &str,
        text: &str,
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().style_sheet_text = Some(text.to_string());
        Ok(())
    }

    fn get_media_queries(&mut self) -> Result<Vec<MediaQuery>, HostError> {
        Ok(self.script.lock().unwrap().media_queries.clone())
    }

    fn get_background_colors(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<Option<String>>, HostError> {
        Ok(self.script.lock().unwrap().background_colors.clone())
    }

    fn node_for_location(&mut self, _x: f64, _y: f64) -> Result<EngineNodeId, HostError> {
        self.script
            .lock()
            .unwrap()
            .hit_test
            .ok_or(HostError::NotFound)
    }

    fn set_ignore_input_events(&mut self, ignore: bool) -> Result<(), HostError> {
        self.script.lock().unwrap().ignore_input = Some(ignore);
        Ok(())
    }

    fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
        self.script.lock().unwrap().mouse_events.push(event.clone());
        Ok(())
    }

    fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
        self.script.lock().unwrap().key_events.push(event.clone());
        Ok(())
    }

    fn dispatch_touch_event(&mut self, event: &TouchEvent) -> Result<(), HostError> {
        self.script.lock().unwrap().touch_events.push(event.clone());
        Ok(())
    }

    fn insert_text(&mut self, text: &str) -> Result<(), HostError> {
        self.script
            .lock()
            .unwrap()
            .inserted_text
            .push(text.to_string());
        Ok(())
    }

    fn get_response_body(
        &mut self,
        request_id: &str,
    ) -> Result<Option<ResourceContent>, HostError> {
        let script = self.script.lock().unwrap();
        Ok(script
            .response_bodies
            .iter()
            .find(|(id, _, _)| id == request_id)
            .map(|(_, bytes, mime)| ResourceContent {
                mime_type: mime.clone(),
                bytes: bytes.clone(),
            }))
    }

    fn get_request_post_data(&mut self, request_id: &str) -> Result<Option<String>, HostError> {
        Ok(self
            .script
            .lock()
            .unwrap()
            .request_post_data
            .iter()
            .find(|(id, _)| id == request_id)
            .map(|(_, text)| text.clone()))
    }

    fn set_fetch_enabled(
        &mut self,
        enabled: bool,
        patterns: &[FetchPattern],
        handle_auth: bool,
    ) -> Result<(), HostError> {
        let mut script = self.script.lock().unwrap();
        script.fetch_enabled = enabled;
        script.fetch_patterns = patterns.to_vec();
        script.fetch_handle_auth = handle_auth;
        Ok(())
    }

    fn drain_intercepted_requests(&mut self) -> Vec<InterceptedRequest> {
        self.script.lock().unwrap().intercepted.drain(..).collect()
    }

    fn resolve_intercepted_request(
        &mut self,
        request_id: &str,
        decision: &FetchDecision,
    ) -> Result<(), HostError> {
        self.script
            .lock()
            .unwrap()
            .fetch_decisions
            .push((request_id.to_string(), decision.clone()));
        Ok(())
    }

    fn set_device_metrics_override(&mut self, metrics: &DeviceMetrics) -> Result<(), HostError> {
        self.script.lock().unwrap().device_metrics = Some(metrics.clone());
        Ok(())
    }

    fn clear_device_metrics_override(&mut self) -> Result<(), HostError> {
        self.script.lock().unwrap().device_metrics = None;
        Ok(())
    }

    fn set_user_agent_override(
        &mut self,
        user_agent: &str,
        _accept_language: Option<&str>,
        _platform: Option<&str>,
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().user_agent_override = Some(user_agent.to_string());
        Ok(())
    }

    fn set_geolocation_override(
        &mut self,
        position: Option<(f64, f64, f64)>,
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().geolocation_override = Some(position);
        Ok(())
    }

    fn set_timezone_override(&mut self, timezone_id: &str) -> Result<(), HostError> {
        self.script.lock().unwrap().timezone_override = Some(timezone_id.to_string());
        Ok(())
    }

    fn set_locale_override(&mut self, locale: Option<&str>) -> Result<(), HostError> {
        self.script.lock().unwrap().locale_override = Some(locale.map(str::to_string));
        Ok(())
    }

    fn set_emulated_media(
        &mut self,
        media: Option<&str>,
        features: &[MediaFeature],
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().emulated_media =
            Some((media.map(str::to_string), features.to_vec()));
        Ok(())
    }

    fn set_scrollbars_hidden(&mut self, hidden: bool) -> Result<(), HostError> {
        self.script.lock().unwrap().scrollbars_hidden = Some(hidden);
        Ok(())
    }

    fn set_touch_emulation_enabled(
        &mut self,
        enabled: bool,
        max_touch_points: Option<u32>,
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().touch_emulation = Some((enabled, max_touch_points));
        Ok(())
    }

    fn set_cpu_throttling_rate(&mut self, rate: f64) -> Result<(), HostError> {
        self.script.lock().unwrap().cpu_throttling_rate = Some(rate);
        Ok(())
    }

    fn set_script_execution_disabled(&mut self, value: bool) -> Result<(), HostError> {
        self.script.lock().unwrap().script_execution_disabled = Some(value);
        Ok(())
    }

    fn overlay_enabled(&mut self, enabled: bool) -> Result<(), HostError> {
        self.script.lock().unwrap().overlay_enabled = Some(enabled);
        Ok(())
    }

    fn set_inspect_mode(
        &mut self,
        mode: &str,
        _config: &HighlightConfig,
    ) -> Result<(), HostError> {
        self.script.lock().unwrap().inspect_mode = Some(mode.to_string());
        Ok(())
    }

    fn highlight_node(
        &mut self,
        node: EngineNodeId,
        config: &HighlightConfig,
    ) -> Result<(), HostError> {
        let mut script = self.script.lock().unwrap();
        script.highlighted_node = Some(node);
        script.highlight_config = Some(config.clone());
        Ok(())
    }

    fn highlight_rect(&mut self, rect: Rect, config: &HighlightConfig) -> Result<(), HostError> {
        let mut script = self.script.lock().unwrap();
        script.highlighted_rect = Some(rect);
        script.highlight_config = Some(config.clone());
        Ok(())
    }

    fn highlight_quad(&mut self, quad: Quad, config: &HighlightConfig) -> Result<(), HostError> {
        let mut script = self.script.lock().unwrap();
        script.highlighted_quad = Some(quad);
        script.highlight_config = Some(config.clone());
        Ok(())
    }

    fn hide_highlight(&mut self) -> Result<(), HostError> {
        self.script.lock().unwrap().highlight_hidden = Some(true);
        Ok(())
    }

    fn performance_metrics(&mut self) -> Result<Vec<(String, f64)>, HostError> {
        Ok(self.script.lock().unwrap().performance_metrics.clone())
    }

    fn security_state(&mut self) -> Result<SecurityState, HostError> {
        Ok(self.script.lock().unwrap().current_security_state.clone())
    }

    fn drain_security_states(&mut self) -> Vec<SecurityState> {
        self.script
            .lock()
            .unwrap()
            .security_states
            .drain(..)
            .collect()
    }

    fn set_ignore_certificate_errors(&mut self, ignore: bool) -> Result<(), HostError> {
        self.script.lock().unwrap().ignore_certificate_errors = Some(ignore);
        Ok(())
    }

    fn handle_certificate_error(&mut self, event_id: i64, action: i32) -> Result<(), HostError> {
        self.script
            .lock()
            .unwrap()
            .certificate_errors
            .push((event_id, action));
        Ok(())
    }

    fn drain_lifecycle(&mut self) -> Vec<LifecycleEvent> {
        self.script.lock().unwrap().lifecycle.drain(..).collect()
    }

    fn drain_console(&mut self) -> Vec<ConsoleEvent> {
        self.script.lock().unwrap().console.drain(..).collect()
    }

    fn drain_network_requests(&mut self) -> Vec<NetworkRequestEvent> {
        self.script.lock().unwrap().requests.drain(..).collect()
    }

    fn drain_network_responses(&mut self) -> Vec<NetworkResponseEvent> {
        self.script.lock().unwrap().responses.drain(..).collect()
    }

    fn drain_network_finished(&mut self) -> Vec<NetworkFinishedEvent> {
        self.script.lock().unwrap().finished.drain(..).collect()
    }

    fn drain_log(&mut self) -> Vec<LogEntry> {
        self.script.lock().unwrap().logs.drain(..).collect()
    }
}

fn truncate(node: &NodeDescription, depth: u32) -> NodeDescription {
    let mut copy = node.clone();
    if depth == 0 {
        copy.children = Vec::new();
    } else {
        copy.children = node
            .children
            .iter()
            .map(|child| truncate(child, depth - 1))
            .collect();
    }
    copy
}

fn find_by_id(node: &NodeDescription, id: EngineNodeId) -> Option<NodeDescription> {
    if node.node == id {
        return Some(node.clone());
    }
    node.children
        .iter()
        .find_map(|child| find_by_id(child, id))
}

fn find_all(node: &NodeDescription, selector: &str) -> Vec<EngineNodeId> {
    let mut found = Vec::new();
    collect_matches(node, selector, &mut found);
    found
}

fn collect_matches(node: &NodeDescription, selector: &str, out: &mut Vec<EngineNodeId>) {
    if node.node_type == 1 && selector_matches(node, selector) {
        out.push(node.node);
    }
    for child in &node.children {
        collect_matches(child, selector, out);
    }
}

fn search_document(node: &NodeDescription, query: &str, out: &mut Vec<EngineNodeId>) {
    let needle = query.to_ascii_lowercase();
    if !needle.is_empty() {
        let mut matched = node.local_name.to_ascii_lowercase().contains(&needle)
            || node.node_value.to_ascii_lowercase().contains(&needle);
        if !matched {
            matched = node.attributes.iter().any(|(name, value)| {
                name.to_ascii_lowercase().contains(&needle)
                    || value.to_ascii_lowercase().contains(&needle)
            });
        }
        if matched {
            out.push(node.node);
        }
    }
    for child in &node.children {
        search_document(child, query, out);
    }
}

fn selector_matches(node: &NodeDescription, selector: &str) -> bool {
    if let Some(id) = selector.strip_prefix('#') {
        return node
            .attributes
            .iter()
            .any(|(name, value)| name == "id" && value == id);
    }
    if let Some(class) = selector.strip_prefix('.') {
        return node.attributes.iter().any(|(name, value)| {
            name == "class" && value.split_whitespace().any(|item| item == class)
        });
    }
    !selector.is_empty() && selector.eq_ignore_ascii_case(&node.local_name)
}

fn outer_html(node: &NodeDescription) -> String {
    if node.node_type == 3 {
        return escape(&node.node_value);
    }
    let mut out = format!("<{}", node.local_name);
    for (name, value) in &node.attributes {
        out.push_str(&format!(" {}=\"{}\"", name, escape(value)));
    }
    out.push('>');
    for child in &node.children {
        out.push_str(&outer_html(child));
    }
    out.push_str(&format!("</{}>", node.local_name));
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_the_mock_document() {
        let mut session = MockSession {
            script: Arc::new(Mutex::new(MockScript::default())),
            target: TargetInfo::page("page-1", "about:blank", "Mock"),
        };
        assert_eq!(
            session.query_selector(EngineNodeId(1), "div").unwrap(),
            Some(EngineNodeId(4))
        );
        assert_eq!(
            session.query_selector(EngineNodeId(1), "#main").unwrap(),
            Some(EngineNodeId(4))
        );
        assert_eq!(
            session.query_selector_all(EngineNodeId(1), "span").unwrap(),
            vec![EngineNodeId(5)]
        );
        assert!(session.get_outer_html(EngineNodeId(4)).unwrap().contains("id=\"main\""));
    }

    #[test]
    fn navigation_records_and_queues_lifecycle() {
        let script = Arc::new(Mutex::new(MockScript::default()));
        let mut session = MockSession {
            script: Arc::clone(&script),
            target: TargetInfo::page("page-1", "about:blank", "Mock"),
        };
        session.navigate("https://example.com/").unwrap();
        assert_eq!(script.lock().unwrap().navigations, ["https://example.com/"]);
        assert_eq!(session.drain_lifecycle().len(), 2);
        assert_eq!(session.drain_lifecycle().len(), 0);
    }
}
