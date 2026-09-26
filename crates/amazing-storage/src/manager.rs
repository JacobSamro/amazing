//! Persistent-storage state and profile-wide usage estimates.
use crate::core::{BucketKey, DiskStore, EntryKey, Error, QuotaLedger, Result, StorageKey};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Estimate { pub usage: u64, pub quota: u64 }

pub struct StorageManager {
    disk: DiskStore,
    quota: Arc<Mutex<QuotaLedger>>,
    persistent: BTreeSet<StorageKey>,
}

impl StorageManager {
    pub fn open(disk: DiskStore, quota: Arc<Mutex<QuotaLedger>>) -> Result<Self> {
        let mut persistent = BTreeSet::new();
        for (entry, value) in disk.memory().entries_in("storage-manager") {
            if entry.key != b"persistent" || value != [1] { return Err(Error::Corrupt); }
            persistent.insert(entry.bucket.storage);
        }
        Ok(Self { disk, quota, persistent })
    }

    pub fn estimate(&self, key: &StorageKey) -> Estimate {
        let quota = self.quota.lock().unwrap();
        Estimate { usage: quota.usage(key), quota: quota.quota(key) }
    }

    pub fn persisted(&self, key: &StorageKey) -> bool { self.persistent.contains(key) }

    /// The embedding browser's current policy grants persistence immediately.
    pub fn persist(&mut self, key: StorageKey) -> Result<bool> {
        if self.persistent.contains(&key) { return Ok(true); }
        self.disk.put(EntryKey {
            bucket: BucketKey::new(key.clone(), "default"),
            namespace: "storage-manager".into(),
            key: b"persistent".to_vec(),
        }, vec![1])?;
        self.persistent.insert(key);
        Ok(true)
    }
}
