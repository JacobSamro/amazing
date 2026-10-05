# Frame-scoped baked viewport replay

Source preparation only; no native qualification has run. Recorded published
base: c08149e33a4a49c9c9a4121396493729625dcd02.

The bb3be232 diagnostic reproduces the original scale2 viewport failure in
both product and resize suites. Both traces returned through paint, display-list
construction and compositor scene setup, then entered `build_frame(2508,1312)`
without exiting before actual capture cleanup. The failing processes emitted
117 events each, below the384-line bound. Scale1 completed all14 actions.
The exact internal operation was not measured; no deadlock assertion is made.

The public baked path replays the full viewport for each256px tile. A2508x1312
viewport has60 tiles versus15 at1254x656, with four times the replay pixels.
This ticket removes that repeated replay, without changing capture deadlines.

`paint::raster_command_surface_at_scale` extracts the existing command walk and
budget checks. The original range API still invokes that same walk, then the
same source-over/clipped copy. `PreparedBakedRaster` owns the completed surface.
`PaintViewSource::build_frame` uses a lazy local prepared surface when all actual
scene chunks have the same item range (the ordinary baked scene's one full-view
range). It drops that surface before returning. A cached frame does no replay;
a subsequent dirty frame starts fresh. Embedding mixed-range scenes retain their
original dispatch. Raw-local, owned retained and parallel worker paths remain
unchanged. No persistent provider/view cache, group flattening, transformed-path
disable, budget increase or blank pixels are introduced.

The complete original viewport91 delta is adapted to current published source.
The only paint conflict combines the current selected-image source-density
natural size with the existing DPR lattice: decoded pixel sampling stays in
image pixels, fitting uses floating CSS natural dimensions; an SVG's separately
supplied logical natural size stays independent of its physical raster size.
Current JS/PTC/binding and driver snapshots are retained. The original owned
D-Bus/real AT-SPI custody adapter and its12 controls are retained unchanged.
Original91/a405/bb3 sources and red receipts remain independent and preserved.

Seven new native groups cover full-painter and legacy tiled parity across
controls, table decorations, opacity, transforms, filters/blending, overflow and
rounded clips, fractional density and edge tiles; actual control-provider calls
prove one replay per dirty frame and zero per cached frame. Selected ranges,
signed offsets, empty/invalid ranges, existing raster-budget rejection, mutable
control/image provider state and mixed embedding ranges are covered. These are
new API/algorithm controls on AFTER; no invented BEFORE API overlay is used.
All original109 GPUI/lower DPR tests,28 real actions at scales1/2, actual measured
viewport oracle, accessibility, and full20 WPT render cohorts remain required.

Planned gates use independent finite default4CPU/8GiB boxes and fresh native
sources/targets. Workspace/docs and GPUI/actions each require real29GiB admission;
full20 render selection is partitioned into the original four cohorts on matched
BEFORE/AFTER sources with actual26.125GiB admission and4GiB cohort raw guards.
No ELF or raw ferry between boxes. Raw stays HOME after explicit budgeted native
publication; compact receipts record actual source, profile, phases and hashes.
One tooling clean rerun at most. Actual regression remains a blocker.

## Completed Linux qualification

Measured source `bd4c4e39495b0a40e06ecb6ddb6525dab61fccac` closes the original scale-2 window failures: all 28 actions pass with the same oracle, accessibility and deadlines. Workspace 4,971/17 skip and doctests exit 0, GPUI 109, proof 17, session 12, and all original 20 render directories are qualified. Render passes remain 1,327 across 2,214 selected cases, with zero lost or removed cases. Native pixel/parity and one-replay controls pass. Exact source/provenance, retained failures, native raw references and bounded actual-window PNGs are in [the compact ticket evidence](evidence/viewport-baked-frame/README.md). Root merged-current/full16 integration remains pending.
