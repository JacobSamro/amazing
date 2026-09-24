//! Typed, bounded IPC over ordered in-process or child stdio transports.
#![forbid(unsafe_code)]
pub mod codec;
pub mod version;
use codec::{Decoder, Encoder, Wire};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Codec(codec::Error),
    Io(String),
    Disconnected,
    Cancelled,
    Remote(String),
    UnknownInterface,
    UnknownMethod,
    Timeout,
}
impl From<codec::Error> for Error {
    fn from(e: codec::Error) -> Self {
        Self::Codec(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
/// One ordered, bidirectional, closable message pipe.
pub trait Transport: Send + Sync + 'static {
    fn send(&self, frame: &[u8]) -> Result<(), Error>;
    fn recv(&self) -> Result<Vec<u8>, Error>;
    fn close(&self);
}
pub struct MemoryTransport {
    tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
}
pub fn memory_pair() -> (Arc<MemoryTransport>, Arc<MemoryTransport>) {
    let (a, b) = mpsc::channel();
    let (c, d) = mpsc::channel();
    (
        Arc::new(MemoryTransport {
            tx: Mutex::new(Some(a)),
            rx: Mutex::new(d),
        }),
        Arc::new(MemoryTransport {
            tx: Mutex::new(Some(c)),
            rx: Mutex::new(b),
        }),
    )
}
impl Transport for MemoryTransport {
    fn send(&self, frame: &[u8]) -> Result<(), Error> {
        if frame.len() > codec::MAX_FRAME {
            return Err(Error::Codec(codec::Error::Limit));
        }
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .ok_or(Error::Disconnected)?
            .send(frame.to_vec())
            .map_err(|_| Error::Disconnected)
    }
    fn recv(&self) -> Result<Vec<u8>, Error> {
        self.rx
            .lock()
            .unwrap()
            .recv()
            .map_err(|_| Error::Disconnected)
    }
    fn close(&self) {
        self.tx.lock().unwrap().take();
    }
}
/// Length-prefixed stdio framing. Reads and writes use separate locks.
pub struct StdioTransport {
    reader: Mutex<Box<dyn Read + Send>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
}
impl StdioTransport {
    pub fn new(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            reader: Mutex::new(Box::new(reader)),
            writer: Mutex::new(Some(Box::new(writer))),
        })
    }
}
impl Transport for StdioTransport {
    fn send(&self, frame: &[u8]) -> Result<(), Error> {
        if frame.len() > codec::MAX_FRAME {
            return Err(Error::Codec(codec::Error::Limit));
        }
        let mut w = self.writer.lock().unwrap();
        let w = w.as_mut().ok_or(Error::Disconnected)?;
        w.write_all(&(frame.len() as u32).to_le_bytes())
            .and_then(|_| w.write_all(frame))
            .and_then(|_| w.flush())
            .map_err(|e| Error::Io(e.to_string()))
    }
    fn recv(&self) -> Result<Vec<u8>, Error> {
        let mut r = self.reader.lock().unwrap();
        let mut n = [0; 4];
        r.read_exact(&mut n).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::Disconnected
            } else {
                Error::Io(e.to_string())
            }
        })?;
        let n = u32::from_le_bytes(n) as usize;
        if n > codec::MAX_FRAME {
            return Err(Error::Codec(codec::Error::Limit));
        }
        let mut frame = vec![0; n];
        r.read_exact(&mut frame)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(frame)
    }
    fn close(&self) {
        self.writer.lock().unwrap().take();
    }
}
#[derive(Debug, Clone)]
pub struct Packet {
    pub interface: u32,
    pub id: u64,
    pub kind: u8,
    pub payload: Vec<u8>,
}
impl Wire for Packet {
    fn encode(&self, e: &mut Encoder) -> Result<(), codec::Error> {
        self.interface.encode(e)?;
        self.id.encode(e)?;
        self.kind.encode(e)?;
        if self.payload.len() > codec::MAX_FRAME - 8 {
            return Err(codec::Error::Limit);
        }
        (self.payload.len() as u32).encode(e)?;
        e.bytes(&self.payload)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, codec::Error> {
        Ok(Self {
            interface: u32::decode(d)?,
            id: u64::decode(d)?,
            kind: u8::decode(d)?,
            payload: codec::ByteBuf::decode(d)?.0,
        })
    }
}
pub const REQUEST: u8 = 1;
pub const RESPONSE: u8 = 2;
pub const CANCEL: u8 = 3;
pub const REMOTE_ERROR: u8 = 4;
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(&self, method: u16, payload: &[u8]) -> Result<Vec<u8>, Error>;
}
pub struct Connection {
    transport: Arc<dyn Transport>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, mpsc::Sender<Result<Vec<u8>, Error>>>>,
    handlers: Mutex<HashMap<u32, Arc<dyn Dispatcher>>>,
    versions: Mutex<Option<std::collections::BTreeMap<u32, u16>>>,
    closed: AtomicBool,
    cancelled: Mutex<std::collections::HashSet<u64>>,
}
impl Connection {
    pub fn new(transport: Arc<dyn Transport>) -> Arc<Self> {
        Self::with_optional_versions(transport, None)
    }
    pub fn with_versions(
        transport: Arc<dyn Transport>,
        versions: std::collections::BTreeMap<u32, u16>,
    ) -> Arc<Self> {
        Self::with_optional_versions(transport, Some(versions))
    }
    fn with_optional_versions(
        transport: Arc<dyn Transport>,
        versions: Option<std::collections::BTreeMap<u32, u16>>,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            transport,
            next: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            handlers: Mutex::new(HashMap::new()),
            versions: Mutex::new(versions),
            closed: AtomicBool::new(false),
            cancelled: Mutex::new(Default::default()),
        });
        let worker = this.clone();
        thread::spawn(move || worker.run());
        this
    }
    pub fn bind(&self, interface: u32, handler: impl Dispatcher) {
        self.handlers
            .lock()
            .unwrap()
            .insert(interface, Arc::new(handler));
    }
    pub fn interface_version(&self, interface: u32) -> Option<u16> {
        self.versions.lock().unwrap().as_ref()?.get(&interface).copied()
    }
    fn allows(&self, interface: u32) -> bool {
        self.versions.lock().unwrap().as_ref().is_none_or(|v| v.contains_key(&interface))
    }
    fn send(&self, p: Packet) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::Disconnected);
        }
        self.transport.send(&codec::encode(&p)?)
    }
    pub fn call(
        &self,
        interface: u32,
        method: u16,
        payload: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, Error> {
        if !self.allows(interface) {
            return Err(Error::UnknownInterface);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut e = Encoder::raw();
        method.encode(&mut e)?;
        e.bytes(&payload)?;
        if let Err(err) = self.send(Packet {
            interface,
            id,
            kind: REQUEST,
            payload: e.finish(),
        }) {
            self.pending.lock().unwrap().remove(&id);
            return Err(err);
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                let _ = self.send(Packet {
                    interface,
                    id,
                    kind: CANCEL,
                    payload: vec![],
                });
                Err(Error::Timeout)
            }
            Err(_) => Err(Error::Disconnected),
        }
    }
    pub fn cancel(&self, id: u64, interface: u32) -> Result<(), Error> {
        self.send(Packet {
            interface,
            id,
            kind: CANCEL,
            payload: vec![],
        })
    }
    pub fn is_cancelled(&self, id: u64) -> bool {
        self.cancelled.lock().unwrap().contains(&id)
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.transport.close();
        self.pending.lock().unwrap().clear();
    }
    fn run(self: Arc<Self>) {
        while let Ok(frame) = self.transport.recv() {
            let Ok(p) = codec::decode::<Packet>(&frame) else {
                break;
            };
            match p.kind {
                RESPONSE | REMOTE_ERROR => {
                    if let Some(tx) = self.pending.lock().unwrap().remove(&p.id) {
                        let r = if p.kind == RESPONSE {
                            Ok(p.payload)
                        } else {
                            Err(Error::Remote(
                                String::from_utf8_lossy(&p.payload).into_owned(),
                            ))
                        };
                        let _ = tx.send(r);
                    }
                }
                CANCEL => {
                    self.cancelled.lock().unwrap().insert(p.id);
                }
                REQUEST => {
                    let handler = self.handlers.lock().unwrap().get(&p.interface).cloned();
                    let conn = self.clone();
                    thread::spawn(move || {
                        let result = (|| {
                            let mut d = Decoder::raw(&p.payload);
                            let method = u16::decode(&mut d)?;
                            let body = d.take(d.remaining())?;
                            if !conn.allows(p.interface) {
                                return Err(Error::UnknownInterface);
                            }
                            handler
                                .ok_or(Error::UnknownInterface)?
                                .dispatch(method, body)
                        })();
                        if conn.is_cancelled(p.id) {
                            conn.cancelled.lock().unwrap().remove(&p.id);
                            return;
                        }
                        let (kind, payload) = match result {
                            Ok(v) => (RESPONSE, v),
                            Err(e) => (REMOTE_ERROR, e.to_string().into_bytes()),
                        };
                        let _ = conn.send(Packet {
                            interface: p.interface,
                            id: p.id,
                            kind,
                            payload,
                        });
                    });
                }
                _ => break,
            }
        }
        self.closed.store(true, Ordering::Release);
        self.pending.lock().unwrap().clear();
        self.transport.close();
    }
}
/// Declares a single-argument request/response interface, proxy and dispatcher.
#[macro_export]
macro_rules! interface {($vis:vis interface $name:ident ($iid:literal) => $proxy:ident, $receiver:ident {$(fn $method:ident ($arg:ident : $req:ty) -> $res:ty = $mid:literal;)*})=>{
 $vis trait $name:Send+Sync+'static {$(fn $method(&self,$arg:$req)->Result<$res,$crate::Error>;)*}
 $vis struct $proxy {connection:std::sync::Arc<$crate::Connection>,timeout:std::time::Duration}
 impl $proxy {pub fn new(connection:std::sync::Arc<$crate::Connection>,timeout:std::time::Duration)->Self{Self{connection,timeout}} $(pub fn $method(&self,$arg:$req)->Result<$res,$crate::Error>{let body=$crate::codec::encode(&$arg)?;let answer=self.connection.call($iid,$mid,body,self.timeout)?;Ok($crate::codec::decode(&answer)?)})*
 /// Decode a request without executing its handler; used by bounded fuzz drivers.
 pub fn validate_request(method:u16,payload:&[u8])->Result<(),$crate::Error>{match method {$($mid=>{let _:$req=$crate::codec::decode(payload)?;Ok(())}),*,_=>Err($crate::Error::UnknownMethod)}}
 }
 $vis struct $receiver<T:$name>(pub T);
 impl<T:$name> $crate::Dispatcher for $receiver<T>{fn dispatch(&self,method:u16,payload:&[u8])->Result<Vec<u8>,$crate::Error>{match method {$($mid=>{let arg:$req=$crate::codec::decode(payload)?;let result=self.0.$method(arg)?;Ok($crate::codec::encode(&result)?)}),* ,_=>Err($crate::Error::UnknownMethod)}}}
};}
