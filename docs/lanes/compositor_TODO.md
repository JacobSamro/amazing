# Compositor lane follow-ups

## COMP-4 boundary

The RC-16 display-list view replays the painter's transform, clip and opacity
groups while rasterizing an item range. COMP-4 therefore uses one neutral
property-state chunk for pixel-exact real-page output. Its property and hint
metadata is imported separately for inspection. The 36-frame win-002 real-page
trace is exact but its baked frame build takes about twice full repaint time.

1. COMP-5 adopted RC-17a raw local-space tiles on fixed base `7b26163`.
   Article, effects, and two pinned replaced-content fixtures match full paint
   pixel for pixel. The 512 px win-002 real-page trace retains all tiles for
   scroll, transform and opacity updates: 0.92%, 0.75%, and 0.67% of full
   repaint time, with zero tile uploads. Controls and replaced content use the
   pixel-exact baked fallback until their commands can be separated.
2. Paint view should expose local ink bounds for content items, scrollable
   content extents and computed sticky insets. Until then layout supplies
   scroll extents, snap points and sticky constraints to the compositor.
3. Integrator-assist wires `GpuiFrame::element()` into the live viewport and
   retains `GpuiPresenter` per scene. Its frame damage tells the shell when to
   request repaint; resource deltas require continuity across frames.
4. When an active win-001 RDP session is available, run the staged
   `C:\Users\server\target\diagnostics\comp4_capture.ps1` through SSH and
   collect the proof PNG. The scheduled tasks clean up in `finally`.
5. A future GPU transform ticket needs a project ADR: the pinned GPUI color
   sprite API has no matrix field. COMP-4 draws axis-aligned tile sprites
   directly and uses a CPU-composed image for rotation, perspective, rounded
   clips, blend modes and effect passes.

## COMP-6 animation selection

The native compositor unit harness projects timing and event assertions from
three files at WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`:

- `web-animations/timing-model/animation-effects/simple-iteration-progress.html`
  and its paired `current-iteration.html`: before/after fill, fractional
  iterations, iteration start, and the two-iteration spec timing example.
- `css/css-animations/animation-iteration-event-001.html`: a seven-iteration,
  one-second animation with a negative four-second delay emits iteration
  elapsed times five and six seconds, then an end event.
- `css/css-transitions/event-dispatch.tentative.html`: run/start/end order and
  elapsed times for a delayed transition; cancellation and negative-delay
  elapsed times are covered by additional native tests.

These are projected native assertions, not an unmodified browser WPT run. The
shell does not yet install CSS Animation/Transition DOM event or Web Animations
bindings. Integrator-assist must connect the compositor's typed style and
keyframe inputs to CSS-14, apply main-thread updates, and dispatch returned
events and promise signals before the unmodified selections are measurable.
