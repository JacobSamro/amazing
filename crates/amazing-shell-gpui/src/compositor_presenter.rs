//! GPUI image resources for compositor frames.
//!
//! GPUI's pinned `paint_image` accepts axis-aligned bounds and an opacity but
//! has no 4x4 transform for color sprites. This presenter uploads tile deltas
//! as `RenderImage`s, exposes their ordered quad records for the viewport, and
//! supplies a pixel-exact composed image for unsupported transforms and passes.
//! Live viewport adoption belongs to integrator-assist.

use std::collections::HashMap;
use std::sync::Arc;

use amazing_compositor::{
    CompositorFrame, DrawQuad, Presenter, PresenterError, Rect, ReferenceCpuPresenter, ResourceId,
    SolidColorQuad, TileQuad, RasterSampling,
};
use gpui::{div, img, px, rgba, AnyElement, IntoElement, ParentElement, RenderImage, Styled};
use image::{Frame, RgbaImage};

#[derive(Clone)]
pub struct GpuiTile {
    pub quad: TileQuad,
    pub image: Arc<RenderImage>,
}

#[derive(Clone)]
enum GpuiDraw {
    Tile(GpuiTile),
    Solid(SolidColorQuad),
}

#[derive(Clone)]
pub struct GpuiFrame {
    /// Exact fallback image for rotations, grouped effects and complex clips.
    pub composed: Option<Arc<RenderImage>>,
    /// Uploaded tile images in back-to-front draw order.
    pub tiles: Vec<GpuiTile>,
    draws: Vec<GpuiDraw>,
    pub damage: Vec<Rect>,
    pub width: u32,
    pub height: u32,
    /// GPUI can place these quads directly with axis-aligned image bounds.
    pub direct_tiles: bool,
}

impl GpuiFrame {
    /// Builds the GPUI element for this frame. The common axis-aligned path
    /// draws cached tile images directly; GPUI's color-sprite API cannot
    /// transform them, so complex frames use the exact composed image.
    pub fn element(&self) -> AnyElement {
        self.element_with_device_scale(1.)
    }

    /// Incoming frame coordinates are physical raster pixels. GPUI expects
    /// logical pixels and applies its own display scale during presentation.
    pub fn element_with_device_scale(&self, device_scale: f32) -> AnyElement {
        assert!(device_scale.is_finite() && device_scale > 0.);
        if !self.direct_tiles {
            return img(self.composed.as_ref().expect("fallback image").clone())
                .w(px((self.width as f32) / device_scale))
                .h(px((self.height as f32) / device_scale))
                .into_any_element();
        }
        let mut root = div()
            .relative()
            .w(px((self.width as f32) / device_scale))
            .h(px((self.height as f32) / device_scale))
            .overflow_hidden();
        for draw in &self.draws {
            if let GpuiDraw::Solid(quad) = draw {
                let x = quad.rect.x.max(quad.clip_rect.x);
                let y = quad.rect.y.max(quad.clip_rect.y);
                let right = (quad.rect.x + quad.rect.w).min(quad.clip_rect.x + quad.clip_rect.w);
                let bottom = (quad.rect.y + quad.rect.h).min(quad.clip_rect.y + quad.clip_rect.h);
                if right > x && bottom > y {
                    let [r, g, b, a] = quad.color;
                    let color =
                        ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32;
                    root = root.child(
                        div()
                            .absolute()
                            .left(px((x) / device_scale))
                            .top(px((y) / device_scale))
                            .w(px((right - x) / device_scale))
                            .h(px((bottom - y) / device_scale))
                            .bg(rgba(color))
                            .opacity(quad.opacity),
                    );
                }
                continue;
            }
            let GpuiDraw::Tile(tile) = draw else {
                unreachable!()
            };
            let Some(world) = tile.quad.transform.map_bounds(tile.quad.rect) else {
                continue;
            };
            let x = world.x.max(tile.quad.clip_rect.x);
            let y = world.y.max(tile.quad.clip_rect.y);
            let right = (world.x + world.w).min(tile.quad.clip_rect.x + tile.quad.clip_rect.w);
            let bottom = (world.y + world.h).min(tile.quad.clip_rect.y + tile.quad.clip_rect.h);
            if right <= x || bottom <= y {
                continue;
            }
            root = root.child(
                div()
                    .absolute()
                    .left(px((x) / device_scale))
                    .top(px((y) / device_scale))
                    .w(px((right - x) / device_scale))
                    .h(px((bottom - y) / device_scale))
                    .overflow_hidden()
                    .child(
                        img(tile.image.clone())
                            .absolute()
                            .left(px((world.x - x) / device_scale))
                            .top(px((world.y - y) / device_scale))
                            .w(px((world.w) / device_scale))
                            .h(px((world.h) / device_scale))
                            .opacity(tile.quad.opacity),
                    ),
            );
        }
        root.into_any_element()
    }
}

#[derive(Default)]
pub struct GpuiPresenter {
    cpu: ReferenceCpuPresenter,
    tiles: HashMap<ResourceId, Arc<RenderImage>>,
    last_composed: Option<Arc<RenderImage>>,
    last_sampling: Option<RasterSampling>,
}

fn render_image(width: u32, height: u32, mut rgba: Vec<u8>) -> Arc<RenderImage> {
    // GPUI's RenderImage atlas expects BGRA bytes even though `image::RgbaImage`
    // provides the storage type. Match the shell's existing conversion path.
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image =
        RgbaImage::from_raw(width, height, rgba).expect("validated compositor resource dimensions");
    Arc::new(RenderImage::new([Frame::new(image)]))
}

fn axis_aligned_unit_transform(quad: &TileQuad, sampling: RasterSampling) -> bool {
    // GPUI has no nearest texture-sampler selector. A fractional physical
    // translation must use the policy-aware CPU composition. The existing
    // Bilinear admission also stays conservative: only whole-pixel placement.
    let requires_integer_translation = match sampling {
        RasterSampling::Nearest | RasterSampling::Bilinear => true,
    };
    let m = quad.transform.0;
    m[0][0] == 1.
        && m[0][1] == 0.
        && m[1][0] == 0.
        && m[1][1] == 1.
        && m[2] == [0., 0., 1., 0.]
        && m[3] == [0., 0., 0., 1.]
        && (!requires_integer_translation
            || (m[0][3].fract() == 0. && m[1][3].fract() == 0.))
}

fn intersect_rects(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    Rect::new(x, y, (right - x).max(0.), (bottom - y).max(0.))
}

/// Matches `amazing_compositor::property`'s private `nested_effect_opacity`
/// exactly (same 8-bit `quantize_opacity` rounding both there and in
/// `amazing_paint::blend`'s own `blend_source_over`/`blend_group`): hoisting
/// a pass's tiles out to the direct-tile path must combine its opacity with
/// an already-flattened descendant's the same way the reference painter's
/// two-step blend (render the group into a transparent surface, then
/// composite that surface into its parent) would, or COMP-19's pixel-parity
/// proof against `ReferenceCpuPresenter` fails on any opacity other than a
/// clean round trip.
fn combine_opacity(outer: f32, inner: f32) -> f32 {
    let outer = u32::from(amazing_paint::quantize_opacity(outer));
    let inner = u32::from(amazing_paint::quantize_opacity(inner));
    ((outer * inner + 127) / 255) as f32 / 255.
}

/// True when this pass's own effect (if any) does nothing GPUI's direct
/// per-tile placement cannot already express: no filter/backdrop/mask, no
/// `local_transform` (COMP-16's per-pixel inverse-sampled compositing), and
/// a plain `Normal` blend. Filters and masks reshape pixels in ways only a
/// real raster pass can reproduce; a `local_transform` group renders its
/// children at local identity and needs `DrawQuad::Pass`'s own inverse-map
/// sampling, which a stack of absolutely-positioned tiles cannot replicate.
fn effect_operations_are_trivial(ops: Option<&amazing_compositor::EffectOperations>) -> bool {
    ops.is_none_or(|ops| {
        ops.filters.is_empty()
            && ops.backdrop_filters.is_empty()
            && ops.clip_path.is_none()
            && ops.mask.is_none()
            && ops.local_transform.is_none()
            && ops.blend == amazing_compositor::EffectBlend::Normal
    })
}

/// True when every `ClipShape` from `id` up to the root is a plain `Rect`.
/// `TileQuad`/`SolidColorQuad`/`PassQuad`'s own `clip_rect` is always just
/// the clip chain's axis-aligned bounding box (see `quad.rs`'s `clip_rect`
/// helper) -- exact for a rect clip, but a rounded corner it would square
/// off, which is why an ordinary (non-passed) tile under a rounded ancestor
/// clip is never left on this direct path either (`quad.rs`'s own
/// `rounded_clip` forces `needs_pass` there). A hoisted pass must pass the
/// same test, since hoisting relies on nothing but `clip_rect` intersection.
fn clip_chain_is_rect_only(trees: &amazing_compositor::PropertyTrees, mut id: amazing_compositor::ClipId) -> bool {
    while id.0 != 0 {
        let Some(node) = trees.clip(id) else { return false };
        if !matches!(node.shape, amazing_compositor::ClipShape::Rect(_)) {
            return false;
        }
        id = node.parent;
    }
    true
}

#[cfg(test)]
std::thread_local! {
    // Count every actual GPUI presentation on the operation's thread, including
    // an accidentally introduced second presenter. Reference-only validation
    // does not present a GPUI frame and deliberately contributes no count.
    static COMPLETED_PRESENTATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl GpuiPresenter {
    #[cfg(test)]
    pub(crate) fn completed_presentations(&self) -> u64 { COMPLETED_PRESENTATIONS.with(std::cell::Cell::get) }

    /// Recursively flattens render pass `pass_id` into direct-tile draws when
    /// every draw it contains (or a pass it in turn references) is provably
    /// safe to place with GPUI's own `opacity`/absolute-bounds API instead of
    /// `ReferenceCpuPresenter`'s full CPU recompose (COMP-19): a `Normal`
    /// blend throughout, no filter/backdrop/mask/`local_transform` on any
    /// effect node in the chain, only rect-shaped clips, and only
    /// axis-aligned unit-scale tile transforms. `accum_opacity`/`accum_clip`
    /// carry the enclosing pass chain's already-combined opacity (via
    /// `combine_opacity`, the same 8-bit rounding a real nested blend would
    /// produce) and clip intersection down into this pass's own quads.
    ///
    /// Returns `Ok(None)` -- not `Err` -- the moment anything here is not
    /// flatten-safe; that is a normal, expected outcome (a filtered or
    /// rounded-clip group, say), and the caller falls back to a full
    /// recompose for the whole frame exactly as it always could, just for a
    /// smaller set of passes than before this ticket. `Err` stays reserved
    /// for a genuinely broken frame (a tile referencing a resource that was
    /// never uploaded), which is a bug regardless of hoisting.
    fn hoist_pass(
        &self,
        frame: &CompositorFrame,
        pass_id: usize,
        accum_opacity: f32,
        accum_clip: Rect,
        depth: usize,
    ) -> Result<Option<(Vec<GpuiTile>, Vec<GpuiDraw>)>, PresenterError> {
        // Matches `ReferenceCpuPresenter::draw_pass`'s own `depth > 16` guard
        // (`PresenterError::PassCycle`): flattening walks the same pass graph,
        // so it is bounded the same way rather than trusting it is acyclic.
        if depth > 16 {
            return Ok(None);
        }
        let Some(pass) = frame.passes.get(pass_id) else {
            return Ok(None);
        };
        if let Some(id) = pass.effect_node {
            if !effect_operations_are_trivial(frame.trees.effect_operations(id)) {
                return Ok(None);
            }
        }
        let mut tiles = Vec::new();
        let mut draws = Vec::new();
        for quad in &pass.quads {
            match quad {
                DrawQuad::Tile(tile) => {
                    if !axis_aligned_unit_transform(tile, frame.raster_sampling)
                        || tile.blend != amazing_compositor::BlendMode::Normal
                    {
                        return Ok(None);
                    }
                    let image = self
                        .tiles
                        .get(&tile.resource_id)
                        .ok_or(PresenterError::MissingResource(tile.resource_id))?
                        .clone();
                    let mut hoisted = tile.clone();
                    hoisted.opacity = combine_opacity(accum_opacity, tile.opacity);
                    hoisted.clip_rect = intersect_rects(accum_clip, tile.clip_rect);
                    let draw = GpuiTile {
                        quad: hoisted,
                        image,
                    };
                    draws.push(GpuiDraw::Tile(draw.clone()));
                    tiles.push(draw);
                }
                DrawQuad::SolidColor(solid) => {
                    if solid.blend != amazing_compositor::BlendMode::Normal {
                        return Ok(None);
                    }
                    let mut hoisted = solid.clone();
                    hoisted.opacity = combine_opacity(accum_opacity, solid.opacity);
                    hoisted.clip_rect = intersect_rects(accum_clip, solid.clip_rect);
                    draws.push(GpuiDraw::Solid(hoisted));
                }
                DrawQuad::Pass(nested) => {
                    if nested.blend != amazing_compositor::BlendMode::Normal
                        || !clip_chain_is_rect_only(&frame.trees, nested.state.clip)
                    {
                        return Ok(None);
                    }
                    let nested_clip = intersect_rects(
                        accum_clip,
                        intersect_rects(nested.rect, nested.clip_rect),
                    );
                    let nested_opacity = combine_opacity(accum_opacity, nested.opacity);
                    match self.hoist_pass(frame, nested.pass_id, nested_opacity, nested_clip, depth + 1)? {
                        Some((mut nested_tiles, mut nested_draws)) => {
                            tiles.append(&mut nested_tiles);
                            draws.append(&mut nested_draws);
                        }
                        None => return Ok(None),
                    }
                }
            }
        }
        Ok(Some((tiles, draws)))
    }
}

impl Presenter for GpuiPresenter {
    type Output = GpuiFrame;
    type Error = PresenterError;

    fn present(&mut self, frame: &CompositorFrame) -> Result<Self::Output, Self::Error> {
        if self.last_sampling != Some(frame.raster_sampling) {
            self.last_composed = None;
            self.last_sampling = Some(frame.raster_sampling);
        }
        for id in &frame.resources.evictions {
            self.tiles.remove(id);
        }
        for upload in &frame.resources.uploads {
            let len = (upload.width as usize)
                .checked_mul(upload.height as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or(PresenterError::InvalidResource)?;
            if upload.pixels.len() != len {
                return Err(PresenterError::InvalidResource);
            }
            self.tiles.insert(
                upload.id,
                render_image(upload.width, upload.height, upload.pixels.clone()),
            );
        }
        let viewport = Rect::new(0., 0., frame.width as f32, frame.height as f32);
        let mut tiles = Vec::new();
        let mut draws = Vec::new();
        let mut direct_tiles = true;
        for quad in &frame.root().quads {
            match quad {
                DrawQuad::Tile(tile) => {
                    let image = self
                        .tiles
                        .get(&tile.resource_id)
                        .ok_or(PresenterError::MissingResource(tile.resource_id))?
                        .clone();
                    direct_tiles &= axis_aligned_unit_transform(tile, frame.raster_sampling)
                        && tile.blend == amazing_compositor::BlendMode::Normal;
                    let draw = GpuiTile {
                        quad: tile.clone(),
                        image,
                    };
                    draws.push(GpuiDraw::Tile(draw.clone()));
                    tiles.push(draw);
                }
                DrawQuad::SolidColor(solid) => {
                    direct_tiles &= solid.blend == amazing_compositor::BlendMode::Normal;
                    draws.push(GpuiDraw::Solid(solid.clone()));
                }
                // COMP-19: a pass whose own effect is opacity/blend-only (no
                // filter/backdrop/mask/local_transform) and whose clip chain
                // is rect-only can be hoisted straight into this frame's
                // direct-tile draws, reusing cached tiles instead of forcing
                // `ReferenceCpuPresenter`'s full CPU recompose for the whole
                // frame just because one group somewhere needs isolation.
                DrawQuad::Pass(pass_quad) => {
                    let safe = pass_quad.blend == amazing_compositor::BlendMode::Normal
                        && clip_chain_is_rect_only(&frame.trees, pass_quad.state.clip);
                    let hoisted = if safe {
                        let clip = intersect_rects(
                            viewport,
                            intersect_rects(pass_quad.rect, pass_quad.clip_rect),
                        );
                        self.hoist_pass(frame, pass_quad.pass_id, pass_quad.opacity, clip, 1)?
                    } else {
                        None
                    };
                    match hoisted {
                        Some((mut hoisted_tiles, mut hoisted_draws)) => {
                            tiles.append(&mut hoisted_tiles);
                            draws.append(&mut hoisted_draws);
                        }
                        None => direct_tiles = false,
                    }
                }
            }
        }
        let composed = if direct_tiles {
            self.cpu.resources.apply(&frame.resources)?;
            self.last_composed = None;
            None
        } else if frame.damage.is_empty() && self.last_composed.is_some() {
            self.cpu.resources.apply(&frame.resources)?;
            self.last_composed.clone()
        } else {
            let surface = self.cpu.present(frame)?;
            let image = render_image(surface.width, surface.height, surface.pixels);
            self.last_composed = Some(image.clone());
            Some(image)
        };
        #[cfg(test)]
        { COMPLETED_PRESENTATIONS.with(|count| count.set(count.get().saturating_add(1))); }
        Ok(GpuiFrame {
            composed,
            tiles,
            draws,
            damage: frame.damage.clone(),
            width: frame.width,
            height: frame.height,
            direct_tiles,
        })
    }
}

#[cfg(not(test))]
struct ProofPaint;

#[cfg(not(test))]
impl amazing_compositor::PaintSource for ProofPaint {
    fn item_count(&self) -> usize {
        2
    }
    fn rasterize(
        &self,
        items: std::ops::Range<usize>,
        tile: Rect,
        out: &mut amazing_paint::Surface,
    ) {
        for index in items {
            let (rect, color) = if index == 0 {
                (Rect::new(0., 0., 160., 120.), [240, 242, 248, 255])
            } else {
                (Rect::new(16., 14., 100., 80.), [180, 55, 32, 255])
            };
            out.fill_rect(rect.x - tile.x, rect.y - tile.y, rect.w, rect.h, color);
        }
    }
}

#[cfg(not(test))]
struct ProofWindow {
    frame: GpuiFrame,
    reference: Arc<RenderImage>,
}

#[cfg(not(test))]
impl gpui::Render for ProofWindow {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .w(px(320.))
            .h(px(120.))
            .child(self.frame.element())
            .child(img(self.reference.clone()).w(px(160.)).h(px(120.)))
    }
}

#[cfg(not(test))]
pub fn main() {
    use amazing_compositor::{
        CompositingReasons, Compositor, PaintChunk, PropertyTreeState, PropertyTrees,
    };
    use gpui::{size, App, AppContext, Bounds, WindowBounds, WindowOptions};
    use gpui_platform::application;

    let mut compositor = Compositor::new(
        PropertyTrees::new(),
        vec![
            PaintChunk {
                items: 0..1,
                state: PropertyTreeState::default(),
                bounds: Rect::new(0., 0., 160., 120.),
                reasons: CompositingReasons::default(),
            },
            PaintChunk {
                items: 1..2,
                state: PropertyTreeState::default(),
                bounds: Rect::new(16., 14., 100., 80.),
                reasons: CompositingReasons::default(),
            },
        ],
        2,
    )
    .unwrap();
    let frame = compositor.build_frame(&ProofPaint, 160, 120).unwrap();
    let reference_surface = ReferenceCpuPresenter::default().present(&frame).unwrap();
    let reference = render_image(
        reference_surface.width,
        reference_surface.height,
        reference_surface.pixels,
    );
    let proof = GpuiPresenter::default().present(&frame).unwrap();
    assert!(proof.direct_tiles);
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(320.), px(120.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| ProofWindow {
                    frame: proof,
                    reference,
                })
            },
        )
        .expect("open compositor GPUI proof window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_compositor::{
        CompositingReasons, Compositor, EffectId, Matrix4, PaintChunk, PaintSource,
        PropertyTreeState, PropertyTrees, TransformId,
    };
    use amazing_paint::Surface;
    use std::ops::Range;

    struct Paint;
    impl PaintSource for Paint {
        fn item_count(&self) -> usize {
            1
        }
        fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
            out.fill_rect(2. - tile.x, 3. - tile.y, 8., 7., [90, 40, 180, 255]);
        }
    }

    #[test]
    fn gpui_resource_upload_and_frame_match_cpu_reference() {
        let mut compositor = Compositor::new(
            PropertyTrees::new(),
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState::default(),
                bounds: Rect::new(2., 3., 8., 7.),
                reasons: CompositingReasons::default(),
            }],
            1,
        )
        .unwrap();
        let mut gpui = GpuiPresenter::default();
        let frame = compositor.build_frame(&Paint, 16, 16).unwrap();
        let actual = gpui.present(&frame).unwrap();
        assert!(actual.direct_tiles);
        assert!(actual.composed.is_none());
        let _ = actual.element();
        assert_eq!(actual.tiles.len(), 1);
        let mut bytes = actual.tiles[0].image.as_bytes(0).unwrap().to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        assert_eq!(bytes, frame.resources.uploads[0].pixels);
        let next = compositor.build_frame(&Paint, 16, 16).unwrap();
        assert!(next.resources.uploads.is_empty());
        assert!(next.damage.is_empty());
        let again = gpui.present(&next).unwrap();
        assert!(again.composed.is_none());
        assert!(Arc::ptr_eq(&actual.tiles[0].image, &again.tiles[0].image));
    }

    #[test]
    fn nearest_fractional_tiles_and_hoisted_passes_use_exact_composition() {
        for translation in [(2., 3.), (2.5, 3.75), (-1.25, 2.5)] {
            for grouped in [false, true] {
                let mut trees = PropertyTrees::new();
                let transform = trees.add_transform(
                    TransformId(0), Matrix4::translation(translation.0, translation.1, 0.),
                    (0., 0., 0.), false,
                ).unwrap();
                let effect = if grouped {
                    trees.add_effect(EffectId(0), 0.5,
                        amazing_compositor::BlendMode::Normal, None).unwrap()
                } else { EffectId(0) };
                let mut scene = Compositor::new(trees, vec![PaintChunk {
                    items: 0..1,
                    state: PropertyTreeState { transform, effect, ..Default::default() },
                    bounds: Rect::new(2., 3., 270., 7.),
                    reasons: CompositingReasons::WILL_CHANGE,
                }], 1).unwrap();
                let mut frame = scene.build_frame(&Paint, 300, 32).unwrap();
                frame.raster_sampling = RasterSampling::Nearest;
                let expected = ReferenceCpuPresenter::default().present(&frame).unwrap();
                let mut presenter = GpuiPresenter::default();
                let actual = presenter.present(&frame).unwrap();
                let integral = translation.0.fract() == 0. && translation.1.fract() == 0.;
                assert_eq!(actual.direct_tiles, integral, "{translation:?}/{grouped}");
                if !integral {
                    let mut bytes = actual.composed.as_ref().unwrap().as_bytes(0).unwrap().to_vec();
                    for pixel in bytes.chunks_exact_mut(4) { pixel.swap(0, 2); }
                    assert_eq!(bytes, expected.pixels, "{translation:?}/{grouped}");
                    let mut cached = frame.clone();
                    cached.damage.clear();
                    cached.resources.uploads.clear();
                    let again = presenter.present(&cached).unwrap();
                    assert!(Arc::ptr_eq(actual.composed.as_ref().unwrap(),
                        again.composed.as_ref().unwrap()));
                    let mut changed = frame.clone();
                    changed.raster_sampling = RasterSampling::Bilinear;
                    let changed_expected = ReferenceCpuPresenter::default().present(&changed).unwrap();
                    changed.damage.clear();
                    changed.resources.uploads.clear();
                    let changed_actual = presenter.present(&changed).unwrap();
                    assert!(!Arc::ptr_eq(again.composed.as_ref().unwrap(),
                        changed_actual.composed.as_ref().unwrap()));
                    let mut changed_bytes = changed_actual.composed.as_ref().unwrap()
                        .as_bytes(0).unwrap().to_vec();
                    for pixel in changed_bytes.chunks_exact_mut(4) { pixel.swap(0, 2); }
                    assert_eq!(changed_bytes, changed_expected.pixels);
                }
            }
        }
    }

    #[test]
    fn raw_page_canvas_solid_stays_on_ordered_direct_path() {
        let mut compositor = Compositor::new(
            PropertyTrees::new(),
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState::default(),
                bounds: Rect::new(2., 3., 8., 7.),
                reasons: CompositingReasons::default(),
            }],
            1,
        )
        .unwrap();
        let mut frame = compositor.build_frame(&Paint, 16, 16).unwrap();
        let solid = SolidColorQuad {
            rect: Rect::new(0., 0., 16., 16.),
            color: [255, 255, 255, 255],
            clip_rect: Rect::new(0., 0., 12., 12.),
            opacity: 0.75,
            blend: amazing_compositor::BlendMode::Normal,
        };
        frame.passes[0]
            .quads
            .insert(0, DrawQuad::SolidColor(solid.clone()));
        let result = GpuiPresenter::default().present(&frame).unwrap();
        assert!(result.direct_tiles);
        assert!(result.composed.is_none());
        assert_eq!(result.tiles.len(), 1);
        assert!(matches!(&result.draws[0], GpuiDraw::Solid(value) if value == &solid));
        assert!(matches!(&result.draws[1], GpuiDraw::Tile(_)));
        let _ = result.element();
    }

    #[test]
    fn rotated_pass_frame_uses_exact_fallback() {
        // Unlike an effect pass (COMP-19, below), a rotation can never be
        // hoisted: GPUI's direct tile placement only has axis-aligned bounds
        // and an opacity, with no 4x4 transform, so this must stay on the
        // exact CPU-composed fallback regardless of any effect-pass fix.
        let mut trees = PropertyTrees::new();
        let transform = trees
            .add_transform(
                TransformId(0),
                Matrix4::rotation_z(0.2),
                (0., 0., 0.),
                false,
            )
            .unwrap();
        let mut compositor = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    transform,
                    ..Default::default()
                },
                bounds: Rect::new(2., 3., 8., 7.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        let frame = compositor.build_frame(&Paint, 16, 16).unwrap();
        let mut presenter = GpuiPresenter::default();
        let result = presenter.present(&frame).unwrap();
        assert!(!result.direct_tiles);
        let expected = ReferenceCpuPresenter::default().present(&frame).unwrap();
        let mut bytes = result.composed.unwrap().as_bytes(0).unwrap().to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        assert_eq!(bytes, expected.pixels);
    }

    /// COMP-19: a plain (no filter/backdrop/mask/transform) opacity effect
    /// spanning more than one raster tile still needs its own `RenderPass`
    /// (`quad.rs`'s `needs_pass`, since a multi-tile group can't carry its
    /// opacity on a single `TileQuad` the way `single_tile_opacity_and_
    /// rect_clip_stay_on_gpui_tile_path` above does) -- but that pass is now
    /// hoisted straight into the direct-tile path instead of forcing
    /// `ReferenceCpuPresenter`'s full CPU recompose for the whole frame.
    #[test]
    fn multi_tile_opacity_pass_hoists_onto_the_direct_tile_path() {
        let mut trees = PropertyTrees::new();
        let effect = trees
            .add_effect(
                EffectId(0),
                0.5,
                amazing_compositor::BlendMode::Normal,
                None,
            )
            .unwrap();
        let mut compositor = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    effect,
                    ..Default::default()
                },
                bounds: Rect::new(2., 3., 270., 7.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        // A wide-enough viewport that both 256px raster tiles the 270px-wide
        // bounds span are actually visible (a narrower viewport would still
        // trigger `needs_pass` on the bounds' full logical span, but only
        // raster the one tile still on screen -- not the multi-tile case
        // this test means to exercise).
        let frame = compositor.build_frame(&Paint, 300, 16).unwrap();
        assert_eq!(frame.passes.len(), 2, "270px-wide bounds must span 2 tiles and need a pass");
        let mut presenter = GpuiPresenter::default();
        let result = presenter.present(&frame).unwrap();
        assert!(result.direct_tiles, "a plain opacity-only pass must now hoist");
        assert!(result.composed.is_none());
        assert_eq!(result.tiles.len(), 2, "one tile per raster tile the bounds span");
        for tile in &result.tiles {
            // Same 8-bit round trip as the single-tile case above: root
            // opacity 1.0 (255) combined with this effect's 0.5 (128) is
            // 128/255, not the raw 0.5.
            assert!((tile.quad.opacity - 128.0 / 255.0).abs() < 1e-6);
        }
        let _ = result.element();

        // Pixel parity against a full repaint (COMP-19's other proof
        // obligation): feed the hoisted, flattened draws back through
        // `ReferenceCpuPresenter` as a single root-only pass, sharing the
        // original frame's resources/trees/tile_lookup (untouched by
        // hoisting -- only `TileQuad::opacity`/`clip_rect` changed), and
        // check it matches the original multi-pass frame's own real
        // CPU-composed output exactly. If `combine_opacity`'s quantized
        // rounding or `intersect_rects`' clip intersection ever drifted from
        // what an isolated pass actually blends, this catches it here rather
        // than as a silent visual regression.
        let mut synthetic = frame.clone();
        synthetic.passes = vec![amazing_compositor::RenderPass {
            id: 0,
            effect_node: None,
            clip_node: None,
            quads: result
                .draws
                .iter()
                .map(|draw| match draw {
                    GpuiDraw::Tile(tile) => DrawQuad::Tile(tile.quad.clone()),
                    GpuiDraw::Solid(solid) => DrawQuad::SolidColor(solid.clone()),
                })
                .collect(),
        }];
        let flattened = ReferenceCpuPresenter::default().present(&synthetic).unwrap();
        let original = ReferenceCpuPresenter::default().present(&frame).unwrap();
        assert_eq!(flattened.pixels, original.pixels);
    }

    #[test]
    fn single_tile_opacity_and_rect_clip_stay_on_gpui_tile_path() {
        let mut trees = PropertyTrees::new();
        let clip = trees
            .add_clip(
                amazing_compositor::ClipId(0),
                TransformId(0),
                amazing_compositor::ClipShape::Rect(Rect::new(0., 0., 12., 12.)),
            )
            .unwrap();
        let effect = trees
            .add_effect(
                EffectId(0),
                0.5,
                amazing_compositor::BlendMode::Normal,
                None,
            )
            .unwrap();
        let mut compositor = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    clip,
                    effect,
                    ..Default::default()
                },
                bounds: Rect::new(2., 3., 8., 7.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        let frame = compositor.build_frame(&Paint, 16, 16).unwrap();
        assert_eq!(frame.passes.len(), 1);
        let output = GpuiPresenter::default().present(&frame).unwrap();
        assert!(output.direct_tiles);
        // Not exactly 0.5: `amazing_compositor::property::nested_effect_opacity`
        // round-trips both the root's (255) and this effect's (0.5 -> 128) opacity
        // through `amazing_paint::quantize_opacity`'s u8 channel before combining
        // ((255*128+127)/255 = 128) and dividing back by 255.0, the standard 8-bit
        // premultiplied-alpha blend formula. That is amazing-compositor's own,
        // deliberate quantization (see property.rs), not a GPUI presenter bug, so
        // this asserts the quantized value rather than the pre-quantization input.
        assert!((output.tiles[0].quad.opacity - 128.0 / 255.0).abs() < 1e-6);
    }
}
