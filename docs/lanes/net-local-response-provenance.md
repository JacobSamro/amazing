# Successful synchronous loader provenance

Recorded base: `570b0718a22e6b5bc34a05ff0f13a6955e873973`.

HTML's genuine initial/literal-global Reporting admission must distinguish an
actual successful local router read from an accepted HTTP response. The legacy
buffered `Response` has public fields and URL text cannot authenticate that
boundary. This ticket adopts the native producer first preserved in
`895beba19509f419a37fd4c65df8abbcb5b1143a`, only its fetch.rs/lib.rs delta,
with eleven complete native control groups and the original authenticated H2
pull-stream control extended to assert genuine network source. No historical driver or Reporting
source is imported.

`fetch_received(&str, &FetchOptions) -> Result<ReceivedResponse, NetError>`
retains a private receipt minted only by successful concrete filesystem or HTTP
router branches. `response()` borrows the unchanged buffered response;
`is_local_file()` exposes actual router provenance; `into_response()` consumes
it. The legacy `fetch` delegates and retains its exact result/error semantics.
Both buffered `source()` and streaming `source()` return opaque Copy
`ReceivedSource`, whose sole public inspector is `is_local_file()`. The token
has no public constructor or Default. Concrete successful file-stream open
mints local source; HTTP/1 and HTTP/2 head construction mint network source.
Copying the token copies no body. Late streaming body failure leaves the
recorded route unchanged, and the token does not claim body completion.

No public buffered receipt constructor or mutable response accessor exists. Cloning preserves
both the immutable response and its actual source provenance.

The controls cover file URLs and bare paths, exact legacy URLs/bodies, spaces
and URL decoding, actual HTTP Reporting headers, every supported redirect
status, rejection of network-to-file redirects even with an existing target,
local missing/oversized/error paths, network truncation and response limits.
Four further streaming groups cover real file open/drain/drop, all network
redirects, rejected network-to-file and local failures, and late HTTP framing
failure. All original assertions remain. Compile-fail doctests check that external
callers cannot mint the authority.

This is native loader provenance, not installed HTML admission proof. HTML owns
actual fresh-global initialization and accepted-response registration; it must
consume this receipt directly and must not use failed HTTP fallback or URL
shape as equivalent authority. Generic Script/module original-source CSP loans
and parked Reporting410/wire-origin source are separate tickets.

Qualification is pending. The complete paired plan retains workspace nextest,
workspace doctests, browser build, original full TLS/net/fetch native corpus and
original eleven CSP WPT directories. Source and commands freeze before any
supported dedicated allocation. No local build/test has run.
