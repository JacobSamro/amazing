# ADR 0006 — A compositor with property trees and tiled raster

Status: accepted (2026-09-24)

## Context

Stage 2 of the roadmap needs frame-time performance: scrolling, transforms,
opacity and animations must not repaint or re-layout the page. Today paint
walks the layout tree into a display list, and the whole page is rasterized
in software on every change. GPUI owns production presentation (ADR 0001), so
a compositor must produce frames that GPUI presents, not open its own window.

## Decision

- **`crates/amazing-compositor`**, std-only with `#![forbid(unsafe_code)]`:
  - Property trees (transform, clip, effect, scroll), as in Chromium's
    "compositing after paint". Each has node ids, and the trees are built from
    paint chunks, which are runs of display items sharing property-tree state.
  - Layerization: chunks are grouped into composited layers by direct
    compositing reasons (will-change, 3D/animated transforms, video, fixed
    position, scrollers) and overlap testing.
  - A tiled raster cache per layer with damage tracking, rasterizing tiles with
    the existing software rasterizer. Tiles stay the future parallelism seam
    (locked decision 2).
  - A frame builder that composites tiles under transforms, clips and opacity
    into a frame for GPUI, plus a scroll/transform/opacity update path that
    touches no layout or paint.
- It consumes `amazing-paint`'s display list read-only. When it needs paint
  chunks with property-tree state, it asks replaced-content (the paint owner)
  for them as an additive display-list extension.
- A GPU raster backend is a later decision. Software-only raster (locked
  decision) stays until this crate's frame-time traces show the need.
- Owner: the new `compositor` lane.

## Consequences

The driver keeps full repaint until an adoption ticket routes frames through
the compositor. The Stage 2 gate is frame-time traces for scroll, transform
and opacity changes against the full-repaint baseline.
