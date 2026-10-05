//! Browser-authored Fetch requests over a bounded, pull-driven IPC contract.
use crate::protocol::*;
use amazing_fetch::api::{
    ApiHeaders, ApiRequest, ApiRequestInit, ApiRequestInput, BodyInput, HeaderGuard,
    NavigationRequestInit, NavigableContext,
};
use amazing_fetch::cookies::{CookieAccess, CookieSite};
use amazing_fetch::service::{
    ApiResponseHead, ClientContext, NetworkService, ServiceEvent, ServiceId,
};
use amazing_fetch::{
    CspDestination, CspPolicyList, Destination, EmbedderPolicy, HeaderList, HttpUrl,
    NoCorsMediaState, Origin, SecurityContext,
};
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{Connection, Error};
use amazing_net::dispatch::DispatchLimits;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use std::time::Instant;

const MAX_ACTIVE: usize = 8;
const MAX_HEADERS: usize = 128;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_URL_BYTES: usize = 8 * 1024;

fn invalid(message: &str) -> Error {
    Error::Remote(message.into())
}

fn check_headers(fields: &[HeaderField]) -> Result<(), Error> {
    if fields.len() > MAX_HEADERS
        || fields
            .iter()
            .try_fold(0usize, |n, h| {
                n.checked_add(h.name.len())?.checked_add(h.value.len())
            })
            .is_none_or(|n| n > MAX_HEADER_BYTES)
    {
        return Err(invalid("header limit"));
    }
    Ok(())
}

fn small(value: &str, limit: usize) -> bool {
    value.len() <= limit
}

fn origin(value: &WireOrigin) -> Result<Origin, Error> {
    match value {
        WireOrigin::Tuple(url) => {
            if url.len() > MAX_URL_BYTES || !url.is_ascii() {
                return Err(invalid("origin limit"));
            }
            let parsed = HttpUrl::parse(url).map_err(|_| invalid("invalid origin"))?;
            if parsed.origin().serialize() != *url {
                return Err(invalid("noncanonical origin"));
            }
            Ok(parsed.origin().clone())
        }
        WireOrigin::Opaque(id) if *id != 0 => Ok(Origin::opaque_with_id(*id)),
        _ => Err(invalid("invalid opaque origin")),
    }
}

/// Convert a browser-owned Fetch origin without losing opaque identity.
pub fn origin_to_wire(value: &Origin) -> WireOrigin {
    match value.opaque_id() {
        Some(id) => WireOrigin::Opaque(id),
        None => WireOrigin::Tuple(value.serialize()),
    }
}

fn destination(value: &str) -> Result<(Destination, CspDestination), Error> {
    Ok(match value {
        "empty" => (Destination::Empty, CspDestination::None),
        "document" => (Destination::Document, CspDestination::None),
        "image" => (Destination::Image, CspDestination::Image),
        "font" => (Destination::Font, CspDestination::Font),
        "audio" => (Destination::Audio, CspDestination::Media),
        "video" => (Destination::Video, CspDestination::Media),
        "script" => (Destination::Script, CspDestination::Script),
        "style" => (Destination::Style, CspDestination::Style),
        "connect" => (Destination::Empty, CspDestination::Connect),
        "worker" => (Destination::Worker, CspDestination::Worker),
        "shared-worker" => (Destination::SharedWorker, CspDestination::Worker),
        "service-worker" => (Destination::ServiceWorker, CspDestination::Worker),
        "frame" => (Destination::Other, CspDestination::Frame),
        "child" => (Destination::Other, CspDestination::Child),
        "object" => (Destination::Other, CspDestination::Object),
        "manifest" => (Destination::Other, CspDestination::Manifest),
        "other" => (Destination::Other, CspDestination::None),
        _ => return Err(invalid("invalid destination")),
    })
}

fn build_request(wire: NetworkStart) -> Result<(ApiRequest, ClientContext), Error> {
    if wire.version != NETWORK_VERSION
        || wire.client_id == 0
        || wire.epoch == 0
        || wire.url.len() > MAX_URL_BYTES
        || wire.base_url.len() > MAX_URL_BYTES
        || wire.response_limit > NETWORK_MAX_RESPONSE_BODY
        || wire.body.0.len() > NETWORK_MAX_REQUEST_BODY
        || (!wire.body_present && !wire.body.0.is_empty())
        || !small(&wire.method, 64)
        || !small(&wire.destination, 64)
        || !small(&wire.mode, 64)
        || !small(&wire.credentials, 64)
        || !small(&wire.redirect, 64)
        || !small(&wire.cache, 64)
        || !small(&wire.referrer, MAX_URL_BYTES)
        || !small(&wire.referrer_policy, 64)
        || !small(&wire.context.embedder_policy, 64)
        || !small(&wire.context.no_cors_media_state, 64)
    {
        return Err(invalid("invalid network start"));
    }
    check_headers(&wire.headers)?;
    check_headers(&wire.context.csp_headers)?;
    let base = HttpUrl::parse(&wire.base_url).map_err(|_| invalid("invalid base URL"))?;
    if !matches!(base.as_parsed().scheme(), "http" | "https") {
        return Err(invalid("network base must be HTTP(S)"));
    }
    let request_origin = origin(&wire.context.origin)?;
    let cookie_site = match &wire.context.cookie_site {
        WireCookieSite::NoClient(()) => CookieSite::NoClient,
        WireCookieSite::Origin(value) => CookieSite::Origin(origin(value)?),
        WireCookieSite::CrossSite(()) => CookieSite::CrossSite,
    };
    let (destination, csp_destination) = destination(&wire.destination)?;
    let embedder_policy = match wire.context.embedder_policy.as_str() {
        "unsafe-none" => EmbedderPolicy::UnsafeNone,
        "require-corp" => EmbedderPolicy::RequireCorp,
        "credentialless" => EmbedderPolicy::Credentialless,
        _ => return Err(invalid("invalid embedder policy")),
    };
    let no_cors_media_state = match wire.context.no_cors_media_state.as_str() {
        "none" => NoCorsMediaState::NotMedia,
        "initial" => NoCorsMediaState::Initial,
        "subsequent" => NoCorsMediaState::Subsequent,
        _ => return Err(invalid("invalid media state")),
    };
    let mut policy_headers = HeaderList::default();
    for h in &wire.context.csp_headers {
        if !matches!(
            h.name.to_ascii_lowercase().as_str(),
            "content-security-policy" | "content-security-policy-report-only"
        ) {
            return Err(invalid("invalid CSP header"));
        }
        policy_headers
            .append(&h.name, &h.value)
            .map_err(|_| invalid("invalid CSP value"))?;
    }
    let csp = CspPolicyList::from_headers(&policy_headers).map_err(|_| invalid("invalid CSP"))?;
    let mut headers = ApiHeaders::new(HeaderGuard::None);
    for h in &wire.headers {
        headers
            .append_string(&h.name, &h.value)
            .map_err(|_| invalid("invalid request header"))?;
    }
    let child_navigation = wire.mode == "navigate" && wire.destination == "frame";
    let top_level_navigation = wire.mode == "navigate" && wire.destination == "document";
    if wire.context.ancestor_origins.len() > 32 || wire.context.sandbox_flags & !0xffff != 0 {
        return Err(invalid("invalid ancestor context"));
    }
    let frame_ancestors: Vec<Origin> = wire.context.ancestor_origins.iter()
        .map(origin).collect::<Result<_, _>>()?;
    let top_level_origin = wire.context.top_level_origin.as_ref().map(origin).transpose()?;
    if child_navigation && (wire.context.top_level_navigation
        || frame_ancestors.is_empty()
        || frame_ancestors.last() != Some(&request_origin)
        || frame_ancestors.first() != top_level_origin.as_ref()) {
        return Err(invalid("invalid child navigation context"));
    }
    let mut request = if wire.mode == "navigate" {
        if !(top_level_navigation && wire.context.top_level_navigation || child_navigation)
            || wire.credentials != "include"
            || wire.redirect != "manual"
            || wire.cache != "default"
        {
            return Err(invalid("invalid navigation options"));
        }
        let url = HttpUrl::parse(&wire.url).map_err(|_| invalid("invalid navigation URL"))?;
        let mut navigation = if child_navigation {
            if wire.method != "GET" || wire.body_present || !wire.body.0.is_empty()
                || wire.navigation_reload_or_history || wire.navigation_client_reserved_id.is_some() {
                return Err(invalid("invalid child navigation method"));
            }
            ApiRequest::child_navigation(url, &NavigableContext {
                top_level_origin: top_level_origin.clone().unwrap(),
                parent_origin: request_origin.clone(),
                ancestor_origins: frame_ancestors.clone(),
                sandbox_flags: wire.context.sandbox_flags,
                frame_csp: csp.clone(),
                referrer: wire.referrer.clone(),
                referrer_policy: wire.referrer_policy.clone(),
                now_unix: wire.context.now_unix,
            })
        } else { ApiRequest::navigation(
            url,
            NavigationRequestInit {
                method: wire.method,
                body: wire.body.0,
                referrer: wire.referrer,
                referrer_policy: wire.referrer_policy,
                initiator_origin: request_origin,
                reload_or_history: wire.navigation_reload_or_history,
                client_reserved_id: wire.navigation_client_reserved_id,
            },
        ) }
        .map_err(|e| Error::Remote(format!("navigation: {e:?}")))?;
        for h in &wire.headers {
            navigation
                .headers
                .append_string(&h.name, &h.value)
                .map_err(|_| invalid("invalid navigation header"))?;
        }
        navigation
    } else {
        if wire.navigation_reload_or_history || wire.navigation_client_reserved_id.is_some() {
            return Err(invalid("navigation fields on subresource"));
        }
        let input: Vec<u16> = wire.url.encode_utf16().collect();
        let body = wire
            .body_present
            .then_some(Some(BodyInput::Bytes(wire.body.0)));
        let mut init = ApiRequestInit::default(); // private allow_url_credentials remains false
        init.method = Some(wire.method.encode_utf16().collect());
        init.headers = Some(headers);
        init.body = body;
        init.referrer = Some(wire.referrer.encode_utf16().collect());
        init.referrer_policy = Some(wire.referrer_policy);
        init.mode = Some(wire.mode);
        init.credentials = Some(wire.credentials);
        init.cache = Some(wire.cache);
        init.redirect = Some(wire.redirect);
        init.integrity = Some(String::new());
        init.keepalive = Some(false);
        init.signal_aborted = Some(false);
        init.priority = Some("auto".into());
        init.window_non_null = Some(false);
        ApiRequest::new(ApiRequestInput::Url(&input), init, &base, request_origin)
            .map_err(|e| Error::Remote(format!("request: {e:?}")))?
    };
    if !matches!(request.url().as_parsed().scheme(), "http" | "https") {
        return Err(invalid("network request must be HTTP(S)"));
    }
    request
        .set_response_body_limit(u64::from(wire.response_limit))
        .map_err(|e| Error::Remote(format!("limit: {e:?}")))?;
    Ok((
        request,
        ClientContext {
            cookie_site: if child_navigation {
                let site = top_level_origin.as_ref().unwrap().site(&amazing_fetch::psl::PublicSuffixList);
                if frame_ancestors.iter().all(|ancestor| ancestor.site(&amazing_fetch::psl::PublicSuffixList) == site) {
                    CookieSite::Origin(top_level_origin.as_ref().unwrap().clone())
                } else { CookieSite::CrossSite }
            } else { cookie_site },
            partition_key: top_level_origin.as_ref()
                .map(|origin| origin.site(&amazing_fetch::psl::PublicSuffixList)),
            frame_ancestors: if child_navigation { frame_ancestors } else { Vec::new() },
            security: SecurityContext {
                client_restricts_mixed_content: wire.context.restricts_mixed_content,
                destination,
                image_set_initiator: wire.context.image_set_initiator,
                top_level_navigation: wire.context.top_level_navigation,
                embedder_policy,
                no_cors_media_state,
            },
            csp,
            csp_destination,
            now_unix: wire.context.now_unix,
        },
    ))
}

struct Active {
    client_id: u64,
    epoch: u64,
    response_limit: u32,
    received: u32,
    credit_remaining: u32,
    seen_head: bool,
    terminal: bool,
    subresource: bool,
    events: VecDeque<NetworkReply>,
}
fn wire_head(id: u64, active: &Active, head: ApiResponseHead) -> Result<NetworkHead, Error> {
    let outgoing = NetworkHead {
        id,
        client_id: active.client_id,
        epoch: active.epoch,
        status: head.status,
        redirected: head.redirected,
        url: head.final_url,
        reason: head.reason,
        headers: head
            .headers
            .into_iter()
            .map(|(name, value)| HeaderField { name, value })
            .collect(),
        response_type: format!("{:?}", head.response_type),
        url_list: head.url_list,
        redirect_tainted: head.redirect_tainted,
    };
    if outgoing.url.len() > MAX_URL_BYTES
        || outgoing.reason.len() > 1024
        || outgoing.response_type.len() > 64
        || check_headers(&outgoing.headers).is_err()
        || outgoing.url_list.len() > 32
        || outgoing
            .url_list
            .iter()
            .any(|url| url.len() > MAX_URL_BYTES)
    {
        return Err(invalid("response head limit"));
    }
    Ok(outgoing)
}
struct State {
    service: NetworkService,
    active: BTreeMap<u64, Active>,
    epochs: BTreeMap<u64, u64>,
}
fn http_cache_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("AMAZING_HTTP_CACHE_DIR") {
        return path.into();
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return std::path::PathBuf::from(path)
            .join("Amazing")
            .join("http-cache");
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return std::path::PathBuf::from(path)
            .join("amazing")
            .join("http-cache");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(path)
            .join(".cache")
            .join("amazing")
            .join("http-cache");
    }
    std::env::temp_dir().join("amazing-http-cache")
}
impl Drop for State {
    fn drop(&mut self) {
        for id in self.active.keys().copied() {
            self.service.cancel(ServiceId(id));
        }
        self.active.clear();
    }
}
impl State {
    fn document_cookie(&mut self, request: DocumentCookie) -> Result<String, Error> {
        if request.client_id == 0 || request.epoch == 0
            || request.document_url.len() > MAX_URL_BYTES
            || request.assignment.as_ref().is_some_and(|value| value.len() > 65536)
            || self.epochs.get(&request.client_id).is_some_and(|epoch| *epoch != request.epoch)
        {
            return Err(invalid("invalid document cookie request"));
        }
        let url = HttpUrl::parse(&request.document_url).map_err(|_| invalid("invalid document URL"))?;
        if !matches!(url.as_parsed().scheme(), "http" | "https") {
            return Err(invalid("invalid document scheme"));
        }
        let top = origin(&request.top_level_origin)?;
        let access = CookieAccess {
            url: &url,
            site_for_cookies: CookieSite::Origin(top.clone()),
            http_api: false,
            top_level_navigation: false,
            method: "GET",
            partition_key: Some(top.site(&amazing_fetch::psl::PublicSuffixList)),
        };
        if let Some(assignment) = request.assignment {
            // HTML's setter silently ignores a rejected cookie.
            if self.service.cookies.set_cookie(assignment.as_bytes(), &access, request.now_unix).is_ok() {
                self.service.persist_cookies(request.now_unix);
            }
            Ok(String::new())
        } else {
            let bytes = self.service.cookies.cookie_header(&access, request.now_unix)
                .map_err(|_| invalid("cookie retrieval failed"))?;
            String::from_utf8(bytes).map_err(|_| invalid("cookie bytes are not UTF-8"))
        }
    }
    fn new() -> Self {
        let mut service = NetworkService::new(DispatchLimits::default());
        match crate::network_cache::DiskCache::open(
            http_cache_dir(), amazing_fetch::cache::CacheLimits::default(),
        ) {
            Ok((cache, disk)) => service.install_cache(cache, Box::new(disk)),
            Err(error) => eprintln!("HTTP cache unavailable: {error}"),
        }
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        match crate::network_hsts::DiskHsts::open(&http_cache_dir(), wall) {
            Ok((hsts, disk)) => service.install_hsts(hsts, Box::new(disk)),
            Err(error) => eprintln!("HSTS store unavailable: {error}"),
        }
        match crate::network_cookies::DiskCookies::open(&http_cache_dir(), wall) {
            Ok((cookies, disk)) => service.install_cookies(cookies, Box::new(disk)),
            Err(error) => eprintln!("cookie store unavailable: {error}"),
        }
        Self {
            service,
            active: BTreeMap::new(),
            epochs: BTreeMap::new(),
        }
    }
    fn collect(&mut self) {
        for event in self.service.step(64, Instant::now()) {
            match event {
                ServiceEvent::ResponseHead { id, head } => {
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    if active.seen_head || active.terminal {
                        continue;
                    }
                    if let Ok(outgoing) = wire_head(id.0, active, head) {
                        active.seen_head = true;
                        active.events.push_back(NetworkReply::Head(outgoing));
                    } else {
                        self.service.cancel(id);
                        active.terminal = true;
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: Some("response head limit".into()),
                            }));
                    }
                }
                ServiceEvent::BodyChunk { id, bytes } => {
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                    if !active.seen_head
                        || active.terminal
                        || len == 0
                        || len > active.credit_remaining
                        || active
                            .received
                            .checked_add(len)
                            .is_none_or(|n| n > active.response_limit)
                    {
                        self.service.cancel(id);
                        active.terminal = true;
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: Some("stream bounds violated".into()),
                            }));
                        continue;
                    }
                    active.credit_remaining -= len;
                    active.received += len;
                    active.events.push_back(NetworkReply::Body(NetworkBody {
                        id: id.0,
                        epoch: active.epoch,
                        bytes: ByteBuf(bytes),
                    }));
                }
                ServiceEvent::StreamFinished { id } => {
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    if !active.terminal {
                        active.terminal = true;
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: (!active.seen_head).then(|| "missing response head".into()),
                            }));
                    }
                }
                ServiceEvent::StreamFailed { id, error } => {
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    if !active.terminal {
                        active.terminal = true;
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: Some(format!("{error:?}")),
                            }));
                    }
                }
                ServiceEvent::SubresourceFinished { id, response } => {
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    if !active.subresource || active.terminal {
                        continue;
                    }
                    active.terminal = true;
                    if response.body.len() > active.response_limit as usize {
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: Some("subresource body limit".into()),
                            }));
                        continue;
                    }
                    let response_type = format!("{:?}", response.response_type);
                    match wire_head(id.0, active, response.head) {
                        Ok(mut head) => {
                            head.response_type = response_type;
                            active.seen_head = true;
                            active.events.push_back(NetworkReply::Head(head));
                            for bytes in response.body.chunks(NETWORK_CHUNK as usize) {
                                active.events.push_back(NetworkReply::Body(NetworkBody {
                                    id: id.0,
                                    epoch: active.epoch,
                                    bytes: ByteBuf(bytes.to_vec()),
                                }));
                            }
                            active
                                .events
                                .push_back(NetworkReply::Complete(NetworkComplete {
                                    id: id.0,
                                    epoch: active.epoch,
                                    error: None,
                                }));
                        }
                        Err(error) => {
                            active
                                .events
                                .push_back(NetworkReply::Complete(NetworkComplete {
                                    id: id.0,
                                    epoch: active.epoch,
                                    error: Some(format!("{error}")),
                                }))
                        }
                    }
                }
                ServiceEvent::SubresourceFailed { id, failure } => {
                    let error=failure.error;
                    let Some(active) = self.active.get_mut(&id.0) else {
                        continue;
                    };
                    if active.subresource && !active.terminal {
                        active.terminal = true;
                        active
                            .events
                            .push_back(NetworkReply::Complete(NetworkComplete {
                                id: id.0,
                                epoch: active.epoch,
                                error: Some(format!("{error:?}")),
                            }));
                    }
                }
                _ => {}
            }
        }
    }
    fn start(&mut self, wire: NetworkStart) -> Result<NetworkStarted, Error> {
        self.start_kind(wire, false)
    }
    fn start_subresource(&mut self, wire: NetworkStart) -> Result<NetworkStarted, Error> {
        self.start_kind(wire, true)
    }
    fn start_kind(
        &mut self,
        wire: NetworkStart,
        subresource: bool,
    ) -> Result<NetworkStarted, Error> {
        let client_id = wire.client_id;
        let epoch = wire.epoch;
        let response_limit = wire.response_limit;
        let requested_destination = wire.destination.clone();
        if subresource
            && (!matches!(
                requested_destination.as_str(),
                "image" | "font" | "style" | "script"
            ) || !matches!(wire.mode.as_str(), "no-cors" | "cors")
                || wire.method != "GET"
                || wire.body_present)
        {
            return Err(invalid("invalid browser subresource"));
        }
        let (request, context) = build_request(wire)?;
        if let Some(&current) = self.epochs.get(&client_id) {
            if epoch < current {
                return Err(invalid("stale epoch"));
            }
            if epoch > current {
                self.cancel_epoch(NetworkEpoch {
                    client_id,
                    epoch: current,
                });
            }
        }
        if self.active.len() >= MAX_ACTIVE {
            return Err(invalid("too many active requests"));
        }
        let id = if subresource {
            self.service
                .start_subresource(request, context, destination(&requested_destination)?.0)
        } else {
            self.service
                .start_stream_with_credit(request, context, 0)
                .map_err(|e| Error::Remote(format!("start: {e:?}")))?
        };
        self.epochs.insert(client_id, epoch);
        self.active.insert(
            id.0,
            Active {
                client_id,
                epoch,
                response_limit,
                received: 0,
                credit_remaining: 0,
                seen_head: false,
                terminal: false,
                subresource,
                events: VecDeque::new(),
            },
        );
        Ok(NetworkStarted {
            id: id.0,
            client_id,
            epoch,
        })
    }
    fn read(&mut self, request: NetworkRead) -> Result<NetworkReply, Error> {
        if request.credit == 0 || request.credit > NETWORK_CHUNK {
            return Err(invalid("invalid credit"));
        }
        let active = self
            .active
            .get(&request.id)
            .ok_or_else(|| invalid("unknown request"))?;
        if (active.client_id, active.epoch) != (request.client_id, request.epoch) {
            return Err(invalid("request ownership"));
        }
        if active.events.is_empty() && !active.terminal {
            if !active.subresource && active.credit_remaining == 0 {
                if !self
                    .service
                    .grant_credit(ServiceId(request.id), request.credit as usize)
                {
                    return Err(invalid("credit rejected"));
                }
                self.active.get_mut(&request.id).unwrap().credit_remaining = request.credit;
            }
            self.collect();
        }
        let active = self
            .active
            .get_mut(&request.id)
            .ok_or_else(|| invalid("unknown request"))?;
        let mut event = active
            .events
            .pop_front()
            .unwrap_or(NetworkReply::Pending(()));
        if let NetworkReply::Body(body) = &mut event {
            if body.bytes.0.len() > request.credit as usize {
                let rest = body.bytes.0.split_off(request.credit as usize);
                active.events.push_front(NetworkReply::Body(NetworkBody {
                    id: body.id,
                    epoch: body.epoch,
                    bytes: ByteBuf(rest),
                }));
            }
        }
        if matches!(event, NetworkReply::Complete(_)) {
            self.active.remove(&request.id);
        }
        Ok(event)
    }
    fn cancel(&mut self, request: NetworkCancel) -> Result<NetworkComplete, Error> {
        let active = self
            .active
            .get(&request.id)
            .ok_or_else(|| invalid("unknown request"))?;
        if (active.client_id, active.epoch) != (request.client_id, request.epoch) {
            return Err(invalid("request ownership"));
        }
        self.active.remove(&request.id);
        self.service.cancel(ServiceId(request.id));
        self.collect();
        Ok(NetworkComplete {
            id: request.id,
            epoch: request.epoch,
            error: Some("cancelled".into()),
        })
    }
    fn cancel_epoch(&mut self, request: NetworkEpoch) -> Vec<NetworkComplete> {
        let ids: Vec<(u64, u64)> = self
            .active
            .iter()
            .filter_map(|(&id, state)| {
                (state.client_id == request.client_id && state.epoch <= request.epoch)
                    .then_some((id, state.epoch))
            })
            .collect();
        let mut cancelled = Vec::with_capacity(ids.len());
        for (id, epoch) in ids {
            self.active.remove(&id);
            self.service.cancel(ServiceId(id));
            cancelled.push(NetworkComplete {
                id,
                epoch,
                error: Some("cancelled".into()),
            });
        }
        if !cancelled.is_empty() {
            self.collect();
        }
        cancelled
    }
}

enum Command {
    Start(NetworkStart, mpsc::Sender<Result<NetworkStarted, Error>>),
    Subresource(NetworkStart, mpsc::Sender<Result<NetworkStarted, Error>>),
    Read(NetworkRead, mpsc::Sender<Result<NetworkReply, Error>>),
    Cancel(NetworkCancel, mpsc::Sender<Result<NetworkComplete, Error>>),
    CancelEpoch(NetworkEpoch, mpsc::Sender<Vec<NetworkComplete>>),
    DocumentCookie(DocumentCookie, mpsc::Sender<Result<String, Error>>),
}
pub struct NetworkEndpointV2 {
    tx: mpsc::SyncSender<Command>,
}
impl NetworkEndpointV2 {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(128);
        std::thread::spawn(move || {
            let mut state = State::new();
            while let Ok(command) = rx.recv() {
                match command {
                    Command::Start(v, out) => {
                        let _ = out.send(state.start(v));
                    }
                    Command::Subresource(v, out) => {
                        let _ = out.send(state.start_subresource(v));
                    }
                    Command::Read(v, out) => {
                        let _ = out.send(state.read(v));
                    }
                    Command::Cancel(v, out) => {
                        let _ = out.send(state.cancel(v));
                    }
                    Command::CancelEpoch(v, out) => {
                        let _ = out.send(state.cancel_epoch(v));
                    }
                    Command::DocumentCookie(v, out) => {
                        let _ = out.send(state.document_cookie(v));
                    }
                }
            }
        });
        Self { tx }
    }
    fn ask<T>(&self, build: impl FnOnce(mpsc::Sender<T>) -> Command) -> Result<T, Error> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .try_send(build(tx))
            .map_err(|_| Error::Disconnected)?;
        rx.recv().map_err(|_| Error::Disconnected)
    }
}
impl Default for NetworkEndpointV2 {
    fn default() -> Self {
        Self::new()
    }
}
impl NetworkRequestsV2 for NetworkEndpointV2 {
    fn start(&self, request: NetworkStart) -> Result<NetworkStarted, Error> {
        self.ask(|tx| Command::Start(request, tx))?
    }
    fn read(&self, request: NetworkRead) -> Result<NetworkReply, Error> {
        self.ask(|tx| Command::Read(request, tx))?
    }
    fn cancel(&self, request: NetworkCancel) -> Result<NetworkComplete, Error> {
        self.ask(|tx| Command::Cancel(request, tx))?
    }
    fn cancel_epoch(&self, request: NetworkEpoch) -> Result<Vec<NetworkComplete>, Error> {
        self.ask(|tx| Command::CancelEpoch(request, tx))
    }
    fn subresource(&self, request: NetworkStart) -> Result<NetworkStarted, Error> {
        self.ask(|tx| Command::Subresource(request, tx))?
    }
    fn document_cookie(&self, request: DocumentCookie) -> Result<String, Error> {
        self.ask(|tx| Command::DocumentCookie(request, tx))?
    }
}
pub fn serve(connection: &Arc<Connection>) {
    connection.bind(10, NetworkReceiverV2(NetworkEndpointV2::new()));
}

/// Browser-owned session. The caller supplies a nonzero browser-minted client
/// identity; a renderer cannot create one through the driver adapter.
pub struct NetworkSession {
    host: crate::ProcessHost,
    remote: NetworkProxyV2,
    client_id: u64,
    epoch: u64,
    timeout: Duration,
}
pub struct NetworkResponse {
    pub head: NetworkHead,
    pub body: Vec<u8>,
}
/// Convenience for the driver's ordinary GET resource loaders. The browser
/// still supplies the trusted context and can use `NetworkStart` directly for
/// navigation, POST, custom headers, or other Fetch modes.
pub fn get_request(
    url: impl Into<String>,
    base_url: impl Into<String>,
    destination: impl Into<String>,
    response_limit: u32,
    context: NetworkContext,
) -> NetworkStart {
    NetworkStart {
        version: NETWORK_VERSION,
        client_id: 0, // overwritten by NetworkSession
        epoch: 0,     // overwritten by NetworkSession
        url: url.into(),
        base_url: base_url.into(),
        method: "GET".into(),
        headers: Vec::new(),
        body_present: false,
        body: ByteBuf(Vec::new()),
        destination: destination.into(),
        mode: "cors".into(),
        credentials: "same-origin".into(),
        redirect: "follow".into(),
        cache: "default".into(),
        referrer: "about:client".into(),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        response_limit,
        navigation_reload_or_history: false,
        navigation_client_reserved_id: None,
        context,
    }
}
impl NetworkSession {
    /// Browser-owned synchronous document.cookie access in the network process.
    pub fn document_cookie(&self, mut request: DocumentCookie) -> Result<String, Error> {
        request.client_id = self.client_id;
        request.epoch = self.epoch;
        self.remote.document_cookie(request)
    }
    pub fn launch(
        exe: &std::path::Path,
        client_id: u64,
        timeout: Duration,
    ) -> Result<Self, crate::Error> {
        if client_id == 0 || timeout.is_zero() {
            return Err(crate::Error::Limit);
        }
        let mut host = crate::ProcessHost::spawn(
            exe,
            crate::Role::Network,
            crate::Capabilities(BTreeMap::new()),
            crate::RestartPolicy::Never,
        )?;
        let ready_deadline = Instant::now() + timeout;
        loop {
            let remaining = ready_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let _ = host.kill();
                return Err(crate::Error::Ipc(Error::Timeout));
            }
            match host.ping(remaining) {
                Ok(()) => break,
                // The child replies to the bootstrap before its Connection
                // binds interfaces. Retry only this pre-ready response; the
                // V2 interface is bound before the ping responder.
                Err(crate::Error::Ipc(Error::Remote(message))) if message == "UnknownInterface" => {
                    std::thread::yield_now();
                }
                Err(error) => {
                    let _ = host.kill();
                    return Err(error);
                }
            }
        }
        let remote = NetworkProxyV2::new(host.connection.clone(), timeout);
        Ok(Self {
            host,
            remote,
            client_id,
            epoch: 1,
            timeout,
        })
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn begin_navigation(&mut self) -> Result<u64, Error> {
        let old = self.epoch;
        let next = old
            .checked_add(1)
            .ok_or_else(|| invalid("epoch exhausted"))?;
        self.remote.cancel_epoch(NetworkEpoch {
            client_id: self.client_id,
            epoch: old,
        })?;
        self.epoch = next;
        Ok(next)
    }
    pub fn start(&self, mut request: NetworkStart) -> Result<NetworkStarted, Error> {
        request.version = NETWORK_VERSION;
        request.client_id = self.client_id;
        request.epoch = self.epoch;
        self.remote.start(request)
    }
    /// The browser's HTML navigation algorithm issues a new manual redirect
    /// hop. It stays in the current navigation epoch; only begin_navigation
    /// advances that epoch and cancels sibling requests.
    pub fn restart_navigation(
        &self,
        previous: &NetworkStarted,
        head: &NetworkHead,
        request: NetworkStart,
    ) -> Result<NetworkStarted, Error> {
        if (previous.client_id, previous.epoch) != (self.client_id, self.epoch)
            || (head.id, head.client_id, head.epoch)
                != (previous.id, previous.client_id, previous.epoch)
            || !matches!(head.status, 301 | 302 | 303 | 307 | 308)
            || !head
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("location") && !h.value.is_empty())
            || request.mode != "navigate"
            || request.redirect != "manual"
            || !(request.destination == "document" && request.context.top_level_navigation
                || request.destination == "frame" && !request.context.top_level_navigation)
        {
            return Err(invalid("invalid navigation restart"));
        }
        // A streamed redirect may still have an active body. A fully fetched
        // redirect has already been retired; both are safe to supersede.
        let _ = self.cancel(previous);
        self.start(request)
    }
    pub fn read(&self, started: &NetworkStarted, credit: u32) -> Result<NetworkReply, Error> {
        if (started.client_id, started.epoch) != (self.client_id, self.epoch) {
            return Err(invalid("stale request"));
        }
        let result = self.remote.read(NetworkRead {
            id: started.id,
            client_id: self.client_id,
            epoch: self.epoch,
            credit,
        });
        if matches!(&result, Err(Error::Timeout | Error::Disconnected)) {
            let _ = self.cancel(started);
        }
        result
    }
    pub fn cancel(&self, started: &NetworkStarted) -> Result<NetworkComplete, Error> {
        if started.client_id != self.client_id {
            return Err(invalid("request ownership"));
        }
        self.remote.cancel(NetworkCancel {
            id: started.id,
            client_id: self.client_id,
            epoch: started.epoch,
        })
    }
    /// Collect a response while preserving the full head and bounded body.
    /// Streaming callers use start/read directly and can consume each chunk.
    pub fn fetch(&self, request: NetworkStart) -> Result<NetworkResponse, Error> {
        let response_limit = request.response_limit as usize;
        let started = self.start(request)?;
        self.collect_response(started, response_limit)
    }
    /// Browser-only subresource fetch. The network role applies Fetch's
    /// destination policy before returning its internal decoded body; this
    /// method must never be used for a JS-visible Response.
    pub fn fetch_subresource(&self, mut request: NetworkStart) -> Result<NetworkResponse, Error> {
        if !matches!(
            request.destination.as_str(),
            "image" | "font" | "style" | "script"
        ) {
            return Err(invalid("invalid subresource destination"));
        }
        let response_limit = request.response_limit as usize;
        request.version = NETWORK_VERSION;
        request.client_id = self.client_id;
        request.epoch = self.epoch;
        let started = self.remote.subresource(request)?;
        self.collect_response(started, response_limit)
    }
    fn collect_response(
        &self,
        started: NetworkStarted,
        response_limit: usize,
    ) -> Result<NetworkResponse, Error> {
        let deadline = Instant::now() + self.timeout;
        let mut head = None;
        let mut body = Vec::new();
        loop {
            if Instant::now() >= deadline {
                let _ = self.cancel(&started);
                return Err(Error::Timeout);
            }
            let reply = match self.read(&started, NETWORK_CHUNK) {
                Ok(reply) => reply,
                Err(error) => {
                    let _ = self.cancel(&started);
                    return Err(error);
                }
            };
            match reply {
                NetworkReply::Pending(()) => std::thread::sleep(Duration::from_millis(1)),
                NetworkReply::Head(value) => {
                    if head.is_some()
                        || (value.id, value.client_id, value.epoch)
                            != (started.id, self.client_id, self.epoch)
                    {
                        let _ = self.cancel(&started);
                        return Err(invalid("invalid response head"));
                    }
                    head = Some(value);
                }
                NetworkReply::Body(chunk) => {
                    if head.is_none()
                        || (chunk.id, chunk.epoch) != (started.id, self.epoch)
                        || body
                            .len()
                            .checked_add(chunk.bytes.0.len())
                            .is_none_or(|n| n > response_limit)
                    {
                        let _ = self.cancel(&started);
                        return Err(invalid("invalid response body"));
                    }
                    body.extend_from_slice(&chunk.bytes.0);
                }
                NetworkReply::Complete(done) => {
                    if (done.id, done.epoch) != (started.id, self.epoch) {
                        return Err(invalid("invalid completion"));
                    }
                    if let Some(error) = done.error {
                        return Err(Error::Remote(error));
                    }
                    return Ok(NetworkResponse {
                        head: head.ok_or_else(|| invalid("missing head"))?,
                        body,
                    });
                }
            }
        }
    }
    pub fn fetch_get(
        &self,
        url: impl Into<String>,
        base_url: impl Into<String>,
        destination: impl Into<String>,
        response_limit: u32,
        context: NetworkContext,
    ) -> Result<NetworkResponse, Error> {
        self.fetch(get_request(
            url,
            base_url,
            destination,
            response_limit,
            context,
        ))
    }
    pub fn crashed(&mut self) -> Result<bool, crate::Error> {
        self.host.crashed()
    }
    pub fn shutdown(&mut self) -> Result<(), crate::Error> {
        let _ = self.remote.cancel_epoch(NetworkEpoch {
            client_id: self.client_id,
            epoch: self.epoch,
        });
        self.host.kill()
    }
}
impl Drop for NetworkSession {
    fn drop(&mut self) {
        let _ = self.remote.cancel_epoch(NetworkEpoch {
            client_id: self.client_id,
            epoch: self.epoch,
        });
        let _ = self.host.kill();
    }
}
