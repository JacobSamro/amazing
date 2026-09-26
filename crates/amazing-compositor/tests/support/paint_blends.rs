use amazing_compositor::*;
use amazing_paint::{controls::NoControls, display_list_view::DisplayListView,
    paint_document_with_text_resources_and_controls, NoImages};
use amazing_text::FallbackTextSystem;

pub fn run(legacy: bool) -> (usize, u64, usize) {
    let modes = ["normal", "multiply", "screen", "overlay", "darken", "lighten",
        "color-dodge", "color-burn", "hard-light", "soft-light", "difference",
        "exclusion", "hue", "saturation", "color", "luminosity", "plus-lighter"];
    let mut failures = 0;
    let mut losses = 0;
    let mut unexpected = 0;
    println!("mode,case,layers,exact,rounding,losses");
    for mode in modes {
        for (case, outer, inner) in [
            ("opaque", "", ""),
            ("alpha", "", "background:rgba(180,70,130,.5);"),
            ("opacity", "opacity:.5;", ""),
            ("isolate", "isolation:isolate;", ""),
            ("transform", "isolation:isolate;transform:translate(2px,3px);", ""),
        ] {
            let text = FallbackTextSystem::new();
            let css = format!("body{{margin:0;background:rgb(90,170,210)}}.outer{{width:40px;height:40px;{outer}}}.inner{{width:24px;height:24px;background:rgb(180,70,130);mix-blend-mode:{mode};{inner}}}");
            let root = amazing_layout::layout_document_with_text(
                "<body><div class=outer><div class=inner></div></div></body>", &css, 64., &text);
            let view = DisplayListView::new(&root, 64, 64, &text, &NoImages, &NoControls);
            let source = PaintViewSource::new(&view);
            assert_eq!(source.mode(), PaintViewRasterMode::RawLocal);
            let mut compositor = source.compositor(64, 64).unwrap();
            for (i, node) in view.effects().iter().enumerate().skip(1) {
                let expected = resolve_css_blend(node.blend).unwrap();
                let ops = compositor.trees().effect_operations(EffectId(i)).cloned().unwrap_or_default();
                assert_eq!(ops.blend, expected);
                assert_eq!(ops.isolate, node.isolate);
                assert_eq!(compositor.trees().effect(EffectId(i)).unwrap().opacity, node.opacity);
                if legacy {
                    // Reproduce the old importer's opacity-only metadata.
                    compositor.set_effect_operations(EffectId(i), EffectOperations::default()).unwrap();
                }
            }
            let frame = source.build_frame(&mut compositor, 64, 64).unwrap();
            let mut presenter = ReferenceCpuPresenter::default();
            let actual = presenter.present(&frame).unwrap();
            let full = paint_document_with_text_resources_and_controls(
                &root, 64, 64, &text, &NoImages, &NoControls);
            let parity = classify_pixel_parity(&presenter, &actual, &full);
            println!("{mode},{case},{},{},{},{}", compositor.layer_counts().layers,
                parity.exact, parity.layer_rounding, parity.losses);
            if parity.losses != 0 {
                failures += 1;
                eprintln!("{mode}/{case}: {:?}", parity.first_loss);
                // RC-24 reference differences, tracked separately from import
                // regressions. Keep strict pixel pairs: a new difference even
                // in these cases is unexpected. Do not loosen parity globally.
                let known_pair = match (mode, case) {
                    ("color-burn", "opaque") =>
                        Some(([21, 0, 167, 255], [22, 0, 167, 255])),
                    ("plus-lighter", "alpha") =>
                        Some(([180, 205, 255, 255], [173, 205, 233, 255])),
                    _ => None,
                };
                let known = known_pair.is_some_and(|(candidate, reference)| {
                    actual.pixels.chunks_exact(4).zip(full.pixels.chunks_exact(4))
                        .all(|(a, b)| a == b || (a == candidate && b == reference))
                });
                if !known { unexpected += 1; }
            }
            losses += parity.losses;
            let cached = source.build_cached_frame(&mut compositor, 64, 64).unwrap();
            assert_eq!(presenter.present(&cached).unwrap(), actual, "{mode}/{case}: cached");
        }
    }
    (failures, losses, unexpected)
}
