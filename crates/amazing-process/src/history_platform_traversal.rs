//! Browser-only atomic multi-owner same-document traversal publication.
//! Planning never activates an owner, admits a child, or assigns a pending step.
use super::*;

/// Authentic process-local plan: only this store can construct its authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSameDocumentPlan {
    session: u64,
    tab: u64,
    epoch: u64,
    source: PlatformDocumentToken,
    current: HistoryStep,
    target: HistoryStep,
    entries: BTreeMap<u64, SessionHistoryEntry>,
    changes: Vec<PlatformSameDocumentChange>,
    required: Vec<PlatformOwnerEligibility>,
}
impl PlatformSameDocumentPlan {
    pub fn target_step(&self) -> u64 { self.target.id }
    /// Each changed canonical entry, including equal URLs with different state.
    pub fn changes(&self) -> &[PlatformSameDocumentChange] { &self.changes }
    /// Required changed owners, source, and unchanged ancestors. The embedding
    /// must replace fully_active with its actual native eligibility result.
    pub fn required_owners(&self) -> &[PlatformOwnerEligibility] { &self.required }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSameDocumentChange {
    pub owner: PlatformDocumentIdentity,
    pub source_token: PlatformDocumentToken,
    pub from: SessionHistoryEntry,
    pub to: SessionHistoryEntry,
    pub security_origin: Origin,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformOwnerEligibility {
    pub token: PlatformDocumentToken,
    pub owner: PlatformDocumentIdentity,
    pub origin: Origin,
    /// Supplied only after native Doc/generation/realm/fully-active checks.
    /// Canonical owner and ancestor validation cannot replace that host check.
    pub fully_active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSnapshotUpdate {
    pub token: PlatformDocumentToken,
    pub snapshot: HistorySnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformTraversalEffect {
    /// Authentic target is already current: no projection, counter or store write.
    Unchanged,
    /// Different assigned step with identical entries: select only the joint step.
    StepOnly,
    Projected,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformJointTraversal {
    pub effect: PlatformTraversalEffect,
    pub traversal: Traversal,
    /// Fresh capabilities for every required participating/ancestor owner.
    pub tokens: BTreeMap<u64, PlatformDocumentToken>,
    pub changes: Vec<PlatformSameDocumentChange>,
}

impl SessionHistory {
    /// Pure derivation of a bounded same-document target. A serialized browser
    /// traversal authority computes the target at execution, not at getter time.
    pub fn plan_platform_same_document_traversal(&self, source: &PlatformDocumentToken,
        owner: PlatformDocumentIdentity, origin: &Origin, target_step: u64)
        -> Result<PlatformSameDocumentPlan, HistoryError>
    {
        if self.pending.is_some() || self.queued_traversal.is_some()
            || self.platform_traversal.is_some() { return Err(HistoryError::Order); }
        if source.session != self.instance_id || source.tab != self.tab
            || *source != self.active_platform_token(source.navigable, owner, origin)? {
            return Err(HistoryError::Stale);
        }
        if source.step == PlatformEntryStep::Pending { return Err(HistoryError::Order); }
        let current = self.steps.get(self.current.ok_or(HistoryError::Stale)?)
            .ok_or(HistoryError::Stale)?;
        let target = self.steps.iter().find(|step| step.id == target_step)
            .ok_or(HistoryError::NotFound)?;
        if current.entries.keys().ne(target.entries.keys()) { return Err(HistoryError::Order); }
        // An unattached initial owner has independent active/pending state. It
        // must be handled by creation attachment before this bounded operation.
        if self.platform_initial_children.values().any(|record| !record.attached) {
            return Err(HistoryError::Order);
        }
        let mut entries = BTreeMap::new();
        let mut changes = Vec::new();
        let mut required = BTreeSet::from([source.navigable]);
        for (nav, id) in &current.entries {
            let from = self.entries.get(id).ok_or(HistoryError::Stale)?;
            let to = self.entries.get(&target.entries[nav]).ok_or(HistoryError::Stale)?;
            if from.navigable != *nav || to.navigable != *nav
                || from.document != to.document
                || from.parent_navigable != to.parent_navigable {
                return Err(HistoryError::Order);
            }
            let document = self.documents.get(&from.document).ok_or(HistoryError::Stale)?;
            let HistoryDocumentOwner::Platform(identity) = document.owner else {
                return Err(HistoryError::Order);
            };
            let actual_origin = self.platform_origins.get(&from.document)
                .ok_or(HistoryError::Stale)?;
            let token = self.active_platform_token(*nav, identity, actual_origin)?;
            if token.step == PlatformEntryStep::Pending { return Err(HistoryError::Order); }
            if from.url.len() > 8192 || to.url.len() > 8192
                || ParsedUrl::parse(&from.url).is_err() || ParsedUrl::parse(&to.url).is_err() {
                return Err(HistoryError::Invalid);
            }
            from.snapshot.validate()?;
            to.snapshot.validate()?;
            entries.insert(from.id, from.clone());
            entries.insert(to.id, to.clone());
            if from.id != to.id {
                required.insert(*nav);
                changes.push(PlatformSameDocumentChange { owner: identity, source_token: token,
                    from: from.clone(), to: to.clone(), security_origin: actual_origin.clone() });
            }
        }
        // Native eligibility is needed even for an unchanged ancestor of a
        // changing child. Each canonical parent chain is independently checked.
        let mut cursor: Vec<_> = required.iter().copied().collect();
        while let Some(nav) = cursor.pop() {
            if let Some(Some(parent)) = self.parents.get(&nav) {
                if !current.entries.contains_key(parent) { return Err(HistoryError::Stale); }
                if required.insert(*parent) { cursor.push(*parent); }
            }
        }
        let mut owners = Vec::new();
        for nav in required {
            let entry = self.entries.get(&current.entries[&nav]).ok_or(HistoryError::Stale)?;
            let document = self.documents.get(&entry.document).ok_or(HistoryError::Stale)?;
            let HistoryDocumentOwner::Platform(identity) = document.owner else {
                return Err(HistoryError::Order);
            };
            let actual_origin = self.platform_origins.get(&entry.document)
                .ok_or(HistoryError::Stale)?;
            owners.push(PlatformOwnerEligibility {
                token: self.active_platform_token(nav, identity, actual_origin)?,
                owner: identity, origin: actual_origin.clone(), fully_active: false,
            });
        }
        Ok(PlatformSameDocumentPlan { session: self.instance_id, tab: self.tab,
            epoch: self.epoch, source: source.clone(), current: current.clone(),
            target: target.clone(), entries, changes, required: owners })
    }

    /// Publish one native transaction and then infallible canonical updates.
    ///
    /// The callback MUST do no author callbacks/conversions/network/event
    /// dispatch. All fallible realm/state preparation precedes this call. On
    /// Err it MUST leave *every* native owner unchanged, either by failing before
    /// writes or rolling back all earlier writes without author code. Calling a
    /// sequence of fallible single-owner URL setters is not that contract.
    /// This API guarantees canonical rollback; the trusted embedding owns and
    /// must prove the corresponding multi-owner native rollback.
    pub fn apply_platform_same_document_plan_with_projection<E>(&mut self,
        plan: &PlatformSameDocumentPlan, eligible: &[PlatformOwnerEligibility],
        snapshots: &[PlatformSnapshotUpdate],
        projection: impl FnOnce(&[PlatformSameDocumentChange]) -> Result<(), E>)
        -> Result<PlatformJointTraversal, PlatformCommitError<E>>
    {
        let validate = (|| {
            if plan.session != self.instance_id || plan.tab != self.tab || plan.epoch != self.epoch {
                return Err(HistoryError::Stale);
            }
            let source_owner = plan.required.iter().find(|owner| owner.token == plan.source)
                .ok_or(HistoryError::Stale)?;
            let actual = self.plan_platform_same_document_traversal(&plan.source,
                source_owner.owner, &source_owner.origin, plan.target.id)?;
            if actual != *plan { return Err(HistoryError::Stale); }
            if eligible.len() != plan.required.len() { return Err(HistoryError::NotEligible); }
            let mut seen = BTreeSet::new();
            for supplied in eligible {
                if !seen.insert(supplied.token.navigable) { return Err(HistoryError::Invalid); }
                let expected = plan.required.iter().find(|owner|
                    owner.token.navigable == supplied.token.navigable).ok_or(HistoryError::NotEligible)?;
                if !supplied.fully_active { return Err(HistoryError::NotEligible); }
                if supplied.token != expected.token || supplied.owner != expected.owner
                    || supplied.origin != expected.origin { return Err(HistoryError::Stale); }
            }
            let mut updates = BTreeMap::new();
            for update in snapshots {
                let change = plan.changes.iter().find(|change| change.source_token == update.token)
                    .ok_or(HistoryError::Stale)?;
                update.snapshot.validate()?;
                if let Some(previous) = updates.insert(change.from.id, update.snapshot.clone()) {
                    if previous != update.snapshot { return Err(HistoryError::Invalid); }
                }
            }
            if updates.len() != plan.changes.len() { return Err(HistoryError::NotEligible); }
            let next_epoch = if plan.current.id == plan.target.id { self.epoch }
                else { self.epoch.checked_add(1).ok_or(HistoryError::Limit)? };
            let index = self.steps.iter().position(|step| step.id == plan.target.id)
                .ok_or(HistoryError::Stale)?;
            Ok((updates, next_epoch, index))
        })();
        let (updates, epoch, index) = validate.map_err(PlatformCommitError::History)?;
        // Prepare owned receipts before native publication. No fallible store
        // operation, parser, snapshot validation or counter mint follows it.
        let selected: BTreeMap<_, _> = plan.target.entries.iter().map(|(nav, id)|
            (*nav, plan.entries[id].clone())).collect();
        let mut tokens = BTreeMap::new();
        for required in &plan.required {
            let mut token = required.token.clone();
            token.entry = plan.target.entries[&token.navigable];
            token.step = PlatformEntryStep::Assigned(plan.target.id);
            tokens.insert(token.navigable, token);
        }
        let effect = if plan.current.id == plan.target.id { PlatformTraversalEffect::Unchanged }
            else if plan.changes.is_empty() { PlatformTraversalEffect::StepOnly }
            else { PlatformTraversalEffect::Projected };
        let result = PlatformJointTraversal { effect, traversal: Traversal { step: plan.target.id,
            epoch, entries: selected }, tokens, changes: plan.changes.clone() };
        if plan.current.id == plan.target.id { return Ok(result); }
        if !plan.changes.is_empty() {
            projection(&plan.changes).map_err(PlatformCommitError::Projection)?;
        }
        for (id, snapshot) in updates {
            self.entries.get_mut(&id).expect("prevalidated outgoing entry").snapshot = snapshot;
        }
        self.current = Some(index);
        self.epoch = epoch;
        self.platform_traversal = None;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "history_platform_traversal_tests.rs"]
mod tests;
