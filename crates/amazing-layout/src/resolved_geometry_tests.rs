//! Actual DOM geometry and applicability for the CSSOM query seam.
use crate::*;
use amazing_css::selectors::PseudoElement;
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_text::FallbackTextSystem;
fn near(actual:f32, expected:f32) { assert!((actual-expected).abs()<0.01,"{actual} != {expected}"); }
fn node(dom:&Dom,id:&str)->NodeIdx {
    (0..dom.node_count()).map(|n|NodeIdx(n as u32)).find(|&n|dom.attrs_of(n)
        .is_some_and(|a|a.iter().any(|(k,v)|k=="id"&&v==id))).unwrap()
}
struct Images<'a>(&'a Dom);
impl ReplacedElementProvider for Images<'_> {
    fn intrinsic_size(&self,node:NodeIdx)->Option<IntrinsicSize> {
        (self.0.name(node)==Some("img")).then_some(IntrinsicSize { width:20.0,height:10.0 })
    }
}
fn layout(html:&str,css:&str)->(Dom,LayoutBox) {
    let dom=amazing_html::parse(&format!("<body>{html}"));
    let root=layout_dom_document_with_text_and_replaced_in_viewport(&dom,
        &format!("body{{margin:0;font:10px/10px serif}}{css}"),300.0,Some(200.0),
        &FallbackTextSystem::new(),&Images(&dom));
    (dom,root)
}
fn geometry(root:&LayoutBox,dom:&Dom,id:&str)->ResolvedStyleGeometry {
    resolved_style_geometry(root,LayoutSource::Element(node(dom,id))).unwrap_or_else(||panic!("missing geometry {id}"))
}
#[test]
fn resolved_absolute_axes_capture_used_equations_and_computed_losers() {
    for direction in ["ltr","rtl"] { for position in ["absolute","fixed"] {
        for (size,margins,expected_over) in [("width:40px;height:20px","margin:0",true),
            ("width:auto;height:auto","margin:0",false),
            ("width:40px;height:20px","margin:auto",false),
            ("width:auto;min-width:400px;height:auto;min-height:300px","margin:0",true)] {
            let (dom,root)=layout("<div id='cb'><div id='target'></div></div>",&format!(
                "html{{direction:{direction}}}#cb{{position:relative;width:180px;height:100px;padding:10px;direction:{direction}}}#target{{position:{position};left:10%;right:20px;top:5px;bottom:10px;{size};{margins}}}"));
            let g=geometry(&root,&dom,"target");let cb=g.containing_block;
            near(g.border_box.x-g.margin.left.unwrap()-cb.x,g.insets.left.unwrap());
            near(cb.x+cb.w-g.border_box.x-g.border_box.w-g.margin.right.unwrap(),g.insets.right.unwrap());
            near(g.border_box.y-g.margin.top.unwrap()-cb.y,g.insets.top.unwrap());
            near(cb.y+cb.h-g.border_box.y-g.border_box.h-g.margin.bottom.unwrap(),g.insets.bottom.unwrap());
            assert_eq!(g.overconstrained.left,expected_over&&direction=="rtl");
            assert_eq!(g.overconstrained.right,expected_over&&direction=="ltr");
            assert_eq!(g.overconstrained.bottom,expected_over);assert!(!g.overconstrained.top);
            near(cb.w,if position=="fixed"{300.0}else{200.0});
            near(cb.h,if position=="fixed"{200.0}else{120.0});
        }
    }}
}
#[test]
fn resolved_relative_and_static_inset_applicability_is_explicit() {
    for direction in ["ltr","rtl"] {
        let (dom,root)=layout("<div id='cb'><div id='target'></div><div id='static'></div><div id='sticky'></div></div>",&format!(
            "#cb{{width:150px;height:80px;direction:{direction}}}#target{{position:relative;left:7px;right:10px;top:20%;bottom:3px;width:20px;height:10px}}#sticky{{position:sticky;top:4px}}"));
        let g=geometry(&root,&dom,"target");let dx=if direction=="rtl"{-10.0}else{7.0};
        near(g.insets.left.unwrap(),dx);near(g.insets.right.unwrap(),-dx);
        near(g.insets.top.unwrap(),16.0);near(g.insets.bottom.unwrap(),-16.0);
        assert_eq!(g.overconstrained.left,direction=="rtl");assert_eq!(g.overconstrained.right,direction=="ltr");
        assert!(g.overconstrained.bottom);near(g.containing_block.w,150.0);
        for id in ["static","sticky"] { let s=geometry(&root,&dom,id);
            assert!(s.insets.top.is_none()&&s.insets.right.is_none()&&s.insets.bottom.is_none()&&s.insets.left.is_none()); }
    }
}
#[test]
fn resolved_sources_distinguish_principals_atomic_replaced_generated_and_no_box() {
    let (dom,root)=layout("<div id='principal'><span id='inline'>A</span><span id='atomic'>B</span><img id='image'><span id='hidden'>X</span><span id='contents'>Y</span></div><table id='table'><tr><td>C</td></tr></table>",
        "#principal{padding:3px;border:2px solid;width:100px;height:30px}#atomic{display:inline-block;width:20px;height:10px}#image{width:15px;height:8px;padding:1px;border:2px solid}#hidden{display:none}#contents{display:contents}#principal::before{content:'P';display:block;width:12px;height:6px}#principal::after{content:url(test.png);width:9px;height:7px}");
    let p=geometry(&root,&dom,"principal");assert_eq!(p.kind,ResolvedBoxKind::Principal);
    near(p.content_box.w,100.0);near(p.border_box.w,110.0);near(p.padding.left,3.0);near(p.border.left,2.0);
    assert_eq!(geometry(&root,&dom,"atomic").kind,ResolvedBoxKind::AtomicInline);
    let image=geometry(&root,&dom,"image");assert_eq!(image.kind,ResolvedBoxKind::Replaced);
    near(image.content_box.w,15.0);near(image.border_box.w,21.0);
    assert_eq!(geometry(&root,&dom,"table").kind,ResolvedBoxKind::TableWrapper);
    for id in ["inline","hidden","contents"] { assert!(resolved_style_geometry(&root,LayoutSource::Element(node(&dom,id))).is_none()); }
    let before=resolved_style_geometry(&root,LayoutSource::Pseudo(node(&dom,"principal"),PseudoElement::Before)).unwrap();
    near(before.content_box.w,12.0);near(before.content_box.h,6.0);
    let after=resolved_style_geometry(&root,LayoutSource::Pseudo(node(&dom,"principal"),PseudoElement::After)).unwrap();
    near(after.content_box.w,9.0);near(after.content_box.h,7.0);
    assert!(resolved_style_geometry(&root,LayoutSource::Pseudo(node(&dom,"principal"),PseudoElement::FirstLine)).is_none());
    let unlaid=box_tree::build_layout_tree(&dom,&amazing_css::compute(&dom,&[]),&font::FontSource::fallback());
    assert!(resolved_style_geometry(&unlaid,LayoutSource::Element(node(&dom,"principal"))).is_none());
}
#[test]
fn resolved_containment_grid_and_zoom_keep_actual_reference_geometry() {
    let (dom,root)=layout("<div id='contained'><div id='fixed'></div></div><div id='grid'><div id='abs'></div></div><div id='zoom'><div id='inner'></div></div>",
        "#contained{contain:layout;width:100px;height:60px;padding:10px;margin-left:20px}#fixed{position:fixed;left:5px;top:7px;width:10px;height:12px}#grid{display:grid;position:relative;grid-template-columns:40px 80px;grid-template-rows:30px;height:30px;width:120px}#abs{position:absolute;grid-column:2;left:2px;top:3px;width:10px;height:8px}#zoom{zoom:2}#inner{zoom:2;width:10px;height:5px;padding:2px;border:1px solid}");
    let c=geometry(&root,&dom,"contained");let f=geometry(&root,&dom,"fixed");
    near(f.containing_block.x,c.border_box.x+c.border.left);near(f.containing_block.w,120.0);
    near(f.border_box.x,f.containing_block.x+5.0);
    let grid=geometry(&root,&dom,"grid");let abs=geometry(&root,&dom,"abs");
    near(abs.containing_block.x,grid.content_box.x+40.0);near(abs.containing_block.w,80.0);
    let zoom=geometry(&root,&dom,"inner");assert_eq!(zoom.effective_zoom,4.0);
    near(zoom.content_box.w,40.0);near(zoom.padding.left,8.0);near(zoom.border.left,4.0);
}
struct NoMetrics;
impl amazing_css::font_metric_context::FontMetricContext for NoMetrics {
    fn aspect_ratio(&self,_:NodeIdx,_:Option<PseudoElement>,_:&amazing_css::ComputedStyle,
        _:amazing_css::font_size_adjust::FontSizeAdjustMetric)->Option<f64>{None}
}
#[test]
fn explicit_document_roots_preserve_arena_identity_and_independent_viewports() {
    let mut dom=Dom::new();let main_html=dom.create_element(NodeIdx(0),"html");
    let main_body=dom.create_element(main_html,"body");
    let document=dom.alloc_unattached(NodeKind::Document);
    let html=dom.create_element(document,"html");let body=dom.create_element(html,"body");
    let target=dom.create_element(body,"div");dom.set_attrs(target,vec![("id".into(),"target".into())]);
    let sheets=[amazing_css::parse_stylesheet("body{margin:0}#target{position:fixed;left:10%;top:20%;width:20px;height:10px}")];
    let style=amazing_css::compute_document_with_font_metrics(&dom,document,&sheets,&amazing_css::MediaEnvironment::default(),None,&NoMetrics);
    let root=layout_styled_document_from_root(&dom,document,&style,&FallbackTextSystem::new(),&NoReplacedElements,90.0,Some(70.0)).unwrap();
    assert_eq!(root.node,Some(body));let g=resolved_style_geometry(&root,LayoutSource::Element(target)).unwrap();
    near(g.containing_block.w,90.0);near(g.containing_block.h,70.0);near(g.border_box.x,9.0);near(g.border_box.y,14.0);
    assert!(resolved_style_geometry(&root,LayoutSource::Element(main_body)).is_none());
    assert!(layout_styled_document_from_root(&dom,body,&style,&FallbackTextSystem::new(),&NoReplacedElements,90.0,Some(70.0)).is_none());
    assert!(layout_styled_document_from_root(&dom,NodeIdx(u32::MAX),&style,&FallbackTextSystem::new(),&NoReplacedElements,90.0,Some(70.0)).is_none());
    let empty=dom.alloc_unattached(NodeKind::Document);
    assert!(layout_styled_document_from_root(&dom,empty,&style,&FallbackTextSystem::new(),&NoReplacedElements,90.0,Some(70.0)).unwrap().node.is_none());
}

#[test]
fn table_query_uses_actual_grid_decoration_wrapper_position_and_family_applicability() {
    let (dom,root)=layout("<table id='table'><caption id='caption'>Caption</caption><colgroup id='cols'><col id='col'></colgroup><tbody id='group'><tr id='row'><td id='cell'>C</td></tr></tbody></table>",
        "#table{zoom:2;width:60px;padding:4px;border:2px solid;margin-left:5px}#cell{padding:3px}");
    let t=geometry(&root,&dom,"table");assert_eq!(t.kind,ResolvedBoxKind::TableWrapper);
    assert!(t.table_wrapper_box.is_some());assert_eq!(t.effective_zoom,2.0);
    near(t.border.left,4.0);assert!(t.applicability.width&&t.applicability.height&&t.applicability.margin&&t.applicability.padding);
    for id in ["group","row"] {let g=geometry(&root,&dom,id);
        assert!(!g.applicability.width&&g.applicability.height&&!g.applicability.margin&&!g.applicability.padding);}
    for id in ["cols","col"] {let g=geometry(&root,&dom,id);
        assert!(g.applicability.width&&!g.applicability.height&&!g.applicability.margin&&!g.applicability.padding);}
    let c=geometry(&root,&dom,"cell");assert!(c.applicability.width&&c.applicability.height&&!c.applicability.margin&&c.applicability.padding);
    let (dom,root)=layout("<div id='multi'></div>","#multi::before{content:url(a.png) url(b.png);width:9px;height:7px}");
    assert!(resolved_style_geometry(&root,LayoutSource::Pseudo(node(&dom,"multi"),PseudoElement::Before)).is_none());
}
