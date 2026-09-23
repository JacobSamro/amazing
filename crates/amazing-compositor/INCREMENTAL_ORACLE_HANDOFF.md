# ADR 0010 rendering-update oracle handoff

Integrator-assist owns the coordinator. Run the incremental and full pipelines
against the **same frozen DOM epoch** after scripts and mutation observers have
settled for that rendering update. Do not replay the mutation or execute script
twice. Keep the previous incremental caches, then compute a fresh full result
from the current DOM as an oracle. A mutation arriving during either computation
belongs to the next epoch.

## Inputs and sequence

1. Capture the DOM mutation journal and stylesheet, viewport, font and resource
   epochs. Record generated node identity as arena index plus generation.
2. Feed the journal to CSS invalidation; propagate style changes to layout;
   feed geometry/style changes to paint; pass changed stable paint keys and
   local old/new ink bounds to `Compositor::apply_paint_delta`. A
   `RequiresFullRebuild` result selects the existing full scene path.
3. Run the existing full style → layout → paint pipeline on the same DOM epoch
   into separate scratch caches. Build a full compositor frame at the same
   viewport and property-tree state. Do not let the oracle mutate the live
   retained caches or dispatch events.
4. Compare before presentation. On mismatch, present the full result, record
   the first divergence and increment a parity-loss counter. Keep the
   incremental result and inputs for diagnosis. On match, present the
   incremental result and retire resources according to its frame delta.

## Ordered comparison

- Computed styles by generated DOM node identity and property, including
  inherited values and pseudo-elements.
- Layout fragments, border/ink/scroll overflow, line boxes, containing-block
  geometry and hit-test regions, with a first differing node/property report.
- Paint item order, stable keys, property state, local and visual bounds, and
  resource identities. Key/index remaps and structural changes take the full
  scene fallback until retained chunk updates can express them.
- Final RGBA bytes at the same viewport, scale factor, color space and font
  environment. Report first mismatching pixel and an image diff. Also compare
  hit testing at changed ink boundaries and overlapping stacking contexts.

The diagnostic mode should count style, layout and paint calls, dirty nodes,
chunks and tiles, tile uploads, full fallbacks, cache bytes, and mutation-to-frame
p50/p95. A transform/opacity/scroll-only update must report zero upstream
style/layout/paint calls and zero tile uploads; the compositor's synthetic
property-only test checks the lower boundary, while the coordinator must prove
the live engine path. Use the fixed article and effects fixtures from COMP-7,
then add structural selectors, text reflow, floats/flex/grid, overflow,
replaced content, shadow/slot changes, overlapping opacity, and old/new damage
crossing tile edges. Require exact pixels before timing targets are accepted.
