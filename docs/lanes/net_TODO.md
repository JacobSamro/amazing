# Net lane decision map

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
