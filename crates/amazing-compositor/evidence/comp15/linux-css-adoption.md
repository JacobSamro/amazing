# COMP-15 CSS values and additive compositing

Implementation `7e6601ea`; final gate source `3c6295d7`, after merging dev
through `9719506f`, including target isolation fix `86c44031`.
Linux Boat pool-paint, sandbox `bx_zk8gduf5`, verified
`CARGO_TARGET_DIR=/home/user/amazing/targets/compositor`.
Windows deferred. No builds/tests ran on macOS.

## Authoritative isolated-target gate

- Workspace: **2741 passed, 10 skipped**, **63.653 seconds**.
- Workspace doctests: **4 passed**.
- Compositor subset: **82 passed**, 0 skipped, 0 doctests.
- Four curated HTML fixtures: **37 resolved values, 14 explicit fallbacks,
  0 failures** (`linux-css-values.csv`).
- IA reflow/effects-scroller/article: **1/4/1 layers**, each **262144 exact
  pixels**, **0 rounding, 0 losses** (`linux-css-layer-parity.csv`).
- Command exit: **0**.

One `bash scripts/linux-run.sh compositor -- '...'` invocation ran:
```sh
cargo nextest run --workspace --locked --no-fail-fast &&
cargo test --workspace --locked --doc &&
cargo run -p amazing-compositor --example css_effects_corpus --locked -- crates/amazing-compositor/fixtures/comp15 &&
cargo run -p amazing-compositor --example effects_trace --locked
```

The IA before corpus (`linux-before-layer-parity.csv`, source `27de5230`)
was measured on the earlier dedicated compositor sandbox `bx_x5x48teg`,
before pooled targets were introduced. Its same layer/pixel counts are unchanged.
Compositor tests increased from that native gate's 76 to 82.
The fixture value corpus is newly introduced; there is no baseline value-adapter
result. It does not measure unmodified WPT or full CSS pixel parity.

## Implementation and limits

The adapter resolves CSS filter lists, all 17 blend modes, isolation and backdrop
bounds atomically. Relative lengths use the producer callback. URL/calc values
and shape/mask mutations explicitly require fallback. The driver rejects effect
deltas before committing any accompanying opacity or cached style change until
the painter supplies reference-space metadata. Plus-lighter adds premultiplied
colors and alpha with saturation; tests cover 500 independent integer-reference
cases and retained snapshot/cached-frame execution.

The new plus-lighter fixture is covered by the value corpus and independent
native pixel assertions, not the opacity-only painter oracle. Full COMP-15
remains pending independent painter effects/reference spaces and installed
driver/IPC adoption.

## Superseded pooled results

The earlier shared-target workspace 2723, targeted 620 and doctest results are
withdrawn as gate evidence. Shared workspace-relative artifact names allowed
one lane to reuse another lane's compiled library. Source hashes matched while
the reused compositor dependency file omitted its new CSS module. Refreshing
timestamps appeared to repair that run but did not establish isolation.

The authoritative run above cold-built in compositor's own target after
`86c44031`; it used no timestamp workaround or shared-target cleanup.
Scratch log: `.agents/compositor/comp15-isolated-final.log`.
