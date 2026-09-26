# COMP-15 CSS values and additive compositing

Source: compositor working tree on fixed base `4c5e877f` (includes local dev
`a24bb0b0`). Linux Boat pool-paint. Windows deferred.

The adapter resolves CSS filter lists, all blend modes, isolation and backdrop
bounds atomically. Relative lengths use the producer callback. URL/calc values
and shape/mask mutations explicitly require fallback. The driver rejects effect
deltas before committing any accompanying opacity or cached style change until
the painter supplies reference-space metadata. Plus-lighter adds premultiplied
colors and alpha with saturation; tests cover 500 independent integer-reference
cases and retained snapshot/cached-frame execution.

Measured targeted batch on preceding base `13e66a57`: 620 passed, 4 skipped
(82 compositor tests; previous native gate 76). All four HTML fixtures traversed:
37 resolved declarations, 14 explicit fallbacks, 0 failures; see
`linux-css-values.csv`. These are curated value-adoption cases, not WPT results
or full CSS pixel parity.

The IA reflow/effects-scroller/article corpus before this change
(`linux-before-layer-parity.csv`, source `27de5230`) and after
(`linux-css-layer-parity.csv`) has 1/4/1 layers; each page has 262144 exact pixels,
0 rounding, 0 losses. The new plus-lighter fixture is covered by the value corpus
and independent native pixel assertions, not the opacity-only painter oracle.

Final workspace gate: **2723 passed, 10 skipped**, 59.154 seconds;
**4 doctests passed**, exit 0. Compositor **82 passed**, 0 skipped,
0 doctests. Both corpora reran with the same numbers shown above.

Command (via `bash scripts/linux-run.sh compositor -- '...'`):
```sh
find crates -type f \( -name "*.rs" -o -name Cargo.toml \) -exec touch {} +
cargo nextest run --workspace --locked --no-fail-fast &&
cargo test --workspace --locked --doc &&
cargo run -p amazing-compositor --example css_effects_corpus --locked -- crates/amazing-compositor/fixtures/comp15 &&
cargo run -p amazing-compositor --example effects_trace --locked
```

Infrastructure diagnosis: initial workspace example compilation reused
`libamazing_compositor-e0177d8bbd1292d1.rlib` without the new CSS exports.
Remote source SHA256 matched local, but the artifact's dependency file omitted
`css_effects.rs`; Cargo marked only the new example dirty. Source mtimes were
refreshed before the final gate, without deleting the shared target. The runner
owner has the pooled-cache invalidation request in lane questions.

Full COMP-15 remains pending independent painter effects/reference spaces and
installed-driver/IPC adoption. This evidence does not establish those exits.
