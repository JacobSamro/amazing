//! Native XMLHttpRequest algorithms, pinned to whatwg/xhr c3476b46f024cfc988f47d8d82118f3bcba562e9.
//! The embedder dispatches returned events and calls `advance`/`pull_response`.
//! No JS objects, DOM events, or scheduler are created in this crate.
use crate::api::{
    byte_string, fetch_api, parse_json, usv_string, ApiError, ApiHeaders, ApiRequest,
    ApiRequestInit, ApiRequestInput, ApiResponse, Blob, BodyInput, HeaderGuard, JsonValue,
    PendingFetchApi,
};
use crate::{FetchError, HttpUrl, NetworkOrCache, Origin, Site};
mod streaming;
pub use streaming::XhrServiceRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum XhrState {
    Unsent = 0,
    Opened = 1,
    HeadersReceived = 2,
    Loading = 3,
    Done = 4,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhrResponseType {
    Default,
    Text,
    Json,
    ArrayBuffer,
    Document,
    Blob,
}
#[derive(Debug, Clone, PartialEq)]
pub enum XhrResponseValue {
    Text(String),
    Json(Option<JsonValue>),
    ArrayBuffer(Vec<u8>),
    Blob(Blob),
    Null,
}
/// The document response bytes and metadata retained until the DOM embedder parses them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XhrDocumentSource {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub url: String,
    pub charset: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhrEventKind {
    ReadyStateChange,
    LoadStart,
    Progress,
    Load,
    Error,
    Abort,
    Timeout,
    LoadEnd,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XhrEventTarget {
    Request,
    Upload,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XhrEvent {
    /// Events from an earlier open/send generation must not be dispatched after re-entry.
    pub generation: u64,
    pub target: XhrEventTarget,
    pub kind: XhrEventKind,
    pub ready_state: XhrState,
    pub loaded: u64,
    pub total: u64,
    pub length_computable: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XhrContext {
    pub window: bool,
    pub document_fully_active: bool,
    /// The embedder decides whether main-thread synchronous XHR is disallowed.
    pub sync_disallowed: bool,
}
impl Default for XhrContext {
    fn default() -> Self {
        Self {
            window: true,
            document_fully_active: true,
            sync_disallowed: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XhrError {
    InvalidStateOpen,
    InvalidStateSend,
    InvalidStateHeader,
    InvalidStateCredentials,
    InvalidStateResponseType,
    InvalidStateResponseText,
    InvalidStateResponseXml,
    InvalidStateOverrideMimeType,
    SyntaxMethod,
    SyntaxUrl,
    SyntaxHeaderName,
    SyntaxHeaderValue,
    SecurityMethod,
    InvalidAccessSyncTimeout,
    InvalidAccessSyncResponseType,
    InvalidAccessSyncContext,
    ByteString,
    Network(FetchError),
    Timeout,
    Abort,
    UnsupportedBlob,
    UnsupportedDocument,
    UnsupportedUrlCredentials,
    UnsupportedFormData,
    BodyTooLarge,
    BodyRead,
}
impl XhrError {
    pub fn dom_exception_name(&self) -> &'static str {
        match self {
            Self::InvalidStateOpen
            | Self::InvalidStateSend
            | Self::InvalidStateHeader
            | Self::InvalidStateCredentials
            | Self::InvalidStateResponseType
            | Self::InvalidStateResponseText
            | Self::InvalidStateResponseXml
            | Self::InvalidStateOverrideMimeType => "InvalidStateError",
            Self::SyntaxMethod
            | Self::SyntaxUrl
            | Self::SyntaxHeaderName
            | Self::SyntaxHeaderValue => "SyntaxError",
            Self::SecurityMethod => "SecurityError",
            Self::InvalidAccessSyncTimeout
            | Self::InvalidAccessSyncResponseType
            | Self::InvalidAccessSyncContext => "InvalidAccessError",
            Self::ByteString => "TypeError",
            Self::Network(_) | Self::UnsupportedUrlCredentials | Self::BodyRead => "NetworkError",
            Self::Timeout => "TimeoutError",
            Self::Abort => "AbortError",
            Self::UnsupportedBlob | Self::UnsupportedDocument | Self::UnsupportedFormData => {
                "NotSupportedError"
            }
            Self::BodyTooLarge => "QuotaExceededError",
        }
    }
}
impl From<ApiError> for XhrError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::UnsupportedFormData => Self::UnsupportedFormData,
            ApiError::UnsupportedBlob => Self::UnsupportedBlob,
            ApiError::BodyTooLarge => Self::BodyTooLarge,
            ApiError::BodyRead => Self::BodyRead,
            ApiError::ByteString => Self::ByteString,
            ApiError::Network(error) => Self::Network(error),
            ApiError::UrlCredentials => Self::UnsupportedUrlCredentials,
            _ => Self::Network(FetchError::InvalidBody),
        }
    }
}
/// The embedding supplies its existing charset decoder bridge (e.g. the HTML
/// decoder), so this lower crate does not depend on the HTML/DOM graph.
pub trait XhrTextDecoder {
    fn decode(&self, bytes: &[u8], charset: Option<&str>) -> String;
}

/// Actual URL-resolution base and native origin are separate. An about/file
/// base must not mint an HTTP origin or require a fabricated replacement URL.
pub struct XhrUrlContext {
    pub base_url: amazing_net::ParsedUrl,
    pub origin: Origin,
}

/// Each call returns a batch for the embedder to dispatch. Dispatch may re-enter
/// `open` or `abort`; the generation field lets the embedder discard stale events.
pub struct Xhr {
    state: XhrState,
    send_flag: bool,
    method: String,
    url: Option<HttpUrl>,
    origin: Option<Origin>,
    asynchronous: bool,
    timeout_ms: u32,
    with_credentials: bool,
    response_type: XhrResponseType,
    override_mime: Option<String>,
    request_headers: ApiHeaders,
    request: Option<ApiRequest>,
    native_client: Option<crate::service::ClientContext>,
    response: Option<ApiResponse>,
    stream_response: Option<streaming::StreamResponse>,
    stream_progress_ms: u64,
    stream_upload_ms: u64,
    received: Vec<u8>,
    pending_body: Option<Vec<u8>>,
    pending_offset: usize,
    upload_has_listeners: bool,
    upload_complete: bool,
    upload_total: Option<u64>,
    started_ms: u64,
    generation: u64,
    last_error: Option<XhrError>,
    context: XhrContext,
}
impl Default for Xhr {
    fn default() -> Self {
        Self::new(XhrContext::default())
    }
}
impl Xhr {
    pub fn new(context: XhrContext) -> Self {
        Self {
            state: XhrState::Unsent,
            send_flag: false,
            method: String::new(),
            url: None,
            origin: None,
            asynchronous: true,
            timeout_ms: 0,
            with_credentials: false,
            response_type: XhrResponseType::Default,
            override_mime: None,
            request_headers: ApiHeaders::new(HeaderGuard::Request),
            request: None,
            native_client: None,
            response: None,
            stream_response: None,
            stream_progress_ms: 0,
            stream_upload_ms: 0,
            received: Vec::new(),
            pending_body: None,
            pending_offset: 0,
            upload_has_listeners: false,
            upload_complete: true,
            upload_total: None,
            started_ms: 0,
            generation: 0,
            last_error: None,
            context,
        }
    }
    pub fn state(&self) -> XhrState {
        self.state
    }
    pub fn send_invoked(&self) -> bool {
        self.send_flag
    }
    pub fn is_asynchronous(&self) -> bool {
        self.asynchronous
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_current_event(&self, event: &XhrEvent) -> bool {
        event.generation == self.generation
    }
    pub fn timeout(&self) -> u32 {
        self.timeout_ms
    }
    pub fn with_credentials(&self) -> bool {
        self.with_credentials
    }
    pub fn response_type(&self) -> XhrResponseType {
        self.response_type
    }
    pub fn last_error(&self) -> Option<&XhrError> {
        self.last_error.as_ref()
    }
    pub fn request_headers(&self) -> &ApiHeaders {
        &self.request_headers
    }
    pub fn is_data_request(&self) -> bool {
        self.url.as_ref().is_some_and(|url| url.as_parsed().scheme() == "data")
    }
    pub fn set_upload_listeners(&mut self, present: bool) {
        self.upload_has_listeners = present;
    }

    /// `method` is WebIDL ByteString, `url` is USVString; `base` and `origin`
    /// are the embedding's relevant settings object. Omitted async is true.
    pub fn open(
        &mut self,
        method: &[u16],
        url: &[u16],
        asynchronous: bool,
        base: &HttpUrl,
        origin: Origin,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        self.open_with_url_context(method,url,asynchronous,
            &XhrUrlContext{base_url:base.as_parsed().clone(),origin})
    }
    pub fn open_with_url_context(&mut self,method:&[u16],url:&[u16],asynchronous:bool,
        context:&XhrUrlContext)->Result<Vec<XhrEvent>,XhrError> {
        if self.context.window && !self.context.document_fully_active {
            return Err(XhrError::InvalidStateOpen);
        }
        let method = byte_string(method).map_err(|_| XhrError::ByteString)?;
        if !crate::headers::token(&method) {
            return Err(XhrError::SyntaxMethod);
        }
        if ["CONNECT", "TRACE", "TRACK"]
            .iter()
            .any(|m| method.eq_ignore_ascii_case(m))
        {
            return Err(XhrError::SecurityMethod);
        }
        let uppercase = method.to_ascii_uppercase();
        let method = if matches!(
            uppercase.as_str(),
            "DELETE" | "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT"
        ) {
            uppercase
        } else {
            method
        };
        let parsed = context.base_url.resolve(&usv_string(url))
            .map_err(|_|XhrError::SyntaxUrl)?;
        let parsed=HttpUrl::from_parsed(parsed).map_err(|_|XhrError::SyntaxUrl)?;
        if !asynchronous && self.context.window {
            if self.timeout_ms != 0 {
                return Err(XhrError::InvalidAccessSyncTimeout);
            }
            if self.response_type != XhrResponseType::Default {
                return Err(XhrError::InvalidAccessSyncResponseType);
            }
            if self.context.sync_disallowed {
                return Err(XhrError::InvalidAccessSyncContext);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.send_flag = false;
        self.method = method;
        self.url = Some(parsed);
        self.origin = Some(context.origin.clone());
        self.asynchronous = asynchronous;
        self.request_headers = ApiHeaders::new(HeaderGuard::Request);
        self.request = None;
        self.native_client = None;
        self.response = None;
        self.stream_response = None;
        self.received.clear();
        self.pending_body = None;
        self.pending_offset = 0;
        self.upload_has_listeners = false;
        self.upload_complete = true;
        self.upload_total = None;
        self.last_error = None;
        if self.state != XhrState::Opened {
            self.state = XhrState::Opened;
            Ok(vec![self.event(
                XhrEventTarget::Request,
                XhrEventKind::ReadyStateChange,
                0,
                None,
            )])
        } else {
            Ok(Vec::new())
        }
    }
    pub fn set_request_header(&mut self, name: &[u16], value: &[u16]) -> Result<(), XhrError> {
        if self.state != XhrState::Opened || self.send_flag {
            return Err(XhrError::InvalidStateHeader);
        }
        let name = byte_string(name).map_err(|_| XhrError::ByteString)?;
        let value = byte_string(value).map_err(|_| XhrError::ByteString)?;
        // ApiHeaders normalizes and validates before its request guard ignores
        // forbidden names. A second set combines duplicate author values.
        let mut candidate = self.request_headers.clone();
        candidate
            .append_string(&name, &value)
            .map_err(map_header_error)?;
        if candidate.raw_entries().len() == self.request_headers.raw_entries().len() {
            return Ok(());
        }
        let old = self
            .request_headers
            .get_string(&name)
            .map_err(map_header_error)?;
        let normalized = candidate
            .raw_entries()
            .last()
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        if let Some(old) = old {
            self.request_headers
                .replace_combined_string(&name, format!("{old}, {normalized}"))
                .map_err(map_header_error)
        } else {
            self.request_headers
                .set_string(&name, normalized)
                .map_err(map_header_error)
        }
    }
    pub fn set_timeout(&mut self, millis: u32) -> Result<(), XhrError> {
        if !self.asynchronous && self.context.window {
            return Err(XhrError::InvalidAccessSyncTimeout);
        }
        self.timeout_ms = millis;
        Ok(())
    }
    pub fn set_with_credentials(&mut self, value: bool) -> Result<(), XhrError> {
        if !matches!(self.state, XhrState::Unsent | XhrState::Opened) || self.send_flag {
            return Err(XhrError::InvalidStateCredentials);
        }
        self.with_credentials = value;
        Ok(())
    }
    pub fn set_response_type(&mut self, value: XhrResponseType) -> Result<(), XhrError> {
        if value == XhrResponseType::Document && !self.context.window {
            return Ok(());
        }
        if matches!(self.state, XhrState::Loading | XhrState::Done) {
            return Err(XhrError::InvalidStateResponseType);
        }
        if !self.asynchronous && self.context.window {
            return Err(XhrError::InvalidAccessSyncResponseType);
        }
        self.response_type = value;
        Ok(())
    }
    pub fn override_mime_type(&mut self, mime: &str) -> Result<(), XhrError> {
        if matches!(self.state, XhrState::Loading | XhrState::Done) {
            return Err(XhrError::InvalidStateOverrideMimeType);
        }
        self.override_mime = Some(if valid_mime(mime) {
            mime.trim().to_owned()
        } else {
            "application/octet-stream".into()
        });
        Ok(())
    }
    /// Starts a request. `now_ms` is the embedding's monotonic clock. For an
    /// async request this only produces loadstart events; `advance` runs Fetch.
    pub fn send(
        &mut self,
        body: Option<BodyInput>,
        now_ms: u64,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        if self.state != XhrState::Opened || self.send_flag {
            return Err(XhrError::InvalidStateSend);
        }
        let body = if matches!(self.method.as_str(), "GET" | "HEAD") {
            None
        } else {
            body
        };
        let upload_total = body.as_ref().and_then(body_length);
        let utf8_body = matches!(
            body.as_ref(),
            Some(BodyInput::String(_) | BodyInput::UrlSearchParams(_) | BodyInput::Document { .. })
        );
        let mut init = ApiRequestInit::default();
        init.method = Some(self.method.encode_utf16().collect());
        init.headers = Some(self.request_headers.clone());
        if utf8_body {
            if let Some(headers) = &mut init.headers {
                if let Some(original) = headers
                    .get_string("content-type")
                    .map_err(map_header_error)?
                {
                    if let Some(rewritten) = force_utf8_charset(&original) {
                        headers
                            .set_string("content-type", &rewritten)
                            .map_err(map_header_error)?;
                    }
                }
            }
        }
        init.allow_url_credentials = true;
        init.credentials = Some(
            if self.with_credentials {
                "include"
            } else {
                "same-origin"
            }
            .into(),
        );
        if let Some(body) = body {
            if body.is_stream() {
                init.duplex = Some("half".into());
            }
            init.body = Some(Some(body));
        }
        let url = self.url.as_ref().ok_or(XhrError::InvalidStateSend)?;
        let units: Vec<_> = url.serialize().encode_utf16().collect();
        let mut request = ApiRequest::new(
            ApiRequestInput::Url(&units),
            init,
            url,
            self.origin.clone().ok_or(XhrError::InvalidStateSend)?,
        )
        .map_err(XhrError::from)?;
        request.set_use_cors_preflight(self.upload_has_listeners);
        self.generation = self.generation.wrapping_add(1);
        self.send_flag = true;
        self.started_ms = now_ms;
        self.request = Some(request);
        self.upload_total = upload_total;
        self.upload_complete = self
            .request
            .as_ref()
            .is_some_and(|r| !r.body.shared_present());
        let mut events = Vec::new();
        if self.asynchronous {
            events.push(self.event(XhrEventTarget::Request, XhrEventKind::LoadStart, 0, None));
            if !self.upload_complete && self.upload_has_listeners {
                events.push(self.event(
                    XhrEventTarget::Upload,
                    XhrEventKind::LoadStart,
                    0,
                    upload_total,
                ));
            }
        }
        Ok(events)
    }
    /// Blocking synchronous variant for worker or permitted document contexts.
    /// The embedder supplies the same transport used by asynchronous `advance`.
    pub fn send_blocking(
        &mut self,
        body: Option<BodyInput>,
        network: &mut impl NetworkOrCache,
        site: &Site,
        now_ms: u64,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        if self.asynchronous {
            return Err(XhrError::InvalidStateSend);
        }
        let mut events = self.send(body, now_ms)?;
        events.extend(self.advance(network, site, now_ms)?);
        Ok(events)
    }
    /// Performs the existing Fetch policy/transport path once. The response is
    /// buffered by that path; subsequent `pull_response` calls expose its bytes
    /// and events incrementally without dispatching DOM events here.
    pub fn advance(
        &mut self,
        network: &mut impl NetworkOrCache,
        site: &Site,
        now_ms: u64,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        if self.state != XhrState::Opened || !self.send_flag {
            return Err(XhrError::InvalidStateSend);
        }
        if self.expired(now_ms) {
            return self.fail(XhrError::Timeout);
        }
        let request = self.request.as_mut().ok_or(XhrError::InvalidStateSend)?;
        let result = fetch_api(request, network, site, &false);
        if self.expired(now_ms) {
            return self.fail(XhrError::Timeout);
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => return self.fail(XhrError::from(error)),
        };
        self.install_response(response)
    }
    /// Starts a cooperative Fetch request after `send`. The event-loop driver
    /// owns the pending handle and feeds it dispatcher events via
    /// `on_dispatch_event`; the blocking `advance` path remains available.
    pub fn start_dispatch<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        &mut self,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now_ms: u64,
        now: std::time::Instant,
    ) -> Result<PendingFetchApi, XhrError> {
        if self.state != XhrState::Opened || !self.send_flag {
            return Err(XhrError::InvalidStateSend);
        }
        if self.expired(now_ms) {
            return Err(XhrError::Timeout);
        }
        PendingFetchApi::start(
            self.request.as_mut().ok_or(XhrError::InvalidStateSend)?,
            dispatcher,
            now,
        )
        .map_err(XhrError::from)
    }
    pub fn on_dispatch_event<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        &mut self,
        pending: &mut PendingFetchApi,
        event: &amazing_net::dispatch::NetEvent,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now_ms: u64,
        now: std::time::Instant,
    ) -> Result<Vec<XhrEvent>, XhrError> {
        if self.expired(now_ms) {
            pending.cancel(dispatcher);
            return self.fail(XhrError::Timeout);
        }
        match pending.on_event(event, dispatcher, now) {
            None => Ok(Vec::new()),
            Some(Ok(response)) => self.install_response(response),
            Some(Err(error)) => self.fail(XhrError::from(error)),
        }
    }
    /// Call on ticks without network events so a stalled request still times
    /// out at the XHR deadline.
    pub fn check_dispatch_timeout<
        R: amazing_net::dispatch::Resolver,
        C: amazing_net::dispatch::Connector,
    >(
        &mut self,
        pending: &PendingFetchApi,
        dispatcher: &mut amazing_net::dispatch::Dispatcher<R, C>,
        now_ms: u64,
    ) -> Result<Option<Vec<XhrEvent>>, XhrError> {
        if self.expired(now_ms) {
            pending.cancel(dispatcher);
            return self.fail(XhrError::Timeout).map(Some);
        }
        Ok(None)
    }
    fn install_response(&mut self, mut response: ApiResponse) -> Result<Vec<XhrEvent>, XhrError> {
        let mut events = self.upload_success_events();
        let bytes = match response.body.bytes() {
            Ok(bytes) => bytes,
            Err(error) => return self.fail(XhrError::from(error)),
        };
        self.response = Some(response);
        if self.asynchronous {
            self.state = XhrState::HeadersReceived;
            events.push(self.event(
                XhrEventTarget::Request,
                XhrEventKind::ReadyStateChange,
                0,
                None,
            ));
            self.pending_body = Some(bytes);
            if self.pending_body.as_ref().is_some_and(Vec::is_empty) {
                events.extend(self.complete());
            }
        } else {
            self.received = bytes;
            events.extend(self.complete());
        }
        Ok(events)
    }
    /// Pulls at most `limit` bytes from a buffered Fetch response. The
    /// embedding may call `abort` or `open` between pulls and event batches.
    pub fn pull_response(&mut self, limit: usize, now_ms: u64) -> Result<Vec<XhrEvent>, XhrError> {
        if self.state != XhrState::HeadersReceived && self.state != XhrState::Loading {
            return Err(XhrError::InvalidStateSend);
        }
        if self.expired(now_ms) {
            return self.fail(XhrError::Timeout);
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let bytes = self
            .pending_body
            .as_ref()
            .ok_or(XhrError::InvalidStateSend)?;
        let end = self.pending_offset.saturating_add(limit).min(bytes.len());
        self.received
            .extend_from_slice(&bytes[self.pending_offset..end]);
        self.pending_offset = end;
        let mut events = Vec::new();
        if self.state == XhrState::HeadersReceived {
            self.state = XhrState::Loading;
        }
        events.push(self.event(
            XhrEventTarget::Request,
            XhrEventKind::ReadyStateChange,
            self.received.len() as u64,
            self.response_length(),
        ));
        events.push(self.event(
            XhrEventTarget::Request,
            XhrEventKind::Progress,
            self.received.len() as u64,
            self.response_length(),
        ));
        if self.pending_offset == bytes.len() {
            events.extend(self.complete());
        }
        Ok(events)
    }
    pub fn abort(&mut self) -> Vec<XhrEvent> {
        // XHR spec abort(): an OPENED request with no send() flag and an UNSENT
        // request are no-ops; an active request runs the request error steps
        // (state DONE, readystatechange + abort + loadend) and is then left
        // UNSENT; a DONE request is reset to UNSENT without any event.
        let active = match self.state {
            XhrState::Opened => self.send_flag,
            XhrState::HeadersReceived | XhrState::Loading => true,
            _ => false,
        };
        if active {
            // Events already queued for the previous generation must not fire
            // after the abort re-enters from a readystatechange handler.
            self.generation = self.generation.wrapping_add(1);
            let events = self.fail(XhrError::Abort).unwrap_or_default();
            self.state = XhrState::Unsent;
            self.send_flag = false;
            events
        } else if self.state == XhrState::Done {
            self.generation = self.generation.wrapping_add(1);
            self.state = XhrState::Unsent;
            self.send_flag = false;
            self.response = None;
        self.stream_response = None;
            self.received.clear();
            self.pending_body = None;
            self.pending_offset = 0;
            self.request = None;
        self.native_client = None;
            Vec::new()
        } else {
            Vec::new()
        }
    }
    /// Native relevant-settings origin retained by open/send, independent of
    /// response URL and the caller of a responseXML getter.
    pub fn response_document_origin(&self)->Option<&Origin> {self.origin.as_ref()}
    pub fn response_url(&self) -> &str {
        self.response.as_ref().map(|r| r.url.split('#').next().unwrap_or(""))
            .or_else(|| self.stream_response.as_ref().map(|r| r.url.as_str())).unwrap_or("")
    }
    pub fn status(&self) -> u16 {
        self.response.as_ref().map(|r| r.status)
            .or_else(|| self.stream_response.as_ref().map(|r| r.head.status)).unwrap_or(0)
    }
    pub fn status_text(&self) -> &str {
        self.response.as_ref().map(|r| r.status_text.as_str())
            .or_else(|| self.stream_response.as_ref().map(|r| r.head.reason.as_str())).unwrap_or("")
    }
    pub fn get_response_header(&self, name: &[u16]) -> Result<Option<String>, XhrError> {
        let name = byte_string(name).map_err(|_| XhrError::ByteString)?;
        if self.state < XhrState::HeadersReceived {
            return Ok(None);
        }
        Ok(self.response_headers().and_then(|headers| headers.get_string(&name).ok().flatten()))
    }
    pub fn get_all_response_headers(&self) -> String {
        if self.state < XhrState::HeadersReceived {
            return String::new();
        }
        let Some(headers) = self.response_headers() else { return String::new(); };
        let mut headers = headers.sorted_entries();
        headers.retain(|(n, _)| !matches!(n.as_str(), "set-cookie" | "set-cookie2"));
        headers.sort_by(|a, b| a.0.to_ascii_uppercase().cmp(&b.0.to_ascii_uppercase()));
        headers
            .into_iter()
            .map(|(n, v)| format!("{n}: {v}\r\n"))
            .collect()
    }
    pub fn response_text(&self, decoder: &impl XhrTextDecoder) -> Result<String, XhrError> {
        if !matches!(
            self.response_type,
            XhrResponseType::Default | XhrResponseType::Text
        ) {
            return Err(XhrError::InvalidStateResponseText);
        }
        if !matches!(self.state, XhrState::Loading | XhrState::Done) {
            return Ok(String::new());
        }
        let charset = self.final_charset();
        Ok(decoder.decode(&self.received, charset.as_deref()))
    }
    /// Returns the source for responseXML/responseType=document. XML parse errors
    /// are handled by the DOM embedder and become a null response.
    pub fn document_source(&self) -> Option<XhrDocumentSource> {
        if self.state != XhrState::Done { return None; }
        if !self.response_body_present() { return None; }
        let header = self.response_headers()?.get_string("content-type").ok().flatten();
        let mime = self.override_mime.as_deref().or(header.as_deref())
            .and_then(crate::mime::MimeType::parse)
            .unwrap_or_else(|| crate::mime::MimeType::parse("text/xml").expect("static MIME"));
        let html = mime.type_ == "text" && mime.subtype == "html";
        let xml = matches!(mime.type_.as_str(), "text" | "application") && mime.subtype == "xml"
            || mime.subtype.ends_with("+xml");
        if !xml && (!html || self.response_type != XhrResponseType::Document) { return None; }
        Some(XhrDocumentSource {
            bytes: self.received.clone(),
            content_type: mime.essence(),
            url: self.response_document_url().to_owned(),
            charset: self.final_charset(),
        })
    }
    pub fn response(&self, decoder: &impl XhrTextDecoder) -> Result<XhrResponseValue, XhrError> {
        match self.response_type {
            XhrResponseType::Default | XhrResponseType::Text => {
                self.response_text(decoder).map(XhrResponseValue::Text)
            }
            XhrResponseType::Document if self.state == XhrState::Done => {
                Err(XhrError::UnsupportedDocument)
            }
            XhrResponseType::Blob if self.state == XhrState::Done => {
                if !self.has_response() { return Ok(XhrResponseValue::Null); }
                let header = self.response_headers().and_then(|headers|
                    headers.get_string("content-type").ok().flatten());
                let content_type = self.override_mime.as_deref().or(header.as_deref());
                let content_type = content_type.and_then(crate::mime::MimeType::parse)
                    .map_or_else(|| "application/octet-stream".to_owned(), |mime| mime.serialize());
                Ok(XhrResponseValue::Blob(Blob::from_bytes(self.received.clone(), &content_type)))
            }
            XhrResponseType::Document | XhrResponseType::Blob => Ok(XhrResponseValue::Null),
            XhrResponseType::Json if self.state != XhrState::Done => Ok(XhrResponseValue::Null),
            XhrResponseType::ArrayBuffer if self.state != XhrState::Done => {
                Ok(XhrResponseValue::Null)
            }
            XhrResponseType::Json => Ok(XhrResponseValue::Json(parse_json(&self.received).ok())),
            XhrResponseType::ArrayBuffer if !self.has_response() => Ok(XhrResponseValue::Null),
            XhrResponseType::ArrayBuffer => {
                Ok(XhrResponseValue::ArrayBuffer(self.received.clone()))
            }
        }
    }
    fn final_charset(&self) -> Option<String> {
        let header = self.response_headers()?.get_string("content-type").ok().flatten();
        self.override_mime
            .as_deref()
            .and_then(charset_parameter)
            .or_else(|| header.as_deref().and_then(charset_parameter))
    }
    fn response_length(&self) -> Option<u64> {
        let value=self.response_headers()?.get_string("content-length").ok().flatten()?;
        extract_response_length(&value)
    }
    fn expired(&self, now_ms: u64) -> bool {
        self.timeout_ms != 0 && now_ms.saturating_sub(self.started_ms) >= u64::from(self.timeout_ms)
    }
    fn complete(&mut self) -> Vec<XhrEvent> {
        let mut events = Vec::new();
        let total = self.response_length();
        if self.asynchronous {
            events.push(self.event(
                XhrEventTarget::Request,
                XhrEventKind::Progress,
                self.received.len() as u64,
                total,
            ));
        }
        self.state = XhrState::Done;
        self.send_flag = false;
        self.pending_body = None;
        self.request = None;
        self.native_client = None;
        events.push(self.event(
            XhrEventTarget::Request,
            XhrEventKind::ReadyStateChange,
            self.received.len() as u64,
            total,
        ));
        events.push(self.event(
            XhrEventTarget::Request,
            XhrEventKind::Load,
            self.received.len() as u64,
            total,
        ));
        events.push(self.event(
            XhrEventTarget::Request,
            XhrEventKind::LoadEnd,
            self.received.len() as u64,
            total,
        ));
        events
    }
    fn upload_success_events(&mut self) -> Vec<XhrEvent> {
        if self.upload_complete {
            return Vec::new();
        }
        self.upload_complete = true;
        if !self.asynchronous || !self.upload_has_listeners {
            return Vec::new();
        }
        let loaded = self.upload_total.unwrap_or(0);
        [
            XhrEventKind::Progress,
            XhrEventKind::Load,
            XhrEventKind::LoadEnd,
        ]
        .into_iter()
        .map(|kind| self.event(XhrEventTarget::Upload, kind, loaded, self.upload_total))
        .collect()
    }
    fn fail(&mut self, error: XhrError) -> Result<Vec<XhrEvent>, XhrError> {
        self.last_error = Some(error.clone());
        self.state = XhrState::Done;
        self.send_flag = false;
        self.request = None;
        self.native_client = None;
        self.response = None;
        self.stream_response = None;
        self.received.clear();
        self.pending_body = None;
        self.pending_offset = 0;
        if !self.asynchronous {
            return Err(error);
        }
        let mut events = vec![self.event(
            XhrEventTarget::Request,
            XhrEventKind::ReadyStateChange,
            0,
            None,
        )];
        let kind = match error {
            XhrError::Timeout => XhrEventKind::Timeout,
            XhrError::Abort => XhrEventKind::Abort,
            _ => XhrEventKind::Error,
        };
        if !self.upload_complete {
            self.upload_complete = true;
            if self.upload_has_listeners {
                events.push(self.event(XhrEventTarget::Upload, kind, 0, None));
                events.push(self.event(XhrEventTarget::Upload, XhrEventKind::LoadEnd, 0, None));
            }
        }
        events.push(self.event(XhrEventTarget::Request, kind, 0, None));
        events.push(self.event(XhrEventTarget::Request, XhrEventKind::LoadEnd, 0, None));
        Ok(events)
    }
    fn event(
        &self,
        target: XhrEventTarget,
        kind: XhrEventKind,
        loaded: u64,
        total: Option<u64>,
    ) -> XhrEvent {
        XhrEvent {
            generation: self.generation,
            target,
            kind,
            ready_state: self.state,
            loaded,
            total: total.unwrap_or(0),
            length_computable: total.is_some_and(|length| length != 0),
        }
    }
}
fn map_header_error(error: ApiError) -> XhrError {
    match error {
        ApiError::InvalidHeaderName => XhrError::SyntaxHeaderName,
        ApiError::InvalidHeaderValue => XhrError::SyntaxHeaderValue,
        ApiError::ByteString => XhrError::ByteString,
        _ => XhrError::SyntaxHeaderValue,
    }
}
fn body_length(body: &BodyInput) -> Option<u64> {
    match body {
        BodyInput::Bytes(v) => Some(v.len() as u64),
        BodyInput::Document { bytes, .. } => Some(bytes.len() as u64),
        BodyInput::String(v) => Some(usv_string(v).len() as u64),
        BodyInput::UrlSearchParams(_)
        | BodyInput::Stream(_)
        | BodyInput::FormData(_)
        | BodyInput::Blob(_) => None,
    }
}
fn valid_mime(mime: &str) -> bool {
    crate::MimeType::parse(mime).is_some()
}
fn charset_parameter(mime: &str) -> Option<String> {
    crate::MimeType::parse(mime)?
        .parameter("charset")
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
fn force_utf8_charset(mime: &str) -> Option<String> {
    let mut record = crate::MimeType::parse(mime)?;
    let charset = record.parameter("charset")?;
    if charset.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    record.set_parameter("charset", "UTF-8");
    Some(record.serialize())
}

// Fetch §3.4: compare the decoded/split values before decimal interpretation.
// Identical duplicate strings are accepted; numeric equivalence is insufficient.
fn extract_response_length(value:&str)->Option<u64> {
    let mut values=value.split(',').map(|value|value.trim_matches([' ', '\t']));
    let candidate=values.next()?;
    if candidate.is_empty() || !candidate.bytes().all(|byte|byte.is_ascii_digit())
        || values.any(|value|value!=candidate) {return None;}
    candidate.parse().ok()
}
#[cfg(test)]
mod response_length_tests {
    use super::extract_response_length;
    #[test]
    fn full_content_length_extraction_matrix() {
        for (value,expected) in [("3",Some(3)),(" 3\t, 3 ",Some(3)),("03,03",Some(3)),
            ("0",Some(0)),("",None),("3,4",None),("03,3",None),("+3",None),("-1",None),
            ("3,",None),("3.0",None),("٣",None),("\"3\"",None),("18446744073709551616",None)] {
            assert_eq!(extract_response_length(value),expected,"{value:?}");
        }
    }
}
