# CSS-31 logical borders

Base `9d0b42d0`, pinned WPT `c2a2fce`, Boat pool-layout `bx_shgjh66q`, Linux.

Logical border expansion and computed serialization: parsing +56 / lost 0
(3076/4292 -> 3132/4292 across 11 selections); css-tables rendering +5 / lost 0
(54/82 -> 59/82). All four table-intrinsic-size fixtures and rules-groups now pass.

`before/compact.json` and `after/compact.json` retain every render classification.
`gate.json` records assertion identities gained and workspace/doctest counts.

Run with the MAIN `scripts/linux-run.sh css --host boat` wrapper. Export
`CARGO_TARGET_DIR=/tmp/amazing-target-css`; run `bash scripts/ensure-wpt.sh`
first. Parsing: `cargo build -p amazing-css --example wpt_conformance --locked`,
then `WPT_ROOT=$HOME/corpora/wpt $CARGO_TARGET_DIR/debug/examples/wpt_conformance`
with `css-logical` plus the default ten selections. Render: build
`cargo build -p amazing --bin amazing --locked`, then `wpt_render_compare.py`
with css/css-tables, 800x600, timeout 20, jobs 4, --compact, and the before baseline.
Workspace: `cargo nextest run --workspace --locked --no-fail-fast` and
`cargo test --workspace --locked --doc`.
