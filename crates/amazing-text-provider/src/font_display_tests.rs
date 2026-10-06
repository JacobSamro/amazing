use super::*;
use amazing_text::{GlyphVisibility, TextSpan, TextSystem};
use std::time::Duration;
fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}
fn face(display: FontDisplay) -> FontFaceDescriptor {
    FontFaceDescriptor {
        family: "Loading".into(),
        display,
        sources: vec![FontFaceSource::Url {
            url: "face.ttf".into(),
            formats: vec![],
            technologies: vec![],
        }],
        ..Default::default()
    }
}
struct Loader {
    state: FontSourceLoad,
    calls: Vec<String>,
}
impl FontFaceLoader for Loader {
    fn load_local(&mut self, name: &str, _: usize) -> Result<Option<Vec<u8>>, TextError> {
        self.calls.push(name.into());
        Ok(None)
    }
    fn load_url(&mut self, r: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> {
        self.calls.push(r.url.into());
        Ok(None)
    }
    fn load_url_state(&mut self, r: FontFetchRequest<'_>) -> Result<FontSourceLoad, TextError> {
        self.calls.push(r.url.into());
        Ok(match &self.state {
            FontSourceLoad::Pending => FontSourceLoad::Pending,
            FontSourceLoad::Failed => FontSourceLoad::Failed,
            FontSourceLoad::Ready(v) => FontSourceLoad::Ready(v.clone()),
        })
    }
}
fn loading(display: FontDisplay) -> (TextProvider, FontDisplayTimeline, FontFaceDescriptor) {
    let descriptor = face(display);
    let timeline = FontDisplayTimeline::new(descriptor.clone());
    let mut p = TextProvider::new();
    p.register_font_face_with_timeline(
        descriptor.clone(),
        &mut Loader {
            state: FontSourceLoad::Pending,
            calls: vec![],
        },
        &timeline,
        ms(0),
    )
    .unwrap();
    (p, timeline, descriptor)
}
fn style() -> TextStyle {
    TextStyle {
        families: vec!["Loading".into()],
        ..Default::default()
    }
}
fn shape(p: &TextProvider, text: &str) -> ShapedParagraph {
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 1,
        style: style(),
        boundary_before: amazing_text::ShapingBoundary::Hard,
    }];
    p.shape(Paragraph {
        text,
        spans: &spans,
        sources: &[],
        bidi_scopes: &[],
        base_direction: Direction::LeftToRight,
    })
    .unwrap()
}
fn raster(p: &TextProvider, run: &ShapedRun) -> Option<GlyphImage> {
    let g = &run.glyphs[0];
    let key = GlyphCacheKey::new(p.generation(), run.font, g.id, run.font_size, 0, 0).unwrap();
    p.rasterize(GlyphRasterRequest {
        key,
        variations: &run.variations,
        synthesis: run.synthesis,
    })
    .unwrap()
}
#[test]
fn all_display_period_boundaries_and_first_use_clock_are_native() {
    for (mode, block, swap) in [
        (FontDisplay::Auto, 3000, None),
        (FontDisplay::Block, 3000, None),
        (FontDisplay::Swap, 0, None),
        (FontDisplay::Fallback, 100, Some(3000)),
        (FontDisplay::Optional, 100, Some(0)),
    ] {
        let (mut p, t, _) = loading(mode);
        p.set_font_display_time(ms(5000)).unwrap();
        assert_eq!(
            t.snapshot(ms(5000)).unwrap().phase,
            FontDisplayPhase::NotUsed
        );
        shape(&p, "A");
        let expected = if block == 0 {
            FontDisplayPhase::Swap
        } else {
            FontDisplayPhase::Block
        };
        assert_eq!(t.snapshot(ms(5000)).unwrap().phase, expected);
        assert_eq!(t.snapshot(ms(5000)).unwrap().first_use, Some(ms(5000)));
        if block > 0 {
            assert_eq!(
                t.snapshot(ms(5000 + block - 1)).unwrap().phase,
                FontDisplayPhase::Block
            )
        }
        let at_block = if swap == Some(0) {
            FontDisplayPhase::Expired
        } else {
            FontDisplayPhase::Swap
        };
        assert_eq!(t.snapshot(ms(5000 + block)).unwrap().phase, at_block);
        if let Some(swap) = swap {
            assert_eq!(
                t.snapshot(ms(5000 + block + swap)).unwrap().phase,
                FontDisplayPhase::Expired
            )
        }
    }
}
#[test]
fn block_hides_actual_raster_coverage_and_preserves_fallback_metrics_and_advances() {
    let (mut p, t, _) = loading(FontDisplay::Block);
    let hidden = shape(&p, "AB");
    assert!(
        hidden
            .runs
            .iter()
            .all(|r| p.glyph_visibility(r.font).unwrap() == GlyphVisibility::InvisibleFallback)
    );
    assert!(raster(&p, &hidden.runs[0]).is_none());
    let before_metrics = hidden.runs[0].metrics;
    let before_advance: Vec<_> = hidden
        .runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(|g| g.advance_x))
        .collect();
    assert_eq!(
        p.font_display_face_state(&t).unwrap().metrics,
        FontDisplayMetrics::Fallback
    );
    let generation = p.generation();
    assert!(p.set_font_display_time(ms(3000)).unwrap());
    assert!(p.generation() > generation);
    let visible = shape(&p, "AB");
    assert_eq!(visible.runs[0].metrics, before_metrics);
    assert_eq!(
        visible
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| g.advance_x))
            .collect::<Vec<_>>(),
        before_advance
    );
    let image = raster(&p, &visible.runs[0]).expect("visible fallback glyph coverage");
    assert!(image.data.iter().any(|byte| *byte != 0));
    assert!(p.glyph_visibility(hidden.runs[0].font).is_err());
}
#[test]
fn swap_is_immediately_visible_and_failed_load_is_visible_even_during_block() {
    let (p, _, _) = loading(FontDisplay::Swap);
    assert!(raster(&p, &shape(&p, "A").runs[0]).is_some());
    let (mut p, t, d) = loading(FontDisplay::Block);
    shape(&p, "A");
    p.register_font_face_with_timeline(
        d,
        &mut Loader {
            state: FontSourceLoad::Failed,
            calls: vec![],
        },
        &t,
        ms(20),
    )
    .unwrap();
    assert_eq!(p.pending_font_loads(), 0);
    assert_eq!(t.snapshot(ms(20)).unwrap().phase, FontDisplayPhase::Failed);
    assert!(raster(&p, &shape(&p, "A").runs[0]).is_some());
}
#[test]
fn downloaded_font_arrival_before_first_use_and_before_expiry_is_selected() {
    for (mode, arrival, first_use) in [
        (FontDisplay::Optional, 8000, false),
        (FontDisplay::Optional, 99, true),
        (FontDisplay::Fallback, 3099, true),
        (FontDisplay::Block, 8000, true),
        (FontDisplay::Swap, 8000, true),
    ] {
        let (mut p, t, d) = loading(mode);
        if first_use {
            shape(&p, "A");
        }
        let key = p
            .register_font_face_with_timeline(
                d,
                &mut Loader {
                    state: FontSourceLoad::Ready(tests::synthetic_font('A')),
                    calls: vec![],
                },
                &t,
                ms(arrival),
            )
            .unwrap()
            .unwrap();
        let shaped = shape(&p, "A");
        assert_eq!(shaped.runs[0].font, key);
        assert_eq!(
            t.snapshot(ms(arrival)).unwrap().phase,
            FontDisplayPhase::Loaded
        );
        assert_eq!(
            p.font_display_face_state(&t).unwrap().metrics,
            FontDisplayMetrics::Downloaded(key)
        );
    }
}
#[test]
fn late_fallback_optional_completion_never_reactivates_after_failure_period() {
    for (mode, expiry) in [(FontDisplay::Fallback, 3100), (FontDisplay::Optional, 100)] {
        let (mut p, t, d) = loading(mode);
        shape(&p, "A");
        p.set_font_display_time(ms(expiry)).unwrap();
        let key = p
            .register_font_face_with_timeline(
                d,
                &mut Loader {
                    state: FontSourceLoad::Ready(tests::synthetic_font('A')),
                    calls: vec![],
                },
                &t,
                ms(expiry),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            t.snapshot(ms(expiry)).unwrap().phase,
            FontDisplayPhase::Expired
        );
        assert_ne!(shape(&p, "A").runs[0].font, key);
        p.set_font_display_time(ms(8000)).unwrap();
        assert_ne!(shape(&p, "A").runs[0].font, key);
        assert_eq!(
            p.font_display_face_state(&t).unwrap().metrics,
            FontDisplayMetrics::Fallback
        );
    }
}
#[test]
fn timeline_survives_provider_rebuild_and_original_deadline_without_clock_restart() {
    let (mut p, t, d) = loading(FontDisplay::Fallback);
    p.set_font_display_time(ms(500)).unwrap();
    shape(&p, "A");
    drop(p);
    let mut next = TextProvider::new();
    next.register_font_face_with_timeline(
        d.clone(),
        &mut Loader {
            state: FontSourceLoad::Pending,
            calls: vec![],
        },
        &t,
        ms(2000),
    )
    .unwrap();
    shape(&next, "A");
    assert_eq!(t.snapshot(ms(2000)).unwrap().first_use, Some(ms(500)));
    assert_eq!(next.next_font_display_deadline().unwrap(), Some(ms(3600)));
    next.set_font_display_time(ms(3600)).unwrap();
    assert_eq!(
        t.snapshot(ms(3600)).unwrap().phase,
        FontDisplayPhase::Expired
    );
    assert_eq!(
        next.set_font_display_time(ms(3599)),
        Err(TextError::InvalidStyle)
    );
    let mut wrong = d;
    wrong.family = "Other".into();
    assert!(
        next.register_font_face_with_timeline(
            wrong,
            &mut Loader {
                state: FontSourceLoad::Pending,
                calls: vec![]
            },
            &t,
            ms(3601)
        )
        .is_err()
    );
}
#[test]
fn unicode_ranges_and_pending_source_order_do_not_hide_unrelated_glyphs() {
    let mut d = face(FontDisplay::Block);
    d.unicode_ranges = vec![UnicodeRange { start: 65, end: 65 }];
    d.sources.push(FontFaceSource::Url {
        url: "second.ttf".into(),
        formats: vec![],
        technologies: vec![],
    });
    let t = FontDisplayTimeline::new(d.clone());
    let mut p = TextProvider::new();
    let mut loader = Loader {
        state: FontSourceLoad::Pending,
        calls: vec![],
    };
    p.register_font_face_with_timeline(d.clone(), &mut loader, &t, ms(0))
        .unwrap();
    assert_eq!(loader.calls, vec!["face.ttf"]);
    let shaped = shape(&p, "AB");
    assert_eq!(shaped.runs.len(), 2);
    assert_eq!(
        p.glyph_visibility(shaped.runs[0].font).unwrap(),
        GlyphVisibility::InvisibleFallback
    );
    assert_eq!(
        p.glyph_visibility(shaped.runs[1].font).unwrap(),
        GlyphVisibility::Visible
    );
    let spans = [TextSpan {
        range: 0..1,
        source: 1,
        style: TextStyle {
            families: vec!["Unrelated".into()],
            ..Default::default()
        },
        boundary_before: amazing_text::ShapingBoundary::Hard,
    }];
    let other = p
        .shape(Paragraph {
            text: "A",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(
        p.glyph_visibility(other.runs[0].font).unwrap(),
        GlyphVisibility::Visible
    );
}
#[test]
fn async_completion_order_preserves_declared_face_precedence_and_duplicate_completion() {
    let (mut p, a, d) = loading(FontDisplay::Block);
    let mut later = d.clone();
    later.sources[0] = FontFaceSource::Url {
        url: "later.ttf".into(),
        formats: vec![],
        technologies: vec![],
    };
    let b = FontDisplayTimeline::new(later.clone());
    let second = p
        .register_font_face_with_timeline(
            later,
            &mut Loader {
                state: FontSourceLoad::Ready(tests::synthetic_font('A')),
                calls: vec![],
            },
            &b,
            ms(0),
        )
        .unwrap()
        .unwrap();
    let first = p
        .register_font_face_with_timeline(
            d.clone(),
            &mut Loader {
                state: FontSourceLoad::Ready(tests::synthetic_font('A')),
                calls: vec![],
            },
            &a,
            ms(10),
        )
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(shape(&p, "A").runs[0].font, second);
    let count = p.fonts.len();
    assert_eq!(
        p.register_font_face_with_timeline(
            d,
            &mut Loader {
                state: FontSourceLoad::Failed,
                calls: vec![]
            },
            &a,
            ms(20)
        )
        .unwrap(),
        Some(first)
    );
    assert_eq!(p.fonts.len(), count);
}

#[test]
fn decoded_completion_clock_not_response_arrival_controls_expiry() {
    let (mut p, t, d) = loading(FontDisplay::Optional);
    shape(&p, "A");
    let count = Cell::new(0);
    let key = p
        .register_font_face_with_clock(
            d,
            &mut Loader {
                state: FontSourceLoad::Ready(tests::synthetic_font('A')),
                calls: vec![],
            },
            &t,
            || {
                let n = count.get();
                count.set(n + 1);
                ms(if n == 0 { 99 } else { 100 })
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(count.get(), 2);
    assert_eq!(
        t.snapshot(ms(100)).unwrap().phase,
        FontDisplayPhase::Expired
    );
    assert_ne!(shape(&p, "A").runs[0].font, key);
}

#[test]
fn rebuilt_provider_rejects_retained_invisible_glyphs_even_with_reused_font_slots() {
    let (old, t, d) = loading(FontDisplay::Block);
    let run = shape(&old, "A").runs.remove(0);
    let key = GlyphCacheKey::new(
        old.generation(),
        run.font,
        run.glyphs[0].id,
        run.font_size,
        0,
        0,
    )
    .unwrap();
    let mut next = TextProvider::new();
    next.register_font_face_with_timeline(
        d,
        &mut Loader {
            state: FontSourceLoad::Ready(tests::synthetic_font('A')),
            calls: vec![],
        },
        &t,
        ms(20),
    )
    .unwrap();
    assert_ne!(old.generation(), next.generation());
    assert_eq!(
        next.rasterize(GlyphRasterRequest {
            key,
            variations: &run.variations,
            synthesis: run.synthesis
        }),
        Err(TextError::MissingFont)
    );
    assert!(
        old.rasterize(GlyphRasterRequest {
            key,
            variations: &run.variations,
            synthesis: run.synthesis
        })
        .unwrap()
        .is_none()
    );
    assert_eq!(
        next.glyph_visibility(shape(&next, "A").runs[0].font)
            .unwrap(),
        GlyphVisibility::Visible
    );
}
