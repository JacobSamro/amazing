#![deny(unsafe_code)]

mod audio_output;
mod engine_viewport;
mod playback_clock;
mod secure_video;

use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, CursorStyle, FocusHandle, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, Styled, Window, WindowBounds, WindowControlArea, WindowOptions, div, img,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

use audio_output::AudioDeviceClock;
use engine_viewport::EngineViewport;
use playback_clock::{PlaybackUpdate, VideoPlaybackClock};
use secure_video::{FRAME_EVENT_CAPACITY, SecureVideoEvent, spawn_secure_video_source};

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
    viewport: Option<EngineViewport>,
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
            viewport,
            history: vec![spec.url.into()],
            history_index: 0,
        }
    }
}

struct BrowserShell {
    tabs: Vec<Tab>,
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
}

impl BrowserShell {
    fn new(address_focus: Option<FocusHandle>) -> Self {
        Self {
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
        }
    }

    fn select_tab(&mut self, index: usize) {
        if self.tabs.get(index).is_some_and(|tab| tab.open) {
            self.active = index;
            self.address = self.tabs[index].url.clone();
            self.committed_address = self.address.clone();
            self.address_selected = false;
            self.downloads_open = false;
            self.menu_open = false;
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
        self.select_tab(self.tabs.len() - 1);
        self.address.clear();
        self.address_selected = true;
    }

    fn close_tab(&mut self, index: usize) {
        if !self.tabs.get(index).is_some_and(|tab| tab.open) {
            return;
        }
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
    }

    fn on_address_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "enter" => {
                self.commit_navigation();
                window.blur();
            }
            "escape" => {
                self.address = self.committed_address.clone();
                self.address_selected = false;
                window.blur();
            }
            "backspace" => {
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

    fn commit_navigation(&mut self) {
        let Some(address) = normalize_address(&self.address) else {
            self.address = self.committed_address.clone();
            return;
        };
        self.load_navigation(&address);
        let tab = &mut self.tabs[self.active];
        if tab.history[tab.history_index] != self.committed_address {
            tab.history.truncate(tab.history_index + 1);
            tab.history.push(self.committed_address.clone());
            tab.history_index = tab.history.len() - 1;
        }
    }

    fn load_navigation(&mut self, address: &str) {
        let (viewport, final_address) = match EngineViewport::load_address(address) {
            Ok((viewport, final_address)) => (viewport, final_address),
            Err(error) => (EngineViewport::error(address, &error), address.to_string()),
        };
        self.address = final_address.clone();
        self.committed_address = final_address.clone();
        let tab = &mut self.tabs[self.active];
        tab.url = final_address.clone();
        tab.kind = TabKind::Engine;
        tab.viewport = Some(viewport);
        if tab.title.starts_with("New tab") {
            tab.title = address_title(&final_address);
            tab.mono = site_initial(&final_address).to_string();
        }
        self.address_selected = false;
    }

    fn go_back(&mut self) {
        let tab = &mut self.tabs[self.active];
        if tab.history_index == 0 {
            return;
        }
        tab.history_index -= 1;
        let address = tab.history[tab.history_index].clone();
        self.load_navigation(&address);
    }

    fn go_forward(&mut self) {
        let tab = &mut self.tabs[self.active];
        if tab.history_index + 1 >= tab.history.len() {
            return;
        }
        tab.history_index += 1;
        let address = tab.history[tab.history_index].clone();
        self.load_navigation(&address);
    }

    fn can_go_back(&self) -> bool {
        self.tabs[self.active].history_index > 0
    }

    fn can_go_forward(&self) -> bool {
        let tab = &self.tabs[self.active];
        tab.history_index + 1 < tab.history.len()
    }

    fn reload(&mut self) {
        let address = self.committed_address.clone();
        self.load_navigation(&address);
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
    }

    fn toggle_downloads(&mut self) {
        self.downloads_open = !self.downloads_open;
        self.menu_open = false;
    }

    fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        self.downloads_open = false;
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
            .child(
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
                    .child(tab.mono),
            )
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
                    .child(tab.title),
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
        let bookmarked = self.tabs[self.active].bookmarked;
        let back_enabled = self.can_go_back();
        let forward_enabled = self.can_go_forward();
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
                        this.reload();
                        cx.notify();
                    }))
                    .child(Self::icon_button("↻", true))
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
                        if let Some(focus) = this.address_focus.as_ref() {
                            focus.focus(window, cx);
                        }
                        this.address_selected = true;
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(Self::on_address_key_down))
                    .child("▣")
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(15.0))
                    .rounded_sm()
                    .bg(rgb(tab.hue))
                    .text_color(rgb(0xffffff))
                    .text_size(px(8.0))
                    .child(tab.mono),
            )
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
                        if let Some(focus) = this.address_focus.as_ref() {
                            focus.focus(window, cx);
                        }
                        this.address_selected = true;
                        cx.notify();
                    }))
                    .child("⌕  Search tabs"),
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
                        if let Some(focus) = this.address_focus.as_ref() {
                            focus.focus(window, cx);
                        }
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
            .child(
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
            .when(!self.downloads_open, |panel| panel.hidden())
    }

    fn browser_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                        if let Some(focus) = this.address_focus.as_ref() {
                            focus.focus(window, cx);
                        }
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
                    .id("menu-downloads")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu_open = false;
                        this.downloads_open = true;
                        cx.notify();
                    }))
                    .child(item("Downloads", "Ctrl+J")),
            )
            .child(div().h(px(1.0)).my_1().bg(rgb(RULE)))
            .child(item("Zoom", "100%"))
            .child(item("Print…", "Ctrl+P"))
            .child(div().h(px(1.0)).my_1().bg(rgb(RULE)))
            .child(item("Settings", ""))
            .child(item("About Amazing", ""))
            .when(!self.menu_open, |menu| menu.hidden())
    }

    fn page(&self) -> impl IntoElement {
        let tab = &self.tabs[self.active];
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
        div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .when_some(tab.viewport.as_ref(), |page, viewport| {
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
                                .id("engine-viewport-scroll")
                                .size_full()
                                .overflow_y_scroll()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(SURFACE))
                                .shadow_md()
                                .child(img(viewport.image.clone()).w_full()),
                        )
                        .child(
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
                        ),
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
                    .child("win-001  ·  GPUI 0.2.2  ·  ThinLTO"),
            )
    }
}

impl Render for BrowserShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = &self.tabs[self.active];
        let active_title = active.title.clone();
        let active_mono = active.mono.clone();
        let active_hue = active.hue;
        let sidebar = self.sidebar;
        div()
            .relative()
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
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(30.0))
                            .mb_1()
                            .mr_2()
                            .rounded_md()
                            .text_color(rgb(ICON))
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
                                        if let Some(focus) = this.address_focus.as_ref() {
                                            focus.focus(window, cx);
                                        }
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
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(17.0))
                                        .rounded_sm()
                                        .bg(rgb(active_hue))
                                        .text_color(rgb(0xffffff))
                                        .text_size(px(9.0))
                                        .child(active_mono),
                                )
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
                                .child(self.page()),
                        )
                    })
                    .when(!sidebar, |body| body.child(self.page())),
            )
            .child(self.downloads_panel(cx))
            .child(self.browser_menu(cx))
    }
}

fn main() {
    let start_with_sidebar = std::env::args().any(|argument| argument == "--sidebar");
    let start_with_video = std::env::args().any(|argument| argument == "--video");
    let start_with_secure_video = std::env::args().any(|argument| argument == "--secure-video");
    let (secure_receiver, secure_status) = if start_with_secure_video {
        match spawn_secure_video_source() {
            Ok(receiver) => (Some(receiver), "CONNECTING SECURE VIDEO…".to_string()),
            Err(error) => (None, format!("SECURE VIDEO ERROR  ·  {error}")),
        }
    } else {
        (None, String::new())
    };
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1080.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(move |cx| {
                    let mut shell = BrowserShell::new(Some(cx.focus_handle().tab_stop(true)));
                    cx.spawn(async move |this, cx| {
                        let started = Instant::now();
                        let mut delay_ms = if start_with_secure_video {
                            SECURE_VIDEO_POLL_MS
                        } else {
                            500
                        };
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(delay_ms))
                                .await;
                            match this.update(cx, |shell: &mut BrowserShell, cx| {
                                let fallback_now_ms =
                                    started.elapsed().as_millis().min(u64::MAX as u128) as u64;
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
                                Ok(next_delay) => delay_ms = next_delay,
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
                    shell
                })
            },
        )
        .expect("open Amazing GPUI shell");
        cx.activate(true);
    });
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
mod tests {
    use super::*;

    #[test]
    fn reference_shell_state_starts_on_status_with_specs_collapsed() {
        let shell = BrowserShell::new(None);
        assert_eq!(shell.tabs.len(), 6);
        assert_eq!(shell.active, 0);
        assert!(!shell.engine_collapsed);
        assert!(shell.specs_collapsed);
        assert!(!shell.sidebar);
        assert_eq!(shell.tabs[0].url, "amazing://status");
    }

    #[test]
    fn reference_groups_keep_engine_and_spec_tabs_distinct() {
        let shell = BrowserShell::new(None);
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
        let mut shell = BrowserShell::new(None);
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

        let mut shell = BrowserShell::new(None);
        shell.address = "example.test/docs".to_string();
        shell.commit_navigation();
        assert_eq!(shell.active, 0);
        assert_eq!(shell.address, "https://example.test/docs");
        assert_eq!(shell.committed_address, shell.address);
    }

    #[test]
    fn decoded_video_frames_cycle_without_changing_tab_state() {
        let mut shell = BrowserShell::new(None);
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
        let mut shell = BrowserShell::new(None);
        shell.address = "first.test".to_string();
        shell.commit_navigation();
        shell.address = "second.test".to_string();
        shell.commit_navigation();
        assert_eq!(shell.tabs[0].history.len(), 3);
        assert_eq!(shell.tabs[0].history_index, 2);

        shell.go_back();
        assert_eq!(shell.committed_address, "https://first.test");
        shell.go_forward();
        assert_eq!(shell.committed_address, "https://second.test");

        shell.go_back();
        shell.address = "branch.test".to_string();
        shell.commit_navigation();
        assert_eq!(
            shell.tabs[0].history,
            [
                "amazing://status",
                "https://first.test",
                "https://branch.test"
            ]
        );
        assert_eq!(shell.tabs[0].history_index, 2);
        shell.go_forward();
        assert_eq!(shell.committed_address, "https://branch.test");
    }

    #[test]
    fn new_and_closed_tabs_keep_independent_browser_state() {
        let mut shell = BrowserShell::new(None);
        shell.add_tab();
        let new_tab = shell.active;
        assert_eq!(shell.tabs[new_tab].url, "amazing://newtab");
        assert_eq!(shell.tabs[new_tab].history, ["amazing://newtab"]);
        shell.address = "localhost:8080/example".into();
        shell.commit_navigation();
        assert_eq!(shell.tabs[new_tab].history.len(), 2);
        assert_eq!(shell.tabs[0].history, ["amazing://status"]);

        shell.close_tab(new_tab);
        assert!(!shell.tabs[new_tab].open);
        assert_eq!(shell.active, 5);
        shell.open_bookmark(0);
        assert_eq!(shell.active, 0);
        assert!(shell.tabs[0].open);
    }
}
