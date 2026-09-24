use amazing_compositor::{
    BlendMode, ClipId, ClipShape, CompositingReasons, Compositor, EffectId, Matrix4, PaintChunk,
    PaintSource, Positioning, Presenter, PropertyTreeState, PropertyTrees, Rect,
    ReferenceCpuPresenter, ScrollId, StickyConstraint, TransformId,
};
use amazing_paint::Surface;
use amazing_process::protocol::{GpuFrame, GpuProxy};
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::BTreeMap;
use std::ops::Range;
use std::time::Duration;

struct Source;
impl PaintSource for Source {
    fn item_count(&self) -> usize {
        1
    }
    fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
        out.fill_rect(-tile.x, -tile.y, 16., 16., [180, 40, 20, 255]);
    }
}

struct ComplexSource;
impl PaintSource for ComplexSource {
    fn item_count(&self) -> usize {
        1
    }
    fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
        out.fill_rect(4. - tile.x, 4. - tile.y, 24., 24., [50, 100, 200, 255]);
    }
}

#[test]
fn gpu_child_preserves_full_property_tree_and_pass_snapshot() {
    let mut trees = PropertyTrees::new();
    let transform = trees
        .add_transform(
            TransformId(0),
            Matrix4::translation(2., 3., 0.),
            (0., 0., 0.),
            false,
        )
        .unwrap();
    let clip = trees
        .add_clip(
            ClipId(0),
            transform,
            ClipShape::Rect(Rect::new(0., 0., 64., 64.)),
        )
        .unwrap();
    let effect = trees
        .add_effect(EffectId(0), 0.8, BlendMode::Multiply, None)
        .unwrap();
    let scroll = trees
        .add_scroll(
            ScrollId(0),
            (3., 0.),
            Rect::new(0., 0., 64., 64.),
            Rect::new(0., 0., 128., 64.),
        )
        .unwrap();
    trees
        .set_snap_points(scroll, vec![0., 32., 64.], vec![0.])
        .unwrap();
    let sticky = trees
        .add_sticky(StickyConstraint {
            scroll,
            normal: Rect::new(4., 4., 24., 24.),
            top: Some(0.),
            right: None,
            bottom: None,
            left: None,
        })
        .unwrap();
    let mut compositor = Compositor::new(
        trees,
        vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState {
                transform,
                clip,
                effect,
                scroll,
                positioning: Positioning::Sticky(sticky),
            },
            bounds: Rect::new(4., 4., 24., 24.),
            reasons: CompositingReasons::WILL_CHANGE,
        }],
        1,
    )
    .unwrap();
    let frame = compositor.build_frame(&ComplexSource, 64, 64).unwrap();
    assert!(frame.passes.len() > 1);
    let expected = ReferenceCpuPresenter::default().present(&frame).unwrap();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let mut gpu = ProcessHost::spawn(
        exe,
        Role::Gpu,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap();
    let proxy = GpuProxy::new(gpu.connection.clone(), Duration::from_secs(5));
    let actual = proxy
        .submit(GpuFrame::from_compositor(9, &frame).unwrap())
        .unwrap();
    assert_eq!(actual.sequence, 9);
    assert_eq!(actual.rgba.0, expected.pixels);
    gpu.kill().unwrap();
}

#[test]
fn gpu_child_retains_resource_deltas_and_matches_cpu_presenter() {
    let chunks = vec![PaintChunk {
        items: 0..1,
        state: PropertyTreeState::default(),
        bounds: Rect::new(0., 0., 16., 16.),
        reasons: CompositingReasons::default(),
    }];
    let mut compositor = Compositor::new(PropertyTrees::new(), chunks, 1).unwrap();
    let mut direct = ReferenceCpuPresenter::default();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_ipc-demo-child"));
    let mut gpu = ProcessHost::spawn(
        exe,
        Role::Gpu,
        Capabilities(BTreeMap::new()),
        RestartPolicy::Never,
    )
    .unwrap();
    let proxy = GpuProxy::new(gpu.connection.clone(), Duration::from_secs(5));
    for sequence in 1..=2 {
        let frame = compositor.build_frame(&Source, 32, 32).unwrap();
        if sequence == 2 {
            assert!(frame.resources.uploads.is_empty());
        }
        let expected = direct.present(&frame).unwrap();
        let submitted = GpuFrame::from_compositor(sequence, &frame).unwrap();
        let actual = proxy.submit(submitted).unwrap();
        assert_eq!(actual.sequence, sequence);
        assert_eq!(actual.rgba.0, expected.pixels);
    }
    gpu.kill().unwrap();
}
