# IPC-9 session history and bfcache evidence

Base: `763faf778903b1f4a1bb6b8915598b5a85f22909` (local dev, including
IPC-8 and accepted ADR 0012). The driver is not enabled by this ticket; IPC-10
will adopt the model behind a default-off option after this series merges.

## Implemented contract

- `SessionHistory` mints tab-scoped navigation, entry, step and document IDs.
  It serializes mutations, carries child navigables in each traversable step,
  clones prior entries for same-document push/replace, prunes forward steps,
  and distinguishes reload from traversal. Same-document URLs require the
  current origin; renderer identity and source document are checked before
  accepting a ticket or snapshot. Cross-site commits accept a browser-selected
  destination renderer. Its queued traversal path preserves the HTML Standard's
  [`history.back()` then synchronous fragment navigation example](https://html.spec.whatwg.org/multipage/browsing-the-web.html#centralized-modifications-of-session-history).
  Unreachable entries are pruned and their document tokens are returned for
  browser-issued discard.
- `HistorySnapshot` bounds history.state, Navigation API state, persisted user
  state, and scroll positions. The browser holds snapshots with entries.
- Bfcache freezing checks the conservative eligibility flags, process token,
  document epoch, quiescence acknowledgment, pagehide/visibility ordering and
  a five-second acknowledgment deadline. The event order follows the
  [HTML document unload](https://html.spec.whatwg.org/multipage/document-lifecycle.html#unloading-documents)
  and [reactivation](https://html.spec.whatwg.org/multipage/browsing-the-web.html#reactivate-a-document)
  algorithms. Restore requires a current-step
  document, matching origin/process identity, and visible/pageshow ordering.
  Eviction is least-recently-frozen under a count, byte and time budget; a
  renderer crash marks its documents discarded so traversal reloads them.
- IPC interface 14 carries activate/freeze/restore/discard. The renderer demo
  binds it before bootstrap acknowledgment and rejects input while all known
  documents are frozen or discarded.

## Tests and fuzz

The eight engine/process tests cover child navigables, push/replace/traverse/
reload, forward pruning, forged tickets and cross-origin rejection, bounded
snapshots, process swap, renderer freeze/restore event order and input
quiescence, timeout, count and memory eviction, crash cleanup, and the HTML
queued traversal race.

| Host | Selection | Result |
| --- | --- | --- |
| venus-001 | focused history nextest, final source | 8/8 passed |
| win-002 | amazing-process nextest | 40/40 passed, 0 skipped |
| win-002 | amazing-process doctests | command passed, 0 doctests |
| win-002 | workspace nextest | 2,266/2,266 passed, 5 skipped |
| win-002 | workspace doctests | 2/2 passed |
| venus-001 | decoder fuzz, 120 seconds, 51 targets | 6,437,330 cases, 0 panics |
| venus-001 | dispatcher fuzz, 120 seconds, 11 targets | 13,979,329 cases, 0 panics |

The integration handoff in `.agents/ipc/questions.md` was superseded by the
primary's IPC-10 ownership decision. IPC will implement the default-off driver
adapter; integrator-assist will add only the exact navigation hook.

Final logs: `/tmp/ipc9-linux-final-focused.log`, `/tmp/ipc9-win-final.log`,
and `/tmp/ipc9-linux-fuzz.log` on the Mac. The Windows gate extracted the
worktree afresh on win-002 from the fixed `763faf7` base.
