//! Browser-only initial entry lineage. Pending is never a renderer step number.
use super::*;

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum PlatformEntryStep { Pending, Assigned(u64) }
impl PlatformEntryStep {
    pub fn assigned(self)->Option<u64> {match self {Self::Pending=>None,Self::Assigned(step)=>Some(step)}}
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct PlatformEntry {
    pub id:u64,pub navigable:u64,pub parent_navigable:Option<u64>,pub document:u64,
    pub step:PlatformEntryStep,pub url:String,pub snapshot:HistorySnapshot,
    pub scroll_restoration:ScrollRestoration,
}
impl PlatformEntry {
    pub fn assigned_entry(&self)->Option<SessionHistoryEntry> {
        Some(SessionHistoryEntry {id:self.id,step:self.step.assigned()?,navigable:self.navigable,
            parent_navigable:self.parent_navigable,document:self.document,url:self.url.clone(),
            snapshot:self.snapshot.clone(),scroll_restoration:self.scroll_restoration})
    }
}
impl From<&SessionHistoryEntry> for PlatformEntry {
    fn from(entry:&SessionHistoryEntry)->Self {Self {id:entry.id,navigable:entry.navigable,
        parent_navigable:entry.parent_navigable,document:entry.document,step:PlatformEntryStep::Assigned(entry.step),
        url:entry.url.clone(),snapshot:entry.snapshot.clone(),scroll_restoration:entry.scroll_restoration}}
}
/// Immutable allocation identity survives replacement of its actual initial owner.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct PlatformInitialChild {session:u64,tab:u64,navigable:u64,parent:u64,allocation:u64}
impl PlatformInitialChild {pub fn navigable_id(&self)->u64 {self.navigable}}
#[derive(Debug,Clone,PartialEq,Eq)]
pub(in crate::history) struct InitialChildRecord {pub(in crate::history) allocation:PlatformInitialChild,pub(in crate::history) entry:PlatformEntry,
    pub(in crate::history) attachment_entry:PlatformEntry,
    pub(in crate::history) attached:bool,
    pub(in crate::history) navigation:NavigationAdmissionState}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum PlatformNavigationHandling { Push, Replace }

/// Fully owned, private publication data. Preparation borrows the one authentic
/// store; this carrier cannot mint authority or escape the canonical operation.
pub(super) struct PreparedPlatformNavigation {
    next_id:u64, epoch:u64,
    entries:BTreeMap<u64,SessionHistoryEntry>, documents:BTreeMap<u64,DocumentState>,
    origins:BTreeMap<u64,Origin>, parent_documents:BTreeMap<u64,u64>,
    initial_documents:BTreeSet<u64>, pub(super) children:BTreeMap<u64,InitialChildRecord>,
    steps:Vec<HistoryStep>, current:Option<usize>, orphaned:Vec<HistoryDiscard>,
    result:(PlatformDocumentToken,PlatformEntry),
}

impl SessionHistory {
    /// Pure browser projection includes genuine pending entries, without insertion
    /// into joint maps or synthetic assigned steps. Renderer access is unchanged.
    pub fn platform_current_entry(&self,navigable:u64)->Option<PlatformEntry> {
        if let Some(child)=self.platform_initial_children.get(&navigable).filter(|child|!child.attached) {
            return Some(child.entry.clone());
        }
        self.active_entry(navigable).map(PlatformEntry::from)
    }
    fn platform_entry_count(&self)->usize {
        self.entries.len()+self.platform_initial_children.values().filter(|child|!child.attached).map(|child|
            usize::from(child.entry.step==PlatformEntryStep::Pending)+usize::from(child.attachment_entry.id!=child.entry.id)).sum::<usize>()
    }
    fn validate_platform_new_owner(&self,identity:PlatformDocumentIdentity,url:&ParsedUrl,snapshot:&HistorySnapshot)
        ->Result<(),HistoryError> {
        snapshot.validate()?;
        if identity.runtime_id==0 || self.documents.values().any(|doc|doc.owner==HistoryDocumentOwner::Platform(identity))
            || self.pending.is_some() || self.queued_traversal.is_some() || self.platform_entry_count()>=MAX_ENTRIES
            || self.documents.len()>=MAX_ENTRIES || url.serialize().len()>8192 {return Err(HistoryError::Invalid);}
        Ok(())
    }
    /// Called only at real initial child allocation, before fetch/author code.
    /// Projection must fail before native mutation; canonical checks all precede it.
    pub fn reserve_platform_initial_child_with_projection<E>(&mut self,parent:&PlatformDocumentToken,
        parent_identity:PlatformDocumentIdentity,parent_origin:&Origin,child_navigable:u64,
        child_identity:PlatformDocumentIdentity,child_origin:&Origin,url:&ParsedUrl,snapshot:HistorySnapshot,
        projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformInitialChild,PlatformDocumentToken,PlatformEntry),PlatformCommitError<E>> {
        let validate=(|| {
            if *parent!=self.active_platform_token(parent.navigable,parent_identity,parent_origin)?
                || self.parents.contains_key(&child_navigable) || child_navigable==0 {return Err(HistoryError::Stale);}
            if self.parents.len()>=MAX_NAVIGABLES {return Err(HistoryError::Limit);}
            self.validate_platform_new_owner(child_identity,url,&snapshot)?;
            self.next_id.checked_add(3).ok_or(HistoryError::Limit)?;
            self.epoch.checked_add(1).ok_or(HistoryError::Limit)
        })();
        let epoch=validate.map_err(PlatformCommitError::History)?;
        let url=url.serialize();let origin=child_origin.clone();let origin_text=origin.serialize();
        projection().map_err(PlatformCommitError::Projection)?;
        let allocation=self.mint().expect("reserved allocation");
        let document=self.mint().expect("reserved document");let id=self.mint().expect("reserved entry");
        let allocation=PlatformInitialChild {session:self.instance_id,tab:self.tab,navigable:child_navigable,
            parent:parent.navigable,allocation};
        let entry=PlatformEntry {id,navigable:child_navigable,parent_navigable:Some(parent.navigable),document,
            step:PlatformEntryStep::Pending,url,snapshot,scroll_restoration:ScrollRestoration::Auto};
        self.parents.insert(child_navigable,Some(parent.navigable));
        self.documents.insert(document,DocumentState {id:document,owner:HistoryDocumentOwner::Platform(child_identity),
            origin:origin_text,status:DocumentStatus::Active,epoch,frozen_at_ms:None,freeze_deadline_ms:None,
            restore_pending:false,memory_bytes:0});
        self.platform_origins.insert(document,origin);
        self.platform_parent_documents.insert(document,parent.document);
        self.platform_initial_documents.insert(document);
        self.platform_initial_children.insert(child_navigable,InitialChildRecord {allocation:allocation.clone(),entry:entry.clone(),attachment_entry:entry.clone(),attached:false,navigation:NavigationAdmissionState::default()});
        self.epoch=epoch;
        let token=self.active_platform_token(child_navigable,child_identity,child_origin).expect("actual initial owner reserved");
        Ok((allocation,token,entry))
    }
    /// Executes serialized creation traversal steps. Parent active DocumentState
    /// is read now, never captured current-step at allocation. Retargeting the
    /// child does not invalidate the immutable allocation lineage.
    pub fn attach_platform_initial_child(&mut self,allocation:&PlatformInitialChild,
        parent:&PlatformDocumentToken,parent_identity:PlatformDocumentIdentity,parent_origin:&Origin)
        ->Result<PlatformEntry,HistoryError> {
        if allocation.session!=self.instance_id || allocation.tab!=self.tab || allocation.parent!=parent.navigable
            || *parent!=self.active_platform_token(parent.navigable,parent_identity,parent_origin)?
            || parent.step==PlatformEntryStep::Pending {return Err(HistoryError::Stale);}
        if self.platform_initial_children.get(&parent.navigable).is_some_and(|parent|!parent.attached) {
            return Err(HistoryError::Order);
        }
        let record=self.platform_initial_children.get(&allocation.navigable).ok_or(HistoryError::Stale)?;
        if record.allocation!=*allocation || record.attached || self.parents.get(&allocation.navigable)!=Some(&Some(parent.navigable))
            || self.documents.get(&record.entry.document).is_none_or(|doc|doc.status!=DocumentStatus::Active) {
            return Err(HistoryError::Stale);
        }
        let first=self.steps.iter().position(|step|step.entries.get(&parent.navigable)
            .and_then(|id|self.entries.get(id)).is_some_and(|entry|entry.document==parent.document)).ok_or(HistoryError::Stale)?;
        let epoch=self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        let mut entry=record.attachment_entry.clone();entry.step=PlatformEntryStep::Assigned(self.steps[first].id);
        let assigned=entry.assigned_entry().expect("real assigned step");
        for step in &mut self.steps[first..] {
            if step.entries.get(&parent.navigable).and_then(|id|self.entries.get(id))
                .is_some_and(|parent_entry|parent_entry.document==parent.document) {
                step.entries.entry(allocation.navigable).or_insert(entry.id);
            }
        }
        self.platform_parent_documents.insert(entry.document,parent.document);
        self.entries.insert(entry.id,assigned);
        let record=self.platform_initial_children.get_mut(&allocation.navigable).expect("validated allocation");
        if record.entry.step==PlatformEntryStep::Pending {record.entry=entry.clone();}
        record.attachment_entry=entry.clone();record.attached=true;self.epoch=epoch;
        self.prune_unreachable();
        Ok(entry)
    }
    /// Actual destruction cancels only this allocation. Retained old tokens and
    /// globals do not authorize recreation or cancellation of a newer child.
    pub fn destroy_platform_initial_child(&mut self,allocation:&PlatformInitialChild)->Result<(),HistoryError> {
        let record=self.platform_initial_children.get(&allocation.navigable).ok_or(HistoryError::Stale)?;
        if allocation.session!=self.instance_id || allocation.tab!=self.tab || record.allocation!=*allocation {
            return Err(HistoryError::Stale);
        }
        let epoch=self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        let mut destroyed=BTreeSet::from([allocation.navigable]);
        loop {
            let previous=destroyed.len();
            for (nav,parent) in &self.parents {if parent.is_some_and(|parent|destroyed.contains(&parent)) {destroyed.insert(*nav);}}
            if destroyed.len()==previous {break;}
        }
        for nav in destroyed {
            if let Some(entry)=self.platform_current_entry(nav) {
                if let Some(document)=self.documents.get_mut(&entry.document) {
                    document.status=DocumentStatus::Discarded;
                }
            }
            self.platform_initial_children.remove(&nav);self.parents.remove(&nav);
            for step in &mut self.steps {step.entries.remove(&nav);}
        }

        self.epoch=epoch;self.prune_unreachable();Ok(())
    }
    fn replace_platform_assigned_entry(&mut self,source:&PlatformDocumentToken,document:u64,url:String,
        snapshot:HistorySnapshot,scroll:ScrollRestoration)->SessionHistoryEntry {
        let old=self.entries.get(&source.entry).expect("validated source entry");let step=old.step;
        let id=self.mint().expect("replacement arithmetic validated");
        let entry=SessionHistoryEntry {id,step,navigable:source.navigable,parent_navigable:self.parents[&source.navigable],
            document,url,snapshot,scroll_restoration:scroll};
        for joint in &mut self.steps {
            if joint.entries.get(&source.navigable)==Some(&source.entry) {joint.entries.insert(source.navigable,id);}
        }
        self.entries.insert(id,entry.clone());self.platform_traversal=None;self.prune_unreachable();entry
    }
    /// Explicit browser API works for pending and assigned owners. The assigned
    /// compatibility API refuses pending callers instead of fabricating a step.
    pub fn commit_platform_entry_with_projection<E>(&mut self,token:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,kind:CommitKind,url:&ParsedUrl,snapshot:HistorySnapshot,
        projection:impl FnOnce()->Result<(),E>)->Result<(PlatformDocumentToken,PlatformEntry),PlatformCommitError<E>> {
        let validate=(|| {
            snapshot.validate()?;
            if !matches!(kind,CommitKind::Push|CommitKind::Replace) || self.pending.is_some() || self.queued_traversal.is_some()
                || self.platform_entry_count()>=MAX_ENTRIES || url.serialize().len()>8192 {return Err(HistoryError::Invalid);}
            if *token!=self.active_platform_token(token.navigable,identity,origin)? {return Err(HistoryError::Stale);}
            let old=self.platform_current_entry(token.navigable).ok_or(HistoryError::Stale)?;
            if !rewrite_allowed(&ParsedUrl::parse(&old.url).map_err(|_|HistoryError::Invalid)?,url) {return Err(HistoryError::Invalid);}
            let kind=if self.platform_initial_documents.contains(&token.document) {CommitKind::Replace}else{kind};
            self.next_id.checked_add(if kind==CommitKind::Push {2}else{1}).ok_or(HistoryError::Limit)?;
            Ok((old,kind))
        })();
        let (old,kind)=validate.map_err(PlatformCommitError::History)?;let url=url.serialize();
        projection().map_err(PlatformCommitError::Projection)?;
        let entry=if old.step==PlatformEntryStep::Pending && kind==CommitKind::Replace {
            let id=self.mint().expect("pending replacement arithmetic");
            let entry=PlatformEntry {id,url,snapshot,..old};
            let record=self.platform_initial_children.get_mut(&token.navigable).expect("pending owner");
            record.entry=entry.clone();record.attachment_entry=entry.clone();entry
        } else if kind==CommitKind::Replace {
            PlatformEntry::from(&self.replace_platform_assigned_entry(token,token.document,url,snapshot,old.scroll_restoration))
        } else {
            PlatformEntry::from(&self.insert_committed_entry(token.navigable,token.document,kind,url,snapshot)
                .expect("assigned push validated"))
        };
        if let Some(record)=self.platform_initial_children.get_mut(&token.navigable) {record.entry=entry.clone();}
        let fresh=self.active_platform_token(token.navigable,identity,origin).expect("same actual owner retained");
        Ok((fresh,entry))
    }
    /// Browser navigation disposition is supplied from actual lifecycle; pending
    /// initial replacement preserves allocation lineage and does not create step.
    pub fn commit_platform_navigation_with_handling_and_projection<E>(&mut self,source:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,new_identity:PlatformDocumentIdentity,new_origin:&Origin,
        url:&ParsedUrl,snapshot:HistorySnapshot,handling:PlatformNavigationHandling,projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,PlatformEntry),PlatformCommitError<E>> {
        let prepared=self.prepare_platform_navigation(source,identity,origin,new_identity,new_origin,
            url,snapshot,handling).map_err(PlatformCommitError::History)?;
        projection().map_err(PlatformCommitError::Projection)?;
        Ok(self.publish_platform_navigation(prepared))
    }

    pub(super) fn prepare_platform_navigation(&self,source:&PlatformDocumentToken,
        identity:PlatformDocumentIdentity,origin:&Origin,new_identity:PlatformDocumentIdentity,new_origin:&Origin,
        url:&ParsedUrl,snapshot:HistorySnapshot,handling:PlatformNavigationHandling)
        ->Result<PreparedPlatformNavigation,HistoryError> {
        if *source!=self.active_platform_token(source.navigable,identity,origin)? {return Err(HistoryError::Stale);}
        self.validate_platform_new_owner(new_identity,url,&snapshot)?;
        if source.step==PlatformEntryStep::Pending && handling!=PlatformNavigationHandling::Replace {return Err(HistoryError::Order);}
        // Preserve the compatibility method's conservative reservation contract.
        self.next_id.checked_add(3).ok_or(HistoryError::Limit)?;
        let epoch=self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        let old=self.platform_current_entry(source.navigable).ok_or(HistoryError::Stale)?;
        let document=self.next_id;
        let push=old.step!=PlatformEntryStep::Pending && handling==PlatformNavigationHandling::Push;
        let id=document+if push {2}else{1};
        let entry=PlatformEntry {id,document,url:url.serialize(),snapshot,
            step:if push {PlatformEntryStep::Assigned(document+1)}else{old.step},
            scroll_restoration:ScrollRestoration::Auto,..old};
        let token=PlatformDocumentToken {session:self.instance_id,tab:self.tab,navigable:source.navigable,
            document,entry:id,identity:new_identity,
            step:if push {entry.step}else{source.step}};
        let mut prepared=PreparedPlatformNavigation {next_id:id+1,epoch,
            entries:self.entries.clone(),documents:self.documents.clone(),origins:self.platform_origins.clone(),
            parent_documents:self.platform_parent_documents.clone(),initial_documents:self.platform_initial_documents.clone(),
            children:self.platform_initial_children.clone(),steps:self.steps.clone(),current:self.current,
            orphaned:self.orphaned.clone(),result:(token,entry.clone())};
        prepared.documents.get_mut(&source.document).ok_or(HistoryError::Stale)?.status=DocumentStatus::Discarded;
        let actual_origin=new_origin.clone();let origin_text=actual_origin.serialize();
        prepared.documents.insert(document,DocumentState {id:document,owner:HistoryDocumentOwner::Platform(new_identity),
            origin:origin_text,status:DocumentStatus::Active,epoch,frozen_at_ms:None,freeze_deadline_ms:None,
            restore_pending:false,memory_bytes:0});
        prepared.origins.insert(document,actual_origin);
        if let Some(parent)=self.platform_parent_documents.get(&source.document).copied() {prepared.parent_documents.insert(document,parent);}
        if entry.step==PlatformEntryStep::Pending {
            let record=prepared.children.get_mut(&source.navigable).ok_or(HistoryError::Stale)?;
            record.entry=entry.clone();record.attachment_entry=entry;
        } else {
            let assigned=entry.assigned_entry().expect("prepared assigned entry");
            if push {
                let index=prepared.current.ok_or(HistoryError::Stale)?;
                let mut map=prepared.steps.get(index).ok_or(HistoryError::Stale)?.entries.clone();
                for child in self.parents.keys().copied() {
                    let mut cursor=child;
                    while let Some(Some(parent))=self.parents.get(&cursor) {
                        if *parent==source.navigable {map.remove(&child);break;}
                        cursor=*parent;
                    }
                }
                map.insert(source.navigable,id);prepared.steps.truncate(index+1);
                prepared.steps.push(HistoryStep {id:assigned.step,entries:map});
                prepared.current=Some(prepared.steps.len()-1);
            } else {
                // Replace every inherited exact source alias, not merely the
                // current joint step or other entries with the same Document.
                for joint in &mut prepared.steps {
                    if joint.entries.get(&source.navigable)==Some(&source.entry) {joint.entries.insert(source.navigable,id);}
                }
            }
            prepared.entries.insert(id,assigned);
            if let Some(record)=prepared.children.get_mut(&source.navigable) {record.entry=entry;}
            let referenced:BTreeSet<u64>=prepared.steps.iter().flat_map(|s|s.entries.values().copied()).collect();
            prepared.entries.retain(|id,_|referenced.contains(id));
            let documents:BTreeSet<u64>=prepared.entries.values().map(|e|e.document)
                .chain(prepared.children.values().filter(|c|!c.attached).flat_map(|c|[c.entry.document,c.attachment_entry.document])).collect();
            let orphaned:Vec<u64>=prepared.documents.keys().copied().filter(|id|!documents.contains(id)).collect();
            for id in orphaned {
                let state=prepared.documents.remove(&id).expect("prepared orphan");
                prepared.origins.remove(&id);prepared.parent_documents.remove(&id);prepared.initial_documents.remove(&id);
                if matches!(state.owner,HistoryDocumentOwner::Renderer(_)) {prepared.orphaned.push(HistoryDiscard {document:id,epoch:state.epoch});}
            }
        }
        Ok(prepared)
    }
    pub(super) fn publish_platform_navigation(&mut self,prepared:PreparedPlatformNavigation)->(PlatformDocumentToken,PlatformEntry) {
        let PreparedPlatformNavigation {next_id,epoch,entries,documents,origins,parent_documents,
            initial_documents,children,steps,current,orphaned,result}=prepared;
        // All owned results, pruning, arithmetic and exact aliases were staged
        // before the native projection. This tail cannot call author code or fail.
        self.next_id=next_id;self.epoch=epoch;self.entries=entries;self.documents=documents;
        self.platform_origins=origins;self.platform_parent_documents=parent_documents;
        self.platform_initial_documents=initial_documents;self.platform_initial_children=children;
        self.steps=steps;self.current=current;self.orphaned=orphaned;self.platform_traversal=None;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(node:u32)->PlatformDocumentIdentity {PlatformDocumentIdentity {runtime_id:1,document:node,node_generation:1}}
    fn url(value:&str)->ParsedUrl {ParsedUrl::parse(value).unwrap()}
    fn snapshot(value:u8)->HistorySnapshot {HistorySnapshot {history_state:Some(vec![value]),..Default::default()}}
    fn root()->(SessionHistory,PlatformDocumentToken,Origin) {
        let mut h=SessionHistory::new(1,1).unwrap();let origin=Origin::opaque();
        let (token,_)=h.attach_platform_document(1,identity(0),&url("https://example.test/a"),&origin,snapshot(1)).unwrap();
        (h,token,origin)
    }
    fn reserve(h:&mut SessionHistory,parent:&PlatformDocumentToken,parent_origin:&Origin,node:u32)
        ->(PlatformInitialChild,PlatformDocumentToken,PlatformEntry,Origin) {
        let origin=Origin::opaque();
        let (allocation,token,entry)=h.reserve_platform_initial_child_with_projection(parent,parent.identity,parent_origin,
            node as u64+1,identity(node),&origin,&url("about:blank"),snapshot(node as u8),||Ok::<_,&str>(())).unwrap();
        (allocation,token,entry,origin)
    }
    #[test]
    fn pending_readonly_reservation_and_projection_failures_preserve_joint_history() {
        let (mut h,parent,origin)=root();let steps=h.steps.clone();let count=h.entries.len();let counter=h.next_id;
        assert_eq!(h.reserve_platform_initial_child_with_projection(&parent,identity(0),&origin,2,identity(1),&Origin::opaque(),
            &url("about:blank"),snapshot(2),||Err("native realm rejected")),Err(PlatformCommitError::Projection("native realm rejected")));
        assert_eq!(h.next_id,counter);assert_eq!(h.parents.len(),1);assert_eq!(h.entries.len(),count);
        let (allocation,token,entry,child_origin)=reserve(&mut h,&parent,&origin,1);
        assert_eq!(entry.step,PlatformEntryStep::Pending);assert!(entry.assigned_entry().is_none());
        assert_eq!(h.entries.len(),count);assert_eq!(h.steps,steps);assert!(h.active_entry(2).is_none());
        for _ in 0..4 {assert_eq!(h.platform_current_entry(2),Some(entry.clone()));
            assert_eq!(h.platform_document_token(2,identity(1),&child_origin),Ok(token.clone()));}
        assert_eq!(h.steps,steps);
        let mut projected=false;
        assert!(h.commit_platform_same_document_with_projection(&token,identity(1),&child_origin,CommitKind::Push,
            &url("about:blank#one"),snapshot(3),||{projected=true;Ok::<_,&str>(())}).is_err());assert!(!projected);
        let (fresh,entry)=h.commit_platform_entry_with_projection(&token,identity(1),&child_origin,CommitKind::Push,
            &url("about:blank#one"),snapshot(3),||Ok::<_,&str>(())).unwrap();
        assert_eq!(entry.step,PlatformEntryStep::Pending);assert_eq!(h.steps,steps);
        let attached=h.attach_platform_initial_child(&allocation,&parent,identity(0),&origin).unwrap();
        assert_eq!(attached.snapshot,snapshot(3));assert_eq!(h.steps.len(),1);assert_ne!(attached.id,token.entry);
        assert!(h.commit_platform_entry_with_projection(&fresh,identity(1),&child_origin,CommitKind::Replace,
            &url("about:blank#two"),snapshot(4),||Ok::<_,&str>(())).is_err());
    }
    #[test]
    fn parent_push_and_pending_retarget_attach_first_parent_document_entry() {
        let (mut h,parent,origin)=root();let first=h.active_entry(1).unwrap().clone();
        let (parent,_)=h.commit_platform_same_document(&parent,identity(0),&origin,CommitKind::Push,
            &url("https://example.test/b"),snapshot(2)).unwrap();
        let (allocation,child,_,child_origin)=reserve(&mut h,&parent,&origin,1);
        let (parent,_)=h.commit_platform_same_document(&parent,identity(0),&origin,CommitKind::Push,
            &url("https://example.test/c"),snapshot(3)).unwrap();
        let fresh_origin=Origin::opaque();
        let (_,replacement)=h.commit_platform_navigation_with_handling_and_projection(&child,identity(1),&child_origin,
            identity(2),&fresh_origin,&url("https://child.test/remote"),snapshot(4),PlatformNavigationHandling::Replace,
            ||Ok::<_,&str>(())).unwrap();
        assert_eq!(replacement.step,PlatformEntryStep::Pending);assert_eq!(h.steps.len(),3);
        let entry=h.attach_platform_initial_child(&allocation,&parent,identity(0),&origin).unwrap();
        assert_eq!(entry.step,PlatformEntryStep::Assigned(first.step));assert_eq!(entry.document,replacement.document);
        assert!(h.steps.iter().all(|step|step.entries.get(&2)==Some(&entry.id)));
        assert_eq!(h.platform_document_token(2,identity(2),&fresh_origin).unwrap().document,entry.document);
        assert!(h.platform_document_token(2,identity(1),&child_origin).is_err());
    }
    #[test]
    fn assigned_source_alias_replace_preserves_root_step_after_child_push() {
        let (mut h,parent,origin)=root();let original=h.active_entry(1).unwrap().clone();
        let (allocation,child,_,child_origin)=reserve(&mut h,&parent,&origin,1);
        h.attach_platform_initial_child(&allocation,&parent,identity(0),&origin).unwrap();
        let child=h.platform_document_token(2,identity(1),&child_origin).unwrap();
        let remote_origin=Origin::opaque();
        let (child,_)=h.commit_platform_navigation_with_handling_and_projection(&child,identity(1),&child_origin,
            identity(2),&remote_origin,&url("https://child.test/a"),snapshot(3),PlatformNavigationHandling::Replace,
            ||Ok::<_,&str>(())).unwrap();
        let (_child,child_entry)=h.commit_platform_entry_with_projection(&child,identity(2),&remote_origin,CommitKind::Push,
            &url("https://child.test/b"),snapshot(4),||Ok::<_,&str>(())).unwrap();
        assert_eq!(h.steps.len(),2);let steps=h.steps.clone();
        let parent=h.platform_document_token(1,identity(0),&origin).unwrap();
        let (_,replacement)=h.commit_platform_same_document(&parent,identity(0),&origin,CommitKind::Replace,
            &url("https://example.test/replaced"),snapshot(5)).unwrap();
        assert_eq!(replacement.step,original.step);assert_ne!(replacement.id,original.id);
        assert!(h.steps.iter().all(|step|step.entries.get(&1)==Some(&replacement.id)));
        assert_eq!(h.steps[1].entries.get(&2),Some(&child_entry.id));assert_eq!(h.steps[0].id,steps[0].id);
        assert!(h.entry(original.id).is_none());
        let child=h.platform_document_token(2,identity(2),&remote_origin).unwrap();
        let (_,replace)=h.commit_platform_entry_with_projection(&child,identity(2),&remote_origin,CommitKind::Replace,
            &url("https://child.test/c"),snapshot(6),||Ok::<_,&str>(())).unwrap();
        assert_eq!(replace.step,child_entry.step);assert_eq!(h.steps[0].entries.get(&2),steps[0].entries.get(&2));
    }
    #[test]
    fn normal_push_before_attachment_preserves_initial_lineage_and_adds_one_joint_step() {
        let (mut h,parent,origin)=root();let first=h.current_step().unwrap();
        let (allocation,child,_,child_origin)=reserve(&mut h,&parent,&origin,1);
        let remote=Origin::opaque();
        let (child,initial)=h.commit_platform_navigation_with_handling_and_projection(&child,identity(1),&child_origin,
            identity(2),&remote,&url("https://child.test/a"),snapshot(2),PlatformNavigationHandling::Replace,
            ||Ok::<_,&str>(())).unwrap();
        let (_,pushed)=h.commit_platform_entry_with_projection(&child,identity(2),&remote,CommitKind::Push,
            &url("https://child.test/b"),snapshot(3),||Ok::<_,&str>(())).unwrap();
        assert_eq!(h.steps.len(),2);assert_ne!(pushed.step,PlatformEntryStep::Pending);
        let parent=h.platform_document_token(1,identity(0),&origin).unwrap();
        let attached=h.attach_platform_initial_child(&allocation,&parent,identity(0),&origin).unwrap();
        assert_eq!(attached.id,initial.id);assert_eq!(attached.step,PlatformEntryStep::Assigned(first));
        assert_eq!(h.steps[0].entries.get(&2),Some(&initial.id));
        assert_eq!(h.steps[1].entries.get(&2),Some(&pushed.id));
        assert_eq!(h.platform_current_entry(2),Some(pushed));
    }
    #[test]
    fn pending_replacement_errors_and_destroyed_lineage_never_publish_projection_or_aliases() {
        let (mut h,parent,origin)=root();let (allocation,child,entry,child_origin)=reserve(&mut h,&parent,&origin,1);
        let counter=h.next_id;let epoch=h.epoch;let steps=h.steps.clone();let docs=h.documents.clone();
        assert!(matches!(h.commit_platform_navigation_with_handling_and_projection(&child,identity(1),&child_origin,
            identity(2),&Origin::opaque(),&url("https://child.test/a"),snapshot(2),PlatformNavigationHandling::Replace,
            ||Err("realm entry failure")),Err(PlatformCommitError::Projection("realm entry failure"))));
        assert_eq!(h.platform_current_entry(2),Some(entry));assert_eq!(h.next_id,counter);assert_eq!(h.epoch,epoch);
        assert_eq!(h.steps,steps);assert_eq!(h.documents,docs);
        h.destroy_platform_initial_child(&allocation).unwrap();
        assert!(h.attach_platform_initial_child(&allocation,&parent,identity(0),&origin).is_err());
        assert!(h.commit_platform_entry_with_projection(&child,identity(1),&child_origin,CommitKind::Replace,
            &url("about:blank#stale"),snapshot(3),|| -> Result<(), &str> {panic!("destroyed before projection")}).is_err());
    }
    #[test]
    fn nested_pending_children_and_two_allocations_have_exact_lifetime_and_opaque_guards() {
        let (mut h,parent,origin)=root();
        let (one,child,_,child_origin)=reserve(&mut h,&parent,&origin,1);
        let (nested,nested_token,_,nested_origin)=reserve(&mut h,&child,&child_origin,2);
        let (two,_,_,_)=reserve(&mut h,&parent,&origin,3);
        assert!(h.attach_platform_initial_child(&nested,&child,identity(1),&child_origin).is_err());
        h.attach_platform_initial_child(&one,&parent,identity(0),&origin).unwrap();
        let child=h.platform_document_token(2,identity(1),&child_origin).unwrap();
        h.attach_platform_initial_child(&nested,&child,identity(1),&child_origin).unwrap();
        h.attach_platform_initial_child(&two,&parent,identity(0),&origin).unwrap();
        assert_eq!(h.steps.len(),1);assert_eq!(h.steps[0].entries.len(),4);
        assert!(h.platform_document_token(3,identity(2),&Origin::opaque()).is_err());
        assert!(h.platform_document_token(3,identity(2),&nested_origin).is_ok());
        h.destroy_platform_initial_child(&one).unwrap();
        assert!(h.platform_current_entry(2).is_none());assert!(h.platform_current_entry(3).is_none());
        assert!(h.retire_platform_document(&nested_token,identity(2),&nested_origin).is_err());
        assert!(h.destroy_platform_initial_child(&one).is_err());assert!(h.platform_current_entry(4).is_some());
        let parent=h.platform_document_token(1,identity(0),&origin).unwrap();
        let (fresh,_,_,_)=reserve(&mut h,&parent,&origin,1);
        assert_ne!(fresh,one);assert!(h.destroy_platform_initial_child(&one).is_err());
    }
    #[test]
    fn execution_parent_document_and_snapshot_counter_checks_are_atomic() {
        let (mut h,parent,origin)=root();let (allocation,child,_,child_origin)=reserve(&mut h,&parent,&origin,1);
        let new_origin=Origin::opaque();let (parent,parent_entry)=h.commit_platform_navigation_with_projection(&parent,identity(0),&origin,
            identity(4),&new_origin,&url("https://example.test/new-document"),snapshot(4),||Ok::<_,&str>(())).unwrap();
        assert!(h.platform_document_token(2,identity(1),&child_origin).is_err());
        let attached=h.attach_platform_initial_child(&allocation,&parent,identity(4),&new_origin).unwrap();
        assert_eq!(attached.step,PlatformEntryStep::Assigned(parent_entry.step));
        assert_eq!(h.steps[0].entries.get(&2),None);assert!(h.platform_document_token(2,identity(1),&child_origin).is_ok());
        let token=h.platform_document_token(2,identity(1),&child_origin).unwrap();let old=h.platform_current_entry(2);
        let steps=h.steps.clone();let counter=h.next_id;
        let bad=HistorySnapshot {history_state:Some(vec![0;MAX_STATE_BYTES+1]),..Default::default()};
        assert!(h.commit_platform_entry_with_projection(&token,identity(1),&child_origin,CommitKind::Push,
            &url("about:blank#bad"),bad,|| -> Result<(), &str> {panic!("validation must precede projection")}).is_err());
        assert_eq!(h.next_id,counter);assert_eq!(h.steps,steps);assert_eq!(h.platform_current_entry(2),old);
        h.next_id=u64::MAX;
        assert!(h.commit_platform_entry_with_projection(&token,identity(1),&child_origin,CommitKind::Replace,
            &url("about:blank#bad"),snapshot(5),|| -> Result<(), &str> {panic!("counter must precede projection")}).is_err());
        assert_eq!(h.platform_current_entry(2),old);assert_ne!(token,child);
    }
}
