//! Bridge from the painter's read-only display-list view.
//!
//! RC-16's range raster already applies its group transforms, clips and
//! opacity. `baked_paint_view_compositor` therefore uses a neutral property
//! state for presentation. `paint_view_metadata` preserves the source property
//! IDs and compositing hints for inspection and hit-test integration. Moving
//! those properties onto compositor quads requires RC-17's raw local raster.

use super::*;
use amazing_paint::display_list_view::{DisplayListView, PaintBounds};

fn rect(bounds: PaintBounds) -> Rect {
    Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)
}

impl PaintSource for DisplayListView<'_> {
    fn item_count(&self) -> usize {
        self.items().len()
    }

    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        // The paint view currently replays into a full-viewport temporary
        // surface before copying this tile. Its bool reports invalid ranges
        // and budget exhaustion; silently showing an incomplete frame would
        // turn a resource limit into a misleading parity pass.
        out.clear([255; 4]);
        assert!(
            self.raster_range(
                items,
                out,
                (-(tile.x as i32), -(tile.y as i32)),
                PaintBounds {
                    x: 0.,
                    y: 0.,
                    width: tile.w,
                    height: tile.h,
                },
            ),
            "paint view range raster failed"
        );
    }
}

/// Converts the view's stable per-build property IDs and direct hints. The
/// returned scene is metadata until raw uncomposited item pixels are exposed.
pub fn paint_view_metadata(
    view: &DisplayListView<'_>,
) -> Result<(PropertyTrees, Vec<PaintChunk>), SceneError> {
    let mut trees = PropertyTrees::new();
    for node in view.transforms().iter().skip(1) {
        let [a, b, c, d, e, f] = node.matrix;
        trees
            .add_transform(
                TransformId(node.parent as usize),
                Matrix4::affine(a, b, c, d, e, f),
                (0., 0., 0.),
                false,
            )
            .map_err(|_| SceneError::InvalidState)?;
    }
    for node in view.clips().iter().skip(1) {
        trees
            .add_clip(
                ClipId(node.parent as usize),
                TransformId(0),
                ClipShape::Rect(rect(node.bounds)),
            )
            .map_err(|_| SceneError::InvalidState)?;
    }
    for node in view.effects().iter().skip(1) {
        trees
            .add_effect(
                EffectId(node.parent as usize),
                node.opacity,
                BlendMode::Normal,
                None,
            )
            .map_err(|_| SceneError::InvalidState)?;
    }
    for node in view.scrolls().iter().skip(1) {
        let bounds = rect(node.bounds);
        trees
            .add_scroll(ScrollId(node.parent as usize), (0., 0.), bounds, bounds)
            .map_err(|_| SceneError::InvalidState)?;
    }
    let chunks = view
        .items()
        .iter()
        .map(|item| {
            let p = item.properties;
            let h = item.hints;
            let mut reasons = CompositingReasons::default();
            if h.will_change {
                reasons = reasons.union(CompositingReasons::WILL_CHANGE);
            }
            if h.fixed || h.sticky {
                reasons = reasons.union(CompositingReasons::FIXED);
            }
            if h.video {
                reasons = reasons.union(CompositingReasons::VIDEO);
            }
            if h.scroller {
                reasons = reasons.union(CompositingReasons::SCROLLER);
            }
            PaintChunk {
                items: item.index..item.index + 1,
                state: PropertyTreeState {
                    transform: TransformId(p.transform as usize),
                    clip: ClipId(p.clip as usize),
                    effect: EffectId(p.effect as usize),
                    scroll: ScrollId(p.scroll as usize),
                },
                bounds: rect(item.local_bounds),
                reasons,
            }
        })
        .collect();
    Ok((trees, chunks))
}

/// Builds the pixel-exact RC-16 path. Its one paint range preserves the
/// painter's group semantics, while its compositor state remains neutral to
/// avoid applying those groups twice. Use `paint_view_metadata` separately for
/// source item inspection. RC-17 will allow property-driven retained tiles.
pub fn baked_paint_view_compositor(
    view: &DisplayListView<'_>,
    width: u32,
    height: u32,
) -> Result<Compositor, SceneError> {
    let viewport = Rect::new(0., 0., width as f32, height as f32);
    Compositor::new(
        PropertyTrees::new(),
        vec![PaintChunk {
            items: 0..view.items().len(),
            state: PropertyTreeState::default(),
            bounds: viewport,
            reasons: CompositingReasons::default(),
        }],
        view.items().len(),
    )
}
