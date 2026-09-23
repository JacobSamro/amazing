# ADR 0011 — GPU transformed color sprites in GPUI

Status: accepted as a local prototype (2026-09-24). Any upstream GPUI submission is an outward-facing action that needs the user's explicit approval first.
Owner: integrator/platform for GPUI changes; compositor for frame contract and parity

## Problem

ADR 0001 gives GPUI the production window; ADR 0006 gives the compositor
property trees and tiled raster. The pinned GPUI revision
`d9ad6aff67e47de43abb270d22de75dd950f1b48` accepts a `RenderImage` in
axis-aligned `paint_image` bounds with clipping and opacity, but its color
sprite path has no public per-sprite matrix. The compositor therefore presents
integer translations, clips and opacity directly, while rotation, perspective,
rounded clips, blend groups and effect passes use a CPU-composed image. That
fallback preserves pixels but gives up the intended GPU transform frame time.

## Options

| Option | Benefit | Cost and risk |
| --- | --- | --- |
| Vendor a GPUI patch | Fastest access to sprite structs, scene batching and renderer shaders; one window/atlas | Carrying a fork of a fast-moving pinned dependency risks merge churn, backend divergence and an unreviewed permanent platform fork. Must keep Windows/macOS/Linux implementations equivalent. |
| Add a custom transformed-sprite primitive upstream | GPUI retains window ownership, texture atlas, clipping, z-order and input coordinate system; a narrow public matrix/quad API can be tested in isolation | Requires upstream agreement and backend work. Sampling, antialiasing, clip masks, opacity passes and perspective must be specified; a new primitive may not land on the project schedule. |
| Composite a separate wgpu surface under GPUI | Complete shader control and later GPU raster path | Two graphics device/swap-chain lifecycles, cross-surface ordering and transparency, resize/DPI synchronization, input/accessibility overlays, texture sharing and frame pacing. It weakens ADR 0001's one-window integration and can force expensive copies. |

## Recommendation

Propose a **custom transformed color-sprite primitive in GPUI**, with a small
project-local prototype patch only to prove the API and submit it upstream.
Keep the current CPU-composed fallback until the primitive is accepted or an
explicit project-level review chooses a maintained vendor patch. Do not add a
second surface for this ticket. The primitive should consume the existing tile
atlas resource ID, a quad in local coordinates, a 4×4 transform, clip state,
opacity and draw order. The compositor remains responsible for property-tree
evaluation and pass grouping; GPUI is responsible for GPU upload and drawing.

The first implementation may support 2D affine transforms with rectangular
clips and source-over opacity, then expand to perspective and rounded/masked
clips. Unsupported passes continue through the reference CPU image, selected
per pass rather than flattening unrelated direct quads. No GPU raster decision
is implied: tile contents remain software-rasterized under ADR 0006.

## Review gate

For each GPUI backend, capture the actual Amazing window with an executable
hash, fixture, viewport and capture path. Compare pixels against
`ReferenceCpuPresenter` for translation, rotation, scale, perspective, nested
clip, opacity, overlapping order, and effect passes. Record p50/p95 frame
time, tile uploads, GPU memory and CPU fallback counts over 60 frames at the
same viewport and device. Require zero unexplained pixel or ordering losses
and no extra upload on property-only changes. Keep the existing GPUI
presenter mode selectable as the parity oracle until the project accepts the
new path.
