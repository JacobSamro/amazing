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

struct RasterGroup {
    parent: Surface,
    clips: Vec<ClipRect>,
    effect: GroupEffect,
    mask: Option<RoundedRect>,
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
        self.begin_group_with_mask(effect, None);
    }

    fn begin_group_with_mask(&mut self, effect: GroupEffect, mask: Option<RoundedRect>) {
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
        let transparent = Surface::new(self.surface.width, self.surface.height);
        let parent = std::mem::replace(&mut self.surface, transparent);
        let clips = std::mem::replace(&mut self.clips, vec![ClipRect::viewport(&self.surface)]);
        self.groups.push(RasterGroup {
            parent,
            clips,
            effect,
            mask,
        });
    }

    fn end_group(&mut self) {
        let Some(group) = self.groups.pop() else {
            return;
        };
        let child = std::mem::replace(&mut self.surface, group.parent);
        self.layer_pixels = self
            .layer_pixels
            .saturating_sub(child.width as usize * child.height as usize);
        self.clips = group.clips;
        let clip = self.clip();
        composite_group(&mut self.surface, &child, group.effect, group.mask, clip);
    }

    fn push_overflow_clip(&mut self, box_: &LayoutBox) {
        let rounded = overflow_rounded_clip(box_);
        let has_rounded = rounded.is_some();
        if let Some(mask) = rounded {
            self.begin_group_with_mask(
                GroupEffect {
                    opacity: 1.0,
                    transform: crate::display_list::Affine::IDENTITY,
                    origin_x: 0.0,
                    origin_y: 0.0,
                },
                Some(mask),
            );
        }
        self.clips.push(overflow_clip(box_, self.clip()));
        self.rounded_clip_stack.push(has_rounded);
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
        if self.rounded_clip_stack.pop().unwrap_or(false) {
            self.end_group();
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

fn composite_group(
    destination: &mut Surface,
    source: &Surface,
    effect: GroupEffect,
    mask: Option<RoundedRect>,
    clip: ClipRect,
) {
    let matrix = effect.matrix_about_origin();
    let Some(inverse) = matrix.inverse() else {
        return;
    };
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
            let destination_pixel = destination
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            destination.set_pixel(
                x as u32,
                y as u32,
                source_over(source_pixel, destination_pixel, effect.opacity),
            );
        }
    }
}

fn source_over(source: [u8; 4], destination: [u8; 4], opacity: f32) -> [u8; 4] {
    let source_alpha = source[3] as f32 / 255.0 * opacity.clamp(0.0, 1.0);
    let destination_alpha = destination[3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if output_alpha <= 0.0 {
        return [0, 0, 0, 0];
    }
    let channel = |index: usize| {
        ((source[index] as f32 * source_alpha
            + destination[index] as f32 * destination_alpha * (1.0 - source_alpha))
            / output_alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        channel(0),
        channel(1),
        channel(2),
        (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Seam through which the painter obtains decoded pixels for replaced nodes.
pub trait ImageProvider {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image>;
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
            }
            PaintCommand::PushOverflowClip(box_) => state.push_overflow_clip(box_),
            PaintCommand::PopClip => state.pop_clip(),
            PaintCommand::EndGroup => state.end_group(),
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
            }
            PaintCommand::PushOverflowClip(box_) => state.push_overflow_clip(box_),
            PaintCommand::PopClip => state.pop_clip(),
            PaintCommand::EndGroup => state.end_group(),
        }
    }
    state.finish()
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
    paint_background(surface, box_, text_color, background.color_clip, clip);
    if !background_raster::paint(surface, box_, background, images, clip, budget) {
        return false;
    }
    paint_inset_box_shadow(surface, box_, text_color, clip);
    paint_borders(surface, box_, text_color, clip);
    if box_.replaced_size.is_some() {
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
) {
    let clipped = clip.intersect(rounded.x, rounded.y, rounded.width, rounded.height);
    let [x0, y0, x1, y1] = clipped.pixels();
    for y in y0.max(0)..y1.min(surface.height as i64) {
        for x in x0.max(0)..x1.min(surface.width as i64) {
            if !rounded.contains(x as f32 + 0.5, y as f32 + 0.5) {
                continue;
            }
            let destination = surface
                .get_pixel(x as u32, y as u32)
                .unwrap_or([0, 0, 0, 0]);
            surface.set_pixel(x as u32, y as u32, source_over(color, destination, 1.0));
        }
    }
}

fn resolve_corner_radii(style: &ComputedStyle, rect: &amazing_layout::Rect) -> CornerRadii {
    if let (Some(top_left), Some(top_right), Some(bottom_right), Some(bottom_left)) = (
        style.get("border-top-left-radius"),
        style.get("border-top-right-radius"),
        style.get("border-bottom-right-radius"),
        style.get("border-bottom-left-radius"),
    ) {
        return normalize_radii(
            CornerRadii {
                top_left: resolve_radius_pair(top_left, rect.w, rect.h),
                top_right: resolve_radius_pair(top_right, rect.w, rect.h),
                bottom_right: resolve_radius_pair(bottom_right, rect.w, rect.h),
                bottom_left: resolve_radius_pair(bottom_left, rect.w, rect.h),
            },
            rect.w,
            rect.h,
        );
    }
    let Some(value) = style.get("border-radius") else {
        return CornerRadii::default();
    };
    let (horizontal, vertical) = value
        .split_once('/')
        .map(|(horizontal, vertical)| (horizontal, Some(vertical)))
        .unwrap_or((value, None));
    let horizontal = expand_radius_values(horizontal, rect.w);
    let vertical = vertical
        .map(|vertical| expand_radius_values(vertical, rect.h))
        .unwrap_or(horizontal);
    normalize_radii(
        CornerRadii {
            top_left: (horizontal[0], vertical[0]),
            top_right: (horizontal[1], vertical[1]),
            bottom_right: (horizontal[2], vertical[2]),
            bottom_left: (horizontal[3], vertical[3]),
        },
        rect.w,
        rect.h,
    )
}

fn resolve_radius_pair(value: &str, width: f32, height: f32) -> (f32, f32) {
    let values = value.split_ascii_whitespace().collect::<Vec<_>>();
    match values.as_slice() {
        [both] => (resolve_radius(both, width), resolve_radius(both, height)),
        [horizontal, vertical] => (
            resolve_radius(horizontal, width),
            resolve_radius(vertical, height),
        ),
        _ => (0.0, 0.0),
    }
}

fn expand_radius_values(value: &str, basis: f32) -> [f32; 4] {
    let values = value
        .split_ascii_whitespace()
        .map(|value| resolve_radius(value, basis))
        .collect::<Vec<_>>();
    match values.as_slice() {
        [one] => [*one; 4],
        [top_left, top_right] => [*top_left, *top_right, *top_left, *top_right],
        [top_left, other, bottom_right] => [*top_left, *other, *bottom_right, *other],
        [top_left, top_right, bottom_right, bottom_left] => {
            [*top_left, *top_right, *bottom_right, *bottom_left]
        }
        _ => [0.0; 4],
    }
}

fn resolve_radius(value: &str, basis: f32) -> f32 {
    let value = value.trim().to_ascii_lowercase();
    if let Some(percent) = value.strip_suffix('%') {
        percent
            .parse::<f32>()
            .ok()
            .map_or(0.0, |value| basis * value / 100.0)
            .max(0.0)
    } else if let Some(px) = value.strip_suffix("px") {
        px.parse::<f32>().unwrap_or(0.0).max(0.0)
    } else {
        0.0
    }
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
    let mut radii = resolve_corner_radii(&box_.style, &border);
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
        for replaced in &line.replaced {
            if let Some(image) = images.image(replaced.node) {
                paint_image(
                    surface,
                    &amazing_layout::Rect {
                        x: replaced.x,
                        y: replaced.y,
                        w: replaced.width,
                        h: replaced.height,
                    },
                    image,
                    images.presentation(replaced.node),
                    clip,
                );
            }
        }
        paint_line_glyphs(surface, line, text, clip);
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
    let outer = resolve_corner_radii(&box_.style, &box_.border_box);
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
fn resolve_text_color(style: &ComputedStyle) -> [u8; 4] {
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
    fill_rounded_rect_clipped(surface, background_raster::area(b, area), bg, clip);
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

    let radii = resolve_corner_radii(&b.style, bb);
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
            if let Some(image) = images.image(replaced.node) {
                paint_image(
                    surface,
                    &amazing_layout::Rect {
                        x: replaced.x,
                        y: replaced.y,
                        w: replaced.width,
                        h: replaced.height,
                    },
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
