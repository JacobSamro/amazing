//! Origin-partitioned Web Locks arbitration shared by documents and the storage process.
use crate::core::StorageKey;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode { Shared, Exclusive }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockInfo {
    pub id: u64,
    pub client_id: u64,
    pub name: String,
    pub mode: Mode,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub held: Vec<LockInfo>,
    pub pending: Vec<LockInfo>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Options {
    pub if_available: bool,
    pub steal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error { InvalidName, IncompatibleOptions }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestOutcome {
    pub id: Option<u64>,
    pub granted: Vec<u64>,
    pub stolen: Vec<u64>,
}

#[derive(Default)]
struct Queue {
    held: Vec<LockInfo>,
    pending: VecDeque<LockInfo>,
}

#[derive(Default)]
pub struct LockCoordinator {
    next_id: u64,
    queues: BTreeMap<(StorageKey, String), Queue>,
}

impl LockCoordinator {
    pub fn request(&mut self, key: StorageKey, client_id: u64, name: String, mode: Mode, options: Options)
        -> Result<RequestOutcome, Error>
    {
        if name.starts_with('-') { return Err(Error::InvalidName); }
        if options.if_available && options.steal { return Err(Error::IncompatibleOptions); }
        self.next_id += 1;
        let id = self.next_id;
        let queue = self.queues.entry((key, name.clone())).or_default();
        let request = LockInfo { id, client_id, name, mode };
        if options.if_available {
            if !queue.pending.is_empty() || !Self::compatible(&queue.held, mode) {
                return Ok(RequestOutcome { id: None, granted: Vec::new(), stolen: Vec::new() });
            }
            queue.held.push(request);
            return Ok(RequestOutcome { id: Some(id), granted: vec![id], stolen: Vec::new() });
        }
        let stolen = if options.steal { queue.held.drain(..).map(|lock| lock.id).collect() } else { Vec::new() };
        if options.steal { queue.pending.push_front(request); } else { queue.pending.push_back(request); }
        let granted = Self::grant(queue);
        Ok(RequestOutcome { id: Some(id), granted, stolen })
    }

    pub fn release(&mut self, id: u64) -> Vec<u64> {
        for queue in self.queues.values_mut() {
            let len = queue.held.len();
            queue.held.retain(|lock| lock.id != id);
            if queue.held.len() != len { return Self::grant(queue); }
            let len = queue.pending.len();
            queue.pending.retain(|lock| lock.id != id);
            if queue.pending.len() != len { return Self::grant(queue); }
        }
        Vec::new()
    }

    pub fn release_client(&mut self, client_id: u64) -> Vec<u64> {
        let mut granted = Vec::new();
        for queue in self.queues.values_mut() {
            queue.held.retain(|lock| lock.client_id != client_id);
            queue.pending.retain(|lock| lock.client_id != client_id);
            granted.extend(Self::grant(queue));
        }
        granted
    }

    pub fn query(&self, key: &StorageKey) -> Snapshot {
        let mut result = Snapshot::default();
        for ((storage, _), queue) in &self.queues {
            if storage == key {
                result.held.extend(queue.held.iter().cloned());
                result.pending.extend(queue.pending.iter().cloned());
            }
        }
        result
    }

    fn compatible(held: &[LockInfo], mode: Mode) -> bool {
        held.is_empty() || mode == Mode::Shared && held.iter().all(|lock| lock.mode == Mode::Shared)
    }

    fn grant(queue: &mut Queue) -> Vec<u64> {
        let mut granted = Vec::new();
        while let Some(next) = queue.pending.front() {
            if !Self::compatible(&queue.held, next.mode) { break; }
            let next = queue.pending.pop_front().unwrap();
            granted.push(next.id);
            queue.held.push(next);
        }
        granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_requests_queue_behind_exclusive_and_release_in_order() {
        let key = StorageKey::new("https://a.test", "https://a.test");
        let mut locks = LockCoordinator::default();
        let first = locks.request(key.clone(), 1, "db".into(), Mode::Exclusive, Options::default()).unwrap();
        let second = locks.request(key.clone(), 2, "db".into(), Mode::Shared, Options::default()).unwrap();
        let third = locks.request(key.clone(), 3, "db".into(), Mode::Shared, Options::default()).unwrap();
        assert_eq!(first.granted, vec![first.id.unwrap()]);
        assert!(second.granted.is_empty());
        assert_eq!(locks.query(&key).pending.len(), 2);
        assert_eq!(locks.release(first.id.unwrap()), vec![second.id.unwrap(), third.id.unwrap()]);
        assert_eq!(locks.query(&key).held.len(), 2);
    }

    #[test]
    fn if_available_and_steal_respect_waiters_and_revoke_holders() {
        let key = StorageKey::new("https://a.test", "https://a.test");
        let mut locks = LockCoordinator::default();
        let first = locks.request(key.clone(), 1, "db".into(), Mode::Shared, Options::default()).unwrap();
        let waiting = locks.request(key.clone(), 2, "db".into(), Mode::Exclusive, Options::default()).unwrap();
        let unavailable = locks.request(key.clone(), 3, "db".into(), Mode::Shared,
            Options { if_available: true, steal: false }).unwrap();
        assert_eq!(unavailable.id, None);
        let stolen = locks.request(key.clone(), 4, "db".into(), Mode::Exclusive,
            Options { if_available: false, steal: true }).unwrap();
        assert_eq!(stolen.stolen, vec![first.id.unwrap()]);
        assert_eq!(stolen.granted, vec![stolen.id.unwrap()]);
        assert_eq!(locks.release(stolen.id.unwrap()), vec![waiting.id.unwrap()]);
    }

    #[test]
    fn client_disconnect_releases_held_and_queued_locks() {
        let key = StorageKey::new("https://a.test", "https://a.test");
        let other = StorageKey::new("https://b.test", "https://b.test");
        let mut locks = LockCoordinator::default();
        let held = locks.request(key.clone(), 1, "db".into(), Mode::Exclusive, Options::default()).unwrap();
        let next = locks.request(key.clone(), 2, "db".into(), Mode::Exclusive, Options::default()).unwrap();
        locks.request(other.clone(), 3, "db".into(), Mode::Exclusive, Options::default()).unwrap();
        assert_eq!(locks.release_client(1), vec![next.id.unwrap()]);
        assert!(locks.query(&key).held.iter().all(|lock| lock.id != held.id.unwrap()));
        assert_eq!(locks.query(&other).held.len(), 1);
    }
}
