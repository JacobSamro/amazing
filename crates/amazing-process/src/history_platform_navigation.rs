//! Browser-only request admission on an authentic child allocation.
//! Attempt-bearing canonical publication is additive; task admission and native
//! activity remain embedding obligations.
use super::*;

/// Store-minted request authority. Not Clone: the browser retains the authentic
/// request rather than manufacturing another owner from an entry or URL.
/// Dropping this value does not cancel its request; cancellation is an exact CAS.
#[derive(Debug, PartialEq, Eq)]
pub struct PlatformNavigationAttempt {
    key: NavigationAdmissionKey,
}
impl PlatformNavigationAttempt {
    pub fn navigable_id(&self) -> u64 { self.key.allocation.navigable_id() }
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationAdmissionKey {
    allocation: PlatformInitialChild,
    sequence: u64,
    document: u64,
    owner: PlatformDocumentIdentity,
    origin: Origin,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::history) struct NavigationAdmissionState {
    sequence: u64,
    current: Option<NavigationAdmissionKey>,
}

impl SessionHistory {
    /// Admit an already accepted browser request for this exact allocation and
    /// current target Document. The embedding performs all conversions and
    /// permission checks first and supplies genuine native target eligibility.
    /// The request initiator and its retained policy snapshot are separate.
    ///
    /// New acceptance supersedes only this allocation's prior request. Sequence
    /// exhaustion or any validation error leaves the old request unchanged.
    /// This does not change History epoch, entries, initial attachment or active
    /// Documents. Traversal exclusion/navigation API events remain host work.
    pub fn admit_platform_child_navigation(&mut self, allocation: &PlatformInitialChild,
        target: &PlatformDocumentToken, owner: PlatformDocumentIdentity, origin: &Origin)
        -> Result<PlatformNavigationAttempt, HistoryError>
    {
        let record = self.platform_initial_children.get(&allocation.navigable_id())
            .ok_or(HistoryError::Stale)?;
        if record.allocation != *allocation || target.navigable != allocation.navigable_id()
            || *target != self.active_platform_token(target.navigable, owner, origin)? {
            return Err(HistoryError::Stale);
        }
        let sequence = record.navigation.sequence.checked_add(1).ok_or(HistoryError::Limit)?;
        let key = NavigationAdmissionKey { allocation: allocation.clone(), sequence,
            document: target.document, owner, origin: origin.clone() };
        // Prepare both owned values before superseding the previous request.
        let attempt = PlatformNavigationAttempt { key: key.clone() };
        let state = &mut self.platform_initial_children.get_mut(&target.navigable)
            .expect("validated allocation").navigation;
        state.sequence = sequence;
        state.current = Some(key);
        Ok(attempt)
    }

    /// Revalidate exact request authority and return a freshly derived CURRENT
    /// entry token. Same-document entry/step changes do not by themselves abort
    /// the request; a different active DocumentState or owner/origin does.
    /// This is not a native fully-active certificate or commit reservation.
    pub fn current_platform_child_navigation(&self, attempt: &PlatformNavigationAttempt,
        owner: PlatformDocumentIdentity, origin: &Origin)
        -> Result<PlatformDocumentToken, HistoryError>
    {
        let key = &attempt.key;
        let nav = key.allocation.navigable_id();
        let record = self.platform_initial_children.get(&nav).ok_or(HistoryError::Stale)?;
        if record.allocation != key.allocation || record.navigation.current.as_ref() != Some(key)
            || key.owner != owner || key.origin != *origin {
            return Err(HistoryError::Stale);
        }
        let token = self.active_platform_token(nav, owner, origin)?;
        if token.session != self.instance_id || token.document != key.document {
            return Err(HistoryError::Stale);
        }
        Ok(token)
    }

    /// Commit one authentic accepted child request. The source token is derived
    /// here, under this same exclusive canonical operation, so same-document
    /// entry changes do not invalidate an otherwise current navigation attempt.
    /// Native projection must be callback-free and atomically rollback on Err.
    /// Error preserves current admission and every canonical field; success
    /// publishes the prevalidated entry and consumes only this exact admission.
    pub fn commit_platform_child_navigation_attempt_with_projection<E>(&mut self,
        attempt:&PlatformNavigationAttempt,owner:PlatformDocumentIdentity,origin:&Origin,
        new_owner:PlatformDocumentIdentity,new_origin:&Origin,url:&ParsedUrl,
        snapshot:HistorySnapshot,handling:PlatformNavigationHandling,
        projection:impl FnOnce()->Result<(),E>)
        ->Result<(PlatformDocumentToken,PlatformEntry),PlatformCommitError<E>> {
        let source=self.current_platform_child_navigation(attempt,owner,origin)
            .map_err(PlatformCommitError::History)?;
        let mut prepared=self.prepare_platform_navigation(&source,owner,origin,new_owner,new_origin,
            url,snapshot,handling).map_err(PlatformCommitError::History)?;
        let record=prepared.children.get_mut(&source.navigable).expect("validated authentic allocation");
        // Prepare the exact consumption before projection. Never call cancel
        // after publication: that method correctly rejects the retired owner.
        debug_assert_eq!(record.navigation.current.as_ref(),Some(&attempt.key));
        record.navigation.current=None;
        projection().map_err(PlatformCommitError::Projection)?;
        Ok(self.publish_platform_navigation(prepared))
    }

    /// Cancel only this currently admitted authentic request. A stale request,
    /// retired owner or foreign allocation cannot clear a newer/sibling request.
    /// Sequence is never rolled back or reused. No native cancellation/events
    /// are dispatched and no History epoch or entry is changed.
    pub fn cancel_platform_child_navigation(&mut self, attempt: &PlatformNavigationAttempt,
        owner: PlatformDocumentIdentity, origin: &Origin) -> Result<(), HistoryError>
    {
        let token = self.current_platform_child_navigation(attempt, owner, origin)?;
        self.platform_initial_children.get_mut(&token.navigable)
            .expect("validated current request").navigation.current = None;
        Ok(())
    }
}

#[cfg(test)]
#[path = "history_platform_navigation_publication_tests.rs"]
mod publication_tests;

#[cfg(test)]
#[path = "history_platform_navigation_tests.rs"]
mod tests;
