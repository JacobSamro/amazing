//! Live-region and event-diff fixtures.

use crate::event::{AxEventKind, Live, diff};
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
        .filter(|event| event.kind == AxEventKind::Changed)
        .collect();
    assert!(!changed.is_empty(), "{events:?}");
    assert!(changed.iter().any(|event| event.text_changed));
    let region = changed.iter().find_map(|event| event.live.as_ref()).unwrap();
    assert_eq!(region.live, Live::Polite);
    assert!(changed.iter().any(|event| event.as_announcement().is_some()));
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
