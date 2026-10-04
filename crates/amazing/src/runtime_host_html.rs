//! Child HTML navigables owned by the driver. Each connected iframe gets its
//! own same-agent realm and stable WindowProxy through JSRT-9
//! (binding_create_realm/binding_create_window_proxy/binding_with_realm);
//! DOM interface prototypes stay arena-shared, so a child realm's `document`
//! wraps the same Document/Element prototypes as the top realm rather than a
//! fully independent intrinsic set.
use super::*;
#[path = "runtime_host_html/document_write.rs"]
mod document_write;
#[path = "runtime_host_html/document_dsd.rs"]
mod document_dsd;
#[path = "runtime_host_html/resource_response.rs"]
mod resource_response;
pub(crate) use resource_response::{HtmlFrameResourceKind, HtmlFrameResourceLoader, HtmlFrameResourceRequest};
use crate::dom_host::BrowserDomHost;
use amazing_dom::document_surface::ReadyState;
use amazing_net::url::ParsedUrl;
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
    source: Option<NavigationSource>,
}

#[derive(Clone)]
struct NavigationSource {
    document: NodeIdx,
    generation: u64,
    base_url: String,
    document_url: String,
    origin: String,
    fetch_origin: amazing_fetch::Origin,
    /// Policy at initiation, retained even when the source Document retires.
    policy: HtmlDocumentContext,
    client_context: Option<amazing_fetch::service::ClientContext>,
    referrer_policy: amazing_fetch::ReferrerPolicy,
}

#[derive(Clone)]
struct HtmlNavigation {
    document: NodeIdx,
    generation: u64,
    url: String,
    replace: bool,
    reload: bool,
    source: NavigationSource,
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
    cross_origin_descriptors: HashMap<(RealmHandle, RealmHandle, String), Property>,
    /// Captured creation/navigation base, retained for old documents too.
    about_base_urls: HashMap<NodeIdx, (u64, String)>,
    /// Inert parser Documents inherit a typed origin without acquiring a
    /// browsing context. Opaque identities are preserved, never URL-derived.
    inert_document_origins: HashMap<NodeIdx, (u64, amazing_fetch::Origin)>,
    /// Queued Location changes carry their actual source, even after it moves.
    pending_navigations: HashMap<RealmHandle, HtmlNavigation>,
    parser_imports: HashMap<NodeIdx, IncrementalImportRef>,
    document_write_sessions: HashMap<NodeIdx, document_write::DocumentWriteSession>,
    /// `(frame, document, fire_load_events)`. `fire_load_events` is false
    /// only for an iframe with neither `src` nor `srcdoc`: its initial
    /// `about:blank` is created for `contentDocument`/`contentWindow` to see,
    /// but per spec no `load` event is queued for it (WPT's
    /// `iframe-nosrc.html` treats any iframe-level `load` before the page's
    /// own as a failure).
    pending_loads: VecDeque<(NodeIdx, Option<NodeIdx>, bool)>,
    resolver: Option<FrameResourceResolver>,
    resource_loader: Option<HtmlFrameResourceLoader>,
    document_referrer_policies: HashMap<NodeIdx, (u64, amazing_fetch::ReferrerPolicy)>,
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
    document_generation: u64,
    frame: Option<NodeIdx>,
    global: JsValue,
    window: JsValue,
    parent: JsValue,
    top: JsValue,
    origin: String,
    fetch_origin: amazing_fetch::Origin,
    document_url: String,
    installed: bool,
    indices: usize,
}

#[derive(Clone, Copy)]
pub(super) enum HtmlWindowProperty {
    Frames, Length, Parent, Top,
    BoundFrames(RealmHandle), BoundLength(RealmHandle), BoundParent(RealmHandle), BoundTop(RealmHandle),
}

impl HtmlWindowProperty {
    fn name(self) -> &'static str {
        match self {
            Self::Frames | Self::BoundFrames(_) => "frames", Self::Length | Self::BoundLength(_) => "length",
            Self::Parent | Self::BoundParent(_) => "parent", Self::Top | Self::BoundTop(_) => "top",
        }
    }
}

/// Immutable policy identity of an actual live browsing-context Document.
#[derive(Clone, Debug)]
pub(crate) struct HtmlDocumentContext {
    pub(crate) document: NodeIdx,
    pub(crate) document_generation: u64,
    pub(crate) document_url: String,
    pub(crate) fallback_base_url: Option<String>,
    pub(crate) origin: amazing_fetch::Origin,
    pub(crate) top_level_origin: amazing_fetch::Origin,
    /// Immediate parent first, top-level last; root has no ancestors.
    pub(crate) ancestor_origins: Vec<amazing_fetch::Origin>,
}

impl FrameRuntime {
    /// The newly allocated context is genuine before its frame entry is
    /// committed. Read only its exact native Document and current realm.
    pub(super) fn document_creation_origin(&self, dom: &Dom, document: NodeIdx,
        realm: RealmHandle) -> Option<amazing_fetch::Origin> {
        let context = self.contexts.get(&realm)?;
        if context.document != document || dom.node_generation(document) != Some(context.document_generation)
            || !matches!(&dom.get(document).kind, amazing_dom::NodeKind::Document) { return None; }
        if context.frame.is_some_and(|frame| self.windows.get(&frame).is_none_or(|(target, _)| *target != realm)) {
            return None;
        }
        Some(context.fetch_origin.clone())
    }

    pub(super) fn document_origin(&self, dom: &Dom, document: NodeIdx) -> Option<amazing_fetch::Origin> {
        let generation = dom.node_generation(document)?;
        if !matches!(&dom.get(document).kind, amazing_dom::NodeKind::Document) { return None; }
        if let Some(context) = self.contexts.iter().find_map(|(realm, context)|
            (context.document == document && context.document_generation == generation
                && self.live_context(context, *realm)).then_some(context)) {
            return Some(context.fetch_origin.clone());
        }
        self.inert_document_origins.get(&document)
            .filter(|(saved, _)| *saved == generation).map(|(_, origin)| origin.clone())
    }

    pub(super) fn set_inert_document_origin(&mut self, dom: &Dom, document: NodeIdx,
        origin: amazing_fetch::Origin) -> Result<(), amazing_dom::DomError> {
        let generation = dom.node_generation(document).ok_or(amazing_dom::DomError::InvalidNode)?;
        if !matches!(&dom.get(document).kind, amazing_dom::NodeKind::Document) {
            return Err(amazing_dom::DomError::TypeError);
        }
        if let Some((saved, old)) = self.inert_document_origins.get(&document) {
            if *saved == generation && *old != origin { return Err(amazing_dom::DomError::InvalidStateError); }
        }
        self.inert_document_origins.insert(document, (generation, origin));
        Ok(())
    }

    pub(super) fn has_document(&self, document: NodeIdx) -> bool {
        document == NodeIdx(0) || self.documents.values().any(|frame| frame.document == Some(document))
    }

    pub(super) fn frame_for_document(&self, document: NodeIdx) -> Option<NodeIdx> {
        self.documents.iter().find_map(|(node, frame)| (frame.document == Some(document)).then_some(*node))
    }

    /// Actual committed document URL, without entering JS or consulting script scope.
    pub(super) fn document_url(&self, document: NodeIdx) -> Option<String> {
        self.contexts.iter().find_map(|(realm, context)|
            (context.document == document && self.live_context(context, *realm))
                .then(|| context.document_url.clone()))
    }

    /// Policy snapshot without realm entry, script callbacks or mutable scope.
    pub(super) fn document_context(&self, dom: &Dom, document: NodeIdx) -> Option<HtmlDocumentContext> {
        let (realm, context) = self.contexts.iter().find(|(realm, context)|
            context.document == document && self.live_context(context, **realm)
                && dom.node_generation(document) == Some(context.document_generation))?;
        let mut seen = HashSet::from([*realm]);
        let mut current = context;
        let mut ancestors = Vec::new();
        while current.parent != current.window {
            let (parent_realm, parent) = self.contexts.iter().find(|(realm, candidate)|
                candidate.window == current.parent && self.live_context(candidate, **realm)
                    && dom.node_generation(candidate.document) == Some(candidate.document_generation))?;
            if !seen.insert(*parent_realm) { return None; }
            ancestors.push(parent.fetch_origin.clone());
            current = parent;
        }
        if current.window != context.top { return None; }
        let fallback_base_url = self.about_base_urls.get(&document).and_then(|(generation, base)| {
            let applicable = ParsedUrl::parse(&context.document_url).is_ok_and(|url|
                matches_about_url(&url, "blank") || (matches_about_url(&url, "srcdoc") && url.query().is_none()));
            (*generation == context.document_generation && applicable).then(|| base.clone())
        });
        Some(HtmlDocumentContext { document, document_generation: context.document_generation,
            document_url: context.document_url.clone(), fallback_base_url,
            origin: context.fetch_origin.clone(), top_level_origin: current.fetch_origin.clone(), ancestor_origins: ancestors })
    }

    /// Actual live Document realm for native callback/handler installation.
    /// This performs no realm entry and never substitutes a caller realm.
    pub(super) fn document_realm(&self, dom: &Dom, document: NodeIdx) -> Option<RealmHandle> {
        if !matches!(dom.node_generation(document).map(|_| &dom.get(document).kind), Some(NodeKind::Document)) {
            return None;
        }
        self.document_context(dom, document)?;
        self.contexts.iter().find(|(realm, context)| context.document == document
            && self.live_context(context, **realm)
            && dom.node_generation(document) == Some(context.document_generation))
            .map(|(realm, _)| *realm)
    }

    /// Resolve the requesting realm's actual live Document policy without JS.
    pub(super) fn document_context_for_realm(&self, dom: &Dom, realm: RealmHandle) -> Option<HtmlDocumentContext> {
        let context = self.contexts.get(&realm)?;
        if !self.live_context(context, realm)
            || dom.node_generation(context.document) != Some(context.document_generation) { return None; }
        self.document_context(dom, context.document)
    }

    /// Genuine Window identity without entering a realm or invoking script.
    pub(super) fn document_for_window(&self, window: &JsValue) -> Option<NodeIdx> {
        let global = self.contexts.iter().find(|(_, context)| *window == context.global);
        let live = self.contexts.iter().find(|(realm, context)| *window == context.window
            && self.live_context(context, **realm));
        let retired = self.contexts.iter().find(|(_, context)| *window == context.window);
        global.or(live).or(retired).map(|(_, context)| context.document)
    }

    fn live_context(&self, context: &WindowContext, realm: RealmHandle) -> bool {
        context.frame.is_none() || context.frame.is_some_and(|frame|
            self.windows.get(&frame).is_some_and(|(current, _)| *current == realm)
                && self.documents.get(&frame).is_some_and(|entry| entry.document == Some(context.document)))
    }
}

fn context_origin(js: &mut Interpreter, context: &WindowContext, realm: RealmHandle) -> Result<String, Throw> {
    if context.frame.is_some() || !context.origin.is_empty() { return Ok(context.origin.clone()); }
    js.binding_with_realm(realm, |js| Ok(ParsedUrl::parse(js.binding_location_href())
        .map(|url| url.origin().serialize()).unwrap_or_else(|_| "null".into())))
}

fn matches_about_url(url: &ParsedUrl, name: &str) -> bool {
    url.scheme() == "about" && url.pathname() == name && url.host().is_none()
        && url.username().is_empty() && url.password().is_empty()
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

fn content_type(response: &amazing_net::Response, logical_url: &str,
    local_file: bool) -> Option<String> {
    let mime = resource_response::response_mime_type(response)
        .map(|mime| mime.essence()).unwrap_or_default();
    let unknown = matches!(mime.as_str(), "" | "unknown/unknown" | "application/unknown" | "*/*");
    let no_sniff = resource_response::response_nosniff(response);
    if unknown && !no_sniff {
        if has_html_signature(&response.body) { return Some("text/html".into()); }
        if has_xml_signature(&response.body) { return Some("text/xml".into()); }
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

/// MIME Sniffing §7.1 XML row is case-sensitive and has no tag terminator.
fn has_xml_signature(body: &[u8]) -> bool {
    let header = &body[..body.len().min(1445)];
    let start = header.iter().position(|byte| !matches!(byte, 9 | 10 | 12 | 13 | 32))
        .unwrap_or(header.len());
    header[start..].starts_with(b"<?xml")
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
        let decoded = amazing_html::xml_encoding::decode(body, charset)
            .map_err(|error| format!("XML decode failed: {error}"))?;
        let parsed = amazing_html::xml::parse_xml_decoded(&decoded.text)
            .map_err(|error| format!("XML parse failed: {error:?}"))?;
        return Ok((parsed, amazing_html::encoding::canonical_name(decoded.encoding)));
    }
    Err(format!("unsupported iframe media type: {mime}"))
}

fn fetch_child_document(logical_url: &str, context: &NavigableContext,
    resolver: Option<&FrameResourceResolver>) -> Result<amazing_net::Response, String> {
    let target = HttpUrl::parse(logical_url).map_err(|error| format!("iframe URL: {error:?}"))?;
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
    pub(super) fn html_set_document_origin(&self, url: &str) {
        let origin = ParsedUrl::parse(url).map(|url| url.origin().serialize()).unwrap_or_else(|_| "null".into());
        let fetch_origin = HttpUrl::parse(url).map(|url| url.origin().clone()).unwrap_or_else(|_| amazing_fetch::Origin::opaque());
        let generation = self.state.borrow().host.borrow().dom.node_generation(NodeIdx(0));
        for context in self.state.borrow_mut().frames.contexts.values_mut().filter(|context| context.frame.is_none()) {
            if let Some(generation) = generation { context.document_generation = generation; }
            context.origin = origin.clone();
            context.fetch_origin = fetch_origin.clone();
            context.document_url = url.to_owned();
        }
    }

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
        child_url: &str, sandbox: SandboxFlags, initiator: Option<&NavigationSource>) -> Result<RealmHandle, Throw> {
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
        } else if ParsedUrl::parse(child_url).is_ok_and(|url| matches_about_url(&url, "blank")
            || (matches_about_url(&url, "srcdoc") && url.query().is_none())) {
            initiator.map(|source| source.origin.clone()).unwrap_or(parent_origin)
        } else {
            ParsedUrl::parse(child_url).map(|url| url.origin().serialize())
                .unwrap_or_else(|_| "null".into())
        };
        let inherits = ParsedUrl::parse(child_url).is_ok_and(|url| matches_about_url(&url, "blank")
            || (matches_about_url(&url, "srcdoc") && url.query().is_none()));
        let fetch_origin = if sandbox.sandboxed && !sandbox.allow_same_origin {
            amazing_fetch::Origin::opaque()
        } else if inherits {
            initiator.map(|source| source.fetch_origin.clone()).or_else(|| parent_context.as_ref()
                .map(|(_, context)| context.fetch_origin.clone())).unwrap_or_else(amazing_fetch::Origin::opaque)
        } else {
            HttpUrl::parse(child_url).map(|url| url.origin().clone()).unwrap_or_else(|_| amazing_fetch::Origin::opaque())
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
        let document_generation = self.state.borrow().host.borrow().dom.node_generation(document)
            .ok_or_else(|| Throw::type_error("Missing frame document"))?;
        self.state.borrow_mut().frames.contexts.insert(realm, WindowContext {
            document, document_generation, frame: Some(frame), global, window: proxy, parent, top, origin, fetch_origin, document_url: child_url.to_owned(),
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
            adapter.dom_register_document_realm(document, realm)?;
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
            // Real Location uses this realm's bound host dispatch. The legacy
            // JS fallback bypasses navigation source capture entirely.
            let arena = adapter.state.borrow().arena;
            let location = adapter.wrap_platform_object(PlatformKey { arena, slot: 0, generation: 1 }, InterfaceBrand::LOCATION)?;
            adapter.js.binding_replace_location_object(location)?;
            let global = adapter.js.binding_global_object();
            adapter.js.binding_install_put_forwards(&global, "location", "href")?;
            if let Some(prototype) = adapter.prototype(Interface::Document) {
                adapter.js.binding_install_put_forwards(&prototype, "location", "href")?;
            }
            // Wrapped here, inside the child realm, so `document` gets the
            // child's own freshly-installed Document.prototype (its host
            // object cache is per-realm) rather than the caller's; the
            // `contentDocument` getter enters this same realm to fetch the
            // identical cached wrapper, so the two stay `===` no matter
            // which is read first.
            let wrapped = adapter.wrap_node(document)?;
            let fresh = adapter.js.binding_create_global_environment_overlay();
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

    /// HTML encoding sniffing step 6 uses the actual container Document,
    /// independently of a navigation's initiating Document or captured policy.
    fn frame_encoding_fallback(&self, frame: NodeIdx, final_url: &str, sandbox: SandboxFlags)
        -> amazing_encoding::Encoding {
        use amazing_encoding::Encoding;
        if sandbox.sandboxed && !sandbox.allow_same_origin { return Encoding::Windows1252; }
        // The final remote navigation URL determines its nonsandboxed origin,
        // exactly as ensure_realm does when committing the actual Document.
        let Ok(url) = HttpUrl::parse(final_url) else { return Encoding::Windows1252; };
        let state = self.state.borrow(); let host = state.host.borrow();
        let inherited = host.dom.owner_document(frame).and_then(|parent| {
            let context = state.frames.document_context(&host.dom, parent)?;
            if &context.origin != url.origin() { return None; }
            let label = host.dom.document_metadata(parent).ok().flatten()
                .map(|metadata| metadata.character_set.as_str())
                .or_else(|| (parent == NodeIdx(0)).then_some(state.document_character_set.as_deref()).flatten())?;
            amazing_encoding::encoding_for_label(label)
        }).filter(|encoding| !matches!(encoding, Encoding::Utf16Be | Encoding::Utf16Le));
        inherited.unwrap_or(Encoding::Windows1252)
    }

    pub(crate) fn navigate_iframes(&self, js: &mut Interpreter) {
        let (host, base, resolver) = {
            let state = self.state.borrow();
            (state.host.clone(), state.document_url.clone(), state.frames.resolver.clone())
        };
        if base.is_none() { return; }
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
            let owner = host.borrow().dom.owner_document(node).unwrap_or(NodeIdx(0));
            // A parent's navigation can retire a document after this sweep
            // collected its descendants. Never retain or recreate their contexts.
            if !self.state.borrow().frames.has_document(owner) {
                let mut state = self.state.borrow_mut();
                state.frames.documents.remove(&node);
                state.frames.windows.remove(&node);
                continue;
            }
            let owner_context = self.state.borrow().frames.contexts.iter()
                .find(|(realm, context)| context.document == owner && self.state.borrow().frames.live_context(context, **realm))
                .map(|(realm, context)| (*realm, context.clone()));
            let Some((owner_realm, owner_context)) = owner_context else { continue; };
            let old_realm = self.state.borrow().frames.windows.get(&node).map(|(realm, _)| *realm);
            let pending = old_realm.and_then(|realm| {
                let navigation = self.state.borrow().frames.pending_navigations.get(&realm).cloned()?;
                let valid = {
                    let state = self.state.borrow(); let host = state.host.borrow();
                    host.dom.node_generation(navigation.document) == Some(navigation.generation)
                        && state.frames.contexts.get(&realm).is_some_and(|context| context.document == navigation.document)
                };
                if !valid { self.state.borrow_mut().frames.pending_navigations.remove(&realm); return None; }
                // Only this actual child realm is consumed. Root navigation remains
                // pending until its scheduler can transfer the creator snapshot.
                let queued = js.binding_with_realm(realm, |i| Ok(i.take_navigation())).ok().flatten();
                if queued.as_deref() != Some(navigation.url.as_str()) {
                    if let Some(url) = queued { let _ = js.binding_with_realm(realm, |i| { i.binding_location_navigate(&url, navigation.replace); Ok(()) }); }
                    return None;
                }
                self.state.borrow_mut().frames.pending_navigations.remove(&realm);
                Some(navigation)
            });
            if pending.is_none() && self.state.borrow().frames.documents.get(&node)
                .is_some_and(|old| old.src == src && old.srcdoc == srcdoc) { continue; }
            let source = if let Some(navigation) = &pending { navigation.source.clone() } else {
                let adapter = RuntimeBindingHost { js, state: self.state.clone() };
                let Ok(base_url) = adapter.dom_document_base_url(owner) else { continue; };
                let generation = host.borrow().dom.node_generation(owner).unwrap_or(0);
                let policy = {
                    let state = self.state.borrow();
                    let host = state.host.borrow();
                    state.frames.document_context(&host.dom, owner)
                };
                let Some(policy) = policy else { continue; };
                let origin = context_origin(adapter.js, &owner_context, owner_realm).unwrap_or_else(|_| "null".into());
                let client_context = adapter.document_network_policy(owner, generation);
                let referrer_policy = adapter.html_document_referrer_policy(owner).unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin);
                NavigationSource { document: owner, generation, base_url,
                    document_url: owner_context.document_url.clone(), origin,
                    fetch_origin: owner_context.fetch_origin.clone(), policy, client_context, referrer_policy }
            };
            let mut ancestor_origins = Vec::new();
            let mut ancestor = Some(owner_context.clone());
            for _ in 0..32 {
                let Some(context) = ancestor.take() else { break; };
                ancestor_origins.push(context.fetch_origin.clone());
                if context.frame.is_none() { break; }
                let state = self.state.borrow();
                ancestor = state.frames.contexts.iter().find(|(realm, candidate)|
                    state.frames.live_context(candidate, **realm) && js.binding_same_value(&candidate.window, &context.parent))
                    .map(|(_, context)| context.clone());
            }
            ancestor_origins.reverse();
            let Some(top_level_origin) = ancestor_origins.first().cloned() else { continue; };
            let request_referrer_policy = if pending.is_some() { source.referrer_policy } else {
                let host = host.borrow();
                resource_response::attribute_referrer_policy(host.dom.get_attribute_ns(node, None, "referrerpolicy"))
                    .unwrap_or(source.referrer_policy)
            };
            let navigation_context = NavigableContext { top_level_origin,
                parent_origin: owner_context.fetch_origin.clone(), ancestor_origins,
                sandbox_flags: sandbox.to_bits(), frame_csp: Default::default(),
                referrer: source.document_url.clone(), referrer_policy: resource_response::referrer_policy_name(request_referrer_policy).into(),
                now_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 };
            let mut response_policy = None;
            let mut response_referrer_policy = None;
            let loaded = (|| -> Result<LoadedChildDocument, String> {
                // `srcdoc` takes priority over `src` per spec: its content is
                // used verbatim and the navigable's URL becomes "about:srcdoc"
                // rather than being resolved/fetched.
                let reload_srcdoc = pending.as_ref().filter(|navigation| navigation.reload
                    && ParsedUrl::parse(&navigation.url).is_ok_and(|url| matches_about_url(&url, "srcdoc") && url.query().is_none()))
                    .and_then(|_| self.state.borrow().frames.documents.get(&node).and_then(|entry| entry.srcdoc.clone()));
                if let Some(html) = reload_srcdoc.as_deref().or_else(|| srcdoc.as_deref().filter(|_| pending.is_none())) {
                    return Ok(LoadedChildDocument::Deferred { source: html.to_owned(),
                        url: "about:srcdoc".into(), mime: "text/html".into(), encoding: "UTF-8" });
                }
                let logical_url = if let Some(navigation) = &pending { navigation.url.clone() }
                    else if let Some(input) = src.as_deref().filter(|source| !source.is_empty()) {
                        ParsedUrl::parse(&source.base_url).and_then(|base| base.resolve(input))
                            .map_err(|error| format!("iframe URL: {error:?}"))?.serialize()
                    } else { "about:blank".into() };
                if ParsedUrl::parse(&logical_url).is_ok_and(|url| matches_about_url(&url, "blank")) {
                    return Ok(LoadedChildDocument::Eager { parsed: amazing_html::parse(""),
                        url: logical_url, mime: "text/html".into(), encoding: "UTF-8" });
                }
                let loader = self.state.borrow().frames.resource_loader.clone();
                let canonical = loader.is_some();
                let response = if let Some(loader) = loader {
                    let request = HtmlFrameResourceRequest { requested_url: logical_url.clone(),
                        initiator: source.policy.clone(), kind: HtmlFrameResourceKind::Document,
                        client_context: source.client_context.clone(),
                        nonce: String::new(), integrity: String::new(),
                        embedding: Some(navigation_context.clone()),
                        request_mode: amazing_fetch::RequestMode::Navigate,
                        credentials: amazing_fetch::CredentialsMode::Include,
                        referrer: navigation_context.referrer.clone(),
                        referrer_policy: amazing_fetch::ReferrerPolicy::parse_header(&navigation_context.referrer_policy)
                            .unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin) };
                    resource_response::load_document_response(&request, &loader)?
                } else { fetch_child_document(&logical_url, &navigation_context, resolver.as_ref())? };
                if !response.is_success() { return Err(format!("iframe status {}", response.status)); }
                let mime = content_type(&response, &logical_url,
                    !canonical && (resolver.is_some() || logical_url.starts_with("file:")))
                    .ok_or_else(|| "iframe response has no supported media type".to_owned())?;
                let response_mime = resource_response::response_mime_type(&response);
                let charset = response_mime.as_ref().and_then(|mime| mime.parameter("charset"));
                let mut policy = amazing_fetch::service::ClientContext::default();
                let mut policy_headers = amazing_fetch::HeaderList::default();
                for (name, value) in response.headers.iter() {
                    policy_headers.append(name, value).map_err(|error| format!("iframe response header: {error:?}"))?;
                }
                policy.csp = amazing_fetch::CspPolicyList::from_headers(&policy_headers)
                    .map_err(|error| format!("iframe response policy: {error:?}"))?;
                response_referrer_policy = Some(resource_response::response_referrer_policy(&response.headers));
                response_policy = Some(policy);
                let document_url = if canonical || resolver.is_none() { response.url.clone() } else { logical_url.clone() };
                // HTML-34: only the HTML case defers to a script-interleaved
                // parse (`ResumableParser` is HTML-only); XML/XHTML iframe
                // documents keep the pre-HTML-34 one-shot parse-then-import,
                // same as before.
                if mime == "text/html" {
                    let fallback = self.frame_encoding_fallback(node, &document_url, sandbox);
                    let decoded = amazing_html::encoding::decode_with_fallback(&response.body, charset, fallback);
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
                Err(_) if pending.is_some() => continue,
                Err(_) => (None, String::new(), None),
            };
            // Per spec, a `load` event is only queued for this navigation
            // when the element has a `src` or `srcdoc` attribute at all
            // (present, even if empty) -- not for the implicit `about:blank`
            // an iframe with neither gets purely from being inserted.
            let fire_load_events = pending.is_some() || src.is_some() || srcdoc.is_some();
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
                // Navigation-created documents permit DSD regardless of sandbox scripting.
                let _ = host.borrow_mut().dom.set_document_allows_declarative_shadow_roots(document, true);
                if ParsedUrl::parse(&url).is_ok_and(|url| matches_about_url(&url, "blank")
                    || (matches_about_url(&url, "srcdoc") && url.query().is_none())) {
                    if let Some(generation) = host.borrow().dom.node_generation(document) {
                        self.state.borrow_mut().frames.about_base_urls.insert(document, (generation, source.base_url.clone()));
                    }
                }
                let _ = self.ensure_realm(js, node, document, &url, sandbox, Some(&source));
                // Only actual about documents inherit their initiating policy.
                // Remote documents receive their own response policy, never
                // the creator's CSP merely because their origin is related.
                let policy = response_policy.clone().or_else(|| source.client_context.clone());
                if let Some(policy) = policy {
                    let adapter = RuntimeBindingHost { js, state: self.state.clone() };
                    if adapter.set_document_network_policy(document, policy).is_err() { continue; }
                }
            }
            if let Some(document) = document {
                let policy = response_referrer_policy.unwrap_or(source.referrer_policy);
                let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
                if adapter.html_set_document_referrer_policy(document, policy).is_err() { continue; }
            }
            let mut state = self.state.borrow_mut();
            let navigation = ChildNavigation { frame: node, requested_src: src.clone(),
                committed_url: url.clone(), document };
            state.frames.documents.insert(node, FrameDocument { src, srcdoc, document, url, sandbox, html, source: Some(source) });
            state.frames.pending_loads.push_back((node, document, fire_load_events));
            let hook = state.frames.navigation_hook.clone();
            drop(state);
            if let Some(hook) = hook { hook(navigation); }
        }
        {
            let mut state = self.state.borrow_mut();
            let live: HashMap<_, _> = state.frames.documents.iter()
                .map(|(frame, entry)| (*frame, entry.document)).collect();
            state.frames.pending_loads.retain(|(frame, document, _)| live.get(frame) == Some(document));
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
        let pending = self.state.borrow().frames.windows.get(&frame)
            .is_some_and(|(realm, _)| self.state.borrow().frames.pending_navigations.contains_key(realm));
        if pending {
            self.navigate_iframes(js);
            if self.state.borrow().frames.documents.get(&frame).and_then(|entry| entry.document) != Some(document) { return Ok(()); }
        }
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
            None => self.ensure_realm(js, frame, document, &child_url, sandbox, None)?,
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
                                    self.load_module_source(i, &url).ok().map(|text| (text, url))
                                } else { self.fetch_document_classic_script(i, document, script.node, src, &child_url, resolver.as_ref()) };
                                if let Some((text, final_url)) = text {
                                    self.execute_document_script(i, script, &text, &final_url)?;
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
        let allow_shadow_roots = host.borrow().dom.document_allows_declarative_shadow_roots(document)
            .map_err(|error| Throw::type_error(format!("iframe shadow policy: {error:?}")))?;
        parser.set_allow_declarative_shadow_roots(allow_shadow_roots);
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
                            .map(|text| (text, url.clone()))
                    } else { self.fetch_document_classic_script(js, document, script.node, src, child_url, resolver) };
                    if let Some((text, final_url)) = text {
                        js.binding_with_realm(realm, |i| self.execute_document_script(i, &script, &text, &final_url))
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
            // Connected parser-created frames need genuine child contexts
            // before this document's scripts can observe contentWindow.
            // Navigation only queues loads; their dispatch remains deferred.
            self.navigate_iframes(js);
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
        if let Some((realm, context)) = state.frames.contexts.iter().find(|(realm, context)|
            context.document == root && state.frames.live_context(context, **realm)) {
            return (Some(*realm), context.document_url.clone());
        }
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
            let (realm, legacy_base) = self.realm_and_base_for_document(root);
            let base = RuntimeBindingHost { js, state: self.state.clone() }
                .dom_document_base_url(root).unwrap_or(legacy_base);
            let script_url = match realm {
                Some(realm) => js.binding_with_realm(realm, |i| Ok(self.page_script_url(i, &script, &base))).unwrap_or_else(|_| base.clone()),
                None => self.page_script_url(js, &script, &base),
            };
            let run_body = |js: &mut Interpreter, body: &str, url: &str| {
                // A script's own uncaught exception does not stop the page
                // or suppress its `load` event (that pair reflects fetch
                // success/failure, not execution success) -- matches
                // `cmd/js.rs`'s existing top-level script driver, which
                // swallows the same `Err` into a diagnostics list rather
                // than propagating it.
                let _ = match realm {
                    Some(realm) => js.binding_with_realm(realm, |i| self.execute_document_script(i, &script, body, url)),
                    None => self.execute_document_script(js, &script, body, url),
                };
            };
            match &script.src {
                None => {
                    run_body(js, &script.body, &script_url);
                    if script.kind == crate::cmd::js::ScriptKind::Classic { let _ = self.dispatch_script_event(js, node, "load"); }
                }
                Some(src) => match if script.kind == crate::cmd::js::ScriptKind::Module {
                    match realm {
                        Some(realm) => js.binding_with_realm(realm, |i| self.load_module_source(i, &script_url).map_err(Throw::type_error)).ok(),
                        None => self.load_module_source(js, &script_url).ok(),
                    }.map(|text| (text, script_url.clone()))
                } else { self.fetch_document_classic_script(js, root, script.node, src, &base, resolver.as_ref()) } {
                    Some((text, final_url)) => {
                        run_body(js, &text, &final_url);
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
    pub(super) fn html_document_creation_origin(&self, document: NodeIdx,
        realm: RealmHandle) -> Option<amazing_fetch::Origin> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        state.frames.document_creation_origin(&host.dom, document, realm)
    }

    pub(super) fn html_document_origin(&self, document: NodeIdx) -> Option<amazing_fetch::Origin> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        state.frames.document_origin(&host.dom, document)
    }

    pub(super) fn html_set_inert_document_origin(&self, document: NodeIdx,
        origin: amazing_fetch::Origin) -> Result<(), amazing_dom::DomError> {
        let mut state = self.state.borrow_mut();
        let host = state.host.clone();
        let host = host.borrow();
        state.frames.set_inert_document_origin(&host.dom, document, origin)
    }

    fn html_navigation_source(&mut self, url: &str, replace: bool) -> Result<HtmlNavigation, Throw> {
        let target = self.js.binding_current_realm();
        let source = self.js.binding_incumbent_realm();
        let contexts = {
            let state = self.state.borrow();
            state.frames.contexts.get(&target).cloned().zip(state.frames.contexts.get(&source).cloned())
        };
        let Some((target_context, source_context)) = contexts else {
            return Err(throw_dom_error(self, DomError::NotSupportedError));
        };
        let base_url = self.dom_document_base_url(source_context.document).map_err(|error| throw_dom_error(self, error))?;
        let origin = context_origin(self.js, &source_context, source)?;
        let generations = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            if !state.frames.live_context(&target_context, target) || !state.frames.live_context(&source_context, source) { None }
            else { host.dom.node_generation(target_context.document).zip(host.dom.node_generation(source_context.document)) }
        };
        let Some((target_generation, source_generation)) = generations else {
            return Err(Throw::type_error("Stale navigation document"));
        };
        let policy = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            state.frames.document_context(&host.dom, source_context.document)
        }.ok_or_else(|| Throw::type_error("Stale navigation source context"))?;
        let client_context = self.document_network_policy(source_context.document, source_generation);
        let referrer_policy = self.html_document_referrer_policy(source_context.document)
            .unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin);
        Ok(HtmlNavigation { document: target_context.document, generation: target_generation,
            url: url.into(), replace, reload: false, source: NavigationSource { document: source_context.document,
                generation: source_generation, base_url, document_url: source_context.document_url,
                origin, fetch_origin: source_context.fetch_origin, policy, client_context, referrer_policy } })
    }

    pub(super) fn html_location_navigate(&mut self, url: &str, replace: bool) -> Result<(), Throw> {
        let navigation = self.html_navigation_source(url, replace)?;
        let target = self.js.binding_current_realm();
        let frame = self.state.borrow().frames.contexts.get(&target).and_then(|context| context.frame);
        let document = navigation.document;
        let mut state = self.state.borrow_mut();
        state.frames.pending_navigations.insert(target, navigation);
        if let Some(frame) = frame { state.frames.pending_loads.push_back((frame, Some(document), false)); }
        drop(state);
        self.js.binding_location_navigate(url, replace);
        Ok(())
    }

    pub(super) fn html_location_reload(&mut self) -> Result<(), Throw> {
        let target = self.js.binding_current_realm();
        let context = self.state.borrow().frames.contexts.get(&target).cloned()
            .ok_or_else(|| Throw::type_error("Missing reload document"))?;
        let url = if context.frame.is_some() { context.document_url } else { self.js.binding_location_href().to_owned() };
        let mut navigation = self.html_navigation_source(&url, false)?;
        navigation.reload = true;
        if let Some(frame) = context.frame {
            if let Some(source) = self.state.borrow().frames.documents.get(&frame).and_then(|entry| entry.source.clone()) {
                navigation.source = source;
            }
        }
        let frame = self.state.borrow().frames.contexts.get(&target).and_then(|context| context.frame);
        let document = navigation.document;
        let mut state = self.state.borrow_mut();
        state.frames.pending_navigations.insert(target, navigation);
        if let Some(frame) = frame { state.frames.pending_loads.push_back((frame, Some(document), false)); }
        drop(state);
        if frame.is_some() { self.js.binding_location_navigate(&url, false); }
        self.js.binding_location_reload();
        Ok(())
    }

    pub(super) fn html_document_location(&mut self, document: NodeIdx) -> Result<Option<JsValue>, Throw> {
        let realm = self.state.borrow().frames.contexts.iter().find_map(|(realm, context)|
            (context.document == document && self.state.borrow().frames.live_context(context, *realm)).then_some(*realm));
        let Some(realm) = realm else { return Ok(None); };
        let state = self.state.clone();
        self.js.binding_with_realm(realm, |js| {
            let mut adapter = RuntimeBindingHost { js, state };
            let arena = adapter.state.borrow().arena;
            adapter.wrap_platform_object(PlatformKey { arena, slot: 0, generation: 1 }, InterfaceBrand::LOCATION).map(Some)
        })
    }

    /// Actual committed URL, independent of a pending Location or script scope.
    pub(super) fn html_document_url(&self, document: NodeIdx) -> Option<String> {
        self.state.borrow().frames.document_url(document)
    }

    /// A navigated child document retains its associated WindowProxy even
    /// when a parent-realm script reads the document's defaultView.
    pub(super) fn iframe_document_window(&self, document: NodeIdx) -> Option<JsValue> {
        let state = self.state.borrow();
        state.frames.contexts.iter().find_map(|(realm, context)| {
            (context.document == document && state.frames.live_context(context, *realm))
                .then(|| context.window.clone())
        })
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
        let document = self.current_document_for_nodes()?;
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
    pub(super) fn html_document_context(&self, document: NodeIdx) -> Option<HtmlDocumentContext> {
        let state = self.state.borrow();
        let host = state.host.borrow();
        state.frames.document_context(&host.dom, document)
    }

    /// HTML's immutable about base URL, only when applicable to this Document.
    /// URL/base element changes in the creator cannot change the saved value.
    pub(super) fn html_document_fallback_base_url(&self, document: NodeIdx) -> Option<String> {
        let state = self.state.borrow();
        let (generation, base) = state.frames.about_base_urls.get(&document)?;
        let host = state.host.borrow();
        if host.dom.node_generation(document) != Some(*generation) { return None; }
        let metadata = host.dom.document_metadata(document).ok()??;
        let url = ParsedUrl::parse(&metadata.url).ok()?;
        if matches_about_url(&url, "blank") || (matches_about_url(&url, "srcdoc") && url.query().is_none()) {
            Some(base.clone())
        } else { None }
    }

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
            let document_url = self.state.borrow().document_url.clone().unwrap_or_else(|| "about:blank".into());
            let fetch_origin = HttpUrl::parse(&document_url).map(|url| url.origin().clone()).unwrap_or_else(|_| amazing_fetch::Origin::opaque());
            let document_generation = self.state.borrow().host.borrow().dom.node_generation(NodeIdx(0))
                .ok_or_else(|| Throw::type_error("Missing root document"))?;
            self.state.borrow().host.borrow_mut().dom.set_document_allows_declarative_shadow_roots(NodeIdx(0), true)
                .map_err(|error| Throw::type_error(format!("Root document shadow policy: {error:?}")))?;

            self.state.borrow_mut().frames.contexts.insert(realm, WindowContext {
                document: NodeIdx(0), document_generation, frame: None, global, parent: window.clone(), top: window.clone(), window,
                origin: ParsedUrl::parse(&document_url).map(|url| url.origin().serialize()).unwrap_or_else(|_| "null".into()), fetch_origin, document_url, installed: false, indices: 0,
            });
        }
        let context = self.state.borrow().frames.contexts[&realm].clone();
        if context.installed { return Ok(()); }
        for property in [HtmlWindowProperty::Frames, HtmlWindowProperty::Length, HtmlWindowProperty::Parent, HtmlWindowProperty::Top] {
            let name = property.name();
            let getter_name = match property { HtmlWindowProperty::Frames => "get frames", HtmlWindowProperty::Length => "get length", HtmlWindowProperty::Parent => "get parent", HtmlWindowProperty::Top => "get top", _ => unreachable!() };
            let setter_name = match property { HtmlWindowProperty::Frames => "set frames", HtmlWindowProperty::Length => "set length", HtmlWindowProperty::Parent => "set parent", HtmlWindowProperty::Top => "set top", _ => unreachable!() };
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
        let state = self.state.clone();
        self.js.binding_set_window_proxy_property_hook(Rc::new(move |js, access| {
            let current = state.borrow().frames.contexts.get(&access.current).cloned();
            let target = state.borrow().frames.contexts.get(&access.target).cloned();
            let current_origin = match current { Some(context) => context_origin(js, &context, access.current)?, None => String::new() };
            let target_origin = match &target { Some(context) => context_origin(js, context, access.target)?, None => access.target_origin };
            use amazing_js::interp::WindowProxyProperty;
            if access.current == access.target || current_origin == target_origin {
                return Ok(WindowProxyProperty::Target);
            }
            let Some(JsValue::Str(name)) = access.key else { return Ok(WindowProxyProperty::Descriptor(None)); };
            let name = name.to_utf8_lossy();
            let key = (access.current, access.target, name.clone());
            if let Some(descriptor) = state.borrow().frames.cross_origin_descriptors.get(&key).cloned() {
                return Ok(WindowProxyProperty::Descriptor(Some(descriptor)));
            }
            let bound = match name.as_str() {
                "window" | "self" | "frames" => Some(HtmlWindowProperty::BoundFrames(access.target)),
                "length" => Some(HtmlWindowProperty::BoundLength(access.target)),
                "parent" => Some(HtmlWindowProperty::BoundParent(access.target)),
                "top" => Some(HtmlWindowProperty::BoundTop(access.target)),
                _ => None,
            };
            let descriptor = if let Some(property) = bound {
                let getter = RuntimeBindingHost { js, state: state.clone() }.native("", 0,
                    NativeEntry::HtmlWindowGet(property), HostCallPolicy::CallOnly);
                Property { accessor: true, value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                    getter: Some(getter), setter: None }
            } else {
                let value = name.parse::<usize>().ok().and_then(|index| {
                    let state = state.borrow();
                    let target = target.as_ref()?;
                    let host = state.host.borrow();
                    host.dom.tree_order(target.document).into_iter()
                        .filter_map(|node| state.frames.windows.get(&node).map(|(_, window)| window.clone())).nth(index)
                });
                let Some(value) = value else { return Ok(WindowProxyProperty::Descriptor(None)); };
                // Indexed child properties are evaluated afresh as the tree changes.
                return Ok(WindowProxyProperty::Descriptor(Some(Property { accessor: false, value, writable: false,
                    enumerable: true, configurable: true, getter: None, setter: None })));
            };
            state.borrow_mut().frames.cross_origin_descriptors.insert(key, descriptor.clone());
            Ok(WindowProxyProperty::Descriptor(Some(descriptor)))
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

    /// Association with a real browsing context, independent of readiness or full activity.
    pub(super) fn html_document_has_browsing_context(&self, document: NodeIdx) -> bool {
        let state = self.state.borrow();
        let host = state.host.borrow();
        let Some(generation) = host.dom.node_generation(document) else { return false; };
        if !matches!(host.dom.get(document).kind, NodeKind::Document) { return false; }
        state.frames.contexts.iter().any(|(realm, context)| context.document == document
            && context.document_generation == generation && state.frames.live_context(context, *realm))
    }

    /// Actual Window receiver and caller policy; never reads author properties or guesses a Document.
    pub(super) fn html_window_document_for_selection(&mut self, receiver: &JsValue) -> Result<NodeIdx, Throw> {
        let (target_realm, target) = self.html_window_receiver(receiver)?;
        let source_realm = self.js.binding_entry_realm();
        let permitted = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            if host.dom.node_generation(target.document) != Some(target.document_generation)
                || !matches!(host.dom.get(target.document).kind, NodeKind::Document) {
                return Err(Throw::type_error("Stale Window Document"));
            }
            let source = state.frames.contexts.get(&source_realm)
                .filter(|source| host.dom.node_generation(source.document) == Some(source.document_generation))
                .ok_or_else(|| Throw::type_error("Window caller has no actual Document"))?;
            source_realm == target_realm || source.fetch_origin == target.fetch_origin
        };
        if !permitted {
            // Allocate through the native append-only exception arena without global getters, constructors or reaction checkpoints.
            let (arena, slot) = {
                let state = self.state.borrow();
                let slot = state.host.borrow_mut().dom.create_dom_exception("SecurityError".encode_utf16().collect(),
                    "Blocked cross-origin Window selection access".encode_utf16().collect());
                (state.arena, slot)
            };
            let error = self.wrap_platform_object(PlatformKey { arena, slot: slot as u64, generation: 1 },
                InterfaceBrand::DOM_EXCEPTION)?;
            return Err(self.js.binding_throw(error));
        }
        Ok(target.document)
    }

    pub(super) fn html_window_get(&mut self, property: HtmlWindowProperty, receiver: JsValue) -> Result<JsValue, Throw> {
        let bound = match property {
            HtmlWindowProperty::BoundFrames(realm) | HtmlWindowProperty::BoundLength(realm)
            | HtmlWindowProperty::BoundParent(realm) | HtmlWindowProperty::BoundTop(realm) => Some(realm),
            _ => None,
        };
        let (realm, context) = if let Some(realm) = bound {
            (realm, self.state.borrow().frames.contexts.get(&realm).cloned()
                .ok_or_else(|| Throw::type_error("Stale Window context"))?)
        } else { self.html_window_receiver(&receiver)? };
        let live = self.state.borrow().frames.live_context(&context, realm);
        Ok(match property {
            HtmlWindowProperty::Frames | HtmlWindowProperty::BoundFrames(_) => context.window,
            HtmlWindowProperty::Length | HtmlWindowProperty::BoundLength(_) => {
                let state = self.state.borrow();
                let host = state.host.borrow();
                let count = if live { host.dom.tree_order(context.document).into_iter()
                    .filter(|node| state.frames.windows.contains_key(node)).count() } else { 0 };
                JsValue::num(count as f64)
            }
            HtmlWindowProperty::Parent | HtmlWindowProperty::BoundParent(_) => if live { context.parent } else { JsValue::Null },
            HtmlWindowProperty::Top | HtmlWindowProperty::BoundTop(_) => if live { context.top } else { JsValue::Null },
        })
    }

    pub(super) fn html_window_replaceable_set(&mut self, property: HtmlWindowProperty, receiver: JsValue,
        args: &[JsValue]) -> Result<JsValue, Throw> {
        let descriptor = self.js.binding_create_object(None)?;
        for (name, value) in [
            ("value", args.first().cloned().unwrap_or(JsValue::Undefined)),
            ("writable", JsValue::Bool(true)), ("enumerable", JsValue::Bool(true)), ("configurable", JsValue::Bool(true)),
        ] {
            self.js.binding_define_data_property(&descriptor, name, value, amazing_js::interp::BindingPropertyFlags::METHOD)?;
        }
        // [Replaceable] uses the receiver's actual [[DefineOwnProperty]],
        // including ordinary Proxy traps and cross-origin Window policy.
        if !self.js.binding_define_own_property(&receiver, property.name(), &descriptor)? {
            return Err(Throw::type_error("Cannot replace Window property"));
        }
        Ok(JsValue::Undefined)
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
        let owner = host.dom.owner_document(node).unwrap_or(NodeIdx(0));
        if host.dom.root(node) != owner || !state.frames.has_document(owner) { return Ok(JsValue::Null); }
        let Some(frame) = state.frames.documents.get(&node) else { return Ok(JsValue::Null) };
        let Some(document) = frame.document else { return Ok(JsValue::Null) };
        // Compare actual caller/target contexts, including inherited blank
        // origins and unique sandbox origins, rather than the top-level URL.
        let realm = state.frames.windows.get(&node).map(|(realm, _)| *realm);
        let caller_realm = self.js.binding_current_realm();
        let caller = state.frames.contexts.get(&caller_realm).cloned();
        let target = realm.and_then(|realm| state.frames.contexts.get(&realm).cloned());
        drop(host);
        drop(state);
        let visible = match (caller, target, realm) {
            (Some(caller), Some(target), Some(realm)) => caller_realm == realm
                || context_origin(self.js, &caller, caller_realm)? == context_origin(self.js, &target, realm)?,
            _ => false,
        };
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
        let host = state.host.borrow();
        let owner = host.dom.owner_document(node).unwrap_or(NodeIdx(0));
        if host.dom.root(node) != owner || !state.frames.has_document(owner) { return Ok(JsValue::Null); }
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

    /// A borrowed Location function follows its genuine receiver's Window,
    /// with the caller's origin checked before entering the target realm.
    fn invoke_html_location(&mut self, operation: Operation, receiver: JsValue,
        args: &[JsValue]) -> Result<JsValue, Throw> {
        let contexts = self.state.borrow().frames.contexts.iter()
            .map(|(realm, context)| (*realm, context.clone())).collect::<Vec<_>>();
        let target = contexts.iter().find(|(_, context)| match &context.global {
            JsValue::Object(global) => global.borrow().get_own("[[LocationObject]]")
                .is_some_and(|location| self.js.binding_same_value(&receiver, &location)),
            _ => false,
        }).cloned();
        let Some((target_realm, target)) = target else {
            return invoke(self, operation, receiver, args);
        };
        let source_realm = self.js.binding_incumbent_realm();
        let source = contexts.iter().find(|(realm, _)| *realm == source_realm).map(|(_, context)| context.clone());
        let source_origin = match &source { Some(context) => context_origin(self.js, context, source_realm)?, None => "null".into() };
        let target_origin = context_origin(self.js, &target, target_realm)?;
        if source_origin != target_origin && !matches!(operation, Operation::LocationSetHref | Operation::LocationReplace) {
            let error = self.js.binding_with_realm(source_realm, |js| {
                let global = js.binding_global_object();
                let constructor = js.binding_get_property(&global, "DOMException")?;
                js.binding_construct(&constructor, vec![JsValue::str("Blocked cross-origin Location access"), JsValue::str("SecurityError")])
            })?;
            return Err(self.js.binding_throw(error));
        }
        let state = self.state.clone();
        let args = if target.frame.is_some() && matches!(operation,
            Operation::LocationSetHref | Operation::LocationAssign | Operation::LocationReplace) {
            let value = args.first().ok_or_else(|| Throw::type_error("Missing required argument"))?;
            let scalar = String::from_utf16_lossy(&self.to_dom_string(value)?);
            let source = source.ok_or_else(|| throw_dom_error(self, DomError::NotSupportedError))?;
            let base = self.dom_document_base_url(source.document).map_err(|error| throw_dom_error(self, error))?;
            let base = ParsedUrl::parse(&base).ok();
            let url = ParsedUrl::parse_with_base(&scalar, base.as_ref())
                .map_err(|_| throw_dom_error(self, DomError::SyntaxError))?.serialize();
            vec![JsValue::str(&url)]
        } else { args.to_vec() };
        self.js.binding_with_realm(target_realm, |js| {
            let mut adapter = RuntimeBindingHost { js, state };
            invoke(&mut adapter, operation, receiver, &args)
        })
    }

    pub(super) fn invoke_operation_with_html_hooks(&mut self, operation: Operation,
        receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        if matches!(operation, Operation::LocationHref | Operation::LocationSetHref | Operation::LocationOrigin
            | Operation::LocationProtocol | Operation::LocationSetProtocol | Operation::LocationHost | Operation::LocationSetHost
            | Operation::LocationHostname | Operation::LocationSetHostname | Operation::LocationPort | Operation::LocationSetPort
            | Operation::LocationPathname | Operation::LocationSetPathname | Operation::LocationSearch | Operation::LocationSetSearch
            | Operation::LocationHash | Operation::LocationSetHash | Operation::LocationAssign | Operation::LocationReplace
            | Operation::LocationReload | Operation::LocationAncestorOrigins | Operation::LocationToString) {
            return self.invoke_html_location(operation, receiver, args);
        }
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
        // XML fragment parsing marks its script elements already started;
        // HTML Range fragments retain the append-executable Fragment policy.
        // These ids are the lower operation's actual imported result, including
        // scripts inside real template contents, never temporary parser ids.
        if operation == Operation::RangeCreateContextualFragment {
            if let Ok(value) = &result {
                if let Some(fragment) = self.node_from_value(value) {
                    let (document, html_hint) = {
                        let state = self.state.borrow();
                        let host = state.host.borrow();
                        let document = host.dom.owner_document(fragment).unwrap_or(fragment);
                        (document, host.dom.document_metadata(document).ok().flatten()
                            .map(|metadata| metadata.content_type == "text/html"))
                    };
                    if !html_hint.unwrap_or_else(|| self.is_html_document(document)) {
                        let scripts = {
                            let state = self.state.borrow();
                            let host = state.host.borrow();
                            let mut pending = vec![fragment];
                            let mut visited = HashSet::new();
                            let mut scripts = Vec::new();
                            while let Some(node) = pending.pop() {
                                if !visited.insert(node) { continue; }
                                if matches!(host.dom.namespace(node), Some(Namespace::Html) | Some(Namespace::Svg))
                                    && host.dom.local_name(node) == Some("script") { scripts.push(node); }
                                pending.extend(host.dom.children(node));
                                if let Some(contents) = host.dom.template_contents(node) { pending.push(contents); }
                            }
                            scripts
                        };
                        self.state.borrow_mut().frames.already_started_scripts.extend(scripts);
                    }
                }
            }
        }
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
        // produces (XML Range fragments are covered above; HTML Range fragments
        // retain their separate append-executable policy).
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
#[path = "runtime_host_html/tests.rs"]
mod tests;
