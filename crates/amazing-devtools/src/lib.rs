//! A Chrome DevTools Protocol server for the Amazing engine (Stage 4).
//!
//! The crate is std-only and engine-agnostic. It provides:
//!
//! * [`server::CdpServer`] — a localhost TCP listener exposing Chrome's
//!   discovery contract (`GET /json/version`, `GET /json/list`) and WebSocket
//!   endpoints (`/devtools/page/<id>`, `/devtools/browser`).
//! * [`ws`] — the RFC 6455 server-side message layer (masked client frames,
//!   fragmentation, ping/pong, close, UTF-8 validation).
//! * [`protocol`] — CDP's JSON-RPC request/response/event framing, including
//!   flattened `sessionId` routing.
//! * [`session`] and [`domains`] — the state machine for the `Target`, `Page`,
//!   `Runtime`, `DOM`, `CSS`, `Network`, `Log`, `Input`, `Fetch` and
//!   `Emulation` domains.
//! * [`host`] — the seam the driver implements: [`host::DevToolsHost`] and
//!   [`host::DevToolsSession`]. Every method is optional and defaults to
//!   "unsupported", so domains can be adopted one at a time.
//! * [`mock`] — an in-memory [`host::DevToolsHost`] for tests and early driver
//!   integration.
//!
//! ## Wiring the server
//!
//! ```no_run
//! use std::sync::{Arc, atomic::AtomicBool};
//! use amazing_devtools::mock::MockHost;
//! use amazing_devtools::server::CdpServer;
//!
//! let host = Arc::new(MockHost::new());
//! let server = CdpServer::bind(host, "127.0.0.1:9222").unwrap();
//! let addr = server.local_addr().unwrap();
//! server.serve(Arc::new(AtomicBool::new(false))).unwrap();
//! # let _ = addr;
//! ```
//!
//! ## Implementing the driver side
//!
//! The driver builds one type implementing [`host::DevToolsHost`] (product
//! strings and the target list) and returns a [`host::DevToolsSession`] per
//! target from `attach`. The session answers engine questions; it never sees
//! JSON or WebSocket bytes. Events are produced by `drain_*` and polled by the
//! connection's own thread.

#![forbid(unsafe_code)]

pub mod base64;
pub mod domains;
pub mod host;
pub mod http;
pub mod json;
pub mod mock;
pub mod protocol;
pub mod server;
pub mod session;
pub mod sha1;
pub mod ws;

pub use host::{
    DevToolsHost, DevToolsSession, EngineNodeId, HostError, HostInfo, TargetInfo,
};
pub use json::Json;
pub use server::{CdpServer, ServerConfig};
