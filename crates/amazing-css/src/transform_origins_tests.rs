use super::*;
fn style(css: &str) -> ComputedStyle {
    let mut dom=amazing_dom::Dom::new();
    let node=dom.create_element(amazing_dom::NodeIdx(0), "div");
    crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))]).get(node).unwrap().clone()
}
fn reference(size: [f64;2]) -> OriginReferenceBox { OriginReferenceBox {offset:[0.0;2],size} }
#[test]
fn all_origin_keyword_axes_resolve_against_actual_dimensions() {
    for (css, expected) in [
        ("left",[0.0,150.0,0.0]),("top",[100.0,0.0,0.0]),
        ("center",[100.0,150.0,0.0]),("bottom right",[200.0,300.0,0.0]),
        ("center left",[0.0,150.0,0.0]),("right 40%",[200.0,120.0,0.0]),
        ("-1px bottom",[-1.0,300.0,0.0]),
    ] {
        for property in ["transform-origin","perspective-origin"] {
            let s=style(&format!("{property}:{css}"));
            assert_eq!(used_origin(&s,property,reference([200.0,300.0])),Some(expected),"{property}:{css}");
        }
    }
}
#[test]
fn computed_percentages_remain_symbolic_and_distinct_actual_boxes_are_used() {
    let s=style("transform-origin:10% 20%;perspective-origin:30% 40%");
    assert_eq!(s.get("transform-origin"),Some("10% 20%"));
    assert_eq!(s.get("perspective-origin"),Some("30% 40%"));
    for size in [[200.0,300.0],[37.0,81.0],[0.0,0.0]] {
        assert_eq!(used_origin(&s,"transform-origin",reference(size)),Some([size[0]*0.1,size[1]*0.2,0.0]));
        assert_eq!(used_origin(&s,"perspective-origin",reference(size)),Some([size[0]*0.3,size[1]*0.4,0.0]));
    }
}
#[test]
fn whole_three_dimensional_origin_and_actual_font_math_are_resolved() {
    let s=style("font-size:40px;transform-origin:calc(-100% + 10px - .5em) calc(10px - .5em) calc(10px - .5em);perspective-origin:right calc(10px - .5em) top calc(10px - .5em)");
    assert_eq!(used_origin(&s,"transform-origin",reference([200.0,300.0])),Some([-210.0,-10.0,-10.0]));
    assert_eq!(used_origin(&s,"perspective-origin",reference([200.0,300.0])),Some([210.0,-10.0,0.0]));
    for (css,expected) in [("left center 6px",[0.0,150.0,6.0]),("right bottom -7px",[200.0,300.0,-7.0]),("10px 20px 0",[10.0,20.0,0.0])] {
        let s=style(&format!("transform-origin:{css}"));
        assert_eq!(used_origin(&s,"transform-origin",reference([200.0,300.0])),Some(expected));
    }
}
#[test]
fn actual_reference_offsets_and_piecewise_axes_remain_explicit() {
    let s=style("transform-origin:min(50%,30px) max(10%,40px) 7px;perspective-origin:right 20px bottom 30px");
    let r=OriginReferenceBox {offset:[13.0,17.0],size:[200.0,300.0]};
    assert_eq!(used_origin(&s,"transform-origin",r),Some([43.0,57.0,7.0]));
    assert_eq!(used_origin(&s,"perspective-origin",r),Some([193.0,287.0,0.0]));
}
#[test]
fn bad_geometry_unknown_properties_and_unresolved_context_decline() {
    let s=style("transform-origin:50% 50%");
    for r in [reference([-1.0,30.0]),reference([f64::NAN,20.0]),reference([20.0,f64::INFINITY]),OriginReferenceBox {offset:[f64::NAN,0.0],size:[20.0,30.0]}] {
        assert!(used_origin(&s,"transform-origin",r).is_none());
    }
    assert!(used_origin(&s,"object-position",reference([20.0,30.0])).is_none());
    let s=style("transform-origin:1cqw 2px;perspective-origin:1ex 2px");
    assert!(used_origin(&s,"transform-origin",reference([20.0,30.0])).is_none());
    assert!(used_origin(&s,"perspective-origin",reference([20.0,30.0])).is_none());
    let s=style("transform-origin:1e308% 50%");
    assert!(used_origin(&s,"transform-origin",reference([f64::MAX,30.0])).is_none());
}

#[test]
fn all_contextual_position_units_remain_symbolic_instead_of_invalid_defaulting() {
    for unit in ["cqw","cqh","cqi","cqb","cqmin","cqmax","vi","vb","svi","svb","lvi","lvb","dvi","dvb"] {
        for property in ["object-position","transform-origin","perspective-origin","background-position"] {
            let value = format!("1{unit} 2px");
            let s = style(&format!("{property}:{value}"));
            assert_eq!(s.get(property), Some(value.as_str()), "{property}: {value}");
            if matches!(property,"transform-origin"|"perspective-origin") {
                assert!(used_origin(&s,property,reference([200.0,300.0])).is_none(), "{property}: {value}");
            }
        }
    }
}
#[test]
fn contextual_position_math_and_end_edges_preserve_real_context_requirements() {
    for unit in ["cqw","cqh","cqi","cqb","cqmin","cqmax","vi","vb","svi","svb","lvi","lvb","dvi","dvb"] {
        for property in ["transform-origin","perspective-origin"] {
            // Transforms1's transform-origin grammar admits XY plus optional Z,
            // while perspective-origin admits the generic four-token position.
            let edge = format!("right 1{unit} bottom 2px");
            let equivalent = format!("calc(100% - 1{unit}) calc(100% - 2px)");
            let end = if property == "transform-origin" { equivalent } else { edge.clone() };
            for value in [format!("calc(1{unit} + 10%) 2px"),end] {
                let s = style(&format!("{property}:{value}"));
                assert!(s.get(property).unwrap().contains(unit), "{property}: {value}");
                assert!(used_origin(&s,property,reference([200.0,300.0])).is_none(), "{property}: {value}");
            }
            assert_eq!(crate::values::parses(property,&edge),property == "perspective-origin", "{property}: {edge}");
        }
    }
}
#[test]
fn contextual_position_admission_keeps_invalid_dimension_and_arity_rejection() {
    for property in ["object-position","transform-origin","perspective-origin","background-position"] {
        for value in ["1unknown 2px","1cqw 2deg","calc(1cqw + 1) 2px","calc(1cqw + 1deg) 2px","1cqw 2px 3px 4px 5px"] {
            assert!(!crate::values::parses(property,value), "{property}: {value}");
        }
    }
}
