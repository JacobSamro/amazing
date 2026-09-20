# Net lane decision map

## External-dependency decisions — user approval required

**Public Suffix List: approve a pinned data snapshot, or defer site/cookie
correctness.** A provider must return public-suffix and registrable-domain
(eTLD+1) results for normalized hosts, implement exact/wildcard/exception rules,
and distinguish IPs and nonregistrable hosts; proposed coverage includes both
ICANN and PRIVATE rules to separate tenants on shared hosting. Vendor the
[upstream list](https://publicsuffix.org/list/) at an exact commit and SHA-256,
retain its license, and pin rule-normalization/generator inputs and fixtures;
propose monthly reviewed update PRs with boundary-change tests and persisted-key
migration review, never silent runtime updates. Without it, correct SameSite,
site-based partitioning and public-suffix Domain-cookie rejection remain blocked:
the current full-host fallback separates sibling subdomains that should share
a site; guessing the last two labels can instead merge unrelated sites.
[Rule format](https://github.com/publicsuffix/list/wiki/Format).

**Brotli: approve a vetted network adapter, or retain gzip/deflate without br.**
Pin `brotli-decompressor =5.0.3`, `alloc-no-stdlib =2.0.4`,
`alloc-stdlib =0.2.4`, lockfile checksums and safe features (no unsafe/seccomp/FFI),
with strict-window decoding, separate output/scratch budgets and malformed-stream
fixtures. The decoder is already transitively present through the text provider,
whose WOFF2 adapter directly pins 5.0.3; that font approval does not authorize
network use. Deferral still permits servers to negotiate gzip/deflate or an
uncompressed response under [HTTP negotiation rules](https://www.rfc-editor.org/rfc/rfc9110.html#section-12.5.3),
so lack of br does not inherently break Brotli-capable pages. The cost is
potentially more transferred bytes and failure for br-only resources or servers
that send br despite negotiation. No real-page failure rate or bandwidth penalty
has been measured here; the larger evaluation below identifies adapter work.

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

### Deferred Unicode/IDNA seam

`IdnaProvider::domain_to_ascii` receives the UTF-8-decoded, percent-decoded
Unicode special host. `AsciiOnly`, the default provider, returns the distinct
`ParseError::IdnaRequired`. This includes malformed UTF-8 decoded with replacement;
such inputs are not misreported as proven URL failures by the corpus runner.
Unicode characters in paths/query/fragments/userinfo, and opaque hosts, do not
need IDNA and are processed normally. ASCII `xn--` labels are retained as ASCII:
the current URL Standard's non-strict ASCII domain branch lowercases even when
strict Unicode validation would diagnose a label. No punycode codec is bundled.

A future provider must supply version-pinned UTS #46 Unicode mapping and ToASCII,
nontransitional processing, CheckBidi/CheckJoiners true, IgnoreInvalidPunycode
false, and non-strict CheckHyphens/UseSTD3ASCIIRules/VerifyDnsLength false. The
full domain-parser diagnostic path also needs strict ToASCII results with those
three flags true; the current API does not surface validation diagnostics.
The provider must handle Unicode normalization/mapping, label separator mapping,
punycode, invalid code points and empty output, not merely lowercase strings.
The URL layer still checks ASCII output, forbidden domain code points and IP
classification after the provider. Provider data/source licensing, Unicode
revision, reproducible generation and update policy need project-level approval.
No Unicode mapping tables, PSL, third-party core crate or provider implementation
is included here. The unit-test provider is only a fixed stub proving the seam.

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
