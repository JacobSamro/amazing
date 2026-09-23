//! Process hosting, bootstrap, liveness and site assignment.
#![forbid(unsafe_code)]
use amazing_ipc::codec::{self, Decoder, Encoder, Wire};
use amazing_ipc::{Connection, StdioTransport};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const BOOTSTRAP_VERSION: u16 = 1;
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Role {
    Browser,
    Renderer,
    Network,
    Gpu,
    Utility,
}
impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Renderer => "renderer",
            Self::Network => "network",
            Self::Gpu => "gpu",
            Self::Utility => "utility",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "browser" => Some(Self::Browser),
            "renderer" => Some(Self::Renderer),
            "network" => Some(Self::Network),
            "gpu" => Some(Self::Gpu),
            "utility" => Some(Self::Utility),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities(pub BTreeMap<String, String>);
impl Wire for Capabilities {
    fn encode(&self, e: &mut Encoder) -> Result<(), codec::Error> {
        self.0.encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, codec::Error> {
        Ok(Self(BTreeMap::decode(d)?))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub version: u16,
    pub role: String,
    pub capabilities: Capabilities,
}
impl Wire for Handshake {
    fn encode(&self, e: &mut Encoder) -> Result<(), codec::Error> {
        self.version.encode(e)?;
        self.role.encode(e)?;
        self.capabilities.encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, codec::Error> {
        Ok(Self {
            version: u16::decode(d)?,
            role: String::decode(d)?,
            capabilities: Capabilities::decode(d)?,
        })
    }
}
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Ipc(amazing_ipc::Error),
    Codec(codec::Error),
    Version,
    Role,
    Dead,
    Limit,
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<amazing_ipc::Error> for Error {
    fn from(e: amazing_ipc::Error) -> Self {
        Self::Ipc(e)
    }
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
fn send_boot(w: &mut impl Write, h: &Handshake) -> Result<(), Error> {
    let b = codec::encode(h)?;
    w.write_all(&(b.len() as u32).to_le_bytes())?;
    w.write_all(&b)?;
    w.flush()?;
    Ok(())
}
fn recv_boot(r: &mut impl Read) -> Result<Handshake, Error> {
    let mut n = [0; 4];
    r.read_exact(&mut n)?;
    let n = u32::from_le_bytes(n) as usize;
    if n > codec::MAX_FRAME {
        return Err(Error::Limit);
    }
    let mut b = vec![0; n];
    r.read_exact(&mut b)?;
    Ok(codec::decode(&b)?)
}
fn verify(h: &Handshake, role: Role) -> Result<(), Error> {
    if h.version != BOOTSTRAP_VERSION {
        return Err(Error::Version);
    }
    if Role::parse(&h.role) != Some(role) {
        return Err(Error::Role);
    }
    Ok(())
}
/// Child entry point called after parsing --type=<role> from argv.
pub fn bootstrap_child(
    role: Role,
    capabilities: Capabilities,
) -> Result<(Arc<Connection>, Capabilities), Error> {
    let mut input = std::io::stdin();
    let mut output = std::io::stdout();
    let parent = recv_boot(&mut input)?;
    verify(&parent, Role::Browser)?;
    send_boot(
        &mut output,
        &Handshake {
            version: BOOTSTRAP_VERSION,
            role: role.name().into(),
            capabilities,
        },
    )?;
    let connection = Connection::new(StdioTransport::new(input, output));
    Ok((connection, parent.capabilities))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnCrash { max_restarts: u32 },
}
/// Owns the child and the IPC connection; polling records crash and heartbeat state.
pub struct ProcessHost {
    child: Child,
    pub connection: Arc<Connection>,
    pub role: Role,
    pub capabilities: Capabilities,
    launched: Instant,
    last_pong: Instant,
    restarts: u32,
    policy: RestartPolicy,
}
impl ProcessHost {
    pub fn spawn(
        exe: &std::path::Path,
        role: Role,
        capabilities: Capabilities,
        policy: RestartPolicy,
    ) -> Result<Self, Error> {
        let mut child = Command::new(exe)
            .arg(format!("--type={}", role.name()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or(Error::Dead)?;
        let mut stdout = child.stdout.take().ok_or(Error::Dead)?;
        send_boot(
            &mut stdin,
            &Handshake {
                version: BOOTSTRAP_VERSION,
                role: Role::Browser.name().into(),
                capabilities,
            },
        )?;
        let response = recv_boot(&mut stdout)?;
        verify(&response, role)?;
        let conn = Connection::new(StdioTransport::new(stdout, stdin));
        Ok(Self {
            child,
            connection: conn,
            role,
            capabilities: response.capabilities,
            launched: Instant::now(),
            last_pong: Instant::now(),
            restarts: 0,
            policy,
        })
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    pub fn crashed(&mut self) -> Result<bool, Error> {
        Ok(self.child.try_wait()?.is_some() || self.connection.is_closed())
    }
    pub fn ping(&mut self, timeout: Duration) -> Result<(), Error> {
        let v = self.connection.call(0, 0, vec![], timeout)?;
        if !v.is_empty() {
            return Err(Error::Dead);
        }
        self.last_pong = Instant::now();
        Ok(())
    }
    pub fn heartbeat_age(&self) -> Duration {
        self.last_pong.elapsed()
    }
    pub fn uptime(&self) -> Duration {
        self.launched.elapsed()
    }
    pub fn restart_if_crashed(
        &mut self,
        exe: &std::path::Path,
        capabilities: Capabilities,
    ) -> Result<bool, Error> {
        if !self.crashed()? {
            return Ok(false);
        }
        let RestartPolicy::OnCrash { max_restarts } = self.policy else {
            return Ok(false);
        };
        if self.restarts >= max_restarts {
            return Ok(false);
        };
        let next = Self::spawn(exe, self.role, capabilities, self.policy)?;
        let count = self.restarts + 1;
        *self = next;
        self.restarts = count;
        Ok(true)
    }
    pub fn kill(&mut self) -> Result<(), Error> {
        self.child.kill()?;
        let _ = self.child.wait();
        self.connection.close();
        Ok(())
    }
}
/// A child binds this responder for host liveness checks.
pub struct Ping;
impl amazing_ipc::Dispatcher for Ping {
    fn dispatch(&self, method: u16, payload: &[u8]) -> Result<Vec<u8>, amazing_ipc::Error> {
        if method == 0 && payload.is_empty() {
            Ok(vec![])
        } else {
            Err(amazing_ipc::Error::Remote("bad ping".into()))
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SiteInstance {
    pub scheme: String,
    pub domain: String,
}
impl SiteInstance {
    pub fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            return None;
        }
        let host = rest
            .split(&['/', ':', '?', '#'][..])
            .next()?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host.is_empty()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        {
            return None;
        }
        let domain = match amazing_fetch::psl::PublicSuffixList.classify(&host) {
            amazing_fetch::psl::DomainBoundary::Registrable(name) => name.to_owned(),
            amazing_fetch::psl::DomainBoundary::Ip
            | amazing_fetch::psl::DomainBoundary::PublicSuffix => host,
            _ => return None,
        };
        Some(Self {
            scheme: scheme.into(),
            domain,
        })
    }
}
/// Assigns each site to one renderer until the configured process limit is reached.
pub struct RendererAssignments {
    limit: usize,
    by_site: HashMap<SiteInstance, u64>,
    loads: HashMap<u64, usize>,
    next: u64,
}
impl RendererAssignments {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            by_site: HashMap::new(),
            loads: HashMap::new(),
            next: 1,
        }
    }
    pub fn assign(&mut self, site: SiteInstance) -> u64 {
        if let Some(&id) = self.by_site.get(&site) {
            return id;
        }
        let id = if self.loads.len() < self.limit {
            let id = self.next;
            self.next += 1;
            id
        } else {
            *self
                .loads
                .iter()
                .min_by_key(|(id, load)| (**load, **id))
                .unwrap()
                .0
        };
        *self.loads.entry(id).or_default() += 1;
        self.by_site.insert(site, id);
        id
    }
    pub fn remove_process(&mut self, id: u64) {
        self.loads.remove(&id);
        self.by_site.retain(|_, v| *v != id)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub role: Role,
    pub file_roots: Vec<std::path::PathBuf>,
    pub allow_network: bool,
    pub allow_gpu: bool,
}
