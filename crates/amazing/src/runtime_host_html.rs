//! Child HTML navigables owned by the driver. Each connected iframe gets its
//! own same-agent realm and stable WindowProxy through JSRT-9
//! (binding_create_realm/binding_create_window_proxy/binding_with_realm);
//! DOM interface prototypes stay arena-shared, so a child realm's `document`
//! wraps the same Document/Element prototypes as the top realm rather than a
//! fully independent intrinsic set.
use super::*;
use crate::dom_host::BrowserDomHost;
use amazing_dom::document_surface::ReadyState;
use amazing_net::url::{ParsedUrl, UrlOrigin};
use amazing_fetch::{HttpUrl, HeaderList, check_frame_embedding,
    api::{ApiRequest, NavigableContext},
    service::{NetworkService, ServiceEvent}};
use amazing_net::dispatch::DispatchLimits;
use amazing_js::interp::RealmHandle;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) type FrameResourceResolver = Rc<dyn Fn(&str) -> Option<String>>;
pub(crate) type ChildNavigationHook = Rc<dyn Fn(ChildNavigation)>;
pub(crate) type FormNavigationHook = Rc<dyn Fn(FormNavigationRequest)>;

#[derive(Clone, Debug)]
pub(crate) struct ChildNavigation {
    pub(crate) frame: NodeIdx,
    pub(crate) requested_src: Option<String>,
    pub(crate) committed_url: String,
    pub(crate) document: Option<NodeIdx>,
}

struct FrameDocument {
    src: Option<String>,
    srcdoc: Option<String>,
    document: Option<NodeIdx>,
    url: String,
    sandbox: SandboxFlags,
    /// HTML-34: the decoded HTML source still waiting to be parsed, for an
    /// HTML (never XML/XHTML) child document -- `navigate_iframes` only
    /// creates an *empty* document node and stashes this; `setup_iframe_
    /// window`'s deferred tick does the actual script-interleaved parse
    /// (`run_interleaved_iframe_parse`), so a script sees the document
    /// built so far, not the finished tree. `None` for an XML/XHTML child
    /// (still built eagerly and fully in `navigate_iframes`, since
    /// `ResumableParser` is HTML-only) or once this document's interleaved
    /// parse has already consumed it.
    html: Option<String>,
}

/// The `sandbox` attribute's recognized tokens, ASCII-case-insensitive and
/// space-separated. Absent attribute means not sandboxed at all (full
/// access, matching an ordinary same-origin child). Tokens this driver does
/// not otherwise enforce (`allow-forms`, `allow-popups`,
/// `allow-top-navigation`, ...) are accepted but currently no-ops, since the
/// features they'd gate (form-target navigation restrictions, popups,
/// top-level navigation from a child) either don't exist yet or aren't
/// enforced here; see docs/lanes/html_TODO.md.
#[derive(Clone, Copy, Default)]
pub(crate) struct SandboxFlags {
    pub(crate) sandboxed: bool,
    pub(crate) allow_scripts: bool,
    pub(crate) allow_same_origin: bool,
}

fn parse_sandbox(attr: Option<&str>) -> SandboxFlags {
    let Some(attr) = attr else { return SandboxFlags::default() };
    let mut flags = SandboxFlags { sandboxed: true, allow_scripts: false, allow_same_origin: false };
    for token in attr.split_ascii_whitespace() {
        match token.to_ascii_lowercase().as_str() {
            "allow-scripts" => flags.allow_scripts = true,
            "allow-same-origin" => flags.allow_same_origin = true,
            _ => {}
        }
    }
    flags
}

/// Bits 0-2 of `NavigableContext::sandbox_flags`; amazing-fetch only checks
/// that the field fits in 16 bits (frame-embedding plumbing owned by the
/// `net` lane), so this driver-local encoding is ours to define.
impl SandboxFlags {
    fn to_bits(self) -> u32 {
        (self.sandboxed as u32) | (self.allow_scripts as u32) << 1 | (self.allow_same_origin as u32) << 2
    }
}

#[derive(Default)]
pub(super) struct FrameRuntime {
    documents: HashMap<NodeIdx, FrameDocument>,
    contexts: HashMap<RealmHandle, WindowContext>,
    parser_imports: HashMap<NodeIdx, IncrementalImportRef>,
    /// `(frame, document, fire_load_events)`. `fire_load_events` is false
    /// only for an iframe with neither `src` nor `srcdoc`: its initial
    /// `about:blank` is created for `contentDocument`/`contentWindow` to see,
    /// but per spec no `load` event is queued for it (WPT's
    /// `iframe-nosrc.html` treats any iframe-level `load` before the page's
    /// own as a failure).
    pending_loads: VecDeque<(NodeIdx, Option<NodeIdx>, bool)>,
    resolver: Option<FrameResourceResolver>,
    navigation_hook: Option<ChildNavigationHook>,
    form_navigation_hook: Option<FormNavigationHook>,
    /// The stable WindowProxy (and its current target realm) for each
    /// connected iframe. The proxy identity survives re-navigation; only the
    /// realm it targets changes, via binding_retarget_window_proxy.
    windows: HashMap<NodeIdx, (RealmHandle, JsValue)>,
    /// Counter minting a fresh opaque-origin string on each navigation of a
    /// sandboxed iframe without `allow-same-origin`, so two such frames (or
    /// the same frame across re-navigations) are never mistaken for the same
    /// origin, matching the spec's "unique opaque origin per navigation".
    opaque_origin_counter: u64,
    /// HTML-35: the spec's "already started" flag for non-parser-inserted
    /// `<script>` elements, kept out-of-band here rather than on
    /// `amazing-dom`'s `Node` -- an HTML-only concept, not a general DOM
    /// one (`docs/lanes/html_TODO.md` records why, and the minimal-hook
    /// alternative considered instead). See `run_pending_scripts` for the
    /// rule for when a script does/doesn't get added here.
    already_started_scripts: HashSet<NodeIdx>,
}

// Each Window is the global target of one actual browsing context. The proxy
// is stable across navigation; stale globals retain their own document identity.
#[derive(Clone)]
struct WindowContext {
    document: NodeIdx,
    frame: Option<NodeIdx>,
    global: JsValue,
    window: JsValue,
    parent: JsValue,
    top: JsValue,
    origin: String,
    installed: bool,
    indices: usize,
}

#[derive(Clone, Copy)]
pub(super) enum HtmlWindowProperty { Frames, Length, Parent, Top }

impl HtmlWindowProperty {
    fn name(self) -> &'static str {
        match self { Self::Frames => "frames", Self::Length => "length", Self::Parent => "parent", Self::Top => "top" }
    }
}

impl FrameRuntime {
    pub(super) fn has_document(&self, document: NodeIdx) -> bool {
        document == NodeIdx(0) || self.documents.values().any(|frame| frame.document == Some(document))
    }

    pub(super) fn frame_for_document(&self, document: NodeIdx) -> Option<NodeIdx> {
        self.documents.iter().find_map(|(node, frame)| (frame.document == Some(document)).then_some(*node))
    }

    fn live_context(&self, context: &WindowContext, realm: RealmHandle) -> bool {
        context.frame.is_none() || context.frame.is_some_and(|frame|
            self.windows.get(&frame).is_some_and(|(current, _)| *current == realm)
                && self.documents.get(&frame).is_some_and(|entry| entry.document == Some(context.document)))
    }
}

fn context_origin(js: &mut Interpreter, context: &WindowContext, realm: RealmHandle) -> Result<String, Throw> {
    if context.frame.is_some() { return Ok(context.origin.clone()); }
    js.binding_with_realm(realm, |js| Ok(ParsedUrl::parse(js.binding_location_href())
        .map(|url| url.origin().serialize()).unwrap_or_else(|_| "null".into())))
}

/// Runnable script elements in tree order for the non-interleaved fallback.
fn collect_classic_scripts(dom: &Dom, node: NodeIdx, out: &mut Vec<crate::cmd::js::PageScript>) {
    if dom.namespace(node) == Some(Namespace::Html) && dom.local_name(node) == Some("script") {
        if let Some(script) = crate::cmd::js::script_source_from_node(dom, node) { out.push(script); }
        return;
    }
    for child in dom.children(node) { collect_classic_scripts(dom, child, out); }
}

/// HTML-29: the shared state behind a document's `document.write` insertion
/// point. A present entry means the document's parser is "active" (either
/// really -- its own top-level scripts are still running -- or synthetically,
/// inside a `document.open()`/`write()` session); `(parent, before)` matches
/// `Dom::pre_insert`'s own signature and names exactly where the next write
/// lands. No entry means a `write()` call must implicitly `open()` (replace
/// the whole document) before it can insert anything, per spec.
///
/// `(document, None)` is a placeholder used only until the first script
/// narrows it (`narrow_write_insertion_point_state`): `write_into_document`
/// treats `parent == document` as "no specific point yet" and falls back to
/// its own body/head/document-element resolution.
fn begin_document_parsing_state(state: &Rc<RefCell<BindingState>>, document: NodeIdx) {
    state.borrow_mut().write_insertion_points.entry(document).or_insert((document, None));
}

/// Narrows `document`'s insertion point to immediately after `script_node`,
/// its own current tree position -- the parser's insertion point while that
/// classic script is the one running. A no-op if the document's parser is
/// not currently marked active.
fn narrow_write_insertion_point_state(state: &Rc<RefCell<BindingState>>, document: NodeIdx, script_node: NodeIdx) {
    let mut state_ref = state.borrow_mut();
    if !state_ref.write_insertion_points.contains_key(&document) { return; }
    let host = state_ref.host.clone();
    let (parent, next_sibling) = {
        let host = host.borrow();
        let node = host.dom.get(script_node);
        (node.parent, node.next_sibling)
    };
    if let Some(parent) = parent {
        state_ref.write_insertion_points.insert(document, (parent, next_sibling));
    }
}

/// Ends `document`'s parser-active window: a later `document.write()` call
/// implicitly replaces the document (`open()`) instead of inserting into it.
fn end_document_parsing_state(state: &Rc<RefCell<BindingState>>, document: NodeIdx) {
    state.borrow_mut().write_insertion_points.remove(&document);
}

/// HTML-31: marks `document` as being built by a live, script-interleaved
/// `amazing_html::ResumableParser` -- `document_write` checks this first and,
/// when present, inserts into the real tokenizer (`ResumableParser::write`)
/// instead of `write_into_document`'s DOM-splice fallback. Also opens the
/// same placeholder `write_insertion_points` entry `begin_document_parsing_
/// state` does, purely so `document.open()`'s existing "already active"
/// no-op guard keeps working unchanged for this document too.
fn register_active_parser_state(
    state: &Rc<RefCell<BindingState>>,
    document: NodeIdx,
    parser: Rc<RefCell<amazing_html::ResumableParser>>,
) {
    let mut state_ref = state.borrow_mut();
    state_ref.write_insertion_points.entry(document).or_insert((document, None));
    state_ref.active_parsers.insert(document, parser);
}

/// Ends `document`'s interleaved-parser window, removing both maps
/// `register_active_parser_state` populated. A later `document.write()`
/// call finds no active parser at all and implicitly reopens the document
/// (`write_into_document`'s existing fallback) -- correct per spec: once
/// parsing reaches EOF, even a same-tick `DOMContentLoaded`/`load` listener's
/// own `write()` call is a full-document replace, not an append.
fn unregister_active_parser_state(state: &Rc<RefCell<BindingState>>, document: NodeIdx) {
    let mut state_ref = state.borrow_mut();
    state_ref.active_parsers.remove(&document);
    state_ref.write_insertion_points.remove(&document);
}

/// Fetch an external classic script for a child document: resolved against
/// the child's own URL (not the top document's), through the pinned fixture
/// resolver when one is set, otherwise a real network fetch. `None` means
/// the URL did not resolve or the fetch/decode failed; the script is then
/// silently skipped, as if no error listener were installed to observe it.
fn fetch_child_script(src: &str, base: &str,
    resolver: Option<&FrameResourceResolver>) -> Option<String> {
    let url = ParsedUrl::parse(base).ok()?.resolve(src).ok()?.serialize();
    let bytes = if let Some(resolve) = resolver {
        let path = resolve(&url)?;
        std::fs::read(path).ok()?
    } else {
        amazing_net::fetch(&url, &amazing_net::FetchOptions {
            max_response_bytes: 16 * 1024 * 1024, ..Default::default()
        }).ok().filter(|response| response.is_success())?.body
    };
    Some(amazing_html::encoding::decode(&bytes, None).text)
}

fn same_origin(parent: &str, child: &str) -> bool {
    let (Ok(parent), Ok(child)) = (ParsedUrl::parse(parent), ParsedUrl::parse(child)) else {
        return false;
    };
    matches!((parent.origin(), child.origin()),
        (UrlOrigin::Tuple(a), UrlOrigin::Tuple(b)) if a == b)
}

fn content_type(response: &amazing_net::Response, logical_url: &str,
    local_file: bool) -> Option<String> {
    let mime = response.content_type().unwrap_or("").to_ascii_lowercase();
    let unknown = matches!(mime.as_str(), "" | "unknown/unknown" | "application/unknown" | "*/*");
    let no_sniff = response.headers.get("x-content-type-options")
        .is_some_and(|value| value.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("nosniff"));
    if unknown && !no_sniff && has_html_signature(&response.body) {
        return Some("text/html".into());
    }
    if mime != "application/octet-stream" && !unknown {
        return Some(mime);
    }
    if !local_file { return None; }
    // Local file fetches currently label XML and XHTML as octet-stream.
    // Their extension is the WPT server's media-type mapping for these files.
    let path = logical_url.split(['?', '#']).next().unwrap_or(logical_url);
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("xml") => Some("application/xml".into()),
        Some("xhtml") | Some("xht") => Some("application/xhtml+xml".into()),
        Some("html") | Some("htm") => Some("text/html".into()),
        _ => None,
    }
}

/// HTML rows of MIME Sniffing §7.1. Match only the resource header, ignoring
/// the five whitespace bytes; a tag terminator is SPACE or '>', never '/'.
fn has_html_signature(body: &[u8]) -> bool {
    let header = &body[..body.len().min(1445)];
    let start = header.iter().position(|byte| !matches!(byte, 9 | 10 | 12 | 13 | 32))
        .unwrap_or(header.len());
    let header = &header[start..];
    const PATTERNS: &[&[u8]] = &[b"<!DOCTYPE HTML", b"<HTML", b"<HEAD", b"<SCRIPT",
        b"<IFRAME", b"<H1", b"<DIV", b"<FONT", b"<TABLE", b"<A", b"<STYLE",
        b"<TITLE", b"<B", b"<BODY", b"<BR", b"<P", b"<!--"];
    PATTERNS.iter().any(|pattern| header.get(..pattern.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(pattern))
        && header.get(pattern.len()).is_some_and(|byte| matches!(byte, b' ' | b'>')))
}

/// HTML-34: `navigate_iframes`'s fetch/decide-mime step hands back either an
/// HTML source string (deferred to `setup_iframe_window`'s interleaved
/// parse, `setup_iframe_window`'s job) or an already-fully-parsed tree
/// (`navigate_iframes` imports it immediately, one-shot, as before).
enum LoadedChildDocument {
    /// `srcdoc`, and any fetched `text/html` response -- both can contain
    /// scripts, which is the entire reason to interleave: a script sees the
    /// document built so far, not (as `import_inert_document` alone would
    /// give it) the whole thing already finished.
    Deferred { source: String, url: String, mime: String, encoding: &'static str },
    /// XML/XHTML (`ResumableParser` is HTML-only, so these still parse
    /// one-shot, exactly as before this ticket) -- and the canned
    /// `about:blank` skeleton, which is *never* going to have a script (it's
    /// a fixed, three-element constant, not fetched or user-supplied
    /// content), so deferring it would only cost every caller the
    /// spec-required synchronous availability of a bare iframe's
    /// `contentDocument.documentElement`/`.body` right after insertion, for
    /// no benefit at all: see `removing_a_freshly_appended_iframe_before_
    /// its_load_dispatch_does_not_panic`'s regression test below.
    Eager { parsed: Dom, url: String, mime: String, encoding: &'static str },
}

fn parse_response(body: &[u8], mime: &str, charset: Option<&str>) -> Result<(Dom, &'static str), String> {
    if mime == "text/html" {
        let decoded = amazing_html::encoding::decode(body, charset);
        return Ok((amazing_html::parse_bytes(body, charset), amazing_html::encoding::canonical_name(decoded.encoding)));
    }
    if matches!(mime, "text/xml" | "application/xml" | "application/xhtml+xml")
        || mime.ends_with("+xml") {
        let decoded = amazing_html::encoding::decode(body, charset);
        let parsed = amazing_html::parse_xml(&decoded.text)
            .map_err(|error| format!("XML parse failed: {error:?}"))?;
        return Ok((parsed, amazing_html::encoding::canonical_name(decoded.encoding)));
    }
    Err(format!("unsupported iframe media type: {mime}"))
}

fn fetch_child_document(logical_url: &str, parent_url: &str,
    resolver: Option<&FrameResourceResolver>, sandbox: SandboxFlags) -> Result<amazing_net::Response, String> {
    let parent = HttpUrl::parse(parent_url).map_err(|error| format!("iframe parent URL: {error:?}"))?;
    let target = HttpUrl::parse(logical_url).map_err(|error| format!("iframe URL: {error:?}"))?;
    let origin = parent.origin().clone();
    let context = NavigableContext {
        top_level_origin: origin.clone(), parent_origin: origin.clone(),
        ancestor_origins: vec![origin], sandbox_flags: sandbox.to_bits(),
        frame_csp: Default::default(), referrer: parent.serialize(),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        now_unix: SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_secs().min(i64::MAX as u64) as i64,
    };
    // A fixture resolver only changes the transport address. Policy always
    // sees the document's logical URL and its real parent origin.
    if let Some(resolve) = resolver {
        ApiRequest::child_navigation(target.clone(), &context)
            .map_err(|error| format!("iframe request: {error:?}"))?;
        let fetch_url = resolve(logical_url)
            .ok_or_else(|| "unavailable pinned iframe resource".to_owned())?;
        let response = amazing_net::fetch(&fetch_url, &amazing_net::FetchOptions {
            max_response_bytes: 16 * 1024 * 1024, ..Default::default()
        }).map_err(|error| format!("iframe fetch: {error}"))?;
        let mut headers = HeaderList::default();
        for (name, value) in response.headers.iter() {
            headers.append(name, value).map_err(|error| format!("iframe header: {error:?}"))?;
        }
        check_frame_embedding(&target, &headers, &context.ancestor_origins)
            .map_err(|error| format!("iframe embedding: {error}"))?;
        return Ok(response);
    }
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_child_navigation(target, &context)
        .map_err(|error| format!("iframe navigation: {error:?}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut network_error = None;
    loop {
        for event in service.step(128, Instant::now()) {
            if let ServiceEvent::Network { id: network_id, event } = &event
                && *network_id == id
                && matches!(event, amazing_net::dispatch::NetEvent::Error { .. }) {
                network_error = Some(format!("{event:?}"));
            }
            if let ServiceEvent::Finished { id: finished, result } = event && finished == id {
                let mut response = result.map_err(|error|
                    format!("iframe fetch: {error:?}; {network_error:?}"))?;
                let mut headers = amazing_net::Headers::new();
                for (name, value) in response.headers.raw_entries() {
                    headers.append(name, value.clone())
                        .map_err(|error| format!("iframe header: {error:?}"))?;
                }
                let body = response.body.bytes()
                    .map_err(|error| format!("iframe body: {error:?}"))?;
                if body.len() > 16 * 1024 * 1024 { return Err("iframe body too large".into()); }
                return Ok(amazing_net::Response { url: response.url, status: response.status,
                    reason: response.status_text, headers, body });
            }
        }
        if Instant::now() >= deadline { return Err("iframe fetch timed out".into()); }
        std::thread::sleep(Duration::from_millis(1));
    }
}

impl RuntimeBindings {
    /// IPC-10 installs this when its navigator can consume method, body,
    /// enctype and target together. The current interpreter entry accepts URL only.
    pub(crate) fn set_form_navigation_hook(&self, hook: FormNavigationHook) {
        self.state.borrow_mut().frames.form_navigation_hook = Some(hook);
    }
    pub(crate) fn set_frame_resource_resolver(&self, resolver: FrameResourceResolver) {
        self.state.borrow_mut().frames.resolver = Some(resolver);
    }

    /// IPC-10 can observe committed child navigations without owning HTML parsing.
    pub(crate) fn set_child_navigation_hook(&self, hook: ChildNavigationHook) {
        self.state.borrow_mut().frames.navigation_hook = Some(hook);
    }

    pub(crate) fn take_iframe_loads(&self) -> Vec<(NodeIdx, Option<NodeIdx>, bool)> {
        self.state.borrow_mut().frames.pending_loads.drain(..).collect()
    }

    /// HTML-29: opens `document`'s `document.write` parser-active window.
    /// Callers running a document's own top-level classic scripts (the WPT
    /// harness's and the real-page `execute_page_scripts_lenient`/
    /// `prepare_page_scripts` loops, `js_scheduler.rs`'s
    /// `dispatch_pending_iframe_loads` for an iframe's document) call this
    /// once before that loop, `narrow_write_insertion_point` before running
    /// each script, and `end_document_parsing` after the load event has
    /// fired -- matching the window during which a real parser has an
    /// insertion point at all. See `write_into_document`.
    pub(crate) fn begin_document_parsing(&self, document: NodeIdx) {
        begin_document_parsing_state(&self.state, document);
    }
    /// Narrows `document`'s write insertion point to right after
    /// `script_node`, immediately before running that top-level script.
    pub(crate) fn narrow_write_insertion_point(&self, document: NodeIdx, script_node: NodeIdx) {
        narrow_write_insertion_point_state(&self.state, document, script_node);
    }
    /// Ends `document`'s parser-active window; see `begin_document_parsing`.
    pub(crate) fn end_document_parsing(&self, document: NodeIdx) {
        end_document_parsing_state(&self.state, document);
    }
    /// HTML-31: registers `document`'s live, script-interleaved parser so
    /// `document.write`/`writeln` calls from a script it pauses at insert
    /// into the real tokenizer instead of splicing a parsed fragment into
    /// the DOM. Callers driving an interleaved parse (`cmd/wpt_harness.rs`'s
    /// `execute_page`, `cmd/js.rs`'s two real-page paths) call this once
    /// instead of `begin_document_parsing` (it opens the same placeholder
    /// write-insertion-point entry), and `unregister_active_parser` once
    /// parsing reaches `Eof`, before running any deferred/async scripts.
    pub(crate) fn register_active_parser(&self, document: NodeIdx, parser: Rc<RefCell<amazing_html::ResumableParser>>) {
        register_active_parser_state(&self.state, document, parser);
    }
    /// Ends `document`'s interleaved-parser window; see `register_active_parser`.
    pub(crate) fn unregister_active_parser(&self, document: NodeIdx) {
        unregister_active_parser_state(&self.state, document);
    }

    /// Everything about a frame's just-committed navigation that must be
    /// visible synchronously -- before any deferred task, including a
    /// script that reads `contentDocument`/`contentWindow` before
    /// `setup_iframe_window`'s later tick ever runs: the realm and
    /// WindowProxy (JSRT-9), its origin, a full per-realm WebIDL interface
    /// set (so `frame.contentWindow.Document !== window.Document`), and
    /// `document` bound as that realm's global. Only running the document's
    /// own classic scripts stays deferred, to `setup_iframe_window`.
    ///
    /// Called once from `navigate_iframes`, synchronously with the
    /// navigation commit; `setup_iframe_window` only falls back to calling
    /// it when somehow no realm exists yet (e.g. a test driving it directly
    /// without a prior `navigate_iframes`).
    fn ensure_realm(&self, js: &mut Interpreter, frame: NodeIdx, document: NodeIdx,
        child_url: &str, sandbox: SandboxFlags) -> Result<RealmHandle, Throw> {
        // `about:srcdoc` and `about:blank` both inherit the parent
        // document's origin (neither is opaque on its own -- an unsandboxed
        // `about:blank` iframe is same-origin with its parent, per the
        // "about:blank" URL's own origin being "the origin of the browsing
        // context that created it"); without this, `ParsedUrl::parse
        // ("about:blank").origin()` fell through to the final branch below
        // and produced a mismatched/opaque origin, tripping JSRT-9's
        // WindowProxy cross-origin gate on the very first same-origin,
        // no-`src` `<iframe>` access. A sandboxed frame without
        // `allow-same-origin` gets a fresh, never-repeating opaque origin
        // per navigation instead of whatever its URL would otherwise
        // resolve to, so it can never observe or be observed as
        // same-origin with anything, including its own earlier navigations.
        let parent_document = self.state.borrow().host.borrow().dom.owner_document(frame).unwrap_or(NodeIdx(0));
        let parent_context = self.state.borrow().frames.contexts.iter()
            .find(|(realm, context)| context.document == parent_document
                && self.state.borrow().frames.live_context(context, **realm))
            .map(|(realm, context)| (*realm, context.clone()));
        let parent_origin = match &parent_context {
            Some((realm, context)) => context_origin(js, context, *realm)?,
            None => "null".into(),
        };
        let origin = if sandbox.sandboxed && !sandbox.allow_same_origin {
            let mut state = self.state.borrow_mut();
            state.frames.opaque_origin_counter += 1;
            format!("null (sandboxed #{})", state.frames.opaque_origin_counter)
        } else if child_url == "about:srcdoc" || child_url == "about:blank" {
            parent_origin
        } else {
            ParsedUrl::parse(child_url).map(|url| url.origin().serialize())
                .unwrap_or_else(|_| "null".into())
        };
        let existing = self.state.borrow().frames.windows.get(&frame).cloned();
        let realm = if let Some((_, proxy)) = existing {
            let bare = js.binding_create_object(None)?;
            let realm = js.binding_create_realm(bare)?;
            js.binding_retarget_window_proxy(&proxy, realm)?;
            self.state.borrow_mut().frames.windows.insert(frame, (realm, proxy));
            realm
        } else {
            let bare = js.binding_create_object(None)?;
            let realm = js.binding_create_realm(bare)?;
            let proxy = js.binding_create_window_proxy(realm)?;
            self.state.borrow_mut().frames.windows.insert(frame, (realm, proxy));
            realm
        };
        js.binding_set_realm_origin(realm, &origin)?;
        let module_base = if matches!(child_url, "about:blank" | "about:srcdoc") {
            self.state.borrow().document_url.clone().unwrap_or_else(|| "about:blank".into())
        } else { child_url.to_owned() };
        let proxy = self.state.borrow().frames.windows[&frame].1.clone();
        let global = js.binding_realm_global(realm)?;
        let (parent, top) = parent_context.map(|(_, context)| (context.window, context.top))
            .unwrap_or_else(|| (proxy.clone(), proxy.clone()));
        self.state.borrow_mut().frames.contexts.insert(realm, WindowContext {
            document, frame: Some(frame), global, window: proxy, parent, top, origin,
            installed: false, indices: 0,
        });
        let state = self.state.clone();
        js.binding_with_realm(realm, |i| {
            self.set_module_document_base(i, &module_base);
            // `host_native_dispatch` is per-realm state (see JSRT-9's
            // `RealmState`), so a freshly created realm has none until we
            // install one; without this every native binding call in the
            // child (DOM methods, `contentWindow`/`contentDocument`
            // getters, etc.) throws "host function dispatcher is not
            // installed".
            i.set_location_url(child_url);
            i.set_host_native_dispatcher(build_host_native_dispatcher(state.clone()));
            let mut adapter = RuntimeBindingHost { js: i, state: state.clone() };
            // A real Document/Element/Array/... surface for this realm, not
            // the top realm's: `frame.contentWindow.Array !== window.Array`
            // and `frame.contentWindow.Document !== window.Document`.
            // NET-42: `install_net()` was missing here entirely, so every
            // net-backed global (starting with `document.cookie`, which is
            // what surfaced this) was simply absent from every child realm.
            // `install_storage_interfaces`/`install_messaging` had the exact
            // same gap -- both only install classes/constructors onto
            // *this* realm's own global object (mirroring the top-level
            // realm's install sequence in `runtime_binding_host.rs`), so
            // both are safe to add the same way. Canvas stays top-only too,
            // unrequested and unrelated to this fix.
            //
            // HTML-34: this alone did NOT give a frame a real, working,
            // origin-scoped `localStorage`/`sessionStorage` -- fixed below,
            // together with `document_url` becoming realm-scoped for the
            // duration of a child realm's own script execution (`setup_
            // iframe_window`/`run_interleaved_iframe_parse`): `Storage`'s
            // own read/write path (`runtime_host_storage.rs::storage_key`)
            // resolves the CURRENT document's origin from that one shared
            // field, so a correctly-scoped `document_url` is what actually
            // makes `install_storage_interfaces`'s accessor-based getter
            // (installed above) key its data to THIS frame's origin instead
            // of the top-level page's. See docs/lanes/html_TODO.md.
            install_interfaces(&mut adapter)?;
            adapter.install_dom_unscopables()?;
            adapter.install_window_event_target()?;
            adapter.install_selection_global()?;
            adapter.install_net()?;
            adapter.install_module_loader()?;
            adapter.install_storage_interfaces()?;
            adapter.install_messaging()?;
            // HTML-37: TextDecoder/TextEncoder are stateless-per-instance
            // globals (no per-origin scoping unlike storage/net above), so
            // installing them onto the child realm's own global object is
            // safe the same way install_messaging() already is.
            adapter.install_text_codec()?;
            adapter.install_iframe_members()?;
            adapter.install_html_form_globals()?;
            adapter.install_document_write_globals()?;
            // Wrapped here, inside the child realm, so `document` gets the
            // child's own freshly-installed Document.prototype (its host
            // object cache is per-realm) rather than the caller's; the
            // `contentDocument` getter enters this same realm to fetch the
            // identical cached wrapper, so the two stay `===` no matter
            // which is read first.
            let wrapped = adapter.wrap_node(document)?;
            let fresh = EnvHandle::new(EnvKind::Global, Some(adapter.js.global_env.clone()));
            fresh.declare_lexical("document", wrapped.clone(), false)?;
            // HTML-34: same stable-identity shadowing as `document` above,
            // now that this realm's own `document_url` scoping (set around
            // its actual script execution, not here at realm-creation time)
            // makes the underlying `Storage` area resolve to this frame's
            // own origin instead of the top-level page's.
            let (local_storage, session_storage) = adapter.storage_lexical_areas()?;
            fresh.declare_lexical("localStorage", local_storage, false)?;
            fresh.declare_lexical("sessionStorage", session_storage, false)?;
            adapter.js.global_env = fresh;
            adapter.js.global_obj.borrow_mut().set_own("document", wrapped);
            Ok(())
        })?;
        Ok(realm)
    }

    pub(crate) fn navigate_iframes(&self, js: &mut Interpreter) {
        let (host, base, resolver) = {
            let state = self.state.borrow();
            (state.host.clone(), state.document_url.clone(), state.frames.resolver.clone())
        };
        let Some(base) = base else { return };
        let candidates = {
            let host = host.borrow();
            let state = self.state.borrow();
            let mut nodes = Vec::new();
            let mut documents = VecDeque::from([NodeIdx(0)]);
            while let Some(document) = documents.pop_front() {
                for node in host.dom.tree_order(document) {
                    if let Some(child) = state.frames.documents.get(&node).and_then(|entry| entry.document) {
                        documents.push_back(child);
                    }
                    nodes.push(node);
                }
            }
            nodes.into_iter().filter_map(|node| {
                (host.dom.namespace(node) == Some(Namespace::Html)
                    && host.dom.local_name(node) == Some("iframe"))
                    .then(|| (node,
                        host.dom.get_attribute_ns(node, None, "src").map(str::to_owned),
                        host.dom.get_attribute_ns(node, None, "srcdoc").map(str::to_owned),
                        parse_sandbox(host.dom.get_attribute_ns(node, None, "sandbox"))))
            }).collect::<Vec<_>>()
        };
        let connected: HashSet<_> = candidates.iter().map(|(node, ..)| *node).collect();
        {
            let mut state = self.state.borrow_mut();
            state.frames.documents.retain(|node, _| connected.contains(node));
            state.frames.windows.retain(|node, _| connected.contains(node));
        }
        for (node, src, srcdoc, sandbox) in candidates {
            let base = {
                let host = host.borrow();
                let owner = host.dom.owner_document(node).unwrap_or(NodeIdx(0));
                if owner == NodeIdx(0) { base.clone() }
                else { host.dom.document_metadata(owner).ok().flatten().map(|metadata| metadata.url.clone()).unwrap_or_else(|| base.clone()) }
            };
            if self.state.borrow().frames.documents.get(&node)
                .is_some_and(|old| old.src == src && old.srcdoc == srcdoc) {
                continue;
            }
            let loaded = (|| -> Result<LoadedChildDocument, String> {
                // `srcdoc` takes priority over `src` per spec: its content is
                // used verbatim and the navigable's URL becomes "about:srcdoc"
                // rather than being resolved/fetched.
                if let Some(html) = srcdoc.as_deref() {
                    return Ok(LoadedChildDocument::Deferred { source: html.to_owned(),
                        url: "about:srcdoc".into(), mime: "text/html".into(), encoding: "UTF-8" });
                }
                let Some(source) = src.as_deref().filter(|source| !source.is_empty()) else {
                    return Ok(LoadedChildDocument::Eager {
                        parsed: amazing_html::parse("<!doctype html><html><head></head><body></body></html>"),
                        url: "about:blank".into(), mime: "text/html".into(), encoding: "UTF-8" });
                };
                let logical_url = ParsedUrl::parse(&base).and_then(|base| base.resolve(source))
                    .map_err(|error| format!("iframe URL: {error:?}"))?.serialize();
                let response = fetch_child_document(&logical_url, &base, resolver.as_ref(), sandbox)?;
                if !response.is_success() { return Err(format!("iframe status {}", response.status)); }
                let mime = content_type(&response, &logical_url,
                    resolver.is_some() || logical_url.starts_with("file:"))
                    .ok_or_else(|| "iframe response has no supported media type".to_owned())?;
                let charset = response.headers.get("content-type").and_then(|header| {
                    header.split(';').skip(1).find_map(|parameter| {
                        let (name, value) = parameter.trim().split_once('=')?;
                        name.trim().eq_ignore_ascii_case("charset")
                            .then(|| value.trim().trim_matches('"'))
                    })
                });
                let document_url = if resolver.is_none() { response.url.clone() } else { logical_url.clone() };
                // HTML-34: only the HTML case defers to a script-interleaved
                // parse (`ResumableParser` is HTML-only); XML/XHTML iframe
                // documents keep the pre-HTML-34 one-shot parse-then-import,
                // same as before.
                if mime == "text/html" {
                    let decoded = amazing_html::encoding::decode(&response.body, charset);
                    return Ok(LoadedChildDocument::Deferred { source: decoded.text,
                        url: document_url, mime, encoding: amazing_html::encoding::canonical_name(decoded.encoding) });
                }
                let (parsed, encoding) = parse_response(&response.body, &mime, charset)?;
                Ok(LoadedChildDocument::Eager { parsed, url: document_url, mime, encoding })
            })();
            let (document, url, html) = match loaded {
                Ok(LoadedChildDocument::Deferred { source, url, mime, encoding }) => {
                    // Created empty; `setup_iframe_window`'s deferred tick
                    // does the actual (interleaved) parse into it, so its
                    // own top-level scripts run as their own task and see
                    // the document built so far at each pause, not the
                    // whole thing already finished. `ready_state` defaults
                    // to `Loading` (`Dom::create_document`'s own default),
                    // flipped to `Complete` only once that parse reaches
                    // `Eof` -- more accurate than the old one-shot path's
                    // immediate `Complete`, not less.
                    let document = host.borrow_mut().dom.create_document();
                    let _ = host.borrow_mut().dom.set_document_metadata(document,
                        amazing_dom::document_surface::DocumentMetadata {
                            url: url.clone(), content_type: mime, character_set: encoding.into(),
                        });
                    (Some(document), url, Some(source))
                }
                Ok(LoadedChildDocument::Eager { parsed, url, mime, encoding }) => {
                    let imported = host.borrow_mut().dom.import_inert_document(&parsed, &url, &mime, encoding);
                    match imported {
                        Ok(document) => {
                            let mode = parsed.document_quirks_mode(NodeIdx(0)).unwrap_or(amazing_dom::QuirksMode::NoQuirks);
                            let _ = host.borrow_mut().dom.set_document_quirks_mode(document, mode);
                            let _ = host.borrow_mut().dom.set_document_ready_state(document, ReadyState::Complete);
                            (Some(document), url, None)
                        }
                        Err(_) => (None, url, None),
                    }
                }
                Err(_) => (None, String::new(), None),
            };
            // Per spec, a `load` event is only queued for this navigation
            // when the element has a `src` or `srcdoc` attribute at all
            // (present, even if empty) -- not for the implicit `about:blank`
            // an iframe with neither gets purely from being inserted.
            let fire_load_events = src.is_some() || srcdoc.is_some();
            // Realm/WindowProxy creation happens here, synchronously with
            // the navigation commit, not deferred to `setup_iframe_window`'s
            // later tick: `contentDocument`/`contentWindow` must see the
            // same wrapper identity whether they're read from an inline
            // script that runs before that tick or from one that runs
            // after (WPT's `iframe-nosrc.html` "contentDocument should not
            // be changed" exercises exactly this ordering).
            if let Some(document) = document {
                // Scripting belongs to the browsing context, including
                // XML/XHTML and the synchronous initial about:blank document.
                let enabled = !(sandbox.sandboxed && !sandbox.allow_scripts);
                let _ = host.borrow_mut().dom.set_document_scripting_enabled(document, enabled);
                let _ = self.ensure_realm(js, node, document, &url, sandbox);
            }
            let mut state = self.state.borrow_mut();
            let navigation = ChildNavigation { frame: node, requested_src: src.clone(),
                committed_url: url.clone(), document };
            state.frames.documents.insert(node, FrameDocument { src, srcdoc, document, url, sandbox, html });
            state.frames.pending_loads.push_back((node, document, fire_load_events));
            let hook = state.frames.navigation_hook.clone();
            drop(state);
            if let Some(hook) = hook { hook(navigation); }
        }
        let _ = self.refresh_window_indices(js);
    }

    fn refresh_window_indices(&self, js: &mut Interpreter) -> Result<(), Throw> {
        let contexts = self.state.borrow().frames.contexts.iter()
            .map(|(realm, context)| (*realm, context.clone())).collect::<Vec<_>>();
        for (realm, context) in contexts {
            let children = {
                let state = self.state.borrow();
                if !state.frames.live_context(&context, realm) { Vec::new() }
                else {
                    let host = state.host.borrow();
                    host.dom.tree_order(context.document).into_iter()
                        .filter_map(|node| state.frames.windows.get(&node).map(|(_, window)| window.clone()))
                        .collect::<Vec<_>>()
                }
            };
            for index in 0..context.indices.max(children.len()) {
                let name = index.to_string();
                if let Some(window) = children.get(index) {
                    js.binding_define_data_property(&context.global, &name, window.clone(),
                        amazing_js::interp::BindingPropertyFlags { writable: false, enumerable: true, configurable: true })?;
                } else { js.binding_delete_property(&context.global, &name)?; }
            }
            if let Some(context) = self.state.borrow_mut().frames.contexts.get_mut(&realm) {
                context.indices = children.len();
            }
        }
        Ok(())
    }

    /// Run a connected iframe's classic scripts, deferred here (rather than
    /// synchronous with the navigation commit) so they execute as their own
    /// task, as spec requires. The realm/proxy, its origin, per-realm
    /// interfaces and the bound `document` global all already exist by now
    /// -- `navigate_iframes` created them synchronously via `ensure_realm`,
    /// so `contentDocument`/`contentWindow` have a stable identity whether a
    /// script reads them before or after this tick runs.
    pub(crate) fn setup_iframe_window(&self, js: &mut Interpreter, frame: NodeIdx,
        document: NodeIdx) -> Result<(), Throw> {
        let (child_url, sandbox, html) = {
            let mut state = self.state.borrow_mut();
            let entry = state.frames.documents.get_mut(&frame);
            (entry.as_ref().map(|entry| entry.url.clone()).unwrap_or_default(),
                entry.as_ref().map(|entry| entry.sandbox).unwrap_or_default(),
                // `take`n, not cloned: this document's interleaved parse
                // consumes its stashed source exactly once.
                entry.and_then(|entry| entry.html.take()))
        };
        // Normally already created by `navigate_iframes`; this fallback
        // only matters for a direct call with no prior navigation commit
        // (e.g. a test driving `setup_iframe_window` on its own). The
        // lookup is bound to an owned `Option` first, rather than matched on
        // directly, so its `Ref` from `borrow()` drops before the `None`
        // arm runs -- otherwise that immutable borrow stays alive for the
        // whole match (matches extend a scrutinee temporary's lifetime
        // across all arms) and `ensure_realm`'s own `borrow_mut()` panics
        // with "already borrowed" the first time a frame's realm is created
        // through this fallback instead of through `navigate_iframes`.
        let existing_realm = self.state.borrow().frames.windows.get(&frame).map(|(realm, _)| *realm);
        let realm = match existing_realm {
            Some(realm) => realm,
            None => self.ensure_realm(js, frame, document, &child_url, sandbox)?,
        };
        let state = self.state.clone();
        // HTML-34: `document_url` is one field shared by the whole host,
        // not realm-scoped (`fetch_base`/`storage_key`/... all read it
        // directly) -- see docs/lanes/html_TODO.md's HTML-34 notes. Every
        // synchronous entry into this child's own script execution below
        // (whether the new interleaved path or the old post-hoc one) needs
        // it pointed at the CHILD's URL for that duration, restored right
        // after, so `document.cookie`/`fetch()`/`localStorage` resolve
        // against the iframe's own origin instead of the top-level page's.
        // This covers every script that runs synchronously from here, but
        // not a callback that fires later, asynchronously, still inside
        // this realm (a timer an iframe's own script registered) -- a real,
        // narrower residual gap than the one this fix closes, flagged
        // rather than silently assumed solved.
        let previous_document_url = state.borrow_mut().document_url.replace(child_url.clone());
        let result = if let Some(html) = html {
            self.run_interleaved_iframe_parse(js, realm, document, &child_url, sandbox, &html)
        } else {
            // No stashed source: an XML/XHTML child (already fully built,
            // one-shot, by `navigate_iframes`) or a direct test call with
            // no prior navigation at all. `sandbox` without `allow-scripts`
            // disables script execution in the child browsing context
            // entirely (its own scripting flag is unset), so nothing runs
            // at all here -- not even collecting scripts. (An HTML child
            // never takes this branch, sandboxed or not: it always has a
            // stashed source, and `run_interleaved_iframe_parse` builds its
            // tree regardless, only skipping script execution itself.)
            (|| -> Result<(), Throw> {
                if sandbox.sandboxed && !sandbox.allow_scripts { return Ok(()); }
                let scripts = {
                    let state = state.borrow();
                    let host = state.host.borrow();
                    let mut scripts = Vec::new();
                    collect_classic_scripts(&host.dom, document, &mut scripts);
                    scripts
                };
                let resolver = state.borrow().frames.resolver.clone();
                js.binding_with_realm(realm, |i| {
                    for script in &scripts {
                        // HTML-29: narrows the child document's write
                        // insertion point to right after this script (its
                        // own tree position) before running it, matching
                        // the top-level page's own begin/narrow/end_
                        // document_parsing bracketing (`dispatch_pending_
                        // iframe_loads`) -- so a `document.write` call from
                        // an iframe's own top-level script inserts exactly
                        // where a parser would, not at the end of the body.
                        narrow_write_insertion_point_state(&state, document, script.node);
                        match &script.src {
                            None => { self.execute_document_script(i, script, &script.body, &child_url)?; }
                            Some(src) => {
                                let url = crate::cmd::js::resolve_script_url(&child_url, src);
                                let text = if script.kind == crate::cmd::js::ScriptKind::Module {
                                    self.load_module_source(i, &url).ok()
                                } else { fetch_child_script(src, &child_url, resolver.as_ref()) };
                                if let Some(text) = text {
                                    self.execute_document_script(i, script, &text, &url)?;
                                }
                                // A fetch failure or unresolvable URL is
                                // silently skipped, matching how a real
                                // load-error listener would observe it
                                // without one installed here.
                            }
                        }
                    }
                    Ok(())
                })
            })()
        };
        state.borrow_mut().document_url = previous_document_url;
        result
    }

    /// HTML-34: the iframe counterpart of `cmd/js.rs::run_interleaved_parse`
    /// for the top-level page -- parses `html` with a script-interleaved
    /// `ResumableParser`, grafting the document built so far into `document`
    /// (already created, empty, by `navigate_iframes`) at each pause via
    /// `amazing_dom::markup::IncrementalDocumentImport`, and running each
    /// parser-blocking classic script against that already-grafted,
    /// connected subtree of the shared `host.dom` -- exactly like a
    /// top-level script does, instead of `setup_iframe_window`'s old
    /// `collect_classic_scripts` walk over the already-fully-built tree
    /// (the same "sees the finished document, not the document-so-far" bug
    /// HTML-31 fixed for the top level, still present for every iframe until
    /// now). It cannot reuse `ResumableParser::swap_dom` directly, since
    /// that swaps an entire `Dom` and the iframe's document must live inside
    /// the SAME arena as its parent page and every sibling frame -- see
    /// `docs/lanes/html_TODO.md`'s HTML-32/HTML-34 notes and `markup.rs`'s
    /// `IncrementalDocumentImport` doc comment for the full design.
    ///
    /// `sandbox` without `allow-scripts` still builds the tree (parsing
    /// itself is unconditional) but never runs any script, matching the old
    /// path's own behavior for that case.
    fn run_interleaved_iframe_parse(&self, js: &mut Interpreter, realm: RealmHandle, document: NodeIdx,
        child_url: &str, sandbox: SandboxFlags, html: &str) -> Result<(), Throw> {
        let state = self.state.clone();
        let host = state.borrow().host.clone();
        let resolver = state.borrow().frames.resolver.clone();
        let can_run_scripts = !(sandbox.sandboxed && !sandbox.allow_scripts);
        let import = amazing_dom::markup::IncrementalDocumentImport::new(document);
        let mut parser = amazing_html::ResumableParser::new(Dom::new());
        if child_url == "about:srcdoc" { parser.set_srcdoc(); }
        if !can_run_scripts { parser.set_scripting_mode(amazing_html::ScriptingMode::Disabled); }
        parser.push_source(html);
        let parser = Rc::new(RefCell::new(parser));
        self.register_active_parser(document, parser.clone());
        let mut deferred_events = Vec::new();
        // No `narrow_write_insertion_point`/`write_insertion_points` upkeep
        // here, unlike the old post-hoc path below: `register_active_parser`
        // (above) makes `document_write` route straight into `parser.write`,
        // which already inserts at the tokenizer's own paused cursor -- the
        // real WHATWG insertion point -- with no external bookkeeping,
        // exactly like `cmd/js.rs::run_interleaved_parse` for the top level.
        let run_one = |js: &mut Interpreter, host: &Rc<RefCell<BrowserDomHost>>, source_node: NodeIdx,
            import: &IncrementalImportRef, child_url: &str,
            resolver: Option<&FrameResourceResolver>| -> Result<(), Throw> {
            let Some(dest_node) = import.borrow().imported(source_node) else {
                // The graft right before this call always maps the node a
                // pause/deferred signal just named; defensive only.
                return Ok(());
            };
            let script = { let host = host.borrow(); crate::cmd::js::script_source_from_node(&host.dom, dest_node) };
            let Some(script) = script else { return Ok(()) };
            let script_url = js.binding_with_realm(realm, |i| Ok(self.page_script_url(i, &script, child_url)))?;
            match &script.src {
                None => js.binding_with_realm(realm, |i| self.execute_document_script(i, &script, &script.body, &script_url)),
                Some(src) => {
                    let url = script_url;
                    let text = if script.kind == crate::cmd::js::ScriptKind::Module {
                        js.binding_with_realm(realm, |i| self.load_module_source(i, &url).map_err(Throw::type_error)).ok()
                    } else { fetch_child_script(src, child_url, resolver) };
                    if let Some(text) = text {
                        js.binding_with_realm(realm, |i| self.execute_document_script(i, &script, &text, &url))
                    } else if script.kind == crate::cmd::js::ScriptKind::Module {
                        js.binding_with_realm(realm, |i| self.finish_module_script(i, &script, None, &url, false))
                    } else { Ok(()) }
                }
            }
        };
        let import = Rc::new(RefCell::new(import));
        state.borrow_mut().frames.parser_imports.insert(document, import.clone());
        let outcome: Result<(), Throw> = loop {
            let signal = parser.borrow_mut().run();
            deferred_events.extend(parser.borrow_mut().take_deferred_or_async());
            {
                let mut host_ref = host.borrow_mut();
                import.borrow_mut().sync(&mut host_ref.dom, parser.borrow().dom())
                    .map_err(|error| Throw::type_error(format!("iframe graft: {error:?}")))?;
                let mode = parser.borrow().dom().document_quirks_mode(NodeIdx(0))
                    .map_err(|error| Throw::type_error(format!("iframe parser mode: {error:?}")))?;
                host_ref.dom.set_document_quirks_mode(document, mode)
                    .map_err(|error| Throw::type_error(format!("iframe document mode: {error:?}")))?;
                host_ref.dom.set_document_scripting_enabled(document, can_run_scripts)
                    .map_err(|error| Throw::type_error(format!("iframe scripting context: {error:?}")))?;
            }
            match signal {
                amazing_html::ParseSignal::Eof => break Ok(()),
                amazing_html::ParseSignal::Script(source_node) => {
                    if !can_run_scripts { continue; }
                    if let Err(error) = run_one(js, &host, source_node, &import, child_url, resolver.as_ref()) {
                        break Err(error);
                    }
                    // HTML-40: the script just ran directly against the
                    // shared, already-grafted subtree in `host.dom` (`sync`,
                    // above, only ever copies forward INTO it) -- reflect
                    // any mutation it made back into the parser's own
                    // private arena before it resumes, or its next
                    // tree-construction step (the adoption agency
                    // algorithm's cycle/document-full checks chief among
                    // them) reasons about stale, pre-mutation structure.
                    let reversed = {
                        let mut parser_ref = parser.borrow_mut();
                        let host_ref = host.borrow();
                        import.borrow_mut().reverse_sync(parser_ref.dom_mut(), &host_ref.dom)
                    };
                    if let Err(error) = reversed {
                        break Err(Throw::type_error(format!("iframe reverse graft: {error:?}")));
                    }
                }
            }
        };
        state.borrow_mut().frames.parser_imports.remove(&document);
        self.unregister_active_parser(document);
        outcome?;
        if can_run_scripts {
            for (source_node, _mode) in deferred_events {
                run_one(js, &host, source_node, &import, child_url, resolver.as_ref())?;
            }
        }
        let _ = host.borrow_mut().dom.set_document_ready_state(document, ReadyState::Complete);
        Ok(())
    }

    /// HTML-35: every `<script>` (HTML or SVG namespace -- `script_source_
    /// from_node` already knows SVG's `href`/`xlink:href` `src` fallback,
    /// HTML-32) reachable from a *known* document root, paired with that
    /// root, in tree order. Walks the top document (`NodeIdx(0)`) plus
    /// every live iframe's own document (`FrameRuntime::documents`).
    ///
    /// A `DOMParser`/`Range.createContextualFragment`/detached-fragment
    /// tree is not a known document root, so a script inside one is
    /// correctly invisible here -- until (if ever) it is adopted into a
    /// document this walk does see, at which point it would wrongly look
    /// like a fresh, runnable candidate (spec instead requires the
    /// fragment-parsing algorithm to mark it already-started at creation
    /// time, for every such API, not just `innerHTML`/`insertAdjacentHTML`
    /// -- the two this ticket's anchors name and `invoke_operation_with_
    /// html_hooks` below actually handles). Flagged as a residual gap in
    /// `docs/lanes/html_TODO.md` rather than silently assumed covered.
    fn connected_script_nodes(&self) -> Vec<(NodeIdx, NodeIdx)> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        let mut roots = vec![NodeIdx(0)];
        roots.extend(state.frames.documents.values().filter_map(|entry| entry.document));
        let mut out = Vec::new();
        for root in roots {
            for node in host.dom.tree_order(root) {
                let is_script = matches!(host.dom.namespace(node), Some(Namespace::Html) | Some(Namespace::Svg))
                    && host.dom.local_name(node) == Some("script");
                if is_script { out.push((root, node)); }
            }
        }
        out
    }

    /// HTML-35: resolve a document root to the realm its scripts should
    /// run in and its base URL for resolving a relative `src`.
    ///
    /// `None` for the realm means "run in whatever realm is already
    /// current" -- correct for the overwhelmingly common case of a script
    /// mutating its own document (same realm the mutating call is already
    /// executing in), and only wrong for the rare cross-realm case of a
    /// child frame reaching into `window.top.document`: there is no
    /// stored handle for "the top realm" to switch to instead (unlike
    /// each iframe's own realm, cached in `frames.windows`). Flagged in
    /// `docs/lanes/html_TODO.md` as a known limitation rather than
    /// silently assumed correct.
    fn realm_and_base_for_document(&self, root: NodeIdx) -> (Option<RealmHandle>, String) {
        let state = self.state.borrow();
        if root == NodeIdx(0) {
            return (None, state.document_url.clone().unwrap_or_default());
        }
        let frame = state.frames.documents.iter()
            .find(|(_, entry)| entry.document == Some(root)).map(|(frame, _)| *frame);
        let base = frame.and_then(|frame| state.frames.documents.get(&frame))
            .map(|entry| entry.url.clone()).unwrap_or_default();
        let realm = frame.and_then(|frame| state.frames.windows.get(&frame)).map(|(realm, _)| *realm);
        (realm, base)
    }

    /// `load`/`error` on a script element are neither bubbling nor
    /// cancelable per spec -- unlike `RuntimeBindings::dispatch_event`'s
    /// hardcoded `bubbles: true` (tuned for iframe's own `load`: a
    /// different element, same event names).
    fn dispatch_script_event(&self, js: &mut Interpreter, node: NodeIdx, name: &str) -> Result<(), Throw> {
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        amazing_bindings::dispatch_user_agent_event(&mut adapter,
            amazing_dom::events::EventTarget::Node(node), name,
            amazing_dom::events::EventInit { bubbles: false, cancelable: false, composed: false })
    }

    /// HTML-35: the ordered candidate list for ONE `invoke_operation_with_
    /// html_hooks` call, matching two distinct WHATWG triggers rather than
    /// a full-document rescan (a rescan double-counts reentrant nested
    /// calls and gets the anchor `dom/nodes/insertion-removing-steps/
    /// Node-appendChild-three-scripts.html` wrong -- see below):
    ///
    /// 1. The script element's own "children changed steps": if
    ///    `mutated_parent` (the node whose child list this operation just
    ///    changed) is *itself* an already-existing `<script>` -- i.e. not
    ///    one of `inserted`, which would make it a *newly connected* node
    ///    instead -- it is checked (and, if eligible, run) FIRST, ahead of
    ///    anything this same operation also just connected. This is what
    ///    makes `s3.appendChild(new Text(...))`, called from *inside*
    ///    `s1`'s own running script body, run `s3` immediately rather than
    ///    waiting for `s3`'s own turn in a document-order walk: `s3` was
    ///    already connected before this nested call, so it is `mutated_
    ///    parent`, not a member of `inserted`.
    /// 2. The DOM standard's `insert` algorithm's own `staticNodeList`:
    ///    the shadow-including-inclusive-descendants of each node THIS
    ///    operation inserted, in tree order, snapshotted once before any
    ///    of them run (the DOM spec's own rationale: "the post-connection
    ///    steps can modify the tree's structure, making live traversal
    ///    unsafe, possibly leading to the post-connection steps being
    ///    called multiple times on the same node").
    fn scoped_script_candidates(&self, mutated_parent: Option<NodeIdx>, inserted: &[NodeIdx]) -> Vec<NodeIdx> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        let is_script = |node: NodeIdx| {
            matches!(host.dom.namespace(node), Some(Namespace::Html) | Some(Namespace::Svg))
                && host.dom.local_name(node) == Some("script")
        };
        let mut out = Vec::new();
        if let Some(parent) = mutated_parent {
            if !inserted.contains(&parent) && is_script(parent) {
                out.push(parent);
            }
        }
        for &node in inserted {
            for descendant in host.dom.tree_order(node) {
                if is_script(descendant) { out.push(descendant); }
            }
        }
        out
    }

    /// HTML-35: `node`'s shadow-including root is a `Document`, per DOM's
    /// own `is_connected` -- but that is also true for a fully detached
    /// `Document`, such as `document.implementation.createHTMLDocument()`'s
    /// return value, which has no browsing context at all. WHATWG HTML's
    /// "scripting is disabled" for exactly that case (`html/semantics/
    /// scripting-1/the-script-element/scripting-enabled.html`: a script
    /// appended to such a document's `documentElement` must never run) --
    /// so a script only qualifies here when its root is a document THIS
    /// driver actually knows as live: the top document, or a connected
    /// iframe's own document (`FrameRuntime::documents`), matching
    /// `connected_script_nodes`'s own root list above.
    fn has_active_document_root(&self, node: NodeIdx) -> bool {
        let state = self.state.borrow();
        let host = state.host.borrow();
        if !host.dom.is_connected(node) { return false; }
        let root = host.dom.root(node);
        root == NodeIdx(0) || state.frames.documents.values().any(|entry| entry.document == Some(root))
    }

    /// HTML-35: "prepare the script element" (WHATWG HTML "processing
    /// model" §"scripting"), for *non-parser-inserted* scripts only --
    /// parser-inserted ones already run through `ResumableParser`'s own
    /// pause signal (`run_interleaved_parse`/`run_interleaved_iframe_
    /// parse` above), which never routes through `invoke_operation_with_
    /// html_hooks` (the node arrives already in the tree directly from the
    /// parser, not via one of that function's hooked JS-binding calls) --
    /// so there is no double-execution risk between the two paths.
    ///
    /// Runs every not-yet-started, *still* rooted in a live document
    /// (`has_active_document_root`, re-checked live, per candidate, right
    /// before it would run -- a still-pending candidate that an earlier
    /// one's script disconnected, the spec's own worked example with
    /// `script1` removing `script2`, must not run), classic-JS-typed,
    /// non-`nomodule` script `scoped_script_candidates` finds for
    /// THIS operation, in that order, marking each already-started
    /// *immediately before* fetching/running it (so a script that mutates
    /// the DOM while running -- including inserting more scripts, or
    /// mutating an already-connected sibling script's children -- can
    /// never see or re-trigger itself; the nested `invoke_operation_with_
    /// html_hooks` call its own mutation causes handles that mutation's
    /// own candidates before returning here, exactly as reentrant WHATWG
    /// "insert" calls nest).
    ///
    /// Module scripts use the realm's module registry and UTF-8 transport;
    /// classic nomodule fallbacks and data blocks remain disqualified.
    ///
    /// Every fetch is synchronous/blocking (`fetch_child_script`, same as
    /// every other script-fetching path in this driver), so `.async`'s
    /// spec distinction from an "in-order" (`async = false`) dynamic
    /// script collapses to no observable difference here: whichever
    /// script this loop reaches first always finishes (fetch and run)
    /// before the next one starts, which already matches document order
    /// for scripts inserted in the same synchronous turn. Honored as an
    /// ordering *outcome*, not by tracking the `.async` IDL flag itself --
    /// see `docs/lanes/html_TODO.md` for why, and for the separate, smaller
    /// pre-existing gap this does NOT fix (`.async`'s own getter reflects
    /// the content attribute directly, so it reads back `false` by default
    /// for a dynamic script rather than spec's `true`-until-explicitly-set;
    /// `amazing-bindings::element_reflection`, not this file).
    fn run_pending_scripts(&self, js: &mut Interpreter, mutated_parent: Option<NodeIdx>, inserted: &[NodeIdx]) {
        let resolver = self.state.borrow().frames.resolver.clone();
        for node in self.scoped_script_candidates(mutated_parent, inserted) {
            if self.state.borrow().frames.already_started_scripts.contains(&node) { continue; }
            if !self.has_active_document_root(node) { continue; }
            let (script, nomodule, is_module) = {
                let state = self.state.borrow();
                let host = state.host.borrow();
                let nomodule = host.dom.get_attribute_ns(node, None, "nomodule").is_some();
                let is_module = host.dom.get_attribute_ns(node, None, "type")
                    .is_some_and(|value| value.eq_ignore_ascii_case("module"));
                (crate::cmd::js::script_source_from_node(&host.dom, node), nomodule, is_module)
            };
            // Neither disqualification marks already-started (see doc
            // comment above): a data-block type, or a classic `nomodule`
            // script, might still become a runnable classic script later
            // if its attributes change and it is reinserted.
            if nomodule && !is_module { continue; }
            let Some(script) = script else { continue }; // wrong type, or no `src`/empty body
            let root = { let state = self.state.borrow(); state.host.borrow().dom.root(node) };
            self.state.borrow_mut().frames.already_started_scripts.insert(node);
            let (realm, base) = self.realm_and_base_for_document(root);
            let script_url = match realm {
                Some(realm) => js.binding_with_realm(realm, |i| Ok(self.page_script_url(i, &script, &base))).unwrap_or_else(|_| base.clone()),
                None => self.page_script_url(js, &script, &base),
            };
            let run_body = |js: &mut Interpreter, body: &str| {
                // A script's own uncaught exception does not stop the page
                // or suppress its `load` event (that pair reflects fetch
                // success/failure, not execution success) -- matches
                // `cmd/js.rs`'s existing top-level script driver, which
                // swallows the same `Err` into a diagnostics list rather
                // than propagating it.
                let _ = match realm {
                    Some(realm) => js.binding_with_realm(realm, |i| self.execute_document_script(i, &script, body, &script_url)),
                    None => self.execute_document_script(js, &script, body, &script_url),
                };
            };
            match &script.src {
                None => {
                    run_body(js, &script.body);
                    if script.kind == crate::cmd::js::ScriptKind::Classic { let _ = self.dispatch_script_event(js, node, "load"); }
                }
                Some(src) => match if script.kind == crate::cmd::js::ScriptKind::Module {
                    match realm {
                        Some(realm) => js.binding_with_realm(realm, |i| self.load_module_source(i, &script_url).map_err(Throw::type_error)).ok(),
                        None => self.load_module_source(js, &script_url).ok(),
                    }
                } else { fetch_child_script(src, &base, resolver.as_ref()) } {
                    Some(text) => {
                        run_body(js, &text);
                        if script.kind == crate::cmd::js::ScriptKind::Classic { let _ = self.dispatch_script_event(js, node, "load"); }
                    }
                    None => { let _ = self.dispatch_script_event(js, node, "error"); }
                },
            }
        }
    }
}

/// A `Rc<RefCell<IncrementalDocumentImport>>` alias, purely so `run_one`'s
/// closure signature above (already long with every other captured piece of
/// per-frame context) does not also spell out the full generic path.
type IncrementalImportRef = Rc<RefCell<amazing_dom::markup::IncrementalDocumentImport>>;

impl RuntimeBindingHost<'_> {
    /// A navigated child document retains its associated WindowProxy even
    /// when a parent-realm script reads the document's defaultView.
    pub(super) fn iframe_document_window(&self, document: NodeIdx) -> Option<JsValue> {
        let state = self.state.borrow();
        let frame = state.frames.documents.iter()
            .find_map(|(frame, entry)| (entry.document == Some(document)).then_some(frame))?;
        state.frames.windows.get(frame).map(|(_, proxy)| proxy.clone())
    }

    /// `[LegacyFactoryFunction=Option]` from the HTML standard: builds a
    /// detached `<option>` with text, value, default-selected and selectedness.
    pub(super) fn install_html_form_globals(&mut self) -> Result<(), Throw> {
        let option = self.native("Option", 4, NativeEntry::OptionConstructor,
            HostCallPolicy::CallAndConstruct);
        let global = self.js.binding_global_object();
        self.define_data_property(&global, "Option", option, true, false, true)
    }

    pub(super) fn option_constructor(&mut self, args: &[JsValue]) -> Result<JsValue, Throw> {
        // WebIDL optional DOMString arguments treat an explicit `undefined` as
        // absent, so `new Option(undefined, undefined)` has no text or value.
        let text = match args.first() {
            Some(value) if !matches!(value, JsValue::Undefined) => Some(self.to_dom_string(value)?),
            _ => None,
        };
        let value = match args.get(1) {
            Some(value) if !matches!(value, JsValue::Undefined) => Some(self.to_dom_string(value)?),
            _ => None,
        };
        let has_default = args.get(2).is_some();
        let default_selected = args.get(2).is_some_and(|v| self.to_boolean(v));
        let has_selected = args.get(3).is_some();
        let selected = args.get(3).is_some_and(|v| self.to_boolean(v));
        let document = self.current_document();
        let node = self.with_dom(|dom| {
            let node = dom.create_element_local_in(
                document, Some("http://www.w3.org/1999/xhtml"), "option")?;
            if let Some(text) = &text {
                if !text.is_empty() {
                    dom.create_text(node, &String::from_utf16_lossy(text));
                }
            }
            if let Some(value) = &value {
                dom.set_attribute(node, "value", &String::from_utf16_lossy(value))?;
            }
            if default_selected {
                dom.set_attribute(node, "selected", "")?;
            }
            // Selectedness comes only from the `selected` argument; a
            // defaultSelected-only option keeps the attribute but is not
            // selected, and neither path sets the dirty flag.
            if has_selected || has_default {
                let selectedness = has_selected && selected;
                dom.set_option_selectedness(node, selectedness)?;
            }
            Ok(node)
        })?;
        self.wrap_node(node)
    }
}

impl RuntimeBindingHost<'_> {
    /// `document.write`/`writeln`/`open`/`close` (HTML-29). While a
    /// document's parser is "active" -- its own top-level scripts are still
    /// running, or it is inside an explicit `open()`/`write()` session --
    /// `write()`/`writeln()` insert at the tracked parser insertion point
    /// (`BindingState::write_insertion_points`); otherwise they implicitly
    /// `open()` first, which replaces the whole document. `open()`/`close()`
    /// still skip the spec's "ignore-opens-during-unload" counter; see
    /// docs/lanes/html_TODO.md.
    pub(super) fn install_document_write_globals(&mut self) -> Result<(), Throw> {
        let prototype = self.prototype(Interface::Document)
            .ok_or_else(|| Throw::type_error("Missing Document prototype"))?;
        let write = self.native("write", 0, NativeEntry::DocumentWrite, HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "write", write, true, false, true)?;
        let writeln = self.native("writeln", 0, NativeEntry::DocumentWriteln, HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "writeln", writeln, true, false, true)?;
        let open = self.native("open", 0, NativeEntry::DocumentOpen, HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "open", open, true, false, true)?;
        let close = self.native("close", 0, NativeEntry::DocumentClose, HostCallPolicy::CallOnly);
        self.define_data_property(&prototype, "close", close, true, false, true)?;
        self.install_html_window_context()
    }

    fn install_html_window_context(&mut self) -> Result<(), Throw> {
        let realm = self.js.binding_current_realm();
        if !self.state.borrow().frames.contexts.contains_key(&realm) {
            let window = self.js.binding_create_window_proxy(realm)?;
            let global = self.js.binding_global_object();
            self.state.borrow_mut().frames.contexts.insert(realm, WindowContext {
                document: NodeIdx(0), frame: None, global, parent: window.clone(), top: window.clone(), window,
                origin: String::new(), installed: false, indices: 0,
            });
        }
        let context = self.state.borrow().frames.contexts[&realm].clone();
        if context.installed { return Ok(()); }
        for property in [HtmlWindowProperty::Frames, HtmlWindowProperty::Length, HtmlWindowProperty::Parent, HtmlWindowProperty::Top] {
            let name = property.name();
            let getter_name = match property { HtmlWindowProperty::Frames => "get frames", HtmlWindowProperty::Length => "get length", HtmlWindowProperty::Parent => "get parent", HtmlWindowProperty::Top => "get top" };
            let setter_name = match property { HtmlWindowProperty::Frames => "set frames", HtmlWindowProperty::Length => "set length", HtmlWindowProperty::Parent => "set parent", HtmlWindowProperty::Top => "set top" };
            let getter = self.native(getter_name, 0, NativeEntry::HtmlWindowGet(property), HostCallPolicy::CallOnly);
            let setter = if matches!(property, HtmlWindowProperty::Top) { None } else {
                Some(self.native(setter_name, 1, NativeEntry::HtmlWindowReplaceableSet(property), HostCallPolicy::CallOnly))
            };
            self.js.binding_define_accessor_property(&context.global, name, Some(&getter), setter.as_ref(),
                true, !matches!(property, HtmlWindowProperty::Top))?;
        }
        self.state.borrow_mut().frames.contexts.get_mut(&realm).unwrap().installed = true;
        let state = self.state.clone();
        self.js.binding_set_window_proxy_access_hook(Rc::new(move |js, access| {
            let current = state.borrow().frames.contexts.get(&access.current).cloned();
            let target = state.borrow().frames.contexts.get(&access.target).cloned();
            let current_origin = match current { Some(context) => context_origin(js, &context, access.current)?, None => String::new() };
            let target_origin = match target { Some(context) => context_origin(js, &context, access.target)?, None => access.target_origin };
            if access.current == access.target || current_origin == target_origin { return Ok(()); }
            let name = match access.key { Some(JsValue::Str(name)) => Some(name.to_utf8_lossy()), _ => None };
            let permitted = name.as_deref().is_some_and(|name| {
                matches!(name, "window" | "self" | "frames" | "length" | "parent" | "top")
                    || name.parse::<u32>().is_ok_and(|index| index != u32::MAX && index.to_string() == name
                        && state.borrow().frames.contexts.get(&access.target).is_some_and(|context| (index as usize) < context.indices))
            });
            if permitted && matches!(access.operation, amazing_js::interp::WindowProxyOperation::Get
                | amazing_js::interp::WindowProxyOperation::GetOwnProperty | amazing_js::interp::WindowProxyOperation::Has) {
                return Ok(());
            }
            let global = js.binding_global_object();
            let constructor = js.binding_get_property(&global, "DOMException")?;
            let error = js.binding_construct(&constructor, vec![JsValue::str("Blocked cross-origin Window access"), JsValue::str("SecurityError")])?;
            Err(js.binding_throw(error))
        }));
        Ok(())
    }

    fn html_window_receiver(&self, receiver: &JsValue) -> Result<(RealmHandle, WindowContext), Throw> {
        let state = self.state.borrow();
        let contexts = &state.frames.contexts;
        let exact_global = contexts.iter().find(|(_, context)| self.js.binding_same_value(receiver, &context.global));
        let live_proxy = contexts.iter().find(|(realm, context)| self.js.binding_same_value(receiver, &context.window)
            && state.frames.live_context(context, **realm));
        let detached_proxy = contexts.iter().find(|(_, context)| self.js.binding_same_value(receiver, &context.window));
        exact_global.or(live_proxy).or(detached_proxy).map(|(realm, context)| (*realm, context.clone()))
            .ok_or_else(|| Throw::type_error("Invalid Window receiver"))
    }

    pub(super) fn html_window_get(&mut self, property: HtmlWindowProperty, receiver: JsValue) -> Result<JsValue, Throw> {
        let (realm, context) = self.html_window_receiver(&receiver)?;
        let live = self.state.borrow().frames.live_context(&context, realm);
        Ok(match property {
            HtmlWindowProperty::Frames => context.window,
            HtmlWindowProperty::Length => {
                let state = self.state.borrow();
                let host = state.host.borrow();
                let count = if live { host.dom.tree_order(context.document).into_iter()
                    .filter(|node| state.frames.windows.contains_key(node)).count() } else { 0 };
                JsValue::num(count as f64)
            }
            HtmlWindowProperty::Parent => if live { context.parent } else { JsValue::Null },
            HtmlWindowProperty::Top => if live { context.top } else { JsValue::Null },
        })
    }

    pub(super) fn html_window_replaceable_set(&mut self, property: HtmlWindowProperty, receiver: JsValue,
        args: &[JsValue]) -> Result<JsValue, Throw> {
        let owner = self.html_window_receiver(&receiver).map(|(_, context)| context.global).unwrap_or(receiver);
        self.js.binding_define_data_property(&owner, property.name(), args.first().cloned().unwrap_or(JsValue::Undefined),
            amazing_js::interp::BindingPropertyFlags { writable: true, enumerable: true, configurable: true })?;
        Ok(JsValue::Undefined)
    }

    fn document_receiver_node(&self, receiver: &JsValue) -> Result<NodeIdx, Throw> {
        let key = self.platform_object(receiver, InterfaceBrand::NODE)
            .ok_or_else(|| Throw::type_error("Invalid Document receiver"))?;
        let node = self.node_from_platform_key(key)
            .ok_or_else(|| Throw::type_error("Stale Document receiver"))?;
        let state = self.state.borrow();
        let host = state.host.borrow();
        if !matches!(host.dom.get(node).kind, NodeKind::Document) {
            return Err(Throw::type_error("Invalid Document receiver"));
        }
        Ok(node)
    }

    pub(super) fn document_open(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        // No-op while already parsing/open (matches treating repeated
        // `open()` calls as idempotent rather than implementing the spec's
        // "ignore-opens-during-unload" counter); otherwise this *is* the
        // implicit replace `write()` performs when called outside a parser.
        if !self.state.borrow().write_insertion_points.contains_key(&document) {
            self.reopen_document(document)?;
        }
        Ok(receiver)
    }

    pub(super) fn document_close(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        end_document_parsing_state(&self.state, document);
        Ok(JsValue::Undefined)
    }

    pub(super) fn document_write(&mut self, receiver: JsValue, args: &[JsValue],
        newline: bool) -> Result<JsValue, Throw> {
        let document = self.document_receiver_node(&receiver)?;
        let mut text = String::new();
        for arg in args {
            let units = self.to_dom_string(arg)?;
            text.push_str(&String::from_utf16_lossy(&units));
        }
        if newline { text.push('\n'); }
        // HTML-31: a live, script-interleaved parser for this document
        // inserts real input-stream text at its own paused cursor -- the
        // actual WHATWG "insertion point" -- instead of `write_into_
        // document`'s DOM-fragment splice, which remains the fallback for
        // every document that never had one (iframes, and any top-level
        // document once its own interleaved parse has reached `Eof`).
        let parser = self.state.borrow().active_parsers.get(&document).cloned();
        if let Some(parser) = parser {
            self.write_into_active_parser(document, &parser, &text)?;
            return Ok(JsValue::Undefined);
        }
        self.write_into_document(document, &text)?;
        Ok(JsValue::Undefined)
    }

    fn write_into_active_parser(&mut self, document: NodeIdx,
        parser: &Rc<RefCell<amazing_html::ResumableParser>>, text: &str) -> Result<(), Throw> {
        let host = self.state.borrow().host.clone();
        let import = self.state.borrow().frames.parser_imports.get(&document).cloned();
        parser.borrow_mut().begin_write(text);
        loop {
            // Top-level parsing shares the arena by swapping. Child parsing
            // uses the registered identity-preserving importer in both directions.
            {
                let mut parser = parser.borrow_mut();
                let mut host = host.borrow_mut();
                if let Some(import) = &import {
                    if let Err(error) = import.borrow_mut().reverse_sync(parser.dom_mut(), &host.dom) {
                        parser.end_write();
                        return Err(Throw::type_error(format!("document.write reverse graft: {error:?}")));
                    }
                } else { parser.swap_dom(&mut host.dom); }
            }
            let script = parser.borrow_mut().run_write();
            {
                let mut parser = parser.borrow_mut();
                let mut host = host.borrow_mut();
                if let Some(import) = &import {
                    if let Err(error) = import.borrow_mut().sync(&mut host.dom, parser.dom()) {
                        if script.is_some() { parser.end_write(); }
                        return Err(Throw::type_error(format!("document.write graft: {error:?}")));
                    }
                } else { parser.swap_dom(&mut host.dom); }
            }
            RuntimeBindings { state: self.state.clone() }.navigate_iframes(self.js);
            let Some(source_node) = script else { return Ok(()); };
            let node = match &import { Some(import) => import.borrow().imported(source_node), None => Some(source_node) };
            let script = node.and_then(|node| crate::cmd::js::script_source_from_node(&host.borrow().dom, node));
            let outcome = if let Some(script) = script {
                let body = match script.src {
                    None => Some(script.body),
                    Some(src) => {
                        let state = self.state.borrow();
                        state.document_url.as_deref().and_then(|base|
                            fetch_child_script(&src, base, state.frames.resolver.as_ref()))
                    }
                };
                match body { Some(body) => self.js.run_script(&body).map(|_| ()), None => Ok(()) }
            } else { Ok(()) };
            if let Err(error) = outcome {
                parser.borrow_mut().end_write();
                return Err(error);
            }
        }
    }

    /// Replaces `document`'s entire content with a fresh, empty
    /// `<html><head></head><body></body></html>` and marks its parser
    /// active with the insertion point at the start of that new, empty
    /// body -- `document.open()`'s full-document replacement, and the step
    /// `write()`/`writeln()` perform implicitly when called while the
    /// document is not already being parsed.
    fn reopen_document(&mut self, document: NodeIdx) -> Result<(), Throw> {
        // No `<!doctype html>`: `import_fragment` copies each of the
        // skeleton's top-level nodes into a `DocumentFragment` one at a
        // time (`import_node_into`/`pre_insert`), and a `DocumentType` is
        // never a valid `DocumentFragment` child -- a real
        // `document.open()` doesn't add one either.
        let skeleton = amazing_html::parse("<html><head></head><body></body></html>");
        let body = self.with_dom(|dom| {
            let fragment = dom.import_fragment(&skeleton, document)?;
            dom.replace_all(document, Some(fragment))?;
            Ok(dom.document_body(document)?.unwrap_or(document))
        })?;
        self.state.borrow_mut().write_insertion_points.insert(document, (body, None));
        Ok(())
    }

    /// Parses `text` as an HTML fragment and inserts it at `document`'s
    /// current parser insertion point (`BindingState::write_insertion_points`),
    /// implicitly performing `document.open()`'s full-document replace first
    /// if the document's parser is not currently marked active -- e.g. a
    /// `write()` call from a `setTimeout` after the document has already
    /// finished loading. Any classic `<script>` elements the fragment
    /// introduces run synchronously and in document order before this
    /// returns, inline immediately and external fetched relative to the
    /// document's own URL, each narrowing the insertion point to its own
    /// position first (a *nested* write from such a script lands right
    /// after that script, not after the original writing script); the
    /// outer insertion point is restored once they have all run, so a
    /// second `write()` call from the original script still lands in
    /// sequence after the first.
    fn write_into_document(&mut self, document: NodeIdx, text: &str) -> Result<(), Throw> {
        if !self.state.borrow().write_insertion_points.contains_key(&document) {
            self.reopen_document(document)?;
        }
        let scripting_enabled = self.with_dom(|dom| dom.document_scripting_enabled(document))?;
        let mode = if scripting_enabled { amazing_html::ScriptingMode::Enabled }
            else { amazing_html::ScriptingMode::Disabled };
        let parsed = amazing_html::parse_fragment_with_scripting_mode("body", Namespace::Html, text, mode)
            .map_err(|_| Throw::type_error("Unable to parse document.write markup"))?;
        let anchor = self.state.borrow().write_insertion_points.get(&document).copied();
        let children = self.with_dom(|dom| {
            let (parent, before) = match anchor {
                // `parent == document` is the placeholder `begin_document_
                // parsing_state` leaves before any script has narrowed it
                // (or a stale one from a document that was never
                // reopened): fall back to the old body/head/document-
                // element resolution rather than inserting directly under
                // the Document node, which `pre_insert` rejects.
                Some((parent, before)) if parent != document => (parent, before),
                _ => {
                    let insertion_parent = dom.document_body(document)?
                        .or(dom.document_head(document)?)
                        .or(dom.document_element(document)?)
                        .unwrap_or(document);
                    (insertion_parent, None)
                }
            };
            let fragment = dom.import_fragment(&parsed, document)?;
            let children: Vec<NodeIdx> = dom.children(fragment).collect();
            dom.pre_insert(fragment, parent, before)?;
            Ok(children)
        })?;
        if !scripting_enabled { return Ok(()); }
        let mut scripts = Vec::new();
        {
            let state = self.state.borrow();
            let host = state.host.borrow();
            for &child in &children {
                collect_classic_scripts(&host.dom, child, &mut scripts);
            }
        }
        if scripts.is_empty() { return Ok(()); }
        let base = self.state.borrow().document_url.clone();
        let resolver = self.state.borrow().frames.resolver.clone();
        let saved = self.state.borrow().write_insertion_points.get(&document).copied();
        let bindings = RuntimeBindings { state: self.state.clone() };
        for script in &scripts {
            narrow_write_insertion_point_state(&self.state, document, script.node);
            let base = base.as_deref().unwrap_or("about:blank");
            match &script.src {
                None => { bindings.execute_document_script(self.js, script, &script.body, base)?; }
                Some(src) => {
                    let url = crate::cmd::js::resolve_script_url(base, src);
                    let text = if script.kind == crate::cmd::js::ScriptKind::Module {
                        bindings.load_module_source(self.js, &url).ok()
                    } else { fetch_child_script(src, base, resolver.as_ref()) };
                    if let Some(text) = text {
                        bindings.execute_document_script(self.js, script, &text, &url)?;
                    }
                }
            }
        }
        if let Some(point) = saved {
            self.state.borrow_mut().write_insertion_points.insert(document, point);
        }
        Ok(())
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn navigate_form(&mut self, request: FormNavigationRequest) -> Result<(), Throw> {
        let hook = self.state.borrow().frames.form_navigation_hook.clone();
        if let Some(hook) = hook {
            hook(request);
            return Ok(());
        }
        // A `target` naming a connected iframe's `name` content attribute
        // lands the navigation in that child browsing context instead of
        // top-level, by setting its `src` the same way `iframe.src = ...`
        // would; the special browsing-context keywords (`_self` and an
        // empty target, `_blank`, `_parent`, `_top`) never match a frame
        // name and always fall through to top-level navigation. POST bodies
        // are not carried into a targeted iframe navigation yet (the
        // existing child-navigation fetch is GET-only): documented in
        // docs/lanes/html_TODO.md rather than silently dropped here.
        if let Some(frame) = self.named_frame(&request.target) {
            let host = self.state.borrow().host.clone();
            host.borrow_mut().dom.set_attribute(frame, "src", &request.url)
                .map_err(|error| Throw::type_error(format!("targeted iframe navigation: {error:?}")))?;
            RuntimeBindings { state: self.state.clone() }.navigate_iframes(self.js);
            return Ok(());
        }
        // Current top-level navigation entry; IPC-10 replaces this URL-only
        // fallback with the structured request above for POST and targets.
        self.js.binding_location_navigate(&request.url, false);
        self.state.borrow_mut().document_url = Some(request.url);
        Ok(())
    }

    /// The connected iframe whose `name` content attribute exactly matches
    /// `target`, or `None` for an empty target, the `_self`/`_blank`/
    /// `_parent`/`_top` keywords (spec special browsing-context names, which
    /// always start with `_` and never match a frame name), or no match.
    fn named_frame(&self, target: &str) -> Option<NodeIdx> {
        if target.is_empty() || target.starts_with('_') { return None; }
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        host.dom.tree_order(NodeIdx(0)).into_iter().find(|&node| {
            host.dom.namespace(node) == Some(Namespace::Html)
                && host.dom.local_name(node) == Some("iframe")
                && host.dom.get_attribute_ns(node, None, "name") == Some(target)
        })
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_iframe_members(&mut self) -> Result<(), Throw> {
        let prototype = self.prototype(Interface::HTMLIFrameElement)
            .ok_or_else(|| Throw::type_error("HTMLIFrameElement prototype missing"))?;
        let getter = self.native("get contentDocument", 0,
            NativeEntry::IFrameContentDocument, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "contentDocument", Some(getter), None, true, true)?;
        let window_getter = self.native("get contentWindow", 0,
            NativeEntry::IFrameContentWindow, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "contentWindow", Some(window_getter), None, true, true)?;
        let src_getter = self.native("get src", 0, NativeEntry::IFrameSrcGet, HostCallPolicy::CallOnly);
        let src_setter = self.native("set src", 1, NativeEntry::IFrameSrcSet, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "src", Some(src_getter), Some(src_setter), true, true)?;
        let srcdoc_getter = self.native("get srcdoc", 0,
            NativeEntry::IFrameSrcdocGet, HostCallPolicy::CallOnly);
        let srcdoc_setter = self.native("set srcdoc", 1,
            NativeEntry::IFrameSrcdocSet, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "srcdoc", Some(srcdoc_getter), Some(srcdoc_setter), true, true)?;
        // `onload` is an `EventHandlerNonNull?` IDL attribute (HTML's
        // GlobalEventHandlers mixin): unlike an `addEventListener`-registered
        // listener, it is content-attribute-style state keyed by (target,
        // event type), the same slot `dom.events()`/`events_mut()` already
        // expose generically and that `HTMLElement.onclick` (see
        // `input_activation_bindings::invoke` in amazing-bindings) already
        // uses for "click". No interface elsewhere in this engine wires
        // `onload` yet (an iframe's own `load` event is HTML-25's, entirely
        // this lane's own concern), so it is installed here rather than
        // filed as a cross-lane request.
        let onload_getter = self.native("get onload", 0,
            NativeEntry::IFrameOnloadGet, HostCallPolicy::CallOnly);
        let onload_setter = self.native("set onload", 1,
            NativeEntry::IFrameOnloadSet, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "onload", Some(onload_getter), Some(onload_setter), true, true)
    }

    fn iframe_node(&self, receiver: &JsValue) -> Result<NodeIdx, Throw> {
        let key = self.platform_object(receiver, InterfaceBrand::NODE)
            .ok_or_else(|| Throw::type_error("Invalid HTMLIFrameElement receiver"))?;
        let node = self.node_from_platform_key(key)
            .ok_or_else(|| Throw::type_error("Stale HTMLIFrameElement receiver"))?;
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        if host.dom.namespace(node) != Some(Namespace::Html)
            || host.dom.local_name(node) != Some("iframe") {
            return Err(Throw::type_error("Invalid HTMLIFrameElement receiver"));
        }
        Ok(node)
    }

    pub(super) fn iframe_src_get(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        let source = state.host.borrow().dom.get_attribute_ns(node, None, "src").map(str::to_owned);
        let Some(source) = source else { return Ok(JsValue::str("")) };
        let resolved = state.document_url.as_deref()
            .and_then(|base| ParsedUrl::parse(base).ok())
            .and_then(|base| base.resolve(&source).ok())
            .map_or(source, |url| url.serialize());
        Ok(JsValue::str(&resolved))
    }

    pub(super) fn iframe_src_set(&mut self, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        self.iframe_node(&receiver)?;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        self.invoke_operation_with_html_hooks(Operation::ElementSetAttribute, receiver,
            &[JsValue::str("src"), value])
    }

    /// `srcdoc` reflects its content attribute directly (a plain `DOMString`,
    /// unlike `src` there is no URL resolution): HTML `attribute DOMString
    /// srcdoc;` on `HTMLIFrameElement`.
    pub(super) fn iframe_srcdoc_get(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        let source = state.host.borrow().dom.get_attribute_ns(node, None, "srcdoc").map(str::to_owned);
        Ok(JsValue::str(source.as_deref().unwrap_or("")))
    }

    pub(super) fn iframe_srcdoc_set(&mut self, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        self.iframe_node(&receiver)?;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        self.invoke_operation_with_html_hooks(Operation::ElementSetAttribute, receiver,
            &[JsValue::str("srcdoc"), value])
    }

    pub(super) fn iframe_onload_get(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let target = amazing_dom::events::EventTarget::Node(node);
        let root = self.state.borrow().host.borrow().dom.events().event_handler(target, "load");
        Ok(root.and_then(|root| self.root_value(root)).unwrap_or(JsValue::Null))
    }

    pub(super) fn iframe_onload_set(&mut self, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        // `EventHandlerNonNull?`: a non-object (including null/undefined)
        // clears the handler without throwing; an object is stored as-is,
        // matching `onclick`'s own handling (callable-ness is only checked
        // when the handler is invoked, never when it is assigned).
        let is_object = self.is_object_value(&value)?;
        let callback = if is_object { Some(self.retain_value(value)) } else { None };
        let target = amazing_dom::events::EventTarget::Node(node);
        let old = self.state.borrow_mut().host.borrow_mut().dom.events_mut()
            .set_event_handler(target, "load", callback);
        if let Some(old) = old { self.release_value(old); }
        Ok(JsValue::Undefined)
    }

    pub(super) fn iframe_content_document(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        let host = state.host.borrow();
        if host.dom.root(node) != NodeIdx(0) { return Ok(JsValue::Null); }
        let Some(frame) = state.frames.documents.get(&node) else { return Ok(JsValue::Null) };
        let Some(document) = frame.document else { return Ok(JsValue::Null) };
        // `about:srcdoc` is same-origin-inherited from the parent, like
        // `about:blank`, so it needs no URL-based origin comparison; a
        // sandboxed frame without `allow-same-origin` gets a fresh opaque
        // origin every navigation (see `setup_iframe_window`) and so is
        // never same-origin with the parent, regardless of its URL.
        let opaque = frame.sandbox.sandboxed && !frame.sandbox.allow_same_origin;
        let visible = !opaque && (frame.url == "about:blank" || frame.url == "about:srcdoc"
            || state.document_url.as_deref().is_some_and(|parent| same_origin(parent, &frame.url)));
        let realm = state.frames.windows.get(&node).map(|(realm, _)| *realm);
        drop(host);
        drop(state);
        if !visible { return Ok(JsValue::Null); }
        // Wrap through the child's own realm when one exists, so the host
        // object cache there (keyed per realm) returns the identical wrapper
        // `setup_iframe_window` already cached as `contentWindow.document`,
        // with that realm's own Document.prototype rather than the caller's.
        match realm {
            Some(realm) => {
                let state = self.state.clone();
                self.js.binding_with_realm(realm, |i| {
                    RuntimeBindingHost { js: i, state: state.clone() }.wrap_node(document)
                })
            }
            None => self.wrap_node(document),
        }
    }

    /// Unlike `contentDocument`, `contentWindow` is not nulled cross-origin:
    /// the WindowProxy itself is always exposed once there is a browsing
    /// context, and cross-origin property access is what the WindowProxy's
    /// own origin check (JSRT-9) restricts. Returns null only before the
    /// realm/proxy has been set up (`setup_iframe_window` runs when the
    /// driver dispatches the frame's pending load).
    pub(super) fn iframe_content_window(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        if state.host.borrow().dom.root(node) != NodeIdx(0) { return Ok(JsValue::Null); }
        let window = state.frames.windows.get(&node).map(|(_, proxy)| proxy.clone());
        Ok(window.unwrap_or(JsValue::Null))
    }

    /// HTML-35: converts a JS-visible `Node` argument or receiver into its
    /// `NodeIdx`, `None` for anything else (a `DOMString` argument to
    /// `append`/`prepend`/`replaceChildren`/`before`/`after`/`replaceWith`,
    /// or a stale/foreign platform object).
    fn node_from_value(&self, value: &JsValue) -> Option<NodeIdx> {
        let key = self.platform_object(value, InterfaceBrand::NODE)?;
        self.node_from_platform_key(key)
    }

    /// HTML-35: the node(s) this operation is about to insert (`Node`
    /// arguments only -- a `DOMString` argument to `append`/`prepend`/
    /// `replaceChildren`/`before`/`after`/`replaceWith` contributes text,
    /// never a script), resolved *before* the operation runs.
    ///
    /// This must run pre-op: `appendChild`/`append`/etc. of a
    /// `DocumentFragment` moves the fragment's children out and leaves it
    /// empty (WHATWG DOM's own `insert` algorithm: "if node is a
    /// DocumentFragment node ... let nodes be node's children"), so
    /// resolving it post-op would see nothing -- exactly the bug that made
    /// `Node-appendChild-three-scripts-from-fragment.html` run zero
    /// scripts instead of three when this only expanded post-op. Expands
    /// each fragment argument to its (pre-op) direct children in their
    /// place; a non-fragment `Node` argument is used as-is.
    fn pre_insertion_nodes(&self, operation: Operation, args: &[JsValue]) -> Vec<NodeIdx> {
        let relevant: &[JsValue] = match operation {
            Operation::AppendChild | Operation::InsertBefore | Operation::ReplaceChild =>
                args.first().map(std::slice::from_ref).unwrap_or(&[]),
            Operation::Append | Operation::Prepend | Operation::ReplaceChildren
            | Operation::Before | Operation::After | Operation::ReplaceWith => args,
            _ => return Vec::new(),
        };
        let nodes: Vec<NodeIdx> = relevant.iter().filter_map(|value| self.node_from_value(value)).collect();
        let state = self.state.borrow();
        let host = state.host.borrow();
        let mut out = Vec::new();
        for node in nodes {
            if matches!(host.dom.get(node).kind, NodeKind::DocumentFragment) {
                out.extend(host.dom.children(node));
            } else {
                out.push(node);
            }
        }
        out
    }

    /// HTML-35: the node whose child list this operation may have just
    /// changed, for `run_pending_scripts`'s "children changed steps"
    /// check -- read *after* the operation has already run, so `Before`/
    /// `After` -- which key off a sibling, not a parent directly -- can
    /// resolve that sibling's (unaffected) parent, and `ReplaceWith` --
    /// whose `receiver` itself just left the tree, so its OWN post-op
    /// parent is gone -- can fall back to one of `pre_inserted`'s (now
    /// correctly placed) nodes instead. `Remove`'s receiver is likewise
    /// the node that just left the tree, with no surviving replacement to
    /// fall back to, so its former parent isn't recoverable here -- a
    /// script losing its only child via `.remove()` called *on that
    /// child* does not re-run it, a narrow, documented gap. `RemoveChild`
    /// and `SetTextContent`'s receiver IS the parent itself, unaffected by
    /// either op, and so is covered normally.
    fn operation_mutated_parent(&self, operation: Operation, receiver: &JsValue,
        pre_inserted: &[NodeIdx]) -> Option<NodeIdx> {
        let parent_of = |node: NodeIdx| {
            let state = self.state.borrow();
            state.host.borrow().dom.get(node).parent
        };
        match operation {
            Operation::Append | Operation::Prepend | Operation::ReplaceChildren
            | Operation::AppendChild | Operation::InsertBefore | Operation::ReplaceChild
            | Operation::RemoveChild | Operation::SetTextContent => self.node_from_value(receiver),
            Operation::Before | Operation::After => self.node_from_value(receiver).and_then(parent_of),
            Operation::ReplaceWith => pre_inserted.first().copied().and_then(parent_of)
                .or_else(|| self.node_from_value(receiver).and_then(parent_of)),
            _ => None,
        }
    }

    pub(super) fn invoke_operation_with_html_hooks(&mut self, operation: Operation,
        receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        // HTML-35: a script produced by a fragment-parsing API (`innerHTML`,
        // `insertAdjacentHTML`) must never execute, however it is later
        // moved (WHATWG HTML: the fragment-parsing algorithm marks every
        // script element it produces already-started, unconditionally).
        // Snapshot which script nodes already exist *before* the parse, so
        // any script that appears only afterward -- this operation's own
        // newly-parsed content, and only that -- can be marked already-
        // started below instead of treated as a fresh candidate by
        // `run_pending_scripts`.
        let fragment_parsed = matches!(operation,
            Operation::ElementSetInnerHTML | Operation::ElementInsertAdjacentHTML);
        let bindings = RuntimeBindings { state: self.state.clone() };
        let before_scripts = fragment_parsed
            .then(|| bindings.connected_script_nodes().into_iter().map(|(_, node)| node)
                .collect::<HashSet<_>>());
        let receiver_for_scripts = receiver.clone();
        let pre_inserted = self.pre_insertion_nodes(operation, args);
        let result = self.invoke_operation_with_canvas_reset(operation, receiver, args);
        if result.is_ok() && matches!(operation,
            Operation::Append | Operation::Prepend | Operation::ReplaceChildren
            | Operation::Before | Operation::After | Operation::ReplaceWith | Operation::Remove
            | Operation::AppendChild | Operation::InsertBefore | Operation::RemoveChild
            | Operation::ReplaceChild | Operation::ElementSetInnerHTML
            | Operation::ElementInsertAdjacentHTML | Operation::ElementSetAttribute
            | Operation::ElementSetAttributeNS | Operation::ElementRemoveAttribute
            | Operation::ElementRemoveAttributeNS | Operation::ElementSetAttributeNode
            | Operation::ElementSetAttributeNodeNS | Operation::ElementRemoveAttributeNode
            | Operation::SetTextContent) {
            bindings.navigate_iframes(self.js);
            if let Some(before) = before_scripts {
                let after: Vec<NodeIdx> = bindings.connected_script_nodes()
                    .into_iter().map(|(_, node)| node).collect();
                let mut state = self.state.borrow_mut();
                for node in after {
                    if !before.contains(&node) { state.frames.already_started_scripts.insert(node); }
                }
            }
            let mutated_parent = self.operation_mutated_parent(operation, &receiver_for_scripts, &pre_inserted);
            bindings.run_pending_scripts(self.js, mutated_parent, &pre_inserted);
            // Follow-up from HTML-35's own write-up: `template.innerHTML`'s
            // fragment-parsing result lands in the template's *own*
            // `.content` (a permanently detached `DocumentFragment`, per
            // WHATWG HTML's "template contents owner document"), which
            // `connected_script_nodes()` above can never see -- it only
            // walks known, live document roots -- so the before/after diff
            // just above never marks scripts placed there. There is no
            // "before" to diff against anyway (a template's `.content` is
            // replaced wholesale on every `innerHTML` set, per `markup_
            // bindings.rs`), so mark every script inside it already-started
            // directly, unconditionally, the same way the diff marks a live
            // root's newly-parsed scripts.
            if matches!(operation, Operation::ElementSetInnerHTML) {
                if let Some(node) = self.node_from_value(&receiver_for_scripts) {
                    let scripts = {
                        let state = self.state.borrow();
                        let host = state.host.borrow();
                        host.dom.template_contents(node).map(|content| {
                            host.dom.tree_order(content).into_iter()
                                .filter(|&n| matches!(host.dom.namespace(n), Some(Namespace::Html) | Some(Namespace::Svg))
                                    && host.dom.local_name(n) == Some("script"))
                                .collect::<Vec<_>>()
                        })
                    };
                    if let Some(scripts) = scripts {
                        let mut state = self.state.borrow_mut();
                        for script in scripts { state.frames.already_started_scripts.insert(script); }
                    }
                }
            }
        }
        // Follow-up from HTML-35's own write-up: `DOMParser.parseFromString`'s
        // result document has no browsing context (WHATWG: parsed "with
        // scripting disabled"), which `has_active_document_root` already
        // keeps from ever running *while the document stays orphaned* -- but
        // a script it produced can still be moved into a real, connected
        // document later (e.g. `document.adoptNode`), at which point it
        // would wrongly look like a fresh, runnable candidate. Marked
        // already-started unconditionally at creation time instead, exactly
        // like every other script a fragment/document-parsing algorithm
        // produces (`Range.createContextualFragment` isn't implemented by
        // this engine at all yet, so there is nothing to cover there).
        if matches!(operation, Operation::DOMParserParseFromString) {
            if let Ok(value) = &result {
                if let Some(document) = self.node_from_value(value) {
                    let scripts = {
                        let state = self.state.borrow();
                        let host = state.host.borrow();
                        host.dom.tree_order(document).into_iter()
                            .filter(|&n| matches!(host.dom.namespace(n), Some(Namespace::Html) | Some(Namespace::Svg))
                                && host.dom.local_name(n) == Some("script"))
                            .collect::<Vec<_>>()
                    };
                    let mut state = self.state.borrow_mut();
                    for script in scripts { state.frames.already_started_scripts.insert(script); }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{Shutdown, TcpListener};

    #[test]
    fn windows_expose_actual_live_children_and_proxy_identity() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe id=a></iframe><iframe id=b></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(r#"
            const a=document.getElementById('a'), b=document.getElementById('b');
            const w=a.contentWindow;
            let checkpoint=0; function check(x) { checkpoint++; if(!x) throw new Error('live Window topology '+checkpoint); }
            check(window===self && window===globalThis && window===document.defaultView);
            check(window.frames===window && window.parent===window && window.top===window);
            check(window.length===2 && window[0]===w && window[1]===b.contentWindow);
            check(w.frames===w && w.parent===window && w.top===window && w.length===0);
            const grand=w.document.createElement('iframe'); w.document.body.appendChild(grand);
            check(w.length===1 && w[0]===grand.contentWindow);
            check(grand.contentWindow.parent===w && grand.contentWindow.top===window);
            const d=Object.getOwnPropertyDescriptor(window,'0');
            check(d.value===w && !d.writable && d.enumerable && d.configurable);
            check(!Reflect.defineProperty(window,'0',{value:42}));
            check(!Reflect.set(window,'0',42)); check(!Reflect.deleteProperty(window,'0'));
            a.srcdoc='<!doctype html><p>new'; check(a.contentWindow===w && window[0]===w);
            a.remove(); check(window.length===1 && window[0]===b.contentWindow && !('1' in window));
            check(w.parent===null && w.top===null && w.length===0 && w.frames===w);
            true
        "#).unwrap_or_else(|error| panic!("Window topology failed: {:?}", scheduler.js.binding_get_property(&error.0, "message"))), JsValue::Bool(true));
    }

    #[test]
    fn window_getters_check_receiver_and_replaceable_descriptors() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(r#"
            function check(x) { if(!x) throw new Error('Window descriptors'); }
            const child=document.querySelector('iframe').contentWindow;
            for(const name of ['frames','length','parent']) {
                const d=Object.getOwnPropertyDescriptor(window,name);
                check(typeof d.get==='function' && typeof d.set==='function' && d.enumerable && d.configurable);
                let rejected=false; try { d.get.call({}); } catch(e) { rejected=e instanceof TypeError; } check(rejected);
                check(d.get.call(child)===(name==='frames'?child:name==='length'?0:window));
                const ordinary={}; d.set.call(ordinary,456); check(ordinary[name]===456);
                d.set.call(child,123); check(child[name]===123);
                const replaced=Object.getOwnPropertyDescriptor(child,name);
                check(replaced.writable && replaced.enumerable && replaced.configurable);
            }
            const top=Object.getOwnPropertyDescriptor(window,'top');
            check(top.get.call(child)===window && top.set===undefined && !top.configurable && top.enumerable);
            true
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn cross_origin_window_topology_is_readable_but_document_is_protected() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><iframe sandbox></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(r#"
            const child=document.querySelector('iframe').contentWindow;
            const valid=child.frames===child && child.parent===window && child.top===window && child.length===0;
            let rejected=false; try { child.document; } catch(e) { rejected=e instanceof DOMException && e.name==='SecurityError'; }
            valid && rejected
        "#).unwrap(), JsValue::Bool(true));
    }

    fn response_with_mime(mime: Option<&str>, no_sniff: bool) -> amazing_net::Response {
        let mut headers = amazing_net::Headers::new();
        if let Some(mime) = mime { headers.append("content-type", mime.into()).unwrap(); }
        if no_sniff { headers.append("x-content-type-options", "NoSniff".into()).unwrap(); }
        amazing_net::Response { url: "https://example.test/encoding.py?label=big5".into(),
            status: 200, reason: "OK".into(), headers,
            body: b"<!doctype html><meta charset=big5><body>decoded".to_vec() }
    }

    #[test]
    fn missing_or_unknown_response_mime_sniffs_real_html_bytes_and_encoding() {
        for supplied in [None, Some("unknown/unknown"), Some("application/unknown"), Some("*/*")] {
            let response = response_with_mime(supplied, false);
            let mime = content_type(&response, &response.url, false).unwrap();
            assert_eq!(mime, "text/html");
            let (dom, encoding) = parse_response(&response.body, &mime, None).unwrap();
            assert!(dom.dump(amazing_dom::NodeIdx(0)).contains("decoded"));
            assert_eq!(encoding, "Big5");
        }
    }

    #[test]
    fn explicit_mime_and_nosniff_are_authoritative_for_html_signatures() {
        for mime in ["text/plain", "application/octet-stream", "image/png", "application/xml"] {
            let response = response_with_mime(Some(mime), false);
            let actual = content_type(&response, &response.url, false);
            assert_eq!(actual.as_deref(), if mime == "application/octet-stream" { None } else { Some(mime) });
        }
        assert_eq!(content_type(&response_with_mime(None, true), "https://example.test/page", false), None);
        let response = response_with_mime(Some("text/html"), true);
        assert_eq!(content_type(&response, &response.url, false).as_deref(), Some("text/html"));
    }

    #[test]
    fn html_signature_matching_uses_exact_whitespace_and_tag_boundaries() {
        for html in ["<!doctype html>", " \t\n\u{c}\r<HTML lang=en>", "<p>", "<!-- comment -->", "<sCrIpT >"] {
            assert!(has_html_signature(html.as_bytes()), "{html:?}");
        }
        for other in ["<htmlish>", "<html/>", "<html\t>", "<!--comment-->", "prefix<html>", "\u{b}<html>", "<html", "<?xml version='1.0'?>", ""] {
            assert!(!has_html_signature(other.as_bytes()), "{other:?}");
        }
        assert!(!has_html_signature(format!("{}<html>", " ".repeat(1445)).as_bytes()));
    }

    #[test]
    fn iframe_document_metadata_uses_canonical_names_after_decode_precedence() {
        for (bytes, transport, name) in [
            (&b"\xef\xbb\xbf<p>bom"[..], Some("big5"), "UTF-8"),
            (&b"<meta charset=big5><p>meta"[..], None, "Big5"),
            (&b"<meta charset=big5><p>transport"[..], Some("shift_jis"), "Shift_JIS"),
            (&b"<p>fallback"[..], None, "windows-1252"),
        ] {
            assert_eq!(parse_response(bytes, "text/html", transport).unwrap().1, name);
        }
        assert_eq!(parse_response(b"<root/>", "application/xml", Some("iso-8859-2")).unwrap().1, "ISO-8859-2");
        assert_eq!(amazing_html::encoding::Encoding::Utf8.name(), "utf-8");
    }

    #[test]
    fn iframe_scripts_observe_mode_at_each_graft_and_srcdoc_is_no_quirks() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-mode-{}.html", std::process::id()));
        std::fs::write(&path, "<script>window.seenMode = document.compatMode;</script><p>child").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><iframe src=child.html></iframe><iframe srcdoc='<script>window.seenMode = document.compatMode;</script><p>srcdoc'></iframe>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resource = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resource.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("document.querySelectorAll('iframe')[0].contentWindow.seenMode + '|' + document.querySelectorAll('iframe')[0].contentDocument.compatMode + '|' + document.querySelectorAll('iframe')[1].contentWindow.seenMode").unwrap(),
            JsValue::str("BackCompat|BackCompat|CSS1Compat"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn root_document_compat_mode_reflects_the_actual_parser_mode() {
        for (source, expected) in [
            ("<body><p>no doctype", "BackCompat"),
            ("<!doctype html><body><p>standard", "CSS1Compat"),
            ("<!doctype html PUBLIC '-//W3C//DTD XHTML 1.0 Transitional//EN' 'x'><p>limited", "CSS1Compat"),
        ] {
            let host = Rc::new(RefCell::new(BrowserDomHost {
                dom: amazing_html::parse(source), native_forwarded: false,
            }));
            let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
            assert_eq!(scheduler.run_script("document.compatMode").unwrap(), JsValue::str(expected));
        }
    }

    #[test]
    fn sandboxed_iframe_uses_disabled_noscript_tree_rules() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><iframe sandbox='allow-same-origin' srcdoc='<noscript><b id=visible>fallback</b></noscript><script>window.shouldNotRun=true;</script>'></iframe>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("document.querySelector('iframe').contentDocument.getElementById('visible').textContent").unwrap(), JsValue::str("fallback"));
        assert_eq!(scheduler.run_script("document.querySelector('iframe').contentWindow.shouldNotRun === undefined").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn iframe_scripting_state_comes_from_context_for_html_xml_xhtml_and_blank() {
        let xml = std::env::temp_dir().join(format!("amazing-scripting-context-{}.xml", std::process::id()));
        let xhtml = xml.with_extension("xhtml");
        std::fs::write(&xml, "<root/>").unwrap();
        std::fs::write(&xhtml, "<html xmlns='http://www.w3.org/1999/xhtml'><body/></html>").unwrap();
        for (sandbox, expected) in [("", true), (" sandbox='allow-scripts allow-same-origin'", true),
            (" sandbox='allow-same-origin'", false), (" sandbox", false)] {
            for (source, mime) in [("", "text/html"), (" srcdoc='<noscript><b>x</b></noscript>'", "text/html"),
                (" src='child.xml'", "application/xml"), (" src='child.xhtml'", "application/xhtml+xml")] {
                let host = Rc::new(RefCell::new(BrowserDomHost {
                    dom: amazing_html::parse(&format!("<!doctype html><iframe{sandbox}{source}></iframe>")), native_forwarded: false,
                }));
                let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
                let xml_path = xml.clone(); let xhtml_path = xhtml.clone();
                scheduler.set_frame_resource_resolver(Rc::new(move |url| Some(
                    if url.ends_with(".xhtml") { xhtml_path.to_string_lossy().into_owned() }
                    else { xml_path.to_string_lossy().into_owned() })));
                let documents = Rc::new(RefCell::new(Vec::new()));
                let observed = documents.clone();
                scheduler.set_child_navigation_hook(Rc::new(move |navigation| {
                    if let Some(document) = navigation.document { observed.borrow_mut().push(document); }
                }));
                scheduler.set_location_url("https://example.test/main.html");
                scheduler.run_script("void document.querySelector('iframe')").unwrap();
                assert!(!documents.borrow().is_empty());
                for &document in documents.borrow().iter() {
                    let dom = &host.borrow().dom;
                    assert_eq!(dom.document_scripting_enabled(document).unwrap(), expected, "{source}{sandbox}");
                    assert_eq!(dom.document_metadata(document).unwrap().unwrap().content_type, mime);
                }
                scheduler.dispatch_pending_iframe_loads().unwrap();
                for &document in documents.borrow().iter() {
                    assert_eq!(host.borrow().dom.document_scripting_enabled(document).unwrap(), expected);
                }
            }
        }
        std::fs::remove_file(xml).unwrap(); std::fs::remove_file(xhtml).unwrap();
    }

    #[test]
    fn inert_document_write_uses_disabled_noscript_and_does_not_execute_scripts() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        assert_eq!(scheduler.run_script(r#"
            var inert = document.implementation.createHTMLDocument('inert');
            inert.write('<noscript><b id="fallback">visible</b></noscript><script>window.inertWriteRan=true;</script>');
            inert.getElementById('fallback').textContent === 'visible' && window.inertWriteRan === undefined;
        "#).unwrap(), JsValue::Bool(true));
    }

    fn serve_xml_once(extra_header: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/child.xml", listener.local_addr().unwrap());
        let extra_header = extra_header.to_owned();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if request.read_line(&mut line).unwrap() == 0 || line == "\r\n" { break; }
            }
            let body = "<?xml version='1.0'?><root>network child</root>";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                body.len(), extra_header, body).unwrap();
            stream.flush().unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let _ = stream.read_to_end(&mut Vec::new());
        });
        (url, worker)
    }

    #[test]
    fn iframe_network_navigation_applies_embedding_policy() {
        for (header, visible) in [("", true), ("X-Frame-Options: DENY\r\n", false)] {
            let (url, worker) = serve_xml_once(header);
            let host = Rc::new(RefCell::new(BrowserDomHost {
                dom: amazing_html::parse(&format!("<body><iframe src='{url}'></iframe></body>")),
                native_forwarded: false,
            }));
            let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
            scheduler.set_location_url(url.replace("/child.xml", "/parent.html"));
            let result = scheduler.run_script(
                "var frame = document.querySelector('iframe'); \
                 frame.contentDocument !== null && \
                 frame.contentDocument.documentElement.textContent === 'network child'")
                .unwrap();
            assert_eq!(result, JsValue::Bool(visible));
            worker.join().unwrap();
        }
    }

    #[test]
    fn iframe_loads_xml_document_before_its_element_load_event() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()));
        std::fs::write(&path, "<?xml version='1.0'?><root>child text</root>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='/child.xml'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        let navigations = Rc::new(RefCell::new(Vec::<ChildNavigation>::new()));
        let observed = navigations.clone();
        scheduler.set_child_navigation_hook(Rc::new(move |navigation| {
            observed.borrow_mut().push(navigation);
        }));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); var order = []; \
             frame.contentDocument.addEventListener('load', () => order.push('child')); \
             frame.addEventListener('load', () => order.push('frame')); \
             frame.contentDocument.contentType + '|' + \
             frame.contentDocument.documentElement.textContent"
        ).unwrap(), JsValue::str("application/xml|child text"));
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("order.join(',')").unwrap(), JsValue::str("child,frame"));
        assert_eq!(navigations.borrow().len(), 1);
        assert_eq!(navigations.borrow()[0].committed_url, "https://example.test/child.xml");
        std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()))).unwrap();
    }

    #[test]
    fn removing_a_freshly_appended_iframe_before_its_load_dispatch_does_not_panic() {
        // WPT regression (html/dom/elements/global-attributes/
        // lang-attribute-document-element-replacement.html): appending a
        // bare `<iframe>` creates its realm synchronously in
        // `navigate_iframes` and queues a `pending_loads` entry;
        // `iframe.remove()` immediately after (still in the same script,
        // before the harness's next `dispatch_pending_iframe_loads` tick)
        // runs `navigate_iframes` again and evicts the now-disconnected
        // frame's entry from `frames.windows` via `retain`. The stale
        // `pending_loads` entry then makes `dispatch_pending_iframe_loads`
        // call `setup_iframe_window`, which finds no realm and falls back to
        // `ensure_realm` -- previously a `RefCell` double-borrow panic
        // ("already borrowed") because the fallback lookup's `match`
        // scrutinee kept its `Ref` alive across the arm that calls
        // `ensure_realm`'s own `borrow_mut()`.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var iframe = document.createElement('iframe'); document.body.append(iframe); \
             var doc = iframe.contentDocument; doc.documentElement.lang = 'de'; \
             var div = document.createElement('div'); doc.body.append(div); \
             iframe.remove(); \
             div.getAttribute !== undefined"
        ).unwrap(), JsValue::Bool(true));
        // The stale queue entry drains without panicking.
        scheduler.dispatch_pending_iframe_loads().unwrap();
    }

    #[test]
    fn iframe_content_document_enforces_same_origin_after_src_change() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()));
        std::fs::write(&path, "<root/>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe></iframe></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentDocument !== null && frame.contentDocument.URL === 'about:blank'"
        ).unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script(
            "frame.src = 'https://other.test/child.xml'; \
             frame.getAttribute('src') === 'https://other.test/child.xml' && \
             frame.contentDocument === null"
        ).unwrap(), JsValue::Bool(true));
        std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()))).unwrap();
    }

    #[test]
    fn iframe_content_window_runs_scripts_and_keeps_identity_across_navigation() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-window-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><script>window.markedByChild = true; document.title = 'child';</script></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='child.html'></iframe></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.run_script("var frame = document.querySelector('iframe');").unwrap();
        assert_eq!(scheduler.run_script("frame.contentWindow !== null").unwrap(), JsValue::Bool(true), "contentWindow non-null");
        assert_eq!(scheduler.run_script("frame.contentWindow.markedByChild").unwrap(), JsValue::Bool(true), "child global set");
        assert_eq!(scheduler.run_script("frame.contentDocument !== null").unwrap(), JsValue::Bool(true), "contentDocument non-null");
        assert_eq!(scheduler.run_script("typeof frame.contentWindow.document").unwrap(), JsValue::str("object"), "child document typeof");
        assert_eq!(scheduler.run_script("frame.contentWindow.document !== null").unwrap(), JsValue::Bool(true), "child document non-null");
        assert_eq!(scheduler.run_script("frame.contentWindow.document === frame.contentDocument").unwrap(), JsValue::Bool(true), "document identity");
        assert_eq!(scheduler.run_script("frame.contentWindow !== window").unwrap(), JsValue::Bool(true), "distinct from parent window");
        assert_eq!(scheduler.run_script("frame.contentWindow.Object !== window.Object").unwrap(), JsValue::Bool(true), "distinct intrinsics");
        assert_eq!(scheduler.run_script(
            "frame.contentWindow === frame.contentWindow"
        ).unwrap(), JsValue::Bool(true));
        scheduler.run_script("globalThis.__savedWin = frame.contentWindow;").unwrap();

        // Re-navigate: the WindowProxy keeps its identity but targets a fresh realm.
        let path2 = std::env::temp_dir().join(format!("amazing-iframe-window2-{}.html", std::process::id()));
        std::fs::write(&path2,
            "<body><script>window.markedByChild = false; window.secondLoad = true;</script></body>").unwrap();
        let resolved2 = path2.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved2.to_string_lossy().into_owned())));
        scheduler.run_script("frame.src = 'other.html';").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "frame.contentWindow === globalThis.__savedWin && \
             frame.contentWindow.secondLoad === true && frame.contentWindow.markedByChild === false"
        ).unwrap(), JsValue::Bool(true));

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&path2).unwrap();
    }

    #[test]
    fn iframe_created_element_has_a_working_local_name_in_an_xml_child_document() {
        // WPT regression (dom/nodes/Document-createElement.html /
        // -createElementNS.html's XML/XHTML-document parametrizations, 488
        // failing subtests flagged by dom-bindings): the actual per-realm
        // wrapping/accessor machinery this exercises (`materialize_accessors`,
        // per-realm `Interface::Element` prototypes from HTML-27) is fine --
        // confirmed by this same scenario passing here. The real bug was in
        // `amazing wpt-harness`'s `FixtureServer` (cmd/wpt_harness.rs), which
        // sent no `Content-Type` header at all, so `content_type()`'s
        // extension fallback (gated on a resolver or `file:` URL, neither
        // true for that loopback HTTP fetch) never ran either: every XML/XHTML
        // iframe failed to navigate at all ("iframe response has no supported
        // media type"), leaving `contentDocument`/`contentWindow.document`
        // pointing at a document that never loaded -- not a cross-realm
        // wrapping bug. Fixed by sending a real extension-derived
        // `Content-Type`. This test keeps the underlying per-realm path
        // covered directly, independent of the fixture server.
        let path = std::env::temp_dir().join(format!("amazing-iframe-localname-{}.xml", std::process::id()));
        std::fs::write(&path, "<foo>Dummy XML document</foo>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='child.xml'></iframe></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "document.querySelector('iframe').contentDocument.documentElement.textContent"
        ).unwrap(), JsValue::str("Dummy XML document"));
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             var el = frame.contentWindow.document.createElement('div'); \
             el.localName === 'div' && el instanceof frame.contentWindow.Element"
        ).unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script(
            "var child = frame.contentDocument; \
             child.defaultView === frame.contentWindow && \
             child.defaultView.document === child && \
             child.defaultView.DOMException === frame.contentWindow.DOMException"
        ).unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script(
            "var caught; try { child.createElementNS('bad', 'xml:x'); } \
             catch (e) { caught = e; } \
             caught.constructor === child.defaultView.DOMException"
        ).unwrap(), JsValue::Bool(true));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn iframe_srcdoc_runs_scripts_same_origin_with_no_network_fetch() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(
                "<body><iframe srcdoc=\"<script>window.markedByChild = true;</script>\"></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        // No resolver and no network reachable: srcdoc must not attempt a fetch.
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentDocument.URL === 'about:srcdoc' && \
             frame.contentWindow.markedByChild === true"
        ).unwrap(), JsValue::Bool(true), "srcdoc uses about:srcdoc and runs inline scripts");
        // Same-origin as the parent (not opaque), so the parent can read into it.
        assert_eq!(scheduler.run_script(
            "frame.contentDocument.body.textContent !== undefined"
        ).unwrap(), JsValue::Bool(true), "srcdoc document readable from the parent realm");
    }

    #[test]
    fn bare_iframe_with_no_src_is_about_blank_and_same_origin_with_its_parent() {
        // A same-origin, unsandboxed `<iframe>` with no `src`/`srcdoc` at all
        // navigates to `about:blank`, which is same-origin with whatever
        // created it -- not opaque -- so `contentWindow` access must not
        // trip JSRT-9's cross-origin WindowProxy gate (`ensure_realm` used to
        // special-case only `about:srcdoc`, leaving `about:blank` to fall
        // through to a mismatched origin from `ParsedUrl::parse`).
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe></iframe></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentWindow.document === frame.contentDocument && \
             frame.contentDocument.URL === 'about:blank'"
        ).unwrap(), JsValue::Bool(true), "bare iframe's contentWindow is reachable and about:blank");
    }

    /// HTML-34: the actual bug this ticket's item 1 fixes -- before, an
    /// iframe's document was fully built (`import_inert_document`, one-shot)
    /// BEFORE any of its own scripts ran, so a script could always see its
    /// own later siblings already in the tree. With `run_interleaved_iframe_
    /// parse`, a script must see only what a real parser would have built
    /// by the time it reaches that `<script>` tag: `<div id=a>` yes,
    /// `<div id=b>` no (not parsed yet).
    #[test]
    fn iframe_interleaved_script_sees_document_so_far_not_the_finished_tree() {
        let path = std::env::temp_dir()
            .join(format!("amazing-iframe-interleave-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><div id=a></div><script>\
             window.sawBBeforeItExisted = (document.getElementById('b') === null);\
             </script><div id=b></div></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='child.html'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); frame.contentWindow.sawBBeforeItExisted"
        ).unwrap(), JsValue::Bool(true), "script ran before its own later sibling was parsed");
        // And by the time the (now finished) parse reached `Eof`, `#b` exists.
        assert_eq!(scheduler.run_script(
            "frame.contentDocument.getElementById('b') !== null"
        ).unwrap(), JsValue::Bool(true), "later sibling exists once parsing has finished");
        std::fs::remove_file(&path).unwrap();
    }

    /// HTML-34 item 2: `localStorage`/`sessionStorage` must be keyed by the
    /// CALLING realm's own origin, not the top-level page's -- before this
    /// ticket's `document_url` scoping fix, `Storage::storage_key()` always
    /// read the single, host-wide `document_url` field, so a cross-origin
    /// iframe's own script would have silently written into the PARENT's
    /// storage area instead of its own. A same-origin case can't tell the
    /// fix apart from the bug (both resolve to the same origin either way),
    /// so this uses a genuinely cross-origin `src` and checks for the
    /// absence of cross-contamination from the parent's own script, which
    /// needs no cross-origin `contentWindow` property access at all (that
    /// stays gated, correctly, by JSRT-9 -- unrelated to this fix).
    #[test]
    fn iframe_localstorage_is_keyed_by_the_frames_own_origin_not_the_parents() {
        let path = std::env::temp_dir()
            .join(format!("amazing-iframe-storage-origin-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><script>localStorage.setItem('cross-origin-leak', 'leaked');</script></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='https://other.test/child.html'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script("localStorage.getItem('cross-origin-leak')").unwrap(),
            JsValue::Null, "baseline: nothing there yet");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("localStorage.getItem('cross-origin-leak')").unwrap(),
            JsValue::Null, "a cross-origin child's write must not land in the parent's own storage area");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn iframe_sandbox_without_allow_same_origin_is_cross_origin_from_parent() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><script>window.markedByChild = true;</script></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe sandbox='allow-scripts' src='child.html'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        // Without `allow-same-origin` the child gets a fresh opaque origin
        // every navigation, so it is cross-origin from the parent even
        // though both come from the same URL origin: `contentDocument` is
        // null, and an arbitrary property read through `contentWindow`
        // (not on the small cross-origin allowlist) throws.
        assert_eq!(scheduler.run_script(
            "document.querySelector('iframe').contentDocument"
        ).unwrap(), JsValue::Null, "opaque-origin child has no contentDocument for the parent");
        assert!(scheduler.run_script(
            "document.querySelector('iframe').contentWindow.markedByChild"
        ).is_err(), "arbitrary property read across an opaque origin throws");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn iframe_sandbox_allow_same_origin_without_allow_scripts_blocks_scripts() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-so-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><script>window.markedByChild = true;</script></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe sandbox='allow-same-origin' src='child.html'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        // `allow-same-origin` alone keeps the child visible to the parent
        // but does not itself permit scripting; only `allow-scripts` does.
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentDocument !== null && frame.contentWindow.markedByChild"
        ).unwrap(), JsValue::Undefined, "same-origin visible, but no script ran without allow-scripts");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn iframe_sandbox_allow_scripts_and_allow_same_origin_runs_scripts() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-sandboxed-scripts-{}.html", std::process::id()));
        std::fs::write(&path,
            "<body><script>window.markedByChild = true;</script></body>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(
                "<body><iframe sandbox='allow-scripts allow-same-origin' src='child.html'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let resolved = path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(resolved.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); frame.contentWindow.markedByChild"
        ).unwrap(), JsValue::Bool(true), "allow-scripts runs the child's scripts");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn form_target_navigates_named_iframe_instead_of_top_level() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(
                "<body><iframe name='result'></iframe>\
                 <form action='https://example.test/submitted' target='result'></form></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "document.forms[0].submit(); \
             document.querySelector('iframe').getAttribute('src')"
        ).unwrap(), JsValue::str("https://example.test/submitted?"),
            "target matching a frame name navigates that frame, not top-level");
        assert_eq!(scheduler.run_script("location.href")
            .unwrap(), JsValue::str("https://example.test/main.html"), "top-level location is untouched");
    }

    #[test]
    fn form_target_blank_still_navigates_top_level() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(
                "<body><iframe name='_blank'></iframe>\
                 <form action='https://example.test/submitted' target='_blank'></form></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("document.forms[0].submit();").unwrap();
        // `_blank` is a special keyword, never a frame name match, even when
        // (invalidly) reused as one.
        assert_eq!(scheduler.run_script("document.querySelector('iframe').getAttribute('src')")
            .unwrap(), JsValue::Null, "the '_blank' keyword never matches a frame name");
        assert_eq!(scheduler.run_script("location.href")
            .unwrap(), JsValue::str("https://example.test/submitted?"), "falls through to top-level navigation");
    }

    #[test]
    fn document_write_runs_its_inserted_inline_script_synchronously_and_in_order() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        // Matches WPT's `execution-timing` idiom: a script that calls
        // `document.write` with an embedded `<script>` observes that script
        // run before `write()` returns, not merely at some later point.
        assert_eq!(scheduler.run_script(
            "var order = []; order.push('before'); \
             document.write('<script>order.push(\"written\")<\\/script>'); \
             order.push('after'); order.join(',')"
        ).unwrap(), JsValue::str("before,written,after"));
        // The written markup is really inserted into the document, not just
        // executed and discarded.
        assert_eq!(scheduler.run_script("document.body.children.length")
            .unwrap(), JsValue::num(1.0));
    }

    #[test]
    fn document_writeln_appends_a_newline_and_write_does_not() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "document.write('a'); document.writeln('b'); document.write('c'); \
             document.body.textContent"
        ).unwrap(), JsValue::str("ab\nc"));
    }

    #[test]
    fn document_open_and_close_do_not_throw_and_open_returns_the_document() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script("document.open() === document")
            .unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script("document.close()")
            .unwrap(), JsValue::Undefined);
    }

    #[test]
    fn document_write_from_a_top_level_script_inserts_at_the_parser_insertion_point() {
        // HTML-29: a classic top-level script's `document.write` inserts
        // right after that script's own position (the real parser's
        // insertion point while it is the one running), not always at the
        // end of the body the way HTML-28's implementation did -- so
        // content written by a script between two elements ends up between
        // them, not after both. Exercised through `prepare_page_scripts`
        // (cmd/js.rs), the same top-level script-phase wiring a real page
        // load uses (`begin_document_parsing`/`narrow_write_insertion_point`).
        // HTML-32: `prepare_page_scripts` now parses `html` itself through
        // the interleaved parser, so `host` starts with an empty `Dom` and
        // the source that used to be pre-parsed via `amazing_html::parse`
        // is passed straight through as its new `html` argument instead.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(),
            native_forwarded: false,
        }));
        let mut scheduler = crate::cmd::js::prepare_page_scripts(
            &host,
            "https://example.test/main.html",
            "<body><div id=a></div><script>document.write('<div id=written></div>')</script><div id=b></div></body>",
            false,
        ).unwrap().unwrap();
        // The writing `<script>` element itself stays in the tree (as an
        // empty-id element between "a" and "written") -- only the *written*
        // content lands at its insertion point, right after it and before
        // "b", not at the end of the body.
        assert_eq!(scheduler.run_script(
            "Array.from(document.body.children).map(e => e.id).join(',')"
        ).unwrap(), JsValue::str("a,,written,b"));
    }

    #[test]
    fn document_write_after_load_implicitly_reopens_the_document() {
        // HTML-29: once a document's top-level script phase has finished and
        // its load event fired, `end_document_parsing` has cleared its
        // marker, so a later `document.write` (the way a `setTimeout`
        // callback firing after load would call it) implicitly performs
        // `document.open()`'s full-document replace instead of appending
        // next to whatever content the page already had.
        // HTML-32: same reshaping as the test above -- `host` starts empty
        // and the source is passed as `prepare_page_scripts`'s `html` arg.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(),
            native_forwarded: false,
        }));
        let mut scheduler = crate::cmd::js::prepare_page_scripts(
            &host,
            "https://example.test/main.html",
            "<body><div id=original></div></body>",
            true,
        ).unwrap().unwrap();
        scheduler.run_script("document.write('hello')").unwrap();
        assert_eq!(scheduler.run_script("document.body.textContent").unwrap(), JsValue::str("hello"));
        assert_eq!(scheduler.run_script("document.getElementById('original')").unwrap(), JsValue::Null);
    }

    /// HTML-31: unlike `prepare_page_scripts`'s tests above (which start from
    /// an already-fully-parsed DOM, since that function's own callers parse
    /// upfront), this drives the real, script-interleaved parser
    /// (`crate::cmd::js::run_interleaved_parse`, wired into
    /// `cmd/wpt_harness.rs::execute_page` for the measured WPT corpora) --
    /// so a script's own later siblings genuinely do not exist yet when it
    /// runs, matching a real browser and fixing the architectural gap
    /// `dom/nodes/MutationObserver-document.html` exposed.
    #[test]
    fn interleaved_parse_runs_a_script_against_the_dom_built_so_far_not_the_finished_document() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        let mut seen_before_later_sibling_exists = None;
        crate::cmd::js::run_interleaved_parse(
            &host, &mut scheduler,
            "<body><div id=a></div><script>var probe = 1;</script><div id=b></div></body>",
            |scheduler, script| {
                seen_before_later_sibling_exists = Some(
                    scheduler.run_script("document.getElementById('b') === null").unwrap()
                );
                scheduler.run_script(&script.body).map_err(|e| e.describe())?;
                Ok(())
            },
        ).unwrap();
        assert_eq!(seen_before_later_sibling_exists, Some(JsValue::Bool(true)));
        // The full document is there once the interleaved parse returns.
        assert_eq!(scheduler.run_script("document.getElementById('b') !== null").unwrap(), JsValue::Bool(true));
    }

    /// The real per-spec behavior HTML-29 approximated with a DOM splice:
    /// `document.write` while paused at a blocking script inserts into the
    /// input stream at the real tokenizer position, so the written markup
    /// (and any further source after the script) both get tokenized and
    /// built the normal way, in the right order.
    #[test]
    fn interleaved_parse_document_write_inserts_at_the_real_tokenizer_position() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        crate::cmd::js::run_interleaved_parse(
            &host, &mut scheduler,
            "<body><div id=a></div><script>document.write('<div id=written></div>')</script><div id=b></div></body>",
            |scheduler, script| { scheduler.run_script(&script.body).map_err(|e| e.describe())?; Ok(()) },
        ).unwrap();
        assert_eq!(
            scheduler.run_script("Array.from(document.body.children).map(e => e.id).join(',')").unwrap(),
            JsValue::str("a,,written,b"),
        );
    }

    /// `defer` only exempts a script with a `src` from blocking (per spec it
    /// is ignored on an inline script, which still blocks like any other);
    /// `async` exempts either. Neither ever pauses construction; both run
    /// after the whole document reaches `Eof`, in document order -- unlike
    /// the blocking script above, which runs synchronously, mid-parse.
    #[test]
    fn interleaved_parse_defer_and_async_scripts_run_after_eof_in_document_order() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        let mut ran = Vec::new();
        // Only the parser-blocking script pauses construction and runs
        // through this closure; `defer`/`async` scripts are returned instead
        // (see below), never invoking it -- exactly the assertion this test
        // makes.
        let deferred = crate::cmd::js::run_interleaved_parse(
            &host, &mut scheduler,
            "<body><script src=deferred.js defer></script>\
             <script src=async.js async></script>\
             <script>1</script></body>",
            |_scheduler, script| {
                ran.push(script.src.clone().unwrap_or_else(|| "blocking".to_owned()));
                Ok(())
            },
        ).unwrap();
        for script in &deferred {
            ran.push(script.src.clone().unwrap_or_else(|| "blocking".to_owned()));
        }
        assert_eq!(ran, vec!["blocking", "deferred.js", "async.js"]);
    }

    /// An inline script's own `defer` attribute is ignored (no `src`): it
    /// still pauses construction like any other classic script, unlike the
    /// external `defer`/`async` cases above.
    #[test]
    fn interleaved_parse_inline_defer_without_src_still_blocks() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_dom::Dom::new(), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        let mut ran = Vec::new();
        crate::cmd::js::run_interleaved_parse(
            &host, &mut scheduler,
            "<body><script defer>1</script></body>",
            |_scheduler, script| { ran.push(script.body.clone()); Ok(()) },
        ).unwrap();
        assert_eq!(ran, vec!["1"], "inline defer (no src) must block like an ordinary script");
    }

    // HTML-35: "prepare the script element" for non-parser-inserted scripts.

    #[test]
    fn dynamic_script_runs_once_on_connection_and_never_again_on_reinsertion() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = 0; \
             var el = document.createElement('script'); \
             el.textContent = 'window.ran += 1;'; \
             document.body.appendChild(el); \
             document.body.removeChild(el); \
             document.body.appendChild(el); \
             window.ran"
        ).unwrap(), JsValue::num(1.0), "already-started must survive removal and reinsertion");
    }

    #[test]
    fn dynamic_script_inserted_via_inner_html_never_executes_even_if_moved_out_afterward() {
        // WHATWG HTML: the fragment-parsing algorithm behind the `innerHTML`
        // setter marks every script element it produces already-started,
        // unconditionally -- unlike a `createElement`-built script, it never
        // gets to run, however it is later moved.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = 0; \
             document.body.innerHTML = '<script>window.ran = 1;<\\/script>'; \
             window.ran"
        ).unwrap(), JsValue::num(0.0), "innerHTML-produced scripts must not execute");
        // Moving it into a fresh connection point afterward still must not
        // run it -- the flag, not the location, is what blocks it.
        assert_eq!(scheduler.run_script(
            "var s = document.body.querySelector('script'); \
             document.body.removeChild(s); \
             document.body.appendChild(s); \
             window.ran"
        ).unwrap(), JsValue::num(0.0), "already-started must stick even once moved out and back in");
    }

    #[test]
    fn template_inner_html_produced_scripts_never_execute_even_after_moving_content_out() {
        // Follow-up from HTML-35: `template.innerHTML`'s fragment-parsing
        // result lands in the template's own `.content`, a detached
        // fragment `connected_script_nodes`'s before/after diff never
        // walks -- must still be marked already-started, or moving the
        // content into a real, connected document later would wrongly run
        // it.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><template id=t></template></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = 0; \
             document.getElementById('t').innerHTML = '<script>window.ran = 1;<\\/script>'; \
             document.body.appendChild(document.getElementById('t').content); \
             window.ran"
        ).unwrap(), JsValue::num(0.0), "template content scripts must not execute once moved into a live document");
    }

    #[test]
    fn dom_parser_parse_from_string_produced_scripts_never_execute_even_after_adoption() {
        // Follow-up from HTML-35: a `DOMParser`-produced document has no
        // browsing context (already blocked from running while it stays
        // orphaned by `has_active_document_root`), but a script from it
        // can still be adopted into a real, connected document -- must be
        // marked already-started at creation time so it never runs even
        // then.
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = 0; \
             var doc = new DOMParser().parseFromString('<script>window.ran = 1;<\\/script>', 'text/html'); \
             var s = doc.querySelector('script'); \
             document.adoptNode(s); \
             document.body.appendChild(s); \
             window.ran"
        ).unwrap(), JsValue::num(0.0), "DOMParser-produced scripts must not execute even once adopted and connected");
    }

    #[test]
    fn dynamic_script_type_and_nomodule_filtering_matches_is_javascript_mime_type() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = []; \
             function make(type, nomodule) { \
               var s = document.createElement('script'); \
               if (type !== undefined) s.type = type; \
               if (nomodule) s.setAttribute('nomodule', ''); \
               s.textContent = 'window.ran.push(\"' + (type || 'default') + '\");'; \
               document.body.appendChild(s); \
             } \
             make(undefined, false); \
             make('application/javascript', false); \
             make('module', false); \
             make('text/plain', false); \
             make('text/javascript', true); \
             window.ran.join(',')"
        ).unwrap(), JsValue::str("default,application/javascript,module"),
            "classic and module scripts run; data blocks and classic nomodule fallbacks are skipped");
    }

    #[test]
    fn dynamic_script_with_external_src_fetches_fires_load_and_runs_in_order() {
        let script_path = std::env::temp_dir()
            .join(format!("amazing-html35-ext-{}.js", std::process::id()));
        std::fs::write(&script_path, "window.extRan = true;").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let path = script_path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var events = []; \
             var s = document.createElement('script'); \
             s.src = 'https://example.test/ext.js'; \
             s.addEventListener('load', () => events.push('load')); \
             s.addEventListener('error', () => events.push('error')); \
             document.body.appendChild(s); \
             events.join(',') + '|' + window.extRan"
        ).unwrap(), JsValue::str("load|true"));
        std::fs::remove_file(&script_path).unwrap();
    }

    #[test]
    fn dynamic_script_with_failing_external_src_fires_error_and_never_runs() {
        let missing_path = std::env::temp_dir()
            .join(format!("amazing-html35-missing-{}.js", std::process::id()));
        let _ = std::fs::remove_file(&missing_path); // ensure it does not exist
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        let path = missing_path.clone();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var events = []; window.extRan2 = false; \
             var s = document.createElement('script'); \
             s.src = 'https://example.test/missing.js'; \
             s.addEventListener('load', () => events.push('load')); \
             s.addEventListener('error', () => events.push('error')); \
             document.body.appendChild(s); \
             events.join(',') + '|' + window.extRan2"
        ).unwrap(), JsValue::str("error|false"));
    }

    #[test]
    fn dynamic_script_mutating_text_or_src_after_it_already_ran_does_not_rerun() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "window.ran = 0; \
             var s = document.createElement('script'); \
             s.textContent = 'window.ran += 1;'; \
             document.body.appendChild(s); \
             s.textContent = 'window.ran += 100;'; \
             s.src = 'https://example.test/should-not-fetch.js'; \
             document.body.appendChild(document.createElement('div')); \
             window.ran"
        ).unwrap(), JsValue::num(1.0),
            "textContent/src set after a script already ran must not re-trigger it");
    }
    fn synchronous_write_fixture() -> &'static str {
        r#"<!doctype html><body><script>
            window.syncPass = true;
            document.write('<div id="written">&am');
            syncPass = syncPass && !!document.getElementById('written')
                && document.getElementById('written').textContent === '';
            document.write('p;</div>');
            syncPass = syncPass && document.getElementById('written').textContent === '&';
            document.write('<scr' + 'ipt>document.write("<i id=inner>inner</i>");'
                + 'window.nestedVisible=!!document.getElementById("inner");</scr' + 'ipt><b id=outer>outer</b>');
            syncPass = syncPass && nestedVisible && !!document.getElementById('outer')
                && document.getElementById('tail') === null;
        </script><p id=tail>tail</p>"#
    }

    #[test]
    fn active_top_level_write_is_visible_before_return_and_preserves_tail() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        crate::cmd::js::run_interleaved_parse(&host, &mut scheduler, synchronous_write_fixture(),
            |scheduler, script| scheduler.run_script(&script.body).map(|_| ()).map_err(|e| format!("{e:?}"))).unwrap();
        assert_eq!(scheduler.run_script("syncPass && !!document.getElementById('tail')").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn active_iframe_write_grafts_synchronously_and_nested_writes_keep_identity() {
        let mut dom = amazing_html::parse("<!doctype html><iframe></iframe>");
        let frame = dom.children(dom.document_body(NodeIdx(0)).unwrap().unwrap()).next().unwrap();
        dom.set_attribute(frame, "srcdoc", synchronous_write_fixture()).unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost { dom, native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/main.html");
        scheduler.run_script("void document.querySelector('iframe')").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("document.querySelector('iframe').contentWindow.syncPass && !!document.querySelector('iframe').contentDocument.getElementById('tail')").unwrap(), JsValue::Bool(true));
    }
}
