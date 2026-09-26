use super::*;
use FilterOperation as F;
use std::cell::Cell;
use std::sync::Arc;

struct Blocks {
    items: Vec<(Rect, [u8; 4])>,
    calls: Cell<usize>,
}
impl PaintSource for Blocks {
    fn item_count(&self) -> usize {
        self.items.len()
    }
    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        self.calls.set(self.calls.get() + 1);
        for (rect, color) in &self.items[items] {
            out.fill_rect(rect.x - tile.x, rect.y - tile.y, rect.w, rect.h, *color);
        }
    }
}
fn scene(
    operations: EffectOperations,
    bounds: Rect,
    color: [u8; 4],
) -> (Compositor, Blocks, EffectId) {
    let mut trees = PropertyTrees::new();
    let effect = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    trees.set_effect_operations(effect, operations).unwrap();
    let chunks = vec![PaintChunk {
        items: 0..1,
        state: PropertyTreeState {
            effect,
            ..Default::default()
        },
        bounds,
        reasons: CompositingReasons::WILL_CHANGE,
    }];
    (
        Compositor::new(trees, chunks, 1).unwrap(),
        Blocks {
            items: vec![(bounds, color)],
            calls: Cell::new(0),
        },
        effect,
    )
}
#[test]
fn color_filter_golden_pixels_and_all_blends_on_transparent_backdrop() {
    let mut source = Surface::new(1, 1);
    source.set_pixel(0, 0, [200, 100, 50, 255]);
    for (filter, expected) in [
        (F::Brightness(0.5), [100, 50, 25, 255]),
        (F::Contrast(0.), [128, 128, 128, 255]),
        (F::Grayscale(1.), [118, 118, 118, 255]),
        (F::Invert(1.), [55, 155, 205, 255]),
        (F::Opacity(0.5), [200, 100, 50, 128]),
        (F::Saturate(0.), [118, 118, 118, 255]),
        (F::Sepia(1.), [165, 147, 114, 255]),
        (F::HueRotate(0.), [200, 100, 50, 255]),
    ] {
        assert_eq!(
            effect_pixels::filters(&source, &[filter]).get_pixel(0, 0),
            Some(expected),
            "{filter:?}"
        );
    }
    source.set_pixel(0, 0, [255, 0, 0, 255]);
    assert_eq!(
        effect_pixels::filters(&source, &[F::HueRotate(180.)]).get_pixel(0, 0),
        Some([0, 109, 109, 255])
    );
    use EffectBlend as B;
    for mode in [
        B::Normal,
        B::Multiply,
        B::Screen,
        B::Overlay,
        B::Darken,
        B::Lighten,
        B::ColorDodge,
        B::ColorBurn,
        B::HardLight,
        B::SoftLight,
        B::Difference,
        B::Exclusion,
        B::Hue,
        B::Saturation,
        B::Color,
        B::Luminosity,
        B::PlusLighter,
    ] {
        assert_eq!(
            effect_pixels::blend([19, 241, 99, 0], [200, 100, 50, 128], 1., mode),
            [200, 100, 50, 128],
            "{mode:?}"
        );
    }
    assert_eq!(
        effect_pixels::blend([100, 150, 200, 255], [200, 100, 50, 255], 1., B::Multiply),
        [78, 59, 39, 255]
    );
    assert_eq!(
        effect_pixels::blend([100, 150, 200, 255], [200, 100, 50, 255], 1., B::Screen),
        [222, 191, 211, 255]
    );
}
#[test]
fn filter_and_blend_animation_reuses_tiles_without_source() {
    let from = EffectOperations {
        filters: vec![F::Brightness(0.5)],
        ..Default::default()
    };
    let to = EffectOperations {
        filters: vec![F::Brightness(1.5)],
        blend: EffectBlend::Screen,
        ..Default::default()
    };
    let (mut c, source, id) = scene(
        from.clone(),
        Rect::new(0., 0., 12., 12.),
        [100, 50, 20, 255],
    );
    let first = c.build_frame(&source, 12, 12).unwrap();
    let mut p = ReferenceCpuPresenter::default();
    p.present(&first).unwrap();
    let calls = source.calls.get();
    let epoch = c.raster_epoch;
    let ids = first
        .resources
        .uploads
        .iter()
        .map(|u| u.id)
        .collect::<Vec<_>>();
    for step in 0..=10 {
        let operations = sample_effect_operations(&from, &to, step as f32 / 10.).unwrap();
        c.set_effect_operations(id, operations).unwrap();
        let frame = c.build_cached_frame(12, 12).unwrap();
        assert!(frame.resources.uploads.is_empty());
        assert!(frame.resources.evictions.is_empty());
        assert_eq!(frame.resource_id_at(0, 0, 0), Some(ids[0]));
        let pixels = p.present(&frame).unwrap();
        assert_eq!(pixels.get_pixel(3, 3).unwrap()[0], 50 + 10 * step);
        if step > 0 {
            assert!(!frame.damage.is_empty());
        }
    }
    assert_eq!(source.calls.get(), calls);
    assert_eq!(c.raster_epoch, epoch);
    let same = c.build_cached_frame(12, 12).unwrap();
    assert!(same.damage.is_empty());
}
#[test]
fn blur_crosses_tile_edges_and_accepts_offscreen_contribution() {
    let operations = EffectOperations {
        filters: vec![F::Blur(2.)],
        ..Default::default()
    };
    let (mut c, source, _) = scene(operations, Rect::new(-4., 0., 265., 8.), [255, 0, 0, 255]);
    let frame = c.build_frame(&source, 262, 10).unwrap();
    let actual = ReferenceCpuPresenter::default().present(&frame).unwrap();
    // Full-surface kernel oracle, explicitly distinct from the pending full
    // CSS painter parity gate. Includes a 6-pixel transparent filter halo.
    let mut full = Surface::new(274, 22);
    full.fill_rect(2., 6., 265., 8., [255, 0, 0, 255]);
    let full = effect_pixels::filters(&full, &[F::Blur(2.)]);
    for y in 0..10 {
        for x in 0..262 {
            assert_eq!(
                actual.get_pixel(x, y),
                full.get_pixel(x + 6, y + 6),
                "{x},{y}"
            );
        }
    }
    assert!(actual.get_pixel(0, 4).unwrap()[3] > 200);
}
#[test]
fn drop_shadow_preserves_foreground_and_offsets_alpha() {
    let operations = EffectOperations {
        filters: vec![F::DropShadow {
            offset: (5., 0.),
            sigma: 0.,
            color: [0, 0, 255, 128],
        }],
        ..Default::default()
    };
    let (mut c, source, _) = scene(operations, Rect::new(1., 1., 2., 2.), [255, 0, 0, 255]);
    let frame = c.build_frame(&source, 10, 6).unwrap();
    let out = ReferenceCpuPresenter::default().present(&frame).unwrap();
    assert_eq!(out.get_pixel(1, 1), Some([255, 0, 0, 255]));
    assert_eq!(out.get_pixel(6, 1), Some([0, 0, 255, 128]));
    assert_eq!(out.get_pixel(4, 1), Some([0; 4]));
}
#[test]
fn clip_shapes_curves_holes_and_luminance_mask() {
    use ClipPathSegment as P;
    let square = vec![(1., 1.), (7., 1.), (7., 7.), (1., 7.)];
    let hole = vec![(3., 3.), (5., 3.), (5., 5.), (3., 5.)];
    let shapes = vec![
        EffectClip::Inset {
            rect: Rect::new(1., 1., 6., 6.),
            radii: [2.; 4],
        },
        EffectClip::Ellipse {
            center: (4., 4.),
            radii: (3., 3.),
        },
        EffectClip::Polygon {
            points: square.clone(),
            rule: FillRule::NonZero,
        },
        EffectClip::Path {
            contours: vec![square, hole],
            rule: FillRule::EvenOdd,
        },
        EffectClip::from_path(
            &[
                P::MoveTo(1., 1.),
                P::CubicTo {
                    first: (3., 1.),
                    second: (5., 1.),
                    end: (7., 1.),
                },
                P::QuadraticTo {
                    control: (7., 4.),
                    end: (7., 7.),
                },
                P::LineTo(1., 7.),
                P::Close,
            ],
            FillRule::NonZero,
        )
        .unwrap(),
    ];
    for (i, shape) in shapes.into_iter().enumerate() {
        let operations = EffectOperations {
            clip_path: Some(shape),
            ..Default::default()
        };
        let (mut c, source, _) = scene(operations, Rect::new(0., 0., 8., 8.), [255, 0, 0, 255]);
        let out = ReferenceCpuPresenter::default()
            .present(&c.build_frame(&source, 8, 8).unwrap())
            .unwrap();
        assert_eq!(out.get_pixel(0, 0).unwrap()[3], 0, "shape {i}");
        assert_eq!(
            out.get_pixel(3, 3).unwrap()[3],
            if i == 3 { 0 } else { 255 },
            "shape {i}"
        );
    }
    let mut image = Surface::new(2, 1);
    image.set_pixel(0, 0, [255, 255, 255, 128]);
    image.set_pixel(1, 0, [0, 0, 0, 255]);
    for mode in [MaskMode::Alpha, MaskMode::Luminance] {
        let operations = EffectOperations {
            mask: Some(EffectMask {
                image: Arc::new(image.clone()),
                rect: Rect::new(0., 0., 8., 8.),
                mode,
            }),
            ..Default::default()
        };
        let (mut c, source, _) = scene(operations, Rect::new(0., 0., 8., 8.), [255, 0, 0, 255]);
        let out = ReferenceCpuPresenter::default()
            .present(&c.build_frame(&source, 8, 8).unwrap())
            .unwrap();
        assert_eq!(out.get_pixel(1, 1).unwrap()[3], 128);
        assert_eq!(
            out.get_pixel(5, 1).unwrap()[3],
            if mode == MaskMode::Alpha { 255 } else { 0 }
        );
    }
}
#[test]
fn backdrop_filter_composites_scoped_image_source_over_parent() {
    let mut trees = PropertyTrees::new();
    let id = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    trees
        .set_effect_operations(
            id,
            EffectOperations {
                backdrop_filters: vec![F::Invert(1.)],
                backdrop_bounds: Some(Rect::new(0., 0., 4., 4.)),
                ..Default::default()
            },
        )
        .unwrap();
    let chunks = vec![
        PaintChunk {
            items: 0..1,
            bounds: Rect::new(0., 0., 8., 8.),
            state: PropertyTreeState::default(),
            reasons: CompositingReasons::default(),
        },
        PaintChunk {
            items: 1..2,
            bounds: Rect::new(0., 0., 4., 4.),
            state: PropertyTreeState {
                effect: id,
                ..Default::default()
            },
            reasons: CompositingReasons::default(),
        },
    ];
    let source = Blocks {
        items: vec![
            (Rect::new(0., 0., 8., 8.), [255, 0, 0, 128]),
            (Rect::new(0., 0., 4., 4.), [0; 4]),
        ],
        calls: Cell::new(0),
    };
    let mut c = Compositor::new(trees, chunks, 2).unwrap();
    let out = ReferenceCpuPresenter::default()
        .present(&c.build_frame(&source, 8, 8).unwrap())
        .unwrap();
    assert_eq!(out.get_pixel(1, 1), Some([85, 170, 170, 192]));
    assert_eq!(out.get_pixel(6, 6), Some([255, 0, 0, 128]));
}
#[test]
fn nested_isolation_changes_blending_backdrop() {
    fn draw(isolate: bool) -> [u8; 4] {
        let mut trees = PropertyTrees::new();
        let parent = trees
            .add_effect(EffectId(0), 1., BlendMode::Normal, None)
            .unwrap();
        let child = trees
            .add_effect(parent, 1., BlendMode::Normal, None)
            .unwrap();
        trees
            .set_effect_operations(
                parent,
                EffectOperations {
                    isolate,
                    ..Default::default()
                },
            )
            .unwrap();
        trees
            .set_effect_operations(
                child,
                EffectOperations {
                    blend: EffectBlend::Multiply,
                    ..Default::default()
                },
            )
            .unwrap();
        let r = Rect::new(0., 0., 4., 4.);
        let source = Blocks {
            items: vec![(r, [255, 0, 0, 255]), (r, [0, 0, 255, 255])],
            calls: Cell::new(0),
        };
        let mut c = Compositor::new(
            trees,
            vec![
                PaintChunk {
                    items: 0..1,
                    bounds: r,
                    state: PropertyTreeState::default(),
                    reasons: CompositingReasons::default(),
                },
                PaintChunk {
                    items: 1..2,
                    bounds: r,
                    state: PropertyTreeState {
                        effect: child,
                        ..Default::default()
                    },
                    reasons: CompositingReasons::default(),
                },
            ],
            2,
        )
        .unwrap();
        ReferenceCpuPresenter::default()
            .present(&c.build_frame(&source, 4, 4).unwrap())
            .unwrap()
            .get_pixel(1, 1)
            .unwrap()
    }
    assert_eq!(draw(false), [0, 0, 0, 255]);
    assert_eq!(draw(true), [0, 0, 255, 255]);
}
#[test]
fn effect_snapshot_roundtrip_and_v1_fail_closed() {
    let (mut c, source, _) = scene(
        EffectOperations {
            filters: vec![F::Grayscale(1.)],
            ..Default::default()
        },
        Rect::new(0., 0., 4., 4.),
        [255, 0, 0, 255],
    );
    let frame = c.build_frame(&source, 4, 4).unwrap();
    assert!(matches!(
        CompositorFrame::from_snapshot(frame.to_snapshot()),
        Err(SnapshotError::UnsupportedVersion)
    ));
    let restored = CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
    assert_eq!(
        ReferenceCpuPresenter::default().present(&frame).unwrap(),
        ReferenceCpuPresenter::default().present(&restored).unwrap()
    );
    let mut bad = frame.to_effects_snapshot();
    bad.operations[0].1.filters = vec![F::Blur(f32::NAN)];
    assert!(CompositorFrame::from_effects_snapshot(bad).is_err());
}
#[test]
fn promotion_squashing_limits_and_validation_are_explicit() {
    let reasons = PromotionHints {
        will_change_transform: true,
        will_change_opacity: true,
        animated_transform_or_opacity: true,
        transform_3d: true,
        video: true,
        canvas: true,
        fixed: true,
        sticky: true,
    }
    .reasons();
    for reason in [
        CompositingReasons::WILL_CHANGE,
        CompositingReasons::ANIMATED,
        CompositingReasons::TRANSFORM_3D,
        CompositingReasons::VIDEO,
        CompositingReasons::CANVAS,
        CompositingReasons::FIXED,
        CompositingReasons::STICKY,
    ] {
        assert!(reasons.contains(reason));
    }
    let mut chunks = (0..4096)
        .map(|i| PaintChunk {
            items: i..i + 1,
            state: PropertyTreeState::default(),
            bounds: Rect::new(0., 0., 4., 4.),
            reasons: CompositingReasons::WILL_CHANGE,
        })
        .collect::<Vec<_>>();
    let c = Compositor::new(PropertyTrees::new(), chunks.clone(), 4096).unwrap();
    assert_eq!(
        c.layer_counts(),
        LayerCounts {
            chunks: 4096,
            layers: 1,
            promoted: 1,
            squashed_chunks: 4095
        }
    );
    for c in &mut chunks {
        c.reasons = CompositingReasons::CANVAS;
    }
    assert!(matches!(
        Compositor::new(PropertyTrees::new(), chunks, 4096),
        Err(SceneError::TooManyLayers)
    ));
    let (mut c, _, id) = scene(
        EffectOperations::default(),
        Rect::new(0., 0., 4., 4.),
        [0; 4],
    );
    assert_eq!(
        c.set_effect_operations(
            id,
            EffectOperations {
                filters: vec![F::Blur(-1.)],
                ..Default::default()
            }
        ),
        Err(EffectError::InvalidValue)
    );
    assert!(!c.trees.has_composited_effects());
}

#[test]
fn nested_filters_apply_once_to_whole_group_and_clip_after_blur() {
    let mut trees = PropertyTrees::new();
    let parent = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    let child = trees
        .add_effect(parent, 1., BlendMode::Normal, None)
        .unwrap();
    trees
        .set_effect_operations(
            parent,
            EffectOperations {
                filters: vec![F::Invert(1.)],
                ..Default::default()
            },
        )
        .unwrap();
    trees
        .set_effect_operations(
            child,
            EffectOperations {
                filters: vec![F::Brightness(0.5)],
                ..Default::default()
            },
        )
        .unwrap();
    let transform = trees
        .add_transform(TransformId(0), Matrix4::IDENTITY, (0., 0., 0.), false)
        .unwrap();
    let r = Rect::new(0., 0., 8., 8.);
    let source = Blocks {
        items: vec![
            (r, [200, 0, 0, 255]),
            (Rect::new(4., 0., 4., 8.), [0, 0, 200, 255]),
        ],
        calls: Cell::new(0),
    };
    let mut c = Compositor::new(
        trees,
        vec![
            PaintChunk {
                items: 0..1,
                bounds: r,
                state: PropertyTreeState {
                    effect: child,
                    ..Default::default()
                },
                reasons: CompositingReasons::default(),
            },
            PaintChunk {
                items: 1..2,
                bounds: r,
                state: PropertyTreeState {
                    effect: child,
                    transform,
                    ..Default::default()
                },
                reasons: CompositingReasons::default(),
            },
        ],
        2,
    )
    .unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    let frame = c.build_frame(&source, 8, 8).unwrap();
    assert_eq!(frame.passes.len(), 3);
    let out = presenter.present(&frame).unwrap();
    assert_eq!(out.get_pixel(1, 1), Some([155, 255, 255, 255]));
    assert_eq!(out.get_pixel(5, 1), Some([255, 255, 155, 255]));
    c.set_effect_operations(
        parent,
        EffectOperations {
            filters: vec![F::Blur(1.)],
            clip_path: Some(EffectClip::Inset {
                rect: Rect::new(2., 2., 4., 4.),
                radii: [0.; 4],
            }),
            ..Default::default()
        },
    )
    .unwrap();
    let frame = c.build_frame(&source, 8, 8).unwrap();
    let out = presenter.present(&frame).unwrap();
    assert_eq!(out.get_pixel(1, 3).unwrap()[3], 0);
    assert!(out.get_pixel(3, 3).unwrap()[3] > 200);
}
#[test]
fn stale_bound_effect_batch_does_not_partially_apply() {
    let (mut c, _, id) = scene(
        EffectOperations::default(),
        Rect::new(0., 0., 4., 4.),
        [255; 4],
    );
    let owner = NodePropertyKey {
        node: 7,
        generation: 3,
        property: BoundProperty::Opacity,
    };
    c.bind_property_nodes(&[(owner, BoundPropertyNode::Opacity(id))])
        .unwrap();
    let operations = EffectOperations {
        filters: vec![F::Invert(1.)],
        ..Default::default()
    };
    let changes = [
        BoundEffectChange {
            owner,
            operations: operations.clone(),
        },
        BoundEffectChange {
            owner: NodePropertyKey {
                generation: 4,
                ..owner
            },
            operations: operations.clone(),
        },
    ];
    assert_eq!(
        c.apply_bound_effect_changes(&changes),
        Err(PropertyBindingError::UnknownKey)
    );
    assert!(!c.trees.has_composited_effects());
    c.apply_bound_effect_changes(&changes[..1]).unwrap();
    assert_eq!(c.trees.effect_operations(id), Some(&operations));
}
#[test]
fn backdrop_blur_mirrors_its_border_and_preserves_uniform_color() {
    let mut input = Surface::new(8, 8);
    input.fill_rect(2., 2., 4., 4., [255, 0, 0, 255]);
    let filtered =
        effect_pixels::backdrop_filters(&input, &[F::Blur(2.)], Rect::new(2., 2., 4., 4.));
    for y in 2..6 {
        for x in 2..6 {
            assert_eq!(filtered.get_pixel(x, y), Some([255, 0, 0, 255]));
        }
    }
}

#[test]
fn reserved_blur_animation_and_worker_jobs_keep_offscreen_tiles_ready() {
    let from = EffectOperations {
        filters: vec![F::Blur(0.)],
        ..Default::default()
    };
    let to = EffectOperations {
        filters: vec![F::Blur(3.)],
        ..Default::default()
    };
    let (mut c, source, id) = scene(from.clone(), Rect::new(-8., 0., 278., 8.), [255, 0, 0, 255]);
    c.reserve_effect_animation(id, &from, &to).unwrap();
    // Copy into an immutable source before worker submission; the borrowed
    // main-thread test counter is deliberately not shared with worker threads.
    struct ImmutableBlocks(Vec<(Rect, [u8; 4])>);
    impl PaintSource for ImmutableBlocks {
        fn item_count(&self) -> usize {
            self.0.len()
        }
        fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
            for (r, c) in &self.0[items] {
                out.fill_rect(r.x - tile.x, r.y - tile.y, r.w, r.h, *c);
            }
        }
    }
    let source = ImmutableBlocks(source.items);
    let first = c
        .build_parallel_frame(
            &source,
            TileRasterRequest {
                width: 262,
                height: 10,
                scroll_direction: (0., 0.),
                mode: TileRasterMode::Deterministic,
            },
        )
        .unwrap()
        .0;
    let mut p = ReferenceCpuPresenter::default();
    p.present(&first).unwrap();
    for step in 1..=4 {
        c.set_effect_operations(
            id,
            sample_effect_operations(&from, &to, step as f32 / 4.).unwrap(),
        )
        .unwrap();
        let frame = c.build_cached_frame(262, 10).unwrap();
        assert!(frame.resources.uploads.is_empty());
        assert!(p.present(&frame).unwrap().get_pixel(0, 4).unwrap()[3] > 180);
    }
}

#[test]
fn nested_animation_reservations_sum_ancestors_but_not_siblings() {
    let from = EffectOperations::default();
    let to = EffectOperations {
        filters: vec![F::Blur(3.)],
        ..Default::default()
    };
    let mut trees = PropertyTrees::new();
    let parent = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    let child = trees
        .add_effect(parent, 1., BlendMode::Normal, None)
        .unwrap();
    let sibling = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    let bounds = Rect::new(-20., 0., 40., 8.);
    let mut c = Compositor::new(
        trees,
        vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState {
                effect: child,
                ..Default::default()
            },
            bounds,
            reasons: CompositingReasons::WILL_CHANGE,
        }],
        1,
    )
    .unwrap();
    for id in [child, parent, sibling] {
        c.reserve_effect_animation(id, &from, &to).unwrap();
    }
    assert_eq!(c.effect_raster_halo(), 18.);
    // A repeated reservation must not reduce an earlier endpoint envelope.
    c.reserve_effect_animation(parent, &from, &from).unwrap();
    assert_eq!(c.effect_raster_halo(), 18.);
    let source = Blocks {
        items: vec![(bounds, [255, 0, 0, 255])],
        calls: Cell::new(0),
    };
    let first = c.build_frame(&source, 20, 8).unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    presenter.present(&first).unwrap();
    let calls = source.calls.get();
    for id in [parent, child] {
        c.set_effect_operations(id, to.clone()).unwrap();
    }
    let cached = c.build_cached_frame(20, 8).unwrap();
    assert!(cached.resources.uploads.is_empty());
    presenter.present(&cached).unwrap();
    assert_eq!(source.calls.get(), calls);
}

#[test]
fn plus_lighter_crossfade_saturation_and_opacity_match_integer_reference() {
    assert_eq!(effect_pixels::blend([255, 0, 0, 64], [0, 0, 255, 128],
        0.5, EffectBlend::PlusLighter), [128, 0, 128, 128]);
    for sa in [0u8, 1, 64, 128, 255] {
        for da in [0u8, 1, 64, 128, 255] {
            for op in [0u8, 1, 64, 128, 255] {
                for (s, d) in [(255, 255), (255, 0), (0, 255), (93, 171)] {
                    let effective = (u32::from(sa) * u32::from(op) + 127) / 255;
                    let alpha = (effective + u32::from(da)).min(255);
                    let color = if alpha == 0 { 0 } else {
                        ((u32::from(s) * effective + u32::from(d) * u32::from(da)
                            + alpha / 2) / alpha).min(255) as u8
                    };
                    assert_eq!(effect_pixels::blend([d, d, d, da], [s, s, s, sa],
                        f32::from(op) / 255., EffectBlend::PlusLighter),
                        [color, color, color, alpha as u8], "sa={sa}, da={da}, op={op}");
                }
            }
        }
    }
}

#[test]
fn plus_lighter_group_and_effect_snapshot_preserve_crossfade() {
    let bounds = Rect::new(0., 0., 8., 8.);
    let mut trees = PropertyTrees::new();
    let effect = trees.add_effect(EffectId(0), 1., BlendMode::Normal, None).unwrap();
    trees.set_effect_operations(effect, EffectOperations {
        blend: EffectBlend::PlusLighter, ..Default::default()
    }).unwrap();
    let chunks = vec![
        PaintChunk { items: 0..1, state: PropertyTreeState::default(), bounds,
            reasons: CompositingReasons::WILL_CHANGE },
        PaintChunk { items: 1..2, state: PropertyTreeState { effect, ..Default::default() },
            bounds, reasons: CompositingReasons::WILL_CHANGE },
    ];
    let source = Blocks { items: vec![(bounds, [255,0,0,64]), (bounds, [0,0,255,64])],
        calls: Cell::new(0) };
    let mut c = Compositor::new(trees, chunks, 2).unwrap();
    let frame = c.build_frame(&source, 8, 8).unwrap();
    let restored = CompositorFrame::from_effects_snapshot(frame.to_effects_snapshot()).unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    assert_eq!(presenter.present(&restored).unwrap().get_pixel(3,3), Some([128,0,128,128]));
    let calls = source.calls.get();
    let cached = c.build_cached_frame(8,8).unwrap();
    assert!(cached.resources.uploads.is_empty());
    assert_eq!(presenter.present(&cached).unwrap().get_pixel(3,3), Some([128,0,128,128]));
    assert_eq!(source.calls.get(), calls);
}
