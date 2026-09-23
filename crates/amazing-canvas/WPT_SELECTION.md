# Canvas 2D native WPT selection

WPT revision: `53bc812da2ed81179ebd1ba76b2d197cc4747d12`.
Source tree: `html/canvas/element/`. The generated tests below are projected
into `tests/canvas_wpt_native.rs` as ordered engine calls and pixel assertions.
This is a native engine selection, not an unmodified browser WPT run.

| Generated test | Native assertion |
| --- | --- |
| `drawing-rectangles-to-the-canvas/2d.fillRect.basic.html` | Opaque fill pixel |
| `drawing-rectangles-to-the-canvas/2d.clearRect.globalalpha.html` | Clear ignores alpha |
| `drawing-rectangles-to-the-canvas/2d.fillRect.negative.html` | Four negative-size quadrants |
| `drawing-rectangles-to-the-canvas/2d.fillRect.clip.html` | Pixel outside clip unchanged |
| `the-canvas-state/2d.state.saverestore.path.html` | Current path survives save/restore |
| `transformations/2d.transformation.translate.basic.html` | Translation moves fill |
| `path-objects/2d.path.rect.basic.html` | Current rect path fills |
| `compositing/2d.composite.clip.copy.html` | Copy does not affect outside clip |
| `compositing/2d.composite.clip.source-in.html` | Source-in does not affect outside clip |

Nine selected generated cases pass on the native harness. The engine also has
local golden-pixel tests for fill rules, antialiasing, stroke caps and dashes,
gradients, patterns, shadows, Porter-Duff modes, image data and TextSystem use.
The WPT harness uses the CSS fallback font and does not assert a browser DOM,
origin-clean state, or exact platform font pixels. Filters are preserved as
opaque unsupported input.
