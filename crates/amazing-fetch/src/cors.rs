use crate::*;
use std::collections::BTreeSet;
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
                && matches!(
                    value
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "application/x-www-form-urlencoded" | "multipart/form-data" | "text/plain"
                )
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
    let mut copy = request.clone();
    let plan = copy.prepare()?;
    if plan.taint == ResponseType::Opaque {
        return Ok(Response::hidden(ResponseType::Opaque));
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
    Ok(Response {
        response_type: plan.taint,
        status: response.status,
        status_text: response.status_text.clone(),
        headers,
        body: response.body.clone(),
        url_list: request.url_list().to_vec(),
    })
}
