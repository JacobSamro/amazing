# DOM-B-NEXT: reflection-page timeout fix, plus HTML-35 (window named access, script.async)

Implementation on fixed base `b72bc231`. Lands the coordinator's ordered list:

1. **Fix the `html/dom/reflection-*.html` timeouts** that were making WPT numbers
   noisy.
2. **HTML-35 item 1: named access on `window`** (`window.<id>`, HTML SS7.2.2.3).
3. **HTML-35 item 2: `HTMLScriptElement.async`** reflects the "force async" flag,
   not a plain boolean content attribute.

## 1. Reflection-page timeouts: root cause and fix

Three compounding harness bugs in
`crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`, not an engine
performance regression:

- **`wai-aria/manual/*.html` (233 files) were wrongly selected.** These pages
  reference `testharness.js` but never call `done()` -- they exist for human
  interaction. The naive `'testharness.js' in path.read_text()` selection rule
  included all of them, so each burned its full per-test deadline for nothing.
  Fixed by excluding any path containing `/manual/`.
- **The script ran the *debug* binary.** `cargo build -p amazing` (dev
  profile) is 4-5x slower than `--release` for this CPU-bound work, which was
  most of why the reflection pages (thousands of subtests each) blew a 20s
  budget. Confirmed release is behaviorally equivalent for WPT purposes: no
  `debug_assert!`/overflow-check dependence in the affected crates. Fixed by
  copying `release/amazing` in the harness instead of `debug/amazing`.
- **The per-test deadline itself was too small** for pages that legitimately
  need tens of seconds even in release (worst measured: `reflection-text.html`
  at 84.2s solo). Fixed by adding `--timeout-ms` (default 25000, unchanged for
  ordinary pages) and a separate `--heavy-timeout-ms` (150000, ~1.8x margin
  over the worst release-build measurement) + `--heavy-workers` (1, avoids CPU
  contention from several heavy pages racing on a 4-vCPU box) for the
  `^html/dom/reflection-[^/]+\.html$` pages specifically.

Result: **0 of the 9 `reflection-*.html` pages time out** on either
`before.json` or `after.json` runs (both use the fixed script). The corpus's
21 remaining file-level timeouts (identical on both sides -- see "Measured"
below) are a separate, real, already-existing cluster: render-blocking,
partial-updates, `document.currentScript`/`readyState` features that are
simply not implemented and hang waiting for events that never fire. Not
touched by this ticket.

## 2. Window named access (HTML SS7.2.2.3)

- `crates/amazing-dom/src/window_names.rs` (new): `Dom::window_named_objects
  (document) -> Vec<(String, Vec<NodeIdx>)>`, a tree-order scan matching the
  spec's supported names -- `name` of `embed`/`form`/`img`/`object`, plus `id`
  of any element -- with same-node-same-key dedup. **Scope cut**: no nested
  browsing-context/iframe-by-name support (this engine has no content
  navigables yet); documented at the call site.
- `Dom::mark_named_objects_dirty`/`take_named_objects_dirty` (a plain bool):
  set from the four real mutation choke points -- `set_attribute_ns`/
  `remove_attribute_ns` for `id`/`name` (`mutation.rs`), `insert_internal`/
  `remove_internal` for tree membership (`tree.rs`).
- `crates/amazing/src/runtime_binding_host.rs`'s `with_dom` -- the one choke
  point every DOM-mutating binding call passes through -- checks the dirty
  flag (one bool per call in the common case) and calls the new
  `refresh_window_named_properties` only when it fired.
  `refresh_window_named_properties` recomputes the named-object list and
  redefines the global object's properties: a single match wraps to that
  node; multiple matches wrap to a `STATIC_NODE_LIST` (a pragmatic
  simplification over a true live `HTMLCollection`).
- **Correctness fix found by the corpus gate, not by inspection**: the first
  version unconditionally defined a data property for every match, which
  clobbers any pre-existing global binding with the same name. `<div
  id="test">` is routine in WPT fixtures and overwrote testharness.js's own
  `test()` function outright, turning `document.images.html` and several
  `nameditem-*.html` tests into hard `TypeError`s (55 lost subtests measured
  on the first corpus diff). Per spec, named access is a `[[Get]]` fallback
  consulted only when ordinary property lookup finds nothing -- it must never
  shadow something already there. There is no engine-level `[[Get]]` hook for
  `window` available to this lane (`amazing-js`'s value model is outside its
  granted paths), so the fix approximates the fallback rule: track which keys
  were defined by the previous refresh, and skip (re)defining a name if it
  newly collides with a property this mechanism doesn't already own. Re-ran
  the corpus after the fix: **0 lost subtests**.
- No change to `amazing-html`'s parser needed: `tree.rs::insert_element`
  (used for parsed elements) is structurally separate from
  `create_element_local_in`/`create_element_ns_in` (used by
  `document.createElement`), so this only needed changes in `amazing-dom`.

## 3. `HTMLScriptElement.async` / "force async" flag

- `crates/amazing-dom/src/lib.rs`: new `Node.force_async: bool` field.
- `crates/amazing-dom/src/namespace.rs`: `init_force_async` (called from
  `create_element_local_in`/`create_element_ns_in`, i.e. `createElement`/
  `createElementNS` -- initializes the flag `true`, per spec), plus
  `force_async`/`set_force_async` accessors. Parser-inserted `<script>`
  elements go through `amazing-html`'s `tree.rs::insert_element`, a different
  primitive that never calls `create_element_local_in`/`create_element_ns_in`,
  so they correctly default to `force_async = false` (parser default) with no
  html-lane change needed.
- `crates/amazing-bindings/src/element_reflection.rs`: the `async` getter
  returns `true` if the flag is set OR the content attribute is present;
  the setter clears the flag before applying the boolean-attribute logic
  (spec: setting `.async` always clears "force async" first).
- New test: `crates/amazing-bindings/src/tests.rs::html_35_script_async_reflects_force_async_flag`
  -- covers `createElement`-initial-true, setter-clears-flag, attribute
  reasserting true, and the low-level `create_element` (parser-style) path
  defaulting to false.

## Measured

Linux venus-001 (build/test host; the k-test gate died mid-run on this ticket
-- see below), isolated `dom-bindings` lane tree, fixed base `b72bc231`.

Workspace **3466 passed / 17 skipped / 0 failed** (`cargo nextest run
--workspace --locked --no-fail-fast`); `cargo test --workspace --locked --doc`
all `ok`, including `amazing_js` and `amazing_html`'s 1/1 doctest suites.
`amazing-bindings` crate: **123/123** (includes the new script.async test).
Combined `amazing-dom` + `amazing-bindings` + `amazing` crate run: **736/736
passed, 6 skipped**.

WPT `html/dom` + `wai-aria` + `web-animations/interfaces/Animatable` (whole
directories), pinned `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, before =
fixed base `b72bc231` rebuilt via `--ref` (using that commit's own,
*unfixed* `run-corpus.py`, deliberately -- this measures the actual
before/after harness-bug delta too), after = this candidate:

| | files selected | files timing out | pass subtests | fail subtests |
|---|---|---|---|---|
| before | 502 | 262 | 35336 | 9678 |
| after | 269 | 21 | 47347 | 12528 |

**262 -> 21 file-level timeouts.** Exact accounting: 233 (`wai-aria/manual/*`,
now correctly excluded) + 8 (`reflection-*.html`, now fixed by the release
binary + heavy-timeout split) + 21 (unchanged, pre-existing, unimplemented
render-blocking/partial-updates/`currentScript`/`readyState` cluster) = 262.
The 21 remaining timeouts are byte-for-byte the same file list before and
after (one, `bypass-cache-revalidation.html`, wobbles `fail` vs `timeout`
across runs right at the deadline boundary -- unrelated to anything this
ticket touched, a pre-existing unimplemented image-cache-revalidation
feature).

**Subtest diff (Counter-based, `compare-corpus.py`): 12011 gained, 0 lost.**
The gain is dominated by reflection pages now completing thousands of
previously-truncated subtests, plus the two features actually implemented.
Zero regressions.

`before.json`/`after.json` are the raw `run-corpus.py` reports (`after.json`
is the *post-shadowing-fix* run); `compare-corpus.py` is the before/after
diff tool, copied unmodified from `evidence/dom-b-reflect/`.

## Infra note: k-test gate died silently, redone on venus-001

The first full gate attempt ran on `hetzner-ktest` (`scripts/linux-run.sh
dom-bindings` with no `--host`, i.e. its default host) via a background SSH
session. It ran for 90+ minutes with zero visible progress; direct inspection
(`ps aux` on the box, remote output-directory timestamps) showed the release
build and corpus run had genuinely finished at ~11:07 UTC, but no
`cargo`/`nextest` process was running at all by 12:34 UTC -- the workspace
gate step had died with no error surfaced locally (a known failure mode of
this box's multiplexed SSH: see `.agents/dom-bindings/log.md`). Re-ran the
whole gate on `venus-001` instead (`--host venus-001 --no-slot`), which also
caught two real bugs the k-test run never got far enough to hit (the
`BindingHost` trait-impl placement error, `error[E0407]`/`E0599`; and the
named-access shadowing regression above) -- both fixed and re-verified on
venus-001, numbers above.

Also found and fixed in passing: venus-001's shared `/data` disk was at
100% (28 MB free on an 871 GB partition), which crashed the workspace-test
*linker* with SIGBUS (`ld terminated with signal 7`) on unrelated crates
(`amazing-compositor`, `amazing-process`) -- nothing to do with this ticket's
code. Freed 9.4 GB by deleting only this lane's own rebuildable
`debug/incremental` cache (`/data/amazing-target/dom-bindings/debug/incremental`,
safe per AGENTS.md disk-hygiene norms, touches no other lane's directory).
Every lane's target dir on that host is similarly sized (9.5-30 GB each); the
underlying disk-fullness is a cross-lane infra issue, flagged in
`questions.md`, not something this lane can fully resolve on its own.

## Reproduction

```
bash scripts/linux-run.sh dom-bindings --host venus-001 --ref b72bc231 --no-slot -- \
  'cargo build -p amazing --release --locked && mkdir -p "$OUT/dom-b-next-before" && \
   cp "$CARGO_TARGET_DIR/release/amazing" "$OUT/dom-b-next-before/amazing" && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py dom-b-next-before b72bc231-before \
     --reuse-binary --directory html/dom --directory wai-aria --directory web-animations/interfaces/Animatable'

bash scripts/linux-run.sh dom-bindings --host venus-001 --no-slot -- \
  'cargo build -p amazing --release --locked && mkdir -p "$OUT/dom-b-next-after" && \
   cp "$CARGO_TARGET_DIR/release/amazing" "$OUT/dom-b-next-after/amazing" && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py dom-b-next-after HEAD \
     --reuse-binary --directory html/dom --directory wai-aria --directory web-animations/interfaces/Animatable'

python3 crates/amazing-bindings/evidence/dom-b-next/compare-corpus.py before.json after.json
```

## Scope not covered here (queued)

- `Node-appendChild-script-in-script.html` (HTML-35's original motivating
  test) is **not** fixed by window named access -- direct inspection showed
  it doesn't use `window.<id>` at all. Its real failure is the HTML
  "children-changed steps" script-execution-model algorithm (the "already
  started" flag plus script preparation triggered by arbitrary DOM mutation),
  which does not exist anywhere in this codebase yet and is a materially
  larger feature, likely needing html/js-runtime collaboration. See
  `questions.md`.
- Window named access: no nested browsing-context/iframe-by-name support (no
  content navigables in this engine yet); multi-match case uses a
  `STATIC_NODE_LIST`, not a true live `HTMLCollection`.
- The 21-file render-blocking/partial-updates/`currentScript`/`readyState`
  timeout cluster: real, unimplemented features, unrelated to this ticket.
