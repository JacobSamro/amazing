# CPU quad sampler source preparation

Recorded base: `09e3a5eb1c8628cfce9c669a864d22a05ea5c857`.
Linux lane qualification complete on the recorded base; root combined product and merge gates remain pending.

The retained CPU presenter previously resolved clip and scroll ancestors and
composed their matrices for every destination pixel. It now prepares those
immutable predicates once per quad. Affine plane solves cache coefficients and
the determinant, preserving the original numerator arithmetic and divide order;
projective solves still call the original `Matrix4::unmap`. Rounded corner
predicates and root/nested scroll ownership are unchanged. Fixed positioning
still ignores scroll-container clipping. Preparation is recreated on each draw,
so a new scroll/transform frame cannot reuse stale sampling state.

Tile sampling caches only a tile whose layer/coordinate lookup matches the
quad's resource ID. Neighboring bilinear taps retain the original tile lookup,
transparent-outside and premultiplied interpolation rules. RasterSampling,
local effect pass grids, masks, opacity, layer-boundary accounting and public
Presenter/CompositorFrame APIs are unchanged. No alternate shell renderer.

All original compositor tests/fixtures remain. New native controls compare
prepared vs original point solves and visibility across affine/projective,
singular, rounded, nested scroll, fixed/sticky and updated-tree cases, plus
negative/mismatched/neighboring tile lookup parity. The cached_quad_trace example
reports native elapsed times and exact direct-render pixel equality at DPR
1/1.25/2/3. It is a lower-level bilinear trace, not a substitute for the original
product Q17 real-NewTab/Find DPR1/DPR2 control or its unchanged eight-second bound.

Qualification when native capacity is available: whole workspace nextest and
docs, all compositor integration tests (including existing nearest/bilinear,
device-raster, cached/snapshot, rounded visibility and filter/blend controls),
whole paint_blends / paint_filter_parity / css_effects_corpus, mutation_trace,
and cached_quad_trace in one bulk command per side. Compare classifications,
pixel losses and native timing without weakening existing oracles. Use matching
benchmark-only overlay on the recorded baseline, supported public Linux wrapper,
finite default4vCPU/8GiB TTL7200, native scratch targets and captured `/srv` proof.
Raw and all measured ELFs stay native; compact summaries only. Root's original
Q17 product gate proves the installed consumer after coherent composition.

## Measured Linux qualification

Tested source `d812fc12843f330684e5c9aff07cd56867cac8fd` against
matching trace-only baseline `d6a99d011ed67de7f417a84c2c213d8215567d04`.
Assigned existing box was xlarge16vCPU/32GiB (native MemTotal33,652,166,656B),
CargoJ4 both sides, identical line-tables-only dev/test and incremental0.
Native/wrapper exit0, docs39 each; workspace5482→5485PASS,17SKIP,0FAIL,
zero lost identities. All three authored mapper/visibility/tile controls pass.
Whole corpus rows unchanged: blend83/85PASS with the original two known
mismatches/1152pixels retained; filter parity32/32, CSS effect4/4,
mutation trace40/40 and cached trace4/4, zero lost passes.

Exact cached pixels at DPR1/1.25/2/3; release median presentation at DPR2
155,722→45,879µs (3.394×). This lower trace does not qualify Q17's installed
window8s interaction by itself. Product consumer composition keeps that
original control and deadline. Native raw/input snapshots and all643ELFs
per side remain under `/srv/amazing-proof/compositor-quad-sampler-reuse-20261006`;
retention readback0 each, recoverable private targets retired afterward.
Initial combined compact fetch rejected duplicate basenames (exit3); one
corrected split public fetch returned0 each, no engine rerun. Compact
counts, passsets, exact lost lists and raw/native paths+hashes are in
`crates/amazing-compositor/evidence/quad-sampler/`.
