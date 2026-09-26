//! Minimal per-statement hook seam for a debugger front end (CDP-20).
//!
//! This is deliberately small. [`crate::interp::Interpreter`] calls
//! [`DebugHook::on_statement`] once before executing every statement, from
//! `Interpreter::exec_stmt` (the choke-point for everything *except* a
//! top-level expression statement) and from the top-level program loop's own
//! fast path for a bare `expr;` statement (which bypasses `exec_stmt`
//! entirely for performance) — both call sites pass the 1-based source line
//! already carried on every `ast::Stmt::span` (`Span::start.line`) plus
//! whichever script id/url the host most recently told the interpreter it is
//! running ([`crate::interp::Interpreter::set_current_script`]).
//!
//! Whether a hit becomes an actual suspend/resume is entirely a host
//! decision — the interpreter does not block, buffer continuations, or know
//! what a "breakpoint" is. It only makes the callback. This crate does not
//! (yet) support true synchronous pausing: this is a single-threaded,
//! tree-walking interpreter with no coroutine/fiber mechanism, so a host
//! cannot block *this* call and still expect to service other work on the
//! same thread from inside the callback. A host wanting real "stop the
//! world, inspect, then continue from the exact same statement" pausing
//! needs the interpreter to expose a resumable/steppable execution API,
//! which is a bigger interpreter change than this hook — see
//! `.agents/js-runtime/questions.md` for the open request from the
//! `devtools` lane.
use std::cell::RefCell;
use std::rc::Rc;

/// Called by the interpreter before running the statement at `line` (1-based,
/// from `ast::Span::start.line`) inside the script most recently identified
/// by [`crate::interp::Interpreter::set_current_script`] (`script_id`/`url`).
/// Implementations are called on *every* statement while installed (there is
/// no cheap pre-filtering by the interpreter), so they should stay fast, and
/// must not panic — a panicking hook would poison the whole interpreter call.
pub trait DebugHook {
    fn on_statement(&mut self, script_id: &str, url: &str, line: u32);
}

/// A hook shared between the interpreter (which calls it) and whatever owns
/// the breakpoint/pause state on the host side (which mutates that state
/// from other, unrelated calls on the same thread). `Rc<RefCell<_>>` rather
/// than `Arc<Mutex<_>>` because the interpreter itself is single-threaded
/// and `Rc`-based (locked decision — see ARCHITECTURE.md).
pub type SharedDebugHook = Rc<RefCell<dyn DebugHook>>;
