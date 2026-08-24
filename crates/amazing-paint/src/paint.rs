//! Painter's-algorithm rendering of a Phase 3 [`LayoutBox`] tree onto a
//! [`Surface`].
//!
//! Traversal is top-down: for each box, its own background and borders are
//! painted first, then its line-box text fragments, then its children —
//! so descendants correctly overpaint ancestors.
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
//! - flex children paint in flow order exactly like block children
//! - normal and positioned child boxes paint in DOM order (before full
//!   stacking-context/z-index ordering lands)
//!
//! # Deferred / deviations
//!
//! - ALL border styles (dashed/dotted/double/groove/…) render as SOLID —
//!   deliberate Phase 4 simplification.
//! - No border-radius, no box shadows, no list markers, no opacity, no
//!   stacking contexts/z-index reordering.
//! - The legacy word painter does not retain inline color differences; the
//!   shaped provider path does.
//! - The canvas starts opaque white regardless of root background
//!   propagation rules.

use amazing_css::ComputedStyle;
use amazing_layout::font::FontSource;
use amazing_layout::{Flow, LayoutBox};
use amazing_text::{GlyphCacheKey, GlyphContent, GlyphImage, TextSystem};

use crate::color::parse_css_color;
use crate::surface::Surface;
use crate::text::{draw_fragment_clipped, GlyphProvider, RectInkProvider};

const CANVAS_WHITE: [u8; 4] = [255, 255, 255, 255];
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

/// Seam through which the painter obtains decoded pixels for replaced nodes.
pub trait ImageProvider {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image>;
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
    let mut surface = Surface::new(viewport_w, viewport_h);
    surface.clear(CANVAS_WHITE);
    let clip = ClipRect::viewport(&surface);
    paint_box(&mut surface, root, fonts, glyphs, images, clip);
    surface
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
    let mut surface = Surface::new(viewport_w, viewport_h);
    surface.clear(CANVAS_WHITE);
    let clip = ClipRect::viewport(&surface);
    paint_box_with_text(&mut surface, root, text, images, clip);
    surface
}

fn paint_box_with_text(
    surface: &mut Surface,
    box_: &LayoutBox,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    clip: ClipRect,
) {
    let text_color = resolve_text_color(&box_.style);
    paint_background(surface, box_, text_color, clip);
    paint_borders(surface, box_, text_color, clip);
    if box_.replaced_size.is_some() {
        if let Some(image) = box_.node.and_then(|node| images.image(node)) {
            paint_image(surface, &box_.content_box, image, clip);
        }
    }
    let descendant_clip = overflow_clip(box_, clip);
    paint_shaped_lines(surface, box_, text, images, descendant_clip);

    let mut children: Vec<&LayoutBox> = match &box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            children.iter().collect()
        },
        Flow::Inlines(_) | Flow::Empty => Vec::new(),
    };
    children.extend(box_.positioned.iter());
    children.sort_by_key(|child| child.node.map(|node| node.0).unwrap_or(0));
    for child in children {
        paint_box_with_text(surface, child, text, images, descendant_clip);
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
                    clip,
                );
            }
        }
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
    let channels = if image.content == GlyphContent::Color { 4 } else { 1 };
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
                    origin_x
                        + ((image.bearing_y as i64 - 1 - row) as f32 * scale_y).floor() as i64,
                    origin_y
                        + ((image.bearing_x as i64 + column) as f32 * scale_x).floor() as i64,
                ),
                amazing_text::GlyphOrientation::SidewaysLeft => (
                    origin_x
                        + ((-image.bearing_y as i64 + row) as f32 * scale_y).floor() as i64,
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
                .unwrap_or([0, 0, 0, 255]);
            let alpha = coverage as u32;
            let inverse = 255 - alpha;
            let blend = |source: u8, destination: u8| {
                ((source as u32 * alpha + destination as u32 * inverse + 127) / 255) as u8
            };
            surface.set_pixel(
                x as u32,
                y as u32,
                [
                    blend(source[0], destination[0]),
                    blend(source[1], destination[1]),
                    blend(source[2], destination[2]),
                    255,
                ],
            );
        }
    }
}

fn paint_box(
    surface: &mut Surface,
    box_: &LayoutBox,
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
    images: &dyn ImageProvider,
    clip: ClipRect,
) {
    let text_color = resolve_text_color(&box_.style);
    paint_background(surface, box_, text_color, clip);
    paint_borders(surface, box_, text_color, clip);
    if box_.replaced_size.is_some() {
        if let Some(image) = box_.node.and_then(|node| images.image(node)) {
            paint_image(surface, &box_.content_box, image, clip);
        }
    }
    let descendant_clip = overflow_clip(box_, clip);
    paint_lines(
        surface,
        box_,
        text_color,
        fonts,
        glyphs,
        images,
        descendant_clip,
    );

    // Positioned descendants and in-flow children share document-order
    // painting while z-index/stacking contexts remain deferred.
    let mut children: Vec<&LayoutBox> = match &box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            children.iter().collect()
        },
        Flow::Inlines(_) | Flow::Empty => Vec::new(),
    };
    children.extend(box_.positioned.iter());
    children.sort_by_key(|child| child.node.map(|node| node.0).unwrap_or(0));
    for child in children {
        paint_box(surface, child, fonts, glyphs, images, descendant_clip);
    }
}

fn overflow_clip(box_: &LayoutBox, inherited: ClipRect) -> ClipRect {
    let clips = box_
        .style
        .get("overflow")
        .is_some_and(|value| !value.eq_ignore_ascii_case("visible"));
    if !clips {
        return inherited;
    }
    let content = box_.content_box;
    inherited.intersect(
        content.x - box_.padding.left,
        content.y - box_.padding.top,
        content.w + box_.padding.left + box_.padding.right,
        content.h + box_.padding.top + box_.padding.bottom,
    )
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
    surface.fill_rect(
        clipped.x0,
        clipped.y0,
        (clipped.x1 - clipped.x0).max(0.0),
        (clipped.y1 - clipped.y0).max(0.0),
        color,
    );
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
    clip: ClipRect,
) {
    let bg = match resolve_color(&b.style, "background-color", text_color) {
        Some(c) => c,
        None => return,
    };
    let cb = &b.content_box;
    let p = &b.padding;
    fill_rect_clipped(
        surface,
        cb.x - p.left,
        cb.y - p.top,
        cb.w + p.left + p.right,
        cb.h + p.top + p.bottom,
        bg,
        clip,
    );
}

/// Four solid strips inside the border edges. Top/bottom span the full
/// width; left/right sit between them so corners are painted once.
/// Sides whose style is none/hidden or whose resolved width is <= 0 are
/// skipped defensively (layout already zeroes them).
fn paint_borders(
    surface: &mut Surface,
    b: &LayoutBox,
    text_color: [u8; 4],
    clip: ClipRect,
) {
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
        resolve_color(&b.style, &format!("border-{side}-color"), text_color)
            .unwrap_or(text_color)
    };

    let bt = width_of("top");
    let br = width_of("right");
    let bbw = width_of("bottom");
    let bl = width_of("left");

    if bt > 0.0 {
        fill_rect_clipped(surface, bb.x, bb.y, bb.w, bt, color_of("top"), clip);
    }
    if bbw > 0.0 {
        fill_rect_clipped(
            surface,
            bb.x,
            bb.y + bb.h - bbw,
            bb.w,
            bbw,
            color_of("bottom"),
            clip,
        );
    }
    if bl > 0.0 {
        fill_rect_clipped(
            surface,
            bb.x,
            bb.y + bt,
            bl,
            bb.h - bt - bbw,
            color_of("left"),
            clip,
        );
    }
    if br > 0.0 {
        fill_rect_clipped(
            surface,
            bb.x + bb.w - br,
            bb.y + bt,
            br,
            bb.h - bt - bbw,
            color_of("right"),
            clip,
        );
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
    clip: ClipRect,
) {
    if image.width == 0 || image.height == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let x0 = rect.x.round() as i64;
    let y0 = rect.y.round() as i64;
    let x1 = (rect.x + rect.w).round() as i64;
    let y1 = (rect.y + rect.h).round() as i64;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let pixel_clip = clip.pixels();
    let clipped_x0 = x0.max(0).max(pixel_clip[0]).min(surface.width as i64);
    let clipped_y0 = y0.max(0).max(pixel_clip[1]).min(surface.height as i64);
    let clipped_x1 = x1.max(0).min(pixel_clip[2]).min(surface.width as i64);
    let clipped_y1 = y1.max(0).min(pixel_clip[3]).min(surface.height as i64);
    for y in clipped_y0..clipped_y1 {
        let source_y = (((y - y0) as u64) * image.height as u64 / (y1 - y0) as u64)
            .min(image.height.saturating_sub(1) as u64) as u32;
        for x in clipped_x0..clipped_x1 {
            let source_x = (((x - x0) as u64) * image.width as u64 / (x1 - x0) as u64)
                .min(image.width.saturating_sub(1) as u64) as u32;
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
            let alpha = source_color[3] as u16;
            let inverse = 255 - alpha;
            let blended = [
                ((source_color[0] as u16 * alpha + destination[0] as u16 * inverse + 127) / 255) as u8,
                ((source_color[1] as u16 * alpha + destination[1] as u16 * inverse + 127) / 255) as u8,
                ((source_color[2] as u16 * alpha + destination[2] as u16 * inverse + 127) / 255) as u8,
                255,
            ];
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
    use amazing_layout::Rect;

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
        let mut b = LayoutBox::new(None, amazing_layout::Display::Block, style(style_pairs), 16.0, 19.2);
        b.border_box = Rect { x, y, w, h };
        b.content_box = b.border_box;
        b.flow = Flow::Empty;
        b
    }

    fn sides(t: f32, r: f32, btm: f32, l: f32) -> amazing_layout::values::Sides {
        amazing_layout::values::Sides { top: t, right: r, bottom: btm, left: l }
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
            &mut surf, 2.0, 9.0, "W", 10.0, [0, 0, 0, 255],
            &fonts, &RectInkProvider,
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
        let b = block(&[("background-color", "definitely-not-a-color")], 0.0, 0.0, 10.0, 10.0);
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
        b.content_box = Rect { x: 5.0, y: 6.0, w: 8.0, h: 2.0 };
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
