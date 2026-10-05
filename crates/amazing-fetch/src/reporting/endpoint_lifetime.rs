//! Endpoint objects belong to one original native global. Same-global prepared
//! settings retain removal state, never a new global's endpoint authority.
use super::{Document, HttpUrl, ReportError};
use std::{collections::BTreeMap, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};

pub(super) struct EndpointLifetime {
    url: HttpUrl,
    removed: AtomicBool,
    failures: AtomicU64,
}
impl EndpointLifetime {
    pub(super) fn new(url: HttpUrl) -> Self {
        Self { url, removed: AtomicBool::new(false), failures: AtomicU64::new(0) }
    }
    pub(super) fn is_active(&self) -> bool { !self.removed.load(Ordering::Acquire) }
    pub(super) fn revoke(&self) { self.removed.store(true, Ordering::Release); }
    pub(super) fn record_failure(&self) {
        let _ = self.failures.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |value| Some(value.saturating_add(1)));
    }
    pub(super) fn failures(&self) -> u64 { self.failures.load(Ordering::Relaxed) }
}
impl Document {
    pub(super) fn named_endpoint(&self, name: &str) -> Result<(HttpUrl, Arc<EndpointLifetime>), ReportError> {
        let state = self.endpoint_lifetimes.get(name).ok_or(ReportError::NoEndpoint)?;
        if !state.is_active() || self.endpoints.get(name) != Some(&state.url) {
            return Err(ReportError::NoEndpoint);
        }
        Ok((state.url.clone(), Arc::clone(state)))
    }
    pub(super) fn owns_endpoint(&self, name: &str, state: &Arc<EndpointLifetime>) -> bool {
        self.endpoint_lifetimes.get(name).is_some_and(|entry| Arc::ptr_eq(entry, state))
    }
    // Conservatively charge retained shared content for each application loan.
    // This describes application custody, not allocator/provider totals.
    pub(super) fn endpoint_lifetime_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<BTreeMap<String, Arc<EndpointLifetime>>>();
        for (name, state) in self.endpoint_lifetimes.iter() {
            bytes = bytes.checked_add(std::mem::size_of::<(String, Arc<EndpointLifetime>)>())?
                .checked_add(std::mem::size_of::<EndpointLifetime>())?
                .checked_add(name.len())?.checked_add(state.url.serialize().len())?;
        }
        Some(bytes)
    }
}
