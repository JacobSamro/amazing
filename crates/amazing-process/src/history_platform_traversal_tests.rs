//! Grouped pure lower matrices. These do not claim installed browser behavior.
use super::*;
use std::cell::Cell;

fn owner(nav: u64) -> PlatformDocumentIdentity {
    PlatformDocumentIdentity { runtime_id: nav + 100, document: nav as u32, node_generation: 3 }
}
fn url(value: &str) -> ParsedUrl { ParsedUrl::parse(value).unwrap() }
fn snapshot(value: u8) -> HistorySnapshot {
    HistorySnapshot { history_state: Some(vec![value]), scroll: vec![(1, value as i32, 2)],
        ..HistorySnapshot::default() }
}
fn origin() -> Origin { HttpUrl::parse("https://example.test/").unwrap().origin().clone() }
fn root() -> (SessionHistory, Origin) {
    let mut h = SessionHistory::new(1, 1).unwrap();
    let origin = origin();
    h.attach_platform_document(1, owner(1), &url("https://example.test/root"),
        &origin, snapshot(1)).unwrap();
    (h, origin)
}
fn token(h: &SessionHistory, nav: u64, origin: &Origin) -> PlatformDocumentToken {
    h.platform_document_token(nav, owner(nav), origin).unwrap()
}
fn add_child(h: &mut SessionHistory, parent: u64, nav: u64, origin: &Origin)
    -> PlatformInitialChild
{
    let parent = token(h, parent, origin);
    let initial = PlatformDocumentIdentity { runtime_id: nav + 1000, ..owner(nav) };
    let (allocation, initial_token, _) = h.reserve_platform_initial_child_with_projection(
        &parent, parent.identity, origin, nav, initial, origin, &url("about:blank"),
        snapshot(0), || Ok::<_, &str>(())).unwrap();
    h.commit_platform_navigation_with_handling_and_projection(&initial_token, initial,
        origin, owner(nav), origin, &url(&format!("https://example.test/child{nav}")),
        snapshot(nav as u8), PlatformNavigationHandling::Replace,
        || Ok::<_, &str>(())).unwrap();
    let parent = token(h, parent.navigable, origin);
    h.attach_platform_initial_child(&allocation, &parent, parent.identity, origin).unwrap();
    allocation
}
fn push(h: &mut SessionHistory, nav: u64, origin: &Origin, value: u8) -> u64 {
    let current = token(h, nav, origin);
    h.commit_platform_same_document(&current, owner(nav), origin, CommitKind::Push,
        &url(&format!("https://example.test/{nav}-{value}")), snapshot(value)).unwrap().1.step
}
fn tree() -> (SessionHistory, Origin, u64) {
    let (mut h, origin) = root();
    add_child(&mut h, 1, 2, &origin);
    add_child(&mut h, 1, 3, &origin);
    let target = h.current_step().unwrap();
    push(&mut h, 1, &origin, 10);
    push(&mut h, 2, &origin, 20);
    push(&mut h, 3, &origin, 30);
    (h, origin, target)
}
fn make_plan(h: &SessionHistory, origin: &Origin, source: u64, step: u64)
    -> PlatformSameDocumentPlan
{
    h.plan_platform_same_document_traversal(&token(h, source, origin), owner(source), origin, step).unwrap()
}
fn eligible(plan: &PlatformSameDocumentPlan) -> Vec<PlatformOwnerEligibility> {
    plan.required_owners().iter().cloned().map(|mut owner| {
        owner.fully_active = true; owner
    }).collect()
}
fn updates(plan: &PlatformSameDocumentPlan) -> Vec<PlatformSnapshotUpdate> {
    plan.changes().iter().map(|change| PlatformSnapshotUpdate {
        token: change.source_token.clone(), snapshot: snapshot(change.from.navigable as u8 + 40),
    }).collect()
}
fn unchanged(h: &SessionHistory) -> String {
    // Every canonical field affected by this area, including counters and
    // transitional lineages, is inspected on failure; no renderer store copy.
    format!("{:?}{:?}", (&h.steps, &h.entries, &h.documents, &h.platform_origins,
        &h.platform_initial_children, &h.platform_parent_documents, &h.parents),
        (&h.platform_initial_documents, h.current, h.epoch, h.next_id,
        &h.pending, h.queued_traversal, &h.platform_traversal))
}

#[test]
fn root_child_and_sibling_changes_publish_exact_snapshots_tokens_and_one_joint_step() {
    for source in [1, 2, 3] {
        let (mut h, origin, target) = tree();
        let plan = make_plan(&h, &origin, source, target);
        assert_eq!(plan.changes.len(), 3);
        assert_eq!(plan.required.len(), 3);
        assert!(plan.required.iter().all(|owner| !owner.fully_active));
        let next = h.next_id;
        let epoch = h.epoch;
        let steps = h.steps.clone();
        let documents = h.documents.clone();
        let updates = updates(&plan);
        let mut projected = BTreeMap::new();
        let result = h.apply_platform_same_document_plan_with_projection(&plan,
            &eligible(&plan), &updates, |changes| {
                for change in changes { projected.insert(change.from.navigable, change.to.url.clone()); }
                Ok::<_, &str>(())
            }).unwrap();
        assert_eq!(result.effect, PlatformTraversalEffect::Projected);
        assert_eq!(h.current_step(), Some(target));
        assert_eq!(h.epoch, epoch + 1);
        assert_eq!(h.next_id, next);
        assert_eq!(h.steps, steps);
        assert_eq!(h.documents, documents);
        assert_eq!(result.tokens.len(), 3);
        for change in plan.changes() {
            assert_eq!(projected[&change.from.navigable], change.to.url);
            assert_eq!(h.active_entry(change.from.navigable), Some(&change.to));
            assert_eq!(h.entries[&change.from.id].snapshot,
                snapshot(change.from.navigable as u8 + 40));
            assert_eq!(result.tokens[&change.from.navigable],
                token(&h, change.from.navigable, &origin));
        }
        // Forward entries remain present and can be selected by a later plan.
        let forward = plan.current.id;
        assert!(h.plan_platform_same_document_traversal(&token(&h, source, &origin),
            owner(source), &origin, forward).is_ok());
    }
}

#[test]
fn unchanged_ancestor_and_equal_url_different_state_are_not_dropped() {
    let (mut h, origin) = root();
    add_child(&mut h, 1, 2, &origin);
    add_child(&mut h, 2, 4, &origin);
    let target = h.current_step().unwrap();
    let old_url = h.active_entry(4).unwrap().url.clone();
    let current = token(&h, 4, &origin);
    h.commit_platform_same_document(&current, owner(4), &origin, CommitKind::Push,
        &url(&old_url), snapshot(99)).unwrap();
    let plan = make_plan(&h, &origin, 4, target);
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].from.url, plan.changes[0].to.url);
    assert_ne!(plan.changes[0].from.snapshot, plan.changes[0].to.snapshot);
    assert_eq!(plan.required.iter().map(|owner| owner.token.navigable).collect::<Vec<_>>(), vec![1, 2, 4]);
    let old = unchanged(&h);
    let calls = Cell::new(0);
    let mut supplied = eligible(&plan);
    supplied[0].fully_active = false;
    assert!(matches!(h.apply_platform_same_document_plan_with_projection(&plan,
        &supplied, &updates(&plan), |_| { calls.set(calls.get()+1); Ok::<_, &str>(()) }),
        Err(PlatformCommitError::History(HistoryError::NotEligible))));
    assert_eq!(calls.get(), 0);
    assert_eq!(unchanged(&h), old);
}

#[test]
fn missing_duplicate_stale_generation_and_typed_origin_checks_precede_projection() {
    for mutation in 0..6 {
        let (mut h, origin, target) = tree();
        let plan = make_plan(&h, &origin, 2, target);
        let mut supplied = eligible(&plan);
        match mutation {
            0 => { supplied.pop(); },
            1 => { supplied[1] = supplied[0].clone(); },
            2 => { supplied[1].owner.node_generation += 1; },
            3 => { supplied[1].token.identity.node_generation += 1; },
            4 => { supplied[1].origin = Origin::opaque(); },
            _ => { supplied[1].token.session += 1; },
        }
        let before = unchanged(&h);
        let calls = Cell::new(0);
        assert!(h.apply_platform_same_document_plan_with_projection(&plan, &supplied,
            &updates(&plan), |_| { calls.set(calls.get()+1); Ok::<_, &str>(()) }).is_err());
        assert_eq!(calls.get(), 0);
        assert_eq!(unchanged(&h), before);
    }
}

#[test]
fn snapshot_alias_dedup_bounds_conflicts_missing_and_epoch_overflow_are_atomic() {
    for mutation in 0..6 {
        let (mut h, origin, target) = tree();
        if mutation == 5 { h.epoch = u64::MAX; }
        let plan = make_plan(&h, &origin, 1, target);
        let mut snapshots = updates(&plan);
        match mutation {
            0 => { snapshots.push(snapshots[0].clone()); },
            1 => { let mut duplicate = snapshots[0].clone(); duplicate.snapshot = snapshot(99); snapshots.push(duplicate); },
            2 => { snapshots[0].snapshot.history_state = Some(vec![0; MAX_STATE_BYTES+1]); },
            3 => { snapshots[0].snapshot.scroll = vec![(1,0,0),(1,1,1)]; },
            4 => { snapshots.pop(); },
            _ => {},
        }
        let before = unchanged(&h);
        let calls = Cell::new(0);
        let result = h.apply_platform_same_document_plan_with_projection(&plan, &eligible(&plan),
            &snapshots, |_| { calls.set(calls.get()+1); Ok::<_, &str>(()) });
        if mutation == 0 { assert!(result.is_ok()); assert_eq!(calls.get(), 1); }
        else { assert!(result.is_err()); assert_eq!(calls.get(), 0); assert_eq!(unchanged(&h), before); }
    }
}

#[test]
fn native_transaction_errors_at_every_owner_roll_back_without_canonical_publication() {
    for failure in 0..3 {
        let (mut h, origin, target) = tree();
        let plan = make_plan(&h, &origin, 1, target);
        let before = unchanged(&h);
        let native: BTreeMap<_, _> = plan.changes.iter().map(|change|
            (change.from.navigable, (change.from.url.clone(), change.from.snapshot.clone()))).collect();
        let mut actual = native.clone();
        let result = h.apply_platform_same_document_plan_with_projection(&plan,
            &eligible(&plan), &updates(&plan), |changes| {
                for (index, change) in changes.iter().enumerate() {
                    actual.insert(change.from.navigable, (change.to.url.clone(), change.to.snapshot.clone()));
                    if index == failure {
                        // Models the trusted native rollback obligation; no
                        // browser/author callback or installed claim here.
                        actual = native.clone();
                        return Err("native transaction rejected");
                    }
                }
                Ok(())
            });
        assert_eq!(result, Err(PlatformCommitError::Projection("native transaction rejected")));
        assert_eq!(actual, native);
        assert_eq!(unchanged(&h), before);
    }
}

#[test]
fn replaced_step_destroyed_child_origin_status_and_saved_source_drift_reject_plans() {
    for mutation in 0..5 {
        let (mut h, origin, target) = tree();
        let plan = make_plan(&h, &origin, 1, target);
        match mutation {
            0 => { let token = token(&h, 1, &origin); h.commit_platform_same_document(&token,
                owner(1), &origin, CommitKind::Replace, &url("https://example.test/replaced"), snapshot(88)).unwrap(); },
            1 => { let allocation = h.platform_initial_children[&2].allocation.clone(); h.destroy_platform_initial_child(&allocation).unwrap(); },
            2 => { let document = h.active_entry(2).unwrap().document; h.platform_origins.insert(document, Origin::opaque()); },
            3 => { let document = h.active_entry(1).unwrap().document; h.documents.get_mut(&document).unwrap().status = DocumentStatus::Discarded; },
            _ => { push(&mut h, 3, &origin, 44); },
        }
        let before = unchanged(&h);
        let calls = Cell::new(0);
        assert!(h.apply_platform_same_document_plan_with_projection(&plan,
            &eligible(&plan), &updates(&plan), |_| { calls.set(calls.get()+1); Ok::<_, &str>(()) }).is_err());
        assert_eq!(calls.get(), 0);
        assert_eq!(unchanged(&h), before);
    }
}

#[test]
fn cross_document_removed_added_renderer_and_pending_states_are_bounded_not_silently_activated() {
    for mutation in 0..5 {
        let (mut h, origin, target) = tree();
        match mutation {
            0 => { let token = token(&h, 2, &origin); h.commit_platform_navigation_with_handling_and_projection(
                &token, owner(2), &origin, PlatformDocumentIdentity { runtime_id: 99, ..owner(2) },
                &origin, &url("https://example.test/newdoc"), snapshot(88), PlatformNavigationHandling::Push,
                || Ok::<_, &str>(())).unwrap(); },
            1 => { let index = h.steps.iter().position(|step|step.id==target).unwrap(); h.steps[index].entries.remove(&3); },
            2 => { let index = h.steps.iter().position(|step|step.id==target).unwrap(); let id=h.steps[index].entries[&3]; h.steps[index].entries.insert(99,id); },
            3 => { let document=h.active_entry(3).unwrap().document; h.documents.get_mut(&document).unwrap().owner=HistoryDocumentOwner::Renderer(12); },
            _ => { let parent=token(&h,1,&origin); h.reserve_platform_initial_child_with_projection(&parent,
                owner(1),&origin,9,owner(9),&origin,&url("about:blank"),snapshot(0),||Ok::<_,&str>(())).unwrap(); },
        }
        let source=token(&h,1,&origin);
        let before=unchanged(&h);
        assert!(h.plan_platform_same_document_traversal(&source,owner(1),&origin,target).is_err());
        assert_eq!(unchanged(&h),before);
    }
}

#[test]
fn opaque_identity_missing_target_and_foreign_store_cannot_forge_authentic_plan() {
    let opaque=Origin::opaque();
    let mut h=SessionHistory::new(1,1).unwrap();
    h.attach_platform_document(1,owner(1),&url("about:blank"),&opaque,snapshot(1)).unwrap();
    let target=h.current_step().unwrap();
    let first=token(&h,1,&opaque);
    h.commit_platform_same_document(&first,owner(1),&opaque,CommitKind::Push,
        &url("about:blank#next"),snapshot(2)).unwrap();
    let current=token(&h,1,&opaque);
    assert!(h.plan_platform_same_document_traversal(&current,owner(1),&Origin::opaque(),target).is_err());
    assert!(h.plan_platform_same_document_traversal(&current,owner(1),&opaque,u64::MAX).is_err());
    let plan=h.plan_platform_same_document_traversal(&current,owner(1),&opaque,target).unwrap();
    let (mut foreign,_)=root();
    let before=unchanged(&foreign);
    assert!(foreign.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),
        &updates(&plan),|_|Ok::<_,&str>(())).is_err());
    assert_eq!(unchanged(&foreign),before);
    h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&updates(&plan),
        |_|Ok::<_,&str>(())).unwrap();
    assert_eq!(token(&h,1,&opaque).document,current.document);
}

#[test]
fn no_op_never_projects_writes_snapshots_mints_epoch_or_implements_reload() {
    let (mut h,origin,target)=tree();
    let current=h.current_step().unwrap();
    let plan=make_plan(&h,&origin,1,current);
    assert!(plan.changes.is_empty());
    let before=unchanged(&h);
    let result=h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&[],
        |_|Err::<(),_>("no-op must never project")).unwrap();
    assert_eq!(result.effect,PlatformTraversalEffect::Unchanged);
    assert_eq!(unchanged(&h),before);
    // Updating an outgoing snapshot is not a no-op navigation operation.
    let update=PlatformSnapshotUpdate {token:token(&h,1,&origin),snapshot:snapshot(99)};
    assert!(h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&[update],
        |_|Ok::<_,&str>(())).is_err());
    assert_eq!(unchanged(&h),before);
    // go(0) reload remains a distinct fresh-document ticket.
    assert!(h.stage_platform_reload(&token(&h,1,&origin),owner(1),&origin).is_ok());
    assert!(h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&[],
        |_|Ok::<_,&str>(())).is_err());
    assert_ne!(target,current);
}

#[test]
fn legacy_callback_refuses_multi_owner_and_preserves_single_owner_qualified_contract() {
    let (mut h,origin,target)=tree();
    let before=unchanged(&h);let calls=Cell::new(0);let current=token(&h,1,&origin);
    assert_eq!(h.traverse_platform_same_document_with_projection(&current,owner(1),&origin,target,
        ||{calls.set(calls.get()+1);Ok::<_,&str>(())}),Err(PlatformCommitError::History(HistoryError::Order)));
    assert_eq!(calls.get(),0);assert_eq!(unchanged(&h),before);
    let (mut h,origin)=root();let target=h.current_step().unwrap();push(&mut h,1,&origin,2);
    let current=token(&h,1,&origin);
    h.traverse_platform_same_document_with_projection(&current,owner(1),&origin,target,
        ||{calls.set(calls.get()+1);Ok::<_,&str>(())}).unwrap();
    assert_eq!(calls.get(),1);
    let current=token(&h,1,&origin);let before=unchanged(&h);
    h.traverse_platform_same_document_with_projection(&current,owner(1),&origin,target,
        ||{calls.set(calls.get()+1);Ok::<_,&str>(())}).unwrap();
    assert_eq!(calls.get(),1);assert_eq!(unchanged(&h),before);
}

#[test]
fn inherited_source_aliases_after_root_replace_update_one_outgoing_snapshot() {
    let (mut h,origin)=root();
    add_child(&mut h,1,2,&origin);
    let target=h.current_step().unwrap();
    push(&mut h,1,&origin,10);
    push(&mut h,2,&origin,20);
    let current=token(&h,1,&origin);
    let (_,replacement)=h.commit_platform_same_document(&current,owner(1),&origin,
        CommitKind::Replace,&url("https://example.test/replaced"),snapshot(11)).unwrap();
    assert_eq!(h.steps.iter().filter(|step|step.entries.get(&1)==Some(&replacement.id)).count(),2);
    let plan=make_plan(&h,&origin,2,target);
    let updates=updates(&plan);
    h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&updates,
        |_|Ok::<_,&str>(())).unwrap();
    assert_eq!(h.entries[&replacement.id].snapshot,snapshot(41));
    assert_eq!(h.steps.iter().filter(|step|step.entries.get(&1)==Some(&replacement.id)).count(),2);
}

#[test]
fn distinct_assigned_step_with_identical_entry_aliases_changes_only_selection() {
    let (mut h,origin)=root();
    let target=h.current_step().unwrap();
    // Model an alias-only canonical joint step, without allocating another
    // entry/owner. The plan still binds this authentic store's exact maps.
    let id=h.mint().unwrap();
    let entries=h.steps[h.current.unwrap()].entries.clone();
    h.steps.push(HistoryStep {id,entries});h.current=Some(h.steps.len()-1);
    let plan=make_plan(&h,&origin,1,target);
    assert!(plan.changes.is_empty());
    let epoch=h.epoch;let next=h.next_id;let entries=h.entries.clone();
    let result=h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&[],
        |_|Err::<(),_>("step-only must never project")).unwrap();
    assert_eq!(result.effect,PlatformTraversalEffect::StepOnly);
    assert_eq!(h.current_step(),Some(target));assert_eq!(h.epoch,epoch+1);
    assert_eq!(h.next_id,next);assert_eq!(h.entries,entries);
    h.epoch=u64::MAX;
    let plan=make_plan(&h,&origin,1,target);let before=unchanged(&h);
    let result=h.apply_platform_same_document_plan_with_projection(&plan,&eligible(&plan),&[],
        |_|Err::<(),_>("current no-op must never project")).unwrap();
    assert_eq!(result.effect,PlatformTraversalEffect::Unchanged);assert_eq!(unchanged(&h),before);
}
