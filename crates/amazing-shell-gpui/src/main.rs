#![deny(unsafe_code)]

mod a11y_bridge;
mod audio_output;
mod dom_runtime;
mod engine_compositor;
mod engine_viewport;
mod viewport_metrics;
mod compositor_presenter;
mod playback_clock;
mod session_state;
mod secure_video;
mod navigation_acquisition;
mod product_navigation;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, FocusHandle, IntoElement, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ScrollWheelEvent, ParentElement, Render, RenderImage, SharedString,
    Styled, Window, WindowBounds, WindowControlArea, WindowOptions, actions, canvas, div, img,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

use audio_output::AudioDeviceClock;
use engine_viewport::{EngineViewport, FormRequest, PageRuntime, RuntimeUpdate, ViewportIdentity};
use viewport_metrics::ViewportMetrics;
use playback_clock::{PlaybackUpdate, VideoPlaybackClock};
use session_state::BrowserSession;
use product_navigation::{PendingNavigation, HistoryCommit};
use secure_video::{FRAME_EVENT_CAPACITY, SecureVideoEvent, spawn_secure_video_source};
use amazing_compositor::{ScrollAxis, ScrollBehavior, ScrollInput, ScrollKey};

const STRIP: u32 = 0xd5dae2;
const STRIP_HOVER: u32 = 0xc6ccd6;
const SURFACE: u32 = 0xffffff;
const SURFACE_2: u32 = 0xeef1f5;
const SURFACE_2_HOVER: u32 = 0xe4e8ee;
const BORDER: u32 = 0xd8dce3;
const TEXT: u32 = 0x1b1e23;
const DIM: u32 = 0x5b6371;
const ICON: u32 = 0x4a5160;
const ACCENT: u32 = 0xb1552b;
const RULE: u32 = 0xe7e9ee;
const SECURE_VIDEO_POLL_MS: u64 = 50;

actions!(
    amazing_shell,
    [
        FocusAddress,
        NewBrowserTab,
        CloseBrowserTab,
        ReloadPage,
        NavigateBack,
        NavigateForward,
        ToggleBookmark,
        ShowDownloads,
        DismissChrome,
        FindInPage,
        NextBrowserTab,
        PreviousBrowserTab
    ]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind {
    Engine,
    Video,
    Document,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabGroup {
    Engine,
    Specs,
    Other,
}

#[derive(Clone)]
struct Tab {
    mono: String,
    title: String,
    url: String,
    kicker: String,
    lede: String,
    hue: u32,
    kind: TabKind,
    group: TabGroup,
    open: bool,
    bookmarked: bool,
    favicon: Option<Arc<RenderImage>>,
    viewport: Option<EngineViewport>,
    render_failure: Option<String>,
    failed_metrics: Option<ViewportMetrics>,
    history: Vec<String>,
    history_index: usize,
}

struct TabSpec<'a> {
    mono: &'a str,
    title: &'a str,
    url: &'a str,
    kicker: &'a str,
    lede: &'a str,
    hue: u32,
    kind: TabKind,
    group: TabGroup,
}

#[derive(Clone)]
struct PageInputState {
    tab: usize,
    node_identity: (u32, u64),
    control: usize,
    value: String,
    rect: (f32, f32, f32, f32),
    selected: bool,
}

struct RuntimeDirective {
    default_prevented: bool,
    navigation: Option<String>,
    history_delta: Option<i32>,
    history_update: Option<(String, bool)>,
}

impl Tab {
    fn from_spec(spec: TabSpec<'_>, viewport: Option<EngineViewport>) -> Self {
        Self {
            mono: spec.mono.into(),
            title: spec.title.into(),
            url: spec.url.into(),
            kicker: spec.kicker.into(),
            lede: spec.lede.into(),
            hue: spec.hue,
            kind: spec.kind,
            group: spec.group,
            open: true,
            bookmarked: false,
            favicon: None,
            viewport,
            render_failure: None,
            failed_metrics: None,
            history: vec![spec.url.into()],
            history_index: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChromeInputOwner {
    Document,
    Address,
    Find,
}

struct BrowserShell {
    last_viewport_receipt: Option<(ViewportMetrics, (f32, f32))>,
    viewport_metrics: ViewportMetrics,
    tabs: Vec<Tab>,
    page_runtimes: Vec<Option<PageRuntime>>,
    pending_navigation: Vec<Option<PendingNavigation>>,
    next_navigation_id: u64,
    last_navigation_trace: String,
    video_frames: Vec<EngineViewport>,
    video_clock: VideoPlaybackClock,
    video_frame_index: usize,
    pending_video_presentation_ms: Vec<u64>,
    secure_video_receiver: Option<std::sync::mpsc::Receiver<SecureVideoEvent>>,
    secure_video_requested: bool,
    secure_video_active: bool,
    secure_video_complete: bool,
    secure_video_recoveries: u64,
    audio_device_clock: Option<AudioDeviceClock>,
    secure_video_status: String,
    shell_focus: Option<FocusHandle>,
    address_focus: Option<FocusHandle>,
    address: String,
    committed_address: String,
    address_selected: bool,
    active: usize,
    next_tab_id: usize,
    engine_collapsed: bool,
    specs_collapsed: bool,
    sidebar: bool,
    downloads_open: bool,
    menu_open: bool,
    site_info_open: bool,
    profile_open: bool,
    page_input: Option<PageInputState>,
    find_focus: Option<FocusHandle>,
    find_open: bool,
    chrome_input_owner: ChromeInputOwner,
    find_query: String,
    find_matches: Vec<amazing_a11y::AxRect>,
    find_active: usize,
    session: Rc<RefCell<BrowserSession>>,
}

impl BrowserShell {
    fn new(address_focus: Option<FocusHandle>, find_focus: Option<FocusHandle>) -> Self {
        let session = if address_focus.is_some() {
            BrowserSession::load_default()
        } else {
            BrowserSession::ephemeral()
        };
        let session = Rc::new(RefCell::new(session));
        let mut shell = Self {
            last_viewport_receipt: None,
            viewport_metrics: ViewportMetrics::default(),
            tabs: vec![
                Tab::from_spec(
                    TabSpec {
                        mono: "A",
                        title: "Amazing — Phase 9 status",
                        url: "amazing://status",
                        kicker: "AMAZING://STATUS",
                        lede: "A browser engine written against the specs in pure Rust — std only in every engine core, with a hand-written tokenizer, cascade, layout, rasterizer, HTTP stack and interpreter.",
                        hue: ACCENT,
                        kind: TabKind::Engine,
                        group: TabGroup::Engine,
                    },
                    Some(EngineViewport::demo()),
                ),
                Tab::from_spec(
                    TabSpec {
                        mono: "V",
                        title: "Secure video · H.264 decoded frame",
                        url: "amazing://media/video",
                        kicker: "AMAZING://MEDIA/VIDEO",
                        lede: "A real H.264 access unit decoded through the isolated codec adapter and presented as RGBA by GPUI.",
                        hue: 0x3f7d4e,
                        kind: TabKind::Video,
                        group: TabGroup::Engine,
                    },
                    None,
                ),
                Tab::from_spec(
                    TabSpec {
                        mono: "H",
                        title: "html5lib tree-construction",
                        url: "localhost:8080/html5lib",
                        kicker: "LOCALHOST:8080",
                        lede: "Tree-construction fixtures for the tokenizer and the namespace-aware tree builder.",
                        hue: 0x2f7f8c,
                        kind: TabKind::Document,
                        group: TabGroup::Engine,
                    },
                    None,
                ),
                Tab::from_spec(
                    TabSpec {
                        mono: "S",
                        title: "HTML Standard § 13.2.5",
                        url: "html.spec.whatwg.org/multipage/parsing.html",
                        kicker: "HTML.SPEC.WHATWG.ORG",
                        lede: "Tokenization states, in the same order the Amazing tokenizer implements them.",
                        hue: 0x3a6ea8,
                        kind: TabKind::Document,
                        group: TabGroup::Specs,
                    },
                    None,
                ),
                Tab::from_spec(
                    TabSpec {
                        mono: "C",
                        title: "CSS Syntax Module Level 3",
                        url: "drafts.csswg.org/css-syntax-3",
                        kicker: "DRAFTS.CSSWG.ORG",
                        lede: "The token stream and parser entry points behind amazing-css.",
                        hue: 0x3a6ea8,
                        kind: TabKind::Document,
                        group: TabGroup::Specs,
                    },
                    None,
                ),
                Tab::from_spec(
                    TabSpec {
                        mono: "E",
                        title: "ECMA-262",
                        url: "tc39.es/ecma262",
                        kicker: "TC39.ES",
                        lede: "Runtime semantics for the tree-walking JavaScript interpreter.",
                        hue: 0x7a5aa8,
                        kind: TabKind::Document,
                        group: TabGroup::Specs,
                    },
                    None,
                ),
            ],
            page_runtimes: Vec::from([None, None, None, None, None, None]),
            pending_navigation: (0..6).map(|_| None).collect(),
            next_navigation_id: 1,
            last_navigation_trace: String::new(),
            video_frames: EngineViewport::video_demo_frames().expect("decode GPUI video demo"),
            video_clock: VideoPlaybackClock::new((0..6).map(|index| index * 500).collect(), 0)
                .expect("valid demo video timeline"),
            video_frame_index: 0,
            pending_video_presentation_ms: Vec::new(),
            secure_video_receiver: None,
            secure_video_requested: false,
            secure_video_active: false,
            secure_video_complete: false,
            secure_video_recoveries: 0,
            audio_device_clock: None,
            secure_video_status: "H.264 → I420 → RGBA".into(),
            shell_focus: None,
            address_focus,
            address: "amazing://status".to_string(),
            committed_address: "amazing://status".to_string(),
            address_selected: false,
            active: 0,
            next_tab_id: 1,
            engine_collapsed: false,
            specs_collapsed: true,
            sidebar: false,
            downloads_open: false,
            menu_open: false,
            site_info_open: false,
            profile_open: false,
            page_input: None,
            find_focus,
            find_open: false,
            chrome_input_owner: ChromeInputOwner::Document,
            find_query: String::new(),
            find_matches: Vec::new(),
            find_active: 0,
            session,
        };
        for tab in &mut shell.tabs {
            tab.bookmarked = shell.session.borrow().is_bookmarked(&tab.url);
        }
        shell
    }

    fn select_tab(&mut self, index: usize) {
        if self.tabs.get(index).is_some_and(|tab| tab.open) {
            self.active = index;
            self.committed_address = self.tabs[index].url.clone();
            self.address = self.pending_navigation[index].as_ref()
                .map_or_else(|| self.committed_address.clone(), |pending| pending.address.clone());
            self.address_selected = false;
            self.downloads_open = false;
            self.menu_open = false;
            self.site_info_open = false;
            self.profile_open = false;
            self.page_input = None;
            // Find-in-page is per-tab, not shell-global: without this a find
            // session opened on one tab (query, matches, the open find bar)
            // would still show once a later tab switch (including the
            // fallback switch `close_tab` makes when the active tab closes)
            // lands on a different tab -- confusing at best, and outright
            // invisible-but-still-`find_open` on a tab with no viewport (see
            // `find_is_available`), which is what SHELL-L9's product smoke
            // first surfaced as an apparent input freeze.
            self.find_open = false;
            self.chrome_input_owner = ChromeInputOwner::Document;
            self.find_query.clear();
            self.find_matches.clear();
            self.find_active = 0;
        }
    }

    fn add_tab(&mut self) {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::from_spec(
            TabSpec {
                mono: "+",
                title: &format!("New tab {id}"),
                url: "amazing://newtab",
                kicker: "AMAZING://NEWTAB",
                lede: "Type an address or search query to get started.",
                hue: 0x6b7280,
                kind: TabKind::Engine,
                group: TabGroup::Other,
            },
            Some(EngineViewport::new_tab()),
        ));
        self.page_runtimes.push(None);
        self.pending_navigation.push(None);
        self.select_tab(self.tabs.len() - 1);
        self.address.clear();
        self.address_selected = true;
    }

    fn close_tab(&mut self, index: usize) {
        if !self.tabs.get(index).is_some_and(|tab| tab.open) {
            return;
        }
        self.pending_navigation[index] = None;
        self.page_runtimes[index] = None;
        self.tabs[index].open = false;
        if index != self.active {
            return;
        }
        let next = ((index + 1)..self.tabs.len())
            .find(|candidate| self.tabs[*candidate].open)
            .or_else(|| {
                (0..index)
                    .rev()
                    .find(|candidate| self.tabs[*candidate].open)
            });
        if let Some(next) = next {
            self.select_tab(next);
        } else {
            self.add_tab();
        }
    }

    fn open_bookmark(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs[index].open = true;
        self.select_tab(index);
    }

    fn toggle_bookmark(&mut self) {
        self.tabs[self.active].bookmarked = !self.tabs[self.active].bookmarked;
        let tab = &self.tabs[self.active];
        self.session
            .borrow_mut()
            .set_bookmarked(&tab.url, tab.bookmarked);
    }

    // Native keyboard focus stays on the surviving shell element. The logical
    // chrome owner routes genuine text/key events once, even when an absolute
    // find bar or address child is rebuilt during reflow at another density.
    fn activate_chrome_input(&mut self, owner: ChromeInputOwner, window: &mut Window, cx: &mut Context<Self>) {
        self.chrome_input_owner = owner;
        if let Some(focus) = self.shell_focus.as_ref() { focus.focus(window, cx); }
        if std::env::var_os("AMAZING_NAVIGATION_TRACE").is_some() {
            eprintln!("AMAZING_CHROME_OWNER owner={owner:?}");
        }
    }

    fn focus_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chrome_input_owner = ChromeInputOwner::Document;
        if let Some(focus) = self.shell_focus.as_ref() {
            focus.focus(window, cx);
        }
        self.address_selected = false;
    }

    fn focus_address(&mut self, _: &FocusAddress, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_chrome_input(ChromeInputOwner::Address, window, cx);
        self.address_selected = true;
        self.downloads_open = false;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
        cx.notify();
    }

    fn new_browser_tab(&mut self, _: &NewBrowserTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab();
        self.activate_chrome_input(ChromeInputOwner::Address, window, cx);
        cx.notify();
    }

    fn step_tab(&mut self, delta: isize) {
        let open: Vec<usize> = self.tabs.iter().enumerate().filter_map(|(index, tab)| tab.open.then_some(index)).collect();
        let Some(position) = open.iter().position(|index| *index == self.active) else { return; };
        let next = (position as isize + delta).rem_euclid(open.len() as isize) as usize;
        self.select_tab(open[next]);
    }
    fn next_browser_tab(&mut self, _: &NextBrowserTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(1); self.focus_shell(window, cx); cx.notify();
    }
    fn previous_browser_tab(&mut self, _: &PreviousBrowserTab, window: &mut Window, cx: &mut Context<Self>) {
        self.step_tab(-1); self.focus_shell(window, cx); cx.notify();
    }

    fn close_browser_tab(
        &mut self,
        _: &CloseBrowserTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_tab(self.active);
        self.focus_shell(window, cx);
        cx.notify();
    }

    fn reload_page(&mut self, _: &ReloadPage, _: &mut Window, cx: &mut Context<Self>) {
        self.reload();
        cx.notify();
    }

    fn navigate_back(&mut self, _: &NavigateBack, _: &mut Window, cx: &mut Context<Self>) {
        self.go_back();
        cx.notify();
    }

    fn navigate_forward(&mut self, _: &NavigateForward, _: &mut Window, cx: &mut Context<Self>) {
        self.go_forward();
        cx.notify();
    }

    fn toggle_bookmark_action(
        &mut self,
        _: &ToggleBookmark,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_bookmark();
        cx.notify();
    }

    /// Find-in-page has nothing to search and nowhere to draw a match
    /// highlight without a rendered page: its bar and overlays live inside
    /// `page()`'s `.when_some(tab.viewport.as_ref(), ...)` branch, so a tab
    /// with no `EngineViewport` (media tabs, the pinned reference/"Specs"
    /// tabs) never shows them. Gate the action on the same condition so
    /// `find_open` can't get stuck true with no UI to ever show or close it.
    fn find_is_available(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.viewport.is_some() && tab.render_failure.is_none())
    }

    fn open_find(&mut self, _: &FindInPage, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find_is_available() {
            return;
        }
        self.find_open = true;
        self.chrome_input_owner = ChromeInputOwner::Find;
        self.find_query.clear();
        self.find_matches.clear();
        self.find_active = 0;
        self.downloads_open = false;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
        self.activate_chrome_input(ChromeInputOwner::Find, window, cx);
        cx.notify();
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chrome_input_owner = ChromeInputOwner::Document;
        self.find_open = false;
        self.find_query.clear();
        self.find_matches.clear();
        self.find_active = 0;
        self.focus_shell(window, cx);
        cx.notify();
    }

    /// Substring (case-insensitive) matches of `query` among the accessibility
    /// tree's rendered text nodes, in reading order. A pure function of
    /// `AxTree` so find-in-page's matching logic is unit-testable without a
    /// GPUI window/`Context`.
    fn find_matches_in_tree(tree: &amazing_a11y::AxTree, query: &str) -> Vec<amazing_a11y::AxRect> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let query_lower = query.to_lowercase();
        let mut matches: Vec<amazing_a11y::AxRect> = tree
            .nodes()
            .iter()
            .filter(|node| {
                matches!(
                    node.role,
                    amazing_a11y::Role::StaticText | amazing_a11y::Role::InlineTextBox
                )
            })
            .filter(|node| node.name.to_lowercase().contains(&query_lower))
            .filter_map(|node| node.bounds)
            .filter(|rect| !rect.is_empty())
            .collect();
        matches.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        matches
    }

    /// The cycled index `delta` steps from `active` among `len` matches
    /// (wrapping both directions); `0` when there are no matches. A pure
    /// function so next/prev cycling is unit-testable without `Context`.
    fn find_step(active: usize, len: usize, delta: isize) -> usize {
        if len == 0 {
            return 0;
        }
        (active as isize + delta).rem_euclid(len as isize) as usize
    }

    fn run_find_search(&mut self, cx: &mut Context<Self>) {
        if !self.document_interaction_available(self.active) {
            self.find_matches.clear(); self.find_active = 0; cx.notify(); return;
        }
        self.find_active = 0;
        self.find_matches = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.viewport.as_ref())
            .and_then(EngineViewport::accessibility_tree)
            .map(|tree| Self::find_matches_in_tree(tree, &self.find_query))
            .unwrap_or_default();
        self.scroll_to_find_active(cx);
        cx.notify();
    }

    fn find_go(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.find_matches.is_empty() {
            return;
        }
        self.find_active = Self::find_step(self.find_active, self.find_matches.len(), delta);
        self.scroll_to_find_active(cx);
        cx.notify();
    }

    /// Scrolls the active tab's viewport so the current match is roughly
    /// centered, jumping directly (no animation) so a capture taken right
    /// after next/prev shows the settled frame.
    fn scroll_to_find_active(&mut self, cx: &mut Context<Self>) {
        if !self.document_interaction_available(self.active) { return; }
        let Some(rect) = self.find_matches.get(self.find_active).copied() else {
            return;
        };
        let Some(viewport) = self.tabs[self.active].viewport.as_ref() else {
            return;
        };
        let Some(node) = viewport.root_scroll_id() else {
            return;
        };
        let target_y = (rect.y - ((viewport.metrics.css_height - rect.height) / 2.0).max(0.0)).max(0.0);
        self.scroll_active_viewport(
            ScrollInput::ScrollTo {
                node,
                target: (0.0, target_y),
                behavior: ScrollBehavior::Instant,
            },
            cx,
        );
    }

    fn on_find_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var_os("AMAZING_NAVIGATION_TRACE").is_some() {
            eprintln!("AMAZING_CHROME_KEY target=find key={:?} char={:?} control={} platform={}", event.keystroke.key, event.keystroke.key_char, event.keystroke.modifiers.control, event.keystroke.modifiers.platform);
        }
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "enter" => {
                if event.keystroke.modifiers.shift {
                    self.find_go(-1, cx);
                } else {
                    self.find_go(1, cx);
                }
            }
            "escape" => self.close_find(window, cx),
            "backspace" | "delete" => {
                self.find_query.pop();
                self.run_find_search(cx);
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                {
                    self.find_query.push_str(text);
                    self.run_find_search(cx);
                }
            }
        }
        cx.notify();
    }

    fn show_downloads(&mut self, _: &ShowDownloads, _: &mut Window, cx: &mut Context<Self>) {
        self.downloads_open = true;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
        cx.notify();
    }

    fn dismiss_chrome(&mut self, _: &DismissChrome, window: &mut Window, cx: &mut Context<Self>) {
        self.downloads_open = false;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
        if self.find_open {
            self.close_find(window, cx);
            return;
        }
        self.stop_navigation();
        self.focus_shell(window, cx);
        cx.notify();
    }

    fn on_address_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_open && self.chrome_input_owner == ChromeInputOwner::Find {
            self.on_find_key_down(event, window, cx); return;
        }
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "a" if event.keystroke.modifiers.control => {
                self.address_selected = true;
            }
            "c" if event.keystroke.modifiers.control => {
                if self.address_selected {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.address.clone()));
                }
            }
            "x" if event.keystroke.modifiers.control => {
                if self.address_selected {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.address.clone()));
                    self.address.clear();
                    self.address_selected = false;
                }
            }
            "v" if event.keystroke.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let text = text.replace(['\r', '\n'], " ");
                    if self.address_selected {
                        self.address = text;
                    } else {
                        self.address.push_str(&text);
                    }
                    self.address_selected = false;
                }
            }
            "enter" => {
                self.commit_navigation();
                self.focus_shell(window, cx);
            }
            "escape" => {
                self.address = self.committed_address.clone();
                self.focus_shell(window, cx);
            }
            "backspace" | "delete" => {
                if self.address_selected {
                    self.address.clear();
                    self.address_selected = false;
                } else {
                    self.address.pop();
                }
            }
            _ => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                {
                    if self.address_selected {
                        self.address.clear();
                        self.address_selected = false;
                    }
                    self.address.push_str(text);
                }
            }
        }
        cx.notify();
    }

    fn reconcile_page_input(&mut self, index: usize) {
        if !self.document_interaction_available(index) {
            if self.page_input.as_ref().is_some_and(|input| input.tab == index) { self.page_input = None; }
            return;
        }
        let Some(input) = self.page_input.as_mut().filter(|input| input.tab == index) else { return };
        let remapped = self.tabs.get(index).and_then(|tab| tab.viewport.as_ref())
            .and_then(|viewport| viewport.text_control_by_identity(input.node_identity));
        match remapped {
            Some((control, rect)) => { input.control = control; input.rect = rect; }
            None => self.page_input = None,
        }
    }

    fn record_measured_frame(&mut self, measured: ViewportMetrics, window_css: (f32, f32)) {
        let receipt = (measured, window_css);
        if self.last_viewport_receipt == Some(receipt) { return; }
        eprintln!("AMAZING_VIEWPORT pid={} css_width={} css_height={} device_scale={} raster_width={} raster_height={} window_css_width={} window_css_height={}",
            std::process::id(), measured.css_width, measured.css_height, measured.device_scale,
            measured.raster_width, measured.raster_height, window_css.0, window_css.1);
        self.last_viewport_receipt = Some(receipt);
    }

    fn apply_viewport_measurement(
        &mut self, index: usize, presented: &ViewportIdentity, measured: ViewportMetrics,
        window_css: (f32, f32), cx: &mut Context<Self>,
    ) {
        if self.active != index { return; }
        let Some(current) = self.tabs.get(index).and_then(|tab| tab.viewport.as_ref()) else { return };
        if !current.accepts_identity(presented) { return; }
        self.viewport_metrics = measured;
        if current.metrics == measured {
            if self.tabs[index].render_failure.is_none() { self.record_measured_frame(measured, window_css); }
            return;
        }
        if self.tabs[index].failed_metrics == Some(measured) { return; }
        let next = match self.page_runtimes.get_mut(index).and_then(Option::as_mut) {
            Some(runtime) => runtime.reflow(measured).map(Some),
            None => current.try_reflow_static(measured),
        };
        if self.publish_checked_viewport(index, next, measured) {
            self.record_measured_frame(measured, window_css);
            self.find_matches = self.tabs[index].viewport.as_ref()
                .and_then(EngineViewport::accessibility_tree)
                .map(|tree| Self::find_matches_in_tree(tree, &self.find_query)).unwrap_or_default();
            self.find_active = self.find_active.min(self.find_matches.len().saturating_sub(1));
        }
        cx.notify();
    }

    fn document_interaction_available(&self, index: usize) -> bool {
        self.tabs.get(index).is_some_and(|tab| tab.viewport.is_some() && tab.render_failure.is_none())
    }

    fn record_render_failure(&mut self, index: usize, error: String, metrics: ViewportMetrics) {
        self.last_viewport_receipt = None;
        self.tabs[index].render_failure = Some(error);
        self.tabs[index].failed_metrics = Some(metrics);
        self.reconcile_page_input(index);
        if self.active == index {
            self.find_matches.clear();
            self.find_active = 0;
        }
        eprintln!("AMAZING_VIEWPORT_FAILURE pid={} device_scale={}", std::process::id(), metrics.device_scale);
    }

    /// Only a checked replacement built from the current document can recover
    /// a failed frame. Retained movement and absent replacements cannot do so.
    fn publish_checked_viewport(&mut self, index: usize,
        result: Result<Option<EngineViewport>, String>, metrics: ViewportMetrics,
    ) -> bool {
        let result = result.and_then(|next| {
            if let (Some(next), Some(previous)) = (&next, &self.tabs[index].viewport) {
                next.restore_root_scroll(previous.root_scroll_offset())?;
            }
            Ok(next)
        });
        match result {
            Ok(Some(next)) => {
                self.tabs[index].viewport = Some(next);
                self.tabs[index].render_failure = None;
                self.tabs[index].failed_metrics = None;
                self.last_viewport_receipt = None;
                self.reconcile_page_input(index);
                true
            }
            Ok(None) => false,
            Err(error) => { self.record_render_failure(index, error, metrics); false }
        }
    }

    fn record_cached_movement(&mut self, index: usize, result: Result<bool, String>) -> bool {
        match result {
            Ok(moved) => moved,
            Err(error) => {
                let metrics = self.tabs[index].viewport.as_ref().map(|view| view.metrics)
                    .unwrap_or(self.viewport_metrics);
                if self.tabs[index].render_failure.is_none() {
                    self.record_render_failure(index, error, metrics);
                }
                false
            }
        }
    }

    fn scroll_active_viewport(&mut self, input: ScrollInput, cx: &mut Context<Self>) -> bool {
        let pointer_input = matches!(input,
            ScrollInput::Wheel { .. } | ScrollInput::Trackpad { .. } | ScrollInput::TouchPan { .. });
        let blocking = pointer_input && self.page_runtimes.get(self.active)
            .and_then(Option::as_ref)
            .is_some_and(PageRuntime::has_blocking_scroll_listener);
        let Some(viewport) = self.tabs[self.active].viewport.as_ref() else { return false };
        let result = viewport.scroll_input(input, blocking);
        if !self.record_cached_movement(self.active, result) { cx.notify(); return false; }
        if self.document_interaction_available(self.active) { self.load_scrolled_lazy_images(); }
        cx.notify();
        true
    }

    /// After the compositor moves the document, lazy images whose retained
    /// layout box entered the viewport are fetched and repainted.
    fn load_scrolled_lazy_images(&mut self) {
        let Some(viewport) = self.tabs[self.active].viewport.as_mut() else { return };
        let result = match self.page_runtimes.get(self.active).and_then(Option::as_ref) {
            Some(runtime) => runtime.load_deferred_lazy_images(viewport),
            None => viewport.load_static_deferred_images(),
        };
        if let Err(error) = result {
            self.record_render_failure(self.active, error, self.viewport_metrics);
        }
    }

    fn on_shell_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A dynamic chrome element can be rebuilt while its request/page is
        // reflowing. Preserve the explicit input owner, rather than dropping
        // typed text when GPUI routes that event through the surviving root.
        if self.find_open && self.chrome_input_owner == ChromeInputOwner::Find {
            self.on_find_key_down(event, window, cx); return;
        }
        if self.chrome_input_owner == ChromeInputOwner::Address {
            self.on_address_key_down(event, window, cx); return;
        }
        self.reconcile_page_input(self.active);
        if self.page_input.is_none() {
            let key = match event.keystroke.key.as_str() {
                "up" => Some(ScrollKey::ArrowUp),
                "down" => Some(ScrollKey::ArrowDown),
                "left" => Some(ScrollKey::ArrowLeft),
                "right" => Some(ScrollKey::ArrowRight),
                "pageup" => Some(ScrollKey::PageUp),
                "pagedown" | "space" => Some(ScrollKey::PageDown),
                "home" => Some(ScrollKey::Home),
                "end" => Some(ScrollKey::End),
                _ => None,
            };
            if let Some((node, key)) = key.and_then(|key|
                self.tabs[self.active].viewport.as_ref()?.root_scroll_id().map(|node| (node, key))) {
                if self.scroll_active_viewport(ScrollInput::Keyboard { node, key }, cx) {
                    cx.stop_propagation();
                }
            }
            return;
        }
        let Some(input) = self.page_input.as_mut() else {
            return;
        };
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "escape" => self.page_input = None,
            "a" if event.keystroke.modifiers.control => input.selected = true,
            "v" if event.keystroke.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    let text = text.replace(['\r', '\n'], " ");
                    if input.selected {
                        input.value = text;
                    } else {
                        input.value.push_str(&text);
                    }
                    input.selected = false;
                }
            },
            "backspace" | "delete" => {
                if input.selected {
                    input.value.clear();
                    input.selected = false;
                } else {
                    input.value.pop();
                }
            },
            "enter" => {
                let input = input.clone();
                let request = self.tabs[input.tab]
                    .viewport
                    .as_ref()
                    .and_then(|viewport| viewport.form_request(input.control, Some(&input.value)));
                self.page_input = None;
                if let Some(request) = request {
                    self.submit_form(request);
                }
            },
            _ => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                {
                    if input.selected {
                        input.value.clear();
                    }
                    input.value.push_str(text);
                    input.selected = false;
                }
            },
        }
        cx.notify();
    }

    fn commit_navigation(&mut self) {
        let Some(address) = normalize_address(&self.address) else {
            self.address = self.committed_address.clone();
            return;
        };
        self.commit_request(address, "GET", Vec::new());
    }

    fn commit_request(&mut self, address: String, method: &str, body: Vec<u8>) {
        self.start_navigation(address, method, body, HistoryCommit::Push);
    }

    fn start_navigation(&mut self, address: String, method: &str, body: Vec<u8>, history: HistoryCommit) {
        let index = self.active;
        self.pending_navigation[index] = None; // Drop cancels every actual outstanding transport.
        self.page_input = None;
        self.address = address.clone();
        self.address_selected = false;
        let id = self.next_navigation_id;
        self.next_navigation_id = self.next_navigation_id.checked_add(1).expect("navigation identity exhausted");
        if address == "amazing://status" || address == "amazing://newtab" {
            let page = EngineViewport::load_request_with_metrics(&address, method, body,
                self.session.clone(), self.viewport_metrics);
            self.finish_navigation(index, id, &address, history, page);
        } else {
            self.pending_navigation[index] = Some(PendingNavigation::new(id, address, method, body, history, &self.session));
        }
    }

    fn stop_navigation(&mut self) {
        if let Some(pending) = self.pending_navigation[self.active].take() {
            let committed = pending.committed;
            drop(pending); // Freeze cancellation before asking the same page to expose failed-font fallback.
            if committed { self.refresh_stopped_font_display(self.active); }
            self.address = self.tabs[self.active].url.clone();
            self.address_selected = false;
        }
    }

    fn refresh_stopped_font_display(&mut self, index: usize) {
        let tab = &mut self.tabs[index];
        let Some(viewport) = tab.viewport.take() else { return };
        let mut page = engine_viewport::LoadedPage { viewport, final_address: tab.url.clone(),
            title: Some(tab.title.clone()), favicon: tab.favicon.take(), runtime: self.page_runtimes[index].take() };
        let result = if let Some(replay) = page.acquisition_replay() {
            navigation_acquisition::with_replay(replay, || EngineViewport::refresh_initial_page(&mut page, self.viewport_metrics))
        } else { EngineViewport::refresh_initial_page(&mut page, self.viewport_metrics) };
        let tab = &mut self.tabs[index]; tab.viewport = Some(page.viewport); tab.favicon = page.favicon;
        if let Some(title) = page.title { tab.title = title; } self.page_runtimes[index] = page.runtime;
        if let Err(error) = result { self.record_render_failure(index, error, self.viewport_metrics); }
    }

    fn poll_navigation(&mut self) {
        for index in 0..self.tabs.len() {
            if !self.tabs[index].open { continue; }
            let Some(mut pending) = self.pending_navigation[index].take() else { continue };
            let already_committed = pending.committed;
            if already_committed {
                let tab = &mut self.tabs[index];
                pending.lend_visible_page(engine_viewport::LoadedPage {
                    viewport: tab.viewport.take().expect("published loading page"),
                    final_address: tab.url.clone(), title: Some(tab.title.clone()),
                    favicon: tab.favicon.take(), runtime: self.page_runtimes[index].take(),
                });
            }
            let result = pending.poll(&self.session, self.viewport_metrics);
            let terminal = result.is_some() && (!pending.committed || already_committed);
            if already_committed {
                let page = match result {
                    Some(Ok(page)) => Some(page),
                    Some(Err(error)) => {
                        self.record_render_failure(index, error, self.viewport_metrics);
                        pending.take_visible_page()
                    },
                    None => pending.take_visible_page(),
                };
                if let Some(page) = page {
                    let tab = &mut self.tabs[index];
                    tab.viewport = Some(page.viewport); tab.favicon = page.favicon;
                    if let Some(title) = page.title { tab.title = title; }
                    self.page_runtimes[index] = page.runtime;
                }
            } else if let Some(result) = result {
                self.finish_navigation(index, pending.id, &pending.address, pending.history, result);
            }
            if !terminal { self.pending_navigation[index] = Some(pending); }
        }
    }

    fn trace_navigation_state(&mut self) {
        if std::env::var_os("AMAZING_NAVIGATION_TRACE").is_none() { return; }
        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let pending = self.pending_navigation[index].as_ref();
            format!("{{\"index\":{index},\"open\":{},\"url\":{:?},\"title\":{:?},\"pending\":{},\"request\":{},\"history_index\":{},\"history_len\":{}}}",
                tab.open, tab.url, tab.title, pending.is_some(), pending.map_or(0, |pending| pending.id), tab.history_index, tab.history.len())
        }).collect::<Vec<_>>().join(",");
        let fonts = self.page_runtimes[self.active].as_ref().map(PageRuntime::font_display_trace)
            .or_else(|| self.tabs[self.active].viewport.as_ref().map(EngineViewport::font_display_trace)).unwrap_or_else(|| "[]".into());
        let decoded_images = self.tabs[self.active].viewport.as_ref().map_or(0, EngineViewport::decoded_image_count);
        let state = format!("{{\"images\":{decoded_images},\"active\":{},\"address\":{:?},\"committed\":{:?},\"find_open\":{},\"find_query\":{:?},\"fonts\":{fonts},\"tabs\":[{tabs}]}}", self.active, self.address, self.committed_address, self.find_open, self.find_query);
        if self.last_navigation_trace != state {
            eprintln!("AMAZING_NAVIGATION_STATE {state}"); self.last_navigation_trace = state;
        }
    }

    fn finish_navigation(&mut self, index: usize, id: u64, requested: &str, history: HistoryCommit,
        result: Result<engine_viewport::LoadedPage, String>) {
        if !self.tabs[index].open { return; }
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                match EngineViewport::error(requested, &error).try_reflow_static(self.viewport_metrics) {
                    Ok(Some(viewport)) => engine_viewport::LoadedPage { viewport,
                        final_address: requested.into(), title: None, favicon: None, runtime: None },
                    result => {
                        let reason = result.err().unwrap_or_else(|| "missing error document".into());
                        self.record_render_failure(index, format!("{error}; error page: {reason}"), self.viewport_metrics);
                        return;
                    },
                }
            },
        };
        let final_address = page.final_address;
        let tab = &mut self.tabs[index];
        match history {
            HistoryCommit::Push => {
                if tab.history[tab.history_index] != final_address {
                    tab.history.truncate(tab.history_index + 1);
                    tab.history.push(final_address.clone());
                    tab.history_index = tab.history.len() - 1;
                }
            },
            HistoryCommit::Traverse(target) => {
                if target >= tab.history.len() { return; }
                tab.history_index = target;
                tab.history[target] = final_address.clone();
            },
            HistoryCommit::Reload => tab.history[tab.history_index] = final_address.clone(),
        }
        tab.url = final_address.clone();
        tab.kind = TabKind::Engine;
        tab.viewport = Some(page.viewport);
        tab.render_failure = None;
        tab.failed_metrics = None;
        tab.title = page.title.unwrap_or_else(|| navigation_title(&final_address));
        tab.mono = site_initial(&final_address).to_string();
        tab.favicon = page.favicon;
        tab.bookmarked = self.session.borrow().is_bookmarked(&final_address);
        self.page_runtimes[index] = page.runtime;
        if let Some(runtime) = self.page_runtimes[index].as_mut() { runtime.set_history_length(tab.history.len()); }
        self.session.borrow_mut().record_history(&final_address);
        if self.active == index {
            self.committed_address = final_address.clone();
            if self.address == requested { self.address = final_address.clone(); }
            self.find_matches.clear(); self.find_active = 0;
        }
        if std::env::var_os("AMAZING_NAVIGATION_TRACE").is_some() {
            eprintln!("AMAZING_NAVIGATION_COMMIT tab={index} request={id} url={final_address}");
        }
    }

    fn submit_form(&mut self, request: FormRequest) {
        self.address = request.target.clone();
        self.commit_request(request.target, &request.method, request.body);
    }

    fn load_navigation(&mut self, address: &str) {
        self.start_navigation(address.into(), "GET", Vec::new(), HistoryCommit::Reload);
    }

    fn traversal_index(&self) -> usize {
        match self.pending_navigation[self.active].as_ref().map(|pending| pending.history) {
            Some(HistoryCommit::Traverse(target)) => target,
            _ => self.tabs[self.active].history_index,
        }
    }

    fn go_back(&mut self) {
        let target = self.traversal_index();
        if target == 0 { return; }
        let target = target - 1;
        let address = self.tabs[self.active].history[target].clone();
        self.start_navigation(address, "GET", Vec::new(), HistoryCommit::Traverse(target));
    }

    fn go_forward(&mut self) {
        let target = self.traversal_index() + 1;
        if target >= self.tabs[self.active].history.len() { return; }
        let address = self.tabs[self.active].history[target].clone();
        self.start_navigation(address, "GET", Vec::new(), HistoryCommit::Traverse(target));
    }

    fn can_go_back(&self) -> bool {
        self.traversal_index() > 0
    }

    fn can_go_forward(&self) -> bool {
        let tab = &self.tabs[self.active];
        self.traversal_index() + 1 < tab.history.len()
    }

    fn reload(&mut self) {
        let (address, method, body, history) = self.pending_navigation[self.active].as_ref()
            .map_or_else(|| (self.committed_address.clone(), "GET".to_owned(), Vec::new(), HistoryCommit::Reload),
                |pending| {
                    let (method, body) = pending.request_data();
                    (pending.address.clone(), method, body, pending.history)
                });
        self.start_navigation(address, &method, body, history);
    }

    fn apply_runtime_update(&mut self, index: usize, update: RuntimeUpdate) -> RuntimeDirective {
        let directive = RuntimeDirective {
            default_prevented: update.default_prevented,
            navigation: update.navigation,
            history_delta: update.history_delta,
            history_update: update.history_update,
        };
        let result = match update.render_failure {
            Some(error) => Err(error),
            None => Ok(update.viewport),
        };
        self.publish_checked_viewport(index, result, update.metrics);
        let tab = &mut self.tabs[index];
        if let Some(title) = update.title {
            tab.title = title;
        }
        tab.favicon = update.favicon;
        self.reconcile_page_input(index);
        directive
    }

    fn dispatch_runtime_click(&mut self, node: u32) -> RuntimeDirective {
        let index = self.active;
        if !self.document_interaction_available(index) {
            return RuntimeDirective { default_prevented: true, navigation: None,
                history_delta: None, history_update: None };
        }
        let update = self.page_runtimes[index]
            .as_mut()
            .map(|runtime| runtime.dispatch_click(node));
        update.map_or(
            RuntimeDirective {
                default_prevented: false,
                navigation: None,
                history_delta: None,
                history_update: None,
            },
            |update| self.apply_runtime_update(index, update),
        )
    }

    fn apply_runtime_directive(&mut self, directive: RuntimeDirective) -> bool {
        if let Some((url, replace)) = directive.history_update {
            let target = if url.is_empty() {
                self.committed_address.clone()
            } else if let Ok(base) = amazing_net::Url::parse(&self.committed_address) {
                amazing_net::resolve(&url, &base)
                    .map(|url| url.to_string())
                    .unwrap_or(url)
            } else {
                url
            };
            let tab = &mut self.tabs[self.active];
            if replace {
                tab.history[tab.history_index] = target.clone();
            } else {
                tab.history.truncate(tab.history_index + 1);
                tab.history.push(target.clone());
                tab.history_index = tab.history.len() - 1;
            }
            tab.url = target.clone();
            self.address = target.clone();
            self.committed_address = target.clone();
            self.session.borrow_mut().record_history(&target);
            if let Some(runtime) = self.page_runtimes[self.active].as_mut() {
                runtime.set_document_url(target);
                runtime.set_history_length(tab.history.len());
            }
        }
        if let Some(delta) = directive.history_delta {
            for _ in 0..delta.unsigned_abs().min(32) {
                if delta < 0 {
                    self.go_back();
                } else if delta > 0 {
                    self.go_forward();
                }
            }
        }
        if let Some(navigation) = directive.navigation {
            self.address = navigation;
            self.commit_navigation();
        }
        directive.default_prevented
    }

    fn pump_active_runtime(&mut self) {
        let index = self.active;
        let update = self.page_runtimes[index]
            .as_mut()
            .and_then(PageRuntime::pump);
        if let Some(update) = update {
            let directive = self.apply_runtime_update(index, update);
            self.apply_runtime_directive(directive);
        }
    }

    fn update_video_clock(&mut self, now_ms: u64) -> PlaybackUpdate {
        let update = self.video_clock.update(now_ms);
        self.video_frame_index = update.frame_index;
        if self.secure_video_complete {
            let audio_ms = self
                .audio_device_clock
                .as_ref()
                .map_or(now_ms, AudioDeviceClock::position_ms);
            let audio_state = if self
                .audio_device_clock
                .as_ref()
                .is_some_and(AudioDeviceClock::is_active)
            {
                "playing"
            } else {
                "done"
            };
            self.secure_video_status = format!(
                "SECURE COMPLETE  ·  AUDIO 48k {} ms {}  ·  {}f  ·  loss {}  ·  IDR {}  ·  late {}  ·  q{}",
                audio_ms,
                audio_state,
                self.video_frames.len(),
                self.secure_video_recoveries,
                self.secure_video_recoveries,
                update.dropped_total,
                FRAME_EVENT_CAPACITY
            );
        }
        update
    }

    fn poll_secure_video(&mut self, now_ms: u64) {
        let mut events = Vec::new();
        if let Some(receiver) = self.secure_video_receiver.as_ref() {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                SecureVideoEvent::Audio(audio) => {
                    match AudioDeviceClock::start(audio.sample_rate, audio.channels, audio.samples)
                    {
                        Ok(clock) => {
                            self.secure_video_status = format!(
                                "AUDIO DEVICE  ·  {} Hz  ·  {} ms queued",
                                audio.sample_rate,
                                clock.total_ms()
                            );
                            self.audio_device_clock = Some(clock);
                        }
                        Err(error) => {
                            self.secure_video_status = format!("SECURE AUDIO ERROR  ·  {error}");
                        }
                    }
                }
                SecureVideoEvent::Frame(frame) => {
                    if let Ok(viewport) =
                        EngineViewport::from_rgba(frame.width, frame.height, frame.rgba)
                    {
                        if !self.secure_video_active {
                            self.video_frames.clear();
                            self.pending_video_presentation_ms.clear();
                            self.video_frame_index = 0;
                            self.secure_video_active = true;
                            self.secure_video_complete = false;
                            self.secure_video_recoveries = 0;
                            self.video_clock
                                .begin_stream(if self.audio_device_clock.is_some() {
                                    0
                                } else {
                                    now_ms
                                });
                        } else if let Err(error) = self
                            .video_clock
                            .append_presentation(frame.presentation_ms as u64)
                        {
                            self.secure_video_status = format!("SECURE VIDEO ERROR  ·  {error}");
                            continue;
                        }
                        if presentation_gap_exceeds_cadence(
                            &self.pending_video_presentation_ms,
                            frame.presentation_ms as u64,
                        ) {
                            self.secure_video_recoveries =
                                self.secure_video_recoveries.saturating_add(1);
                        }
                        self.video_frames.push(viewport);
                        self.pending_video_presentation_ms
                            .push(frame.presentation_ms as u64);
                        self.secure_video_status = format!(
                            "LIVE SRTP → JITTER → H.264 → RGBA  ·  frame {}  ·  A/V {} ms  ·  RTP {}  ·  rec {}",
                            frame.index + 1,
                            frame.presentation_ms,
                            frame.timestamp,
                            self.secure_video_recoveries
                        );
                    }
                }
                SecureVideoEvent::Complete => {
                    if self.pending_video_presentation_ms.len() != self.video_frames.len() {
                        self.secure_video_status =
                            "SECURE VIDEO ERROR  ·  frame timeline length mismatch".into();
                    } else {
                        self.video_clock.seal_stream();
                        self.secure_video_complete = true;
                        let audio_ms = self
                            .audio_device_clock
                            .as_ref()
                            .map_or(now_ms, AudioDeviceClock::position_ms);
                        self.secure_video_status = format!(
                            "SECURE COMPLETE  ·  AUDIO 48k {} ms  ·  {}f  ·  loss {}  ·  IDR {}  ·  late 0  ·  q{}",
                            audio_ms,
                            self.video_frames.len(),
                            self.secure_video_recoveries,
                            self.secure_video_recoveries,
                            FRAME_EVENT_CAPACITY
                        );
                    }
                }
                SecureVideoEvent::Error(error) => {
                    self.secure_video_status = format!("SECURE VIDEO ERROR  ·  {error}");
                }
            }
        }
    }

    fn toggle_engine_group(&mut self) {
        self.engine_collapsed = !self.engine_collapsed;
    }

    fn toggle_specs_group(&mut self) {
        self.specs_collapsed = !self.specs_collapsed;
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
        self.downloads_open = false;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
    }

    fn toggle_downloads(&mut self) {
        self.downloads_open = !self.downloads_open;
        self.menu_open = false;
        self.site_info_open = false;
        self.profile_open = false;
    }

    fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        self.downloads_open = false;
        self.site_info_open = false;
        self.profile_open = false;
    }

    fn toggle_site_info(&mut self) {
        self.site_info_open = !self.site_info_open;
        self.downloads_open = false;
        self.menu_open = false;
        self.profile_open = false;
    }

    fn toggle_profile(&mut self) {
        self.profile_open = !self.profile_open;
        self.downloads_open = false;
        self.menu_open = false;
        self.site_info_open = false;
    }

    fn icon_button(label: &'static str, enabled: bool) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(30.0))
            .rounded_md()
            .text_color(rgb(if enabled { ICON } else { DIM }))
            .text_size(px(16.0))
            .when(enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(SURFACE_2_HOVER)))
            })
            .child(label)
    }

    fn tab(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tab = self.tabs[index].clone();
        let active = self.active == index;
        let loading = self.pending_navigation[index].is_some();
        let favicon = if let Some(image) = tab.favicon.clone() {
            div()
                .size(px(16.0))
                .overflow_hidden()
                .rounded_sm()
                .child(img(image).size_full())
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.0))
                .rounded_sm()
                .bg(rgb(tab.hue))
                .text_color(rgb(0xffffff))
                .text_size(px(9.0))
                .font_weight(gpui::FontWeight::BOLD)
                .child(tab.mono.clone())
                .into_any_element()
        };
        div()
            .id(SharedString::from(format!("tab-{index}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(34.0))
            .w(px(if active { 228.0 } else { 176.0 }))
            .px_2()
            .rounded_t_lg()
            .bg(rgb(if active { SURFACE } else { STRIP }))
            .text_color(rgb(if active { TEXT } else { DIM }))
            .shadow_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(if active { SURFACE } else { STRIP_HOVER })))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_tab(index);
                cx.notify();
            }))
            .child(favicon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(12.0))
                    .font_weight(if active {
                        gpui::FontWeight::MEDIUM
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .child(if loading { format!("Loading · {}", tab.title) } else { tab.title }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("close-tab-{index}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.0))
                    .rounded_full()
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(SURFACE_2_HOVER)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_tab(index);
                        cx.notify();
                    }))
                    .child("×"),
            )
            .into_any_element()
    }

    fn group_chip(
        title: &'static str,
        count: usize,
        color: u32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("group-{title}")))
            .flex()
            .items_center()
            .gap_1()
            .h(px(22.0))
            .px_2()
            .rounded_md()
            .bg(rgb(color))
            .text_color(rgb(0xffffff))
            .text_size(px(11.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_specs_group();
                cx.notify();
            }))
            .child(title)
            .child(
                div()
                    .id("sidebar-search")
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(16.0))
                    .h(px(16.0))
                    .px_1()
                    .rounded_full()
                    .bg(gpui::rgba(0xffffff42))
                    .text_size(px(10.0))
                    .child(count.to_string()),
            )
    }

    fn navigation_toolbar(&self, cx: &mut Context<Self>, visible: bool) -> gpui::AnyElement {
        let loading = self.pending_navigation[self.active].is_some();
        let bookmarked = self.tabs[self.active].bookmarked;
        let back_enabled = self.can_go_back();
        let forward_enabled = self.can_go_forward();
        let site_glyph = if self.committed_address.starts_with("amazing://") {
            "A"
        } else if self.committed_address.starts_with("https://") {
            "●"
        } else {
            "ⓘ"
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .h(px(44.0))
            .px_2()
            .bg(rgb(SURFACE))
            .children([
                div()
                    .id("history-back")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_back();
                        cx.notify();
                    }))
                    .child(Self::icon_button("←", back_enabled))
                    .into_any_element(),
                div()
                    .id("history-forward")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_forward();
                        cx.notify();
                    }))
                    .child(Self::icon_button("→", forward_enabled))
                    .into_any_element(),
                div()
                    .id("history-reload")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.pending_navigation[this.active].is_some() { this.stop_navigation(); }
                        else { this.reload(); }
                        cx.notify();
                    }))
                    .child(Self::icon_button(if loading { "×" } else { "↻" }, true))
                    .into_any_element(),
            ])
            .child(
                div()
                    .id("address-bar")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .h(px(32.0))
                    .mx_2()
                    .px_3()
                    .rounded_full()
                    .bg(rgb(SURFACE_2))
                    .text_size(px(13.0))
                    .text_color(rgb(TEXT))
                    .cursor(CursorStyle::IBeam)
                    .when_some(self.address_focus.as_ref(), |address, focus| {
                        address.track_focus(focus)
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.activate_chrome_input(ChromeInputOwner::Address, window, cx);
                        this.address_selected = true;
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(Self::on_address_key_down))
                    .child(site_glyph)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .when(self.address_selected, |text| {
                                text.bg(rgb(0xc9ddf5)).rounded_sm().px_1()
                            })
                            .child(self.address.clone()),
                    )
                    .child(
                        div()
                            .id("bookmark-current")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_2_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_bookmark();
                                cx.notify();
                            }))
                            .child(if bookmarked { "★" } else { "☆" }),
                    )
                    .child(
                        div()
                            .id("site-controls")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(24.0))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_2_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_site_info();
                                cx.notify();
                            }))
                            .child("◈"),
                    ),
            )
            .child(
                div()
                    .id("sidebar-toggle")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar();
                        cx.notify();
                    }))
                    .child(Self::icon_button("◫", true)),
            )
            .child(
                div()
                    .id("downloads-toggle")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_downloads();
                        cx.notify();
                    }))
                    .child(Self::icon_button("⇩", true)),
            )
            .child(
                div()
                    .id("profile-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0))
                    .mx_2()
                    .rounded_full()
                    .bg(rgb(ACCENT))
                    .text_color(rgb(0xffffff))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x934522)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_profile();
                        cx.notify();
                    }))
                    .child("J"),
            )
            .child(
                div()
                    .id("browser-menu-toggle")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_menu();
                        cx.notify();
                    }))
                    .child(Self::icon_button("⋮", true)),
            )
            .when(!visible, |toolbar| toolbar.hidden())
            .into_any_element()
    }

    fn sidebar_tab(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.active == index;
        let tab = self.tabs[index].clone();
        let favicon = if let Some(image) = tab.favicon.clone() {
            div()
                .size(px(15.0))
                .overflow_hidden()
                .rounded_sm()
                .child(img(image).size_full())
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(15.0))
                .rounded_sm()
                .bg(rgb(tab.hue))
                .text_color(rgb(0xffffff))
                .text_size(px(8.0))
                .child(tab.mono.clone())
                .into_any_element()
        };
        div()
            .id(SharedString::from(format!("side-tab-{index}")))
            .flex()
            .items_center()
            .gap_2()
            .h(px(30.0))
            .px_2()
            .rounded_md()
            .bg(rgb(if active { SURFACE } else { STRIP }))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_tab(index);
                cx.notify();
            }))
            .child(favicon)
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(rgb(TEXT))
                    .child(tab.title),
            )
            .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .h_full()
            .p_2()
            .gap_1()
            .bg(rgb(STRIP))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .id("sidebar-search")
                    .flex()
                    .items_center()
                    .h(px(30.0))
                    .px_2()
                    .rounded_md()
                    .bg(rgb(SURFACE_2))
                    .text_size(px(11.0))
                    .text_color(rgb(DIM))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.activate_chrome_input(ChromeInputOwner::Address, window, cx);
                        this.address_selected = true;
                        cx.notify();
                    }))
                    .child("⌕  Search or type URL"),
            )
            .child(
                div()
                    .id("sidebar-new-tab")
                    .flex()
                    .items_center()
                    .h(px(28.0))
                    .px_2()
                    .rounded_md()
                    .text_size(px(11.0))
                    .text_color(rgb(TEXT))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(STRIP_HOVER)))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_tab();
                        this.activate_chrome_input(ChromeInputOwner::Address, window, cx);
                        cx.notify();
                    }))
                    .child("＋  New tab"),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .mt_1()
                    .text_size(px(11.0))
                    .text_color(rgb(DIM))
                    .child("PINNED"),
            )
            .child(
                div()
                    .id("sidebar-pinned")
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(30.0))
                    .px_2()
                    .rounded_md()
                    .bg(rgb(SURFACE))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_bookmark(0);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(15.0))
                            .rounded_sm()
                            .bg(rgb(ACCENT))
                            .text_color(rgb(0xffffff))
                            .text_size(px(8.0))
                            .child("A"),
                    )
                    .child("Amazing — start"),
            )
            .child(
                div()
                    .id("sidebar-engine-group")
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt_2()
                    .px_2()
                    .py_1()
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(ACCENT))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_engine_group();
                        cx.notify();
                    }))
                    .child("ENGINE")
                    .child("3"),
            )
            .when(!self.engine_collapsed, |sidebar| {
                sidebar.children(
                    (0..self.tabs.len())
                        .filter(|index| {
                            self.tabs[*index].open && self.tabs[*index].group == TabGroup::Engine
                        })
                        .map(|index| self.sidebar_tab(index, cx)),
                )
            })
            .child(
                div()
                    .id("sidebar-specs-group")
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt_2()
                    .px_2()
                    .py_1()
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x3a6ea8))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_specs_group();
                        cx.notify();
                    }))
                    .child("SPECS")
                    .child("3"),
            )
            .when(!self.specs_collapsed, |sidebar| {
                sidebar.children(
                    (0..self.tabs.len())
                        .filter(|index| {
                            self.tabs[*index].open && self.tabs[*index].group == TabGroup::Specs
                        })
                        .map(|index| self.sidebar_tab(index, cx)),
                )
            })
            .children(
                (0..self.tabs.len())
                    .filter(|index| {
                        self.tabs[*index].open && self.tabs[*index].group == TabGroup::Other
                    })
                    .map(|index| self.sidebar_tab(index, cx)),
            )
    }

    fn downloads_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let downloads = self.session.borrow().downloads().to_vec();
        div()
            .absolute()
            .top(px(82.0))
            .right(px(18.0))
            .w(px(340.0))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Downloads"),
                    )
                    .child(
                        div()
                            .id("downloads-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_2)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.downloads_open = false;
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            )
            .when(downloads.is_empty(), |panel| {
                panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .h(px(150.0))
                        .gap_2()
                        .text_color(rgb(DIM))
                        .child(div().text_size(px(28.0)).child("⇩"))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(TEXT))
                                .child("No downloads yet"),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .child("Downloaded files will appear here."),
                        ),
                )
            })
            .when(!downloads.is_empty(), |panel| {
                panel.child(
                    div().flex().flex_col().gap_1().mt_2().children(
                        downloads.into_iter().rev().take(6).map(|download| {
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .p_2()
                                .rounded_md()
                                .hover(|style| style.bg(rgb(SURFACE_2)))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(download.name),
                                )
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_size(px(10.5))
                                        .text_color(rgb(DIM))
                                        .child(download.path),
                                )
                        }),
                    ),
                )
            })
            .when(!self.downloads_open, |panel| panel.hidden())
    }

    fn site_info_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (connection, color) = if self.committed_address.starts_with("amazing://") {
            ("Internal Amazing page", ACCENT)
        } else if self.committed_address.starts_with("https://") {
            ("HTTPS address", 0x3f7d4e)
        } else if self.committed_address.starts_with("http://") {
            ("Not secure", 0xb3261e)
        } else {
            ("Page information", DIM)
        };
        div()
            .absolute()
            .top(px(80.0))
            .right(px(145.0))
            .w(px(360.0))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(div().text_color(rgb(color)).child("●"))
                            .child(connection),
                    )
                    .child(
                        div()
                            .id("site-info-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_2)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.site_info_open = false;
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .mt_2()
                    .px_2()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(SURFACE_2))
                    .text_size(px(12.0))
                    .text_color(rgb(DIM))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(self.committed_address.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt_3()
                    .px_1()
                    .text_size(px(12.0))
                    .child("Renderer")
                    .child(div().text_color(rgb(DIM)).child("Amazing engine")),
            )
            .when(!self.site_info_open, |panel| panel.hidden())
    }

    fn profile_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top(px(80.0))
            .right(px(48.0))
            .w(px(300.0))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(42.0))
                            .rounded_full()
                            .bg(rgb(ACCENT))
                            .text_color(rgb(SURFACE))
                            .text_size(px(16.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("J"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("jMac"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(DIM))
                                    .child("Local profile"),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("profile-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_2)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.profile_open = false;
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(SURFACE_2))
                    .text_size(px(12.0))
                    .line_height(gpui::relative(1.4))
                    .text_color(rgb(DIM))
                    .child("Browsing data stays on this device. Sync is off."),
            )
            .when(!self.profile_open, |panel| panel.hidden())
    }

    fn browser_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bookmark_label = if self.tabs[self.active].bookmarked {
            "Remove bookmark"
        } else {
            "Bookmark this tab"
        };
        let sidebar_label = if self.sidebar {
            "Hide side panel"
        } else {
            "Show side panel"
        };
        let item = |label: &'static str, shortcut: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(32.0))
                .px_3()
                .rounded_md()
                .text_size(px(12.5))
                .text_color(rgb(TEXT))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(SURFACE_2)))
                .child(label)
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(DIM))
                        .child(shortcut),
                )
        };
        div()
            .absolute()
            .top(px(80.0))
            .right(px(10.0))
            .w(px(270.0))
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE))
            .shadow_lg()
            .child(
                div()
                    .id("menu-new-tab")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_tab();
                        this.menu_open = false;
                        this.activate_chrome_input(ChromeInputOwner::Address, window, cx);
                        cx.notify();
                    }))
                    .child(item("New tab", "Ctrl+T")),
            )
            .child(
                div()
                    .id("menu-history-back")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_back();
                        this.menu_open = false;
                        cx.notify();
                    }))
                    .child(item("Back", "Alt+Left")),
            )
            .child(
                div()
                    .id("menu-history-forward")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_forward();
                        this.menu_open = false;
                        cx.notify();
                    }))
                    .child(item("Forward", "Alt+Right")),
            )
            .child(
                div()
                    .id("menu-reload")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reload();
                        this.menu_open = false;
                        cx.notify();
                    }))
                    .child(item("Reload", "Ctrl+R")),
            )
            .child(
                div()
                    .id("menu-downloads")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu_open = false;
                        this.downloads_open = true;
                        cx.notify();
                    }))
                    .child(item("Downloads", "Ctrl+J")),
            )
            .child(div().h(px(1.0)).my_1().bg(rgb(RULE)))
            .child(
                div()
                    .id("menu-bookmark")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_bookmark();
                        this.menu_open = false;
                        cx.notify();
                    }))
                    .child(item(bookmark_label, "Ctrl+D")),
            )
            .child(
                div()
                    .id("menu-sidebar")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar();
                        this.menu_open = false;
                        cx.notify();
                    }))
                    .child(item(sidebar_label, "")),
            )
            .when(!self.menu_open, |menu| menu.hidden())
    }

    fn page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = &self.tabs[self.active];
        let entity = cx.entity();
        let page_input = self
            .page_input
            .clone()
            .filter(|input| input.tab == self.active);
        let video_viewport = &self.video_frames[self.video_frame_index];
        let video_badge = if self.secure_video_requested {
            self.secure_video_status.clone()
        } else {
            format!(
                "H.264 → I420 → RGBA  ·  {} × {}",
                video_viewport.width, video_viewport.height
            )
        };
        let page_url = tab.url.clone();
        let measured_tab = self.active;
        div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .when_some(tab.viewport.as_ref(), |page, viewport| {
                let painted_owner = viewport.identity();
                page.child(
                    div()
                        .relative()
                        .flex()
                        .flex_1()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .bg(rgb(0xe9edf2))
                        .p_3()
                        .child(
                            div()
                                .id("engine-viewport-clip")
                                .size_full()
                                .overflow_hidden()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(SURFACE))
                                .shadow_md()
                                .child(
                                    div()
                                        .relative()
                                        .size_full()
                                        .child(match &viewport.compositor {
                                            Some(scene) => viewport.scroll_element()
                                                .unwrap_or_else(|| scene.borrow().frame().element_with_device_scale(viewport.metrics.device_scale)),
                                            None => img(viewport.image.clone())
                                                .w(px(viewport.metrics.css_width))
                                                .h(px(viewport.metrics.css_height))
                                                .into_any_element(),
                                        })
                                        .when_some(self.tabs[self.active].render_failure.as_ref(), |el, error| {
                                            el.child(div().absolute().top_0().left_0().p_2()
                                                .bg(rgb(0x7f1d1d)).text_color(rgb(0xffffff))
                                                .child(format!("Render failure: {error}")))
                                        })
                                        // A11Y-11: the live page's accessibility tree, as an
                                        // unpainted geometry sibling (see `a11y_bridge`'s
                                        // `build_element` — it never paints or occupies flow
                                        // space) so GPUI's own AccessKit walk exposes it over
                                        // AT-SPI2 on Linux, matching A11Y-9's proof.
                                        .when_some(self.document_interaction_available(self.active).then(|| viewport.accessibility_tree()).flatten(), |el, tree| {
                                            el.child(a11y_bridge::ax_tree_element_with_scroll(tree, viewport.root_scroll_offset()))
                                        })
                                        .child(
                                            canvas(
                                                {
                                                    let entity = entity.clone();
                                                    let presented = viewport.identity();
                                                    move |bounds, window, app| {
                                                        if let Some(measured) = ViewportMetrics::new(
                                                            bounds.size.width.as_f32(), bounds.size.height.as_f32(),
                                                            window.scale_factor(),
                                                        ) {
                                                            let size = window.viewport_size();
                                                            let window_css = (size.width.as_f32(), size.height.as_f32());
                                                            if presented.is_document() {
                                                                window.defer(app, move |_, app| {
                                                                    entity.update(app, |this, cx| {
                                                                        this.apply_viewport_measurement(measured_tab,
                                                                            &presented, measured, window_css, cx);
                                                                    });
                                                                });
                                                            }
                                                        }
                                                        bounds
                                                    }
                                                },
                                                move |bounds, _, window, _| {
                                                    let wheel_entity = entity.clone();
                                                    let drag_entity = entity.clone();
                                                    let wheel_bounds = bounds.clone();
                                                    let drag_bounds = bounds.clone();
                                                    let entity = entity.clone();
                                                    let click_owner = painted_owner.clone();
                                                    let wheel_owner = painted_owner.clone();
                                                    let drag_owner = painted_owner.clone();
                                                    window.on_mouse_event(move |event: &MouseDownEvent, _, _, app| {
                                                        if event.button != MouseButton::Left
                                                            || !bounds.contains(&event.position)
                                                            || bounds.size.width.as_f32() <= 0.0
                                                        {
                                                            return;
                                                        }
                                                        entity.update(app, |this, cx| {
                                                            if this.active != measured_tab || !this.document_interaction_available(measured_tab) { return; }
                                                            let Some(viewport) = this.tabs[this.active].viewport.as_ref() else { return; };
                                                            if !viewport.accepts_identity(&click_owner)
                                                                || !viewport.metrics.matches_logical_bounds(bounds.size.width.as_f32(), bounds.size.height.as_f32())
                                                            { return; }
                                                            let local_x = (event.position.x - bounds.origin.x).as_f32();
                                                            if local_x >= bounds.size.width.as_f32() - 12. {
                                                                if let Some(node) = viewport.root_scroll_id() {
                                                                    let fraction = ((event.position.y - bounds.origin.y).as_f32()
                                                                        / bounds.size.height.as_f32()).clamp(0., 1.);
                                                                    this.scroll_active_viewport(ScrollInput::ScrollbarDrag {
                                                                        node, axis: ScrollAxis::Vertical, fraction,
                                                                    }, cx);
                                                                }
                                                                return;
                                                            }
                                                            let Some((x, y)) = viewport.metrics.document_point(
                                                                ((event.position.x - bounds.origin.x).as_f32(),
                                                                 (event.position.y - bounds.origin.y).as_f32()),
                                                                (bounds.size.width.as_f32(), bounds.size.height.as_f32()),
                                                                viewport.root_scroll_offset(),
                                                            ) else { return; };
                                                            let link = viewport
                                                                .link_hit_at(x, y)
                                                                .map(|(node, target)| {
                                                                    (node, target.to_string())
                                                                });
                                                            let control = viewport.control_at(x, y);
                                                            let text_control = control.and_then(|index| {
                                                                viewport.text_control(index).and_then(|(value, rect)| {
                                                                    viewport.control_identity(index).map(|identity|
                                                                        (index, identity, value.to_string(), rect))
                                                                })
                                                            });
                                                            let submit_request = control
                                                                .filter(|index| {
                                                                    viewport.text_control(*index).is_none()
                                                                })
                                                                .and_then(|index| {
                                                                    viewport.form_request(index, None)
                                                                });
                                                            if let Some((node, target)) = link {
                                                                let directive =
                                                                    this.dispatch_runtime_click(node);
                                                                let script_navigated =
                                                                    directive.navigation.is_some();
                                                                let prevented = this
                                                                    .apply_runtime_directive(directive);
                                                                if !prevented && !script_navigated {
                                                                    this.address = target;
                                                                    this.commit_navigation();
                                                                }
                                                            } else if let Some(request) = submit_request {
                                                                this.submit_form(request);
                                                            } else if let Some((control, node_identity, value, rect)) = text_control {
                                                                this.page_input = Some(PageInputState {
                                                                    tab: this.active,
                                                                    node_identity,
                                                                    control,
                                                                    value,
                                                                    rect,
                                                                    selected: true,
                                                                });
                                                            }
                                                            cx.notify();
                                                        });
                                                    });
                                                    window.on_mouse_event(move |event: &ScrollWheelEvent, _, window, app| {
                                                        if !wheel_bounds.contains(&event.position) { return; }
                                                        let delta = event.delta.pixel_delta(window.line_height());
                                                        wheel_entity.update(app, |this, cx| {
                                                            if this.active != measured_tab || !this.document_interaction_available(measured_tab) { return; }
                                                            let Some(viewport) = this.tabs[this.active].viewport.as_ref() else { return; };
                                                            if !viewport.accepts_identity(&wheel_owner)
                                                                || !viewport.metrics.matches_logical_bounds(wheel_bounds.size.width.as_f32(), wheel_bounds.size.height.as_f32())
                                                            { return; }
                                                            let x = (event.position.x - wheel_bounds.origin.x).as_f32()
                                                                * viewport.metrics.css_width / wheel_bounds.size.width.as_f32();
                                                            let y = (event.position.y - wheel_bounds.origin.y).as_f32()
                                                                * viewport.metrics.css_height / wheel_bounds.size.height.as_f32();
                                                            let delta = (-delta.x.as_f32(), -delta.y.as_f32());
                                                            let input = if event.delta.precise() {
                                                                ScrollInput::Trackpad { x, y, delta }
                                                            } else {
                                                                ScrollInput::Wheel { x, y, delta }
                                                            };
                                                            if this.scroll_active_viewport(input, cx) { cx.stop_propagation(); }
                                                        });
                                                    });
                                                    window.on_mouse_event(move |event: &MouseMoveEvent, _, _, app| {
                                                        if event.pressed_button != Some(MouseButton::Left)
                                                            || !drag_bounds.contains(&event.position)
                                                            || (event.position.x - drag_bounds.origin.x).as_f32()
                                                                < drag_bounds.size.width.as_f32() - 12. { return; }
                                                        drag_entity.update(app, |this, cx| {
                                                            if this.active != measured_tab || !this.document_interaction_available(measured_tab) { return; }
                                                            let Some(viewport) = this.tabs[this.active].viewport.as_ref() else { return; };
                                                            if !viewport.accepts_identity(&drag_owner)
                                                                || !viewport.metrics.matches_logical_bounds(drag_bounds.size.width.as_f32(), drag_bounds.size.height.as_f32())
                                                            { return; }
                                                            let Some(node) = viewport.root_scroll_id() else { return; };
                                                            let fraction = ((event.position.y - drag_bounds.origin.y).as_f32()
                                                                / drag_bounds.size.height.as_f32()).clamp(0., 1.);
                                                            this.scroll_active_viewport(ScrollInput::ScrollbarDrag {
                                                                node, axis: ScrollAxis::Vertical, fraction,
                                                            }, cx);
                                                        });
                                                    });
                                                },
                                            )
                                            .absolute()
                                            .inset_0(),
                                        )
                                        .when_some(viewport.root_scroll_thumb(), |surface, (top, size)| {
                                            surface.child(
                                                div()
                                                    .absolute()
                                                    .right_0()
                                                    .top(gpui::relative(top))
                                                    .w(px(8.0))
                                                    .h(gpui::relative(size))
                                                    .rounded_md()
                                                    .bg(rgb(0x8a95a5)),
                                            )
                                        })
                                        .when_some(page_input.clone(), |surface, input| {
                                            let (x, y, width, height) = input.rect;
                                            let (scroll_x, scroll_y) = viewport.root_scroll_offset();
                                            surface.child(
                                                div()
                                                    .absolute()
                                                    .left(px(x - scroll_x))
                                                    .top(px(y - scroll_y))
                                                    .w(px(width))
                                                    .h(px(height))
                                                    .flex()
                                                    .items_center()
                                                    .px_2()
                                                    .border_2()
                                                    .border_color(rgb(0x1a73e8))
                                                    .bg(rgb(SURFACE))
                                                    .text_color(rgb(TEXT))
                                                    .text_size(px(13.0))
                                                    .child(if input.selected {
                                                        input.value
                                                    } else {
                                                        format!("{}│", input.value)
                                                    }),
                                            )
                                        })
                                        .children({
                                            let (scroll_x, scroll_y) = viewport.root_scroll_offset();
                                            self.find_matches.iter().enumerate().map(move |(index, rect)| {
                                                let active = index == self.find_active;
                                                div()
                                                    .id(SharedString::from(format!("find-match-{index}")))
                                                    .absolute()
                                                    .left(px(rect.x - scroll_x))
                                                    .top(px(rect.y - scroll_y))
                                                    .w(px(rect.width))
                                                    .h(px(rect.height))
                                                    .when(active, |el| {
                                                        el.bg(gpui::rgba(0xff8c0088))
                                                            .border_2()
                                                            .border_color(rgb(0xff8c00))
                                                    })
                                                    .when(!active, |el| el.bg(gpui::rgba(0xffe06655)))
                                                    .into_any_element()
                                            })
                                        }),
                                ),
                        )
                        .when(!self.find_open, |surface| {
                            surface.child(
                                div()
                                    .absolute()
                                    .top_4()
                                    .right_4()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(TEXT))
                                    .text_color(rgb(SURFACE))
                                    .text_size(px(10.5))
                                    .child(format!(
                                        "AMAZING ENGINE  ·  {} × {}",
                                        viewport.width, viewport.height
                                    )),
                            )
                        })
                        .when(self.find_open, |surface| {
                            let total = self.find_matches.len();
                            let position_label = if total == 0 {
                                "0/0".to_string()
                            } else {
                                format!("{}/{}", self.find_active + 1, total)
                            };
                            surface.child(
                                div()
                                    .id("find-in-page-bar")
                                    .absolute()
                                    .top_4()
                                    .right_4()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .h(px(32.0))
                                    .px_2()
                                    .rounded_md()
                                    .bg(rgb(SURFACE))
                                    .border_1()
                                    .border_color(rgb(BORDER))
                                    .shadow_md()
                                    .text_size(px(12.0))
                                    .text_color(rgb(TEXT))
                                    .on_key_down(cx.listener(Self::on_find_key_down))
                                    .child(
                                        div()
                                            .id("find-in-page-input")
                                            .min_w(px(90.0))
                                            .cursor(CursorStyle::IBeam)
                                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                                                this.activate_chrome_input(ChromeInputOwner::Find, window, cx);
                                                cx.stop_propagation(); cx.notify();
                                            }))
                                            .child(format!("{}│", self.find_query)),
                                    )
                                    .child(
                                        div()
                                            .id("find-in-page-count")
                                            .text_color(rgb(DIM))
                                            .child(position_label),
                                    )
                                    .child(
                                        div()
                                            .id("find-in-page-prev")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.find_go(-1, cx);
                                            }))
                                            .child("↑"),
                                    )
                                    .child(
                                        div()
                                            .id("find-in-page-next")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.find_go(1, cx);
                                            }))
                                            .child("↓"),
                                    )
                                    .child(
                                        div()
                                            .id("find-in-page-close")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.close_find(window, cx);
                                            }))
                                            .child("✕"),
                                    ),
                            )
                        }),
                )
            })
            .when(tab.kind == TabKind::Video, |page| {
                page.child(
                    div()
                        .relative()
                        .flex()
                        .flex_1()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .bg(rgb(0x111820))
                        .p_6()
                        .child(
                            div()
                                .size_full()
                                .overflow_hidden()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(0x33404d))
                                .bg(rgb(0x080c10))
                                .shadow_lg()
                                .child(img(video_viewport.image.clone()).size_full()),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_8()
                                .right_8()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(0x12291b))
                                .text_color(rgb(0xbde7c6))
                                .text_size(px(10.5))
                                .child(video_badge),
                        ),
                )
            })
            .when(tab.kind == TabKind::Document, |page| {
                page.child(
                    div()
                        .w(px(760.0))
                        .px(px(64.0))
                        .py(px(56.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.0))
                                .text_color(rgb(DIM))
                                .child(div().w(px(18.0)).h(px(1.0)).bg(rgb(ACCENT)))
                                .child(tab.kicker.clone()),
                        )
                        .child(
                            div()
                                .mt_4()
                                .font_family("Georgia")
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_size(px(46.0))
                                .line_height(gpui::relative(1.1))
                                .text_color(rgb(TEXT))
                                .child(tab.title.clone()),
                        )
                        .child(
                            div()
                                .mt_4()
                                .w(px(620.0))
                                .text_size(px(16.0))
                                .line_height(gpui::relative(1.6))
                                .text_color(rgb(DIM))
                                .child(tab.lede.clone()),
                        ),
                )
            })
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .max_w(px(520.0))
                    .px_2()
                    .py_1()
                    .rounded_tr_md()
                    .border_t_1()
                    .border_r_1()
                    .border_color(rgb(RULE))
                    .bg(rgb(SURFACE))
                    .text_size(px(11.0))
                    .text_color(rgb(DIM))
                    .child(page_url),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .px_2()
                    .py_1()
                    .text_size(px(10.5))
                    .text_color(rgb(DIM))
                    .child(build_info_label()),
            )
    }
}

impl Render for BrowserShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = &self.tabs[self.active];
        let active_title = active.title.clone();
        let active_mono = active.mono.clone();
        let active_hue = active.hue;
        let active_icon = if let Some(image) = active.favicon.clone() {
            div()
                .size(px(17.0))
                .overflow_hidden()
                .rounded_sm()
                .child(img(image).size_full())
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(17.0))
                .rounded_sm()
                .bg(rgb(active_hue))
                .text_color(rgb(0xffffff))
                .text_size(px(9.0))
                .child(active_mono)
                .into_any_element()
        };
        let sidebar = self.sidebar;
        div()
            .id("browser-shell")
            .relative()
            .when_some(self.shell_focus.as_ref(), |shell, focus| {
                shell.track_focus(focus)
            })
            .on_action(cx.listener(Self::focus_address))
            .on_action(cx.listener(Self::new_browser_tab))
            .on_action(cx.listener(Self::close_browser_tab))
            .on_action(cx.listener(Self::next_browser_tab))
            .on_action(cx.listener(Self::previous_browser_tab))
            .on_action(cx.listener(Self::reload_page))
            .on_action(cx.listener(Self::navigate_back))
            .on_action(cx.listener(Self::navigate_forward))
            .on_action(cx.listener(Self::toggle_bookmark_action))
            .on_action(cx.listener(Self::show_downloads))
            .on_action(cx.listener(Self::open_find))
            .on_action(cx.listener(Self::dismiss_chrome))
            .on_key_down(cx.listener(Self::on_shell_key_down))
            .flex()
            .flex_col()
            .size_full()
            .font_family("Segoe UI")
            .bg(rgb(STRIP))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .flex()
                    .items_end()
                    .h(px(40.0))
                    .pl_2()
                    .child(
                        div()
                            .id("title-sidebar-toggle")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(30.0))
                            .mb_1()
                            .mr_2()
                            .rounded_md()
                            .text_color(rgb(ICON))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(STRIP_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar();
                                cx.notify();
                            }))
                            .child("▣"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .h(px(34.0))
                            .gap_2()
                            .child(
                                div()
                                    .id("pinned-amazing")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .h(px(34.0))
                                    .w(px(44.0))
                                    .rounded_t_lg()
                                    .bg(rgb(SURFACE))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_bookmark(0);
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(18.0))
                                            .rounded_sm()
                                            .bg(rgb(ACCENT))
                                            .text_color(rgb(0xffffff))
                                            .text_size(px(10.0))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .child("A"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h(px(34.0))
                                    .px_1()
                                    .rounded_t_lg()
                                    .bg(gpui::rgba(0xb1552b29))
                                    .child(
                                        div()
                                            .id("engine-group")
                                            .flex()
                                            .items_center()
                                            .h(px(22.0))
                                            .px_2()
                                            .mr_1()
                                            .rounded_md()
                                            .bg(rgb(ACCENT))
                                            .text_color(rgb(0xffffff))
                                            .text_size(px(11.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_engine_group();
                                                cx.notify();
                                            }))
                                            .child("Engine  3"),
                                    )
                                    .when(!self.engine_collapsed, |group| {
                                        group.children(
                                            (0..self.tabs.len())
                                                .filter(|index| {
                                                    self.tabs[*index].open
                                                        && self.tabs[*index].group
                                                            == TabGroup::Engine
                                                })
                                                .map(|index| self.tab(index, cx)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .h(px(34.0))
                                    .px_1()
                                    .rounded_t_lg()
                                    .bg(gpui::rgba(0x3a6ea829))
                                    .child(Self::group_chip("Specs", 3, 0x3a6ea8, cx))
                                    .when(!self.specs_collapsed, |group| {
                                        group.children(
                                            (0..self.tabs.len())
                                                .filter(|index| {
                                                    self.tabs[*index].open
                                                        && self.tabs[*index].group
                                                            == TabGroup::Specs
                                                })
                                                .map(|index| self.tab(index, cx)),
                                        )
                                    }),
                            )
                            .children(
                                (0..self.tabs.len())
                                    .filter(|index| {
                                        self.tabs[*index].open
                                            && self.tabs[*index].group == TabGroup::Other
                                    })
                                    .map(|index| self.tab(index, cx)),
                            )
                            .child(
                                div()
                                    .id("new-tab")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_tab();
                                        this.activate_chrome_input(ChromeInputOwner::Address, window, cx);
                                        cx.notify();
                                    }))
                                    .child(Self::icon_button("+", true)),
                            )
                            .when(sidebar, |row| row.hidden()),
                    )
                    .when(sidebar, |bar| {
                        bar.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .h(px(34.0))
                                .w(px(260.0))
                                .px_2()
                                .rounded_t_lg()
                                .bg(rgb(SURFACE))
                                .child(active_icon)
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_size(px(12.0))
                                        .child(active_title),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    )
                    .children([
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(46.0))
                            .h_full()
                            .text_color(rgb(ICON))
                            .hover(|style| style.bg(gpui::rgba(0x78808c38)))
                            .window_control_area(WindowControlArea::Min)
                            .child("—"),
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(46.0))
                            .h_full()
                            .text_color(rgb(ICON))
                            .hover(|style| style.bg(gpui::rgba(0x78808c38)))
                            .window_control_area(WindowControlArea::Max)
                            .child("□"),
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(46.0))
                            .h_full()
                            .text_color(rgb(ICON))
                            .hover(|style| style.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff)))
                            .window_control_area(WindowControlArea::Close)
                            .child("×"),
                    ]),
            )
            .child(self.navigation_toolbar(cx, !sidebar))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(34.0))
                    .px_3()
                    .bg(rgb(SURFACE))
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .children(
                        [
                            ("■", "Phase 9 board", ACCENT, Some(0)),
                            ("■", "WHATWG HTML", 0x3a6ea8, Some(3)),
                            ("■", "CSS Syntax L3", 0x3f7d4e, Some(4)),
                            ("■", "ECMA-262", 0x7a5aa8, Some(5)),
                            ("▤", "Downloads", 0x6b7280, None),
                        ]
                        .into_iter()
                        .map(|(icon, label, color, target)| {
                            div()
                                .id(SharedString::from(format!("bookmark-{label}")))
                                .flex()
                                .items_center()
                                .gap_1()
                                .h(px(24.0))
                                .px_2()
                                .rounded_md()
                                .text_size(px(12.0))
                                .text_color(rgb(TEXT))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(SURFACE_2)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(index) = target {
                                        this.open_bookmark(index);
                                    } else {
                                        this.toggle_downloads();
                                    }
                                    cx.notify();
                                }))
                                .child(div().text_color(rgb(color)).child(icon))
                                .child(label)
                        }),
                    )
                    .when(sidebar, |row| row.hidden()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(sidebar, |body| {
                        body.child(self.sidebar(cx)).child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(self.navigation_toolbar(cx, true))
                                .child(self.page(cx)),
                        )
                    })
                    .when(!sidebar, |body| body.child(self.page(cx))),
            )
            .child(self.downloads_panel(cx))
            .child(self.site_info_panel(cx))
            .child(self.profile_panel(cx))
            .child(self.browser_menu(cx))
    }
}

fn main() {
    if std::env::args().any(|argument| argument == "--type=utility") {
        let child = amazing_process::bootstrap_child(
            amazing_process::Role::Utility,
            amazing_process::Capabilities(std::collections::BTreeMap::new()),
            |connection, _, channel| {
                amazing_process::utility::serve_utility(connection, channel);
                Ok(())
            },
        );
        let Ok(connection) = child else { std::process::exit(1); };
        while !connection.is_closed() {
            std::thread::sleep(Duration::from_millis(10));
        }
        return;
    }
    let start_with_sidebar = std::env::args().any(|argument| argument == "--sidebar");
    let start_with_video = std::env::args().any(|argument| argument == "--video");
    let start_with_secure_video = std::env::args().any(|argument| argument == "--secure-video");
    let start_url = parse_flag_value("--url");
    let start_size = parse_flag_value("--size").and_then(|value| parse_size_flag(&value));
    let (secure_receiver, secure_status) = if start_with_secure_video {
        match spawn_secure_video_source() {
            Ok(receiver) => (Some(receiver), "CONNECTING SECURE VIDEO…".to_string()),
            Err(error) => (None, format!("SECURE VIDEO ERROR  ·  {error}")),
        }
    } else {
        (None, String::new())
    };
    application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("ctrl-l", FocusAddress, None),
            KeyBinding::new("f6", FocusAddress, None),
            KeyBinding::new("ctrl-t", NewBrowserTab, None),
            KeyBinding::new("ctrl-w", CloseBrowserTab, None),
            KeyBinding::new("ctrl-tab", NextBrowserTab, None),
            KeyBinding::new("ctrl-shift-tab", PreviousBrowserTab, None),
            KeyBinding::new("ctrl-r", ReloadPage, None),
            KeyBinding::new("f5", ReloadPage, None),
            KeyBinding::new("alt-left", NavigateBack, None),
            KeyBinding::new("alt-right", NavigateForward, None),
            KeyBinding::new("ctrl-d", ToggleBookmark, None),
            KeyBinding::new("ctrl-j", ShowDownloads, None),
            KeyBinding::new("ctrl-f", FindInPage, None),
            KeyBinding::new("escape", DismissChrome, None),
        ]);
        let (window_width, window_height) = start_size.unwrap_or((1080.0, 760.0));
        let bounds = Bounds::centered(None, size(px(window_width), px(window_height)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(move |cx| {
                    let shell_focus = cx.focus_handle();
                    shell_focus.focus(window, cx);
                    let mut shell = BrowserShell::new(
                        Some(cx.focus_handle().tab_stop(true)),
                        Some(cx.focus_handle().tab_stop(true)),
                    );
                    shell.shell_focus = Some(shell_focus);
                    cx.spawn(async move |this, cx| {
                        let started = Instant::now();
                        let mut delay_ms = if start_with_secure_video {
                            SECURE_VIDEO_POLL_MS
                        } else {
                            25
                        };
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(delay_ms))
                                .await;
                            match this.update(cx, |shell: &mut BrowserShell, cx| {
                                let fallback_now_ms =
                                    started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                                shell.poll_navigation();
                                shell.trace_navigation_state();
                                shell.pump_active_runtime();
                                shell.poll_secure_video(fallback_now_ms);
                                let clock_now_ms = shell
                                    .audio_device_clock
                                    .as_ref()
                                    .map_or(fallback_now_ms, AudioDeviceClock::position_ms);
                                let update = shell.update_video_clock(clock_now_ms);
                                let next_delay_ms =
                                    if shell.secure_video_requested && !shell.secure_video_active {
                                        SECURE_VIDEO_POLL_MS
                                    } else {
                                        update.next_delay_ms
                                    };
                                cx.notify();
                                next_delay_ms
                            }) {
                                Ok(next_delay) => delay_ms = next_delay.min(25),
                                Err(_) => break,
                            }
                        }
                    })
                    .detach();
                    shell.sidebar = start_with_sidebar;
                    shell.secure_video_receiver = secure_receiver;
                    if start_with_secure_video {
                        shell.secure_video_requested = true;
                        shell.secure_video_status = secure_status;
                    }
                    if start_with_video || start_with_secure_video {
                        shell.select_tab(1);
                    }
                    if start_with_sidebar {
                        shell.specs_collapsed = false;
                    }
                    if let Some(url) = start_url.clone() {
                        shell.address = url;
                        shell.commit_navigation();
                    }
                    shell
                })
            },
        )
        .expect("open Amazing GPUI shell");
        cx.activate(true);
    });
}

/// The new-tab status line was a Windows-only literal ("win-001 · GPUI 0.2.2
/// · ThinLTO") baked in when the shell only ever ran on the win-001 build
/// server. Now that it also runs on Linux Boat sandboxes, report the actual
/// host and build profile instead of a fixed string. Computed once and
/// cached: this renders on every frame the new-tab page is visible, and the
/// hostname lookup can shell out on platforms with no direct env var.
fn build_info_label() -> &'static str {
    static LABEL: OnceLock<String> = OnceLock::new();
    LABEL.get_or_init(|| {
        let host = host_label();
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        format!("{host}  ·  GPUI 0.2.2  ·  {profile}")
    })
}

/// Best-effort local hostname, without any FFI (this crate denies
/// `unsafe_code`): environment variables first, then a Linux-specific
/// `/proc` read, then a `hostname` subprocess as a last resort covering
/// macOS and any other Unix that lacks `HOSTNAME` in its environment.
fn host_label() -> String {
    if let Ok(name) = std::env::var("COMPUTERNAME") {
        if !name.trim().is_empty() {
            return name;
        }
    }
    if let Ok(name) = std::env::var("HOSTNAME") {
        if !name.trim().is_empty() {
            return name;
        }
    }
    if let Ok(contents) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let trimmed = contents.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(output) = std::process::Command::new("hostname").output() {
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "unknown-host".to_string()
}

/// Reads `--flag value` or `--flag=value` from the process argv. Used for
/// `--url` and `--size`, which captures pass to open a specific page/size
/// without touching the interactive default-tab layout.
fn parse_flag_value(flag: &str) -> Option<String> {
    let mut arguments = std::env::args();
    let prefix = format!("{flag}=");
    while let Some(argument) = arguments.next() {
        if argument == flag {
            return arguments.next();
        }
        if let Some(value) = argument.strip_prefix(&prefix) {
            return Some(value.to_string());
        }
    }
    None
}

/// Parses a `--size WxH` value such as `1280x800` into `(width, height)`.
fn parse_size_flag(value: &str) -> Option<(f32, f32)> {
    let (width, height) = value.split_once(['x', 'X'])?;
    let width: f32 = width.trim().parse().ok()?;
    let height: f32 = height.trim().parse().ok()?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn normalize_address(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else if value.contains("://") {
        Some(value.to_string())
    } else if !value.chars().any(char::is_whitespace)
        && (value.contains('.')
            || value.starts_with("localhost")
            || value.starts_with("127.0.0.1")
            || value.starts_with("[::1]"))
    {
        let scheme = if value.starts_with("localhost")
            || value.starts_with("127.0.0.1")
            || value.starts_with("[::1]")
        {
            "http"
        } else {
            "https"
        };
        Some(format!("{scheme}://{value}"))
    } else {
        Some(format!(
            "https://www.google.com/search?q={}",
            encode_search_query(value)
        ))
    }
}

fn encode_search_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn address_title(address: &str) -> String {
    if address == "amazing://newtab" {
        return "New tab".into();
    }
    address
        .split_once("://")
        .map_or(address, |(_, rest)| rest)
        .split('/')
        .next()
        .filter(|title| !title.is_empty())
        .unwrap_or("New tab")
        .to_string()
}

fn navigation_title(address: &str) -> String {
    match address {
        "amazing://status" => "Amazing — Phase 9 status".into(),
        "amazing://newtab" => "New tab".into(),
        _ => address_title(address),
    }
}

fn site_initial(address: &str) -> char {
    address_title(address)
        .chars()
        .find(|character| character.is_alphanumeric())
        .map_or('+', |character| character.to_ascii_uppercase())
}

fn presentation_gap_exceeds_cadence(presentation_ms: &[u64], next_ms: u64) -> bool {
    let Some(previous) = presentation_ms.last().copied() else {
        return false;
    };
    let expected_interval = if presentation_ms.len() >= 2 {
        previous.saturating_sub(presentation_ms[presentation_ms.len() - 2])
    } else {
        500
    };
    next_ms.saturating_sub(previous) > expected_interval
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod navigation_tests;

#[cfg(test)]
mod tests {
    fn settle_navigation(shell: &mut super::BrowserShell) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while shell.pending_navigation.iter().any(Option::is_some) {
            shell.poll_navigation();
            assert!(std::time::Instant::now() < deadline, "navigation did not settle");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    use super::*;

    #[test]
    fn size_flag_parses_widthxheight_and_rejects_bad_input() {
        assert_eq!(parse_size_flag("1280x800"), Some((1280.0, 800.0)));
        assert_eq!(parse_size_flag("640X480"), Some((640.0, 480.0)));
        assert_eq!(parse_size_flag("0x480"), None);
        assert_eq!(parse_size_flag("nope"), None);
        assert_eq!(parse_size_flag("800x"), None);
    }

    #[test]
    fn build_info_label_reports_real_host_and_profile_not_win_001() {
        let label = build_info_label();
        assert!(!label.contains("win-001"));
        assert!(!label.contains("ThinLTO"));
        assert!(label.contains("GPUI 0.2.2"));
        assert!(label.contains("debug") || label.contains("release"));
        // Cached: a second call returns the identical pointer's contents.
        assert_eq!(label, build_info_label());
    }

    #[test]
    fn host_label_is_never_empty() {
        assert!(!host_label().trim().is_empty());
    }

    #[test]
    fn reference_shell_state_starts_on_status_with_specs_collapsed() {
        let shell = BrowserShell::new(None, None);
        assert_eq!(shell.tabs.len(), 6);
        assert_eq!(shell.active, 0);
        assert!(!shell.engine_collapsed);
        assert!(shell.specs_collapsed);
        assert!(!shell.sidebar);
        assert_eq!(shell.tabs[0].url, "amazing://status");
    }

    #[test]
    fn reference_groups_keep_engine_and_spec_tabs_distinct() {
        let shell = BrowserShell::new(None, None);
        assert_eq!(
            &shell.tabs[0..3]
                .iter()
                .map(|tab| tab.mono.as_str())
                .collect::<Vec<_>>(),
            &["A", "V", "H"]
        );
        assert_eq!(
            &shell.tabs[3..6]
                .iter()
                .map(|tab| tab.mono.as_str())
                .collect::<Vec<_>>(),
            &["S", "C", "E"]
        );
    }

    #[test]
    fn shell_model_transitions_cover_tabs_groups_and_sidebar() {
        let mut shell = BrowserShell::new(None, None);
        shell.select_tab(4);
        shell.toggle_engine_group();
        shell.toggle_specs_group();
        shell.toggle_sidebar();
        assert_eq!(shell.active, 4);
        assert!(shell.engine_collapsed);
        assert!(!shell.specs_collapsed);
        assert!(shell.sidebar);
        shell.select_tab(99);
        assert_eq!(shell.active, 4, "out-of-range selection is ignored");
    }

    #[test]
    fn chrome_popovers_are_exclusive_and_bookmarks_toggle() {
        let mut shell = BrowserShell::new(None, None);
        shell.toggle_downloads();
        assert!(shell.downloads_open);
        shell.toggle_menu();
        assert!(shell.menu_open);
        assert!(!shell.downloads_open);
        shell.toggle_site_info();
        assert!(shell.site_info_open);
        assert!(!shell.menu_open);
        shell.toggle_profile();
        assert!(shell.profile_open);
        assert!(!shell.site_info_open);
        shell.toggle_bookmark();
        assert!(shell.tabs[0].bookmarked);
        shell.toggle_bookmark();
        assert!(!shell.tabs[0].bookmarked);
    }

    #[test]
    fn address_normalization_and_navigation_update_the_engine_surface() {
        assert_eq!(
            normalize_address("example.test/docs"),
            Some("https://example.test/docs".to_string())
        );
        assert_eq!(normalize_address("   "), None);
        assert_eq!(
            normalize_address("amazing browser"),
            Some("https://www.google.com/search?q=amazing+browser".to_string())
        );
        assert_eq!(
            normalize_address("localhost:8080/status"),
            Some("http://localhost:8080/status".to_string())
        );

        let mut shell = BrowserShell::new(None, None);
        shell.address = "example.test/docs".to_string();
        shell.commit_navigation();
        settle_navigation(&mut shell);
        assert_eq!(shell.active, 0);
        assert_eq!(shell.address, "https://example.test/docs");
        assert_eq!(shell.committed_address, shell.address);
    }

    #[test]
    fn decoded_video_frames_cycle_without_changing_tab_state() {
        let mut shell = BrowserShell::new(None, None);
        shell.select_tab(1);
        assert_eq!(shell.video_frames.len(), 6);
        shell.update_video_clock(3_000);
        assert_eq!(shell.video_frame_index, 0);
        assert_eq!(shell.active, 1);
        assert_eq!(shell.update_video_clock(3_000).next_delay_ms, 500);
    }

    #[test]
    fn presentation_gap_identifies_one_recovery_discontinuity() {
        assert!(!presentation_gap_exceeds_cadence(&[], 0));
        assert!(!presentation_gap_exceeds_cadence(&[0], 500));
        assert!(presentation_gap_exceeds_cadence(&[0, 500], 1_500));
        assert!(!presentation_gap_exceeds_cadence(&[0, 500, 1_500], 2_000));
    }

    #[test]
    fn history_back_forward_and_branching_follow_browser_rules() {
        // Preserve the original normalization and history assertions, while the
        // asynchronous lifecycle uses real deterministic HTTP rather than public DNS.
        assert_eq!(normalize_address("first.test").as_deref(), Some("https://first.test"));
        assert_eq!(normalize_address("second.test").as_deref(), Some("https://second.test"));
        assert_eq!(normalize_address("branch.test").as_deref(), Some("https://branch.test"));
        let fixture = navigation_tests::Fixture::new();
        let (first, second, branch) = (fixture.url("/first"), fixture.url("/second"), fixture.url("/branch"));
        let mut shell = BrowserShell::new(None, None);
        shell.address = first.clone();
        shell.commit_navigation();
        settle_navigation(&mut shell);
        shell.address = second.clone();
        shell.commit_navigation();
        settle_navigation(&mut shell);
        assert_eq!(shell.tabs[0].history.len(), 3);
        assert_eq!(shell.tabs[0].history_index, 2);

        shell.go_back();
        settle_navigation(&mut shell);
        assert_eq!(shell.committed_address, first);
        shell.go_forward();
        settle_navigation(&mut shell);
        assert_eq!(shell.committed_address, second);

        shell.go_back();
        settle_navigation(&mut shell);
        shell.address = branch.clone();
        shell.commit_navigation();
        settle_navigation(&mut shell);
        assert_eq!(
            shell.tabs[0].history,
            [
                "amazing://status".to_owned(),
                first.clone(),
                branch.clone()
            ]
        );
        assert_eq!(shell.tabs[0].history_index, 2);
        shell.go_forward();
        settle_navigation(&mut shell);
        assert_eq!(shell.committed_address, branch);
    }

    #[test]
    fn script_history_directives_update_tab_without_reloading() {
        let mut shell = BrowserShell::new(None, None);
        shell.committed_address = "https://example.test/root".into();
        shell.address = shell.committed_address.clone();
        shell.tabs[0].url = shell.committed_address.clone();
        shell.tabs[0].history = vec![shell.committed_address.clone()];
        let original_image = shell.tabs[0].viewport.as_ref().unwrap().image.clone();
        shell.apply_runtime_directive(RuntimeDirective {
            default_prevented: false,
            navigation: None,
            history_delta: None,
            history_update: Some(("virtual/page".into(), false)),
        });
        assert_eq!(
            shell.committed_address,
            "https://example.test/virtual/page"
        );
        assert_eq!(shell.tabs[0].history.len(), 2);
        assert!(Arc::ptr_eq(
            &original_image,
            &shell.tabs[0].viewport.as_ref().unwrap().image
        ));
    }

    #[test]
    fn new_and_closed_tabs_keep_independent_browser_state() {
        let mut shell = BrowserShell::new(None, None);
        shell.add_tab();
        let new_tab = shell.active;
        assert_eq!(shell.tabs[new_tab].url, "amazing://newtab");
        assert_eq!(shell.tabs[new_tab].history, ["amazing://newtab"]);
        shell.address = "localhost:8080/example".into();
        shell.commit_navigation();
        settle_navigation(&mut shell);
        assert_eq!(shell.tabs[new_tab].history.len(), 2);
        assert_eq!(shell.tabs[0].history, ["amazing://status"]);

        shell.close_tab(new_tab);
        assert!(!shell.tabs[new_tab].open);
        assert_eq!(shell.active, 5);
        shell.open_bookmark(0);
        assert_eq!(shell.active, 0);
        assert!(shell.tabs[0].open);
    }

    #[test]
    fn find_matches_in_tree_locates_static_text_case_insensitively_in_reading_order() {
        let tree = amazing_a11y::build_from_parsed_html(
            "<body><p>Hello Amazing World</p><p>Second Amazing paragraph</p></body>",
            "",
            800.0,
            Some(600.0),
        );
        let matches = BrowserShell::find_matches_in_tree(&tree, "amazing");
        assert_eq!(matches.len(), 2, "one match per paragraph containing the query");
        for rect in &matches {
            assert!(rect.width > 0.0 && rect.height > 0.0);
        }
        assert!(matches[0].y <= matches[1].y, "matches are in reading (top-to-bottom) order");

        assert_eq!(
            BrowserShell::find_matches_in_tree(&tree, "AMAZING"),
            matches,
            "matching is case-insensitive"
        );
        assert!(BrowserShell::find_matches_in_tree(&tree, "").is_empty());
        assert!(BrowserShell::find_matches_in_tree(&tree, "no-such-text").is_empty());
    }

    #[test]
    fn find_step_wraps_forward_and_backward_and_is_stable_with_no_matches() {
        assert_eq!(BrowserShell::find_step(0, 3, 1), 1);
        assert_eq!(BrowserShell::find_step(2, 3, 1), 0, "wraps forward past the last match");
        assert_eq!(BrowserShell::find_step(0, 3, -1), 2, "wraps backward past the first match");
        assert_eq!(BrowserShell::find_step(0, 0, 1), 0, "no matches: stays at 0");
    }

    #[test]
    fn find_state_defaults_to_closed_and_empty_on_a_fresh_shell() {
        let shell = BrowserShell::new(None, None);
        assert!(!shell.find_open);
        assert!(shell.find_query.is_empty());
        assert!(shell.find_matches.is_empty());
        assert_eq!(shell.find_active, 0);
    }

    #[test]
    fn find_is_available_requires_the_active_tab_to_have_a_viewport() {
        // SHELL-L9's product smoke surfaced this the hard way: closing a tab
        // can fall back to a pinned reference tab (Document kind, no
        // `EngineViewport`, e.g. "ECMA-262" below) or a media tab (Video
        // kind), and `open_find`'s bar/overlays only render inside `page()`'s
        // `.when_some(tab.viewport.as_ref(), ...)` branch. Without this guard
        // ctrl-f flips `find_open` to true with no UI anywhere to show or
        // close it -- indistinguishable, on screen, from the shell simply no
        // longer responding to input.
        let mut shell = BrowserShell::new(None, None);
        assert!(shell.find_is_available(), "tab 0 is the Engine demo tab");
        shell.select_tab(1); // Secure video (TabKind::Video, no viewport)
        assert!(!shell.find_is_available());
        shell.select_tab(2); // html5lib tree-construction (TabKind::Document)
        assert!(!shell.find_is_available());
        shell.select_tab(0);
        assert!(shell.find_is_available());
    }

    #[test]
    fn switching_tabs_closes_any_open_find_session() {
        // Find-in-page is per-tab state in every real browser; keeping it on
        // `BrowserShell` (there is no per-`Tab` field for it) means a find
        // session left open on one tab would otherwise still show -- with
        // its old query and matches -- after switching to an unrelated tab.
        let mut shell = BrowserShell::new(None, None);
        shell.find_open = true;
        shell.find_query = "engine".to_string();
        shell.find_matches = vec![amazing_a11y::AxRect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        }];
        shell.find_active = 0;
        shell.select_tab(1);
        assert!(!shell.find_open);
        assert!(shell.find_query.is_empty());
        assert!(shell.find_matches.is_empty());
        assert_eq!(shell.find_active, 0);
    }

    #[test]
    fn actual_failed_runtime_keeps_error_through_cached_movement_and_recovers_only_checked() {
        let mut shell = BrowserShell::new(None, None);
        let mut runtime = PageRuntime::failure_fixture();
        let initial = runtime.fixture_update("");
        assert!(initial.render_failure.is_none());
        shell.apply_runtime_update(0, initial);
        let old_image = shell.tabs[0].viewport.as_ref().unwrap().image.clone();
        let failed = runtime.fixture_update("document.querySelector('#target').setAttribute('style','width:1500px;height:1500px;border:4px solid red;border-image-source:linear-gradient(red,blue)');");
        assert!(failed.viewport.is_none());
        assert!(failed.render_failure.is_some(), "genuine nonunit generated source admission fails");
        shell.apply_runtime_update(0, failed);
        assert!(Arc::ptr_eq(&old_image, &shell.tabs[0].viewport.as_ref().unwrap().image));
        let error = shell.tabs[0].render_failure.clone();
        assert!(!shell.document_interaction_available(0));
        assert!(!shell.find_is_available());
        assert!(shell.dispatch_runtime_click(0).default_prevented);
        assert!(!shell.publish_checked_viewport(0, Ok(None), runtime.metrics_for_fixture()));
        for y in [0.0, 30.0, 30.0, 0.0] {
            let view = shell.tabs[0].viewport.as_ref().unwrap();
            let node = view.root_scroll_id().expect("real scroll root");
            let result = view.scroll_input(ScrollInput::ScrollTo { node, target:(0.0,y),
                behavior: ScrollBehavior::Instant }, false);
            assert!(result.is_ok());
            shell.record_cached_movement(0, result);
            assert_eq!(shell.tabs[0].render_failure, error);
            assert!(!shell.document_interaction_available(0));
        }
        let recovered = runtime.fixture_update("document.querySelector('#target').setAttribute('style','width:40px;height:40px;border:4px solid red');");
        assert!(recovered.viewport.is_some());
        shell.apply_runtime_update(0, recovered);
        assert!(shell.tabs[0].render_failure.is_none());
        assert!(shell.document_interaction_available(0));
        assert!(!Arc::ptr_eq(&old_image, &shell.tabs[0].viewport.as_ref().unwrap().image));
    }

    #[test]
    fn failed_current_dom_reflow_preserves_directives_and_rejects_stale_frame_interaction() {
        let mut shell = BrowserShell::new(None, None);
        let mut runtime = PageRuntime::failure_fixture();
        shell.apply_runtime_update(0, runtime.fixture_update(""));
        let old_owner = shell.tabs[0].viewport.as_ref().unwrap().identity();
        let failed = runtime.fixture_update("document.querySelector('#target').setAttribute('style','width:1500px;height:1500px;border:4px solid red;border-image-source:linear-gradient(red,blue)');history.pushState({},'', 'failed-current');");
        assert!(failed.render_failure.is_some());
        let directive = shell.apply_runtime_update(0, failed);
        assert!(directive.history_update.is_some(), "render failure must not discard real directives");
        assert!(shell.tabs[0].viewport.as_ref().unwrap().accepts_identity(&old_owner));
        assert!(!shell.document_interaction_available(0), "old frame identity alone cannot authorize hits");
        let metrics = ViewportMetrics::new(140.0, 100.0, 2.0).unwrap();
        let reflow = runtime.reflow(metrics).map(Some);
        assert!(reflow.is_err());
        assert!(!shell.publish_checked_viewport(0, reflow, metrics));
        assert_eq!(shell.tabs[0].failed_metrics, Some(metrics));
        assert!(!shell.document_interaction_available(0));
        runtime.fixture_update("document.querySelector('#target').setAttribute('style','width:40px;height:40px;border:4px solid red');");
        let checked = runtime.reflow(metrics).map(Some);
        assert!(shell.publish_checked_viewport(0, checked, metrics));
        assert!(shell.tabs[0].render_failure.is_none());
        assert!(shell.document_interaction_available(0));
        assert!(!shell.tabs[0].viewport.as_ref().unwrap().accepts_identity(&old_owner));
    }


    #[test]
    fn current_render_failure_does_not_suspend_real_runtime_timer_recovery() {
        let mut shell = BrowserShell::new(None, None);
        let mut runtime = PageRuntime::failure_fixture();
        shell.apply_runtime_update(0, runtime.fixture_update(""));
        let failed = runtime.fixture_update("document.querySelector('#target').setAttribute('style','width:1500px;height:1500px;border:4px solid red;border-image-source:linear-gradient(red,blue)');setTimeout(function(){document.querySelector('#target').setAttribute('style','width:40px;height:40px;border:4px solid red');document.title='Recovered by timer';},20);");
        assert!(failed.render_failure.is_some());
        shell.apply_runtime_update(0, failed);
        shell.page_runtimes[0] = Some(runtime);
        std::thread::sleep(Duration::from_millis(30));
        shell.pump_active_runtime();
        assert!(shell.tabs[0].render_failure.is_none());
        assert_eq!(shell.tabs[0].title, "Recovered by timer");
        assert!(shell.document_interaction_available(0));
    }

    #[test]
    fn real_newtab_find_scroll_finishes_at_both_window_densities() {
        for (scale, width, height) in [(1.0, 1254.0, 656.0), (2.0, 614.0, 256.0)] {
            let metrics = ViewportMetrics::new(width, height, scale).unwrap();
            let view = EngineViewport::new_tab().try_reflow_static(metrics).unwrap().unwrap();
            for query in ["A", "Amazing", "engine", "not-present-proof"] {
                let matches = BrowserShell::find_matches_in_tree(view.accessibility_tree().unwrap(), query);
                assert_eq!(matches.is_empty(), query == "not-present-proof");
                if let Some(rect) = matches.first() {
                    let node = view.root_scroll_id().unwrap();
                    let target_y = (rect.y - ((metrics.css_height - rect.height) / 2.0).max(0.0)).max(0.0);
                    let before = view.completed_presentations_for_fixture();
                    let moved = view.scroll_input(ScrollInput::ScrollTo { node, target:(0.0,target_y), behavior:ScrollBehavior::Instant }, false).unwrap();
                    assert_eq!(view.completed_presentations_for_fixture() - before, u64::from(moved), "one real scroll must produce one finished frame through one resource owner");
                    if moved { assert!(view.scroll_element().is_some()); }
                }
            }
        }
    }

}
