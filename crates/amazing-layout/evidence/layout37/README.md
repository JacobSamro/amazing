# LAYOUT-37: nested nowrap float source order

Fixed base: `e23ad2406671163c75ef48acc93cbc9322cb2231`.
CSS2.2 [inline formatting](https://www.w3.org/TR/CSS22/visuren.html#inline-formatting)
and [floats](https://www.w3.org/TR/CSS22/visuren.html#floats) require unbreakable
inline content to remain together while respecting preceding normal-flow content.

The first float inside a nested nowrap scope used to be appended ahead of
already buffered outer text. That text is now flushed first. Content inherited
from an enclosing nowrap scope is carried separately through recursive inline
flattening, so deeper wrappers do not incorrectly restart an uninterrupted run.
Actual block interruptions reset that prefix and the current inline range.

The corrected source-order regression covers twelve combinations of outer text,
inline wrappers, empty wrappers and indentation. Four deeper inherited-nowrap
cases protect continuous runs, and a block-interruption case protects the reset.
This closes the known box-tree ordering defect; it does not claim complete
float insertion into partially occupied line boxes or inline box fragmentation.

`gate.sh` runs on Boat through `scripts/linux-run.sh layout --host boat`.
It copies source to native ext4, uses `/tmp/amazing-layout36-target`, builds the
fixed baseline renderer and measures all eligible reftests in five full pinned
WPT directories. It restores candidate source, runs workspace nextest plus
doctests and the seven LAYOUT-36 harness checks, rebuilds the renderer, then
repeats the five directories with per-test zero-loss comparisons. The baseline
input `$OUT/layout37/before-box-tree.rs` is generated with
`git show e23ad240:crates/amazing-layout/src/box_tree.rs` and transferred to Boat
before invoking the gate. `LAYOUT37_BASELINE_SOURCE` can select another path.
Logs and full pixel artifacts live under `$OUT/layout37`; compact maps and the
summary are copied here immediately when the gate completes.

Measured gate: **3517 workspace tests / 17 skipped**, **4 doctests** and
**7 harness checks** passed. All **323** eligible reftests completed without
error, timeout, unsupported or crash. Floats improved **28→29/39** through
`float-nowrap-4.html`; both references match exactly (4279→0 differing pixels).
Normal-flow **24/29**, tables **54/82**, sizing **90/138** and display **27/35**
were unchanged. Total **223→224 passes**, one gain, zero losses or other
classification changes. `summary.json` records renderer and source hashes.
