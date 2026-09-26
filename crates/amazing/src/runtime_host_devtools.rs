//! Driver adoption of the CDP host seam (`amazing-devtools::host`), CDP-16.
//!
//! This is the devtools lane's own driver-adapter file (see AGENTS.md "Driver
//! adapter per lane"). integrator-assist landed the first slice
//! (`Page.navigate`/`getFrameTree`/`getLayoutMetrics`, `Runtime.evaluate`) with
//! a documented gap: `evaluate` built a *fresh* JS realm per call, because the
//! `amazing-js` heap is `Rc`-based (single-threaded execution is a locked
//! decision) while `DevToolsSession: Send` and a CDP connection owns its
//! session on its own thread.
//!
//! CDP-16 fixes that by giving the page a single owning thread instead of
//! trying to make the engine itself `Send`. [`PageOwner`] holds the live DOM
//! (`Rc<RefCell<BrowserDomHost>>`) and a persistent [`DriverScheduler`] (one JS
//! realm, kept alive across calls) and never leaves the thread
//! [`spawn_page_thread`] starts for it. [`DriverDevToolsHost`] and
//! [`DriverPageSession`] are thin, `Send`/`Sync` handles that talk to that
//! thread over an `std::sync::mpsc` channel of [`PageCommand`]s, each carrying
//! its own one-shot reply channel — the same request/response shape a
//! multi-process browser uses between its DevTools and renderer processes,
//! done here with threads and channels because Amazing is still
//! single-process (ADR "no-sandbox/no-multi-process non-goal", superseded by
//! ADR 0005 but not yet implemented).
//!
//! `Runtime.evaluate`, `Page.navigate`, `DOM.getDocument`/`querySelector(All)`
//! and `Accessibility.getFullAXTree` all now go through that one channel to
//! the one page thread, so they see the same live DOM and the same JS realm
//! (globals, closures) across calls — the regression test
//! `evaluate_persists_full_js_realm_state_across_calls` is the proof.
//!
//! CDP-17 makes the session usable by a real DevTools frontend: `DOM`
//! mutation (`describeNode`, `getOuterHTML`, `setAttributeValue`,
//! `setOuterHTML`, `removeNode`, `requestChildNodes`), comma-separated
//! selector lists, `CSS` computed/matched styles and stylesheet text,
//! `Input.dispatchMouseEvent`/`dispatchKeyEvent` into the page's real event
//! dispatch, and `Page.captureScreenshot` as PNG through the existing
//! `amazing render` raster path (`amazing_canvas::encode_png`, reused rather
//! than duplicated — see its doc comment for the stored-DEFLATE zlib it
//! writes).
//!
//! ## Known limitations (follow-ups, not silently claimed as done)
//!
//! * `Page.captureScreenshot` only supports `format: "png"`; `jpeg`/`webp`
//!   stay `Unsupported` (no encoder for either in the workspace). `clip` crops
//!   the rendered surface; `quality`/`captureBeyondViewport`/`fromSurface`
//!   are accepted but not honored beyond the existing raster path's own
//!   viewport sizing.
//! * `Input.dispatchMouseEvent` hit-tests the *normal-flow and positioned*
//!   layout tree (`amazing_layout::Flow::Blocks/Flex/Grid` plus
//!   `LayoutBox::positioned`, topmost-child-wins) to resolve a target node
//!   from `(x, y)`; it does not descend into `Flow::Inlines` line boxes, so a
//!   click inside inline text resolves to the nearest block/inline-container
//!   ancestor, not the exact inline run. The dispatched DOM event carries no
//!   coordinate/button detail properties — no binding-host API accepts a rich
//!   init dict at the `dispatch_dom_event` layer this driver already uses for
//!   scripted event dispatch.
//! * `Input.dispatchKeyEvent` dispatches to `Dom::active_element`, falling
//!   back to the document root when nothing is focused; likewise carries no
//!   key/text detail properties on the constructed event object.
//! * `CSS.getMatchedStylesForNode` walks the page's own author stylesheets
//!   (`amazing_css::selectors::matches` against each rule's
//!   `selector_srcs`) and reports every match as `StyleOrigin::Author`; the
//!   user-agent/built-in sheet is not separately exposed, so UA-only rules
//!   are not listed.
//! * `CSS.getStyleSheetText` uses the 0-based index into the page's collected
//!   author stylesheets as an ad-hoc `styleSheetId` (this driver does not
//!   otherwise mint or track stylesheet ids) and reconstructs CSS text from
//!   parsed `selector_srcs`/`declarations` — an honest re-serialization, not
//!   the original source bytes (the parser does not retain those).
//! * `DOM.setOuterHTML` re-parses the replacement HTML with
//!   `amazing_html::parse_fragment` in the *parent* element's tag-name
//!   context (or `"body"` if the parent isn't an element) and splices the
//!   result in at the original position; it does not attempt the spec's
//!   exact context-element algorithm for every edge case (e.g. `<template>`
//!   contents, foreign-namespace contexts).
//! * `Accessibility.getFullAXTree` (like `amazing axtree`) lays out with
//!   `amazing_text::FallbackTextSystem` and `amazing_layout::NoReplacedElements`,
//!   so replaced content and real glyph metrics are not reflected — the same
//!   documented gap as the a11y lane's own CLI entry point. The same fallback
//!   layout is used for hit-testing and for computed style's box geometry.
//! * `depth` on `Accessibility.getFullAXTree` is ignored (the whole tree is
//!   always returned), matching the CDP `-1` convention this crate already
//!   normalizes `depth` to at the domain layer.
use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use amazing_devtools::host::{
    AxNode, DevToolsHost, DevToolsSession, EngineNodeId, EvaluateParams, EvaluateResult,
    ExceptionDetails, FrameInfo, FrameTree, HostError, KeyEvent, LayoutMetrics, MatchedRule,
    MouseEvent, NodeDescription, RemoteObject, Screenshot, ScreenshotFormat, ScreenshotParams,
    StyleOrigin, TargetInfo, Viewport,
};
use amazing_devtools::server::CdpServer;

use crate::dom_host::BrowserDomHost;
use crate::js_scheduler::DriverScheduler;

/// Reply channel for one [`PageCommand`].
type Reply<T> = Sender<Result<T, HostError>>;

/// One request the session/host side sends to the page thread. Every variant
/// is `Send` (plain owned data plus a `Sender`, itself `Send` when its payload
/// is), which is what lets [`DriverPageSession`] satisfy `DevToolsSession: Send`
/// without the engine itself being `Send`.
enum PageCommand {
    Navigate(String, Reply<FrameInfo>),
    FrameTree(Reply<FrameTree>),
    LayoutMetrics(Reply<LayoutMetrics>),
    Evaluate(EvaluateParams, Reply<EvaluateResult>),
    GetDocument(u32, Reply<NodeDescription>),
    QuerySelector(EngineNodeId, String, Reply<Option<EngineNodeId>>),
    QuerySelectorAll(EngineNodeId, String, Reply<Vec<EngineNodeId>>),
    FullAxTree(Reply<Vec<AxNode>>),
    DescribeNode(EngineNodeId, u32, Reply<NodeDescription>),
    GetOuterHtml(EngineNodeId, Reply<String>),
    SetAttributeValue(EngineNodeId, String, Option<String>, Reply<()>),
    SetOuterHtml(EngineNodeId, String, Reply<()>),
    RemoveNode(EngineNodeId, Reply<()>),
    ComputedStyleForNode(EngineNodeId, Reply<Vec<(String, String)>>),
    MatchedStylesForNode(EngineNodeId, Reply<Vec<MatchedRule>>),
    StyleSheetText(String, Reply<Option<String>>),
    DispatchMouseEvent(MouseEvent, Reply<()>),
    DispatchKeyEvent(KeyEvent, Reply<()>),
    CaptureScreenshot(ScreenshotParams, Reply<Screenshot>),
    Shutdown,
}

/// Sends `make(reply_tx)` to the page thread and blocks for its answer. The
/// only failure mode besides the command's own `HostError` is the page thread
/// having already exited (e.g. the host was dropped mid-call).
fn request<T>(sender: &Sender<PageCommand>, make: impl FnOnce(Reply<T>) -> PageCommand) -> Result<T, HostError> {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    sender
        .send(make(reply_tx))
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?;
    reply_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread is gone".into()))?
}

fn origin_of(url: &str) -> String {
    amazing_net::Url::parse(url)
        .map(|parsed| format!("{}://{}", parsed.scheme, parsed.host))
        .unwrap_or_else(|_| url.to_owned())
}

fn frame_info(target_id: &str, url: &str) -> FrameInfo {
    FrameInfo {
        id: target_id.to_owned(),
        parent_id: None,
        url: url.to_owned(),
        name: String::new(),
        security_origin: origin_of(url),
        mime_type: "text/html".into(),
    }
}

fn js_value_to_remote_object(value: &amazing_js::value::JsValue) -> RemoteObject {
    use amazing_js::value::JsValue;
    match value {
        JsValue::Undefined => RemoteObject::undefined(),
        JsValue::Null => RemoteObject::null(),
        JsValue::Bool(b) => RemoteObject::boolean(*b),
        JsValue::Num(n) => RemoteObject::number(*n),
        JsValue::Str(s) => RemoteObject::string(s.to_utf8_lossy()),
        // No live-object bridge yet (`Runtime.getProperties`, previews, object
        // ids that outlive the call): report identity and a debug description
        // only. Real pages' `evaluate()` calls are overwhelmingly primitives;
        // this is a documented gap, not a silent wrong answer.
        JsValue::BigInt(_) | JsValue::Symbol(_) | JsValue::Object(_) => RemoteObject {
            object_type: match value {
                JsValue::BigInt(_) => "bigint",
                JsValue::Symbol(_) => "symbol",
                _ => "object",
            }
            .to_string(),
            subtype: None,
            class_name: None,
            value: None,
            unserializable_value: None,
            description: Some(format!("{value:?}")),
            object_id: None,
        },
    }
}

fn base_description(node: EngineNodeId, node_type: u16, name: &str, value: &str) -> NodeDescription {
    NodeDescription {
        node,
        parent: None,
        backend_node_id: node.0,
        node_type,
        node_name: name.to_owned(),
        local_name: String::new(),
        node_value: value.to_owned(),
        attributes: Vec::new(),
        child_node_count: 0,
        children: Vec::new(),
    }
}

/// Builds a CDP `DOM.Node` tree from the live arena DOM, expanding `depth`
/// levels of children (0 = the node itself only). `node_type` numbering
/// follows the DOM spec (`Node.ELEMENT_NODE` etc), matching what
/// `amazing-devtools`' own `NodeDescription` helpers assume.
fn describe_node(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, depth: u32) -> NodeDescription {
    use amazing_dom::NodeKind;
    let engine = EngineNodeId(node.0 as u64);
    let children: Vec<amazing_dom::NodeIdx> = dom.children(node).collect();
    let mut desc = match &dom.get(node).kind {
        NodeKind::Document => base_description(engine, 9, "#document", ""),
        NodeKind::DocumentFragment => base_description(engine, 11, "#document-fragment", ""),
        NodeKind::DocumentType { name, .. } => base_description(engine, 10, name, ""),
        NodeKind::Element { name, attrs, .. } => {
            NodeDescription::element(engine, name.clone()).with_attributes(attrs.clone())
        }
        NodeKind::Text { data } => NodeDescription::text(engine, data.clone()),
        NodeKind::Comment { data } => base_description(engine, 8, "#comment", data),
        NodeKind::CDATASection { data } => base_description(engine, 4, "#cdata-section", data),
        NodeKind::ProcessingInstruction { target, data } => base_description(engine, 7, target, data),
        NodeKind::Attr { .. } => base_description(engine, 2, "", ""),
    };
    desc.child_node_count = children.len();
    if depth > 0 {
        desc.children = children
            .into_iter()
            .map(|child| describe_node(dom, child, depth - 1))
            .collect();
    }
    desc
}

/// Splits a CDP/CSS selector list on top-level commas, respecting
/// parentheses, brackets and quoted strings (`:is(a, b)`, `[data-x="a,b"]`),
/// so `DOM.querySelector(All)` accepts the same comma-separated lists a page's
/// own CSS does.
fn split_selector_list(selector: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0usize;
    for (index, ch) in selector.char_indices() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(selector[start..index].trim());
                    start = index + 1;
                }
                _ => {}
            },
        }
    }
    parts.push(selector[start..].trim());
    parts
}

fn parse_selector_list(selector: &str) -> Result<Vec<amazing_css::selectors::Selector>, HostError> {
    split_selector_list(selector)
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| {
            amazing_css::selectors::parse_selector(part)
                .ok_or_else(|| HostError::InvalidParams(format!("bad selector: {part}")))
        })
        .collect()
}

fn matches_any(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
) -> bool {
    selectors.iter().any(|selector| amazing_css::selectors::matches(dom, node, selector))
}

/// Copies `source_node`'s subtree (from a standalone fragment `Dom`, as
/// `amazing_html::parse_fragment` returns) into `target`, appended under
/// `parent`. Mirrors `amazing-html`'s own `parse_fragment::clone_subtree`
/// (private to that crate), since there is no cross-`Dom` "adopt" primitive.
fn clone_subtree_into(
    source: &amazing_dom::Dom,
    source_node: amazing_dom::NodeIdx,
    target: &mut amazing_dom::Dom,
    parent: amazing_dom::NodeIdx,
) -> amazing_dom::NodeIdx {
    let created = target.append_child(parent, source.get(source_node).kind.clone());
    for child in source.children(source_node) {
        clone_subtree_into(source, child, target, created);
    }
    created
}

fn find_first(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
) -> Option<EngineNodeId> {
    if matches_any(dom, node, selectors) {
        return Some(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        if let Some(found) = find_first(dom, child, selectors) {
            return Some(found);
        }
    }
    None
}

fn find_all(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    selectors: &[amazing_css::selectors::Selector],
    out: &mut Vec<EngineNodeId>,
) {
    if matches_any(dom, node, selectors) {
        out.push(EngineNodeId(node.0 as u64));
    }
    for child in dom.children(node) {
        find_all(dom, child, selectors, out);
    }
}

/// Serializes `node`'s outer HTML, reusing the same spec markup serializer
/// (`Dom::serialize_node`) the JS `outerHTML` getter binding uses
/// (`amazing-bindings/src/markup_bindings.rs`), so DevTools and scripted
/// access agree.
fn outer_html(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx) -> Result<String, HostError> {
    dom.serialize_node(node)
        .map(|units| String::from_utf16_lossy(&units))
        .map_err(|error| HostError::Internal(format!("{error:?}")))
}

/// Crops a rendered RGBA8 surface to `clip` (`Page.captureScreenshot`'s
/// viewport-relative clip rect), clamped to the surface's own bounds.
fn crop_surface(surface: &amazing_paint::Surface, clip: &amazing_devtools::host::Rect) -> amazing_paint::Surface {
    let x = (clip.x.max(0.0) as u32).min(surface.width);
    let y = (clip.y.max(0.0) as u32).min(surface.height);
    let w = (clip.width.max(0.0) as u32).min(surface.width.saturating_sub(x));
    let h = (clip.height.max(0.0) as u32).min(surface.height.saturating_sub(y));
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for row in y..y + h {
        let start = (row as usize * surface.width as usize + x as usize) * 4;
        let end = start + w as usize * 4;
        pixels.extend_from_slice(&surface.pixels[start..end]);
    }
    amazing_paint::Surface { width: w, height: h, pixels }
}

/// Depth-first, topmost-child-wins point-in-box hit test over a laid-out
/// page. Checks absolutely/fixed-positioned descendants (`positioned`, which
/// paint above normal flow) before normal-flow children, and walks
/// `Flow::Blocks`/`Flex`/`Grid` children in reverse (later paint order wins).
/// Does not descend into `Flow::Inlines` line boxes — see the module's
/// "Known limitations".
fn hit_test<'a>(box_: &'a amazing_layout::LayoutBox, x: f32, y: f32) -> Option<&'a amazing_layout::LayoutBox> {
    // Positioned descendants are checked unconditionally, before this box's
    // own bounds: an absolutely/fixed-positioned box commonly extends beyond
    // the shrink-to-fit box of the ancestor that hoisted it (e.g. a
    // `position: absolute` child of an otherwise-empty `<body>`), so gating
    // on the ancestor's own `border_box` first would reject points that are
    // only inside the positioned descendant.
    for child in box_.positioned.iter().rev() {
        if let Some(hit) = hit_test(child, x, y) {
            return Some(hit);
        }
    }
    let b = &box_.border_box;
    if x < b.x || y < b.y || x > b.x + b.w || y > b.y + b.h {
        return None;
    }
    match &box_.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => {
            for child in children.iter().rev() {
                if let Some(hit) = hit_test(child, x, y) {
                    return Some(hit);
                }
            }
        }
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => {}
    }
    Some(box_)
}

/// Converts the a11y lane's own tree into the CDP-shaped seam, matching the
/// pattern `amazing-devtools::domains::accessibility` documents: this crate
/// stays independent of `amazing-a11y`, so the conversion lives entirely on
/// the driver side.
fn convert_ax_tree(tree: &amazing_a11y::AxTree) -> Vec<AxNode> {
    tree.nodes()
        .iter()
        .map(|node| AxNode {
            ax_node_id: node.id.0.to_string(),
            ignored: false,
            role: Some(node.role.as_str().to_owned()),
            name: (!node.name.is_empty()).then(|| node.name.clone()),
            description: (!node.description.is_empty()).then(|| node.description.clone()),
            value: None,
            node: node.dom_node.map(|idx| EngineNodeId(idx.0 as u64)),
            child_ids: node.children.iter().map(|id| id.0.to_string()).collect(),
        })
        .collect()
}

/// The one piece of state that must live on a single thread: the live DOM and
/// a persistent JS realm bound to it. Not `Send` (the JS heap is `Rc`-based),
/// so it never leaves the thread [`spawn_page_thread`] gives it.
struct PageOwner {
    dom_host: Rc<RefCell<BrowserDomHost>>,
    scheduler: DriverScheduler,
    target_id: String,
    url: String,
    width: f32,
}

impl PageOwner {
    /// Builds a page owner from an already-parsed document: installs the JS
    /// realm, runs every `<script>`/inline handler and dispatches
    /// `DOMContentLoaded`/`load`, exactly as `amazing render`/`render-page` do
    /// (`force_scheduler: true`, so even a script-less page still gets a
    /// realm for `Runtime.evaluate`).
    fn from_parsed(target_id: &str, url: &str, dom: amazing_dom::Dom, width: f32) -> Result<Self, HostError> {
        let dom_host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let scheduler = crate::cmd::js::prepare_page_scripts(&dom_host, url, true)
            .map_err(HostError::Internal)?
            .expect("force_scheduler=true always yields a scheduler");
        Ok(Self { dom_host, scheduler, target_id: target_id.to_owned(), url: url.to_owned(), width })
    }

    fn fetch_and_load(target_id: &str, url: &str, width: f32) -> Result<Self, HostError> {
        let (final_url, html, content_type) =
            crate::cmd::real_pages::fetch_document(url).map_err(HostError::Internal)?;
        let document = crate::document::parse_document_bytes(&html, &final_url, content_type.as_deref())
            .map_err(|error| HostError::Internal(error.to_string()))?;
        Self::from_parsed(target_id, &final_url, document.dom, width)
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let reloaded = Self::fetch_and_load(&self.target_id, url, self.width)?;
        *self = reloaded;
        Ok(frame_info(&self.target_id, &self.url))
    }

    fn frame_tree(&self) -> FrameTree {
        FrameTree { frame: frame_info(&self.target_id, &self.url), child_frames: Vec::new() }
    }

    fn layout_metrics(&self) -> Result<LayoutMetrics, HostError> {
        let surface = crate::cmd::render::paint_host_surface(&self.dom_host, &self.url, self.width, None, "")
            .map_err(|error| HostError::Internal(error.to_string()))?;
        let viewport = Viewport {
            page_x: 0.0,
            page_y: 0.0,
            client_width: self.width as f64,
            client_height: surface.height as f64,
        };
        Ok(LayoutMetrics {
            layout_viewport: viewport,
            visual_viewport: viewport,
            content_width: surface.width as f64,
            content_height: surface.height as f64,
        })
    }

    /// Runs `expression` in this page's persistent realm. Unlike the first
    /// slice, this is the *same* `DriverScheduler` (and so the same globals
    /// and closures) across every call for this page's lifetime.
    fn evaluate(&mut self, params: &EvaluateParams) -> EvaluateResult {
        match self.scheduler.run_script(&params.expression) {
            Ok(value) => EvaluateResult::value(js_value_to_remote_object(&value)),
            Err(thrown) => EvaluateResult {
                result: js_value_to_remote_object(&thrown.0),
                exception: Some(ExceptionDetails {
                    text: thrown.describe(),
                    line_number: 0,
                    column_number: 0,
                    url: None,
                    exception: Some(js_value_to_remote_object(&thrown.0)),
                    stack_trace: None,
                }),
            },
        }
    }

    fn get_document(&self, depth: u32) -> NodeDescription {
        let host = self.dom_host.borrow();
        describe_node(&host.dom, amazing_dom::NodeIdx(0), depth)
    }

    fn query_selector(&self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let parsed = parse_selector_list(selector)?;
        let host = self.dom_host.borrow();
        Ok(find_first(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed))
    }

    fn query_selector_all(&self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let parsed = parse_selector_list(selector)?;
        let host = self.dom_host.borrow();
        let mut out = Vec::new();
        find_all(&host.dom, amazing_dom::NodeIdx(node.0 as u32), &parsed, &mut out);
        Ok(out)
    }

    fn describe_node(&self, node: EngineNodeId, depth: u32) -> NodeDescription {
        let host = self.dom_host.borrow();
        describe_node(&host.dom, amazing_dom::NodeIdx(node.0 as u32), depth)
    }

    fn get_outer_html(&self, node: EngineNodeId) -> Result<String, HostError> {
        let host = self.dom_host.borrow();
        outer_html(&host.dom, amazing_dom::NodeIdx(node.0 as u32))
    }

    fn set_attribute_value(&mut self, node: EngineNodeId, name: &str, value: Option<&str>) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let result = match value {
            Some(value) => host.dom.set_attribute(idx, name, value),
            None => host.dom.remove_attribute(idx, name),
        };
        result.map_err(|error| HostError::Internal(format!("{error:?}")))
    }

    /// Reparses `html` in the parent element's tag-name context (falling
    /// back to `"body"` when the parent isn't an element, e.g. the document
    /// root), splices the result in at `node`'s original position, then
    /// detaches `node`. See the module doc's "Known limitations".
    fn set_outer_html(&mut self, node: EngineNodeId, html: &str) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let entry = host.dom.get(idx);
        let parent = entry
            .parent
            .ok_or_else(|| HostError::InvalidParams("node has no parent to splice into".into()))?;
        let next_sibling = entry.next_sibling;
        let context = match &host.dom.get(parent).kind {
            amazing_dom::NodeKind::Element { name, .. } => name.clone(),
            _ => "body".to_owned(),
        };
        let fragment = amazing_html::parse_fragment(&context, html)
            .map_err(HostError::Internal)?;
        let children: Vec<amazing_dom::NodeIdx> = fragment.children(amazing_dom::NodeIdx(0)).collect();
        for child in children {
            let created = clone_subtree_into(&fragment, child, &mut host.dom, parent);
            host.dom
                .detach(created);
            host.dom
                .pre_insert(created, parent, next_sibling)
                .map_err(|error| HostError::Internal(format!("{error:?}")))?;
        }
        host.dom.detach(idx);
        Ok(())
    }

    fn remove_node(&mut self, node: EngineNodeId) -> Result<(), HostError> {
        let mut host = self.dom_host.borrow_mut();
        host.dom.detach(amazing_dom::NodeIdx(node.0 as u32));
        Ok(())
    }

    fn media_environment(&self) -> amazing_css::MediaEnvironment {
        amazing_css::MediaEnvironment { viewport_width_px: self.width.max(1.0), ..amazing_css::MediaEnvironment::default() }
    }

    /// The page's own `<style>`/`<link rel=stylesheet>` sheets, in document
    /// order — the same collection `full_ax_tree` and `layout_metrics` (via
    /// `paint_host_surface`) use.
    fn author_sheets(&self, dom: &amazing_dom::Dom) -> Vec<amazing_css::Stylesheet> {
        let mut sources = Vec::new();
        let _ = crate::page::collect_author_css(dom, amazing_dom::NodeIdx(0), &self.url, &mut sources);
        sources.into_iter().map(|(_, _, sheet)| sheet).collect()
    }

    fn computed_style_for_node(&self, node: EngineNodeId) -> Result<Vec<(String, String)>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let sheets = self.author_sheets(dom);
        let styled = amazing_css::compute_with_environment(dom, &sheets, &self.media_environment());
        let style = styled
            .get(amazing_dom::NodeIdx(node.0 as u32))
            .ok_or(HostError::NotFound)?;
        Ok(style.iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect())
    }

    /// Matched rules from the page's own author stylesheets. Every match is
    /// reported as `StyleOrigin::Author` — see the module's "Known
    /// limitations" on the missing user-agent sheet.
    fn matched_styles_for_node(&self, node: EngineNodeId) -> Result<Vec<MatchedRule>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let idx = amazing_dom::NodeIdx(node.0 as u32);
        let sheets = self.author_sheets(dom);
        let mut matched = Vec::new();
        for sheet in &sheets {
            for rule in &sheet.rules {
                for selector_src in &rule.selector_srcs {
                    let Some(selector) = amazing_css::selectors::parse_selector(selector_src) else { continue };
                    if amazing_css::selectors::matches(dom, idx, &selector) {
                        matched.push(MatchedRule {
                            selector: selector_src.clone(),
                            origin: StyleOrigin::Author,
                            properties: rule
                                .declarations
                                .iter()
                                .map(|decl| (decl.property.clone(), decl.value.clone()))
                                .collect(),
                        });
                    }
                }
            }
        }
        Ok(matched)
    }

    /// `style_sheet_id` is the 0-based index into this page's own collected
    /// author stylesheets (see the module's "Known limitations"); the text is
    /// re-serialized from parsed rules, not the original source bytes.
    fn style_sheet_text(&self, style_sheet_id: &str) -> Result<Option<String>, HostError> {
        let index: usize = style_sheet_id
            .parse()
            .map_err(|_| HostError::InvalidParams(format!("bad styleSheetId: {style_sheet_id}")))?;
        let host = self.dom_host.borrow();
        let sheets = self.author_sheets(&host.dom);
        let Some(sheet) = sheets.get(index) else { return Ok(None) };
        let mut text = String::new();
        for rule in &sheet.rules {
            if rule.selector_srcs.is_empty() {
                continue;
            }
            text.push_str(&rule.selector_srcs.join(", "));
            text.push_str(" {\n");
            for decl in &rule.declarations {
                text.push_str("  ");
                text.push_str(&decl.property);
                text.push_str(": ");
                text.push_str(&decl.value);
                if decl.important {
                    text.push_str(" !important");
                }
                text.push_str(";\n");
            }
            text.push_str("}\n");
        }
        Ok(Some(text))
    }

    /// Computes style + the same fallback layout `full_ax_tree` uses, then
    /// hit-tests it. Returns `HostError::NotFound` when `(x, y)` misses every
    /// box (e.g. outside the page).
    fn hit_test_node(&self, x: f64, y: f64) -> Result<EngineNodeId, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let sheets = self.author_sheets(dom);
        let styled = amazing_css::compute_with_environment(dom, &sheets, &self.media_environment());
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_styled_document(
            dom, &styled, &text, &amazing_layout::NoReplacedElements, self.width, None,
        );
        hit_test(&layout, x as f32, y as f32)
            .and_then(|box_| box_.node)
            .map(|node| EngineNodeId(node.0 as u64))
            .ok_or(HostError::NotFound)
    }

    fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
        let dom_event = match event.kind.as_str() {
            "mousePressed" => "mousedown",
            "mouseReleased" => "mouseup",
            "mouseMoved" => "mousemove",
            "mouseWheel" => "wheel",
            other => other,
        };
        let target = self.hit_test_node(event.x, event.y)?;
        self.scheduler
            .dispatch_dom_event(target.0 as u32, dom_event)
            .map_err(|thrown| HostError::Internal(thrown.describe()))
    }

    fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
        let dom_event = match event.kind.as_str() {
            "keyDown" | "rawKeyDown" => "keydown",
            "keyUp" => "keyup",
            "char" => "keypress",
            other => other,
        };
        let target = {
            let host = self.dom_host.borrow();
            host.dom
                .active_element(amazing_dom::NodeIdx(0))
                .unwrap_or(amazing_dom::NodeIdx(0))
        };
        self.scheduler
            .dispatch_dom_event(target.0, dom_event)
            .map_err(|thrown| HostError::Internal(thrown.describe()))
    }

    /// Renders through the same `paint_host_surface` path `amazing
    /// render`/`render-page` use, then encodes the RGBA8 surface as PNG with
    /// `amazing_canvas::encode_png` (reused, not duplicated).
    fn capture_screenshot(&self, params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        if !matches!(params.format, ScreenshotFormat::Png) {
            return Err(HostError::Unsupported);
        }
        let mut surface = crate::cmd::render::paint_host_surface(&self.dom_host, &self.url, self.width, None, "")
            .map_err(|error| HostError::Internal(error.to_string()))?;
        if let Some(clip) = &params.clip {
            surface = crop_surface(&surface, clip);
        }
        let image = amazing_canvas::ImageData::from_rgba(surface.width, surface.height, surface.pixels)
            .map_err(|error| HostError::Internal(format!("{error:?}")))?;
        Ok(Screenshot { data: amazing_canvas::encode_png(&image), format: ScreenshotFormat::Png })
    }

    /// Computes style + layout (the same minimal `FallbackTextSystem` /
    /// `NoReplacedElements` path `amazing axtree` uses) on the live DOM and
    /// builds the accessibility tree through the a11y lane's own driver
    /// adapter, per the ticket's "through `runtime_host_a11y::build_ax_tree`".
    fn full_ax_tree(&self) -> Result<Vec<AxNode>, HostError> {
        let host = self.dom_host.borrow();
        let dom = &host.dom;
        let mut sources = Vec::new();
        let _ = crate::page::collect_author_css(dom, amazing_dom::NodeIdx(0), &self.url, &mut sources);
        let sheets: Vec<amazing_css::Stylesheet> = sources.into_iter().map(|(_, _, sheet)| sheet).collect();
        let styled = amazing_css::compute_with_environment(
            dom,
            &sheets,
            &amazing_css::MediaEnvironment {
                viewport_width_px: self.width.max(1.0),
                ..amazing_css::MediaEnvironment::default()
            },
        );
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_styled_document(
            dom,
            &styled,
            &text,
            &amazing_layout::NoReplacedElements,
            self.width,
            None,
        );
        let tree = crate::runtime_host_a11y::build_ax_tree(dom, &styled, &layout);
        Ok(convert_ax_tree(&tree))
    }
}

/// What [`spawn_page_thread`] needs to build the page's [`PageOwner`] — kept
/// separate from `PageOwner` itself so the *construction inputs* (plain
/// `Send` data: strings, an `amazing_dom::Dom`, a width) cross the thread
/// boundary in the `spawn` closure, while the engine state `PageOwner` builds
/// (the `Rc`-based JS heap) never does. `std::thread::spawn` requires the
/// closure to be `Send`, which only constrains what it *captures*, not what
/// it constructs after starting — so `PageOwner::from_parsed`/`fetch_and_load`
/// run inside the spawned thread, not before it.
enum PageInit {
    Parsed { target_id: String, url: String, dom: amazing_dom::Dom, width: f32 },
    Fetch { target_id: String, url: String, width: f32 },
}

impl PageInit {
    fn load(self) -> Result<PageOwner, HostError> {
        match self {
            PageInit::Parsed { target_id, url, dom, width } => PageOwner::from_parsed(&target_id, &url, dom, width),
            PageInit::Fetch { target_id, url, width } => PageOwner::fetch_and_load(&target_id, &url, width),
        }
    }
}

fn run_page_thread(mut page: PageOwner, rx: Receiver<PageCommand>) {
    while let Ok(command) = rx.recv() {
        match command {
            PageCommand::Navigate(url, reply) => {
                let _ = reply.send(page.navigate(&url));
            }
            PageCommand::FrameTree(reply) => {
                let _ = reply.send(Ok(page.frame_tree()));
            }
            PageCommand::LayoutMetrics(reply) => {
                let _ = reply.send(page.layout_metrics());
            }
            PageCommand::Evaluate(params, reply) => {
                let _ = reply.send(Ok(page.evaluate(&params)));
            }
            PageCommand::GetDocument(depth, reply) => {
                let _ = reply.send(Ok(page.get_document(depth)));
            }
            PageCommand::QuerySelector(node, selector, reply) => {
                let _ = reply.send(page.query_selector(node, &selector));
            }
            PageCommand::QuerySelectorAll(node, selector, reply) => {
                let _ = reply.send(page.query_selector_all(node, &selector));
            }
            PageCommand::FullAxTree(reply) => {
                let _ = reply.send(page.full_ax_tree());
            }
            PageCommand::DescribeNode(node, depth, reply) => {
                let _ = reply.send(Ok(page.describe_node(node, depth)));
            }
            PageCommand::GetOuterHtml(node, reply) => {
                let _ = reply.send(page.get_outer_html(node));
            }
            PageCommand::SetAttributeValue(node, name, value, reply) => {
                let _ = reply.send(page.set_attribute_value(node, &name, value.as_deref()));
            }
            PageCommand::SetOuterHtml(node, html, reply) => {
                let _ = reply.send(page.set_outer_html(node, &html));
            }
            PageCommand::RemoveNode(node, reply) => {
                let _ = reply.send(page.remove_node(node));
            }
            PageCommand::ComputedStyleForNode(node, reply) => {
                let _ = reply.send(page.computed_style_for_node(node));
            }
            PageCommand::MatchedStylesForNode(node, reply) => {
                let _ = reply.send(page.matched_styles_for_node(node));
            }
            PageCommand::StyleSheetText(style_sheet_id, reply) => {
                let _ = reply.send(page.style_sheet_text(&style_sheet_id));
            }
            PageCommand::DispatchMouseEvent(event, reply) => {
                let _ = reply.send(page.dispatch_mouse_event(&event));
            }
            PageCommand::DispatchKeyEvent(event, reply) => {
                let _ = reply.send(page.dispatch_key_event(&event));
            }
            PageCommand::CaptureScreenshot(params, reply) => {
                let _ = reply.send(page.capture_screenshot(&params));
            }
            PageCommand::Shutdown => break,
        }
    }
}

/// Starts the dedicated page thread and returns the channel to talk to it,
/// plus the loaded page's final URL. Blocks until the initial
/// parse/script/load finishes (or fails), so a bad URL is reported to the
/// caller synchronously rather than silently wedging every later command. The
/// thread runs until it receives [`PageCommand::Shutdown`] (sent by
/// [`DriverDevToolsHost`]'s `Drop`), so it never outlives its host.
fn spawn_page_thread(init: PageInit) -> Result<(Sender<PageCommand>, String), HostError> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<String, HostError>>();
    std::thread::Builder::new()
        .name("devtools-page".into())
        .spawn(move || match init.load() {
            Ok(page) => {
                let _ = ready_tx.send(Ok(page.url.clone()));
                run_page_thread(page, rx);
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .expect("spawn devtools page thread");
    let url = ready_rx
        .recv()
        .map_err(|_| HostError::Internal("devtools page thread died before starting".into()))??;
    Ok((tx, url))
}

/// One attached page target: a `Send` handle to the page thread. It holds no
/// engine state itself, so it trivially satisfies `DevToolsSession: Send`
/// regardless of how many worker threads a `CdpServer` connection spawns.
pub(crate) struct DriverPageSession {
    target_id: String,
    sender: Sender<PageCommand>,
    current_url: Arc<Mutex<String>>,
}

impl DevToolsSession for DriverPageSession {
    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn navigate(&mut self, url: &str) -> Result<FrameInfo, HostError> {
        let frame = request(&self.sender, |reply| PageCommand::Navigate(url.to_owned(), reply))?;
        if let Ok(mut current) = self.current_url.lock() {
            *current = frame.url.clone();
        }
        Ok(frame)
    }

    fn frame_tree(&mut self) -> Result<FrameTree, HostError> {
        request(&self.sender, PageCommand::FrameTree)
    }

    fn get_layout_metrics(&mut self) -> Result<LayoutMetrics, HostError> {
        request(&self.sender, PageCommand::LayoutMetrics)
    }

    fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::Evaluate(params, reply))
    }

    fn get_document(&mut self, depth: u32) -> Result<NodeDescription, HostError> {
        request(&self.sender, |reply| PageCommand::GetDocument(depth, reply))
    }

    fn query_selector(&mut self, node: EngineNodeId, selector: &str) -> Result<Option<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelector(node, selector, reply))
    }

    fn query_selector_all(&mut self, node: EngineNodeId, selector: &str) -> Result<Vec<EngineNodeId>, HostError> {
        let selector = selector.to_owned();
        request(&self.sender, |reply| PageCommand::QuerySelectorAll(node, selector, reply))
    }

    fn get_full_ax_tree(&mut self, _depth: Option<u32>) -> Result<Vec<AxNode>, HostError> {
        request(&self.sender, PageCommand::FullAxTree)
    }

    fn describe_node(&mut self, node: EngineNodeId, depth: u32) -> Result<NodeDescription, HostError> {
        request(&self.sender, |reply| PageCommand::DescribeNode(node, depth, reply))
    }

    fn get_outer_html(&mut self, node: EngineNodeId) -> Result<String, HostError> {
        request(&self.sender, |reply| PageCommand::GetOuterHtml(node, reply))
    }

    fn set_attribute_value(&mut self, node: EngineNodeId, name: &str, value: Option<&str>) -> Result<(), HostError> {
        let name = name.to_owned();
        let value = value.map(str::to_owned);
        request(&self.sender, |reply| PageCommand::SetAttributeValue(node, name, value, reply))
    }

    fn set_outer_html(&mut self, node: EngineNodeId, html: &str) -> Result<(), HostError> {
        let html = html.to_owned();
        request(&self.sender, |reply| PageCommand::SetOuterHtml(node, html, reply))
    }

    fn remove_node(&mut self, node: EngineNodeId) -> Result<(), HostError> {
        request(&self.sender, |reply| PageCommand::RemoveNode(node, reply))
    }

    fn get_computed_style_for_node(&mut self, node: EngineNodeId) -> Result<Vec<(String, String)>, HostError> {
        request(&self.sender, |reply| PageCommand::ComputedStyleForNode(node, reply))
    }

    fn get_matched_styles_for_node(&mut self, node: EngineNodeId) -> Result<Vec<MatchedRule>, HostError> {
        request(&self.sender, |reply| PageCommand::MatchedStylesForNode(node, reply))
    }

    fn get_style_sheet_text(&mut self, style_sheet_id: &str) -> Result<Option<String>, HostError> {
        let style_sheet_id = style_sheet_id.to_owned();
        request(&self.sender, |reply| PageCommand::StyleSheetText(style_sheet_id, reply))
    }

    fn dispatch_mouse_event(&mut self, event: &MouseEvent) -> Result<(), HostError> {
        let event = event.clone();
        request(&self.sender, |reply| PageCommand::DispatchMouseEvent(event, reply))
    }

    fn dispatch_key_event(&mut self, event: &KeyEvent) -> Result<(), HostError> {
        let event = event.clone();
        request(&self.sender, |reply| PageCommand::DispatchKeyEvent(event, reply))
    }

    fn capture_screenshot(&mut self, params: &ScreenshotParams) -> Result<Screenshot, HostError> {
        let params = params.clone();
        request(&self.sender, |reply| PageCommand::CaptureScreenshot(params, reply))
    }
}

/// The browser-level seam: one page target, backed by a real page (fetched or,
/// for tests, parsed directly) running on its own thread.
pub(crate) struct DriverDevToolsHost {
    target_id: String,
    sender: Mutex<Sender<PageCommand>>,
    current_url: Arc<Mutex<String>>,
}

impl DriverDevToolsHost {
    fn spawn(target_id: impl Into<String>, init: PageInit) -> Result<Self, HostError> {
        let (sender, url) = spawn_page_thread(init)?;
        Ok(Self { target_id: target_id.into(), sender: Mutex::new(sender), current_url: Arc::new(Mutex::new(url)) })
    }
}

impl Drop for DriverDevToolsHost {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(PageCommand::Shutdown);
        }
    }
}

impl DevToolsHost for DriverDevToolsHost {
    fn list_targets(&self) -> Vec<TargetInfo> {
        let url = self.current_url.lock().expect("devtools url poisoned").clone();
        vec![TargetInfo::page(self.target_id.clone(), url.clone(), url)]
    }

    fn attach(&self, target_id: &str) -> Result<Box<dyn DevToolsSession>, HostError> {
        if target_id != self.target_id {
            return Err(HostError::NotFound);
        }
        let sender = self.sender.lock().expect("devtools sender poisoned").clone();
        Ok(Box::new(DriverPageSession {
            target_id: self.target_id.clone(),
            sender,
            current_url: self.current_url.clone(),
        }))
    }
}

/// A running CDP server on its own thread, stopped on drop.
pub(crate) struct DevToolsHandle {
    pub(crate) addr: SocketAddr,
    pub(crate) browser_id: String,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for DevToolsHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl DevToolsHandle {
    /// Blocks the calling thread until the server thread exits. Nothing in
    /// this handle sets the shutdown flag itself, so this is for a foreground
    /// CLI command that should serve until the process is killed, not for
    /// code that wants to stop the server (drop the handle for that).
    pub(crate) fn wait(mut self) -> io::Result<()> {
        match self.join.take() {
            Some(join) => join.join().unwrap_or(Ok(())),
            None => Ok(()),
        }
    }
}

/// Fetches and parses `initial_url`, starts its dedicated page thread, then
/// binds and starts a CDP server for it on `addr` (`"127.0.0.1:0"` for an
/// ephemeral port) on a background thread.
pub(crate) fn spawn_devtools_server(initial_url: &str, addr: &str) -> Result<DevToolsHandle, String> {
    let init = PageInit::Fetch { target_id: "page-1".into(), url: initial_url.to_owned(), width: 1024.0 };
    let host = Arc::new(DriverDevToolsHost::spawn("page-1", init).map_err(|error| error.to_string())?);
    let server = CdpServer::bind(host, addr).map_err(|error| format!("bind {addr}: {error}"))?;
    let resolved_addr = server.local_addr().map_err(|error| error.to_string())?;
    let browser_id = server.browser_id().to_owned();
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = shutdown.clone();
    let join = std::thread::spawn(move || server.serve(worker_shutdown));
    Ok(DevToolsHandle { addr: resolved_addr, browser_id, shutdown, join: Some(join) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_devtools::json::Json;
    use amazing_devtools::ws::{Message, WsConnection};
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    /// Builds a page thread directly from an in-memory document, bypassing
    /// network I/O, exactly like `prepare_page_scripts` production path
    /// (`force_scheduler: true`).
    fn spawn_test_host(url: &str, html: &str) -> Arc<DriverDevToolsHost> {
        let document =
            crate::document::parse_document_bytes(html.as_bytes(), url, None).expect("parse fixture document");
        let init = PageInit::Parsed { target_id: "page-1".into(), url: url.to_owned(), dom: document.dom, width: 800.0 };
        Arc::new(DriverDevToolsHost::spawn("page-1", init).expect("spawn devtools host"))
    }

    /// A one-shot local HTTP server for exercising the real `navigate()`
    /// path (which always fetches, matching production) without reaching the
    /// network: everything stays on `127.0.0.1`.
    fn serve_once(html: &'static str) -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = html.as_bytes();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        addr
    }

    #[test]
    fn attach_unknown_target_is_not_found() {
        let host = spawn_test_host("http://example.test/fixture.html", "<title>fixture</title>");
        match host.attach("no-such-target") {
            Err(HostError::NotFound) => {}
            other => panic!("expected HostError::NotFound, got {}", other.is_ok()),
        }
    }

    #[test]
    fn navigate_fetches_a_real_page_and_evaluate_reads_the_new_dom() {
        let host = spawn_test_host("http://example.test/start.html", "<title>start</title><body>a</body>");
        let mut session = host.attach("page-1").expect("attach");
        let addr = serve_once("<title>second</title><body>b</body>");
        let frame = session.navigate(&format!("http://{addr}/")).expect("navigate");
        assert!(frame.url.contains(&addr.to_string()));
        let tree = session.frame_tree().expect("frame_tree");
        assert_eq!(tree.frame.url, frame.url);

        let result = session
            .evaluate(&EvaluateParams { expression: "document.title".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert_eq!(result.result, RemoteObject::string("second"));
    }

    #[test]
    fn evaluate_persists_full_js_realm_state_across_calls() {
        // The bug CDP-16 fixes: the first slice built a fresh `Interpreter`
        // per `evaluate()` call, so `globalThis` state never survived between
        // calls even though the DOM (plain owned data) did. With one page
        // thread and one persistent `DriverScheduler`, it now does.
        let host = spawn_test_host("http://example.test/doc.html", "<title>t</title><body></body>");
        let mut session = host.attach("page-1").expect("attach");
        let expr = "globalThis.counter = (globalThis.counter || 0) + 1; globalThis.counter";
        let first = session
            .evaluate(&EvaluateParams { expression: expr.into(), ..EvaluateParams::default() })
            .expect("evaluate #1");
        assert_eq!(first.result, RemoteObject::number(1.0));
        let second = session
            .evaluate(&EvaluateParams { expression: expr.into(), ..EvaluateParams::default() })
            .expect("evaluate #2");
        assert_eq!(second.result, RemoteObject::number(2.0), "globalThis state should survive across evaluate() calls");
    }

    #[test]
    fn evaluate_reports_a_thrown_exception_instead_of_erroring_the_call() {
        let host = spawn_test_host("http://example.test/doc.html", "<title>t</title>");
        let mut session = host.attach("page-1").expect("attach");
        let result = session
            .evaluate(&EvaluateParams { expression: "undefinedGlobal.member".into(), ..EvaluateParams::default() })
            .expect("evaluate");
        assert!(result.exception.is_some());
    }

    #[test]
    fn get_document_and_query_selector_see_the_live_dom() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>t</title><body><h1 id=\"h\">Hi</h1><p class=\"x\">one</p><p class=\"x\">two</p></body>",
        );
        let mut session = host.attach("page-1").expect("attach");
        let root = session.get_document(64).expect("get_document");
        assert_eq!(root.node_type, 9, "document root should be nodeType 9");

        let found = session
            .query_selector(root.node, "#h")
            .expect("query_selector")
            .expect("#h should resolve to a node");
        let all = session.query_selector_all(root.node, ".x").expect("query_selector_all");
        assert_eq!(all.len(), 2, "both <p class=x> nodes should be found");
        assert_ne!(found, EngineNodeId(0));
    }

    #[test]
    fn get_full_ax_tree_reflects_the_live_page() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>t</title><body><h1>Hello</h1><button>Go</button></body>",
        );
        let mut session = host.attach("page-1").expect("attach");
        let nodes = session.get_full_ax_tree(None).expect("get_full_ax_tree");
        assert!(nodes.iter().any(|node| node.role.as_deref() == Some("heading")), "{nodes:?}");
        assert!(nodes.iter().any(|node| node.role.as_deref() == Some("button")), "{nodes:?}");
    }

    /// A minimal scripted CDP WebSocket client, mirroring the pattern
    /// `amazing-devtools`' own `tests/cdp.rs` uses against `MockHost`. This is
    /// the "prove it" test the ticket asks for: a real client, real JSON-RPC
    /// framing over a real `WsConnection`, against the real page thread.
    struct Client {
        ws: WsConnection<TcpStream>,
        next_id: u64,
    }

    impl Client {
        fn connect(addr: SocketAddr, path: &str) -> Self {
            let mut stream = TcpStream::connect(addr).expect("connect");
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let key = "dGhlIHNhbXBsZSBub25jZQ==";
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).unwrap();
            stream.flush().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).expect("handshake");
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.starts_with("HTTP/1.1 101"), "unexpected handshake: {head}");
            let expected = amazing_devtools::sha1::websocket_accept(key);
            assert!(
                head.to_ascii_lowercase().contains(&format!("sec-websocket-accept: {}", expected.to_ascii_lowercase())),
                "bad accept header: {head}"
            );
            Self { ws: WsConnection::as_client(stream), next_id: 0 }
        }

        fn recv_json(&mut self) -> Json {
            match self.ws.poll() {
                Ok(Some(Message::Text(text))) => Json::parse(&text).expect("server json"),
                Ok(Some(Message::Binary(_))) => panic!("unexpected binary frame"),
                Ok(None) => panic!("timed out waiting for a server message"),
                Err(error) => panic!("websocket error: {error}"),
            }
        }

        fn result(&mut self, method: &str, params: Json) -> Json {
            self.next_id += 1;
            let id = self.next_id;
            let message = Json::Object(vec![
                ("id".into(), Json::Number(id as f64)),
                ("method".into(), Json::String(method.into())),
                ("params".into(), params),
            ]);
            self.ws.write_message(&message.to_string()).expect("write message");
            loop {
                let reply = self.recv_json();
                if reply.get("id").and_then(Json::as_f64) == Some(id as f64) {
                    return reply
                        .get("result")
                        .cloned()
                        .unwrap_or_else(|| panic!("{method} returned no result: {reply}"));
                }
                // Not our reply (a push event) — keep waiting.
            }
        }
    }

    #[test]
    fn end_to_end_cdp_client_evaluates_queries_dom_and_reads_the_ax_tree() {
        let host = spawn_test_host(
            "http://example.test/doc.html",
            "<title>Live</title><body><h1 id=\"h\">Hello</h1><button>Go</button></body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");

        // Runtime.evaluate against the live realm.
        let evaluate = client.result(
            "Runtime.evaluate",
            Json::Object(vec![
                ("expression".into(), Json::String("21 * 2".into())),
                ("returnByValue".into(), Json::Bool(true)),
            ]),
        );
        assert_eq!(
            evaluate.get("result").and_then(|result| result.get("value")).and_then(Json::as_f64),
            Some(42.0),
            "{evaluate}"
        );

        // DOM.getDocument + DOM.querySelector/querySelectorAll on the live DOM.
        let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
        let root_node_id = document
            .get("root")
            .and_then(|root| root.get("nodeId"))
            .and_then(Json::as_f64)
            .expect("root nodeId") as u64;

        let query = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_node_id as f64)),
                ("selector".into(), Json::String("#h".into())),
            ]),
        );
        let found_node_id = query.get("nodeId").and_then(Json::as_f64).unwrap_or(0.0);
        assert!(found_node_id > 0.0, "#h should resolve to a real node: {query}");

        let heading_query = client.result(
            "DOM.querySelectorAll",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_node_id as f64)),
                ("selector".into(), Json::String("h1".into())),
            ]),
        );
        assert_eq!(
            heading_query.get("nodeIds").and_then(Json::as_array).map(<[Json]>::len),
            Some(1),
            "{heading_query}"
        );

        // Accessibility.getFullAXTree through runtime_host_a11y::build_ax_tree.
        let ax_tree = client.result("Accessibility.getFullAXTree", Json::Object(Vec::new()));
        let nodes = ax_tree.get("nodes").and_then(Json::as_array).expect("nodes array");
        let has_role = |role: &str| {
            nodes.iter().any(|node| node.get("role").and_then(|r| r.get("value")).and_then(Json::as_str) == Some(role))
        };
        assert!(has_role("heading"), "{nodes:?}");
        assert!(has_role("button"), "{nodes:?}");

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    /// CDP-17: `DOM.describeNode`/`getOuterHTML`/`setAttributeValue`/
    /// `setOuterHTML`/`removeNode`, plus comma-separated selector lists in
    /// `querySelectorAll`, all through a real WebSocket client against the
    /// live DOM.
    #[test]
    fn end_to_end_dom_mutation_and_selector_lists_work_through_the_real_websocket_client() {
        let host = spawn_test_host(
            "http://example.test/dom.html",
            "<title>Dom</title><body><div id=\"container\"><span id=\"target\" class=\"x\">old</span><p class=\"x\">p</p></div></body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");
        let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
        let root_id = document
            .get("root")
            .and_then(|root| root.get("nodeId"))
            .and_then(Json::as_f64)
            .expect("root nodeId") as u64;

        // Comma-separated selector list: the <span class=x> and <p class=x>.
        let all = client.result(
            "DOM.querySelectorAll",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_id as f64)),
                ("selector".into(), Json::String("span.x, p.x".into())),
            ]),
        );
        assert_eq!(
            all.get("nodeIds").and_then(Json::as_array).map(<[Json]>::len),
            Some(2),
            "{all}"
        );

        let target = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_id as f64)),
                ("selector".into(), Json::String("#target".into())),
            ]),
        );
        let target_id = target.get("nodeId").and_then(Json::as_f64).expect("target nodeId") as u64;
        assert!(target_id > 0, "{target}");

        let describe = client.result(
            "DOM.describeNode",
            Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
        );
        assert_eq!(
            describe.get("node").and_then(|node| node.get("nodeName")).and_then(Json::as_str),
            Some("SPAN"),
            "{describe}"
        );

        let outer = client.result(
            "DOM.getOuterHTML",
            Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
        );
        let outer_html = outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
        assert!(outer_html.contains("old"), "{outer_html}");

        client.result(
            "DOM.setAttributeValue",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(target_id as f64)),
                ("name".into(), Json::String("data-x".into())),
                ("value".into(), Json::String("1".into())),
            ]),
        );
        let outer_after_attr = client.result(
            "DOM.getOuterHTML",
            Json::Object(vec![("nodeId".into(), Json::Number(target_id as f64))]),
        );
        let outer_after_attr = outer_after_attr.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
        assert!(outer_after_attr.contains("data-x=\"1\""), "{outer_after_attr}");

        client.result(
            "DOM.setOuterHTML",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(target_id as f64)),
                ("outerHTML".into(), Json::String("<span id=\"target\">new</span>".into())),
            ]),
        );
        let requeried = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_id as f64)),
                ("selector".into(), Json::String("#target".into())),
            ]),
        );
        let requeried_id = requeried.get("nodeId").and_then(Json::as_f64).expect("requeried nodeId") as u64;
        assert!(requeried_id > 0, "{requeried}");
        let new_outer = client.result(
            "DOM.getOuterHTML",
            Json::Object(vec![("nodeId".into(), Json::Number(requeried_id as f64))]),
        );
        let new_outer = new_outer.get("outerHTML").and_then(Json::as_str).unwrap_or_default();
        assert!(new_outer.contains("new") && !new_outer.contains("old"), "{new_outer}");

        client.result(
            "DOM.removeNode",
            Json::Object(vec![("nodeId".into(), Json::Number(requeried_id as f64))]),
        );
        let gone = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_id as f64)),
                ("selector".into(), Json::String("#target".into())),
            ]),
        );
        assert_eq!(gone.get("nodeId").and_then(Json::as_f64), Some(0.0), "{gone}");

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    /// CDP-17: `CSS.getComputedStyleForNode`, `getMatchedStylesForNode` (the
    /// matched rules from `amazing-css`) and `getStyleSheetText`.
    #[test]
    fn end_to_end_css_computed_matched_styles_and_stylesheet_text() {
        let host = spawn_test_host(
            "http://example.test/css.html",
            "<title>Css</title><style>#head { color: red; }</style><body><h1 id=\"head\">Hi</h1></body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");
        let document = client.result("DOM.getDocument", Json::Object(vec![("depth".into(), Json::Number(-1.0))]));
        let root_id = document
            .get("root")
            .and_then(|root| root.get("nodeId"))
            .and_then(Json::as_f64)
            .expect("root nodeId") as u64;
        let head = client.result(
            "DOM.querySelector",
            Json::Object(vec![
                ("nodeId".into(), Json::Number(root_id as f64)),
                ("selector".into(), Json::String("#head".into())),
            ]),
        );
        let head_id = head.get("nodeId").and_then(Json::as_f64).expect("head nodeId") as u64;
        assert!(head_id > 0, "{head}");

        let computed = client.result(
            "CSS.getComputedStyleForNode",
            Json::Object(vec![("nodeId".into(), Json::Number(head_id as f64))]),
        );
        let computed = computed.get("computedStyle").and_then(Json::as_array).expect("computedStyle array");
        let color = computed
            .iter()
            .find(|entry| entry.get("name").and_then(Json::as_str) == Some("color"))
            .unwrap_or_else(|| panic!("no color property in {computed:?}"));
        assert!(
            color.get("value").and_then(Json::as_str).unwrap_or_default().contains("red"),
            "{color}"
        );

        let matched = client.result(
            "CSS.getMatchedStylesForNode",
            Json::Object(vec![("nodeId".into(), Json::Number(head_id as f64))]),
        );
        let rules = matched.get("matchedCSSRules").and_then(Json::as_array).expect("matchedCSSRules array");
        assert!(!rules.is_empty(), "{rules:?}");
        let first_rule = rules[0].get("rule").expect("rule field");
        assert_eq!(first_rule.get("origin").and_then(Json::as_str), Some("author"), "{first_rule}");
        assert!(
            first_rule
                .get("selectorList")
                .and_then(|list| list.get("text"))
                .and_then(Json::as_str)
                .unwrap_or_default()
                .contains("head"),
            "{first_rule}"
        );

        let sheet_text = client.result(
            "CSS.getStyleSheetText",
            Json::Object(vec![("styleSheetId".into(), Json::String("0".into()))]),
        );
        let text = sheet_text.get("text").and_then(Json::as_str).unwrap_or_default();
        assert!(text.contains("color") && text.contains("red"), "{text}");

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    /// CDP-17: `Input.dispatchMouseEvent`/`dispatchKeyEvent` reach real
    /// `addEventListener` handlers on the live page (hit-tested for the mouse
    /// event, `document.activeElement` for the key event).
    #[test]
    fn end_to_end_input_dispatch_reaches_real_dom_event_listeners() {
        let host = spawn_test_host(
            "http://example.test/input.html",
            "<title>Input</title><body>\
             <button id=\"btn\" style=\"position:absolute;left:0;top:0;width:200px;height:200px\">Go</button>\
             <input id=\"field\">\
             <script>\
               document.getElementById('btn').addEventListener('mousedown', () => { window.mouseHit = true; });\
               document.getElementById('field').addEventListener('keydown', () => { window.keyHit = true; });\
             </script>\
             </body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");

        client.result(
            "Input.dispatchMouseEvent",
            Json::Object(vec![
                ("type".into(), Json::String("mousePressed".into())),
                ("x".into(), Json::Number(50.0)),
                ("y".into(), Json::Number(50.0)),
                ("button".into(), Json::String("left".into())),
                ("buttons".into(), Json::Number(1.0)),
                ("clickCount".into(), Json::Number(1.0)),
            ]),
        );
        let mouse_hit = client.result(
            "Runtime.evaluate",
            Json::Object(vec![
                ("expression".into(), Json::String("window.mouseHit === true".into())),
                ("returnByValue".into(), Json::Bool(true)),
            ]),
        );
        assert_eq!(
            mouse_hit.get("result").and_then(|result| result.get("value")).and_then(Json::as_bool),
            Some(true),
            "{mouse_hit}"
        );

        client.result(
            "Runtime.evaluate",
            Json::Object(vec![(
                "expression".into(),
                Json::String("document.getElementById('field').focus()".into()),
            )]),
        );
        client.result(
            "Input.dispatchKeyEvent",
            Json::Object(vec![
                ("type".into(), Json::String("keyDown".into())),
                ("key".into(), Json::String("a".into())),
                ("code".into(), Json::String("KeyA".into())),
            ]),
        );
        let key_hit = client.result(
            "Runtime.evaluate",
            Json::Object(vec![
                ("expression".into(), Json::String("window.keyHit === true".into())),
                ("returnByValue".into(), Json::Bool(true)),
            ]),
        );
        assert_eq!(
            key_hit.get("result").and_then(|result| result.get("value")).and_then(Json::as_bool),
            Some(true),
            "{key_hit}"
        );

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    /// CDP-17: `Page.captureScreenshot` returns a real, decodable PNG through
    /// the existing raster path.
    #[test]
    fn end_to_end_capture_screenshot_returns_a_valid_png() {
        let host = spawn_test_host(
            "http://example.test/shot.html",
            "<title>Shot</title><body style=\"background:white\"><h1>Hello</h1></body>",
        );
        let server = CdpServer::bind(host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut client = Client::connect(addr, "/devtools/page/page-1");
        let shot = client.result(
            "Page.captureScreenshot",
            Json::Object(vec![("format".into(), Json::String("png".into()))]),
        );
        let data = shot.get("data").and_then(Json::as_str).expect("data field");
        let bytes = amazing_devtools::base64::decode(data).expect("valid base64");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG signature: {bytes:?}");
        assert!(bytes.len() > 100, "suspiciously small PNG: {} bytes", bytes.len());

        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }

    #[test]
    fn spawn_devtools_server_serves_json_version_over_the_bound_port() {
        let html_host = spawn_test_host("http://example.test/wired.html", "<title>wired</title>");
        let server = CdpServer::bind(html_host, "127.0.0.1:0").expect("bind");
        let addr = server.local_addr().expect("local_addr");
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let join = std::thread::spawn(move || server.serve(worker_shutdown));

        let mut response = String::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut last_error = None;
        while std::time::Instant::now() < deadline {
            match TcpStream::connect(addr) {
                Ok(mut stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let _ = stream.write_all(
                        format!("GET /json/version HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
                    );
                    let _ = stream.read_to_string(&mut response);
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        assert!(response.contains("200"), "no /json/version response; last connect error: {last_error:?}, got: {response}");
        shutdown.store(true, Ordering::Relaxed);
        let _ = join.join();
    }
}
