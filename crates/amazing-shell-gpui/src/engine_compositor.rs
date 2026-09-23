//! Retained paint-frame state for one live engine viewport.
//!
//! A paint view borrows layout, text, images, and controls. Each caller keeps
//! those inputs alive through `build_frame` and `present`; only the compositor,
//! presenter resources, and finished GPUI frame survive between updates.

use amazing_compositor::{Compositor, PaintViewRasterMode, PaintViewSource, Presenter};
use amazing_paint::display_list_view::DisplayListView;

use crate::compositor_presenter::{GpuiFrame, GpuiPresenter};

pub struct EngineCompositor {
    compositor: Compositor,
    presenter: GpuiPresenter,
    frame: GpuiFrame,
    mode: PaintViewRasterMode,
    width: u32,
    height: u32,
    work: FrameWork,
    #[cfg(test)]
    reference: amazing_compositor::ReferenceCpuPresenter,
    #[cfg(test)]
    reference_pixels: amazing_paint::Surface,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameWork {
    pub raster_tiles: usize,
    pub uploads: usize,
    pub evictions: usize,
    pub damage_regions: usize,
    pub damaged_pixels: u64,
}

impl FrameWork {
    fn from_frame(frame: &amazing_compositor::CompositorFrame, raster_tiles: usize) -> Self {
        Self {
            raster_tiles,
            uploads: frame.resources.uploads.len(),
            evictions: frame.resources.evictions.len(),
            damage_regions: frame.damage.len(),
            damaged_pixels: frame.damage.iter().map(|rect| (rect.w * rect.h).max(0.) as u64).sum(),
        }
    }
}

impl EngineCompositor {
    pub fn new(view: &DisplayListView<'_>, width: u32, height: u32) -> Result<Self, String> {
        let source = PaintViewSource::new(view);
        let mode = source.mode();
        let mut compositor = source.compositor(width, height).map_err(|e| e.to_string())?;
        let cached_before = compositor.cached_tiles();
        let frame = source
            .build_frame(&mut compositor, width, height)
            .map_err(|e| e.to_string())?;
        let work = FrameWork::from_frame(&frame, compositor.cached_tiles() - cached_before);
        let mut presenter = GpuiPresenter::default();
        #[cfg(test)]
        let mut reference = amazing_compositor::ReferenceCpuPresenter::default();
        #[cfg(test)]
        let reference_pixels = reference.present(&frame).map_err(|e| format!("{e:?}"))?;
        let frame = presenter.present(&frame).map_err(|e| format!("{e:?}"))?;
        Ok(Self {
            compositor,
            presenter,
            frame,
            mode,
            width,
            height,
            work,
            #[cfg(test)]
            reference,
            #[cfg(test)]
            reference_pixels,
        })
    }

    pub fn frame(&self) -> &GpuiFrame {
        &self.frame
    }

    pub fn mode(&self) -> PaintViewRasterMode {
        self.mode
    }

    pub fn work(&self) -> FrameWork {
        self.work
    }

    #[cfg(test)]
    pub fn reference_pixels(&self) -> &amazing_paint::Surface {
        &self.reference_pixels
    }

    pub fn compositor_mut(&mut self) -> &mut Compositor {
        &mut self.compositor
    }

    pub fn invalidate_item(&mut self, index: usize) {
        self.compositor.invalidate_item(index);
    }

    /// Rebuilds from the same scene after property or item changes. Returns
    /// whether GPUI needs a notification for pixel damage.
    pub fn update(&mut self, view: &DisplayListView<'_>) -> Result<bool, String> {
        let source = PaintViewSource::new(view);
        if source.mode() != self.mode {
            return Err("paint-view raster mode changed; rebuild the scene".into());
        }
        let raster_tiles = self.compositor.dirty_tiles();
        let frame = source
            .build_frame(&mut self.compositor, self.width, self.height)
            .map_err(|e| e.to_string())?;
        let damaged = !frame.damage.is_empty();
        self.work = FrameWork::from_frame(&frame, raster_tiles);
        #[cfg(test)]
        {
            self.reference_pixels = self.reference.present(&frame).map_err(|e| format!("{e:?}"))?;
        }
        self.frame = self
            .presenter
            .present(&frame)
            .map_err(|e| format!("{e:?}"))?;
        Ok(damaged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_compositor::{EffectId, Matrix4, ReferenceCpuPresenter, ScrollId, TransformId};
    use amazing_paint::{NoImages, controls::NoControls};
    use amazing_text_provider::TextProvider;

    static NO_IMAGES: NoImages = NoImages;
    static NO_CONTROLS: NoControls = NoControls;

    fn scene<'a>(
        root: &'a amazing_layout::LayoutBox,
        text: &'a TextProvider,
    ) -> DisplayListView<'a> {
        DisplayListView::new(root, 96, 64, text, &NO_IMAGES, &NO_CONTROLS)
    }

    #[test]
    fn initial_page_frame_matches_full_painter_and_noop_has_no_damage() {
        let text = TextProvider::new();
        let css = "body{margin:0;background:#fff}div{width:48px;height:32px;background:#b53726}";
        let root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>", css, 96., &text,
        );
        let view = scene(&root, &text);
        let source = PaintViewSource::new(&view);
        let mut reference_compositor = source.compositor(96, 64).unwrap();
        let reference_frame = source.build_frame(&mut reference_compositor, 96, 64).unwrap();
        let composed = ReferenceCpuPresenter::default().present(&reference_frame).unwrap();
        let full = amazing_paint::paint_document_with_text_resources_and_controls(
            &root, 96, 64, &text, &NoImages, &NoControls,
        );
        assert_eq!(composed.pixels, full.pixels);

        let mut live = EngineCompositor::new(&view, 96, 64).unwrap();
        assert_eq!(live.reference_pixels().pixels, full.pixels);
        assert_eq!(live.mode(), source.mode());
        assert!(live.frame().direct_tiles);
        eprintln!("ia-compositor initial mode={:?} direct={} work={:?} full_pixels={}",
            live.mode(), live.frame().direct_tiles, live.work(), 96 * 64);
        assert!(!live.frame().damage.is_empty());
        assert!(!live.update(&view).unwrap());
        assert!(live.frame().damage.is_empty());
        assert_eq!(live.work().raster_tiles, 0);
        assert_eq!(live.work().uploads, 0);
    }

    #[test]
    fn property_updates_keep_tile_resources_and_avoid_raster() {
        let text = TextProvider::new();
        let css = "body{margin:0}div{width:36px;height:24px;background:#b53726;transform:translateX(4px);opacity:.8}";
        let root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>", css, 96., &text,
        );
        let view = scene(&root, &text);
        assert!(view.transforms().len() > 1);
        assert!(view.effects().len() > 1);
        let mut live = EngineCompositor::new(&view, 96, 64).unwrap();
        let initial = live.work();
        let ids = live.frame().tiles.iter().map(|tile| tile.quad.resource_id).collect::<Vec<_>>();
        assert!(!ids.is_empty());
        let transform = TransformId(1);
        live.compositor_mut()
            .set_transform(transform, Matrix4::translation(11., 0., 0.))
            .unwrap();
        assert!(live.update(&view).unwrap());
        assert_eq!(live.work().raster_tiles, 0);
        assert_eq!(live.work().uploads, 0);
        assert_eq!(
            live.frame().tiles.iter().map(|tile| tile.quad.resource_id).collect::<Vec<_>>(),
            ids,
        );
        let translated = live.work();
        let translated_root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>",
            "body{margin:0}div{width:36px;height:24px;background:#b53726;transform:translateX(11px);opacity:.8}",
            96., &text,
        );
        let translated_full = amazing_paint::paint_document_with_text_resources_and_controls(
            &translated_root, 96, 64, &text, &NO_IMAGES, &NO_CONTROLS,
        );
        assert_eq!(live.reference_pixels().pixels, translated_full.pixels);
        live.compositor_mut().set_opacity(EffectId(1), 0.5).unwrap();
        assert!(live.update(&view).unwrap());
        assert_eq!(live.work().raster_tiles, 0);
        assert_eq!(live.work().uploads, 0);
        let faded = live.work();
        let faded_root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>",
            "body{margin:0}div{width:36px;height:24px;background:#b53726;transform:translateX(11px);opacity:.5}",
            96., &text,
        );
        let faded_full = amazing_paint::paint_document_with_text_resources_and_controls(
            &faded_root, 96, 64, &text, &NO_IMAGES, &NO_CONTROLS,
        );
        assert_eq!(live.reference_pixels().pixels, faded_full.pixels);
        live.invalidate_item(0);
        assert!(live.update(&view).unwrap());
        assert!(live.work().raster_tiles > 0);
        eprintln!("ia-compositor property mode={:?} initial={initial:?} translate={translated:?} fade={faded:?} invalidate={:?} full_pixels={}",
            live.mode(), live.work(), 96 * 64);
    }

    #[test]
    fn scene_rebuild_resets_presenter_and_complex_controls_use_fallback() {
        let text = TextProvider::new();
        let first = amazing_layout::layout_document_with_text(
            "<body><div></div></body>",
            "body{margin:0}div{width:20px;height:20px;background:red}",
            96., &text,
        );
        let first_view = scene(&first, &text);
        let first_scene = EngineCompositor::new(&first_view, 96, 64).unwrap();
        assert!(!first_scene.frame().damage.is_empty());

        let second = amazing_layout::layout_document_with_text(
            "<body><input type=checkbox></body>", "body{margin:0}", 96., &text,
        );
        let second_view = scene(&second, &text);
        let mut second_scene = EngineCompositor::new(&second_view, 96, 64).unwrap();
        assert_eq!(second_scene.mode(), PaintViewRasterMode::BakedFallback);
        assert!(!second_scene.frame().damage.is_empty());
        assert!(!second_scene.update(&second_view).unwrap());
    }

    #[test]
    fn scroll_update_reuses_uploaded_tiles() {
        let text = TextProvider::new();
        let css = "body{margin:0}section{overflow:auto;width:48px;height:24px}div{width:96px;height:72px;background:blue}";
        let root = amazing_layout::layout_document_with_text(
            "<body><section><div></div></section></body>", css, 96., &text,
        );
        let view = scene(&root, &text);
        assert!(view.scrolls().len() > 1);
        let mut live = EngineCompositor::new(&view, 96, 64).unwrap();
        let initial = live.work();
        let ids = live.frame().tiles.iter().map(|tile| tile.quad.resource_id).collect::<Vec<_>>();
        live.compositor_mut().set_scroll(ScrollId(1), (0., 8.)).unwrap();
        assert!(live.update(&view).unwrap());
        assert_eq!(live.work().raster_tiles, 0);
        assert_eq!(live.work().uploads, 0);
        assert_eq!(
            live.frame().tiles.iter().map(|tile| tile.quad.resource_id).collect::<Vec<_>>(),
            ids,
        );
        eprintln!("ia-compositor scroll mode={:?} initial={initial:?} scrolled={:?} full_pixels={}",
            live.mode(), live.work(), 96 * 64);
    }

    #[test]
    fn rotated_page_uses_composed_frame() {
        let text = TextProvider::new();
        let root = amazing_layout::layout_document_with_text(
            "<body><div></div></body>",
            "body{margin:0}div{width:30px;height:20px;background:red;transform:rotate(20deg)}",
            96., &text,
        );
        let view = scene(&root, &text);
        let live = EngineCompositor::new(&view, 96, 64).unwrap();
        assert!(!live.frame().direct_tiles);
        assert!(live.frame().composed.is_some());
    }
}
