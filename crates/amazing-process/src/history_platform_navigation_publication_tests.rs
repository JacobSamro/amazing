//! Pure canonical publication matrices. No host activity flag proves native
//! eligibility, installer rollback, DOM chronology or installed navigation.
use super::*;
use std::cell::Cell;
fn owner(n:u64)->PlatformDocumentIdentity {PlatformDocumentIdentity {runtime_id:100+n,document:n as u32,node_generation:7}}
fn origin()->Origin {HttpUrl::parse("https://example.test/").unwrap().origin().clone()}
fn url(s:&str)->ParsedUrl {ParsedUrl::parse(s).unwrap()}
fn snap(n:u8)->HistorySnapshot {HistorySnapshot {history_state:Some(vec![n]),..Default::default()}}
fn setup(attached:bool)->(SessionHistory,Origin,PlatformInitialChild) {
    let o=origin();let mut h=SessionHistory::new(1,1).unwrap();
    let (parent,_)=h.attach_platform_document(1,owner(1),&url("https://example.test/root"),&o,snap(1)).unwrap();
    let (a,t,_)=h.reserve_platform_initial_child_with_projection(&parent,owner(1),&o,2,owner(2),&o,
        &url("about:blank"),snap(2),||Ok::<_,&str>(())).unwrap();
    if attached {
        h.commit_platform_navigation_with_handling_and_projection(&t,owner(2),&o,owner(3),&o,
            &url("https://example.test/child"),snap(3),PlatformNavigationHandling::Replace,||Ok::<_,&str>(())).unwrap();
        h.attach_platform_initial_child(&a,&parent,owner(1),&o).unwrap();
    }
    (h,o,a)
}
fn current(h:&SessionHistory,a:&PlatformInitialChild,o:&Origin)->PlatformDocumentToken {
    let e=h.platform_current_entry(a.navigable_id()).unwrap();
    let HistoryDocumentOwner::Platform(identity)=h.documents[&e.document].owner else {panic!("platform owner")};
    h.platform_document_token(a.navigable_id(),identity,o).unwrap()
}
fn admit(h:&mut SessionHistory,a:&PlatformInitialChild,o:&Origin)->PlatformNavigationAttempt {
    let t=current(h,a,o);h.admit_platform_child_navigation(a,&t,t.identity,o).unwrap()
}
fn canonical(h:&SessionHistory)->String {
    format!("{:?}|{:?}",(&h.parents,&h.entries,&h.documents,&h.platform_origins,&h.platform_parent_documents,
        &h.platform_initial_documents,&h.platform_initial_children,&h.steps,h.current,h.next_id,h.epoch),
        (&h.pending,&h.queued_traversal,&h.platform_traversal,&h.orphaned))
}
fn entry_projection(h:&SessionHistory)->String {
    let children:Vec<_>=h.platform_initial_children.values().map(|r|(&r.entry,&r.attachment_entry,r.attached)).collect();
    format!("{:?}",(&h.parents,&h.entries,&h.documents,&h.platform_origins,&h.platform_parent_documents,
        &h.platform_initial_documents,children,&h.steps,h.current,h.next_id,h.epoch,&h.orphaned))
}
fn commit(h:&mut SessionHistory,a:&PlatformNavigationAttempt,o:&Origin,handling:PlatformNavigationHandling)
    ->Result<(PlatformDocumentToken,PlatformEntry),PlatformCommitError<&'static str>> {
    let t=h.current_platform_child_navigation(a,a.key.owner,o).map_err(PlatformCommitError::History)?;
    h.commit_platform_child_navigation_attempt_with_projection(a,t.identity,o,owner(4),o,
        &url("https://example.test/new"),snap(4),handling,||Ok(()))
}
#[test]
fn successful_pending_publication_consumes_exact_attempt_and_preserves_attachment_lineage() {
    let (mut h,o,a)=setup(false);let attempt=admit(&mut h,&a,&o);let old=current(&h,&a,&o);
    let allocation=h.platform_initial_children[&2].allocation.clone();let sequence=h.platform_initial_children[&2].navigation.sequence;
    let (fresh,e)=commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
    assert_eq!(e.step,PlatformEntryStep::Pending);assert_eq!(h.platform_initial_children[&2].allocation,allocation);
    assert_eq!(h.platform_initial_children[&2].attachment_entry,e);assert_eq!(h.platform_initial_children[&2].navigation.sequence,sequence);
    assert!(h.platform_initial_children[&2].navigation.current.is_none());assert_ne!(fresh.document,old.document);
    assert!(h.current_platform_child_navigation(&attempt,old.identity,&o).is_err());
    let p=h.platform_document_token(1,owner(1),&o).unwrap();h.attach_platform_initial_child(&a,&p,owner(1),&o).unwrap();
    assert_eq!(h.platform_current_entry(2).unwrap().document,fresh.document);
}
#[test]
fn borrowed_attempt_projection_failure_preserves_every_canonical_field_and_can_retry() {
    for attached in [false,true] {
        let (mut h,o,a)=setup(attached);let attempt=admit(&mut h,&a,&o);let old=current(&h,&a,&o);let before=canonical(&h);let calls=Cell::new(0);
        let failed=h.commit_platform_child_navigation_attempt_with_projection(&attempt,old.identity,&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),PlatformNavigationHandling::Replace,||{calls.set(calls.get()+1);Err("native rollback")});
        assert_eq!(failed,Err(PlatformCommitError::Projection("native rollback")));assert_eq!(calls.get(),1);assert_eq!(canonical(&h),before);
        assert_eq!(h.current_platform_child_navigation(&attempt,old.identity,&o).unwrap(),old);
        commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
    }
}
#[test]
fn supersession_and_exact_cancel_reject_before_native_projection() {
    for cancel in [false,true] {
        let (mut h,o,a)=setup(false);let old=admit(&mut h,&a,&o);
        if cancel {h.cancel_platform_child_navigation(&old,owner(2),&o).unwrap();}else{let _new=admit(&mut h,&a,&o);}
        let before=canonical(&h);let calls=Cell::new(0);
        assert!(h.commit_platform_child_navigation_attempt_with_projection(&old,owner(2),&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),PlatformNavigationHandling::Replace,||{calls.set(1);Ok::<_,&str>(())}).is_err());
        assert_eq!(calls.get(),0);assert_eq!(canonical(&h),before);
    }
}
#[test]
fn compatibility_and_attempt_publication_have_exact_shared_entry_projection() {
    for (attached,handling) in [(false,PlatformNavigationHandling::Replace),(true,PlatformNavigationHandling::Replace),(true,PlatformNavigationHandling::Push)] {
        let (mut legacy,o,a)=setup(attached);let (mut actual,o2,a2)=setup(attached);let attempt=admit(&mut actual,&a2,&o2);
        let source=current(&legacy,&a,&o);
        let (_,e1)=legacy.commit_platform_navigation_with_handling_and_projection(&source,source.identity,&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),handling,||Ok::<_,&str>(())).unwrap();
        let (_,e2)=commit(&mut actual,&attempt,&o2,handling).unwrap();assert_eq!(e1,e2);
        assert_eq!(entry_projection(&legacy),entry_projection(&actual));
    }
    // Actual canonical allocation and attachment of a nested descendant. A
    // cross-document Push drops it only from the new joint map; earlier entries
    // and their snapshots remain reachable, matching the compatibility path.
    for attempt_path in [false,true] {
        let (mut h,o,a)=setup(true);let source=current(&h,&a,&o);
        let (nested,_,nested_entry)=h.reserve_platform_initial_child_with_projection(&source,source.identity,&o,6,owner(6),&o,
            &url("about:blank"),snap(6),||Ok::<_,&str>(())).unwrap();
        h.attach_platform_initial_child(&nested,&source,source.identity,&o).unwrap();
        let prior=h.steps.clone();let count=h.steps.len();
        if attempt_path {let attempt=admit(&mut h,&a,&o);commit(&mut h,&attempt,&o,PlatformNavigationHandling::Push).unwrap();}
        else {h.commit_platform_navigation_with_handling_and_projection(&source,source.identity,&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),PlatformNavigationHandling::Push,||Ok::<_,&str>(())).unwrap();}
        assert_eq!(h.steps.len(),count+1);assert_eq!(&h.steps[..count],prior.as_slice());
        assert!(!h.steps.last().unwrap().entries.contains_key(&6));
        assert_eq!(h.entries[&nested_entry.id].snapshot,snap(6));
        assert!(h.documents.contains_key(&nested_entry.document));
    }
}
#[test]
fn same_document_push_replace_and_entry_drift_rederive_current_source() {
    for kind in [CommitKind::Push,CommitKind::Replace] {
        let (mut h,o,a)=setup(true);let attempt=admit(&mut h,&a,&o);let old=current(&h,&a,&o);
        let (moved,_)=h.commit_platform_entry_with_projection(&old,old.identity,&o,kind,
            &url("https://example.test/state"),snap(8),||Ok::<_,&str>(())).unwrap();
        assert_ne!(moved.entry,old.entry);assert_eq!(moved.document,old.document);
        let (_,entry)=commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
        assert_eq!(h.platform_current_entry(2),Some(entry));
    }
}
#[test]
fn attachment_between_acceptance_and_publication_uses_actual_assigned_entry() {
    let (mut h,o,a)=setup(false);let attempt=admit(&mut h,&a,&o);
    let parent=h.platform_document_token(1,owner(1),&o).unwrap();h.attach_platform_initial_child(&a,&parent,owner(1),&o).unwrap();
    let (_,entry)=commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
    assert!(entry.step.assigned().is_some());assert!(h.platform_initial_children[&2].attached);
}
#[test]
fn validation_owner_origin_snapshot_and_capacity_errors_never_project_or_supersede() {
    for failure in 0..6 {
        let (mut h,o,a)=setup(false);let attempt=admit(&mut h,&a,&o);let t=current(&h,&a,&o);
        let mut new=owner(4);let mut snapshot=snap(4);let mut old=t.identity;let mut old_origin=o.clone();
        match failure {0=>new.runtime_id=0,1=>new=owner(1),2=>snapshot.history_state=Some(vec![0;MAX_STATE_BYTES+1]),
            3=>h.next_id=u64::MAX-2,4=>old.node_generation+=1,5=>old_origin=Origin::opaque(),_=>unreachable!()}
        let before=canonical(&h);let calls=Cell::new(0);
        assert!(h.commit_platform_child_navigation_attempt_with_projection(&attempt,old,&old_origin,new,&o,
            &url("https://example.test/new"),snapshot,PlatformNavigationHandling::Replace,||{calls.set(1);Ok::<_,&str>(())}).is_err());
        assert_eq!(calls.get(),0);assert_eq!(canonical(&h),before);
    }
}
#[test]
fn pending_push_and_epoch_exhaustion_preserve_authentic_current_admission() {
    for exhaust in [false,true] {
        let (mut h,o,a)=setup(false);let attempt=admit(&mut h,&a,&o);if exhaust {h.epoch=u64::MAX;}
        let before=canonical(&h);let calls=Cell::new(0);
        assert!(h.commit_platform_child_navigation_attempt_with_projection(&attempt,owner(2),&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),if exhaust{PlatformNavigationHandling::Replace}else{PlatformNavigationHandling::Push},
            ||{calls.set(1);Ok::<_,&str>(())}).is_err());assert_eq!(calls.get(),0);assert_eq!(canonical(&h),before);
    }
}
#[test]
fn sibling_admission_survives_successful_exact_consumption() {
    let (mut h,o,a)=setup(false);let parent=h.platform_document_token(1,owner(1),&o).unwrap();
    let (sibling,t,_)=h.reserve_platform_initial_child_with_projection(&parent,owner(1),&o,5,owner(5),&o,
        &url("about:blank"),snap(5),||Ok::<_,&str>(())).unwrap();
    let sib=h.admit_platform_child_navigation(&sibling,&t,owner(5),&o).unwrap();let attempt=admit(&mut h,&a,&o);
    commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
    assert_eq!(h.current_platform_child_navigation(&sib,owner(5),&o).unwrap(),t);
}
#[test]
fn destruction_recreation_parent_retirement_and_foreign_store_do_not_project() {
    for mode in 0..3 {
        let (mut h,o,a)=setup(false);let attempt=admit(&mut h,&a,&o);
        match mode {0=>{h.destroy_platform_initial_child(&a).unwrap();let parent=h.platform_document_token(1,owner(1),&o).unwrap();h.reserve_platform_initial_child_with_projection(&parent,owner(1),&o,2,owner(2),&o,&url("about:blank"),snap(2),||Ok::<_,&str>(())).unwrap();},1=>{h.retire_platform_document(&h.platform_document_token(1,owner(1),&o).unwrap(),owner(1),&o).unwrap();},
            2=>{let (other,_,_)=setup(false);h=other;},_=>unreachable!()}
        let before=canonical(&h);let calls=Cell::new(0);
        assert!(h.commit_platform_child_navigation_attempt_with_projection(&attempt,owner(2),&o,owner(4),&o,
            &url("https://example.test/new"),snap(4),PlatformNavigationHandling::Replace,||{calls.set(1);Ok::<_,&str>(())}).is_err());
        assert_eq!(calls.get(),0);assert_eq!(canonical(&h),before);
    }
}
#[test]
fn distinct_opaque_target_and_new_origin_identity_survive_real_publication() {
    let (mut h,o,a)=setup(false);let old=current(&h,&a,&o);let opaque=Origin::opaque();
    let (fresh,_)=h.commit_platform_navigation_with_handling_and_projection(&old,old.identity,&o,owner(3),&opaque,
        &url("about:blank"),snap(3),PlatformNavigationHandling::Replace,||Ok::<_,&str>(())).unwrap();
    let attempt=h.admit_platform_child_navigation(&a,&fresh,owner(3),&opaque).unwrap();let new=Origin::opaque();
    let (committed,_)=h.commit_platform_child_navigation_attempt_with_projection(&attempt,owner(3),&opaque,owner(4),&new,
        &url("about:blank"),snap(4),PlatformNavigationHandling::Replace,||Ok::<_,&str>(())).unwrap();
    assert!(h.platform_document_token(2,owner(4),&opaque).is_err());assert_eq!(h.platform_document_token(2,owner(4),&new).unwrap(),committed);
}
#[test]
fn assigned_replace_updates_all_exact_aliases_and_repeated_commit_is_stale() {
    let (mut h,o,a)=setup(true);let source=current(&h,&a,&o);
    let parent=h.platform_document_token(1,owner(1),&o).unwrap();h.commit_platform_same_document(&parent,owner(1),&o,CommitKind::Push,
        &url("https://example.test/root-state"),snap(9)).unwrap();
    assert!(h.steps.iter().filter(|s|s.entries.get(&2)==Some(&source.entry)).count()>1);
    let attempt=admit(&mut h,&a,&o);let (_,entry)=commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).unwrap();
    assert!(h.steps.iter().all(|s|s.entries.get(&2)!=Some(&source.entry)));assert!(h.steps.iter().filter(|s|s.entries.get(&2)==Some(&entry.id)).count()>1);
    let before=canonical(&h);assert!(commit(&mut h,&attempt,&o,PlatformNavigationHandling::Replace).is_err());assert_eq!(canonical(&h),before);
}
