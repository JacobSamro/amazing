# Goal Prompt: Ferric — A From-Scratch, Zero-Dependency Rust Browser Engine

## Mission
Build **Ferric**, a web browser engine written entirely in pure Rust, using **only the Rust standard library** — zero third-party crates for any core engine component (HTML parsing, CSS parsing, layout, JS engine, networking, rendering, GUI). The goal is a genuinely independent alternative to Chromium-family browsers: not a wrapper, not a fork, not a Servo/Ladybird embed.

This is a multi-year systems project. Treat every phase below as a standalone milestone with its own working demo, not a checkbox on the way to something else. Do not skip ahead to later phases before earlier ones pass their exit criteria.

> Repo note: the working tree implements these components under the `amazing-*`
> crate names (`amazing-html`, `amazing-css`, `amazing-dom`, `amazing-layout`,
> `amazing-js`, `amazing-net`, `amazing-paint`). "Ferric" is the project's
> codename; build/test happens on win-001 per AGENTS.md.

## Hard constraints
- **No external crates** in the engine core (`ferric-html`, `ferric-css`, `ferric-dom`, `ferric-layout`, `ferric-js`, `ferric-net`, `ferric-paint`). `std` only.
- **GUI shell may use a minimal platform-native windowing layer** (raw `winapi`/`x11`/`cocoa` FFI written by hand, or `std`-only pixel buffer blitting) — justify any exception explicitly before writing it.
- **Memory safety**: no `unsafe` except in isolated, heavily-commented modules for raw syscalls/FFI at the platform boundary. Justify every `unsafe` block in a comment.
- **Every phase must produce a runnable artifact** — not just passing unit tests. If it can't render something to a window or dump something inspectable to a terminal, the phase isn't done.
- **Track conformance against Web Platform Tests (WPT)** from Phase 3 onward. Report pass rate per phase, not aspirational total coverage.

## Phased plan

### Phase 0 — Project skeleton
- Cargo workspace with crates per component (see hard constraints).
- Byte-stream reader/tokenizer utilities shared across HTML/CSS parsers, hand-written (no `nom`/`pest`).
- Decide and document internal architecture: single-threaded first, note where parallelism (layout, paint) gets added in a later phase.
- **Exit criteria:** workspace builds, empty binary runs, CI (even just a shell script) runs `cargo test` on every push.

### Phase 1 — HTML parser (`ferric-html`)
- Implement the WHATWG HTML5 tokenizer and tree-construction algorithm by hand, state-machine style, following the spec's named states directly (don't invent your own simplified grammar — the spec's error-recovery behavior *is* the interop contract).
- Produce a real DOM tree structure (arena-allocated, indices not `Rc<RefCell<>>` — avoids reference-cycle pain later).
- **Exit criteria:** parses malformed real-world HTML (mismatched tags, missing closing tags, implicit `<tbody>`, etc.) matching browser behavior; dump DOM tree to terminal as indented text.

### Phase 2 — CSS parser + selector matching (`ferric-css`)
- Hand-written CSS tokenizer + parser per CSS Syntax Module Level 3.
- Selector matching engine (type, class, id, attribute, pseudo-classes, combinators) with correct specificity calculation.
- Cascade + inheritance resolution producing a computed-style tree over the DOM.
- **Exit criteria:** given HTML + CSS, print computed style values per DOM node.

### Phase 3 — Layout engine (`ferric-layout`)
- Start with block and inline formatting contexts only. Explicitly defer flexbox/grid to Phase 3.5.
- Box model: margin/border/padding/content, containing-block resolution.
- Text layout: line breaking, basic font metrics (you'll need a hand-written TTF/OTF glyph-metrics parser — no `rusttype`/`fontdue`).
- **Exit criteria:** produces a layout tree with absolute pixel positions/sizes for a real webpage's DOM; begin tracking WPT `css/CSS2` pass rate.

### Phase 3.5 — Flexbox/Grid
- Only after 3 is solid. Same exit-criteria pattern against WPT `css-flexbox`/`css-grid`.

### Phase 4 — Rasterization/paint (`ferric-paint`)
- Software rasterizer first (scanline fill, anti-aliasing) — pure Rust, no `skia`/`tiny-skia`/`raqote`.
- Pixel buffer output to a window via minimal platform windowing (Phase 8 dependency — build a stub `.ppm`/`.png`-file-writer first so Phase 4 has an exit criterion without needing a GUI yet).
- **Exit criteria:** render a real webpage's layout tree to a pixel buffer, dump to an image file, diff visually against a reference screenshot.

### Phase 5 — Networking (`ferric-net`)
- Hand-written HTTP/1.1 client over `std::net::TcpStream`. Defer HTTP/2 and TLS complexity — for TLS, note explicitly that hand-rolling TLS 1.3 is a security-critical, multi-month sub-project on its own; decide early whether this is truly in-scope for "zero dependencies" or whether it's the one justified exception (most serious from-scratch browser efforts treat a vetted TLS implementation as infrastructure, not "the browser").
- **Exit criteria:** fetch a real HTTP page over the network and feed it into the Phase 1 parser.

### Phase 6 — JavaScript engine (`ferric-js`)
- This is the single largest sub-project in the whole plan — comparable in scope to Phases 0–5 combined. Hand-written lexer, parser (produce an AST per ECMA-262), and a tree-walking interpreter first (skip JIT entirely for v1 — Ladybird made the same call for security/complexity reasons).
- Implement enough of the spec to run real-world scripts: `Object`/`Array`/`Function` semantics, prototype chains, closures, `async`/`Promise`, basic `Map`/`Set`.
- **Exit criteria:** track Test262 pass rate explicitly per milestone; report the number, don't estimate it.

### Phase 7 — DOM/JS bridge
- Wire `ferric-js` objects to live DOM nodes (`document.querySelector`, `element.style`, event listeners, `addEventListener`). This is where most real-world site breakage will surface — expect this phase to take longer than it looks.
- **Exit criteria:** a real page's `onclick` handlers and DOM mutations visibly work end-to-end.

### Phase 8 — GUI shell
- Minimal windowing + input handling per platform (start with one OS only — pick Linux/X11 or macOS, not both).
- Tabs, address bar, back/forward — application-level only, no engine changes required.
- **Exit criteria:** a double-clickable app that loads a URL you type and renders it interactively.

### Phase 9 (stretch) — Media / WebRTC / hardware accel
- Only after 0–8 are genuinely stable. This is where the earlier Ladybird/Servo conversation's WebRTC discussion applies — treat it as its own multi-phase sub-project, not a bolt-on.

## Reporting format for each phase
When you (the agent) complete a phase, report:
1. What's implemented vs. spec section explicitly deferred
2. Exit criteria met, with the actual runnable demo command
3. Known correctness gaps
4. WPT/Test262 pass rate where applicable (from Phase 3 / Phase 6 onward)
5. Recommended next phase — do not auto-proceed without confirmation

## Non-goals (explicitly out of scope for v1)
- Extensions/add-ons API
- DevTools
- Sandboxing/multi-process architecture (huge security-relevant scope on its own — note as a required pre-production milestone, not a v1 feature)
- Full CSS3+ (animations, transforms, filters) until core box model + flexbox/grid are stable
