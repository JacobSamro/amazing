//! `cargo run -p amazing-compositor --bin frame_trace --release -- 120`
//! Emits a reproducible full-repaint / retained-raster frame-time trace.
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
fn main() {
    let frames = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 10000)
        .unwrap_or(120);
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
    let mut retained = Compositor::new(trees, chunks, source.item_count()).unwrap();
    retained.render(&source, 640, 480).unwrap();
    let mut total_full = 0u128;
    let mut total_retained = 0u128;
    let mut retained_raster = 0u64;
    let mut full_raster = 0u64;
    println!("frame,full_repaint_us,compositor_us,full_raster_calls,compositor_raster_calls");
    for frame in 0..frames {
        let f = frame as f32;
        retained
            .set_scroll(scroll, ((f * 3.) % 220., (f * 2.) % 140.))
            .unwrap();
        retained
            .set_transform(transform, Matrix4::rotation_z((f * 0.01).sin() * 0.15))
            .unwrap();
        retained
            .set_opacity(effect, 0.5 + 0.5 * (f * 0.04).cos())
            .unwrap();
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
        assert_eq!(baseline, fast, "frame {frame} pixel mismatch");
        let full_calls = full.stats().raster_calls;
        let fast_calls = retained.stats().raster_calls;
        total_full += full_us;
        total_retained += fast_us;
        full_raster += full_calls;
        retained_raster += fast_calls;
        println!("{frame},{full_us},{fast_us},{full_calls},{fast_calls}")
    }
    eprintln!("frames={frames} full_mean_us={} compositor_mean_us={} full_raster_calls={full_raster} compositor_raster_calls={retained_raster}",total_full/frames as u128,total_retained/frames as u128);
}
