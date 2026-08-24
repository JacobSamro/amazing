# Browser rendering TODO

Canonical sequence for turning the GPUI shell from a document viewer into a
usable browser. Resolve one ticket, verify its exit criteria on `win-001`, then
update this map before starting the next ticket.

Frontier: **#6 — external scripts and live DOM repaint**

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
The dynamic probe verified `q=rust+browser` navigation. POST is rejected rather
than rewritten because the network core does not yet expose request bodies.

## #6: External scripts and live DOM repaint

Blocked by: #5a
Type: Prototype
Status: ⏳ Next

### Question

How should classic external scripts, event dispatch, timers, and DOM mutation
reuse the Phase 7 runtime inside each GPUI tab without blocking browser input?

### Answer

Pending.

## #7: Browser session state

Blocked by: #5a, #6
Type: Discuss
Status: ⬜ Pending

### Question

What are the minimum cookie, cache, local-storage, download, and persisted
history semantics required for normal authenticated browsing?

### Answer

Pending.
