//! Amazing net — HTTP/1.1 over `std::net::TcpStream`.
//!
//! **Status: Phase 5 (not started).**
//!
//! Scope decision recorded now (per project constraints):
//! - v1 ships HTTP/1.1 only: request line, headers, chunked transfer,
//!   content-length bodies, redirects, connection reuse.
//! - TLS is the designated exception candidate. Hand-rolling TLS 1.3 is a
//!   security-critical multi-month sub-project; the working assumption is
//!   that Amazing will treat a vetted TLS implementation as infrastructure
//!   (the same call every serious from-scratch browser makes). Final
//!   decision + justification due before Phase 5 starts.
//! - HTTP/2 deferred indefinitely for v1.

#[cfg(test)]
mod tests {
    #[test]
    fn phase5_not_started() {
        assert!(true);
    }
}
