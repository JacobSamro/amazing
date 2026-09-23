//! Bridge from the painter's read-only display-list view.
//!
//! RC-17's raw local raster leaves transforms, clips and opacity to the
//! property trees. Unsupported paint commands use RC-16's baked range raster
//! with a neutral property state.

use super::*;
use amazing_paint::display_list_view::{DisplayListView, PaintBounds, RawRasterError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintViewRasterMode {
    RawLocal,
    BakedFallback,
}

/// The raw mode rasterizes document-space item pixels once. Unsupported
/// commands use the existing baked full-paint path for pixel correctness.
pub struct PaintViewSource<'view, 'paint> {
    view: &'view DisplayListView<'paint>,
    mode: PaintViewRasterMode,
}

impl<'view, 'paint> PaintViewSource<'view, 'paint> {
    pub fn new(view: &'view DisplayListView<'paint>) -> Self {
        let mut probe = Surface::new(1, 1);
        let mode = match view.raster_raw_local(0..view.items().len(), &mut probe, (0, 0)) {
            Ok(()) => PaintViewRasterMode::RawLocal,
            Err(RawRasterError::UnseparableItem(_)) => PaintViewRasterMode::BakedFallback,
            Err(error) => panic!("raw paint-view probe failed: {error:?}"),
        };
        Self { view, mode }
    }

    pub fn mode(&self) -> PaintViewRasterMode {
        self.mode
    }

    pub fn compositor(&self, width: u32, height: u32) -> Result<Compositor, SceneError> {
        match self.mode {
            PaintViewRasterMode::RawLocal => {
                let (trees, chunks) = paint_view_metadata(self.view)?;
                let mut grouped: Vec<PaintChunk> = Vec::new();
                for chunk in chunks {
                    if let Some(last) = grouped.last_mut() {
                        if last.state == chunk.state && last.items.end == chunk.items.start {
                            last.items.end = chunk.items.end;
                            last.bounds = last.bounds.union(chunk.bounds);
                            last.reasons = last.reasons.union(chunk.reasons);
                            continue;
                        }
                    }
                    grouped.push(chunk);
                }
                Compositor::new(trees, grouped, self.view.items().len())
            }
            PaintViewRasterMode::BakedFallback => {
                baked_paint_view_compositor(self.view, width, height)
            }
        }
    }
    /// Add the painter's default opaque canvas behind retained local tiles.
    pub fn build_frame(
        &self,
        compositor: &mut Compositor,
        width: u32,
        height: u32,
    ) -> Result<CompositorFrame, SceneError> {
        let mut frame = compositor.build_frame(self, width, height)?;
        if self.mode == PaintViewRasterMode::RawLocal {
            let viewport = Rect::new(0., 0., width as f32, height as f32);
            frame.passes[0].quads.insert(
                0,
                DrawQuad::SolidColor(SolidColorQuad {
                    rect: viewport,
                    color: [255, 255, 255, 255],
                    clip_rect: viewport,
                    opacity: 1.,
                    blend: BlendMode::Normal,
                }),
            );
        }
        Ok(frame)
    }
}

impl PaintSource for PaintViewSource<'_, '_> {
    fn item_count(&self) -> usize {
        self.view.items().len()
    }

    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        match self.mode {
            PaintViewRasterMode::RawLocal => {
                self.view
                    .raster_raw_local(items, out, (tile.x as i32, tile.y as i32))
                    .expect("raw paint-view item changed support after scene construction");
            }
            PaintViewRasterMode::BakedFallback => self.view.rasterize(items, tile, out),
        }
    }
}

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

/// Converts the view's stable per-build property IDs and direct hints.
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
                    clip: if h.fixed {
                        ClipId(0)
                    } else {
                        ClipId(p.clip as usize)
                    },
                    effect: EffectId(p.effect as usize),
                    scroll: ScrollId(p.scroll as usize),
                    positioning: if h.fixed {
                        Positioning::Fixed
                    } else {
                        Positioning::Flow
                    },
                },
                bounds: rect(item.local_bounds),
                reasons,
            }
        })
        .collect();
    Ok((trees, chunks))
}

/// Builds the pixel-exact fallback for paint commands RC-17 cannot separate.
/// Its one paint range preserves the painter's group semantics and neutral
/// property state avoids applying those groups twice.
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
