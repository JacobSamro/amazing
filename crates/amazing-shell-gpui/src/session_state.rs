use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use amazing_fetch::HeaderList;
use amazing_fetch::cache::{
    CacheEntry, CacheKind, CacheLimits, CacheLookup, CacheRequest, CacheResponse, CacheStorage,
    CacheStore,
};
use amazing_net::{FetchOptions, Headers, NetError, Response, Url};

const MAX_CACHE_ENTRIES: usize = 64;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HISTORY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    persistent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRecord {
    pub name: String,
    pub path: String,
    pub url: String,
}

#[derive(Clone, Debug)]
struct CachedResponseMeta {
    legacy_key: String,
    vary: Vec<(String, Option<String>)>,
    url: String,
    reason: String,
}

pub struct BrowserSession {
    state_path: Option<PathBuf>,
    cookies: Vec<Cookie>,
    cache: CacheStore,
    cache_meta: VecDeque<CachedResponseMeta>,
    storage: BTreeMap<(String, String), String>,
    history: Vec<String>,
    bookmarks: Vec<String>,
    downloads: Vec<DownloadRecord>,
}

impl std::fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession")
            .field("state_path", &self.state_path)
            .field("cookies", &self.cookies)
            .field("cache_entries", &self.cache.len())
            .field("cache_bytes", &self.cache.bytes())
            .field("storage", &self.storage)
            .field("history", &self.history)
            .field("bookmarks", &self.bookmarks)
            .field("downloads", &self.downloads)
            .finish()
    }
}

impl BrowserSession {
    pub fn load_default() -> Self {
        let path = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Amazing")
            .join("session-state-v1.txt");
        Self::load(Some(path))
    }

    pub fn ephemeral() -> Self {
        Self::load(None)
    }

    fn load(state_path: Option<PathBuf>) -> Self {
        let mut session = Self {
            state_path,
            cookies: Vec::new(),
            cache: new_shell_cache(),
            cache_meta: VecDeque::new(),
            storage: BTreeMap::new(),
            history: Vec::new(),
            bookmarks: Vec::new(),
            downloads: Vec::new(),
        };
        let Some(path) = session.state_path.as_ref() else {
            return session;
        };
        let Ok(source) = std::fs::read_to_string(path) else {
            return session;
        };
        for line in source.lines() {
            let fields = line.split('\t').collect::<Vec<_>>();
            match fields.as_slice() {
                ["C", name, value, domain, path, secure] => {
                    if let (Some(name), Some(value), Some(domain), Some(path)) =
                        (decode(name), decode(value), decode(domain), decode(path))
                    {
                        session.cookies.push(Cookie {
                            name,
                            value,
                            domain,
                            path,
                            secure: *secure == "1",
                            persistent: true,
                        });
                    }
                }
                ["L", origin, key, value] => {
                    if let (Some(origin), Some(key), Some(value)) =
                        (decode(origin), decode(key), decode(value))
                    {
                        session.storage.insert((origin, key), value);
                    }
                }
                ["H", url] => {
                    if let Some(url) = decode(url) {
                        session.history.push(url);
                    }
                }
                ["B", url] => {
                    if let Some(url) = decode(url) {
                        session.bookmarks.push(url);
                    }
                }
                ["D", name, path, url] => {
                    if let (Some(name), Some(path), Some(url)) =
                        (decode(name), decode(path), decode(url))
                    {
                        session.downloads.push(DownloadRecord { name, path, url });
                    }
                }
                _ => {}
            }
        }
        session.history.truncate(MAX_HISTORY);
        session
    }

    pub fn fetch(
        &mut self,
        input: &str,
        options: &FetchOptions,
    ) -> Result<Response, amazing_net::NetError> {
        let parsed = Url::parse(input).ok();
        let cookie = parsed
            .as_ref()
            .map(|url| self.cookie_header(url))
            .unwrap_or_default();
        let mut request_options = options.clone();
        if !cookie.is_empty() {
            request_options
                .request_headers
                .push(("Cookie".into(), cookie.clone()));
        }
        let request_headers = cache_request_headers(&request_options);
        let cache_request = CacheRequest {
            uri: input,
            method: &options.method,
            headers: &request_headers,
            cookie: &cookie,
        };
        let mut prior_meta = None;
        let mut conditional = None;
        match self.cache.lookup(&cache_request, unix_seconds()) {
            Ok(CacheLookup::Fresh(entry)) => {
                let meta = self
                    .meta_for(entry)
                    .ok_or_else(|| {
                        NetError::MalformedResponse("cached response metadata missing".into())
                    })?
                    .clone();
                let legacy_key = entry.legacy_key.clone();
                let vary = entry.vary.clone();
                let response = cached_net_response(&meta, entry.response_at(unix_seconds()))?;
                self.promote_cache_entry(&legacy_key, &vary);
                return Ok(response);
            }
            Ok(CacheLookup::Validate { entry, headers }) => {
                prior_meta = Some(
                    self.meta_for(entry)
                        .ok_or_else(|| {
                            NetError::MalformedResponse("cached response metadata missing".into())
                        })?
                        .clone(),
                );
                conditional = Some(headers);
            }
            Ok(CacheLookup::Miss) | Err(_) => {} // An out-of-bounds cache key never blocks transport.
        }
        if let Some(headers) = conditional.as_ref() {
            for (name, value) in headers.iter() {
                request_options
                    .request_headers
                    .push((name.to_owned(), value.to_owned()));
            }
        }
        let request_time = unix_seconds();
        let response = amazing_net::fetch(input, &request_options)?;
        let response_time = unix_seconds().max(request_time);
        if let Ok(final_url) = Url::parse(&response.url) {
            for header in response.headers.get_all("set-cookie") {
                self.store_cookie(&final_url, header);
            }
        }
        if response.status == 304 && conditional.is_some() {
            if let (Some(meta), Some(headers)) = (prior_meta, cache_response_headers(&response)) {
                let updated = self
                    .cache
                    .update_304(&cache_request, &headers, request_time, response_time)
                    .map_err(|error| {
                        NetError::MalformedResponse(format!("cache 304 update: {error:?}"))
                    })?;
                if let Some(updated) = updated {
                    if let Ok(CacheLookup::Fresh(entry) | CacheLookup::Validate { entry, .. }) =
                        self.cache.lookup(&cache_request, response_time)
                    {
                        self.record_cache_meta(
                            entry.legacy_key.clone(),
                            entry.vary.clone(),
                            meta.url.clone(),
                            meta.reason.clone(),
                        );
                    }
                    self.prune_cache_meta();
                    self.persist();
                    return cached_net_response(&meta, updated);
                }
            }
        }
        if options.method.eq_ignore_ascii_case("GET")
            && response.status == 200
            && response.body.len() <= MAX_CACHE_BYTES / 2
            && let Some(headers) = cache_response_headers(&response)
        {
            let candidate = CacheResponse {
                status: response.status,
                headers,
                body: response.body.clone(),
            };
            if matches!(
                self.cache
                    .store(&cache_request, candidate, request_time, response_time),
                Ok(true)
            ) {
                if let Some(entry) = self.cache.storage().get(self.cache.len() - 1) {
                    self.record_cache_meta(
                        entry.legacy_key.clone(),
                        entry.vary.clone(),
                        response.url.clone(),
                        response.reason.clone(),
                    );
                }
            }
        }
        self.cache
            .invalidate_unsafe(&options.method, input, response.status);
        self.prune_cache_meta();
        self.persist();
        Ok(response)
    }

    pub fn storage_get(&self, origin: &str, key: &str) -> Option<String> {
        self.storage
            .get(&(origin.to_string(), key.to_string()))
            .cloned()
    }

    pub fn storage_set(&mut self, origin: &str, key: &str, value: &str) {
        self.storage
            .insert((origin.to_string(), key.to_string()), value.to_string());
        self.persist();
    }

    pub fn storage_remove(&mut self, origin: &str, key: &str) {
        self.storage.remove(&(origin.to_string(), key.to_string()));
        self.persist();
    }

    pub fn storage_clear(&mut self, origin: &str) {
        self.storage
            .retain(|(stored_origin, _), _| stored_origin != origin);
        self.persist();
    }

    pub fn record_history(&mut self, url: &str) {
        if self.history.last().is_none_or(|last| last != url) {
            self.history.push(url.to_string());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
            self.persist();
        }
    }

    pub fn set_bookmarked(&mut self, url: &str, bookmarked: bool) {
        self.bookmarks.retain(|entry| entry != url);
        if bookmarked {
            self.bookmarks.push(url.to_string());
        }
        self.persist();
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|entry| entry == url)
    }

    pub fn save_download(&mut self, response: &Response) -> Result<DownloadRecord, String> {
        let directory = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
            .join("Amazing");
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let suggested = response
            .headers
            .get("content-disposition")
            .and_then(filename_from_disposition)
            .or_else(|| response.url.split('/').next_back().map(str::to_string))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "download.bin".into());
        let suggested = sanitize_filename(&suggested);
        let path = unique_path(&directory, &suggested);
        std::fs::write(&path, &response.body).map_err(|error| error.to_string())?;
        let record = DownloadRecord {
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("download.bin")
                .to_string(),
            path: path.to_string_lossy().into_owned(),
            url: response.url.clone(),
        };
        self.downloads.push(record.clone());
        self.persist();
        Ok(record)
    }

    pub fn downloads(&self) -> &[DownloadRecord] {
        &self.downloads
    }

    fn meta_for(&self, entry: &CacheEntry) -> Option<&CachedResponseMeta> {
        self.cache_meta
            .iter()
            .find(|meta| meta.legacy_key == entry.legacy_key && meta.vary == entry.vary)
    }

    fn record_cache_meta(
        &mut self,
        legacy_key: String,
        vary: Vec<(String, Option<String>)>,
        url: String,
        reason: String,
    ) {
        self.cache_meta
            .retain(|meta| meta.legacy_key != legacy_key || meta.vary != vary);
        self.cache_meta.push_back(CachedResponseMeta {
            legacy_key,
            vary,
            url,
            reason,
        });
    }

    fn prune_cache_meta(&mut self) {
        self.cache_meta.retain(|meta| {
            (0..self.cache.len()).any(|index| {
                self.cache.storage().get(index).is_some_and(|entry| {
                    entry.legacy_key == meta.legacy_key && entry.vary == meta.vary
                })
            })
        });
    }

    fn promote_cache_entry(&mut self, legacy_key: &str, vary: &[(String, Option<String>)]) {
        let Some(index) = (0..self.cache.len()).find(|&index| {
            self.cache
                .storage()
                .get(index)
                .is_some_and(|entry| entry.legacy_key == legacy_key && entry.vary == vary)
        }) else {
            return;
        };
        let prior = std::mem::replace(&mut self.cache, new_shell_cache());
        let mut storage = prior.into_storage();
        let entry = storage.remove(index).expect("matched cache entry");
        storage.push(entry);
        self.cache = CacheStore::with_storage(storage, CacheKind::Private, shell_cache_limits())
            .expect("existing bounded cache storage");
    }

    fn cookie_header(&self, url: &Url) -> String {
        self.cookies
            .iter()
            .filter(|cookie| {
                (!cookie.secure || url.scheme == "https")
                    && (url.host == cookie.domain
                        || url.host.ends_with(&format!(".{}", cookie.domain)))
                    && url.path.starts_with(&cookie.path)
            })
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn store_cookie(&mut self, url: &Url, header: &str) {
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|pair| pair.trim().split_once('=')) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let mut cookie = Cookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: url.host.to_ascii_lowercase(),
            path: default_cookie_path(&url.path),
            secure: false,
            persistent: false,
        };
        let mut delete = false;
        for attribute in parts {
            let (name, value) = attribute
                .trim()
                .split_once('=')
                .map_or((attribute.trim(), ""), |(name, value)| {
                    (name.trim(), value.trim())
                });
            match name.to_ascii_lowercase().as_str() {
                "domain" if !value.is_empty() => {
                    let domain = value.trim_start_matches('.').to_ascii_lowercase();
                    if url.host == domain || url.host.ends_with(&format!(".{domain}")) {
                        cookie.domain = domain;
                    }
                }
                "path" if value.starts_with('/') => cookie.path = value.to_string(),
                "secure" => cookie.secure = true,
                "max-age" => {
                    cookie.persistent = true;
                    delete = value.parse::<i64>().is_ok_and(|seconds| seconds <= 0);
                }
                _ => {}
            }
        }
        self.cookies.retain(|existing| {
            !(existing.name == cookie.name
                && existing.domain == cookie.domain
                && existing.path == cookie.path)
        });
        if !delete {
            self.cookies.push(cookie);
        }
    }

    fn persist(&self) {
        let Some(path) = self.state_path.as_ref() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut output = String::new();
        for cookie in self.cookies.iter().filter(|cookie| cookie.persistent) {
            output.push_str(&format!(
                "C\t{}\t{}\t{}\t{}\t{}\n",
                encode(&cookie.name),
                encode(&cookie.value),
                encode(&cookie.domain),
                encode(&cookie.path),
                u8::from(cookie.secure)
            ));
        }
        for ((origin, key), value) in &self.storage {
            output.push_str(&format!(
                "L\t{}\t{}\t{}\n",
                encode(origin),
                encode(key),
                encode(value)
            ));
        }
        for url in &self.history {
            output.push_str(&format!("H\t{}\n", encode(url)));
        }
        for url in &self.bookmarks {
            output.push_str(&format!("B\t{}\n", encode(url)));
        }
        for download in &self.downloads {
            output.push_str(&format!(
                "D\t{}\t{}\t{}\n",
                encode(&download.name),
                encode(&download.path),
                encode(&download.url)
            ));
        }
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, output).is_ok() {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::rename(temporary, path);
        }
    }
}

fn shell_cache_limits() -> CacheLimits {
    CacheLimits {
        entries: MAX_CACHE_ENTRIES,
        bytes: MAX_CACHE_BYTES,
        entry_bytes: MAX_CACHE_BYTES / 2,
    }
}

fn new_shell_cache() -> CacheStore {
    CacheStore::new(CacheKind::Private, shell_cache_limits())
        .expect("fixed shell cache limits are valid")
}

fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn cache_request_headers(options: &FetchOptions) -> HeaderList {
    let mut headers = HeaderList::default();
    for (name, value) in &options.request_headers {
        // amazing-net also ignores caller fields containing HTTP controls.
        let _ = headers.append(name, value);
    }
    headers
}

fn cache_response_headers(response: &Response) -> Option<HeaderList> {
    let mut headers = HeaderList::default();
    for (name, value) in response.headers.iter() {
        headers.append(name, value).ok()?;
    }
    Some(headers)
}

fn cached_net_response(
    meta: &CachedResponseMeta,
    cached: CacheResponse,
) -> Result<Response, NetError> {
    let mut headers = Headers::new();
    for (name, value) in cached.headers.iter() {
        headers.append(name, value.to_owned()).map_err(|error| {
            NetError::MalformedResponse(format!("cached header {name}: {error:?}"))
        })?;
    }
    Ok(Response {
        url: meta.url.clone(),
        status: cached.status,
        reason: meta.reason.clone(),
        headers,
        body: cached.body,
    })
}

fn default_cookie_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or("/")
        .to_string()
}

fn filename_from_disposition(header: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        name.eq_ignore_ascii_case("filename")
            .then(|| value.trim_matches(['"', '\'']).to_string())
    })
}

fn sanitize_filename(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "download.bin".into()
    } else {
        sanitized
    }
}

fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let initial = directory.join(name);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|extension| extension.to_str());
    for suffix in 1..10_000 {
        let candidate = match extension {
            Some(extension) => directory.join(format!("{stem}-{suffix}.{extension}")),
            None => directory.join(format!("{stem}-{suffix}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-overflow"))
}

fn encode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn decode(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn persistent_state_round_trips_storage_history_and_bookmarks() {
        let path = std::env::temp_dir().join(format!(
            "amazing-session-{}-{}.txt",
            std::process::id(),
            line!()
        ));
        let mut session = BrowserSession::load(Some(path.clone()));
        session.storage_set("https://example.test", "theme", "dark");
        session.record_history("https://example.test/page");
        session.set_bookmarked("https://example.test/page", true);
        drop(session);

        let restored = BrowserSession::load(Some(path.clone()));
        assert_eq!(
            restored
                .storage_get("https://example.test", "theme")
                .as_deref(),
            Some("dark")
        );
        assert_eq!(restored.history, ["https://example.test/page"]);
        assert!(restored.is_bookmarked("https://example.test/page"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cookie_matching_respects_domain_path_and_secure() {
        let mut session = BrowserSession::ephemeral();
        let url = Url::parse("https://accounts.example.test/login/start").unwrap();
        session.store_cookie(&url, "sid=abc; Path=/login; Secure; Max-Age=3600");
        assert_eq!(session.cookie_header(&url), "sid=abc");
        assert!(
            session
                .cookie_header(&Url::parse("http://accounts.example.test/login").unwrap())
                .is_empty()
        );
        assert!(
            session
                .cookie_header(&Url::parse("https://accounts.example.test/other").unwrap())
                .is_empty()
        );
    }

    #[test]
    fn download_names_are_sanitized_and_unique() {
        assert_eq!(sanitize_filename("../bad name?.txt"), ".._bad_name_.txt");
        assert_eq!(
            filename_from_disposition("attachment; filename=report.pdf").as_deref(),
            Some("report.pdf")
        );
    }

    #[test]
    fn set_cookie_is_sent_on_the_next_matching_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 2048];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    bytes.extend_from_slice(&buffer[..count]);
                    if count == 0 || bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                sender.send(bytes).unwrap();
                let extra = if index == 0 {
                    "Set-Cookie: sid=abc; Path=/; Max-Age=3600\r\n"
                } else {
                    ""
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\n{extra}Cache-Control: no-store\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                )
                .unwrap();
            }
        });
        let url = format!("http://127.0.0.1:{port}/cookie");
        let mut session = BrowserSession::ephemeral();
        session.fetch(&url, &FetchOptions::default()).unwrap();
        session.fetch(&url, &FetchOptions::default()).unwrap();
        server.join().unwrap();
        let first = String::from_utf8(receiver.recv().unwrap()).unwrap();
        let second = String::from_utf8(receiver.recv().unwrap()).unwrap();
        assert!(!first.contains("Cookie:"));
        assert!(second.contains("Cookie: sid=abc\r\n"));
    }

    #[test]
    fn cache_hit_reuses_response_without_second_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut stream = (0..100)
                .find_map(|_| match listener.accept() {
                    Ok((stream, _)) => Some(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    }
                    Err(error) => panic!("cache fixture accept: {error}"),
                })
                .unwrap();
            stream.set_nonblocking(false).unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
            );
        });
        let url = format!("http://127.0.0.1:{port}/cached");
        let mut session = BrowserSession::ephemeral();
        let first = session.fetch(&url, &FetchOptions::default()).unwrap();
        let second = session.fetch(&url, &FetchOptions::default()).unwrap();
        assert_eq!(first.url, second.url);
        assert_eq!(first.status, second.status);
        assert_eq!(first.body, second.body);
        let age = second.headers.get("age").unwrap().parse::<u64>().unwrap();
        assert!(age < 60, "fresh cache hit has age {age}");
        server.join().unwrap();
    }

    #[test]
    fn stale_etag_revalidates_and_merges_304() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "missing conditional request"
                            );
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("revalidation accept: {error}"),
                    }
                };
                let mut request = Vec::new();
                let mut bytes = [0; 2048];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut bytes).unwrap();
                    assert!(count > 0, "incomplete request");
                    request.extend_from_slice(&bytes[..count]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                if index == 0 {
                    assert!(!request.contains("if-none-match:"));
                    stream.write_all(b"HTTP/1.1 200 OK\r\nCache-Control: no-cache\r\nETag: \"v1\"\r\nContent-Type: text/plain\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncached").unwrap();
                } else {
                    assert!(request.contains("if-none-match: \"v1\"\r\n"), "{request}");
                    stream.write_all(b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n").unwrap();
                }
            }
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
                "third fetch must use renewed cache entry"
            );
        });
        let url = format!("http://127.0.0.1:{port}/revalidate");
        let mut session = BrowserSession::ephemeral();
        let first = session.fetch(&url, &FetchOptions::default()).unwrap();
        assert_eq!(first.body, b"cached");
        let second = session.fetch(&url, &FetchOptions::default()).unwrap();
        assert_eq!(second.status, 200);
        assert_eq!(second.body, b"cached");
        assert_eq!(second.headers.get("cache-control"), Some("max-age=60"));
        assert_eq!(second.headers.get("content-type"), Some("text/plain"));
        let third = session.fetch(&url, &FetchOptions::default()).unwrap();
        assert_eq!(third.body, b"cached");
        server.join().unwrap();
    }

    #[test]
    fn cache_partitions_on_cookie_and_vary_field() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for index in 0..3 {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "missing partitioned request"
                            );
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("partition accept: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 2048];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut bytes).unwrap();
                    assert!(count > 0, "incomplete partition request");
                    request.extend_from_slice(&bytes[..count]);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                let (extra, body) = match index {
                    0 => {
                        assert!(!request.contains("cookie:"));
                        assert!(request.contains("accept-language: en"));
                        ("Set-Cookie: sid=one; Path=/; Max-Age=3600\r\n", "plain")
                    }
                    1 => {
                        assert!(request.contains("cookie: sid=one"));
                        assert!(request.contains("accept-language: en"));
                        ("", "english")
                    }
                    _ => {
                        assert!(request.contains("cookie: sid=one"));
                        assert!(request.contains("accept-language: fr"));
                        ("", "french")
                    }
                };
                write!(stream, "HTTP/1.1 200 OK\r\n{extra}Cache-Control: max-age=60\r\nVary: Accept-Language\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
            );
        });
        let url = format!("http://127.0.0.1:{port}/vary");
        let mut session = BrowserSession::ephemeral();
        let mut english = FetchOptions::default();
        english
            .request_headers
            .push(("Accept-Language".into(), "en".into()));
        let mut french = FetchOptions::default();
        french
            .request_headers
            .push(("Accept-Language".into(), "fr".into()));
        assert_eq!(session.fetch(&url, &english).unwrap().body, b"plain");
        assert_eq!(session.fetch(&url, &english).unwrap().body, b"english");
        assert_eq!(session.fetch(&url, &french).unwrap().body, b"french");
        assert_eq!(session.fetch(&url, &french).unwrap().body, b"french");
        assert_eq!(session.fetch(&url, &english).unwrap().body, b"english");
        server.join().unwrap();
    }

    #[test]
    fn session_state_v1_keeps_exact_records_and_never_serializes_cache() {
        let directory = std::env::temp_dir().join(format!(
            "amazing-cache-v1-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session-state-v1.txt");
        let page = directory.join("page.html");
        std::fs::write(&page, b"<p>cached</p>").unwrap();
        let mut session = BrowserSession::load(Some(path.clone()));
        let url = Url::parse("https://example.test/path").unwrap();
        session.store_cookie(&url, "sid=one; Path=/; Max-Age=3600");
        session.storage_set("https://example.test", "theme", "dark");
        session.record_history("https://example.test/path");
        session.set_bookmarked("https://example.test/path", true);
        session.downloads.push(DownloadRecord {
            name: "note.txt".into(),
            path: "C:\\Downloads\\note.txt".into(),
            url: "https://example.test/note".into(),
        });
        session.persist();
        let before = std::fs::read(&path).unwrap();
        let expected = format!(
            "C\t{}\t{}\t{}\t{}\t0\nL\t{}\t{}\t{}\nH\t{}\nB\t{}\nD\t{}\t{}\t{}\n",
            encode("sid"),
            encode("one"),
            encode("example.test"),
            encode("/"),
            encode("https://example.test"),
            encode("theme"),
            encode("dark"),
            encode("https://example.test/path"),
            encode("https://example.test/path"),
            encode("note.txt"),
            encode("C:\\Downloads\\note.txt"),
            encode("https://example.test/note")
        );
        assert_eq!(before, expected.as_bytes());
        session
            .fetch(page.to_str().unwrap(), &FetchOptions::default())
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(session);
        let restored = BrowserSession::load(Some(path.clone()));
        assert_eq!(
            restored
                .storage_get("https://example.test", "theme")
                .as_deref(),
            Some("dark")
        );
        assert_eq!(restored.cookie_header(&url), "sid=one");
        assert_eq!(restored.downloads.len(), 1);
        assert_eq!(restored.cache.len(), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
