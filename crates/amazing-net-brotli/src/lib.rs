//! Bounded brotli decoding for HTTP content coding.
//!
//! Approved as a project-level dependency exception in `ARCHITECTURE.md` 6d and
//! ADR 0002. This is a **separate** exception from the font-scoped approval in
//! decision 4d, even though `brotli-decompressor` is already pinned
//! transitively through `wuff` in `amazing-text-provider`, because this path
//! decodes attacker-supplied response bodies.
//!
//! Therefore: the caller-selected maximum decoded size is enforced **before**
//! allocation, so a compression bomb fails closed rather than expanding first.
//! Decode only — there is no encoder. Dependency types never cross this
//! boundary; `amazing-net` receives bounded owned bytes and errors.
//!
#![forbid(unsafe_code)]
use brotli_decompressor::{
    Allocator, BrotliDecompressStream, BrotliResult, BrotliState, SliceWrapper, SliceWrapperMut,
};
use std::cell::Cell;
use std::rc::Rc;

/// Maximum simultaneous decoder scratch, separate from caller-selected output.
/// Strict RFC7932 window (at most 16 MiB) plus tables fits within this budget.
pub const MAX_SCRATCH_BYTES: usize = 32 * 1024 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    OutputLimit,
    ScratchLimit,
    Allocation,
    Invalid,
    Truncated,
    TrailingData,
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "brotli: {self:?}")
    }
}
impl std::error::Error for DecodeError {}

struct Budget {
    live: Cell<usize>,
    peak: Cell<usize>,
    error: Cell<Option<DecodeError>>,
    limit: usize,
}
impl Budget {
    fn new(limit: usize) -> Self {
        Self {
            live: Cell::new(0),
            peak: Cell::new(0),
            error: Cell::new(None),
            limit,
        }
    }
}
struct Memory<T> {
    data: Vec<T>,
    budget: Option<Rc<Budget>>,
    charged: usize,
}
impl<T> Default for Memory<T> {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            budget: None,
            charged: 0,
        }
    }
}
impl<T> SliceWrapper<T> for Memory<T> {
    fn slice(&self) -> &[T] {
        &self.data
    }
}
impl<T> SliceWrapperMut<T> for Memory<T> {
    fn slice_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}
impl<T> Drop for Memory<T> {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.live.set(budget.live.get() - self.charged);
        }
    }
}
struct BoundedAlloc(Rc<Budget>);
impl<T: Default + Clone> Allocator<T> for BoundedAlloc {
    type AllocatedMemory = Memory<T>;
    fn alloc_cell(&mut self, len: usize) -> Memory<T> {
        let Some(bytes) = len.checked_mul(std::mem::size_of::<T>()) else {
            self.0.error.set(Some(DecodeError::ScratchLimit));
            return Memory::default();
        };
        if bytes > self.0.limit.saturating_sub(self.0.live.get()) {
            self.0.error.set(Some(DecodeError::ScratchLimit));
            return Memory::default();
        }
        let mut data = Vec::new();
        if data.try_reserve_exact(len).is_err() {
            self.0.error.set(Some(DecodeError::Allocation));
            return Memory::default();
        }
        data.resize(len, T::default());
        self.0.live.set(self.0.live.get() + bytes);
        self.0.peak.set(self.0.peak.get().max(self.0.live.get()));
        Memory {
            data,
            budget: Some(self.0.clone()),
            charged: bytes,
        }
    }
    fn free_cell(&mut self, data: Memory<T>) {
        drop(data);
    }
}

struct Output {
    bytes: Vec<u8>,
    limit: usize,
}
impl Output {
    fn append(&mut self, chunk: &[u8]) -> Result<(), DecodeError> {
        // This check MUST precede reserve/resize/copy. Never read_to_end then check.
        if chunk.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(DecodeError::OutputLimit);
        }
        self.bytes
            .try_reserve_exact(chunk.len())
            .map_err(|_| DecodeError::Allocation)?;
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
}

/// Decode one complete RFC7932 stream; reject extensions, truncation and trailing
/// bytes. No output allocation request exceeds `max_output`. A fixed stack chunk
/// and independently capped decoder scratch are used to detect expansion.
pub fn decode(input: &[u8], max_output: usize) -> Result<Vec<u8>, DecodeError> {
    decode_with_budget(input, max_output, Rc::new(Budget::new(MAX_SCRATCH_BYTES)))
}
fn decode_with_budget(
    input: &[u8],
    max_output: usize,
    budget: Rc<Budget>,
) -> Result<Vec<u8>, DecodeError> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut state = BrotliState::new_strict(
            BoundedAlloc(budget.clone()),
            BoundedAlloc(budget.clone()),
            BoundedAlloc(budget.clone()),
        );
        let mut output = Output {
            bytes: Vec::new(),
            limit: max_output,
        };
        let mut input_pos = 0;
        let mut available_in = input.len();
        let mut written = 0;
        let mut chunk = [0u8; 8192];
        loop {
            // One probe byte after the bound detects a bomb without growing output.
            let mut available_out = chunk.len().min(
                max_output
                    .saturating_sub(output.bytes.len())
                    .saturating_add(1),
            );
            let mut output_pos = 0;
            let status = BrotliDecompressStream(
                &mut available_in,
                &mut input_pos,
                input,
                &mut available_out,
                &mut output_pos,
                &mut chunk,
                &mut written,
                &mut state,
            );
            if let Some(error) = budget.error.get() {
                return Err(error);
            }
            output.append(&chunk[..output_pos])?;
            match status {
                BrotliResult::ResultSuccess => {
                    if input_pos != input.len() {
                        return Err(DecodeError::TrailingData);
                    }
                    return Ok(output.bytes);
                }
                BrotliResult::ResultFailure => return Err(DecodeError::Invalid),
                BrotliResult::NeedsMoreInput => return Err(DecodeError::Truncated),
                BrotliResult::NeedsMoreOutput => {}
            }
        }
    }));
    // Allocation-refusal paths in a dependency may panic on an empty slice;
    // no such unwind crosses our boundary and all charged memory is released.
    result.unwrap_or_else(|_| Err(budget.error.get().unwrap_or(DecodeError::Invalid)))
}

#[cfg(test)]
mod tests;
