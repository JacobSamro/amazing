use super::*;
use crate::transform_matrix;
fn matrix(css: &str, size: Option<[f64; 2]>, origin: [f64; 3]) -> [[f64; 4]; 4] {
    transform_matrix(&parse(css).unwrap(), size, origin).unwrap()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}
fn style(css: &str) -> crate::ComputedStyle {
    let mut dom = amazing_dom::Dom::new();
    let node = dom.create_element(amazing_dom::NodeIdx(0), "div");
    crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))])
        .get(node)
        .unwrap()
        .clone()
}
#[test]
fn whole_function_grammar_preserves_all_function_families() {
    for css in [
        "none",
        "translate(1px)",
        "translate(-10%,2em)",
        "translateX(0)",
        "translateY(3px)",
        "translateZ(-4px)",
        "translate3d(10%,20%,3px)",
        "scale(250%)",
        "scale(-500%, -620%)",
        "scale(1,200%)",
        "scaleX(720%)",
        "scaleY(-85%)",
        "scaleZ(25%)",
        "scale3d(-50%,250%,-300%)",
        "rotate(0)",
        "rotateX(1turn)",
        "rotateY(-200grad)",
        "rotateZ(2rad)",
        "rotate3d(1,2,3,45deg)",
        "rotate3d(0,0,0,90deg)",
        "skew(0)",
        "skew(90deg,0)",
        "skewX(0deg)",
        "skewY(-90deg)",
        "matrix(1,0,0,1,10,20)",
        "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,10,20,30,1)",
        "perspective(10px)",
        "perspective(0)",
        "perspective(none)",
        "translate(1px)rotate(90deg)scale(2)",
        "translate(calc(10% + 2px), min(5px,10%))",
        "scale(calc(200%),3)",
        "rotate(calc(1turn / 2))",
        "perspective(calc(-1px))",
        "translate(1px",
    ] {
        assert!(parse(css).is_some(), "{css}");
        assert!(crate::values::parses("transform", css), "{css}");
    }
}
#[test]
fn every_family_rejects_wrong_dimension_range_arity_and_separator() {
    for css in [
        "",
        "none scale(1)",
        "translate()",
        "translate(1)",
        "translate(1px 2px)",
        "translate(1px,2px,3px)",
        "translateZ(2%)",
        "translate3d(1px,2px,3%)",
        "translate3d(1px,2px,calc(1% + 1px))",
        "scale(2px)",
        "scale(1 2)",
        "scale3d(1,2)",
        "scaleZ(1,2)",
        "rotate(1)",
        "rotate(20%)",
        "rotate3d(1%,0,0,1deg)",
        "rotate3d(1,0,0)",
        "skew(1px)",
        "skew(0,0,0)",
        "matrix(1,0,0,1,0)",
        "matrix(1%,0,0,1,0,0)",
        "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,1,2,3)",
        "perspective(-1px)",
        "perspective(1%)",
        "perspective(1)",
        "perspective(none,1px)",
        "bogus(1)",
        "scale(1),rotate(1deg)",
        "scale(1))",
        "scale(1,,2)",
    ] {
        assert!(parse(css).is_none(), "{css}");
    }
}
#[test]
fn typed_percent_scale_and_three_dimensional_translations_keep_bases() {
    assert!(
        matches!(parse("scale3d(50%,200%,-300%)").unwrap()[0],Transform::Scale3D([a,b,c]) if a==0.5&&b==2.0&&c==-3.0)
    );
    let m = matrix("translate3d(10%,20%,30px)", Some([200.0, 100.0]), [0.0; 3]);
    close(m[0][3], 20.0);
    close(m[1][3], 20.0);
    close(m[2][3], 30.0);
    assert!(transform_matrix(&parse("translateX(10%)").unwrap(), None, [0.0; 3]).is_none());
    assert!(
        transform_matrix(
            &parse("translateX(2em)").unwrap(),
            Some([200.0, 100.0]),
            [0.0; 3]
        )
        .is_none()
    );
}
#[test]
fn css_matrix_lists_are_column_major_and_functions_post_multiply() {
    let m = matrix(
        "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,10,20,30,1)",
        None,
        [0.0; 3],
    );
    close(m[0][3], 10.0);
    close(m[1][3], 20.0);
    close(m[2][3], 30.0);
    let m = matrix("translateX(10px) scale(2)", None, [0.0; 3]);
    close(m[0][0], 2.0);
    close(m[0][3], 10.0);
    let reversed = matrix("scale(2) translateX(10px)", None, [0.0; 3]);
    close(reversed[0][3], 20.0);
    let m = matrix("matrix(1,2,3,4,5,6)", None, [0.0; 3]);
    close(m[0][1], 3.0);
    close(m[1][0], 2.0);
    close(m[0][3], 5.0);
    close(m[1][3], 6.0);
}
#[test]
fn rotation_normalizes_axes_and_zero_axis_is_identity() {
    let m = matrix("rotate3d(0,0,100,90deg)", None, [0.0; 3]);
    close(m[0][0], 0.0);
    close(m[1][0], 1.0);
    close(m[0][1], -1.0);
    let m = matrix("rotateX(90deg)", None, [0.0; 3]);
    close(m[1][2], -1.0);
    close(m[2][1], 1.0);
    let m = matrix("rotateY(90deg)", None, [0.0; 3]);
    close(m[0][2], 1.0);
    close(m[2][0], -1.0);
    let m = matrix("rotate3d(0,0,0,30deg)", None, [0.0; 3]);
    assert_eq!(m, matrix("none", None, [0.0; 3]));
    let m = matrix("rotate3d(1e300,0,0,90deg)", None, [0.0; 3]);
    close(m[1][2], -1.0);
}
#[test]
fn origin_is_three_dimensional_and_skew_is_one_matrix() {
    let m = matrix("rotate(90deg)", Some([100.0, 100.0]), [50.0, 50.0, 0.0]);
    close(m[0][3], 100.0);
    close(m[1][3], 0.0);
    let m = matrix("scaleZ(2)", None, [0.0, 0.0, 10.0]);
    close(m[2][3], -10.0);
    let m = matrix("skew(45deg,45deg)", None, [0.0; 3]);
    close(m[0][0], 1.0);
    close(m[0][1], 1.0);
    close(m[1][0], 1.0);
}
#[test]
fn perspective_none_identity_and_subpixel_used_depth_are_distinct() {
    let identity = matrix("none", None, [0.0; 3]);
    assert_eq!(matrix("perspective(none)", None, [0.0; 3]), identity);
    let m = matrix("perspective(100px)", None, [0.0; 3]);
    close(m[3][2], -0.01);
    for css in [
        "perspective(0px)",
        "perspective(.5px)",
        "perspective(calc(-1px))",
    ] {
        close(matrix(css, None, [0.0; 3])[3][2], -1.0);
    }
    assert!(!parse("perspective(none)").unwrap().is_empty());
}
#[test]
fn computed_font_math_retains_reference_box_percentages() {
    let s = style("font-size:40px;transform:translate(calc(10% + .5em),1em) scale3d(50%,200%,1)");
    let Some(crate::values::ComputedValue::Transform(ops)) = s.typed("transform") else {
        panic!("typed transform lost")
    };
    let m = transform_matrix(ops, Some([200.0, 100.0]), [0.0; 3]).unwrap();
    close(m[0][3], 40.0);
    close(m[1][3], 40.0);
    close(m[0][0], 0.5);
    close(m[1][1], 2.0);
    assert!(s.get("transform").unwrap().contains("10%"));
}
#[test]
fn cascade_substitution_inheritance_and_invalid_winner_use_same_grammar() {
    let mut dom = amazing_dom::Dom::new();
    let parent = dom.create_element(amazing_dom::NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    let sheet = crate::parse_stylesheet(
        "div{--move:translateZ(2em) scaleZ(50%);font-size:20px;transform:var(--move)}span{transform:inherit}",
    );
    let styles = crate::compute(&dom, &[sheet]);
    assert_eq!(
        styles.get(parent).unwrap().typed("transform"),
        styles.get(child).unwrap().typed("transform")
    );
    assert!(crate::parse_stylesheet("@supports (transform:perspective(none)) and (transform:scale3d(50%,200%,3)){div{color:green}}").rules.len()>0);
    let s = style("transform:scaleZ(2);--bad:translateZ(10%);transform:var(--bad)");
    assert_eq!(s.get("transform"), Some("none"));
}
#[test]
fn unavailable_reference_context_and_nonfinite_handoff_are_declined() {
    assert!(transform_matrix(&parse("scale(2)").unwrap(), Some([-1.0, 2.0]), [0.0; 3]).is_none());
    assert!(transform_matrix(&parse("scale(2)").unwrap(), None, [f64::NAN, 0.0, 0.0]).is_none());
    assert!(transform_matrix(&[Transform::Scale(f64::INFINITY, 1.0)], None, [0.0; 3]).is_none());
    assert!(
        crate::animation::ResolvedTransform::from_css("translateZ(2px)", [100.0, 100.0]).is_none()
    );
    for (axis, radians) in [([f64::NAN, 0.0, 0.0], 0.0), ([0.0; 3], f64::INFINITY)] {
        assert!(
            transform_matrix(
                &[Transform::Rotate3D(axis, Angle { radians })],
                None,
                [0.0; 3]
            )
            .is_none()
        );
    }
}

#[test]
fn computed_angle_text_preserves_degree_values_across_every_transform_family() {
    for (function, args, expected) in [
        ("rotate", "30deg", "30deg"),
        ("rotateX", "30deg", "30deg"),
        ("rotateY", "-30deg", "-30deg"),
        ("rotateZ", "720.25deg", "720.25deg"),
        ("rotate3d", "1, 2, 3, 30deg", "1, 2, 3, 30deg"),
        ("skew", "30deg, -30deg", "30deg, -30deg"),
        ("skewX", "30deg", "30deg"),
        ("skewY", "-0.125deg", "-0.125deg"),
    ] {
        let authored = format!("{function}({args})");
        let expected = format!("{function}({expected})");
        let computed = style(&format!("transform:{authored}"));
        assert_eq!(computed.get("transform"), Some(expected.as_str()), "{authored}");
        let typed = parse(computed.get("transform").unwrap()).unwrap();
        let authored_typed = parse(&authored).unwrap();
        let a = transform_matrix(&typed, None, [0.0; 3]).unwrap();
        let b = transform_matrix(&authored_typed, None, [0.0; 3]).unwrap();
        for row in 0..4 { for col in 0..4 { close(a[row][col], b[row][col]); } }
    }
}
#[test]
fn all_angle_units_and_math_use_a_degree_basis_without_relaxing_dimensions() {
    for (value, expected) in [
        ("30deg", 30.0), ("100grad", 90.0), (".5turn", 180.0),
        ("1rad", 1.0_f64.to_degrees()), ("0", 0.0), ("/**/0", 0.0),
        ("calc(1turn / 12 + 10grad)", 39.0),
        ("calc(30deg + .25turn - 10grad)", 111.0),
        ("min(30deg, 100grad)", 30.0),
        ("max(-30deg, -.25turn)", -30.0),
        ("clamp(10deg, .25turn, 30deg)", 30.0),
    ] {
        assert_eq!(crate::math::computed_angle_degrees(value), Some(expected), "{value}");
        let expected = format!("rotate({expected}deg)");
        assert_eq!(style(&format!("transform:rotate({value})")).get("transform"), Some(expected.as_str()), "{value}");
    }
    for invalid in ["1", "30%", "1px", "calc(30deg + 1)", "calc(30deg + 1px)",
                    "calc(1turn / 0)", "calc(1turn * 1deg)", "1e999deg"] {
        assert_eq!(crate::math::computed_angle_degrees(invalid), None, "{invalid}");
    }
}
#[test]
fn individual_rotation_composes_the_same_angle_unit_and_math_serializer() {
    for (value, expected) in [("30deg", "30deg"), ("x 100grad", "x 90deg"),
                              ("y .5turn", "y 180deg"),
                              ("calc(1turn / 12 + 10grad)", "39deg")] {
        assert_eq!(style(&format!("rotate:{value}")).get("rotate"), Some(expected), "{value}");
    }
}
