//! Line-relative alignment after bidi reshaping, before physical column placement.
use crate::{GlyphFrag, InlineDecorationFrag, ReplacedFrag, WordFrag};
use amazing_text::{Direction, WritingMode};
use std::collections::HashSet;

/// One paragraph's computed indentation, resolved against its own inline size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Indentation {
    amount: f32,
    hanging: bool,
    each_line: bool,
    first_eligible: bool,
}
impl Default for Indentation {
    fn default() -> Self { Self { amount: 0.0, hanging: false, each_line: false, first_eligible: true } }
}
impl Indentation {
    pub(crate) fn from_style(style: &amazing_css::ComputedStyle, inline_size: f32, first_eligible: bool, intrinsic: bool) -> Self {
        Self::from_style_with_units(style, inline_size, first_eligible, intrinsic, None)
    }
    pub(crate) fn from_style_with_units(style: &amazing_css::ComputedStyle, inline_size: f32,
        first_eligible: bool, intrinsic: bool, units: Option<crate::values::FontRelativeUnits>) -> Self {
        let Some(policy) = style.get("text-indent").and_then(amazing_css::TextIndent::parse) else { return Self { first_eligible, ..Self::default() }; };
        let zoom = style.effective_zoom();
        let basis = if intrinsic {0.0} else {inline_size};
        let amount = amazing_css::used_text_indent(style, f64::from(basis) / zoom, intrinsic)
            .map(|n| (n * zoom) as f32)
            .or_else(|| units.and_then(|units| crate::values::used_length_percentage_with_units(style, policy.length_css(), units)
                .and_then(|length| length.resolve(Some(basis)))))
            .filter(|n| n.is_finite()).unwrap_or(0.0);
        Self { amount, hanging: policy.hanging, each_line: policy.each_line, first_eligible }
    }
    pub(crate) fn first_line_eligible(self) -> bool { self.first_eligible }
    pub(crate) fn is_zero(self) -> bool { self.amount == 0.0 }

    pub(crate) fn for_line(self, first: bool, follows_forced_break: bool) -> f32 {
        let selected = first && (self.first_eligible || self.each_line) || self.each_line && follows_forced_break;
        if selected != self.hanging { self.amount } else { 0.0 }
    }
}

pub(super) struct Alignment<'a> {
    pub value: &'a str,
    pub last: &'a str,
    /// CSS Text 3 `text-justify`: `"none"` disables justification outright,
    /// regardless of what `value`/`last` resolve to -- any left-over space
    /// stays unfilled, same as a non-justified alignment.
    pub justify: &'a str,
    pub final_line: bool,
    pub direction: Direction,
    pub writing_mode: WritingMode,
    pub start: f32,
    pub end: f32,
    pub used_end: f32,
    /// Fitting starts on the physical low edge. Reversed logical lines move
    /// this margin to their actual high/start edge after bidi reshaping.
    pub indentation: f32,
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
    // CSS Text 3 s7.4: `text-justify: none` disables justification outright.
    // Any left-over space at the end of the line stays unfilled -- treated
    // the same as a non-justified `start` alignment, not stretched.
    if a.justify.eq_ignore_ascii_case("none") && matches!(value, "justify" | "justify-all") {
        value = "start";
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
    let indent_shift = if reversed { -a.indentation } else { 0.0 };
    let offset = |position: f32| {
        indent_shift + if opportunities.is_empty() {
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
    fn text_justify_none_disables_stretching_from_text_align_last() {
        // CSS Text 3 s7.4 (WPT text-justify-none-001.html's own property
        // combo): `text-justify: none` disables justification outright,
        // regardless of what text-align/text-align-last resolve to. A line
        // that would otherwise stretch under `text-align-last: justify`
        // instead keeps its natural, unstretched position -- same as no
        // justification requested at all.
        let baseline = layout("x x", "body{}");
        let last = baseline.lines[0].glyphs.last().unwrap().x;
        let stretched = layout("x x", "body{text-align-last:justify}");
        assert!(stretched.lines[0].glyphs.last().unwrap().x > last);
        let disabled = layout("x x", "body{text-align-last:justify;text-justify:none}");
        near(disabled.lines[0].glyphs.last().unwrap().x, last);
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

    fn indent_layout(html: &str, css: &str) -> LayoutBox {
        layout_document_with_text(&format!("<body>{html}</body>"),
            &format!("body{{margin:0;width:100px;height:100px;font:20px/20px serif}}{css}"),
            500.0, &amazing_text::FallbackTextSystem::new())
    }
    fn inline_bounds(line: &LineBox, vertical: bool) -> (f32,f32) {
        line.glyphs.iter().fold((f32::INFINITY,f32::NEG_INFINITY), |(start,end),g| {
            let origin = if vertical {g.y-g.offset_y} else {g.x-g.offset_x};
            let advance = g.advance_x.abs().max(g.advance_y.abs());
            (start.min(origin),end.max(origin+advance))
        })
    }
    #[test]
    fn whole_indentation_first_forced_and_soft_lines_remain_distinct() {
        for (value, first, forced, wrapped) in [
            ("30px",30.,0.,0.), ("30px each-line",30.,30.,0.),
            ("30px hanging",0.,30.,30.), ("30px hanging each-line",0.,0.,30.),
        ] {
            let b=indent_layout("A<br>BBBB BBBB BBBB",&format!("body{{text-indent:{value}}}"));
            assert!(b.lines.len()>=3,"{value}");
            near(inline_bounds(&b.lines[0],false).0,first);
            near(inline_bounds(&b.lines[1],false).0,forced);
            near(inline_bounds(&b.lines[2],false).0,wrapped);
        }
    }
    #[test]
    fn signed_percentage_font_and_zoom_use_the_actual_paragraph_context() {
        for (css, expected) in [
            ("width:200px;text-indent:20%",40.),
            ("width:200px;text-indent:-20%",-40.),
            ("width:200px;text-indent:calc(2em + 10%)",60.),
            ("width:100px;zoom:2;text-indent:10px",20.),
            ("width:100px;zoom:2;text-indent:25%",50.),
        ] {
            let b=indent_layout("A",&format!("body{{{css}}}"));
            near(inline_bounds(&b.lines[0],false).0,b.content_box.x+expected);
        }
        let b=indent_layout("A","html{font-size:30px}body{font-size:10px;text-indent:2rem}");
        near(inline_bounds(&b.lines[0],false).0,60.);
    }
    #[test]
    fn actual_anonymous_continuations_do_not_restart_first_formatted_line() {
        for (value, first, continuation) in [("20px",20.,0.),("20px hanging",0.,20.),("20px each-line",20.,20.),("20px hanging each-line",0.,0.)] {
            let b=indent_layout("A<div>inside</div>B",&format!("body{{text-indent:{value}}}"));
            let Flow::Blocks(children)=&b.flow else {panic!("real anonymous blocks required")};
            let runs=children.iter().filter(|child| child.node.is_none() && matches!(child.flow,Flow::Inlines(_))).collect::<Vec<_>>();
            assert_eq!(runs.len(),2);
            near(inline_bounds(&runs[0].lines[0],false).0,first);
            near(inline_bounds(&runs[1].lines[0],false).0,continuation);
        }
    }
    #[test]
    fn vertical_and_sideways_line_start_keep_the_actual_inline_basis() {
        for mode in ["vertical-rl","vertical-lr","sideways-rl"] {
            let b=indent_layout("AB",&format!("body{{width:200px;height:100px;writing-mode:{mode};text-indent:25%}}"));
            near(inline_bounds(&b.lines[0],true).0,25.);
        }
        let b=indent_layout("AB","body{writing-mode:sideways-lr;text-indent:20px}");
        near(inline_bounds(&b.lines[0],true).1,80.);
    }
    #[test]
    fn float_exclusions_keep_signed_indent_and_own_percentage_basis() {
        for (value,expected) in [("20px",60.),("-20px",20.),("50%",115.)] {
            let b=indent_layout("<div style='float:left;width:40px;height:40px'></div>A<br>B",
                &format!("body{{width:150px;text-indent:{value}}}"));
            let Flow::Blocks(children)=&b.flow else {panic!("float and anonymous inline run")};
            let run=children.iter().find(|child| matches!(child.flow,Flow::Inlines(_)) && !child.lines.is_empty()).unwrap();
            near(inline_bounds(&run.lines[0],false).0,expected);
            near(inline_bounds(&run.lines[1],false).0,40.);
        }
    }
    #[test]
    fn installed_atomic_inline_participates_in_indented_fitting_and_placement() {
        let b=indent_layout("<span style='display:inline-block;width:30px;height:20px'></span>","body{text-indent:20px}");
        assert_eq!(b.lines[0].replaced.len(),1);
        near(b.lines[0].replaced[0].x,20.);
    }
    #[test]
    fn intrinsic_percentage_zero_and_signed_absolute_contributions_are_distinct() {
        let plain=indent_layout("AAAA","body{width:max-content;height:auto}");
        let percent=indent_layout("AAAA","body{width:max-content;height:auto;text-indent:50%}");
        let positive=indent_layout("AAAA","body{width:max-content;height:auto;text-indent:20px}");
        let negative=indent_layout("AAAA","body{width:max-content;height:auto;text-indent:-20px}");
        near(percent.content_box.w,plain.content_box.w);
        near(positive.content_box.w,plain.content_box.w+20.);
        near(negative.content_box.w,(plain.content_box.w-20.).max(0.));
        let plain=indent_layout("AAAA AAAA","body{width:min-content;height:auto}");
        let negative=indent_layout("AAAA AAAA","body{width:min-content;height:auto;text-indent:-20px}");
        near(negative.content_box.w,plain.content_box.w);
    }

    #[test]
    fn provider_font_units_and_real_line_heights_reach_installed_paragraph_geometry() {
        use amazing_text::TextSystem;
        let provider=amazing_text::FallbackTextSystem::new();
        let mut requested=amazing_text::TextStyle::default(); requested.size=20.;
        let metrics=provider.font_relative_metrics(&requested);
        for (css,expected) in [("2ch",2.*metrics.ch_advance),("2ex",2.*metrics.x_height),
            ("2cap",2.*metrics.cap_height),("2ic",2.*metrics.ic_advance),("1lh",20.),
            ("calc(2ch + 10%)",2.*metrics.ch_advance+10.)] {
            let b=indent_layout("A",&format!("body{{text-indent:{css}}}"));
            near(inline_bounds(&b.lines[0],false).0,expected);
        }
    }
    #[test]
    fn non_atomic_margins_use_signed_percentage_and_font_context_without_painting() {
        for (value, expected) in [("10px",10.),("-10px",-10.),("25%",50.),("calc(10% - 10px)",10.),("1em",20.),("auto",0.)] {
            let b=indent_layout(&format!("<span style='margin-left:{value};background:yellow'>A</span>"),"body{width:200px}");
            near(inline_bounds(&b.lines[0],false).0,expected);
            let left=b.lines[0].decorations.iter().filter(|d|d.color!="transparent")
                .map(|d|d.x).fold(f32::INFINITY,f32::min);
            near(left,expected);
        }
    }
    #[test]
    fn nested_margin_and_padding_edges_keep_their_distinct_background_boxes() {
        let b=indent_layout("<span style='margin-left:7px;padding-left:3px'><span style='margin-left:10px;padding:4px;background:yellow'>A</span></span>","");
        near(inline_bounds(&b.lines[0],false).0,24.);
        let left=b.lines[0].decorations.iter().filter(|d|d.color=="yellow").map(|d|d.x).fold(f32::INFINITY,f32::min);
        near(left,20.);
    }
    #[test]
    fn closing_empty_and_generated_margin_edges_advance_following_real_content() {
        for html in ["<span style='margin-left:10px;margin-right:15px'></span>A",
            "<span style='margin-left:10px'><span style='margin-right:15px'></span></span>A"] {
            let b=indent_layout(html,"");near(inline_bounds(&b.lines[0],false).0,25.);
        }
        let b=indent_layout("<span></span>A","span::before{content:'B';margin-left:10px;margin-right:15px}");
        near(inline_bounds(&b.lines[0],false).0,10.);
        assert_eq!(b.lines[0].glyphs.len(),2);
        let first=&b.lines[0].glyphs[0];let second=&b.lines[0].glyphs[1];
        near(second.x-second.offset_x,first.x-first.offset_x+first.advance_x.abs()+15.);
    }
    #[test]
    fn scope_start_margin_is_not_repeated_by_soft_wraps_or_hanging_indentation() {
        for (value,first,wrapped) in [("20px",30.,0.),("20px hanging",10.,20.)] {
            let b=indent_layout("<span style='margin-left:10px'>AAAA AAAA AAAA AAAA</span>",&format!("body{{text-indent:{value}}}"));
            assert!(b.lines.len()>=2);
            near(inline_bounds(&b.lines[0],false).0,first);
            near(inline_bounds(&b.lines[1],false).0,wrapped);
        }
    }
    #[test]
    fn inline_axis_margins_leave_non_replaced_cross_axis_line_metrics_unchanged() {
        let plain=indent_layout("AB","body{writing-mode:vertical-rl}");
        let b=indent_layout("<span style='margin-top:15px;margin-left:999px'>AB</span>","body{writing-mode:vertical-rl}");
        near(inline_bounds(&b.lines[0],true).0,15.);
        near(b.lines[0].height,plain.lines[0].height+15.);
        let plain=indent_layout("AB","");
        let b=indent_layout("<span style='margin-top:999px;margin-bottom:-999px'>AB</span>","");
        near(b.lines[0].height,plain.lines[0].height);
        near(inline_bounds(&b.lines[0],false).0,0.);
    }
    #[test]
    fn intrinsic_inline_margins_keep_cyclic_percentages_zero_and_absolute_signs() {
        let plain=indent_layout("<span>AAAA</span>","body{width:max-content;height:auto}");
        for (margin,extra) in [("50%",0.),("10px",10.),("-10px",-10.)] {
            let b=indent_layout(&format!("<span style='margin-left:{margin}'>AAAA</span>"),"body{width:max-content;height:auto}");
            near(b.content_box.w,(plain.content_box.w+extra).max(0.));
        }
    }

    #[test]
    fn empty_non_replaced_margin_scope_retains_strut_without_margin_height() {
        let b=indent_layout("<span style='margin-top:999px;margin-bottom:-999px'></span>","body{height:auto}");
        assert_eq!(b.lines.len(),1);
        assert!(b.lines[0].glyphs.is_empty());
        near(b.lines[0].height,20.);
        near(b.content_box.h,20.);
    }

    #[test]
    fn cross_axis_inline_margins_do_not_create_struts_around_block_descendants() {
        fn all_lines<'a>(box_: &'a LayoutBox) -> Vec<&'a LineBox> {
            let mut lines = box_.lines.iter().collect::<Vec<_>>();
            if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
                for child in children { lines.extend(all_lines(child)); }
            }
            lines
        }
        for contain in ["none", "layout", "paint"] {
            let reference = indent_layout("<div style='margin:1em 0'>A</div>", "body{height:auto}");
            let actual = indent_layout(&format!("<span style='margin:1em 0;contain:{contain}'><div style='margin:1em 0'>A</div></span>"), "body{height:auto}");
            let reference_lines = all_lines(&reference);
            let actual_lines = all_lines(&actual);
            assert_eq!(actual_lines.len(), reference_lines.len(), "{contain}");
            assert_eq!(actual.content_box.h, reference.content_box.h, "{contain}");
            assert_eq!(actual_lines[0].y_top, reference_lines[0].y_top, "{contain}");
        }
    }

}
