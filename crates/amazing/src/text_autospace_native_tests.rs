//! Autospace through the real page provider and installed Layout consumer.
//! Expected spacing is derived from the real requested owner's ic metric.
use amazing_layout::LayoutBox;
use amazing_text::{TextStyle, TextSystem};

fn sample(html: &str, rules: &str) -> LayoutBox {
    let css = format!("body{{margin:0;width:max-content;font:20px/30px serif;white-space:pre}}{rules}");
    let text = crate::page::load_page_text(&css, "file:///autospace-native.html");
    amazing_layout::layout_document_with_text(&format!("<body lang=ja>{html}</body>"), &css,
        800.0, &text.provider)
}
fn ic() -> f32 {
    let text = crate::page::load_page_text("", "file:///autospace-native.html");
    let mut style = TextStyle::default();
    style.size = 20.0; style.families = vec!["serif".into()]; style.language = Some("ja".into());
    text.provider.font_relative_metrics(&style).ic_advance
}
fn near(a: f32, b: f32) { assert!((a - b).abs() < 0.05, "{a} != {b}"); }
fn advance(b: &LayoutBox) -> f32 {
    b.lines.iter().flat_map(|line| &line.glyphs)
        .map(|glyph| glyph.advance_x.abs().max(glyph.advance_y.abs())).sum()
}

#[test]
fn installed_normal_spacing_uses_innermost_common_owner_metric() {
    for html in ["漢A", "<span style='font-size:40px'>漢</span><span style='font-size:30px'>A</span>"] {
        let none = sample(html, "body{text-autospace:no-autospace}");
        let normal = sample(html, "body{text-autospace:normal}");
        near(advance(&normal) - advance(&none), ic() / 8.0);
        near(normal.content_box.w - none.content_box.w, ic() / 8.0);
        assert_eq!(normal.lines[0].glyphs.len(), none.lines[0].glyphs.len());
    }
}

#[test]
fn zero_scope_edges_are_transparent_and_real_edges_exclude_spacing() {
    for edges in ["padding:0;margin:0;border:0", "margin-right:9px", "margin-right:-9px", "padding-right:9px"] {
        let html = format!("<span style='{edges}'>漢</span><span>A</span>");
        let none = sample(&html, "body{text-autospace:no-autospace}");
        let normal = sample(&html, "body{text-autospace:normal}");
        let expected = if edges == "padding:0;margin:0;border:0" {ic()/8.0} else {0.0};
        near(normal.content_box.w - none.content_box.w, expected);
    }
}

#[test]
fn replacement_preserves_source_and_drops_space_word_spacing() {
    let replaced = sample("漢 A", "body{text-autospace:ideograph-alpha replace;word-spacing:9px}");
    let adjacent = sample("漢A", "body{text-autospace:ideograph-alpha insert;word-spacing:9px}");
    near(replaced.content_box.w, adjacent.content_box.w);
    near(advance(&replaced), advance(&adjacent));
    assert!(replaced.lines[0].words.iter().any(|word| word.text.contains('A')));
    assert!(replaced.lines[0].glyphs.iter().any(|glyph| glyph.source_range == (4..5)));
    // The surviving A still addresses the original "漢 A" source bytes.
}

#[test]
fn forced_lines_do_not_create_cross_paragraph_spacing() {
    for html in ["漢<br>A", "漢\nA"] {
        let normal = sample(html, "body{text-autospace:normal}");
        let none = sample(html, "body{text-autospace:no-autospace}");
        assert_eq!(normal.lines.len(), 2); assert_eq!(none.lines.len(), 2);
        near(advance(&normal), advance(&none));
    }
}

#[test]
fn actual_ruby_base_installs_policy_and_combined_upright_is_atomic() {
    let none = sample("<ruby>漢A<rt>x</rt></ruby>", "body{text-autospace:no-autospace}");
    let normal = sample("<ruby>漢A<rt>x</rt></ruby>", "body{text-autospace:normal}");
    near(normal.content_box.w - none.content_box.w, ic()/8.0);
    let combined = sample("<span>12</span>", "body{writing-mode:vertical-rl}span{text-combine-upright:all;text-autospace:normal}");
    let plain = sample("<span>12</span>", "body{writing-mode:vertical-rl}span{text-combine-upright:all;text-autospace:no-autospace}");
    near(advance(&combined), advance(&plain));
}

#[test]
fn policy_only_span_changes_preserve_real_latin_ligatures() {
    let split = sample("<span style='text-autospace:no-autospace'>f</span><span style='text-autospace:normal'>i</span>", "");
    let plain = sample("fi", "body{text-autospace:no-autospace}");
    assert_eq!(split.lines[0].glyphs.len(), plain.lines[0].glyphs.len());
    for (a,b) in split.lines[0].glyphs.iter().zip(&plain.lines[0].glyphs) {
        assert_eq!(a.font,b.font); assert_eq!(a.glyph,b.glyph); near(a.advance_x,b.advance_x);
    }
}

#[test]
fn french_replacement_keeps_the_original_nonbreaking_punctuation_boundary() {
    let b = sample("<span lang=fr>Bonjour :</span>",
        "body{width:1px;white-space:normal;text-autospace:punctuation replace}");
    assert_eq!(b.lines.len(), 1, "an original ASCII-space break must not split the nonbreaking replacement");
}
