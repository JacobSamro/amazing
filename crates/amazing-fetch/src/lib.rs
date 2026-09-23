//! HTTP Fetch policy, cookie and cache algorithms. No I/O, persistence or JS bindings.
//! HTTP(S) URL syntax and resolution use amazing-net ParsedUrl; IDNA remains
//! deferred behind its provider seam. This policy model has no live dispatcher.
#![forbid(unsafe_code)]
pub mod api;
pub mod cache;
pub mod cookies;
mod cors;
pub mod csp;
pub mod data_url;
mod headers;
pub mod mime;
mod model;
mod origin;
pub mod psl;
mod redirect;
mod referrer;
mod security;
pub mod service;
pub mod xhr;
pub use cors::*;
pub use csp::*;
pub use data_url::*;
pub use headers::*;
pub use mime::*;
pub use model::*;
pub use origin::*;
pub use redirect::*;
pub use referrer::*;
pub use security::*;
pub use service::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    InvalidUrl,
    InvalidHeader,
    InvalidMethod,
    InvalidBody,
    SameOrigin,
    Cors,
    NoCors,
    Redirect,
    RedirectLimit,
    BodyNotReplayable,
    BadPort,
    MixedContent,
    CrossOriginResourcePolicy,
    MimeType,
    Nosniff,
    OpaqueResponse,
    Csp(Vec<CspViolation>),
    Transport,
}
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fetch policy error: {self:?}")
    }
}
impl std::error::Error for FetchError {}
