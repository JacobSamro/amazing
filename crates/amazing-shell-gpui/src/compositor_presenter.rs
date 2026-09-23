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
    TileQuad,
};
use gpui::{AnyElement, IntoElement, ParentElement, RenderImage, Styled, div, img, px};
use image::{Frame, RgbaImage};

pub struct GpuiTile {
    pub quad: TileQuad,
    pub image: Arc<RenderImage>,
}

pub struct GpuiFrame {
    /// Exact fallback image for rotations, grouped effects and complex clips.
    pub composed: Option<Arc<RenderImage>>,
    /// Uploaded tile images in back-to-front draw order.
    pub tiles: Vec<GpuiTile>,
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
        if !self.direct_tiles {
            return img(self.composed.as_ref().expect("fallback image").clone())
                .w(px(self.width as f32))
                .h(px(self.height as f32))
                .into_any_element();
        }
        let mut root = div()
            .relative()
            .w(px(self.width as f32))
            .h(px(self.height as f32))
            .overflow_hidden();
        for tile in &self.tiles {
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
                    .left(px(x))
                    .top(px(y))
                    .w(px(right - x))
                    .h(px(bottom - y))
                    .overflow_hidden()
                    .child(
                        img(tile.image.clone())
                            .absolute()
                            .left(px(world.x - x))
                            .top(px(world.y - y))
                            .w(px(world.w))
                            .h(px(world.h))
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

fn axis_aligned_unit_transform(quad: &TileQuad) -> bool {
    let m = quad.transform.0;
    m[0][0] == 1.
        && m[0][1] == 0.
        && m[1][0] == 0.
        && m[1][1] == 1.
        && m[2] == [0., 0., 1., 0.]
        && m[3] == [0., 0., 0., 1.]
        && m[0][3].fract() == 0.
        && m[1][3].fract() == 0.
}

impl Presenter for GpuiPresenter {
    type Output = GpuiFrame;
    type Error = PresenterError;

    fn present(&mut self, frame: &CompositorFrame) -> Result<Self::Output, Self::Error> {
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
        let mut tiles = Vec::new();
        let mut direct_tiles = frame.passes.len() == 1;
        for quad in &frame.root().quads {
            if let DrawQuad::Tile(tile) = quad {
                let image = self
                    .tiles
                    .get(&tile.resource_id)
                    .ok_or(PresenterError::MissingResource(tile.resource_id))?
                    .clone();
                direct_tiles &= axis_aligned_unit_transform(tile)
                    && tile.blend == amazing_compositor::BlendMode::Normal;
                tiles.push(GpuiTile {
                    quad: tile.clone(),
                    image,
                });
            } else {
                direct_tiles = false;
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
        Ok(GpuiFrame {
            composed,
            tiles,
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
    use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, size};
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
    fn rotated_and_effect_pass_frames_use_exact_fallback() {
        let mut trees = PropertyTrees::new();
        let transform = trees
            .add_transform(
                TransformId(0),
                Matrix4::rotation_z(0.2),
                (0., 0., 0.),
                false,
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
        for state in [
            PropertyTreeState {
                transform,
                ..Default::default()
            },
            PropertyTreeState {
                effect,
                ..Default::default()
            },
        ] {
            let bounds = if state.effect.0 != 0 {
                Rect::new(2., 3., 270., 7.)
            } else {
                Rect::new(2., 3., 8., 7.)
            };
            let mut compositor = Compositor::new(
                trees.clone(),
                vec![PaintChunk {
                    items: 0..1,
                    state,
                    bounds,
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
        assert_eq!(output.tiles[0].quad.opacity, 0.5);
    }
}
