# Compositor lane follow-ups

## COMP-4 boundary

The RC-16 display-list view replays the painter's transform, clip and opacity
groups while rasterizing an item range. COMP-4 therefore uses one neutral
property-state chunk for pixel-exact real-page output. Its property and hint
metadata is imported separately for inspection. The 36-frame win-002 real-page
trace is exact but its baked frame build takes about twice full repaint time.

1. Adopt RC-17's raw local-space tile raster seam, then make the imported
   property-tree chunks the active retained scene. Re-run the real-page scroll,
   transform and opacity traces and require zero pixel losses.
2. Integrator-assist wires `GpuiFrame::element()` into the live viewport and
   retains `GpuiPresenter` per scene. Its frame damage tells the shell when to
   request repaint; resource deltas require continuity across frames.
3. When an active win-001 RDP session is available, run the staged
   `C:\Users\server\target\diagnostics\comp4_capture.ps1` through SSH and
   collect the proof PNG. The scheduled tasks clean up in `finally`.
4. A future GPU transform ticket needs a project ADR: the pinned GPUI color
   sprite API has no matrix field. COMP-4 draws axis-aligned tile sprites
   directly and uses a CPU-composed image for rotation, perspective, rounded
   clips, blend modes and effect passes.
