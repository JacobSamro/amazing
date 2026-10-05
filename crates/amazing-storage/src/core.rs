//! Partitioned buckets, quota accounting, and recoverable byte storage.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StorageKey {
    pub origin: String,
    pub top_level_site: String,
}
impl StorageKey {
    pub fn new(origin: impl Into<String>, top_level_site: impl Into<String>) -> Self {
        Self {
            origin: origin.into(),
            top_level_site: top_level_site.into(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BucketKey {
    pub storage: StorageKey,
    pub name: String,
}
impl BucketKey {
    pub fn new(storage: StorageKey, name: impl Into<String>) -> Self {
        Self {
            storage,
            name: name.into(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EntryKey {
    pub bucket: BucketKey,
    pub namespace: String,
    pub key: Vec<u8>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Io(String),
    Corrupt,
    QuotaExceeded,
    InvalidInput,
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// One quota ledger spans the Web Storage, IndexedDB, and Cache logs for a profile.
pub struct QuotaLedger {
    default_limit: u64,
    limits: BTreeMap<StorageKey, u64>,
    stores: BTreeMap<u64, BTreeMap<StorageKey, u64>>,
}
impl QuotaLedger {
    pub fn new(default_limit: u64) -> Self {
        Self {
            default_limit,
            limits: BTreeMap::new(),
            stores: BTreeMap::new(),
        }
    }
    pub fn set_quota(&mut self, key: StorageKey, bytes: u64) {
        self.limits.insert(key, bytes);
    }
    pub fn quota(&self, key: &StorageKey) -> u64 {
        self.limits.get(key).copied().unwrap_or(self.default_limit)
    }
    pub fn usage(&self, key: &StorageKey) -> u64 {
        self.stores.values().fold(0u64, |total, store| {
            total.saturating_add(store.get(key).copied().unwrap_or(0))
        })
    }
    fn replace(&mut self, id: u64, key: &StorageKey, usage: u64) -> Result<()> {
        let old = self
            .stores
            .get(&id)
            .and_then(|store| store.get(key))
            .copied()
            .unwrap_or(0);
        if self.usage(key).saturating_sub(old).saturating_add(usage) > self.quota(key) {
            return Err(Error::QuotaExceeded);
        }
        self.stores
            .entry(id)
            .or_default()
            .insert(key.clone(), usage);
        Ok(())
    }
    fn register(&mut self, id: u64, memory: &MemoryStore) {
        let mut uses = BTreeMap::new();
        for key in memory.entries.keys() {
            uses.entry(key.bucket.storage.clone())
                .or_insert_with(|| memory.usage(&key.bucket.storage));
        }
        self.stores.insert(id, uses);
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    entries: BTreeMap<EntryKey, Vec<u8>>,
    limits: BTreeMap<StorageKey, u64>,
    default_limit: u64,
    access: BTreeMap<BucketKey, u64>,
    clock: u64,
}
impl MemoryStore {
    pub fn new(default_limit: u64) -> Self {
        Self {
            default_limit,
            ..Self::default()
        }
    }
    pub fn set_quota(&mut self, key: StorageKey, bytes: u64) {
        self.limits.insert(key, bytes);
    }
    pub fn quota(&self, key: &StorageKey) -> u64 {
        self.limits.get(key).copied().unwrap_or(self.default_limit)
    }
    pub fn usage(&self, key: &StorageKey) -> u64 {
        self.entries
            .iter()
            .filter(|(k, _)| &k.bucket.storage == key)
            .map(|(k, v)| (k.key.len() + k.namespace.len() + v.len()) as u64)
            .sum()
    }
    pub fn get(&mut self, key: &EntryKey) -> Option<Vec<u8>> {
        let value = self.entries.get(key).cloned();
        if value.is_some() {
            self.touch(&key.bucket);
        }
        value
    }
    pub fn peek(&self, key: &EntryKey) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }
    pub fn scan(&self, bucket: &BucketKey, namespace: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.entries
            .iter()
            .filter(|(k, _)| &k.bucket == bucket && k.namespace == namespace)
            .map(|(k, v)| (k.key.clone(), v.clone()))
            .collect()
    }
    pub fn entries_in(&self, namespace: &str) -> Vec<(EntryKey, Vec<u8>)> {
        self.entries
            .iter()
            .filter(|(k, _)| k.namespace == namespace)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    pub fn put(&mut self, key: EntryKey, value: Vec<u8>) -> Result<()> {
        let old = self.entries.insert(key.clone(), value);
        if self.usage(&key.bucket.storage) > self.quota(&key.bucket.storage) {
            match old {
                Some(v) => {
                    self.entries.insert(key, v);
                }
                None => {
                    self.entries.remove(&key);
                }
            }
            return Err(Error::QuotaExceeded);
        }
        self.touch(&key.bucket);
        Ok(())
    }
    pub fn delete(&mut self, key: &EntryKey) -> bool {
        self.entries.remove(key).is_some()
    }
    pub fn clear_namespace(&mut self, bucket: &BucketKey, namespace: &str) {
        self.entries
            .retain(|k, _| &k.bucket != bucket || k.namespace != namespace);
    }
    pub fn delete_bucket(&mut self, bucket: &BucketKey) {
        self.entries.retain(|k, _| &k.bucket != bucket);
        self.access.remove(bucket);
    }
    pub fn buckets(&self, key: &StorageKey) -> Vec<BucketKey> {
        self.access
            .keys()
            .filter(|b| &b.storage == key)
            .cloned()
            .collect()
    }
    /// Evict least recently accessed buckets until usage is at most `target`.
    /// Buckets explicitly marked persistent by the caller are spared.
    pub fn evict_to(
        &mut self,
        key: &StorageKey,
        target: u64,
        persistent: &[BucketKey],
    ) -> Vec<BucketKey> {
        let mut candidates: Vec<_> = self
            .access
            .iter()
            .filter(|(b, _)| &b.storage == key && !persistent.contains(b))
            .map(|(b, t)| (*t, b.clone()))
            .collect();
        candidates.sort();
        let mut removed = Vec::new();
        for (_, bucket) in candidates {
            if self.usage(key) <= target {
                break;
            }
            self.delete_bucket(&bucket);
            removed.push(bucket);
        }
        removed
    }
    fn touch(&mut self, bucket: &BucketKey) {
        self.clock = self.clock.saturating_add(1);
        self.access.insert(bucket.clone(), self.clock);
    }
    pub fn transaction<F, T>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Self) -> Result<T>,
    {
        // Stage separately so unwinding cannot expose a partially applied transaction.
        let mut next = self.clone();
        let value = f(&mut next)?;
        *self = next;
        Ok(value)
    }
}

// All integers are little endian; every variable byte string has a u32 length.
pub(crate) fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
}
pub(crate) fn string(out: &mut Vec<u8>, value: &str) {
    bytes(out, value.as_bytes());
}
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Corrupt)?;
        let part = self.data.get(self.pos..end).ok_or(Error::Corrupt)?;
        self.pos = end;
        Ok(part)
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| Error::Corrupt)?,
        ))
    }
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| Error::Corrupt)?,
        ))
    }
    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub(crate) fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| Error::Corrupt)
    }
    pub(crate) fn done(&self) -> bool {
        self.pos == self.data.len()
    }
}
fn encode_key(out: &mut Vec<u8>, key: &EntryKey) {
    string(out, &key.bucket.storage.origin);
    string(out, &key.bucket.storage.top_level_site);
    string(out, &key.bucket.name);
    string(out, &key.namespace);
    bytes(out, &key.key);
}
fn decode_key(r: &mut Reader<'_>) -> Result<EntryKey> {
    Ok(EntryKey {
        bucket: BucketKey::new(StorageKey::new(r.string()?, r.string()?), r.string()?),
        namespace: r.string()?,
        key: r.bytes()?,
    })
}
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    !crc
}
const LOG_MAGIC: &[u8; 8] = b"AMSTLOG1";
const SNAP_MAGIC: &[u8; 8] = b"AMSTSN01";
const FORMAT_MAGIC: &[u8; 8] = b"AMSTFMT1";
/// On-disk format version. Bump this whenever the log or checkpoint byte
/// layout changes incompatibly; an on-disk store stamped with any other
/// version is treated the same as a corrupt store (quarantined, not opened).
const FORMAT_VERSION: u32 = 1;
const MAX_RECORD: usize = 64 * 1024 * 1024;
fn read_format_version(path: &Path) -> Result<u32> {
    let data = fs::read(path)?;
    if data.len() != 12 || &data[..8] != FORMAT_MAGIC {
        return Err(Error::Corrupt);
    }
    Ok(u32::from_le_bytes(
        data[8..12].try_into().map_err(|_| Error::Corrupt)?,
    ))
}
fn write_format_version(path: &Path) -> Result<()> {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(FORMAT_MAGIC);
    data.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    fs::write(path, data)?;
    Ok(())
}
fn read_checkpoint(path: &Path) -> Result<(u64, BTreeMap<EntryKey, Vec<u8>>)> {
    let data = fs::read(path)?;
    if data.len() < 24 || &data[..8] != SNAP_MAGIC {
        return Err(Error::Corrupt);
    }
    let generation = u64::from_le_bytes(data[8..16].try_into().map_err(|_| Error::Corrupt)?);
    let len = u32::from_le_bytes(data[16..20].try_into().map_err(|_| Error::Corrupt)?) as usize;
    if generation == 0
        || len > MAX_RECORD
        || data.len() != len + 24
        || crc32(&data[20..20 + len])
            != u32::from_le_bytes(data[20 + len..].try_into().map_err(|_| Error::Corrupt)?)
    {
        return Err(Error::Corrupt);
    }
    let mut r = Reader::new(&data[20..20 + len]);
    let count = r.u32()?;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let key = decode_key(&mut r)?;
        let value = r.bytes()?;
        if entries.insert(key, value).is_some() {
            return Err(Error::Corrupt);
        }
    }
    if !r.done() {
        return Err(Error::Corrupt);
    }
    Ok((generation, entries))
}
fn replay_operation(r: &mut Reader<'_>, entries: &mut BTreeMap<EntryKey, Vec<u8>>) -> Result<()> {
    let op = r.take(1)?[0];
    replay_value(op, r, entries)
}
fn replay_value(
    op: u8,
    r: &mut Reader<'_>,
    entries: &mut BTreeMap<EntryKey, Vec<u8>>,
) -> Result<()> {
    let key = decode_key(r)?;
    match op {
        1 => {
            entries.insert(key, r.bytes()?);
        }
        2 => {
            entries.remove(&key);
        }
        _ => return Err(Error::Corrupt),
    }
    Ok(())
}
fn sync_directory(path: &Path) -> io::Result<()> {
    // Unix requires a directory fsync to make a newly created log or renamed snapshot
    // durable. Windows does not support opening directories with std::fs::File.
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
/// A committed transaction is one complete checksummed log record. A torn tail is ignored.
/// Checkpoints rotate between two slots so Windows renames never replace the active slot.
pub struct DiskStore {
    path: PathBuf,
    memory: MemoryStore,
    generation: u64,
    shared_quota: Option<Arc<Mutex<QuotaLedger>>>,
    quota_id: u64,
    poisoned: bool,
}
impl DiskStore {
    /// Open the store at `path`, never failing the caller because the on-disk
    /// data is unreadable. A store that fails to parse (torn/corrupt log or
    /// checkpoint) or carries an unrecognized format version is quarantined
    /// -- moved aside with a timestamped suffix, logged once -- and a fresh,
    /// empty store is opened in its place. A browser must never fail to load
    /// a page because its local storage on disk is stale or damaged.
    pub fn open(path: impl AsRef<Path>, default_limit: u64) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        match Self::try_open(&path, default_limit) {
            Ok(store) => Ok(store),
            Err(Error::Corrupt) => {
                Self::quarantine(&path)?;
                Self::try_open(&path, default_limit)
            }
            Err(e) => Err(e),
        }
    }
    /// Move an unreadable store directory aside so a fresh one can be
    /// created in its place. A no-op if nothing is there yet.
    fn quarantine(path: &Path) -> Result<()> {
        if !path.exists() {
            return Ok(());
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("store")
            .to_string();
        let quarantined = path.with_file_name(format!("{name}.corrupt-{stamp}"));
        fs::rename(path, &quarantined)?;
        eprintln!(
            "amazing-storage: {} was corrupt or an unrecognized format version; \
             quarantined to {} and starting a fresh store",
            path.display(),
            quarantined.display()
        );
        Ok(())
    }
    fn try_open(path: &Path, default_limit: u64) -> Result<Self> {
        let path = path.to_path_buf();
        fs::create_dir_all(&path)?;
        let format_path = path.join("format");
        if format_path.exists() {
            let version = read_format_version(&format_path)?;
            if version != FORMAT_VERSION {
                return Err(Error::Corrupt);
            }
        } else {
            write_format_version(&format_path)?;
        }
        let mut memory = MemoryStore::new(default_limit);
        let mut generation = 0;
        let mut saw_checkpoint = false;
        for slot in 0..2 {
            let snap = path.join(format!("checkpoint.{slot}"));
            if !snap.exists() {
                continue;
            }
            saw_checkpoint = true;
            if let Ok((candidate, entries)) = read_checkpoint(&snap) {
                if candidate > generation {
                    generation = candidate;
                    memory.entries = entries;
                }
            }
        }
        if saw_checkpoint && generation == 0 {
            return Err(Error::Corrupt);
        }
        for key in memory.entries.keys() {
            memory.access.entry(key.bucket.clone()).or_insert(0);
        }
        let log = path.join("log");
        if log.exists() {
            let data = fs::read(&log)?;
            if data.len() < 8 {
                if !LOG_MAGIC.starts_with(&data) {
                    return Err(Error::Corrupt);
                }
                let file = OpenOptions::new().write(true).open(&log)?;
                file.set_len(0)?;
                let mut file = OpenOptions::new().write(true).open(&log)?;
                file.write_all(LOG_MAGIC)?;
                return Ok(Self {
                    path,
                    memory,
                    generation,
                    shared_quota: None,
                    quota_id: 0,
                    poisoned: false,
                });
            }
            if &data[..8] != LOG_MAGIC {
                return Err(Error::Corrupt);
            }
            let mut offset = 8;
            while offset + 8 <= data.len() {
                let len = u32::from_le_bytes(
                    data[offset..offset + 4]
                        .try_into()
                        .map_err(|_| Error::Corrupt)?,
                ) as usize;
                if len > MAX_RECORD || offset + 8 + len > data.len() {
                    break;
                }
                let payload = &data[offset + 4..offset + 4 + len];
                let expected = u32::from_le_bytes(
                    data[offset + 4 + len..offset + 8 + len]
                        .try_into()
                        .map_err(|_| Error::Corrupt)?,
                );
                if crc32(payload) != expected {
                    break;
                }
                let mut r = Reader::new(payload);
                let op = r.take(1)?[0];
                if op == 3 {
                    // Validate the whole batch before exposing any of its operations.
                    let count = r.u32()?;
                    let mut entries = memory.entries.clone();
                    for _ in 0..count {
                        replay_operation(&mut r, &mut entries)?;
                    }
                    if !r.done() {
                        return Err(Error::Corrupt);
                    }
                    memory.entries = entries;
                } else {
                    replay_value(op, &mut r, &mut memory.entries)?;
                    if !r.done() {
                        return Err(Error::Corrupt);
                    }
                }
                offset += len + 8;
            }
            // A future append must replace a torn tail, or the next valid record is invisible.
            let file = OpenOptions::new().write(true).open(log)?;
            file.set_len(offset as u64)?;
        }
        for key in memory.entries.keys() {
            memory.access.entry(key.bucket.clone()).or_insert(0);
        }
        Ok(Self {
            path,
            memory,
            generation,
            shared_quota: None,
            quota_id: 0,
            poisoned: false,
        })
    }
    pub fn attach_quota(&mut self, ledger: Arc<Mutex<QuotaLedger>>) {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        ledger.lock().unwrap().register(id, &self.memory);
        self.shared_quota = Some(ledger);
        self.quota_id = id;
    }
    pub fn memory(&self) -> &MemoryStore {
        &self.memory
    }
    pub fn get(&mut self, key: &EntryKey) -> Option<Vec<u8>> {
        self.memory.get(key)
    }
    pub fn put(&mut self, key: EntryKey, value: Vec<u8>) -> Result<()> {
        let mut next = self.memory.clone();
        next.put(key.clone(), value.clone())?;
        let ledger = self.shared_quota.clone();
        let mut quota = ledger.as_ref().map(|ledger| ledger.lock().unwrap());
        let usage = next.usage(&key.bucket.storage);
        if let Some(quota) = quota.as_mut() {
            let old = quota
                .stores
                .get(&self.quota_id)
                .and_then(|store| store.get(&key.bucket.storage))
                .copied()
                .unwrap_or(0);
            if quota
                .usage(&key.bucket.storage)
                .saturating_sub(old)
                .saturating_add(usage)
                > quota.quota(&key.bucket.storage)
            {
                return Err(Error::QuotaExceeded);
            }
        }
        let mut payload = vec![1];
        encode_key(&mut payload, &key);
        bytes(&mut payload, &value);
        self.append(&payload)?;
        self.memory = next;
        if let Some(quota) = quota.as_mut() {
            quota.replace(self.quota_id, &key.bucket.storage, usage)?;
        }
        Ok(())
    }
    pub fn delete(&mut self, key: &EntryKey) -> Result<bool> {
        let ledger = self.shared_quota.clone();
        let mut quota = ledger.as_ref().map(|ledger| ledger.lock().unwrap());
        let mut payload = vec![2];
        encode_key(&mut payload, key);
        self.append(&payload)?;
        let deleted = self.memory.delete(key);
        let usage = self.memory.usage(&key.bucket.storage);
        if let Some(quota) = quota.as_mut() {
            quota
                .stores
                .entry(self.quota_id)
                .or_default()
                .insert(key.bucket.storage.clone(), usage);
        }
        Ok(deleted)
    }
    /// Stage a multi-key update and durably publish its final delta as one log record.
    /// The callback sees its own writes; readers and quota accounting see only committed
    /// state. Errors (including quota or persistence errors) discard the staged state.
    /// A panic also leaves this store untouched. Transactions are serialized by `&mut self`.
    pub fn transaction<F, T>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&mut MemoryStore) -> Result<T>,
    {
        let mut next = self.memory.clone();
        let value = f(&mut next)?;
        let mut operations = Vec::new();
        let mut count = 0u32;
        for key in self.memory.entries.keys() {
            if !next.entries.contains_key(key) {
                operations.push(2);
                encode_key(&mut operations, key);
                count = count.checked_add(1).ok_or(Error::InvalidInput)?;
            }
        }
        for (key, value) in &next.entries {
            if self.memory.entries.get(key) != Some(value) {
                operations.push(1);
                encode_key(&mut operations, key);
                bytes(&mut operations, value);
                count = count.checked_add(1).ok_or(Error::InvalidInput)?;
            }
        }
        let mut partitions = std::collections::BTreeSet::new();
        for key in self.memory.entries.keys().chain(next.entries.keys()) {
            partitions.insert(key.bucket.storage.clone());
        }
        let ledger = self.shared_quota.clone();
        let mut quota = ledger.as_ref().map(|ledger| ledger.lock().unwrap());
        for key in &partitions {
            let usage = next.usage(key);
            if usage > next.quota(key) {
                return Err(Error::QuotaExceeded);
            }
            if let Some(quota) = quota.as_ref() {
                let old = quota
                    .stores
                    .get(&self.quota_id)
                    .and_then(|s| s.get(key))
                    .copied()
                    .unwrap_or(0);
                if quota.usage(key).saturating_sub(old).saturating_add(usage) > quota.quota(key) {
                    return Err(Error::QuotaExceeded);
                }
            }
        }
        if count != 0 {
            let mut payload = vec![3];
            payload.extend_from_slice(&count.to_le_bytes());
            payload.extend_from_slice(&operations);
            self.append(&payload)?;
        }
        self.memory = next;
        if let Some(quota) = quota.as_mut() {
            for key in partitions {
                quota
                    .stores
                    .entry(self.quota_id)
                    .or_default()
                    .insert(key.clone(), self.memory.usage(&key));
            }
        }
        Ok(value)
    }
    fn append(&mut self, payload: &[u8]) -> Result<()> {
        if self.poisoned {
            return Err(Error::Io(
                "store needs reopening after failed log repair".into(),
            ));
        }
        if payload.len() > MAX_RECORD {
            return Err(Error::InvalidInput);
        }
        let path = self.path.join("log");
        let new = !path.exists();
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let original_len = file.metadata()?.len();
        let result = (|| -> io::Result<()> {
            if original_len == 0 {
                file.write_all(LOG_MAGIC)?;
            }
            file.write_all(&(payload.len() as u32).to_le_bytes())?;
            file.write_all(payload)?;
            file.write_all(&crc32(payload).to_le_bytes())?;
            file.sync_all()?;
            if new {
                sync_directory(&self.path)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            // A failed append must not hide a later successful commit behind a torn tail.
            if file
                .set_len(original_len)
                .and_then(|_| file.sync_all())
                .is_err()
            {
                self.poisoned = true;
            }
            return Err(error.into());
        }
        Ok(())
    }
    pub fn checkpoint(&mut self) -> Result<()> {
        if self.poisoned {
            return Err(Error::Io(
                "store needs reopening after failed log repair".into(),
            ));
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&(self.memory.entries.len() as u32).to_le_bytes());
        for (key, value) in &self.memory.entries {
            encode_key(&mut payload, key);
            bytes(&mut payload, value);
        }
        if payload.len() > MAX_RECORD {
            return Err(Error::InvalidInput);
        }
        let next = self.generation.checked_add(1).ok_or(Error::InvalidInput)?;
        let slot = next % 2;
        let temporary = self.path.join(format!("checkpoint.{slot}.tmp"));
        let mut file = File::create(&temporary)?;
        file.write_all(SNAP_MAGIC)?;
        file.write_all(&next.to_le_bytes())?;
        file.write_all(&(payload.len() as u32).to_le_bytes())?;
        file.write_all(&payload)?;
        file.write_all(&crc32(&payload).to_le_bytes())?;
        file.sync_all()?;
        drop(file);
        let target = self.path.join(format!("checkpoint.{slot}"));
        if target.exists() {
            fs::remove_file(&target)?;
        }
        fs::rename(&temporary, target)?;
        sync_directory(&self.path)?;
        self.generation = next;
        let mut log = File::create(self.path.join("log"))?;
        log.write_all(LOG_MAGIC)?;
        log.sync_all()?;
        Ok(())
    }
}

impl Drop for DiskStore {
    fn drop(&mut self) {
        if let Some(ledger) = &self.shared_quota {
            ledger.lock().unwrap().stores.remove(&self.quota_id);
        }
    }
}
