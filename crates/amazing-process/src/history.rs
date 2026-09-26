//! Browser-owned traversable session history and bounded bfcache policy.
use crate::protocol::{
    HistoryAck, HistoryActivate, HistoryDiscard, HistoryFreeze, HistoryLifecycle,
    HistoryLifecycleEvent, HistoryReceiver, HistoryRestore,
};
use amazing_fetch::HttpUrl;
use amazing_ipc::{Connection, Error as IpcError};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_STATE_BYTES: usize = 1024 * 1024;
pub const MAX_SCROLL_AREAS: usize = 4096;
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_NAVIGABLES: usize = 256;
pub const MAX_FROZEN: usize = 8;
pub const MAX_FROZEN_BYTES: usize = 128 * 1024 * 1024;
pub const FROZEN_TTL_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    Invalid,
    Stale,
    Limit,
    NotEligible,
    NotFound,
    Order,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistorySnapshot {
    /// Bounded, versioned structured-clone graph encoded by the browser adapter.
    pub history_state: Option<Vec<u8>>,
    pub navigation_state: Vec<u8>,
    pub persisted_user_state: Vec<u8>,
    pub scroll: Vec<(u64, i32, i32)>,
}
impl HistorySnapshot {
    pub fn validate(&self) -> Result<(), HistoryError> {
        if self
            .history_state
            .as_ref()
            .is_some_and(|state| state.len() > MAX_STATE_BYTES)
            || self.navigation_state.len() > MAX_STATE_BYTES
            || self.persisted_user_state.len() > MAX_STATE_BYTES
            || self.scroll.len() > MAX_SCROLL_AREAS
            || self
                .scroll
                .iter()
                .map(|(id, _, _)| *id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.scroll.len()
        {
            return Err(HistoryError::Limit);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentStatus {
    Active,
    FreezePending,
    Frozen,
    Discarded,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentState {
    pub id: u64,
    pub renderer: u64,
    pub origin: String,
    pub status: DocumentStatus,
    pub epoch: u64,
    pub frozen_at_ms: Option<u64>,
    pub freeze_deadline_ms: Option<u64>,
    pub restore_pending: bool,
    pub memory_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHistoryEntry {
    pub id: u64,
    pub step: u64,
    pub navigable: u64,
    pub parent_navigable: Option<u64>,
    pub document: u64,
    pub url: String,
    pub snapshot: HistorySnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStep {
    pub id: u64,
    pub entries: BTreeMap<u64, u64>, // navigable -> entry ID
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    Navigate,
    Push,
    Replace,
    Reload,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationTicket {
    pub tab: u64,
    pub id: u64,
    pub epoch: u64,
    pub navigable: u64,
    pub source_document: Option<u64>,
    pub renderer: u64,
    pub kind: CommitKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traversal {
    pub step: u64,
    pub epoch: u64,
    pub entries: BTreeMap<u64, SessionHistoryEntry>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Eligibility {
    pub pending_navigation: bool,
    pub outstanding_ipc: bool,
    pub active_socket: bool,
    pub media_or_device: bool,
    pub storage_transaction: bool,
    pub unload_handler: bool,
    pub child_ineligible: bool,
    pub policy_mismatch: bool,
}
impl Eligibility {
    pub fn eligible(self) -> bool {
        self == Self::default()
    }
}
/// Only this browser-owned value mints entry, document, step and navigation IDs.
/// Renderer requests must carry a ticket or an existing document token.
pub struct SessionHistory {
    tab: u64,
    root: u64,
    next_id: u64,
    epoch: u64,
    parents: BTreeMap<u64, Option<u64>>,
    entries: BTreeMap<u64, SessionHistoryEntry>,
    documents: BTreeMap<u64, DocumentState>,
    steps: Vec<HistoryStep>,
    current: Option<usize>,
    pending: Option<NavigationTicket>,
    queued_traversal: Option<u64>,
    orphaned: Vec<HistoryDiscard>,
}
impl SessionHistory {
    pub fn new(tab: u64, root_navigable: u64) -> Result<Self, HistoryError> {
        if tab == 0 || root_navigable == 0 {
            return Err(HistoryError::Invalid);
        }
        Ok(Self {
            tab,
            root: root_navigable,
            next_id: 1,
            epoch: 0,
            parents: BTreeMap::from([(root_navigable, None)]),
            entries: BTreeMap::new(),
            documents: BTreeMap::new(),
            steps: Vec::new(),
            current: None,
            pending: None,
            queued_traversal: None,
            orphaned: Vec::new(),
        })
    }
    fn mint(&mut self) -> Result<u64, HistoryError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(HistoryError::Limit)?;
        Ok(id)
    }
    pub fn add_child(&mut self, parent: u64, child: u64) -> Result<(), HistoryError> {
        if child == 0
            || !self.parents.contains_key(&parent)
            || self.parents.contains_key(&child)
            || self.parents.len() >= MAX_NAVIGABLES
        {
            return Err(HistoryError::Invalid);
        }
        self.parents.insert(child, Some(parent));
        Ok(())
    }
    pub fn current_step(&self) -> Option<u64> {
        self.current.map(|i| self.steps[i].id)
    }
    pub fn steps(&self) -> &[HistoryStep] {
        &self.steps
    }
    pub fn document(&self, id: u64) -> Option<&DocumentState> {
        self.documents.get(&id)
    }
    pub fn entry(&self, id: u64) -> Option<&SessionHistoryEntry> {
        self.entries.get(&id)
    }
    pub fn active_entry(&self, navigable: u64) -> Option<&SessionHistoryEntry> {
        let id = self.steps.get(self.current?)?.entries.get(&navigable)?;
        self.entries.get(id)
    }
    pub fn begin(
        &mut self,
        renderer: u64,
        navigable: u64,
        source_document: Option<u64>,
        kind: CommitKind,
    ) -> Result<NavigationTicket, HistoryError> {
        if renderer == 0
            || !self.parents.contains_key(&navigable)
            || self.pending.is_some()
            || self.entries.len() >= MAX_ENTRIES
            || self.documents.len() >= MAX_ENTRIES
            || self.orphaned.len() >= MAX_ENTRIES
        {
            return Err(HistoryError::Invalid);
        }
        if self.parents[&navigable].is_some_and(|parent| self.active_entry(parent).is_none()) {
            return Err(HistoryError::Stale);
        }
        let current = self.active_entry(navigable);
        match (current, source_document) {
            (None, None) if kind == CommitKind::Navigate => {}
            (Some(entry), Some(source))
                if entry.document == source
                    && self.documents.get(&source).is_some_and(|d| {
                        d.renderer == renderer
                            && (d.status == DocumentStatus::Active
                                || (d.status == DocumentStatus::Discarded
                                    && matches!(kind, CommitKind::Navigate | CommitKind::Reload)))
                    }) => {}
            _ => return Err(HistoryError::Stale),
        }
        if matches!(
            kind,
            CommitKind::Push | CommitKind::Replace | CommitKind::Reload
        ) && current.is_none()
        {
            return Err(HistoryError::Invalid);
        }
        self.epoch = self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        let ticket = NavigationTicket {
            tab: self.tab,
            id: self.mint()?,
            epoch: self.epoch,
            navigable,
            source_document,
            renderer,
            kind,
        };
        self.pending = Some(ticket.clone());
        Ok(ticket)
    }
    pub fn cancel(&mut self, ticket: &NavigationTicket) -> Result<(), HistoryError> {
        if self.pending.as_ref() != Some(ticket) {
            return Err(HistoryError::Stale);
        }
        self.pending = None;
        Ok(())
    }
    pub fn commit(
        &mut self,
        ticket: NavigationTicket,
        url: String,
        snapshot: HistorySnapshot,
    ) -> Result<SessionHistoryEntry, HistoryError> {
        let destination_renderer = ticket.renderer;
        self.commit_to_renderer(ticket, destination_renderer, url, snapshot)
    }
    /// The browser chooses the destination process after site assignment. A
    /// renderer's source ticket never grants it control of a new process.
    pub fn commit_to_renderer(
        &mut self,
        ticket: NavigationTicket,
        destination_renderer: u64,
        url: String,
        snapshot: HistorySnapshot,
    ) -> Result<SessionHistoryEntry, HistoryError> {
        if self.pending.as_ref() != Some(&ticket) {
            return Err(HistoryError::Stale);
        }
        if destination_renderer == 0
            || (matches!(ticket.kind, CommitKind::Push | CommitKind::Replace)
                && destination_renderer != ticket.renderer)
        {
            return Err(HistoryError::Invalid);
        }
        snapshot.validate()?;
        let parsed = HttpUrl::parse(&url).map_err(|_| HistoryError::Invalid)?;
        if !matches!(parsed.as_parsed().scheme(), "http" | "https") || url.len() > 8192 {
            return Err(HistoryError::Invalid);
        }
        if matches!(ticket.kind, CommitKind::Push | CommitKind::Replace) {
            let old = self
                .active_entry(ticket.navigable)
                .ok_or(HistoryError::Stale)?;
            let old_url = HttpUrl::parse(&old.url).map_err(|_| HistoryError::Invalid)?;
            if old_url.origin() != parsed.origin() {
                return Err(HistoryError::Invalid);
            }
        }
        let document = if matches!(ticket.kind, CommitKind::Push | CommitKind::Replace) {
            ticket.source_document.ok_or(HistoryError::Stale)?
        } else {
            let id = self.mint()?;
            self.documents.insert(
                id,
                DocumentState {
                    id,
                    renderer: destination_renderer,
                    origin: parsed.origin().serialize(),
                    status: DocumentStatus::Active,
                    epoch: ticket.epoch,
                    frozen_at_ms: None,
                    freeze_deadline_ms: None,
                    restore_pending: false,
                    memory_bytes: 0,
                },
            );
            id
        };
        let step = if matches!(ticket.kind, CommitKind::Replace | CommitKind::Reload) {
            self.current_step().ok_or(HistoryError::Stale)?
        } else {
            self.mint()?
        };
        let entry = SessionHistoryEntry {
            id: self.mint()?,
            step,
            navigable: ticket.navigable,
            parent_navigable: self.parents[&ticket.navigable],
            document,
            url,
            snapshot,
        };
        if matches!(ticket.kind, CommitKind::Replace | CommitKind::Reload) {
            let index = self.current.ok_or(HistoryError::Stale)?;
            self.steps[index].entries.insert(ticket.navigable, entry.id);
        } else {
            let mut map = self
                .current
                .map(|i| self.steps[i].entries.clone())
                .unwrap_or_default();
            // A cross-document navigation discards child navigables of the replaced tree.
            if ticket.kind == CommitKind::Navigate {
                let descendants: Vec<u64> = self
                    .parents
                    .keys()
                    .copied()
                    .filter(|child| {
                        let mut cursor = *child;
                        while let Some(Some(parent)) = self.parents.get(&cursor) {
                            if *parent == ticket.navigable {
                                return true;
                            }
                            cursor = *parent;
                        }
                        false
                    })
                    .collect();
                for child in descendants {
                    map.remove(&child);
                }
            }
            map.insert(ticket.navigable, entry.id);
            if let Some(index) = self.current {
                self.steps.truncate(index + 1);
            }
            self.steps.push(HistoryStep {
                id: step,
                entries: map,
            });
            self.current = Some(self.steps.len() - 1);
        }
        self.entries.insert(entry.id, entry.clone());
        self.pending = None;
        self.prune_unreachable();
        Ok(entry)
    }
    fn prune_unreachable(&mut self) {
        let referenced: BTreeSet<u64> = self
            .steps
            .iter()
            .flat_map(|s| s.entries.values().copied())
            .collect();
        self.entries.retain(|id, _| referenced.contains(id));
        let documents: BTreeSet<u64> = self.entries.values().map(|e| e.document).collect();
        let orphaned: Vec<u64> = self
            .documents
            .keys()
            .copied()
            .filter(|id| !documents.contains(id))
            .collect();
        for id in orphaned {
            let state = self.documents.remove(&id).unwrap();
            self.orphaned.push(HistoryDiscard {
                document: id,
                epoch: state.epoch,
            });
        }
    }
    /// Browser sends these to their renderer after a replace or forward prune.
    pub fn take_orphaned_discards(&mut self) -> Vec<HistoryDiscard> {
        std::mem::take(&mut self.orphaned)
    }
    pub fn save_snapshot(
        &mut self,
        renderer: u64,
        document: u64,
        snapshot: HistorySnapshot,
    ) -> Result<(), HistoryError> {
        snapshot.validate()?;
        let state = self
            .documents
            .get(&document)
            .ok_or(HistoryError::NotFound)?;
        if state.renderer != renderer || state.status != DocumentStatus::Active {
            return Err(HistoryError::Stale);
        }
        let entry = self
            .steps
            .get(self.current.ok_or(HistoryError::Stale)?)
            .and_then(|s| {
                s.entries.values().find_map(|id| {
                    self.entries
                        .get(id)
                        .filter(|e| e.document == document)
                        .map(|e| e.id)
                })
            })
            .ok_or(HistoryError::Stale)?;
        self.entries.get_mut(&entry).unwrap().snapshot = snapshot;
        Ok(())
    }
    pub fn traverse(&mut self, step: u64) -> Result<Traversal, HistoryError> {
        if self.pending.is_some() || self.queued_traversal.is_some() {
            return Err(HistoryError::Order);
        }
        let index = self
            .steps
            .iter()
            .position(|s| s.id == step)
            .ok_or(HistoryError::NotFound)?;
        if let Some(current) = self.current {
            for (navigable, id) in &self.steps[current].entries {
                let current_document = self.entries[id].document;
                let target_document = self.steps[index]
                    .entries
                    .get(navigable)
                    .and_then(|target| self.entries.get(target))
                    .map(|entry| entry.document);
                if Some(current_document) != target_document
                    && self
                        .documents
                        .get(&current_document)
                        .is_some_and(|doc| doc.status == DocumentStatus::Active)
                {
                    return Err(HistoryError::Order);
                }
            }
        }
        self.epoch = self.epoch.checked_add(1).ok_or(HistoryError::Limit)?;
        self.current = Some(index);
        Ok(Traversal {
            step,
            epoch: self.epoch,
            entries: self.steps[index]
                .entries
                .iter()
                .filter_map(|(nav, id)| self.entries.get(id).cloned().map(|e| (*nav, e)))
                .collect(),
        })
    }
    /// Queue a traversal so a synchronous same-document navigation can commit
    /// before the target step is applied, as required by HTML's traversal queue.
    pub fn queue_traversal(&mut self, step: u64) -> Result<(), HistoryError> {
        if self.pending.is_some() || self.queued_traversal.is_some() {
            return Err(HistoryError::Order);
        }
        if !self.steps.iter().any(|item| item.id == step) {
            return Err(HistoryError::NotFound);
        }
        self.queued_traversal = Some(step);
        Ok(())
    }
    pub fn apply_queued_traversal(&mut self) -> Result<Traversal, HistoryError> {
        let step = self.queued_traversal.take().ok_or(HistoryError::Order)?;
        match self.traverse(step) {
            Ok(result) => Ok(result),
            Err(error) => {
                self.queued_traversal = Some(step);
                Err(error)
            }
        }
    }
    pub fn freeze_request(
        &mut self,
        document: u64,
        renderer: u64,
        epoch: u64,
        eligibility: Eligibility,
        memory_bytes: usize,
        now_ms: u64,
    ) -> Result<HistoryFreeze, HistoryError> {
        let pending_allows_document = self.pending.as_ref().is_none_or(|pending| {
            if !matches!(pending.kind, CommitKind::Navigate | CommitKind::Reload) {
                return false;
            }
            if pending.source_document == Some(document) {
                return true;
            }
            let Some(navigable) = self
                .current
                .and_then(|current| self.steps.get(current))
                .and_then(|step| {
                    step.entries.iter().find_map(|(nav, id)| {
                        self.entries
                            .get(id)
                            .is_some_and(|entry| entry.document == document)
                            .then_some(*nav)
                    })
                })
            else {
                return false;
            };
            let mut cursor = navigable;
            while let Some(Some(parent)) = self.parents.get(&cursor) {
                if *parent == pending.navigable {
                    return true;
                }
                cursor = *parent;
            }
            false
        });
        if !eligibility.eligible()
            || memory_bytes == 0
            || memory_bytes > MAX_FROZEN_BYTES
            || !pending_allows_document
        {
            return Err(HistoryError::NotEligible);
        }
        let state = self
            .documents
            .get_mut(&document)
            .ok_or(HistoryError::NotFound)?;
        if state.renderer != renderer
            || state.epoch != epoch
            || state.status != DocumentStatus::Active
        {
            return Err(HistoryError::Stale);
        }
        state.status = DocumentStatus::FreezePending;
        state.restore_pending = false;
        state.memory_bytes = memory_bytes;
        state.freeze_deadline_ms = Some(now_ms.saturating_add(5000));
        Ok(HistoryFreeze {
            document,
            epoch,
            deadline_ms: state.freeze_deadline_ms.unwrap(),
        })
    }
    pub fn freeze_ack(&mut self, ack: HistoryAck, now_ms: u64) -> Result<Vec<u64>, HistoryError> {
        let state = self
            .documents
            .get_mut(&ack.document)
            .ok_or(HistoryError::NotFound)?;
        if state.epoch != ack.epoch || state.status != DocumentStatus::FreezePending {
            return Err(HistoryError::Stale);
        }
        if state
            .freeze_deadline_ms
            .is_some_and(|deadline| now_ms > deadline)
        {
            state.status = DocumentStatus::Discarded;
            state.freeze_deadline_ms = None;
            state.restore_pending = false;
            state.memory_bytes = 0;
            return Err(HistoryError::Stale);
        }
        if !ack.quiescent
            || ack.events
                != [
                    HistoryLifecycleEvent::PageHidePersisted(()),
                    HistoryLifecycleEvent::VisibilityHidden(()),
                ]
        {
            return Err(HistoryError::Order);
        }
        state.status = DocumentStatus::Frozen;
        state.frozen_at_ms = Some(now_ms);
        state.freeze_deadline_ms = None;
        Ok(self.evict(now_ms))
    }
    pub fn restore_request(
        &mut self,
        document: u64,
        renderer: u64,
        now_ms: u64,
    ) -> Result<HistoryRestore, HistoryError> {
        self.evict(now_ms);
        let state = self
            .documents
            .get(&document)
            .ok_or(HistoryError::NotFound)?;
        if state.renderer != renderer || state.status != DocumentStatus::Frozen {
            return Err(HistoryError::Stale);
        }
        if state.restore_pending {
            return Err(HistoryError::Order);
        }
        let entry = self
            .steps
            .get(self.current.ok_or(HistoryError::Stale)?)
            .and_then(|s| {
                s.entries
                    .values()
                    .find_map(|id| self.entries.get(id).filter(|e| e.document == document))
            });
        let Some(entry) = entry else {
            return Err(HistoryError::Stale);
        };
        let url = HttpUrl::parse(&entry.url).map_err(|_| HistoryError::Invalid)?;
        if url.origin().serialize() != state.origin {
            return Err(HistoryError::Stale);
        }
        let epoch = state.epoch;
        self.documents.get_mut(&document).unwrap().restore_pending = true;
        Ok(HistoryRestore { document, epoch })
    }
    pub fn restore_ack(&mut self, ack: HistoryAck) -> Result<(), HistoryError> {
        let state = self
            .documents
            .get_mut(&ack.document)
            .ok_or(HistoryError::NotFound)?;
        if state.epoch != ack.epoch
            || state.status != DocumentStatus::Frozen
            || !state.restore_pending
        {
            return Err(HistoryError::Stale);
        }
        if !ack.quiescent
            || ack.events
                != [
                    HistoryLifecycleEvent::VisibilityVisible(()),
                    HistoryLifecycleEvent::PageShowPersisted(()),
                ]
        {
            return Err(HistoryError::Order);
        }
        state.status = DocumentStatus::Active;
        state.frozen_at_ms = None;
        state.freeze_deadline_ms = None;
        state.restore_pending = false;
        state.memory_bytes = 0;
        Ok(())
    }
    pub fn discard(&mut self, document: u64) -> Result<HistoryDiscard, HistoryError> {
        let state = self
            .documents
            .get_mut(&document)
            .ok_or(HistoryError::NotFound)?;
        state.status = DocumentStatus::Discarded;
        state.frozen_at_ms = None;
        state.freeze_deadline_ms = None;
        state.restore_pending = false;
        state.memory_bytes = 0;
        Ok(HistoryDiscard {
            document,
            epoch: state.epoch,
        })
    }
    pub fn evict(&mut self, now_ms: u64) -> Vec<u64> {
        let mut frozen: Vec<(u64, u64, usize)> = self
            .documents
            .values()
            .filter_map(|d| {
                (d.status == DocumentStatus::Frozen).then_some((
                    d.id,
                    d.frozen_at_ms.unwrap_or(0),
                    d.memory_bytes,
                ))
            })
            .collect();
        frozen.sort_by_key(|(id, at, _)| (*at, *id));
        let mut bytes: usize = frozen.iter().map(|(_, _, size)| size).sum();
        let mut count = frozen.len();
        let mut evicted = Vec::new();
        for (id, at, size) in frozen {
            if now_ms.saturating_sub(at) >= FROZEN_TTL_MS
                || count > MAX_FROZEN
                || bytes > MAX_FROZEN_BYTES
            {
                let state = self.documents.get_mut(&id).unwrap();
                state.status = DocumentStatus::Discarded;
                state.frozen_at_ms = None;
                state.freeze_deadline_ms = None;
                state.restore_pending = false;
                state.memory_bytes = 0;
                evicted.push(id);
                bytes -= size;
                count -= 1;
            }
        }
        evicted
    }
    pub fn renderer_crashed(&mut self, renderer: u64) -> Vec<u64> {
        let mut dropped = Vec::new();
        for state in self
            .documents
            .values_mut()
            .filter(|d| d.renderer == renderer)
        {
            state.status = DocumentStatus::Discarded;
            state.frozen_at_ms = None;
            state.freeze_deadline_ms = None;
            state.restore_pending = false;
            state.memory_bytes = 0;
            dropped.push(state.id);
        }
        dropped
    }
    pub fn root_navigable(&self) -> u64 {
        self.root
    }
    pub fn parent_navigable(&self, navigable: u64) -> Option<u64> {
        self.parents.get(&navigable).copied().flatten()
    }
}

/// Demo renderer lifecycle endpoint. It enforces document/epoch identity and
/// acknowledges only after recording the observable event sequence.
#[derive(Default, Clone)]
pub struct HistoryRenderer {
    state: Arc<Mutex<BTreeMap<u64, (u64, DocumentStatus, Option<Vec<u8>>)>>>,
    activated_once: Arc<AtomicBool>,
}
impl HistoryRenderer {
    pub fn is_quiescent(&self) -> bool {
        let state = self.state.lock().unwrap();
        self.activated_once.load(Ordering::Acquire)
            && state
                .values()
                .all(|(_, status, _)| *status != DocumentStatus::Active)
    }
}
impl HistoryLifecycle for HistoryRenderer {
    fn activate(&self, request: HistoryActivate) -> Result<bool, IpcError> {
        if request.document == 0 || request.epoch == 0 {
            return Err(IpcError::Remote("token".into()));
        }
        if request
            .history_state
            .as_ref()
            .is_some_and(|state| state.0.len() > MAX_STATE_BYTES)
        {
            return Err(IpcError::Remote("history.state limit".into()));
        }
        let mut state = self.state.lock().unwrap();
        if state.contains_key(&request.document) {
            return Err(IpcError::Remote("duplicate token".into()));
        }
        state.insert(
            request.document,
            (
                request.epoch,
                DocumentStatus::Active,
                request.history_state.map(|state| state.0),
            ),
        );
        self.activated_once.store(true, Ordering::Release);
        Ok(true)
    }
    fn freeze(&self, request: HistoryFreeze) -> Result<HistoryAck, IpcError> {
        if request.deadline_ms == 0 {
            return Err(IpcError::Remote("deadline".into()));
        }
        let mut state = self.state.lock().unwrap();
        match state.get_mut(&request.document) {
            Some((epoch, status, _))
                if *epoch == request.epoch && *status == DocumentStatus::Active =>
            {
                *status = DocumentStatus::Frozen;
                Ok(HistoryAck {
                    document: request.document,
                    epoch: request.epoch,
                    quiescent: true,
                    events: vec![
                        HistoryLifecycleEvent::PageHidePersisted(()),
                        HistoryLifecycleEvent::VisibilityHidden(()),
                    ],
                })
            }
            _ => Err(IpcError::Remote("stale freeze".into())),
        }
    }
    fn restore(&self, request: HistoryRestore) -> Result<HistoryAck, IpcError> {
        let mut state = self.state.lock().unwrap();
        match state.get_mut(&request.document) {
            Some((epoch, status, _))
                if *epoch == request.epoch && *status == DocumentStatus::Frozen =>
            {
                *status = DocumentStatus::Active;
                Ok(HistoryAck {
                    document: request.document,
                    epoch: request.epoch,
                    quiescent: true,
                    events: vec![
                        HistoryLifecycleEvent::VisibilityVisible(()),
                        HistoryLifecycleEvent::PageShowPersisted(()),
                    ],
                })
            }
            _ => Err(IpcError::Remote("stale restore".into())),
        }
    }
    fn discard(&self, request: HistoryDiscard) -> Result<bool, IpcError> {
        let mut state = self.state.lock().unwrap();
        match state.get(&request.document) {
            Some((epoch, _, _)) if *epoch == request.epoch => {
                Ok(state.remove(&request.document).is_some())
            }
            _ => Err(IpcError::Remote("stale discard".into())),
        }
    }
}
pub fn serve_history_renderer(connection: &Arc<Connection>, endpoint: HistoryRenderer) {
    connection.bind(14, HistoryReceiver(endpoint));
}

#[cfg(test)]
mod opaque_state_tests {
    use super::*;
    use crate::protocol::ByteBuf;

    #[test]
    fn renderer_retains_opaque_history_state_until_discard() {
        let renderer = HistoryRenderer::default();
        renderer
            .activate(HistoryActivate {
                document: 10,
                epoch: 3,
                history_state: Some(ByteBuf(vec![0, 255, 1, 0])),
            })
            .unwrap();
        assert_eq!(
            renderer.state.lock().unwrap()[&10].2,
            Some(vec![0, 255, 1, 0])
        );
        renderer
            .discard(HistoryDiscard {
                document: 10,
                epoch: 3,
            })
            .unwrap();
        assert!(!renderer.state.lock().unwrap().contains_key(&10));
    }
}
