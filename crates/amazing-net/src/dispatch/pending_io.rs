//! Process-wide bounded native DNS/connect work. Cancellation suppresses delivery; std OS
//! operations already running are not interrupted and retain their allocation.
use std::{
    collections::VecDeque,
    io,
    net::{SocketAddr, TcpStream},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::Instant,
};
const WORKERS_PER_KIND: usize = 4;
const MAX_OWNED: usize = 64;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Resolve,
    Connect,
}
impl Kind {
    fn index(self) -> usize {
        match self {
            Self::Resolve => 0,
            Self::Connect => 1,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Running,
    Ready,
    CancelledRunning,
    Cancelled,
    Consumed,
}
pub(crate) enum Value {
    Addresses(Vec<SocketAddr>),
    Socket(TcpStream),
}
struct Completion {
    phase: Phase,
    value: Option<io::Result<Value>>,
}
struct Core {
    key: IoKey,
    kind: Kind,
    completion: Mutex<Completion>,
    pool: Weak<Pool>,
}
struct Job {
    core: Arc<Core>,
    work: Box<dyn FnOnce() -> io::Result<Value> + Send>,
}
struct Data {
    queue: VecDeque<Job>,
    owned: usize,
    running: [usize; 2],
    cancelled_running: [usize; 2],
    ready: usize,
    late_sockets_closed: u64,
    closed: bool,
    next_owner: u64,
}
struct Pool {
    data: Mutex<Data>,
    changed: Condvar,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct IoKey {
    pub owner: u64,
    pub request: u64,
    pub generation: u64,
    pub websocket: bool,
}
/// Process-wide physical work facts across all pending-I/O dispatchers.
/// Running means the provider callback is executing; it does not prove a specific
/// syscall is active. These counters do not claim OS interruption or memory totals.
#[derive(Clone, Copy, Debug)]
pub struct PendingIoSnapshot {
    pub owned_operations: usize,
    pub queued_resolution: usize,
    pub queued_connect: usize,
    pub running_resolution: usize,
    pub running_connect: usize,
    pub cancelled_running_resolution: usize,
    pub cancelled_running_connect: usize,
    pub ready_unread: usize,
    pub late_sockets_closed: u64,
}
/// A native provider receives this context from an actual dispatcher operation.
/// It contains no Window, runtime, policy, cookie or Reporting roots.
pub struct PendingIoContext {
    pool: Arc<Pool>,
    key: IoKey,
    deadline: Instant,
}
/// Move-only operation, privately constructed by a bounded native submission.
/// Dropping it cancels delivery and closes any unread ready stream.
pub struct PendingIoOperation {
    core: Arc<Core>,
}
#[derive(Clone)]
pub(crate) struct PendingIoMode {
    pool: Arc<Pool>,
    owner: u64,
}
impl PendingIoMode {
    pub(crate) fn new() -> io::Result<Self> {
        static SHARED: OnceLock<Result<Arc<Pool>, String>> = OnceLock::new();
        let pool = SHARED
            .get_or_init(Pool::start)
            .as_ref()
            .map_err(|s| io::Error::other(s.clone()))?
            .clone();
        let mut data = pool.data.lock().unwrap();
        let owner = data.next_owner;
        data.next_owner = owner
            .checked_add(1)
            .ok_or_else(|| io::Error::other("pending IO owner identity exhausted"))?;
        drop(data);
        Ok(Self { pool, owner })
    }
    pub(crate) fn context(
        &self,
        request: u64,
        generation: u64,
        websocket: bool,
        deadline: Instant,
    ) -> PendingIoContext {
        PendingIoContext {
            pool: self.pool.clone(),
            key: IoKey {
                owner: self.owner,
                request,
                generation,
                websocket,
            },
            deadline,
        }
    }
    pub(crate) fn snapshot(&self) -> PendingIoSnapshot {
        self.pool.snapshot()
    }
}
impl Pool {
    fn start() -> Result<Arc<Self>, String> {
        let pool = Arc::new(Self {
            data: Mutex::new(Data {
                queue: VecDeque::new(),
                owned: 0,
                running: [0; 2],
                cancelled_running: [0; 2],
                ready: 0,
                late_sockets_closed: 0,
                closed: false,
                next_owner: 1,
            }),
            changed: Condvar::new(),
        });
        for kind in [Kind::Resolve, Kind::Connect] {
            for index in 0..WORKERS_PER_KIND {
                let worker = pool.clone();
                if let Err(e) = std::thread::Builder::new()
                    .name(format!("amazing-io-{}-{index}", kind.index()))
                    .spawn(move || worker.worker(kind))
                {
                    pool.data.lock().unwrap().closed = true;
                    pool.changed.notify_all();
                    return Err(e.to_string());
                }
            }
        }
        Ok(pool)
    }
    fn worker(self: Arc<Self>, kind: Kind) {
        loop {
            let job = {
                let mut data = self.data.lock().unwrap();
                loop {
                    if data.closed {
                        return;
                    }
                    if let Some(index) = data.queue.iter().position(|job| job.core.kind == kind) {
                        let job = data.queue.remove(index).unwrap();
                        job.core.completion.lock().unwrap().phase = Phase::Running;
                        data.running[kind.index()] += 1;
                        break job;
                    }
                    data = self.changed.wait(data).unwrap();
                }
            };
            // All OS/provider work happens outside the actor and the pool lock.
            let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.work))
                .unwrap_or_else(|_| Err(io::Error::other("native IO provider panicked")));
            let mut data = self.data.lock().unwrap();
            let mut completion = job.core.completion.lock().unwrap();
            data.running[kind.index()] -= 1;
            let mut discarded = None;
            let mut closed_socket = false;
            if completion.phase == Phase::CancelledRunning {
                data.cancelled_running[kind.index()] -= 1;
                if matches!(&value, Ok(Value::Socket(_))) {
                    closed_socket = true;
                }
                discarded = Some(value);
                completion.phase = Phase::Cancelled;
            } else {
                completion.value = Some(value);
                completion.phase = Phase::Ready;
                data.ready += 1;
            }
            drop(completion);
            drop(data);
            // Socket disposal can enter the OS; keep the physical core owned
            // until disposal returns, without holding the shared pool lock.
            drop(discarded);
            if closed_socket {
                self.data.lock().unwrap().late_sockets_closed += 1;
            }
            drop(job.core);
        }
    }
    fn submit(
        self: &Arc<Self>,
        key: IoKey,
        kind: Kind,
        work: Box<dyn FnOnce() -> io::Result<Value> + Send>,
    ) -> io::Result<PendingIoOperation> {
        let mut data = self.data.lock().unwrap();
        if data.closed {
            return Err(io::Error::other("pending IO pool closed"));
        }
        if data.owned == MAX_OWNED {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let core = Arc::new(Core {
            key,
            kind,
            completion: Mutex::new(Completion {
                phase: Phase::Queued,
                value: None,
            }),
            pool: Arc::downgrade(self),
        });
        data.owned += 1;
        data.queue.push_back(Job {
            core: core.clone(),
            work,
        });
        drop(data);
        self.changed.notify_all();
        Ok(PendingIoOperation { core })
    }
    fn snapshot(&self) -> PendingIoSnapshot {
        let d = self.data.lock().unwrap();
        PendingIoSnapshot {
            owned_operations: d.owned,
            queued_resolution: d
                .queue
                .iter()
                .filter(|j| j.core.kind == Kind::Resolve)
                .count(),
            queued_connect: d
                .queue
                .iter()
                .filter(|j| j.core.kind == Kind::Connect)
                .count(),
            running_resolution: d.running[0],
            running_connect: d.running[1],
            cancelled_running_resolution: d.cancelled_running[0],
            cancelled_running_connect: d.cancelled_running[1],
            ready_unread: d.ready,
            late_sockets_closed: d.late_sockets_closed,
        }
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            pool.data.lock().unwrap().owned -= 1;
        }
    }
}
impl PendingIoContext {
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn resolve<F>(&self, work: F) -> io::Result<PendingIoOperation>
    where
        F: FnOnce() -> io::Result<Vec<SocketAddr>> + Send + 'static,
    {
        let deadline = self.deadline;
        self.pool.submit(
            self.key,
            Kind::Resolve,
            Box::new(move || {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                work().map(Value::Addresses)
            }),
        )
    }
    pub fn connect<F>(&self, work: F) -> io::Result<PendingIoOperation>
    where
        F: FnOnce() -> io::Result<TcpStream> + Send + 'static,
    {
        let deadline = self.deadline;
        self.pool.submit(
            self.key,
            Kind::Connect,
            Box::new(move || {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                work().map(Value::Socket)
            }),
        )
    }
}
impl PendingIoOperation {
    pub(crate) fn poll(&mut self, context: &PendingIoContext) -> Option<io::Result<Value>> {
        if self.core.key != context.key {
            return Some(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "foreign or stale pending IO operation",
            )));
        }
        let pool = self.core.pool.upgrade()?;
        let mut data = pool.data.lock().unwrap();
        let mut c = self.core.completion.lock().unwrap();
        if c.phase != Phase::Ready {
            return None;
        }
        let value = c.value.take();
        c.phase = Phase::Consumed;
        data.ready -= 1;
        value
    }
}
impl Drop for PendingIoOperation {
    fn drop(&mut self) {
        let Some(pool) = self.core.pool.upgrade() else {
            return;
        };
        let mut data = pool.data.lock().unwrap();
        let mut c = self.core.completion.lock().unwrap();
        let mut removed = None;
        let mut discarded = None;
        let mut closed_socket = false;
        match c.phase {
            Phase::Queued => {
                if let Some(i) = data
                    .queue
                    .iter()
                    .position(|j| Arc::ptr_eq(&j.core, &self.core))
                {
                    removed = data.queue.remove(i);
                }
                c.phase = Phase::Cancelled;
            }
            Phase::Running => {
                c.phase = Phase::CancelledRunning;
                data.cancelled_running[self.core.kind.index()] += 1;
            }
            Phase::Ready => {
                if matches!(c.value.as_ref(), Some(Ok(Value::Socket(_)))) {
                    closed_socket = true;
                }
                discarded = c.value.take();
                c.phase = Phase::Cancelled;
                data.ready -= 1;
            }
            _ => {}
        }
        drop(c);
        drop(data);
        // A queued provider closure may own another operation. Its destructor
        // must be free to cancel that operation without recursively locking us.
        drop(removed);
        drop(discarded);
        if closed_socket {
            pool.data.lock().unwrap().late_sockets_closed += 1;
        }
    }
}
pub(crate) fn remaining_connect_timeout(
    deadline: Instant,
    cap: std::time::Duration,
) -> io::Result<std::time::Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .map(|d| d.min(cap))
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::mpsc, time::Duration};

    struct Fixture(Arc<Pool>);
    impl Fixture {
        fn new() -> Self {
            Self(Pool::start().unwrap())
        }
        fn context(&self, request: u64) -> PendingIoContext {
            PendingIoContext {
                pool: self.0.clone(),
                key: IoKey {
                    owner: 1,
                    request,
                    generation: 1,
                    websocket: false,
                },
                deadline: Instant::now() + Duration::from_secs(10),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.0.data.lock().unwrap().closed = true;
            self.0.changed.notify_all();
        }
    }
    fn wait(mut condition: impl FnMut() -> bool) {
        let limit = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < limit, "native operation stalled");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn cancelled_running_work_reserves_capacity_and_queued_cancellation_removes_work() {
        let fixture = Fixture::new();
        let (entered, seen) = mpsc::channel();
        let mut releases = Vec::new();
        let mut operations = Vec::new();
        for request in 0..WORKERS_PER_KIND {
            let (release, held) = mpsc::channel();
            releases.push(release);
            let entered = entered.clone();
            operations.push(
                fixture
                    .context(request as u64)
                    .resolve(move || {
                        entered.send(()).unwrap();
                        held.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(Vec::new())
                    })
                    .unwrap(),
            );
        }
        for _ in 0..WORKERS_PER_KIND {
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        for request in WORKERS_PER_KIND..MAX_OWNED {
            operations.push(
                fixture
                    .context(request as u64)
                    .resolve(|| panic!("cancelled queued callback ran"))
                    .unwrap(),
            );
        }
        assert_eq!(fixture.0.snapshot().owned_operations, MAX_OWNED);
        assert_eq!(
            fixture
                .context(100)
                .resolve(|| Ok(Vec::new()))
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        // Cancelling queued work releases physical ownership immediately; running
        // provider work still owns its core until the actual callback returns.
        drop(operations.pop());
        assert_eq!(fixture.0.snapshot().owned_operations, MAX_OWNED - 1);
        drop(operations);
        let snapshot = fixture.0.snapshot();
        assert_eq!(snapshot.owned_operations, WORKERS_PER_KIND);
        assert_eq!(snapshot.cancelled_running_resolution, WORKERS_PER_KIND);
        assert_eq!(snapshot.queued_resolution, 0);
        for release in releases {
            release.send(()).unwrap();
        }
        wait(|| fixture.0.snapshot().owned_operations == 0);
        assert_eq!(fixture.0.snapshot().cancelled_running_resolution, 0);
    }

    #[test]
    fn cancelled_connect_closes_late_real_socket_without_sending() {
        use std::io::Read;
        let fixture = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (connected, seen) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let operation = fixture
            .context(1)
            .connect(move || {
                let socket = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
                connected.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(socket)
            })
            .unwrap();
        seen.recv_timeout(Duration::from_secs(5)).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        drop(operation);
        assert_eq!(fixture.0.snapshot().cancelled_running_connect, 1);
        assert_eq!(fixture.0.snapshot().owned_operations, 1);
        release.send(()).unwrap();
        assert_eq!(
            peer.read(&mut [0; 1]).unwrap(),
            0,
            "late socket emitted request data"
        );
        wait(|| fixture.0.snapshot().owned_operations == 0);
        assert_eq!(fixture.0.snapshot().late_sockets_closed, 1);
    }

    #[test]
    fn unread_ready_socket_and_wrong_owner_generation_namespace_are_not_delivered() {
        use std::io::Read;
        let fixture = Fixture::new();
        let context = fixture.context(7);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut operation = context
            .connect(move || TcpStream::connect_timeout(&address, Duration::from_secs(2)))
            .unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        wait(|| fixture.0.snapshot().ready_unread == 1);
        for field in 0..4 {
            let mut wrong = fixture.context(7);
            match field {
                0 => wrong.key.owner += 1,
                1 => wrong.key.request += 1,
                2 => wrong.key.generation += 1,
                _ => wrong.key.websocket = true,
            }
            assert_eq!(
                operation.poll(&wrong).unwrap().err().unwrap().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(fixture.0.snapshot().ready_unread, 1);
        }
        drop(operation);
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
        assert_eq!(fixture.0.snapshot().owned_operations, 0);
        assert_eq!(fixture.0.snapshot().late_sockets_closed, 1);
    }

    #[test]
    fn callback_panic_and_expired_queued_deadline_are_typed_terminal_errors() {
        let fixture = Fixture::new();
        let context = fixture.context(1);
        let mut operation = context
            .resolve(|| panic!("provider fixture panic"))
            .unwrap();
        wait(|| fixture.0.snapshot().ready_unread == 1);
        assert_eq!(
            operation.poll(&context).unwrap().err().unwrap().kind(),
            io::ErrorKind::Other
        );
        assert!(operation.poll(&context).is_none(), "completion replayed");
        drop(operation);
        let mut expired = fixture.context(2);
        expired.deadline = Instant::now();
        let mut operation = expired
            .resolve(|| panic!("expired callback executed"))
            .unwrap();
        wait(|| fixture.0.snapshot().ready_unread == 1);
        assert_eq!(
            operation.poll(&expired).unwrap().err().unwrap().kind(),
            io::ErrorKind::TimedOut
        );
        drop(operation);
        assert_eq!(fixture.0.snapshot().owned_operations, 0);
        assert_eq!(
            remaining_connect_timeout(Instant::now(), Duration::from_secs(3))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(
            remaining_connect_timeout(
                Instant::now() + Duration::from_millis(100),
                Duration::from_secs(3)
            )
            .unwrap()
                <= Duration::from_millis(100)
        );
    }
    #[test]
    fn cancelling_queued_provider_drops_nested_operation_outside_pool_lock() {
        let fixture = Fixture::new();
        let (entered, seen) = mpsc::channel();
        let mut running = Vec::new();
        let mut releases = Vec::new();
        for request in 0..WORKERS_PER_KIND {
            let (release, held) = mpsc::channel();
            releases.push(release);
            let entered = entered.clone();
            running.push(
                fixture
                    .context(request as u64)
                    .resolve(move || {
                        entered.send(()).unwrap();
                        held.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(Vec::new())
                    })
                    .unwrap(),
            );
        }
        for _ in 0..WORKERS_PER_KIND {
            seen.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        let inner = fixture
            .context(10)
            .resolve(|| panic!("cancelled inner callback ran"))
            .unwrap();
        let outer = fixture
            .context(11)
            .resolve(move || {
                drop(inner);
                Ok(Vec::new())
            })
            .unwrap();
        assert_eq!(fixture.0.snapshot().queued_resolution, 2);
        drop(outer);
        assert_eq!(fixture.0.snapshot().queued_resolution, 0);
        assert_eq!(fixture.0.snapshot().owned_operations, WORKERS_PER_KIND);
        drop(running);
        for release in releases {
            release.send(()).unwrap();
        }
        wait(|| fixture.0.snapshot().owned_operations == 0);
    }
}
