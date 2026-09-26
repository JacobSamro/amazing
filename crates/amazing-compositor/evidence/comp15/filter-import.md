# Filter/backdrop producer adapter preparation

Fixed source base `431a0a04`. Linux Boat pool-paint, isolated
`/home/user/amazing/targets/compositor`; Windows deferred.

Prepared `PaintFilterSource` / `PaintFilterInput`,
`paint_view_metadata_with_filters` and `PaintViewSource::with_filters`.
The producer supplies resolved ordered native filter lists and backdrop border
bounds for existing effect IDs. Import preserves blend/isolation/opacity/ancestry,
snapshots input, validates the whole batch and rejects duplicate/root/unknown
IDs, invalid filters/bounds, local-space inputs and baked sources.

## Measured gate

- Compositor **86/86 passed**, 0 skipped, 46.987 seconds, 0 doctests.
- Workspace **2775/2775 passed**, 10 skipped, 60.020 seconds.
- Workspace doctests **4 passed**; exit **0**.
- Ten filter operations have imported-value/pixel/cached-frame checks.
- Backdrop invert has border-bound and cached-frame checks.
- Invalid input batches fail; empty input preserves the ordinary importer.
- Before (prior blend importer) and after existing corpora unchanged:
  blend **83/85**, **1152 known reference-difference pixels**, **0 unexpected**;
  four CSS fixture files **37 resolved/14 fallback/0 failures**;
  IA **1/4/1 layers**, **262144 exact pixels/page**, **0 losses**.
- Compositor tests increased **83 → 86**.

Bulk iteration used build + compositor nextest + complete blend/value/IA corpora.
Final invocation used workspace nextest with `--locked --no-fail-fast`,
workspace `cargo test --locked --doc`, then all three corpora.
Logs: `.agents/compositor/comp15-filter-import-bulk.log` and
`.agents/compositor/comp15-filter-import-final.log`.

## Not yet wired to painter filter metadata

At `431a0a04`, painter EffectNode still contains only parent/owner/opacity/
blend/isolate; GroupEffect also has no filter/backdrop values. No filter-only
groups or independent filter/backdrop reference renderer are available. This is
tested producer-boundary preparation, **not completed live CSS filter import**,
full-painter filter parity, or unmodified WPT coverage.

Replaced-content must expose filter-only groups, computed lists, current color/
length resolution, backdrop bounds and reference spaces. Only post-transform
raster-space inputs are accepted; arbitrary local CSS filters cannot be relabeled
as raster operations. The owner request and proposed trait are in lane questions.
