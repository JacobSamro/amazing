# Browser rendering TODO

Canonical sequence for turning the GPUI shell from a document viewer into a
usable browser. Resolve one ticket, verify its exit criteria on `win-001`, then
update this map before starting the next ticket.

Frontier: **#2 — external stylesheets**

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
Status: ⏳ Next

### Question

How should `<link rel="stylesheet">` URLs be resolved, fetched, ordered with
inline styles, and contained when one stylesheet fails?

### Answer

Pending. Exit requires a deterministic redirect/relative-URL fixture, an
external-CSS regression test, and visibly styled Google chrome without a
navigation failure when an individual sheet cannot load.

## #3: Web image resources

Blocked by: #1, #2
Type: Prototype
Status: ⬜ Pending

### Question

How should `<img>` resources be resolved and decoded into intrinsic layout and
paint surfaces while keeping image codecs outside engine cores?

### Answer

Pending. Reuse the GPUI shell's existing `image` dependency as the codec
adapter and feed owned RGBA images through the established replaced-element
and `ImageProvider` seams.

## #4: Document identity

Blocked by: #2, #3
Type: Prototype
Status: ⬜ Pending

### Question

How should fetched `<title>` and favicon resources update each tab after
redirects and failures?

### Answer

Pending.

## #5: Page hit testing and navigation

Blocked by: #2, #3
Type: Prototype
Status: ⬜ Pending

### Question

How should painted links, buttons, text inputs, and forms expose hit regions
and route activation back into per-tab navigation?

### Answer

Pending.

## #6: External scripts and live DOM repaint

Blocked by: #5
Type: Prototype
Status: ⬜ Pending

### Question

How should classic external scripts, event dispatch, timers, and DOM mutation
reuse the Phase 7 runtime inside each GPUI tab without blocking browser input?

### Answer

Pending.

## #7: Browser session state

Blocked by: #5, #6
Type: Discuss
Status: ⬜ Pending

### Question

What are the minimum cookie, cache, local-storage, download, and persisted
history semantics required for normal authenticated browsing?

### Answer

Pending.
