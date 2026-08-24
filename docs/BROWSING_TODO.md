# Browser rendering TODO

Canonical sequence for turning the GPUI shell from a document viewer into a
usable browser. Resolve one ticket, verify its exit criteria on `win-001`, then
update this map before starting the next ticket.

Frontier: **#8 — GPUI browsing-frame media and visual precision**

## #1: Verified HTTPS navigation

Blocked by: none
Type: Research
Status: ✅ Resolved

### Question

How should Amazing load public HTTPS pages without hand-rolling TLS or leaking
provider types into the std-only network core?

### Answer

Use the isolated, safe `amazing-net-tls` adapter. It pins rustls 0.23.43 and
Mozilla roots, verifies server names and certificates, advertises only
HTTP/1.1, bounds response memory, and returns owned response bytes. The HTTP
core retains redirect and framing validation, including rejecting an unclean
TLS EOF when the response has no explicit HTTP body length.

Exit evidence: focused network tests 62/62; adapter tests 2/2; live ignored
Google regression passes; `amazing fetch https://google.com` returns 200 and
the GPUI address bar reaches `https://www.google.com` without an error page.

## #2: External stylesheet loading

Blocked by: #1
Type: Prototype
Status: ✅ Resolved

### Question

How should `<link rel="stylesheet">` URLs be resolved, fetched, ordered with
inline styles, and contained when one stylesheet fails?

### Answer

Walk `<style>` and applicable `<link rel="stylesheet">` nodes in document
order. Resolve links against the final post-redirect URL, cap the page at 32
external sheets/4 MiB CSS, and isolate individual fetch/type failures. A
loopback regression proves relative resolution and cascade order. Google's
current linked sheet legitimately returns an empty body; the remaining visual
differences are CSS-engine coverage rather than a resource-fetch failure.

## #3: Web image resources

Blocked by: #1, #2
Type: Prototype
Status: ✅ Resolved

### Question

How should `<img>` resources be resolved and decoded into intrinsic layout and
paint surfaces while keeping image codecs outside engine cores?

### Answer

Reuse core BMP/PPM decoding first, then the GPUI shell's existing `image`
dependency for bounded PNG/JPEG/GIF/WebP/ICO decoding. Resolved `<img>` pixels
flow through `ReplacedElementProvider` and `ImageProvider`; width/height
attributes preserve aspect ratio. The loopback intrinsic-size test passes and
the live Google logo is visible in the GPUI viewport.

## #4: Document identity

Blocked by: #2, #3
Type: Prototype
Status: ✅ Resolved

### Question

How should fetched `<title>` and favicon resources update each tab after
redirects and failures?

### Answer

The navigation result carries the normalized first HTML `<title>` and a
resolved/decoded `rel=icon` resource (with `/favicon.ico` fallback) alongside
the final address and viewport. Tabs render the favicon in horizontal/sidebar
modes and fall back to the site initial only when metadata is absent. The live
Google tab displays “Google” and its fetched icon after redirect.

## #5: Painted link hit testing and navigation

Blocked by: #2, #3
Type: Prototype
Status: ✅ Resolved

### Question

How should painted links and linked images expose hit regions and route
activation back into per-tab navigation?

### Answer

Derive link rectangles from laid-out word and replaced-image fragments, resolve
each ancestor anchor against the final document URL, and store page-coordinate
regions beside the rendered surface. A bounds-aware GPUI canvas maps window
pointer input back into those coordinates without product screen constants.
The dynamic probe clicked a painted local link and verified the target title,
address, and page content.

## #5a: Text inputs, buttons, and basic forms

Blocked by: #5
Type: Prototype
Status: ✅ Resolved

### Question

How should painted text inputs and buttons expose focus/edit state, and how
should GET/POST form submission construct the next navigation request?

### Answer

Layout-derived control regions distinguish editable text-like inputs from
submit controls. Clicking a text input creates a visible GPUI edit overlay;
typing, selection, paste, deletion, Escape, and Enter are handled by shell
focus. GET submission resolves the form action and URL-encodes named values.
POST sends a bounded form body with content type/length through the same
session/TLS path. The dynamic probe verified `q=rust+browser` GET navigation;
loopback gates verify real POST bytes and rendered POST responses.

## #6: External scripts and live DOM repaint

Blocked by: #5a
Type: Prototype
Status: ✅ Resolved

### Question

How should classic external scripts, event dispatch, timers, and DOM mutation
reuse the Phase 7 runtime inside each GPUI tab without blocking browser input?

### Answer

The GPUI loader collects bounded inline and external classic scripts in DOM
order, resolves/fetches external sources, installs a shell DOM host, executes
them against the arena, and pumps bounded initial timers before stylesheet,
resource, layout, and paint passes. Failures remain page-local. The live fixture
proves external text/title mutation is visible after relayout.

## #6a: Post-paint events and timers

Blocked by: #6
Type: Prototype
Status: ✅ Resolved

### Question

How should each tab retain its interpreter/DOM after first paint so pointer
events and future timer turns mutate and repaint the current page?

### Answer

Each script-bearing tab retains its DOM host and interpreter. The shell clock
pumps future timer turns and regenerates metadata/resources/layout/paint after
mutations. Painted anchor clicks dispatch DOM `click` first; script navigation
is honored and `preventDefault` suppresses native link navigation. The live
fixture proves both delayed timer repaint and click-driven title/content repaint
without leaving the source URL.

## #7: Browser session state

Blocked by: #5a, #6a
Type: Discuss
Status: ✅ Resolved

### Question

What are the minimum cookie, cache, local-storage, download, and persisted
history semantics required for normal authenticated browsing?

### Answer

The device-local `BrowserSession` owns domain/path/secure cookies, a
cookie-varying 64-entry/16 MiB response cache, per-origin persistent
`localStorage`, bounded persisted history/bookmarks, sanitized unique download
paths, and persisted download records. `amazing-net` accepts validated embedding
headers and bounded GET/POST bodies while keeping policy in the shell. Script
history directives update per-tab history without repainting/reloading. Live
evidence proves storage survives a process restart (visit 1 → 2), downloads are
saved under `Downloads/Amazing` and shown after restart, and final Google
navigation still succeeds.

## #8: GPUI browsing-frame media and visual precision

Blocked by: render-engine #6 and #7, browsing #7
Type: Prototype
Status: 🌫️ Fog

### Question

How will GPUI compose precise shell chrome, document interaction overlays, and
standards-driven `<audio>`/`<video>` playback inside one browsing frame while
the engine cores remain provider-neutral?

### Exit criteria

- The checked-in shell reference has measured geometry, typography, icon,
  focus, hover, disabled, menu, sidebar, and viewport screenshot assertions at
  the win-001 reference size; the target window is captured unobscured through
  `scripts\capture-gpui-shell.ps1`.
- HTML media elements own resource selection, ready/network/playback state,
  timing, events, layout slots, paint order, accessibility, and controls inside
  the active document. Existing media tabs and codec probes are regression
  dependencies, not acceptance evidence.
- GPUI renders every visible desktop surface and consumes only owned engine
  presentation data; no Chromium/WebView or second production UI path is used.
- Pinned applicable WPT/testharness cases, local interaction fixtures, A/V
  timing probes, full regression gates, and repeat screenshot artifacts are
  recorded with measured results.

### Answer

Pending until the render-engine paint/compositing and replaced-content phases
expose the required presentation descriptors.
