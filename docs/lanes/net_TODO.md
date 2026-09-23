# Net lane decision map

## Approved external-data and decoder decisions

PSL data and isolated network Brotli decoding were approved in architecture
6c/6d and ADR 0002. NET-6 implements them below. The PSL snapshot includes ICANN
and PRIVATE rules and changes only through reviewed PRs with persisted-key
migration review. Brotli is a separate network exception from font decision 4d:
attacker-supplied bodies require a caller output limit checked before allocation.
Runtime IDNA and live shell cookie/persistence adoption remain separate work.

## NET-1 — Fetch policy prototype

Pure request/response policy: no socket dispatcher or JS API yet. The registered
workspace crate uses no new external dependency. Its existing amazing-net path
dependency retains the reviewed TLS exception; policy uses only std and forbids
unsafe. Baseline rebased to origin/dev e9c1e21, including Fetch/regex registration.

Sources consulted 2026-09-20:
- https://fetch.spec.whatwg.org/ (living standard dated 2026-09-02): headers,
  requests/responses, HTTP redirects, CORS checks and preflights.
- https://w3c.github.io/webappsec-referrer-policy/: policy selection and redirects.

### Design

`Request` owns an origin, nonempty URL list, validated method/body pair, mode,
credentials, redirect mode, headers and referrer settings. `prepare` produces
per-hop credential/preflight/referrer decisions and retains response taint.
`redirect` prepares and checks CORS on the source hop before changing its target.

`InternalResponse` is transport-owned; `filter_response` produces an owned
`Response` with no pointer back to hidden data. Basic/CORS views remove cookie
headers. Opaque views hide body/status/headers/URLs. Manual redirects retain the
source URL list; navigation/manual returns `NavigationPending` for the controller.

Redirects accept an already-resolved `HttpUrl`. Missing Location differs from
parse failure: the adapter must surface failed resolution as an error. Chains
stop after 20 hops. The model handles method/body rewrites, replayability,
fragment inheritance, cross-origin Authorization removal, and referrer policy.

`Origin` has normalized tuple identity or unique opaque identity. `Site` requires
a trusted `RegistrableDomain` provider using a pinned public suffix list, never a
last-two-label guess. `HttpUrl` accepts strict ASCII HTTP(S), normalizing host,
default ports and IP literals. It rejects userinfo, non-ASCII, backslashes,
percent hosts, empty DNS labels and legacy IPv4. It is not a WHATWG URL parser:
relative resolution, IDNA and full special-URL parsing await the shared URL layer.
Callers supply normalized paths and resolved Location targets.

`HeaderList` validates names and CR/LF/NUL injection, preserves repeated response
fields and supports combined lookup. CORS checks combined request values too,
preventing duplicate-field bypasses. This is conservative: repeated individually
safelisted values can force preflight. Values are UTF-8 strings, not WebIDL
ByteStrings. The type is an internal list, not the JS Headers API.

All eight referrer policies are represented. Fragments are stripped; values over
4096 bytes reduce to origin; later redirects cannot restore removed paths.
HTTPS-to-HTTP always counts as downgrade in this subset; trustworthy localhost
exceptions await Secure Contexts integration.

### CORS decision table

| Request | Result | Preflight |
|---|---|---|
| Same-origin URL, ordinary request | Basic | No |
| Cross-origin, same-origin mode | Error | No |
| Cross-origin, no-cors, safe method/headers, follow | Opaque | No |
| No-cors with unsafe method/headers or other redirect mode | Error | No |
| Cross-origin cors, safe method/headers | CORS after check | No |
| Cross-origin cors, unsafe method/header or forced flag | CORS after check | Yes |
| Navigate | Basic; manual redirects handed to controller | No |
| CORS chain returning to initiator | Remains CORS | Per method/headers |

Credentials `include` requires exact ACAO and case-sensitive ACAC `true`; other
modes permit ACAO `*`. Preflight sends OPTIONS, sorted unique unsafe header names,
and no credentials. Wildcard method/header grants apply only without `include`;
Authorization always needs an explicit grant. Exposed-header wildcard is literal
with `include`. Redirect-tainted origin serializes as `null` in CORS without
collapsing opaque identity.

### NET-1 follow-up seam proposal (now deferred to NET-3)

`NetworkOrCache::exchange(request, plan, top_level_site)` is a trusted single-hop
boundary implemented by the shell/driver. Cookies/cache stay in the shell. It
must obey `plan.credentials` for credential sending and accepting Set-Cookie,
partition internal cached responses by site, and reapply CORS/filtering on every
delivery. Transport framing must regenerate Content-Length from the current
body after redirect rewriting.

1. Agree with integrator on single-hop transport supporting OPTIONS/arbitrary
   methods/bodies, no automatic redirects, unfiltered response headers, and
   Location resolution preserving URL-credential information.
2. Route shell cookies/cache through the seam. Keep author request-header guards
   and UA-inserted Origin/Referer/Cookie separate from preflight author headers.
3. Add loading coordinator/preflight cache: prepare; send and validate required
   preflight; exchange; redirect; repeat on Follow; filter final response.
   NavigationPending needs an HTML controller, not script response delivery.
4. Pin WPT fetch/cookies after testharness/DOM integration. Later gates cover
   streaming/cancellation, destinations/cache modes, body consumption, CSP/CORP,
   mixed content, full URL and Secure Contexts behavior.

### Evidence

Baseline win-001: amazing-net 65 passed, TLS adapter 2 passed, no failures or
ignored tests. Fetch skeleton had no tests. This is a prototype gate: WPT and
Test262 were not run; no conformance percentage is claimed.

### Integrator decisions (2026-09-20)

The NET-2 exchange seam is agreed in principle, pending integrator confirmation
when shell work is taken. Only a trait proposal exists in NET-1; no adapter,
cookie/cache migration or coordinator is implemented. Shell retains stores and
credentials per locked decision 6a.1; Fetch retains redirect/CORS policy.

Strict ASCII HTTP(S) stays in NET-1. Do not grow a shared URL implementation
inside this crate. The cross-lane URL layer must unify amazing-net/url.rs and
other consumers: canonical host/IDNA/IP/default ports, relative Location
resolution, special-URL parsing, dot segments and percent encoding, credential
presence, fragments, origin computation, and trustworthy-origin classification.
It must preserve enough input state to reject credential-bearing CORS targets
instead of silently discarding userinfo. Its public types should replace the
prototype HttpUrl at the integration boundary without separate origin parsing.

A PSL provider must consume normalized hosts, distinguish IPs and nonregistrable
hosts, implement exact/wildcard/exception rules and the agreed private-domain
policy, and expose a pinned data revision with update/reproducibility policy.
A pinned PSL is a new external data dependency: selecting/adopting it is a
project-level decision the integrator must take to the user. NET-1 adds no PSL
data or external package.

Final lane gate, twice on win-001 with unchanged source:
`cargo test -p amazing-fetch -p amazing-net -p amazing-net-tls --locked`.

| Suite | Before | After, run 1 | After, run 2 |
|---|---:|---:|---:|
| amazing-fetch policy tests | Skeleton; not measured | 39 passed | 39 passed |
| amazing-net | 65 passed | 65 passed | 65 passed |
| amazing-net-tls | 2 passed | 2 passed | 2 passed |

Both final runs: 0 failed, 0 ignored. Gate is native Rust policy tests, not WPT;
WPT revision/selection/counts and Test262 counts are not applicable (not run).
SHA-256 over sorted `sha256  relative-path\n` entries for all eight Fetch `.rs`
files: `0d0e3b6b1225f5d8f55560ef5bb2d5205694b5d58077bd817be6beb443faeb17`.
Per-file hashes captured from Windows match local bytes. Logs and hash manifest
are in `.agents/net/{baseline,gate-1,gate-2}.txt` and `source-hashes.txt`.

Full Windows workspace gate: `cargo test --workspace --locked` exited 0; 764 passed, 0 failed, 3 ignored (sum of 47 test-target/doc-test summaries).
The isolated GPUI shell is excluded by the root workspace manifest.

## NET-2 — Additive shared WHATWG URL layer

**Legacy `amazing_net::Url` remains the live engine path.** The corpus results
below measure the new `ParsedUrl` API in isolation, not production page loading.
The integrator approved this boundary on 2026-09-20; neither legacy `Url` nor
Fetch `HttpUrl::parse` changes behavior in this ticket.

### Public surface and implementation

`amazing_net::ParsedUrl` / `amazing_net::url::ParsedUrl` is an owned URL record
with private fields and no unchecked/default constructor or mutation API.
`parse`, `parse_with_base`, `resolve`, and `parse_with_idna` construct validated
records; read-only getters expose encoded components. Null query/fragment and
present-but-empty query/fragment are distinct. Host absence, an empty host,
segmented path and opaque path are also distinct. `Port` is an optional u16
(non-default only); `effective_port` supplies the scheme default when needed.

The parser follows the URL Standard's named basic-parser states directly:
scheme start/scheme/no scheme, relative/authority variants, host/port, file
variants, path start/path/opaque path, query and fragment. Recoverable syntax
conditions continue in the specified state; fatal errors return typed
`ParseError`. No validation-diagnostic stream is exposed yet. Parsing takes
Rust scalar-value strings and uses UTF-8; WebIDL USVString conversion belongs at
the binding boundary. Setter state overrides, legacy query encodings, and
URLSearchParams are outside this construction API.

`Host` distinguishes domain, IPv4, IPv6, opaque and empty forms. Special hosts
percent-decode before domain processing. ASCII domains lowercase without DNS
label heuristics. IPv4 uses the standard radix/short-form and range rules;
IPv6 uses std's validated address parser plus a WHATWG hex-only serializer
(first longest zero sequence, never dotted mapped-address output). Non-special
opaque hosts preserve case and percent-encode UTF-8 without IDNA.

Percent encoding exposes the C0-control, fragment, query, special-query, path,
userinfo and component sets. Path processing handles encoded dot segments and
platform-independent Windows drive letters; serialization disambiguates a
hostless path beginning `//` with `/.`.

`UrlOrigin::Tuple(TupleOrigin)` projects normalized scheme/host/port.
`UrlOrigin::Opaque` instructs the embedding to allocate an opaque security
origin; it deliberately does not implement equality. It is not a shared `null`
identity. File URLs receive opaque origins; blob origins derive only from an
HTTP(S) inner URL. `TupleOrigin` has private fields and supports equality/hash.

Sources: https://url.spec.whatwg.org/ (living standard dated 2026-09-10),
especially basic URL parsing, host parsing, serialization, and origin sections.
The parser does not select a PSL or introduce dependencies.

### Unicode/IDNA seam (NET-18)

`IdnaProvider::domain_to_ascii` receives the UTF-8-decoded, percent-decoded
Unicode special host. The bundled `Idna` is the default provider; `AsciiOnly`
remains available to measure the old unsupported baseline. Unicode characters
in paths/query/fragments/userinfo and opaque hosts do not need IDNA. ASCII
`xn--` labels remain ASCII under the URL Standard's non-strict compatibility
branch, even when strict Unicode validation would diagnose the label.

The provider implements version-pinned Unicode 17.0 UTS #46 mapping, NFC,
Punycode, label validation, CheckBidi and CheckJoiners. It uses nontransitional
processing, with CheckHyphens, UseSTD3ASCIIRules and VerifyDnsLength false.
The full domain-parser diagnostic path still needs strict ToASCII results with
those three flags true; this API does not expose validation diagnostics.
The URL layer checks ASCII output, forbidden domain code points and IP
classification after the provider.

Sources and hashes are in `crates/amazing-net/tests/wpt/idna/PROVENANCE.md`.
Generated table source is 400,566 bytes across 2,702 lines, with 9,262
binary-searched mapping spans. Unicode 18.0 assigned U+3E8AC, which the pinned
WPT revision rejects; 17.0 is the fixed data version for this gate. No
third-party core crate is used.

### Concrete adoption order for a future brief

All steps are deferred; integrator coordinates consumer owners. NET-3 must define
whether compatibility wrappers preserve old acceptance or intentionally broaden
it, and gate each semantic change separately.

| Order | Consumer and adoption | Compatibility work / behavior that changes |
|---|---|---|
| 1 | `amazing-net/src/http.rs` accepts a typed HTTP-only view derived from ParsedUrl; prepare a single-hop transport entry point. | Replace public `.scheme/.host/.port/.path` reads with getters; require HTTP(S), host and effective port; produce origin-form targets, omit fragments/userinfo from wire targets. New parser accepts schemes the transport cannot load, so parsing success is not transport authorization. |
| 2 | `amazing-net/src/fetch.rs` uses ParsedUrl for initial URL and Location resolution; keep a compatibility entry point for existing callers until integration. | `NetError::Url(UrlError)` needs explicit ParseError mapping or a new variant. Preserve bare filesystem-path routing; define safe file-to-path/UNC handling instead of silently ignoring remote file hosts. Preserve redirect policy outside single-hop I/O. Canonical root `/`, empty `?`, encoded dot segments, backslashes, IPv4 spellings and userinfo now differ from legacy. Existing behavior tests require intentional updates, not deletion. |
| 3 | `amazing-fetch/src/origin.rs` wraps/adopts ParsedUrl inside `HttpUrl`, maps TupleOrigin into existing Origin identity; `model.rs`, `redirect.rs`, `referrer.rs` consume the wrapper. | Keep NET-1 method signatures. Strict parse currently rejects recovery forms, userinfo and legacy IPv4: broadening acceptance is a behavior change even without a Rust API change. Add an explicit validated-conversion entry point first; preserve credential presence until CORS rejects it. Opaque identity must remain unique. Fragment inheritance needs a validated update method, not mutable fields. Referrer stripping must remove userinfo as well as fragment. |
| 4 | GPUI `session_state.rs`: BrowserSession::fetch, cookie_header, store_cookie, and cache keys adopt the exact same parsed request supplied by Fetch. | Cookie domains/paths and cache keys currently use legacy host/path and raw input. Canonicalizing can merge old keys; retain embedding ownership and design persisted-data migration. Do not infer PSL/SameSite policy from URL parsing. The single-hop FetchPlan/Site seam remains a proposal pending shell confirmation. |
| 5 | GPUI `dom_runtime.rs`: resolve/fetch host hooks and `storage_origin` adopt ParsedUrl and tuple/opaque origin handling. | Current storage_origin reconstructs scheme/host/port and falls back to arbitrary input. Canonical tuples can change persisted storage namespaces; opaque URLs must not share serialized `null` as a key. DOM host API behavior and security identity need integration tests. |
| 6 | GPUI `engine_viewport.rs::resolve_resource_address` and link navigation in `main.rs` adopt ParsedUrl::resolve; feed session/Fetch with the same record. | Stylesheets, scripts, images and navigation acquire WHATWG relative recovery and serialization. Retain explicit local-file fallback separately; do not silently treat parser failure or unsupported IDNA as a filesystem path. Update URL display versus navigation serialization expectations. |
| 7 | Driver `crates/amazing/src/main.rs` resource-resolution helpers and fallback `gui.rs` navigation/resource loading adopt the shared API. | Same relative/file/serialization changes as GPUI; legacy GUI remains diagnostic. CLI output and fixture expectations may gain root slashes, empty queries, normalized hosts or encoded paths. Integrator-owned files cannot be changed by net lane. |
| 8 | Deprecate/remove public-field legacy Url and free resolve only after a repository-wide consumer audit and coordinated release. | This is a Rust API break for public-field construction/access and UrlError matches; requires integrator sign-off. Only then can production conformance claims cite the new parser's gates. |

### NET-2 corpus scope and reproduction

Pinned WPT revision: `c48d58747e1f211527fb695fd60548a997fae617`.
Selection: every object record in `url/resources/urltestdata.json` (896 objects;
comment strings are not cases). Vendored data retains object order and original
array-index identifiers. `tests/wpt/PROVENANCE.md` records upstream raw SHA-256
and license. JSON is whitespace-normalized to one record per line to respect
the repository's 4000-line limit. No cases are selected based on parser output.

`tests/wpt/generate.py` converts pinned data to Rust constants without executing
tests. It performs USVString replacement for unpaired surrogates, matching the
upstream URL constructor boundary. `cargo run -p amazing-net --example url_wpt
--locked` executes on win-001 only. It checks failure or href, origin (when
specified), protocol, username, password, host, hostname, port, pathname, search
and hash. `searchParams` expectations and `relativeTo` extra variant expansion
are not exercised: this is the explicit input/base constructor selection, not
all URL APIs or all WPT url/ tests. Unsupported counts come only from a distinct
IdnaRequired parse result, are listed by original case index, and never count
as passes even when upstream expects failure. The runner exits nonzero for any
mismatch and prints pass/fail/timeout/unsupported totals.

### Measured NET-2 gate

Both final win-001 runs at WPT revision
`c48d58747e1f211527fb695fd60548a997fae617`, on the 896-object input/base selection:

| Run | Pass | Fail | Timeout | Unsupported |
|---|---:|---:|---:|---:|
| 1 | 864 | 0 | 0 | 32 (IDNA) |
| 2 | 864 | 0 | 0 | 32 (IDNA) |

The normalized stdout SHA-256 is identical for both runs:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
Sorted per-file hash-manifest SHA-256 (source, tests, corpus, fixture generator):
`9f489b47886842edcb1a6cf8f061ba25780d3d818d9d68b8d87fb152c4ebd2f0`.
Hashes matched the local tree before and the Windows tree before/after both
runs. AppleDouble `._*` metadata is excluded; it is not compiled source.
Evidence is `.agents/net/net2-wpt-run-{1,2}.{stdout,stderr}` and
`net2-source-hashes.txt`.

Existing lane gates: amazing-net **65 passed**, TLS adapter **2 passed**,
amazing-fetch **39 passed** (all unchanged). Added **13 passing** native tests
for the URL API, percent sets, IP validation, provider boundary, recovery,
relative/file/opaque paths, serialization and origins. Total **119 passed**,
**0 failed**, **0 ignored** in the final lane run. No baseline WPT result exists
for ParsedUrl: the API/runner are new; legacy Url was deliberately not scored as
though it were the replacement parser. Test262 was not run or claimed.

Full Windows workspace gate: `cargo test --workspace --locked` exited 0; **777 passed, 0 failed, 3 ignored** across 48 test-target/doc-test summaries. The root workspace excludes the standalone GPUI shell.
Ignored tests are external HTTPS and two real-GPU probes; none added by NET-2.

## NET-3 — Adopt the shared parser in transport and Fetch policy

### Production boundary and unresolved integration hazard

`amazing-net::fetch` now parses URL input and resolves HTTP Location using
`ParsedUrl`. Its HTTP/HTTPS transport uses canonical host/port and origin-form
path/query from that record. File URL loading uses the same parser with an
explicit local-file conversion policy. Bare filesystem paths retain their
separate loading route. The public loading API and NetError variants remain
compatible; ParseError maps to NetError::Url(UrlError::InvalidUrl).

`amazing-fetch::HttpUrl` now wraps ParsedUrl. Origin equality and CORS origin
serialization derive from its TupleOrigin; referrers strip userinfo and
fragments. `HttpUrl::resolve` and `redirect_location` resolve raw Location
values through the shared parser, preserving absent versus empty fragments.
Credential-bearing redirect targets remain represented until CORS policy checks
reject them where required. This is the production implementation of the
policy crate, **not a newly connected live Fetch dispatcher**: shell orchestration,
preflight exchange, cookies and cache are still outside this ticket.

**Correctness hazard until shell adoption: shell URL and origin decisions can
differ from the now-canonical transport.** The public-field legacy Url and free
resolve retain their previous semantics for GPUI `session_state.rs` cookie and
cache keys, `dom_runtime.rs` storage origins/resource resolution,
`engine_viewport.rs`, shell `main.rs` navigation, and driver `main.rs`/`gui.rs`.
They are documented as compatibility-only; no shell or driver files changed.
The integrator explicitly owns this migration using the NET-2 table. Persisted
cookie, cache and storage keys under local app data need a deliberate migration
plan before canonicalization changes their identity. This ticket does not
establish whole-browser URL/origin consistency.

### Intentional behavior changes and bounded scope

- The original 39 Fetch policy tests remain, with one explicitly renamed and
  updated: `ambiguous_urls_fail_closed` becomes
  `url_parser_recovery_replaces_the_strict_ascii_shortcut`. Userinfo, short/radix
  IPv4, backslashes, embedded CR/LF and percent-encoded hosts now follow WHATWG
  canonicalization rather than being rejected. Unicode paths encode as UTF-8.
  Non-HTTP(S) Fetch URLs, Unicode special hosts without IDNA and invalid ports
  still reject. Five additional policy tests cover adoption and its boundaries.
- Transport canonicalizes encoded dot segments, host spellings, default ports,
  Unicode paths and empty queries. Fragments/userinfo never enter the HTTP
  request target. Redirects inherit a missing fragment, retain an explicit empty
  fragment, and cannot switch a network load into file or another unsupported
  scheme. Userinfo does not automatically generate HTTP authentication.
- File URLs normalize localhost and drive letters, percent-decode valid UTF-8,
  and reject remote hosts, NUL, invalid UTF-8 and encoded filesystem separators.
  Windows conversion also rejects UNC/device-shaped paths; bare native paths
  preserve the existing separate route. File parsing alone does not authorize
  remote file access.
- Legacy shim tests preserve legacy semantics. IDNA/Unicode tables, PSL data,
  parser state-machine changes, cookie/cache migration, arbitrary-method
  single-hop Fetch transport and shell dispatcher integration remain deferred.

### Validation

NET-2 baseline: 39 Fetch policy + 65 net + 13 URL API + 2 TLS = 119 passing
native tests; pinned constructor corpus 864 pass / 0 fail / 32 unsupported.
NET-3 final win-001 lane gate: **132 passed / 0 failed / 0 ignored**:
44 Fetch policy (39 retained + 5 new), 65 existing net, 13 URL API,
8 new transport integration, 2 TLS. All builds/tests ran on win-001 with
CARGO_TARGET_DIR=C:/Users/server/target/net and four build jobs.

Pinned corpus revision/selection is unchanged from NET-2. Both runs measured
**864 pass / 0 fail / 0 timeout / 32 IDNA unsupported**, out of 896 objects.
Both normalized stdout SHA-256 values:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
Sorted 30-file lane source manifest SHA-256:
`7b4b1ba8f21df26fb17a3e7f2c4081ed987d93a3b7844da746af5e88c507a012`.
Local and Windows source hashes matched before/after the two corpus runs.

Full Windows workspace: `cargo test --workspace --locked` exited 0;
**857 passed / 0 failed / 3 ignored**, across 51 test-target/doc-test summaries.
GPUI's isolated manifest is outside this gate. The published NET-2 workspace
count was 777; the refreshed sibling tree also contributes to the increase,
so only the 13 new lane tests are attributed to NET-3. No Test262 or Fetch WPT
measurement is claimed. Evidence: `.agents/net/net3-{unit-final,workspace}.txt`,
`net3-wpt-{1,2}.{stdout,stderr}`, `net3-hashes-{before,after}.txt`, and
`net3-source-hashes.txt` in the integrator checkout.

Rebased on origin/dev `2960422` (only AGENTS.md differs from the integrator's
`26c64de` refresh), then re-synced owned source after that refresh. No sibling
source, root manifest, lockfile, shell or driver changes. NET-3 ends here.

## NET-4 — Bounded HTTP content coding

### Design and behavior

The std-only core now implements DEFLATE stored/fixed/dynamic blocks, canonical
Huffman decoding, overlapping length/distance copies with a 32 KiB window,
gzip framing (optional header fields/FHCRC, concatenated members, CRC32/ISIZE),
and zlib framing (CMF/FLG/window/Adler32). Every gzip member resets history but
shares the caller's total output budget. HTTP `deflate` means zlib-wrapped
DEFLATE; malformed raw-deflate servers do not get a speculative fallback.
Preset dictionaries reject. No new dependency or parser provider is used.

Source specifications: [RFC 1951](https://www.rfc-editor.org/rfc/rfc1951),
[RFC 1952](https://www.rfc-editor.org/rfc/rfc1952),
[RFC 1950](https://www.rfc-editor.org/rfc/rfc1950), and
[HTTP/1.1 framing, RFC 9112 §6–7](https://www.rfc-editor.org/rfc/rfc9112).

The transport advertises `Accept-Encoding: gzip, deflate` and suppresses
embedding-supplied Accept-Encoding overrides so it never advertises `br`.
It also accepts the gzip alias `x-gzip`. Response processing validates encoded
wire framing, removes transfer codings in reverse order, then content codings
in reverse order. Repeated coding headers preserve order. Original headers
remain wire metadata; `Response.body` is decoded, so wire Content-Length must
not be compared with its length, nor Content-Encoding applied a second time.
The Response field documentation records this integration contract.

Content-Length must exactly match encoded bytes. This intentionally changes
`content_length_truncates_overlong_stream_data` into
`content_length_rejects_overlong_stream_data`. Equal repeated/comma-separated
lengths are accepted; conflicting/malformed lengths reject. Transfer-Encoding
plus Content-Length rejects. Chunked may appear once and last; complete trailer
termination is required even for an unclean TLS EOF. EOF-delimited transfer
coding needs a clean TLS close. Extra bytes after chunk framing, unknown codings,
wrong-header codings, checksum mismatches and truncated streams fail closed.
204/304 metadata is not decompressed as a body. Informational responses/upgrades
remain unsupported and now fail explicitly instead of being returned as final.

### Allocation and work bounds

`FetchOptions.max_response_bytes` is the decoded output ceiling, capped at
32 MiB, and applies to every decoding layer. Tiny decoded limits still allow
encoded header/trailer overhead: wire reads retain the limit + 64 KiB bound;
encoded bodies are independently capped at 32 MiB. Each literal, stored block,
and back-reference checks remaining output budget before allocation or copying.
Growth is geometric but clamped to the caller's ceiling and uses fallible exact
reservation; gzip ISIZE is never trusted for allocation. Huffman tables and
code-length arrays have fixed sizes. Uncompressed framing checks the smaller
caller limit before copying. At most eight total coding layers are accepted.

The input and decoded buffers coexist because the transport is buffered; this
is a per-buffer bound, not a claim that total peak memory equals the decoded
limit. Stacked encodings whose intermediate output exceeds the caller limit
reject even if the final representation would fit. DEFLATE checks declared
zlib window limits; no back-reference crosses a gzip member boundary.

### Brotli evaluation — proposal only, requires project approval

The existing workspace lock pins `brotli-decompressor` **5.0.3** (checksum
`3a32acac15fe1967bc3986b2a6347dffc965602354ea6f450ad07e8bfd253583`),
`alloc-no-stdlib` **2.0.4**, and `alloc-stdlib` **0.2.4**. Text provider
`Cargo.toml` already directly pins `=5.0.3` for its WOFF2 callback (so this is
stronger than merely an incidental transitive version). That does **not**
authorize use for network bodies. No dependency or manifest is changed here,
and `br` is neither advertised nor accepted.

A future vetted adapter would need:

1. A project-level ADR approving an isolated `amazing-net-brotli` exception,
   a manifest owned by the integrator, exact version/feature/lockfile pins, and
   an owned byte/error boundary with no provider types in engine APIs. Retain
   BSD-3-Clause/MIT licensing notices from the crate; review the bundled
   dictionary/source notices. Pin the current known 5.0.3 initially only after
   a fresh security review, not a floating upgrade to the repository's head.
2. A bounded streaming output sink plus a separate scratch/history allocation
   budget. The high-level text-provider BoundedWriter pattern controls output,
   but does not by itself cap decoder ring buffers and tables. Inspected
   5.0.3 `BrotliState::new` enables the large-window extension, while
   `new_strict` disables it: use the strict state for standard HTTP Brotli.
   Use the low-level stream API with bounded allocators (u8/u32/HuffmanCode),
   establish how allocator exhaustion becomes a closed error, and test that
   path before enabling the coding. Panic isolation does not catch OOM aborts.
3. Pin safe feature settings: no `unsafe`, `seccomp`, or `ffi-api`; audit Cargo
   feature unification with the text provider's default `std`. The default
   std allocator helper is already locked, but an explicit budget allocator
   still needs implementation and review. Require full stream consumption,
   reject truncation/trailing garbage, and retain caller limits at each layer.
4. Differential fixtures against a pinned reference encoder/decoder, malformed
   and expansion-bomb cases, scratch exhaustion, chunked/content-coding order,
   TLS framing checks, and a repeatable Windows gate. Measure throughput and
   peak scratch/output memory before advertising `br`.

Cost is a separate adapter/integration/review ticket, not just adding one
function call. Decoder algorithms and the static dictionary can be reused;
allocation-budget handling, dependency approval and validation are the main
new work. No delivery-time or performance estimate is claimed without a spike.
Version details above come from the local locked 5.0.3 manifest/source; the
[upstream project](https://github.com/dropbox/rust-brotli-decompressor) documents
its allocator design and default/safe feature choices. Existing text-provider
approval remains limited to fonts.

### Final measured evidence and artifact freshness

The integrator found the old refresh preserved archive mtimes and could reuse
stale first-party rlibs even with byte-identical source. All earlier NET-4
Windows runs are superseded/suspect and are **not final evidence**. Source hashes
alone do not prove artifact freshness.

Fresh final gates use integrator-refreshed **d8437f3**, Cargo.lock **3FEAF813**.
The lane rebased onto that shared commit and re-synced owned files using
`tar -xmf` (current extraction timestamps). Before **each** workspace run,
`cargo clean -p ...` cleaned all 18 first-party workspace packages;
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`, and
`CARGO_TARGET_DIR=C:/Users/server/target/net`. Both build logs explicitly show
all 18 first-party packages compiling. Third-party dependencies remained cached.
A subsequent integrator refresh superseded the initial valid 57f0e69 gates.
The final runs below were repeated on current dev d8437f3 after rebasing and
re-syncing. Earlier 57f0e69 evidence is archived separately.

| Gate | Fresh run 1 | Fresh run 2 |
|---|---:|---:|
| amazing-net unit tests | 90 passed | 90 passed |
| amazing-net URL API | 13 passed | 13 passed |
| amazing-net transport URL integration | 8 passed | 8 passed |
| amazing-fetch policy (unchanged) | 44 passed | 44 passed |
| amazing-net-tls (unchanged) | 2 passed | 2 passed |
| Lane total | 157 passed / 0 failed / 0 ignored | 157 passed / 0 failed / 0 ignored |
| Full workspace | 1017 passed / 0 failed / 3 ignored | 1017 passed / 0 failed / 3 ignored |

Workspace gates each have 52 target/doc-test summaries and exit 0; the standalone
GPUI manifest is excluded. The lane adds 25 tests (17 decoder/boundary + 8 HTTP)
to NET-3's reported 132-test set. The integrator independently reported a clean baseline of 992 passed / 0 failed /
3 ignored on the same merged head; this lane did not rerun that unmodified baseline.

Fixtures in `tests/coding/` are independently encoded by Python's zlib, with
57 deterministic raw/zlib/gzip cases and a separate stacked-coding fixture.
Generation is data preparation only; decoder and HTTP tests execute on win-001.
Tests compare exact bytes, exercise stored/fixed/dynamic blocks, the maximum
32768-byte distance and cross-block history, truncate representative streams
at every byte, corrupt checksums/headers, and verify refusal before capacity
changes. Loopback HTTP tests verify advertising, gzip/zlib/chunked decoding,
checksum failure and caller limits. These are native tests, not Fetch WPT.

The unchanged pinned URL constructor selection was also rerun twice:
**864 pass / 0 fail / 0 timeout / 32 IDNA unsupported** out of 896 objects,
revision `c48d58747e1f211527fb695fd60548a997fae617`.
Both normalized stdout SHA-256 values:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.

All 229 tracked workspace/input files matched the Mac before and after both
gates. Sorted source manifest SHA-256:
`6ec570f7c222c151bbe28f18c2750a9e939fc5d0032b2829ac7f64a416918983`.
Evidence in the integrator checkout `.agents/net/`: `net4-clean-packages.txt`,
`net4-clean-{1,2}.txt`, `net4-workspace-{1,2}.txt`, `net4-lane-{1,2}.txt`,
`net4-wpt-{1,2}.{stdout,stderr}`, `net4-local-source-hashes.txt`,
`net4-source-before.txt`, `net4-source-after-{1,2}.txt`, and `net4-results.json`.
No Test262 measurement is claimed. No shell/driver, other lane source,
manifest or dependency changes. NET-4 ends at this boundary.

## NET-5 — In-memory cookie algorithms

### API and specification boundary

`amazing_fetch::cookies` implements parsing, storage and retrieval against
[draft-ietf-httpbis-rfc6265bis-22](https://www.ietf.org/archive/id/draft-ietf-httpbis-rfc6265bis-22.html),
sections 5.1 and 5.5–5.8. This names a specific draft, not a claim that a final
RFC or all browser cookie behavior is implemented. The profile/embedding owns
CookieStore, persistence and its clock; shell persisted state and cookie dispatch
are untouched. No new dependency, PSL snapshot, Unicode table or lockfile change.

`set_cookie` consumes a byte slice so non-UTF-8 octets and percent-looking bytes
are preserved. It rejects forbidden controls anywhere, trims only SP/HTAB,
accepts unnamed cookies, applies last-valid duplicate attributes and bounds
name+value at 4096 bytes and individual attribute values at 1024. Unknown and
oversize attributes are ignored. Dates use the specified token grammar, UTC,
70/69 two-digit-year pivot, Gregorian validity and 1601 lower year bound.
Expires and Max-Age clamp at 400 days; Max-Age wins, overflow saturates safely,
and nonpositive Max-Age deletes. No clock is read internally: operations take
signed UTC epoch seconds. The caller supplies a consistent clock; expiration
is enforced on every operation and via explicit `expire`.

The store enforces Secure (HTTPS only, no localhost trust exception), HttpOnly
for HTTP versus non-HTTP APIs, case-insensitive __Secure-/__Host- prefixes,
unnamed prefix-spoof rejection, and insecure overlays of existing Secure cookies
including deletion attempts. Identity includes name/domain/host-only/path;
replacement preserves creation time. Path matching is byte-exact and never
percent-decodes. Cookie headers sort by descending path length then original
creation time, and update last-access only after the complete bounded header
can be produced. Headers are bytes; the existing UTF-8 HeaderList cannot carry
arbitrary non-UTF-8 cookie bytes without a future transport-boundary decision.

`CookieAccess` supplies the URL, current redirect-hop method, HTTP/non-HTTP API,
top-level-navigation flag and `CookieSite`. NoClient is explicit and differs
from CrossSite/opaque origins. The embedding must derive site-for-cookies from
ancestor/worker context and account for redirects and UI reload rules; handing
in the address-bar origin alone is insufficient. CookieStore compares sites
schemefully through the provider. Strict, Lax and None are distinct; None needs
Secure. The configurable lax-allowing-unsafe mode applies only to Default cookies
aged at most 120 seconds, on top-level HTTP navigation. Replacement cannot renew
that grace. Cross-site subresources/non-HTTP contexts cannot set non-None cookies;
a cross-site top-level HTTP navigation may set them. Fetch credentials policy
must still gate both cookie sending and acceptance when a dispatcher adopts this
library; this ticket does not connect that dispatcher.

### Domain provider contract and exactly what PSL adoption changes

Reuse the existing `RegistrableDomain` trait rather than introducing a competing
site definition. `registrable_domain(host)` returns normalized eTLD+1 or None;
additive `is_public_suffix(host) -> Option<bool>` distinguishes proven suffix,
proven non-suffix, and **unknown**. Old implementations continue to compile but
default to unknown for cookie authorization. A trusted implementation must use
the same version-pinned data and ICANN/PRIVATE policy for both methods, implement
exact/wildcard/exception rules, and classify IPs as non-suffixes with no registrable
domain. Matching itself requires exact equality or a dot-boundary suffix, with
IP addresses matching only exactly. A proven suffix identical to the request
host becomes host-only; any other public suffix rejects. Retrieval rechecks
non-host-only domains so a list update cannot keep sending a newly invalid
Domain cookie.

`ConservativeDomains` has no data and returns unknown: it accepts host-only
cookies but rejects **every nonempty Domain attribute**, including same-host
Domain attributes. Site equality then falls back to identical scheme+host,
never a last-two-label guess. Once PSL data is approved, replace only that
provider: proven non-suffix Domain cookies become available and sibling
subdomains can share a registrable site. Store/parser/matching algorithms do
not change, no persisted entries are rewritten automatically, and shell
integration still needs its recorded migration plan. Snapshot pinning, update
review and persistence impact remain the user-level decision described above.

Canonical cookie hosts reject malformed LDH labels and unproven A-labels.
`validated_a_label` is a separate default-deny method on the provider for a
future trusted IDNA validator; supplying PSL data alone must not authorize fake
punycode. Unicode URL hosts remain deferred by ParsedUrl. An approved ASCII PSL
provider may leave A-label validation unsupported and retain that restriction.

### Resource policy and eviction

Defaults are 3000 cookies total, 180 per exact domain field, and a 64 KiB Cookie
header. Callers may lower count limits; total cannot exceed 3000, and header
budget cannot exceed 16 MiB. Additional conservative policy rejects Set-Cookie
fields over 64 KiB and URL-derived default paths over 1024 bytes before copying.
These explicit limits can reject inputs a less restrictive UA would retain.
Expiry runs before quota enforcement. Eviction priority is non-Secure cookies
in over-quota domains, then other cookies in those domains, then all cookies;
within a priority it chooses least recent access. `end_session` removes session
cookies. Successful setting means accepted processing, not guaranteed retention
when quotas immediately evict the new cookie. No persistence import API exists.

### Evidence

Final Windows gates ran twice on origin/dev `156cc13` plus this ticket (carried
docs-only parent `866daf4`), lock prefix `3FEAF813`. Each run measured:

- Workspace: **1058 passed / 0 failed / 3 ignored**, 54 test summaries.
- Network lane: **192 passed / 0 failed / 0 ignored**: Fetch 79 (35 new cookie
  tests + 44 unchanged policy), net 111, TLS 2. NET-4 lane baseline was 157/0/0;
  all 35 added cases pass and the existing 157 remain green.
- Pinned URL WPT: **864 passed / 0 failed / 0 timeout / 32 unsupported**, 896
  objects at `c48d58747e1f211527fb695fd60548a997fae617`. Normalized stdout SHA-256
  remains `584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.

Before the gates, self-refresh used `bash scripts/refresh-lane-trees.sh net`,
then owned source was re-synced with current mtimes. Each run separately cleaned
all first-party packages with incremental compilation disabled; compile logs
prove all 18 rebuilt. All 239 workspace source/input hashes match the Mac before
and after both runs; normalized manifest SHA-256 is
`3934508c3a29e7aec10db27fa6829ad5cfa643bad9bb221e443b8792150d25d1`.
Evidence is in `.agents/net/net5-{workspace,lane,clean}-{1,2}.txt`,
`net5-wpt-{1,2}.{stdout,stderr}`, `net5-clean-packages.txt`,
`net5-local-source-hashes.txt`, `net5-source-before.txt`,
`net5-source-after-{1,2}.txt`, and `net5-results.json`.
Cookie tests are focused native Rust cases, not WPT cookie conformance
percentages. No shell/driver, other lane source, manifest or dependency changes.
NET-5 ends at this library/provider boundary.

## NET-6 — Pinned PSL and bounded network Brotli

Architecture 6c/6d and accepted ADR 0002 supersede the earlier pending decisions
above. No additional runtime IDNA approval is implied.

### PSL provider and update procedure

`amazing_fetch::psl::PublicSuffixList` implements the existing RegistrableDomain
seam with the same ICANN + PRIVATE snapshot for registrable domains and suffix
classification. `classify` separately exposes IP, invalid, public-suffix and
registrable results. It implements longest exact/wildcard matches, exception
priority, and the implicit one-label wildcard for unknown TLDs. Input must be
canonical lowercase ASCII; the URL/IDNA layer owns normalization. A-label
lookup does not grant `validated_a_label` cookie authorization. A terminal root
dot is preserved in lookup results; cookie validation retains its own stricter
host grammar. Default CookieStore remains ConservativeDomains; an embedding
opts in by passing PublicSuffixList, with no parser/store algorithm changes.

Upstream commit `645b36c9a51ff0b3718df0ba02f6570aa6921632`, original list SHA-256
`d3fba7b2934b0e1b9c958baf42bef85107b53cd0115ff7e81a7e61c7119a36d0`.
`crates/amazing-fetch/data/psl/pin.json` pins the list, MPL-2.0 licence and CC0
upstream test inputs. Lossless 3,000-line shards retain the entire list and
comments within the repository line limit. Offline `tools/normalize_psl.py`
uses RFC3492 punycode for upstream Unicode spelling, without IDNA2003 mappings
or runtime fetching. `generated.json` pins generator and normalized outputs;
Windows `--check` must regenerate identical bytes.

Review updates monthly, but ship only explicit reviewed PRs:

1. Select an exact upstream commit, obtain list/licence/fixture bytes, verify
   their hashes, and split the original list losslessly as recorded by pin.json.
2. Regenerate both sections offline. Review normalized additions, removals,
   wildcard/exception changes and generated fixture diffs; adjust the static
   rule-shard includes if the shard count changes. Retain the licence.
3. Add boundary-change cases for affected domains, child hosts, siblings and
   private tenants. Repeat native PSL/cookie and pinned URL gates on win-001.
4. Explicitly review persisted-key migration before deployment: old/new eTLD+1
   mappings, changed schemeful SameSite decisions, newly invalid Domain cookies,
   cookie quota grouping and cache/partition-key splits or merges. Retrieval
   revalidation rejects newly public Domain cookies, but is not a full migration.
5. The embedding/integrator owns profile migration or invalidation, version tags
   on persisted data, rollback behavior and clearing affected caches. Never
   merge old site partitions merely because the new list combines them; decide
   whether discarding affected state is safer. Host-only cookies and unchanged
   boundaries need a documented preservation decision. This library has no
   persistence I/O and does not silently rewrite profiles or fetch list updates.

### Brotli adapter and HTTP adoption

The integrator registered `amazing-net-brotli` and granted lane ownership in
`784dc59`. The first-party adapter forbids unsafe and pins brotli-decompressor
5.0.3; root Cargo.lock pins alloc-no-stdlib 2.0.4 and alloc-stdlib 0.2.4 and
registry checksums. Workspace feature inspection must exclude unsafe, seccomp,
ffi-api and pass-through-ffi-panics. No encoder is linked.

`decode(encoded, max_output)` borrows input and returns owned bytes or an owned
error. It uses strict RFC7932 window decoding and rejects the optional larger
window extension, incomplete streams, concatenated streams and trailing bytes.
Each output append checks remaining allowance **before** reserve/copy. A fixed
8 KiB stack chunk probes at most one byte beyond the remaining limit, rather
than expanding into a growing Vec before checking. Separate allocator objects
share a 32 MiB live scratch-request budget covering byte/word/Huffman memory;
checked multiplication and remaining-budget checks precede allocation requests.
Provider panics fail closed under workspace panic-unwind settings.

The caller output limit is not an RSS limit: decoder history/tables have a
separate cap and can exceed a small output allowance; allocator metadata and
rounding are outside requested-payload accounting. Input remains borrowed;
HTTP retains its existing independent encoded-body cap and eight-layer limit.
No output or dependency types escape on failure.

The existing amazing-net content-coding route now accepts br and applies the
caller-selected decoded ceiling to every layer, including intermediate results.
OutputLimit maps to ResponseTooLarge, truncation to IncompleteBody; all other
adapter failures reject as malformed content. Headers retain the original wire
coding metadata, as they do for gzip. Transfer-coding ordering and complete HTTP
framing are still required before an unclean TLS EOF can be accepted.

After the production decoder/HTTP gate passed, Accept-Encoding became
`gzip, deflate, br`. Native tests verify this through loopback fetches, fixed
length and chunked framing, gzip+br stacking, intermediate expansion limits,
truncation, trailing data, and TLS-EOF completion checks. User-supplied
Accept-Encoding still cannot override the controlled header.

Eighteen round-trip vectors come from Node 22.23.2 / Google Brotli 1.1.0 at
qualities 0/5/11: empty/text/Unicode/binary/repetition/incompressible input. A
nineteenth valid large-window vector must reject in strict mode. The retained
offline generator and pin.json hashes make fixtures reviewable; no encoder is
in production. Tests also truncate every prefix of small vectors, inspect output
capacity before/after bound refusal, test shared scratch/overflow/release, and
reject a 1 MiB compression bomb at a 16-byte output limit.

### Evidence

Rebased onto integrator lockfile commit `af05025`, refreshed only net with the
corrected refresh script (lock prefix `E4D68B10`), then re-synced owned files.
Earlier checkpoints and the gate overlapped by an integrator refresh were
discarded. Both final runs below used the landed lockfile and `--locked`.

| Gate | Run 1 | Run 2 |
| --- | --- | --- |
| Full workspace | 1155 passed / 0 failed / 3 ignored | 1155 / 0 / 3 |
| Network lane | 213 passed / 0 failed / 0 ignored | 213 / 0 / 0 |
| Pinned URL WPT | 864 passed / 0 failed / 0 timeout / 32 unsupported | identical |

Native lane breakdown: Fetch 87 (35 existing cookie + 44 existing policy +
8 PSL), net 114, TLS 2, Brotli 10. Historical NET-5 lane evidence was 192/0/0;
this ticket adds 8 PSL, 3 HTTP and 10 adapter tests. PSL tests cover all 10,326
rules and 78 normalized upstream fixtures; decoder tests cover 18 independent
round trips and a nineteenth valid extension that strict mode rejects. These
native cookie/provider tests are not WPT cookie conformance percentages.

Each final run separately cleaned all first-party packages with incremental
compilation disabled. Compile logs prove all 19 rebuilt. All 288 workspace
source/input hashes match the local tree before and after both runs; normalized
manifest SHA-256:
`8bb8761e8c4554ff39e2103e194e628a8078d27a3b380ff56f8de9d313557605`.
The final decision-map documentation edit is outside that source/input manifest.

The 896-object URL corpus remains pinned at
`c48d58747e1f211527fb695fd60548a997fae617`. Both normalized stdout SHA-256 values:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
Offline PSL regeneration and all Brotli fixture hashes pass on Windows. Resolved
workspace feature trees for the decoder and both allocators contain none of the
unsafe/seccomp/FFI options. The runnable URL artifact is
`C:\Users\server\target\net\debug\examples\url_wpt.exe`.

Evidence: integrator checkout `.agents/net/net6-final-evidence/`, containing
results.json, clean/workspace/lane/WPT logs for both runs, source manifests,
PSL regeneration and feature inspection. No Test262, WPT fetch/cookie percentage,
UI proof or external-page compatibility rate is claimed. No shell/persistence
migration is performed. NET-6 ends at this provider and HTTP adapter boundary.

## NET-7 — HTTP/2 over the verified TLS adapter

The integrator's NET-7 brief adds HTTP/2; integrator decision 6 update `11dd8a8`
widens ALPN without a new dependency or a certificate/server-name verification
exception. The earlier PSL and Brotli exceptions were user-approved separately.
Production TLS prefers `h2`, falls back to `http/1.1` when selected or when ALPN
is absent, and retains Mozilla roots. The legacy byte-exchange API explicitly
advertises only HTTP/1.1. No rustls types cross the adapter boundary.

The std-only core implements RFC 9113 framing and a hand-written RFC 7541 HPACK
codec. Published Appendix C vectors anchor integer, literal, Huffman and dynamic
index behavior; Appendix A/B numerical tables have a pinned source digest.
Decoder accepts all literal forms and Huffman strings, rejects EOS/padding,
overflow, invalid indexes and late/oversized table updates. Acknowledging a
smaller advertised table limit requires the peer's HPACK size update before
its next fields. Response TE is rejected; request TE permits only trailers.
Encoder uses indexed fields and plain literals, inserts/evicts dynamic entries
and emits minimum/final
table size updates; sensitive cookie/authorization fields are never indexed.
Outbound Huffman encoding is optional and not implemented.

`http2::Client<S: Read + Write>` owns a connection, starts several streams, polls
incoming frames and retrieves replies independently. Upload chunks rotate across
streams subject to both connection and stream credit; SETTINGS window changes
can leave negative stream credit. DATA padding consumes credit. Closed-stream
DATA still counts at connection level, and closed-stream HEADERS still decode to
maintain HPACK state, as allowed by RFC 9113 section 5.1. No unbounded tombstone
set is retained. Continuations cannot interleave. Informational responses,
trailers, content-length and bodyless response rules are checked. RST_STREAM
isolates failure; GOAWAY stops new streams and identifies unprocessed requests
without automatically replaying them. Fatal errors send a best-effort GOAWAY
and drop the owned transport. A complete response survives NO_ERROR upload reset.

| Resource | Hard ceiling / default |
| --- | --- |
| Frame payload | 16 KiB, validated before payload reserve |
| Compressed header block | 64 KiB across HEADERS/CONTINUATION |
| Decoded header list | 64 KiB including RFC per-field 32-byte accounting |
| Dynamic table | 4 KiB in each direction, RFC entry accounting |
| Concurrent retained streams | 32, further reduced by peer SETTINGS |
| Encoded body | 32 MiB per stream and 32 MiB aggregate upload/response bytes |
| Incoming frames | 100,000 default, configurable up to 1,000,000 |
| Informational responses | 8 per stream |

Caller limits can lower these ceilings. Length/credit/accounting checks precede
peer-sized payload copies and reserves. Container overhead, allocator rounding,
TLS internals and a fixed Huffman trie are separate from byte-payload accounting;
these are bounded protocol structures, not a process-RSS guarantee. Decoded
content reuses the existing per-layer gzip/deflate/Brotli caller limit. HEAD
responses do not attempt to decompress the absent body.

The blocking fetch dispatcher creates one connection per fetch; multiplexing is
exposed through Client but connection pooling is not implemented. h2c, push,
prioritization, CONNECT, automatic replay, HTTP/3 and asynchronous integration
remain outside this ticket. Existing Response header strings still require UTF-8.
The transport uses existing I/O timeouts; this is not an event-loop scheduler.

Authenticated loopback tests use pinned, test-only CA/leaf/key data through an
opt-in adapter test-support feature. They exercise the production post-ALPN
fetch dispatch, Brotli body decoding and HTTP/1.1/no-ALPN fallback. Production
connect still rejects that CA. No verification bypass or new dependency exists.
`cargo run -p amazing-net --locked --example h2_fetch -- https://nghttp2.org/`
is a separate public-root TLS interoperability artifact; live site sizes are not
stable conformance measurements. Deterministic fixture tests are the gate.

Standards: https://www.rfc-editor.org/rfc/rfc9113.html and
https://www.rfc-editor.org/rfc/rfc7541.html. No broad WPT fetch percentage claimed.
Final nextest, separate doctest and pinned URL measurements are recorded below.

### NET-7 final evidence

Rebased onto `11dd8a8`, refreshed net alone with the corrected script (root lock
`E4D68B10`), then re-synced owned files. All builds/tests ran on win-001 with
`CARGO_TARGET_DIR=C:\Users\server\target\net` and incremental compilation off.
Each final run cleaned and rebuilt all 19 first-party workspace packages.

| Gate | Run 1 | Run 2 |
| --- | --- | --- |
| Workspace nextest | 1239 passed / 0 failed / 3 skipped | identical |
| Workspace doctests, separate `cargo test --doc` | 2 passed / 0 failed / 0 ignored | identical |
| Lane nextest | 244 passed / 0 failed / 0 skipped | identical |
| Lane doctests, separate `cargo test --doc` | 0 tests | identical |
| Pinned URL WPT | 864 passed / 0 failed / 0 timeout / 32 unsupported | identical |

The 213-test lane baseline is preserved: net now has 143 tests (114 + 29 HTTP/2),
TLS 4 (2 + 2), Fetch 87 and Brotli 10. Focused tests include RFC HPACK vectors,
malformed frames, allocation/stream limits, multiplexing, independent credit,
window shrink/overflow, GOAWAY/RST, late closed frames, transport teardown,
HPACK acknowledgement, TE validation, HEAD and verified TLS protocol dispatch.
The authenticated h2/fallback fixture also passed five explicit repeated runs.

The second workspace nextest run reported **six LEAK notices** in existing
`amazing::phase2_style` and `amazing::phase5_flex_grid` integration tests; all
assertions passed and nextest returned success. No NET-7 test received a notice.
Run 1 had none. This is recorded and routed to the integrator, not attributed to
an established cause or silently suppressed. The evidence parser was corrected
to handle `passed (6 leaky), 3 skipped`; existing successful logs were reconciled
on Windows without changing/re-running the tested source. Per-run notices are
retained in `runner-notices.json`.

An earlier, superseded lane run caught Windows TCP-reset timing during fixture
teardown. The fixture now waits for the client's final connection WINDOW_UPDATE
before closing; no sleep or automatic test retry masks the issue. Neither that
run nor pre-review source counts form part of the final evidence above.

All 367 source/input hashes match locally and on Windows before/after both final
runs. Manifest SHA-256:
`5799d5bc8736fb7015623fbf318b7a514eb0f369d30ac44e95e811af7af4652b`.
This final decision-map-only edit is outside the source/input manifest.
URL corpus revision `c48d58747e1f211527fb695fd60548a997fae617`, 896 objects,
normalized stdout SHA-256 in both runs:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
TLS/Brotli fixture hashes, PSL regeneration and decoder feature checks passed.

Public-root TLS interoperability against `https://nghttp2.org/` returned
`protocol=h2 status=200 body_bytes=6324 headers=14 trailers=0`, including after
both clean gates. The live body size is one measured observation, not a pinned
corpus. Runnable artifacts in `C:\Users\server\target\net\debug\examples\`:
`h2_fetch.exe` and `url_wpt.exe`.

Canonical evidence: integrator checkout `.agents/net/net7-final-evidence/`
(results, both nextest/doc/lane/URL logs, source manifests, fixture/feature
checks, runner notices and finalization). Recipe/input files: `net7_gate.py`,
`net7-inputs.json`; `net7_finalize.py` records the summary-parser reconciliation
and final public fetch/hash audit. Superseded logs are retained separately.
NET-7 stops at this protocol/transport boundary; no next ticket was started.

## NET-8 — RFC6455 WebSocket client

The bounded blocking `amazing_net::websocket` client connects over `ws` and
verified `wss` with an HTTP/1.1 Upgrade. The existing TLS adapter supplies
caller-buffer CSPRNG bytes to both schemes for the 16-byte handshake nonce and
a new 32-bit mask for every outgoing frame. Its `connect_http1` path keeps wss
on HTTP/1.1 ALPN. Core handshake, SHA-1 Accept challenge, base64, framing,
fragment assembly and incremental UTF-8 validation are std-only. The SHA-1
routine is fixed-input challenge processing, not an authentication API.

The client validates status, Upgrade, Connection, Accept and selected offered
subprotocol, rejects extensions, and leaves the first coalesced frame unread.
Frames reject RSV bits, unknown opcodes, server masks, nonminimal lengths and
invalid control shapes. Fragmented messages retain type and size accounting;
Ping/Pong can interleave. Invalid UTF-8 fails with close 1007, resource refusal
with 1009 and malformed protocol data with 1002 when a close can be sent.
Peer, local and simultaneous close paths send at most one close frame and expose
an explicit `finish_close` wait for server transport EOF. Failed connections
drop the owned transport.

| Resource | Default / hard ceiling |
| --- | --- |
| Handshake response | 16 KiB; 128 fields |
| Input URL | 8 KiB |
| Offered protocols | 32 entries; 256 bytes each |
| Frame payload | 1 MiB / 16 MiB |
| Assembled message | 8 MiB / 32 MiB |
| Retained Ping/Pong notifications | 32 |
| Control payload | 125 bytes |
| Lifetime incoming frames | 100,000 / 1,000,000 |

Peer-sized frame and message checks precede payload reserve/read. The fixed
control buffer and notification cap bound retained control data. These are
payload ceilings, not a process RSS guarantee. One `poll` consumes one frame;
there is no reader thread or unbounded event queue. No RFC8441 extended CONNECT,
compression, redirects, cookies, proxying or automatic retries are included.
Browser embeddings must supply initiating Origin and enforce CSP, mixed-content
and credential policy themselves; this module is not a browser WebSocket binding.

### NET-8 evidence

Rebased `lane/net` onto dev `ffb143b`, including shared coordination fix
`7811124`. Refreshed only the net Windows tree via the primary checkout script,
then re-synced lane-owned files. A shared REFRESH-HOLD protected the final run.
All builds and tests ran on win-001 with `CARGO_TARGET_DIR=C:\Users\server\target\net`,
`CARGO_INCREMENTAL=0`, `--locked`, and the primary checkout's global build slot.
Pre-outage and pre-refresh results were excluded. Both final runs cleaned and
rebuilt all 20 first-party workspace packages.

| Gate | Run 1 | Run 2 |
| --- | --- | --- |
| Workspace nextest | 1289 passed / 0 failed / 3 skipped | identical |
| Workspace doctests (`cargo test --doc`) | 2 passed / 0 failed / 0 ignored | identical |
| Network lane nextest | 271 passed / 0 failed / 0 skipped | identical |
| Network lane doctests (`cargo test --doc`) | 0 tests | identical |
| Pinned URL WPT | 864 passed / 0 failed / 0 timeout / 32 unsupported | identical |

Lane native breakdown: net 170, TLS 4, Fetch 87, Brotli 10. The 27 new WebSocket
tests cover RFC handshake and masked-Hello vectors, rejected handshakes and
malformed frames, fragmentation/control interleaving, UTF-8 splits/errors,
close codes, entropy failure/per-frame calls, pre-allocation capacity/read
positions, and live loopback ws and verified wss echo. No nextest LEAK notices
appeared in either final workspace run. The pinned URL corpus remains 896
objects at revision `c48d58747e1f211527fb695fd60548a997fae617`;
normalized stdout SHA-256 was
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`
in both runs.

All 407 tracked workspace source/input hashes matched local files and stayed
unchanged before/after both runs; source manifest SHA-256:
`eb9117bb4dc196446901545b36e66ec5f29589aa6cc1740f85f45af63e66c385`.
PSL regeneration, TLS/Brotli fixture pins and decoder feature checks passed.
Canonical logs/results/source manifests: integrator checkout
`.agents/net/net8-final-evidence/`. Gate recipe and input hashes:
`.agents/net/net8_gate.py` and `.agents/net/net8-inputs.json`.

Sources: [RFC6455](https://www.rfc-editor.org/rfc/rfc6455.html) and the
[IANA WebSocket close-code registry](https://www.iana.org/assignments/websocket).
This ticket stops at the protocol/transport boundary.

## NET-9 — pull response bodies

`amazing_net::fetch_stream(input, &FetchOptions, StreamOptions)` returns final
URL, status, reason and headers after receiving the response head. Each
`StreamingResponse::next_chunk()` call advances the body and returns at most
`chunk_bytes`; dropping the response stops reads and closes its owned transport.
The buffered `fetch` API remains available. Redirect bodies are dropped before
opening the next URL. HTTP/1.1, negotiated HTTPS h2 and local files use the
same pull surface; callers drive it on the existing single thread.

| Bound | Default | Hard ceiling |
| --- | --- | --- |
| Returned chunk | 8 KiB | 64 KiB |
| Decoded bytes | `FetchOptions::max_response_bytes` (32 MiB) | 32 MiB |
| Encoded HTTP body | 32 MiB | 32 MiB |
| Decoded/encoded cumulative ratio | 1024 plus 64 KiB grace | 4096 plus 64 KiB grace |
| HTTP/1.1 response head | 64 KiB | 64 KiB |
| DEFLATE history | 32 KiB | 32 KiB |
| Brotli decoder scratch | 32 MiB | 32 MiB |

Each transfer/content coding layer has its own cumulative decoded byte cap.
The final layer checks cumulative decoded bytes against counted encoded bytes;
a one-byte probe rejects the violating byte in fixed caller storage before
delivery or body-sized allocation. DEFLATE preserves bit, block, Huffman and
history state across reads. Gzip/zlib preserve header, checksum and trailer
state; Brotli uses a resumable strict decoder with bounded scratch. HTTP/1.1
framing parses Content-Length, chunked and EOF bodies incrementally. HTTP/2
releases each DATA frame after consumption while retaining cumulative length
accounting. One-byte input and varied output-size tests compare decoded bytes
with the buffered decoders, including a real high-expansion Brotli vector.

### NET-9 evidence

The only branch touched was `lane/net`, on fixed base `2021707`. The
`2021707..origin/dev` diff in all four net-owned crates was empty. The net
Windows tree was refreshed to that base before NET-9 work and protected with
`REFRESH-HOLD` during final gates. All builds/tests ran on win-001 with
`CARGO_TARGET_DIR=C:\Users\server\target\net` and `--locked`; each full
workspace run held a shared build slot and rebuilt all 20 workspace packages.

| Gate | Run 1 | Run 2 |
| --- | --- | --- |
| Workspace nextest | 1327 passed / 0 failed / 3 skipped | identical |
| Workspace doctests | 2 passed / 0 failed | identical |
| Network lane nextest | 281 passed / 0 failed / 0 skipped | identical |
| Network lane doctests | 0 tests | identical |
| Pinned URL WPT | 864 passed / 0 failed / 0 timeout / 32 unsupported | identical |

Lane native breakdown: net 180, TLS 4, Fetch 87, Brotli 10. The previous
NET-8 lane count was 271. New coverage includes one-byte compressed input,
1/2/7/8192-byte decoded output splits, whole-body decoder parity, a real
1 MiB Brotli expansion rejected by the cumulative ratio, incremental HTTP/1.1
chunk lines, unclean TLS EOF after complete framing, and authenticated h2
DATA delivery before the next frame. The initial NET-9 checkpoint's compile
error and an h2 fixture credit-order error were corrected before these gates.

All 409 tracked workspace source/input hashes matched local files and stayed
unchanged before and after both final runs. Source manifest SHA-256:
`c8639be509339e0406b345f909885b20f4d99b8e61ab74534133988e04766990`.
Pinned URL normalized stdout SHA-256:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
PSL regeneration, TLS/Brotli fixture pins and decoder feature checks passed;
neither workspace run emitted a nextest leak notice. Canonical gate logs,
results, source manifests, recipe and input hashes are under the integrator
checkout's `.agents/net/net9-final-evidence/`, `net9_gate.py` and
`net9-inputs.json`. NET-9 stops at this transport API boundary.

## NET-10 — RFC 9111 cache policy

`amazing-fetch::cache` owns private/shared cache semantics behind a generic
`CacheStorage` interface. The default `MemoryCacheStorage` is in memory only;
the embedding supplies requests, completed responses, clock times, transport
and any persistence. `CacheStore` makes the freshness, validation, Vary,
invalidation, and admission decisions. `CacheEntry` is an owned record an
embedding can serialize without adopting a prescribed file format.

The primary key retains the shell's exact `input + "\\n" + Cookie` shape.
Vary compares the named outbound request fields, including Cookie, and `Vary:
*` is never stored. The shell can later adapt its 64-entry/16 MiB queue and
response conversion to this seam; this ticket does not edit shell code or its
`session-state-v1.txt` format. The current shell file contains no cache records,
so adopting the policy does not require changing that file's existing records.

Responses use RFC 9111 Age calculations from request/response times, Date and
Age. Shared caches prefer `s-maxage`; both modes then use `max-age`, Expires,
or a 10%-of-Date-minus-Last-Modified heuristic capped at one day. Invalid or
duplicate freshness information is stale. `no-store` prevents admission,
`no-cache` forces validation, and stale `must-revalidate` responses are never
served without validation. Shared caches reject `private` and unauthorised
responses unless an explicit shared-cache directive permits admission. ETag
and Last-Modified generate conditional fields; matching 304 metadata updates
the stored response while keeping its body. Successful unsafe methods remove
all variants of the target URI.

Default and hard ceilings mirror the shell: 64 entries, 16 MiB aggregate and
8 MiB body per entry. Cache keys are capped at 8 KiB, request/response headers
at 64 KiB each, and Vary at 32 fields. The policy checks lengths before it
copies metadata or inserts an entry. Transport remains responsible for its
own response-body allocation cap. This policy deliberately handles complete
200 GET responses; the shell integration can add other cacheable statuses and
methods as a separate measured ticket.

NET-10 final gate used fixed base `dc184d24c76005003ff7cb874bb6df16bcb34afd`
and local implementation commit `6392f42`. The net build tree held that base
plus the six changed files under `REFRESH-HOLD`. Two artifact-clean, `--locked`
Windows runs under the shared build slot produced identical results:

| Gate | NET-9 before | NET-10 run 1 | NET-10 run 2 |
|---|---:|---:|---:|
| Net lane nextest | 281 pass / 0 fail / 0 skip | 293 / 0 / 0 | 293 / 0 / 0 |
| Net lane doctests | 0 pass / 0 fail | 0 / 0 | 0 / 0 |
| Workspace nextest | 1327 pass / 0 fail / 3 skip | 1357 / 0 / 3 | 1357 / 0 / 3 |
| Workspace doctests | 2 pass / 0 fail | 2 / 0 | 2 / 0 |
| Pinned URL WPT | 864 pass / 0 fail / 0 timeout / 32 unsupported | 864 / 0 / 0 / 32 | 864 / 0 / 0 / 32 |

The 12 new focused cache tests all pass. All 20 workspace packages rebuilt in
each run. The 417 tracked source/input hashes matched the Mac worktree and
stayed stable before, between, and after runs; source manifest SHA-256 is
`e224acebfbe043c7d820df2e264d1bad2c74434b106acf7a4b457372aac99d35`.
Pinned URL stdout SHA-256 stayed
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
PSL regeneration, TLS/Brotli fixture pins, dependency feature checks, and
nextest leak-notice checks passed. Canonical logs, results, source manifests,
recipe and input hashes are in the integrator checkout's
`.agents/net/net10-final-evidence/`, `net10_gate.py`, and `net10-inputs.json`.
This ticket stops at the cache policy and storage seam; shell adoption remains
with the integrator.

## NET-11 — Fetch main-fetch blocking checks

Fixed base: `f830379` (NET-10 merge). Pinned normative snapshots used for
implementation: [Fetch Living Standard](https://github.com/whatwg/fetch/tree/357bd98924d94b81fbe8608192a2ee1f123b82f4)
at `357bd98924d94b81fbe8608192a2ee1f123b82f4` (21 September 2026;
§2.9 bad ports, §2.10 MIME blocking, §3.6 nosniff, §3.7 CORP, §4.1 main fetch),
and [Mixed Content Level 2](https://github.com/w3c/webappsec-mixed-content/tree/a4eacecdbaab867f6de126c4048662bac2ad4b4)
at `a4eacecdbaab867f6de126c4048662bac2ad4b4` (§2 authenticated URLs,
§4.1 upgrade, §4.4 request blocking, §4.5 response blocking). The current
Fetch snapshot does not contain the ORB algorithm under §3.7; that section is
CORP. ORB follows the [separate processing-model draft](https://github.com/annevk/orb/tree/61c1be761fca35288b6c90249da66ab563f1074c)
at `61c1be761fca35288b6c90249da66ab563f1074c`, read 23 September 2026.

`Request::prepare` upgrades eligible no-cors image/audio/video URLs before
checking the exact Fetch bad-port table and Mixed Content blocking. The
embedding supplies secure-context state, destination, image-set initiator,
top-level navigation state, and COEP `require-corp`; this crate cannot infer
those from a DOM. `redirect` checks a no-cors hop's CORP header before adopting
its target. `filter_response` applies internal-response checks before exposing
even an opaque result: Mixed Content, CORP using the pinned PSL, MIME type,
nosniff, and ORB. Each block has a distinct `FetchError` variant.

ORB reads at most a borrowed 1024-byte body prefix. Its full-body
JavaScript-versus-JSON parse step has no implementation here: when earlier
MIME and signature steps have not allowed a response, policy blocks it. A
streaming transport waits for 1024 bytes or EOF before calling the prefix
check. This conservative boundary requires neither whole-body buffering nor
`amazing-js`. No shell or driver adoption is claimed.

Once `fetch()` and element fetching are bound, WPT `fetch/security/` and
`fetch/api/` can measure bad ports, `fetch/cross-origin-resource-policy/`
can measure CORP/COEP, `fetch/nosniff/` can measure MIME blocking,
`fetch/orb/` and `fetch/corb/` can measure opaque blocking, and
`mixed-content/generic/` and `mixed-content/tentative/` can measure upgrades
and blocking. Those suites are not counted as passing by the crate tests in
this ticket; only the existing pinned URL WPT corpus runs as a regression gate.

NET-11 implementation commit `502769e` was tested twice on win-001 from fixed
base `f830379` with `--locked` and artifact-clean rebuilds under the shared
build slot. The standalone Fetch crate gate passed 107/0 nextest and 0
doctests before the final gates. Final results:

| Gate | NET-10 before | NET-11 run 1 | NET-11 run 2 |
|---|---:|---:|---:|
| Net lane nextest | 293 pass / 0 fail / 0 skip | 301 / 0 / 0 | 301 / 0 / 0 |
| Net lane doctests | 0 / 0 | 0 / 0 | 0 / 0 |
| Workspace nextest | 1357 pass / 0 fail / 3 skip | 1399 / 0 / 3 | 1399 / 0 / 3 |
| Workspace doctests | 2 / 0 | 2 / 0 | 2 / 0 |
| Pinned URL WPT | 864 pass / 0 fail / 0 timeout / 32 unsupported | 864 / 0 / 0 / 32 | 864 / 0 / 0 / 32 |

The before columns are NET-10's prior base, so the 42-test workspace increase
also includes intervening sibling merges; the eight new Fetch tests account
for the lane increase. All 20 workspace crates rebuilt each run. The 429
tracked source/input hashes matched the Mac worktree and remained identical
before, between, and after both passes. Source manifest SHA-256:
`09bde2087f710da2e61dde1bdaaf3629e6a7355c7522b4975342d5de109b3e10`.
Pinned URL stdout SHA-256:
`584d63e8c231193dda33dfd9e1fdfa5af350afe6ea55839cad3fdc745f24d51f`.
PSL regeneration, TLS/Brotli fixture pins, dependency feature checks and
nextest leak-notice checks passed. Logs, results, source manifests, recipe and
input hashes are in the integrator checkout's `.agents/net/net11-final-evidence/`,
`net11_gate.py`, and `net11-inputs.json`. NET-11 stops at the policy seam.

## NET-12 — CSP3 fetch checks and ORB compatibility

Fixed base: `d29e1ff`. CSP algorithms are pinned to
[W3C CSP Level 3 `b1bad631`](https://github.com/w3c/webappsec-csp/blob/b1bad631b091cdca904138d6947a16b89edfd2b4/index.bs),
particularly serialized policy lists (§2.1.1), fetch pre/post checks (§4.1.2–3),
script source checks (§6.7.1), expression matching (§6.7.2.7–12), and fetch
fallback lists (§6.8.3). The brief calls the path-after-redirect rule §6.7.2.9;
in this pinned revision it is §6.7.2.8, step 3.6.

`amazing-fetch::csp` parses enforcement and Report-Only policy lists with a
65,536-byte total header limit, 64-policy limit and 256-directive limit checked
before source allocation. First duplicate directive wins. The embedder supplies
the policy container's self-origin, destination, nonce, integrity hashes and
parser-inserted state. `Request::prepare` runs pre-request checks on each hop;
the response-security path runs post-request checks and redirect processing
runs them before following a hop. The public `CspCheck` returns typed
violations and never blocks for Report-Only. Report delivery, inline script and
style checks, and eval checks are adoption seams for DOM/JS bindings. The
WebSocket embedder can call `csp_source_matches` with its parsed ws/wss URL;
the HTTP-only `HttpUrl` Fetch path cannot carry that scheme itself.

The ORB final fallback follows [Chromium's ORB v0.1 shipping decision](https://groups.google.com/a/chromium.org/g/blink-dev/c/ScjhKz3Z6U4/m/q3nEd1lcBAAJ)
the [Chromium 138.0.7204.49 ORB source revision](https://chromium.googlesource.com/chromium/src/+/138.0.7204.49/services/network/orb/orb_impl.cc),
and the pinned [CORB sniffer source `600fb323`](https://chromium.googlesource.com/chromium/src/+/600fb3234c1b8ce8e05bcb57637c79cc11e0388e/services/network/public/cpp/cross_origin_read_blocking.cc):
it uses bounded CORB-style HTML/XML/JSON and security-prefix sniffing, allowing
plain mislabeled JavaScript after earlier MIME/media/image checks. This differs
from the [ORB draft `61c1be7`](https://github.com/annevk/orb/tree/61c1be761fca35288b6c90249da66ab563f1074c)
final full-body JavaScript-versus-JSON parse. The borrowed prefix remains capped
at 1024 bytes; no whole-body buffer is added.

Once DOM/Fetch bindings pass policy containers and element metadata through to
this crate, WPT `content-security-policy/default-src/`,
`content-security-policy/script-src/`, `content-security-policy/style-src/`,
`content-security-policy/img-src/`, `content-security-policy/connect-src/`,
`content-security-policy/font-src/`, `content-security-policy/media-src/`,
`content-security-policy/object-src/`, `content-security-policy/child-src/`,
`content-security-policy/frame-src/`, `content-security-policy/worker-src/`,
`content-security-policy/script-src-attr-elem/`,
`content-security-policy/style-src-attr-elem/`,
`content-security-policy/parsing/`, and
`content-security-policy/reporting/` become measurable. They are not counted
as passing by the native tests here. Inline/eval coverage requires the
DOM/JS adoption seam; network report delivery requires a reporting dispatcher.

## NET-13 — native Fetch API objects

Fixed base: `a858fd0`. The native object algorithms follow pinned
[WHATWG Fetch §5 (`357bd989`)](https://github.com/whatwg/fetch/blob/357bd98924d94b81fbe8608192a2ee1f123b82f4/fetch.bs).
`amazing_fetch::api` adds guarded `ApiHeaders`, typed `ApiRequest` and
`ApiResponse`, bounded `ApiBody` consumption, and `fetch_api` glue to the
existing per-hop `NetworkOrCache` policy path. A `StreamingResponse` from NET-9
implements the body pull trait. Body clones share a bounded spool and retain
independent disturbed/locked state. `fetch_api` checks an explicit abort input
between hops; cancellation inside a blocking transport exchange must be supplied
by the embedding's transport implementation. FormData and Blob conversions
return typed Unsupported variants until their DOM data models are available.

WebIDL conversion boundary: header names/values, method and statusText are
`ByteString` from UTF-16, rejecting code points above U+00FF. URL and referrer
are `USVString`, replacing unpaired surrogates. Integrity is `DOMString`.
The typed init record preserves the difference between a missing body and an
explicit null body. The native classes do not install JS constructors,
iterators, promises or `AbortSignal`; those are DOM-7/runtime adoption seams.
The current policy URL type accepts HTTP(S), so tests using `about:blank`
require the embedding's broader URL path. Existing transport headers are
string-backed; raw Latin-1 octet serialization is a lower transport seam.

The following are **ported native assertion inputs**, not WPT runs, from the
pinned [WPT revision `c48d587`](https://github.com/web-platform-tests/wpt/tree/c48d58747e1f211527fb695fd60548a997fae617/fetch/api):

| WPT source file | Native assertion area |
|---|---|
| [headers-basic.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-basic.any.js) | append, set, has, get, delete, order |
| [headers-casing.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-casing.any.js) | case-insensitive names |
| [headers-combine.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-combine.any.js) | duplicate combining and iteration |
| [headers-errors.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-errors.any.js) | invalid sequence rows, names, values and ByteString |
| [headers-forbidden-override.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-forbidden-override.any.js) | request guard |
| [headers-no-cors.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-no-cors.any.js) | no-CORS safelist |
| [headers-normalize.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-normalize.any.js) | value whitespace normalization |
| [header-values-normalize.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/header-values-normalize.any.js) | normalized request values |
| [header-values.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/header-values.any.js) | invalid/Latin-1 values |
| [header-setcookie.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/header-setcookie.any.js) | per-field Set-Cookie iteration |
| [headers-record.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-record.any.js) | typed sequence/record filling; JS Proxy order remains bindings work |
| [headers-structure.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/headers/headers-structure.any.js) | API method surface; JS property descriptors remain bindings work |
| [request-structure.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/request/request-structure.any.js) | defaults and read-only values |
| [request-bad-port.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/request/request-bad-port.any.js) | pre-network bad-port block |
| [request-error.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/request/request-error.any.js) and [request-error.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/request/request-error.js) | constructor TypeError matrix |
| [response-error.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/response/response-error.any.js) | status, statusText, null-body status |
| [response-init-001.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/response/response-init-001.any.js) | defaults and status |
| [response-init-002.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/response/response-init-002.any.js) | body type and headers |
| [response-init-contenttype.any.js](https://github.com/web-platform-tests/wpt/blob/c48d58747e1f211527fb695fd60548a997fae617/fetch/api/response/response-init-contenttype.any.js) | content type preservation |

Once bindings install these classes and the fetch method, WPT
`fetch/api/headers/`, `fetch/api/request/`, `fetch/api/response/` and
`fetch/api/basic/` can be measured. The native assertion count and two-pass
Windows gate results are recorded in `.agents/net/report.md` after gating.

## NET-14 — native XMLHttpRequest model

Fixed base: `5c92b4a`. The native XHR algorithms are pinned to
[WHATWG XHR `c3476b46`](https://github.com/whatwg/xhr/blob/c3476b46f024cfc988f47d8d82118f3bcba562e9/xhr.bs).
`amazing_fetch::xhr` exposes the five-state machine, typed exception variants
with DOMException names, guarded author headers, request-body extraction through
NET-13, timeout/credentials/responseType rules, response accessors, and typed
request/upload event batches. `send()` initiates asynchronous work; the embedder
calls `advance()` for the existing Fetch policy/transport path and
`pull_response()` for bounded response-byte/event steps. `send_blocking()` is
the explicit synchronous variant. Every event carries a generation so a DOM
dispatcher can discard a stale tail after `open()` or `abort()` re-entry.

The existing Fetch path currently returns a bounded complete body rather than
a live pull response. XHR exposes that buffered body in pull steps so event
ordering and re-entry can be bound now; live transport chunk callbacks and
mid-exchange cancellation need a later network dispatcher seam. The embedder
supplies a monotonic clock for timeout checks and implements `XhrTextDecoder`
using the existing HTML charset bridge. The native layer never dispatches DOM
events or installs JS objects. Document/Blob responses and FormData request
bodies remain typed Unsupported until their owning DOM data models land.

Pinned [WPT `xhr/` inventory](net14-wpt-inventory.md) lists every file at
revision `2f7c70099b6ecc6a1f6a821217afd79b6ff81f28`: 47 source files
contribute selected native assertions, while each excluded file has a reason.
These are assertion ports, not WPT execution. Final assertion and Windows gate
counts are recorded in `.agents/net/report.md`.

## NET-15 — cooperative network dispatcher

Fixed base: `bf085b7` (NET-14 merged). `amazing_net::dispatch::Dispatcher`
accepts HTTP(S) requests with stable ids and advances one state per request on
`step(budget, now)`: resolve, connect, retained TLS handshake, write, read head,
framed body, completion, error or cancellation. Callers supply a monotonic
`Instant`; timeout is checked before each step. The dispatcher allows one idle
HTTP/1.1 connection per origin and reuses it only after an explicitly framed
response. It emits head/body/done events, with typed admission, budget, timeout
and buffer errors. A zero step budget does no transport work. The event-loop
networking task source and DOM callbacks remain driver work.

`Resolver` and `Connector` isolate the two blocking points in the std-only
implementation. `StdResolver` uses `ToSocketAddrs`; DNS has no configured
time bound.
`StdConnector` uses `TcpStream::connect_timeout` once per step, defaulting to
three seconds, then sets the socket non-blocking. This is the integrator's
NET-15 decision in `.agents/net/decision-net15.md`; a true non-blocking
connect can replace the trait implementation without changing the dispatcher.
`amazing_net_tls::PendingTls` retains the rustls connection and socket across
`WouldBlock`; each handshake step limits encrypted socket I/O. The old blocking
TLS connect entry points loop that same state machine. Encrypted response I/O
uses bounded adapter methods; caller-facing rustls types remain private.

HTTP/1.1 is the dispatcher protocol. HTTP/2 requires a separate cooperative
connection/stream scheduler and remains on the existing blocking path for now.
The dispatcher parses framing incrementally and emits unencoded body bytes as
chunks. It requests `Accept-Encoding: identity`; if a server sends codings
anyway, the bounded NET-9 decoder runs when the encoded message completes and
decoded bytes are then emitted in step-sized chunks. This means encoded
responses do not deliver decoded bytes before the encoded body completes.
Fetch policy hops and XHR have pending-event adapters in `amazing_fetch`; the
existing blocking `fetch_api` and `Xhr::advance` continue to work. The driver
must route events, call the XHR timeout check on idle ticks, and schedule
further steps. It remains responsible for its
cookie/cache transport seam and DOM event dispatch.

The deterministic loopback tests use a non-blocking server stepped by the same
test loop as the dispatcher. They cover out-of-order completion, abort after a
body chunk, timeout, HTTP/1.1 keep-alive reuse, and typed bounds. TLS has an
isolated verified localhost fixture to exercise retained handshake state
across `WouldBlock`. The measured two-pass gate and source manifest are in
`.agents/net/report.md`.

## NET-16 — incremental decode, dispatcher HTTP/2, NetworkService

Fixed base: `9952e73d2b9241ffaab7254e195e0aaa76c43f18`. Encoded HTTP/1.1
and HTTP/2 body chunks enter the NET-9 pull decoder before the wire response
ends. The decoder emits through the caller's step budget. NET-9's gzip,
deflate and Brotli readers treat an input EOF as final and several parsing
states cannot resume after `WouldBlock`, so the dispatcher retains a bounded
encoded prefix and replays the decoder when a later chunk arrives, skipping
already emitted output. Replay work consumes the step budget. This preserves
the existing size and expansion-ratio caps and delivers early bytes, but costs
more CPU for highly fragmented compressed bodies. A genuinely resumable NET-9
decoder is a later performance improvement.

The dispatcher now offers both `h2` and HTTP/1.1 in TLS ALPN. One retained
HTTP/2 connection per origin owns an RFC 9113 client with stream-local state,
flow-control windows and reset handling. A complete-frame adapter buffers
partial TLS reads before the protocol parser sees them; queued protocol output
drains through bounded TLS writes. Requests to an origin while its first TLS
handshake is pending wait for the negotiated protocol; after `h2` selection
they start streams on the same connection. Cancel sends `RST_STREAM(CANCEL)`
and releases its local stream. The existing HTTP/1.1 keep-alive path remains.
The authenticated loopback test drives two dispatcher streams on one TLS
connection, interleaves DATA, aborts the first and completes the second.

`amazing_fetch::NetworkService` owns the dispatcher, cookie jar, private
`CacheStore`, pending Fetch policy state and client-supplied CSP/security
context. Its driver API is `start`, `step` and `abort`, with stable service ids.
It emits wire progress and policy-filtered completion events. Cookies are
chosen per hop after Fetch policy, never sent on preflight, and recomputed on
redirect. Fresh cache entries are filtered through the current client policy
before delivery; completed GET responses enter the cache. Cache validation
responses currently go to the network as an unconditional request; conditional
304 replay remains a follow-up. The existing `PendingFetchApi`, `fetch_api`
and XHR APIs continue to work.

## NET-17 — MIME records, data: URLs, 304 service revalidation

Fixed base: `55d3fc1a12f2dd226d7a0f4170dd03e2c048a965`. A MIME type is an
ordered record of lowercase type/subtype and first-wins parameter names.
Parsing and serialization implement MIME Sniffing §4, including HTTP whitespace,
quoted strings, invalid parameter omission, and duplicate parameters. Fetch
§3.5 extraction shares that parser, keeps the last valid Content-Type, and
inherits charset across equal essences. Nosniff/ORB and XHR now use the record
instead of splitting at semicolons or commas themselves.

The Fetch §6 data: URL processor runs on the serialized URL without its
fragment, percent-decodes the body, applies Infra forgiving-base64 when the
metadata has a final `;base64`, and supplies the default MIME type on parse
failure. `fetch_api` and `NetworkService` serve it without dispatching HTTP.
The local scheme returns a basic filtered response in basic and CORS request
modes; same-origin mode still rejects a cross-origin data URL. Data URL bodies
share Fetch API's 32 MiB body bound.

`NetworkService` now uses `CacheStore::lookup`'s validator headers for stale
GET entries. They go directly to the transport after Fetch policy, so a
conditional request does not alter the public request header guard or cause a
CORS preflight. On a matching 304, `CacheStore::update_304` merges metadata,
retains the body, and the service applies current policy to the stored 200
representation. A malformed or unmatched 304 becomes a transport error.

The native corpus runner measures the four pinned WPT JSON files recorded in
`crates/amazing-fetch/tests/wpt/PROVENANCE.md`. Before NET-17, the repository
had no MIME record, data URL processor, forgiving-base64 API, or runner, so the
four corpus baselines are zero implemented cases (the security code only had
an ad-hoc MIME essence check). These native results do not claim a broad
Fetch or MIME WPT percentage.

## NET-19 — cooperative WebSocket and CORS-preflight cache

Fixed base: `e0adf152ddc775053e4e30c52cdd185311e5e0ce`.

The dispatcher retains WebSocket DNS, bounded blocking connect, HTTP/1.1 TLS
handshake, upgrade request, response parsing, frames, fragmented message state,
and close state across `step_websockets` calls. All socket I/O uses the
dispatcher's byte budget and nonblocking transports. Outbound client frames
are masked, and queued bytes have a bounded buffer limit. The blocking NET-8
client remains available. `NetworkService::open_websocket` applies mixed
content, CSP `connect-src` on the original `ws`/`wss` URL, and cookies selected
for the mapped HTTP(S) handshake URL. The service returns open, text/binary
message, close, and error events. WebSocket redirect and extensions remain
unsupported, as in NET-8.

`NetworkService` now owns a Fetch §4.8 preflight cache. Entries use the
initiating origin, URL without fragment, credentials mode, method, and sorted
unsafe header names. Validated responses grant the request for the server's
max-age (default five seconds, capped at two hours); zero is not stored. A
credentialed grant can satisfy an uncredentialed request, while the reverse
cannot. The cache holds at most 256 entries and discards expired entries on
lookup and insert. Redirect hops consult it using the new URL and headers.

## NET-20 — native cookie vectors and page-sequence service reference

Fixed base: `cb3064bc248d7ea363957517474e18c757b4a850`.

The pinned WPT `cookies/` revision no longer carries a standalone
`http-state/` fixture. Its HTTP Set-Cookie vectors live in eight HTML files.
The checked-in extractor generates 192 native cases for Max-Age, Expires,
Domain, Path, attribute/name parsing, and byte-size limits. The first full
pass was 191/192: `Domain=.` was incorrectly normalized to an empty Domain
attribute and accepted as host-only. The parser now preserves the invalid
single dot. The resulting corpus passes 192/192; provenance and raw source
hashes are in `crates/amazing-fetch/tests/wpt/PROVENANCE.md`.

The `network_service_page` integration test is the driver's reference for a
single `NetworkService` session: it fetches a document, reads its CSP header,
loads a cookie-bearing script, blocks an image before dispatch, follows a
Set-Cookie redirect, validates an ETag-cached response with 304, and opens a
cookie-bearing WebSocket. The browser driver still supplies the page's
`ClientContext`, schedules `step`, and interprets service events.

## NET-21 — URL setters, search parameters, and body data models

Fixed base: `d8a228dc73eb5f83c9dd4bb95fd7c6b1b17cb79d` (2026-09-23).
The setter state overrides mutate `ParsedUrl` without reparsing a whole URL for
component assignments. The pinned WPT `setters_tests.json` 278-object runner
reports each failing property/index/comment and compares all expected getters.
`UrlWithSearchParams` owns a shared URL record; each attached
`UrlSearchParams` handle reads the current query on access and writes a freshly
serialized query after mutation. Standalone parameters preserve duplicate entry
order; stable sort compares UTF-16 code units, matching ECMAScript string order.
The form-urlencoded parser operates on body bytes after lossy UTF-8 decoding;
only the URLSearchParams constructor removes one leading `?`.

`amazing-fetch::api` now has owned `Blob`, `File`, `FormData`, and `FormValue`
models. Blob parts combine bytes/text/other blobs, normalize printable ASCII
MIME types to lowercase, and slice with relative indices. Multipart serialization
uses a per-process unique boundary, CRLF line ending normalization for text
fields, and `%0D`, `%0A`, `%22` filename/name escaping. FormData extraction
sets the Content-Type including boundary. Body consumption decodes multipart or
form-urlencoded by MIME type and retains the existing body-used limit and clone
semantics. WebIDL constructors, default lastModified clock, and exposed JS
promises are a dom-bindings adoption request in `.agents/net/questions.md`.

Final Windows fixed-source gate, twice: workspace nextest 1,735 passed / 3 skipped;
workspace doctests 2 passed; net lane nextest 384 passed and doctests 0.
Pinned WPT results: URL 896, IDNA 87 + 2,671, MIME/data 1,107, cookies
192, and setters 92/278 before versus 278/278 after. Both passes used the
same 183-file source manifest SHA-256
`f945d97546b56dee3dc85f5c1b5b16a477aa3bdd51efec9cada8fb242b64ea19`.
Full evidence is in `.agents/net/report.md`.
