# IPC-10 — process-backed navigation adoption

Base (this run): `lane/ipc` `1b92ad5f` on dev `0d6b60b0` (COMP-13b/COMP-14 compositor
fix and the JSRT sparse-array fix merged). In-flight checkpoint was `ad74b323` on
dev `34049fcc`. Pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

`navigator.rs` owns one controller per tab for direct and opt-in process navigation
(`AMAZING_IPC_HISTORY=1`). It routes top-level loads, manual redirects, same-document
history changes, traversal, reload, browser-minted iframe navigables, and scroll
snapshots. `gui::App` and the headless WPT runner enter this controller. The WPT
runner serves pinned fixture bytes over a bounded loopback HTTP server, so the IPC
mode fetches the page and child frames through `NetworkSession`; the direct mode uses
`amazing_net`. The option defaults off.

The navigator stores JSS-SC `SerializedValue` history.state, encodes a versioned
graph into the existing bounded `Option<Vec<u8>>` IPC snapshot field (`state_wire`,
version 1), and deserializes state in the active JS realm on traversal. A wire/realm
test covers cycles and repeated references, and a bad-reference test rejects forged
graphs. `runtime_host_ipc.rs` owns the process adapter (browser-minted tickets,
renderer/document tokens, freeze/restore, eviction, renderer crash restart, manual
redirects under one epoch, child-navigation context).

## Pinned history selection

All paths are under `html/browsers/history/the-history-interface/`. Fixed-base rerun
2026-09-26 on venus-001; mode 0 = direct, mode 1 = `AMAZING_IPC_HISTORY=1`.
Raw per-file JSON: [ipc-10-parity/](ipc-10-parity/) (all `.err` files empty — no
process `Network(Transport)` flake). Machine-readable tsv: [ipc-10-history-linux.tsv](ipc-10-history-linux.tsv).

| File | direct | IPC |
| --- | --- | --- |
| `history_pushstate.html` | pass | pass |
| `history_replacestate_nooptionalparam.html` | pass | pass |
| `history_back.html` | timeout | timeout |
| `history_go_zero.html` | fail | fail |
| `history_state.html` | pass | pass |
| `history_pushstate_url.html` | fail | fail |
| `iframe_history_go_0.html` | fail | fail |

Direct **3 pass, 3 fail, 1 timeout**; IPC **3 pass, 3 fail, 1 timeout**. Zero
classification change on/off and no intermittent transport error. The four
non-passing cases are engine/DOM gaps shared by both modes (`history_back.html`
awaits `popstate` state, `history_go_zero.html` uses `window.open`,
`history_pushstate_url.html` expects synchronous URL rewriting,
`iframe_history_go_0.html` needs a child document realm); they are not IPC gaps.

An earlier preliminary Windows run reproduced the intermittent process
`Network(Transport)` under three concurrent win-001 render builds; the worker-specific
`AMAZING_HTTP_CACHE_DIR` isolation and bounded concurrent fixture server are in place.

## Linux gates (venus-001, exact source `1b92ad5f`)

| Command | Result |
| --- | --- |
| `cargo nextest run -p amazing -p amazing-process` | 240/240, 6 skipped |
| `cargo nextest run -p amazing-compositor` | 62/62, 0 skipped |

Earlier on the pre-compositor-fix tree `7cc90146`: `amazing-process` 52/52 (3 skipped),
`amazing` 186/186 (3 skipped; bin-only crate, no doctests).

History-message decoder/dispatcher fuzz (`ipc-fuzz`, 180 s each, corpus + all
`HistoryActivate`/`HistoryFreeze`/`HistoryRestore`/`HistoryDiscard`/
`HistoryLifecycleEvent`/`HistoryAck` targets):

```
target=decoders    seconds=180 cases=14,995,918  panics=0
target=dispatchers seconds=180 cases=21,964,941  panics=0
```

## Windows gate

`scripts/win-run.sh ipc -- 'cargo nextest run --workspace --locked --no-fail-fast; cargo test --workspace --locked --doc'`
on win-001 (win-002 offline; `win-run.sh` fallback), exact source `1b92ad5f`:

- workspace nextest: **2471/2471 passed**, 8 skipped, exit 0 (zero failures).
- workspace doctests: **2/2 passed**.

An earlier run on the pre-compositor-fix tree `a886dbed` failed two
`amazing-compositor::paint_view_pages` cases
(`paint_view_owner_mapping_binds_dom_properties_to_tree_nodes` 4 vs 3 bindings;
`retained_real_page_property_trace` scroll rerasterized 2 vs 0). Merging dev
`0d6b60b0` (COMP-13b/COMP-14 root-viewport binding assertions) resolved both;
compositor is 62/62 on Linux and green in the workspace run above.
