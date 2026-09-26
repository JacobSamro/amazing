//! Display-list rendering of a [`LayoutBox`] tree onto a [`Surface`].
//!
//! `display_list` owns layout-tree traversal and stacking phases. This module
//! consumes its flat commands, rasterizes box/content primitives, and scopes
//! clips and effect groups without reconstructing layout relationships.
//!
//! # Implemented
//!
//! - background-color fill over the content+padding rect (the padding
//!   box) whenever it parses as a non-transparent color
//! - borders as four filled rectangles from the layout-resolved widths;
//!   every style other than none/hidden paints solid (documented
//!   deviation below); `currentcolor` resolves against the element's
//!   computed `color`
//! - shaped text: layout-provided font/glyph ids, offsets, colors, inline
//!   backgrounds, and advances are rasterized through `amazing-text` without
//!   remeasurement; the legacy word/`FontSource` painter remains available
//! - everything clips to the viewport surface (`Surface::fill_rect`
//!   saturating-cast clipping; glyphs clip per pixel)
//! - non-`visible` overflow clips descendant backgrounds, borders, text, and
//!   images to the ancestor padding box, with nested clip intersection
//! - descendants paint in negative, in-flow, positioned-auto/zero, and
//!   positive phases; nested contexts remain atomic
//! - opacity and 2D transforms composite bounded transparent layers
//! - rounded backgrounds, border rings, overflow clips, background-clip,
//!   zero-blur shadows, and double-border thirds
//!
//! # Deferred / deviations
//!
//! - Border styles other than `double` render as solid.
//! - Blurred/multiple shadows, transform-origin grammar, 3D transforms,
//!   blend modes, filters, outlines, and list markers remain deferred.
//! - The legacy word painter does not retain inline color differences; the
//!   shaped provider path does.
//! - The canvas starts opaque white regardless of root background
//!   propagation rules.

use amazing_css::ComputedStyle;
use amazing_layout::font::FontSource;
use amazing_layout::LayoutBox;
use amazing_text::{GlyphCacheKey, GlyphContent, GlyphImage, TextSystem};
use std::ops::Range;

use crate::color::parse_css_color;
use crate::display_list::{build_display_list, GroupEffect, PaintCommand};
use crate::image::ImagePresentation;
use crate::surface::Surface;
use crate::text::{draw_fragment_clipped, GlyphProvider, RectInkProvider};

const CANVAS_WHITE: [u8; 4] = [255, 255, 255, 255];
const MAX_LAYER_PIXELS: usize = 16 * 1024 * 1024;
const MAX_RASTER_GROUP_DEPTH: usize = 16;
/// Used when the `color` property exists but does not parse: the CSS
/// initial value of `color` is black.
const FALLBACK_TEXT_COLOR: [u8; 4] = [0, 0, 0, 255];

#[derive(Debug, Clone, Copy, PartialEq)]
struct ClipRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl ClipRect {
    fn viewport(surface: &Surface) -> Self {
        Self {
            x0: 0.0,
            y0: 0.0,
            x1: surface.width as f32,
            y1: surface.height as f32,
        }
    }

    fn intersect(self, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x0: self.x0.max(x),
            y0: self.y0.max(y),
            x1: self.x1.min(x + width),
            y1: self.y1.min(y + height),
        }
    }

    fn pixels(self) -> [i64; 4] {
        [
            self.x0.round() as i64,
            self.y0.round() as i64,
            self.x1.round() as i64,
            self.y1.round() as i64,
        ]
    }
}

/// A resolved raster-space clip mask for a group layer: either the existing
/// per-corner rounded-rect test (CSS overflow clipping and `inset()`/
/// `circle()`/`ellipse()` clip-path), or a polygon winding test
/// (`polygon()`/a straight-edged `path()` clip-path). Both share
/// [`crate::clip::point_in_polygon`]'s tested containment check.
#[derive(Clone, Copy)]
enum RasterMask {
    Rounded(RoundedRect),
    Polygon { points: [(f32, f32); crate::clip::MAX_CLIP_POINTS], len: u8, even_odd: bool },
}

impl RasterMask {
    fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            RasterMask::Rounded(rounded) => rounded.contains(x, y),
            RasterMask::Polygon { points, len, even_odd } => {
                crate::clip::point_in_polygon(&points[..*len as usize], x, y, *even_odd)
            }
        }
    }
}

impl From<crate::clip::ClipMask> for RasterMask {
    fn from(clip: crate::clip::ClipMask) -> Self {
        match clip {
            crate::clip::ClipMask::Rect { x, y, width, height, radii } => RasterMask::Rounded(RoundedRect {
                x,
                y,
                width,
                height,
                radii: CornerRadii {
                    top_left: radii[0],
                    top_right: radii[1],
                    bottom_right: radii[2],
                    bottom_left: radii[3],
                },
            }),
            crate::clip::ClipMask::Polygon { points, len, even_odd } => {
                RasterMask::Polygon { points, len, even_odd }
            }
        }
    }
}

struct RasterGroup {
    parent: Surface,
    clips: Vec<ClipRect>,
    effect: GroupEffect,
    mask: Option<RasterMask>,
}

struct RasterState {
    surface: Surface,
    clips: Vec<ClipRect>,
    groups: Vec<RasterGroup>,
    rounded_clip_stack: Vec<bool>,
    layer_pixels: usize,
    background_samples: usize,
    failed: bool,
}

impl RasterState {
    fn new(width: u32, height: u32) -> Self {
        let mut surface = Surface::new(width, height);
        surface.clear(CANVAS_WHITE);
        let viewport = ClipRect::viewport(&surface);
        Self {
            surface,
            clips: vec![viewport],
            groups: Vec::new(),
            rounded_clip_stack: Vec::new(),
            layer_pixels: 0,
            background_samples: 64 * 1024 * 1024,
            failed: false,
        }
    }

    fn clip(&self) -> ClipRect {
        *self.clips.last().expect("raster clip stack is never empty")
    }

    fn begin_group(&mut self, effect: GroupEffect) {
        let mask = effect.clip.map(RasterMask::from);
        self.begin_group_with_mask(effect, mask);
    }

    fn begin_group_with_mask(&mut self, effect: GroupEffect, mask: Option<RasterMask>) {
        if self.groups.len() >= MAX_RASTER_GROUP_DEPTH {
            self.failed = true;
            return;
        }
        let pixels = self.surface.width as usize * self.surface.height as usize;
        let Some(total) = self.layer_pixels.checked_add(pixels) else {
            self.failed = true;
            return;
        };
        if total > MAX_LAYER_PIXELS {
            self.failed = true;
            return;
        }
        self.layer_pixels = total;
        let mut transparent = Surface::new(self.surface.width, self.surface.height);
        // `backdrop-filter` reads the content already painted behind this
        // box's border box, before this box's own decorations are drawn on
        // top. Seed the fresh group layer with that filtered snapshot so the
        // box's own (possibly semi-transparent) painting blends over it
        // exactly as it would blend over any other backdrop.
        if !effect.backdrop_filter.is_empty() {
            let filtered = effect.backdrop_filter.apply_to_surface(&self.surface);
            copy_rect(&filtered, &mut transparent, effect.backdrop_bounds);
        }
        let parent = std::mem::replace(&mut self.surface, transparent);
        let clips = std::mem::replace(&mut self.clips, vec![ClipRect::viewport(&self.surface)]);
        self.groups.push(RasterGroup {
            parent,
            clips,
            effect,
            mask,
        });
    }

    fn end_group(&mut self, images: &dyn ImageProvider) {
        let Some(group) = self.groups.pop() else {
            return;
        };
        let child = std::mem::replace(&mut self.surface, group.parent);
        self.layer_pixels = self
            .layer_pixels
            .saturating_sub(child.width as usize * child.height as usize);
        self.clips = group.clips;
        let clip = self.clip();
        composite_group(&mut self.surface, &child, group.effect, group.mask, clip, images);
    }

    fn push_overflow_clip(&mut self, box_: &LayoutBox) {
        let rounded = overflow_rounded_clip(box_);
        let has_rounded = rounded.is_some();
        if let Some(mask) = rounded {
            self.begin_group_with_mask(
                GroupEffect {
                    owner: box_.node,
                    opacity: 1.0,
                    transform: crate::display_list::Affine::IDENTITY,
                    origin_x: 0.0,
                    origin_y: 0.0,
                    blend: amazing_css::effects::BlendMode::Normal,
                    isolate: false,
                    filter: crate::filter::FilterPlan::default(),
                    backdrop_filter: crate::filter::FilterPlan::default(),
                    backdrop_bounds: amazing_layout::Rect::default(),
                    clip: None,
                    mask_image: None,
                    mask_bounds: amazing_layout::Rect::default(),
                },
                Some(RasterMask::Rounded(mask)),
            );
        }
        self.clips.push(overflow_clip(box_, self.clip()));
        self.rounded_clip_stack.push(has_rounded);
    }

    fn pop_clip(&mut self, images: &dyn ImageProvider) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
        if self.rounded_clip_stack.pop().unwrap_or(false) {
            self.end_group(images);
        }
    }

    fn finish(self) -> Surface {
        if self.failed || !self.groups.is_empty() {
            let mut surface = Surface::new(self.surface.width, self.surface.height);
            surface.clear(CANVAS_WHITE);
            surface
        } else {
            self.surface
        }
    }
}

fn copy_rect(source: &Surface, destination: &mut Surface, bounds: amazing_layout::Rect) {
    let round = |v: f32| -> i64 {
        if v.is_nan() { 0 } else { v.round() as i64 }
    };
    let x0 = round(bounds.x).max(0);
    let y0 = round(bounds.y).max(0);
    let x1 = round(bounds.x + bounds.w).max(x0);
    let y1 = round(bounds.y + bounds.h).max(y0);
    let x0 = x0.min(destination.width as i64) as u32;
    let y0 = y0.min(destination.height as i64) as u32;
    let x1 = x1.min(destination.width as i64) as u32;
    let y1 = y1.min(destination.height as i64) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(pixel) = source.get_pixel(x, y) {
                destination.set_pixel(x, y, pixel);
            }
        }
    }
}

fn composite_group(
    destination: &mut Surface,
    source: &Surface,
    effect: GroupEffect,
    mask: Option<RasterMask>,
    clip: ClipRect,
    images: &dyn ImageProvider,
) {
    let matrix = effect.matrix_about_origin();
    let Some(inverse) = matrix.inverse() else {
        return;
    };
    // `blur`/`drop-shadow` read neighboring pixels, so they must run once
    // over the whole layer rather than per destination sample; color-only
    // plans take this same path (a no-op-cost clone plus a pointwise map),
    // keeping one filter application rule for the group.
    let filtered = if effect.filter.is_empty() {
        None
    } else {
        Some(effect.filter.apply_to_surface(source))
    };
    let source = filtered.as_ref().unwrap_or(source);
    let mask_image = effect
        .mask_image
        .and_then(|layer| images.image_by_url(layer.url()).map(|image| (layer, image)));
    let [clip_x0, clip_y0, clip_x1, clip_y1] = clip.pixels();
    let x0 = clip_x0.max(0).min(destination.width as i64);
    let y0 = clip_y0.max(0).min(destination.height as i64);
    let x1 = clip_x1.max(0).min(destination.width as i64);
    let y1 = clip_y1.max(0).min(destination.height as i64);
    for y in y0..y1 {
        for x in x0..x1 {
            let (source_x, source_y) = inverse.map(x as f32 + 0.5, y as f32 + 0.5);
            let sx = source_x.floor() as i64;
            let sy = source_y.floor() as i64;
            if sx < 0 || sy < 0 || sx >= source.width as i64 || sy >= source.height as i64 {
                continue;
            }
            if mask.is_some_and(|mask| !mask.contains(source_x, source_y)) {
                continue;
            }
            let Some(source_pixel) = source.get_pixel(sx as u32, sy as u32) else {
                continue;
            };
            if source_pixel[3] == 0 {
                continue;
            }
            // `mask-image` is stretched to exactly fill `mask_bounds` (the
            // masked box's own border box); outside that extent, or when the
            // reference does not resolve, `MaskLayer::sample` returns 0.0/
            // treats the element as unmasked respectively (a resource that
            // fails to load is `none` per CSS Masking, not a hard failure).
            let mask_value = match mask_image {
                Some((layer, image)) if effect.mask_bounds.w > 0.0 && effect.mask_bounds.h > 0.0 => {
                    let u = (source_x - effect.mask_bounds.x) / effect.mask_bounds.w;
                    let v = (source_y - effect.mask_bounds.y) / effect.mask_bounds.h;
                    layer.sample(image, u, v)
                }
                _ => 1.0,
            };
            if mask_value <= 0.0 {
                continue;
            }
            let destination_pixel = destination
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            destination.set_pixel(
                x as u32,
                y as u32,
                crate::blend::blend_group(
                    effect.blend,
                    source_pixel,
                    destination_pixel,
                    crate::quantize_opacity(effect.opacity * mask_value),
                ),
            );
        }
    }
}

fn source_over(source: [u8; 4], destination: [u8; 4], opacity: f32) -> [u8; 4] {
    let opacity = crate::quantize_opacity(opacity);
    crate::blend_source_over(source, destination, opacity)
}

/// Seam through which the painter obtains decoded pixels for replaced nodes.
pub trait ImageProvider {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image>;
    /// Resolved URL lookup for generated `content: url(...)` fragments, which
    /// have geometry but no distinct DOM image node.
    fn image_by_url(&self, _url: &str) -> Option<&amazing_media::Image> {
        None
    }
    /// Rasterize an inline SVG root for this content viewport. The embedding
    /// adapter owns DOM/style access; paint only receives bounded RGBA pixels.
    fn svg(
        &self,
        _node: amazing_dom::NodeIdx,
        _width: u32,
        _height: u32,
        _dpr: f32,
    ) -> Option<amazing_media::Image> {
        None
    }
    fn device_scale(&self) -> f32 {
        1.0
    }
    /// Decoded background resource resolved by the embedding loader. No I/O during paint.
    fn background_image(
        &self,
        _node: amazing_dom::NodeIdx,
        _url: &str,
    ) -> Option<&amazing_media::Image> {
        None
    }
    /// Inline fragments retain node identity; callers supply its computed style.
    fn presentation(&self, _node: amazing_dom::NodeIdx) -> ImagePresentation {
        ImagePresentation::default()
    }
}

/// Adapter used by callers that have no decoded images.
pub struct NoImages;

impl ImageProvider for NoImages {
    fn image(&self, _node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        None
    }
}

/// Paints `root` into a fresh `viewport_w × viewport_h` surface using the
/// default rect-ink glyph provider.
pub fn paint_document(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    fonts: &FontSource,
) -> Surface {
    paint_document_with_resources(
        root,
        viewport_w,
        viewport_h,
        fonts,
        &RectInkProvider,
        &NoImages,
    )
}

/// Like [`paint_document`] but with a caller-chosen [`GlyphProvider`]
/// (e.g. a future real-outline rasterizer).
pub fn paint_document_with(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
) -> Surface {
    paint_document_with_resources(root, viewport_w, viewport_h, fonts, glyphs, &NoImages)
}

/// Paints text and decoded replaced content supplied by caller-owned adapters.
pub fn paint_document_with_resources(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
    images: &dyn ImageProvider,
) -> Surface {
    let commands = build_display_list(root);
    let mut state = RasterState::new(viewport_w, viewport_h);
    for command in commands {
        if state.failed {
            break;
        }
        match command {
            PaintCommand::BeginGroup(effect) => state.begin_group(effect),
            PaintCommand::PaintDecorations(box_, background) => {
                let color = resolve_text_color(&box_.style);
                let clip = state.clip();
                if !paint_decorations(
                    &mut state.surface,
                    box_,
                    color,
                    images,
                    &background,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
            }
            PaintCommand::PaintControl(control) => {
                let clip = state.clip();
                let content = crate::controls::ControlContent::default();
                if !control_raster::paint(
                    &mut state.surface,
                    &control,
                    &content,
                    images,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
            }
            PaintCommand::PaintContent(box_) => {
                let color = resolve_text_color(&box_.style);
                let clip = state.clip();
                paint_lines(&mut state.surface, box_, color, fonts, glyphs, images, clip);
                paint_table_borders(&mut state.surface, box_, clip);
            }
            PaintCommand::PushOverflowClip(box_) => state.push_overflow_clip(box_),
            PaintCommand::PopClip => state.pop_clip(images),
            PaintCommand::EndGroup => state.end_group(images),
        }
    }
    state.finish()
}

/// Provider-neutral paint entry point. Layout-established glyph ids and
/// advances are rasterized directly; this path never measures text.
pub fn paint_document_with_text_resources(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
) -> Surface {
    paint_document_with_text_resources_and_controls(
        root,
        viewport_w,
        viewport_h,
        text,
        images,
        &crate::controls::NoControls,
    )
}

/// Additive state/glyph seam. Labels must already be shaped and positioned by layout.
pub fn paint_document_with_text_resources_and_controls(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    controls: &dyn crate::controls::ControlProvider,
) -> Surface {
    let commands = build_display_list(root);
    let mut state = RasterState::new(viewport_w, viewport_h);
    for command in commands {
        if state.failed {
            break;
        }
        match command {
            PaintCommand::BeginGroup(effect) => state.begin_group(effect),
            PaintCommand::PaintDecorations(box_, background) => {
                let color = resolve_text_color(&box_.style);
                let clip = state.clip();
                if !paint_decorations(
                    &mut state.surface,
                    box_,
                    color,
                    images,
                    &background,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
            }
            PaintCommand::PaintControl(control) => {
                let clip = state.clip();
                let content = controls.content(control.geometry.node.unwrap());
                if !control_raster::paint(
                    &mut state.surface,
                    &control,
                    &content,
                    images,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
                let rect = control.geometry.content_box;
                let text_clip = clip.intersect(rect.x, rect.y, rect.w, rect.h);
                for line in content.lines {
                    paint_line_glyphs(&mut state.surface, line, text, text_clip);
                }
            }
            PaintCommand::PaintContent(box_) => {
                let clip = state.clip();
                paint_shaped_lines(&mut state.surface, box_, text, images, clip);
                paint_table_borders(&mut state.surface, box_, clip);
            }
            PaintCommand::PushOverflowClip(box_) => state.push_overflow_clip(box_),
            PaintCommand::PopClip => state.pop_clip(images),
            PaintCommand::EndGroup => state.end_group(images),
        }
    }
    state.finish()
}

/// Replays the command stream so clips and effect groups around the selected
/// items retain their original semantics, then translates the selected ink
/// into a caller-owned target. This is the additive compositor raster seam.
pub(crate) fn raster_command_range(
    commands: &[PaintCommand<'_>],
    selected: Range<usize>,
    viewport: (u32, u32),
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    controls: &dyn crate::controls::ControlProvider,
    target: &mut Surface,
    offset: (i32, i32),
    target_clip: crate::display_list_view::PaintBounds,
) -> bool {
    let Some(pixels) = (viewport.0 as usize).checked_mul(viewport.1 as usize) else {
        return false;
    };
    if pixels > MAX_LAYER_PIXELS || selected.start > selected.end || selected.end > commands.len() {
        return false;
    }
    let mut state = RasterState::new(viewport.0, viewport.1);
    state.surface.clear([0, 0, 0, 0]);
    for (index, command) in commands.iter().enumerate() {
        if state.failed {
            break;
        }
        let draw = selected.contains(&index);
        match command {
            PaintCommand::BeginGroup(effect) => state.begin_group(*effect),
            PaintCommand::EndGroup => state.end_group(images),
            PaintCommand::PushOverflowClip(box_) => state.push_overflow_clip(box_),
            PaintCommand::PopClip => state.pop_clip(images),
            PaintCommand::PaintDecorations(box_, background) if draw => {
                let color = resolve_text_color(&box_.style);
                let clip = state.clip();
                if !paint_decorations(
                    &mut state.surface,
                    box_,
                    color,
                    images,
                    background,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
            }
            PaintCommand::PaintControl(control) if draw => {
                let clip = state.clip();
                let content = controls.content(control.geometry.node.unwrap());
                if !control_raster::paint(
                    &mut state.surface,
                    control,
                    &content,
                    images,
                    clip,
                    &mut state.background_samples,
                ) {
                    state.failed = true;
                }
                let rect = control.geometry.content_box;
                let text_clip = clip.intersect(rect.x, rect.y, rect.w, rect.h);
                for line in content.lines {
                    paint_line_glyphs(&mut state.surface, line, text, text_clip);
                }
            }
            PaintCommand::PaintContent(box_) if draw => {
                let clip = state.clip();
                paint_shaped_lines(&mut state.surface, box_, text, images, clip);
                paint_table_borders(&mut state.surface, box_, clip);
            }
            _ => {}
        }
    }
    if state.failed || !state.groups.is_empty() {
        return false;
    }
    if !target_clip.x.is_finite()
        || !target_clip.y.is_finite()
        || !target_clip.width.is_finite()
        || !target_clip.height.is_finite()
        || target_clip.width <= 0.0
        || target_clip.height <= 0.0
    {
        return false;
    }
    let clip = ClipRect::viewport(target).intersect(
        target_clip.x,
        target_clip.y,
        target_clip.width,
        target_clip.height,
    );
    let [x0, y0, x1, y1] = clip.pixels();
    let (dx, dy) = (i64::from(offset.0), i64::from(offset.1));
    let sx0 = (x0 - dx).max(0).min(i64::from(viewport.0));
    let sy0 = (y0 - dy).max(0).min(i64::from(viewport.1));
    let sx1 = (x1 - dx).max(0).min(i64::from(viewport.0));
    let sy1 = (y1 - dy).max(0).min(i64::from(viewport.1));
    for sy in sy0..sy1 {
        for sx in sx0..sx1 {
            let source = state.surface.get_pixel(sx as u32, sy as u32).unwrap();
            if source[3] == 0 {
                continue;
            }
            let tx = (sx + dx) as u32;
            let ty = (sy + dy) as u32;
            let destination = target.get_pixel(tx, ty).unwrap();
            target.set_pixel(tx, ty, source_over(source, destination, 1.0));
        }
    }
    true
}

/// Paint one independent command directly into a tile in untransformed
/// document coordinates. Group state is deliberately absent: the compositor
/// owns its transform, opacity and clip property trees.
pub(crate) fn raster_raw_item(
    command: &PaintCommand<'_>,
    target: &mut Surface,
    tile_origin: (i32, i32),
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
) -> bool {
    match command {
        PaintCommand::PaintDecorations(box_, background) => {
            if box_.table_decoration().suppress_background
                || box_.table_decoration().suppress_border
                || !box_.table_decoration().background_holes.is_empty() { return false; }
            let mut local = LayoutBox::new(
                box_.node,
                box_.display,
                box_.style.clone(),
                box_.font_size,
                box_.line_height,
            );
            let shift = |mut rect: amazing_layout::Rect| {
                rect.x -= tile_origin.0 as f32;
                rect.y -= tile_origin.1 as f32;
                rect
            };
            local.border_box = shift(box_.border_box);
            local.content_box = shift(box_.content_box);
            local.padding = box_.padding;
            local.border = box_.border;
            local.replaced_size = box_.replaced_size;
            local.svg_root = box_.svg_root;
            let color = resolve_text_color(&local.style);
            let clip = ClipRect::viewport(target);
            let mut budget = 0;
            paint_decorations(
                target,
                &local,
                color,
                images,
                background,
                clip,
                &mut budget,
            )
        }
        PaintCommand::PaintContent(box_) => {
            if !box_.table_decoration().borders.is_empty() { return false; }
            let mut local = LayoutBox::new(
                box_.node,
                box_.display,
                box_.style.clone(),
                box_.font_size,
                box_.line_height,
            );
            let (ox, oy) = (tile_origin.0 as f32, tile_origin.1 as f32);
            local.lines = box_.lines.iter().map(|line| {
                let mut glyphs = line.glyphs.clone();
                for glyph in &mut glyphs {
                    glyph.x -= ox;
                    if glyph.writing_mode.is_vertical_flow() { glyph.y -= oy; }
                }
                let mut decorations = line.decorations.clone();
                for decoration in &mut decorations {
                    decoration.x -= ox;
                    decoration.y -= oy;
                }
                amazing_layout::LineBox {
                    y_top: line.y_top - oy,
                    height: line.height,
                    baseline: line.baseline - oy,
                    words: Vec::new(),
                    glyphs,
                    decorations,
                    replaced: Vec::new(),
                }
            }).collect();
            let clip = ClipRect::viewport(target);
            // Inline replaced fragments resolve against the original box, whose
            // inline flow still carries the styled fragment metadata (generated
            // URLs, SVG roots). They paint first at the tile offset, then the
            // shifted glyphs follow. `paint_shaped_lines` cannot be reused here:
            // it would look the replaced metadata up on `local`, which has no
            // inline flow of its own.
            for (line, local_line) in box_.lines.iter().zip(local.lines.iter()) {
                paint_replaced_fragments(target, box_, line, images, clip, (ox, oy));
                paint_line_glyphs(target, local_line, text, clip);
            }
            true
        }
        _ => false,
    }
}

fn paint_decorations(
    surface: &mut Surface,
    box_: &LayoutBox,
    text_color: [u8; 4],
    images: &dyn ImageProvider,
    background: &crate::background::BackgroundPlan,
    clip: ClipRect,
    budget: &mut usize,
) -> bool {
    paint_box_shadow(surface, box_, text_color, clip);
    if !box_.table_decoration().suppress_background {
        paint_background(surface, box_, text_color, background.color_clip, clip);
        if !background_raster::paint(surface, box_, background, images, clip, budget) {
            return false;
        }
    }
    paint_inset_box_shadow(surface, box_, text_color, clip);
    paint_borders(surface, box_, text_color, clip);
    paint_outline(surface, box_, text_color, clip);
    if let Some(node) = box_.svg_root {
        if let Some(svg) = svg_for_rect(images, node, &box_.content_box) {
            paint_image(
                surface,
                &box_.content_box,
                &svg,
                images.presentation(node),
                clip,
            );
        }
    } else if box_.replaced_size.is_some() {
        if let Some(image) = box_.node.and_then(|node| images.image(node)) {
            paint_image(
                surface,
                &box_.content_box,
                image,
                ImagePresentation::from_style(&box_.style),
                clip,
            );
        }
    }
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct CornerRadii {
    top_left: (f32, f32),
    top_right: (f32, f32),
    bottom_right: (f32, f32),
    bottom_left: (f32, f32),
}

#[derive(Debug, Clone, Copy)]
struct RoundedRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radii: CornerRadii,
}

impl RoundedRect {
    fn contains(self, x: f32, y: f32) -> bool {
        if x < self.x || y < self.y || x >= self.x + self.width || y >= self.y + self.height {
            return false;
        }
        let corners = [
            (
                self.x,
                self.y,
                self.radii.top_left,
                x < self.x + self.radii.top_left.0 && y < self.y + self.radii.top_left.1,
            ),
            (
                self.x + self.width,
                self.y,
                self.radii.top_right,
                x >= self.x + self.width - self.radii.top_right.0
                    && y < self.y + self.radii.top_right.1,
            ),
            (
                self.x + self.width,
                self.y + self.height,
                self.radii.bottom_right,
                x >= self.x + self.width - self.radii.bottom_right.0
                    && y >= self.y + self.height - self.radii.bottom_right.1,
            ),
            (
                self.x,
                self.y + self.height,
                self.radii.bottom_left,
                x < self.x + self.radii.bottom_left.0
                    && y >= self.y + self.height - self.radii.bottom_left.1,
            ),
        ];
        for (corner_x, corner_y, (radius_x, radius_y), applies) in corners {
            if !applies || radius_x <= 0.0 || radius_y <= 0.0 {
                continue;
            }
            let center_x = if corner_x == self.x {
                corner_x + radius_x
            } else {
                corner_x - radius_x
            };
            let center_y = if corner_y == self.y {
                corner_y + radius_y
            } else {
                corner_y - radius_y
            };
            let dx = (x - center_x) / radius_x;
            let dy = (y - center_y) / radius_y;
            return dx * dx + dy * dy <= 1.0;
        }
        true
    }
}

fn fill_rounded_rect_clipped(
    surface: &mut Surface,
    rounded: RoundedRect,
    color: [u8; 4],
    clip: ClipRect,
    holes: &[amazing_layout::Rect],
) {
    let clipped = clip.intersect(rounded.x, rounded.y, rounded.width, rounded.height);
    let [x0, y0, x1, y1] = clipped.pixels();
    for y in y0.max(0)..y1.min(surface.height as i64) {
        for x in x0.max(0)..x1.min(surface.width as i64) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            if !rounded.contains(px, py) || holes.iter().any(|hole| {
                px >= hole.x && px < hole.x + hole.w && py >= hole.y && py < hole.y + hole.h
            }) {
                continue;
            }
            let destination = surface
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            surface.set_pixel(x as u32, y as u32, source_over(color, destination, 1.0));
        }
    }
}

fn resolve_corner_radii(box_: &LayoutBox, rect: &amazing_layout::Rect) -> CornerRadii {
    // Layout owns CSS math and axis-specific percentage resolution. The box
    // carries its used em; it does not yet retain the selected font's ch width.
    let em = box_.font_size;
    let ch = em * 0.5;
    let [top_left, top_right, bottom_right, bottom_left] =
        amazing_layout::values::resolve_corner_radii(&box_.style, em, ch, rect.w, rect.h);
    normalize_radii(
        CornerRadii {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        },
        rect.w,
        rect.h,
    )
}

fn normalize_radii(mut radii: CornerRadii, width: f32, height: f32) -> CornerRadii {
    let ratios = [
        width / (radii.top_left.0 + radii.top_right.0).max(width),
        width / (radii.bottom_left.0 + radii.bottom_right.0).max(width),
        height / (radii.top_left.1 + radii.bottom_left.1).max(height),
        height / (radii.top_right.1 + radii.bottom_right.1).max(height),
    ];
    let scale = ratios.into_iter().fold(1.0f32, f32::min).clamp(0.0, 1.0);
    for radius in [
        &mut radii.top_left,
        &mut radii.top_right,
        &mut radii.bottom_right,
        &mut radii.bottom_left,
    ] {
        radius.0 *= scale;
        radius.1 *= scale;
    }
    radii
}

#[derive(Debug, Clone, Copy)]
struct BoxShadow {
    inset: bool,
    offset_x: f32,
    offset_y: f32,
    blur: f32,
    spread: f32,
    color: [u8; 4],
}

fn paint_box_shadow(surface: &mut Surface, box_: &LayoutBox, text_color: [u8; 4], clip: ClipRect) {
    let Some(value) = box_.style.get("box-shadow") else {
        return;
    };
    let Some(shadow) = parse_box_shadow(value, text_color) else {
        return;
    };
    if shadow.inset || shadow.blur > 0.0 {
        return;
    }
    let border = box_.border_box;
    let spread = shadow.spread;
    let mut radii = resolve_corner_radii(box_, &border);
    for radius in [
        &mut radii.top_left,
        &mut radii.top_right,
        &mut radii.bottom_right,
        &mut radii.bottom_left,
    ] {
        radius.0 = (radius.0 + spread).max(0.0);
        radius.1 = (radius.1 + spread).max(0.0);
    }
    fill_rounded_rect_clipped(
        surface,
        RoundedRect {
            x: border.x + shadow.offset_x - spread,
            y: border.y + shadow.offset_y - spread,
            width: border.w + spread * 2.0,
            height: border.h + spread * 2.0,
            radii,
        },
        shadow.color,
        clip,
        &[],
    );
}

fn paint_inset_box_shadow(
    surface: &mut Surface,
    box_: &LayoutBox,
    text_color: [u8; 4],
    clip: ClipRect,
) {
    let Some(value) = box_.style.get("box-shadow") else {
        return;
    };
    let Some(shadow) = parse_box_shadow(value, text_color) else {
        return;
    };
    if !shadow.inset || shadow.blur > 0.0 {
        return;
    }
    let content = box_.content_box;
    let padding = amazing_layout::Rect {
        x: content.x - box_.padding.left,
        y: content.y - box_.padding.top,
        w: content.w + box_.padding.left + box_.padding.right,
        h: content.h + box_.padding.top + box_.padding.bottom,
    };
    let all_sides = shadow.spread.max(0.0);
    let left = (shadow.offset_x + shadow.spread).max(0.0);
    let right = (-shadow.offset_x + shadow.spread).max(0.0);
    let top = (shadow.offset_y + shadow.spread).max(0.0);
    let bottom = (-shadow.offset_y + shadow.spread).max(0.0);
    for (x, y, width, height) in [
        (padding.x, padding.y, left, padding.h),
        (padding.x + padding.w - right, padding.y, right, padding.h),
        (padding.x, padding.y, padding.w, top),
        (padding.x, padding.y + padding.h - bottom, padding.w, bottom),
        (padding.x, padding.y, padding.w, all_sides),
    ] {
        fill_rect_clipped(surface, x, y, width, height, shadow.color, clip);
    }
}

fn parse_box_shadow(value: &str, current_color: [u8; 4]) -> Option<BoxShadow> {
    if value.trim().eq_ignore_ascii_case("none") {
        return None;
    }
    let components = top_level_space_components(value)?;
    let mut inset = false;
    let mut color = None;
    let mut lengths = Vec::new();
    for component in components {
        if component.eq_ignore_ascii_case("inset") {
            if inset {
                return None;
            }
            inset = true;
        } else if component.eq_ignore_ascii_case("currentcolor") {
            if color.is_some() {
                return None;
            }
            color = Some(current_color);
        } else if let Some(parsed) = parse_css_color(component) {
            if color.is_some() {
                return None;
            }
            color = Some(parsed);
        } else {
            lengths.push(resolve_shadow_length(component)?);
        }
    }
    if !(2..=4).contains(&lengths.len()) {
        return None;
    }
    Some(BoxShadow {
        inset,
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur: lengths.get(2).copied().unwrap_or(0.0).max(0.0),
        spread: lengths.get(3).copied().unwrap_or(0.0),
        color: color.unwrap_or(current_color),
    })
}

fn top_level_space_components(value: &str) -> Option<Vec<&str>> {
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            ',' if depth == 0 => return None,
            character if character.is_whitespace() && depth == 0 => {
                if start < index {
                    output.push(value[start..index].trim());
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    if start < value.len() {
        output.push(value[start..].trim());
    }
    let output = output
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!output.is_empty()).then_some(output)
}

fn resolve_shadow_length(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(px) = value.strip_suffix("px") {
        px.parse::<f32>().ok().filter(|value| value.is_finite())
    } else if value == "0" || value == "0.0" {
        Some(0.0)
    } else {
        None
    }
}

fn paint_shaped_lines(
    surface: &mut Surface,
    box_: &LayoutBox,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    clip: ClipRect,
) {
    for line in &box_.lines {
        paint_replaced_fragments(surface, box_, line, images, clip, (0.0, 0.0));
        paint_line_glyphs(surface, line, text, clip);
    }
}

/// Paints one line's inline replaced fragments using `box_`'s styled fragment
/// metadata. `offset` shifts the fragment rectangles into the caller's
/// coordinate space, which lets the raw tile path reuse the same
/// generated/URL/SVG resolution as the full painter without rebuilding the
/// box's inline flow (where `styled_replaced_fragments` reads its metadata).
fn paint_replaced_fragments(
    surface: &mut Surface,
    box_: &LayoutBox,
    line: &amazing_layout::LineBox,
    images: &dyn ImageProvider,
    clip: ClipRect,
    offset: (f32, f32),
) {
    for replaced in &line.replaced {
        let rect = amazing_layout::Rect {
            x: replaced.x - offset.0,
            y: replaced.y - offset.1,
            w: replaced.width,
            h: replaced.height,
        };
        if let Some((url, presentation, font_size)) = generated_image_url(box_, replaced) {
            paint_generated_image(surface, &rect, &url, presentation, font_size, images, clip);
            continue;
        }
        if let Some(svg) = svg_for_rect(images, replaced.node, &rect) {
            paint_image(
                surface,
                &rect,
                &svg,
                images.presentation(replaced.node),
                clip,
            );
        } else if let Some(image) = resolved_replaced_image(images, replaced.node, None) {
            paint_image(
                surface,
                &rect,
                image,
                images.presentation(replaced.node),
                clip,
            );
        }
    }
}

fn generated_image_url(
    box_: &LayoutBox,
    replaced: &amazing_layout::ReplacedFrag,
) -> Option<(String, ImagePresentation, f32)> {
    box_.styled_replaced_fragments().find_map(|fragment| {
        (fragment.node == replaced.node
            && fragment.source_order == replaced.source_order)
            .then(|| {
                fragment.generated_url.map(|url| {
                    (url, ImagePresentation::from_style(fragment.style), fragment.font_size)
                })
            })
            .flatten()
    })
}

fn resolved_replaced_image<'a>(
    images: &'a dyn ImageProvider,
    node: amazing_dom::NodeIdx,
    generated_url: Option<&str>,
) -> Option<&'a amazing_media::Image> {
    match generated_url {
        Some(url) => images.image_by_url(url),
        None => images.image(node),
    }
}

fn paint_generated_image(
    surface: &mut Surface,
    rect: &amazing_layout::Rect,
    key: &str,
    presentation: ImagePresentation,
    font_size: f32,
    images: &dyn ImageProvider,
    clip: ClipRect,
) {
    if let Some(image) = images.image_by_url(key) {
        paint_image(surface, rect, image, presentation, clip);
        return;
    }
    if !rect.w.is_finite() || !rect.h.is_finite() || rect.w <= 0.0 || rect.h <= 0.0
        || rect.w > u32::MAX as f32 || rect.h > u32::MAX as f32 { return; }
    if let Some(image) = crate::gradient::raster(key, rect.w.ceil() as u32, rect.h.ceil() as u32, font_size) {
        paint_image(surface, rect, &image, presentation, clip);
    }
}

fn paint_line_glyphs(
    surface: &mut Surface,
    line: &amazing_layout::LineBox,
    text: &dyn TextSystem,
    clip: ClipRect,
) {
    for decoration in &line.decorations {
        if let Some(color) = parse_css_color(&decoration.color) {
            if color[3] != 0 {
                fill_rect_clipped(
                    surface,
                    decoration.x,
                    decoration.y,
                    decoration.width,
                    decoration.height,
                    color,
                    clip,
                );
            }
        }
    }
    for glyph in &line.glyphs {
        let color = parse_css_color(&glyph.color).unwrap_or(FALLBACK_TEXT_COLOR);
        let baseline = if glyph.writing_mode.is_vertical_flow() {
            glyph.y
        } else {
            line.baseline + glyph.y - glyph.offset_y
        };
        let subpixel = |value: f32| ((value.fract().rem_euclid(1.0) * 64.0).round() as u8).min(63);
        let Some(key) = GlyphCacheKey::new(
            text.generation(),
            glyph.font,
            glyph.glyph,
            glyph.font_size,
            subpixel(glyph.x),
            subpixel(baseline),
        ) else {
            continue;
        };
        let key = key.with_style(&glyph.variations, glyph.synthesis);
        let Ok(Some(image)) = text.rasterize(amazing_text::GlyphRasterRequest {
            key,
            variations: &glyph.variations,
            synthesis: glyph.synthesis,
        }) else {
            continue;
        };
        blit_text_image(
            surface,
            &image,
            GlyphBlit {
                origin_x: glyph.x.floor() as i64,
                origin_y: baseline.floor() as i64,
                orientation: glyph.orientation,
                scale_x: glyph.scale_x,
                scale_y: glyph.scale_y,
                color,
            },
            clip,
        );
    }
}

struct GlyphBlit {
    origin_x: i64,
    origin_y: i64,
    orientation: amazing_text::GlyphOrientation,
    scale_x: f32,
    scale_y: f32,
    color: [u8; 4],
}

fn blit_text_image(surface: &mut Surface, image: &GlyphImage, blit: GlyphBlit, clip: ClipRect) {
    let GlyphBlit {
        origin_x,
        origin_y,
        orientation,
        scale_x,
        scale_y,
        color,
    } = blit;
    if !scale_x.is_finite() || !scale_y.is_finite() || scale_x <= 0.0 || scale_y <= 0.0 {
        return;
    }
    let channels = if image.content == GlyphContent::Color {
        4
    } else {
        1
    };
    let Some(expected) = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .and_then(|bytes| usize::try_from(bytes).ok())
    else {
        return;
    };
    if image.data.len() != expected {
        return;
    }
    let [clip_x0, clip_y0, clip_x1, clip_y1] = clip.pixels();
    for row in 0..image.height as i64 {
        for column in 0..image.width as i64 {
            let (x, y) = match orientation {
                amazing_text::GlyphOrientation::Upright => (
                    origin_x + ((image.bearing_x as i64 + column) as f32 * scale_x).floor() as i64,
                    origin_y + ((-image.bearing_y as i64 + row) as f32 * scale_y).floor() as i64,
                ),
                amazing_text::GlyphOrientation::SidewaysRight => (
                    origin_x + ((image.bearing_y as i64 - 1 - row) as f32 * scale_y).floor() as i64,
                    origin_y + ((image.bearing_x as i64 + column) as f32 * scale_x).floor() as i64,
                ),
                amazing_text::GlyphOrientation::SidewaysLeft => (
                    origin_x + ((-image.bearing_y as i64 + row) as f32 * scale_y).floor() as i64,
                    origin_y
                        + ((-image.bearing_x as i64 - 1 - column) as f32 * scale_x).floor() as i64,
                ),
            };
            if x < clip_x0
                || x >= clip_x1
                || y < clip_y0
                || y >= clip_y1
                || x < 0
                || x >= surface.width as i64
                || y < 0
                || y >= surface.height as i64
            {
                continue;
            }
            let pixel = (row as usize * image.width as usize) + column as usize;
            let (source, coverage) = if image.content == GlyphContent::Color {
                let start = pixel * 4;
                let source = [
                    image.data[start],
                    image.data[start + 1],
                    image.data[start + 2],
                    image.data[start + 3],
                ];
                (source, source[3])
            } else {
                (color, image.data[pixel])
            };
            if coverage == 0 {
                continue;
            }
            let destination = surface
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            let effective_alpha = if image.content == GlyphContent::Color {
                coverage
            } else {
                ((source[3] as u16 * coverage as u16 + 127) / 255) as u8
            };
            surface.set_pixel(
                x as u32,
                y as u32,
                source_over(
                    [source[0], source[1], source[2], effective_alpha],
                    destination,
                    1.0,
                ),
            );
        }
    }
}

fn overflow_clip(box_: &LayoutBox, inherited: ClipRect) -> ClipRect {
    let (clip_x, clip_y) = overflow_axes(box_);
    if !clip_x && !clip_y {
        return inherited;
    }
    let content = box_.content_box;
    let x0 = content.x - box_.padding.left;
    let y0 = content.y - box_.padding.top;
    let x1 = x0 + content.w + box_.padding.left + box_.padding.right;
    let y1 = y0 + content.h + box_.padding.top + box_.padding.bottom;
    ClipRect {
        x0: if clip_x {
            inherited.x0.max(x0)
        } else {
            inherited.x0
        },
        y0: if clip_y {
            inherited.y0.max(y0)
        } else {
            inherited.y0
        },
        x1: if clip_x {
            inherited.x1.min(x1)
        } else {
            inherited.x1
        },
        y1: if clip_y {
            inherited.y1.min(y1)
        } else {
            inherited.y1
        },
    }
}

fn overflow_axes(box_: &LayoutBox) -> (bool, bool) {
    let shorthand = box_
        .style
        .get("overflow")
        .is_some_and(|value| !value.eq_ignore_ascii_case("visible"));
    let clip_x = shorthand
        || box_
            .style
            .get("overflow-x")
            .is_some_and(|value| !value.eq_ignore_ascii_case("visible"));
    let clip_y = shorthand
        || box_
            .style
            .get("overflow-y")
            .is_some_and(|value| !value.eq_ignore_ascii_case("visible"));
    (clip_x, clip_y)
}

fn overflow_rounded_clip(box_: &LayoutBox) -> Option<RoundedRect> {
    let (clip_x, clip_y) = overflow_axes(box_);
    if !clip_x || !clip_y {
        return None;
    }
    let outer = resolve_corner_radii(box_, &box_.border_box);
    if outer == CornerRadii::default() {
        return None;
    }
    let content = box_.content_box;
    let rect = amazing_layout::Rect {
        x: content.x - box_.padding.left,
        y: content.y - box_.padding.top,
        w: content.w + box_.padding.left + box_.padding.right,
        h: content.h + box_.padding.top + box_.padding.bottom,
    };
    Some(RoundedRect {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
        radii: CornerRadii {
            top_left: (
                (outer.top_left.0 - box_.border.left).max(0.0),
                (outer.top_left.1 - box_.border.top).max(0.0),
            ),
            top_right: (
                (outer.top_right.0 - box_.border.right).max(0.0),
                (outer.top_right.1 - box_.border.top).max(0.0),
            ),
            bottom_right: (
                (outer.bottom_right.0 - box_.border.right).max(0.0),
                (outer.bottom_right.1 - box_.border.bottom).max(0.0),
            ),
            bottom_left: (
                (outer.bottom_left.0 - box_.border.left).max(0.0),
                (outer.bottom_left.1 - box_.border.bottom).max(0.0),
            ),
        },
    })
}

fn fill_rect_clipped(
    surface: &mut Surface,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: [u8; 4],
    clip: ClipRect,
) {
    let clipped = clip.intersect(x, y, width, height);
    let [x0, y0, x1, y1] = clipped.pixels();
    let x0 = x0.max(0).min(surface.width as i64);
    let y0 = y0.max(0).min(surface.height as i64);
    let x1 = x1.max(0).min(surface.width as i64);
    let y1 = y1.max(0).min(surface.height as i64);
    for py in y0..y1 {
        for px in x0..x1 {
            if color[3] == 255 {
                surface.set_pixel(px as u32, py as u32, color);
            } else if color[3] != 0 {
                let destination = surface
                    .get_pixel(px as u32, py as u32)
                    .unwrap_or([0, 0, 0, 0]);
                surface.set_pixel(px as u32, py as u32, source_over(color, destination, 1.0));
            }
        }
    }
}

/// The element's computed `color`, used both for ink and as the meaning of
/// `currentcolor`. Unparseable values fall back to the CSS initial black.
pub(crate) fn resolve_text_color(style: &ComputedStyle) -> [u8; 4] {
    match style.get("color").and_then(parse_css_color) {
        Some(c) => c,
        None => FALLBACK_TEXT_COLOR,
    }
}

/// Resolves one color-bearing property. Fully transparent values yield
/// `None` (nothing to paint); `currentcolor` resolves against the text
/// color; unparseable values yield `None`.
fn resolve_color(style: &ComputedStyle, prop: &str, text_color: [u8; 4]) -> Option<[u8; 4]> {
    let raw = style.get(prop)?;
    if raw.eq_ignore_ascii_case("currentcolor") {
        return Some(text_color);
    }
    match parse_css_color(raw) {
        Some(c) if c[3] != 0 => Some(c),
        _ => None,
    }
}

/// Fills the padding box (content box grown by the resolved paddings).
/// A transparent or unparseable background paints nothing.
fn paint_background(
    surface: &mut Surface,
    b: &LayoutBox,
    text_color: [u8; 4],
    area: crate::background::BoxArea,
    clip: ClipRect,
) {
    let bg = match resolve_color(&b.style, "background-color", text_color) {
        Some(c) => c,
        None => return,
    };
    fill_rounded_rect_clipped(surface, background_raster::area(b, area), bg, clip, &b.table_decoration().background_holes);
}

fn inset_radii(
    outer: CornerRadii,
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
    width: f32,
    height: f32,
) -> CornerRadii {
    normalize_radii(
        CornerRadii {
            top_left: (
                (outer.top_left.0 - left).max(0.0),
                (outer.top_left.1 - top).max(0.0),
            ),
            top_right: (
                (outer.top_right.0 - right).max(0.0),
                (outer.top_right.1 - top).max(0.0),
            ),
            bottom_right: (
                (outer.bottom_right.0 - right).max(0.0),
                (outer.bottom_right.1 - bottom).max(0.0),
            ),
            bottom_left: (
                (outer.bottom_left.0 - left).max(0.0),
                (outer.bottom_left.1 - bottom).max(0.0),
            ),
        },
        width,
        height,
    )
}

/// Four solid strips inside the border edges. Top/bottom span the full
/// width; left/right sit between them so corners are painted once.
/// Sides whose style is none/hidden or whose resolved width is <= 0 are
/// skipped defensively (layout already zeroes them).
fn paint_borders(surface: &mut Surface, b: &LayoutBox, text_color: [u8; 4], clip: ClipRect) {
    if b.table_decoration().suppress_border { return; }
    let bb = &b.border_box;

    let width_of = |side: &str| -> f32 {
        if let Some(st) = b.style.get(&format!("border-{side}-style")) {
            if st.eq_ignore_ascii_case("none") || st.eq_ignore_ascii_case("hidden") {
                return 0.0;
            }
        }
        match side {
            "top" => b.border.top,
            "right" => b.border.right,
            "bottom" => b.border.bottom,
            _ => b.border.left,
        }
    };
    // A missing border-color declaration falls back to currentcolor.
    let color_of = |side: &str| -> [u8; 4] {
        match b.style.get(&format!("border-{side}-color")) {
            None => text_color,
            Some(value) if value.eq_ignore_ascii_case("currentcolor") => text_color,
            Some(value) => parse_css_color(value).unwrap_or(text_color),
        }
    };
    let double = |side: &str| {
        b.style
            .get(&format!("border-{side}-style"))
            .is_some_and(|value| value.eq_ignore_ascii_case("double"))
    };

    let bt = width_of("top");
    let br = width_of("right");
    let bbw = width_of("bottom");
    let bl = width_of("left");

    let radii = resolve_corner_radii(b, bb);
    if radii != CornerRadii::default() {
        paint_rounded_border(
            surface,
            RoundedRect {
                x: bb.x,
                y: bb.y,
                width: bb.w,
                height: bb.h,
                radii,
            },
            [bt, br, bbw, bl],
            [
                color_of("top"),
                color_of("right"),
                color_of("bottom"),
                color_of("left"),
            ],
            clip,
        );
        return;
    }

    if bt > 0.0 {
        paint_border_strip(
            surface,
            [bb.x, bb.y, bb.w, bt],
            color_of("top"),
            double("top"),
            true,
            clip,
        );
    }
    if bbw > 0.0 {
        paint_border_strip(
            surface,
            [bb.x, bb.y + bb.h - bbw, bb.w, bbw],
            color_of("bottom"),
            double("bottom"),
            true,
            clip,
        );
    }
    if bl > 0.0 {
        paint_border_strip(
            surface,
            [bb.x, bb.y + bt, bl, bb.h - bt - bbw],
            color_of("left"),
            double("left"),
            false,
            clip,
        );
    }
    if br > 0.0 {
        paint_border_strip(
            surface,
            [bb.x + bb.w - br, bb.y + bt, br, bb.h - bt - bbw],
            color_of("right"),
            double("right"),
            false,
            clip,
        );
    }
}

/// RC-32: `outline` frames the border box from the *outside*, offset by
/// `outline-offset` (which may be negative, pulling the ring inward, even
/// on top of the border) and `outline-width` thick. Unlike `border` it has
/// no per-side properties and never affects layout -- it is read straight
/// from the raw style map like `border`/`box-shadow` already are, and
/// reuses `paint_border_strip` for the four uniform-color strips (including
/// its `double` handling). `outline-style: auto` (the UA-defined focus
/// ring) is painted the same as `solid`, a documented simplification -- this
/// painter has no concept of a platform focus-ring appearance. Not clipped
/// to any rounded shape of its own (a plain rectangular frame): where a
/// test needs a rounded *result*, that comes from `clip-path`'s own mask
/// (e.g. `clip-path: margin-box` with `border-radius`), not from the
/// outline rounding its own corners to match `border-radius` (CSS UI 4
/// allows, but does not require, an outline that follows the border
/// radius).
fn paint_outline(surface: &mut Surface, b: &LayoutBox, text_color: [u8; 4], clip: ClipRect) {
    let style = b.style.get("outline-style").unwrap_or("none");
    if style.eq_ignore_ascii_case("none") {
        return;
    }
    let width = b
        .style
        .get("outline-width")
        .and_then(|value| resolve_outline_width(value))
        .unwrap_or(3.0); // "medium", matching border-width's own default.
    if width <= 0.0 {
        return;
    }
    let offset = b
        .style
        .get("outline-offset")
        .and_then(|value| resolve_shadow_length(value))
        .unwrap_or(0.0);
    let color = match b.style.get("outline-color") {
        None => text_color,
        // `invert` (also legal per CSS UI 4, XOR-inverting the pixels
        // underneath) has no meaning for this painter's flat compositing;
        // `currentcolor` and an absent/invalid color both fall back to the
        // computed text color, the same policy `border-color` already uses.
        Some(value) if value.eq_ignore_ascii_case("currentcolor") || value.eq_ignore_ascii_case("invert") => {
            text_color
        }
        Some(value) => parse_css_color(value).unwrap_or(text_color),
    };
    let double = style.eq_ignore_ascii_case("double");
    let bb = &b.border_box;
    let inflate = offset + width;
    let x = bb.x - inflate;
    let y = bb.y - inflate;
    let w = bb.w + 2.0 * inflate;
    let h = bb.h + 2.0 * inflate;
    if !(w > 0.0 && h > 0.0) {
        return;
    }
    paint_border_strip(surface, [x, y, w, width], color, double, true, clip);
    paint_border_strip(surface, [x, y + h - width, w, width], color, double, true, clip);
    let middle = (h - 2.0 * width).max(0.0);
    paint_border_strip(surface, [x, y + width, width, middle], color, double, false, clip);
    paint_border_strip(surface, [x + w - width, y + width, width, middle], color, double, false, clip);
}

/// `outline-width` accepts the same `thin`/`medium`/`thick` keywords as
/// `border-width` (same px mapping); otherwise it is a plain length, like
/// `resolve_shadow_length` already handles for `box-shadow`.
fn resolve_outline_width(value: &str) -> Option<f32> {
    match value.trim().to_ascii_lowercase().as_str() {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        other => resolve_shadow_length(other),
    }
}

/// Layout has selected the winning collapsed edges and split them at grid
/// intersections. Paint consumes their absolute rectangles exactly once.
fn paint_table_borders(surface: &mut Surface, table: &LayoutBox, clip: ClipRect) {
    for edge in &table.table_decoration().borders {
        let rect = edge.rect;
        if ![rect.x, rect.y, rect.w, rect.h].iter().all(|v| v.is_finite())
            || rect.w <= 0.0 || rect.h <= 0.0 { continue; }
        paint_border_strip(
            surface,
            [rect.x, rect.y, rect.w, rect.h],
            edge.color,
            edge.style.eq_ignore_ascii_case("double"),
            edge.horizontal,
            clip,
        );
    }
}

fn paint_border_strip(
    surface: &mut Surface,
    rect: [f32; 4],
    color: [u8; 4],
    double: bool,
    horizontal: bool,
    clip: ClipRect,
) {
    let [x, y, width, height] = rect;
    if !double {
        fill_rect_clipped(surface, x, y, width, height, color, clip);
        return;
    }
    let thickness = if horizontal { height } else { width };
    let third = (thickness / 3.0).max(1.0);
    if horizontal {
        fill_rect_clipped(surface, x, y, width, third, color, clip);
        fill_rect_clipped(surface, x, y + height - third, width, third, color, clip);
    } else {
        fill_rect_clipped(surface, x, y, third, height, color, clip);
        fill_rect_clipped(surface, x + width - third, y, third, height, color, clip);
    }
}

fn paint_rounded_border(
    surface: &mut Surface,
    outer: RoundedRect,
    widths: [f32; 4],
    colors: [[u8; 4]; 4],
    clip: ClipRect,
) {
    let [top, right, bottom, left] = widths;
    let inner = RoundedRect {
        x: outer.x + left,
        y: outer.y + top,
        width: (outer.width - left - right).max(0.0),
        height: (outer.height - top - bottom).max(0.0),
        radii: CornerRadii {
            top_left: (
                (outer.radii.top_left.0 - left).max(0.0),
                (outer.radii.top_left.1 - top).max(0.0),
            ),
            top_right: (
                (outer.radii.top_right.0 - right).max(0.0),
                (outer.radii.top_right.1 - top).max(0.0),
            ),
            bottom_right: (
                (outer.radii.bottom_right.0 - right).max(0.0),
                (outer.radii.bottom_right.1 - bottom).max(0.0),
            ),
            bottom_left: (
                (outer.radii.bottom_left.0 - left).max(0.0),
                (outer.radii.bottom_left.1 - bottom).max(0.0),
            ),
        },
    };
    let clipped = clip.intersect(outer.x, outer.y, outer.width, outer.height);
    let [x0, y0, x1, y1] = clipped.pixels();
    for y in y0.max(0)..y1.min(surface.height as i64) {
        for x in x0.max(0)..x1.min(surface.width as i64) {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !outer.contains(px, py) || inner.contains(px, py) {
                continue;
            }
            let distances = [
                py - outer.y,
                outer.x + outer.width - px,
                outer.y + outer.height - py,
                px - outer.x,
            ];
            let side = distances
                .iter()
                .enumerate()
                .min_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .unwrap_or(0);
            if colors[side][3] == 0 {
                continue;
            }
            let destination = surface
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            surface.set_pixel(
                x as u32,
                y as u32,
                source_over(colors[side], destination, 1.0),
            );
        }
    }
}

/// Draws every word of every line box at its absolute position, per
/// character, on the line's baseline.
fn paint_lines(
    surface: &mut Surface,
    b: &LayoutBox,
    text_color: [u8; 4],
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
    images: &dyn ImageProvider,
    clip: ClipRect,
) {
    for line in &b.lines {
        for replaced in &line.replaced {
            let rect = amazing_layout::Rect {
                x: replaced.x,
                y: replaced.y,
                w: replaced.width,
                h: replaced.height,
            };
            if let Some((url, presentation, font_size)) = generated_image_url(b, replaced) {
                paint_generated_image(surface, &rect, &url, presentation, font_size, images, clip);
                continue;
            }
            if let Some(svg) = svg_for_rect(
                images,
                replaced.node,
                &rect,
            ) {
                paint_image(
                    surface,
                    &rect,
                    &svg,
                    images.presentation(replaced.node),
                    clip,
                );
            } else if let Some(image) = resolved_replaced_image(images, replaced.node, None) {
                paint_image(
                    surface,
                    &rect,
                    image,
                    images.presentation(replaced.node),
                    clip,
                );
            }
        }
        for w in &line.words {
            draw_fragment_clipped(
                surface,
                w.x,
                line.baseline,
                &w.text,
                w.font_size,
                text_color,
                fonts,
                glyphs,
                Some(clip.pixels()),
            );
        }
    }
}

fn svg_for_rect(
    images: &dyn ImageProvider,
    node: amazing_dom::NodeIdx,
    rect: &amazing_layout::Rect,
) -> Option<amazing_media::Image> {
    if !rect.w.is_finite() || !rect.h.is_finite() || rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let width = rect.w.ceil();
    let height = rect.h.ceil();
    if width > u32::MAX as f32 || height > u32::MAX as f32 {
        return None;
    }
    images.svg(node, width as u32, height as u32, images.device_scale())
}

/// Nearest-neighbor image scaling with clipped, source-over RGBA pixels.
fn paint_image(
    surface: &mut Surface,
    rect: &amazing_layout::Rect,
    image: &amazing_media::Image,
    presentation: ImagePresentation,
    clip: ClipRect,
) {
    let Some(expected) = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return;
    };
    if image.pixels.len() != expected {
        return;
    }
    let clip = clip.intersect(rect.x, rect.y, rect.w, rect.h);
    let Some(destination) = presentation.destination(*rect, (image.width, image.height)) else {
        return;
    };
    let rect = &destination;
    if image.width == 0 || image.height == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let x0 = (rect.x as f64 - 0.5).ceil() as i64;
    let y0 = (rect.y as f64 - 0.5).ceil() as i64;
    let x1 = (rect.x as f64 + rect.w as f64 - 0.5).ceil() as i64;
    let y1 = (rect.y as f64 + rect.h as f64 - 0.5).ceil() as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let pixel_clip = clip.pixels();
    let clipped_x0 = x0.max(0).max(pixel_clip[0]).min(surface.width as i64);
    let clipped_y0 = y0.max(0).max(pixel_clip[1]).min(surface.height as i64);
    let clipped_x1 = x1.max(0).min(pixel_clip[2]).min(surface.width as i64);
    let clipped_y1 = y1.max(0).min(pixel_clip[3]).min(surface.height as i64);
    for y in clipped_y0..clipped_y1 {
        let source_y = (((y as f64 + 0.5 - rect.y as f64) / rect.h as f64) * image.height as f64)
            .floor()
            .clamp(0.0, image.height.saturating_sub(1) as f64) as u32;
        for x in clipped_x0..clipped_x1 {
            let source_x = (((x as f64 + 0.5 - rect.x as f64) / rect.w as f64) * image.width as f64)
                .floor()
                .clamp(0.0, image.width.saturating_sub(1) as f64) as u32;
            let source_index = ((source_y as usize * image.width as usize + source_x as usize) * 4)
                .min(image.pixels.len());
            let Some(source) = image.pixels.get(source_index..source_index + 4) else {
                continue;
            };
            let source_color = [source[0], source[1], source[2], source[3]];
            if source_color[3] == 0 {
                continue;
            }
            let destination = surface
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            let blended = source_over(source_color, destination, 1.0);
            surface.set_pixel(x as u32, y as u32, blended);
        }
    }
}

// ---------------------------------------------------------------------------
// tests — trees built BY HAND, everything in memory, deterministic,
// Windows-safe (no hardcoded paths)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_url_image_does_not_fall_back_to_originating_dom_image() {
        struct Both {
            node_image: amazing_media::Image,
            url_image: amazing_media::Image,
        }
        impl ImageProvider for Both {
            fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
                Some(&self.node_image)
            }
            fn image_by_url(&self, url: &str) -> Option<&amazing_media::Image> {
                (url == "https://example.test/generated.png").then_some(&self.url_image)
            }
        }
        let provider = Both {
            node_image: amazing_media::Image { width: 1, height: 1, pixels: vec![0, 0, 255, 255] },
            url_image: amazing_media::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255] },
        };
        let node = amazing_dom::NodeIdx(1);
        assert_eq!(resolved_replaced_image(&provider, node, Some("https://example.test/generated.png")).unwrap().pixels, provider.url_image.pixels);
        assert!(resolved_replaced_image(&provider, node, Some("https://example.test/missing.png")).is_none());
        assert_eq!(resolved_replaced_image(&provider, node, None).unwrap().pixels, provider.node_image.pixels);
    }

    #[test]
    fn two_generated_images_on_one_node_paint_by_distinct_url_keys() {
        struct TwoUrls {
            red: amazing_media::Image,
            blue: amazing_media::Image,
        }
        impl ImageProvider for TwoUrls {
            fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> { None }
            fn image_by_url(&self, url: &str) -> Option<&amazing_media::Image> {
                match url {
                    "https://example.test/a.png" => Some(&self.red),
                    "https://example.test/b.png" => Some(&self.blue),
                    _ => None,
                }
            }
        }
        let text = amazing_text::FallbackTextSystem::new();
        let root = amazing_layout::layout_document_with_text(
            "<body></body>",
            "body{margin:0}body::before{content:url(https://example.test/a.png) url(https://example.test/b.png)}",
            700.0,
            &text,
        );
        let fragments: Vec<_> = root.styled_replaced_fragments().collect();
        assert_eq!(fragments.len(), 2);
        assert_ne!(fragments[0].source_order, fragments[1].source_order);
        let provider = TwoUrls {
            red: amazing_media::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255] },
            blue: amazing_media::Image { width: 1, height: 1, pixels: vec![0, 0, 255, 255] },
        };
        let surface = paint_document_with_text_resources(&root, 700, 200, &text, &provider);
        let pixel = |index: usize| {
            let rect = fragments[index].content_box;
            surface.get_pixel((rect.x + 1.0) as u32, (rect.y + 1.0) as u32)
        };
        assert_eq!(pixel(0), Some([255, 0, 0, 255]));
        assert_eq!(pixel(1), Some([0, 0, 255, 255]));
    }

    #[test]
    fn generated_gradient_renders_through_replaced_image_key() {
        let text = amazing_text::FallbackTextSystem::new();
        let mut root = amazing_layout::layout_document_with_text(
            "<body></body>",
            "body{margin:0}body::before{content:url(https://example.test/image.png);width:20px;height:10px}",
            100.0,
            &text,
        );
        // CSS's generated-content parser currently admits url() but not
        // gradients. Substitute the paint-facing key to exercise the shared
        // generated-image seam until the CSS owner opens that syntax.
        if let amazing_layout::Flow::Inlines(items) = &mut root.flow {
            for item in items {
                if let amazing_layout::InlineNode::Replaced { replaced, .. } = item {
                    replaced.generated_url = Some("linear-gradient(to right, red, blue)".into());
                }
            }
        }
        let fragment = root.styled_replaced_fragments().next().expect("generated gradient fragment");
        assert!(fragment.generated_url.as_deref().is_some_and(|key| key.starts_with("linear-gradient")));
        let surface = paint_document_with_text_resources(&root, 100, 50, &text, &NoImages);
        let y = (fragment.content_box.y + 2.0) as u32;
        let left = surface.get_pixel((fragment.content_box.x + 2.0) as u32, y).unwrap();
        let right = surface.get_pixel((fragment.content_box.x + fragment.content_box.w - 3.0) as u32, y).unwrap();
        assert!(left[0] > left[2], "left gradient pixel: {left:?}");
        assert!(right[2] > right[0], "right gradient pixel: {right:?}");
    }

    #[test]
    fn list_style_gradient_reaches_generated_image_painter() {
        fn generated(box_: &LayoutBox) -> Option<(String, amazing_layout::Rect)> {
            for fragment in box_.styled_replaced_fragments() {
                if let Some(key) = fragment.generated_url {
                    if key.starts_with("linear-gradient") { return Some((key, fragment.content_box)); }
                }
            }
            if let amazing_layout::Flow::Blocks(children)
                | amazing_layout::Flow::Flex(children)
                | amazing_layout::Flow::Grid(children) = &box_.flow {
                for child in children { if let Some(found) = generated(child) { return Some(found); } }
            }
            for child in &box_.positioned {
                if let Some(found) = generated(child) { return Some(found); }
            }
            None
        }
        let text = amazing_text::FallbackTextSystem::new();
        let root = amazing_layout::layout_document_with_text(
            "<body><ul><li>item</li></ul></body>",
            "body{margin:0}ul{margin:0 0 0 200px}li{list-style-image:linear-gradient(red,blue)}",
            500.0,
            &text,
        );
        let (key, rect) = generated(&root).expect("gradient list marker");
        assert!(key.starts_with("linear-gradient"));
        let surface = paint_document_with_text_resources(&root, 500, 300, &text, &NoImages);
        let point = surface.get_pixel((rect.x + rect.w * 0.5) as u32, (rect.y + rect.h * 0.25) as u32).unwrap();
        assert!(point[0] > point[2] && point[2] > 0, "gradient marker pixel: {point:?}");
    }
    use crate::text::{draw_fragment, RectInkProvider};
    use amazing_layout::{Flow, Rect};

    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn style(pairs: &[(&str, &str)]) -> ComputedStyle {
        pairs
            .iter()
            .map(|&(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    /// A leaf block box placed manually (layout already ran, conceptually):
    /// zero margin/padding/border unless the caller adds them.
    fn block(style_pairs: &[(&str, &str)], x: f32, y: f32, w: f32, h: f32) -> LayoutBox {
        let mut b = LayoutBox::new(
            None,
            amazing_layout::Display::Block,
            style(style_pairs),
            16.0,
            19.2,
        );
        b.border_box = Rect { x, y, w, h };
        b.content_box = b.border_box;
        b.flow = Flow::Empty;
        b
    }

    fn sides(t: f32, r: f32, btm: f32, l: f32) -> amazing_layout::values::Sides {
        amazing_layout::values::Sides {
            top: t,
            right: r,
            bottom: btm,
            left: l,
        }
    }

    #[test]
    fn shaped_painter_rasterizes_glyph_ids_without_legacy_words() {
        let mut root = block(&[], 0.0, 0.0, 20.0, 20.0);
        root.lines.push(amazing_layout::LineBox {
            y_top: 0.0,
            height: 12.0,
            baseline: 10.0,
            words: Vec::new(),
            glyphs: vec![amazing_layout::GlyphFrag {
                node: amazing_dom::NodeIdx(1),
                font: amazing_text::FontKey(0),
                glyph: amazing_text::GlyphId('A' as u32),
                cluster: 0..1,
                source_range: 0..1,
                source_mappings: Vec::new(),
                x: 2.0,
                y: 0.0,
                advance_x: 5.5,
                advance_y: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                font_size: 10.0,
                writing_mode: amazing_text::WritingMode::HorizontalTb,
                orientation: amazing_text::GlyphOrientation::Upright,
                scale_x: 1.0,
                scale_y: 1.0,
                variations: Vec::new(),
                synthesis: amazing_text::FontSynthesis::default(),
                color: "black".into(),
            }],
            decorations: Vec::new(),
            replaced: Vec::new(),
        });
        let text = amazing_text::FallbackTextSystem::new();
        let surface = paint_document_with_text_resources(&root, 20, 20, &text, &NoImages);

        assert_eq!(surface.get_pixel(3, 5), Some([0, 0, 0, 255]));
    }

    #[test]
    fn sideways_glyph_pose_rotates_the_rasterized_ink() {
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_document_with_text(
            "<body><p style=\"writing-mode:sideways-lr;font-size:20px;line-height:20px\">A</p></body>",
            "body{margin:20px}p{margin:0}",
            100.0,
            &text,
        );
        let surface = paint_document_with_text_resources(&layout, 100, 100, &text, &NoImages);
        let ink = (0..surface.height)
            .flat_map(|y| (0..surface.width).map(move |x| (x, y)))
            .filter(|(x, y)| surface.get_pixel(*x, *y) == Some([0, 0, 0, 255]))
            .collect::<Vec<_>>();
        let min_x = ink.iter().map(|(x, _)| *x).min().unwrap();
        let max_x = ink.iter().map(|(x, _)| *x).max().unwrap();
        let min_y = ink.iter().map(|(_, y)| *y).min().unwrap();
        let max_y = ink.iter().map(|(_, y)| *y).max().unwrap();

        assert!(max_x - min_x > max_y - min_y);
    }

    #[test]
    fn vertical_rl_starts_its_column_at_the_logical_right_edge() {
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_document_with_text(
            "<body><p style=\"writing-mode:vertical-rl;width:100px;height:100px;font-size:20px;line-height:20px\">漢</p></body>",
            "body,p{margin:0}",
            100.0,
            &text,
        );
        let surface = paint_document_with_text_resources(&layout, 100, 100, &text, &NoImages);
        let ink = (0..surface.height)
            .flat_map(|y| (0..surface.width).map(move |x| (x, y)))
            .filter(|(x, y)| surface.get_pixel(*x, *y) == Some([0, 0, 0, 255]))
            .collect::<Vec<_>>();
        let min_x = ink.iter().map(|(x, _)| *x).min().unwrap();
        let max_x = ink.iter().map(|(x, _)| *x).max().unwrap();

        assert!(min_x >= 70, "vertical-rl ink started at x={min_x}");
        assert!(max_x < 100, "vertical-rl ink escaped at x={max_x}");
    }

    #[test]
    fn shaped_painter_fills_inline_decorations_before_glyphs() {
        let mut root = block(&[], 0.0, 0.0, 20.0, 20.0);
        root.lines.push(amazing_layout::LineBox {
            y_top: 0.0,
            height: 12.0,
            baseline: 10.0,
            words: Vec::new(),
            glyphs: Vec::new(),
            decorations: vec![amazing_layout::InlineDecorationFrag {
                node: amazing_dom::NodeIdx(1),
                x: 2.0,
                y: 3.0,
                width: 4.0,
                height: 5.0,
                color: "black".into(),
            }],
            replaced: Vec::new(),
        });
        let text = amazing_text::FallbackTextSystem::new();
        let surface = paint_document_with_text_resources(&root, 20, 20, &text, &NoImages);

        assert_eq!(surface.get_pixel(3, 4), Some([0, 0, 0, 255]));
    }

    // ------------------------------------------------------------------
    // Smoke test: a two-box LayoutTree built BY HAND.
    // ------------------------------------------------------------------

    #[test]
    fn two_box_tree_paints_backgrounds_in_painter_order() {
        // Parent red at (0,0) 10x10; child blue at (5,5) 10x10 overflowing
        // both the parent and the 20x10 viewport.
        let mut parent = block(&[("background-color", "red")], 0.0, 0.0, 10.0, 10.0);
        let child = block(&[("background-color", "blue")], 5.0, 5.0, 10.0, 10.0);
        parent.flow = Flow::Blocks(vec![child]);

        let fonts = FontSource::fallback();
        let mut surf = paint_document(&parent, 20, 10, &fonts);

        // Background present INSIDE each box...
        assert_eq!(surf.get_pixel(2, 2), Some([255, 0, 0, 255])); // parent only
        assert_eq!(surf.get_pixel(1, 7), Some([255, 0, 0, 255])); // parent only
        assert_eq!(surf.get_pixel(7, 7), Some([0, 0, 255, 255])); // both: child wins
        assert_eq!(surf.get_pixel(12, 8), Some([0, 0, 255, 255])); // child, clipped
        assert_eq!(surf.get_pixel(14, 9), Some([0, 0, 255, 255])); // child's last col
                                                                   // ...and ABSENT outside them.
        assert_eq!(surf.get_pixel(18, 1), Some(WHITE)); // right of parent
        assert_eq!(surf.get_pixel(11, 2), Some(WHITE)); // right of parent
        assert_eq!(surf.get_pixel(15, 9), Some(WHITE)); // past child's right edge

        // Ink bar above a baseline, drawn through the same entry point
        // paint_lines uses (hand-building a LineBox would require DOM node
        // ids, which carry no meaning to the painter).
        draw_fragment(
            &mut surf,
            2.0,
            9.0,
            "W",
            10.0,
            [0, 0, 0, 255],
            &fonts,
            &RectInkProvider,
        );
        // Ink height at 10px = 7 -> rows 2..=8 sit on baseline 9. Column 3
        // is inside the parent box, so the surrounding background is red:
        // the ink must sit strictly between two red rows.
        assert_eq!(surf.get_pixel(3, 8), Some([0, 0, 0, 255])); // ON the baseline
        assert_eq!(surf.get_pixel(3, 2), Some([0, 0, 0, 255])); // top ink row
        assert_eq!(surf.get_pixel(3, 1), Some([255, 0, 0, 255])); // red above ink
        assert_eq!(surf.get_pixel(3, 9), Some([255, 0, 0, 255])); // red below baseline
    }

    #[test]
    fn positioned_child_paints_after_normal_flow_inside_relative_parent() {
        let mut root = block(&[], 0.0, 0.0, 12.0, 12.0);
        let mut relative = block(&[("position", "relative")], 0.0, 0.0, 10.0, 10.0);
        relative.node = Some(amazing_dom::NodeIdx(1));
        let mut red = block(&[("background-color", "red")], 0.0, 0.0, 8.0, 10.0);
        red.node = Some(amazing_dom::NodeIdx(3));
        let mut blue = block(&[("background-color", "blue")], 0.0, 0.0, 8.0, 10.0);
        blue.node = Some(amazing_dom::NodeIdx(4));
        let mut green = block(
            &[("position", "absolute"), ("background-color", "green")],
            5.0,
            0.0,
            5.0,
            10.0,
        );
        // The positioned child precedes both flow siblings in DOM order.
        green.node = Some(amazing_dom::NodeIdx(2));
        relative.flow = Flow::Blocks(vec![red, blue]);
        relative.positioned.push(green);
        root.flow = Flow::Blocks(vec![relative]);

        let surface = paint_document(&root, 12, 12, &FontSource::fallback());
        assert_eq!(surface.get_pixel(2, 5), Some([0, 0, 255, 255]));
        assert_eq!(surface.get_pixel(6, 5), Some([0, 128, 0, 255]));
        assert_eq!(surface.get_pixel(9, 5), Some([0, 128, 0, 255]));
        assert_eq!(surface.get_pixel(11, 5), Some(WHITE));
    }

    #[test]
    fn calc_border_radius_uses_layouts_axis_specific_used_values() {
        let b = block(
            &[("border-radius", "calc((1/32 * 100%) + 5px) calc((1/64 * 100%) - 2px) calc(10px + (1/256 * 100%)) calc((1/16 * 100%) - 3px) / calc((1/32 * 100%) - (1px + (1/128 * 100%))) calc(1/16 * 100%) calc(10px) 3px")],
            0.0,
            0.0,
            512.0,
            256.0,
        );
        let radii = resolve_corner_radii(&b, &b.border_box);
        assert_eq!(radii.top_left, (21.0, 5.0));
        assert_eq!(radii.top_right, (6.0, 16.0));
        assert_eq!(radii.bottom_right, (12.0, 10.0));
        assert_eq!(radii.bottom_left, (29.0, 3.0));
    }

    #[test]
    fn overflow_hidden_clips_descendant_background_to_padding_box() {
        let mut parent = block(&[("overflow", "hidden")], 0.0, 0.0, 10.0, 10.0);
        let child = block(&[("background-color", "red")], 0.0, 8.0, 10.0, 10.0);
        parent.flow = Flow::Blocks(vec![child]);

        let surface = paint_document(&parent, 10, 20, &FontSource::fallback());
        assert_eq!(surface.get_pixel(5, 8), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(5, 9), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(5, 10), Some(WHITE));
        assert_eq!(surface.get_pixel(5, 17), Some(WHITE));
    }

    struct OneImage {
        node: amazing_dom::NodeIdx,
        image: amazing_media::Image,
    }

    impl ImageProvider for OneImage {
        fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
            (node == self.node).then_some(&self.image)
        }
    }

    struct OneSvg(amazing_dom::NodeIdx);
    impl ImageProvider for OneSvg {
        fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
            None
        }
        fn svg(
            &self,
            node: amazing_dom::NodeIdx,
            width: u32,
            height: u32,
            dpr: f32,
        ) -> Option<amazing_media::Image> {
            assert_eq!((width, height, dpr), (2, 1, 1.0));
            (node == self.0).then_some(amazing_media::Image {
                width: 2,
                height: 1,
                pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
            })
        }
    }

    #[test]
    fn svg_provider_paints_inline_content_rect() {
        let node = amazing_dom::NodeIdx(7);
        let mut root = block(&[], 0.0, 0.0, 4.0, 2.0);
        root.lines.push(amazing_layout::LineBox {
            y_top: 0.0,
            height: 1.0,
            baseline: 1.0,
            words: Vec::new(),
            glyphs: Vec::new(),
            decorations: Vec::new(),
            replaced: vec![amazing_layout::ReplacedFrag {
                node,
                source_order: None,
                x: 1.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            }],
        });
        let surface = paint_document_with_resources(
            &root,
            4,
            2,
            &FontSource::fallback(),
            &RectInkProvider,
            &OneSvg(node),
        );
        assert_eq!(surface.get_pixel(0, 0), Some(WHITE));
        assert_eq!(surface.get_pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(2, 0), Some([0, 255, 0, 255]));
        assert_eq!(surface.get_pixel(3, 0), Some(WHITE));
    }

    #[test]
    fn svg_root_paints_block_content_rect() {
        let node = amazing_dom::NodeIdx(9);
        let mut root = block(&[], 0.0, 0.0, 2.0, 1.0);
        root.svg_root = Some(node);
        let surface = paint_document_with_resources(
            &root,
            3,
            1,
            &FontSource::fallback(),
            &RectInkProvider,
            &OneSvg(node),
        );
        assert_eq!(surface.get_pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(1, 0), Some([0, 255, 0, 255]));
        assert_eq!(surface.get_pixel(2, 0), Some(WHITE));
    }

    #[test]
    fn mix_blend_mode_group_blends_child_over_parent_backdrop() {
        let mut parent = block(&[("background-color", "#808080")], 0.0, 0.0, 4.0, 4.0);
        let child = block(
            &[("background-color", "red"), ("mix-blend-mode", "multiply")],
            0.0,
            0.0,
            2.0,
            2.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 4, 4, &FontSource::fallback());
        assert_eq!(surface.get_pixel(1, 1), Some([128, 0, 0, 255]));
        assert_eq!(surface.get_pixel(3, 3), Some([128, 128, 128, 255]));
    }

    #[test]
    fn isolation_opens_a_group_without_changing_normal_pixels() {
        let mut parent = block(&[("background-color", "#808080")], 0.0, 0.0, 4.0, 4.0);
        let child = block(
            &[("background-color", "red"), ("isolation", "isolate")],
            0.0,
            0.0,
            2.0,
            2.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 4, 4, &FontSource::fallback());
        assert_eq!(surface.get_pixel(1, 1), Some([255, 0, 0, 255]));
    }

    #[test]
    fn filter_color_function_applies_to_only_the_element_subtree() {
        let mut parent = block(&[("background-color", "white")], 0.0, 0.0, 4.0, 4.0);
        let child = block(
            &[("background-color", "red"), ("filter", "grayscale(1)")],
            0.0,
            0.0,
            2.0,
            2.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 4, 4, &FontSource::fallback());
        let luma = (0.2126_f32 * 255.0).round() as u8;
        assert_eq!(surface.get_pixel(1, 1), Some([luma, luma, luma, 255]));
        assert_eq!(surface.get_pixel(3, 3), Some([255, 255, 255, 255]));
    }

    #[test]
    fn backdrop_filter_group_filters_the_snapshot_behind_it() {
        // The box itself paints no background, so its own group layer is
        // seeded only from the backdrop-filtered snapshot of the parent's
        // red fill within the box's own border box.
        let mut parent = block(&[("background-color", "red")], 0.0, 0.0, 4.0, 4.0);
        let child = block(&[("backdrop-filter", "grayscale(1)")], 0.0, 0.0, 2.0, 2.0);
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 4, 4, &FontSource::fallback());
        let luma = (0.2126_f32 * 255.0).round() as u8;
        assert_eq!(surface.get_pixel(1, 1), Some([luma, luma, luma, 255]));
        // Outside the box's border box, the backdrop is untouched.
        assert_eq!(surface.get_pixel(3, 3), Some([255, 0, 0, 255]));
    }

    #[test]
    fn blur_filter_group_softens_its_own_content_edge_into_its_surrounding_transparency() {
        // `blur(1px)`'s three-box-pass support reaches 2px past the box's
        // own edge; a 4px margin on every side keeps the far corner clear.
        let mut parent = block(&[("background-color", "white")], 0.0, 0.0, 14.0, 14.0);
        let child = block(
            &[("background-color", "black"), ("filter", "blur(1px)")],
            4.0,
            4.0,
            6.0,
            6.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 14, 14, &FontSource::fallback());
        assert_eq!(surface.get_pixel(7, 7), Some([0, 0, 0, 255]));
        let edge = surface.get_pixel(3, 7).expect("just outside the unblurred box");
        assert!(edge[0] < 255, "blur should darken just outside the box: {edge:?}");
        assert_eq!(
            surface.get_pixel(0, 0),
            Some([255, 255, 255, 255]),
            "far corner stays untouched"
        );
    }

    #[test]
    fn drop_shadow_filter_group_draws_its_shadow_behind_its_own_content() {
        let mut parent = block(&[("background-color", "white")], 0.0, 0.0, 8.0, 8.0);
        let child = block(
            &[
                ("background-color", "black"),
                ("filter", "drop-shadow(2px 0px 0px red)"),
            ],
            3.0,
            3.0,
            2.0,
            2.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 8, 8, &FontSource::fallback());
        // The box itself stays opaque black.
        assert_eq!(surface.get_pixel(3, 3), Some([0, 0, 0, 255]));
        // The shadow, offset 2px right with zero blur, shows red just past
        // the box's own right edge (the box spans x in [3, 5)).
        assert_eq!(surface.get_pixel(6, 3), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(0, 0), Some([255, 255, 255, 255]));
    }

    #[test]
    fn clip_path_circle_clips_the_elements_own_content_to_the_disc() {
        let mut parent = block(&[("background-color", "white")], 0.0, 0.0, 10.0, 10.0);
        let child = block(
            &[("background-color", "black"), ("clip-path", "circle(3px at center)")],
            2.0,
            2.0,
            6.0,
            6.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 10, 10, &FontSource::fallback());
        // The circle's center (also the box's own center) stays black.
        assert_eq!(surface.get_pixel(5, 5), Some([0, 0, 0, 255]));
        // A corner of the box's own bounding rect, outside the inscribed
        // circle, is clipped away and shows the parent's white background.
        assert_eq!(surface.get_pixel(2, 2), Some([255, 255, 255, 255]));
    }

    #[test]
    fn clip_path_polygon_clips_the_elements_own_content_to_the_triangle() {
        let mut parent = block(&[("background-color", "white")], 0.0, 0.0, 10.0, 10.0);
        let child = block(
            &[("background-color", "black"), ("clip-path", "polygon(0 0, 6px 0, 0 6px)")],
            2.0,
            2.0,
            6.0,
            6.0,
        );
        parent.flow = Flow::Blocks(vec![child]);
        let surface = paint_document(&parent, 10, 10, &FontSource::fallback());
        // Near the child's own top-left corner (the triangle's right angle).
        assert_eq!(surface.get_pixel(3, 3), Some([0, 0, 0, 255]));
        // The child's opposite corner, outside the triangle, shows the
        // parent's white background through instead of the child's own.
        assert_eq!(surface.get_pixel(7, 7), Some([255, 255, 255, 255]));
    }

    #[test]
    fn mask_image_group_uses_the_referenced_images_luminance_as_alpha() {
        struct MaskProvider {
            mask: amazing_media::Image,
        }
        impl ImageProvider for MaskProvider {
            fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
                None
            }
            fn image_by_url(&self, url: &str) -> Option<&amazing_media::Image> {
                (url == "https://example.test/mask.png").then(|| &self.mask)
            }
        }
        let mut root = block(&[], 0.0, 0.0, 4.0, 2.0);
        let child = block(
            &[("background-color", "black"), ("mask-image", "url(https://example.test/mask.png)")],
            0.0,
            0.0,
            4.0,
            2.0,
        );
        root.flow = Flow::Blocks(vec![child]);
        // A 2x1 mask, stretched to fill the child's 4x2 border box: the left
        // half is opaque white (full luminance mask), the right half fully
        // transparent (masked out regardless of its color).
        let images = MaskProvider {
            mask: amazing_media::Image {
                width: 2,
                height: 1,
                pixels: vec![255, 255, 255, 255, 255, 255, 255, 0],
            },
        };
        let surface = paint_document_with_resources(&root, 4, 2, &FontSource::fallback(), &RectInkProvider, &images);
        assert_eq!(surface.get_pixel(0, 0), Some([0, 0, 0, 255]), "left half is unmasked");
        assert_eq!(surface.get_pixel(0, 1), Some([0, 0, 0, 255]), "left half is unmasked");
        assert_eq!(surface.get_pixel(3, 0), Some([255, 255, 255, 255]), "right half is masked out to the canvas");
        assert_eq!(surface.get_pixel(3, 1), Some([255, 255, 255, 255]), "right half is masked out to the canvas");
    }

    #[test]
    fn paints_inline_replaced_content_through_the_resource_seam() {
        let node = amazing_dom::NodeIdx(7);
        let mut root = block(&[], 0.0, 0.0, 4.0, 2.0);
        root.lines.push(amazing_layout::LineBox {
            y_top: 0.0,
            height: 1.0,
            baseline: 1.0,
            words: Vec::new(),
            glyphs: Vec::new(),
            decorations: Vec::new(),
            replaced: vec![amazing_layout::ReplacedFrag {
                node,
                source_order: None,
                x: 1.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
            }],
        });
        let images = OneImage {
            node,
            image: amazing_media::Image {
                width: 2,
                height: 1,
                pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
            },
        };

        let surface = paint_document_with_resources(
            &root,
            4,
            2,
            &FontSource::fallback(),
            &RectInkProvider,
            &images,
        );
        assert_eq!(surface.get_pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(surface.get_pixel(2, 0), Some([0, 255, 0, 255]));
    }

    // ------------------------------------------------------------------

    #[test]
    fn transparent_background_paints_nothing() {
        let b = block(&[("background-color", "transparent")], 0.0, 0.0, 10.0, 10.0);
        let surf = paint_document(&b, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(5, 5), Some(WHITE));
    }

    #[test]
    fn garbage_background_value_paints_nothing() {
        let b = block(
            &[("background-color", "definitely-not-a-color")],
            0.0,
            0.0,
            10.0,
            10.0,
        );
        let surf = paint_document(&b, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(5, 5), Some(WHITE));
    }

    #[test]
    fn background_covers_padding_box_exactly() {
        // Border box (4,4) 10x6; paddings t2 r1 b2 l1; content box inset
        // accordingly. With zero borders the padding box equals the border
        // box: cols 4..=13, rows 4..=9.
        let mut b = block(&[("background-color", "green")], 4.0, 4.0, 10.0, 6.0);
        b.padding = sides(2.0, 1.0, 2.0, 1.0);
        b.content_box = Rect {
            x: 5.0,
            y: 6.0,
            w: 8.0,
            h: 2.0,
        };
        let surf = paint_document(&b, 30, 30, &FontSource::fallback());
        assert_eq!(surf.get_pixel(4, 4), Some([0, 128, 0, 255])); // first px
        assert_eq!(surf.get_pixel(13, 9), Some([0, 128, 0, 255])); // last px
        assert_eq!(surf.get_pixel(3, 4), Some(WHITE)); // left of padding box
        assert_eq!(surf.get_pixel(4, 3), Some(WHITE)); // above it
        assert_eq!(surf.get_pixel(14, 4), Some(WHITE)); // right of it
        assert_eq!(surf.get_pixel(4, 10), Some(WHITE)); // below it
    }

    #[test]
    fn solid_border_strips_frame_the_box() {
        let mut b = block(&[], 2.0, 2.0, 10.0, 8.0);
        b.border = sides(1.0, 2.0, 1.0, 2.0);
        b.style = style(&[
            ("border-top-style", "solid"),
            ("border-right-style", "solid"),
            ("border-bottom-style", "solid"),
            ("border-left-style", "solid"),
            ("border-top-color", "#00ff00"),
            ("border-right-color", "black"),
            ("border-bottom-color", "black"),
            ("border-left-color", "black"),
        ]);
        let surf = paint_document(&b, 20, 20, &FontSource::fallback());
        assert_eq!(surf.get_pixel(5, 2), Some([0, 255, 0, 255])); // top strip
        assert_eq!(surf.get_pixel(5, 9), Some([0, 0, 0, 255])); // bottom strip
        assert_eq!(surf.get_pixel(2, 5), Some([0, 0, 0, 255])); // left strip
        assert_eq!(surf.get_pixel(11, 5), Some([0, 0, 0, 255])); // right strip
                                                                 // Interior untouched; outside untouched.
        assert_eq!(surf.get_pixel(5, 5), Some(WHITE));
        assert_eq!(surf.get_pixel(1, 5), Some(WHITE));
        assert_eq!(surf.get_pixel(13, 5), Some(WHITE));
    }

    #[test]
    fn none_style_border_is_skipped_even_with_width() {
        let mut b = block(&[], 0.0, 0.0, 6.0, 6.0);
        b.border = sides(5.0, 0.0, 0.0, 0.0);
        b.style = style(&[
            ("border-top-style", "none"),
            ("border-top-color", "red"),
            ("color", "blue"),
        ]);
        let surf = paint_document(&b, 8, 8, &FontSource::fallback());
        assert_eq!(surf.get_pixel(3, 0), Some(WHITE)); // no strip despite width 5

        // Same geometry but dashed: still solid ink (documented deviation),
        // resolving currentcolor to the computed color (blue).
        b.style = style(&[("border-top-style", "dashed"), ("color", "blue")]);
        let surf = paint_document(&b, 8, 8, &FontSource::fallback());
        assert_eq!(surf.get_pixel(3, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn currentcolor_border_resolves_against_computed_color() {
        let mut b = block(&[], 0.0, 0.0, 6.0, 6.0);
        b.border = sides(2.0, 0.0, 0.0, 0.0);
        b.style = style(&[
            ("border-top-style", "solid"),
            ("border-top-color", "currentcolor"),
            ("color", "purple"),
        ]);
        let surf = paint_document(&b, 8, 8, &FontSource::fallback());
        assert_eq!(surf.get_pixel(2, 0), Some([128, 0, 128, 255]));
    }

    #[test]
    fn outline_draws_outside_the_border_box_at_its_own_offset() {
        // A 4x4 border box at (5,5), outline 2px solid red, offset 1px:
        // the ring sits between (5-1-2=2) and (5-1=4) on each side.
        let mut b = block(&[], 5.0, 5.0, 4.0, 4.0);
        b.style = style(&[
            ("outline-style", "solid"),
            ("outline-color", "red"),
            ("outline-width", "2px"),
            ("outline-offset", "1px"),
        ]);
        let surf = paint_document(&b, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(3, 3), Some([255, 0, 0, 255]), "outer ring pixel");
        assert_eq!(surf.get_pixel(2, 2), Some([255, 0, 0, 255]), "corner of the ring");
        assert_eq!(surf.get_pixel(4, 4), Some(WHITE), "gap between border box and ring (the offset)");
        assert_eq!(surf.get_pixel(1, 1), Some(WHITE), "outside the ring entirely");
        assert_eq!(surf.get_pixel(6, 6), Some(WHITE), "inside the border box, untouched");
    }

    #[test]
    fn outline_none_or_zero_width_paints_nothing() {
        let mut b = block(&[], 2.0, 2.0, 4.0, 4.0);
        b.style = style(&[("outline-style", "none"), ("outline-color", "red"), ("outline-width", "9px")]);
        let surf = paint_document(&b, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(1, 1), Some(WHITE));

        b.style = style(&[("outline-style", "solid"), ("outline-color", "red"), ("outline-width", "0px")]);
        let surf = paint_document(&b, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(1, 1), Some(WHITE));

        // No outline-style at all defaults to `none`, same as the initial value.
        let plain = block(&[("outline-color", "red"), ("outline-width", "9px")], 2.0, 2.0, 4.0, 4.0);
        let surf = paint_document(&plain, 12, 12, &FontSource::fallback());
        assert_eq!(surf.get_pixel(1, 1), Some(WHITE));
    }

    #[test]
    fn outline_negative_offset_pulls_the_ring_inward_over_the_border() {
        let mut b = block(&[], 4.0, 4.0, 8.0, 8.0);
        b.style = style(&[
            ("outline-style", "solid"),
            ("outline-color", "lime"),
            ("outline-width", "2px"),
            ("outline-offset", "-2px"),
        ]);
        let surf = paint_document(&b, 16, 16, &FontSource::fallback());
        // offset -2, width 2: the ring's outer edge coincides with the
        // border box's own edge (offset + width == 0), so it paints right
        // on top of the box's own perimeter, leaving only the inner core
        // (well past the 2px ring thickness on every side) untouched.
        assert_eq!(surf.get_pixel(4, 4), Some([0, 255, 0, 255]), "top-left corner of the box");
        assert_eq!(surf.get_pixel(11, 11), Some([0, 255, 0, 255]), "bottom-right corner of the box");
        assert_eq!(surf.get_pixel(8, 8), Some(WHITE), "well inside the ring, untouched");
    }

    #[test]
    fn outline_currentcolor_and_medium_keyword_resolve_like_border() {
        let mut b = block(&[], 3.0, 3.0, 4.0, 4.0);
        b.style = style(&[
            ("outline-style", "solid"),
            ("outline-color", "currentcolor"),
            ("outline-width", "medium"), // 3px, same mapping as border-width.
            ("color", "blue"),
        ]);
        let surf = paint_document(&b, 16, 16, &FontSource::fallback());
        assert_eq!(surf.get_pixel(0, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn outline_double_style_paints_two_strips_like_double_border() {
        let mut b = block(&[], 5.0, 5.0, 6.0, 6.0);
        b.style = style(&[
            ("outline-style", "double"),
            ("outline-color", "black"),
            ("outline-width", "3px"),
        ]);
        let surf = paint_document(&b, 20, 20, &FontSource::fallback());
        // The 3px-thick top ring splits into two 1px strips with a 1px gap;
        // outer edge at y=2 (5 - 3), so rows 2 and 4 are ink, row 3 is not.
        assert_eq!(surf.get_pixel(6, 2), Some([0, 0, 0, 255]));
        assert_eq!(surf.get_pixel(6, 3), Some(WHITE));
        assert_eq!(surf.get_pixel(6, 4), Some([0, 0, 0, 255]));
    }

    #[test]
    fn deep_tree_paints_children_after_parents_everywhere() {
        // grandparent(red) > mid(green) > leaf(blue), overlapping stack.
        let leaf = block(&[("background-color", "blue")], 6.0, 6.0, 8.0, 8.0);
        let mut mid = block(&[("background-color", "green")], 3.0, 3.0, 10.0, 10.0);
        mid.flow = Flow::Blocks(vec![leaf]);
        let mut gp = block(&[("background-color", "red")], 0.0, 0.0, 16.0, 16.0);
        gp.flow = Flow::Blocks(vec![mid]);

        let surf = paint_document(&gp, 20, 20, &FontSource::fallback());
        assert_eq!(surf.get_pixel(1, 1), Some([255, 0, 0, 255]));
        assert_eq!(surf.get_pixel(4, 4), Some([0, 128, 0, 255]));
        assert_eq!(surf.get_pixel(10, 10), Some([0, 0, 255, 255]));
        // Outside everything.
        assert_eq!(surf.get_pixel(19, 19), Some(WHITE));
    }

    #[test]
    fn oversized_geometry_clips_without_panicking() {
        // Extreme coordinates exercise saturating casts in fill_rect.
        let huge = block(&[("background-color", "gray")], -1e9, -1e9, 4e9, 4e9);
        let offscreen = block(&[("background-color", "red")], 1e9, 1e9, 10.0, 10.0);
        let mut root = block(&[], 0.0, 0.0, 0.0, 0.0);
        root.flow = Flow::Blocks(vec![offscreen, huge]);
        let surf = paint_document(&root, 5, 5, &FontSource::fallback());
        assert_eq!(surf.get_pixel(0, 0), Some([128, 128, 128, 255]));
        assert_eq!(surf.get_pixel(4, 4), Some([128, 128, 128, 255]));
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod image_tests;

#[path = "background_raster.rs"]
mod background_raster;

#[cfg(test)]
#[path = "background_tests.rs"]
mod background_tests;

#[path = "control_raster.rs"]
mod control_raster;

#[cfg(test)]
#[path = "control_tests.rs"]
mod control_tests;
