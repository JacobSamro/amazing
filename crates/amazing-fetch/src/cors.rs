use crate::*;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// Bounded Fetch §4.8 preflight grants. A grant covers exactly the method and
/// unsafe header-name set that was checked against the preflight response.
pub struct PreflightCache {
    entries: Vec<PreflightGrant>,
}
struct PreflightGrant {
    origin: Origin,
    url: String,
    credentials: bool,
    method: String,
    names: Vec<String>,
    expires: Instant,
}
impl Default for PreflightCache {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}
impl PreflightCache {
    pub fn contains(&mut self, request: &Request, now: Instant) -> bool {
        self.entries.retain(|entry| entry.expires > now);
        let names = unsafe_request_header_names(&request.headers);
        self.entries.iter().any(|entry| {
            entry.origin == request.origin
                && entry.url == request.url().without_fragment()
                && (entry.credentials || request.credentials != CredentialsMode::Include)
                && entry.method == request.method()
                && entry.names == names
        })
    }
    /// Call only after `check_preflight` succeeds. Invalid max-age falls back
    /// to Fetch's five-second default; a server cannot retain a grant forever.
    pub fn insert(&mut self, request: &Request, response: &InternalResponse, now: Instant) {
        let seconds = response
            .headers
            .get("access-control-max-age")
            .and_then(|s| {
                (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| s.parse::<u64>().ok())
                    .flatten()
            })
            .unwrap_or(5)
            .min(7200);
        if seconds == 0 {
            return;
        }
        self.entries.retain(|entry| entry.expires > now);
        let names = unsafe_request_header_names(&request.headers);
        let url = request.url().without_fragment();
        self.entries.retain(|entry| {
            !(entry.origin == request.origin
                && entry.url == url
                && entry.credentials == (request.credentials == CredentialsMode::Include)
                && entry.method == request.method()
                && entry.names == names)
        });
        if self.entries.len() == 256 {
            self.entries.remove(0);
        }
        self.entries.push(PreflightGrant {
            origin: request.origin.clone(),
            url,
            credentials: request.credentials == CredentialsMode::Include,
            method: request.method().to_owned(),
            names,
            expires: now + Duration::from_secs(seconds),
        });
    }
}
pub fn safelisted_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "POST")
}
fn unsafe_byte(b: u8) -> bool {
    (b < 32 && b != 9) || b == 127 || b"\"():<>?@[\\]{}".contains(&b)
}
pub fn safelisted_request_header(name: &str, value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    match name.to_ascii_lowercase().as_str() {
        "accept" => !value.bytes().any(unsafe_byte),
        "accept-language" | "content-language" => value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" *,-.;=".contains(&b)),
        "content-type" => {
            !value.bytes().any(unsafe_byte)
                && crate::MimeType::parse(value).is_some_and(|mime| {
                    matches!(
                        mime.essence().as_str(),
                        "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                    )
                })
        }
        "range" => {
            let Some(v) = value.strip_prefix("bytes=") else {
                return false;
            };
            let Some((first, last)) = v.split_once('-') else {
                return false;
            };
            if first.is_empty()
                || !first.bytes().all(|b| b.is_ascii_digit())
                || !last.bytes().all(|b| b.is_ascii_digit())
            {
                return false;
            }
            let Ok(start) = first.parse::<u64>() else {
                return false;
            };
            last.is_empty() || last.parse::<u64>().is_ok_and(|end| end >= start)
        }
        _ => false,
    }
}
pub fn unsafe_request_header_names(headers: &HeaderList) -> Vec<String> {
    let mut unsafe_names = BTreeSet::new();
    let mut safe_names = BTreeSet::new();
    let mut size = 0usize;
    for (name, value) in headers.iter() {
        if safelisted_request_header(name, value) {
            safe_names.insert(name.to_owned());
            size = size.saturating_add(value.len());
        } else {
            unsafe_names.insert(name.to_owned());
        }
    }
    if size > 1024 {
        unsafe_names.extend(safe_names);
    }
    // Fetch Headers combines duplicates before transport. Check the combined
    // value too so a multimap cannot bypass the MIME-type / range restrictions.
    for name in headers.iter().map(|(n, _)| n) {
        if !safelisted_request_header(name, &headers.get(name).unwrap()) {
            unsafe_names.insert(name.to_owned());
        }
    }
    unsafe_names.into_iter().collect()
}
/// Redirect-tainted origins serialize to null even when the initiator was a
/// tuple origin; do not collapse opaque identity to the string "null".
pub fn cors_origin(request: &Request) -> String {
    for pair in request.url_list().windows(2) {
        if pair[0].origin() != pair[1].origin() && request.origin != *pair[0].origin() {
            return "null".into();
        }
    }
    request.origin.serialize()
}
pub fn cors_check(request: &Request, headers: &HeaderList) -> Result<(), FetchError> {
    let allowed = headers
        .get("access-control-allow-origin")
        .ok_or(FetchError::Cors)?;
    if request.credentials != CredentialsMode::Include && allowed == "*" {
        return Ok(());
    }
    if allowed != cors_origin(request) {
        return Err(FetchError::Cors);
    }
    if request.credentials == CredentialsMode::Include
        && headers.get("access-control-allow-credentials").as_deref() != Some("true")
    {
        return Err(FetchError::Cors);
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub method: &'static str,
    pub headers: HeaderList,
    /// Preflight never sends cookies or HTTP authentication.
    pub credentials: bool,
}
pub fn preflight(request: &Request) -> Result<Option<Preflight>, FetchError> {
    let mut copy = request.clone();
    if !copy.prepare()?.preflight {
        return Ok(None);
    }
    let mut headers = HeaderList::default();
    headers.append("origin", &cors_origin(request))?;
    headers.append("access-control-request-method", request.method())?;
    let names = unsafe_request_header_names(&request.headers);
    if !names.is_empty() {
        headers.append("access-control-request-headers", &names.join(","))?;
    }
    Ok(Some(Preflight {
        method: "OPTIONS",
        headers,
        credentials: false,
    }))
}
fn tokens(headers: &HeaderList, name: &str) -> Result<Vec<String>, FetchError> {
    let Some(value) = headers.get(name) else {
        return Ok(vec![]);
    };
    value
        .split(',')
        .map(|s| {
            let s = s.trim_matches([' ', '\t']);
            if crate::headers::token(s) {
                Ok(s.to_owned())
            } else {
                Err(FetchError::Cors)
            }
        })
        .collect()
}
pub fn check_preflight(request: &Request, response: &InternalResponse) -> Result<(), FetchError> {
    cors_check(request, &response.headers)?;
    if !(200..=299).contains(&response.status) {
        return Err(FetchError::Cors);
    }
    let methods = tokens(&response.headers, "access-control-allow-methods")?;
    let names = tokens(&response.headers, "access-control-allow-headers")?;
    let wildcard = request.credentials != CredentialsMode::Include;
    if !safelisted_method(request.method())
        && !methods
            .iter()
            .any(|m| m == request.method() || (wildcard && m == "*"))
    {
        return Err(FetchError::Cors);
    }
    for name in unsafe_request_header_names(&request.headers) {
        if !names.iter().any(|n| {
            n.eq_ignore_ascii_case(&name) || (wildcard && n == "*" && name != "authorization")
        }) {
            return Err(FetchError::Cors);
        }
    }
    Ok(())
}
fn safe_response(name: &str) -> bool {
    matches!(
        name,
        "cache-control"
            | "content-language"
            | "content-length"
            | "content-type"
            | "expires"
            | "last-modified"
            | "pragma"
    )
}
/// Validate policy and return only the script-visible data. Internal cookie
/// storage must run in the trusted network/cache layer, before this filtering.
pub fn filter_response(
    request: &Request,
    response: &InternalResponse,
) -> Result<Response, FetchError> {
    filter_response_with_csp_progress(request,response,&mut None)
}
pub(crate) fn filter_response_with_csp_progress(request:&Request,response:&InternalResponse,
    progress:&mut Option<CspResponseProvenance>) -> Result<Response,FetchError> {
    let mut copy = request.clone();
    let preparation=copy.prepare();
    *progress=copy.csp_pre_request_checked.then(||CspResponseProvenance::checked(&copy,Vec::new()));
    let plan=preparation?;
    let mut final_receipts=Vec::new();
    let security=crate::security::response_security_csp_progress(
        &copy,response,response.body.as_deref().unwrap_or_default(),&crate::psl::PublicSuffixList,&mut final_receipts);
    let provenance=CspResponseProvenance::checked(&copy,final_receipts);
    *progress=Some(provenance.clone());
    security?;
    let metadata = crate::sri::Metadata::parse(&request.integrity);
    if !metadata.is_empty() && plan.taint == ResponseType::Opaque {
        return Err(FetchError::Integrity);
    }
    if plan.taint == ResponseType::Opaque {
        let mut filtered = Response::hidden(ResponseType::Opaque);
        filtered.csp_provenance = Some(provenance);
        filtered.credential_receipt = Some(crate::ResponseCredentialReceipt::received(response));
        return Ok(filtered);
    }
    let mut headers = response.headers.clone();
    if plan.taint == ResponseType::Cors {
        cors_check(request, &headers)?;
        // Invalid expose list means no additional exposed names.
        let exposed = tokens(&headers, "access-control-expose-headers").unwrap_or_default();
        let wildcard =
            request.credentials != CredentialsMode::Include && exposed.iter().any(|n| n == "*");
        headers.retain(|name| {
            !crate::headers::forbidden_response(name)
                && (safe_response(name)
                    || wildcard
                    || exposed.iter().any(|n| n.eq_ignore_ascii_case(name)))
        });
    } else {
        headers.retain(|name| !crate::headers::forbidden_response(name));
    }
    if !metadata.matches(response.body.as_deref().unwrap_or_default()) {
        return Err(FetchError::Integrity);
    }
    Ok(Response {
        csp_provenance: Some(provenance),
        credential_receipt: Some(crate::ResponseCredentialReceipt::received(response)),
        response_type: plan.taint,
        status: response.status,
        status_text: response.status_text.clone(),
        headers,
        body: response.body.clone(),
        url_list: request.url_list().to_vec(),
    })
}

#[cfg(test)]
mod preflight_cache_tests {
    use super::*;

    #[test]
    fn hit_expiry_credentials_and_header_separation() {
        let origin = HttpUrl::parse("https://client.example/")
            .unwrap()
            .origin()
            .clone();
        let url = HttpUrl::parse("https://api.example/resource").unwrap();
        let mut request = Request::new(url, origin);
        request.set_method_body("PUT", None).unwrap();
        request.headers.append("x-token", "one").unwrap();
        request.credentials = CredentialsMode::Include;
        let mut headers = HeaderList::default();
        headers
            .append("access-control-allow-origin", "https://client.example")
            .unwrap();
        headers
            .append("access-control-allow-credentials", "true")
            .unwrap();
        headers
            .append("access-control-allow-methods", "PUT")
            .unwrap();
        headers
            .append("access-control-allow-headers", "x-token")
            .unwrap();
        headers.append("access-control-max-age", "2").unwrap();
        let response = InternalResponse {
            request_includes_credentials: true,
            status: 204,
            status_text: String::new(),
            headers,
            body: None,
        };
        check_preflight(&request, &response).unwrap();
        let now = Instant::now();
        let mut cache = PreflightCache::default();
        assert!(!cache.contains(&request, now));
        cache.insert(&request, &response, now);
        assert!(cache.contains(&request, now + Duration::from_secs(1)));
        let mut changed = request.clone();
        changed.headers.append("x-extra", "two").unwrap();
        assert!(!cache.contains(&changed, now + Duration::from_secs(1)));
        assert!(!cache.contains(&request, now + Duration::from_secs(2)));
        let mut uncredentialed = request.clone();
        uncredentialed.credentials = CredentialsMode::Omit;
        cache.insert(&uncredentialed, &response, now);
        assert!(!cache.contains(&request, now));
        assert!(cache.contains(&uncredentialed, now));
    }
}
