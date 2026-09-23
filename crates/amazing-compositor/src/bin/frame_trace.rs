//! `cargo run -p amazing-compositor --bin frame_trace --release -- 60`
//! CSV trace and p50/p95 for full repaint versus damage-driven composition.
use amazing_compositor::*;
use amazing_paint::Surface;
use std::ops::Range;
use std::time::Instant;
struct Synthetic {
    rects: Vec<(Rect, [u8; 4])>,
}
impl PaintSource for Synthetic {
    fn item_count(&self) -> usize {
        self.rects.len()
    }
    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        for i in items {
            let (r, color) = self.rects[i];
            out.fill_rect(r.x - tile.x, r.y - tile.y, r.w, r.h, color);
            for stripe in 0..256u16 {
                let shade = [
                    color[0].wrapping_add((stripe * 3) as u8),
                    color[1].wrapping_add((stripe * 5) as u8),
                    color[2],
                    255,
                ];
                out.fill_rect(
                    r.x - tile.x + (stripe % 46) as f32,
                    r.y - tile.y,
                    0.5,
                    r.h,
                    shade,
                )
            }
        }
    }
}
fn fixture() -> (Compositor, Synthetic, TransformId, ScrollId, EffectId) {
    let mut trees = PropertyTrees::new();
    let scroll = trees
        .add_scroll(
            ScrollId(0),
            (0., 0.),
            Rect::new(0., 0., 640., 480.),
            Rect::new(0., 0., 1024., 768.),
        )
        .unwrap();
    let transform = trees
        .add_transform(TransformId(0), Matrix4::IDENTITY, (320., 240., 0.), false)
        .unwrap();
    let effect = trees
        .add_effect(EffectId(0), 1., BlendMode::Normal, None)
        .unwrap();
    let state = PropertyTreeState {
        transform,
        scroll,
        effect,
        clip: ClipId(0),
        positioning: Positioning::Flow,
    };
    let mut source = Synthetic { rects: Vec::new() };
    let mut chunks = Vec::new();
    for y in 0..12 {
        for x in 0..16 {
            let rect = Rect::new(x as f32 * 56., y as f32 * 52., 46., 42.);
            let color = [(x * 13) as u8, (y * 19) as u8, ((x + y) * 7) as u8, 255];
            let index = source.rects.len();
            source.rects.push((rect, color));
            chunks.push(PaintChunk {
                items: index..index + 1,
                state,
                bounds: rect,
                reasons: CompositingReasons::default(),
            })
        }
    }
    (
        Compositor::new(trees, chunks, source.item_count()).unwrap(),
        source,
        transform,
        scroll,
        effect,
    )
}
fn quantile(values: &[u128], p: f64) -> u128 {
    let mut v = values.to_vec();
    v.sort_unstable();
    v[((v.len() - 1) as f64 * p).ceil() as usize]
}
fn run(name: &str, frames: usize) {
    let (mut retained, source, transform, scroll, effect) = fixture();
    let (mut quad, _, _, _, _) = fixture();
    let mut presenter = ReferenceCpuPresenter::default();
    retained.render(&source, 640, 480).unwrap();
    if name == "animation" {
        let timing = Timing {
            delay_ms: 0.,
            duration_ms: 1000.,
            iterations: 1.,
            direction: Direction::Normal,
            fill: Fill::Forwards,
            easing: Easing::CubicBezier(0.25, 0.1, 0.25, 1.),
        };
        retained
            .add_animation(
                0.,
                timing.clone(),
                AnimationTarget::Transform(transform),
                vec![
                    Keyframe {
                        offset: 0.,
                        value: AnimatedValue::Transform(Matrix4::IDENTITY),
                        easing: Easing::Linear,
                    },
                    Keyframe {
                        offset: 1.,
                        value: AnimatedValue::Transform(Matrix4::rotation_z(0.15)),
                        easing: Easing::Linear,
                    },
                ],
            )
            .unwrap();
        quad.add_animation(
            0.,
            timing.clone(),
            AnimationTarget::Transform(transform),
            vec![
                Keyframe {
                    offset: 0.,
                    value: AnimatedValue::Transform(Matrix4::IDENTITY),
                    easing: Easing::Linear,
                },
                Keyframe {
                    offset: 1.,
                    value: AnimatedValue::Transform(Matrix4::rotation_z(0.15)),
                    easing: Easing::Linear,
                },
            ],
        )
        .unwrap();
        retained
            .add_animation(
                0.,
                timing.clone(),
                AnimationTarget::Opacity(effect),
                vec![
                    Keyframe {
                        offset: 0.,
                        value: AnimatedValue::Opacity(1.),
                        easing: Easing::Linear,
                    },
                    Keyframe {
                        offset: 1.,
                        value: AnimatedValue::Opacity(0.5),
                        easing: Easing::Linear,
                    },
                ],
            )
            .unwrap();
        quad.add_animation(
            0.,
            timing,
            AnimationTarget::Opacity(effect),
            vec![
                Keyframe {
                    offset: 0.,
                    value: AnimatedValue::Opacity(1.),
                    easing: Easing::Linear,
                },
                Keyframe {
                    offset: 1.,
                    value: AnimatedValue::Opacity(0.5),
                    easing: Easing::Linear,
                },
            ],
        )
        .unwrap();
    }
    let (mut full_times, mut fast_times, mut quad_times) = (Vec::new(), Vec::new(), Vec::new());
    let (mut full_calls, mut fast_calls) = (0u64, 0u64);
    for frame in 0..frames {
        let f = frame as f32;
        match name {
            "scroll" => {
                for c in [&mut retained, &mut quad] {
                    c.set_scroll(scroll, (f + 1., 0.)).unwrap()
                }
            }
            "rotate" => {
                for c in [&mut retained, &mut quad] {
                    c.set_transform(transform, Matrix4::rotation_z((f + 1.) * 0.0025))
                        .unwrap()
                }
            }
            "fade" => {
                for c in [&mut retained, &mut quad] {
                    c.set_opacity(effect, 1. - (f + 1.) / (frames as f32 * 2.))
                        .unwrap()
                }
            }
            "animation" => {
                retained.tick((frame as f64 + 1.) * 1000. / 60.).unwrap();
                quad.tick((frame as f64 + 1.) * 1000. / 60.).unwrap();
            }
            _ => unreachable!(),
        };
        let mut full = Compositor::new(
            retained.trees().clone(),
            retained.chunks().to_vec(),
            source.item_count(),
        )
        .unwrap();
        let a = Instant::now();
        let baseline = full.render(&source, 640, 480).unwrap();
        let full_us = a.elapsed().as_micros();
        let b = Instant::now();
        let fast = retained.render(&source, 640, 480).unwrap();
        let fast_us = b.elapsed().as_micros();
        assert_eq!(baseline, fast, "{name} frame {frame} pixel mismatch");
        let c = Instant::now();
        let quad_frame = quad.build_frame(&source, 640, 480).unwrap();
        let quad_us = c.elapsed().as_micros();
        let presented = presenter.present(&quad_frame).unwrap();
        assert_eq!(
            baseline, presented,
            "{name} frame {frame} quad pixel mismatch"
        );
        full_times.push(full_us);
        fast_times.push(fast_us);
        quad_times.push(quad_us);
        full_calls += full.stats().raster_calls;
        fast_calls += retained.stats().raster_calls;
        println!(
            "{name},{frame},{full_us},{fast_us},{quad_us},{},{},{},{},{},{},{}",
            full.stats().raster_calls,
            retained.stats().raster_calls,
            retained.stats().blitted_pixels,
            retained.stats().damaged_pixels,
            quad_frame.quad_count(),
            quad_frame.resources.uploads.len(),
            quad_frame.resources.evictions.len(),
        )
    }
    eprintln!("scenario={name} frames={frames} full_p50_us={} full_p95_us={} compositor_p50_us={} compositor_p95_us={} quad_build_p50_us={} quad_build_p95_us={} full_raster_calls={full_calls} compositor_raster_calls={fast_calls}",quantile(&full_times,0.5),quantile(&full_times,0.95),quantile(&fast_times,0.5),quantile(&fast_times,0.95),quantile(&quad_times,0.5),quantile(&quad_times,0.95));
}
fn main() {
    let frames = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 10000)
        .unwrap_or(60);
    println!("scenario,frame,full_repaint_us,compositor_us,quad_build_us,full_raster_calls,compositor_raster_calls,blitted_pixels,damaged_pixels,quad_count,uploads,evictions");
    for name in ["scroll", "rotate", "fade", "animation"] {
        run(name, if name == "animation" { 60 } else { frames })
    }
}
