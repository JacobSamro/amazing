use super::{
    frame::{self, Frame},
    hpack::{Decoder, Encoder},
    semantics, Error, Field, Limits, Reply, MAX_FRAME, PREFACE,
};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
};
/// Complete frames are fed into this adapter by the cooperative dispatcher.
/// It prevents a partial socket read from consuming any protocol parser state.
#[derive(Default)]
pub struct FrameIo {
    input: VecDeque<u8>,
    output: Vec<u8>,
}
impl FrameIo {
    pub fn feed(&mut self, frame: &[u8]) {
        self.input.extend(frame);
    }
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}
impl Read for FrameIo {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.len() > self.input.len() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        for byte in out.iter_mut() {
            *byte = self.input.pop_front().unwrap();
        }
        Ok(out.len())
    }
}
impl Write for FrameIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Failure {
    Reset(u32),
    Unprocessed,
}
struct Stream {
    send_window: i64,
    recv_window: i64,
    send_done: bool,
    recv_done: bool,
    outgoing: Vec<u8>,
    sent: usize,
    headers: Option<(u16, Vec<Field>)>,
    trailers: Vec<Field>,
    body: Vec<u8>,
    received: usize,
    streaming: bool,
    head_emitted: bool,
    length: Option<usize>,
    head: bool,
    informational: usize,
    error: Option<Failure>,
}
struct Block {
    stream: u32,
    end: bool,
    bytes: Vec<u8>,
    reject: Option<u32>,
}
/// A caller owns and drives one connection. Start several requests, call poll,
/// and take completed replies in any order. No dependency/provider types.
pub struct Client<S> {
    io: Option<S>,
    limits: Limits,
    decoder: Decoder,
    encoder: Encoder,
    streams: BTreeMap<u32, Stream>,
    next: u32,
    peer_concurrent: usize,
    peer_initial: i64,
    peer_headers: usize,
    send_window: i64,
    recv_window: i64,
    buffered: usize,
    pending: Option<Block>,
    initialized: bool,
    settings_outstanding: bool,
    goaway: Option<(u32, u32)>,
    failed: bool,
    frames: usize,
}
/// One pull event from a response stream. DATA is bounded by MAX_FRAME.
#[derive(Debug)]
pub enum StreamEvent {
    Head { status: u16, headers: Vec<Field> },
    Data(Vec<u8>),
    End { trailers: Vec<Field> },
}
impl<S: Read + Write> Client<S> {
    pub fn new(io: S, limits: Limits) -> Result<Self, Error> {
        let mut client = Self::new_pending(io, limits)?;
        client.poll()?;
        Ok(client)
    }
    /// Send the client preface but leave server SETTINGS to a later complete frame.
    pub fn new_pending(mut io: S, limits: Limits) -> Result<Self, Error> {
        let limits = limits.validate()?;
        io.write_all(PREFACE).map_err(Error::Io)?;
        let mut settings = Vec::new();
        for (id, value) in [
            (1u16, limits.dynamic_table_bytes as u32),
            (2, 0),
            (3, limits.concurrent_streams as u32),
            (6, limits.header_list_bytes as u32),
        ] {
            settings.extend_from_slice(&id.to_be_bytes());
            settings.extend_from_slice(&value.to_be_bytes());
        }
        frame::write(&mut io, 4, 0, 0, &settings)?;
        io.flush().map_err(Error::Io)?;
        let client = Self {
            io: Some(io),
            limits,
            decoder: Decoder::new(limits.dynamic_table_bytes),
            encoder: Encoder::new(),
            streams: BTreeMap::new(),
            next: 1,
            peer_concurrent: usize::MAX,
            peer_initial: 65535,
            peer_headers: 65536,
            send_window: 65535,
            recv_window: 65535,
            buffered: 0,
            pending: None,
            initialized: false,
            settings_outstanding: true,
            goaway: None,
            failed: false,
            frames: limits.frames,
        };
        Ok(client)
    }
    pub fn io_mut(&mut self) -> Option<&mut S> {
        self.io.as_mut()
    }
    pub fn start(&mut self, headers: &[Field], body: &[u8]) -> Result<u32, Error> {
        if self.failed {
            return Err(Error::Closed);
        }
        if let Some((last_stream, code)) = self.goaway {
            return Err(Error::GoAway { last_stream, code });
        }
        if self.streams.len() >= self.limits.concurrent_streams.min(self.peer_concurrent)
            || self.next > 0x7fffffff
            || body.len() > self.limits.body_bytes
            || body.len()
                > self
                    .limits
                    .buffered_body_bytes
                    .saturating_sub(self.buffered)
        {
            return Err(Error::Limit);
        }
        let head = semantics::request(headers)?;
        if semantics::length(headers)?.is_some_and(|n| n != body.len()) {
            return Err(Error::Protocol);
        }
        let block = self.encoder.encode(
            headers,
            self.limits.header_list_bytes.min(self.peer_headers),
        )?;
        if block.len() > 65536 {
            self.failed = true;
            self.io.take();
            return Err(Error::Limit);
        }
        let id = self.next;
        self.next += 2;
        self.buffered += body.len();
        self.streams.insert(
            id,
            Stream {
                send_window: self.peer_initial,
                recv_window: 65535,
                send_done: body.is_empty(),
                recv_done: false,
                outgoing: body.to_vec(),
                sent: 0,
                headers: None,
                trailers: Vec::new(),
                body: Vec::new(),
                received: 0,
                streaming: false,
                head_emitted: false,
                length: None,
                head,
                informational: 0,
                error: None,
            },
        );
        let result = (|| {
            let chunks = block.chunks(MAX_FRAME);
            let count = chunks.len();
            for (i, chunk) in chunks.enumerate() {
                frame::write(
                    self.io.as_mut().unwrap(),
                    if i == 0 { 1 } else { 9 },
                    (if i + 1 == count { 4 } else { 0 })
                        | (if i == 0 && body.is_empty() { 1 } else { 0 }),
                    id,
                    chunk,
                )?;
            }
            self.flush_pending()
        })();
        if let Err(error) = result {
            self.failed = true;
            self.io.take();
            return Err(error);
        }
        Ok(id)
    }
    /// Marks a stream cancelled without retaining an unbounded tombstone set.
    pub fn cancel(&mut self, id: u32) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Closed);
        }
        if !self.streams.contains_key(&id) {
            return Err(Error::Closed);
        }
        self.mark_reset(id, Failure::Reset(8));
        let result = frame::write(self.io.as_mut().unwrap(), 3, 0, id, &8u32.to_be_bytes())
            .and_then(|_| self.io.as_mut().unwrap().flush().map_err(Error::Io));
        if result.is_err() {
            self.failed = true;
            self.io.take();
        }
        result
    }
    /// Start a response whose DATA frames are released one at a time.
    pub fn start_streaming(&mut self, headers: &[Field], body: &[u8]) -> Result<u32, Error> {
        let id = self.start(headers, body)?;
        self.streams.get_mut(&id).unwrap().streaming = true;
        Ok(id)
    }
    /// Drive one stream until its next head, DATA chunk, or end event.
    pub fn next_event(&mut self, id: u32) -> Result<StreamEvent, Error> {
        loop {
            if let Some(event) = self.next_event_ready(id)? {
                return Ok(event);
            }
            self.poll()?;
        }
    }
    /// Return a queued event without reading the transport.
    pub fn next_event_ready(&mut self, id: u32) -> Result<Option<StreamEvent>, Error> {
        let s = self.streams.get_mut(&id).ok_or(Error::Closed)?;
        if !s.streaming {
            return Err(Error::Protocol);
        }
        if let Some(error) = s.error {
            self.remove(id);
            return Err(match error {
                Failure::Reset(code) => Error::StreamReset { stream: id, code },
                Failure::Unprocessed => Error::Unprocessed(id),
            });
        }
        if !s.head_emitted {
            if let Some((status, headers)) = &s.headers {
                s.head_emitted = true;
                return Ok(Some(StreamEvent::Head {
                    status: *status,
                    headers: headers.clone(),
                }));
            }
        }
        if !s.body.is_empty() {
            let data = std::mem::take(&mut s.body);
            self.buffered -= data.len();
            return Ok(Some(StreamEvent::Data(data)));
        }
        if s.recv_done {
            let s = self.remove(id);
            return Ok(Some(StreamEvent::End {
                trailers: s.trailers,
            }));
        }
        Ok(None)
    }
    pub fn take_reply(&mut self, id: u32) -> Result<Option<Reply>, Error> {
        let stream = self.streams.get(&id).ok_or(Error::Closed)?;
        if let Some(error) = stream.error {
            self.remove(id);
            return Err(match error {
                Failure::Reset(code) => Error::StreamReset { stream: id, code },
                Failure::Unprocessed => Error::Unprocessed(id),
            });
        }
        if !stream.recv_done {
            if self.failed {
                return Err(Error::Closed);
            }
            return Ok(None);
        }
        let stream = self.remove(id);
        // A server can complete its response before consuming an upload. Stop
        // sending that upload, without discarding the validated response.
        if !stream.send_done {
            if let Some(io) = self.io.as_mut() {
                // Failure to cancel an upload closes the transport, but cannot
                // invalidate a response already received in full.
                if frame::write(io, 3, 0, id, &8u32.to_be_bytes())
                    .and_then(|_| io.flush().map_err(Error::Io))
                    .is_err()
                {
                    self.failed = true;
                    self.io.take();
                }
            }
        }
        let (status, headers) = stream.headers.ok_or(Error::Protocol)?;
        Ok(Some(Reply {
            status,
            headers,
            trailers: stream.trailers,
            body: stream.body,
        }))
    }
    fn remove(&mut self, id: u32) -> Stream {
        let s = self.streams.remove(&id).unwrap();
        self.buffered -= s.body.len() + s.outgoing.len();
        s
    }
    fn mark_reset(&mut self, id: u32, error: Failure) {
        if let Some(s) = self.streams.get_mut(&id) {
            self.buffered -= s.body.len() + s.outgoing.len();
            s.body = Vec::new();
            s.outgoing = Vec::new();
            s.error = Some(error);
            s.send_done = true;
            s.recv_done = true;
        }
    }
    fn flush_pending(&mut self) -> Result<(), Error> {
        // A round visits each active stream once before returning to the first.
        loop {
            let mut progressed = false;
            for (&id, s) in &mut self.streams {
                if s.send_done || s.error.is_some() {
                    continue;
                }
                let n = (self.send_window.min(s.send_window).max(0) as usize)
                    .min(MAX_FRAME)
                    .min(s.outgoing.len() - s.sent);
                if n == 0 {
                    continue;
                }
                let end = s.sent + n;
                let done = end == s.outgoing.len();
                frame::write(
                    self.io.as_mut().unwrap(),
                    0,
                    u8::from(done),
                    id,
                    &s.outgoing[s.sent..end],
                )?;
                self.send_window -= n as i64;
                s.send_window -= n as i64;
                s.sent = end;
                progressed = true;
                if done {
                    self.buffered -= s.outgoing.len();
                    s.outgoing = Vec::new();
                    s.send_done = true;
                }
            }
            if !progressed {
                break;
            }
        }
        self.io.as_mut().unwrap().flush().map_err(Error::Io)
    }
    pub fn poll(&mut self) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Closed);
        }
        let result = self.receive();
        match result {
            Err(Error::StreamReset { stream, code }) => {
                self.mark_reset(stream, Failure::Reset(code));
                if let Err(error) =
                    frame::write(self.io.as_mut().unwrap(), 3, 0, stream, &code.to_be_bytes())
                        .and_then(|_| self.io.as_mut().unwrap().flush().map_err(Error::Io))
                {
                    self.failed = true;
                    self.io.take();
                    return Err(error);
                }
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                let mut payload = [0; 8];
                payload[4..].copy_from_slice(&error.code().to_be_bytes());
                let _ = frame::write(self.io.as_mut().unwrap(), 7, 0, 0, &payload);
                let _ = self.io.as_mut().unwrap().flush();
                self.io.take();
                Err(error)
            }
            Ok(()) => {
                let result = self.flush_pending();
                if result.is_err() {
                    self.failed = true;
                    self.io.take();
                }
                result
            }
        }
    }
    fn receive(&mut self) -> Result<(), Error> {
        if self.frames == 0 {
            return Err(Error::Limit);
        }
        self.frames -= 1;
        let frame = Frame::read(self.io.as_mut().unwrap())?;
        if !self.initialized && (frame.kind != 4 || frame.flags & 1 != 0) {
            return Err(Error::Protocol);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|block| frame.kind != 9 || frame.stream != block.stream)
        {
            return Err(Error::Protocol);
        }
        match frame.kind {
            0 => self.data(frame),
            1 | 9 => self.headers(frame),
            2 => {
                if frame::number(&frame.payload) & 0x7fffffff == frame.stream {
                    return Err(Error::StreamReset {
                        stream: frame.stream,
                        code: 1,
                    });
                }
                Ok(())
            }
            3 => {
                self.known(frame.stream)?;
                if self
                    .streams
                    .get(&frame.stream)
                    .is_some_and(|s| s.send_done && s.recv_done)
                {
                    return Ok(());
                }
                // NO_ERROR after a fully received response may merely stop an upload.
                let code = frame::number(&frame.payload);
                if code == 0
                    && self
                        .streams
                        .get(&frame.stream)
                        .is_some_and(|s| s.recv_done && s.headers.is_some())
                {
                    let s = self.streams.get_mut(&frame.stream).unwrap();
                    self.buffered -= s.outgoing.len();
                    s.outgoing = Vec::new();
                    s.send_done = true;
                } else {
                    self.mark_reset(frame.stream, Failure::Reset(code));
                }
                Ok(())
            }
            4 => self.settings(frame),
            6 => {
                if frame.flags & 1 == 0 {
                    frame::write(self.io.as_mut().unwrap(), 6, 1, 0, &frame.payload)?;
                }
                Ok(())
            }
            7 => {
                let last = frame::number(&frame.payload) & 0x7fffffff;
                let code = frame::number(&frame.payload[4..]);
                if self.goaway.is_some_and(|(old, _)| last > old) {
                    return Err(Error::Protocol);
                }
                self.goaway = Some((last, code));
                let ids: Vec<_> = self.streams.keys().copied().collect();
                for id in ids {
                    if self
                        .streams
                        .get(&id)
                        .is_some_and(|s| s.send_done && s.recv_done)
                    {
                        continue;
                    }
                    if id > last {
                        self.mark_reset(id, Failure::Unprocessed);
                    } else if code != 0 {
                        self.mark_reset(id, Failure::Reset(code));
                    }
                }
                Ok(())
            }
            8 => {
                let amount = frame::number(&frame.payload) & 0x7fffffff;
                if frame.stream == 0 {
                    if amount == 0 {
                        return Err(Error::Protocol);
                    }
                    self.send_window += i64::from(amount);
                    if self.send_window > 0x7fffffff {
                        return Err(Error::FlowControl);
                    }
                } else {
                    self.known(frame.stream)?;
                    if let Some(s) = self.streams.get_mut(&frame.stream) {
                        if (s.send_done && s.recv_done) || s.error.is_some() {
                            return Ok(());
                        }
                        if amount == 0 {
                            return Err(Error::StreamReset {
                                stream: frame.stream,
                                code: 1,
                            });
                        }
                        s.send_window += i64::from(amount);
                        if s.send_window > 0x7fffffff {
                            return Err(Error::StreamReset {
                                stream: frame.stream,
                                code: 3,
                            });
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()), // Unknown extensions consume bounded frame/CPU budgets.
        }
    }
    fn known(&self, id: u32) -> Result<(), Error> {
        if id == 0 || id % 2 == 0 || id >= self.next {
            Err(Error::Protocol)
        } else {
            Ok(())
        }
    }
    fn settings(&mut self, frame: Frame) -> Result<(), Error> {
        if frame.flags & 1 != 0 {
            if !self.settings_outstanding {
                return Err(Error::Protocol);
            }
            self.settings_outstanding = false;
            self.decoder.acknowledge_limit();
            return Ok(());
        }
        // Validate the complete payload before applying any of it.
        for setting in frame.payload.chunks_exact(6) {
            let id = u16::from_be_bytes([setting[0], setting[1]]);
            let value = frame::number(&setting[2..]);
            if id == 2 {
                return Err(Error::Protocol);
            }
            if id == 4 && value > 0x7fffffff {
                return Err(Error::FlowControl);
            }
            if id == 5 && !(16384..=16777215).contains(&value) {
                return Err(Error::Protocol);
            }
        }
        for setting in frame.payload.chunks_exact(6) {
            let id = u16::from_be_bytes([setting[0], setting[1]]);
            let value = frame::number(&setting[2..]);
            match id {
                1 => self.encoder.resize(value as usize),
                3 => self.peer_concurrent = value as usize,
                4 => {
                    let change = i64::from(value) - self.peer_initial;
                    for s in self.streams.values_mut() {
                        if !(s.send_done && s.recv_done) && s.error.is_none() {
                            s.send_window += change;
                            if s.send_window > 0x7fffffff {
                                return Err(Error::FlowControl);
                            }
                        }
                    }
                    self.peer_initial = i64::from(value);
                }
                6 => self.peer_headers = (value as usize).min(65536),
                _ => {} // Sending <=16384 always satisfies peer MAX_FRAME_SIZE.
            }
        }
        self.initialized = true;
        frame::write(self.io.as_mut().unwrap(), 4, 1, 0, &[])
    }
    fn headers(&mut self, frame: Frame) -> Result<(), Error> {
        if frame.kind == 1 {
            self.known(frame.stream)?;
            let mut data = frame::unpad(&frame.payload, frame.flags & 8 != 0)?;
            let mut reject = None;
            if frame.flags & 32 != 0 {
                if data.len() < 5 {
                    return Err(Error::FrameSize);
                }
                if frame::number(data) & 0x7fffffff == frame.stream {
                    reject = Some(1);
                }
                data = &data[5..];
            }
            self.pending = Some(Block {
                stream: frame.stream,
                end: frame.flags & 1 != 0,
                bytes: Vec::new(),
                reject,
            });
            self.append_block(data)?;
        } else {
            if self.pending.is_none() {
                return Err(Error::Protocol);
            }
            self.append_block(&frame.payload)?;
        }
        if frame.flags & 4 == 0 {
            return Ok(());
        }
        let block = self.pending.take().unwrap();
        let fields = self
            .decoder
            .decode(&block.bytes, self.limits.header_list_bytes)?;
        if let Some(code) = block.reject {
            return Err(Error::StreamReset {
                stream: block.stream,
                code,
            });
        }
        let Some(s) = self.streams.get_mut(&block.stream) else {
            return Ok(());
        }; // Still decode closed streams to preserve HPACK state.
        if s.error.is_some() || (s.send_done && s.recv_done) {
            return Ok(());
        }
        let err = || Error::StreamReset {
            stream: block.stream,
            code: 1,
        };
        if s.recv_done {
            return Err(Error::StreamReset {
                stream: block.stream,
                code: 5,
            });
        }
        let trailers = s.headers.is_some();
        let status = semantics::response(&fields, trailers).map_err(|_| err())?;
        if trailers {
            if !block.end {
                return Err(err());
            }
            s.trailers = fields;
        } else {
            let status = status.unwrap();
            if status < 200 {
                s.informational += 1;
                if block.end
                    || s.informational > 8
                    || fields.iter().any(|(name, _)| name == b"content-length")
                {
                    return Err(err());
                }
                return Ok(());
            }
            s.length = semantics::length(&fields).map_err(|_| err())?;
            if status == 204 && s.length.is_some() {
                return Err(err());
            }
            let fields = fields
                .into_iter()
                .filter(|(n, _)| !n.starts_with(b":"))
                .collect();
            s.headers = Some((status, fields));
        }
        if block.end {
            Self::end(s, block.stream)?;
        }
        Ok(())
    }
    fn append_block(&mut self, data: &[u8]) -> Result<(), Error> {
        let block = self.pending.as_mut().unwrap();
        if data.len() > 65536usize.saturating_sub(block.bytes.len()) {
            return Err(Error::Limit);
        }
        block
            .bytes
            .try_reserve_exact(data.len())
            .map_err(|_| Error::Limit)?;
        block.bytes.extend_from_slice(data);
        Ok(())
    }
    fn end(s: &mut Stream, id: u32) -> Result<(), Error> {
        let status = s
            .headers
            .as_ref()
            .ok_or(Error::StreamReset {
                stream: id,
                code: 1,
            })?
            .0;
        if !s.head && status != 304 && s.length.is_some_and(|n| n != s.received) {
            return Err(Error::StreamReset {
                stream: id,
                code: 1,
            });
        }
        s.recv_done = true;
        Ok(())
    }
    fn data(&mut self, frame: Frame) -> Result<(), Error> {
        self.known(frame.stream)?;
        let size = frame.payload.len() as i64;
        self.recv_window -= size;
        if self.recv_window < 0 {
            return Err(Error::FlowControl);
        }
        let data = frame::unpad(&frame.payload, frame.flags & 8 != 0)?;
        // Full DATA payload (including padding) consumes connection credit even
        // for a reset/closed stream. Return that credit after bounded consumption.
        if size > 0 {
            frame::write(
                self.io.as_mut().unwrap(),
                8,
                0,
                0,
                &(size as u32).to_be_bytes(),
            )?;
            self.recv_window += size;
        }
        let Some(s) = self.streams.get_mut(&frame.stream) else {
            return Ok(());
        };
        if s.error.is_some() || (s.send_done && s.recv_done) {
            return Ok(());
        }
        if s.recv_done {
            return Err(Error::StreamReset {
                stream: frame.stream,
                code: 5,
            });
        }
        s.recv_window -= size;
        if s.recv_window < 0 {
            return Err(Error::StreamReset {
                stream: frame.stream,
                code: 3,
            });
        }
        let status = s
            .headers
            .as_ref()
            .ok_or(Error::StreamReset {
                stream: frame.stream,
                code: 1,
            })?
            .0;
        if (s.head || matches!(status, 204 | 304)) && !data.is_empty() {
            return Err(Error::StreamReset {
                stream: frame.stream,
                code: 1,
            });
        }
        if data.len() > self.limits.body_bytes.saturating_sub(s.received)
            || data.len()
                > self
                    .limits
                    .buffered_body_bytes
                    .saturating_sub(self.buffered)
        {
            return Err(Error::Limit);
        }
        s.body
            .try_reserve_exact(data.len())
            .map_err(|_| Error::Limit)?;
        s.body.extend_from_slice(data);
        s.received += data.len();
        self.buffered += data.len();
        if frame.flags & 1 != 0 {
            Self::end(s, frame.stream)?;
        } else if size > 0 {
            frame::write(
                self.io.as_mut().unwrap(),
                8,
                0,
                frame.stream,
                &(size as u32).to_be_bytes(),
            )?;
            s.recv_window += size;
        }
        Ok(())
    }
}
