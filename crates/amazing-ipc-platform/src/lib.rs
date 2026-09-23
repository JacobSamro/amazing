//! Audited OS shared-memory and handle transfer for amazing-ipc (ADR 0007).
//! All unsafe code is confined to platform modules.
#![deny(unsafe_code)]

use std::collections::VecDeque;
use std::io;

pub const MAX_MAPPED_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_OUTSTANDING: usize = 3;
pub const MAX_DIMENSION: u32 = 8192;
pub const FORMAT_RGBA8: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameMetadata {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub byte_len: u64,
    pub handle_id: u64,
}
impl FrameMetadata {
    pub fn validate(&self) -> Result<usize, Error> {
        if self.generation == 0
            || self.handle_id == 0
            || self.format != FORMAT_RGBA8
            || self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
            || self.stride != self.width.checked_mul(4).ok_or(Error::InvalidMetadata)?
        {
            return Err(Error::InvalidMetadata);
        }
        let len = u64::from(self.stride) * u64::from(self.height);
        if len != self.byte_len || len > MAX_MAPPED_BYTES as u64 {
            return Err(Error::InvalidMetadata);
        }
        usize::try_from(len).map_err(|_| Error::InvalidMetadata)
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidMetadata,
    Budget,
    StaleGeneration,
    DuplicateHandle,
    Authentication,
    Unsupported,
    Io(io::Error),
}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// Reserves mapped bytes before allocation. Acquired frames are protected.
pub struct FrameBudget {
    entries: VecDeque<(u64, usize, bool)>,
    bytes: usize,
    last_generation: u64,
}
impl FrameBudget {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            last_generation: 0,
        }
    }
    /// Returns the generations whose handles must be closed before allocation.
    pub fn reserve(&mut self, generation: u64, bytes: usize) -> Result<Vec<u64>, Error> {
        if generation == 0 || generation <= self.last_generation {
            return Err(Error::StaleGeneration);
        }
        if bytes == 0 || bytes > MAX_MAPPED_BYTES {
            return Err(Error::Budget);
        }
        let mut replaced = Vec::new();
        let mut count = self.entries.len();
        let mut total = self.bytes;
        for &(old, size, presented) in &self.entries {
            if count < MAX_OUTSTANDING && total + bytes <= MAX_MAPPED_BYTES {
                break;
            }
            if !presented {
                replaced.push(old);
                count -= 1;
                total -= size;
            }
        }
        if count >= MAX_OUTSTANDING || total + bytes > MAX_MAPPED_BYTES {
            return Err(Error::Budget);
        }
        self.entries.retain(|(id, _, _)| !replaced.contains(id));
        self.bytes = total;
        self.entries.push_back((generation, bytes, false));
        self.bytes += bytes;
        self.last_generation = generation;
        Ok(replaced)
    }
    pub fn protect(&mut self, generation: u64) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|(id, _, _)| *id == generation) {
            entry.2 = true;
            true
        } else {
            false
        }
    }
    pub fn release(&mut self, generation: u64) -> bool {
        if let Some(index) = self.entries.iter().position(|(id, _, _)| *id == generation) {
            if let Some((_, bytes, _)) = self.entries.remove(index) {
                self.bytes -= bytes;
            }
            true
        } else {
            false
        }
    }
    /// Close pending frames on navigation replacement, retaining acquired views.
    pub fn discard_unpresented(&mut self) -> Vec<u64> {
        let mut discarded = Vec::new();
        self.entries.retain(|(id, bytes, protected)| {
            if *protected {
                true
            } else {
                discarded.push(*id);
                self.bytes -= *bytes;
                false
            }
        });
        discarded
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    pub fn mapped_bytes(&self) -> usize {
        self.bytes
    }
    pub fn outstanding(&self) -> usize {
        self.entries.len()
    }
}
impl Default for FrameBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{FrameHandle, FrameView, HandleChannel};
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;
#[cfg(windows)]
pub use windows::{FrameHandle, FrameView, HandleChannel};
#[cfg(windows)]
pub use windows_sandbox::{probe_open_process, spawn_renderer_appcontainer, WindowsSandboxChild};
#[cfg(not(any(target_os = "linux", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "linux", windows)))]
pub use unsupported::{FrameHandle, FrameView, HandleChannel};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod linux_sandbox;
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_sandbox;
/// Enter the reviewed Linux renderer profile before processing document input.
/// A caller must abort child bootstrap if this returns an error.
pub fn enter_renderer_sandbox(frame_fd: i32) -> Result<(), Error> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return linux_sandbox::enter(frame_fd).map_err(Error::Io);
    }
    #[cfg(windows)]
    {
        let _ = frame_fd;
        return windows_sandbox::verify_current_appcontainer().map_err(Error::Io);
    }
    #[cfg(not(any(all(target_os = "linux", target_arch = "x86_64"), windows)))]
    {
        let _ = frame_fd;
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_and_budget() {
        let mut budget = FrameBudget::new();
        for id in 1..=3 {
            assert!(budget.reserve(id, 100).unwrap().is_empty());
        }
        assert_eq!(budget.reserve(4, 100).unwrap(), vec![1]);
        assert!(budget.protect(2));
        assert_eq!(budget.reserve(5, 100).unwrap(), vec![3]);
        assert!(budget.protect(4));
        assert!(budget.protect(5));
        assert!(matches!(budget.reserve(6, 100), Err(Error::Budget)));
        assert_eq!(budget.mapped_bytes(), 300);
        assert!(budget.release(4));
        assert!(matches!(
            budget.reserve(5, 100),
            Err(Error::StaleGeneration)
        ));
        assert!(matches!(
            budget.reserve(6, MAX_MAPPED_BYTES + 1),
            Err(Error::Budget)
        ));
        let mut mapped = FrameBudget::new();
        assert!(mapped.reserve(1, MAX_MAPPED_BYTES / 2).unwrap().is_empty());
        assert!(mapped.reserve(2, MAX_MAPPED_BYTES / 2).unwrap().is_empty());
        assert_eq!(mapped.reserve(3, 1).unwrap(), vec![1]);
        assert!(mapped.mapped_bytes() <= MAX_MAPPED_BYTES);
        mapped.protect(2);
        mapped.protect(3);
        assert!(matches!(
            mapped.reserve(4, MAX_MAPPED_BYTES),
            Err(Error::Budget)
        ));
        let mut seed = 0x7612_8394_abc1_ef56u64;
        for _ in 0..10_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let metadata = FrameMetadata {
                generation: seed,
                width: (seed >> 32) as u32,
                height: seed as u32,
                stride: (seed >> 16) as u32,
                format: (seed >> 40) as u32,
                byte_len: seed,
                handle_id: seed.rotate_left(13),
            };
            let _ = metadata.validate();
        }
    }
}
