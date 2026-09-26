//! Line-relative alignment after bidi reshaping, before physical column placement.
use crate::{GlyphFrag, InlineDecorationFrag, ReplacedFrag, WordFrag};
use amazing_text::{Direction, WritingMode};
use std::collections::HashSet;

pub(super) struct Alignment<'a> {
    pub value: &'a str,
    pub last: &'a str,
    pub final_line: bool,
    pub direction: Direction,
    pub writing_mode: WritingMode,
    pub start: f32,
    pub end: f32,
    pub used_end: f32,
}

pub(super) fn apply(
    a: Alignment<'_>,
    words: &mut [WordFrag],
    glyphs: &mut [GlyphFrag],
    decorations: &mut [InlineDecorationFrag],
    replaced: &mut [ReplacedFrag],
    spaces: &HashSet<(u32, usize)>,
) {
    let vertical = a.writing_mode.is_vertical_flow();
    let reversed =
        (a.direction == Direction::RightToLeft) ^ (a.writing_mode == WritingMode::SidewaysLr);
    let start_offset = |free: f32| if reversed { free } else { 0.0 };
    let mut value = a.value;
    if a.final_line {
        value = match a.last {
            "auto" if matches!(value, "justify" | "justify-all") => {
                if value == "justify-all" {
                    "justify"
                } else {
                    "start"
                }
            }
            "auto" => value,
            last => last,
        };
    }
    let free = a.end - a.used_end;
    let mut opportunities = Vec::new();
    if free > 0.0 && matches!(value, "justify" | "justify-all") {
        for glyph in glyphs.iter() {
            if spaces.contains(&(glyph.node.0, glyph.cluster.start)) {
                let origin = if vertical {
                    glyph.y - glyph.offset_y
                } else {
                    glyph.x - glyph.offset_x
                };
                let advance = glyph.advance_x.abs().max(glyph.advance_y.abs());
                let end = origin + advance;
                if origin > a.start && end < a.used_end {
                    opportunities.push(end);
                }
            }
        }
        opportunities.sort_by(f32::total_cmp);
        opportunities.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    }
    let shift = if free < 0.0 {
        start_offset(free)
    } else {
        match value {
            "center" => free * 0.5,
            "end" => free - start_offset(free),
            "left" => {
                if a.writing_mode == WritingMode::SidewaysLr {
                    free
                } else {
                    0.0
                }
            }
            "right" => {
                if a.writing_mode == WritingMode::SidewaysLr {
                    0.0
                } else {
                    free
                }
            }
            _ => start_offset(free),
        }
    };
    let expansion = if opportunities.is_empty() {
        0.0
    } else {
        free / opportunities.len() as f32
    };
    let offset = |position: f32| {
        if opportunities.is_empty() {
            shift
        } else {
            opportunities.partition_point(|edge| *edge <= position + 0.001) as f32 * expansion
        }
    };
    for glyph in glyphs {
        if vertical {
            glyph.y += offset(glyph.y - glyph.offset_y);
        } else {
            glyph.x += offset(glyph.x - glyph.offset_x);
        }
    }
    for word in words {
        word.x += offset(word.x);
    }
    for fragment in replaced {
        if vertical {
            fragment.y += offset(fragment.y);
        } else {
            fragment.x += offset(fragment.x);
        }
    }
    for decoration in decorations {
        if vertical {
            let before = offset(decoration.y);
            decoration.height += offset(decoration.y + decoration.height) - before;
            decoration.y += before;
        } else {
            let before = offset(decoration.x);
            decoration.width += offset(decoration.x + decoration.width) - before;
            decoration.x += before;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    fn layout(html: &str, css: &str) -> LayoutBox {
        layout_document_with_text(
            &format!("<body>{html}</body>"),
            &format!("body{{margin:0;width:100px;height:100px;font:20px/20px serif}}{css}"),
            500.0,
            &amazing_text::FallbackTextSystem::new(),
        )
    }
    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 0.05, "{a} != {b}");
    }
    #[test]
    fn horizontal_alignment_uses_free_inline_space() {
        for (value, expected) in [
            ("left", 0.),
            ("right", 78.),
            ("center", 39.),
            ("start", 0.),
            ("end", 78.),
        ] {
            let b = layout("xx", &format!("body{{text-align:{value}}}"));
            near(b.lines[0].glyphs[0].x, expected);
        }
    }
    #[test]
    fn rtl_start_end_are_resolved_against_container_direction() {
        for (value, expected) in [("start", 78.), ("end", 0.), ("left", 0.), ("right", 78.)] {
            let b = layout("xx", &format!("body{{direction:rtl;text-align:{value}}}"));
            near(
                b.lines[0]
                    .glyphs
                    .iter()
                    .map(|g| g.x)
                    .fold(f32::INFINITY, f32::min),
                expected,
            );
        }
    }
    #[test]
    fn vertical_alignment_moves_along_the_inline_axis() {
        for mode in ["vertical-rl", "vertical-lr"] {
            for (align, expected) in [("left", 0.), ("right", 78.), ("center", 39.)] {
                let b = layout(
                    "xx",
                    &format!(
                        "body{{writing-mode:{mode};text-orientation:upright;text-align:{align}}}"
                    ),
                );
                let g = &b.lines[0].glyphs[0];
                near(g.y - g.offset_y, expected);
            }
        }
    }
    #[test]
    fn oversized_line_overflows_at_inline_end() {
        let b = layout("xxxxxxxxxx", "body{white-space:nowrap;text-align:center}");
        near(b.lines[0].glyphs[0].x, 0.);
    }
    #[test]
    fn final_line_alignment_and_forced_breaks_do_not_justify_by_default() {
        let b = layout("x x<br>x x", "body{text-align:justify}");
        near(b.lines[0].glyphs[0].x, 0.);
        near(b.lines[1].glyphs[0].x, 0.);
        near(b.lines[0].glyphs.last().unwrap().x, 17.);
        let b = layout("xx", "body{text-align:left;text-align-last:right}");
        near(b.lines[0].glyphs[0].x, 78.);
    }
    #[test]
    fn explicit_last_line_justification_distributes_interword_space() {
        let b = layout("x x", "body{text-align-last:justify}");
        near(b.lines[0].glyphs[0].x, 0.);
        near(b.lines[0].glyphs.last().unwrap().x, 89.);
    }
    #[test]
    fn atomic_vertical_advance_is_height_and_cross_extent_is_width() {
        for mode in ["vertical-rl", "vertical-lr"] {
            let b = layout(
                "<span></span><span></span>",
                &format!(
                    "body{{writing-mode:{mode}}}span{{display:inline-block;width:40px;height:20px}}"
                ),
            );
            assert_eq!(b.lines.len(), 1);
            assert_eq!(b.lines[0].replaced.len(), 2);
            let first = &b.lines[0].replaced[0];
            let second = &b.lines[0].replaced[1];
            near(first.y, 0.);
            near(second.y, 20.);
            near(first.x, if mode == "vertical-rl" { 60. } else { 0. });
            near(second.x, first.x);
        }
    }
    #[test]
    fn vertical_atomic_wrap_uses_height_not_width() {
        let b = layout(
            "<span></span><span></span>",
            "body{writing-mode:vertical-rl;height:30px}span{display:inline-block;width:40px;height:20px}",
        );
        assert_eq!(b.lines.len(), 2);
        near(b.lines[0].replaced[0].x, 60.);
        near(b.lines[1].replaced[0].x, 20.);
        near(b.lines[1].replaced[0].y, 0.);
    }
    #[test]
    fn inline_child_font_does_not_replace_parent_line_strut() {
        let b = layout("<span>x</span>", "span{font-size:10px}");
        near(b.lines[0].height, 20.);
    }
    #[test]
    fn vertical_auto_extent_remains_stable_when_shaped_again() {
        let mut b = layout(
            "<span>xx<br>xx</span>",
            "span{display:inline-block;writing-mode:vertical-rl}",
        );
        let child = &b.positioned[0];
        near(child.content_box.w, 40.);
        near(child.content_box.h, 22.);
        assert_eq!(child.lines.len(), 2);
        crate::apply_shaped_text(&mut b, &amazing_text::FallbackTextSystem::new());
        near(b.positioned[0].content_box.h, 22.);
        near(b.positioned[0].content_box.w, 40.);
    }
    #[test]
    fn orthogonal_available_max_height_does_not_make_percent_height_definite() {
        let b = layout(
            "<main><div>漢 漢 漢 漢</div></main>",
            "main{max-height:20px}div{writing-mode:vertical-rl}",
        );
        let Flow::Blocks(outer) = &b.flow else {
            panic!("blocks")
        };
        let Flow::Blocks(children) = &outer[0].flow else {
            panic!("children")
        };
        let child = &children[0];
        assert_eq!(child.containing_block_height, None);
        assert_eq!(child.available_height, Some(20.));
        assert_eq!(child.lines.len(), 4);
        near(child.content_box.h, 20.);
    }
}
