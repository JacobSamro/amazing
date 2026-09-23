//! Cooperative HTTP/1.1 dispatcher. One call advances each request by one
//! state, with socket I/O capped by the caller and configured byte budget.
//! DNS is a blocking seam; std-only connect_timeout is a bounded blocking seam.
use crate::{
    coding::stream::{decode_layer, CapSource, CountSource, IoSource, Source},
    fetch::FetchOptions,
    headers::Headers,
    http, http2, NetError, ParsedUrl,
};
mod websocket;
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap},
    io::{self, Cursor, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    rc::Rc,
    time::{Duration, Instant},
};
use websocket::WebSocketConnection;
pub use websocket::{WebSocketEvent, WebSocketId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

#[derive(Clone, Copy, Debug)]
pub struct DispatchLimits {
    pub max_in_flight: usize,
    pub max_per_origin: usize,
    pub bytes_per_step: usize,
    pub max_buffered_bytes: usize,
    pub connect_timeout: Duration,
}
impl Default for DispatchLimits {
    fn default() -> Self {
        Self {
            max_in_flight: 32,
            max_per_origin: 6,
            bytes_per_step: 64 * 1024,
            max_buffered_bytes: 4 * 1024 * 1024,
            connect_timeout: Duration::from_secs(3),
        }
    }
}

#[derive(Debug)]
pub enum DispatchError {
    TooManyInFlight,
    TooManyForOrigin,
    InvalidBudget,
    BufferLimit,
    Timeout,
    Network(NetError),
    UnsupportedProtocol,
}
impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DispatchError {}
impl From<NetError> for DispatchError {
    fn from(value: NetError) -> Self {
        Self::Network(value)
    }
}
impl From<io::Error> for DispatchError {
    fn from(value: io::Error) -> Self {
        Self::Network(NetError::Io(value))
    }
}

#[derive(Debug)]
pub enum NetEvent {
    Head {
        id: RequestId,
        url: String,
        status: u16,
        reason: String,
        headers: Headers,
    },
    Body {
        id: RequestId,
        bytes: Vec<u8>,
    },
    Done {
        id: RequestId,
    },
    Error {
        id: RequestId,
        error: DispatchError,
    },
    Aborted {
        id: RequestId,
    },
}

/// A synchronous resolver seam. The default uses std ToSocketAddrs and can
/// block during DNS; a future resolver can supply ready addresses instead.
pub trait Resolver {
    fn resolve(&mut self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>>;
}
pub struct StdResolver;
impl Resolver for StdResolver {
    fn resolve(&mut self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        (host, port).to_socket_addrs().map(Iterator::collect)
    }
}
/// Connect seam. The std implementation has a caller-set short blocking bound.
pub trait Connector {
    fn connect(&mut self, address: SocketAddr, timeout: Duration) -> io::Result<TcpStream>;
}
pub struct StdConnector;
impl Connector for StdConnector {
    fn connect(&mut self, address: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        TcpStream::connect_timeout(&address, timeout)
    }
}

enum Transport {
    Plain(TcpStream),
    Tls(amazing_net_tls::TlsStream),
}
impl Transport {
    fn read_step(&mut self, b: &mut [u8], budget: usize) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(b),
            Self::Tls(s) => s.read_step(b, budget),
        }
    }
    fn write_step(&mut self, b: &[u8], budget: usize) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(b),
            Self::Tls(s) => s.write_step(b, budget),
        }
    }
}

enum State {
    Resolve,
    WaitForOrigin,
    Connect(Vec<SocketAddr>),
    Handshake(amazing_net_tls::PendingTls),
    Write,
    Read,
    Emit,
    H2Start,
    H2Read,
}
struct H2Connection {
    transport: amazing_net_tls::TlsStream,
    client: http2::Client<http2::FrameIo>,
    inbound: Vec<u8>,
    outbound: Vec<u8>,
    written: usize,
}
impl H2Connection {
    fn new(
        transport: amazing_net_tls::TlsStream,
        buffer_limit: usize,
    ) -> Result<Self, DispatchError> {
        let h2_limit = buffer_limit.min(32 * 1024 * 1024);
        let limits = http2::Limits {
            body_bytes: h2_limit,
            buffered_body_bytes: h2_limit,
            ..http2::Limits::default()
        };
        let client = http2::Client::new_pending(http2::FrameIo::default(), limits)
            .map_err(|e| NetError::MalformedResponse(e.to_string()))?;
        let mut conn = Self {
            transport,
            client,
            inbound: Vec::new(),
            outbound: Vec::new(),
            written: 0,
        };
        conn.collect_output();
        Ok(conn)
    }
    fn collect_output(&mut self) {
        if let Some(io) = self.client.io_mut() {
            self.outbound.extend(io.take_output());
        }
    }
    fn step(&mut self, remaining: &mut usize, limit: usize) -> Result<(), DispatchError> {
        if *remaining == 0 {
            return Ok(());
        }
        self.collect_output();
        if self.outbound.len().saturating_add(self.inbound.len()) > limit {
            return Err(DispatchError::BufferLimit);
        }
        if self.written < self.outbound.len() {
            let end = self
                .outbound
                .len()
                .min(self.written.saturating_add(*remaining));
            match self
                .transport
                .write_step(&self.outbound[self.written..end], *remaining)
            {
                Ok(n) => {
                    self.written += n;
                    *remaining -= n;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
            if self.written == self.outbound.len() {
                self.outbound.clear();
                self.written = 0;
            }
            return Ok(());
        }
        let cap = (*remaining)
            .min(8192)
            .min(limit.saturating_sub(self.inbound.len()));
        if cap == 0 {
            return Err(DispatchError::BufferLimit);
        }
        let mut bytes = vec![0; cap];
        match self.transport.read_step(&mut bytes, *remaining) {
            Ok(0) => return Err(NetError::IncompleteBody.into()),
            Ok(n) => {
                self.inbound.extend_from_slice(&bytes[..n]);
                *remaining -= n;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        while self.inbound.len() >= 9 {
            let len = ((self.inbound[0] as usize) << 16)
                | ((self.inbound[1] as usize) << 8)
                | self.inbound[2] as usize;
            if len > http2::MAX_FRAME {
                return Err(NetError::MalformedResponse("HTTP/2 frame size".into()).into());
            }
            if self.inbound.len() < 9 + len {
                break;
            }
            let frame: Vec<u8> = self.inbound.drain(..9 + len).collect();
            self.client.io_mut().expect("h2 transport").feed(&frame);
            self.client
                .poll()
                .map_err(|e| NetError::MalformedResponse(e.to_string()))?;
            self.collect_output();
        }
        Ok(())
    }
}
struct Head {
    headers: Headers,
    framing: Framing,
    reusable: bool,
    compressed: bool,
}
enum Framing {
    Empty,
    Length(usize),
    Chunked,
    Eof,
}
struct Entry {
    url: ParsedUrl,
    origin: String,
    options: FetchOptions,
    deadline: Instant,
    state: State,
    transport: Option<Transport>,
    request: Vec<u8>,
    written: usize,
    buffer: Vec<u8>,
    head: Option<Head>,
    encoded: Vec<u8>,
    decoder: Option<Box<dyn Source>>,
    decoder_input_len: usize,
    decoded_emitted: usize,
    replay_skip: usize,
    wire_done: bool,
    h2_stream: Option<u32>,
    h2_pending: Vec<u8>,
    body_total: usize,
}

pub struct Dispatcher<R = StdResolver, C = StdConnector> {
    resolver: R,
    connector: C,
    limits: DispatchLimits,
    next_id: u64,
    active: BTreeMap<RequestId, Entry>,
    idle: HashMap<String, Transport>,
    h2: HashMap<String, H2Connection>,
    cancelled: Vec<RequestId>,
    websockets: BTreeMap<WebSocketId, WebSocketConnection>,
}
impl Dispatcher<StdResolver, StdConnector> {
    pub fn new(limits: DispatchLimits) -> Self {
        Self::with_io(limits, StdResolver, StdConnector)
    }
}
impl<R: Resolver, C: Connector> Dispatcher<R, C> {
    pub fn with_io(limits: DispatchLimits, resolver: R, connector: C) -> Self {
        Self {
            resolver,
            connector,
            limits,
            next_id: 1,
            active: BTreeMap::new(),
            idle: HashMap::new(),
            h2: HashMap::new(),
            cancelled: Vec::new(),
            websockets: BTreeMap::new(),
        }
    }
    pub fn in_flight(&self) -> usize {
        self.active.len() + self.websockets.len()
    }
    pub fn submit(
        &mut self,
        input: &str,
        options: FetchOptions,
        now: Instant,
    ) -> Result<RequestId, DispatchError> {
        if self.in_flight() >= self.limits.max_in_flight {
            return Err(DispatchError::TooManyInFlight);
        }
        if self.limits.bytes_per_step == 0
            || self.limits.max_buffered_bytes == 0
            || self.limits.connect_timeout.is_zero()
        {
            return Err(DispatchError::InvalidBudget);
        }
        let url = ParsedUrl::parse(input).map_err(NetError::from)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(NetError::UnsupportedScheme(url.scheme().into()).into());
        }
        let origin = format!(
            "{}://{}:{}",
            url.scheme(),
            url.hostname(),
            url.effective_port().unwrap_or(0)
        );
        if self.active.values().filter(|e| e.origin == origin).count() >= self.limits.max_per_origin
        {
            return Err(DispatchError::TooManyForOrigin);
        }
        let method = options.method.to_ascii_uppercase();
        if method.is_empty()
            || !method
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || matches!(method.as_str(), "CONNECT" | "TRACE" | "TRACK")
        {
            return Err(NetError::UnsupportedMethod(method).into());
        }
        let mut serialization = options.clone();
        serialization.method = "GET".into();
        serialization
            .request_headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("connection"));
        let mut request = http::request_bytes(&url, &serialization)?;
        if method != "GET" {
            request.splice(..3, method.bytes());
        }
        let head_end = request
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| NetError::MalformedResponse("request head".into()))?
            + 4;
        let body = request.split_off(head_end);
        let mut text = String::from_utf8(request)
            .map_err(|_| NetError::MalformedResponse("request utf8".into()))?;
        text = text.replace("Connection: close\r\n", "Connection: keep-alive\r\n");
        text = text.replace(
            "Accept-Encoding: gzip, deflate, br\r\n",
            "Accept-Encoding: identity\r\n",
        );
        let mut request = text.into_bytes();
        request.extend_from_slice(&body);
        if self.buffered_bytes().saturating_add(request.len()) > self.limits.max_buffered_bytes {
            return Err(DispatchError::BufferLimit);
        }
        let id = RequestId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let h2_ready = self.h2.contains_key(&origin);
        let h2_wait = url.scheme() == "https"
            && self
                .active
                .values()
                .any(|e| e.origin == origin && !matches!(e.state, State::WaitForOrigin));
        let transport = if h2_ready {
            None
        } else {
            self.idle.remove(&origin)
        };
        let state = if h2_ready {
            State::H2Start
        } else if transport.is_some() {
            State::Write
        } else if h2_wait {
            State::WaitForOrigin
        } else {
            State::Resolve
        };
        self.active.insert(
            id,
            Entry {
                url,
                origin,
                options: options.clone(),
                deadline: now + options.timeout,
                state,
                transport,
                request,
                written: 0,
                buffer: Vec::new(),
                head: None,
                encoded: Vec::new(),
                decoder: None,
                decoder_input_len: 0,
                decoded_emitted: 0,
                replay_skip: 0,
                wire_done: false,
                h2_stream: None,
                h2_pending: Vec::new(),
                body_total: 0,
            },
        );
        Ok(id)
    }
    pub fn cancel(&mut self, id: RequestId) -> bool {
        if let Some(entry) = self.active.remove(&id) {
            if let Some(stream) = entry.h2_stream {
                if let Some(conn) = self.h2.get_mut(&entry.origin) {
                    let _ = conn.client.cancel(stream);
                    let _ = conn.client.next_event_ready(stream);
                    conn.collect_output();
                }
            }
            self.cancelled.push(id);
            true
        } else {
            false
        }
    }
    pub fn buffered_bytes(&self) -> usize {
        self.active
            .values()
            .map(|e| {
                e.request.len()
                    + e.buffer.len()
                    + e.encoded.len()
                    + e.decoder_input_len
                    + e.h2_pending.len()
            })
            .sum::<usize>()
            + self
                .h2
                .values()
                .map(|h| h.inbound.len() + h.outbound.len())
                .sum::<usize>()
            + self
                .websockets
                .values()
                .map(WebSocketConnection::held)
                .sum::<usize>()
    }
    pub fn step(&mut self, budget: usize, now: Instant) -> Vec<NetEvent> {
        let mut events: Vec<NetEvent> = self
            .cancelled
            .drain(..)
            .map(|id| NetEvent::Aborted { id })
            .collect();
        let mut remaining = budget.min(self.limits.bytes_per_step);
        let ids: Vec<_> = self.active.keys().copied().collect();
        for id in ids {
            let Some(mut entry) = self.active.remove(&id) else {
                continue;
            };
            if now >= entry.deadline {
                events.push(NetEvent::Error {
                    id,
                    error: DispatchError::Timeout,
                });
                continue;
            }
            let result = self.advance(id, &mut entry, &mut remaining, &mut events);
            match result {
                Ok(done) => {
                    if !done {
                        self.active.insert(id, entry);
                    } else if entry.head.as_ref().is_some_and(|h| h.reusable) {
                        if let Some(io) = entry.transport.take() {
                            self.idle.insert(entry.origin, io);
                        }
                    }
                }
                Err(error) => events.push(NetEvent::Error { id, error }),
            }
        }
        events
    }
    fn advance(
        &mut self,
        id: RequestId,
        e: &mut Entry,
        remaining: &mut usize,
        events: &mut Vec<NetEvent>,
    ) -> Result<bool, DispatchError> {
        if *remaining == 0 {
            return Ok(false);
        }
        match &mut e.state {
            State::WaitForOrigin => {
                if self.h2.contains_key(&e.origin) {
                    e.state = State::H2Start;
                } else if !self.active.values().any(|other| {
                    other.origin == e.origin && !matches!(other.state, State::WaitForOrigin)
                }) {
                    e.state = State::Resolve;
                }
                Ok(false)
            }
            State::Resolve => {
                let hostname = e.url.hostname();
                let host = hostname.trim_start_matches('[').trim_end_matches(']');
                let port = e
                    .url
                    .effective_port()
                    .ok_or_else(|| NetError::MalformedResponse("missing port".into()))?;
                let addresses = self.resolver.resolve(host, port)?;
                if addresses.is_empty() {
                    return Err(
                        io::Error::new(io::ErrorKind::AddrNotAvailable, "no address").into(),
                    );
                }
                e.state = State::Connect(addresses);
                Ok(false)
            }
            State::Connect(addresses) => {
                let address = addresses.remove(0);
                match self.connector.connect(address, self.limits.connect_timeout) {
                    Ok(socket) => {
                        socket.set_nonblocking(true)?;
                        if e.url.scheme() == "https" {
                            let hostname = e.url.hostname();
                            let host = hostname.trim_start_matches('[').trim_end_matches(']');
                            let tls = amazing_net_tls::PendingTls::new(host, socket)
                                .map_err(|x| NetError::Tls(x.to_string()))?;
                            e.state = State::Handshake(tls);
                        } else {
                            e.transport = Some(Transport::Plain(socket));
                            e.state = State::Write;
                        }
                        return Ok(false);
                    }
                    Err(error) if !addresses.is_empty() => {
                        let _ = error;
                        Ok(false)
                    }
                    Err(error) => Err(error.into()),
                }
            }
            State::Handshake(tls) => {
                if *remaining == 0 {
                    return Ok(false);
                }
                let cap = *remaining;
                match tls.step(cap).map_err(|x| NetError::Tls(x.to_string()))? {
                    Some(stream) => {
                        *remaining = 0;
                        if stream.protocol() == amazing_net_tls::Protocol::H2 {
                            if self.h2.len() >= self.limits.max_in_flight {
                                let idle = self
                                    .h2
                                    .keys()
                                    .find(|origin| {
                                        !self.active.values().any(|other| other.origin == **origin)
                                    })
                                    .cloned();
                                if let Some(origin) = idle {
                                    self.h2.remove(&origin);
                                } else {
                                    return Err(DispatchError::TooManyInFlight);
                                }
                            }
                            self.h2.insert(
                                e.origin.clone(),
                                H2Connection::new(stream, self.limits.max_buffered_bytes)?,
                            );
                            e.state = State::H2Start;
                        } else {
                            e.transport = Some(Transport::Tls(stream));
                            e.state = State::Write;
                        }
                    }
                    None => {
                        *remaining = 0;
                    }
                }
                Ok(false)
            }
            State::Write => {
                if *remaining == 0 {
                    return Ok(false);
                }
                let end = e.request.len().min(e.written.saturating_add(*remaining));
                match e
                    .transport
                    .as_mut()
                    .expect("connected")
                    .write_step(&e.request[e.written..end], *remaining)
                {
                    Ok(0) => {
                        return Err(io::Error::new(io::ErrorKind::WriteZero, "request write").into())
                    }
                    Ok(n) => {
                        e.written += n;
                        *remaining -= n;
                        if e.written == e.request.len() {
                            e.request.clear();
                            e.request.shrink_to_fit();
                            e.state = State::Read;
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                    Err(err) => return Err(err.into()),
                }
                Ok(false)
            }
            State::Read => {
                if *remaining == 0 {
                    return Ok(false);
                }
                let used = self.buffered_bytes()
                    + e.request.len()
                    + e.buffer.len()
                    + e.encoded.len()
                    + e.decoder_input_len;
                let cap = (*remaining)
                    .min(8192)
                    .min(self.limits.max_buffered_bytes.saturating_sub(used));
                if cap == 0 {
                    return Err(DispatchError::BufferLimit);
                }
                let mut bytes = vec![0; cap];
                match e
                    .transport
                    .as_mut()
                    .expect("connected")
                    .read_step(&mut bytes, *remaining)
                {
                    Ok(0) => {
                        if e.head.is_none() {
                            return Err(NetError::IncompleteBody.into());
                        }
                        if !matches!(e.head.as_ref().unwrap().framing, Framing::Eof) {
                            return Err(NetError::IncompleteBody.into());
                        }
                        self.finish(id, e, events)
                    }
                    Ok(n) => {
                        *remaining -= n;
                        e.buffer.extend_from_slice(&bytes[..n]);
                        self.consume(id, e, events)
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(false),
                    Err(err) => Err(err.into()),
                }
            }
            State::Emit => {
                if *remaining == 0 {
                    return Ok(false);
                }
                if e.decoder.is_none() {
                    match self.prepare_decoder(e) {
                        Err(DispatchError::Network(NetError::IncompleteBody)) if !e.wire_done => {
                            e.decoder_input_len = 0;
                            e.state = if e.h2_stream.is_some() {
                                State::H2Read
                            } else {
                                State::Read
                            };
                            return Ok(false);
                        }
                        result => result?,
                    }
                }
                let mut output = vec![0; (*remaining).min(8192)];
                if e.replay_skip > 0 {
                    output.truncate(output.len().min(e.replay_skip));
                    let n = match e.decoder.as_mut().expect("decoder").read(&mut output) {
                        Ok(n) => n,
                        Err(NetError::IncompleteBody) if !e.wire_done => {
                            e.decoder = None;
                            e.decoder_input_len = 0;
                            e.state = if e.h2_stream.is_some() {
                                State::H2Read
                            } else {
                                State::Read
                            };
                            return Ok(false);
                        }
                        Err(error) => return Err(error.into()),
                    };
                    if n == 0 {
                        return Err(NetError::IncompleteBody.into());
                    }
                    e.replay_skip -= n;
                    *remaining -= n;
                    return Ok(false);
                }
                let n = match e.decoder.as_mut().expect("decoder").read(&mut output) {
                    Ok(n) => n,
                    Err(NetError::IncompleteBody) if !e.wire_done => {
                        e.decoder = None;
                        e.decoder_input_len = 0;
                        e.state = if e.h2_stream.is_some() {
                            State::H2Read
                        } else {
                            State::Read
                        };
                        return Ok(false);
                    }
                    Err(error) => return Err(error.into()),
                };
                if n == 0 {
                    e.decoder = None;
                    e.decoder_input_len = 0;
                    if e.wire_done {
                        events.push(NetEvent::Done { id });
                        Ok(true)
                    } else {
                        e.state = if e.h2_stream.is_some() {
                            State::H2Read
                        } else {
                            State::Read
                        };
                        Ok(false)
                    }
                } else {
                    output.truncate(n);
                    e.decoded_emitted += n;
                    *remaining -= n;
                    events.push(NetEvent::Body { id, bytes: output });
                    Ok(false)
                }
            }
            State::H2Start => {
                let conn = self
                    .h2
                    .get_mut(&e.origin)
                    .ok_or(DispatchError::UnsupportedProtocol)?;
                let fields = http2::request_fields(&e.url, &e.options)?;
                let stream = conn
                    .client
                    .start_streaming(&fields, &e.options.body)
                    .map_err(|x| NetError::MalformedResponse(x.to_string()))?;
                conn.collect_output();
                e.h2_stream = Some(stream);
                e.request.clear();
                e.state = State::H2Read;
                Ok(false)
            }
            State::H2Read => {
                if !e.h2_pending.is_empty() {
                    let n = e.h2_pending.len().min(*remaining);
                    let bytes: Vec<_> = e.h2_pending.drain(..n).collect();
                    *remaining -= n;
                    events.push(NetEvent::Body { id, bytes });
                    return Ok(false);
                }
                let conn = self
                    .h2
                    .get_mut(&e.origin)
                    .ok_or(DispatchError::UnsupportedProtocol)?;
                if let Err(error) = conn.step(remaining, self.limits.max_buffered_bytes) {
                    self.h2.remove(&e.origin);
                    return Err(error);
                }
                let stream = e.h2_stream.expect("started h2 stream");
                let Some(event) = conn
                    .client
                    .next_event_ready(stream)
                    .map_err(|x| NetError::MalformedResponse(x.to_string()))?
                else {
                    return Ok(false);
                };
                match event {
                    http2::StreamEvent::Head { status, headers } => {
                        let mut fields = Headers::new();
                        for (name, value) in headers {
                            let name = std::str::from_utf8(&name).map_err(|_| {
                                NetError::MalformedResponse("h2 header name".into())
                            })?;
                            let value = String::from_utf8(value).map_err(|_| {
                                NetError::MalformedResponse("h2 header value".into())
                            })?;
                            fields
                                .append(name, value)
                                .map_err(|_| NetError::MalformedResponse("h2 header".into()))?;
                        }
                        let compressed = !e.options.method.eq_ignore_ascii_case("HEAD")
                            && !matches!(status, 204 | 304)
                            && http::codings(&fields, "content-encoding")?
                                .iter()
                                .any(|c| c != "identity");
                        e.head = Some(Head {
                            headers: fields.clone(),
                            framing: Framing::Empty,
                            reusable: false,
                            compressed,
                        });
                        events.push(NetEvent::Head {
                            id,
                            url: e.url.to_string(),
                            status,
                            reason: String::new(),
                            headers: fields,
                        });
                    }
                    http2::StreamEvent::Data(bytes) => {
                        e.body_total = e.body_total.saturating_add(bytes.len());
                        if e.body_total > e.options.max_response_bytes.min(http::MAX_BODY_BYTES) {
                            return Err(NetError::ResponseTooLarge.into());
                        }
                        if e.head.as_ref().is_some_and(|h| h.compressed) {
                            if e.encoded.len().saturating_add(bytes.len())
                                > self.limits.max_buffered_bytes
                            {
                                return Err(DispatchError::BufferLimit);
                            }
                            e.encoded.extend_from_slice(&bytes);
                            e.decoder = None;
                            e.decoder_input_len = 0;
                            e.state = State::Emit;
                        } else {
                            e.h2_pending = bytes;
                        }
                    }
                    http2::StreamEvent::End { .. } => {
                        if e.head.as_ref().is_some_and(|h| h.compressed) {
                            e.wire_done = true;
                            e.state = State::Emit;
                        } else {
                            events.push(NetEvent::Done { id });
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            }
        }
    }
    fn consume(
        &mut self,
        id: RequestId,
        e: &mut Entry,
        events: &mut Vec<NetEvent>,
    ) -> Result<bool, DispatchError> {
        if e.head.is_none() {
            let Some(pos) = e.buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                if e.buffer.len() > 65536 {
                    return Err(DispatchError::BufferLimit);
                }
                return Ok(false);
            };
            if pos > 65536 {
                return Err(DispatchError::BufferLimit);
            }
            let (status, reason, headers) = http::parse_head(&e.buffer[..pos])?;
            if (100..200).contains(&status) {
                return Err(NetError::MalformedResponse("informational response".into()).into());
            }
            let transfer = http::codings(&headers, "transfer-encoding")?;
            let content = http::codings(&headers, "content-encoding")?;
            if content.iter().any(|c| c == "chunked") || transfer.iter().any(|c| c == "identity") {
                return Err(NetError::MalformedResponse("coding in wrong header".into()).into());
            }
            if !transfer.is_empty() && headers.contains("content-length") {
                return Err(NetError::MalformedResponse(
                    "Transfer-Encoding with Content-Length".into(),
                )
                .into());
            }
            if transfer.iter().filter(|c| c.as_str() == "chunked").count() > 1
                || transfer
                    .iter()
                    .position(|c| c == "chunked")
                    .is_some_and(|i| i != transfer.len() - 1)
            {
                return Err(NetError::MalformedResponse("invalid chunked coding".into()).into());
            }
            let compressed = !e.options.method.eq_ignore_ascii_case("HEAD")
                && !matches!(status, 204 | 304)
                && transfer
                    .iter()
                    .chain(content.iter())
                    .any(|c| c != "chunked" && c != "identity");
            let framing =
                if e.options.method.eq_ignore_ascii_case("HEAD") || matches!(status, 204 | 304) {
                    Framing::Empty
                } else if transfer.last().is_some_and(|c| c == "chunked") {
                    Framing::Chunked
                } else if let Some(length) = http::content_length(&headers)? {
                    Framing::Length(length)
                } else {
                    Framing::Eof
                };
            let connection = headers.get("connection").unwrap_or("");
            let has_token = |token: &str| {
                connection
                    .split(',')
                    .any(|v| v.trim().eq_ignore_ascii_case(token))
            };
            let http10 = e.buffer.starts_with(b"HTTP/1.0 ");
            let reusable = !matches!(framing, Framing::Eof)
                && !has_token("close")
                && (!http10 || has_token("keep-alive"));
            e.buffer.drain(..pos + 4);
            e.head = Some(Head {
                headers: headers.clone(),
                framing,
                reusable,
                compressed,
            });
            events.push(NetEvent::Head {
                id,
                url: e.url.to_string(),
                status,
                reason,
                headers,
            });
        }
        loop {
            let head = e.head.as_mut().expect("parsed head");
            match &mut head.framing {
                Framing::Empty => {
                    return self.finish(id, e, events);
                }
                Framing::Length(left) => {
                    if *left == 0 {
                        return self.finish(id, e, events);
                    }
                    if e.buffer.is_empty() {
                        return Ok(false);
                    }
                    let n = (*left).min(e.buffer.len());
                    let data: Vec<_> = e.buffer.drain(..n).collect();
                    *left -= n;
                    Self::body(id, e, data, events, self.limits.max_buffered_bytes)?;
                }
                Framing::Chunked => {
                    let Some(line_end) = e.buffer.windows(2).position(|w| w == b"\r\n") else {
                        return Ok(false);
                    };
                    let line = std::str::from_utf8(&e.buffer[..line_end])
                        .map_err(|_| NetError::MalformedResponse("chunk size".into()))?;
                    let size = usize::from_str_radix(line.split(';').next().unwrap_or(""), 16)
                        .map_err(|_| NetError::MalformedResponse("chunk size".into()))?;
                    if size == 0 {
                        if e.buffer.len() < line_end + 4 {
                            return Ok(false);
                        }
                        if e.buffer[line_end + 2..].starts_with(b"\r\n") {
                            e.buffer.drain(..line_end + 4);
                            return self.finish(id, e, events);
                        }
                        let tail = &e.buffer[line_end + 2..];
                        if let Some(tail_end) = tail.windows(4).position(|w| w == b"\r\n\r\n") {
                            e.buffer.drain(..line_end + 2 + tail_end + 4);
                            return self.finish(id, e, events);
                        }
                        return Ok(false);
                    }
                    if size > e.options.max_response_bytes
                        || e.buffer.len() < line_end + 2 + size + 2
                    {
                        return if size > e.options.max_response_bytes {
                            Err(NetError::ResponseTooLarge.into())
                        } else {
                            Ok(false)
                        };
                    }
                    let start = line_end + 2;
                    let end = start + size;
                    if &e.buffer[end..end + 2] != b"\r\n" {
                        return Err(NetError::MalformedResponse("chunk terminator".into()).into());
                    }
                    let data = e.buffer[start..end].to_vec();
                    e.buffer.drain(..end + 2);
                    Self::body(id, e, data, events, self.limits.max_buffered_bytes)?;
                }
                Framing::Eof => {
                    if e.buffer.is_empty() {
                        return Ok(false);
                    }
                    let data = std::mem::take(&mut e.buffer);
                    Self::body(id, e, data, events, self.limits.max_buffered_bytes)?;
                    return Ok(false);
                }
            }
        }
    }
    fn body(
        id: RequestId,
        e: &mut Entry,
        data: Vec<u8>,
        events: &mut Vec<NetEvent>,
        limit: usize,
    ) -> Result<(), DispatchError> {
        e.body_total = e.body_total.saturating_add(data.len());
        if e.body_total > e.options.max_response_bytes.min(http::MAX_BODY_BYTES) {
            return Err(NetError::ResponseTooLarge.into());
        }
        if e.head.as_ref().is_some_and(|h| h.compressed) {
            if e.encoded.len().saturating_add(data.len()) > limit {
                return Err(DispatchError::BufferLimit);
            }
            e.encoded.extend_from_slice(&data);
            e.decoder = None;
            e.decoder_input_len = 0;
            e.state = State::Emit;
        } else {
            events.push(NetEvent::Body { id, bytes: data });
        }
        Ok(())
    }
    fn prepare_decoder(&self, e: &mut Entry) -> Result<(), DispatchError> {
        let head = e.head.as_ref().expect("parsed head");
        let transfer = http::codings(&head.headers, "transfer-encoding")?;
        let content = http::codings(&head.headers, "content-encoding")?;
        let limit = e.options.max_response_bytes.min(http::MAX_BODY_BYTES);
        let input = e.encoded.clone();
        e.decoder_input_len = input.len();
        let counted = Rc::new(Cell::new(0));
        let mut source: Box<dyn Source> = Box::new(CountSource {
            inner: Box::new(IoSource(Cursor::new(input))),
            count: counted.clone(),
        });
        for coding in transfer.iter().rev().chain(content.iter().rev()) {
            if coding != "chunked" {
                source = decode_layer(source, coding, limit)?;
            }
        }
        e.decoder = Some(Box::new(CapSource {
            inner: source,
            total: 0,
            limit,
            ratio: Some((counted, 1024)),
        }));
        e.replay_skip = e.decoded_emitted;
        Ok(())
    }
    fn finish(
        &self,
        id: RequestId,
        e: &mut Entry,
        events: &mut Vec<NetEvent>,
    ) -> Result<bool, DispatchError> {
        if !e.buffer.is_empty() {
            if let Some(head) = &mut e.head {
                head.reusable = false;
            }
        }
        if let Some(head) = &e.head {
            if head.compressed {
                e.wire_done = true;
                e.state = State::Emit;
                return Ok(false);
            }
        }
        events.push(NetEvent::Done { id });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn h2_frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
        let mut head = [0; 9];
        head[..3].copy_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
        head[3] = kind;
        head[4] = flags;
        head[5..].copy_from_slice(&stream.to_be_bytes());
        [head.as_slice(), payload].concat()
    }
    fn h2_read(io: &mut impl Read) -> (u8, u32, Vec<u8>) {
        let mut head = [0; 9];
        io.read_exact(&mut head).unwrap();
        let len = ((head[0] as usize) << 16) | ((head[1] as usize) << 8) | head[2] as usize;
        let mut body = vec![0; len];
        io.read_exact(&mut body).unwrap();
        (
            head[3],
            u32::from_be_bytes(head[5..].try_into().unwrap()) & 0x7fff_ffff,
            body,
        )
    }

    #[test]
    fn tls_h2_dispatch_multiplexes_and_aborts_one_stream() {
        use amazing_net_tls::test_support;
        let (release, hold) = std::sync::mpsc::channel();
        let (port, thread) = test_support::serve(vec![b"h2".to_vec()], move |io| {
            assert_eq!(io.protocol(), Some(b"h2".as_slice()));
            let mut preface = [0; 24];
            io.read_exact(&mut preface).unwrap();
            assert_eq!(&preface, http2::PREFACE);
            assert_eq!(h2_read(io).0, 4);
            io.write_all(&h2_frame(4, 0, 0, &[])).unwrap();
            io.flush().unwrap();
            let mut streams = Vec::new();
            while streams.len() < 2 {
                let (kind, stream, _) = h2_read(io);
                if kind == 1 {
                    streams.push(stream);
                }
            }
            assert_eq!(streams, vec![1, 3]);
            for stream in &streams {
                io.write_all(&h2_frame(1, 4, *stream, &[0x88])).unwrap();
            }
            io.write_all(&h2_frame(0, 0, 1, b"a")).unwrap();
            io.write_all(&h2_frame(0, 0, 3, b"b")).unwrap();
            io.flush().unwrap();
            loop {
                let (kind, stream, payload) = h2_read(io);
                if kind == 3 && stream == 1 {
                    assert_eq!(payload, 8u32.to_be_bytes());
                    break;
                }
            }
            io.write_all(&h2_frame(0, 1, 3, b"c")).unwrap();
            io.flush().unwrap();
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let tls = test_support::connect_nonblocking(port).unwrap();
        assert_eq!(tls.protocol(), amazing_net_tls::Protocol::H2);
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let origin = format!("https://localhost:{port}");
        dispatcher.h2.insert(
            origin,
            H2Connection::new(tls, dispatcher.limits.max_buffered_bytes).unwrap(),
        );
        let first = dispatcher
            .submit(
                &format!("https://localhost:{port}/one"),
                FetchOptions::default(),
                now,
            )
            .unwrap();
        let second = dispatcher
            .submit(
                &format!("https://localhost:{port}/two"),
                FetchOptions::default(),
                now,
            )
            .unwrap();
        let mut one = Vec::new();
        let mut two = Vec::new();
        let mut aborted = false;
        let mut done = false;
        for _ in 0..100_000 {
            for event in dispatcher.step(4096, Instant::now()) {
                match event {
                    NetEvent::Body { id, bytes } if id == first => one.extend(bytes),
                    NetEvent::Body { id, bytes } if id == second => two.extend(bytes),
                    NetEvent::Aborted { id } if id == first => aborted = true,
                    NetEvent::Done { id } if id == second => done = true,
                    NetEvent::Error { error, .. } => panic!("{error:?}"),
                    _ => {}
                }
            }
            if one == b"a" && two == b"b" && !aborted {
                assert!(dispatcher.cancel(first));
            }
            if done && aborted {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(one, b"a");
        assert_eq!(two, b"bc");
        assert!(aborted && done);
        release.send(()).unwrap();
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn tls_h2_head_does_not_decode_advertised_encoding() {
        use amazing_net_tls::test_support;
        let (release, hold) = std::sync::mpsc::channel();
        let (port, thread) = test_support::serve(vec![b"h2".to_vec()], move |io| {
            let mut preface = [0; 24];
            io.read_exact(&mut preface).unwrap();
            assert_eq!(h2_read(io).0, 4);
            io.write_all(&h2_frame(4, 0, 0, &[])).unwrap();
            io.flush().unwrap();
            loop {
                let (kind, stream, _) = h2_read(io);
                if kind == 1 {
                    assert_eq!(stream, 1);
                    break;
                }
            }
            // Indexed :status 200, literal content-encoding: gzip.
            let block = [0x88, 0x0f, 0x0b, 0x04, b'g', b'z', b'i', b'p'];
            io.write_all(&h2_frame(1, 5, 1, &block)).unwrap();
            io.flush().unwrap();
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let tls = test_support::connect_nonblocking(port).unwrap();
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let origin = format!("https://localhost:{port}");
        let max_buffered = dispatcher.limits.max_buffered_bytes;
        dispatcher
            .h2
            .insert(origin, H2Connection::new(tls, max_buffered).unwrap());
        let mut options = FetchOptions::default();
        options.method = "HEAD".into();
        let id = dispatcher
            .submit(&format!("https://localhost:{port}/"), options, now)
            .unwrap();
        let mut done = false;
        for _ in 0..100_000 {
            for event in dispatcher.step(4096, Instant::now()) {
                match event {
                    NetEvent::Done { id: found } if found == id => done = true,
                    NetEvent::Body { id: found, .. } if found == id => panic!("HEAD body"),
                    NetEvent::Error { id: found, error } if found == id => panic!("{error:?}"),
                    _ => {}
                }
            }
            if done {
                break;
            }
            std::thread::yield_now();
        }
        assert!(done);
        release.send(()).unwrap();
        thread.join().unwrap().unwrap();
    }

    struct Peer {
        socket: TcpStream,
        input: Vec<u8>,
        late: Option<(usize, Vec<u8>)>,
    }
    struct Server {
        listener: TcpListener,
        peers: Vec<Peer>,
        ticks: usize,
        accepted: usize,
    }
    impl Server {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            Self {
                listener,
                peers: Vec::new(),
                ticks: 0,
                accepted: 0,
            }
        }
        fn url(&self, path: &str) -> String {
            format!(
                "http://127.0.0.1:{}{path}",
                self.listener.local_addr().unwrap().port()
            )
        }
        fn tick(&mut self) {
            self.ticks += 1;
            loop {
                match self.listener.accept() {
                    Ok((socket, _)) => {
                        socket.set_nonblocking(true).unwrap();
                        self.peers.push(Peer {
                            socket,
                            input: Vec::new(),
                            late: None,
                        });
                        self.accepted += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("accept: {e}"),
                }
            }
            for peer in &mut self.peers {
                if let Some((when, bytes)) = peer.late.take() {
                    if self.ticks >= when {
                        let _ = peer.socket.write_all(&bytes);
                    } else {
                        peer.late = Some((when, bytes));
                    }
                }
                let mut buf = [0; 4096];
                loop {
                    match peer.socket.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => peer.input.extend_from_slice(&buf[..n]),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) => panic!("read: {e}"),
                    }
                }
                while let Some(end) = peer.input.windows(4).position(|w| w == b"\r\n\r\n") {
                    let request = String::from_utf8_lossy(&peer.input[..end]);
                    let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                    peer.input.drain(..end + 4);
                    match path.as_str() {
                        "/chunk" => {
                            peer.socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\no\r\n1\r\nk\r\n0\r\nX-End: yes\r\n\r\n").unwrap();
                        }
                        "/gzip" | "/gzip-slow" => {
                            const BODY: [u8; 25] = [
                                31, 139, 8, 0, 0, 0, 0, 0, 2, 19, 203, 72, 205, 201, 201, 7, 0,
                                134, 166, 16, 54, 5, 0, 0, 0,
                            ];
                            let mut reply = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 25\r\n\r\n".to_vec();
                            reply.extend_from_slice(&BODY);
                            if path == "/gzip-slow" {
                                peer.socket.write_all(&reply[..reply.len() - 8]).unwrap();
                                peer.late =
                                    Some((self.ticks + 20, reply[reply.len() - 8..].to_vec()));
                            } else {
                                peer.socket.write_all(&reply).unwrap();
                            }
                        }
                        "/slow" | "/abort" => {
                            peer.socket
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
                                .unwrap();
                            peer.late = Some((self.ticks + 20, b"b".to_vec()));
                        }
                        "/timeout" => {}
                        _ => {
                            peer.socket
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
    fn spin(
        server: &mut Server,
        dispatch: &mut Dispatcher,
        start: Instant,
        ticks: usize,
    ) -> Vec<NetEvent> {
        let mut all = Vec::new();
        for i in 0..ticks {
            server.tick();
            all.extend(dispatch.step(1024, start + Duration::from_millis(i as u64)));
        }
        all
    }
    #[test]
    fn interleaved_out_of_order_and_abort_mid_body() {
        let mut server = Server::new();
        let start = Instant::now();
        let mut d = Dispatcher::new(DispatchLimits::default());
        let slow = d
            .submit(&server.url("/slow"), FetchOptions::default(), start)
            .unwrap();
        let fast = d
            .submit(&server.url("/fast"), FetchOptions::default(), start)
            .unwrap();
        let events = spin(&mut server, &mut d, start, 15);
        assert!(events
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == fast)));
        assert!(!events
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == slow)));
        assert!(events
            .iter()
            .any(|e| matches!(e, NetEvent::Body { id, bytes } if *id == slow && bytes == b"a")));
        assert!(d.cancel(slow));
        let events = spin(&mut server, &mut d, start, 2);
        assert!(events
            .iter()
            .any(|e| matches!(e, NetEvent::Aborted { id } if *id == slow)));
    }
    #[test]
    fn timeout_and_keepalive_reuse() {
        let mut server = Server::new();
        let start = Instant::now();
        let mut d = Dispatcher::new(DispatchLimits::default());
        let first = d
            .submit(&server.url("/one"), FetchOptions::default(), start)
            .unwrap();
        assert!(spin(&mut server, &mut d, start, 20)
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == first)));
        let second = d
            .submit(&server.url("/two"), FetchOptions::default(), start)
            .unwrap();
        assert!(spin(&mut server, &mut d, start, 20)
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == second)));
        assert_eq!(server.accepted, 1);
        let mut options = FetchOptions::default();
        options.timeout = Duration::from_millis(3);
        let timeout = d.submit(&server.url("/timeout"), options, start).unwrap();
        let events = spin(&mut server, &mut d, start, 20);
        assert!(events.iter().any(
            |e| matches!(e, NetEvent::Error { id, error: DispatchError::Timeout } if *id == timeout)
        ));
    }
    #[test]
    fn configured_bounds_have_typed_signals() {
        let server = Server::new();
        let now = Instant::now();
        let limits = DispatchLimits {
            max_in_flight: 1,
            ..DispatchLimits::default()
        };
        let mut d = Dispatcher::new(limits);
        d.submit(&server.url("/a"), FetchOptions::default(), now)
            .unwrap();
        assert!(matches!(
            d.submit(&server.url("/b"), FetchOptions::default(), now),
            Err(DispatchError::TooManyInFlight)
        ));
        let mut d = Dispatcher::new(DispatchLimits {
            max_per_origin: 1,
            ..DispatchLimits::default()
        });
        d.submit(&server.url("/a"), FetchOptions::default(), now)
            .unwrap();
        assert!(matches!(
            d.submit(&server.url("/b"), FetchOptions::default(), now),
            Err(DispatchError::TooManyForOrigin)
        ));
        let mut d = Dispatcher::new(DispatchLimits {
            bytes_per_step: 0,
            ..DispatchLimits::default()
        });
        assert!(matches!(
            d.submit(&server.url("/a"), FetchOptions::default(), now),
            Err(DispatchError::InvalidBudget)
        ));
        let mut d = Dispatcher::new(DispatchLimits {
            max_buffered_bytes: 1,
            ..DispatchLimits::default()
        });
        assert!(matches!(
            d.submit(&server.url("/a"), FetchOptions::default(), now),
            Err(DispatchError::BufferLimit)
        ));
        let mut d = Dispatcher::new(DispatchLimits::default());
        d.submit(&server.url("/a"), FetchOptions::default(), now)
            .unwrap();
        let used = d.buffered_bytes();
        d.limits.max_buffered_bytes = used * 2 - 1;
        assert!(matches!(
            d.submit(&server.url("/b"), FetchOptions::default(), now),
            Err(DispatchError::BufferLimit)
        ));
    }
    #[test]
    fn gzip_decodes_after_framing_and_emits_within_step_budget() {
        let mut server = Server::new();
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let id = dispatcher
            .submit(&server.url("/gzip"), FetchOptions::default(), now)
            .unwrap();
        let mut body = Vec::new();
        let mut done = false;
        for tick in 0..500 {
            server.tick();
            for event in dispatcher.step(2, now + Duration::from_millis(tick)) {
                match event {
                    NetEvent::Body { id: found, bytes } if found == id => {
                        assert!(bytes.len() <= 2);
                        body.extend_from_slice(&bytes);
                    }
                    NetEvent::Done { id: found } if found == id => done = true,
                    NetEvent::Error { id: found, error } if found == id => panic!("{error:?}"),
                    _ => {}
                }
            }
            if done {
                break;
            }
        }
        assert!(done);
        assert_eq!(body, b"hello");
    }
    #[test]
    fn gzip_emits_decoded_bytes_before_wire_body_completes() {
        let mut server = Server::new();
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let id = dispatcher
            .submit(&server.url("/gzip-slow"), FetchOptions::default(), now)
            .unwrap();
        let early = spin(&mut server, &mut dispatcher, now, 15);
        assert!(early.iter().any(|e| matches!(e, NetEvent::Body { id: found, bytes } if *found == id && bytes == b"hello")));
        assert!(!early
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id: found } if *found == id)));
        let late = spin(&mut server, &mut dispatcher, now, 50);
        assert!(late
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id: found } if *found == id)));
    }
    #[test]
    fn chunked_trailer_completes_and_connection_is_reused() {
        let mut server = Server::new();
        let now = Instant::now();
        let mut d = Dispatcher::new(DispatchLimits::default());
        let first = d
            .submit(&server.url("/chunk"), FetchOptions::default(), now)
            .unwrap();
        let events = spin(&mut server, &mut d, now, 30);
        let body: Vec<u8> = events
            .iter()
            .filter_map(|e| match e {
                NetEvent::Body { id, bytes } if *id == first => Some(bytes.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        assert_eq!(body, b"ok");
        assert!(events
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == first)));
        let second = d
            .submit(&server.url("/fast"), FetchOptions::default(), now)
            .unwrap();
        assert!(spin(&mut server, &mut d, now, 30)
            .iter()
            .any(|e| matches!(e, NetEvent::Done { id } if *id == second)));
        assert_eq!(server.accepted, 1);
    }
}
