use super::*;
use crate::{compute,parse_stylesheet,ComputedStyle};
use amazing_dom::{Dom,NodeIdx};

#[test]
fn precise_absolute_alpha_is_never_quantized_before_transparency() {
    for function in ["rgb(1 2 3 / 0.000000000000001)","hsl(0 50% 50% / 1e-15)",
        "hwb(0 20% 30% / 1e-15)","lab(50 10 20 / 1e-15)","lch(50 10 20 / 1e-15)",
        "oklab(.5 .1 .2 / 1e-15)","oklch(.5 .1 20 / 1e-15)","color(display-p3 .1 .2 .3 / 1e-15)"] {
        let color=ColorValue::parse(function).unwrap();assert_eq!(color.alpha,Some(1e-15),"{function}");
        assert!(!color.serialize().ends_with(" / 0)"));
    }
    for function in ["transparent","rgb(none 2 3 / none)","lab(50 10 20 / 0)","color(srgb 1 0 0 / -2)"] {
        assert_eq!(ColorValue::parse(function).unwrap().alpha.unwrap_or(0.0),0.0);
    }
}
#[test]
fn relative_coordinates_and_alpha_keep_actual_source_precision() {
    let source=ColorValue::parse("rgb(10.125 20.25 30.5 / 1e-15)").unwrap();
    for function in ["rgb","hsl","hwb","lab","lch","oklab","oklch"] {
        let channels=match function {"rgb"=>"r g b","hsl"=>"h s l","hwb"=>"h w b", "lab"|"oklab"=>"l a b",_=>"l c h"};
        let css=format!("{function}(from currentcolor {channels} / calc(alpha * 2))");
        assert_eq!(ColorValue::resolve(&css,Some(&source),false).unwrap().alpha,Some(2e-15),"{css}");
    }
    let color=ColorValue::resolve("rgb(from currentcolor r g b)",Some(&source),false).unwrap();
    assert_eq!(color.channels,source.channels);assert_eq!(color.alpha,source.alpha);
    for space in ["srgb","srgb-linear","display-p3","a98-rgb","rec2020","prophoto-rgb"] {
        let css=format!("color(from currentcolor {space} r g b / alpha)");
        assert_eq!(ColorValue::resolve(&css,Some(&source),false).unwrap().alpha,source.alpha);
    }
    assert!(ColorValue::resolve("rgb(from currentcolor r g b)",None,false).is_none());
    let missing=ColorValue::resolve("rgb(from rgb(none 20 30 / none) r g b / alpha)",None,false).unwrap();
    assert_eq!(missing.channels[0],None);assert_eq!(missing.alpha,None);
    let outside=ColorValue::resolve("hsl(from color(srgb 2 3 4 / 1e-15) h s l / alpha)",None,false).unwrap();
    assert!(outside.channels[1].unwrap()>=0.0);assert_eq!(outside.alpha,Some(1e-15));
    let coords=advanced::coordinates::convert(&outside.space,"srgb",outside.channels.map(|n|n.unwrap())).unwrap();
    for (actual,expected) in coords.into_iter().zip([2.0,3.0,4.0]) {assert!((actual-expected).abs()<1e-10,"{coords:?}");}

}
#[test]
fn winning_variables_inheritance_currentcolor_and_scheme_are_precise() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
    let parent=dom.create_element(html,"div");let child=dom.create_element(parent,"span");
    let styles=compute(&dom,&[parse_stylesheet("div{--tiny:rgb(10 20 30 / 1e-15);color:var(--tiny);background-color:var(--tiny)}span{background-color:inherit}")]);
    let child_style=styles.get(child).unwrap();assert!(!child_style.has_transparent_background());
    assert!(child_style.resolved_color_value("color").unwrap().contains("0.000000000000001"));
    for css in ["background-color:currentcolor","background-color:rgb(from currentcolor r g b / alpha)",
        "color-scheme:dark;background-color:light-dark(transparent,rgb(1 2 3 / 1e-15))",
        "background-color:rgb(from rgb(1 2 3 / 1e-15) r g b / alpha)"] {
        let styles=compute(&dom,&[parse_stylesheet(&format!("div{{color:rgb(1 2 3 / 1e-15)}}span{{{css}}}"))]);
        assert!(!styles.get(child).unwrap().has_transparent_background(),"{css}");
    }
    let styles=compute(&dom,&[parse_stylesheet("span{color-scheme:light;background-color:light-dark(transparent,rgb(1 2 3 / 1e-15))}")]);
    assert!(styles.get(child).unwrap().has_transparent_background());
}
#[test]
fn mutation_clone_and_every_image_layer_preserve_conservative_transparency() {
    let mut dom=Dom::new();let node=dom.create_element(NodeIdx(0),"div");
    let styles=compute(&dom,&[]);let mut style=styles.get(node).unwrap().clone();
    assert!(style.has_transparent_background());
    for image in ["none","none, none","NONE,none, none"] {style.set("background-image",image);assert!(style.has_transparent_background(),"{image}");}
    for image in ["url(a.png)","none, url(a.png)","linear-gradient(transparent, transparent)","none,",""] {
        style.set("background-image",image);assert!(!style.has_transparent_background(),"{image}");
    }
    style.set("background-image","none");style.set("background-color","currentcolor");
    style.set("color","rgb(1 2 3 / 1e-15)");assert!(!style.has_transparent_background());
    let retained=style.clone();style.set("color","transparent");assert!(style.has_transparent_background());assert!(!retained.has_transparent_background());
    for css in ["CanvasText","var(--unknown)","rgb(from currentcolor missing g b)","unrecognized"] {
        style.set("background-color",css);assert!(!style.has_transparent_background(),"{css}");
    }
    assert!(!ComputedStyle::default().has_transparent_background());
}

#[test]
fn mixes_keep_tiny_alpha_missing_components_and_weight_normalization() {
    let current=ColorValue::parse("rgb(10.125 20.25 30.5 / 1e-15)").unwrap();
    for space in ["srgb","srgb-linear","display-p3","lab","oklab","lch","oklch","hsl","hwb"] {
        let css=format!("color-mix(in {space}, currentcolor, rgb(from currentcolor r g b / alpha))");
        assert_eq!(ColorValue::resolve(&css,Some(&current),false).unwrap().alpha,Some(1e-15),"{css}");
    }
    let value=ColorValue::resolve("color-mix(in srgb, rgb(none 0 0 / none), rgb(255 0 0 / 1e-15) 20%)",None,false).unwrap();
    assert_eq!(value.alpha,Some(1e-15));assert_eq!(value.channels[0],Some(1.0));
    let value=ColorValue::resolve("color-mix(in srgb, red 10%, blue 10%)",None,false).unwrap();assert_eq!(value.alpha,Some(0.2));
}

#[test]
fn actual_shadow_pseudo_and_layer_winners_share_precise_context() {
    use crate::{MediaEnvironment,compute_document_with_shadow_font_metrics,compute_document_pseudo_with_shadow_font_metrics};
    use crate::font_metric_context::FontMetricContext;
    use crate::selectors::PseudoElement;
    use crate::font_size_adjust::FontSizeAdjustMetric;
    use amazing_dom::shadow::{ShadowOptions,ShadowMode,SlotAssignment};
    struct Metrics;
    impl FontMetricContext for Metrics {
        fn aspect_ratio(&self,_:NodeIdx,_:Option<PseudoElement>,_:&ComputedStyle,_:FontSizeAdjustMetric)->Option<f64> {None}
    }
    let mut dom=Dom::new();let host=dom.create_element(NodeIdx(0),"section");
    let root=dom.attach_shadow(host,ShadowOptions {mode:ShadowMode::Closed,slot_assignment:SlotAssignment::Named,delegates_focus:false,clonable:false,serializable:false}).unwrap();
    let inner=dom.create_element(root,"span");
    let sheets=[parse_stylesheet("section{color:rgb(1 2 3 / 1e-15)}span{color:red;background-color:red}")];
    let scoped=[(root,vec![parse_stylesheet("@layer first,second;@layer first{span{background-color:red}}@layer second{span{background-color:currentcolor}span::before{background-color:rgb(from currentcolor r g b / alpha)}}")])];
    let styles=compute_document_with_shadow_font_metrics(&dom,NodeIdx(0),&sheets,&scoped,&MediaEnvironment::default(),None,&Metrics);
    assert!(!styles.get(inner).unwrap().has_transparent_background());
    assert!(styles.get(inner).unwrap().resolved_color_value("background-color").unwrap().contains("0.000000000000001"));
    let pseudo=compute_document_pseudo_with_shadow_font_metrics(&dom,NodeIdx(0),inner,PseudoElement::Before,&sheets,&scoped,&MediaEnvironment::default(),None,&Metrics).unwrap();
    assert!(!pseudo.has_transparent_background());
    let mut dom=Dom::new();let node=dom.create_element(NodeIdx(0),"div");
    let styles=compute(&dom,&[parse_stylesheet("div{color:rgb(from currentcolor r g b / 1e-15);background-color:currentcolor}")]);
    assert!(!styles.get(node).unwrap().has_transparent_background());
}
