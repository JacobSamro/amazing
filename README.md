# Amazing

A from-scratch web browser engine in pure Rust — **zero third-party crates**
in every engine core component. Not a wrapper, not a fork, not a Chromium/
Servo/Ladybird embed.

## What's built so far

Measured on `dev` (`c103cdcef`, 2026-10-18): 34 crates, about 540,000 lines of Rust, 3,173 commits
since 2026-08-21. Every number below comes from a gated Linux run on Boat; none are estimates.

| Area | What works | Measured |
|------|-----------|----------|
| JavaScript language | Parser, bytecode VM (default for top-level code; tree-walking interpreter as fallback), modules, generators, async, classes, Proxy, Annex B | **Test262 53,561 / 53,578 files (99.97%)**, 102,891 variants; 17 files fail |
| JavaScript built-ins | Full ES library incl. Temporal (all calendars), Intl (CLDR dates ported from lumen, time zones, plural rules, collation), Atomics, TypedArrays, iterator helpers | included in the Test262 count above |
| JavaScript speed | Interned property keys, fast hashing, small-object linear scan, leaner calls | 12-program benchmark: about 1.5× faster at top level, 2× in functions since 2026-10-11 (it was 11–63× slower than `node --jitless` before that; not re-measured against Node since) |
| DOM | Nodes, Range, events, MutationObserver, custom elements incl. ElementInternals and form association, reflected attributes, `:state()` / `:defined` | WPT `dom/` suite 288 files; `dom/` + `html/dom/` + `custom-elements/` 510 of 1,001 files |
| HTML | WHATWG tokenizer and tree construction, encodings | html5lib/WPT tree subset 243 / 243 |
| CSS and layout | Cascade, selectors, block/inline, floats, tables, flexbox, grid incl. Grid Lanes dense packing, text-autospace, CJK fallback | WPT render: css-grid 487, css-flexbox 350, css-sizing 100, css-text 246 passing reftests |
| Networking | HTTP/1.1, HTTP/2 client, TLS, cookies, CORS, HTTP cache with revalidation and cache modes, XHR | WPT `fetch/` 114 files (2,464 subtests), `xhr/` 222 files |
| Web APIs | Fetch, Streams (readable/writable/transform, byte streams), compression streams, `blob:` URLs, File/FileReader, ProgressEvent, WebSocket | WPT `streams/` 59, `FileAPI/` 27, `compression/` 13 files |
| WebAssembly | Wasm 2.0 core engine (own crate); JS API, SIMD and exception handling are in review | pinned Wasm spec testsuite passes in full |
| Paint and compositor | Software rasterizer, tiled compositor, retained frames | render corpora above; frame-time report in `amazing-compositor` tests |
| Media and WebRTC | Images, PCM/Opus audio, VP8/H.264 video, ICE/TURN, DTLS-SRTP, live Chromium interop | Phase 9 artifacts and screenshots below |
| Browser shell | GPUI window with tabs, tab groups, address bar, back/forward/reload, bookmarks bar, find-in-page, downloads panel, session history | runs on Windows and Linux; no macOS build yet |
| Workspace tests | Unit and integration tests across all crates | 5,804 nextest tests + 39 doctests, 0 failing |

Not built yet: incremental style/layout invalidation, GPU raster, the multi-process model and
sandboxing, a macOS port, DevTools, extensions, Web Workers, IndexedDB, and a cycle-collecting
garbage collector (designed in `docs/lanes/js-gc-design.md`, first step in review).

## Why "Amazing"?

The name is a nod to [Andreas Kling's tweet](https://x.com/awesomekling/status/2090508736380936288)
about raising ambitions for Ladybird:

> "Good enough" performance is no longer the goal. We are now aiming for
> ***amazing*** performance! 🤓

This project takes the word and runs with it — from the opposite direction:
instead of optimizing an existing engine, it rebuilds one from first principles.

## Hard constraints

- **`std` only** in the engine cores: `amazing-html`, `amazing-css`,
  `amazing-dom`, `amazing-layout`, `amazing-js`, `amazing-net`,
  `amazing-media`, `amazing-paint`, `amazing-webrtc`.
- No `nom`/`pest` for parsing, no `skia`/`tiny-skia` for painting, no
  `rusttype`/`fontdue` for fonts, no `hyper`/`reqwest` for networking.
  Everything is hand-written against the relevant specs:
  - WHATWG HTML Standard (tokenizer §13.2.5, tree construction §13.2.6)
  - CSS Syntax Module Level 3
  - ECMA-262 (bytecode VM with a tree-walking fallback, no JIT)
  - HTTP/1.1 and HTTP/2 over `std::net::TcpStream`, TLS through the isolated adapter
- First-party `unsafe` only at platform boundaries (Phase 8+), isolated and
  justified per-block. The documented vetted TLS exception may contain
  dependency-internal unsafe; its first-party adapter forbids unsafe.
- Every phase ships a runnable artifact — a terminal dump, an image file,
  or a real interactive window. Passing unit tests alone doesn't count.

## Crates

| Crate | Role | Status |
|-------|------|--------|
| `amazing-common` | shared byte-stream/text utils | ✅ |
| `amazing-dom` | arena-allocated DOM (`u32` indices, no `Rc<RefCell>`) | ✅ core |
| `amazing-html` | WHATWG tokenizer + namespace-aware tree construction | 🔄 expanded html5lib/WPT subset 243/243; broader gaps tracked |
| `amazing-css` | Syntax-3 parser, selectors, cascade | ✅ Phase 2 |
| `amazing-layout` | block/inline FC → flexbox/grid | ✅ Phase 3/3.5 |
| `amazing-paint` | software rasterizer | ✅ Phase 4 |
| `amazing-net` | HTTP/1.1 client | ✅ Phase 5 |
| `amazing-net-tls` | isolated rustls HTTPS transport | ✅ Phase 5.1 |
| `amazing-js` | ECMA-262 lexer/parser/interpreter | ✅ Phase 6 milestone |
| `amazing-media` | BMP/P6 PPM images + RIFF/WAVE PCM audio | Phase 9.1/9.2 |
| `amazing-webrtc` | ICE/TURN + live Chromium DTLS-SRTP media | Phase 9.3–9.47 |
| `amazing-webrtc-codecs` | Opus, VP8 decode, H.264 encode/decode | Phase 9.20–9.23 |
| `amazing-webrtc-crypto` | vetted DTLS-SRTP + secure UDP endpoint | Phase 9.9–9.13 |
| `amazing` | DOM/JS tracer + GPUI launcher + Win32 fallback (`domrun`, `gui`) | ✅ Phase 7/8 |

The Windows GUI shell includes tabs, address/history navigation, live DOM
events and mutation, classic scripts/external resources, and software-paint
reflow. Relative `<img>` resources are fetched, decoded by `amazing-media`,
given intrinsic inline layout geometry, and painted with nearest-neighbor
scaling. CI runs the workspace build and test gate on every push and pull
request via `.github/workflows/ci.yml`.

![Ferric Phase 8 running on win-001](docs/screenshots/phase8-win001.png)

![Ferric Phase 9 decoded-image render](docs/screenshots/phase9-media.png)

![Amazing GPUI decoded H.264 video surface](docs/screenshots/gpui-video-presentation-win001.png)

Clocked decoded-video proof: [frame A](docs/screenshots/gpui-video-frame-a-win001.png)
and [frame B](docs/screenshots/gpui-video-frame-b-win001.png).

Direct secure-session GPUI proof: [frame A](docs/screenshots/gpui-secure-video-win001.png)
and [frame B](docs/screenshots/gpui-secure-video-b-win001.png).

Secure 90 kHz RTP-clock proof: [frame A](docs/screenshots/gpui-rtp-clock-a-win001.png)
and [frame B](docs/screenshots/gpui-rtp-clock-b-win001.png).

Secure RTCP A/V-clock proof: [frame A](docs/screenshots/gpui-rtcp-av-clock-a-win001.png)
and [frame B](docs/screenshots/gpui-rtcp-av-clock-b-win001.png).

Absolute A/V deadline proof: [frame A](docs/screenshots/gpui-absolute-clock-a-win001.png)
and [frame B](docs/screenshots/gpui-absolute-clock-b-win001.png).

Progressive secure-stream proof: [frame 1](docs/screenshots/gpui-live-stream-a-win001.png)
and [frame 2](docs/screenshots/gpui-live-stream-b-win001.png).

Incremental SRTP decode proof: [frame 1](docs/screenshots/gpui-incremental-decode-a-win001.png)
and [frame 2](docs/screenshots/gpui-incremental-decode-b-win001.png).

Bounded decoded-frame queue proof:
[complete session](docs/screenshots/gpui-bounded-queue-win001.png).

Authenticated PLI/IDR recovery proof:
[complete session](docs/screenshots/gpui-pli-recovery-win001.png).

Actual RTP-loss recovery proof:
[complete session](docs/screenshots/gpui-rtp-loss-recovery-win001.png).

Windows audio-device clock proof: [cursor A](docs/screenshots/gpui-audio-clock-a-win001.png)
and [cursor B](docs/screenshots/gpui-audio-clock-b-win001.png).

Live Chromium interoperability proof:
[decoded H.264 session](docs/screenshots/chromium-webrtc-interop-win001.png).

Phase 4 visual gate (Amazing at left, Edge reference at right; 800×804):

![Ferric Phase 4 visual comparison](docs/screenshots/phase4-side-by-side.png)

Reproduction details and measured RMSE: [Phase 4 visual audit](docs/phase4-visual-audit.md).

## Usage

```sh
# launch the end-user GPUI browser shell on Windows
cargo run

# pipeline status
amazing version

# parse an HTML file (or stdin) and dump the DOM tree
amazing parse page.html
cat page.html | amazing parse
cat fragment.html | amazing parse-fragment div

# execute a classic JavaScript file (or read the script from stdin)
amazing run script.js
cat script.js | amazing run

# execute a script against a live parsed DOM and dump the mutation result
amazing domrun page.html script.js

# open the legacy native Win32 fallback explicitly
amazing gui [url]

# Phase 9 image-media artifact
amazing render examples/phase9-media.html phase9-media.bmp 400

# Phase 9 PCM audio artifacts
amazing audio-info examples/phase9-audio.wav
amazing play-audio examples/phase9-audio.wav

# Phase 9 WebRTC transport artifact
amazing webrtc-loopback
amazing turn-probe
amazing rtp-jitter-probe
amazing opus-probe
amazing video-frame-probe
amazing vp8-decode-probe
amazing h264-roundtrip-probe
amazing h264-rtp-probe
amazing sdp-roundtrip examples/phase9-offer.sdp
amazing sdp-roundtrip examples/phase9-chromium-offer.sdp
amazing sdp-browser-answer examples/phase9-chromium-offer.sdp
amazing browser-peer 8787
amazing secure-media-loopback
amazing secure-media-processes
amazing secure-video-stream 127.0.0.1:<listener-port>

# Phase 9 hardware capability artifact (Windows)
amazing gpu-info
amazing gpu-present-smoke
```

```sh
cargo test --workspace --locked

# External conformance corpora checked out beside the repo on win-001
python scripts/test262.py C:\Users\server\src\test262 --binary target\debug\amazing.exe
python scripts/html5lib.py C:\Users\server\src\wpt --binary target\release\amazing.exe
python scripts/wpt.py C:\Users\server\src\wpt --binary target\release\amazing.exe
```

## Architecture notes

- Single-threaded by design for now; parallel layout/paint slots are marked
  for a later phase (see `ARCHITECTURE.md` for locked decisions).
- Conformance is measured, not estimated: WPT tracking starts at Phase 3
  (`css/CSS2`), Test262 tracking at Phase 6.

## License

MIT
