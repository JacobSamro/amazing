//! Browser-only platform entry projection into canonical session history.
//! This API never accepts a renderer ticket or recreates opaque origin identity
//! from a serialized URL. The embedding validates the live native document
//! before passing its immutable identity and actual security origin.
use super::*;
use amazing_net::url::ParsedUrl;
#[path="history_platform_navigation.rs"]
mod navigation;
pub use navigation::PlatformNavigationAttempt;
pub(super) use navigation::NavigationAdmissionState;
#[path="history_platform_child.rs"]
mod child;
pub use child::{PlatformEntry,PlatformEntryStep,PlatformInitialChild,PlatformNavigationHandling};
pub(super) use child::InitialChildRecord;
#[path="history_platform_traversal.rs"]
mod traversal;
pub use traversal::{PlatformSameDocumentPlan, PlatformSameDocumentChange, PlatformOwnerEligibility, PlatformSnapshotUpdate, PlatformJointTraversal, PlatformTraversalEffect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformDocumentIdentity {
    pub runtime_id:u64,
    pub document:u32,
    pub node_generation:u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDocumentOwner { Renderer(u64), Platform(PlatformDocumentIdentity) }
/// Process-local browser capability. It is intentionally not a wire record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDocumentToken {
    session:u64,
    tab:u64,
    navigable:u64,
    document:u64,
    entry:u64,
    identity:PlatformDocumentIdentity,
    step:PlatformEntryStep,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformCommitError<E> { History(HistoryError), Projection(E) }
/// A pending browser reload of an existing history step. Staging does not
/// deactivate the current document or publish the target step. The capability
/// is private to this store and is invalidated by a newer navigation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformTraversalTicket {
    request:u64,
    source:PlatformDocumentToken,
    target_step:u64,
    target_entry:u64,
}
impl PlatformTraversalTicket {
    pub fn request_id(&self)->u64 {self.request}
    pub fn target_step(&self)->u64 {self.target_step}
    pub fn navigable_id(&self)->u64 {self.source.navigable}
}
impl PlatformDocumentToken {
    pub fn document_id(&self)->u64 {self.document}
    pub fn navigable_id(&self)->u64 {self.navigable}
}
fn rewrite_allowed(old:&ParsedUrl,new:&ParsedUrl)->bool {
    if old.scheme()!=new.scheme() || old.username()!=new.username() || old.password()!=new.password()
        || old.host()!=new.host() || old.port()!=new.port() {return false;}
    match old.scheme() {
        "http"|"https"=>true,
        "file"=>old.pathname()==new.pathname(),
        _=>old.pathname()==new.pathname() && old.query()==new.query(),
    }
}
impl SessionHistory {
    /// Publish an actually successful browser navigation while retaining the
    /// old active document until the trusted native projection succeeds.
    pub fn commit_platform_navigation_with_projection<E>(&mut self,source:&PlatformDocumentToken,
        source_identity:PlatformDocumentIdentity,source_origin:&Origin,new_identity:PlatformDocumentIdentity,
        new_origin:&Origin,url:&ParsedUrl,snapshot:HistorySnapshot,projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,SessionHistoryEntry),PlatformCommitError<E>> {
        if source.step==PlatformEntryStep::Pending {return Err(PlatformCommitError::History(HistoryError::Order));}
        let (token,entry)=self.commit_platform_navigation_with_handling_and_projection(source,source_identity,source_origin,
            new_identity,new_origin,url,snapshot,PlatformNavigationHandling::Push,projection)?;
        Ok((token,entry.assigned_entry().expect("assigned legacy platform navigation")))
    }

    /// Apply a same-document traversal only after the embedding projects the
    /// genuine active Document URL. All fallible store checks precede projection.
    pub fn traverse_platform_same_document_with_projection<E>(&mut self,
        source:&PlatformDocumentToken, identity:PlatformDocumentIdentity,
        origin:&Origin, target_step:u64, projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,Traversal),PlatformCommitError<E>> {
        let plan=self.plan_platform_same_document_traversal(source,identity,origin,target_step)
            .map_err(PlatformCommitError::History)?;
        // The legacy callback certifies only its actual owner. Another changed
        // same-document entry needs the explicit multi-owner transaction API.
        if plan.changes().iter().any(|change|change.from.navigable!=source.navigable) {
            return Err(PlatformCommitError::History(HistoryError::Order));
        }
        let snapshots:Vec<_>=plan.changes().iter().map(|change|PlatformSnapshotUpdate {
            token:change.source_token.clone(),snapshot:change.from.snapshot.clone(),
        }).collect();
        let mut eligible=plan.required_owners().to_vec();
        // Legacy callers retain their existing native fully-active validation
        // contract; the explicit API never invents that host result.
        for owner in &mut eligible {owner.fully_active=true;}
        let result=self.apply_platform_same_document_plan_with_projection(&plan,
            &eligible,&snapshots,|_|projection())?;
        let token=result.tokens[&source.navigable].clone();
        Ok((token,result.traversal))
    }
    /// Stage a cross-document traversal reload without retiring the active
    /// source. The embedding retains this ticket with its actual navigation
    /// request; a failed or superseded response cannot activate its target.
    pub fn stage_platform_traversal(&mut self,source:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,target_step:u64)
        ->Result<(PlatformTraversalTicket,SessionHistoryEntry),HistoryError> {
        self.stage_platform_reload_target(source,identity,origin,target_step,false)
    }
    /// Reload the currently selected existing entry into a fresh actual owner.
    /// This is distinct from a same-document traversal and does not insert a step.
    pub fn stage_platform_reload(&mut self,source:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin)
        ->Result<(PlatformTraversalTicket,SessionHistoryEntry),HistoryError> {
        self.stage_platform_reload_target(source,identity,origin,source.step.assigned().ok_or(HistoryError::Order)?,true)
    }
    fn stage_platform_reload_target(&mut self,source:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,target_step:u64,allow_current:bool)
        ->Result<(PlatformTraversalTicket,SessionHistoryEntry),HistoryError> {
        if self.pending.is_some() || self.queued_traversal.is_some() {return Err(HistoryError::Order);}
        if source.session!=self.instance_id || source.tab!=self.tab
            || *source!=self.active_platform_token(source.navigable,identity,origin)? {return Err(HistoryError::Stale);}
        let step=self.steps.iter().find(|step|step.id==target_step).ok_or(HistoryError::NotFound)?;
        let entry=step.entries.get(&source.navigable).and_then(|entry|self.entries.get(entry))
            .ok_or(HistoryError::NotFound)?.clone();
        if (!allow_current && entry.document==source.document)
            || !matches!(self.documents.get(&entry.document).map(|doc|doc.owner),Some(HistoryDocumentOwner::Platform(_))) {
            return Err(HistoryError::Invalid);
        }
        let ticket=PlatformTraversalTicket {request:self.mint()?,source:source.clone(),target_step,target_entry:entry.id};
        self.platform_traversal=Some(ticket.clone());
        Ok((ticket,entry))
    }
    /// Aborting a pending fetch leaves the active document, canonical current
    /// step and forward entries unchanged. A stale request cannot abort a newer
    /// request. This is controller cancellation, not web navigation cancellation.
    pub fn abort_platform_traversal(&mut self,ticket:&PlatformTraversalTicket)->Result<(),HistoryError> {
        if self.platform_traversal.as_ref()!=Some(ticket) {return Err(HistoryError::Stale);}
        self.platform_traversal=None;
        Ok(())
    }
    /// Activate a freshly loaded actual document at the selected existing step.
    /// This browser-only path does not reuse an old opaque origin identity or
    /// simulate bfcache. The target entry's snapshot and restoration mode are
    /// retained, and no forward entry is inserted or truncated.
    pub fn activate_platform_traversal_with_projection<E>(&mut self,ticket:&PlatformTraversalTicket,
        source_identity:PlatformDocumentIdentity,source_origin:&Origin,
        new_identity:PlatformDocumentIdentity,new_origin:&Origin,url:&ParsedUrl,
        projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,SessionHistoryEntry),PlatformCommitError<E>> {
        let validate=||->Result<(usize,u64),HistoryError> {
            if self.platform_traversal.as_ref()!=Some(ticket) || self.pending.is_some() || self.queued_traversal.is_some()
                || ticket.source.session!=self.instance_id || ticket.source.tab!=self.tab
                || ticket.source!=self.active_platform_token(ticket.source.navigable,source_identity,source_origin)? {
                return Err(HistoryError::Stale);
            }
            if new_identity.runtime_id==0 || self.documents.values().any(|doc|doc.owner==HistoryDocumentOwner::Platform(new_identity)) || self.documents.len()>=MAX_ENTRIES
                || url.serialize().len()>8192 {return Err(HistoryError::Invalid);}
            let index=self.steps.iter().position(|step|step.id==ticket.target_step).ok_or(HistoryError::Stale)?;
            if self.steps[index].entries.get(&ticket.source.navigable)!=Some(&ticket.target_entry)
                || !self.entries.contains_key(&ticket.target_entry) {return Err(HistoryError::Stale);}
            let mut ancestor=ticket.source.navigable;
            while let Some(Some(parent))=self.parents.get(&ancestor) {
                let active=self.steps[index].entries.get(parent).and_then(|id|self.entries.get(id))
                    .and_then(|entry|self.documents.get(&entry.document)).ok_or(HistoryError::Stale)?;
                if active.status!=DocumentStatus::Active {return Err(HistoryError::Stale);}
                ancestor=*parent;
            }
            // Until whole cross-document descendant activation is installed,
            // reject a joint step that would silently change another active
            // navigable's document. Same-document sibling entries remain valid.
            if let Some(current)=self.current {
                for (nav,entry) in &self.steps[current].entries {
                    if *nav==ticket.source.navigable {continue;}
                    let document=self.entries[entry].document;
                    if self.steps[index].entries.get(nav).and_then(|id|self.entries.get(id)).map(|entry|entry.document)!=Some(document)
                        && self.documents.get(&document).is_some_and(|doc|doc.status==DocumentStatus::Active) {
                        return Err(HistoryError::Order);
                    }
                }
            }
            self.next_id.checked_add(1).ok_or(HistoryError::Limit)?;
            let epoch=self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
            Ok((index,epoch))
        };
        let (index,epoch)=validate().map_err(PlatformCommitError::History)?;
        let url=url.serialize();let serialized_origin=new_origin.serialize();
        let actual_origin=new_origin.clone();
        // All fallible native entry staging must precede publication. No
        // author callbacks, conversions, network or navigation under this borrow.
        projection().map_err(PlatformCommitError::Projection)?;
        let document=self.mint().expect("activation arithmetic validated before projection");
        self.documents.get_mut(&ticket.source.document).expect("validated source owner").status=DocumentStatus::Discarded;
        self.documents.insert(document,DocumentState {id:document,owner:HistoryDocumentOwner::Platform(new_identity),
            origin:serialized_origin,status:DocumentStatus::Active,epoch,frozen_at_ms:None,freeze_deadline_ms:None,
            restore_pending:false,memory_bytes:0});
        self.platform_origins.insert(document,actual_origin);
        if let Some(parent)=self.parents[&ticket.source.navigable] {
            let parent_document=self.steps[index].entries.get(&parent).and_then(|entry|self.entries.get(entry))
                .expect("validated active target parent").document;
            self.platform_parent_documents.insert(document,parent_document);
        }
        let entry=self.entries.get_mut(&ticket.target_entry).expect("validated selected entry");
        entry.document=document;entry.url=url;
        self.current=Some(index);self.epoch=epoch;self.platform_traversal=None;
        let entry=self.entries[&ticket.target_entry].clone();
        let token=self.active_platform_token(ticket.source.navigable,new_identity,new_origin)
            .expect("selected browser document is active after validated publication");
        Ok((token,entry))
    }
    fn active_platform_token(&self,navigable:u64,identity:PlatformDocumentIdentity,origin:&Origin)
        ->Result<PlatformDocumentToken,HistoryError> {
        let entry=self.platform_current_entry(navigable).ok_or(HistoryError::Stale)?;
        let document=self.documents.get(&entry.document).ok_or(HistoryError::Stale)?;
        if document.owner!=HistoryDocumentOwner::Platform(identity) || document.status!=DocumentStatus::Active
            || self.platform_origins.get(&entry.document)!=Some(origin) {return Err(HistoryError::Stale);}
        let mut cursor=navigable;
        while let Some(Some(parent))=self.parents.get(&cursor) {
            let ancestor=self.platform_current_entry(*parent).ok_or(HistoryError::Stale)?;
            let current=self.platform_current_entry(cursor).ok_or(HistoryError::Stale)?;
            if self.documents.get(&ancestor.document).is_none_or(|doc|doc.status!=DocumentStatus::Active)
                || self.platform_parent_documents.get(&current.document).is_some_and(|expected|*expected!=ancestor.document) {return Err(HistoryError::Stale);}
            cursor=*parent;
        }
        Ok(PlatformDocumentToken {session:self.instance_id,tab:self.tab,navigable,document:entry.document,entry:entry.id,identity,
            step:if entry.step==PlatformEntryStep::Pending {PlatformEntryStep::Pending}
                else {PlatformEntryStep::Assigned(self.current_step().ok_or(HistoryError::Stale)?)}})
    }

    /// Obtain a fresh capability after an actual same-document traversal. The
    /// caller must recheck its native document/generation before deserialization.
    pub fn platform_document_token(&self,navigable:u64,identity:PlatformDocumentIdentity,origin:&Origin)
        ->Result<PlatformDocumentToken,HistoryError> {
        self.active_platform_token(navigable,identity,origin)
    }
    /// Project an actually committed browser document. A replaced document must
    /// first be retired; this does not simulate cross-document bfcache activation.
    pub fn attach_platform_document(&mut self,navigable:u64,identity:PlatformDocumentIdentity,
        url:&ParsedUrl,origin:&Origin,snapshot:HistorySnapshot)
        ->Result<(PlatformDocumentToken,SessionHistoryEntry),HistoryError> {
        snapshot.validate()?;
        if identity.runtime_id==0 || !self.parents.contains_key(&navigable) || self.pending.is_some()
            || self.queued_traversal.is_some() || self.entries.len()>=MAX_ENTRIES || self.documents.len()>=MAX_ENTRIES
            || url.serialize().len()>8192 {return Err(HistoryError::Invalid);}
        if self.active_entry(navigable).and_then(|entry|self.documents.get(&entry.document))
            .is_some_and(|document|document.status!=DocumentStatus::Discarded) {return Err(HistoryError::Order);}
        let mut cursor=navigable;
        while let Some(Some(parent))=self.parents.get(&cursor) {
            let ancestor=self.active_entry(*parent).and_then(|entry|self.documents.get(&entry.document))
                .ok_or(HistoryError::Stale)?;
            if ancestor.status!=DocumentStatus::Active {return Err(HistoryError::Stale);}
            cursor=*parent;
        }
        // Reserve all arithmetic before mutating any canonical map/counter.
        self.next_id.checked_add(3).ok_or(HistoryError::Limit)?;
        let epoch=self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        let document=self.mint()?;
        self.epoch=epoch;
        self.documents.insert(document,DocumentState {id:document,owner:HistoryDocumentOwner::Platform(identity),
            origin:origin.serialize(),status:DocumentStatus::Active,epoch,frozen_at_ms:None,
            freeze_deadline_ms:None,restore_pending:false,memory_bytes:0});
        self.platform_origins.insert(document,origin.clone());
        if let Some(parent)=self.parents[&navigable] {
            let parent_document=self.platform_current_entry(parent).expect("validated parent owner").document;
            self.platform_parent_documents.insert(document,parent_document);
        }
        let entry=self.insert_committed_entry(navigable,document,CommitKind::Navigate,url.serialize(),snapshot)?;
        let token=self.active_platform_token(navigable,identity,origin)?;
        Ok((token,entry))
    }
    /// Atomic same-document Push/Replace after observable storage serialization
    /// and native fully-active validation in the host. The canonical step,
    /// captured native owner and exact typed origin are all rechecked here.
    pub fn commit_platform_same_document(&mut self,token:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,kind:CommitKind,url:&ParsedUrl,snapshot:HistorySnapshot)
        ->Result<(PlatformDocumentToken,SessionHistoryEntry),HistoryError> {
        match self.commit_platform_same_document_with_projection(token,identity,origin,kind,url,snapshot,
            ||Ok::<(),std::convert::Infallible>(())) {
            Ok(value)=>Ok(value),Err(PlatformCommitError::History(error))=>Err(error),
            Err(PlatformCommitError::Projection(error))=>match error {},
        }
    }
    /// The callback is a trusted browser-native transaction boundary, not a
    /// script hook. It may only stage entry into the actual existing realm and
    /// update that native document's URL metadata/backing location. It must
    /// fail before its first mutation, or roll back all staged mutations.
    /// No script conversion, author callback, network or navigation may run.
    pub fn commit_platform_same_document_with_projection<E>(&mut self,token:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,kind:CommitKind,url:&ParsedUrl,snapshot:HistorySnapshot,
        projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,SessionHistoryEntry),PlatformCommitError<E>> {
        if token.step==PlatformEntryStep::Pending {return Err(PlatformCommitError::History(HistoryError::Order));}
        let (token,entry)=self.commit_platform_entry_with_projection(token,identity,origin,kind,url,snapshot,projection)?;
        Ok((token,entry.assigned_entry().expect("assigned caller remains assigned")))
    }

    /// Web-visible entry metadata changes do not mint an entry or advance the
    /// joint history step. The caller performs enum conversion first, then
    /// supplies a freshly validated native owner capability.
    pub fn set_platform_scroll_restoration(&mut self,token:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,mode:ScrollRestoration)->Result<(),HistoryError> {
        if token.session!=self.instance_id || token.tab!=self.tab
            || *token!=self.active_platform_token(token.navigable,identity,origin)? {
            return Err(HistoryError::Stale);
        }
        if let Some(child)=self.platform_initial_children.get_mut(&token.navigable) {
            child.entry.scroll_restoration=mode;
            if child.attachment_entry.id==token.entry {child.attachment_entry.scroll_restoration=mode;}
        }
        if token.step!=PlatformEntryStep::Pending {
            self.entries.get_mut(&token.entry).ok_or(HistoryError::Stale)?.scroll_restoration=mode;
        }

        Ok(())
    }
    pub fn retire_platform_document(&mut self,token:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin)->Result<(),HistoryError> {
        // Retirement is a browser lifecycle operation, including descendants
        // after their parent became inactive. It must not require fully-active
        // ancestry or the same joint step as a web-visible mutation.
        if token.session!=self.instance_id || token.tab!=self.tab || token.identity!=identity
            || self.platform_origins.get(&token.document)!=Some(origin)
            || self.documents.get(&token.document).is_none_or(|document|
                document.owner!=HistoryDocumentOwner::Platform(identity) || document.status!=DocumentStatus::Active) {
            return Err(HistoryError::Stale);
        }
        self.documents.get_mut(&token.document).unwrap().status=DocumentStatus::Discarded;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity()->PlatformDocumentIdentity {PlatformDocumentIdentity {runtime_id:7,document:0,node_generation:3}}
    fn url(value:&str)->ParsedUrl {ParsedUrl::parse(value).unwrap()}
    fn snapshot(value:u8)->HistorySnapshot {HistorySnapshot {history_state:Some(vec![value]),..Default::default()}}
    fn attached(value:&str)->(SessionHistory,PlatformDocumentToken,Origin) {
        let mut history=SessionHistory::new(1,1).unwrap();let origin=Origin::opaque();
        let (token,_)=history.attach_platform_document(1,identity(),&url(value),&origin,snapshot(1)).unwrap();
        (history,token,origin)
    }
    #[test]
    fn browser_push_replace_traverse_and_forward_prune_use_canonical_entries() {
        let (mut h,token,origin)=attached("https://example.test/a");let first=h.current_step().unwrap();
        let (token,b)=h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/b"),snapshot(2)).unwrap();
        let (_,c)=h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Replace,&url("https://example.test/c"),snapshot(3)).unwrap();
        assert_eq!(b.step,c.step);assert_eq!(h.steps().len(),2);assert_eq!(b.document,c.document);
        let traversal=h.traverse(first).unwrap();assert_eq!(traversal.entries[&1].snapshot.history_state,Some(vec![1]));
        let token=h.platform_document_token(1,identity(),&origin).unwrap();
        let (_,d)=h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/d"),snapshot(4)).unwrap();
        assert_eq!(h.steps().len(),2);assert!(h.entry(c.id).is_none());assert_eq!(h.active_entry(1),Some(&d));
    }
    #[test]
    fn platform_tokens_reject_wrong_owner_generation_step_and_opaque_origin_atomically() {
        let (mut h,token,origin)=attached("about:blank");let old=h.active_entry(1).unwrap().clone();
        for wrong in [PlatformDocumentIdentity {runtime_id:8,..identity()},PlatformDocumentIdentity {node_generation:4,..identity()}] {
            assert!(h.commit_platform_same_document(&token,wrong,&origin,CommitKind::Push,&url("about:blank#x"),snapshot(2)).is_err());
            assert_eq!(h.active_entry(1),Some(&old));
        }
        let foreign=Origin::opaque();assert_eq!(origin.serialize(),foreign.serialize());
        assert!(h.commit_platform_same_document(&token,identity(),&foreign,CommitKind::Push,&url("about:blank#x"),snapshot(2)).is_err());
        let (_,new)=h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("about:blank#x"),snapshot(2)).unwrap();
        assert!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("about:blank#y"),snapshot(3)).is_err());
        assert_eq!(h.active_entry(1),Some(&new));
    }
    #[test]
    fn rewrite_permission_covers_http_file_and_opaque_components() {
        for (old,allowed,denied) in [
            ("https://example.test/a","https://example.test/b?q#f","https://other.test/b"),
            ("file:///a","file:///a?q#f","file:///b"),
            ("about:blank","about:blank#f","about:blank?q"),
            ("data:text/plain,a","data:text/plain,a#f","data:text/plain,b"),
        ] {
            let (mut h,token,origin)=attached(old);let original=h.active_entry(1).unwrap().clone();
            assert!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url(denied),snapshot(2)).is_err());
            assert_eq!(h.active_entry(1),Some(&original));
            assert!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url(allowed),snapshot(2)).is_ok());
        }
    }
    #[test]
    fn platform_entries_never_enter_renderer_freeze_restore_snapshot_or_crash_paths() {
        let (mut h,token,origin)=attached("https://example.test/a");let document=token.document_id();
        assert!(h.begin(9,1,Some(document),CommitKind::Push).is_err());
        assert!(h.save_snapshot(9,document,snapshot(2)).is_err());
        assert!(h.freeze_request(document,9,h.document(document).unwrap().epoch,Eligibility::default(),1024,0).is_err());
        assert!(h.restore_request(document,9,0).is_err());assert!(h.discard(document).is_err());
        assert!(h.renderer_crashed(9).is_empty());assert!(h.platform_document_token(1,identity(),&origin).is_ok());
    }
    #[test]
    fn bounded_snapshot_and_counter_failures_leave_history_unchanged() {
        let (mut h,token,origin)=attached("https://example.test/a");let original=h.active_entry(1).unwrap().clone();
        let huge=HistorySnapshot {history_state:Some(vec![0;MAX_STATE_BYTES+1]),..Default::default()};
        assert_eq!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/b"),huge),Err(HistoryError::Limit));
        h.next_id=u64::MAX;assert_eq!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/b"),snapshot(2)),Err(HistoryError::Limit));
        assert_eq!(h.active_entry(1),Some(&original));assert_eq!(h.steps().len(),1);
    }
    #[test]
    fn replacement_document_gets_fresh_private_origin_and_retired_capability_cannot_restore() {
        let (mut h,token,origin)=attached("about:blank");
        assert!(h.attach_platform_document(1,identity(),&url("about:blank"),&origin,snapshot(2)).is_err());
        h.retire_platform_document(&token,identity(),&origin).unwrap();
        let fresh=Origin::opaque();let owner=PlatformDocumentIdentity {runtime_id:8,..identity()};
        let (new,_)=h.attach_platform_document(1,owner,&url("about:blank"),&fresh,snapshot(2)).unwrap();
        assert_ne!(new.document_id(),token.document_id());assert!(h.platform_document_token(1,identity(),&origin).is_err());
        assert!(h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("about:blank#x"),snapshot(3)).is_err());
    }
    #[test]
    fn projection_failure_and_validation_errors_leave_entries_steps_and_forward_branch_atomic() {
        use std::cell::Cell;
        let (mut h,first,origin)=attached("https://example.test/a");let step=h.current_step().unwrap();
        h.commit_platform_same_document(&first,identity(),&origin,CommitKind::Push,&url("https://example.test/b"),snapshot(2)).unwrap();
        h.traverse(step).unwrap();let token=h.platform_document_token(1,identity(),&origin).unwrap();
        let entries=h.entries.clone();let steps=h.steps.clone();let next_id=h.next_id;let epoch=h.epoch;
        let calls=Cell::new(0);let native_url=Cell::new("https://example.test/a");
        for kind in [CommitKind::Push,CommitKind::Replace] {
            let result=h.commit_platform_same_document_with_projection(&token,identity(),&origin,kind,&url("https://example.test/c"),snapshot(3),||{
                calls.set(calls.get()+1);Err("realm entry rejected")
            });
            assert_eq!(result,Err(PlatformCommitError::Projection("realm entry rejected")));
            assert_eq!(h.entries,entries);assert_eq!(h.steps,steps);assert_eq!(h.next_id,next_id);assert_eq!(h.epoch,epoch);
            assert_eq!(h.current_step(),Some(step));assert_eq!(native_url.get(),"https://example.test/a");
        }
        let count=calls.get();
        assert!(matches!(h.commit_platform_same_document_with_projection(&token,identity(),&origin,CommitKind::Push,&url("https://other.test/"),snapshot(3),||{calls.set(calls.get()+1);Ok::<(),&str>(())}),Err(PlatformCommitError::History(_))));
        assert_eq!(calls.get(),count);
        h.commit_platform_same_document_with_projection(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/c"),snapshot(3),||{
            native_url.set("https://example.test/c");Ok::<(),&str>(())
        }).unwrap();
        assert_eq!(native_url.get(),h.active_entry(1).unwrap().url);assert_eq!(h.steps().len(),2);
    }

    #[test]
    fn canonical_scroll_restoration_is_per_entry_inherited_and_owner_guarded() {
        let (mut h,token,origin)=attached("https://example.test/a");let first=h.current_step().unwrap();
        assert_eq!(h.active_entry(1).unwrap().scroll_restoration,ScrollRestoration::Auto);
        h.set_platform_scroll_restoration(&token,identity(),&origin,ScrollRestoration::Manual).unwrap();
        let (next,_)=h.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("https://example.test/b"),snapshot(2)).unwrap();
        assert_eq!(h.active_entry(1).unwrap().scroll_restoration,ScrollRestoration::Manual);
        assert!(h.set_platform_scroll_restoration(&token,identity(),&origin,ScrollRestoration::Auto).is_err());
        h.set_platform_scroll_restoration(&next,identity(),&origin,ScrollRestoration::Auto).unwrap();
        let (_,replacement)=h.commit_platform_same_document(&next,identity(),&origin,CommitKind::Replace,&url("https://example.test/c"),snapshot(3)).unwrap();
        assert_eq!(replacement.scroll_restoration,ScrollRestoration::Auto);
        h.traverse(first).unwrap();assert_eq!(h.active_entry(1).unwrap().scroll_restoration,ScrollRestoration::Manual);
        assert_eq!(h.steps().len(),2);
    }

    #[test]
    fn process_local_capabilities_do_not_cross_even_identically_numbered_stores() {
        let origin=Origin::opaque();let mut one=SessionHistory::new(1,1).unwrap();let mut two=SessionHistory::new(1,1).unwrap();
        let (token,_)=one.attach_platform_document(1,identity(),&url("about:blank"),&origin,snapshot(1)).unwrap();
        two.attach_platform_document(1,identity(),&url("about:blank"),&origin,snapshot(1)).unwrap();
        assert!(two.commit_platform_same_document(&token,identity(),&origin,CommitKind::Push,&url("about:blank#x"),snapshot(2)).is_err());
        assert_eq!(two.steps().len(),1);
    }

    #[test]
    fn platform_joint_steps_preserve_child_entries_and_retirement_does_not_require_live_ancestors() {
        let (mut h,root,origin)=attached("https://example.test/root");h.add_child(1,2).unwrap();
        let child_owner=PlatformDocumentIdentity {document:1,..identity()};let child_origin=Origin::opaque();
        let (child,child_entry)=h.attach_platform_document(2,child_owner,&url("about:blank"),&child_origin,snapshot(2)).unwrap();
        let fresh=h.platform_document_token(1,identity(),&origin).unwrap();
        let (_,root_entry)=h.commit_platform_same_document(&fresh,identity(),&origin,CommitKind::Push,&url("https://example.test/root#next"),snapshot(3)).unwrap();
        assert_eq!(h.active_entry(2),Some(&child_entry));assert_eq!(h.active_entry(1),Some(&root_entry));
        h.retire_platform_document(&root,identity(),&origin).unwrap();
        assert!(h.platform_document_token(2,child_owner,&child_origin).is_err());
        h.retire_platform_document(&child,child_owner,&child_origin).unwrap();
        assert_eq!(h.document(child.document_id()).unwrap().status,DocumentStatus::Discarded);
    }

    fn two_documents()->(SessionHistory,PlatformDocumentToken,Origin,u64) {
        let (mut h,first,origin)=attached("about:blank");let step=h.current_step().unwrap();
        h.retire_platform_document(&first,identity(),&origin).unwrap();
        let current_origin=Origin::opaque();let current_owner=PlatformDocumentIdentity {runtime_id:8,..identity()};
        let (current,_)=h.attach_platform_document(1,current_owner,&url("about:blank#second"),&current_origin,snapshot(2)).unwrap();
        (h,current,current_origin,step)
    }
    #[test]
    fn pending_reload_failure_newer_request_and_projection_error_preserve_active_source_and_forward_entries() {
        let (mut h,current,origin,target)=two_documents();
        let current_owner=PlatformDocumentIdentity {runtime_id:8,..identity()};
        let steps=h.steps.clone();let entries=h.entries.clone();let current_step=h.current_step();
        let (old,_)=h.stage_platform_traversal(&current,current_owner,&origin,target).unwrap();
        assert_eq!(h.current_step(),current_step);assert!(h.platform_document_token(1,current_owner,&origin).is_ok());
        h.abort_platform_traversal(&old).unwrap();
        assert_eq!(h.steps,steps);assert_eq!(h.entries,entries);
        let (old,_)=h.stage_platform_traversal(&current,current_owner,&origin,target).unwrap();
        let (new,_)=h.stage_platform_traversal(&current,current_owner,&origin,target).unwrap();
        let fresh_owner=PlatformDocumentIdentity {runtime_id:9,..identity()};let fresh_origin=Origin::opaque();
        assert!(matches!(h.activate_platform_traversal_with_projection(&old,current_owner,&origin,fresh_owner,&fresh_origin,&url("about:blank"),||Ok::<_,&str>(())),Err(PlatformCommitError::History(HistoryError::Stale))));
        assert_eq!(h.abort_platform_traversal(&old),Err(HistoryError::Stale));
        assert_eq!(h.activate_platform_traversal_with_projection(&new,current_owner,&origin,fresh_owner,&fresh_origin,&url("about:blank"),||Err("native activation rejected")),Err(PlatformCommitError::Projection("native activation rejected")));
        assert_eq!(h.steps,steps);assert_eq!(h.entries,entries);assert_eq!(h.current_step(),current_step);
        assert_eq!(h.documents[&current.document].status,DocumentStatus::Active);
        assert_eq!(h.platform_traversal.as_ref(),Some(&new));
    }
    #[test]
    fn successful_navigation_retires_only_at_publication_and_failed_projection_preserves_current_owner() {
        let (mut h,source,origin)=attached("https://example.test/a");
        let old_entry=h.active_entry(1).unwrap().clone();let old_steps=h.steps.clone();let old_counter=h.next_id;
        let next=PlatformDocumentIdentity {runtime_id:8,..identity()};let fresh_origin=Origin::opaque();
        let result=h.commit_platform_navigation_with_projection(&source,identity(),&origin,next,&fresh_origin,
            &url("https://example.test/b"),snapshot(2),||Err("activation failed"));
        assert_eq!(result,Err(PlatformCommitError::Projection("activation failed")));
        assert_eq!(h.active_entry(1),Some(&old_entry));assert_eq!(h.steps,old_steps);assert_eq!(h.next_id,old_counter);
        assert_eq!(h.document(source.document_id()).unwrap().status,DocumentStatus::Active);
        let (_,entry)=h.commit_platform_navigation_with_projection(&source,identity(),&origin,next,&fresh_origin,
            &url("https://example.test/b"),snapshot(2),||Ok::<_,&str>(())).unwrap();
        assert_eq!(h.steps.len(),2);assert_eq!(entry.snapshot,snapshot(2));
        assert_eq!(h.document(source.document_id()).unwrap().status,DocumentStatus::Discarded);
        assert!(h.platform_document_token(1,next,&fresh_origin).is_ok());
        assert!(h.commit_platform_navigation_with_projection(&source,identity(),&origin,
            PlatformDocumentIdentity {runtime_id:9,..identity()},&Origin::opaque(),&url("https://example.test/stale"),snapshot(3),
            ||Ok::<_,&str>(())).is_err());
        assert_eq!(h.active_entry(1),Some(&entry));
    }
    #[test]
    fn successful_reload_retains_selected_snapshot_entry_count_and_mints_fresh_opaque_identity() {
        let (mut h,current,origin,target)=two_documents();let count=h.entries.len();let steps=h.steps.clone();
        let current_owner=PlatformDocumentIdentity {runtime_id:8,..identity()};
        let (ticket,selected)=h.stage_platform_traversal(&current,current_owner,&origin,target).unwrap();
        let fresh_owner=PlatformDocumentIdentity {runtime_id:9,node_generation:4,..identity()};let fresh_origin=Origin::opaque();
        let (fresh,entry)=h.activate_platform_traversal_with_projection(&ticket,current_owner,&origin,fresh_owner,&fresh_origin,&url("about:blank"),||Ok::<_,&str>(())).unwrap();
        assert_eq!(h.entries.len(),count);assert_eq!(h.steps,steps);assert_eq!(h.current_step(),Some(target));
        assert_eq!(entry.id,selected.id);assert_eq!(entry.snapshot,selected.snapshot);assert_eq!(entry.scroll_restoration,selected.scroll_restoration);
        assert_ne!(entry.document,selected.document);assert_eq!(h.documents[&current.document].status,DocumentStatus::Discarded);
        assert!(h.platform_document_token(1,fresh_owner,&origin).is_err());
        assert!(h.platform_document_token(1,fresh_owner,&fresh_origin).is_ok());
        assert!(h.retire_platform_document(&current,current_owner,&origin).is_err());
        let other_step=steps.last().unwrap().id;
        let (back,_) =h.stage_platform_traversal(&fresh,fresh_owner,&fresh_origin,other_step).unwrap();
        let next_owner=PlatformDocumentIdentity {runtime_id:10,..identity()};let next_origin=Origin::opaque();
        h.activate_platform_traversal_with_projection(&back,fresh_owner,&fresh_origin,next_owner,&next_origin,&url("about:blank#second"),||Ok::<_,&str>(())).unwrap();
        assert_eq!(h.entries.len(),count);assert_eq!(h.steps,steps);
    }
    #[test]
    fn reload_capabilities_reject_other_store_stale_step_and_wrong_origin_before_projection() {
        use std::cell::Cell;
        let (mut h,current,origin,target)=two_documents();let owner=PlatformDocumentIdentity {runtime_id:8,..identity()};
        let (ticket,_)=h.stage_platform_traversal(&current,owner,&origin,target).unwrap();let calls=Cell::new(0);
        let wrong_origin=Origin::opaque();let next=PlatformDocumentIdentity {runtime_id:9,..identity()};
        assert!(h.activate_platform_traversal_with_projection(&ticket,owner,&wrong_origin,next,&Origin::opaque(),&url("about:blank"),||{calls.set(calls.get()+1);Ok::<_,&str>(())}).is_err());
        h.commit_platform_same_document(&current,owner,&origin,CommitKind::Push,&url("about:blank#changed"),snapshot(3)).unwrap();
        assert!(h.activate_platform_traversal_with_projection(&ticket,owner,&origin,next,&Origin::opaque(),&url("about:blank"),||{calls.set(calls.get()+1);Ok::<_,&str>(())}).is_err());
        let (mut other,other_current,other_origin,other_target)=two_documents();
        other.stage_platform_traversal(&other_current,owner,&other_origin,other_target).unwrap();
        assert!(other.activate_platform_traversal_with_projection(&ticket,owner,&other_origin,next,&Origin::opaque(),&url("about:blank"),||{calls.set(calls.get()+1);Ok::<_,&str>(())}).is_err());
        assert_eq!(calls.get(),0);
    }

    #[test]
    fn projected_same_document_traversal_is_atomic_and_preserves_canonical_entries() {
        let (mut h,first,origin)=attached("https://example.test/a");
        let step=first.step.assigned().unwrap();
        let (current,entry)=h.commit_platform_same_document(&first,identity(),&origin,
            CommitKind::Push,&url("https://example.test/b"),snapshot(2)).unwrap();
        let epoch=h.epoch;let steps=h.steps.clone();let next=h.next_id;
        let result=h.traverse_platform_same_document_with_projection(&current,identity(),&origin,step,
            ||Err::<(),_>("realm entry failed"));
        assert_eq!(result,Err(PlatformCommitError::Projection("realm entry failed")));
        assert_eq!(h.active_entry(1),Some(&entry));assert_eq!(h.epoch,epoch);
        assert_eq!(h.steps,steps);assert_eq!(h.next_id,next);
        let (fresh,traversal)=h.traverse_platform_same_document_with_projection(&current,identity(),&origin,step,
            ||Ok::<_,&str>(())).unwrap();
        assert_eq!(traversal.entries[&1].snapshot.history_state,Some(vec![1]));
        assert_eq!(fresh.document,current.document);assert_eq!(h.steps,steps);
        assert!(h.traverse_platform_same_document_with_projection(&current,identity(),&origin,entry.step,
            ||Ok::<_,&str>(())).is_err());
    }
    #[test]
    fn reload_current_entry_retains_step_state_and_requires_fresh_owner_at_activation() {
        let (mut h,current,origin)=attached("about:blank");
        let step=current.step.assigned().unwrap();let original=h.active_entry(1).unwrap().clone();
        let (ticket,target)=h.stage_platform_reload(&current,identity(),&origin).unwrap();
        assert_eq!(target,original);assert_eq!(h.current_step(),Some(step));
        assert_eq!(h.active_entry(1),Some(&original));
        assert!(h.activate_platform_traversal_with_projection(&ticket,identity(),&origin,identity(),&origin,
            &url("about:blank"),||Ok::<_,&str>(())).is_err());
        let new=PlatformDocumentIdentity {runtime_id:2,document:0,node_generation:1};
        let (_,activated)=h.activate_platform_traversal_with_projection(&ticket,identity(),&origin,new,
            &Origin::opaque(),&url("about:blank"),||Ok::<_,&str>(())).unwrap();
        assert_eq!(activated.id,original.id);assert_eq!(activated.step,original.step);
        assert_eq!(activated.snapshot,original.snapshot);assert_eq!(h.steps.len(),1);
    }

}
