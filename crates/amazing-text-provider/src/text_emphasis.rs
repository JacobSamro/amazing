//! CSS Text Decoration Module Level 3's `text-emphasis-style` mark glyphs.
//!
//! `text-emphasis-style` computes to a `[filled | open] || [dot | circle |
//! double-circle | triangle | sesame]` pair (either half may be omitted --
//! `filled` defaults the shape to `circle`, `dot` defaults the fill to
//! `filled`) or the keyword `none`, or an author `<string>` (of which only
//! the first grapheme cluster is used, which the caller can do directly and
//! needs no helper here). This module only supplies the fixed
//! keyword-pair -> mark-character table (CSS Text Decoration 3 section 4's
//! "Suggested Mark Rendering" values, which every engine uses verbatim);
//! computing the mark's actual on-glyph position and painting it is layout
//! and paint's job; see the routing note in `.agents/layout/questions.md`
//! and `.agents/paint/questions.md` (TEXT-23) for the rest of the feature.

/// Resolves an already-normalized, lowercase, space-separated
/// `text-emphasis-style` computed value (e.g. `"filled circle"`, `"open"`,
/// `"dot"`, `"sesame"`, or `"none"`) to its mark character. Returns `None`
/// for `"none"` (no mark) or for a value this table does not recognize
/// (e.g. because the caller passed the `<string>` form, which has no table
/// entry and is used as-is).
pub fn text_emphasis_mark(computed_style: &str) -> Option<char> {
    let mut filled = None;
    let mut shape = None;
    for keyword in computed_style.split_whitespace() {
        match keyword {
            "none" => return None,
            "filled" => filled = Some(true),
            "open" => filled = Some(false),
            "dot" | "circle" | "double-circle" | "triangle" | "sesame" => shape = Some(keyword),
            _ => return None,
        }
    }
    let filled = filled.unwrap_or(true);
    match (shape.unwrap_or("circle"), filled) {
        ("dot", true) => Some('\u{2022}'),          // BULLET
        ("dot", false) => Some('\u{25E6}'),          // WHITE BULLET
        ("circle", true) => Some('\u{25CF}'),        // BLACK CIRCLE
        ("circle", false) => Some('\u{25CB}'),       // WHITE CIRCLE
        ("double-circle", true) => Some('\u{25C9}'), // FISHEYE
        ("double-circle", false) => Some('\u{25CE}'), // BULLSEYE
        ("triangle", true) => Some('\u{25B2}'),      // BLACK UP-POINTING TRIANGLE
        ("triangle", false) => Some('\u{25B3}'),     // WHITE UP-POINTING TRIANGLE
        ("sesame", true) => Some('\u{FE45}'),        // SESAME DOT
        ("sesame", false) => Some('\u{FE46}'),       // WHITE SESAME DOT
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_mark() {
        assert_eq!(text_emphasis_mark("none"), None);
    }

    #[test]
    fn a_bare_shape_defaults_to_filled() {
        assert_eq!(text_emphasis_mark("dot"), Some('\u{2022}'));
        assert_eq!(text_emphasis_mark("sesame"), Some('\u{FE45}'));
    }

    #[test]
    fn a_bare_fill_defaults_to_circle() {
        assert_eq!(text_emphasis_mark("filled"), Some('\u{25CF}'));
        assert_eq!(text_emphasis_mark("open"), Some('\u{25CB}'));
    }

    #[test]
    fn every_fill_and_shape_combination_has_a_distinct_mark() {
        let combos = [
            ("filled dot", '\u{2022}'),
            ("open dot", '\u{25E6}'),
            ("filled circle", '\u{25CF}'),
            ("open circle", '\u{25CB}'),
            ("filled double-circle", '\u{25C9}'),
            ("open double-circle", '\u{25CE}'),
            ("filled triangle", '\u{25B2}'),
            ("open triangle", '\u{25B3}'),
            ("filled sesame", '\u{FE45}'),
            ("open sesame", '\u{FE46}'),
        ];
        for (style, expected) in combos {
            assert_eq!(text_emphasis_mark(style), Some(expected), "style {style}");
        }
    }

    #[test]
    fn order_of_the_two_keywords_does_not_matter() {
        assert_eq!(text_emphasis_mark("circle filled"), text_emphasis_mark("filled circle"));
    }

    #[test]
    fn an_unrecognized_value_such_as_a_string_form_has_no_table_entry() {
        assert_eq!(text_emphasis_mark("\"x\""), None);
    }
}
