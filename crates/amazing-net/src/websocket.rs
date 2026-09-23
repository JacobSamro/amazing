//! Bounded, blocking RFC6455 client. No extensions or HTTP/2 extended CONNECT.
//! Browser embeddings must supply the initiating Origin and enforce their own
//! CSP/mixed-content/credential policy. This module is transport, not a JS API.
mod client;
mod frame;
mod handshake;
mod utf8;
pub use client::Client;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Tls(String),
    Entropy,
    Handshake,
    InvalidInput,
    Protocol,
    Utf8,
    Limit,
    Closed,
    AbnormalClosure,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebSocket: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl Error {
    fn close_code(&self) -> u16 {
        match self {
            Self::Utf8 => 1007,
            Self::Limit => 1009,
            _ => 1002,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub frame_bytes: usize,
    pub message_bytes: usize,
    /// Received Ping/Pong notifications retained until pop_control(). Replies
    /// are written synchronously; there is no unbounded outbound control queue.
    pub queued_controls: usize,
    pub frames: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            frame_bytes: 1024 * 1024,
            message_bytes: 8 * 1024 * 1024,
            queued_controls: 32,
            frames: 100000,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<Self, Error> {
        if self.frame_bytes == 0
            || self.frame_bytes > 16 * 1024 * 1024
            || self.message_bytes > 32 * 1024 * 1024
            || self.queued_controls > 32
            || self.frames == 0
            || self.frames > 1000000
        {
            return Err(Error::Limit);
        }
        Ok(self)
    }
}
#[derive(Clone, Debug)]
pub struct Options {
    /// Required for a browser embedding; None is for non-browser callers.
    pub origin: Option<String>,
    pub protocols: Vec<String>,
    pub timeout: Duration,
    pub limits: Limits,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            origin: None,
            protocols: Vec::new(),
            timeout: Duration::from_secs(10),
            limits: Limits::default(),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Close {
    pub code: u16,
    pub reason: String,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Binary(Vec<u8>),
    Close(Option<Close>),
}
#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Closing,
    Closed,
    Failed,
}

/// Concrete transport exposes only first-party types.
pub enum Transport {
    Tcp(TcpStream),
    Tls(amazing_net_tls::TlsStream),
}
impl Read for Transport {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(b),
            Self::Tls(s) => s.read(b),
        }
    }
}
impl Write for Transport {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.write(b),
            Self::Tls(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}
fn random(bytes: &mut [u8]) -> Result<(), Error> {
    amazing_net_tls::fill_random(bytes).map_err(|_| Error::Entropy)
}
/// Connect ws/wss without redirects, cookies, proxying or extension negotiation.
/// wss offers only HTTP/1.1 ALPN, preserving RFC6455's Upgrade handshake.
pub fn connect(input: &str, options: &Options) -> Result<Client<Transport>, Error> {
    if input.len() > 8192 {
        return Err(Error::Limit);
    }
    let url = crate::ParsedUrl::parse(input).map_err(|_| Error::InvalidInput)?;
    if !matches!(url.scheme(), "ws" | "wss")
        || url.fragment().is_some()
        || !url.username().is_empty()
        || !url.password().is_empty()
    {
        return Err(Error::InvalidInput);
    }
    let host = url.hostname();
    if host.is_empty() {
        return Err(Error::InvalidInput);
    }
    let mut target = url.pathname();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    let authority = url.host_and_port();
    handshake::validate_request(&authority, &target, options)?;
    options.limits.validate()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.effective_port().ok_or(Error::InvalidInput)?;
    let io = if url.scheme() == "wss" {
        Transport::Tls(
            amazing_net_tls::connect_http1(host, port, options.timeout)
                .map_err(|e| Error::Tls(e.to_string()))?,
        )
    } else {
        let io = TcpStream::connect((host, port))?;
        io.set_read_timeout(Some(options.timeout))?;
        io.set_write_timeout(Some(options.timeout))?;
        Transport::Tcp(io)
    };
    Client::handshake(io, &authority, &target, options)
}
#[cfg(test)]
mod tests;
