//! Storage Buckets: named buckets with per-bucket persistence, durability,
//! expiration and quota policy, persisted in the profile's storage directory.
//!
//! A bucket handle is valid only while its name still exists and its epoch is
//! unchanged, so a deleted (or deleted-and-recreated) bucket makes every stale
//! handle reject with `UnknownError`.
use crate::core::{BucketKey, DiskStore, EntryKey, Error, Reader, Result, StorageKey};
use crate::idb::Durability;
use std::collections::BTreeMap;

/// Maximum accepted bucket name length, in UTF-16 code units. Valid names are
/// ASCII, so bytes and code units coincide.
pub const MAX_BUCKET_NAME: usize = 64;

/// §"StorageBucketManager": a name is `[a-z0-9][a-z0-9_-]{0,63}`.
pub fn valid_bucket_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    let mut len = 1usize;
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
            return false;
        }
        len += 1;
    }
    len <= MAX_BUCKET_NAME
}

/// The requested policy for one bucket. Reopening an existing bucket keeps
/// the policy it was created with, per the Storage Buckets specification.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Policy {
    pub persisted: bool,
    pub durability: Durability,
    pub expires: Option<i64>,
    pub quota: Option<u64>,
}

pub struct BucketRegistry {
    disk: DiskStore,
    buckets: BTreeMap<StorageKey, BTreeMap<String, Policy>>,
    /// In-memory per-name epoch counter; bumped on delete so stale handles fail.
    epochs: BTreeMap<(StorageKey, String), u64>,
}

impl BucketRegistry {
    pub fn load(disk: DiskStore) -> Result<Self> {
        let mut buckets: BTreeMap<StorageKey, BTreeMap<String, Policy>> = BTreeMap::new();
        for (entry, data) in disk.memory().entries_in("storage-buckets") {
            let name = String::from_utf8(entry.key).map_err(|_| Error::Corrupt)?;
            let policy = decode_policy(&data)?;
            buckets.entry(entry.bucket.storage).or_default().insert(name, policy);
        }
        Ok(Self { disk, buckets, epochs: BTreeMap::new() })
    }

    pub fn keys(&self, key: &StorageKey) -> Vec<String> {
        self.buckets.get(key).map(|map| map.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn policy(&self, key: &StorageKey, name: &str) -> Option<&Policy> {
        self.buckets.get(key).and_then(|map| map.get(name))
    }

    pub fn handle_valid(&self, key: &StorageKey, name: &str, epoch: u64) -> bool {
        self.policy(key, name).is_some() && self.epoch(key, name) == epoch
    }

    /// Creates the bucket when absent, otherwise returns the existing one
    /// unchanged. Returns the current handle epoch.
    pub fn open(&mut self, key: StorageKey, name: String, policy: Policy) -> Result<u64> {
        let exists = self.buckets.get(&key).is_some_and(|map| map.contains_key(&name));
        if !exists {
            self.persist(&key, &name, &policy)?;
            self.buckets.entry(key.clone()).or_default().insert(name.clone(), policy);
        }
        Ok(self.epoch(&key, &name))
    }

    pub fn delete(&mut self, key: &StorageKey, name: &str) -> Result<bool> {
        let existed = self.buckets.get(key).is_some_and(|map| map.contains_key(name));
        if !existed {
            return Ok(false);
        }
        self.disk.delete(&entry_for(key, name))?;
        if let Some(map) = self.buckets.get_mut(key) {
            map.remove(name);
        }
        let counter = self.epochs.entry((key.clone(), name.to_owned())).or_insert(0);
        *counter = counter.saturating_add(1);
        Ok(true)
    }

    /// Replaces the policy of an existing bucket; ignores unknown names.
    pub fn update(&mut self, key: &StorageKey, name: &str, policy: Policy) -> Result<()> {
        if self.policy(key, name).is_none() {
            return Ok(());
        }
        self.persist(key, name, &policy)?;
        self.buckets.entry(key.clone()).or_default().insert(name.to_owned(), policy);
        Ok(())
    }

    fn epoch(&self, key: &StorageKey, name: &str) -> u64 {
        self.epochs.get(&(key.clone(), name.to_owned())).copied().unwrap_or(0)
    }

    fn persist(&mut self, key: &StorageKey, name: &str, policy: &Policy) -> Result<()> {
        self.disk.put(entry_for(key, name), encode_policy(policy))
    }
}

fn entry_for(key: &StorageKey, name: &str) -> EntryKey {
    EntryKey {
        bucket: BucketKey::new(key.clone(), name.to_owned()),
        namespace: "storage-buckets".into(),
        key: name.as_bytes().to_vec(),
    }
}

fn encode_policy(policy: &Policy) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(u8::from(policy.persisted));
    out.push(match policy.durability {
        Durability::Relaxed => 0,
        Durability::Strict => 1,
        Durability::Default => 2,
    });
    match policy.expires {
        Some(expires) => {
            out.push(1);
            out.extend_from_slice(&expires.to_le_bytes());
        }
        None => out.push(0),
    }
    match policy.quota {
        Some(quota) => {
            out.push(1);
            out.extend_from_slice(&quota.to_le_bytes());
        }
        None => out.push(0),
    }
    out
}

fn decode_policy(data: &[u8]) -> Result<Policy> {
    let mut r = Reader::new(data);
    let persisted = r.take(1)?[0] != 0;
    let durability = match r.take(1)?[0] {
        0 => Durability::Relaxed,
        1 => Durability::Strict,
        2 => Durability::Default,
        _ => return Err(Error::Corrupt),
    };
    let expires = if r.take(1)?[0] != 0 {
        Some(i64::from_le_bytes(r.take(8)?.try_into().map_err(|_| Error::Corrupt)?))
    } else {
        None
    };
    let quota = if r.take(1)?[0] != 0 {
        Some(u64::from_le_bytes(r.take(8)?.try_into().map_err(|_| Error::Corrupt)?))
    } else {
        None
    };
    if !r.done() {
        return Err(Error::Corrupt);
    }
    Ok(Policy { persisted, durability, expires, quota })
}
