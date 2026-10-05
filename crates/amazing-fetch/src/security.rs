//! Fetch main-fetch blocking policy. No transport, DOM, or JS dependency.
//! Snapshot: Fetch Living Standard (2026-09-21), Mixed Content Level 2 editor's
//! draft (2026-09-23), and the annevk/orb processing-model draft (2026-09-23).
use crate::{
    FetchError, HeaderList, HttpUrl, InternalResponse, RegistrableDomain, Request, RequestMode,
    ResponseType, Site, Origin,
};
use crate::csp::{CspDisposition, CspPolicyList};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Destination {
    #[default]
    Empty,
    Document,
    Image,
    Font,
    Audio,
    Video,
    Script,
    Worker,
    SharedWorker,
    ServiceWorker,
    Style,
    Other,
}
impl Destination {
    fn script_like(self) -> bool {
        matches!(
            self,
            Self::Script | Self::Worker | Self::SharedWorker | Self::ServiceWorker
        )
    }
    fn upgradeable(self) -> bool {
        matches!(self, Self::Image | Self::Audio | Self::Video)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderPolicy {
    UnsafeNone,
    RequireCorp,
    Credentialless,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NoCorsMediaState {
    #[default]
    NotMedia,
    /// Only the trusted media loader may supply these states.
    Initial,
    Subsequent,
}
impl Default for EmbedderPolicy {
    fn default() -> Self {
        Self::UnsafeNone
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SecurityContext {
    pub client_restricts_mixed_content: bool,
    pub destination: Destination,
    pub image_set_initiator: bool,
    pub top_level_navigation: bool,
    pub embedder_policy: EmbedderPolicy,
    pub no_cors_media_state: NoCorsMediaState,
}

/// Fetch §2.9: this is the exact table from the pinned revision. ParsedUrl's
/// absent default port is not a bad port; explicit port 0 is preserved.
pub fn bad_port(url: &HttpUrl) -> bool {
    const PORTS: &[u16] = &[
        0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95,
        101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179,
        389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601,
        636, 989, 990, 993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566,
        6665, 6666, 6667, 6668, 6669, 6679, 6697, 10080,
    ];
    matches!(url.as_parsed().scheme(), "http" | "https")
        && url
            .as_parsed()
            .port()
            .is_some_and(|port| PORTS.contains(&port))
}

/// Mixed Content Level 2 §2. Secure Contexts treats loopback and localhost
/// HTTP URLs as potentially trustworthy. Only HTTP(S) is representable here.
pub fn a_priori_authenticated(url: &HttpUrl) -> bool {
    if url.as_parsed().scheme() == "data" {
        return true;
    }
    if url.is_secure() {
        return true;
    }
    let Some(host) = url.as_parsed().host() else {
        return false;
    };
    let host = host.serialize();
    let unbracketed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    host == "localhost" || host.ends_with(".localhost")
}

/// Level 2 §4.1, before port and mixed-content blocking in main fetch.
pub fn upgrade_mixed_content(request: &mut Request) -> Result<bool, FetchError> {
    let context = request.security;
    if !context.client_restricts_mixed_content
        || a_priori_authenticated(request.url())
        || !context.destination.upgradeable()
        || (context.destination == Destination::Image && context.image_set_initiator)
        || request.url().as_parsed().scheme() != "http"
        || request.url().as_parsed().host().is_some_and(|host| {
            host.serialize().trim_matches(&['[', ']'][..]).parse::<IpAddr>().is_ok()
        })
    {
        return Ok(false);
    }
    let original = request.url().serialize();
    let upgraded = HttpUrl::parse(&format!("https{}", &original[4..]))?;
    *request.url_list.last_mut().expect("nonempty URL list") = upgraded;
    Ok(true)
}

/// Main fetch request checks, evaluated per hop after mixed-content upgrade.
pub fn check_request_security(request: &Request) -> Result<(), FetchError> {
    if bad_port(request.url()) {
        return Err(FetchError::BadPort);
    }
    if request.security.client_restricts_mixed_content
        && !a_priori_authenticated(request.url())
        && !(request.security.destination == Destination::Document
            && request.security.top_level_navigation)
    {
        return Err(FetchError::MixedContent);
    }
    Ok(())
}

/// Check each embedding ancestor against the response's enforced
/// frame-ancestors policy, or X-Frame-Options when no enforced directive exists.
pub fn check_frame_embedding(
    response_url: &HttpUrl, headers: &HeaderList, ancestors: &[crate::Origin],
) -> Result<(), FetchError> {
    if ancestors.is_empty() { return Ok(()); }
    let policies = CspPolicyList::from_headers(headers).map_err(|_| FetchError::FrameEmbedding)?;
    let mut has_frame_ancestors = false;
    for policy in &policies.policies {
        if policy.disposition != CspDisposition::Enforce { continue; }
        let Some(directive) = policy.directives.iter().find(|d| d.name == "frame-ancestors") else { continue; };
        has_frame_ancestors = true;
        for ancestor in ancestors {
            let parsed = HttpUrl::parse(&ancestor.serialize()).map_err(|_| FetchError::FrameEmbedding)?;
            if !directive.sources.iter().any(|source|
                crate::csp::csp_source_matches(source, parsed.as_parsed(), response_url.origin(), false)) {
                return Err(FetchError::FrameEmbedding);
            }
        }
    }
    if has_frame_ancestors { return Ok(()); }
    for (_, value) in headers.iter().filter(|(name, _)| *name == "x-frame-options") {
        let value = value.trim();
        if value.eq_ignore_ascii_case("deny") { return Err(FetchError::FrameEmbedding); }
        if value.eq_ignore_ascii_case("sameorigin") {
            if ancestors.iter().any(|origin| origin != response_url.origin()) {
                return Err(FetchError::FrameEmbedding);
            }
        } else {
            // Conflicting or malformed XFO values must not relax an existing denial.
            return Err(FetchError::FrameEmbedding);
        }
    }
    Ok(())
}

/// Fetch §3.7 internal CORP check. Header values are case sensitive. The
/// current response URL is the checked URL; the initiator origin stays fixed
/// across redirects. A no-cors redirect is checked before following it.
pub fn check_corp(
    request: &Request,
    headers: &HeaderList,
    domains: &impl RegistrableDomain,
) -> Result<(), FetchError> {
    check_corp_with_credentials(request, headers, request.credentials_allowed(), false, domains)
}
/// CORP uses the authentic response's request-includes-credentials receipt,
/// including stored responses. The navigation owner supplies for_navigation
/// only for a true nested navigation; it is not inferred from a destination.
pub fn check_corp_with_credentials(
    request: &Request,
    headers: &HeaderList,
    request_includes_credentials: bool,
    for_navigation: bool,
    domains: &impl RegistrableDomain,
) -> Result<(), FetchError> {
    let (checking_origin,embedder_policy)=if for_navigation {
        request.embedding_owner.as_ref().map(|owner|(owner.origin(),owner.embedder_policy()))
            .unwrap_or((&request.origin,request.security.embedder_policy))
    } else {(&request.origin,request.security.embedder_policy)};
    corp_internal(checking_origin,embedder_policy,request.url(),request.mode,headers,
        request_includes_credentials,for_navigation,domains)
}
pub(crate) fn corp_internal(checking_origin:&Origin,embedder_policy:EmbedderPolicy,target:&HttpUrl,
    mode:RequestMode,headers:&HeaderList,request_includes_credentials:bool,for_navigation:bool,
    domains:&impl RegistrableDomain)->Result<(),FetchError> {
    if for_navigation && embedder_policy == EmbedderPolicy::UnsafeNone {
        return Ok(());
    }
    if (!for_navigation && mode != RequestMode::NoCors) || checking_origin == target.origin() {
        return Ok(());
    }
    let policy = headers.get("cross-origin-resource-policy");
    let policy = match policy.as_deref() {
        Some("same-origin") => "same-origin",
        Some("same-site") => "same-site",
        Some("cross-origin") => "cross-origin",
        _ if embedder_policy == EmbedderPolicy::RequireCorp
            || (embedder_policy == EmbedderPolicy::Credentialless
                && (request_includes_credentials || for_navigation)) => "same-origin",
        _ => "cross-origin",
    };
    match policy {
        "cross-origin" => Ok(()),
        "same-origin" => Err(FetchError::CrossOriginResourcePolicy),
        "same-site" => {
            match (
                checking_origin.site(domains),
                target.origin().site(domains),
            ) {
                (
                    Site::Tuple {
                        scheme: a,
                        domain: x,
                    },
                    Site::Tuple {
                        scheme: b,
                        domain: y,
                    },
                ) if x == y && (a == "https" || b != "https") => Ok(()),
                _ => Err(FetchError::CrossOriginResourcePolicy),
            }
        }
        _ => unreachable!(),
    }
}
pub fn check_redirect_corp(
    request: &Request,
    response: &InternalResponse,
    domains: &impl RegistrableDomain,
) -> Result<(), FetchError> {
    check_corp_with_credentials(request, &response.headers, response.request_includes_credentials, request.embedding_owner.is_some() && request.mode==RequestMode::Navigate, domains)
}

fn mime_essence(headers: &HeaderList) -> Option<String> {
    crate::mime::extract_mime_type(headers).map(|record| record.essence())
}
fn javascript_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}
/// Fetch §3.6: only the first decoded/split value controls nosniff.
pub fn determine_nosniff(headers: &HeaderList) -> bool {
    headers
        .get("x-content-type-options")
        .and_then(|v| {
            v.split(',')
                .next()
                .map(|s| s.trim_matches([' ', '\t']).eq_ignore_ascii_case("nosniff"))
        })
        .unwrap_or(false)
}
pub fn check_nosniff(destination: Destination, headers: &HeaderList) -> Result<(), FetchError> {
    if !determine_nosniff(headers) {
        return Ok(());
    }
    let mime = mime_essence(headers);
    if (destination.script_like() && !mime.as_deref().is_some_and(javascript_mime))
        || (destination == Destination::Style && mime.as_deref() != Some("text/css"))
    {
        return Err(FetchError::Nosniff);
    }
    Ok(())
}
fn check_script_mime(destination: Destination, headers: &HeaderList) -> Result<(), FetchError> {
    if destination.script_like()
        && mime_essence(headers).is_some_and(|mime| {
            mime.starts_with("audio/")
                || mime.starts_with("image/")
                || mime.starts_with("video/")
                || mime == "text/csv"
        })
    {
        return Err(FetchError::MimeType);
    }
    Ok(())
}

fn image_signature(prefix: &[u8]) -> bool {
    prefix.starts_with(b"\x89PNG\r\n\x1a\n")
        || prefix.starts_with(b"\xff\xd8\xff")
        || prefix.starts_with(b"GIF87a")
        || prefix.starts_with(b"GIF89a")
        || prefix.starts_with(b"BM")
        || prefix.starts_with(b"\0\0\x01\0")
        || (prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"WEBP")
}
fn media_signature(prefix: &[u8]) -> bool {
    prefix.starts_with(b"ID3")
        || prefix.starts_with(b"OggS")
        || (prefix.len() >= 12 && &prefix[4..8] == b"ftyp")
        || prefix.starts_with(b"\x1a\x45\xdf\xa3")
}
fn orb_blocklisted(mime: &str) -> bool {
    matches!(
        mime,
        "text/html" | "text/xml" | "application/xml" | "application/json"
    ) || mime.ends_with("+json")
        || mime.ends_with("+xml")
}
fn orb_never_sniffed(mime: &str) -> bool {
    matches!(
        mime,
        "application/dash+xml"
            | "application/gzip"
            | "application/msexcel"
            | "application/mspowerpoint"
            | "application/msword"
            | "application/msword-template"
            | "application/pdf"
            | "application/vnd.apple.mpegurl"
            | "application/vnd.ces-quickpoint"
            | "application/vnd.ces-quicksheet"
            | "application/vnd.ces-quickword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-excel.sheet.macroenabled.12"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.ms-powerpoint.presentation.macroenabled.12"
            | "application/vnd.ms-word"
            | "application/vnd.ms-word.document.12"
            | "application/vnd.ms-word.document.macroenabled.12"
            | "application/vnd.msword"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | "application/vnd.openxmlformats-officedocument.presentationml.template"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.template"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.template"
            | "application/vnd.presentation-openxml"
            | "application/vnd.presentation-openxmlm"
            | "application/vnd.spreadsheet-openxml"
            | "application/vnd.wordprocessing-openxml"
            | "application/x-gzip"
            | "application/x-protobuf"
            | "application/x-protobuffer"
            | "application/zip"
            | "audio/mpegurl"
            | "multipart/byteranges"
            | "multipart/signed"
            | "text/event-stream"
            | "text/csv"
            | "text/vtt"
    )
}
/// Shipping Chromium ORB compatibility: its final sniff checks the bounded
/// prefix for CORB-protected HTML/XML/JSON, rather than parsing the full body
/// as JavaScript. Plain mislabeled JavaScript is allowed at this step.
/// `prefix` is read-only and at most its first 1024 bytes are inspected.
pub fn check_opaque_response(
    status: u16,
    headers: &HeaderList,
    prefix: &[u8],
    media_state: NoCorsMediaState,
) -> Result<(), FetchError> {
    let mime = mime_essence(headers);
    let nosniff = determine_nosniff(headers);
    if let Some(ref mime) = mime {
        if javascript_mime(mime) || matches!(mime.as_str(), "text/css" | "image/svg+xml") {
            return Ok(());
        }
        if orb_never_sniffed(mime)
            || (status == 206 && orb_blocklisted(mime))
            || (nosniff && (orb_blocklisted(mime) || mime == "text/plain"))
        {
            return Err(FetchError::OpaqueResponse);
        }
    }
    if media_state == NoCorsMediaState::Subsequent {
        return Ok(());
    }
    if status == 206 && !valid_partial_response(headers) {
        return Err(FetchError::OpaqueResponse);
    }
    let prefix = &prefix[..prefix.len().min(1024)];
    if media_signature(prefix) {
        return if media_state == NoCorsMediaState::Initial && matches!(status, 200 | 206) {
            Ok(())
        } else {
            Err(FetchError::OpaqueResponse)
        };
    }
    if media_state != NoCorsMediaState::NotMedia {
        return Err(FetchError::OpaqueResponse);
    }
    if image_signature(prefix) {
        return Ok(());
    }
    // A streaming implementation must wait for 1024 bytes or EOF before this
    // fallback. It can call this function once that bounded prefix is ready.
    if corb_document_prefix(prefix) {
        Err(FetchError::OpaqueResponse)
    } else {
        Ok(())
    }
}
fn corb_document_prefix(prefix: &[u8]) -> bool {
    let bytes = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    let bytes = bytes.trim_ascii_start();
    if bytes.starts_with(b")]}'") || bytes.starts_with(b"{}&&") || bytes.starts_with(b"for(;;);") {
        return true;
    }
    if json_object_prefix(bytes) {
        return true;
    }
    [
        b"<!doctype html".as_slice(),
        b"<html".as_slice(),
        b"<head".as_slice(),
        b"<script".as_slice(),
        b"<body".as_slice(),
        b"<iframe".as_slice(),
        b"<div".as_slice(),
        b"<table".as_slice(),
        b"<style".as_slice(),
        b"<?xml".as_slice(),
    ]
    .iter()
    .any(|tag| {
        bytes
            .get(..tag.len())
            .is_some_and(|b| b.eq_ignore_ascii_case(tag))
    })
}
fn json_object_prefix(bytes: &[u8]) -> bool {
    let Some(mut tail) = bytes.strip_prefix(b"{") else {
        return false;
    };
    tail = tail.trim_ascii_start();
    let Some(rest) = tail.strip_prefix(b"\"") else {
        return false;
    };
    let mut escaped = false;
    for (i, byte) in rest.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => return rest[i + 1..].trim_ascii_start().starts_with(b":"),
            b'\n' | b'\r' | 0..=31 => return false,
            _ => (),
        }
    }
    false
}
fn valid_partial_response(headers: &HeaderList) -> bool {
    let Some(range) = headers.get("content-range") else {
        return false;
    };
    let Some(rest) = range.strip_prefix("bytes 0-") else {
        return false;
    };
    let Some((end, total)) = rest.split_once('/') else {
        return false;
    };
    let Ok(end) = end.parse::<u64>() else {
        return false;
    };
    total == "*" || total.parse::<u64>().is_ok_and(|total| end < total)
}

/// Apply response checks before opaque filtering can hide security metadata.
/// Existing complete-body callers pass a borrowed slice; streaming callers can
/// pass a bounded prefix to the same policy without allocating a whole body.
pub fn check_response_security_with_prefix(
    request: &Request,
    response: &InternalResponse,
    prefix: &[u8],
    domains: &impl RegistrableDomain,
) -> Result<(), FetchError> {
    response_security_csp_receipts(request, response, prefix, domains).map(|_| ())
}
pub(crate) fn response_security_csp_receipts(
    request: &Request,
    response: &InternalResponse,
    prefix: &[u8],
    domains: &impl RegistrableDomain,
) -> Result<Vec<crate::CspViolation>, FetchError> {
    let mut receipts=Vec::new();
    response_security_csp_progress(request,response,prefix,domains,&mut receipts)?;
    Ok(receipts)
}
pub(crate) fn response_security_csp_progress(
    request:&Request,response:&InternalResponse,prefix:&[u8],domains:&impl RegistrableDomain,
    receipts:&mut Vec<crate::CspViolation>,
) -> Result<(),FetchError> {
    let csp = request.csp.post_request(request, request.url());
    *receipts=csp.violations.clone();
    if csp.blocked {
        return Err(FetchError::Csp(crate::CspResponseProvenance::checked(request, csp.violations).violations().to_vec()));
    }
    if request.security.client_restricts_mixed_content
        && !a_priori_authenticated(request.url())
        && !(request.security.destination == Destination::Document
            && request.security.top_level_navigation)
    {
        return Err(FetchError::MixedContent);
    }
    // HTTP fetch checks CORP on the internal response before main fetch's
    // MIME/nosniff checks or the opaque response can be exposed.
    // Actual child-navigation custody comes from the native constructor, not
    // destination/URL heuristics. Redirect responses with Location belong to
    // the navigation controller; enforce this response check at the final hop.
    let nested_response = request.nested_navigation && (request.embedding_owner.is_some() || !(matches!(response.status,301|302|303|307|308)
        && response.headers.get("location").is_some()));
    // Fetch's data scheme branch produces a basic response even in no-cors
    // mode. HTTP CORP/opaque checks must not turn that local response opaque.
    let http_response = matches!(request.url().as_parsed().scheme(), "http" | "https");
    if (request.mode == RequestMode::NoCors && http_response) || nested_response {
        check_corp_with_credentials(request, &response.headers, response.request_includes_credentials, nested_response, domains)?;
    }
    check_script_mime(request.security.destination, &response.headers)?;
    check_nosniff(request.security.destination, &response.headers)?;
    if request.taint == ResponseType::Opaque
        || (http_response && request.mode == RequestMode::NoCors && request.origin != *request.url().origin())
    {
        check_opaque_response(
            response.status,
            &response.headers,
            prefix,
            request.security.no_cors_media_state,
        )?;
    }
    Ok(())
}
pub fn check_response_security(
    request: &Request,
    response: &InternalResponse,
    domains: &impl RegistrableDomain,
) -> Result<(), FetchError> {
    check_response_security_with_prefix(
        request,
        response,
        response.body.as_deref().unwrap_or(&[]),
        domains,
    )
}
