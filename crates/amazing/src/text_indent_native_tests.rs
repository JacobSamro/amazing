//! Text indentation through the installed provider's Unicode bidi resolution.
//! The original Hebrew input and 80px assertion are unchanged; the intentionally
//! non-bidi FallbackTextSystem fixture is preserved on source e63c6e8c.
use amazing_layout::{LayoutBox, LineBox};

fn indent_layout(html: &str, css: &str) -> LayoutBox {
    amazing_layout::layout_document_with_text(&format!("<body>{html}</body>"),
        &format!("body{{margin:0;width:100px;height:100px;font:20px/20px serif}}{css}"),
        500.0, &amazing_text_provider::TextProvider::new())
}
fn near(a: f32, b: f32) { assert!((a-b).abs() < 0.05, "{a} != {b}"); }
fn inline_bounds(line: &LineBox, vertical: bool) -> (f32,f32) {
    line.glyphs.iter().fold((f32::INFINITY,f32::NEG_INFINITY), |(start,end),g| {
        let origin=if vertical {g.y-g.offset_y} else {g.x-g.offset_x};
        let advance=g.advance_x.abs().max(g.advance_y.abs());
        (start.min(origin),end.max(origin+advance))
    })
}

#[test]
fn logical_indentation_follows_actual_rtl_and_plaintext_line_direction() {
    for (html,css) in [("AB","direction:rtl"),("אב","direction:ltr;unicode-bidi:plaintext")] {
        let b=indent_layout(html,&format!("body{{{css};text-indent:20px}}"));
        near(inline_bounds(&b.lines[0],false).1,b.content_box.x+b.content_box.w-20.);
    }
    for (direction,expected) in [("ltr",60.),("rtl",40.)] {
        let b=indent_layout("AB",&format!("body{{direction:{direction};text-align:center;text-indent:20px}}"));
        let (start,end)=inline_bounds(&b.lines[0],false);
        near((start+end)*0.5,expected);
    }
}

#[test]
fn actual_bidi_provider_places_inline_scope_margins_at_logical_start() {
    for (html, css, property) in [("AB", "direction:rtl", "margin-inline-start"),
        ("אב", "direction:ltr;unicode-bidi:plaintext", "margin-inline-end")] {
        // Logical properties map through the element's specified direction;
        // plaintext changes paragraph alignment, not that property mapping.
        let b=indent_layout(&format!("<span style='{property}:20px;background:yellow'>{html}</span>"),&format!("body{{{css}}}"));
        near(inline_bounds(&b.lines[0],false).1,b.content_box.x+b.content_box.w-20.);
    }
}

#[test]
fn authentic_justified_indentation_and_scope_margin_reference_have_equal_geometry() {
    let text="This is a long piece of text that will wrap to multiple lines. ".repeat(12);
    let html=format!("<span style='background:yellow'>{text}</span>");
    let indented=indent_layout(&html,"body{width:400px;height:auto;text-align:justify;text-indent:100px}");
    let margined=indent_layout(&html,"body{width:400px;height:auto;text-align:justify}span{margin-left:100px}");
    assert_eq!(indented.lines.len(),margined.lines.len());
    for (a,b) in indented.lines.iter().zip(&margined.lines) {
        assert_eq!(a.glyphs.len(),b.glyphs.len());
        for (a,b) in a.glyphs.iter().zip(&b.glyphs) {near(a.x,b.x);near(a.y,b.y);}
        assert_eq!(a.decorations.len(),b.decorations.len());
        for (a,b) in a.decorations.iter().zip(&b.decorations) {near(a.x,b.x);near(a.width,b.width);}
    }
}
