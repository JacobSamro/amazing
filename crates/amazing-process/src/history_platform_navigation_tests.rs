//! Pure store matrices only. Native accepted-navigation/fully-active proof is
//! an embedding prerequisite, not a flag supplied by these lower tests.
use super::*;

fn owner(nav: u64) -> PlatformDocumentIdentity {
    PlatformDocumentIdentity {runtime_id: 100 + nav, document: nav as u32, node_generation: 7}
}
fn origin() -> Origin { HttpUrl::parse("https://example.test/").unwrap().origin().clone() }
fn url(value: &str) -> ParsedUrl { ParsedUrl::parse(value).unwrap() }
fn snapshot(value: u8) -> HistorySnapshot {
    HistorySnapshot {history_state: Some(vec![value]), ..Default::default()}
}
fn root() -> (SessionHistory, Origin) {
    let mut h = SessionHistory::new(1, 1).unwrap(); let origin = origin();
    h.attach_platform_document(1, owner(1), &url("https://example.test/root"), &origin,
        snapshot(1)).unwrap(); (h, origin)
}
fn token(h: &SessionHistory, nav: u64, origin: &Origin) -> PlatformDocumentToken {
    h.platform_document_token(nav, owner(nav), origin).unwrap()
}
fn reserve(h: &mut SessionHistory, parent: u64, nav: u64, origin: &Origin)
    -> PlatformInitialChild
{
    let parent = token(h, parent, origin);
    h.reserve_platform_initial_child_with_projection(&parent, owner(parent.navigable), origin,
        nav, owner(nav), origin, &url("about:blank"), snapshot(0), || Ok::<_, &str>(()))
        .unwrap().0
}
fn attached(h: &mut SessionHistory, parent: u64, nav: u64, origin: &Origin)
    -> PlatformInitialChild
{
    let allocation = reserve(h, parent, nav, origin);
    let old = token(h, nav, origin);
    let replacement = PlatformDocumentIdentity {runtime_id: 1000 + nav, ..owner(nav)};
    // Remove genuine initial-about:blank behavior with an actual lower replacement.
    let (fresh, _) = h.commit_platform_navigation_with_handling_and_projection(&old, owner(nav),
        origin, replacement, origin, &url("https://example.test/child"), snapshot(2),
        PlatformNavigationHandling::Replace, || Ok::<_, &str>(())).unwrap();
    // Tests use this actual replacement owner from now on, never fake activity.
    let parent = token(h, parent, origin);
    h.attach_platform_initial_child(&allocation, &parent, owner(parent.navigable), origin).unwrap();
    assert_eq!(fresh.identity, replacement);
    allocation
}
fn active(h: &SessionHistory, allocation: &PlatformInitialChild, origin: &Origin)
    -> PlatformDocumentToken
{
    let entry = h.platform_current_entry(allocation.navigable_id()).unwrap();
    let HistoryDocumentOwner::Platform(owner) = h.documents[&entry.document].owner else {panic!("platform")};
    h.platform_document_token(allocation.navigable_id(), owner, origin).unwrap()
}
fn canonical(h: &SessionHistory) -> String {
    format!("{:?}", (&h.parents, &h.entries, &h.documents, &h.platform_origins,
        &h.platform_parent_documents, &h.platform_initial_documents, &h.steps,
        h.current, h.epoch, h.next_id))
}
fn admit(h: &mut SessionHistory, allocation: &PlatformInitialChild, origin: &Origin)
    -> PlatformNavigationAttempt
{
    let target = active(h, allocation, origin);
    h.admit_platform_child_navigation(allocation, &target, target.identity, origin).unwrap()
}

#[test]
fn accepted_requests_supersede_same_target_and_stale_cancel_cannot_clear_newer() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let before = canonical(&h); let first = admit(&mut h, &allocation, &origin);
    let second = admit(&mut h, &allocation, &origin); let third = admit(&mut h, &allocation, &origin);
    assert_ne!(first, second); assert_ne!(first, third); assert_ne!(second, third);
    for old in [&first, &second] {
        assert_eq!(h.current_platform_child_navigation(old, owner(2), &origin), Err(HistoryError::Stale));
        assert_eq!(h.cancel_platform_child_navigation(old, owner(2), &origin), Err(HistoryError::Stale));
    }
    assert_eq!(h.current_platform_child_navigation(&third, owner(2), &origin).unwrap(), token(&h, 2, &origin));
    assert_eq!(canonical(&h), before);
}
#[test]
fn exact_cancel_is_not_drop_and_never_reuses_cancelled_sequence() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let first = admit(&mut h, &allocation, &origin); let sequence = first.key.sequence;
    let before = canonical(&h);
    h.cancel_platform_child_navigation(&first, owner(2), &origin).unwrap();
    assert_eq!(h.cancel_platform_child_navigation(&first, owner(2), &origin), Err(HistoryError::Stale));
    let next = admit(&mut h, &allocation, &origin); assert!(next.key.sequence > sequence);
    drop(next);
    assert!(h.platform_initial_children[&2].navigation.current.is_some());
    assert_eq!(canonical(&h), before);
}
#[test]
fn admission_exhaustion_preserves_previous_authority_and_all_canonical_state() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let old = admit(&mut h, &allocation, &origin);
    h.platform_initial_children.get_mut(&2).unwrap().navigation.sequence = u64::MAX;
    let before = canonical(&h); let state = h.platform_initial_children[&2].navigation.clone();
    let target = token(&h, 2, &origin);
    assert_eq!(h.admit_platform_child_navigation(&allocation, &target, owner(2), &origin), Err(HistoryError::Limit));
    assert_eq!(h.platform_initial_children[&2].navigation, state);
    assert_eq!(h.current_platform_child_navigation(&old, owner(2), &origin).unwrap(), target);
    assert_eq!(canonical(&h), before);
}
#[test]
fn invalid_owner_generation_token_allocation_and_origin_never_supersede() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let sibling = reserve(&mut h, 1, 3, &origin); let old = admit(&mut h, &allocation, &origin);
    let target = token(&h, 2, &origin); let before = canonical(&h);
    for invalid in [PlatformDocumentIdentity {node_generation: 8, ..owner(2)}, owner(3)] {
        assert!(h.admit_platform_child_navigation(&allocation, &target, invalid, &origin).is_err());
        assert!(h.cancel_platform_child_navigation(&old, invalid, &origin).is_err());
    }
    assert!(h.admit_platform_child_navigation(&sibling, &target, owner(2), &origin).is_err());
    assert!(h.admit_platform_child_navigation(&allocation, &token(&h, 3, &origin), owner(3), &origin).is_err());
    assert!(h.admit_platform_child_navigation(&allocation, &target, owner(2), &Origin::opaque()).is_err());
    assert_eq!(h.current_platform_child_navigation(&old, owner(2), &origin).unwrap(), target);
    assert_eq!(canonical(&h), before);
}
#[test]
fn foreign_store_and_tab_cannot_admit_current_or_cancel_equal_native_identities() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let attempt = admit(&mut h, &allocation, &origin); let target = token(&h, 2, &origin);
    for tab in [1, 2] {
        let mut foreign = SessionHistory::new(tab, 1).unwrap();
        foreign.attach_platform_document(1, owner(1), &url("https://example.test/root"), &origin, snapshot(1)).unwrap();
        let own_allocation = reserve(&mut foreign, 1, 2, &origin);
        let own_attempt = admit(&mut foreign, &own_allocation, &origin); let before = canonical(&foreign);
        assert!(foreign.admit_platform_child_navigation(&allocation, &target, owner(2), &origin).is_err());
        assert!(foreign.current_platform_child_navigation(&attempt, owner(2), &origin).is_err());
        assert!(foreign.cancel_platform_child_navigation(&attempt, owner(2), &origin).is_err());
        assert!(foreign.current_platform_child_navigation(&own_attempt, owner(2), &origin).is_ok());
        assert_eq!(canonical(&foreign), before);
    }
}
#[test]
fn siblings_and_unrelated_history_epochs_do_not_abort_current_attempt() {
    let (mut h, origin) = root(); let one = reserve(&mut h, 1, 2, &origin); let two = reserve(&mut h, 1, 3, &origin);
    let first = admit(&mut h, &one, &origin); let second = admit(&mut h, &two, &origin);
    let root = token(&h, 1, &origin);
    h.commit_platform_same_document(&root, owner(1), &origin, CommitKind::Push,
        &url("https://example.test/root#next"), snapshot(3)).unwrap();
    h.cancel_platform_child_navigation(&second, owner(3), &origin).unwrap();
    assert!(h.current_platform_child_navigation(&first, owner(2), &origin).is_ok());
    assert!(h.current_platform_child_navigation(&second, owner(3), &origin).is_err());
}
#[test]
fn same_document_push_replace_and_current_step_drift_rederive_actual_token() {
    let (mut h, origin) = root(); let allocation = attached(&mut h, 1, 2, &origin);
    let initial = active(&h, &allocation, &origin); let attempt = admit(&mut h, &allocation, &origin);
    for (kind, value) in [(CommitKind::Push, 3), (CommitKind::Replace, 4), (CommitKind::Push, 5)] {
        let target = active(&h, &allocation, &origin);
        h.commit_platform_same_document(&target, target.identity, &origin, kind,
            &url(&format!("https://example.test/child#{value}")), snapshot(value)).unwrap();
        let current = h.current_platform_child_navigation(&attempt, initial.identity, &origin).unwrap();
        assert_eq!(current, active(&h, &allocation, &origin)); assert_eq!(current.document, initial.document);
        assert_ne!(current.entry, initial.entry);
        assert!(h.admit_platform_child_navigation(&allocation, &initial, initial.identity, &origin).is_err());
    }
    h.cancel_platform_child_navigation(&attempt, initial.identity, &origin).unwrap();
}
#[test]
fn pending_to_assigned_attachment_preserves_attempt_without_capture_of_numeric_step() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let initial = token(&h, 2, &origin); let attempt = admit(&mut h, &allocation, &origin);
    let parent = token(&h, 1, &origin);
    h.attach_platform_initial_child(&allocation, &parent, owner(1), &origin).unwrap();
    let current = h.current_platform_child_navigation(&attempt, owner(2), &origin).unwrap();
    assert_eq!(initial.step, PlatformEntryStep::Pending);
    assert!(matches!(current.step, PlatformEntryStep::Assigned(_))); assert_eq!(current.document, initial.document);
}
#[test]
fn replacement_and_retirement_reject_old_document_without_semantic_commit_refactor() {
    for retire in [false, true] {
        let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
        let attempt = admit(&mut h, &allocation, &origin); let old = token(&h, 2, &origin);
        if retire {h.retire_platform_document(&old, owner(2), &origin).unwrap();}
        else {
            let new = PlatformDocumentIdentity {node_generation: 8, ..owner(2)};
            h.commit_platform_navigation_with_handling_and_projection(&old, owner(2), &origin, new, &origin,
                &url("https://example.test/replacement"), snapshot(2), PlatformNavigationHandling::Replace,
                || Ok::<_, &str>(())).unwrap();
        }
        let before = canonical(&h); let state = h.platform_initial_children[&2].navigation.clone();
        assert!(h.current_platform_child_navigation(&attempt, owner(2), &origin).is_err());
        assert!(h.cancel_platform_child_navigation(&attempt, owner(2), &origin).is_err());
        assert_eq!(h.platform_initial_children[&2].navigation, state); assert_eq!(canonical(&h), before);
        if !retire {
            let new_owner = active(&h, &allocation, &origin).identity;
            let next = admit(&mut h, &allocation, &origin);
            assert!(h.cancel_platform_child_navigation(&attempt, new_owner, &origin).is_err());
            assert!(h.current_platform_child_navigation(&next, new_owner, &origin).is_ok());
        }
    }
}
#[test]
fn actual_destruction_and_recreation_reject_old_allocation_and_attempt() {
    let (mut h, origin) = root(); let allocation = reserve(&mut h, 1, 2, &origin);
    let old = admit(&mut h, &allocation, &origin); h.destroy_platform_initial_child(&allocation).unwrap();
    let fresh = reserve(&mut h, 1, 2, &origin); let next = admit(&mut h, &fresh, &origin);
    assert_ne!(fresh, allocation); let before = canonical(&h);
    assert!(h.cancel_platform_child_navigation(&old, owner(2), &origin).is_err());
    assert!(h.current_platform_child_navigation(&old, owner(2), &origin).is_err());
    assert!(h.current_platform_child_navigation(&next, owner(2), &origin).is_ok());
    assert_eq!(canonical(&h), before);
}
#[test]
fn ancestor_retirement_and_nested_destruction_reject_actual_descendant_authority() {
    for destroy in [false, true] {
        let (mut h, origin) = root(); let parent_allocation = reserve(&mut h, 1, 2, &origin);
        let child = reserve(&mut h, 2, 3, &origin); let attempt = admit(&mut h, &child, &origin);
        if destroy {h.destroy_platform_initial_child(&parent_allocation).unwrap();}
        else {h.retire_platform_document(&token(&h, 2, &origin), owner(2), &origin).unwrap();}
        let before = canonical(&h);
        assert!(h.current_platform_child_navigation(&attempt, owner(3), &origin).is_err());
        assert!(h.cancel_platform_child_navigation(&attempt, owner(3), &origin).is_err());
        assert_eq!(canonical(&h), before);
    }
}
#[test]
fn actual_opaque_origin_identity_is_not_reconstructed_from_serialized_null() {
    let (mut h, parent_origin) = root(); let actual = Origin::opaque(); let other = Origin::opaque();
    let parent = token(&h, 1, &parent_origin);
    let (allocation, target, _) = h.reserve_platform_initial_child_with_projection(&parent, owner(1), &parent_origin,
        2, owner(2), &actual, &url("about:blank"), snapshot(0), || Ok::<_, &str>(())).unwrap();
    let attempt = h.admit_platform_child_navigation(&allocation, &target, owner(2), &actual).unwrap();
    assert_eq!(actual.serialize(), other.serialize()); let before = canonical(&h);
    assert!(h.current_platform_child_navigation(&attempt, owner(2), &other).is_err());
    assert!(h.cancel_platform_child_navigation(&attempt, owner(2), &other).is_err());
    assert_eq!(h.current_platform_child_navigation(&attempt, owner(2), &actual).unwrap(), target);
    assert_eq!(canonical(&h), before);
}
