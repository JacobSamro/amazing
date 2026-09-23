//! Cache API engine values, matching and Vary processing.
use crate::core::{self, BucketKey, DiskStore, EntryKey, Error, Reader, Result};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub url: String,
    pub method: String,
    pub headers: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MatchOptions {
    pub ignore_search: bool,
    pub ignore_method: bool,
    pub ignore_vary: bool,
}
#[derive(Clone, Debug)]
struct Entry {
    request: Request,
    response: Response,
}
#[derive(Clone, Debug, Default)]
pub struct Cache {
    entries: Vec<Entry>,
}
impl Cache {
    pub fn put(&mut self, request: Request, response: Response) -> Result<()> {
        if request.method != "GET" {
            return Err(Error::InvalidInput);
        }
        if header(&response.headers, "vary")
            .is_some_and(|v| v.split(',').any(|part| part.trim() == "*"))
        {
            return Err(Error::InvalidInput);
        }
        self.delete(&request, MatchOptions::default());
        self.entries.push(Entry { request, response });
        Ok(())
    }
    pub fn match_one(&self, request: &Request, options: MatchOptions) -> Option<&Response> {
        self.entries
            .iter()
            .find(|entry| matches(entry, request, options))
            .map(|e| &e.response)
    }
    pub fn match_all(&self, request: Option<&Request>, options: MatchOptions) -> Vec<&Response> {
        self.entries
            .iter()
            .filter(|e| request.is_none_or(|r| matches(e, r, options)))
            .map(|e| &e.response)
            .collect()
    }
    pub fn delete(&mut self, request: &Request, options: MatchOptions) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !matches(e, request, options));
        before - self.entries.len()
    }
    pub fn keys(&self) -> Vec<&Request> {
        self.entries.iter().map(|e| &e.request).collect()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            core::string(&mut out, &entry.request.url);
            core::string(&mut out, &entry.request.method);
            encode_headers(&mut out, &entry.request.headers);
            out.extend_from_slice(&entry.response.status.to_le_bytes());
            encode_headers(&mut out, &entry.response.headers);
            core::bytes(&mut out, &entry.response.body);
        }
        out
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let count = r.u32()?;
        if count > 1_000_000 {
            return Err(Error::Corrupt);
        }
        let mut cache = Self::default();
        for _ in 0..count {
            let request = Request {
                url: r.string()?,
                method: r.string()?,
                headers: decode_headers(&mut r)?,
            };
            let status = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| Error::Corrupt)?);
            let response = Response {
                status,
                headers: decode_headers(&mut r)?,
                body: r.bytes()?,
            };
            cache.entries.push(Entry { request, response });
        }
        if !r.done() {
            return Err(Error::Corrupt);
        }
        Ok(cache)
    }
}
fn encode_headers(out: &mut Vec<u8>, headers: &BTreeMap<String, String>) {
    out.extend_from_slice(&(headers.len() as u32).to_le_bytes());
    for (name, value) in headers {
        core::string(out, name);
        core::string(out, value)
    }
}
fn decode_headers(r: &mut Reader<'_>) -> Result<BTreeMap<String, String>> {
    let count = r.u32()?;
    if count > 100_000 {
        return Err(Error::Corrupt);
    }
    let mut headers = BTreeMap::new();
    for _ in 0..count {
        headers.insert(r.string()?, r.string()?);
    }
    Ok(headers)
}
fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}
fn matches(entry: &Entry, request: &Request, options: MatchOptions) -> bool {
    if !options.ignore_method && request.method != "GET" {
        return false;
    }
    let stored_url = if options.ignore_search {
        entry
            .request
            .url
            .split_once('?')
            .map(|(head, _)| head)
            .unwrap_or(&entry.request.url)
    } else {
        &entry.request.url
    };
    let requested_url = if options.ignore_search {
        request
            .url
            .split_once('?')
            .map(|(head, _)| head)
            .unwrap_or(&request.url)
    } else {
        &request.url
    };
    if stored_url != requested_url {
        return false;
    }
    if !options.ignore_vary {
        if let Some(vary) = header(&entry.response.headers, "vary") {
            for name in vary.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if header(&entry.request.headers, name).unwrap_or("")
                    != header(&request.headers, name).unwrap_or("")
                {
                    return false;
                }
            }
        }
    }
    true
}
#[derive(Clone, Debug, Default)]
pub struct CacheStorage {
    caches: BTreeMap<(BucketKey, String), Cache>,
}
impl CacheStorage {
    pub fn open(&mut self, bucket: BucketKey, name: impl Into<String>) -> &mut Cache {
        self.caches.entry((bucket, name.into())).or_default()
    }
    pub fn has(&self, bucket: &BucketKey, name: &str) -> bool {
        self.caches.contains_key(&(bucket.clone(), name.to_owned()))
    }
    pub fn delete(&mut self, bucket: &BucketKey, name: &str) -> bool {
        self.caches
            .remove(&(bucket.clone(), name.to_owned()))
            .is_some()
    }
    pub fn keys(&self, bucket: &BucketKey) -> Vec<&str> {
        self.caches
            .keys()
            .filter(|(b, _)| b == bucket)
            .map(|(_, n)| n.as_str())
            .collect()
    }
    pub fn match_one(
        &self,
        bucket: &BucketKey,
        request: &Request,
        options: MatchOptions,
    ) -> Option<&Response> {
        self.caches
            .iter()
            .filter(|((b, _), _)| b == bucket)
            .find_map(|(_, c)| c.match_one(request, options))
    }
}
/// Writes a complete cache mutation as one checksummed log transaction.
pub struct PersistentCacheStorage {
    disk: DiskStore,
    caches: CacheStorage,
}
impl PersistentCacheStorage {
    pub fn open(disk: DiskStore) -> Result<Self> {
        let mut caches = CacheStorage::default();
        for (entry, data) in disk.memory().entries_in("cache") {
            let name = String::from_utf8(entry.key).map_err(|_| Error::Corrupt)?;
            caches
                .caches
                .insert((entry.bucket, name), Cache::decode(&data)?);
        }
        Ok(Self { disk, caches })
    }
    pub fn get(&self, bucket: &BucketKey, name: &str) -> Option<&Cache> {
        self.caches.caches.get(&(bucket.clone(), name.to_owned()))
    }
    pub fn change<F, T>(
        &mut self,
        bucket: BucketKey,
        name: impl Into<String>,
        change: F,
    ) -> Result<T>
    where
        F: FnOnce(&mut Cache) -> Result<T>,
    {
        let name = name.into();
        let mut cache = self
            .caches
            .caches
            .get(&(bucket.clone(), name.clone()))
            .cloned()
            .unwrap_or_default();
        let result = change(&mut cache)?;
        self.disk.put(
            EntryKey {
                bucket: bucket.clone(),
                namespace: "cache".into(),
                key: name.as_bytes().to_vec(),
            },
            cache.encode(),
        )?;
        self.caches.caches.insert((bucket, name), cache);
        Ok(result)
    }
    pub fn delete(&mut self, bucket: &BucketKey, name: &str) -> Result<bool> {
        let entry = EntryKey {
            bucket: bucket.clone(),
            namespace: "cache".into(),
            key: name.as_bytes().to_vec(),
        };
        if !self
            .caches
            .caches
            .contains_key(&(bucket.clone(), name.to_owned()))
        {
            return Ok(false);
        }
        self.disk.delete(&entry)?;
        self.caches
            .caches
            .remove(&(bucket.clone(), name.to_owned()));
        Ok(true)
    }
    pub fn checkpoint(&mut self) -> Result<()> {
        self.disk.checkpoint()
    }
}
