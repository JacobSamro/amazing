# CORS preflight grants and failure lifecycle

Recorded origin/dev base: `5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58`.

Current primary [Fetch §4.8–4.9](https://fetch.spec.whatwg.org/#cors-preflight-cache)
requires scalar server method/header permissions, matching the network partition,
serialized request origin, current URL and credential authorization. Its main-fetch
network-error step clears the matching key. The old native cache instead stores
only the request's exact method/header set, ignores ClientContext.partition_key,
and retains grants after failed request completion. This is engine behavior,
independent of missing installed JS/Window dispatch.

The whole producer replaces aggregate grants with bounded independent permissions:
explicit server methods and header names, wildcard permissions excluding Authorization,
credentialed literal-star preservation (Fetch §3.3.5), forced safelisted-method fallback, default/capped expiry, matching-permission refresh,
zero-age expiration, credential asymmetry and 256-entry early eviction. Invalid
preflight responses cannot create grants through direct native insertion.

NetworkService supplies its actual captured ClientContext partition; PendingFetchApi
retains it across original request/hops. No URL-derived or substitute receiving
client is created. Legacy pure-policy callers retain their unpartitioned namespace.
Matching origin uses the actual request-origin serialization including redirect taint;
existing typed opaque origin identity remains unchanged elsewhere. URL matching
retains the existing without-fragment transport key behavior. The current ClientContext
partition type is the engine's schemeful Site, not a claim of complete network-key
or installed top-level factory coverage.

Preflight preserves the actual request URL lineage/referrer/security with a shared native preflight constructor that copies its scalar/lineage facts
and replaces method/body/headers/credential mode without cloning the request body.
Credentialless OPTIONS sends no cookies or HTTP authentication. Pending terminal
policy/transport/framing errors and explicit native abort clear the key. The common
service terminal path covers cache-response filtering errors. Genuine modern Reporting
uploads consult/insert/clear through their native null-client partition; legacy CSP
uploads retain their separate reserved non-preflight route.

Fifteen grouped native controls cover independent methods/subsets/extra names,
combined grants, wildcard/Authorization, forced-method fallback, refresh/zero/default/
capped expiry, invalid responses, origins/URLs/credentials/eviction, and real HTTP
method/header reuse, actual partition isolation, CORS/framing failure retry, genuine
abort, credential asymmetry and source referrer custody. Test-only BEFORE adds the
identical controls to otherwise unchanged production; newly exposed failures are
measured as baseline engine gaps. Original tests/assertions and their IDs remain.

Pinned WPT reference is
[c2a2fce69e5c2509d9dc28440ec8b42532e5c293 cors-preflight-cache.any.js](https://github.com/web-platform-tests/wpt/blob/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/fetch/api/cors/cors-preflight-cache.any.js).
The historical installed cors-preflight.any.js timeout is retained as an installation
boundary, not relabelled as fixed by these native controls. No permissively licensed
Rust algorithm was ported; these std-only adapters follow the primary directly.
No Cargo or dependencies change.

Qualification remains pending. Frozen whole workspace/nextest/doctests/browser/native
TLS-net-fetch-process + original 11 CSP directory commands will measure both sources.
Actual installed JS Fetch/CORS, Window/Worker factories and report consumers remain
owned driver prerequisites; there is no installed/full-WPT claim.


Failed implementation17eb retained: 5276 workspace and848 native passes, one new
real-peer source-referrer control failed. Source inspection found the referrer fix
had reached only synchronous fetch_api, leaving PendingFetchApi's actual producer
unchanged. The corrected whole producer shares preflight_policy across both paths,
keeps the assertion, and also emits exactly one UA Origin using genuine redirect
lineage. An additional two-origin real307/OPTIONS/main peer control proves null-tainted
origin and original referrer through both hops. This is a source correction, not a
passing qualification or an unchanged rerun. BEFORE759c14-group proof remains exact;
AFTER adds the related fifteenth control, with every prior assertion retained.

Qualification complete: source `a1993cde94bacf13d3b77c7470abadfa837c8f0b`; native AFTER/controller/fetch actual0. Workspace5266→5278 (+12/lost[]), docs38, native838→850 (+12/lost[]). BEFORE actual1 reflects11 newly exposed failures in otherwise unchanged production, not a passing baseline. All15 AFTER groups pass. Fresh original11-directory CSP33files/82subtests retained (historical32/81 subset also retained). All1113 ELF artifacts, source/generated/raw remain native/tmp; manifest `/tmp/amazing-native-retained/net-cors-preflight-origin-sourcefix-20261006/cors-grants-origin-after-a1993cde/retained/manifests/cors-grants-origin-after-a1993cde.json` SHA010185b7b826c58b03eb45fe28ce87a320518c39bd0398a55d4b2d76810b1581. Root merged-head workspace/full16 remain publication gates.
