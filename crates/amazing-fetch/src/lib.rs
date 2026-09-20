//! Fetch standard model — the single loading path for page subresources.
//!
//! This crate owns the WHATWG Fetch request/response model, the fetch
//! algorithm's redirect handling, CORS (preflight decisions, response tainting,
//! header filtering), referrer policy, and the origin/site types those rules
//! depend on. `amazing-net` remains the transport underneath it.
//!
//! Like every engine core it is **std-only**: no third-party crates. The vetted
//! TLS exception lives in `amazing-net-tls` and does not extend here.
//!
//! Skeleton registered by the integrator so the workspace resolves; the `net`
//! lane implements it under ticket NET-1.

#![forbid(unsafe_code)]
