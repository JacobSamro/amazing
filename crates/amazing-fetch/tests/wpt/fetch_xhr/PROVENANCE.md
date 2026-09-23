# NET-23 pinned Fetch/XHR network expectations

WPT revision: `c48d58747e1f211527fb695fd60548a997fae617`. Exact upstream
files are in `source/`; their SHA-256 values are in `source-hashes.json`.
They were downloaded from `raw.githubusercontent.com/web-platform-tests/wpt`
under that revision. The WPT license is in `../upstream-LICENSE.md`.
The native expectation runner is `../../wpt_fetch_xhr.rs` and the std-only
server shim is `../../../../amazing-net/tests/support/wpt_server.rs`.

Selected network assertions:

| Upstream tests | Native expectations |
| --- | --- |
| `fetch/api/basic/request-headers.any.js` | method/body, Origin, Content-Length and user-agent response echoes |
| `fetch/api/redirect/redirect-mode.any.js` | 60 same/cross origin x 301/302/303/307/308 x follow/manual/error x cors/no-cors outcomes |
| `fetch/api/cors/cors-basic.any.js`, `cors-preflight.any.js` | opaque/CORS/error filtering and allowed/denied preflight |
| `fetch/api/credentials/cookies.any.js` | include/same-origin/omit storing and sending |
| `fetch/api/abort/general.any.js`, `headers/header-setcookie.any.js`, `response/response-consume-empty.any.js` | pre/in-flight abort, forbidden author/response headers, body mixins |
| `xhr/event-*.any.js`, `responsetype.any.js`, `send-redirect.htm`, `status-basic.htm`, `getresponseheader-chunked-trailer.htm` | states/events, text/json/arraybuffer, redirects, 27 status combinations, chunked trailer visibility |
| `xhr/event-timeout.any.js` | timeout transition and event |

Ported server handlers and their bounded selections:

| Handler | Behaviour in `wpt_server.rs` |
| --- | --- |
| `fetch/api/resources/inspect-headers.py` | `headers` echo, CORS response headers |
| `fetch/api/resources/redirect.py` | status, Location, CORS response, preflight OPTIONS |
| `fetch/api/resources/preflight.py` | method/header grants, status, token keyed preflight observation |
| `fetch/api/resources/echo-content.py`, `status.py` | request-body echo, status/text/content/type |
| `fetch/api/resources/top.txt`, `cors-top.txt` and `.headers` | static body, CORS wildcard, `pipe=header(...)` cookie fixtures |
| `fetch/api/resources/infinite-slow-response.py` | chunked delayed body sufficient for abort before completion |
| `xhr/resources/content.py`, `status.py`, `redirect.py` | query/body echo, status/text, redirect target and followed response |
| `xhr/resources/chunked.py`, `trickle.py`, `delay.py` | chunked body/trailer, delayed chunks, bounded delay for timeout |

The shim deliberately uses two loopback ports for same/cross origin, with HTTP
only. It does not emulate WPT's server stash, authentication, HTTPS, worker
realms, or full `pipe` grammar. The 129 assertions are a **native network
expectation selection**, not JS-level WPT results or the complete files' pass
counts. JS constructor descriptors, Promise scheduling, signal reasons,
`responseXML`, Blob/Document objects, and worker variants require bindings
and runner adoption listed in `.agents/net/questions.md`.

Linux native baseline on fixed base `f8bdb33`: **120/129 pass, 9 fail**.
Failures: four same-origin request header cases (Origin and empty POST length)
and five same-origin no-cors manual redirects. After net-owned fixes:
**129/129 pass, 0 fail**. Raw logs are in `.agents/net/net23-baseline.txt`
and `.agents/net/net23-after-linux.txt`.
