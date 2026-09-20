//! Pure HTTP Fetch policy prototype. No I/O, cookies, cache or JS bindings.
//! URL input is deliberately limited to canonical ASCII HTTP(S); this is not
//! a WHATWG URL parser. A future URL crate must supply normalized URLs.
#![forbid(unsafe_code)]
mod cors;
mod headers;
mod model;
mod origin;
mod redirect;
mod referrer;
pub use cors::*;
pub use headers::*;
pub use model::*;
pub use origin::*;
pub use redirect::*;
pub use referrer::*;

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
}
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fetch policy error: {self:?}")
    }
}
impl std::error::Error for FetchError {}
