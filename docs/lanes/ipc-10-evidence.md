# IPC-10 — process-backed navigation adoption

Base: `34049fcc` (NET-30 child navigation and JSS-SC). WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

`navigator.rs` owns one controller per tab for direct and opt-in process navigation (`AMAZING_IPC_HISTORY=1`). It routes top-level loads, manual redirects, same-document history changes, traversal, reload, browser-minted iframe navigables, and scroll snapshots. `gui::App` and the headless WPT runner enter this controller. The WPT runner serves pinned fixture bytes over a bounded loopback HTTP server, so the IPC mode fetches the page and child frames through `NetworkSession`; the direct mode uses `amazing_net`. The option defaults off. The navigator stores JSS-SC `SerializedValue`, encodes a versioned graph into the existing bounded `Option<Vec<u8>>` IPC snapshot field, and deserializes state in the active JS realm on traversal. A wire/realm test covers cycles and repeated references. The JS-owned synchronous `pushState` clone/error boundary is tracked with js-stdlib.

## Pinned history selection

All paths are under `html/browsers/history/the-history-interface/`. Raw Linux per-file results from the earlier NET-30 base are in [ipc-10-history-linux.tsv](ipc-10-history-linux.tsv). A preliminary Windows run that reproduced the transport flake is in [ipc-10-history-windows-preliminary.tsv](ipc-10-history-windows-preliminary.tsv). Both modes ran each test through the same `wpt-harness` entry point with isolated storage profiles and a 10-second deadline.

| File | Linux direct | Linux IPC |
| --- | --- | --- |
| `history_pushstate.html` | pass | pass |
| `history_replacestate_nooptionalparam.html` | pass | pass |
| `history_back.html` | timeout | timeout |
| `history_go_zero.html` | fail | fail |
| `history_state.html` | pass | pass |
| `history_pushstate_url.html` | fail | fail |
| `iframe_history_go_0.html` | fail | fail |

Earlier Linux total: direct **3 pass, 3 fail, 1 timeout**; IPC **3 pass, 3 fail, 1 timeout**. This proves no classification loss across the selected files. It does not establish conformance for the failing cases. `history_back.html` awaits `popstate` and its state value, `history_go_zero.html` uses `window.open`, `history_pushstate_url.html` expects synchronous URL rewriting, and `iframe_history_go_0.html` needs a child document realm; these remain engine/DOM gaps shared by both modes.

Fixed-base exact-source win-002 gate: amazing-process 54/54, amazing 187/187, workspace 2,413/2,413 (7 skipped); workspace doctests passed. The Windows on/off WPT classification remains pending. The WPT harness now gives each worker its own HTTP cache directory; previously `AMAZING_STORAGE_DIR` alone left network children on the shared `%LOCALAPPDATA%\Amazing\http-cache` path. Cache isolation alone did not eliminate intermittent `Network(Transport)` under three concurrent win-001 render builds. The fixture server now accepts bounded concurrent requests and waits up to the 15-second navigation deadline for request headers; the final parity rerun is pending a quiet host. Net owns a separate production shared-cache coordination review.
