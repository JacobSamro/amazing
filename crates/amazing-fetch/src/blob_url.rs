//! File API blob URL store and Fetch's blob: scheme fetch.
//!
//! `URL.createObjectURL` registers a snapshot of the Blob bytes under a fresh
//! `blob:<origin>/<uuid>` URL; `revokeObjectURL` removes it. Fetch resolves the
//! URL (without fragment) at the start of the request, so a revoked URL is a
//! network error but an already started fetch keeps its bytes.
use crate::{api::Blob, FetchError, HeaderList, InternalResponse};
use amazing_net::ParsedUrl;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn store() -> &'static Mutex<HashMap<String, Blob>> {
    static STORE: OnceLock<Mutex<HashMap<String, Blob>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn without_fragment(url: &str) -> &str {
    url.split_once('#').map_or(url, |(head, _)| head)
}

/// Fresh version-4 style UUID string from process-local entropy.
pub fn new_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0u8; 16];
    for half in 0..2 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(SERIAL.fetch_add(1, Ordering::Relaxed));
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        bytes[half * 8..half * 8 + 8].copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// Registers `blob` and returns its new URL. `origin` is the serialized
/// origin of the creating settings object (`null` for opaque origins).
pub fn create_object_url(origin: &str, blob: Blob) -> String {
    let url = format!("blob:{origin}/{}", new_uuid());
    store().lock().unwrap().insert(url.clone(), blob);
    url
}

/// Removes the entry for `url` (fragment ignored). Unknown or non-blob URLs
/// are ignored, as the File API requires.
pub fn revoke_object_url(url: &str) {
    let Ok(parsed) = ParsedUrl::parse(url) else { return };
    if parsed.scheme() != "blob" {
        return;
    }
    store().lock().unwrap().remove(without_fragment(&parsed.serialize()));
}

pub fn lookup_object_url(url: &ParsedUrl) -> Option<Blob> {
    store().lock().unwrap().get(without_fragment(&url.serialize())).cloned()
}

/// Fetch "single range header value" parser: `bytes=a-b`, `bytes=a-`, `bytes=-n`.
fn parse_single_range(value: &str) -> Option<(Option<u64>, Option<u64>)> {
    let rest = value.strip_prefix("bytes=")?;
    let (start, end) = rest.split_once('-')?;
    let digits = |s: &str| -> Option<Option<u64>> {
        if s.is_empty() {
            return Some(None);
        }
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok().map(Some)
    };
    let (start, end) = (digits(start)?, digits(end)?);
    if start.is_none() && end.is_none() {
        return None;
    }
    if let (Some(s), Some(e)) = (start, end) {
        if s > e {
            return None;
        }
    }
    Some((start, end))
}

/// Scheme fetch for `blob:` URLs, including Range requests.
pub(crate) fn blob_response(
    url: &ParsedUrl,
    method: &str,
    request_headers: &HeaderList,
) -> Result<InternalResponse, FetchError> {
    if method != "GET" {
        return Err(FetchError::Transport);
    }
    let blob = lookup_object_url(url).ok_or(FetchError::Transport)?;
    let full = blob.bytes.len() as u64;
    let mut headers = HeaderList::default();
    let (status, status_text, body) = match request_headers.get("range") {
        None => (200, "OK", blob.bytes.clone()),
        Some(value) => {
            let (start, end) = parse_single_range(&value).ok_or(FetchError::Transport)?;
            let (start, end) = match (start, end) {
                (None, Some(suffix)) => (full.saturating_sub(suffix), full.saturating_sub(1)),
                (Some(s), None) => (s, full.saturating_sub(1)),
                (Some(s), Some(e)) => (s, e.min(full.saturating_sub(1))),
                (None, None) => return Err(FetchError::Transport),
            };
            if start >= full {
                return Err(FetchError::Transport);
            }
            headers.append("Content-Range", &format!("bytes {start}-{end}/{full}"))?;
            (206, "Partial Content", blob.bytes[start as usize..=end as usize].to_vec())
        }
    };
    headers.append("Content-Length", &body.len().to_string())?;
    if !blob.content_type.is_empty() {
        headers.append("Content-Type", &blob.content_type)?;
    }
    Ok(InternalResponse {
        request_includes_credentials: true,
        status,
        status_text: status_text.into(),
        headers,
        body: Some(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_lookup_revoke_and_ranges() {
        let url = create_object_url("http://a.test", Blob::from_bytes(b"0123456789".to_vec(), "Text/Plain"));
        assert!(url.starts_with("blob:http://a.test/"));
        let parsed = ParsedUrl::parse(&format!("{url}#frag")).unwrap();
        let none = HeaderList::default();
        let full = blob_response(&parsed, "GET", &none).unwrap();
        assert_eq!((full.status, full.body.as_deref()), (200, Some(&b"0123456789"[..])));
        assert_eq!(full.headers.get("content-type").as_deref(), Some("text/plain"));
        let mut range = HeaderList::default();
        range.append("Range", "bytes=-3").unwrap();
        let part = blob_response(&parsed, "GET", &range).unwrap();
        assert_eq!(part.status, 206);
        assert_eq!(part.body.as_deref(), Some(&b"789"[..]));
        assert_eq!(part.headers.get("content-range").as_deref(), Some("bytes 7-9/10"));
        let mut bad = HeaderList::default();
        bad.append("Range", "bytes=20-").unwrap();
        assert!(blob_response(&parsed, "GET", &bad).is_err());
        assert!(blob_response(&parsed, "POST", &none).is_err());
        revoke_object_url(&url);
        assert!(blob_response(&parsed, "GET", &none).is_err());
    }
}
