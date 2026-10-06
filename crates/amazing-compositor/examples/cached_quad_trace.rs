//! Native cached-frame sampler trace, with exact direct-render pixel checks.
//! This lower-level fixture does not replace the product's original Find control.
use amazing_compositor::*;
use amazing_paint::Surface;
use std::{ops::Range, time::Instant};

struct Source;
impl PaintSource for Source {
    fn item_count(&self) -> usize { 4 }
    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        for item in items {
            let y = item as f32 * 120.;
            out.fill_rect(-tile.x, y - tile.y, 614., 112.,
                [30 + item as u8 * 45, 60, 180 - item as u8 * 20, 255]);
        }
    }
}
fn run(scale: f32) {
    let mut trees = PropertyTrees::new();
    let transform = trees.add_transform(TransformId(0), Matrix4::scale(scale, scale, 1.),
        (0., 0., 0.), false).unwrap();
    let scroll = trees.add_scroll_with_owner(ScrollId(0), (0., 10.400002),
        Rect::new(0., 0., 614., 256.), Rect::new(0., 0., 614., 600.), Some(transform)).unwrap();
    let clip = trees.add_clip_with_scroll(ClipId(0), transform, ScrollId(0),
        ClipShape::RoundedRect { rect: Rect::new(0., 0., 614., 256.), radii: [8., 13., 5., 9.] }).unwrap();
    let chunks = (0..4).map(|item| PaintChunk { items: item..item + 1,
        state: PropertyTreeState { transform, scroll, clip, ..Default::default() },
        bounds: Rect::new(0., item as f32 * 120., 614., 112.),
        reasons: CompositingReasons::WILL_CHANGE }).collect();
    let mut scene = Compositor::new(trees, chunks, 4).unwrap();
    let sampling = scene.raster_sampling();
    let width = (614. * scale) as u32;
    let height = (256. * scale) as u32;
    let expected = scene.render(&Source, width, height).unwrap();
    let first = scene.build_frame(&Source, width, height).unwrap();
    let mut presenter = ReferenceCpuPresenter::default();
    assert_eq!(presenter.present(&first).unwrap(), expected);
    let cached = scene.build_cached_frame(width, height).unwrap();
    assert!(cached.resources.uploads.is_empty());
    let mut times = Vec::new();
    for _ in 0..12 {
        let started = Instant::now();
        let pixels = presenter.present(&cached).unwrap();
        times.push(started.elapsed().as_micros());
        assert_eq!(pixels, expected);
    }
    times.sort_unstable();
    println!("{scale},{sampling:?},{width},{height},{},{},{},0", cached.quad_count(), times[6], times[11]);
}
fn main() {
    println!("scale,sampling,width,height,quads,p50_us,max_us,pixel_losses");
    for scale in [1., 1.25, 2., 3.] {
        run(scale);
    }
}
