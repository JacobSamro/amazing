# RC-4: author provenance and bounded SVG transforms

WPT pin: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. All builds and renders
run on win-001; no Mac build/test. Before artifacts were built on CSS-4
`cc3761e` after a lane-only artifact-invalidating refresh. Final artifacts use
dev `3d0c2b0` plus the RC-4 implementation, after another lane-only refresh,
re-scp of all 38 owned files and explicit invalidation of the isolated image
target's first-party artifacts. Every Cargo command uses --locked and
CARGO_INCREMENTAL=0. The intervening dev changes include the DOM/HTML namespace
integration; before/after are named saved artifacts, not an isolated claim that
all possible differences come from this ticket. No root graph, dependency or
lockfile changes are made by RC-4.

## Implementation

Control chrome queries `ComputedStyle::is_author_specified` for background
color/image and side border longhands. Explicit author transparent/none and
initial defaults now suppress native chrome; absence of author styling retains
it. Visible synthetic backgrounds/borders still honor their values, and
appearance:none still suppresses chrome. The explicit-default ambiguity from
RC-3 is resolved through CSS provenance rather than inference.

DOM control state and layout-positioned label glyph providers remain separate,
still unwired in production. RC-4 changes neither that ownership nor its claims.
An exposed CSS shorthand bug is routed: `border:revert` reports author=true on
its defaulted style/width longhands. Paint follows that API; explicit-longhand
rollback is tested without a paint-side workaround.

SVG image decoding now supports matrix, translate, scale, rotate (including
pivot), skewX and skewY attribute lists on the existing groups and shapes.
Lists post-multiply; nested matrices compose; inverse sampling evaluates the
original rectangle/ellipse geometry. Singular matrices hide their subtree.
Transform parsing is capped at 32 functions per attribute and six arguments
per function; nonfinite/overflowing geometry fails the whole image. Existing
input, depth, element, pixel/allocation and sample-work budgets still apply.
No new general XML parser, CSS computation, DOM traversal in paint, or external
dependency is introduced. Root SVG transforms, strokes, paths, CSS, text,
references, filters and animation remain outside this static subset.

Algorithm reference: [SVG 1.1 coordinate transformations](https://www.w3.org/TR/SVG11/coords.html#TransformAttribute).
This is geometry support in the existing decoder, not general inline SVG or
SVG conformance.

## Why this cluster

The existing 48-entry scope is retained, rather than replaced with easy cases.
The current baseline ranks Canvas first (8/8 non-passes), controls next (6/8),
SVG next (4/8), images (2/8), and embedded/media tied (1/8 each). Canvas is
blocked on DOM/JS context/worker bindings and driver script-resource loading.
The remaining controls need supplied state/text and specialized widget/layout
behavior. SVG is therefore the highest-ranked unblocked remainder.

These are observed non-passes, not expected gains. In particular, one canvas
error names `/common/reftest-wait.js`: local driver resource resolution does not
map root-relative paths to the WPT checkout. That integrator-assist follow-up is
routed separately from the already-routed reftest-link bug. Seventeen renderer
errors must not be described as seventeen missing paint features.

The 12-case retained SVG image selection uses inline CSS in its SVG resources,
which the decoder still rejects. Raw matches there earn **zero SVG conformance
credit**. The transform implementation does not pretend to resolve that seam.

## Owned rendered proof

`../rc4-fixtures/` is explicitly an owned diagnostic corpus, not upstream WPT.
Three 64x64 HTML cases compare a button with explicit transparent/none,
initial, or explicit-longhand defaults to the same solid background. Six SVG
image cases compare transformed geometry with equivalent untransformed shapes:
nested translate/scale, pivot rotation, reflection matrix, nonuniform circle
scale, transform inside a viewBox, and a singular subtree with a visible sibling.
References do not use the new transform support.

Both phases are rendered twice by the saved executables using
`../run-rc4-fixtures.ps1`. The initial diagnostic references lacked the control
page's body height; both sides were given a fixed 64px height and both phases
were rerun. Only the corrected corpus is retained. This change does not touch
upstream WPT or affect its selection.

## Reproduction and evidence

Build the root driver with `cargo build -p amazing --locked` into the lane
root target. Build `render_reftest` with
`cargo build --manifest-path crates/amazing-image/Cargo.toml --example render_reftest --locked`
into the isolated image target. Save them as `target/rc4-driver-<phase>.exe`
and `target/rc4-images-<phase>.exe` for each source revision, before proceeding.
Run `../run-rc4-gates.ps1 -Phase <before|after> -RepositoryRoot <tree>` and
`../run-rc4-fixtures.ps1` with the same arguments. The WPT checkout is pinned
and retains the sparse roots used by RC-1/2/3, including css/reference.
Viewport/font providers remain those recorded in each report (WPT 800x600,
scale 1; owned proof 64x64). Root driver handles scope/controls/focused; the
static image adapter handles PNG/SVG resources and transform proof.

`../collect-rc4.ps1` collects run reports and executable/source hashes.
`../analyze-rc4.py` compares saved data; it does not execute builds/tests.
Source hash equality is supplementary to explicit artifact invalidation.
Headless render evidence is not a GPUI window screenshot.

`render-proof.png` shows actual headless before/after pixels at nearest-neighbor
4x enlargement. Its provenance case changes 764 pixels (sample 10,10: gray
239/239/239 → lime 0/255/0); the SVG translate/scale case changes 256 pixels
(white → red). `render-proof.json` records source PPM hashes, viewport, renderer
and capture paths. No UI or broad-conformance inference is made from this image.

## Measured counts

| Selection | Before | After |
| --- | ---: | ---: |
| Original 48-entry scope | 26 pass / 4 mismatch / 1 unsupported / 17 renderer errors | 26 / 4 / 1 / 17 |
| Form controls | 21 pass / 2 mismatch / 1 unsupported | 21 / 2 / 1 |
| CSS Images PNG | 12/12 | 12/12 |
| SVG counterparts | 12/12 raw, zero credit | 12/12 raw, zero credit |
| Focused paint | 24/24 | 24/24 |
| Owned provenance + transform diagnostic | 0/9 | 9/9 |

There are no timeouts. The only renderer errors are the 17 in the scope corpus.
No new upstream conformance passes are claimed. The explicit-default limitation
is resolved by real rendered change; the CSS-wide border shorthand rollback
API defect remains routed to its owner. Canvas and script/reftest-link loading
findings were accepted and routed by the integrator.

Windows validation: workspace **1,135 passed / zero failed / three ignored over
56 targets**, paint **87 passed**, image adapter **17 passed** (4 unit,
3 presentation, 10 SVG). Root driver and isolated image example builds succeed
with --locked. All 180 checked tracked Rust/Cargo build inputs match between
Mac and server, in addition to the artifact invalidation and re-sync protocol.

All ten WPT before/after repeated pairs are stable in classifications, difference
counts and actual/reference/diff pixel hashes. All WPT actual and reference
hashes are also unchanged across phases, despite the intervening namespace
integration. Both owned-diagnostic pairs are stable. Each of the nine diagnostic
actuals changes, while its reference remains unchanged. The 0/9 → 9/9 result
therefore exercises the new behavior rather than changes to both sides.
