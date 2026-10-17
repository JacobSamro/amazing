//! RFC 9111 cache policy. The embedding owns transport, clock, and persistence.
//! The default storage bounds mirror the shell's existing in-memory cache.
use crate::{cookies::date::parse_date, HeaderList};
use std::collections::VecDeque;

const MAX_KEY_BYTES: usize = 8192;
const MAX_HEADER_BYTES: usize = 65536;
const MAX_VARY_FIELDS: usize = 32;
const MAX_DELTA: u64 = 2_147_483_648;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheKind {
    Private,
    Shared,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheLimits {
    pub entries: usize,
    pub bytes: usize,
    pub entry_bytes: usize,
}
impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            entries: 64,
            bytes: 16 * 1024 * 1024,
            entry_bytes: 8 * 1024 * 1024,
        }
    }
}
impl CacheLimits {
    fn valid(self) -> bool {
        self.entries > 0
            && self.entries <= 64
            && self.bytes > 0
            && self.bytes <= 16 * 1024 * 1024
            && self.entry_bytes > 0
            && self.entry_bytes <= self.bytes / 2
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheError {
    Limits,
    TooLarge,
    InvalidHeader,
    InvalidTime,
}

/// `cookie` is the exact outbound Cookie field. The primary key intentionally
/// retains the shell's `input + "\\n" + cookie` shape, even without Vary: Cookie.
pub struct CacheRequest<'a> {
    pub uri: &'a str,
    pub method: &'a str,
    pub headers: &'a HeaderList,
    pub cookie: &'a str,
}
impl CacheRequest<'_> {
    pub fn legacy_key(&self) -> Result<String, CacheError> {
        let size = self
            .uri
            .len()
            .checked_add(self.cookie.len())
            .and_then(|n| n.checked_add(1))
            .ok_or(CacheError::TooLarge)?;
        if size > MAX_KEY_BYTES {
            return Err(CacheError::TooLarge);
        }
        let mut key = String::with_capacity(size);
        key.push_str(self.uri);
        key.push('\n');
        key.push_str(self.cookie);
        Ok(key)
    }
    fn field(&self, name: &str) -> Option<String> {
        if name == "cookie" && !self.cookie.is_empty() {
            Some(normalize_field(self.cookie))
        } else {
            self.headers.get(name).map(|v| normalize_field(&v))
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheResponse {
    /// Receipt from the stored response, never a later cache request.
    pub request_includes_credentials: bool,
    pub status: u16,
    pub headers: HeaderList,
    pub body: Vec<u8>,
}
/// Owned entry can be serialized by an embedding without prescribing its file format.
#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub uri: String,
    pub legacy_key: String,
    pub vary: Vec<(String, Option<String>)>,
    pub response: CacheResponse,
    pub request_time: i64,
    pub response_time: i64,
}
impl CacheEntry {
    pub fn current_age(&self, now: i64) -> u64 {
        let date = header_date(&self.response.headers, "date").unwrap_or(self.response_time);
        let apparent = self.response_time.saturating_sub(date).max(0) as u64;
        let age = delta(self.response.headers.get("age").as_deref()).unwrap_or(0);
        let delay = self.response_time.saturating_sub(self.request_time).max(0) as u64;
        apparent
            .max(age.saturating_add(delay))
            .saturating_add(now.saturating_sub(self.response_time).max(0) as u64)
            .min(MAX_DELTA)
    }
    pub fn freshness_lifetime(&self, kind: CacheKind) -> u64 {
        let cc = directives(&self.response.headers);
        if cc.invalid_freshness {
            return 0;
        }
        if kind == CacheKind::Shared {
            if let Some(value) = cc.s_maxage {
                return value;
            }
        }
        if let Some(value) = cc.max_age {
            return value;
        }
        let date = header_date(&self.response.headers, "date").unwrap_or(self.response_time);
        if self.response.headers.get("expires").is_some() {
            return header_date(&self.response.headers, "expires")
                .map(|expires| expires.saturating_sub(date).max(0) as u64)
                .unwrap_or(0);
        }
        if let Some(modified) = header_date(&self.response.headers, "last-modified") {
            return (date.saturating_sub(modified).max(0) as u64 / 10).min(86400);
        }
        0
    }
    /// A cache hit emits a fresh Age field and retains the stored body and status.
    pub fn response_at(&self, now: i64) -> CacheResponse {
        let mut response = self.response.clone();
        response.headers.remove("age");
        response
            .headers
            .append("Age", &self.current_age(now).to_string())
            .expect("valid Age");
        response
    }
}

/// Storage is deliberately transport and persistence agnostic. An embedding can
/// keep its own LRU or file format while the policy below owns all decisions.
pub trait CacheStorage {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&CacheEntry>;
    fn get_mut(&mut self, index: usize) -> Option<&mut CacheEntry>;
    fn remove(&mut self, index: usize) -> Option<CacheEntry>;
    fn push(&mut self, entry: CacheEntry);
}
#[derive(Default)]
pub struct MemoryCacheStorage(VecDeque<CacheEntry>);
impl CacheStorage for MemoryCacheStorage {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize) -> Option<&CacheEntry> {
        self.0.get(index)
    }
    fn get_mut(&mut self, index: usize) -> Option<&mut CacheEntry> {
        self.0.get_mut(index)
    }
    fn remove(&mut self, index: usize) -> Option<CacheEntry> {
        self.0.remove(index)
    }
    fn push(&mut self, entry: CacheEntry) {
        self.0.push_back(entry)
    }
}

pub enum CacheLookup<'a> {
    Miss,
    Fresh(&'a CacheEntry),
    Validate {
        entry: &'a CacheEntry,
        headers: HeaderList,
    },
}
pub struct CacheStore<S: CacheStorage = MemoryCacheStorage> {
    storage: S,
    kind: CacheKind,
    limits: CacheLimits,
    bytes: usize,
}
impl CacheStore<MemoryCacheStorage> {
    pub fn new(kind: CacheKind, limits: CacheLimits) -> Result<Self, CacheError> {
        Self::with_storage(MemoryCacheStorage::default(), kind, limits)
    }
}
impl<S: CacheStorage> CacheStore<S> {
    pub fn with_storage(
        storage: S,
        kind: CacheKind,
        limits: CacheLimits,
    ) -> Result<Self, CacheError> {
        if !limits.valid() || storage.len() > limits.entries {
            return Err(CacheError::Limits);
        }
        let mut bytes = 0usize;
        for i in 0..storage.len() {
            let entry = storage.get(i).ok_or(CacheError::Limits)?;
            if entry.uri.len() > MAX_KEY_BYTES
                || entry.legacy_key.len() > MAX_KEY_BYTES
                || headers_size(&entry.response.headers)? > MAX_HEADER_BYTES
                || entry.vary.len() > MAX_VARY_FIELDS
            {
                return Err(CacheError::TooLarge);
            }
            let size = entry_size(entry)?;
            if entry.response.body.len() > limits.entry_bytes {
                return Err(CacheError::TooLarge);
            }
            bytes = bytes.checked_add(size).ok_or(CacheError::TooLarge)?;
        }
        if bytes > limits.bytes {
            return Err(CacheError::TooLarge);
        }
        Ok(Self {
            storage,
            kind,
            limits,
            bytes,
        })
    }
    pub fn storage(&self) -> &S {
        &self.storage
    }
    pub fn into_storage(self) -> S {
        self.storage
    }
    pub fn len(&self) -> usize {
        self.storage.len()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    fn matching_index(&self, request: &CacheRequest<'_>, key: &str) -> Option<usize> {
        (0..self.storage.len()).rev().find(|&i| {
            self.storage.get(i).is_some_and(|entry| {
                entry.legacy_key == key
                    && entry
                        .vary
                        .iter()
                        .all(|(name, value)| request.field(name) == *value)
            })
        })
    }
    pub fn lookup(
        &self,
        request: &CacheRequest<'_>,
        now: i64,
    ) -> Result<CacheLookup<'_>, CacheError> {
        if !request.method.eq_ignore_ascii_case("GET") {
            return Ok(CacheLookup::Miss);
        }
        if headers_size(request.headers)? > MAX_HEADER_BYTES {
            return Err(CacheError::TooLarge);
        }
        let req_cc = directives(request.headers);
        if req_cc.no_store {
            return Ok(CacheLookup::Miss);
        }
        let key = request.legacy_key()?;
        let Some(index) = self.matching_index(request, &key) else {
            return Ok(CacheLookup::Miss);
        };
        let entry = self.storage.get(index).expect("matched entry");
        let cc = directives(&entry.response.headers);
        let age = entry.current_age(now);
        let lifetime = entry.freshness_lifetime(self.kind);
        let request_fresh = req_cc.max_age.is_none_or(|max| age < max);
        let min_fresh = req_cc.min_fresh.unwrap_or(0);
        let fresh = age.saturating_add(min_fresh) < lifetime;
        // max-stale lets a stale response through unless the response forbids it.
        let stale_ok = !cc.must_revalidate && req_cc.max_stale.is_some_and(|allowed| {
            allowed.is_none_or(|limit| age.saturating_sub(lifetime) <= limit)
        });
        if !req_cc.no_cache
            && !cc.no_cache
            && !cc.invalid_freshness
            && (fresh || stale_ok)
            && request_fresh
        {
            return Ok(CacheLookup::Fresh(entry));
        }
        if req_cc.only_if_cached {
            return Ok(CacheLookup::Miss);
        }
        let mut headers = HeaderList::default();
        if let Some(etag) = entry.response.headers.get("etag") {
            headers
                .append("If-None-Match", &etag)
                .map_err(|_| CacheError::InvalidHeader)?;
        }
        if let Some(modified) = entry.response.headers.get("last-modified") {
            headers
                .append("If-Modified-Since", &modified)
                .map_err(|_| CacheError::InvalidHeader)?;
        }
        Ok(CacheLookup::Validate { entry, headers })
    }
    /// Stored representation for the URL regardless of freshness (`force-cache`,
    /// `only-if-cached` request modes).
    pub fn lookup_stored(&self, request: &CacheRequest<'_>) -> Result<Option<&CacheEntry>, CacheError> {
        if !request.method.eq_ignore_ascii_case("GET") {
            return Ok(None);
        }
        let key = request.legacy_key()?;
        Ok(self.matching_index(request, &key).and_then(|index| self.storage.get(index)))
    }
    /// Takes an already bounded transport response. Rejects size before cache
    /// metadata copies or insertion; transport must enforce its own body cap.
    pub fn store(
        &mut self,
        request: &CacheRequest<'_>,
        response: CacheResponse,
        request_time: i64,
        response_time: i64,
    ) -> Result<bool, CacheError> {
        if request_time > response_time {
            return Err(CacheError::InvalidTime);
        }
        if !request.method.eq_ignore_ascii_case("GET") || !storable_status(&response) {
            return Ok(false);
        }
        if response.body.len() > self.limits.entry_bytes {
            return Err(CacheError::TooLarge);
        }
        let key_len = request
            .uri
            .len()
            .checked_add(request.cookie.len())
            .and_then(|n| n.checked_add(1))
            .ok_or(CacheError::TooLarge)?;
        if key_len > MAX_KEY_BYTES
            || headers_size(request.headers)? > MAX_HEADER_BYTES
            || headers_size(&response.headers)? > MAX_HEADER_BYTES
        {
            return Err(CacheError::TooLarge);
        }
        let req_cc = directives(request.headers);
        let cc = directives(&response.headers);
        if req_cc.no_store || cc.no_store || (self.kind == CacheKind::Shared && cc.private) {
            return Ok(false);
        }
        if self.kind == CacheKind::Shared
            && request.headers.get("authorization").is_some()
            && !(cc.public || cc.s_maxage.is_some() || cc.must_revalidate)
        {
            return Ok(false);
        }
        let names = vary_names(&response.headers)?;
        let Some(names) = names else {
            return Ok(false);
        };
        let mut vary = Vec::with_capacity(names.len());
        for name in names {
            vary.push((name.clone(), request.field(&name)));
        }
        let entry = CacheEntry {
            uri: request.uri.to_owned(),
            legacy_key: request.legacy_key()?,
            vary,
            response,
            request_time,
            response_time,
        };
        let size = entry_size(&entry)?;
        if size > self.limits.bytes {
            return Err(CacheError::TooLarge);
        }
        let mut i = 0;
        while i < self.storage.len() {
            if self
                .storage
                .get(i)
                .is_some_and(|old| old.legacy_key == entry.legacy_key && old.vary == entry.vary)
            {
                self.remove_at(i);
            } else {
                i += 1;
            }
        }
        while self.storage.len() >= self.limits.entries
            || size > self.limits.bytes.saturating_sub(self.bytes)
        {
            self.remove_at(0);
        }
        self.bytes += size;
        self.storage.push(entry);
        Ok(true)
    }
    /// Merge a successful 304 into the selected representation; retain its body.
    /// Returns the updated response, or None if validators do not identify it.
    pub fn update_304(
        &mut self,
        request: &CacheRequest<'_>,
        headers: &HeaderList,
        request_time: i64,
        response_time: i64,
    ) -> Result<Option<CacheResponse>, CacheError> {
        self.update_304_inner(request, headers, request_time, response_time, None)
    }
    /// A 304 answering a conditional request this cache itself issued for the
    /// selected entry: a validator-less 304 identifies it (RFC 9111 4.3.4) as long as
    /// the stored response was obtained with the same credentials mode.
    pub fn update_304_revalidated(
        &mut self,
        request: &CacheRequest<'_>,
        headers: &HeaderList,
        request_time: i64,
        response_time: i64,
        request_includes_credentials: bool,
    ) -> Result<Option<CacheResponse>, CacheError> {
        self.update_304_inner(request, headers, request_time, response_time, Some(request_includes_credentials))
    }
    fn update_304_inner(
        &mut self,
        request: &CacheRequest<'_>,
        headers: &HeaderList,
        request_time: i64,
        response_time: i64,
        revalidated_with: Option<bool>,
    ) -> Result<Option<CacheResponse>, CacheError> {
        let revalidated = revalidated_with.is_some();
        if request_time > response_time {
            return Err(CacheError::InvalidTime);
        }
        if !request.method.eq_ignore_ascii_case("GET") {
            return Ok(None);
        }
        if headers_size(request.headers)? > MAX_HEADER_BYTES
            || headers_size(headers)? > MAX_HEADER_BYTES
        {
            return Err(CacheError::TooLarge);
        }
        if directives(request.headers).no_store {
            return Ok(None);
        }
        let key = request.legacy_key()?;
        let Some(index) = self.matching_index(request, &key) else {
            return Ok(None);
        };
        let old = self.storage.get(index).expect("matched entry");
        if let Some(etag) = headers.get("etag") {
            if old.response.headers.get("etag").as_deref() != Some(etag.as_str()) {
                return Ok(None);
            }
        } else if let Some(modified) = headers.get("last-modified") {
            if old.response.headers.get("last-modified").as_deref() != Some(modified.as_str()) {
                return Ok(None);
            }
        } else if revalidated_with.is_some_and(|credentials| credentials != old.response.request_includes_credentials) {
            // A validator-less 304 may not cross credentials modes.
            return Ok(None);
        } else if (!revalidated && (old.response.headers.get("etag").is_some()
            || old.response.headers.get("last-modified").is_some()))
            || (0..self.storage.len())
                .filter(|&i| {
                    self.storage.get(i).is_some_and(|candidate| {
                        candidate.legacy_key == key
                            && candidate
                                .vary
                                .iter()
                                .all(|(name, value)| request.field(name) == *value)
                    })
                })
                .count()
                != 1
        {
            return Ok(None);
        }
        if let Some(length) = headers.get("content-length") {
            // A 304 has no body; `Content-Length: 0` (or the 200 length) is not a mismatch.
            if length != "0" && length.parse::<usize>().ok() != Some(old.response.body.len()) {
                return Ok(None);
            }
        }
        let mut merged = old.response.headers.clone();
        let mut replaced = Vec::<String>::new();
        for (name, value) in headers.iter() {
            if matches!(name, "connection" | "transfer-encoding" | "content-length") {
                continue;
            }
            if !replaced.iter().any(|old| old == name) {
                merged.remove(name);
                replaced.push(name.to_owned());
            }
            merged
                .append(name, value)
                .map_err(|_| CacheError::InvalidHeader)?;
        }
        if headers_size(&merged)? > MAX_HEADER_BYTES {
            return Err(CacheError::TooLarge);
        }
        let cc = directives(&merged);
        let names = vary_names(&merged)?;
        let keep =
            !cc.no_store && !(self.kind == CacheKind::Shared && cc.private) && names.is_some();
        let mut vary = Vec::with_capacity(names.as_ref().map_or(0, Vec::len));
        if let Some(names) = names {
            for name in names {
                vary.push((name.clone(), request.field(&name)));
            }
        }
        let mut entry = self.storage.remove(index).expect("entry");
        self.bytes = self.bytes.saturating_sub(entry_size(&entry)?);
        entry.response.headers = merged;
        entry.vary = vary;
        entry.request_time = request_time;
        entry.response_time = response_time;
        let new_size = entry_size(&entry)?;
        if !keep || new_size > self.limits.bytes {
            return Ok(Some(entry.response_at(response_time)));
        }
        while self.storage.len() >= self.limits.entries
            || new_size > self.limits.bytes.saturating_sub(self.bytes)
        {
            self.remove_at(0);
        }
        let response = entry.response_at(response_time);
        self.bytes += new_size;
        self.storage.push(entry);
        Ok(Some(response))
    }
    /// A successful unsafe request invalidates every variant of its target URI.
    pub fn invalidate_unsafe(&mut self, method: &str, uri: &str, status: u16) {
        if matches!(
            method.to_ascii_uppercase().as_str(),
            "GET" | "HEAD" | "OPTIONS" | "TRACE"
        ) || !(200..400).contains(&status)
        {
            return;
        }
        let mut i = 0;
        while i < self.storage.len() {
            if self.storage.get(i).is_some_and(|entry| entry.uri == uri) {
                self.remove_at(i);
            } else {
                i += 1;
            }
        }
    }
    fn remove_at(&mut self, index: usize) {
        if let Some(entry) = self.storage.remove(index) {
            self.bytes = self.bytes.saturating_sub(entry_size(&entry).unwrap_or(0));
        }
    }
}

/// True when the request asks for `Cache-Control: only-if-cached`.
pub fn request_only_if_cached(headers: &HeaderList) -> bool {
    directives(headers).only_if_cached
}

/// Canonical reason phrase for a stored status (the cache keeps no status text).
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK", 201 => "Created", 202 => "Accepted", 203 => "Non-Authoritative Information",
        204 => "No Content", 205 => "Reset Content", 206 => "Partial Content",
        300 => "Multiple Choices", 301 => "Moved Permanently", 302 => "Found", 303 => "See Other",
        304 => "Not Modified", 307 => "Temporary Redirect", 308 => "Permanent Redirect",
        400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found",
        405 => "Method Not Allowed", 410 => "Gone", 414 => "URI Too Long",
        500 => "Internal Server Error", 501 => "Not Implemented", 502 => "Bad Gateway",
        503 => "Service Unavailable", 504 => "Gateway Timeout",
        _ => "",
    }
}

fn storable_status(response: &CacheResponse) -> bool {
    if !(200..=599).contains(&response.status) || matches!(response.status, 206 | 304) {
        return false;
    }
    if matches!(response.status, 200 | 203 | 204 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501) {
        return true;
    }
    let cc = directives(&response.headers);
    cc.max_age.is_some() || cc.s_maxage.is_some()
        || header_date(&response.headers, "expires").is_some()
}

#[derive(Default)]
struct Directives {
    max_age: Option<u64>,
    s_maxage: Option<u64>,
    no_store: bool,
    no_cache: bool,
    must_revalidate: bool,
    private: bool,
    public: bool,
    invalid_freshness: bool,
    min_fresh: Option<u64>,
    max_stale: Option<Option<u64>>,
    only_if_cached: bool,
}
fn directives(headers: &HeaderList) -> Directives {
    let mut out = Directives::default();
    let Some(value) = headers.get("cache-control") else {
        return out;
    };
    for raw in value.split(',') {
        let (name, argument) = raw
            .trim()
            .split_once('=')
            .map_or((raw.trim(), None), |(a, b)| {
                (a.trim(), Some(b.trim().trim_matches('"')))
            });
        let number = || argument.and_then(|v| delta(Some(v)));
        match name.to_ascii_lowercase().as_str() {
            "max-age" => {
                if out.max_age.is_some() {
                    out.invalid_freshness = true;
                }
                out.max_age = number();
                if out.max_age.is_none() {
                    out.invalid_freshness = true;
                }
            }
            "s-maxage" => {
                if out.s_maxage.is_some() {
                    out.invalid_freshness = true;
                }
                out.s_maxage = number();
                if out.s_maxage.is_none() {
                    out.invalid_freshness = true;
                }
            }
            "no-store" => out.no_store = true,
            "no-cache" => out.no_cache = true,
            "must-revalidate" => out.must_revalidate = true,
            "private" => out.private = true,
            "public" => out.public = true,
            "min-fresh" => out.min_fresh = number(),
            "max-stale" => out.max_stale = Some(number()),
            "only-if-cached" => out.only_if_cached = true,
            _ => {}
        }
    }
    out
}
fn delta(value: Option<&str>) -> Option<u64> {
    let value = value?.trim().split(',').next()?.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(value.parse::<u64>().unwrap_or(MAX_DELTA).min(MAX_DELTA))
}
fn header_date(headers: &HeaderList, name: &str) -> Option<i64> {
    let value = headers.get(name)?;
    if value.len() > 128 || !value.trim().to_ascii_lowercase().ends_with("gmt") {
        return None;
    }
    parse_date(value.as_bytes())
}
fn headers_size(headers: &HeaderList) -> Result<usize, CacheError> {
    headers
        .iter()
        .try_fold(0usize, |sum, (name, value)| {
            sum.checked_add(name.len())?
                .checked_add(value.len())?
                .checked_add(4)
        })
        .ok_or(CacheError::TooLarge)
}
fn entry_size(entry: &CacheEntry) -> Result<usize, CacheError> {
    let mut size = entry
        .uri
        .len()
        .checked_add(entry.legacy_key.len())
        .and_then(|n| n.checked_add(entry.response.body.len()))
        .and_then(|n| n.checked_add(headers_size(&entry.response.headers).ok()?))
        .ok_or(CacheError::TooLarge)?;
    for (name, value) in &entry.vary {
        size = size
            .checked_add(name.len())
            .and_then(|n| n.checked_add(value.as_ref().map_or(0, String::len)))
            .ok_or(CacheError::TooLarge)?;
    }
    Ok(size)
}
fn normalize_field(value: &str) -> String {
    value.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
fn vary_names(headers: &HeaderList) -> Result<Option<Vec<String>>, CacheError> {
    let Some(value) = headers.get("vary") else {
        return Ok(Some(Vec::new()));
    };
    let mut names = Vec::new();
    for field in value.split(',') {
        let name = field.trim().to_ascii_lowercase();
        if name == "*" {
            return Ok(None);
        }
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        {
            return Err(CacheError::InvalidHeader);
        }
        if !names.contains(&name) {
            names.push(name);
        }
        if names.len() > MAX_VARY_FIELDS {
            return Err(CacheError::TooLarge);
        }
    }
    Ok(Some(names))
}
