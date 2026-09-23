//! Ordered Web Storage maps. Values are UTF-16 code units, including unpaired surrogates.
use crate::core::{BucketKey, DiskStore, EntryKey, Error, Result, StorageKey};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageChange {
    pub key: Option<Vec<u16>>,
    pub old_value: Option<Vec<u16>>,
    pub new_value: Option<Vec<u16>>,
    pub source: u64,
}
#[derive(Clone, Debug, Default)]
pub struct StorageArea {
    items: Vec<(Vec<u16>, Vec<u16>)>,
    quota: usize,
}
impl StorageArea {
    pub fn new(quota: usize) -> Self {
        Self {
            quota,
            items: Vec::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn key(&self, index: usize) -> Option<&[u16]> {
        self.items.get(index).map(|(k, _)| k.as_slice())
    }
    pub fn get(&self, key: &[u16]) -> Option<&[u16]> {
        self.items
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_slice())
    }
    pub fn usage(&self) -> usize {
        self.items
            .iter()
            .map(|(k, v)| (k.len() + v.len()) * 2)
            .sum()
    }
    pub fn set(
        &mut self,
        key: Vec<u16>,
        value: Vec<u16>,
        source: u64,
    ) -> Result<Option<StorageChange>> {
        let old = self.get(&key).map(<[u16]>::to_vec);
        if old.as_ref() == Some(&value) {
            return Ok(None);
        }
        let new_usage = self.usage() - old.as_ref().map(|v| (key.len() + v.len()) * 2).unwrap_or(0)
            + (key.len() + value.len()) * 2;
        if new_usage > self.quota {
            return Err(Error::QuotaExceeded);
        }
        if let Some((_, v)) = self.items.iter_mut().find(|(k, _)| *k == key) {
            *v = value.clone();
        } else {
            self.items.push((key.clone(), value.clone()));
        }
        Ok(Some(StorageChange {
            key: Some(key),
            old_value: old,
            new_value: Some(value),
            source,
        }))
    }
    pub fn remove(&mut self, key: &[u16], source: u64) -> Option<StorageChange> {
        let index = self.items.iter().position(|(k, _)| k == key)?;
        let (key, old) = self.items.remove(index);
        Some(StorageChange {
            key: Some(key),
            old_value: Some(old),
            new_value: None,
            source,
        })
    }
    pub fn clear(&mut self, source: u64) -> Option<StorageChange> {
        if self.items.is_empty() {
            return None;
        }
        self.items.clear();
        Some(StorageChange {
            key: None,
            old_value: None,
            new_value: None,
            source,
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.items.len() as u32).to_le_bytes());
        for (k, v) in &self.items {
            for units in [k, v] {
                out.extend_from_slice(&(units.len() as u32).to_le_bytes());
                for unit in units {
                    out.extend_from_slice(&unit.to_le_bytes());
                }
            }
        }
        out
    }
    pub fn decode(data: &[u8], quota: usize) -> Result<Self> {
        let mut r = crate::core::Reader::new(data);
        let count = r.u32()?;
        let mut area = Self::new(quota);
        for _ in 0..count {
            let read_units = |r: &mut crate::core::Reader<'_>| -> Result<Vec<u16>> {
                let len = r.u32()? as usize;
                if len > quota / 2 {
                    return Err(Error::Corrupt);
                }
                let data = r.take(len * 2)?;
                Ok(data
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect())
            };
            let key = read_units(&mut r)?;
            let value = read_units(&mut r)?;
            area.set(key, value, 0)?;
        }
        if !r.done() {
            return Err(Error::Corrupt);
        }
        Ok(area)
    }
}
#[derive(Clone, Debug)]
pub struct WebStorage {
    local: BTreeMap<BucketKey, StorageArea>,
    session: BTreeMap<(u64, StorageKey), StorageArea>,
    quota: usize,
}
impl WebStorage {
    pub fn new(quota: usize) -> Self {
        Self {
            local: BTreeMap::new(),
            session: BTreeMap::new(),
            quota,
        }
    }
    pub fn local(&mut self, bucket: BucketKey) -> &mut StorageArea {
        self.local
            .entry(bucket)
            .or_insert_with(|| StorageArea::new(self.quota))
    }
    pub fn session(&mut self, session: u64, key: StorageKey) -> &mut StorageArea {
        self.session
            .entry((session, key))
            .or_insert_with(|| StorageArea::new(self.quota))
    }
    pub fn close_session(&mut self, session: u64) {
        self.session.retain(|(id, _), _| *id != session);
    }
    pub fn clone_session(&mut self, from: u64, to: u64) {
        let copies: Vec<_> = self
            .session
            .iter()
            .filter(|((id, _), _)| *id == from)
            .map(|((_, key), area)| (key.clone(), area.clone()))
            .collect();
        for (key, area) in copies {
            self.session.insert((to, key), area);
        }
    }
}

/// Local areas are committed to the shared log before becoming visible.
/// Session areas are deliberately process/session scoped and never written.
pub struct PersistentWebStorage {
    disk: DiskStore,
    areas: WebStorage,
}
impl PersistentWebStorage {
    pub fn open(disk: DiskStore, quota: usize) -> Result<Self> {
        let mut areas = WebStorage::new(quota);
        for (entry, data) in disk.memory().entries_in("web-local") {
            if entry.key != b"area" {
                return Err(Error::Corrupt);
            }
            areas
                .local
                .insert(entry.bucket, StorageArea::decode(&data, quota)?);
        }
        Ok(Self { disk, areas })
    }
    pub fn local(&self, bucket: &BucketKey) -> Option<&StorageArea> {
        self.areas.local.get(bucket)
    }
    pub fn change_local<F>(&mut self, bucket: BucketKey, change: F) -> Result<Option<StorageChange>>
    where
        F: FnOnce(&mut StorageArea) -> Result<Option<StorageChange>>,
    {
        let mut area = self
            .areas
            .local
            .get(&bucket)
            .cloned()
            .unwrap_or_else(|| StorageArea::new(self.areas.quota));
        let record = change(&mut area)?;
        if record.is_some() {
            let entry = EntryKey {
                bucket: bucket.clone(),
                namespace: "web-local".into(),
                key: b"area".to_vec(),
            };
            self.disk.put(entry, area.encode())?;
            self.areas.local.insert(bucket, area);
        }
        Ok(record)
    }
    pub fn session(&mut self, session: u64, key: StorageKey) -> &mut StorageArea {
        self.areas.session(session, key)
    }
    pub fn close_session(&mut self, session: u64) {
        self.areas.close_session(session)
    }
    pub fn checkpoint(&mut self) -> Result<()> {
        self.disk.checkpoint()
    }
}
