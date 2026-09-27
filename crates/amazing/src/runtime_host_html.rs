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
}

/// Classic inline `<script>` bodies in tree order. External `src` and
/// `type="module"` scripts are not fetched or evaluated for iframe
/// documents yet; this mirrors the WPT harness's classic-only scheduling.
struct ChildScript { node: NodeIdx, src: Option<String>, body: String }

/// Classic `<script>` elements in tree order, inline or external.
/// `type="module"` is still not fetched or evaluated for iframe documents:
/// the same limitation the top-level WPT harness has for module/async/defer.
fn collect_classic_scripts(dom: &Dom, node: NodeIdx, out: &mut Vec<ChildScript>) {
    if dom.namespace(node) == Some(Namespace::Html) && dom.local_name(node) == Some("script") {
        let src = dom.get_attribute_ns(node, None, "src").map(str::to_owned);
        let module = dom.get_attribute_ns(node, None, "type")
            .is_some_and(|value| value.eq_ignore_ascii_case("module"));
        if !module {
            let mut source = String::new();
            for child in dom.children(node) {
                if let NodeKind::Text { data } = &dom.get(child).kind { source.push_str(data); }
            }
            if src.is_some() || !source.trim().is_empty() {
                out.push(ChildScript { node, src, body: source });
            }
        }
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
    if mime != "application/octet-stream" && !mime.is_empty() {
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
        return Ok((amazing_html::parse_bytes(body, charset), decoded.encoding.name()));
    }
    if matches!(mime, "text/xml" | "application/xml" | "application/xhtml+xml")
        || mime.ends_with("+xml") {
        let decoded = amazing_html::encoding::decode(body, charset);
        let parsed = amazing_html::parse_xml(&decoded.text)
            .map_err(|error| format!("XML parse failed: {error:?}"))?;
        return Ok((parsed, decoded.encoding.name()));
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
        let origin = if sandbox.sandboxed && !sandbox.allow_same_origin {
            let mut state = self.state.borrow_mut();
            state.frames.opaque_origin_counter += 1;
            format!("null (sandboxed #{})", state.frames.opaque_origin_counter)
        } else if child_url == "about:srcdoc" || child_url == "about:blank" {
            self.state.borrow().document_url.as_deref()
                .and_then(|parent| ParsedUrl::parse(parent).ok())
                .map(|url| url.origin().serialize())
                .unwrap_or_else(|| "null".into())
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
        let state = self.state.clone();
        js.binding_with_realm(realm, |i| {
            // `host_native_dispatch` is per-realm state (see JSRT-9's
            // `RealmState`), so a freshly created realm has none until we
            // install one; without this every native binding call in the
            // child (DOM methods, `contentWindow`/`contentDocument`
            // getters, etc.) throws "host function dispatcher is not
            // installed".
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
            adapter.install_storage_interfaces()?;
            adapter.install_messaging()?;
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
            host.dom.tree_order(NodeIdx(0)).into_iter().filter_map(|node| {
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
                        url: document_url, mime, encoding: decoded.encoding.name() });
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
                    let mut adapter = RuntimeBindingHost { js: i, state: state.clone() };
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
                            None => { adapter.js.run_script(&script.body)?; }
                            Some(src) => {
                                if let Some(text) = fetch_child_script(src, &child_url, resolver.as_ref()) {
                                    adapter.js.run_script(&text)?;
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
            match &script.src {
                None => js.binding_with_realm(realm, |i| i.run_script(&script.body).map(|_| ())),
                Some(src) => {
                    if let Some(text) = fetch_child_script(src, child_url, resolver) {
                        js.binding_with_realm(realm, |i| i.run_script(&text).map(|_| ()))
                    } else { Ok(()) }
                }
            }
        };
        let import = Rc::new(RefCell::new(import));
        let outcome: Result<(), Throw> = loop {
            let signal = parser.borrow_mut().run();
            deferred_events.extend(parser.borrow_mut().take_deferred_or_async());
            {
                let mut host_ref = host.borrow_mut();
                import.borrow_mut().sync(&mut host_ref.dom, parser.borrow().dom())
                    .map_err(|error| Throw::type_error(format!("iframe graft: {error:?}")))?;
            }
            match signal {
                amazing_html::ParseSignal::Eof => break Ok(()),
                amazing_html::ParseSignal::Script(source_node) => {
                    if !can_run_scripts { continue; }
                    if let Err(error) = run_one(js, &host, source_node, &import, child_url, resolver.as_ref()) {
                        break Err(error);
                    }
                }
            }
        };
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
}

/// A `Rc<RefCell<IncrementalDocumentImport>>` alias, purely so `run_one`'s
/// closure signature above (already long with every other captured piece of
/// per-frame context) does not also spell out the full generic path.
type IncrementalImportRef = Rc<RefCell<amazing_dom::markup::IncrementalDocumentImport>>;

impl RuntimeBindingHost<'_> {
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
        self.define_data_property(&prototype, "close", close, true, false, true)
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
            parser.borrow_mut().write(&text);
            return Ok(JsValue::Undefined);
        }
        self.write_into_document(document, &text)?;
        Ok(JsValue::Undefined)
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
        let parsed = amazing_html::parse_fragment("body", text)
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
        for script in scripts {
            narrow_write_insertion_point_state(&self.state, document, script.node);
            match script.src {
                None => { self.js.run_script(&script.body)?; }
                Some(src) => {
                    if let Some(base) = base.as_deref() {
                        if let Some(text) = fetch_child_script(&src, base, resolver.as_ref()) {
                            self.js.run_script(&text)?;
                        }
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
        self.define_accessor_property(&prototype, "src", Some(src_getter), Some(src_setter), true, true)
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

    pub(super) fn invoke_operation_with_html_hooks(&mut self, operation: Operation,
        receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let result = self.invoke_operation_with_canvas_reset(operation, receiver, args);
        if result.is_ok() && matches!(operation,
            Operation::Append | Operation::Prepend | Operation::ReplaceChildren
            | Operation::Before | Operation::After | Operation::ReplaceWith | Operation::Remove
            | Operation::AppendChild | Operation::InsertBefore | Operation::RemoveChild
            | Operation::ReplaceChild | Operation::ElementSetInnerHTML
            | Operation::ElementInsertAdjacentHTML | Operation::ElementSetAttribute
            | Operation::ElementSetAttributeNS | Operation::ElementRemoveAttribute
            | Operation::ElementRemoveAttributeNS | Operation::ElementSetAttributeNode
            | Operation::ElementSetAttributeNodeNS | Operation::ElementRemoveAttributeNode) {
            RuntimeBindings { state: self.state.clone() }.navigate_iframes(self.js);
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
}
