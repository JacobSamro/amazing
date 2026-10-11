//! Cross-agent SharedArrayBuffer storage and the Atomics wait/notify waiter list.
//!
//! A `SharedBlock` owns one heap allocation that several agents (OS threads,
//! each with its own `Interpreter`) alias. Each agent's `BinaryBuffer` keeps a
//! `Vec<u8>` view over the block that is never freed or reallocated by that
//! agent (the block's capacity is the buffer's maximum length). Atomics
//! read-modify-write operations and wait/notify serialize on `SHARED`, which
//! also makes the "value still equals" check in `Atomics.wait` atomic with the
//! registration of the waiter.
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub struct SharedBlock {
    ptr: *mut u8,
    cap: usize,
    len: AtomicUsize,
    id: u64,
    max: usize,
    growable: bool,
}

// The block is raw shared memory by design; all synchronization that ECMAScript
// promises goes through `lock()`.
unsafe impl Send for SharedBlock {}
unsafe impl Sync for SharedBlock {}

static NEXT_BLOCK: AtomicU64 = AtomicU64::new(1);

impl SharedBlock {
    /// Move `bytes` into a fresh block with room for `capacity` bytes.
    pub(crate) fn new(bytes: &[u8], capacity: usize, growable: bool) -> Option<Arc<SharedBlock>> {
        let cap = capacity.max(bytes.len());
        let mut owner: Vec<u8> = Vec::new();
        owner.try_reserve_exact(cap).ok()?;
        owner.extend_from_slice(bytes);
        let mut owner = ManuallyDrop::new(owner);
        Some(Arc::new(SharedBlock {
            ptr: owner.as_mut_ptr(),
            cap: owner.capacity(),
            len: AtomicUsize::new(bytes.len()),
            id: NEXT_BLOCK.fetch_add(1, Ordering::Relaxed),
            max: capacity.max(bytes.len()),
            growable,
        }))
    }

    pub fn id(&self) -> u64 { self.id }
    pub fn max_byte_length(&self) -> usize { self.max }
    pub fn growable(&self) -> bool { self.growable }
    pub fn len(&self) -> usize { self.len.load(Ordering::Relaxed) }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub(crate) fn publish_len(&self, len: usize) { self.len.store(len, Ordering::Relaxed); }

    /// A `Vec` aliasing the block. The caller must never drop it (it is
    /// forgotten by `BinaryBuffer::drop`) and must never grow it past `cap`.
    pub(crate) fn alias(&self) -> Vec<u8> {
        unsafe { Vec::from_raw_parts(self.ptr, self.len(), self.cap) }
    }
}

impl Drop for SharedBlock {
    fn drop(&mut self) {
        drop(unsafe { Vec::from_raw_parts(self.ptr, 0, self.cap) });
    }
}

struct Waiter {
    block: u64,
    offset: usize,
    id: u64,
    notified: bool,
}

#[derive(Default)]
pub(crate) struct SharedState {
    waiters: Vec<Waiter>,
    next_id: u64,
}

static SHARED: Mutex<SharedState> = Mutex::new(SharedState { waiters: Vec::new(), next_id: 0 });
static CHANGED: Condvar = Condvar::new();
static AGENTS_STARTED: AtomicUsize = AtomicUsize::new(0);

// Process-wide: coroutine worker threads of one agent must observe the same value.
static CAN_BLOCK: AtomicBool = AtomicBool::new(true);

/// Whether the agents of this process may suspend in `Atomics.wait` ([[CanBlock]]).
pub fn can_block() -> bool { CAN_BLOCK.load(Ordering::SeqCst) }
/// Host hook: set [[CanBlock]] (the Test262 host's `--cannot-block` mode).
pub fn set_can_block(value: bool) { CAN_BLOCK.store(value, Ordering::SeqCst); }
/// Host hook: record that a second agent exists, so waits honour their timeout in full.
pub fn note_agent_started() { AGENTS_STARTED.fetch_add(1, Ordering::SeqCst); }
pub(crate) fn other_agents_exist() -> bool { AGENTS_STARTED.load(Ordering::SeqCst) > 0 }

pub(crate) fn lock() -> MutexGuard<'static, SharedState> {
    SHARED.lock().unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) enum WaitOutcome { Ok, TimedOut }

/// Register a waiter on (block, offset) and park until notified or timed out.
/// `guard` must be the lock under which the caller verified the value.
pub(crate) fn wait_registered(
    mut guard: MutexGuard<'static, SharedState>, block: u64, offset: usize, timeout: Option<Duration>,
) -> WaitOutcome {
    let id = guard.next_id;
    guard.next_id += 1;
    guard.waiters.push(Waiter { block, offset, id, notified: false });
    let deadline = timeout.map(|t| Instant::now() + t);
    let outcome = loop {
        if guard.waiters.iter().any(|w| w.id == id && w.notified) { break WaitOutcome::Ok; }
        match deadline {
            None => guard = CHANGED.wait(guard).unwrap_or_else(|p| p.into_inner()),
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline { break WaitOutcome::TimedOut; }
                guard = CHANGED.wait_timeout(guard, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
            }
        }
    };
    guard.waiters.retain(|w| w.id != id);
    outcome
}

/// Register a pending `Atomics.waitAsync` waiter (verified under `guard`'s lock).
pub(crate) fn register_async(mut guard: MutexGuard<'static, SharedState>, block: u64, offset: usize) -> u64 {
    let id = guard.next_id;
    guard.next_id += 1;
    guard.waiters.push(Waiter { block, offset, id, notified: false });
    id
}

/// Settle an async waiter owned by the calling agent: `Some(true)` when it was
/// notified, `Some(false)` when `expired` (it is withdrawn), `None` while pending.
pub(crate) fn resolve_async(id: u64, expired: bool) -> Option<bool> {
    let mut guard = lock();
    let position = guard.waiters.iter().position(|w| w.id == id);
    match position {
        None => Some(true),
        Some(at) if guard.waiters[at].notified => { guard.waiters.remove(at); Some(true) }
        Some(at) if expired => { guard.waiters.remove(at); Some(false) }
        Some(_) => None,
    }
}

/// Wake up to `count` waiters (FIFO) on (block, offset); returns how many.
pub(crate) fn notify(guard: &mut MutexGuard<'static, SharedState>, block: u64, offset: usize, count: f64) -> usize {
    let mut woken = 0usize;
    for w in guard.waiters.iter_mut() {
        if (woken as f64) >= count { break; }
        if w.block == block && w.offset == offset && !w.notified {
            w.notified = true;
            woken += 1;
        }
    }
    if woken > 0 { CHANGED.notify_all(); }
    woken
}
