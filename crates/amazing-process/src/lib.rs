//! Process hosting, bootstrap, liveness and site assignment.
#![forbid(unsafe_code)]
use amazing_ipc::codec::{self, Decoder, Encoder, Wire};
use amazing_ipc::version::{negotiate, VersionRange};
use amazing_ipc::{Connection, StdioTransport};
use amazing_ipc_platform::{FrameBudget, FrameMetadata, FrameView, HandleChannel};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const BOOTSTRAP_VERSION: u16 = 2;
const INTERFACES: &[VersionRange] = &[
    VersionRange {
        interface: 0,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 7,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 8,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 9,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 10,
        min: 2,
        max: 2,
    },
    VersionRange {
        interface: 11,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 17,
        min: 1,
        max: 1,
    },
    VersionRange {
        interface: 18,
        min: 1,
        max: 1,
    },
];
const FRAME_NONCE_CAPABILITY: &str = "__frame_nonce";
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
    pub interfaces: Vec<VersionRange>,
}
impl Wire for Handshake {
    fn encode(&self, e: &mut Encoder) -> Result<(), codec::Error> {
        self.version.encode(e)?;
        self.role.encode(e)?;
        self.capabilities.encode(e)?;
        self.interfaces.encode(e)
    }
    fn decode(d: &mut Decoder<'_>) -> Result<Self, codec::Error> {
        Ok(Self {
            version: u16::decode(d)?,
            role: String::decode(d)?,
            capabilities: Capabilities::decode(d)?,
            interfaces: Vec::<VersionRange>::decode(d)?,
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
    Superseded(u64),
    Busy,
    Platform(amazing_ipc_platform::Error),
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
impl From<amazing_ipc_platform::Error> for Error {
    fn from(e: amazing_ipc_platform::Error) -> Self {
        Self::Platform(e)
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
    let versions = negotiate(INTERFACES, &h.interfaces)?;
    if versions.get(&0) != Some(&1) {
        return Err(Error::Version);
    }
    Ok(())
}
/// Child entry point called after parsing --type=<role> from argv. The bind
/// callback installs every role handler before the bootstrap acknowledgment
/// makes this connection visible to the browser.
pub fn bootstrap_child(
    role: Role,
    mut capabilities: Capabilities,
    bind: impl FnOnce(&Arc<Connection>, &Capabilities, Option<HandleChannel>) -> Result<(), Error>,
) -> Result<Arc<Connection>, Error> {
    let frame_channel = if role == Role::Renderer && cfg!(any(target_os = "linux", windows)) {
        Some(HandleChannel::from_child_env()?)
    } else {
        None
    };
    let mut input = std::io::stdin();
    let mut output = std::io::stdout();
    let mut parent = recv_boot(&mut input)?;
    verify(&parent, Role::Browser)?;
    if let Some(channel) = &frame_channel {
        if parent.capabilities.0.get(FRAME_NONCE_CAPABILITY) != Some(&channel.nonce_hex()) {
            return Err(Error::Role);
        }
        capabilities
            .0
            .insert(FRAME_NONCE_CAPABILITY.into(), channel.nonce_hex());
        parent.capabilities.0.remove(FRAME_NONCE_CAPABILITY);
    }
    let sandbox_requested = std::env::var_os("AMAZING_RENDERER_SANDBOX").is_some();
    std::env::remove_var("AMAZING_RENDERER_SANDBOX");
    if sandbox_requested {
        if role != Role::Renderer {
            return Err(Error::Role);
        }
        #[cfg(target_os = "linux")]
        let frame_fd = frame_channel.as_ref().ok_or(Error::Role)?.sandbox_keep_fd();
        #[cfg(not(target_os = "linux"))]
        let frame_fd = -1;
        amazing_ipc_platform::enter_renderer_sandbox(frame_fd)?;
    }
    let versions = negotiate(INTERFACES, &parent.interfaces)?;
    let connection = Connection::with_versions(StdioTransport::new(input, output), versions);
    bind(&connection, &parent.capabilities, frame_channel)?;
    connection.bind(0, Ping);
    // The browser cannot issue a request until it reads this acknowledgment.
    // Every role handler, including Ping, is bound before that happens.
    send_boot(
        &mut std::io::stdout(),
        &Handshake {
            version: BOOTSTRAP_VERSION,
            role: role.name().into(),
            capabilities,
            interfaces: INTERFACES.to_vec(),
        },
    )?;
    Ok(connection)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnCrash { max_restarts: u32 },
}
enum HostedChild {
    Standard(Child),
    #[cfg(windows)]
    Sandboxed(amazing_ipc_platform::WindowsSandboxChild),
}
impl HostedChild {
    fn id(&self) -> u32 {
        match self {
            Self::Standard(child) => child.id(),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.id(),
        }
    }
    fn stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        match self {
            Self::Standard(child) => child.stdin.take().map(|pipe| Box::new(pipe) as _),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.take_stdin().map(|pipe| Box::new(pipe) as _),
        }
    }
    fn stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        match self {
            Self::Standard(child) => child.stdout.take().map(|pipe| Box::new(pipe) as _),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.take_stdout().map(|pipe| Box::new(pipe) as _),
        }
    }
    fn attach_frame_channel(&self, channel: &mut HandleChannel) -> Result<(), Error> {
        match self {
            Self::Standard(child) => channel.attach_child(child)?,
            #[cfg(windows)]
            Self::Sandboxed(child) => channel.attach_sandbox_child(child)?,
        }
        Ok(())
    }
    fn try_wait(&mut self) -> std::io::Result<bool> {
        match self {
            Self::Standard(child) => child.try_wait().map(|status| status.is_some()),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.try_wait(),
        }
    }
    fn kill(&mut self) -> std::io::Result<()> {
        match self {
            Self::Standard(child) => child.kill(),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.kill(),
        }
    }
    fn wait(&mut self) -> std::io::Result<()> {
        match self {
            Self::Standard(child) => child.wait().map(|_| ()),
            #[cfg(windows)]
            Self::Sandboxed(child) => child.wait(),
        }
    }
}
/// Owns the child and the IPC connection; polling records crash and heartbeat state.
pub struct ProcessHost {
    child: HostedChild,
    pub connection: Arc<Connection>,
    pub role: Role,
    pub capabilities: Capabilities,
    launched: Instant,
    last_pong: Instant,
    restarts: u32,
    policy: RestartPolicy,
    sandboxed_renderer: bool,
    frame_channel: Mutex<Option<HandleChannel>>,
    frame_budget: Arc<Mutex<FrameBudget>>,
}
/// A browser mapping charged to the renderer until the browser drops its view.
pub struct BrowserFrame {
    view: FrameView,
    budget: Arc<Mutex<FrameBudget>>,
    generation: u64,
}
impl BrowserFrame {
    pub fn bytes(&self) -> &[u8] {
        self.view.bytes()
    }
    pub fn metadata(&self) -> FrameMetadata {
        self.view.metadata()
    }
}
impl Drop for BrowserFrame {
    fn drop(&mut self) {
        self.budget.lock().unwrap().release(self.generation);
    }
}
impl ProcessHost {
    pub fn spawn(
        exe: &std::path::Path,
        role: Role,
        capabilities: Capabilities,
        policy: RestartPolicy,
    ) -> Result<Self, Error> {
        Self::spawn_inner(exe, role, capabilities, policy, false)
    }
    /// A renderer launch that fails closed if the Linux profile cannot be
    /// installed. No ordinary-process retry is performed.
    pub fn spawn_sandboxed_renderer(
        exe: &std::path::Path,
        capabilities: Capabilities,
        policy: RestartPolicy,
    ) -> Result<Self, Error> {
        Self::spawn_inner(exe, Role::Renderer, capabilities, policy, true)
    }
    fn spawn_inner(
        exe: &std::path::Path,
        role: Role,
        mut capabilities: Capabilities,
        policy: RestartPolicy,
        sandboxed_renderer: bool,
    ) -> Result<Self, Error> {
        let mut command = Command::new(exe);
        command
            .arg(format!("--type={}", role.name()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.env_remove("AMAZING_RENDERER_SANDBOX");
        if sandboxed_renderer {
            command.env("AMAZING_RENDERER_SANDBOX", "1");
        }
        let mut frame_channel = if role == Role::Renderer && cfg!(any(target_os = "linux", windows))
        {
            Some(HandleChannel::prepare_child(&mut command)?)
        } else {
            None
        };
        if let Some(channel) = &frame_channel {
            capabilities
                .0
                .insert(FRAME_NONCE_CAPABILITY.into(), channel.nonce_hex());
        }
        #[cfg(windows)]
        let mut child = if sandboxed_renderer {
            let nonce = frame_channel.as_ref().ok_or(Error::Role)?.nonce_hex();
            HostedChild::Sandboxed(amazing_ipc_platform::spawn_renderer_appcontainer(
                exe, &nonce,
            )?)
        } else {
            HostedChild::Standard(command.spawn()?)
        };
        #[cfg(not(windows))]
        let mut child = HostedChild::Standard(command.spawn()?);
        let boot = (|| {
            if let Some(channel) = &mut frame_channel {
                child.attach_frame_channel(channel)?;
            }
            let mut stdin = child.stdin().ok_or(Error::Dead)?;
            let mut stdout = child.stdout().ok_or(Error::Dead)?;
            send_boot(
                &mut stdin,
                &Handshake {
                    version: BOOTSTRAP_VERSION,
                    role: Role::Browser.name().into(),
                    capabilities,
                    interfaces: INTERFACES.to_vec(),
                },
            )?;
            let mut response = recv_boot(&mut stdout)?;
            verify(&response, role)?;
            if let Some(channel) = &frame_channel {
                if response.capabilities.0.get(FRAME_NONCE_CAPABILITY) != Some(&channel.nonce_hex())
                {
                    return Err(Error::Role);
                }
                response.capabilities.0.remove(FRAME_NONCE_CAPABILITY);
            }
            let versions = negotiate(INTERFACES, &response.interfaces)?;
            Ok((stdin, stdout, response, versions))
        })();
        let (stdin, stdout, response, versions) = match boot {
            Ok(boot) => boot,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let conn = Connection::with_versions(StdioTransport::new(stdout, stdin), versions);
        let mut host = Self {
            child,
            connection: conn,
            role,
            capabilities: response.capabilities,
            launched: Instant::now(),
            last_pong: Instant::now(),
            restarts: 0,
            policy,
            sandboxed_renderer,
            frame_channel: Mutex::new(frame_channel),
            frame_budget: Arc::new(Mutex::new(FrameBudget::new())),
        };
        host.wait_ready(Duration::from_secs(5))?;
        Ok(host)
    }
    fn wait_ready(&mut self, timeout: Duration) -> Result<(), Error> {
        match self.ping(timeout) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.kill();
                Err(error)
            }
        }
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    /// Consume a shared frame only from this host's launched renderer.
    pub fn receive_frame(&self, ready: &protocol::SharedFrameReady) -> Result<BrowserFrame, Error> {
        if self.role != Role::Renderer {
            return Err(Error::Role);
        }
        let metadata = ready.metadata()?;
        let mut channel_guard = self.frame_channel.lock().unwrap();
        let channel = channel_guard.as_ref().ok_or(Error::Role)?;
        let mut budget = self.frame_budget.lock().unwrap();
        budget.reserve(metadata.generation, metadata.byte_len as usize)?;
        let proxy = protocol::RendererProxy::new(self.connection.clone(), Duration::from_secs(5));
        match proxy.acquire_frame(protocol::AcquireFrame {
            generation: metadata.generation,
        }) {
            Ok(protocol::AcquireResult::Transferred(())) => {}
            Ok(protocol::AcquireResult::Superseded(latest)) => {
                budget.release(metadata.generation);
                return Err(Error::Superseded(latest));
            }
            Ok(protocol::AcquireResult::Busy(())) => {
                budget.release(metadata.generation);
                return Err(Error::Busy);
            }
            Err(error) => {
                budget.release(metadata.generation);
                channel_guard.take();
                self.connection.close();
                return Err(error.into());
            }
        }
        let view = match channel.receive(metadata, self.child.id()) {
            Ok(view) => view,
            Err(error) => {
                budget.release(metadata.generation);
                channel_guard.take();
                self.connection.close();
                return Err(error.into());
            }
        };
        let acknowledged = proxy.frame_received(protocol::AcquireFrame {
            generation: metadata.generation,
        });
        if !matches!(acknowledged, Ok(true)) {
            budget.release(metadata.generation);
            channel_guard.take();
            self.connection.close();
            return Err(Error::Dead);
        }
        budget.protect(metadata.generation);
        drop(budget);
        Ok(BrowserFrame {
            view,
            budget: self.frame_budget.clone(),
            generation: metadata.generation,
        })
    }
    pub fn crashed(&mut self) -> Result<bool, Error> {
        Ok(self.child.try_wait()? || self.connection.is_closed())
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
        let next = Self::spawn_inner(
            exe,
            self.role,
            capabilities,
            self.policy,
            self.sandboxed_renderer,
        )?;
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

pub mod gpu;
pub mod gpu_snapshot_wire;
pub mod network;
pub mod network_v2;
pub mod protocol;
pub mod renderer;

/// Browser-side liveness observer. A crash is emitted once per renderer host.
pub struct RendererMonitor {
    reported: bool,
}
impl RendererMonitor {
    pub fn new() -> Self {
        Self { reported: false }
    }
    pub fn poll(&mut self, host: &mut ProcessHost) -> Result<Option<protocol::SadTab>, Error> {
        if self.reported || !host.crashed()? {
            return Ok(None);
        }
        self.reported = true;
        Ok(Some(protocol::SadTab {
            pid: host.pid(),
            reason: "renderer process exited".into(),
        }))
    }
}
impl Default for RendererMonitor {
    fn default() -> Self {
        Self::new()
    }
}
