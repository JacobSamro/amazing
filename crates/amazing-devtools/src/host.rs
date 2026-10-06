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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
    /// A shallow summary of the object's own enumerable properties
    /// (CDP-24). Real Chrome always attaches one for an object-typed
    /// `Runtime.getProperties` value; for `Runtime.evaluate`/
    /// `callFunctionOn` it is present only when that call's own
    /// `generatePreview` was `true`. `None` for a primitive or an
    /// unrequested preview — never a stand-in for `objectId`, which is set
    /// independently.
    pub preview: Option<ObjectPreview>,
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
            preview: None,
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

/// CDP `Runtime.ObjectPreview` (CDP-24): a shallow, best-effort summary of
/// an object's own enumerable properties, attached to a [`RemoteObject`] —
/// see [`RemoteObject::preview`]'s own doc comment for exactly when.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectPreview {
    pub object_type: String,
    pub subtype: Option<String>,
    pub description: Option<String>,
    /// `true` when more own enumerable properties exist than `properties`
    /// lists (a host may cap the list; see the implementing driver for its
    /// own limit).
    pub overflow: bool,
    pub properties: Vec<PropertyPreview>,
}

/// One entry in an [`ObjectPreview::properties`] list.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyPreview {
    pub name: String,
    /// `"object"`/`"function"`/`"undefined"`/`"string"`/`"number"`/
    /// `"boolean"`/`"symbol"`/`"bigint"`, matching `RemoteObject::object_type`.
    pub object_type: String,
    /// A short rendered value, present for a primitive; `None` for a nested
    /// object/function (real Chrome nests another preview there via
    /// `valuePreview`; a host may choose not to build one — see the
    /// implementing driver's own doc comment on why).
    pub value: Option<String>,
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

/// `Runtime.getHeapUsage` result, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeapUsage {
    pub used_heap_size: f64,
    pub total_heap_size: f64,
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

/// A four-corner quadrilateral, in the order CDP uses: top-left, top-right,
/// bottom-right, bottom-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub points: [(f64, f64); 4],
}

impl Quad {
    /// An axis-aligned rectangle.
    pub fn rect(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            points: [
                (x, y),
                (x + width, y),
                (x + width, y + height),
                (x, y + height),
            ],
        }
    }
}

/// CDP `DOM.BoxModel`: the content, padding, border and margin quads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxModel {
    pub content: Quad,
    pub padding: Quad,
    pub border: Quad,
    pub margin: Quad,
    pub width: i64,
    pub height: i64,
}

impl BoxModel {
    /// A box whose four quads are one rectangle, as a simple element produces.
    pub fn from_rect(x: f64, y: f64, width: f64, height: f64) -> Self {
        let quad = Quad::rect(x, y, width, height);
        Self {
            content: quad,
            padding: quad,
            border: quad,
            margin: quad,
            width: width.round() as i64,
            height: height.round() as i64,
        }
    }
}

/// CDP `Page.Viewport`, used for the layout and visual viewports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub page_x: f64,
    pub page_y: f64,
    pub client_width: f64,
    pub client_height: f64,
}

/// CDP `Page.LayoutMetrics`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutMetrics {
    pub layout_viewport: Viewport,
    pub visual_viewport: Viewport,
    pub content_width: f64,
    pub content_height: f64,
}

/// CDP `Input.dispatchMouseEvent` data.
#[derive(Clone, Debug, PartialEq)]
pub struct MouseEvent {
    /// `"mousePressed"`, `"mouseReleased"`, `"mouseMoved"`, `"mouseWheel"`.
    pub kind: String,
    pub x: f64,
    pub y: f64,
    /// CDP modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8.
    pub modifiers: u32,
    /// `"none"`, `"left"`, `"middle"`, `"right"`, `"back"`, `"forward"`.
    pub button: String,
    /// Bitmask of pressed buttons, same values as `button`.
    pub buttons: u32,
    pub click_count: u32,
    pub delta_x: f64,
    pub delta_y: f64,
    /// Seconds since the Unix epoch.
    pub timestamp: f64,
}

/// CDP `Input.dispatchKeyEvent` data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyEvent {
    /// `"keyDown"`, `"keyUp"`, `"rawKeyDown"`, `"char"`.
    pub kind: String,
    pub modifiers: u32,
    pub text: Option<String>,
    pub unmodified_text: Option<String>,
    pub key: Option<String>,
    pub code: Option<String>,
    pub windows_virtual_key_code: Option<i64>,
    pub native_virtual_key_code: Option<i64>,
    pub auto_repeat: bool,
    pub is_keypad: bool,
    pub is_system_key: bool,
    /// `"standard"`, `"left"`, `"right"`, `"numpad"`.
    pub location: Option<String>,
}

/// One CDP `Input.TouchPoint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchPoint {
    pub x: f64,
    pub y: f64,
    pub radius_x: f64,
    pub radius_y: f64,
    pub rotation_angle: f64,
    pub force: f64,
    pub id: f64,
}

/// CDP `Input.dispatchTouchEvent` data.
#[derive(Clone, Debug, PartialEq)]
pub struct TouchEvent {
    /// `"touchStart"`, `"touchEnd"`, `"touchMove"`, `"touchCancel"`.
    pub kind: String,
    pub points: Vec<TouchPoint>,
    pub modifiers: u32,
    pub timestamp: f64,
}

/// One CDP `Fetch.RequestPattern`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchPattern {
    /// Wildcard URL pattern (`*` and `?`); `None` or `"*"` matches everything.
    pub url_pattern: Option<String>,
    /// `"Document"`, `"Stylesheet"`, `"Image"`, `"XHR"`, `"Fetch"`, ...
    pub resource_type: Option<String>,
    /// `"Request"` (default) or `"Response"`.
    pub request_stage: Option<String>,
}

/// A request the engine paused for Fetch interception.
#[derive(Clone, Debug, PartialEq)]
pub struct InterceptedRequest {
    pub request_id: String,
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub resource_type: String,
    pub frame_id: String,
    /// The matching `Network.*` request id, when available.
    pub network_id: Option<String>,
    /// `"Request"` or `"Response"`.
    pub stage: String,
}

/// A `Fetch.continueRequest` decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FetchContinue {
    pub url: Option<String>,
    pub method: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub post_data: Option<String>,
}

/// A `Fetch.fulfillRequest` decision.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchFulfill {
    pub response_code: u16,
    pub response_phrase: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What the client decided for a paused Fetch request.
#[derive(Clone, Debug, PartialEq)]
pub enum FetchDecision {
    Continue(FetchContinue),
    Fail(String),
    Fulfill(FetchFulfill),
}

/// CDP `Emulation.setDeviceMetricsOverride` inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceMetrics {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub mobile: bool,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
    pub position_x: Option<i64>,
    pub position_y: Option<i64>,
    pub dont_set_visible_size: bool,
    /// `"portraitPrimary"`, `"portraitSecondary"`, `"landscapePrimary"` or
    /// `"landscapeSecondary"`.
    pub orientation_type: Option<String>,
    pub orientation_angle: Option<i64>,
}

/// One `Emulation.setEmulatedMedia` feature.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaFeature {
    pub name: String,
    pub value: String,
}

/// A CDP RGBA color (`Overlay.*` highlight colors).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub alpha: f64,
}

/// CDP `Overlay.HighlightConfig`, reduced to the node-highlight inputs an
/// engine needs. Unset colors fall back to the engine's own defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HighlightConfig {
    pub content_color: Option<Rgba>,
    pub padding_color: Option<Rgba>,
    pub border_color: Option<Rgba>,
    pub margin_color: Option<Rgba>,
    pub event_target_color: Option<Rgba>,
    pub shape_color: Option<Rgba>,
    pub show_info: bool,
    pub show_styles: bool,
    pub show_rulers: bool,
    pub show_extension_lines: bool,
}

/// CDP `Security.securityStateChanged` data.
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityState {
    /// `"unknown"`, `"neutral"`, `"insecure"`, `"secure"`, `"info"`,
    /// `"insecure-broken"` or `"warning"`.
    pub security_state: String,
    pub scheme_is_cryptographic: bool,
    pub explanations: Vec<String>,
    /// `"unknown"`, `"blocked"`, `"allowed"` or `"info"`.
    pub insecure_content_status: String,
}

impl Default for SecurityState {
    fn default() -> Self {
        Self {
            security_state: "unknown".into(),
            scheme_is_cryptographic: false,
            explanations: Vec::new(),
            insecure_content_status: "unknown".into(),
        }
    }
}

/// One node of the CDP `Accessibility.AXNode` tree. The driver builds these
/// from its own accessibility-tree representation (for this engine, the a11y
/// lane's `amazing-a11y::AxTree`); this crate stays independent of that crate
/// and only defines the CDP-shaped seam.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxNode {
    /// The CDP `AXNodeId` (a string, distinct from DOM node ids).
    pub ax_node_id: String,
    /// Whether the node is pruned from the tree (still present so tools can
    /// show ignored subtrees on request).
    pub ignored: bool,
    /// The computed ARIA role, e.g. `"button"`, `"WebArea"`, `"generic"`.
    pub role: Option<String>,
    /// The computed accessible name.
    pub name: Option<String>,
    /// The computed accessible description.
    pub description: Option<String>,
    /// The computed value, for widgets that have one (inputs, sliders, …).
    pub value: Option<String>,
    /// The originating DOM node, if any (the synthetic root may have none).
    pub node: Option<EngineNodeId>,
    /// Child `AXNodeId`s, in tree order.
    pub child_ids: Vec<String>,
}

/// A CDP `Debugger.Location`: one place in one script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptLocation {
    pub script_id: String,
    pub line_number: u32,
    pub column_number: Option<u32>,
}

/// `Debugger.scriptParsed` data for one parsed script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptInfo {
    pub script_id: String,
    pub url: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// A hash the client can use to detect edits; the driver may leave this
    /// empty if it does not track script hashes.
    pub hash: String,
}

/// One CDP `Debugger.Scope` (`Debugger.CallFrame.scopeChain`'s elements).
/// `object` is a real, live `RemoteObject` — `Runtime.getProperties`/
/// `callFunctionOn` resolve its `objectId` back to the actual engine value
/// the same way any other object reference in this crate does, not a
/// snapshot or a fake.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    /// `"global"`, `"local"`, `"closure"`, `"block"`, `"catch"`, `"with"`,
    /// `"eval"`, `"module"`, `"script"`.
    pub scope_type: String,
    pub object: RemoteObject,
}

/// One CDP `Debugger.CallFrame`, simplified to the fields a front end needs
/// to show a stack and evaluate in a frame's scope. Chrome's full schema also
/// carries a `this` value and generator/async detail; the driver can add
/// those later without breaking this seam (`evaluateOnCallFrame` already
/// returns a full `EvaluateResult`). `scope_chain` was added in CDP-22 (see
/// the driver's own doc comment for what it actually contains, given this
/// engine's single-threaded, non-suspending debug model).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallFrame {
    pub call_frame_id: String,
    pub function_name: String,
    pub location: ScriptLocation,
    pub url: String,
    pub scope_chain: Vec<Scope>,
}

/// `Debugger.paused` data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PausedEvent {
    pub call_frames: Vec<CallFrame>,
    /// `"XHR"`, `"DOM"`, `"EventListener"`, `"exception"`, `"assert"`,
    /// `"debugCommand"`, `"promiseRejection"`, `"OOM"`, `"other"`,
    /// `"ambiguous"`, or a step landing (also `"other"` by convention).
    pub reason: String,
    pub hit_breakpoints: Vec<String>,
}

/// One node of a `Profiler.stop` CPU profile tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuProfileNode {
    pub id: u32,
    pub function_name: String,
    pub script_id: String,
    pub url: String,
    pub line_number: u32,
    pub hit_count: u32,
    pub children: Vec<u32>,
}

/// A `Profiler.Profile` (`Profiler.stop`'s result).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuProfile {
    pub nodes: Vec<CpuProfileNode>,
    pub start_time: f64,
    pub end_time: f64,
    /// Node id sampled at each tick.
    pub samples: Vec<u32>,
    /// Microseconds between consecutive samples.
    pub time_deltas: Vec<i64>,
}

/// One node of a `DOMSnapshot.captureSnapshot` document, before string
/// interning (this crate performs that when serializing the wire format, so
/// the seam itself stays in plain strings).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomSnapshotNode {
    /// A CDP `Node.nodeType` value (1 = element, 3 = text, 9 = document, …).
    pub node_type: u32,
    pub node_name: String,
    pub node_value: String,
    pub backend_node_id: EngineNodeId,
    pub attributes: Vec<(String, String)>,
    /// Indices into this document's flat `nodes`, in tree order.
    pub children: Vec<u32>,
}

/// One document of a `DOMSnapshot.captureSnapshot` result: a flat,
/// depth-first list of [`DomSnapshotNode`]s starting at the document node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomSnapshotDocument {
    pub document_url: String,
    pub base_url: String,
    pub nodes: Vec<DomSnapshotNode>,
}

/// A CDP `Network.Cookie`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: f64,
    pub http_only: bool,
    pub secure: bool,
    /// `"Strict"`, `"Lax"`, `"None"`, or empty when unset.
    pub same_site: String,
}

/// Identifies one `localStorage`/`sessionStorage` area
/// (`DOMStorage.StorageId`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomStorageId {
    pub security_origin: String,
    pub is_local_storage: bool,
}

/// CDP `Page.ResourceType`. Stored as a string so the driver can report any
/// resource kind without this crate enumerating them.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceInfo {
    pub url: String,
    pub mime_type: String,
    pub resource_type: String,
    pub content_size: Option<u64>,
    /// Seconds since the Unix epoch, if known.
    pub last_modified: Option<f64>,
    pub cached: bool,
}

/// CDP `Page.FrameResourceTree`.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameResourceTree {
    pub frame: FrameInfo,
    pub resources: Vec<ResourceInfo>,
    pub child_frames: Vec<FrameResourceTree>,
}

/// The body of one `Page.getResourceContent` response.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceContent {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// CDP `CSS.MediaQuery`.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaQuery {
    pub text: String,
    /// `"mediaRule"`, `"importRule"`, `"linkedSheet"` or `"inlineSheet"`.
    pub source: String,
    pub source_url: Option<String>,
}

/// Why the target list changed, polled by the browser endpoint and emitted as
/// the `Target.*` lifecycle events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetEventKind {
    Created,
    InfoChanged,
    Destroyed,
    Attached,
    Detached,
}

/// One target-list change. `session_id` is set for [`TargetEventKind::Attached`]
/// and [`TargetEventKind::Detached`].
#[derive(Clone, Debug, PartialEq)]
pub struct TargetEvent {
    pub kind: TargetEventKind,
    pub target: TargetInfo,
    pub session_id: Option<String>,
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

/// `Network.loadingFailed` data.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkFailedEvent {
    pub request_id: String,
    /// `"Document"`, `"Script"`, ... matching the request's own type.
    pub resource_type: String,
    pub error_text: String,
    pub canceled: bool,
    /// Monotonic seconds.
    pub timestamp: f64,
}

/// `Runtime.exceptionThrown` data: an [`ExceptionDetails`] plus the wall time
/// it was observed at (the event needs its own timestamp; `evaluate`'s
/// synchronous result does not).
#[derive(Clone, Debug, PartialEq)]
pub struct ExceptionEvent {
    pub details: ExceptionDetails,
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
    /// Layout and visual viewport geometry for `Page.getLayoutMetrics`.
    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        Err(HostError::Unsupported)
    }
    /// The frame tree annotated with each frame's resources.
    fn get_resource_tree(&mut self) -> Result<FrameResourceTree, HostError> {
        Err(HostError::Unsupported)
    }
    /// The body of one resource, by frame id and URL.
    fn get_resource_content(
        &mut self,
        _frame_id: &str,
        _url: &str,
    ) -> Result<ResourceContent, HostError> {
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
    /// Drops every remote object handle minted under `object_group`
    /// (`Runtime.releaseObjectGroup`) — the group an `evaluate`/
    /// `callFunctionOn` call's own `objectGroup` parameter tagged them with,
    /// or `""` (CDP's default/ungrouped bucket) when that parameter was
    /// absent.
    fn release_object_group(&mut self, _object_group: &str) -> Result<(), HostError> {
        Ok(())
    }
    /// Compiles a script and returns its `scriptId` (`Runtime.compileScript`).
    fn compile_script(
        &mut self,
        _source: &str,
        _url: Option<&str>,
        _persist: bool,
    ) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }
    /// Runs a script previously returned by `compileScript`.
    fn run_script(&mut self, _script_id: &str) -> Result<EvaluateResult, HostError> {
        Err(HostError::Unsupported)
    }
    /// The JS isolate id, if the engine exposes one.
    fn get_isolate_id(&mut self) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    /// Heap usage (`Runtime.getHeapUsage`).
    fn get_heap_usage(&mut self) -> Result<HeapUsage, HostError> {
        Err(HostError::Unsupported)
    }
    /// Names in the global lexical scope.
    fn global_lexical_scope_names(&mut self) -> Result<Vec<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// An object handle holding the objects whose prototype is
    /// `prototype_object_id` (`Runtime.queryObjects`).
    fn query_objects(
        &mut self,
        _prototype_object_id: &str,
    ) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Depth of async stack traces to capture.
    fn set_async_call_stack_depth(&mut self, _depth: u32) -> Result<(), HostError> {
        Ok(())
    }
    /// Drops buffered console entries (`Runtime.discardConsoleEntries`).
    fn discard_console_entries(&mut self) -> Result<(), HostError> {
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
    /// Replaces `node`'s entire outer HTML (`DOM.setOuterHTML`).
    fn set_outer_html(&mut self, _node: EngineNodeId, _html: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Detaches `node` from the tree (`DOM.removeNode`).
    fn remove_node(&mut self, _node: EngineNodeId) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Resolves an engine node from a `nodeId`/`backendNodeId` pair.
    fn resolve_node(&mut self, _backend_node_id: u64) -> Result<EngineNodeId, HostError> {
        Err(HostError::Unsupported)
    }
    /// The border-box quads for `node` (`DOM.getBoxModel`).
    fn get_box_model(&mut self, _node: EngineNodeId) -> Result<BoxModel, HostError> {
        Err(HostError::Unsupported)
    }
    /// Every content box overlapping `node` (`DOM.getContentQuads`).
    fn get_content_quads(&mut self, _node: EngineNodeId) -> Result<Vec<BoxModel>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Replaces the value of a text node or attribute-less node.
    fn set_node_value(&mut self, _node: EngineNodeId, _value: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Finds nodes whose text or attributes contain `query`
    /// (`DOM.performSearch`). The driver owns any result cache.
    fn perform_search(&mut self, _query: &str) -> Result<Vec<EngineNodeId>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Resolves several `backendNodeId`s at once.
    fn push_nodes(&mut self, _backend_node_ids: &[u64]) -> Result<Vec<EngineNodeId>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Hit-tests the document at viewport coordinates (`DOM.getNodeForLocation`).
    fn node_for_location(&mut self, _x: f64, _y: f64) -> Result<EngineNodeId, HostError> {
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
    /// The text of one stylesheet, by CDP style-sheet id.
    fn get_style_sheet_text(
        &mut self,
        _style_sheet_id: &str,
    ) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Replaces the text of one stylesheet (`CSS.setStyleSheetText`).
    fn set_style_sheet_text(
        &mut self,
        _style_sheet_id: &str,
        _text: &str,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Media queries in the document, in source order.
    fn get_media_queries(&mut self) -> Result<Vec<MediaQuery>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Background colors for `node`, cascade-resolved (`CSS.getBackgroundColors`).
    fn get_background_colors(
        &mut self,
        _node: EngineNodeId,
    ) -> Result<Vec<Option<String>>, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Input ------------------------------------------------------------
    /// Enables or suppresses engine input handling (`Input.setIgnoreInputEvents`).
    fn set_ignore_input_events(&mut self, _ignore: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Dispatches a synthesized mouse event.
    fn dispatch_mouse_event(&mut self, _event: &MouseEvent) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Dispatches a synthesized keyboard event.
    fn dispatch_key_event(&mut self, _event: &KeyEvent) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Dispatches a synthesized touch event.
    fn dispatch_touch_event(&mut self, _event: &TouchEvent) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Inserts text as if typed (`Input.insertText`).
    fn insert_text(&mut self, _text: &str) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }

    // --- Network ----------------------------------------------------------
    /// The decoded body of a finished response (`Network.getResponseBody` /
    /// `Fetch.getResponseBody`). `Ok(None)` means the body is gone or opaque.
    fn get_response_body(
        &mut self,
        _request_id: &str,
    ) -> Result<Option<ResourceContent>, HostError> {
        Err(HostError::Unsupported)
    }
    /// The posted body of a request (`Network.getRequestPostData`).
    fn get_request_post_data(&mut self, _request_id: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Disables/re-enables the HTTP cache for future requests
    /// (`Network.setCacheDisabled`). Advisory, like the `Emulation` hints
    /// below: a driver with no cache of its own still succeeds, since the
    /// *absence* of caching already satisfies "disabled".
    fn set_cache_disabled(&mut self, _disabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Replaces the extra headers sent on every future request
    /// (`Network.setExtraHTTPHeaders`) — a full replace each call, not a
    /// merge, matching CDP.
    fn set_extra_http_headers(&mut self, _headers: &[(String, String)]) -> Result<(), HostError> {
        Ok(())
    }

    // --- Fetch ------------------------------------------------------------
    /// Enables/disables request interception for the given patterns.
    fn set_fetch_enabled(
        &mut self,
        _enabled: bool,
        _patterns: &[FetchPattern],
        _handle_auth: bool,
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Requests the engine has paused and is waiting for a decision on. Must
    /// not block; default is no requests.
    fn drain_intercepted_requests(&mut self) -> Vec<InterceptedRequest> {
        Vec::new()
    }
    /// Applies the client's decision to a paused request, unblocking it.
    fn resolve_intercepted_request(
        &mut self,
        _request_id: &str,
        _decision: &FetchDecision,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }

    // --- Emulation --------------------------------------------------------
    //
    // These are advisory: a driver that does not implement one still succeeds,
    // because an emulation hint the engine cannot honor must not fail the CDP
    // command. Implementations apply the override to the engine's own viewport,
    // UA, locale, media or scheduler.
    /// Applies a device-metrics override (`Emulation.setDeviceMetricsOverride`).
    fn set_device_metrics_override(&mut self, _metrics: &DeviceMetrics) -> Result<(), HostError> {
        Ok(())
    }
    /// Clears the device-metrics override.
    fn clear_device_metrics_override(&mut self) -> Result<(), HostError> {
        Ok(())
    }
    /// Overrides the user agent string.
    fn set_user_agent_override(
        &mut self,
        _user_agent: &str,
        _accept_language: Option<&str>,
        _platform: Option<&str>,
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Overrides geolocation; `None` clears it.
    fn set_geolocation_override(
        &mut self,
        _position: Option<(f64, f64, f64)>,
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Overrides the time zone.
    fn set_timezone_override(&mut self, _timezone_id: &str) -> Result<(), HostError> {
        Ok(())
    }
    /// Overrides the locale; `None` clears it.
    fn set_locale_override(&mut self, _locale: Option<&str>) -> Result<(), HostError> {
        Ok(())
    }
    /// Emulates a media type and/or features.
    fn set_emulated_media(
        &mut self,
        _media: Option<&str>,
        _features: &[MediaFeature],
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Hides or shows scrollbars.
    fn set_scrollbars_hidden(&mut self, _hidden: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Enables or disables touch emulation.
    fn set_touch_emulation_enabled(
        &mut self,
        _enabled: bool,
        _max_touch_points: Option<u32>,
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Throttles the CPU by `rate` (`1.0` is unthrottled).
    fn set_cpu_throttling_rate(&mut self, _rate: f64) -> Result<(), HostError> {
        Ok(())
    }
    /// Enables or disables script execution.
    fn set_script_execution_disabled(&mut self, _value: bool) -> Result<(), HostError> {
        Ok(())
    }

    // --- Overlay ----------------------------------------------------------
    /// Enables or disables the inspector overlay.
    fn overlay_enabled(&mut self, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Sets the inspect mode (`"none"`, `"searchForNode"`, `"searchForUAShadowDOM"`,
    /// `"captureAreaScreenshot"`, `"showDistances"`).
    fn set_inspect_mode(
        &mut self,
        _mode: &str,
        _config: &HighlightConfig,
    ) -> Result<(), HostError> {
        Ok(())
    }
    /// Highlights one node with the given configuration.
    fn highlight_node(
        &mut self,
        _node: EngineNodeId,
        _config: &HighlightConfig,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Highlights a viewport rectangle.
    fn highlight_rect(&mut self, _rect: Rect, _config: &HighlightConfig) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Highlights a page quad.
    fn highlight_quad(&mut self, _quad: Quad, _config: &HighlightConfig) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Clears the current highlight.
    fn hide_highlight(&mut self) -> Result<(), HostError> {
        Ok(())
    }

    // --- Performance ------------------------------------------------------
    /// Named metric counters and gauges (`Performance.getMetrics`).
    fn performance_metrics(&mut self) -> Result<Vec<(String, f64)>, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Security ---------------------------------------------------------
    /// The current security state of the target.
    fn security_state(&mut self) -> Result<SecurityState, HostError> {
        Ok(SecurityState::default())
    }
    /// Security-state changes since the last poll
    /// (`Security.securityStateChanged`).
    fn drain_security_states(&mut self) -> Vec<SecurityState> {
        Vec::new()
    }
    /// Whether certificate errors are ignored.
    fn set_ignore_certificate_errors(&mut self, _ignore: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Resolves a certificate error (`eventId` plus the CDP action code).
    fn handle_certificate_error(&mut self, _event_id: i64, _action: i32) -> Result<(), HostError> {
        Ok(())
    }

    // --- Accessibility ------------------------------------------------------
    /// Enables/disables accessibility-tree tracking (`Accessibility.enable`/
    /// `disable`). Advisory; the driver may use it to start or stop building
    /// the tree.
    fn accessibility_enabled(&mut self, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// The full accessibility tree, root first (`Accessibility.getFullAXTree`).
    /// `depth` limits how many levels below the root are expanded; `None`
    /// means the whole tree.
    fn get_full_ax_tree(&mut self, _depth: Option<u32>) -> Result<Vec<AxNode>, HostError> {
        Err(HostError::Unsupported)
    }
    /// The accessibility subtree rooted at `node`, plus its ancestors when
    /// `fetch_relatives` is set (`Accessibility.getPartialAXTree`).
    fn get_partial_ax_tree(
        &mut self,
        _node: EngineNodeId,
        _fetch_relatives: bool,
    ) -> Result<Vec<AxNode>, HostError> {
        Err(HostError::Unsupported)
    }
    /// The direct children of an accessibility node by its `AXNodeId`
    /// (`Accessibility.getChildAXNodes`).
    fn get_child_ax_nodes(&mut self, _ax_node_id: &str) -> Result<Vec<AxNode>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Accessibility nodes under `node` (the whole tree when `None`) matching
    /// an optional accessible name and/or role (`Accessibility.queryAXTree`).
    fn query_ax_tree(
        &mut self,
        _node: Option<EngineNodeId>,
        _accessible_name: Option<&str>,
        _role: Option<&str>,
    ) -> Result<Vec<AxNode>, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Debugger -----------------------------------------------------------
    /// Enables/disables the debugger (`Debugger.enable`/`disable`). Advisory.
    fn debugger_enabled(&mut self, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Sets a breakpoint by URL pattern, returning its id plus every location
    /// it resolved to right now (`Debugger.setBreakpointByUrl`).
    fn set_breakpoint_by_url(
        &mut self,
        _url: &str,
        _line_number: u32,
        _column_number: Option<u32>,
        _condition: Option<&str>,
    ) -> Result<(String, Vec<ScriptLocation>), HostError> {
        Err(HostError::Unsupported)
    }
    /// Removes a breakpoint by id (`Debugger.removeBreakpoint`).
    fn remove_breakpoint(&mut self, _breakpoint_id: &str) -> Result<(), HostError> {
        Ok(())
    }
    /// Pauses at the next statement (`Debugger.pause`).
    fn debugger_pause(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Resumes execution (`Debugger.resume`).
    fn debugger_resume(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Steps into the next function call (`Debugger.stepInto`).
    fn debugger_step_into(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Steps over the current line (`Debugger.stepOver`).
    fn debugger_step_over(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Steps out of the current function (`Debugger.stepOut`).
    fn debugger_step_out(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Evaluates `expression` in the scope of `call_frame_id`
    /// (`Debugger.evaluateOnCallFrame`).
    fn evaluate_on_call_frame(
        &mut self,
        _call_frame_id: &str,
        _expression: &str,
    ) -> Result<EvaluateResult, HostError> {
        Err(HostError::Unsupported)
    }
    /// The full source text of a previously parsed script
    /// (`Debugger.getScriptSource`).
    fn get_script_source(&mut self, _script_id: &str) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }

    /// Exact native source units for Debugger.getScriptSource. Existing scalar
    /// hosts preserve their text; browser hosts override with native custody.
    fn get_script_source_utf16(&mut self, script_id: &str) -> Result<Vec<u16>, HostError> {
        self.get_script_source(script_id).map(|source| source.encode_utf16().collect())
    }

    // --- Profiler -----------------------------------------------------------
    /// Enables/disables the CPU profiler (`Profiler.enable`/`disable`).
    /// Advisory.
    fn profiler_enabled(&mut self, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Sets the sampling interval in microseconds
    /// (`Profiler.setSamplingInterval`).
    fn set_sampling_interval(&mut self, _microseconds: u32) -> Result<(), HostError> {
        Ok(())
    }
    /// Starts CPU profiling (`Profiler.start`).
    fn start_profiling(&mut self) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Stops CPU profiling and returns the collected profile
    /// (`Profiler.stop`).
    fn stop_profiling(&mut self) -> Result<CpuProfile, HostError> {
        Err(HostError::Unsupported)
    }

    // --- HeapProfiler ---------------------------------------------------------
    /// Enables/disables heap-object tracking (`HeapProfiler.enable`/
    /// `disable`). Advisory.
    fn heap_profiler_enabled(&mut self, _enabled: bool) -> Result<(), HostError> {
        Ok(())
    }
    /// Captures a heap snapshot, returned as one or more chunks of the
    /// snapshot's JSON text (`HeapProfiler.takeHeapSnapshot`, streamed as
    /// `addHeapSnapshotChunk` events by the domain layer).
    fn take_heap_snapshot(&mut self) -> Result<Vec<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Forces a garbage collection (`HeapProfiler.collectGarbage`).
    fn collect_garbage(&mut self) -> Result<(), HostError> {
        Ok(())
    }
    /// The heap snapshot object id for a `Runtime` `objectId`
    /// (`HeapProfiler.getHeapObjectId`).
    fn get_heap_object_id(&mut self, _object_id: &str) -> Result<String, HostError> {
        Err(HostError::Unsupported)
    }
    /// Resolves a heap snapshot object id back to a `Runtime.RemoteObject`
    /// (`HeapProfiler.getObjectByHeapObjectId`).
    fn get_object_by_heap_object_id(
        &mut self,
        _heap_object_id: &str,
        _object_group: Option<&str>,
    ) -> Result<RemoteObject, HostError> {
        Err(HostError::Unsupported)
    }

    // --- DOMSnapshot ----------------------------------------------------------
    /// A flattened snapshot of every document (`DOMSnapshot.captureSnapshot`).
    /// `computed_styles` lists the CSS property names to include per node;
    /// the driver may leave a node's `computed_styles` empty for properties
    /// it does not compute. This crate performs the wire format's string
    /// interning; the seam stays in plain structs.
    fn capture_dom_snapshot(
        &mut self,
        _computed_styles: &[String],
    ) -> Result<Vec<DomSnapshotDocument>, HostError> {
        Err(HostError::Unsupported)
    }

    // --- Storage --------------------------------------------------------------
    /// Every cookie visible to the target (`Storage.getCookies`).
    fn get_cookies(&mut self) -> Result<Vec<Cookie>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Replaces the cookie jar with `cookies` (`Storage.setCookies`).
    fn set_cookies(&mut self, _cookies: &[Cookie]) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Clears every cookie (`Storage.clearCookies`).
    fn clear_cookies(&mut self) -> Result<(), HostError> {
        Ok(())
    }
    /// Every `localStorage`/`sessionStorage` item for `storage_id`
    /// (`DOMStorage.getDOMStorageItems`).
    fn get_dom_storage_items(
        &mut self,
        _storage_id: &DomStorageId,
    ) -> Result<Vec<(String, String)>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Sets one item (`DOMStorage.setDOMStorageItem`).
    fn set_dom_storage_item(
        &mut self,
        _storage_id: &DomStorageId,
        _key: &str,
        _value: &str,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Removes one item (`DOMStorage.removeDOMStorageItem`).
    fn remove_dom_storage_item(
        &mut self,
        _storage_id: &DomStorageId,
        _key: &str,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Clears every item for `storage_id` (`DOMStorage.clear`).
    fn clear_dom_storage(&mut self, _storage_id: &DomStorageId) -> Result<(), HostError> {
        Ok(())
    }
    /// Every IndexedDB database name for `security_origin`
    /// (`IndexedDB.requestDatabaseNames`).
    fn request_database_names(&mut self, _security_origin: &str) -> Result<Vec<String>, HostError> {
        Err(HostError::Unsupported)
    }
    /// Clears one object store (`IndexedDB.clearObjectStore`).
    fn clear_object_store(
        &mut self,
        _security_origin: &str,
        _database_name: &str,
        _object_store_name: &str,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Deletes a whole database (`IndexedDB.deleteDatabase`).
    fn delete_database(
        &mut self,
        _security_origin: &str,
        _database_name: &str,
    ) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }

    // --- Tracing ------------------------------------------------------------
    /// The category names the engine can trace (`Tracing.getCategories`).
    fn tracing_categories(&mut self) -> Result<Vec<String>, HostError> {
        Ok(Vec::new())
    }
    /// Starts tracing the given categories, or every category when empty
    /// (`Tracing.start`).
    fn start_tracing(&mut self, _categories: &[String]) -> Result<(), HostError> {
        Err(HostError::Unsupported)
    }
    /// Stops tracing (`Tracing.end`). Collected events drain through
    /// [`DevToolsSession::drain_trace_events`].
    fn stop_tracing(&mut self) -> Result<(), HostError> {
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
    /// `Network.loadingFailed`.
    fn drain_network_failed(&mut self) -> Vec<NetworkFailedEvent> {
        Vec::new()
    }
    /// `Runtime.exceptionThrown` — uncaught errors from event handlers,
    /// timers and other callbacks the page didn't itself catch (distinct from
    /// `Runtime.evaluate`'s own synchronous `exceptionDetails`).
    fn drain_exceptions(&mut self) -> Vec<ExceptionEvent> {
        Vec::new()
    }
    /// `Log.entryAdded`.
    fn drain_log(&mut self) -> Vec<LogEntry> {
        Vec::new()
    }
    /// Scripts parsed since the last poll (`Debugger.scriptParsed`).
    fn drain_scripts_parsed(&mut self) -> Vec<ScriptInfo> {
        Vec::new()
    }
    /// Pause events since the last poll (`Debugger.paused`). The connection
    /// emits `Debugger.resumed` directly from `Debugger.resume`, not through
    /// a drain queue.
    fn drain_debugger_paused(&mut self) -> Vec<PausedEvent> {
        Vec::new()
    }
    /// Buffered Trace Event Format entries since the last poll, batched into
    /// `Tracing.dataCollected`.
    fn drain_trace_events(&mut self) -> Vec<crate::json::Json> {
        Vec::new()
    }
    /// Accessibility nodes changed since the last poll
    /// (`Accessibility.nodesUpdated`).
    fn drain_ax_nodes_updated(&mut self) -> Vec<AxNode> {
        Vec::new()
    }
    /// The accessibility tree's root, once, the first time it becomes ready
    /// after `Accessibility.enable` (`Accessibility.loadComplete`; CDP-25).
    /// Real Chrome fires this once per page load, mirroring the load-complete
    /// event a browser sends assistive technology; this driver's honest
    /// stand-in fires it once per `enable` against whatever tree is current
    /// at that moment, since it has no separate "page finished loading"
    /// signal of its own to key off instead. `None` when nothing is queued
    /// (already drained, or `enable` was never called).
    fn drain_ax_load_complete(&mut self) -> Option<AxNode> {
        None
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
    /// Target-list changes since the previous call, for the browser endpoint's
    /// `Target.*` lifecycle events. Must not block; default is no events.
    fn drain_target_events(&self) -> Vec<TargetEvent> {
        Vec::new()
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
