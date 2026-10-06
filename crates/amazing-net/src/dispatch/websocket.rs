//! Retained RFC6455 client driven by the dispatcher's DNS, connect and TLS seams.
use super::*;
use crate::websocket::{self as ws, handshake, utf8::Validator};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WebSocketId(pub u64);

#[derive(Debug)]
pub enum WebSocketEvent {
    Open {
        id: WebSocketId,
        protocol: Option<String>,
    },
    Message {
        id: WebSocketId,
        message: ws::Event,
    },
    Close {
        id: WebSocketId,
        close: Option<ws::Close>,
    },
    Error {
        id: WebSocketId,
        error: ws::Error,
    },
    /// Payload bytes from `send()` calls that have now actually been
    /// written to the transport, for the caller to decrement its
    /// `bufferedAmount` by (WHATWG: bytes "queued but not yet transmitted").
    /// Control frames (close/ping/pong) never contribute to this.
    Sent {
        id: WebSocketId,
        bytes: u64,
    },
}

enum Phase {
    Resolve,
    Resolving(PendingIoOperation),
    Connecting(Vec<SocketAddr>,PendingIoOperation),
    Connect(Vec<SocketAddr>),
    Tls(amazing_net_tls::PendingTls),
    Request,
    Response,
    Open,
}

pub(super) struct WebSocketConnection {
    io_generation:u64,
    url: ParsedUrl,
    options: ws::Options,
    phase: Phase,
    deadline: Instant,
    transport: Option<Transport>,
    expected: String,
    incoming: Vec<u8>,
    /// Queued frames awaiting write, paired with the number of payload
    /// bytes to credit to `Sent` once the frame fully drains (the data
    /// frame's own payload length, or 0 for a control frame).
    outgoing: VecDeque<(Vec<u8>, u64)>,
    written: usize,
    fragmented_opcode: Option<u8>,
    message: Vec<u8>,
    validator: Validator,
    frames: usize,
    sent_close: bool,
    received_close: bool,
    /// Data-frame payload bytes flushed to the transport since the last
    /// `Sent` event was emitted for this connection.
    flushed_bytes: u64,
}

impl<R: Resolver, C: Connector> Dispatcher<R, C> {
    pub fn websockets_in_flight(&self) -> usize {
        self.websockets.len()
    }

    /// Drop a connection that never reached `Open` (e.g. a JS-level `close()`
    /// called while CONNECTING) without sending a close frame. The owned
    /// transport, if any, is dropped, which closes the underlying socket.
    pub fn forget_websocket(&mut self, id: WebSocketId) -> bool {
        self.websockets.remove(&id).is_some()
    }

    pub fn open_websocket(
        &mut self,
        input: &str,
        options: ws::Options,
        cookie: Option<&str>,
        now: Instant,
    ) -> Result<WebSocketId, ws::Error> {
        if input.len() > 8192
            || self.active.len() + self.websockets.len() >= self.limits.max_in_flight
            || self.limits.bytes_per_step == 0
            || self.limits.connect_timeout.is_zero()
        {
            return Err(ws::Error::Limit);
        }
        options.limits.validate()?;
        let url = ParsedUrl::parse(input).map_err(|_| ws::Error::InvalidInput)?;
        if !matches!(url.scheme(), "ws" | "wss")
            || url.fragment().is_some()
            || url.hostname().is_empty()
        {
            return Err(ws::Error::InvalidInput);
        }
        // URL Standard userinfo, same as `fetch()`/XHR: username/password in
        // the URL become a `Basic` `Authorization` header on the handshake
        // request rather than being rejected outright (WPT
        // `websockets/basic-auth.any.js` expects `ws://foo:bar@host/...` to
        // authenticate and open normally).
        let authorization = url.includes_credentials().then(|| {
            let mut credentials = crate::url::encoding::percent_decode(url.username());
            credentials.push(b':');
            credentials.extend(crate::url::encoding::percent_decode(url.password()));
            format!("Basic {}", handshake::base64(&credentials))
        });
        if self
            .websockets
            .values()
            .filter(|connection| {
                connection.url.scheme() == url.scheme()
                    && connection.url.hostname() == url.hostname()
                    && connection.url.effective_port() == url.effective_port()
            })
            .count()
            >= self.limits.max_per_origin
        {
            return Err(ws::Error::Limit);
        }
        let mut target = url.pathname();
        if let Some(query) = url.query() {
            target.push('?');
            target.push_str(query);
        }
        let authority = url.host_and_port();
        handshake::validate_request(&authority, &target, &options)?;
        if let Some(value) = cookie {
            if value.len() > 4096 || !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
                return Err(ws::Error::InvalidInput);
            }
        }
        let mut nonce = [0; 16];
        ws::random(&mut nonce)?;
        let key = handshake::base64(&nonce);
        let mut request = format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n");
        if let Some(origin) = &options.origin {
            request.push_str(&format!("Origin: {origin}\r\n"));
        }
        if !options.protocols.is_empty() {
            request.push_str(&format!(
                "Sec-WebSocket-Protocol: {}\r\n",
                options.protocols.join(", ")
            ));
        }
        if let Some(cookie) = cookie {
            request.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        if let Some(authorization) = &authorization {
            request.push_str(&format!("Authorization: {authorization}\r\n"));
        }
        request.push_str("\r\n");
        if request.len() > handshake::MAX_HANDSHAKE
            || self.buffered_bytes().saturating_add(request.len()) > self.limits.max_buffered_bytes
        {
            return Err(ws::Error::Limit);
        }
        let id = WebSocketId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.websockets.insert(
            id,
            WebSocketConnection {
                io_generation:0,
                url,
                options: options.clone(),
                phase: Phase::Resolve,
                deadline: now + options.timeout,
                transport: None,
                expected: handshake::accept(&key),
                incoming: Vec::new(),
                outgoing: VecDeque::from([(request.into_bytes(), 0)]),
                written: 0,
                fragmented_opcode: None,
                message: Vec::new(),
                validator: Validator::default(),
                frames: 0,
                sent_close: false,
                received_close: false,
                flushed_bytes: 0,
            },
        );
        Ok(id)
    }

    pub fn send_websocket(&mut self, id: WebSocketId, message: ws::Event) -> Result<(), ws::Error> {
        let available = self
            .limits
            .max_buffered_bytes
            .saturating_sub(self.buffered_bytes());
        let connection = self.websockets.get_mut(&id).ok_or(ws::Error::Closed)?;
        if !matches!(connection.phase, Phase::Open) || connection.sent_close {
            return Err(ws::Error::Closed);
        }
        let (opcode, bytes) = match message {
            ws::Event::Text(s) => (1, s.into_bytes()),
            ws::Event::Binary(b) => (2, b),
            ws::Event::Close(_) => return Err(ws::Error::InvalidInput),
        };
        if bytes.len() > connection.options.limits.frame_bytes
            || bytes.len() > connection.options.limits.message_bytes
            || bytes.len().saturating_add(14) > available
        {
            return Err(ws::Error::Limit);
        }
        let credit = bytes.len() as u64;
        connection.queue(opcode, &bytes, self.limits.max_buffered_bytes, credit)
    }

    pub fn close_websocket(
        &mut self,
        id: WebSocketId,
        close: Option<ws::Close>,
    ) -> Result<(), ws::Error> {
        let connection = self.websockets.get_mut(&id).ok_or(ws::Error::Closed)?;
        if !matches!(connection.phase, Phase::Open) || connection.sent_close {
            return Err(ws::Error::Closed);
        }
        let mut payload = Vec::new();
        if let Some(close) = close {
            if !ws::frame::valid_code(close.code, false) || close.reason.len() > 123 {
                return Err(ws::Error::InvalidInput);
            }
            payload.extend_from_slice(&close.code.to_be_bytes());
            payload.extend_from_slice(close.reason.as_bytes());
        }
        connection.queue(8, &payload, self.limits.max_buffered_bytes, 0)?;
        connection.sent_close = true;
        Ok(())
    }

    pub fn step_websockets(&mut self, budget: usize, now: Instant) -> Vec<WebSocketEvent> {
        let mut remaining = budget.min(self.limits.bytes_per_step);
        let mut events = Vec::new();
        for id in self.websockets.keys().copied().collect::<Vec<_>>() {
            let Some(mut connection) = self.websockets.remove(&id) else {
                continue;
            };
            if now >= connection.deadline && !matches!(connection.phase, Phase::Open) {
                events.push(WebSocketEvent::Error {
                    id,
                    error: ws::Error::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "WebSocket handshake",
                    )),
                });
                continue;
            }
            let mut limits = self.limits;
            limits.max_buffered_bytes = limits
                .max_buffered_bytes
                .saturating_sub(self.buffered_bytes());
            match connection.step(
                id,
                &mut self.resolver,
                &mut self.connector,
                &self.tls_policy,
                self.pending_io.as_ref(),
                limits,
                &mut remaining,
                now,
                &mut events,
            ) {
                Ok(done) => {
                    if !done {
                        self.websockets.insert(id, connection);
                    }
                }
                Err(error) => events.push(WebSocketEvent::Error { id, error }),
            }
        }
        events
    }
}

impl WebSocketConnection {
    fn install_socket(&mut self,socket:TcpStream,tls_policy:&ClientPolicy)->Result<(),ws::Error> {
        socket.set_nonblocking(true)?;
        if self.url.scheme()=="wss" {
            let hostname=self.url.hostname();let host=hostname.trim_start_matches('[').trim_end_matches(']');
            self.phase=Phase::Tls(amazing_net_tls::PendingTls::new_http1_with_policy(host,socket,tls_policy).map_err(|e|ws::Error::Tls(e.to_string()))?);
        } else {self.transport=Some(Transport::Plain(socket));self.phase=Phase::Request;}
        Ok(())
    }

    pub(super) fn has_pending_output(&self) -> bool {
        !self.outgoing.is_empty()
    }
    pub(super) fn held(&self) -> usize {
        self.incoming.len()
            + self.message.len()
            + self.outgoing.iter().map(|(f, _)| f.len()).sum::<usize>()
    }
    fn queue(
        &mut self,
        opcode: u8,
        bytes: &[u8],
        limit: usize,
        credit: u64,
    ) -> Result<(), ws::Error> {
        let held = self.held();
        if held.saturating_add(bytes.len()).saturating_add(14) > limit {
            return Err(ws::Error::Limit);
        }
        let mut mask = [0; 4];
        ws::random(&mut mask)?;
        let mut frame = Vec::with_capacity(bytes.len() + 14);
        frame.push(0x80 | opcode);
        if bytes.len() < 126 {
            frame.push(0x80 | bytes.len() as u8);
        } else if bytes.len() <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.outgoing.push_back((frame, credit));
        Ok(())
    }

    fn write(&mut self, remaining: &mut usize) -> Result<(), ws::Error> {
        let Some((frame, _)) = self.outgoing.front() else {
            return Ok(());
        };
        if *remaining == 0 {
            return Ok(());
        }
        if self.written == frame.len() {
            if self
                .transport
                .as_mut()
                .expect("connected")
                .flush_step(remaining)?
            {
                let (_, credit) = self.outgoing.pop_front().unwrap();
                self.flushed_bytes += credit;
                self.written = 0;
            }
            return Ok(());
        }
        let end = frame.len().min(self.written.saturating_add(*remaining));
        match self
            .transport
            .as_mut()
            .expect("connected")
            .write_step(&frame[self.written..end], remaining)
        {
            Ok(0) => Err(ws::Error::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                "WebSocket write",
            ))),
            Ok(n) => {
                self.written += n;
                if self.written == frame.len()
                    && self
                        .transport
                        .as_mut()
                        .expect("connected")
                        .flush_step(remaining)?
                {
                    let (_, credit) = self.outgoing.pop_front().unwrap();
                    self.flushed_bytes += credit;
                    self.written = 0;
                }
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn read(&mut self, remaining: &mut usize, limit: usize) -> Result<(), ws::Error> {
        if *remaining == 0 {
            return Ok(());
        }
        let cap = (*remaining)
            .min(8192)
            .min(limit.saturating_sub(self.held()));
        if cap == 0 {
            return Err(ws::Error::Limit);
        }
        let mut bytes = vec![0; cap];
        match self
            .transport
            .as_mut()
            .expect("connected")
            .read_step(&mut bytes, remaining)
        {
            Ok(0) => Err(ws::Error::AbnormalClosure),
            Ok(n) => {
                self.incoming.extend_from_slice(&bytes[..n]);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn step<R: Resolver, C: Connector>(
        &mut self,
        id: WebSocketId,
        resolver: &mut R,
        connector: &mut C,
        tls_policy: &ClientPolicy,
        pending_io: Option<&PendingIoMode>,
        limits: DispatchLimits,
        remaining: &mut usize,
        now: Instant,
        events: &mut Vec<WebSocketEvent>,
    ) -> Result<bool, ws::Error> {
        if *remaining == 0 {
            return Ok(false);
        }
        match &mut self.phase {
            Phase::Resolve => {
                let hostname=self.url.hostname();let host=hostname.trim_start_matches('[').trim_end_matches(']');let port=self.url.effective_port().ok_or(ws::Error::InvalidInput)?;
                let result=if let Some(mode)=pending_io {
                    self.io_generation=self.io_generation.checked_add(1).ok_or(ws::Error::Limit)?;
                    resolver.begin_resolve(host,port,&mode.context(id.0,self.io_generation,true,self.deadline))
                } else {ResolveStep::Ready(resolver.resolve(host,port))};
                match result {ResolveStep::Busy=>{},ResolveStep::Pending(operation)=>self.phase=Phase::Resolving(operation),ResolveStep::Ready(result)=>{let addresses=result?;if addresses.is_empty(){return Err(io::Error::new(io::ErrorKind::AddrNotAvailable,"WebSocket address").into());}self.phase=Phase::Connect(addresses);}}
            }
            Phase::Resolving(operation) => {
                let mode=pending_io.ok_or(ws::Error::InvalidInput)?;
                if let Some(result)=operation.poll(&mode.context(id.0,self.io_generation,true,self.deadline)) {
                    match result? {IoValue::Addresses(addresses)=>{if addresses.is_empty(){return Err(io::Error::new(io::ErrorKind::AddrNotAvailable,"WebSocket address").into());}self.phase=Phase::Connect(addresses);},IoValue::Socket(_)=>return Err(ws::Error::InvalidInput)}
                }
            }
            Phase::Connect(addresses) => {
                let address=addresses[0];let timeout=super::connect_timeout(self.deadline,now,limits.connect_timeout)?;
                let result=if let Some(mode)=pending_io {
                    self.io_generation=self.io_generation.checked_add(1).ok_or(ws::Error::Limit)?;
                    connector.begin_connect(address,timeout,&mode.context(id.0,self.io_generation,true,self.deadline))
                } else {ConnectStep::Ready(connector.connect(address,timeout))};
                match result {
                    ConnectStep::Busy=>{},
                    ConnectStep::Pending(operation)=>{addresses.remove(0);let rest=std::mem::take(addresses);self.phase=Phase::Connecting(rest,operation);},
                    ConnectStep::Ready(result)=>{addresses.remove(0);match result {Ok(socket)=>self.install_socket(socket,tls_policy)?,Err(_)if !addresses.is_empty()=>{},Err(e)=>return Err(e.into())}}
                }
            }
            Phase::Connecting(addresses,operation) => {
                let mode=pending_io.ok_or(ws::Error::InvalidInput)?;
                if let Some(result)=operation.poll(&mode.context(id.0,self.io_generation,true,self.deadline)) {
                    match result {Ok(IoValue::Socket(socket))=>self.install_socket(socket,tls_policy)?,Ok(IoValue::Addresses(_))=>return Err(ws::Error::InvalidInput),Err(_)if !addresses.is_empty()=>{let rest=std::mem::take(addresses);self.phase=Phase::Connect(rest);},Err(e)=>return Err(e.into())}
                }
            }
            Phase::Tls(tls) => {
                let step = tls.step_metered(*remaining);
                *remaining -= step.socket_bytes;
                let stream = step.result.map_err(|e| ws::Error::Tls(e.to_string()))?;
                if let Some(stream) = stream {
                    self.transport = Some(Transport::Tls(stream));
                    self.phase = Phase::Request;
                }
            }
            Phase::Request => {
                self.write(remaining)?;
                if self.outgoing.is_empty() {
                    self.phase = Phase::Response;
                }
            }
            Phase::Response => {
                self.read(remaining, handshake::MAX_HANDSHAKE + 8192)?;
                if let Some(end) = self.incoming.windows(4).position(|w| w == b"\r\n\r\n") {
                    let end = end + 4;
                    let protocol = handshake::validate_response(
                        &self.incoming[..end],
                        &self.expected,
                        &self.options.protocols,
                    )?;
                    self.incoming.drain(..end);
                    self.phase = Phase::Open;
                    events.push(WebSocketEvent::Open { id, protocol });
                } else if self.incoming.len() > handshake::MAX_HANDSHAKE {
                    return Err(ws::Error::Limit);
                }
            }
            Phase::Open => {
                self.write(remaining)?;
                if self.flushed_bytes > 0 {
                    events.push(WebSocketEvent::Sent {
                        id,
                        bytes: std::mem::take(&mut self.flushed_bytes),
                    });
                }
                self.consume(id, limits.max_buffered_bytes, events)?;
                if self.received_close && self.outgoing.is_empty() {
                    return Ok(true);
                }
                if !self.received_close {
                    self.read(remaining, limits.max_buffered_bytes)?;
                    self.consume(id, limits.max_buffered_bytes, events)?;
                }
            }
        }
        Ok(false)
    }

    fn consume(
        &mut self,
        id: WebSocketId,
        limit: usize,
        events: &mut Vec<WebSocketEvent>,
    ) -> Result<(), ws::Error> {
        let mut processed = 0;
        loop {
            if processed == 64 {
                return Ok(());
            }
            if self.incoming.len() < 2 {
                return Ok(());
            }
            let first = self.incoming[0];
            let second = self.incoming[1];
            let opcode = first & 0x0f;
            let fin = first & 0x80 != 0;
            if first & 0x70 != 0 || second & 0x80 != 0 || !matches!(opcode, 0 | 1 | 2 | 8 | 9 | 10)
            {
                return Err(ws::Error::Protocol);
            }
            let short = second & 0x7f;
            let (header, len) = match short {
                126 if self.incoming.len() >= 4 => (
                    4,
                    u16::from_be_bytes([self.incoming[2], self.incoming[3]]) as u64,
                ),
                127 if self.incoming.len() >= 10 => (
                    10,
                    u64::from_be_bytes(self.incoming[2..10].try_into().unwrap()),
                ),
                126 | 127 => return Ok(()),
                n => (2, n as u64),
            };
            if (short == 126 && len < 126)
                || (short == 127 && (len < 65536 || len >> 63 != 0))
                || opcode >= 8 && (!fin || len > 125)
            {
                return Err(ws::Error::Protocol);
            }
            if len > self.options.limits.frame_bytes as u64 || len > limit as u64 {
                return Err(ws::Error::Limit);
            }
            if self.incoming.len() < header + len as usize {
                return Ok(());
            }
            let payload = self.incoming[header..header + len as usize].to_vec();
            self.incoming.drain(..header + len as usize);
            self.frames += 1;
            processed += 1;
            if self.frames > self.options.limits.frames {
                return Err(ws::Error::Limit);
            }
            match opcode {
                9 => self.queue(10, &payload, limit, 0)?,
                10 => {}
                8 => {
                    if payload.len() == 1 {
                        return Err(ws::Error::Protocol);
                    }
                    let close = if payload.is_empty() {
                        None
                    } else {
                        let code = u16::from_be_bytes([payload[0], payload[1]]);
                        if !ws::frame::valid_code(code, true) {
                            return Err(ws::Error::Protocol);
                        }
                        let reason = std::str::from_utf8(&payload[2..])
                            .map_err(|_| ws::Error::Utf8)?
                            .to_owned();
                        Some(ws::Close { code, reason })
                    };
                    self.received_close = true;
                    if !self.sent_close {
                        self.queue(8, &payload, limit, 0)?;
                        self.sent_close = true;
                    }
                    events.push(WebSocketEvent::Close { id, close });
                    return Ok(());
                }
                0 | 1 | 2 => {
                    if opcode == 0 && self.fragmented_opcode.is_none()
                        || opcode != 0 && self.fragmented_opcode.is_some()
                    {
                        return Err(ws::Error::Protocol);
                    }
                    let kind = if opcode == 0 {
                        self.fragmented_opcode.unwrap()
                    } else {
                        opcode
                    };
                    if self.message.len().saturating_add(payload.len())
                        > self.options.limits.message_bytes
                    {
                        return Err(ws::Error::Limit);
                    }
                    if kind == 1 {
                        self.validator.feed(&payload, fin)?;
                    }
                    self.message.extend_from_slice(&payload);
                    if fin {
                        let bytes = std::mem::take(&mut self.message);
                        let message = if kind == 1 {
                            ws::Event::Text(String::from_utf8(bytes).map_err(|_| ws::Error::Utf8)?)
                        } else {
                            ws::Event::Binary(bytes)
                        };
                        self.validator = Validator::default();
                        self.fragmented_opcode = None;
                        events.push(WebSocketEvent::Message { id, message });
                    } else {
                        self.fragmented_opcode = Some(kind);
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn wss_frame_credit_and_close_reply_wait_for_ciphertext_drain() {
        use amazing_net_tls::test_support;
        for budget in [1, 7, 31, 128, 1024] {
            let (peer_done, done) = std::sync::mpsc::channel();
            let (port, peer) = test_support::serve(vec![b"http/1.1".to_vec()], move |io| {
                let (opcode, payload) = client_frame(io);
                assert_eq!(opcode, 1);
                assert_eq!(payload, b"hello");
                io.write_all(&[0x88, 2, 3, 232]).unwrap();
                io.flush().unwrap();
                let (opcode, payload) = client_frame(io);
                assert_eq!(opcode, 8);
                assert_eq!(payload, [3, 232]);
                peer_done.send(()).unwrap();
            });
            let tls = test_support::connect_nonblocking(port).unwrap();
            let mut dispatcher = Dispatcher::new(DispatchLimits::default());
            let id = dispatcher
                .open_websocket(
                    &format!("wss://localhost:{port}/"),
                    ws::Options::default(),
                    None,
                    Instant::now(),
                )
                .unwrap();
            let connection = dispatcher.websockets.get_mut(&id).unwrap();
            connection.transport = Some(Transport::Tls(tls));
            connection.phase = Phase::Open;
            connection.outgoing.clear();
            dispatcher
                .send_websocket(id, ws::Event::Text("hello".into()))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut sent = 0;
            let mut closed = 0;
            while dispatcher.websockets_in_flight() != 0 {
                assert!(Instant::now() < deadline, "WSS reply stalled");
                for event in dispatcher.step_websockets(budget, Instant::now()) {
                    match event {
                        WebSocketEvent::Sent { bytes, .. } => {
                            sent += bytes;
                            if let Some(connection) = dispatcher.websockets.get(&id) {
                                if let Some(Transport::Tls(tls)) = &connection.transport {
                                    assert!(tls.output_drained(), "Sent preceded TLS drain");
                                }
                            }
                        }
                        WebSocketEvent::Close { .. } => closed += 1,
                        WebSocketEvent::Error { error, .. } => panic!("{error:?}"),
                        _ => {}
                    }
                }
                std::thread::yield_now();
            }
            assert_eq!(sent, 5);
            assert_eq!(closed, 1);
            done.recv_timeout(Duration::from_secs(5)).unwrap();
            peer.join().unwrap().unwrap();
        }
    }

    fn handshake(stream: &mut TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let text = String::from_utf8(request).unwrap();
        let key = text
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
            .unwrap()
            .trim();
        let response = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", handshake::accept(key));
        stream.write_all(response.as_bytes()).unwrap();
    }

    fn client_frame(stream: &mut impl Read) -> (u8, Vec<u8>) {
        let mut head = [0; 2];
        stream.read_exact(&mut head).unwrap();
        assert_ne!(head[1] & 0x80, 0);
        let len = match head[1] & 0x7f {
            126 => {
                let mut n = [0; 2];
                stream.read_exact(&mut n).unwrap();
                u16::from_be_bytes(n) as usize
            }
            n => n as usize,
        };
        let mut mask = [0; 4];
        stream.read_exact(&mut mask).unwrap();
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).unwrap();
        for (i, b) in bytes.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
        (head[0] & 0x0f, bytes)
    }

    fn pump(dispatcher: &mut Dispatcher, now: Instant) -> Vec<WebSocketEvent> {
        let mut all = Vec::new();
        for _ in 0..1000 {
            all.extend(dispatcher.step_websockets(4096, now));
            if !all.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        all
    }

    #[test]
    fn fragmented_echo_ping_pong_and_client_close() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            handshake(&mut stream);
            assert_eq!(client_frame(&mut stream), (1, b"hello".to_vec()));
            stream
                .write_all(&[
                    0x01, 2, b'h', b'e', 0x89, 1, b'x', 0x80, 3, b'l', b'l', b'o',
                ])
                .unwrap();
            assert_eq!(client_frame(&mut stream), (10, b"x".to_vec()));
            assert_eq!(
                client_frame(&mut stream),
                (8, 1000u16.to_be_bytes().to_vec())
            );
            stream.write_all(&[0x88, 2, 3, 232]).unwrap();
        });
        let now = Instant::now();
        let mut dispatcher = Dispatcher::new(DispatchLimits::default());
        let id = dispatcher
            .open_websocket(
                &format!("ws://127.0.0.1:{port}/"),
                ws::Options::default(),
                None,
                now,
            )
            .unwrap();
        assert!(pump(&mut dispatcher, now)
            .iter()
            .any(|e| matches!(e, WebSocketEvent::Open { .. })));
        dispatcher
            .send_websocket(id, ws::Event::Text("hello".into()))
            .unwrap();
        let mut message = false;
        for _ in 0..1000 {
            if pump(&mut dispatcher, now).iter().any(|e| matches!(e, WebSocketEvent::Message { message: ws::Event::Text(s), .. } if s == "hello")) { message = true; break; }
        }
        assert!(message);
        dispatcher
            .close_websocket(
                id,
                Some(ws::Close {
                    code: 1000,
                    reason: String::new(),
                }),
            )
            .unwrap();
        assert!(pump(&mut dispatcher, now).iter().any(|e| matches!(
            e,
            WebSocketEvent::Close {
                close: Some(ws::Close { code: 1000, .. }),
                ..
            }
        )));
        server.join().unwrap();
    }

    #[test]
    fn server_close_and_oversized_frame() {
        for oversized in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                handshake(&mut stream);
                if oversized {
                    stream.write_all(&[0x82, 126, 0x00, 0x7e]).unwrap();
                } else {
                    stream.write_all(&[0x88, 2, 3, 232]).unwrap();
                    assert_eq!(client_frame(&mut stream).0, 8);
                }
            });
            let now = Instant::now();
            let mut dispatcher = Dispatcher::new(DispatchLimits::default());
            let mut options = ws::Options::default();
            options.limits.frame_bytes = 64;
            dispatcher
                .open_websocket(&format!("ws://127.0.0.1:{port}/"), options, None, now)
                .unwrap();
            assert!(pump(&mut dispatcher, now)
                .iter()
                .any(|e| matches!(e, WebSocketEvent::Open { .. })));
            let events = pump(&mut dispatcher, now);
            if oversized {
                assert!(events.iter().any(|e| matches!(
                    e,
                    WebSocketEvent::Error {
                        error: ws::Error::Limit,
                        ..
                    }
                )));
            } else {
                assert!(events
                    .iter()
                    .any(|e| matches!(e, WebSocketEvent::Close { .. })));
                // The close event precedes the nonblocking write of its reply.
                for _ in 0..100 {
                    dispatcher.step_websockets(4096, now);
                    if dispatcher.websockets.is_empty() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
            server.join().unwrap();
        }
    }
}
