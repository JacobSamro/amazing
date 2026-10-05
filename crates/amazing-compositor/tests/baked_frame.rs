//! Exact baked fallback parity and replay ownership for a single frame.
use amazing_compositor::{PaintViewRasterMode, PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_paint::{
    controls::{ControlContent, ControlProvider},
    display_list_view::{DisplayListView, PaintBounds},
    paint_document_at_viewport, NoImages, RasterViewport, Surface,
};
use amazing_text::FallbackTextSystem;
use std::cell::Cell;

#[derive(Default)]
struct Controls {
    calls: Cell<usize>,
    checked: Cell<bool>,
}
impl ControlProvider for Controls {
    fn content(&self, _: amazing_dom::NodeIdx) -> ControlContent<'_> {
        self.calls.set(self.calls.get() + 1);
        ControlContent {
            checked: self.checked.get(),
            ..Default::default()
        }
    }
}
fn root(dom: &amazing_dom::Dom, css: &str, w: f32, h: f32) -> amazing_layout::LayoutBox {
    amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        dom,
        css,
        w,
        Some(h),
        &FallbackTextSystem,
        &amazing_layout::NoReplacedElements,
    )
}

#[test]
fn fallback_groups_controls_clips_filters_and_edge_tiles_match_legacy_and_full_paint() {
    let html="<body><div class=clip><div class=group><button>button</button><input type=checkbox checked><span>ink</span></div></div><table><tr><td>table</td></tr></table></body>";
    let clusters=[
        ".group{opacity:.4;transform:translate(3px,-2px)}",
        ".clip{overflow:hidden;width:181px;height:71px;border-radius:13px}.group{background:rgba(30,80,170,.5)}",
        ".group{filter:blur(2px);background:#a13}",
        ".group{mix-blend-mode:multiply;background:#8ca;isolation:isolate}",
        ".clip{transform:rotate(3deg)}.group{opacity:.7;background:linear-gradient(red,blue)}",
        "table{border-collapse:collapse}td{border:3px solid #148}.group{box-shadow:2px 3px 4px #789}",
    ];
    for extra in clusters {
        for (w, h) in [(257., 259.), (513., 265.)] {
            for scale in [1., 1.25, 2.] {
                let dom = amazing_html::parse(html);
                let css=format!("body{{margin:0;background:#eec}}button{{width:93px;height:35px}}input{{width:19px;height:19px}}{extra}");
                let root = root(&dom, &css, w, h);
                let controls = Controls::default();
                let metrics =
                    RasterViewport::new((w as f64, h as f64), (w as f64, h as f64), scale).unwrap();
                let view = DisplayListView::with_dom_generations(
                    &root,
                    w as u32,
                    h as u32,
                    &FallbackTextSystem,
                    &NoImages,
                    &controls,
                    &dom,
                )
                .with_raster_viewport(metrics)
                .unwrap();
                let source = PaintViewSource::new(&view);
                assert_eq!(source.mode(), PaintViewRasterMode::BakedFallback);
                let (pw, ph) = metrics.physical_size();
                let mut optimized = source.compositor(pw, ph).unwrap();
                let frame = source.build_frame(&mut optimized, pw, ph).unwrap();
                let result = ReferenceCpuPresenter::default().present(&frame).unwrap();
                let expected = paint_document_at_viewport(
                    &root,
                    metrics,
                    &FallbackTextSystem,
                    &NoImages,
                    &controls,
                )
                .unwrap();
                assert_eq!(result, expected, "full {extra}/{w}/{h}/{scale}");
                let mut legacy = source.compositor(pw, ph).unwrap();
                let old = legacy.build_frame(&view, pw, ph).unwrap();
                assert_eq!(
                    ReferenceCpuPresenter::default().present(&old).unwrap(),
                    result,
                    "legacy {extra}/{scale}"
                );
            }
        }
    }
}

#[test]
fn genuine_control_invocations_prove_one_replay_per_dirty_frame_and_zero_on_cached_frame() {
    let dom = amazing_html::parse("<body><button>one</button></body>");
    let root = root(
        &dom,
        "body{margin:0;background:white}button{width:80px;height:30px}",
        513.,
        265.,
    );
    let controls = Controls::default();
    for scale in [1., 2.] {
        let metrics = RasterViewport::new((513., 265.), (513., 265.), scale).unwrap();
        let (pw, ph) = metrics.physical_size();
        let view = DisplayListView::with_dom_generations(
            &root,
            513,
            265,
            &FallbackTextSystem,
            &NoImages,
            &controls,
            &dom,
        )
        .with_raster_viewport(metrics)
        .unwrap();
        let source = PaintViewSource::new(&view);
        assert_eq!(source.mode(), PaintViewRasterMode::BakedFallback);
        let mut scene = source.compositor(pw, ph).unwrap();
        controls.calls.set(0);
        let first = source.build_frame(&mut scene, pw, ph).unwrap();
        assert_eq!(controls.calls.get(), 1, "one actual control traversal");
        assert!(first.resources.uploads.len() > 1);
        source.build_frame(&mut scene, pw, ph).unwrap();
        assert_eq!(
            controls.calls.get(),
            1,
            "cached frame does not consult provider"
        );
        scene.invalidate_item(0);
        source.build_frame(&mut scene, pw, ph).unwrap();
        assert_eq!(controls.calls.get(), 2, "new frame rerenders exactly once");
        let mut legacy = source.compositor(pw, ph).unwrap();
        controls.calls.set(0);
        let old = legacy.build_frame(&view, pw, ph).unwrap();
        assert_eq!(
            controls.calls.get() as u64,
            old.resources.uploads.len() as u64
        );
        assert_eq!(
            old.resources.uploads.len() as u64,
            u64::from(pw.div_ceil(256)) * u64::from(ph.div_ceil(256))
        );
    }
}

#[test]
fn prepared_ranges_preserve_group_context_empty_ranges_and_signed_tile_composition() {
    let dom =
        amazing_html::parse("<body><div><button>A</button><input type=checkbox></div></body>");
    let root=root(&dom,"body{margin:0;background:rgba(10,80,90,.3)}div{opacity:.6;transform:translate(5px,3px);overflow:hidden;width:270px;height:90px;border-radius:11px}",301.,267.);
    let controls = Controls::default();
    for scale in [1., 1.5, 2.] {
        let metrics = RasterViewport::new((301., 267.), (301., 267.), scale).unwrap();
        let view = DisplayListView::with_dom_generations(
            &root,
            301,
            267,
            &FallbackTextSystem,
            &NoImages,
            &controls,
            &dom,
        )
        .with_raster_viewport(metrics)
        .unwrap();
        let n = view.items().len();
        for range in [0..n, 0..0, 1..n, 0..1, n - 1..n] {
            let prepared = view.prepare_baked_range(range.clone()).unwrap();
            for offset in [(0, 0), (-256, -256), (7, -3), (-800, 1)] {
                let mut old = Surface::new(257, 259);
                old.clear([31, 91, 7, 120]);
                let mut new = old.clone();
                let clip = PaintBounds {
                    x: 1.5,
                    y: 2.25,
                    width: 255.,
                    height: 255.,
                };
                assert!(view.raster_range(range.clone(), &mut old, offset, clip));
                assert!(prepared.copy_to(&mut new, offset, clip));
                assert_eq!(new, old, "{range:?}/{offset:?}/{scale}");
            }
        }
        assert!(view.prepare_baked_range(n + 1..n + 2).is_none());
    }
}

#[test]
fn new_resource_state_is_observed_on_each_dirty_frame_without_persistent_baked_pixels() {
    let dom = amazing_html::parse("<body><input type=checkbox></body>");
    let root = root(
        &dom,
        "body{margin:0;background:white}input{width:20px;height:20px}",
        301.,
        267.,
    );
    let controls = Controls::default();
    let metrics = RasterViewport::new((301., 267.), (301., 267.), 2.).unwrap();
    let view = DisplayListView::with_dom_generations(
        &root,
        301,
        267,
        &FallbackTextSystem,
        &NoImages,
        &controls,
        &dom,
    )
    .with_raster_viewport(metrics)
    .unwrap();
    let source = PaintViewSource::new(&view);
    let (pw, ph) = metrics.physical_size();
    let mut scene = source.compositor(pw, ph).unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    let before = presenter
        .present(&source.build_frame(&mut scene, pw, ph).unwrap())
        .unwrap();
    controls.checked.set(true);
    for index in 0..view.items().len() {
        scene.invalidate_item(index);
    }
    let after = presenter
        .present(&source.build_frame(&mut scene, pw, ph).unwrap())
        .unwrap();
    assert_ne!(before, after);
    let expected =
        paint_document_at_viewport(&root, metrics, &FallbackTextSystem, &NoImages, &controls)
            .unwrap();
    assert_eq!(after, expected);
}

#[test]
fn embedding_mixed_ranges_keep_original_dispatch_and_property_state() {
    use amazing_compositor::{
        CompositingReasons, Compositor, PaintChunk, PropertyTreeState, PropertyTrees, Rect,
    };
    let dom = amazing_html::parse("<body><button>A</button><button>B</button></body>");
    let root = root(
        &dom,
        "body{margin:0}button{width:31px;height:21px}",
        301.,
        267.,
    );
    let controls = Controls::default();
    let view = DisplayListView::with_dom_generations(
        &root,
        301,
        267,
        &FallbackTextSystem,
        &NoImages,
        &controls,
        &dom,
    );
    let source = PaintViewSource::new(&view);
    assert_eq!(source.mode(), PaintViewRasterMode::BakedFallback);
    let n = view.items().len();
    assert!(n > 1);
    let scene = || {
        Compositor::new(
            PropertyTrees::new(),
            vec![
                PaintChunk {
                    items: 0..1,
                    state: PropertyTreeState::default(),
                    bounds: Rect::new(0., 0., 301., 267.),
                    reasons: CompositingReasons::default(),
                },
                PaintChunk {
                    items: 1..n,
                    state: PropertyTreeState::default(),
                    bounds: Rect::new(0., 0., 301., 267.),
                    reasons: CompositingReasons::default(),
                },
            ],
            n,
        )
        .unwrap()
    };
    let mut current = scene();
    let mut legacy = scene();
    let a = source.build_frame(&mut current, 301, 267).unwrap();
    let b = legacy.build_frame(&view, 301, 267).unwrap();
    assert_eq!(
        ReferenceCpuPresenter::default().present(&a).unwrap(),
        ReferenceCpuPresenter::default().present(&b).unwrap()
    );
}
