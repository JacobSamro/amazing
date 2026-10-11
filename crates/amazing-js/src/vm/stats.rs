//! Opt-in VM coverage counting. Off unless `AMAZING_JS_VM_STATS=<file>` is set;
//! when off every entry point is one cached `OnceLock` read.
//!
//! Output is append-only TSV, one write per line (atomic enough for the many
//! Test262 host processes sharing a file). Each line starts with the pid:
//!   `<pid> P <outcome> <reason>`   a program (script) execution decision
//!   `<pid> F <ok|fail> <reason>`   a callable compile attempt (at instantiation)
//!   `<pid> C <vm_calls> <tree_calls>`  JS-function calls since the last flush
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static TARGET: OnceLock<Option<PathBuf>> = OnceLock::new();

thread_local! {
    static REASON: Cell<&'static str> = const { Cell::new("") };
    static CALLS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
    static FILE: RefCell<Option<std::fs::File>> = const { RefCell::new(None) };
}

fn target() -> Option<&'static PathBuf> {
    TARGET.get_or_init(|| std::env::var_os("AMAZING_JS_VM_STATS").map(PathBuf::from)).as_ref()
}

#[inline]
pub(crate) fn enabled() -> bool {
    target().is_some()
}

fn emit(line: String) {
    let Some(path) = target() else { return };
    FILE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
        }
        if let Some(file) = slot.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    });
}

/// Eligibility leaves record the construct that forced fallback.
#[inline]
pub(crate) fn note_reason(reason: &'static str) {
    REASON.with(|cell| cell.set(reason));
}

pub(crate) fn take_reason() -> &'static str {
    REASON.with(|cell| cell.replace(""))
}

pub(crate) fn program(outcome: &str, reason: &str) {
    if enabled() {
        emit(format!("{}\tP\t{outcome}\t{reason}\n", std::process::id()));
    }
}

pub(crate) fn compile_attempt(reason: Option<&str>) {
    if enabled() {
        let (state, reason) = match reason { None => ("ok", ""), Some(reason) => ("fail", reason) };
        emit(format!("{}\tF\t{state}\t{reason}\n", std::process::id()));
    }
}

#[inline]
pub(crate) fn count_call(vm: bool) {
    if enabled() {
        CALLS.with(|cell| {
            let (a, b) = cell.get();
            cell.set(if vm { (a + 1, b) } else { (a, b + 1) });
        });
    }
}

pub(crate) fn flush_calls() {
    if enabled() {
        let (vm, tree) = CALLS.with(|cell| cell.replace((0, 0)));
        if vm + tree > 0 {
            emit(format!("{}\tC\t{vm}\t{tree}\n", std::process::id()));
        }
    }
}
