//! A11Y-9 proof binary: mounts a real-pages fixture's accessibility tree in a
//! bare GPUI window, through `a11y_bridge`, so an AT-SPI client on the
//! sandbox (python3-pyatspi, `busctl`/`gdbus introspect` against the
//! accessibility bus) can read it as evidence. This intentionally does not
//! run the full interactive `amazing-shell` browser chrome — it proves the
//! bridge mechanism itself (see `docs/lanes/a11y_TODO.md`'s A11Y-9 entry for
//! why wiring the bridge into the live browser tab is filed as a request in
//! `.agents/shell/questions.md` instead of guessed at here).
//!
//! Usage: `amazing-a11y-proof [path/to/page.html]`, defaulting to
//! `tests/real-pages/pages/app-analytics.html`.

#[path = "../a11y_bridge.rs"]
mod a11y_bridge;

fn main() {
    use gpui::{App, Bounds, IntoElement, WindowBounds, WindowOptions, div, prelude::*, px, size};
    use gpui_platform::application;

    let default_fixture = format!(
        "{}/../../tests/real-pages/pages/app-analytics.html",
        env!("CARGO_MANIFEST_DIR")
    );
    let path = std::env::args().nth(1).unwrap_or(default_fixture);
    let html = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("amazing-a11y-proof: cannot read {path}: {error}"));

    let dom = amazing_html::parse(&html);
    let sheet = amazing_css::parse_stylesheet("");
    let styled = amazing_css::compute(&dom, &[sheet]);
    let text = amazing_text::FallbackTextSystem::new();
    let layout = amazing_layout::layout_styled_document(
        &dom,
        &styled,
        &text,
        &amazing_layout::NoReplacedElements,
        1024.0,
        Some(768.0),
    );
    let tree = amazing_a11y::build(&dom, &styled, &layout);
    eprintln!(
        "amazing-a11y-proof: built an accessibility tree with {} nodes from {path}",
        tree.nodes().len()
    );

    struct ProofWindow {
        tree: amazing_a11y::AxTree,
    }

    impl gpui::Render for ProofWindow {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .id("a11y-proof-root")
                .role(gpui::accesskit::Role::Document)
                .size_full()
                .child(a11y_bridge::ax_tree_element(&self.tree))
        }
    }

    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(400.), px(300.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| cx.new(|_| ProofWindow { tree }),
        )
        .expect("open a11y proof window");
        cx.activate(true);
    });
}
