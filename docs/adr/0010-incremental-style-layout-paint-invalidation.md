# ADR 0010 — Incremental style, layout and paint invalidation

Status: draft for project review (2026-09-24)
Owner: compositor for the design; dom-bindings, css, layout, replaced-content and integrator-assist for implementation

## Context and target

The live-DOM path currently calls `amazing_css::compute` over the whole document,
rebuilds the layout box tree, and reconstructs the paint display list after a
mutation. `DisplayListView` and its property IDs live for one build. The
compositor retains tiles and property trees for scrolling, transform and
opacity, but a style or geometry change still rebuilds upstream stages. The
baseline mutation-to-frame trace on two existing real-page fixtures is recorded
in `docs/lanes/compositor_TODO.md`. This draft changes no locked execution
order: one rendering update remains DOM → style → layout → paint → compositor.

The required result is pixel and hit-test parity with a fresh full recompute,
including stacking order, overflow, text shaping and replaced content. Speed
is measured only after that parity gate. No DOM or CSS crate depends upward on
the compositor.

## Dirty data flow

1. **DOM journal (`amazing-dom`, dom-bindings owner).** Every checked tree,
   attribute, text, state and shadow-slot mutation increments an epoch and
   records the target plus old/new selector-relevant tokens. This journal is
   engine-internal; it is separate from script-visible `MutationObserver`
   delivery and is drained once at the rendering-update boundary. Node handles
   pair `NodeIdx` with a generation so a recycled arena slot cannot inherit old
   style or paint caches. A mutation during style/layout starts the next epoch.
2. **Style invalidation (`amazing-css`, css owner).** Compile selector features
   at stylesheet load into self, descendant and sibling invalidation sets,
   indexed by class, id, attribute and dynamic pseudo-class/state. Attribute
   old and new values both seed invalidation. Tree insertion/removal and
   `:has()`/structural selectors take conservative ancestor/subtree/sibling
   fallbacks until precise dependency tracking is proved. A stylesheet/media,
   font or viewport change has a separate whole-document or scoped epoch.
   Apply sets after coalescing the DOM journal, mark style recalc roots, and
   traverse only those roots, propagating inherited-property changes through
   descendants. Shadow scope and slot assignment are explicit boundaries, not
   implicit selector matches.
3. **Layout invalidation (`amazing-layout`, layout owner).** Diff old/new
   computed styles into `Repaint`, `Reposition`, `Relayout`, and `RebuildBoxes`
   severity. Maintain dirty bits on retained boxes keyed to DOM generation.
   Inline text/metrics dirty the containing line layout; intrinsic size changes
   propagate through dependent containing blocks, flex/grid tracks and floats.
   Layout containment and independent formatting contexts may stop propagation
   only after their size/overflow contracts are unchanged. A changed box's
   outgoing geometry invalidates following siblings and positioned descendants
   as required. Escalate uncertain dependencies to the nearest safe ancestor
   or full root.
4. **Paint invalidation (`amazing-paint`, replaced-content owner).** Assign
   stable item keys from DOM generation, paint phase and local item ordinal;
   compare old/new chunks, bounds and property state. Damage is the union of
   old and new visual bounds, expanded for antialiasing, shadows and filters.
   Reordered or unseparable stacking groups invalidate the whole affected
   group. Expose changed item ranges and resource lifetimes through an additive
   display-list delta. The compositor maps these to `invalidate_item`/tile
   damage and retains unchanged tile resources. Rebuilt views may remap stable
   keys without relying on old array indexes.
5. **Property-only path (`amazing-compositor`, compositor owner).** An
   unchanged transform or opacity effect updates the retained transform/effect
   node through `set_transform`/`set_opacity`, then builds a frame. Layout and
   paint stay untouched. If transform changes containing-block geometry, hit
   regions, or any dependent CSS layout input, classify it upstream instead.
   Scroll offsets use the retained scroll tree and only raster newly exposed
   tiles. Animated non-compositor properties return main-thread style updates
   and enter the normal dirty pipeline.

## Minimal seams by owner

| Owner | Additive API seam | Conservative fallback |
| --- | --- | --- |
| dom-bindings | `take_render_mutations(epoch) -> Vec<RenderMutation>` with node generation, old/new selector tokens and structural scope | Whole-document dirty for unsupported mutation kind |
| css | `StyleInvalidator::collect(mutations, stylesheet_epoch) -> RecalcRoots`; `recompute_roots(dom, old_styles, roots) -> StyleDelta` | Full `compute` for unknown selector dependency |
| layout | `LayoutTree::update(style_delta, text/replaced changes) -> GeometryDelta` with dirty boundaries | Existing full layout entry point |
| replaced-content | `DisplayListView::update(geometry_delta, style_delta) -> PaintDelta` with stable keys, old/new ink bounds and resource retirement | Existing full `DisplayListView::new` |
| compositor | `apply_paint_delta` maps changed keys to tile damage; property setters already exist | Rebuild scene and compare to full paint |
| integrator-assist | one rendering-update coordinator retains caches and runs a debug full oracle | Switch to full pipeline on mismatch |

These are interface shapes for owning lanes, not signatures that this draft
claims are already implemented. Each API should be bounded by an epoch and
return an explicit `RequiresFullRebuild` result when its cache is invalid.

## Rollout and proof

1. Add the DOM journal and a debug oracle that computes both paths, compares
   computed style maps and records the first differing DOM node/property.
2. Add CSS invalidation sets for self and descendants, then siblings and
   structural/`:has()` fallbacks. Include class/id/attribute changes,
   pseudo-class state, tree edits, shadow scopes and stylesheet replacement.
3. Retain layout boxes and compare geometry, overflow, fragments and hit-test
   regions against full layout. Enable independent relayout only where the
   boundary's output geometry is stable.
4. Add paint chunk keys/deltas and compare display item order/bounds and final
   RGBA pixels to full paint on the pinned render corpus. Property-only
   mutations must show zero style/layout/paint calls and zero tile uploads.
5. Opt in the driver behind a diagnostic mode. For every mutation in a fixed
   script and WPT selection, run both paths at the same epoch and assert exact
   style/geometry/item/pixel equality. On mismatch, print the first divergence,
   retain the full result for presentation, and count a parity failure. Only
   then switch the default, preserving the full oracle for debug and CI.

Report p50/p95 mutation-to-frame times, style/layout/view/frame splits, dirty
node/item/tile counts, cache bytes and full fallbacks on the same fixed
fixtures. The first performance target is at most half the measured Windows
p95: **20 ms** for the article class mutation (40.501 ms baseline) and **31 ms**
for the effects scroller class mutation (63.946 ms baseline). A property-only
transform/opacity/scroll frame targets **zero style, layout and paint calls and
zero tile uploads**. Every target also requires zero unexplained parity losses;
no target may be met by skipping an observable update. The [Blink style invalidation design](https://chromium.googlesource.com/chromium/src/+/master/third_party/blink/renderer/core/css/style-invalidation.md)
is a reference for feature-indexed descendant/sibling sets, not a dependency.
