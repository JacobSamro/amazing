use amazing_layout::font::FontSource;

fn render(html: &str, width: u32, height: u32) -> amazing_paint::Surface {
    let fonts = FontSource::fallback();
    let layout = amazing_layout::layout_document(html, "", width as f32, &fonts);
    amazing_paint::paint_document(&layout, width, height, &fonts)
}

#[test]
fn positioned_siblings_paint_in_z_index_order() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green;z-index:2"></div>
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:red;z-index:1"></div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn nested_stacking_context_is_atomic_against_a_higher_sibling() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green;z-index:2"></div>
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:red;z-index:1">
            <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:blue;z-index:999"></div>
          </div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn fixed_auto_stacking_context_is_atomic_against_a_positive_sibling() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div style="position:fixed;left:0;top:0;width:20px;height:20px;background:red">
            <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:blue;z-index:999"></div>
          </div>
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green;z-index:1"></div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn invalid_later_z_index_does_not_override_valid_z_index() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green;z-index:2;z-index:banana"></div>
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:red;z-index:1"></div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn opacity_composites_the_whole_box_group_once() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:20px;height:20px;background:black;opacity:0.5"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([128, 128, 128, 255]));
}

#[test]
fn translate_transform_moves_the_painted_subtree() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;background:green;transform:translate(20px,10px)"></div></body>"#,
        40,
        30,
    );
    assert_eq!(surface.get_pixel(1, 1), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(21, 11), Some([0, 128, 0, 255]));
}

#[test]
fn border_radius_shapes_the_background_corner() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:20px;height:20px;background:green;border-radius:10px"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(0, 0), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn zero_blur_box_shadow_paints_behind_the_box() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;background:#ff9900;box-shadow:5px 5px black"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(5, 5), Some([255, 153, 0, 255]));
    assert_eq!(surface.get_pixel(12, 12), Some([0, 0, 0, 255]));
}

#[test]
fn background_clip_content_box_excludes_padding() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;padding:5px;background:green;background-clip:content-box"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(2, 2), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(7, 7), Some([0, 128, 0, 255]));
}

#[test]
fn rounded_overflow_clips_descendant_paint() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:20px;height:20px;border-radius:10px;overflow:hidden"><div style="width:20px;height:20px;background:red"></div></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(0, 0), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(10, 10), Some([255, 0, 0, 255]));
}

#[test]
fn inset_shadow_paints_inside_the_padding_edge() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:20px;height:20px;background:blue;box-shadow:inset 5px 0 orange"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(2, 10), Some([255, 165, 0, 255]));
    assert_eq!(surface.get_pixel(10, 10), Some([0, 0, 255, 255]));
}

#[test]
fn double_border_leaves_the_middle_third_for_the_background() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;background:green;border:6px double black"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(0, 10), Some([0, 0, 0, 255]));
    assert_eq!(surface.get_pixel(2, 10), Some([0, 128, 0, 255]));
    assert_eq!(surface.get_pixel(4, 10), Some([0, 0, 0, 255]));
}

#[test]
fn deeply_nested_effect_groups_fail_closed() {
    let mut html = "<body style=\"margin:0\">".to_owned();
    for _ in 0..20 {
        html.push_str("<div style=\"opacity:0.99\">");
    }
    html.push_str("<div style=\"width:10px;height:10px;background:green\"></div>");
    for _ in 0..20 {
        html.push_str("</div>");
    }
    html.push_str("</body>");

    let surface = render(&html, 20, 20);
    assert_eq!(surface.get_pixel(5, 5), Some([255, 255, 255, 255]));
}

#[test]
fn positive_descendant_escapes_a_non_context_ancestor() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div><div style="position:absolute;left:0;top:0;width:20px;height:20px;background:red;z-index:999"></div></div>
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green;z-index:2"></div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([255, 0, 0, 255]));
}

#[test]
fn positioned_auto_paints_after_later_in_flow_content() {
    let surface = render(
        r#"<body style="margin:0;position:relative">
          <div style="position:absolute;left:0;top:0;width:20px;height:20px;background:green"></div>
          <div style="width:20px;height:20px;background:red"></div>
        </body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn positioned_auto_replays_non_context_ancestor_clip() {
    let surface = render(
        r#"<body style="margin:0"><div style="position:relative;width:10px;height:10px;overflow:clip"><div style="position:absolute;left:0;top:0;width:20px;height:20px;background:red"></div></div></body>"#,
        30,
        20,
    );
    assert_eq!(surface.get_pixel(5, 5), Some([255, 0, 0, 255]));
    assert_eq!(surface.get_pixel(15, 5), Some([255, 255, 255, 255]));
}

#[test]
fn overflow_x_longhand_clips_descendant_paint() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;overflow-x:clip"><div style="width:20px;height:20px;background:red"></div></div></body>"#,
        30,
        20,
    );
    assert_eq!(surface.get_pixel(5, 5), Some([255, 0, 0, 255]));
    assert_eq!(surface.get_pixel(15, 5), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(5, 15), Some([255, 0, 0, 255]));
}

#[test]
fn hidden_overflow_couples_visible_cross_axis_to_auto() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;overflow-x:hidden"><div style="width:20px;height:20px;background:red"></div></div></body>"#,
        30,
        20,
    );
    assert_eq!(surface.get_pixel(5, 5), Some([255, 0, 0, 255]));
    assert_eq!(surface.get_pixel(15, 5), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(5, 15), Some([255, 255, 255, 255]));
}

#[test]
fn percentage_opacity_composites_as_a_number() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;background:black;opacity:50%"></div></body>"#,
        20,
        20,
    );
    assert_eq!(surface.get_pixel(5, 5), Some([128, 128, 128, 255]));
}

#[test]
fn deeply_nested_rounded_clips_fail_closed() {
    let mut html = "<body style=\"margin:0\">".to_owned();
    for _ in 0..20 {
        html.push_str("<div style=\"border-radius:2px;overflow:hidden\">");
    }
    html.push_str("<div style=\"width:10px;height:10px;background:green\"></div>");
    for _ in 0..20 {
        html.push_str("</div>");
    }
    html.push_str("</body>");
    let surface = render(&html, 20, 20);
    assert_eq!(surface.get_pixel(5, 5), Some([255, 255, 255, 255]));
}

#[test]
fn variable_radius_expands_before_paint() {
    let surface = render(
        r#"<body style="margin:0"><div style="--r:10px;width:20px;height:20px;background:green;border-radius:var(--r)"></div></body>"#,
        30,
        30,
    );
    assert_eq!(surface.get_pixel(0, 0), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(10, 10), Some([0, 128, 0, 255]));
}

#[test]
fn invalid_later_transform_does_not_override_valid_transform() {
    let surface = render(
        r#"<body style="margin:0"><div style="width:10px;height:10px;background:green;transform:translate(20px);transform:translate(1px,)"></div></body>"#,
        40,
        20,
    );
    assert_eq!(surface.get_pixel(1, 1), Some([255, 255, 255, 255]));
    assert_eq!(surface.get_pixel(21, 1), Some([0, 128, 0, 255]));
}
