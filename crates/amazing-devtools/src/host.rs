//! The engine-facing seam: what the driver must implement for CDP to work.
//!
//! `amazing-devtools` owns the wire protocol, the WebSocket/HTTP server and the
//! JSON-RPC state machine. It knows nothing about the DOM, layout, the JS
//! runtime or the network stack. Everything it needs from the browser is
//! expressed by two object-safe traits:
//!
//! * [`DevToolsHost`] is the browser as a whole: product/version strings and the
//!   list of debuggable targets. The driver hands one `Arc<dyn DevToolsHost>` to
//!   [`crate::server::CdpServer::bind`].
//! * [`DevToolsSession`] is one attached target (a page). It is handed to a
//!   WebSocket connection when a client opens `/devtools/page/<id>`, and lives
//!   for that connection. Every method has a default implementation returning
//!   [`HostError::Unsupported`], so the driver can adopt domains one at a time
//!   without writing stubs for the rest.
//!
//! ## Identity rules
//!
//! CDP node ids are per-session integers assigned by this crate; engine node
//! identity is [`EngineNodeId`], a stable `u64` the driver controls (typically
//! an arena slot plus generation). `backendNodeId` on the wire is
//! `EngineNodeId.0`, so [`DevToolsSession`] never has to mint ids.
//!
//! ## Event delivery
//!
//! The server polls `drain_*` on each connection's own thread, after every
//! inbound command and whenever the socket is idle. A driver that produces
//! events from other threads queues them internally and returns them here; the
//! methods must not block.

/// Why a [`DevToolsSession`] or [`DevToolsHost`] call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// This domain/method is not wired up yet. The CDP caller sees
    /// `-32601 method not found` (or `-32000` for a known-but-unsupported
    /// parameter).
    Unsupported,
    /// The target, node or object referenced does not exist.
    NotFound,
    /// The request was syntactically valid but semantically rejected.
    InvalidParams(String),
    /// The engine failed while servicing a valid request.
    Internal(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("operation not supported by this engine build"),
            Self::NotFound => f.write_str("referenced target, node or object not found"),
            Self::InvalidParams(m) => write!(f, "invalid parameters: {m}"),
            Self::Internal(m) => write!(f, "engine error: {m}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Browser-level identity, surfaced by `/json/version` and `Target.getBrowserVersion`.
#[derive(Clone, Debug, PartialEq)]
pub struct HostInfo {
    /// `Browser.getVersion.product`, e.g. `"Amazing/0.0.1"`.
    pub product: String,
    /// `Browser.getVersion.userAgent`.
    pub user_agent: String,
    /// `Browser.getVersion.revision`, usually the source commit.
    pub revision: String,
    /// `Browser.getVersion.jsVersion`, e.g. the JS engine version.
    pub js_version: String,
    /// The CDP protocol version advertised to clients.
    pub protocol_version: String,
}

impl Default for HostInfo {
    fn default() -> Self {
        Self {
            product: "Amazing/0.0.1".into(),
            user_agent: format!("Mozilla/5.0 (compatible; Amazing/0.0.1)"),
            revision: String::new(),
            js_version: String::new(),
            protocol_version: "1.3".into(),
        }
    }
}

/// One debuggable target, as returned by `Target.getTargets` and `/json/list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetInfo {
    /// Stable opaque id, used in `/devtools/page/<id>` and `Target.attachToTarget`.
    pub id: String,
    /// CDP target type: `"page"`, `"browser"`, `"iframe"`, ...
    pub target_type: String,
    /// Page title.
    pub title: String,
    /// Current URL.
    pub url: String,
    /// Whether a devtools client is currently attached.
    pub attached: bool,
    /// The target that opened this one, for popups.
    pub opener_id: Option<String>,
    /// Browser context the target belongs to.
    pub browser_context_id: Option<String>,
}

impl TargetInfo {
    /// A minimal page target.
    pub fn page(id: impl Into<String>, url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            target_type: "page".into(),
            title: title.into(),
            url: url.into(),
            attached: false,
            opener_id: None,
            browser_context_id: None,
        }
    }
}

/// Stable engine node identity (arena slot plus generation, or equivalent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EngineNodeId(pub u64);

/// CDP `Runtime.RemoteObject`.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteObject {
    /// `"object"`, `"function"`, `"undefined"`, `"string"`, `"number"`,
    /// `"boolean"`, `"symbol"`, `"bigint"`.
    pub object_type: String,
    pub subtype: Option<String>,
    pub class_name: Option<String>,
    /// Present when the value was serialized by value.
    pub value: Option<crate::json::Json>,
    /// Present for bigint values the JSON model cannot carry.
    pub unserializable_value: Option<String>,
    pub description: Option<String>,
    /// Opaque handle for `Runtime.callFunctionOn` / `Runtime.getProperties`.
    pub object_id: Option<String>,
}

impl Default for RemoteObject {
    fn default() -> Self {
        Self::undefined()
    }
}

impl RemoteObject {
    /// The `undefined` remote object.
    pub fn undefined() -> Self {
        Self {
            object_type: "undefined".into(),
            subtype: None,
            class_name: None,
            value: None,
            unserializable_value: None,
            description: None,
            object_id: None,
        }
    }

    /// A remote number.
    pub fn number(value: f64) -> Self {
        Self {
            object_type: "number".into(),
            value: Some(crate::json::Json::Number(value)),
            description: Some(crate::json::Json::Number(value).to_string()),
            ..Self::undefined()
        }
    }

    /// A remote string.
    pub fn string(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            object_type: "string".into(),
            value: Some(crate::json::Json::String(value.clone())),
            description: Some(value),
            ..Self::undefined()
        }
    }

    /// A remote boolean.
    pub fn boolean(value: bool) -> Self {
        Self {
            object_type: "boolean".into(),
            value: Some(crate::json::Json::Bool(value)),
            description: Some(value.to_string()),
            ..Self::undefined()
        }
    }

    /// A remote `null`.
    pub fn null() -> Self {
        Self {
            object_type: "object".into(),
            subtype: Some("null".into()),
            value: Some(crate::json::Json::Null),
            description: Some("null".into()),
            ..Self::undefined()
        }
    }

    /// A remote plain object serialized by value.
    pub fn object(value: crate::json::Json) -> Self {
        Self {
            object_type: "object".into(),
            value: Some(value),
            ..Self::undefined()
        }
    }

    /// A remote object handle (by reference).
    pub fn handle(object_id: impl Into<String>, class_name: impl Into<String>) -> Self {
        Self {
            object_type: "object".into(),
            class_name: Some(class_name.into()),
            object_id: Some(object_id.into()),
            ..Self::undefined()
        }
    }
}

/// CDP `Runtime.CallArgument`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallArgument {
    pub value: Option<crate::json::Json>,
    pub object_id: Option<String>,
}

/// CDP `Runtime.Evaluate` inputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluateParams {
    pub expression: String,
    pub object_group: Option<String>,
    pub include_command_line_api: bool,
    pub silent: bool,
    pub return_by_value: bool,
    pub generate_preview: bool,
    pub await_promise: bool,
    pub context_id: Option<u64>,
    pub user_gesture: bool,
}

/// CDP `Runtime.CallFunctionOn` inputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallFunctionParams {
    pub function_declaration: String,
    pub object_id: Option<String>,
    pub arguments: Vec<CallArgument>,
    pub return_by_value: bool,
    pub await_promise: bool,
    pub object_group: Option<String>,
}

/// CDP `Runtime.ExceptionDetails`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExceptionDetails {
    pub text: String,
    pub line_number: u32,
    pub column_number: u32,
    pub url: Option<String>,
    pub exception: Option<RemoteObject>,
    pub stack_trace: Option<String>,
}

/// Outcome of `Runtime.evaluate` / `Runtime.callFunctionOn`.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluateResult {
    pub result: RemoteObject,
    pub exception: Option<ExceptionDetails>,
}

impl EvaluateResult {
    /// A successful by-value result.
    pub fn value(result: RemoteObject) -> Self {
        Self {
            result,
            exception: None,
        }
    }
}

/// CDP `Runtime.PropertyDescriptor`, used by `Runtime.getProperties`.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyDescriptor {
    pub name: String,
    pub value: RemoteObject,
    pub writable: bool,
    pub configurable: bool,
    pub enumerable: bool,
    pub is_own: bool,
}

impl PropertyDescriptor {
    /// A read-only own data property.
    pub fn own(name: impl Into<String>, value: RemoteObject) -> Self {
        Self {
            name: name.into(),
            value,
            writable: false,
            configurable: true,
            enumerable: true,
            is_own: true,
        }
    }
}

/// CDP `Page.Frame`.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub url: String,
    pub name: String,
    pub security_origin: String,
    pub mime_type: String,
}

/// CDP `Page.FrameTree`.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTree {
    pub frame: FrameInfo,
    pub child_frames: Vec<FrameTree>,
}

/// CDP `DOM.Node`, translated from the engine's node identity.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDescription {
    pub node: EngineNodeId,
    pub parent: Option<EngineNodeId>,
    /// Usually `node.0`; the engine may override it.
    pub backend_node_id: u64,
    /// DOM `nodeType`, e.g. 1 for elements.
    pub node_type: u16,
    pub node_name: String,
    pub local_name: String,
    pub node_value: String,
    pub attributes: Vec<(String, String)>,
    /// Number of children when not all are included.
    pub child_node_count: usize,
    /// Children included to the requested depth.
    pub children: Vec<NodeDescription>,
}

impl NodeDescription {
    /// An element node with no children.
    pub fn element(node: EngineNodeId, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            node,
            parent: None,
            backend_node_id: node.0,
            node_type: 1,
            node_name: name.to_ascii_uppercase(),
            local_name: name.to_ascii_lowercase(),
            node_value: String::new(),
            attributes: Vec::new(),
            child_node_count: 0,
            children: Vec::new(),
        }
    }

    /// A text node.
    pub fn text(node: EngineNodeId, value: impl Into<String>) -> Self {
        Self {
            node,
            parent: None,
            backend_node_id: node.0,
            node_type: 3,
            node_name: "#text".into(),
            local_name: String::new(),
            node_value: value.into(),
            attributes: Vec::new(),
            child_node_count: 0,
            children: Vec::new(),
        }
    }

    /// Sets the attribute list and returns `self`.
    pub fn with_attributes(mut self, attributes: Vec<(String, String)>) -> Self {
        self.attributes = attributes;
        self
    }

    /// Attaches children, updating `child_node_count`, and returns `self`.
    pub fn with_children(mut self, children: Vec<NodeDescription>) -> Self {
        self.child_node_count = children.len();
        self.children = children;
        self
    }
}

/// A CSS origin for `CSS.getMatchedStylesForNode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleOrigin {
    UserAgent,
    User,
    Author,
    Inspector,
}

/// One matched rule from `CSS.getMatchedStylesForNode`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchedRule {
    pub selector: String,
    pub origin: StyleOrigin,
    pub properties: Vec<(String, String)>,
}

/// CDP `Page.Viewport` / screenshot clip rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// Image format for `Page.captureScreenshot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
    Webp,
}

/// `Page.captureScreenshot` inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenshotParams {
    pub format: ScreenshotFormat,
    pub quality: Option<u8>,
    pub clip: Option<Rect>,
    pub capture_beyond_viewport: bool,
    pub from_surface: bool,
}

impl Default for ScreenshotParams {
    fn default() -> Self {
        Self {
            format: ScreenshotFormat::Png,
            quality: None,
            clip: None,
            capture_beyond_viewport: false,
            from_surface: true,
        }
    }
}

/// An encoded screenshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screenshot {
    pub data: Vec<u8>,
    pub format: ScreenshotFormat,
}

/// `Page.frameNavigated` / `Page.lifecycleEvent` data.
#[derive(Clone, Debug, PartialEq)]
pub struct LifecycleEvent {
    pub frame_id: String,
    /// `Page.lifecycleEvent.name`, e.g. `"DOMContentLoaded"`, `"load"`.
    pub name: String,
    /// `"init"`, `"navigate"`, `"same-document"`, ...
    pub loader_id: String,
    /// Monotonic seconds.
    pub timestamp: f64,
}

/// `Runtime.consoleAPICalled` data.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleEvent {
    /// `"log"`, `"warn"`, `"error"`, `"info"`, `"debug"`, ...
    pub kind: String,
    pub args: Vec<RemoteObject>,
    pub execution_context_id: u64,
    /// Monotonic seconds.
    pub timestamp: f64,
}

/// `Network.requestWillBeSent` data.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkRequestEvent {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    /// `"Document"`, `"Script"`, `"Stylesheet"`, `"Image"`, `"Fetch"`, ...
    pub resource_type: String,
    pub frame_id: String,
    pub loader_id: String,
    /// Monotonic seconds.
    pub timestamp: f64,
    /// Seconds since the Unix epoch.
    pub wall_time: f64,
}

/// `Network.responseReceived` data.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkResponseEvent {
    pub request_id: String,
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub mime_type: String,
    pub headers: Vec<(String, String)>,
    pub encoded_data_length: u64,
    pub from_disk_cache: bool,
    pub resource_type: String,
    /// Monotonic seconds.
    pub timestamp: f64,
}

/// `Network.loadingFinished` data.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkFinishedEvent {
    pub request_id: String,
    pub encoded_data_length: u64,
    pub decoded_body_length: u64,
    /// Monotonic seconds.
    pub timestamp: f64,
}

/// `Log.entryAdded` data.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// `"javascript"`, `"network"`, `"security"`, `"deprecation"`, ...
    pub source: String,
    /// `"verbose"`, `"info"`, `"warning"`, `"error"`.
    pub level: String,
    pub text: String,
    /// Monotonic seconds.
    pub timestamp: f64,
    pub url: Option<String>,
    pub line_number: Option<u32>,
    pub stack_trace: Option<String>,
}

/// One attached page target.
///
/// Every method has a default so the driver can wire one domain at a time; an
/// unimplemented method yields [`HostError::Unsupported`] and the client sees
/// `method not found`. The two `drain_*` families must not block: return
/// whatever events have accumulated since the previous call.
pub trait DevToolsSession: Send {
    /// The [`TargetInfo::id`] this session is attached to.
    fn target_id(&self) -> &str {
        ""
    }

    // --- Page -------------------------------------------------------------
    /// Navigates the main frame; returns the committed (or provisional) frame.
    fn navigate(&mut self, _url: &str) -> Result<FrameInfo, HostError> {
        Err(HostError::Unsupported)
    }
    /// Reloads the main frame. `ignore_cache` maps to `Page.reload`.
    fn reload(&mut self, _ignore_cache: bool) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Returns the current frame tree rooted at the main frame.
    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        Err(HostError::Unsupported)
    }
    /// Renders the viewport (or clip) to an encoded image.
    fn capture_screenshot(&mut self, _params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Runtime ----------------------------------------------------------
    /// Evaluates a JS expression in the page's main execution context.
    fn evaluate(&mut self, _params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        Err(HostError::Unsupported)
    }
    /// Calls a function with `this` bound to `object_id` (or the global).
    fn call_function_on(
        &mut self,
        _params: &CallFunctionParams,
    ) -> Result<EvaluateResult, HostError> {
        Err(HostError::Unsupported)
    }
    /// Returns properties of a remote object handle.
    fn get_properties(
        &mut self,
        _object_id: &str,
        _own_only: bool,
    ) -> Result<Vec<PropertyDescriptor>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Drops a remote object handle.
    fn release_object(&mut self, _object_id: &str) -> Result<(), HostError> {
        Ok(())
    }

    // --- DOM --------------------------------------------------------------
    /// Returns the document root, expanded `depth` levels (0 = root only).
    fn get_document(&mut self, _depth: u32) -> Result<NodeDescription, HostError> {
        Err(HostError::Unsupported)
    }
    /// Returns the first descendant matching `selector` under `node`.
    fn query_selector(
        &mut self,
        _node: EngineNodeId,
        _selector: &str,
    ) -> Result<Option<EngineNodeId>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Returns every descendant matching `selector` under `node`.
    fn query_selector_all(
        &mut self,
        _node: EngineNodeId,
        _selector: &str,
    ) -> Result<Vec<EngineNodeId>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Serializes `node`'s subtree to HTML.
    fn get_outer_html(&mut self, _node: EngineNodeId) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }
    /// Describes `node`, expanding `depth` levels of children.
    fn describe_node(
        &mut self,
        _node: EngineNodeId,
        _depth: u32,
    ) -> Result<NodeDescription, HostError> {
        Err(HostError::Unsupported)
    }
    /// Sets an attribute; `value: None` removes it.
    fn set_attribute_value(
        &mut self,
        _node: EngineNodeId,
        _name: &str,
        _value: Option<&str>,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Resolves an engine node from a `nodeId`/`backendNodeId` pair.
    fn resolve_node(&mut self, _backend_node_id: u64) -> Result<EngineNodeId, HostError> {
        Err(HostError::Unsupported)
    }

    // --- CSS --------------------------------------------------------------
    /// Computed longhands for `node`, in declaration order.
    fn get_computed_style_for_node(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<(String, String)>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Matched rules for `node`, in cascade order.
    fn get_matched_styles_for_node(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<MatchedRule>, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Events -----------------------------------------------------------
    /// Lifecycle events (`Page.lifecycleEvent`).
    fn drain_lifecycle(&mut self) -> Vec<LifecycleEvent> {
        Vec::new()
    }
    /// Console API calls (`Runtime.consoleAPICalled`).
    fn drain_console(&mut self) -> Vec<ConsoleEvent> {
        Vec::new()
    }
    /// `Network.requestWillBeSent`.
    fn drain_network_requests(&mut self) -> Vec<NetworkRequestEvent> {
        Vec::new()
    }
    /// `Network.responseReceived`.
    fn drain_network_responses(&mut self) -> Vec<NetworkResponseEvent> {
        Vec::new()
    }
    /// `Network.loadingFinished`.
    fn drain_network_finished(&mut self) -> Vec<NetworkFinishedEvent> {
        Vec::new()
    }
    /// `Log.entryAdded`.
    fn drain_log(&mut self) -> Vec<LogEntry> {
        Vec::new()
    }
}

/// The browser as a whole. The driver implements this once.
pub trait DevToolsHost: Send + Sync + 'static {
    /// Product/version strings for `/json/version` and `Browser.getVersion`.
    fn info(&self) -> HostInfo {
        HostInfo::default()
    }
    /// All current targets, including non-page targets if any.
    fn list_targets(&self) -> Vec<TargetInfo> {
        Vec::new()
    }
    /// Attaches to `target_id`, returning a session for the connection.
    fn attach(&self, _target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        Err(HostError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_unsupported_not_panics() {
        struct Empty;
        impl DevToolsSession for Empty {}
        let mut session = Empty;
        assert_eq!(session.navigate("https://x"), Err(HostError::Unsupported));
        assert_eq!(session.get_document(0), Err(HostError::Unsupported));
        assert!(session.drain_log().is_empty());
    }

    #[test]
    fn remote_object_constructors() {
        assert_eq!(RemoteObject::number(2.0).value, Some(crate::json::Json::Number(2.0)));
        assert_eq!(RemoteObject::string("x").object_type, "string");
        assert_eq!(RemoteObject::null().subtype.as_deref(), Some("null"));
        assert_eq!(RemoteObject::handle("1", "Object").object_id.as_deref(), Some("1"));
    }

    #[test]
    fn node_description_builders() {
        let child = NodeDescription::text(EngineNodeId(2), "hi");
        let node = NodeDescription::element(EngineNodeId(1), "div")
            .with_attributes(vec![("id".into(), "a".into())])
            .with_children(vec![child]);
        assert_eq!(node.child_node_count, 1);
        assert_eq!(node.node_name, "DIV");
        assert_eq!(node.backend_node_id, 1);
    }
}
