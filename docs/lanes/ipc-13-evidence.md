# IPC-13: a real renderer process, proved over the sealed-frame channel

Base: local `dev` at `4cb1dd40`. Commit: `9c8248f5`.

## What was already true

`amazing_process::renderer` (`crates/amazing-process/src/renderer.rs`) already
ran the real engine end to end before this ticket -- `render_document` parses
with `amazing_html::parse_bytes`, styles and lays out with
`amazing_layout::layout_dom_document_with_replaced_in_viewport`, and paints
with `amazing_paint::paint_document_with_resources`; the function's own doc
comment describes it as "the exact same public engine path" run inside the
renderer process and inside a browser-side parity oracle. `navigate_shared`
already builds a `FrameHandle` (the sealed-memfd path IPC-12 just fixed) and
`acquire_frame`/`receive_frame` already ship it to the browser process over
the real sealed-frame channel. Input (`InputEvent`) and navigation
(`Navigate`) RPCs already flow browser -> renderer, with `FrameReady` events
flowing back. `crates/amazing-process/tests/split.rs`'s
`renderer_network_frame_parity_and_input_events` already spawned two real
processes and checked this against the in-process oracle, using inline HTML
string literals.

## What this ticket adds

A dedicated proof against an actual fixture **file**
(`crates/amazing-process/tests/fixtures/ipc13-page.html`), loaded from disk
via `include_bytes!` the way a browser process loads a page from storage, not
constructed as a Rust string literal at the test call site. The fixture
exercises an inline `<style>` block, a `<link rel=stylesheet>` pointing at a
`data:` URL (fetched through the real network child), an `<img>` pointing at
an inline SVG `data:` URL, and text -- four distinct, independently
verifiable pixel signatures.

`crates/amazing-process/tests/ipc13_renderer_process.rs`
(`real_renderer_process_renders_a_fixture_page_over_the_sealed_frame_channel`):

1. Spawns real `Role::Network` and `Role::Renderer` processes.
2. Renders the fixture in-process via `render_document` (the parity oracle)
   and asserts the body background, and SVG image pixels are both present --
   a sanity check that the fixture is actually exercising parse+style+layout+paint,
   not just returning a blank frame.
3. Navigates the real renderer process with the identical fixture bytes via
   `navigate_shared`, waits for the `FrameReady` notification, and maps the
   frame back via the sealed-frame channel (`receive_frame`), asserting the
   cross-process pixels are byte-for-byte identical to the in-process oracle.
4. Sends two `InputEvent`s (`Scroll`, `Click`) through the live renderer and
   confirms each is acked and reflected in a new `FrameReady` notification --
   proving the reverse (renderer -> browser) direction isn't a one-shot.
5. Performs a **second**, distinct `navigate_shared` (different URL and
   viewport) on the same live renderer process and again checks the
   sealed-frame pixels against a fresh in-process oracle render -- proving
   navigation keeps working repeatedly on one process, not only once at
   startup.

## A fixture bug, not an engine bug

The first draft's fixture used a 96x64 viewport. The paragraph's
stylesheet-colored text and the SVG image were laid out below that height
and were silently absent from the rendered pixels -- not a rendering defect.
Diagnosed with a throwaway debug test (removed once confirmed) that dumped
the set of distinct RGBA colors present for the link-only fragment, the
img-only fragment, and the full fixture, at both the original and a widened
viewport; widening the viewport to fit all of heading + image + paragraph
made all five expected colors appear. This is recorded here so a future
reader of the commit history understands the detour without re-deriving it.

## Gate

Both hosts, `dev` base `4cb1dd40` plus this commit:

| Host | Kernel | `cargo nextest run --workspace --locked --no-fail-fast` | `cargo test --workspace --locked --doc` |
| --- | --- | --- | --- |
| worker-3 (Ubuntu 24.04) | 6.8.0-71-generic | **3339/3339 passed**, 17 skipped, exit 0 (250.5s) | all doctests ok |
| venus-001 | 7.0 | **3339/3339 passed**, 17 skipped, exit 0 (65.5s) | all doctests ok |

Zero regressions on either host. `-p amazing-process -p amazing-ipc-platform`
(69/69, 3 skipped) was also run standalone on venus-001 during iteration.
