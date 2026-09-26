# RC-24 painter blend/isolation import

Fixed source base `7356971c` includes painter RC-24 `08b0d2d4`.
Linux Boat pool-paint, compositor's isolated target
`/home/user/amazing/targets/compositor`. Windows deferred.

`paint_view_metadata` retains each non-root painter effect's CSS blend and
isolation on the corresponding compositor effect ID. Opacity and parent IDs
remain unchanged; neutral operations stay absent. All 17 CSS blend values use
the existing exhaustive converter. Filters/backdrop/clip/mask are not imported.

## Measured validation

- Compositor: **83 passed**, 0 skipped, 46.865 seconds; 0 doctests.
- Workspace: **2765 passed**, 10 skipped, 60.682 seconds.
- Workspace doctests: **4 passed**. Final command exit **0**.
- Existing IA corpus: 3 pages, **1/4/1 layers**, **262144 exact pixels/page**,
  **0 rounding, 0 losses**.
- Existing four-file CSS value corpus: **37 resolved, 14 explicit fallbacks,
  0 failures**.
- New painter corpus: 17 modes × 5 contexts = **85 cases**.
  Opacity-only import **53/85 passing**, **18432 loss pixels** →
  blend/isolation import **83/85 passing**, **1152 loss pixels**.
  **30 gained cases, 0 lost cases**, **0 unexpected differences**.
  After import: **344128 exact**, **2880 allowed layer-rounding** and
  **1152 known reference-difference pixels** across 348160 pixels.
  Layer counts range **1–3**. All **85 cached frames** equal their initial
  frame without rerasterizing.

The before mode is a controlled reconstruction of the old importer on the same
RC-24 painter: clear only imported EffectOperations before the first frame,
leaving opacity, ancestry, chunks and raster inputs intact. It is not a
separate old-commit run. CSVs: `paint-blends-before.csv`,
`paint-blends-after.csv`. The cases cover opaque color, source alpha, ancestor
opacity, isolation, and isolation plus translation. These are curated native
comparisons, not unmodified WPT.

## Two exact reference discrepancies

- Color-burn opaque: background [90,170,210], source [180,70,130].
  Compositor [21,0,167,255], painter [22,0,167,255], **576 pixels**.
  The painter truncates integer division; the compositor rounds its float result.
- Plus-lighter alpha: same colors, source alpha 128.
  Compositor [180,205,255,255], painter [173,205,233,255], **576 pixels**.
  Painter uses source-over plus a saturated straight-color sum; native compositor
  uses the premultiplied additive operator.

Both are routed to replaced-content in lane questions. Regression checks permit
only those exact differing pixel pairs in those exact cases. The global parity
classifier is unchanged; all other differences fail. No zero-loss full-painter
claim is made. Source blend/isolation mapping and cached frames are complete.

## Reproduction

Via `scripts/linux-run.sh compositor -- '...'`, in one batch:

```sh
cargo build -p amazing-compositor --examples --locked &&
cargo nextest run -p amazing-compositor --locked --no-fail-fast &&
cargo run -p amazing-compositor --example paint_blends --locked -- --legacy &&
cargo run -p amazing-compositor --example paint_blends --locked &&
cargo run -p amazing-compositor --example effects_trace --locked &&
cargo run -p amazing-compositor --example css_effects_corpus --locked -- crates/amazing-compositor/fixtures/comp15 &&
cargo nextest run --workspace --locked --no-fail-fast &&
cargo test --workspace --locked --doc
```

Scratch log: `.agents/compositor/comp15-paint-blends-final.log`.
