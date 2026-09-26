//! Driver adapter for `amazing-a11y`: builds the accessibility tree from the
//! live document (parsed DOM + computed style + layout tree) and exposes a
//! snapshot API. This is A11Y-6, the driver-adoption ticket filed against
//! this crate in `.agents/a11y/questions.md`.
//!
//! Per the `AGENTS.md` driver-adapter-per-lane rule, this file is this
//! lane's entire footprint in the shared driver: `main.rs` gains one `mod`
//! line and one CLI dispatch line, nothing else.
//!
//! [`build_ax_tree`] is the seam CDP `Accessibility.getFullAXTree`
//! (`crates/amazing/src/runtime_host_devtools.rs`, CDP-9) calls once the
//! driver holds a live document's DOM, computed style and layout tree (e.g.
//! from a `gui` frame or a `BrowserDomHost`); it is a thin wrapper over
//! `amazing_a11y::build`. [`diff_ax_tree`] is the A11Y-8 companion: the seam
//! `Accessibility.nodesUpdated` (CDP-15, `amazing-devtools::host::AxNode`/
//! `drain_ax_nodes_updated`) needs once the driver holds a mutation
//! checkpoint's DOM/style/layout plus the tree it built last time — a thin
//! wrapper over `amazing_a11y::rebuild_and_diff`, which rebuilds the whole
//! tree and diffs it against the one passed in, per node and per changed
//! aspect (added, removed, name/description/role/state/property/
//! active-descendant changed — see `amazing_a11y::event`'s module doc for
//! why a whole-tree rebuild still yields a correct incremental diff, e.g. for
//! a name that changed because a node it references did). Hooking
//! `drain_ax_nodes_updated` onto this is filed in
//! `.agents/devtools/questions.md` since `runtime_host_devtools.rs` is the
//! devtools lane's file, not this lane's. [`ax_tree_for_page`] drives the
//! same headless parse/style/layout pipeline `amazing layout`/`amazing
//! render` use, for the `a11y-tree` CLI subcommand and this file's own test.
//!
//! A11Y-14 adds [`AxTreeWatcher`], the stateful per-document wrapper around
//! [`diff_ax_tree`] that a shell or devtools panel drains for a live change-
//! event stream (see its own doc comment for the call-site handoff), and
//! confirms — by running `amazing wpt-harness` against the pinned WPT files
//! directly (not just this crate's own tests) — that `amazing_a11y`'s
//! shadow-DOM-aware content collection (added this ticket in
//! `amazing-a11y::accname`) plus the driver's existing `get_computed_label`/
//! `get_computed_role` testdriver hooks (`runtime_host_integrator_assist.rs`,
//! IA-TESTDRIVER) already make `accname/name/shadowdom/{basic,slot}.html`
//! pass with no further wiring here: those files build their tested content
//! with real `attachShadow`/`<slot>` calls that only need to run through the
//! real DOM+JS engine once, not be re-executed by this file.
//! `html-aam/roles-dynamic-switch.tentative.window.js` mostly passes through
//! that same existing hook too, except for the `switch` IDL attribute itself
//! (`input.switch = true`), which needs a WebIDL boolean-attribute
//! reflection this lane cannot add without editing `amazing-bindings` — see
//! `.agents/a11y/questions.md`. `wai-aria/aria-actions/idl-reflection.
//! tentative.html` needs `ariaActionsElements` (an element-reference/
//! FrozenArray reflection over custom elements and shadow trees), also filed
//! there rather than attempted here.

use std::collections::VecDeque;
use std::process::ExitCode;

use amazing_a11y::{AxEvent, AxTree};
use amazing_css::StyledDom;
use amazing_dom::{Dom, NodeIdx};
use amazing_layout::LayoutBox;

use crate::document::{DocumentKind, PageError, parse_document_bytes};
use crate::page::{collect_author_css, load_page_media_in_viewport, load_page_text_from_sheets};

/// Builds the accessibility tree for a document whose DOM, computed style
/// and layout are already available — the driver-adoption seam. Devtools'
/// future `Accessibility.getFullAXTree` backs onto `AxTree::snapshot()` from
/// the tree this returns.
pub(crate) fn build_ax_tree(dom: &Dom, styled: &StyledDom, layout: &LayoutBox) -> AxTree {
    amazing_a11y::build(dom, styled, layout)
}

/// Rebuilds the accessibility tree after a DOM mutation checkpoint and diffs
/// it against `previous` (the tree from the checkpoint before), returning the
/// new tree plus the per-node, per-aspect changes between them. A caller
/// with continuous access to a document's DOM/style/layout — a `gui` frame,
/// `BrowserDomHost`, or a future `Accessibility.nodesUpdated` handler in
/// `runtime_host_devtools.rs` — calls this once per batch of applied
/// mutations, the same granularity a real browser coalesces mutation-observer
/// callbacks into, and keeps the returned tree as `previous` for the next
/// call.
pub(crate) fn diff_ax_tree(
    dom: &Dom,
    styled: &StyledDom,
    layout: &LayoutBox,
    previous: &AxTree,
) -> (AxTree, Vec<AxEvent>) {
    amazing_a11y::rebuild_and_diff(dom, styled, layout, previous)
}

/// A11Y-14: the stateful half of [`diff_ax_tree`] — holds the tree from the
/// last checkpoint and a drainable queue of the events every checkpoint
/// since has produced, so a caller with continuous access to a live
/// document (a `gui` frame, `BrowserDomHost`, or `amazing wpt-harness`'s
/// `test_driver` hooks) does not have to thread `previous` through itself
/// call to call, or poll for changes it might otherwise miss between two
/// reads. One watcher per live document/tab; `record_checkpoint` is meant to
/// be called at the same granularity `deliver_mutations`/
/// `deliver_custom_element_reactions` already coalesce mutation-observer
/// callbacks to (see `runtime_binding_host.rs`'s `deliver_mutations`) —
/// once per batch of applied mutations, *after* that batch's style and
/// layout have been refreshed, not once per individual attribute/child-list
/// change.
///
/// Devtools' future `Accessibility.nodesUpdated` (CDP-15,
/// `amazing-devtools::host::drain_ax_nodes_updated`) and a shell's live
/// accessibility panel are both consumers of [`Self::drain_events`]; wiring
/// either to this type's `record_checkpoint` is filed in
/// `.agents/a11y/questions.md` / the corresponding lane's `questions.md`,
/// since the call site (wherever a live document's style+layout are
/// actually refreshed) lives in another lane's file, not this one — the
/// same reasoning [`diff_ax_tree`]'s own doc comment already gives for CDP.
#[derive(Default)]
pub(crate) struct AxTreeWatcher {
    current: Option<AxTree>,
    pending: VecDeque<AxEvent>,
}

impl AxTreeWatcher {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The tree as of the last [`Self::record_checkpoint`] call, or `None`
    /// before the first one.
    pub(crate) fn current_tree(&self) -> Option<&AxTree> {
        self.current.as_ref()
    }

    /// Rebuilds the tree from `dom`/`styled`/`layout` and diffs it against
    /// whatever checkpoint came before (or builds fresh, with no events, on
    /// the first call for this watcher). Newly produced events are appended
    /// to the pending queue in the order they occurred; call
    /// [`Self::drain_events`] to take them.
    pub(crate) fn record_checkpoint(&mut self, dom: &Dom, styled: &StyledDom, layout: &LayoutBox) {
        let (next, events) = match &self.current {
            Some(previous) => diff_ax_tree(dom, styled, layout, previous),
            None => (build_ax_tree(dom, styled, layout), Vec::new()),
        };
        self.pending.extend(events);
        self.current = Some(next);
    }

    /// Takes every event queued since the last drain, oldest first. Once
    /// taken, an event is not replayed to a later drain — the same
    /// at-most-once delivery a `MutationObserver` callback queue gives.
    pub(crate) fn drain_events(&mut self) -> Vec<AxEvent> {
        self.pending.drain(..).collect()
    }

    /// Whether anything is waiting to be drained, for a caller that only
    /// wants to do work (e.g. wake a devtools panel) when there is some.
    pub(crate) fn has_pending_events(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Parses, styles (author `<style>`/`<link rel=stylesheet>`, this page's
/// viewport), loads replaced content and fonts, and lays out `page`
/// exactly as `amazing render`/`amazing layout` do, then builds its
/// accessibility tree. `fixed_height` behaves as it does for those commands:
/// `None` sizes to content. `page` may be a local file path or an
/// `http(s)://` URL, fetched the same way `amazing render-page` does
/// (folded in from the now-removed `amazing axtree` command, per the
/// integrator's request to keep one `a11y-tree` subcommand).
pub(crate) fn ax_tree_for_page(
    page: &str,
    width: f32,
    fixed_height: Option<u32>,
) -> Result<AxTree, PageError> {
    let remote = page.starts_with("http://") || page.starts_with("https://");
    let (base_url, html, content_type) = if remote {
        let (base_url, html, content_type) = crate::cmd::real_pages::fetch_document(page)
            .map_err(|error| PageError::Render(format!("fetch: {error}")))?;
        (base_url, html, content_type)
    } else {
        let html = std::fs::read(page)
            .map_err(|error| format!("cannot read {page}: {error}"))?;
        (page.to_owned(), html, None)
    };
    let document = parse_document_bytes(&html, &base_url, content_type.as_deref())?;
    if document.kind != DocumentKind::Html {
        return Err(PageError::Unsupported("XML documents are not supported yet"));
    }
    let dom = document.dom;

    let mut sources = Vec::new();
    collect_author_css(&dom, NodeIdx(0), &base_url, &mut sources)?;
    let font_sheets: Vec<(String, String)> = sources
        .iter()
        .map(|(source, url, _)| (source.clone(), url.clone()))
        .collect();
    let sheets: Vec<amazing_css::Stylesheet> =
        sources.into_iter().map(|(_, _, sheet)| sheet).collect();

    let environment = amazing_css::MediaEnvironment {
        viewport_width_px: width.max(1.0),
        viewport_height_px: fixed_height.unwrap_or(600).max(1) as f32,
        ..amazing_css::MediaEnvironment::default()
    };
    let styled = amazing_css::compute_with_environment(&dom, &sheets, &environment);
    let media = load_page_media_in_viewport(&dom, &base_url, &styled, width, 1.0);
    let page_text = load_page_text_from_sheets(&font_sheets, &base_url);
    let layout = amazing_layout::layout_styled_document(
        &dom,
        &styled,
        &page_text.provider,
        &media,
        width,
        fixed_height.map(|height| height as f32),
    );
    Ok(build_ax_tree(&dom, &styled, &layout))
}

/// `amazing a11y-tree <page.html|url> [viewport_width]`: headless dump of the
/// accessibility tree — one indented `role "name" (description) states…`
/// line per node (see `SchemaSnapshot::dump`). Accepts a local file path or
/// an `http(s)://` URL, fetched like `amazing render-page`.
pub(crate) fn cmd_a11y_tree(args: &[String]) -> ExitCode {
    let Some(page) = args.first() else {
        eprintln!("usage: amazing a11y-tree <page.html|url> [viewport_width]");
        return ExitCode::FAILURE;
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(800.0);
    match ax_tree_for_page(page, width, None) {
        Ok(tree) => {
            print!("{}", tree.snapshot().dump());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing a11y-tree: {error}");
            error.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_a11y::Role;

    /// `tests/real-pages/pages/app-analytics.html`: a header with a nav and
    /// an `alt`-bearing logo image, an `<h1>`, a data table, and a footer.
    /// Every asset it references (`../assets/base.css`, `../assets/logo.svg`,
    /// the deferred scripts) is a local file, so this exercises the same
    /// author-CSS and replaced-content loading `amazing render-page` uses on
    /// this fixture in the IA-REAL-PAGES corpus, without spawning a process.
    fn fixture_path() -> String {
        format!(
            "{}/../../tests/real-pages/pages/app-analytics.html",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// Builds a document's DOM/style/layout the way `ax_tree_for_page` does,
    /// but from an inline HTML string with no author CSS or replaced
    /// content — enough for [`diff_ax_tree`]'s own test, which only needs a
    /// name/role to change between two checkpoints.
    fn build_pipeline(html: &str) -> (Dom, StyledDom, LayoutBox) {
        let dom = amazing_html::parse(html);
        let styled = amazing_css::compute(&dom, &[]);
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_styled_document(
            &dom,
            &styled,
            &text,
            &amazing_layout::NoReplacedElements,
            800.0,
            Some(600.0),
        );
        (dom, styled, layout)
    }

    #[test]
    fn diff_ax_tree_reports_a_mutation_checkpoint() {
        use amazing_a11y::AxEventKind;

        let (dom, styled, layout) = build_pipeline("<div role=\"status\">old</div>");
        let before = build_ax_tree(&dom, &styled, &layout);

        let (dom, styled, layout) = build_pipeline("<div role=\"status\">new</div>");
        let (after, events) = diff_ax_tree(&dom, &styled, &layout, &before);

        assert!(after.node(after.root().unwrap()).is_some());
        let changed = events
            .iter()
            .find(|event| event.kind == AxEventKind::NameChanged)
            .unwrap_or_else(|| panic!("expected a NameChanged event, got {events:?}"));
        assert_eq!(changed.previous_name.as_deref(), Some("old"));
        assert_eq!(changed.name, "new");
    }

    /// A11Y-14: a watcher's first checkpoint has no `previous`, so it builds
    /// fresh and reports no events — matching a freshly attached observer
    /// seeing no mutations before it started watching.
    #[test]
    fn watcher_first_checkpoint_reports_no_events() {
        let mut watcher = AxTreeWatcher::new();
        assert!(watcher.current_tree().is_none());

        let (dom, styled, layout) = build_pipeline("<div role=\"status\">hello</div>");
        watcher.record_checkpoint(&dom, &styled, &layout);

        assert!(watcher.current_tree().is_some());
        assert!(!watcher.has_pending_events());
        assert!(watcher.drain_events().is_empty());
    }

    /// A11Y-14: successive checkpoints accumulate events across calls until
    /// drained, and a drain empties the queue without losing anything that
    /// arrives after — the same at-most-once, no-polling-gap contract a
    /// `MutationObserver` callback queue gives.
    #[test]
    fn watcher_accumulates_and_drains_events_across_checkpoints() {
        use amazing_a11y::AxEventKind;

        let mut watcher = AxTreeWatcher::new();
        let (dom, styled, layout) = build_pipeline("<div role=\"status\">old</div>");
        watcher.record_checkpoint(&dom, &styled, &layout);
        assert!(watcher.drain_events().is_empty(), "first checkpoint has nothing to diff against");

        let (dom, styled, layout) = build_pipeline("<div role=\"status\">new</div>");
        watcher.record_checkpoint(&dom, &styled, &layout);
        assert!(watcher.has_pending_events());

        let (dom, styled, layout) = build_pipeline(
            "<div role=\"status\">new</div><button>added</button>",
        );
        watcher.record_checkpoint(&dom, &styled, &layout);

        let events = watcher.drain_events();
        assert!(
            events.iter().any(|event| event.kind == AxEventKind::NameChanged),
            "expected a NameChanged event from the first->second checkpoint, got {events:?}"
        );
        assert!(
            events.iter().any(|event| event.kind == AxEventKind::Added),
            "expected an Added event from the second->third checkpoint, got {events:?}"
        );
        assert!(watcher.drain_events().is_empty(), "drain must empty the queue");

        // A live-region announcement (LiveRegionInfo on the changed status
        // node) is carried on the event itself, not a separate stream: a
        // consumer filters `drain_events()` by `event.live.is_some()` for
        // "what should be announced," matching how `event::diff` already
        // attaches `live` per A11Y-8.
        let (dom, styled, layout) = build_pipeline(
            "<div role=\"status\">new</div><button>added</button>",
        );
        watcher.record_checkpoint(&dom, &styled, &layout);
        assert!(watcher.drain_events().is_empty(), "no change since the last checkpoint");
    }

    /// A11Y-14: removing one of two siblings is reported as a `Removed`
    /// event naming the removed node, not merely a size change on its
    /// former parent — even though the watcher rebuilds the whole tree on
    /// every checkpoint rather than tracking a diff incrementally
    /// node-by-node (per `amazing_a11y::event`'s own module doc on why a
    /// whole-tree rebuild still yields a correct incremental diff).
    #[test]
    fn watcher_reports_node_removal() {
        use amazing_a11y::AxEventKind;

        let mut watcher = AxTreeWatcher::new();
        let (dom, styled, layout) =
            build_pipeline("<button>keep</button><button>bye</button>");
        watcher.record_checkpoint(&dom, &styled, &layout);
        watcher.drain_events();

        let (dom, styled, layout) = build_pipeline("<button>keep</button>");
        watcher.record_checkpoint(&dom, &styled, &layout);

        let events = watcher.drain_events();
        assert!(
            events
                .iter()
                .any(|event| event.kind == AxEventKind::Removed && event.name == "bye"),
            "expected a Removed event for the \"bye\" button, got {events:?}"
        );
    }

    #[test]
    fn real_page_fixture_produces_a_populated_accessibility_tree() {
        let tree = ax_tree_for_page(&fixture_path(), 1024.0, None)
            .expect("app-analytics.html should build an accessibility tree");
        let snapshot = tree.snapshot();
        let dump = snapshot.dump();
        assert!(!dump.is_empty(), "snapshot dump was empty");

        let roles = |role: Role| snapshot.nodes.iter().filter(|node| node.role == role).count();
        assert_eq!(roles(Role::Navigation), 1, "header <nav> should expose a navigation landmark\n{dump}");
        assert_eq!(roles(Role::Contentinfo), 1, "<footer> should expose a contentinfo landmark\n{dump}");
        assert_eq!(roles(Role::Table), 1, "the data table should be exposed\n{dump}");
        assert_eq!(roles(Role::Row), 9, "1 header row + 8 data rows\n{dump}");
        assert_eq!(roles(Role::Columnheader), 3, "Name/Value/Status\n{dump}");

        let heading = snapshot
            .nodes
            .iter()
            .find(|node| node.role == Role::Heading)
            .unwrap_or_else(|| panic!("no heading in tree\n{dump}"));
        assert_eq!(heading.name, "Analytics");

        let logo = snapshot
            .nodes
            .iter()
            .find(|node| node.role == Role::Img)
            .unwrap_or_else(|| panic!("no image in tree\n{dump}"));
        assert_eq!(logo.name, "logo", "logo's accessible name should come from alt");

        let menu_button = snapshot
            .nodes
            .iter()
            .find(|node| node.role == Role::Button)
            .unwrap_or_else(|| panic!("no button in tree\n{dump}"));
        assert_eq!(menu_button.name, "Menu");
    }
}
