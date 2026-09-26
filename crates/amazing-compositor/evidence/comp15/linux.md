# COMP-15 native recovery: Linux final gate

Final source `642ffc6993a39fbf159b2377c3c1e7fcf574dfe9`, halo fix `5196e881`,
local dev base `4518bea7`. Boat sandbox `bx_x5x48teg`, lane compositor.
Linux is the authoritative gate under the 2026-09-26 Linux-first rule.
No builds or tests ran on macOS; Windows was deferred.

- Workspace nextest: **2526 passed, 10 skipped**, no failures (54.912 seconds).
- Compositor subset: **76 passed**, no skipped tests.
- Workspace doctests: **2 passed**, no failures; compositor **0 doctests**.
- Final command exit: **0**.

Commands used `scripts/linux-run.sh compositor --ref <sha>`:

```sh
cargo run -p amazing-compositor --example effects_trace --locked
cargo nextest run --workspace --locked --no-fail-fast
cargo test --workspace --locked --doc
```

Before corpus source `27de52307cb32d55178fde937030c802a9a81195`; after source
is the final source above. Both runs: IA reflow / effects-scroller / article
layers **1/4/1**, each **262144 exact pixels, 0 layer-rounding, 0 losses**.
See `linux-before-layer-parity.csv` and `linux-after-layer-parity.csv`.
The corpus uses 512×512 fallback-text full-painter comparisons. No lost passes.

The halo regression adds coverage for cumulative ancestor reservations,
independent siblings, nonshrinking endpoint envelopes and source-free cached
frames. The native area is ready for integration. The full COMP-15 CSS-effect
fixture gate is still unavailable: painter metadata exports opacity only.
CSS registration, resolved effect reference spaces, painter reference effects,
and driver/IPC adoption remain owner dependencies; no full CSS parity claimed.

Scratch logs: `.agents/compositor/comp15-linux-before.log` and
`.agents/compositor/comp15-linux-final.log`.
