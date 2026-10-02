# CSS-32 image argument validation

Fixed base `03e13e3f`, pinned WPT `c2a2fce`, Boat pool-layout `bx_shgjh66q`, Linux.

The image list and its radial-gradient/cross-fade argument checks now run both
before declaration ranking and after variable substitution. Tests cover invalid
parsed declarations retaining lower-ranked values, invalid substituted winners
resetting to initial, recursive cross-fade items, negative color-stop offsets,
and the distinction between circle lengths and ellipse length-percentages.

The validator preserves balanced linear/conic gradients and other established
image functions for the downstream paint parser; their independent argument
grammars and advanced computed serialization remain outside this ticket.

Run the MAIN wrapper with `css --host boat -- 'bash crates/amazing-css/evidence/css32/gate.sh'`. This batches workspace nextest,
doctests, twelve parsing selections, and both complete static render selections.
The script uses native `/tmp/amazing-target-css`; source/results remain under HOME.
Run before render selections on the base revision and retain their compact
classifications under `$OUT/css32-{backgrounds,images}-before/compact.json`.

`gate.json` records measured counts and exact parsing assertion identities gained.
`before/` and `after/` retain every render classification for both selections.
