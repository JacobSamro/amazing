# NET-29 HTTP cache WPT selection

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The exact upstream sources from `fetch/http-cache/` are in `source/`.
The repository's WPT license is in `../upstream-LICENSE.md` (relative to the
parent `wpt/` directory). `tests/wpt_http_cache.rs` ports selected network
expectations; `amazing-process/tests/network_cache.rs` adds restart and size-cap checks that WPT
does not exercise. This selection is **native**, not a complete JS WPT run.

| Source | SHA-256 | Selected behavior |
| --- | --- | --- |
| `304-update.any.js` | `c48d9a4feb02925bcdc1c9b82c7abab79d837fa58aa1b8bb6cfc6f6694d9e33c` | ETag and Last-Modified conditional requests, 304 metadata and body |
| `cache-mode.any.js` | `9538bd9c3c1fd58186294cce5d78fe97f47656a2739ee4998c1562656859be62` | Five Fetch cache-mode request header cases |
| `cc-request.any.js` | `9938314e76fd33e87e57e19a6464227494f5b21ee86fa2f1d3933c1bba48a118` | max-age, no-cache, no-store requests |
| `freshness.any.js` | `7a20cae2177a413b6cdceab33b8abbb22a8c00f33f3b4705a90d9e2720571b4c` | Expires, max-age, Age, no-store, no-cache |
| `vary.any.js` | `c83d30731ea2c2253188dc607215e38bd22696938459b75e385b9b0f06d5f448` | matching and mismatching variants, absent field, `Vary: *` |
| `http-cache.js` | `536b28598736e13afe26362baae2bb9d7299457c82c59ebca216b34067578805` | Original WPT test driver for context |
| `invalidate.any.js` | `6c65ca248428e3dcb81fc7fd012d38ab07253f9426e10dbc13a8ad923a497c3a` | Successful POST invalidates cached target |
| `status.any.js` | `1f1de5fd8541df17f28fd1b2ceada3ff688dcb4fc5c5c9b2a1855b902d153c99` | 13 status codes with explicit fresh and stale responses |

The WPT runner's JavaScript-level result remains separate from this native
selection. The upstream test helper depends on the WPT server stash, dynamic
date substitution, and browser Fetch promises. Those are not emulated by this
test. NET-26's held Fetch/XHR runtime host and its pending JS reaction fix must
land before a JS-level HTTP cache remeasure can be used as the gate.
