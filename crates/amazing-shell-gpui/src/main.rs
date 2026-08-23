#![forbid(unsafe_code)]

mod engine_viewport;

use gpui::{
    App, Bounds, Context, CursorStyle, FocusHandle, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, Styled, Window, WindowBounds, WindowControlArea, WindowOptions, div, img,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

use engine_viewport::EngineViewport;

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

#[derive(Clone)]
struct Tab {
    mono: &'static str,
    title: &'static str,
    url: &'static str,
    kicker: &'static str,
    lede: &'static str,
    hue: u32,
}

struct BrowserShell {
    tabs: Vec<Tab>,
    viewport: EngineViewport,
    address_focus: Option<FocusHandle>,
    address: String,
    committed_address: String,
    address_selected: bool,
    active: usize,
    engine_collapsed: bool,
    specs_collapsed: bool,
    sidebar: bool,
}

impl BrowserShell {
    fn new(address_focus: Option<FocusHandle>) -> Self {
        Self {
            tabs: vec![
                Tab {
                    mono: "A",
                    title: "Amazing — Phase 9 status",
                    url: "amazing://status",
                    kicker: "AMAZING://STATUS",
                    lede: "A browser engine written against the specs in pure Rust — std only in every engine core, with a hand-written tokenizer, cascade, layout, rasterizer, HTTP stack and interpreter.",
                    hue: ACCENT,
                },
                Tab {
                    mono: "W",
                    title: "WPT · flexbox + grid gate",
                    url: "localhost:8080/wpt/css/flex",
                    kicker: "LOCALHOST:8080",
                    lede: "Flex and grid reference runs for the layout gate suite.",
                    hue: 0x3f7d4e,
                },
                Tab {
                    mono: "H",
                    title: "html5lib tree-construction",
                    url: "localhost:8080/html5lib",
                    kicker: "LOCALHOST:8080",
                    lede: "Tree-construction fixtures for the tokenizer and the namespace-aware tree builder.",
                    hue: 0x2f7f8c,
                },
                Tab {
                    mono: "S",
                    title: "HTML Standard § 13.2.5",
                    url: "html.spec.whatwg.org/multipage/parsing.html",
                    kicker: "HTML.SPEC.WHATWG.ORG",
                    lede: "Tokenization states, in the same order the Amazing tokenizer implements them.",
                    hue: 0x3a6ea8,
                },
                Tab {
                    mono: "C",
                    title: "CSS Syntax Module Level 3",
                    url: "drafts.csswg.org/css-syntax-3",
                    kicker: "DRAFTS.CSSWG.ORG",
                    lede: "The token stream and parser entry points behind amazing-css.",
                    hue: 0x3a6ea8,
                },
                Tab {
                    mono: "E",
                    title: "ECMA-262",
                    url: "tc39.es/ecma262",
                    kicker: "TC39.ES",
                    lede: "Runtime semantics for the tree-walking JavaScript interpreter.",
                    hue: 0x7a5aa8,
                },
            ],
            viewport: EngineViewport::demo(),
            address_focus,
            address: "amazing://status".to_string(),
            committed_address: "amazing://status".to_string(),
            address_selected: false,
            active: 0,
            engine_collapsed: false,
            specs_collapsed: true,
            sidebar: false,
        }
    }

    fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
            self.address = self.tabs[index].url.to_string();
            self.committed_address = self.address.clone();
            self.address_selected = false;
        }
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
        match EngineViewport::load_address(&address) {
            Ok((viewport, final_address)) => {
                self.viewport = viewport;
                self.address = final_address.clone();
                self.committed_address = final_address;
            }
            Err(error) => {
                self.viewport = EngineViewport::error(&address, &error);
                self.address = address.clone();
                self.committed_address = address;
            }
        }
        self.active = 0;
        self.address_selected = false;
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

    fn icon_button(label: &'static str) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(30.0))
            .rounded_md()
            .text_color(rgb(ICON))
            .text_size(px(16.0))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(SURFACE_2_HOVER)))
            .child(label)
    }

    fn tab(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.0))
                    .rounded_full()
                    .text_size(px(12.0))
                    .child("×"),
            )
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
        div()
            .flex()
            .items_center()
            .gap_1()
            .h(px(44.0))
            .px_2()
            .bg(rgb(SURFACE))
            .children([
                Self::icon_button("←").into_any_element(),
                Self::icon_button("→").into_any_element(),
                Self::icon_button("↻").into_any_element(),
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
                    .child("☆")
                    .child("⌘"),
            )
            .child(
                div()
                    .id("sidebar-toggle")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar();
                        cx.notify();
                    }))
                    .child(Self::icon_button("◫")),
            )
            .child(Self::icon_button("⇩"))
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
            .child(Self::icon_button("⋮"))
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
                    .flex()
                    .items_center()
                    .h(px(30.0))
                    .px_2()
                    .rounded_md()
                    .bg(rgb(SURFACE_2))
                    .text_size(px(11.0))
                    .text_color(rgb(DIM))
                    .child("⌕  Search tabs"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(28.0))
                    .px_2()
                    .rounded_md()
                    .text_size(px(11.0))
                    .text_color(rgb(TEXT))
                    .hover(|style| style.bg(rgb(STRIP_HOVER)))
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
                        this.select_tab(0);
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
                sidebar.children((0..3).map(|index| self.sidebar_tab(index, cx)))
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
                sidebar.children((3..6).map(|index| self.sidebar_tab(index, cx)))
            })
    }

    fn page(&self) -> impl IntoElement {
        let tab = &self.tabs[self.active];
        let page_url = if self.active == 0 {
            self.committed_address.clone()
        } else {
            tab.url.to_string()
        };
        div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .when(self.active == 0, |page| {
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
                                .size_full()
                                .overflow_hidden()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(SURFACE))
                                .shadow_md()
                                .child(img(self.viewport.image.clone()).size_full()),
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
                                    self.viewport.width, self.viewport.height
                                )),
                        ),
                )
            })
            .when(self.active != 0, |page| {
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
                                .child(tab.kicker),
                        )
                        .child(
                            div()
                                .mt_4()
                                .font_family("Georgia")
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_size(px(46.0))
                                .line_height(gpui::relative(1.1))
                                .text_color(rgb(TEXT))
                                .child(tab.title),
                        )
                        .child(
                            div()
                                .mt_4()
                                .w(px(620.0))
                                .text_size(px(16.0))
                                .line_height(gpui::relative(1.6))
                                .text_color(rgb(DIM))
                                .child(tab.lede),
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
        let active_title = active.title;
        let active_mono = active.mono;
        let active_hue = active.hue;
        let sidebar = self.sidebar;
        div()
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
                                        this.select_tab(0);
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
                                        group
                                            .child(self.tab(0, cx))
                                            .child(self.tab(1, cx))
                                            .child(self.tab(2, cx))
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
                                        group
                                            .child(self.tab(3, cx))
                                            .child(self.tab(4, cx))
                                            .child(self.tab(5, cx))
                                    }),
                            )
                            .child(Self::icon_button("+"))
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
                            ("■", "Phase 9 board", ACCENT),
                            ("■", "WHATWG HTML", 0x3a6ea8),
                            ("■", "CSS Syntax L3", 0x3f7d4e),
                            ("■", "ECMA-262", 0x7a5aa8),
                            ("▤", "win-001 CI", 0x6b7280),
                        ]
                        .into_iter()
                        .map(|(icon, label, color)| {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .h(px(24.0))
                                .px_2()
                                .rounded_md()
                                .text_size(px(12.0))
                                .text_color(rgb(TEXT))
                                .hover(|style| style.bg(rgb(SURFACE_2)))
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
    }
}

fn main() {
    let start_with_sidebar = std::env::args().any(|argument| argument == "--sidebar");
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
                    shell.sidebar = start_with_sidebar;
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
                .map(|tab| tab.mono)
                .collect::<Vec<_>>(),
            &["A", "W", "H"]
        );
        assert_eq!(
            &shell.tabs[3..6]
                .iter()
                .map(|tab| tab.mono)
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

        let mut shell = BrowserShell::new(None);
        shell.address = "example.test/docs".to_string();
        shell.commit_navigation();
        assert_eq!(shell.active, 0);
        assert_eq!(shell.address, "https://example.test/docs");
        assert_eq!(shell.committed_address, shell.address);
    }
}

fn normalize_address(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else if value.contains("://") {
        Some(value.to_string())
    } else {
        Some(format!("https://{value}"))
    }
}
