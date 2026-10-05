//! Bridge from the painter's read-only display-list view.
//!
//! The raw local raster leaves transforms, clips, opacity, blend and isolation to the
//! property trees. Unsupported paint commands use RC-16's baked range raster
//! with a neutral property state.

use super::*;
use amazing_paint::display_list_view::{
    DisplayListView, PaintBounds, PaintItemKind, RawRasterError, RetainedPaintView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintViewRasterMode {
    RawLocal,
    BakedFallback,
}

/// Apply the paint view's opaque page canvas to a source-free retained frame.
pub fn add_paint_view_canvas_background(mode: PaintViewRasterMode, frame: &mut CompositorFrame) {
    if mode == PaintViewRasterMode::RawLocal {
        let viewport = Rect::new(0., 0., frame.width as f32, frame.height as f32);
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
}

/// The raw mode rasterizes document-space item pixels once. Unsupported
/// commands use the existing baked full-paint path for pixel correctness.
pub struct PaintViewSource<'view, 'paint> {
    view: &'view DisplayListView<'paint>,
    mode: PaintViewRasterMode,
    filter_inputs: Vec<PaintFilterInput>,
}

impl<'view, 'paint> PaintViewSource<'view, 'paint> {
    pub fn new(view: &'view DisplayListView<'paint>) -> Self {
        let mut probe = Surface::new(1, 1);
        let mode = match view.raster_raw_local(0..view.items().len(), &mut probe, (0, 0)) {
            Ok(()) => PaintViewRasterMode::RawLocal,
            // `UnseparableItem` is a paint command outside the two supported
            // variants; `RasterFailed` is one of those two variants declining
            // a specific item it cannot separate (observed on real pages with
            // `border-collapse` table decorations: `raster_raw_item` returns
            // `false` for a `PaintDecorations` box with `suppress_background`,
            // `suppress_border` or non-empty `background_holes`, and for a
            // `PaintContent` box with non-empty table borders — all painted by
            // a different, non-per-item table code path). Both mean the same
            // thing to this probe: fall back to the pixel-exact baked range
            // raster rather than panicking on real-world content.
            Err(RawRasterError::UnseparableItem(_) | RawRasterError::RasterFailed(_)) => {
                PaintViewRasterMode::BakedFallback
            }
            Err(error) => panic!("raw paint-view probe failed: {error:?}"),
        };
        let imported = PainterFilterSource::from_view(view);
        let (mode, filter_inputs) = match imported {
            Ok(provider) => (mode, provider.filter_inputs().to_vec()),
            Err(_) => (PaintViewRasterMode::BakedFallback, Vec::new()),
        };
        Self {
            view,
            mode,
            filter_inputs,
        }
    }

    /// Attach producer-resolved filters to raw paint groups. Construction is
    /// atomic: unsupported spaces, nodes or baked sources return an error.
    pub fn with_filters(
        view: &'view DisplayListView<'paint>,
        provider: &impl PaintFilterSource,
    ) -> Result<Self, SceneError> {
        let mut source = Self::new(view);
        let inputs = provider.filter_inputs();
        if !inputs.is_empty() && source.mode != PaintViewRasterMode::RawLocal {
            return Err(SceneError::InvalidState);
        }
        paint_view_metadata_with_filters(view, inputs)?;
        source.filter_inputs = inputs.to_vec();
        Ok(source)
    }

    pub fn mode(&self) -> PaintViewRasterMode {
        self.mode
    }

    /// Resolve paint's stable DOM owner and property into this view's tree ID.
    /// Missing generations or ambiguous owners disable the property-only path.
    pub fn property_bindings(&self) -> Option<Vec<(NodePropertyKey, BoundPropertyNode)>> {
        if self.mode != PaintViewRasterMode::RawLocal {
            return None;
        }
        let owners = self.view.property_owner_mapping()?;
        let mut bindings = Vec::with_capacity(
            owners.transforms.len() + owners.effects.len() + owners.scrolls.len(),
        );
        for (owner, id) in owners.transforms {
            bindings.push((
                NodePropertyKey {
                    node: owner.node.0,
                    generation: owner.generation,
                    property: BoundProperty::Transform,
                },
                BoundPropertyNode::Transform(TransformId(id as usize)),
            ));
        }
        for (owner, id) in owners.effects {
            bindings.push((
                NodePropertyKey {
                    node: owner.node.0,
                    generation: owner.generation,
                    property: BoundProperty::Opacity,
                },
                BoundPropertyNode::Opacity(EffectId(id as usize)),
            ));
        }
        for (owner, id) in owners.scrolls {
            bindings.push((
                NodePropertyKey {
                    node: owner.node.0,
                    generation: owner.generation,
                    property: BoundProperty::Scroll,
                },
                BoundPropertyNode::Scroll(ScrollId(id as usize)),
            ));
        }
        Some(bindings)
    }

    pub fn compositor(&self, width: u32, height: u32) -> Result<Compositor, SceneError> {
        if self
            .view
            .raster_viewport()
            .is_some_and(|viewport| viewport.physical_size() != (width, height))
        {
            return Err(SceneError::InvalidState);
        }
        match self.mode {
            PaintViewRasterMode::RawLocal => {
                let (trees, chunks) =
                    paint_view_metadata_with_filters(self.view, &self.filter_inputs)?;
                let mut grouped: Vec<PaintChunk> = Vec::new();
                for chunk in chunks {
                    // Keep the public one-record-per-item metadata contract.
                    // Only scene emission omits finite Content records with no
                    // owned ink; stable view keys and item ranges are unchanged.
                    let item = self
                        .view
                        .items()
                        .get(chunk.items.start)
                        .ok_or(SceneError::InvalidState)?;
                    let b = chunk.bounds;
                    if item.kind == PaintItemKind::Content
                        && [b.x, b.y, b.w, b.h].iter().all(|v| v.is_finite())
                        && b.w >= 0.
                        && b.h >= 0.
                        && (b.w == 0. || b.h == 0.)
                    {
                        continue;
                    }
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
                    .map(|scene| scene.with_raster_sampling(RasterSampling::Nearest))
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
        if self
            .view
            .raster_viewport()
            .is_some_and(|viewport| viewport.physical_size() != (width, height))
        {
            return Err(SceneError::InvalidState);
        }
        // The genuine baked scene has one full-view range. An embedding may
        // supply another compositor; optimize a shared range only, retaining
        // original dispatch for mixed-range scenes rather than combining them.
        let common = compositor.chunks.first().map(|chunk| chunk.items.clone())
            .filter(|range| compositor.chunks.iter().all(|chunk| chunk.items == *range));
        let frame = if self.mode == PaintViewRasterMode::BakedFallback {
            if let Some(range) = common {
                let source = FrameBakedSource { view: self.view, range,
                    pixels: std::cell::RefCell::new(None) };
                compositor.build_frame(&source, width, height)?
            } else { compositor.build_frame(self, width, height)? }
        } else { compositor.build_frame(self, width, height)? };
        Ok(self.with_canvas_background(frame))
    }

    /// Assemble a frame from ready worker tiles. The caller retains its last
    /// complete frame when this returns `MissingCachedTile`.
    pub fn build_cached_frame(
        &self,
        compositor: &mut Compositor,
        width: u32,
        height: u32,
    ) -> Result<CompositorFrame, SceneError> {
        if self
            .view
            .raster_viewport()
            .is_some_and(|viewport| viewport.physical_size() != (width, height))
        {
            return Err(SceneError::InvalidState);
        }
        let frame = compositor.build_cached_frame(width, height)?;
        Ok(self.with_canvas_background(frame))
    }

    fn with_canvas_background(&self, mut frame: CompositorFrame) -> CompositorFrame {
        if self.mode == PaintViewRasterMode::RawLocal {
            let viewport = Rect::new(0., 0., frame.width as f32, frame.height as f32);
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
        frame
    }

    /// Build from RC-20's owned rasters while preserving the paint-view
    /// canvas background used by the ordinary raw-local frame path.
    pub fn build_parallel_frame(
        &self,
        compositor: &mut Compositor,
        retained: &RetainedPaintView,
        request: TileRasterRequest,
    ) -> Result<(CompositorFrame, TileRasterBatch), SceneError> {
        let (frame, batch) = compositor.build_parallel_frame(retained, request)?;
        Ok((self.with_canvas_background(frame), batch))
    }
}

// Borrowed for exactly one build_frame call. Lazy preparation avoids replay
// on an unchanged cached frame. Drop releases pixels before returning a frame.
struct FrameBakedSource<'view, 'paint> {
    view: &'view DisplayListView<'paint>,
    range: Range<usize>,
    pixels: std::cell::RefCell<Option<amazing_paint::display_list_view::PreparedBakedRaster>>,
}
impl PaintSource for FrameBakedSource<'_, '_> {
    fn item_count(&self) -> usize { self.view.items().len() }
    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        assert_eq!(items, self.range, "frame baked range changed");
        let mut pixels = self.pixels.borrow_mut();
        if pixels.is_none() {
            *pixels = Some(self.view.prepare_baked_range(items)
                .expect("paint view range raster failed"));
        }
        out.clear([255; 4]);
        assert!(pixels.as_ref().unwrap().copy_to(out,
            (-(tile.x as i32), -(tile.y as i32)),
            PaintBounds { x: 0., y: 0., width: tile.w, height: tile.h }),
            "paint view range copy failed");
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

/// RC-20's owned item rasters are immutable and thread-safe, so tile workers
/// can replay them without borrowing layout, text, images or the main thread.
impl PaintSource for RetainedPaintView {
    fn item_count(&self) -> usize {
        self.items().count()
    }

    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
        for item in self.items().skip(items.start).take(items.len()) {
            let Some(raster) = item.key.and_then(|key| self.raster(key)) else {
                continue;
            };
            let (ox, oy) = raster.origin;
            let src = raster.surface;
            let x0 = (tile.x as i32).max(ox);
            let y0 = (tile.y as i32).max(oy);
            let x1 = ((tile.x + tile.w) as i32).min(ox + src.width as i32);
            let y1 = ((tile.y + tile.h) as i32).min(oy + src.height as i32);
            for y in y0..y1 {
                for x in x0..x1 {
                    let si = (((y - oy) as usize * src.width as usize) + (x - ox) as usize) * 4;
                    let di = (((y - tile.y as i32) as usize * out.width as usize)
                        + (x - tile.x as i32) as usize)
                        * 4;
                    let source = <[u8; 4]>::try_from(&src.pixels[si..si + 4]).unwrap();
                    if source[3] == 0 {
                        continue;
                    }
                    let dest = <[u8; 4]>::try_from(&out.pixels[di..di + 4]).unwrap();
                    out.pixels[di..di + 4]
                        .copy_from_slice(&amazing_paint::blend_source_over(source, dest, 255));
                }
            }
        }
    }
}

/// Converts the view's stable per-build property IDs and direct hints.
pub(crate) fn paint_view_metadata_base(
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
    for node in view.scrolls().iter().skip(1) {
        let bounds = rect(node.bounds);
        trees
            .add_scroll_with_owner(
                ScrollId(node.parent as usize),
                node.initial_offset,
                bounds,
                rect(node.contents),
                Some(TransformId(node.transform as usize)),
            )
            .map_err(|_| SceneError::InvalidState)?;
    }
    for node in view.clips().iter().skip(1) {
        trees
            .add_clip_with_scroll(
                ClipId(node.parent as usize),
                TransformId(node.transform as usize),
                ScrollId(node.scroll as usize),
                ClipShape::Rect(rect(node.bounds)),
            )
            .map_err(|_| SceneError::InvalidState)?;
    }
    for node in view.effects().iter().skip(1) {
        let id = trees
            .add_effect(
                EffectId(node.parent as usize),
                node.opacity,
                BlendMode::Normal,
                None,
            )
            .map_err(|_| SceneError::InvalidState)?;
        // RC-24 exports CSS blend values; retain them on the same effect ID
        // so ancestry, opacity bindings and cached frames preserve the group.
        trees
            .set_effect_operations(
                id,
                EffectOperations {
                    blend: resolve_css_blend(node.blend).map_err(|_| SceneError::InvalidState)?,
                    isolate: node.isolate,
                    ..Default::default()
                },
            )
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
            if h.fixed {
                reasons = reasons.union(CompositingReasons::FIXED);
            }
            if h.sticky {
                reasons = reasons.union(CompositingReasons::STICKY);
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
