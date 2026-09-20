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

### NET-2 proposed seam

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
