//! Driver adapter for `amazing-a11y`: builds the accessibility tree from the
//! live document (parsed DOM + computed style + layout tree) and exposes a
//! snapshot API. This is A11Y-6, the driver-adoption ticket filed against
//! this crate in `.agents/a11y/questions.md`.
//!
//! Per the `AGENTS.md` driver-adapter-per-lane rule, this file is this
//! lane's entire footprint in the shared driver: `main.rs` gains one `mod`
//! line and one CLI dispatch line, nothing else.
//!
//! [`build_ax_tree`] is the seam a future CDP `Accessibility.getFullAXTree`
//! handler in `amazing-devtools` calls once the driver holds a live
//! document's DOM, computed style and layout tree (e.g. from a `gui` frame
//! or a `BrowserDomHost`); it is a thin wrapper over `amazing_a11y::build`.
//! [`ax_tree_for_page`] drives the same headless parse/style/layout pipeline
//! `amazing layout`/`amazing render` use, for the `a11y-tree` CLI
//! subcommand and this file's own test.

use std::process::ExitCode;

use amazing_a11y::AxTree;
use amazing_css::StyledDom;
use amazing_dom::{Dom, NodeIdx};
use amazing_layout::LayoutBox;

use crate::document::{PageError, parse_document_bytes};
use crate::page::{collect_author_css, load_page_media_in_viewport, load_page_text_from_sheets};

/// Builds the accessibility tree for a document whose DOM, computed style
/// and layout are already available — the driver-adoption seam. Devtools'
/// future `Accessibility.getFullAXTree` backs onto `AxTree::snapshot()` from
/// the tree this returns.
pub(crate) fn build_ax_tree(dom: &Dom, styled: &StyledDom, layout: &LayoutBox) -> AxTree {
    amazing_a11y::build(dom, styled, layout)
}

/// Parses, styles (author `<style>`/`<link rel=stylesheet>`, this page's
/// viewport), loads replaced content and fonts, and lays out `html_path`
/// exactly as `amazing render`/`amazing layout` do, then builds its
/// accessibility tree. `fixed_height` behaves as it does for those commands:
/// `None` sizes to content.
pub(crate) fn ax_tree_for_page(
    html_path: &str,
    width: f32,
    fixed_height: Option<u32>,
) -> Result<AxTree, PageError> {
    let html = std::fs::read(html_path)
        .map_err(|error| format!("cannot read {html_path}: {error}"))?;
    let dom = parse_document_bytes(&html, html_path, None)?.dom;

    let mut sources = Vec::new();
    collect_author_css(&dom, NodeIdx(0), html_path, &mut sources)?;
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
    let media = load_page_media_in_viewport(&dom, html_path, &styled, width, 1.0);
    let page_text = load_page_text_from_sheets(&font_sheets, html_path);
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

/// `amazing a11y-tree <page.html> [viewport_width]`: headless dump of the
/// accessibility tree — one indented `role "name" (description) states…`
/// line per node (see `SchemaSnapshot::dump`).
pub(crate) fn cmd_a11y_tree(args: &[String]) -> ExitCode {
    let Some(html_path) = args.first() else {
        eprintln!("usage: amazing a11y-tree <page.html> [viewport_width]");
        return ExitCode::FAILURE;
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(800.0);
    match ax_tree_for_page(html_path, width, None) {
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
