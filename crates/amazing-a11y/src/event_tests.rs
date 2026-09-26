//! Live-region and event-diff fixtures.

use crate::event::{AxEventKind, Live, diff, rebuild_and_diff};
use crate::role::Role;
use crate::test_support::document;

fn events(previous: &str, next: &str) -> Vec<crate::AxEvent> {
    let before = document(previous);
    let after = document(next);
    diff(&before.tree, &after.tree)
}

#[test]
fn no_change_produces_no_events() {
    let html = "<div role=\"log\" aria-live=\"polite\"><span>a</span></div>";
    assert!(events(html, html).is_empty());
}

#[test]
fn added_node_in_a_live_region() {
    let before = "<div role=\"log\" aria-live=\"polite\"><span>a</span></div>";
    let after = "<div role=\"log\" aria-live=\"polite\"><span>a</span><span>b</span></div>";
    let events = events(before, after);
    let added: Vec<_> = events
        .iter()
        .filter(|event| event.kind == AxEventKind::Added)
        .collect();
    assert_eq!(added.len(), 1, "{events:?}");
    assert_eq!(added[0].name, "b");
    let region = added[0].live.as_ref().expect("live region");
    assert_eq!(region.live, Live::Polite);
    assert_eq!(region.role, Role::Log);
    // Default relevance (additions text) accepts the addition.
    assert!(added[0].as_announcement().is_some());
}

#[test]
fn removed_node_in_a_live_region() {
    let before = "<div role=\"log\" aria-live=\"polite\"><span>a</span><span>b</span></div>";
    let after = "<div role=\"log\" aria-live=\"polite\"><span>a</span></div>";
    let events = events(before, after);
    let removed: Vec<_> = events
        .iter()
        .filter(|event| event.kind == AxEventKind::Removed)
        .collect();
    assert_eq!(removed.len(), 1, "{events:?}");
    assert_eq!(removed[0].name, "b");
    // Removals are not relevant by default.
    assert!(removed[0].as_announcement().is_none());
}

#[test]
fn text_change_is_announced_by_status() {
    let before = "<div role=\"status\">old</div>";
    let after = "<div role=\"status\">new</div>";
    let events = events(before, after);
    let changed: Vec<_> = events
        .iter()
        .filter(|event| event.kind == AxEventKind::NameChanged)
        .collect();
    assert_eq!(changed.len(), 1, "{events:?}");
    assert_eq!(changed[0].name, "new");
    assert_eq!(changed[0].previous_name.as_deref(), Some("old"));
    let region = changed[0].live.as_ref().unwrap();
    assert_eq!(region.live, Live::Polite);
    assert!(changed[0].as_announcement().is_some());
}

#[test]
fn role_change_is_reported_separately_from_name_change() {
    // Both the role (via the `role` attribute) and the name change at once;
    // each must surface as its own event rather than one opaque bundle.
    let before = "<div role=\"button\" aria-label=\"old\"></div>";
    let after = "<div role=\"link\" aria-label=\"new\"></div>";
    let events = events(before, after);
    let name = events.iter().find(|event| event.kind == AxEventKind::NameChanged).expect("name event");
    assert_eq!(name.previous_name.as_deref(), Some("old"));
    assert_eq!(name.name, "new");
    let role = events.iter().find(|event| event.kind == AxEventKind::RoleChanged).expect("role event");
    assert_eq!(role.previous_role, Some(Role::Button));
    assert_eq!(role.role, Role::Link);
    // Exactly these two events for this one persisting node — no leftover
    // catch-all `Changed` event and no duplicate.
    assert_eq!(events.len(), 2, "{events:?}");
}

#[test]
fn state_change_is_reported_as_its_own_kind() {
    let before = "<button aria-pressed=\"false\">x</button>";
    let after = "<button aria-pressed=\"true\">x</button>";
    let events = events(before, after);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, AxEventKind::StateChanged);
    assert!(events[0].as_announcement().is_none(), "not inside a live region");
}

#[test]
fn property_change_is_reported_as_its_own_kind() {
    let before = "<div role=\"slider\" aria-valuenow=\"1\" aria-valuemin=\"0\" aria-valuemax=\"10\"></div>";
    let after = "<div role=\"slider\" aria-valuenow=\"2\" aria-valuemin=\"0\" aria-valuemax=\"10\"></div>";
    let events = events(before, after);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, AxEventKind::PropertyChanged);
}

#[test]
fn active_descendant_change_is_reported_as_its_own_kind() {
    let before = concat!(
        "<ul role=\"listbox\" aria-activedescendant=\"a\">",
        "<li id=\"a\" role=\"option\">a</li><li id=\"b\" role=\"option\">b</li></ul>",
    );
    let after = concat!(
        "<ul role=\"listbox\" aria-activedescendant=\"b\">",
        "<li id=\"a\" role=\"option\">a</li><li id=\"b\" role=\"option\">b</li></ul>",
    );
    let events = events(before, after);
    let listbox = events
        .iter()
        .find(|event| event.kind == AxEventKind::ActiveDescendantChanged)
        .expect("active-descendant event");
    assert_eq!(listbox.role, Role::Listbox);
}

#[test]
fn inserting_at_the_head_shifts_dom_identity() {
    // A pre-order insert shifts every dom index after it, and the diff has no
    // identity beyond `dom_node`: the DOM arena index that used to hold "a"
    // now holds "z" (and the one that held "b" now holds "a"), so the diff
    // reports a cascade of NameChanged (matched by index, content changed)
    // rather than one clean Added for "z" — never a "moved" node, and no
    // Removed either, since every old index still exists in the new tree,
    // just renamed. This pins that documented limitation (see the module doc
    // comment) so a future change to the comparison key doesn't silently
    // alter it without a test noticing.
    let before = "<div><span id=\"a\">a</span><span id=\"b\">b</span></div>";
    let after = "<div><span id=\"z\">z</span><span id=\"a\">a</span><span id=\"b\">b</span></div>";
    let events = events(before, after);
    assert!(events.iter().all(|event| event.kind == AxEventKind::NameChanged || event.kind == AxEventKind::Added), "{events:?}");
    let renamed: Vec<&str> = events
        .iter()
        .filter(|event| event.kind == AxEventKind::NameChanged)
        .map(|event| event.name.as_str())
        .collect();
    assert_eq!(renamed, vec!["z", "a"], "{events:?}");
    assert_eq!(
        events.iter().filter(|event| event.kind == AxEventKind::Added).count(),
        1,
        "{events:?}"
    );
}

#[test]
fn name_invalidation_propagates_through_a_labelledby_reference() {
    // The button's own subtree never changes; only the div it references via
    // `aria-labelledby` does. A full rebuild-and-diff still reports the
    // button's name as changed, because accname re-resolves the reference
    // from scratch on every rebuild.
    let before = "<div id=\"x\">Hello</div><button aria-labelledby=\"x\">unused</button>";
    let after = "<div id=\"x\">Goodbye</div><button aria-labelledby=\"x\">unused</button>";
    let events = events(before, after);
    let button_name = events
        .iter()
        .find(|event| event.kind == AxEventKind::NameChanged && event.role == Role::Button)
        .expect("button name-changed event");
    assert_eq!(button_name.previous_name.as_deref(), Some("Hello"));
    assert_eq!(button_name.name, "Goodbye");
}

#[test]
fn rebuild_and_diff_matches_build_then_diff() {
    let before = document("<div role=\"status\">old</div>");
    let mutated = amazing_html::parse("<div role=\"status\">new</div>");
    let sheet = amazing_css::parse_stylesheet("");
    let styled = amazing_css::compute(&mutated, &[sheet]);
    let text = amazing_text::FallbackTextSystem::new();
    let layout = amazing_layout::layout_styled_document(
        &mutated,
        &styled,
        &text,
        &amazing_layout::NoReplacedElements,
        800.0,
        Some(600.0),
    );
    let (next, events) = rebuild_and_diff(&mutated, &styled, &layout, &before.tree);
    let expected = diff(&before.tree, &next);
    assert_eq!(events, expected);
    assert!(events.iter().any(|event| event.kind == AxEventKind::NameChanged));
}

#[test]
fn alert_is_assertive() {
    let before = "<div role=\"alert\"><span>a</span></div>";
    let after = "<div role=\"alert\"><span>a</span><span>b</span></div>";
    let events = events(before, after);
    let added = events
        .iter()
        .find(|event| event.kind == AxEventKind::Added)
        .unwrap();
    assert_eq!(added.live.as_ref().unwrap().live, Live::Assertive);
}

#[test]
fn aria_live_off_is_explicitly_silent() {
    let before = "<div role=\"log\" aria-live=\"off\"><span>a</span></div>";
    let after = "<div role=\"log\" aria-live=\"off\"><span>a</span><span>b</span></div>";
    let events = events(before, after);
    let added = events
        .iter()
        .find(|event| event.kind == AxEventKind::Added)
        .unwrap();
    assert_eq!(added.live.as_ref().unwrap().live, Live::Off);
    assert!(added.as_announcement().is_none());
}

#[test]
fn relevant_text_rejects_additions() {
    let before = "<div role=\"log\" aria-live=\"polite\" aria-relevant=\"text\"><span>a</span></div>";
    let after =
        "<div role=\"log\" aria-live=\"polite\" aria-relevant=\"text\"><span>a</span><span>b</span></div>";
    let events = events(before, after);
    let added = events
        .iter()
        .find(|event| event.kind == AxEventKind::Added)
        .unwrap();
    assert!(
        added.as_announcement().is_none(),
        "addition should be filtered by relevant=text"
    );
}

#[test]
fn nearest_live_region_wins() {
    let before =
        "<div role=\"log\" aria-live=\"polite\"><div role=\"alert\"><span>a</span></div></div>";
    let after =
        "<div role=\"log\" aria-live=\"polite\"><div role=\"alert\"><span>a</span><span>b</span></div></div>";
    let events = events(before, after);
    let added = events
        .iter()
        .find(|event| event.kind == AxEventKind::Added)
        .unwrap();
    let region = added.live.as_ref().unwrap();
    assert_eq!(region.role, Role::Alert);
    assert_eq!(region.live, Live::Assertive);
}

#[test]
fn changes_outside_a_live_region_are_not_announcements() {
    let before = "<button>a</button>";
    let after = "<button>ab</button>";
    let events = events(before, after);
    assert!(events.iter().all(|event| event.live.is_none()));
    assert!(events.iter().all(|event| event.as_announcement().is_none()));
}

#[test]
fn atomic_region_is_recorded() {
    let before =
        "<div role=\"log\" aria-live=\"polite\" aria-atomic=\"true\"><span>a</span></div>";
    let after =
        "<div role=\"log\" aria-live=\"polite\" aria-atomic=\"true\"><span>a</span><span>b</span></div>";
    let events = events(before, after);
    let added = events
        .iter()
        .find(|event| event.kind == AxEventKind::Added)
        .unwrap();
    assert!(added.live.as_ref().unwrap().atomic);
}
