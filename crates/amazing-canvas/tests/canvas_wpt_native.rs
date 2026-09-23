//! Native projections of generated WPT Canvas tests at revision
//! 53bc812da2ed81179ebd1ba76b2d197cc4747d12. This is not a browser run.
use amazing_canvas::{Canvas2D, CompositeOperation, FillRule};

#[derive(Clone, Copy)]
enum Step {
    Fill(&'static str, f64, f64, f64, f64),
    Clear(f64, f64, f64, f64),
    Alpha(f64),
    Composite(CompositeOperation),
    Save,
    Restore,
    Translate(f64, f64),
    Begin,
    Rect(f64, f64, f64, f64),
    Clip,
    FillPath,
}

struct Case {
    wpt: &'static str,
    steps: &'static [Step],
    pixels: &'static [(u32, u32, [u8; 4])],
}

#[test]
fn pinned_canvas_element_pixel_selection() {
    use Step::*;
    let cases = [
        Case {
            wpt: "drawing-rectangles-to-the-canvas/2d.fillRect.basic.html",
            steps: &[Fill("#0f0", 0., 0., 100., 50.)],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
        Case {
            wpt: "drawing-rectangles-to-the-canvas/2d.clearRect.globalalpha.html",
            steps: &[
                Fill("#f00", 0., 0., 100., 50.),
                Alpha(0.1),
                Clear(0., 0., 100., 50.),
            ],
            pixels: &[(50, 25, [0, 0, 0, 0])],
        },
        Case {
            wpt: "drawing-rectangles-to-the-canvas/2d.fillRect.negative.html",
            steps: &[
                Fill("#f00", 0., 0., 100., 50.),
                Fill("#0f0", 0., 0., 50., 25.),
                Fill("#0f0", 100., 0., -50., 25.),
                Fill("#0f0", 0., 50., 50., -25.),
                Fill("#0f0", 100., 50., -50., -25.),
            ],
            pixels: &[
                (25, 12, [0, 255, 0, 255]),
                (75, 12, [0, 255, 0, 255]),
                (25, 37, [0, 255, 0, 255]),
                (75, 37, [0, 255, 0, 255]),
            ],
        },
        Case {
            wpt: "drawing-rectangles-to-the-canvas/2d.fillRect.clip.html",
            steps: &[
                Fill("#0f0", 0., 0., 100., 50.),
                Begin,
                Rect(0., 0., 16., 16.),
                Clip,
                Fill("#f00", 0., 0., 100., 50.),
                Fill("#0f0", 0., 0., 16., 16.),
            ],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
        Case {
            wpt: "the-canvas-state/2d.state.saverestore.path.html",
            steps: &[
                Fill("#f00", 0., 0., 100., 50.),
                Save,
                Rect(0., 0., 100., 50.),
                Restore,
                Fill("#0f0", 0., 0., 0., 0.),
                FillPath,
            ],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
        Case {
            wpt: "transformations/2d.transformation.translate.basic.html",
            steps: &[
                Fill("#f00", 0., 0., 100., 50.),
                Translate(100., 50.),
                Fill("#0f0", -100., -50., 100., 50.),
            ],
            pixels: &[(90, 40, [0, 255, 0, 255])],
        },
        Case {
            wpt: "path-objects/2d.path.rect.basic.html",
            steps: &[
                Fill("#f00", 0., 0., 100., 50.),
                Begin,
                Rect(0., 0., 100., 50.),
                Fill("#0f0", 0., 0., 0., 0.),
                FillPath,
            ],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
        Case {
            wpt: "compositing/2d.composite.clip.copy.html",
            steps: &[
                Fill("#0f0", 0., 0., 100., 50.),
                Composite(CompositeOperation::Copy),
                Rect(-20., -20., 10., 10.),
                Clip,
                Fill("#f00", 0., 0., 50., 50.),
            ],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
        Case {
            wpt: "compositing/2d.composite.clip.source-in.html",
            steps: &[
                Fill("#0f0", 0., 0., 100., 50.),
                Composite(CompositeOperation::SourceIn),
                Rect(-20., -20., 10., 10.),
                Clip,
                Fill("#f00", 0., 0., 50., 50.),
            ],
            pixels: &[(50, 25, [0, 255, 0, 255])],
        },
    ];
    for case in cases {
        let mut canvas = Canvas2D::new(100, 50);
        for step in case.steps {
            match *step {
                Fill(color, x, y, w, h) => {
                    assert!(canvas.set_fill_color(color));
                    canvas.fill_rect(x, y, w, h);
                }
                Clear(x, y, w, h) => canvas.clear_rect(x, y, w, h),
                Alpha(v) => canvas.set_global_alpha(v),
                Composite(mode) => canvas.state.composite_operation = mode,
                Save => canvas.save(),
                Restore => canvas.restore(),
                Translate(x, y) => canvas.translate(x, y),
                Begin => canvas.begin_path(),
                Rect(x, y, w, h) => canvas.rect(x, y, w, h),
                Clip => canvas.clip(FillRule::NonZero),
                FillPath => canvas.fill(FillRule::NonZero),
            }
        }
        for &(x, y, expected) in case.pixels {
            assert_eq!(
                canvas.surface.get_pixel(x, y),
                Some(expected),
                "{} at ({x},{y})",
                case.wpt
            );
        }
    }
}
