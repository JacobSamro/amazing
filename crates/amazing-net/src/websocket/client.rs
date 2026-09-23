use super::{frame, handshake, utf8, Close, Control, Error, Event, Limits, Options, State};
use std::{
    collections::VecDeque,
    io::{Read, Write},
};
/// Owns one blocking connection. poll() handles one frame (including controls),
/// so callers can drain pop_control() between frames. No hidden reader thread.
pub struct Client<S> {
    io: Option<S>,
    limits: Limits,
    random: fn(&mut [u8]) -> Result<(), Error>,
    selected: Option<String>,
    state: State,
    fragment: Option<u8>,
    message: Vec<u8>,
    utf8: utf8::Validator,
    controls: VecDeque<Control>,
    frames: usize,
}
impl<S: Read + Write> Client<S> {
    /// The supplied transport must already be connected/authenticated and use
    /// HTTP/1.1 (never an h2-negotiated TLS stream). connect() enforces that.
    pub fn handshake(
        mut io: S,
        authority: &str,
        target: &str,
        options: &Options,
    ) -> Result<Self, Error> {
        let limits = options.limits.validate()?;
        let selected = handshake::perform(&mut io, authority, target, options, super::random)?;
        Ok(Self::established(io, limits, selected, super::random))
    }
    fn established(
        io: S,
        limits: Limits,
        selected: Option<String>,
        random: fn(&mut [u8]) -> Result<(), Error>,
    ) -> Self {
        Self {
            io: Some(io),
            limits,
            random,
            selected,
            state: State::Open,
            fragment: None,
            message: Vec::new(),
            utf8: utf8::Validator::default(),
            controls: VecDeque::new(),
            frames: limits.frames,
        }
    }
    pub fn protocol(&self) -> Option<&str> {
        self.selected.as_deref()
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn pop_control(&mut self) -> Option<Control> {
        self.controls.pop_front()
    }
    pub fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.send_data(1, text.as_bytes())
    }
    pub fn send_binary(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.send_data(2, bytes)
    }
    fn send_data(&mut self, opcode: u8, bytes: &[u8]) -> Result<(), Error> {
        if self.state != State::Open {
            return Err(Error::Closed);
        }
        if bytes.len() > self.limits.message_bytes {
            return Err(Error::Limit);
        }
        let result = (|| {
            let io = self.io.as_mut().ok_or(Error::Closed)?;
            if bytes.is_empty() {
                frame::write(io, self.random, opcode, true, bytes)?;
            } else {
                let chunks = bytes.chunks(self.limits.frame_bytes);
                let count = chunks.len();
                for (i, chunk) in chunks.enumerate() {
                    frame::write(
                        io,
                        self.random,
                        if i == 0 { opcode } else { 0 },
                        i + 1 == count,
                        chunk,
                    )?;
                }
            }
            io.flush()?;
            Ok(())
        })();
        self.outcome(result)
    }
    pub fn ping(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.send_control(9, bytes)
    }
    pub fn pong(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.send_control(10, bytes)
    }
    fn send_control(&mut self, opcode: u8, bytes: &[u8]) -> Result<(), Error> {
        if matches!(self.state, State::Closed | State::Failed) {
            return Err(Error::Closed);
        }
        if bytes.len() > 125 || bytes.len() > self.limits.frame_bytes {
            return Err(Error::Limit);
        }
        let result = (|| {
            let io = self.io.as_mut().ok_or(Error::Closed)?;
            frame::write(io, self.random, opcode, true, bytes)?;
            io.flush()?;
            Ok(())
        })();
        self.outcome(result)
    }
    /// Send one close and await Event::Close via poll(). None has no wire status
    /// (1005/1006/1015 are never transmitted). Timeout/EOF fails the connection.
    pub fn close(&mut self, code: Option<u16>, reason: &str) -> Result<(), Error> {
        if self.state != State::Open {
            return Err(Error::Closed);
        }
        if reason.len() > 123 {
            return Err(Error::Limit);
        }
        if code.is_none() && !reason.is_empty()
            || code.is_some_and(|c| !frame::valid_code(c, false))
        {
            return Err(Error::InvalidInput);
        }
        let mut bytes = [0; 125];
        let len = if let Some(c) = code {
            bytes[..2].copy_from_slice(&c.to_be_bytes());
            bytes[2..2 + reason.len()].copy_from_slice(reason.as_bytes());
            2 + reason.len()
        } else {
            0
        };
        self.send_control(8, &bytes[..len])?;
        self.state = State::Closing;
        Ok(())
    }
    /// After both Close frames, wait for the server's TCP/TLS close using the
    /// transport's read timeout, then release it. No further data is accepted.
    pub fn finish_close(&mut self) -> Result<(), Error> {
        if self.state != State::Closed {
            return Err(Error::Closed);
        }
        let Some(mut io) = self.io.take() else {
            return Ok(());
        };
        let mut byte = [0];
        match io.read(&mut byte) {
            Ok(0) => Ok(()),
            Ok(_) => Err(Error::Protocol),
            Err(e) => Err(Error::Io(e)),
        }
    }
    fn outcome<T>(&mut self, result: Result<T, Error>) -> Result<T, Error> {
        if result.is_err() {
            self.state = State::Failed;
            self.io.take();
            self.message = Vec::new();
            self.controls.clear();
        }
        result
    }
    /// One frame per call. Invalid UTF-8 fails immediately with close 1007;
    /// malformed framing uses 1002 and resource refusal 1009. Errors poison the
    /// connection and drop its transport; partial messages are never returned.
    pub fn poll(&mut self) -> Result<Option<Event>, Error> {
        if matches!(self.state, State::Closed | State::Failed) {
            return Err(Error::Closed);
        }
        let result = self.receive();
        if let Err(error) = &result {
            if self.state == State::Open
                && !matches!(
                    error,
                    Error::Io(_) | Error::AbnormalClosure | Error::Entropy
                )
            {
                if let Some(io) = self.io.as_mut() {
                    let _ =
                        frame::write(io, self.random, 8, true, &error.close_code().to_be_bytes());
                    let _ = io.flush();
                }
            }
        }
        self.outcome(result)
    }
    fn receive(&mut self) -> Result<Option<Event>, Error> {
        if self.frames == 0 {
            return Err(Error::Limit);
        }
        self.frames -= 1;
        let io = self.io.as_mut().ok_or(Error::Closed)?;
        let head = frame::Header::read(io, self.limits.frame_bytes)?;
        if head.opcode >= 8 {
            if head.opcode != 8 && self.controls.len() >= self.limits.queued_controls {
                return Err(Error::Limit);
            }
            let mut bytes = [0; 125];
            io.read_exact(&mut bytes[..head.len])?;
            let bytes = &bytes[..head.len];
            if head.opcode == 8 {
                if head.len == 1 {
                    return Err(Error::Protocol);
                }
                let close = if bytes.is_empty() {
                    None
                } else {
                    let code = u16::from_be_bytes(bytes[..2].try_into().unwrap());
                    if !frame::valid_code(code, true) {
                        return Err(Error::Protocol);
                    }
                    let reason = std::str::from_utf8(&bytes[2..]).map_err(|_| Error::Utf8)?;
                    Some(Close {
                        code,
                        reason: reason.into(),
                    })
                };
                if self.state == State::Open {
                    frame::write(io, self.random, 8, true, bytes)?;
                    io.flush()?;
                }
                self.state = State::Closed;
                self.message = Vec::new();
                self.fragment = None;
                self.controls.clear();
                return Ok(Some(Event::Close(close)));
            }
            if head.opcode == 9 {
                frame::write(io, self.random, 10, true, bytes)?;
                io.flush()?;
            }
            self.controls.push_back(if head.opcode == 9 {
                Control::Ping(bytes.to_vec())
            } else {
                Control::Pong(bytes.to_vec())
            });
            return Ok(None);
        }
        let opcode = if head.opcode == 0 {
            self.fragment.ok_or(Error::Protocol)?
        } else {
            if self.fragment.is_some() {
                return Err(Error::Protocol);
            }
            head.opcode
        };
        if head.len > self.limits.message_bytes.saturating_sub(self.message.len()) {
            return Err(Error::Limit);
        }
        // All attacker-sized checks precede the reserve and any payload read.
        self.message
            .try_reserve_exact(head.len)
            .map_err(|_| Error::Limit)?;
        let mut remaining = head.len;
        let mut scratch = [0; 4096];
        while remaining > 0 {
            let count = remaining.min(scratch.len());
            io.read_exact(&mut scratch[..count])?;
            if opcode == 1 {
                self.utf8.feed(&scratch[..count], false)?;
            }
            self.message.extend_from_slice(&scratch[..count]);
            remaining -= count;
        }
        if !head.fin {
            self.fragment = Some(opcode);
            return Ok(None);
        }
        if opcode == 1 {
            self.utf8.feed(&[], true)?;
        }
        self.fragment = None;
        self.utf8 = utf8::Validator::default();
        let bytes = std::mem::take(&mut self.message);
        if self.state == State::Closing {
            return Ok(None);
        }
        Ok(Some(if opcode == 1 {
            Event::Text(String::from_utf8(bytes).map_err(|_| Error::Utf8)?)
        } else {
            Event::Binary(bytes)
        }))
    }
    #[cfg(test)]
    pub(super) fn test_client(
        io: S,
        limits: Limits,
        random: fn(&mut [u8]) -> Result<(), Error>,
    ) -> Self {
        Self::established(io, limits, None, random)
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    use std::io::Cursor;
    fn random(b: &mut [u8]) -> Result<(), Error> {
        b.fill(0);
        Ok(())
    }
    #[test]
    fn frame_rejection_precedes_reserve_and_payload_read() {
        let mut c = Client::established(
            Cursor::new(vec![0x82, 127, 0, 0, 0, 1, 0, 0, 0, 0]),
            Limits::default(),
            None,
            random,
        );
        assert!(matches!(c.receive(), Err(Error::Limit)));
        assert_eq!(c.message.capacity(), 0);
        assert_eq!(c.io.as_ref().unwrap().position(), 10);
    }
    #[test]
    fn message_rejection_preserves_existing_capacity_before_payload_read() {
        let mut c = Client::established(
            Cursor::new(vec![0x80, 3]),
            Limits {
                message_bytes: 5,
                ..Limits::default()
            },
            None,
            random,
        );
        c.message = vec![1, 2, 3];
        c.fragment = Some(2);
        let before = c.message.capacity();
        assert!(matches!(c.receive(), Err(Error::Limit)));
        assert_eq!(c.message.capacity(), before);
        assert_eq!(c.message, vec![1, 2, 3]);
        assert_eq!(c.io.as_ref().unwrap().position(), 2);
    }
    #[test]
    fn control_queue_rejection_precedes_payload_read_and_allocation() {
        let mut c = Client::established(
            Cursor::new(vec![0x89, 125]),
            Limits {
                queued_controls: 0,
                ..Limits::default()
            },
            None,
            random,
        );
        assert!(matches!(c.receive(), Err(Error::Limit)));
        assert_eq!(c.controls.capacity(), 0);
        assert_eq!(c.io.as_ref().unwrap().position(), 2);
    }
}
