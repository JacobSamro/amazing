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
//! - text: each `WordFrag` drawn per character via [`crate::text`] with
//!   advances from [`FontSource::measure`], ink sitting on the line's
//!   baseline; fragment color is the containing block's `color`
//! - everything clips to the viewport surface (`Surface::fill_rect`
//!   saturating-cast clipping; glyphs clip per pixel)
//! - flex children paint in flow order exactly like block children
//!
//! # Deferred / deviations
//!
//! - ALL border styles (dashed/dotted/double/groove/…) render as SOLID —
//!   deliberate Phase 4 simplification.
//! - No border-radius, no box shadows, no images/replaced content, no
//!   list markers, no opacity, no stacking contexts/z-index reordering,
//!   no overflow clipping beyond the viewport.
//! - Word-level color differences inside one line are not tracked by the
//!   layout tree (`WordFrag` carries no style), so all words of a line
//!   use the container's color.
//! - The canvas starts opaque white regardless of root background
//!   propagation rules.

use amazing_css::ComputedStyle;
use amazing_layout::font::FontSource;
use amazing_layout::{Flow, LayoutBox};

use crate::color::parse_css_color;
use crate::surface::Surface;
use crate::text::{draw_fragment, GlyphProvider, RectInkProvider};

const CANVAS_WHITE: [u8; 4] = [255, 255, 255, 255];
/// Used when the `color` property exists but does not parse: the CSS
/// initial value of `color` is black.
const FALLBACK_TEXT_COLOR: [u8; 4] = [0, 0, 0, 255];

/// Paints `root` into a fresh `viewport_w × viewport_h` surface using the
/// default rect-ink glyph provider.
pub fn paint_document(
    root: &LayoutBox,
    viewport_w: u32,
    viewport_h: u32,
    fonts: &FontSource,
) -> Surface {
    paint_document_with(root, viewport_w, viewport_h, fonts, &RectInkProvider)
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
    let mut surface = Surface::new(viewport_w, viewport_h);
    surface.clear(CANVAS_WHITE);
    paint_box(&mut surface, root, fonts, glyphs);
    surface
}

fn paint_box(
    surface: &mut Surface,
    box_: &LayoutBox,
    fonts: &FontSource,
    glyphs: &dyn GlyphProvider,
) {
    let text_color = resolve_text_color(&box_.style);
    paint_background(surface, box_, text_color);
    paint_borders(surface, box_, text_color);
    paint_lines(surface, box_, text_color, fonts, glyphs);

    // Children come AFTER this box's own painting (painter's algorithm).
    match &box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) => {
            for child in children {
                paint_box(surface, child, fonts, glyphs);
            }
        },
        Flow::Inlines(_) | Flow::Empty => {},
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
fn paint_background(surface: &mut Surface, b: &LayoutBox, text_color: [u8; 4]) {
    let bg = match resolve_color(&b.style, "background-color", text_color) {
        Some(c) => c,
        None => return,
    };
    let cb = &b.content_box;
    let p = &b.padding;
    surface.fill_rect(
        cb.x - p.left,
        cb.y - p.top,
        cb.w + p.left + p.right,
        cb.h + p.top + p.bottom,
        bg,
    );
}

/// Four solid strips inside the border edges. Top/bottom span the full
/// width; left/right sit between them so corners are painted once.
/// Sides whose style is none/hidden or whose resolved width is <= 0 are
/// skipped defensively (layout already zeroes them).
fn paint_borders(surface: &mut Surface, b: &LayoutBox, text_color: [u8; 4]) {
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
        surface.fill_rect(bb.x, bb.y, bb.w, bt, color_of("top"));
    }
    if bbw > 0.0 {
        surface.fill_rect(bb.x, bb.y + bb.h - bbw, bb.w, bbw, color_of("bottom"));
    }
    if bl > 0.0 {
        surface.fill_rect(bb.x, bb.y + bt, bl, bb.h - bt - bbw, color_of("left"));
    }
    if br > 0.0 {
        surface.fill_rect(bb.x + bb.w - br, bb.y + bt, br, bb.h - bt - bbw, color_of("right"));
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
) {
    for line in &b.lines {
        for w in &line.words {
            draw_fragment(
                surface,
                w.x,
                line.baseline,
                &w.text,
                w.font_size,
                text_color,
                fonts,
                glyphs,
            );
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
