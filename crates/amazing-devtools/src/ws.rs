//! The RFC 6455 WebSocket server-side message layer.
//!
//! CDP runs over a WebSocket. This module implements only what the server side
//! needs: it accepts masked client frames, reassembles fragmented messages,
//! answers pings, validates UTF-8 on text messages, echoes close, and emits
//! unmasked frames (RFC 6455 §5.1 requires servers never to mask).
//!
//! `WsConnection` is buffered and poll-based rather than blocking on a whole
//! frame. The server sets a short socket read timeout so it can interleave
//! incoming commands with outgoing engine events; a timeout is "no message
//! yet", not a lost partial frame.

use std::io::{self, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default largest single frame payload accepted.
pub const DEFAULT_MAX_FRAME: usize = 16 * 1024 * 1024;
/// Default largest reassembled message accepted.
pub const DEFAULT_MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// WebSocket frame opcodes used by CDP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl OpCode {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Continuation,
            1 => Self::Text,
            2 => Self::Binary,
            8 => Self::Close,
            9 => Self::Ping,
            10 => Self::Pong,
            _ => return None,
        })
    }

    fn as_byte(self) -> u8 {
        match self {
            Self::Continuation => 0,
            Self::Text => 1,
            Self::Binary => 2,
            Self::Close => 8,
            Self::Ping => 9,
            Self::Pong => 10,
        }
    }

    fn is_control(self) -> bool {
        matches!(self, Self::Close | Self::Ping | Self::Pong)
    }
}

/// A single decoded frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: OpCode,
    pub payload: Vec<u8>,
}

/// A complete, reassembled application message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
}

/// A WebSocket protocol, I/O or closure condition.
#[derive(Debug)]
pub enum WsError {
    Io(io::Error),
    /// The peer violated RFC 6455. The connection must be failed.
    Protocol(&'static str),
    /// A frame or message exceeded the configured limit.
    Limit,
    /// A text message was not valid UTF-8.
    Utf8,
    /// The transport ended without a close handshake.
    Eof,
    /// The peer sent a close frame.
    Closed { code: u16, reason: String },
}

impl From<io::Error> for WsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "websocket i/o error: {e}"),
            Self::Protocol(m) => write!(f, "websocket protocol error: {m}"),
            Self::Limit => f.write_str("websocket frame or message limit exceeded"),
            Self::Utf8 => f.write_str("websocket text message is not valid UTF-8"),
            Self::Eof => f.write_str("websocket transport closed unexpectedly"),
            Self::Closed { code, reason } => write!(f, "websocket closed ({code}): {reason}"),
        }
    }
}

impl std::error::Error for WsError {}

/// Encodes one frame. `mask` is `Some` only for the client role, which tests use.
pub fn encode_frame(opcode: OpCode, fin: bool, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(opcode.as_byte() | if fin { 0x80 } else { 0 });
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    let len = payload.len();
    if len < 126 {
        out.push(mask_bit | len as u8);
    } else if len <= u16::MAX as usize {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// Tries to decode one frame from the front of `buf`.
///
/// Returns `Ok(Some((frame, consumed)))` when a complete frame is available,
/// `Ok(None)` when more bytes are needed, and `Err` for a fatal violation.
/// `require_mask` is true for the server role; a frame that does not match is
/// a protocol error (RFC 6455 §5.1).
pub fn try_parse_frame(
    buf: &[u8],
    max_frame: usize,
    require_mask: bool,
) -> Result<Option<(Frame, usize)>, WsError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let b0 = buf[0];
    let b1 = buf[1];
    let fin = b0 & 0x80 != 0;
    if b0 & 0x70 != 0 {
        return Err(WsError::Protocol("reserved bits set"));
    }
    let opcode =
        OpCode::from_byte(b0 & 0x0f).ok_or(WsError::Protocol("unknown opcode"))?;
    let masked = b1 & 0x80 != 0;
    if masked != require_mask {
        return Err(WsError::Protocol("unexpected mask bit"));
    }
    let short = (b1 & 0x7f) as usize;
    if opcode.is_control() && (short > 125 || !fin) {
        return Err(WsError::Protocol("invalid control frame"));
    }
    let mut offset = 2;
    let len = match short {
        126 => {
            if buf.len() < offset + 2 {
                return Ok(None);
            }
            let n = u16::from_be_bytes([buf[offset], buf[offset + 1]]) as usize;
            offset += 2;
            if n < 126 {
                return Err(WsError::Protocol("non-minimal length"));
            }
            n
        }
        127 => {
            if buf.len() < offset + 8 {
                return Ok(None);
            }
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&buf[offset..offset + 8]);
            let n = u64::from_be_bytes(raw);
            offset += 8;
            if n >> 63 != 0 || n < 65536 {
                return Err(WsError::Protocol("invalid 64-bit length"));
            }
            usize::try_from(n).map_err(|_| WsError::Limit)?
        }
        n => n,
    };
    if len > max_frame {
        return Err(WsError::Limit);
    }
    let mut key = [0u8; 4];
    if masked {
        if buf.len() < offset + 4 {
            return Ok(None);
        }
        key.copy_from_slice(&buf[offset..offset + 4]);
        offset += 4;
    }
    if buf.len() < offset + len {
        return Ok(None);
    }
    let mut payload = buf[offset..offset + len].to_vec();
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }
    Ok(Some((
        Frame {
            fin,
            opcode,
            payload,
        },
        offset + len,
    )))
}

/// A pollable WebSocket over any `Read + Write` transport.
pub struct WsConnection<S> {
    io: S,
    buf: Vec<u8>,
    fragment: Option<(OpCode, Vec<u8>)>,
    max_frame: usize,
    max_message: usize,
    require_mask: bool,
    closed: bool,
    mask_counter: u64,
}

impl<S: Read + Write> WsConnection<S> {
    /// Creates a server-role connection: client frames must be masked.
    pub fn new(io: S) -> Self {
        Self::with_limits(io, DEFAULT_MAX_FRAME, DEFAULT_MAX_MESSAGE)
    }

    /// Creates a server-role connection with explicit limits.
    pub fn with_limits(io: S, max_frame: usize, max_message: usize) -> Self {
        Self {
            io,
            buf: Vec::new(),
            fragment: None,
            max_frame,
            max_message,
            require_mask: true,
            closed: false,
            mask_counter: 0,
        }
    }

    /// Creates a client-role connection: peer frames must not be masked.
    pub fn as_client(io: S) -> Self {
        let mut conn = Self::with_limits(io, DEFAULT_MAX_FRAME, DEFAULT_MAX_MESSAGE);
        conn.require_mask = false;
        conn
    }

    /// Whether a close frame has been sent or received.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Returns the next complete message, or `None` if the transport would
    /// block. Pings are answered and close is surfaced as [`WsError::Closed`].
    pub fn poll(&mut self) -> Result<Option<Message>, WsError> {
        loop {
            match try_parse_frame(&self.buf, self.max_frame, self.require_mask)? {
                Some((frame, used)) => {
                    self.buf.drain(..used);
                    if let Some(message) = self.handle_frame(frame)? {
                        return Ok(Some(message));
                    }
                }
                None => {
                    if !self.read_more()? {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// Sends a text message (unmasked, as a server must).
    pub fn write_message(&mut self, text: &str) -> Result<(), WsError> {
        self.write_frame(OpCode::Text, true, text.as_bytes())
    }

    /// Sends a binary message.
    pub fn write_binary(&mut self, payload: &[u8]) -> Result<(), WsError> {
        self.write_frame(OpCode::Binary, true, payload)
    }

    /// Sends a ping.
    pub fn write_ping(&mut self, payload: &[u8]) -> Result<(), WsError> {
        self.write_frame(OpCode::Ping, true, payload)
    }

    /// Sends a close frame and marks the connection closed.
    pub fn write_close(&mut self, code: u16, reason: &str) -> Result<(), WsError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut payload = Vec::with_capacity(reason.len() + 2);
        payload.extend_from_slice(&code.to_be_bytes());
        // A control frame payload is at most 125 bytes, so 123 for the reason.
        let mut end = reason.len().min(123);
        while end > 0 && !reason.is_char_boundary(end) {
            end -= 1;
        }
        payload.extend_from_slice(&reason.as_bytes()[..end]);
        self.write_frame(OpCode::Close, true, &payload)
    }

    fn write_frame(&mut self, opcode: OpCode, fin: bool, payload: &[u8]) -> Result<(), WsError> {
        if self.closed {
            return Err(WsError::Closed {
                code: 1006,
                reason: "connection closed".into(),
            });
        }
        // Servers must never mask; clients must. The mask only needs to be
        // unpredictable to a proxy, so a counter mixed with the clock is
        // enough here (and needs no external RNG).
        let mask = if self.require_mask {
            None
        } else {
            Some(self.generate_mask())
        };
        let frame = encode_frame(opcode, fin, payload, mask);
        self.io.write_all(&frame)?;
        self.io.flush()?;
        Ok(())
    }

    fn generate_mask(&mut self) -> [u8; 4] {
        self.mask_counter = self.mask_counter.wrapping_add(1);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut x = nanos ^ self.mask_counter.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let bytes = x.to_le_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3]]
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<Option<Message>, WsError> {
        match frame.opcode {
            OpCode::Ping => {
                self.write_frame(OpCode::Pong, true, &frame.payload)?;
                Ok(None)
            }
            OpCode::Pong => Ok(None),
            OpCode::Close => {
                let (code, reason) = parse_close(&frame.payload)?;
                if !self.closed {
                    self.closed = true;
                    let _ = self.io.write_all(&encode_frame(
                        OpCode::Close,
                        true,
                        &frame.payload,
                        None,
                    ));
                    let _ = self.io.flush();
                }
                Err(WsError::Closed { code, reason })
            }
            OpCode::Text | OpCode::Binary => {
                if self.fragment.is_some() {
                    return Err(WsError::Protocol("new data frame during fragmentation"));
                }
                if frame.fin {
                    self.finish(frame.opcode, frame.payload)
                } else {
                    if frame.payload.len() > self.max_message {
                        return Err(WsError::Limit);
                    }
                    self.fragment = Some((frame.opcode, frame.payload));
                    Ok(None)
                }
            }
            OpCode::Continuation => {
                let (opcode, buffer) = self
                    .fragment
                    .as_mut()
                    .ok_or(WsError::Protocol("continuation without a start frame"))?;
                let opcode = *opcode;
                buffer.extend_from_slice(&frame.payload);
                if buffer.len() > self.max_message {
                    return Err(WsError::Limit);
                }
                if frame.fin {
                    let (_, payload) = self.fragment.take().expect("checked above");
                    self.finish(opcode, payload)
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn finish(&self, opcode: OpCode, payload: Vec<u8>) -> Result<Option<Message>, WsError> {
        if payload.len() > self.max_message {
            return Err(WsError::Limit);
        }
        match opcode {
            OpCode::Text => {
                let text = String::from_utf8(payload).map_err(|_| WsError::Utf8)?;
                Ok(Some(Message::Text(text)))
            }
            OpCode::Binary => Ok(Some(Message::Binary(payload))),
            _ => Err(WsError::Protocol("invalid message opcode")),
        }
    }

    /// Reads once into the buffer. `Ok(false)` means the transport would block;
    /// an unexpected end of stream is [`WsError::Eof`].
    fn read_more(&mut self) -> Result<bool, WsError> {
        let mut scratch = [0u8; 16 * 1024];
        match self.io.read(&mut scratch) {
            Ok(0) => Err(WsError::Eof),
            Ok(n) => {
                self.buf.extend_from_slice(&scratch[..n]);
                Ok(true)
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                Ok(false)
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(true),
            Err(e) => Err(WsError::Io(e)),
        }
    }
}

/// Parses a close-frame payload into a code and reason. An absent code is
/// reported as 1005 ("no status received"), which is never sent on the wire.
pub fn parse_close(payload: &[u8]) -> Result<(u16, String), WsError> {
    match payload.len() {
        0 => Ok((1005, String::new())),
        1 => Err(WsError::Protocol("one-byte close payload")),
        _ => {
            let code = u16::from_be_bytes([payload[0], payload[1]]);
            if !valid_close_code(code) {
                return Err(WsError::Protocol("invalid close code"));
            }
            let reason = String::from_utf8(payload[2..].to_vec())
                .map_err(|_| WsError::Protocol("close reason is not UTF-8"))?;
            Ok((code, reason))
        }
    }
}

/// Close codes legal on the wire (RFC 6455 §7.4 plus registered 1012-1014).
pub fn valid_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read-from-`input` / write-to-`output` test transport. Reads return
    /// `WouldBlock` once `input` is exhausted, unless `eof` is set.
    struct Duplex {
        input: Vec<u8>,
        pos: usize,
        output: Vec<u8>,
        eof: bool,
        drip: bool,
    }

    impl Duplex {
        fn new(input: Vec<u8>) -> Self {
            Self {
                input,
                pos: 0,
                output: Vec::new(),
                eof: false,
                drip: false,
            }
        }
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos == self.input.len() {
                return if self.eof {
                    Ok(0)
                } else {
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                };
            }
            if buf.is_empty() {
                return Ok(0);
            }
            let available = self.input.len() - self.pos;
            let n = if self.drip {
                1
            } else {
                buf.len().min(available)
            };
            buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn masked(opcode: OpCode, fin: bool, payload: &[u8]) -> Vec<u8> {
        encode_frame(opcode, fin, payload, Some([0x12, 0x34, 0x56, 0x78]))
    }

    #[test]
    fn round_trips_a_text_message() {
        let raw = masked(OpCode::Text, true, b"hello");
        let mut conn = WsConnection::new(Duplex::new(raw));
        assert_eq!(
            conn.poll().unwrap(),
            Some(Message::Text("hello".into()))
        );
        assert_eq!(conn.poll().unwrap(), None);
    }

    #[test]
    fn reassembles_fragmented_text_with_control_frames_between() {
        let mut raw = masked(OpCode::Text, false, b"he");
        raw.extend_from_slice(&masked(OpCode::Ping, true, b"p"));
        raw.extend_from_slice(&masked(OpCode::Continuation, true, b"llo"));
        let mut conn = WsConnection::new(Duplex::new(raw));
        assert_eq!(conn.poll().unwrap(), Some(Message::Text("hello".into())));
        // The pong was written unmasked and echoes the ping payload.
        assert_eq!(conn.io.output, encode_frame(OpCode::Pong, true, b"p", None));
    }

    #[test]
    fn close_is_surfaced_and_echoed() {
        let payload = 1000u16.to_be_bytes();
        let mut conn = WsConnection::new(Duplex::new(masked(OpCode::Close, true, &payload)));
        match conn.poll() {
            Err(WsError::Closed { code, .. }) => assert_eq!(code, 1000),
            other => panic!("expected close, got {other:?}"),
        }
        assert!(conn.is_closed());
        assert_eq!(conn.io.output, encode_frame(OpCode::Close, true, &payload, None));
    }

    #[test]
    fn rejects_unmasked_client_frame() {
        let raw = encode_frame(OpCode::Text, true, b"hi", None);
        let mut conn = WsConnection::new(Duplex::new(raw));
        assert!(matches!(conn.poll(), Err(WsError::Protocol(_))));
    }

    #[test]
    fn accepts_unmasked_server_frame_as_client() {
        let raw = encode_frame(OpCode::Text, true, b"hi", None);
        let mut conn = WsConnection::as_client(Duplex::new(raw));
        assert_eq!(conn.poll().unwrap(), Some(Message::Text("hi".into())));
    }

    #[test]
    fn rejects_bad_control_frames() {
        // Fragmented ping.
        let raw = masked(OpCode::Ping, false, b"x");
        assert!(matches!(
            WsConnection::new(Duplex::new(raw)).poll(),
            Err(WsError::Protocol(_))
        ));
        // Ping with 126-byte payload.
        let big = vec![0u8; 126];
        let raw = masked(OpCode::Ping, true, &big);
        assert!(matches!(
            WsConnection::new(Duplex::new(raw)).poll(),
            Err(WsError::Protocol(_))
        ));
    }

    #[test]
    fn rejects_reserved_bits_and_unknown_opcodes() {
        let mut raw = masked(OpCode::Text, true, b"x");
        raw[0] |= 0x40; // RSV1
        assert!(matches!(
            WsConnection::new(Duplex::new(raw)).poll(),
            Err(WsError::Protocol(_))
        ));
        let mut raw = masked(OpCode::Text, true, b"x");
        raw[0] = (raw[0] & 0xf0) | 3; // opcode 3
        assert!(matches!(
            WsConnection::new(Duplex::new(raw)).poll(),
            Err(WsError::Protocol(_))
        ));
    }

    #[test]
    fn rejects_invalid_utf8_text() {
        let raw = masked(OpCode::Text, true, &[0xff, 0xfe]);
        assert!(matches!(
            WsConnection::new(Duplex::new(raw)).poll(),
            Err(WsError::Utf8)
        ));
    }

    #[test]
    fn enforces_frame_and_message_limits() {
        let raw = masked(OpCode::Text, true, b"0123456789");
        let mut conn = WsConnection::with_limits(Duplex::new(raw), 4, 4);
        assert!(matches!(conn.poll(), Err(WsError::Limit)));

        // Two 4-byte fragments exceed a 6-byte message limit only on finish.
        let mut raw = masked(OpCode::Text, false, b"abcd");
        raw.extend_from_slice(&masked(OpCode::Continuation, true, b"efgh"));
        let mut conn = WsConnection::with_limits(Duplex::new(raw), 8, 6);
        assert!(matches!(conn.poll(), Err(WsError::Limit)));
    }

    #[test]
    fn parses_partial_frames_across_reads() {
        let raw = masked(OpCode::Text, true, b"partial message");
        let raw_len = raw.len();
        let mut duplex = Duplex::new(raw);
        duplex.drip = true;
        let mut conn = WsConnection::new(duplex);
        let mut seen = None;
        for _ in 0..=raw_len {
            if let Some(message) = conn.poll().unwrap() {
                seen = Some(message);
                break;
            }
        }
        assert_eq!(seen, Some(Message::Text("partial message".into())));
    }

    #[test]
    fn eof_during_frame_is_reported() {
        let raw = masked(OpCode::Text, true, b"truncated");
        let mut duplex = Duplex::new(raw[..4].to_vec());
        duplex.eof = true;
        let mut conn = WsConnection::new(duplex);
        assert!(matches!(conn.poll(), Err(WsError::Eof)));
    }

    #[test]
    fn close_codes_are_validated() {
        assert!(valid_close_code(1000));
        assert!(valid_close_code(1011));
        assert!(valid_close_code(4999));
        assert!(!valid_close_code(1005));
        assert!(!valid_close_code(1015));
        assert!(!valid_close_code(1006));
        assert!(!valid_close_code(999));
        assert_eq!(parse_close(&[]).unwrap(), (1005, String::new()));
        assert!(matches!(parse_close(&[0x00]), Err(WsError::Protocol(_))));
        assert!(parse_close(&[0x03, 0xe8]).unwrap().0 == 1000);
    }

    #[test]
    fn encode_frame_uses_minimal_lengths() {
        assert_eq!(encode_frame(OpCode::Binary, true, &[0u8; 125], None)[1], 125);
        assert_eq!(encode_frame(OpCode::Binary, true, &[0u8; 126], None)[1], 126);
        let long = encode_frame(OpCode::Binary, true, &[0u8; 65536], None);
        assert_eq!(long[1], 127);
    }
}
