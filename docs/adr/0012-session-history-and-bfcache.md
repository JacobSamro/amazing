# ADR 0012 — Session history and the back-forward cache

Status: proposed for project review (2026-09-24). Design only; no runtime adoption.
Owner: IPC lane for the process contract; HTML and driver owners review the
navigation algorithms and bindings. Depends on ADR 0005, 0008, and 0009.

## Context

The browser process must own tab lifetime, navigation, site assignment, and
cross-process history. A renderer owns a live `Document` and its event loop.
The [HTML navigation and session-history model](https://html.spec.whatwg.org/multipage/browsing-the-web.html)
has one traversable navigable history spanning a top-level document and child
navigables. A history step can activate several navigables; it is not simply a
URL stack. A document state may retain a live `Document` for later traversal or
lose it and repopulate from its stored URL and resource. The latter distinction
is the basis of bfcache.

## Proposed decision

The browser is the single writer of a tab's traversable history. Each
`SessionHistoryEntry` has a browser-minted entry ID and step, navigable ID,
parent navigable ID, URL, referrer and response metadata needed for reload,
method/body replay policy, serialized `history.state` and Navigation API state,
scroll-restoration mode, persisted user state, and a document-state ID. The
document state records origin/site assignment, the renderer process/document
token if live, and whether it is active, frozen, or discarded. Multiple
same-document entries may refer to one document state. No renderer-chosen ID,
step, origin, or URL becomes authoritative until the browser validates it.

Each top-level tab has one ordered set of history steps and a current step.
Child navigable entries are associated with steps in that same traversable.
Push clears forward steps; replace changes the active step without adding one;
same-document fragment and History API changes retain the document state;
cross-document navigation creates a new document state. Reload and traversal
are distinct operations. The browser serializes all step changes so racing
renderer requests cannot reorder them. These choices follow the [HTML history
entry and traversal algorithms](https://html.spec.whatwg.org/multipage/browsing-the-web.html).

The renderer owns the live DOM, JS heap, active timers, focus/selection, form
controls, per-document scrollable regions, and event delivery. It sends
bounded, structured snapshots of `history.state`, persisted user state, and
scroll positions to the browser at defined commit/freeze points. The browser
stores those snapshots with the entry and sends them on activation or reload.
The renderer cannot directly write the durable history list or activate a
different site's document. Session storage follows its own browser-brokered
partition rules; a bfcache entry retains its live renderer state but does not
own durable storage authority.

### Navigation transaction and IPC

All messages include a tab/navigable ID, browser-minted navigation ID, history
entry ID where applicable, navigation epoch, site assignment, and bounded
payloads. The browser rejects stale epochs and unexpected process identities.

| Direction | Message | Meaning |
| --- | --- | --- |
| Renderer → browser | `NavigateIntent` | Candidate URL, source document token, user activation, history behavior, referrer and sandbox flags; browser validates and starts the navigation. |
| Browser → network | `StartNavigation` / `RestartNavigation` | Browser-approved fetch context. A redirect restart stays in the same epoch; a new navigation advances it. |
| Browser → renderer | `CommitNavigation` | Final response and new entry/document tokens; renderer creates the document only for the assigned site/process. |
| Renderer → browser | `HistoryUpdate` | Same-document URL, serialized state and scroll mode; browser validates same-origin and assigns the step. |
| Browser → renderer | `FreezeDocument` / `RestoreDocument` | Freeze for bfcache, or restore a known live document token with persisted state and event ordering. |
| Renderer → browser | `FreezeAck` / `RestoreAck` | Confirm quiescence or activation; timeout causes eviction and reload. |
| Browser → renderer | `DiscardDocument` | Drop a frozen document and its renderer-side resources. |
| Browser → renderer | `TraverseToEntry` | Activate same-document entry or request a cross-document restore under a specific epoch. |

Redirect heads come from the network role through the browser. The browser
resolves and validates `Location`, applies method rewrite and hop limits,
updates redirect tainting, and issues a new request under the same navigation
epoch. The renderer never chooses the trusted redirect context. The proposed
wire names are descriptive; protocol numbers and field layouts require a
separate review before implementation.

### Bfcache

For an eligible cross-document departure, the browser asks the old renderer
to freeze before committing the replacement. The renderer finishes the HTML
page transition ordering, reports `pagehide`/visibility state, pauses event-loop
tasks, timers and network observers, and acknowledges a quiescent document.
The browser retains the process and document token but removes its active
presentation and input authority. Restore revalidates process/site identity,
delivers persisted state and `pageshow`/visibility transitions in HTML order,
then resumes tasks. The [HTML document lifecycle](https://html.spec.whatwg.org/multipage/document-lifecycle.html)
requires the intent to keep a document in bfcache to be false when that
document or a descendant is not salvageable.

The initial eligibility policy is conservative: no pending navigation or
unacknowledged IPC, no active WebSocket/WebRTC/media capture/device grant,
no in-flight storage transaction, no unload-dependent state, no mismatched
site isolation or sandbox policy, and every child navigable eligible. These are
project policy gates, to be relaxed only with evidence; they are not claimed
as the complete HTML eligibility algorithm. A frozen entry holds a bounded
per-tab and global memory charge. Least-recently-used frozen documents are
evicted first under pressure, on process crash, on security-policy change, or
when a page becomes ineligible. The history entry remains and is repopulated
through network on traversal. A crash must never leave a history step pointing
to an activatable dead renderer.

The browser saves scroll positions and optional persisted form state before
freeze or discard, then restores them under the entry's scroll-restoration
mode. This follows the [HTML persisted-state steps](https://html.spec.whatwg.org/multipage/browsing-the-web.html)
and must be tested separately from heap-preserving bfcache restore.

## Alternatives and review gates

A per-renderer history stack loses ordering across child navigables and process
swaps. A browser-only serialized DOM/heap snapshot would duplicate JS runtime
state and cannot preserve live task semantics. The proposed model keeps the
history authority in the browser while freezing eligible live documents.

Before adoption, review the transaction ordering with HTML and DOM owners;
add pinned history/navigation WPT selections for push, replace, fragment,
cross-origin traversal, iframe traversal, redirected navigation, scroll and
form restoration, pagehide/pageshow, eviction, and renderer/browser crash.
Measure memory and restore latency against reload, fuzz every new history
message, and verify stale-token/epoch rejection and site isolation on both
Windows and Linux. Enable bfcache only after the lifecycle and security gates
pass on both platforms.
