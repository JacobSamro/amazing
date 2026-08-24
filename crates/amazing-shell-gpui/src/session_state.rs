use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use amazing_net::{FetchOptions, Response, Url};

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

#[derive(Debug)]
pub struct BrowserSession {
    state_path: Option<PathBuf>,
    cookies: Vec<Cookie>,
    cache: VecDeque<(String, Response)>,
    cache_bytes: usize,
    storage: BTreeMap<(String, String), String>,
    history: Vec<String>,
    bookmarks: Vec<String>,
    downloads: Vec<DownloadRecord>,
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
            cache: VecDeque::new(),
            cache_bytes: 0,
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
                    if let (Some(name), Some(value), Some(domain), Some(path)) = (
                        decode(name),
                        decode(value),
                        decode(domain),
                        decode(path),
                    ) {
                        session.cookies.push(Cookie {
                            name,
                            value,
                            domain,
                            path,
                            secure: *secure == "1",
                            persistent: true,
                        });
                    }
                },
                ["L", origin, key, value] => {
                    if let (Some(origin), Some(key), Some(value)) =
                        (decode(origin), decode(key), decode(value))
                    {
                        session.storage.insert((origin, key), value);
                    }
                },
                ["H", url] => {
                    if let Some(url) = decode(url) {
                        session.history.push(url);
                    }
                },
                ["B", url] => {
                    if let Some(url) = decode(url) {
                        session.bookmarks.push(url);
                    }
                },
                ["D", name, path, url] => {
                    if let (Some(name), Some(path), Some(url)) =
                        (decode(name), decode(path), decode(url))
                    {
                        session.downloads.push(DownloadRecord { name, path, url });
                    }
                },
                _ => {},
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
        let cookie = parsed.as_ref().map(|url| self.cookie_header(url));
        let cacheable_method = options.method.eq_ignore_ascii_case("GET");
        let cache_key = format!("{input}\n{}", cookie.as_deref().unwrap_or(""));
        if cacheable_method
            && let Some(position) = self.cache.iter().position(|(key, _)| key == &cache_key)
        {
            let entry = self.cache.remove(position).expect("cache position");
            let response = entry.1.clone();
            self.cache.push_back(entry);
            return Ok(response);
        }

        let mut options = options.clone();
        if let Some(cookie) = cookie.filter(|cookie| !cookie.is_empty()) {
            options.request_headers.push(("Cookie".into(), cookie));
        }
        let response = amazing_net::fetch(input, &options)?;
        if let Ok(final_url) = Url::parse(&response.url) {
            for header in response.headers.get_all("set-cookie") {
                self.store_cookie(&final_url, header);
            }
        }
        let cache_control = response
            .headers
            .get("cache-control")
            .unwrap_or("")
            .to_ascii_lowercase();
        if cacheable_method
            && response.status == 200
            && !cache_control.contains("no-store")
            && response.body.len() <= MAX_CACHE_BYTES / 2
        {
            self.insert_cache(cache_key, response.clone());
        }
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
        self.storage.retain(|(stored_origin, _), _| stored_origin != origin);
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

    fn insert_cache(&mut self, key: String, response: Response) {
        let size = response.body.len();
        self.cache.push_back((key, response));
        self.cache_bytes += size;
        while self.cache.len() > MAX_CACHE_ENTRIES || self.cache_bytes > MAX_CACHE_BYTES {
            if let Some((_, removed)) = self.cache.pop_front() {
                self.cache_bytes = self.cache_bytes.saturating_sub(removed.body.len());
            }
        }
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
                .map_or((attribute.trim(), ""), |(name, value)| (name.trim(), value.trim()));
            match name.to_ascii_lowercase().as_str() {
                "domain" if !value.is_empty() => {
                    let domain = value.trim_start_matches('.').to_ascii_lowercase();
                    if url.host == domain || url.host.ends_with(&format!(".{domain}")) {
                        cookie.domain = domain;
                    }
                },
                "path" if value.starts_with('/') => cookie.path = value.to_string(),
                "secure" => cookie.secure = true,
                "max-age" => {
                    cookie.persistent = true;
                    delete = value.parse::<i64>().is_ok_and(|seconds| seconds <= 0);
                },
                _ => {},
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
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("download");
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
            restored.storage_get("https://example.test", "theme").as_deref(),
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
        assert!(session
            .cookie_header(&Url::parse("http://accounts.example.test/login").unwrap())
            .is_empty());
        assert!(session
            .cookie_header(&Url::parse("https://accounts.example.test/other").unwrap())
            .is_empty());
    }

    #[test]
    fn download_names_are_sanitized_and_unique() {
        assert_eq!(sanitize_filename("../bad name?.txt"), ".._bad_name_.txt");
        assert_eq!(filename_from_disposition("attachment; filename=report.pdf").as_deref(), Some("report.pdf"));
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
                    },
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
            assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
        });
        let url = format!("http://127.0.0.1:{port}/cached");
        let mut session = BrowserSession::ephemeral();
        let first = session.fetch(&url, &FetchOptions::default()).unwrap();
        let second = session.fetch(&url, &FetchOptions::default()).unwrap();
        assert_eq!(first, second);
        server.join().unwrap();
    }
}
