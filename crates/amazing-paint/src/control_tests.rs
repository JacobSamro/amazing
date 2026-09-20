use super::*;
use crate::controls::{ControlContent, ControlProvider};
use amazing_layout::{LayoutBox, LineBox, Rect};
use amazing_text::{
    FallbackTextSystem, FontMetrics, GlyphRasterRequest, LineRequest, Paragraph, ShapedLine,
    ShapedParagraph, TextError, TextStyle,
};

struct State {
    checked: bool,
    disabled: bool,
    lines: Vec<LineBox>,
}
impl ControlProvider for State {
    fn content(&self, _: amazing_dom::NodeIdx) -> ControlContent<'_> {
        ControlContent {
            checked: self.checked,
            disabled: self.disabled,
            lines: &self.lines,
        }
    }
}
fn state(checked: bool) -> State {
    State {
        checked,
        disabled: false,
        lines: vec![],
    }
}
fn layout(tag: &str, css: &str) -> (LayoutBox, Rect, Rect) {
    let html = format!("<body>{tag}</body>");
    let css = format!(
        "body{{margin:0;font-size:10px;line-height:20px}}input,button,select,textarea{{width:20px;height:20px}}{css}"
    );
    let root =
        amazing_layout::layout_document_with_text(&html, &css, 64.0, &FallbackTextSystem::new());
    let (outer, inner) = build_display_list(&root)
        .into_iter()
        .find_map(|c| match c {
            PaintCommand::PaintControl(c) => Some((c.geometry.border_box, c.geometry.content_box)),
            _ => None,
        })
        .expect("layout should expose an atomic control");
    (root, outer, inner)
}
fn draw(root: &LayoutBox, s: &State) -> Surface {
    paint_document_with_text_resources_and_controls(
        root,
        64,
        64,
        &FallbackTextSystem::new(),
        &NoImages,
        s,
    )
}
fn at(s: &Surface, r: Rect, x: f32, y: f32) -> [u8; 4] {
    s.get_pixel((r.x + x) as u32, (r.y + y) as u32).unwrap()
}
#[test]
fn text_input_has_field_chrome() {
    let (b, r, _) = layout("<input>", "");
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 10.0, 0.0), [100, 100, 100, 255]);
    assert_eq!(at(&s, r, 10.0, 10.0), [255; 4]);
}
#[test]
fn textarea_has_field_chrome() {
    let (b, r, _) = layout("<textarea>not painted without glyph payload</textarea>", "");
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 0.0, 10.0), [100, 100, 100, 255]);
    assert_eq!(at(&s, r, 10.0, 10.0), [255; 4]);
}
#[test]
fn button_has_face_and_waits_for_positioned_label() {
    let (b, r, _) = layout("<button>not synthesized</button>", "");
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 10.0, 10.0), [239, 239, 239, 255]);
    assert_eq!(at(&s, r, 10.0, 0.0), [100, 100, 100, 255]);
}
#[test]
fn select_has_face_and_directional_indicator() {
    let (b, r, _) = layout("<select><option>not synthesized</option></select>", "");
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 13.0, 10.0), [100, 100, 100, 255]);
    assert_eq!(at(&s, r, 6.0, 10.0), [239, 239, 239, 255]);
    let (b, r, _) = layout("<select></select>", "select{direction:rtl}");
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 6.0, 10.0), [100, 100, 100, 255]);
}
#[test]
fn checkbox_mark_comes_only_from_provider_state() {
    let (b, r, _) = layout("<input type=checkbox checked>", "");
    let blank = draw(&b, &state(false));
    let checked = draw(&b, &state(true));
    assert_eq!(at(&blank, r, 8.0, 13.0), [255; 4]);
    assert_eq!(at(&checked, r, 8.0, 13.0), [0, 120, 212, 255]);
    assert_eq!(at(&checked, r, 10.0, 0.0), [100, 100, 100, 255]);
}
#[test]
fn radio_has_round_outline_and_provider_dot() {
    let (b, r, _) = layout("<input type=radio>", "");
    let blank = draw(&b, &state(false));
    let checked = draw(&b, &state(true));
    assert_eq!(at(&blank, r, 0.0, 0.0), [255; 4]);
    assert_eq!(at(&blank, r, 10.0, 0.0), [100, 100, 100, 255]);
    assert_eq!(at(&blank, r, 10.0, 10.0), [255; 4]);
    assert_eq!(at(&checked, r, 10.0, 10.0), [0, 120, 212, 255]);
}
#[test]
fn author_background_border_and_appearance_none_override_chrome() {
    for tag in [
        "<input>",
        "<textarea></textarea>",
        "<button></button>",
        "<select></select>",
        "<input type=checkbox>",
        "<input type=radio>",
    ] {
        let (b, r, _) = layout(
            tag,
            "input,textarea,button,select{background:lime;border:2px solid red;appearance:none}",
        );
        let s = draw(&b, &state(true));
        assert_eq!(at(&s, r, 0.0, 10.0), [255, 0, 0, 255], "{tag}");
        assert_eq!(at(&s, r, 10.0, 10.0), [0, 255, 0, 255], "{tag}");
    }
    let (b, r, _) = layout("<button></button>", "button{background:blue}");
    assert_eq!(
        at(&draw(&b, &state(false)), r, 10.0, 10.0),
        [0, 0, 255, 255]
    );
    let (b, r, _) = layout("<input type=checkbox>", "input{appearance:none}");
    assert_eq!(at(&draw(&b, &state(true)), r, 8.0, 13.0), [255; 4]);
}
#[test]
fn disabled_state_and_accent_color_are_provider_driven() {
    let (b, r, _) = layout("<input type=radio>", "input{accent-color:red}");
    assert_eq!(at(&draw(&b, &state(true)), r, 10.0, 10.0), [255, 0, 0, 255]);
    let mut disabled = state(true);
    disabled.disabled = true;
    assert_eq!(
        at(&draw(&b, &disabled), r, 10.0, 10.0),
        [160, 160, 160, 255]
    );
}
#[test]
fn explicit_author_defaults_suppress_chrome_with_css_provenance() {
    let (a, _, _) = layout("<button></button>", "");
    let (b, _, _) = layout(
        "<button></button>",
        "button{background:transparent;border:none}",
    );
    assert_ne!(
        draw(&a, &state(false)).pixels,
        draw(&b, &state(false)).pixels
    );
    assert!(
        draw(&b, &state(false))
            .pixels
            .chunks_exact(4)
            .all(|p| p == [255; 4])
    );
}
#[test]
fn control_opacity_and_ancestor_clip_use_display_list_groups() {
    let (b, r, _) = layout(
        "<button></button>",
        "button{background:red;appearance:none;opacity:.5}body{height:8px;overflow:hidden}",
    );
    let s = draw(&b, &state(false));
    assert_eq!(at(&s, r, 5.0, 4.0), [255, 128, 128, 255]);
    assert_eq!(at(&s, r, 5.0, 15.0), [255; 4]);
}

struct RasterOnly;
impl TextSystem for RasterOnly {
    fn generation(&self) -> u64 {
        0
    }
    fn metrics(&self, _: &TextStyle) -> Result<FontMetrics, TextError> {
        panic!("paint must not measure")
    }
    fn shape(&self, _: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
        panic!("paint must not shape")
    }
    fn shape_line(&self, _: LineRequest<'_>) -> Result<ShapedLine, TextError> {
        panic!("paint must not shape lines")
    }
    fn rasterize(&self, r: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
        FallbackTextSystem::new().rasterize(r)
    }
}
#[test]
fn positioned_control_labels_use_existing_glyph_ids_and_content_clip() {
    let (b, r, inner) = layout(
        "<button>ignored DOM label</button>",
        "button{appearance:none;background:white;border:2px solid red}",
    );
    let glyph = amazing_layout::GlyphFrag {
        node: amazing_dom::NodeIdx(1),
        font: amazing_text::FontKey(0),
        glyph: amazing_text::GlyphId('Z' as u32),
        cluster: 0..1,
        source_range: 0..1,
        source_mappings: vec![],
        x: inner.x + 2.0,
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
        variations: vec![],
        synthesis: Default::default(),
        color: "black".into(),
    };
    let mut outside = glyph.clone();
    outside.x = inner.x - 7.0;
    let provider = State {
        checked: false,
        disabled: false,
        lines: vec![LineBox {
            y_top: inner.y,
            height: 12.0,
            baseline: inner.y + 10.0,
            words: vec![],
            glyphs: vec![glyph, outside],
            decorations: vec![],
            replaced: vec![],
        }],
    };
    let s = paint_document_with_text_resources_and_controls(
        &b,
        64,
        64,
        &RasterOnly,
        &NoImages,
        &provider,
    );
    assert_eq!(at(&s, inner, 3.0, 5.0), [0, 0, 0, 255]);
    assert_eq!(at(&s, r, 0.0, 6.0), [255, 0, 0, 255]);
}

#[test]
fn positioned_stacking_contexts_remain_above_or_below_controls() {
    for (z, expected) in [(2, [0, 255, 0, 255]), (-1, [239, 239, 239, 255])] {
        let html = format!("<body><div></div><button></button></body>");
        let css = format!(
            "body{{margin:0;position:relative}}button{{width:20px;height:20px}}div{{position:absolute;left:0;top:0;width:64px;height:64px;background:lime;z-index:{z}}}"
        );
        let b = amazing_layout::layout_document_with_text(
            &html,
            &css,
            64.0,
            &FallbackTextSystem::new(),
        );
        let r = build_display_list(&b)
            .into_iter()
            .find_map(|c| match c {
                PaintCommand::PaintControl(c) => Some(c.geometry.border_box),
                _ => None,
            })
            .unwrap();
        assert_eq!(at(&draw(&b, &state(false)), r, 10.0, 10.0), expected);
    }
}

#[test]
fn author_initial_unset_and_rollback_control_native_chrome() {
    let (base, _, _) = layout("<button></button>", "");
    let baseline = draw(&base, &state(false)).pixels;
    for keyword in ["initial", "unset"] {
        let (root, _, _) = layout(
            "<button></button>",
            &format!("button{{background:{keyword};border:{keyword}}}"),
        );
        assert_ne!(draw(&root, &state(false)).pixels, baseline);
    }
    let (rolled_back, _, _) = layout(
        "<button></button>",
        "button{background-color:revert;border-top-style:revert;border-right-style:revert;border-bottom-style:revert;border-left-style:revert}",
    );
    assert!(draw(&rolled_back, &state(false)).pixels == baseline);
    let (inherited, _, _) = layout(
        "<button></button>",
        "body{background:transparent;border:none}",
    );
    assert_eq!(draw(&inherited, &state(false)).pixels, baseline);
}
