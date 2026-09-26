//! Tree construction fixtures: pruning (display/visibility/hidden/aria-hidden),
//! presentational promotion, generic-container flattening, and the snapshot API.

use crate::{Role, build_document};
use crate::test_support::{
    ax_of_tag, count_role, document, document_with_css, dom_of_tag, first_named_role, first_role,
    shape,
};

fn all_names(doc: &crate::AxDocument) -> Vec<String> {
    doc.tree.nodes().iter().map(|node| node.name.clone()).collect()
}

#[test]
fn display_none_prunes_subtree() {
    let doc = document_with_css(
        "<div><span class=\"gone\">Gone</span>Shown</div>",
        ".gone{display:none}",
    );
    let names = all_names(&doc);
    assert!(names.iter().any(|n| n == "Shown"));
    assert!(!names.iter().any(|n| n == "Gone"));
}

#[test]
fn hidden_attribute_prunes_subtree() {
    let doc = document("<div>Keep<span hidden>Drop</span></div>");
    let names = all_names(&doc);
    assert!(names.iter().any(|n| n == "Keep"));
    assert!(!names.iter().any(|n| n == "Drop"));
}

#[test]
fn aria_hidden_prunes_subtree() {
    let doc = document("<div aria-hidden=\"true\"><span>Secret</span></div><p>Public</p>");
    let names = all_names(&doc);
    assert!(!names.iter().any(|n| n == "Secret"));
    assert!(names.iter().any(|n| n == "Public"));
}

#[test]
fn visibility_hidden_prunes_and_a_descendant_can_override() {
    let doc = document_with_css(
        "<div class=\"hid\">HiddenText<span class=\"vis\">Shown</span></div>",
        ".hid{visibility:hidden}.vis{visibility:visible}",
    );
    let names = all_names(&doc);
    assert!(!names.iter().any(|n| n == "HiddenText"), "hidden text leaked: {names:?}");
    assert!(names.iter().any(|n| n == "Shown"), "override lost: {names:?}");
}

#[test]
fn presentational_role_promotes_children() {
    let doc = document("<div role=\"presentation\"><button>Go</button></div>");
    let div = dom_of_tag(&doc, "div").unwrap();
    assert!(doc.tree.find_dom(div).is_none(), "presentational div exposed");
    assert!(first_named_role(&doc, Role::Button, "Go").is_some());
}

#[test]
fn text_only_generic_containers_are_flattened() {
    let doc = document("<div>x</div>");
    let div = dom_of_tag(&doc, "div").unwrap();
    let body = dom_of_tag(&doc, "body").unwrap();
    assert!(doc.tree.find_dom(div).is_none(), "text-only div exposed");
    assert!(doc.tree.find_dom(body).is_none(), "empty generic body exposed");
    assert_eq!(count_role(&doc, Role::StaticText), 1);
}

#[test]
fn focusable_or_aria_generics_are_kept() {
    let doc = document("<div tabindex=\"0\"></div>");
    assert!(ax_of_tag(&doc, "div").is_some());
    assert!(ax_of_tag(&doc, "div").unwrap().states.focusable);

    let doc = document("<div aria-live=\"polite\"></div>");
    assert!(ax_of_tag(&doc, "div").is_some());
}

#[test]
fn non_rendered_elements_have_no_accessibility_node() {
    let doc = document(
        "<head><title>t</title><style>p{}</style>\
         <meta charset=\"utf-8\"><script>1</script></head>\
         <body><p>Text</p></body>",
    );
    assert!(all_names(&doc).iter().any(|n| n == "Text"));
    for tag in ["title", "style", "meta", "script"] {
        if let Some(node) = dom_of_tag(&doc, tag) {
            assert!(
                doc.tree.find_dom(node).is_none(),
                "<{tag}> should not be exposed"
            );
        }
    }
}

#[test]
fn text_nodes_become_static_text() {
    let doc = document("<p>Hello <b>world</b></p>");
    assert_eq!(count_role(&doc, Role::StaticText), 2);
    let names: Vec<&str> = doc
        .tree
        .nodes()
        .iter()
        .filter(|n| n.role == Role::StaticText)
        .map(|n| n.name.as_str())
        .collect();
    assert!(names.contains(&"Hello"));
    assert!(names.contains(&"world"));
}

#[test]
fn blank_text_nodes_are_dropped() {
    let doc = document("<p>   </p>");
    assert_eq!(count_role(&doc, Role::StaticText), 0);
}

#[test]
fn document_root_and_preorder() {
    let doc = document("<h1>H</h1><p>P</p>");
    let root = doc.tree.root().unwrap();
    assert_eq!(doc.tree.node(root).unwrap().role, Role::Document);
    let order = doc.tree.walk(None);
    assert_eq!(order.first().copied(), Some(root));
    assert_eq!(order.len(), doc.tree.len());
}

#[test]
fn find_dom_maps_back_to_the_arena() {
    let doc = document("<button>Go</button>");
    let button = dom_of_tag(&doc, "button").unwrap();
    let id = doc.tree.find_dom(button).unwrap();
    assert_eq!(doc.tree.node(id).unwrap().name, "Go");
}

#[test]
fn children_and_parents_are_consistent() {
    let doc = document("<ul><li>One</li><li>Two</li></ul>");
    let list = first_role(&doc, Role::List).unwrap();
    assert_eq!(list.children.len(), 2);
    for child in &list.children {
        let node = doc.tree.node(*child).unwrap();
        assert_eq!(node.parent, Some(list.id));
        assert_eq!(node.role, Role::Listitem);
    }
}

#[test]
fn snapshot_detaches_and_dumps() {
    let doc = document("<button aria-pressed=\"false\">Play</button>");
    let snapshot = doc.tree.snapshot();
    let root = snapshot.root.unwrap();
    assert!(snapshot.node(root).is_some());
    let button = snapshot
        .nodes
        .iter()
        .find(|node| node.role == Role::Button)
        .unwrap();
    assert_eq!(button.name, "Play");
    assert_eq!(button.states.pressed, crate::Tristate::False);
    assert!(snapshot.node(button.parent.unwrap()).is_some());
    assert!(snapshot.dump().contains("button \"Play\""));
}

#[test]
fn shape_lists_role_and_name_pairs() {
    let doc = document("<button>Go</button>");
    let shape = shape(&doc);
    assert!(shape.iter().any(|(role, name)| *role == "button" && name == "Go"));
}

/// A11Y-13: the tree the shell (`a11y_bridge`) and devtools (a future CDP
/// `Accessibility.getFullAXTree`) lanes consume is a rebuild-from-scratch on
/// every call (Stage 1 has no incremental style/layout invalidation — see
/// `event`'s module doc), so its `SchemaSnapshot` must serialize identically
/// every time for the same input, or every consumer downstream of it
/// (`a11y_bridge`'s AccessKit-identity-by-`dom_node` diffing, a devtools
/// CDP snapshot cache) would see spurious churn on a page that never
/// changed. Builds `tests/real-pages/pages/app-analytics.html` — a real,
/// non-trivial fixture (nav landmark, heading, an 8-row data table, footer;
/// every asset it references is local) — twice from the same bytes and
/// checks the two `SchemaSnapshot`s are `==` (structural equality, covering
/// every node's id/role/name/states/properties/bounds/children/parent/
/// relations) and that their `dump()` strings match too (the human-readable
/// form a driver or a debugging session would actually diff).
#[test]
fn real_page_fixture_ax_tree_snapshot_is_idempotent_across_rebuilds() {
    let fixture_dir =
        format!("{}/../../tests/real-pages/pages", env!("CARGO_MANIFEST_DIR"));
    let html = std::fs::read_to_string(format!("{fixture_dir}/app-analytics.html"))
        .expect("read app-analytics.html fixture");
    // `build_document` takes author CSS as one string, separate from the
    // HTML, and never fetches `<link>`s itself (see A11Y-7's
    // `wpt_conformance.rs::inline_style_css` for the same constraint against
    // WPT fixtures) — the fixture's one external stylesheet is read
    // directly, the same way a real driver's `collect_author_css`
    // (`crates/amazing/src/page.rs`, out of this crate's dependency graph)
    // would resolve a relative `<link href>`.
    let css = std::fs::read_to_string(format!("{fixture_dir}/../assets/base.css"))
        .expect("read app-analytics.html's linked base.css");

    let first = build_document(&html, &css, 1024.0, None);
    let second = build_document(&html, &css, 1024.0, None);

    let first_snapshot = first.tree.snapshot();
    let second_snapshot = second.tree.snapshot();
    assert!(!first_snapshot.dump().is_empty(), "snapshot dump was empty");
    assert_eq!(
        first_snapshot, second_snapshot,
        "rebuilding the same document produced a structurally different \
         accessibility tree snapshot"
    );
    assert_eq!(
        first_snapshot.dump(),
        second_snapshot.dump(),
        "rebuilding the same document produced a different dump() string"
    );

    // Sanity: this is the same real, populated tree A11Y-6's
    // `runtime_host_a11y::tests::real_page_fixture_produces_a_populated_
    // accessibility_tree` checks via the full driver pipeline — a nav
    // landmark and the 8-row data table, confirming the fixture load above
    // (raw HTML + one concatenated stylesheet, no driver) built the same
    // shape rather than an empty or degenerate tree that would trivially
    // satisfy the equality checks above.
    let nav_count = first_snapshot.nodes.iter().filter(|node| node.role == Role::Navigation).count();
    assert_eq!(nav_count, 1, "expected exactly one nav landmark\n{}", first_snapshot.dump());
    let row_count = first_snapshot.nodes.iter().filter(|node| node.role == Role::Row).count();
    assert_eq!(row_count, 9, "expected 9 table rows (1 header + 8 data)\n{}", first_snapshot.dump());
}
