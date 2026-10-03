use super::*;
use crate::{ComputedStyle, MediaEnvironment, StyledDom, Stylesheet, parse_stylesheet};
use amazing_dom::shadow::{ShadowMode,ShadowOptions,SlotAssignment};
use crate::font_metric_context::FontMetricContext;
struct Metrics;
impl FontMetricContext for Metrics {
    fn aspect_ratio(&self,_:NodeIdx,_:Option<PseudoElement>,_:&ComputedStyle,_:crate::font_size_adjust::FontSizeAdjustMetric)->Option<f64>{None}
}
fn attach(dom:&mut Dom,host:NodeIdx,mode:ShadowMode,assignment:SlotAssignment)->NodeIdx {
    dom.attach_shadow(host,ShadowOptions {mode,slot_assignment:assignment,delegates_focus:false,clonable:false,serializable:false}).unwrap()
}
fn scoped(dom:&Dom,sheets:&[Stylesheet],shadows:&[(NodeIdx,Vec<Stylesheet>)])->StyledDom {
    crate::compute_document_with_shadow_font_metrics(dom,NodeIdx(0),sheets,shadows,&MediaEnvironment::default(),None,&Metrics)
}
fn matching(dom:&Dom,node:NodeIdx,tree:NodeIdx,source:&str)->bool {
    matches_in_tree(dom,node,&parse_selector(source).unwrap(),None,tree)
}

#[test]
fn shadow_functions_obey_compound_grammar_specificity_and_alias_boundaries() {
    for source in [":host()",":host(.a,.b)",":host(.a > b)",":host-context",":host-context()",":host-context(div span)",
        "::host",":slotted(i)","::slotted()","::slotted(i,b)","::slotted(i b)","::slotted(::before)",
        "::slotted(i).foo","::slotted(i) > b","::slotted(i)::slotted(b)","::slotted(i)::selection",":has-slotted(*)"] {
        assert!(parse_selector(source).is_none(),"{source}");
    }
    for (source,want) in [(":host",Specificity{a:0,b:1,c:0}), (":host(#host)",Specificity{a:1,b:1,c:0}),
        (":host-context(.theme)",Specificity{a:0,b:2,c:0}), ("slot::slotted(.item)::before",Specificity{a:0,b:1,c:3}),
        ("::slotted(:where(#id))",Specificity{a:0,b:0,c:1})] {
        assert_eq!(parse_selector(source).unwrap().specificity(),want,"{source}");
    }
    assert!(parse_selector(":is(::slotted(i), .valid)").is_some());
    assert!(parse_selector(":not(::slotted(i))").is_none());
    for source in ["::slotted(i):hover","::slotted(i):is(:hover,:focus)","::slotted(i)::placeholder","::slotted(i)::file-selector-button"] {
        assert!(parse_selector(source).is_some(),"{source}");
    }
    assert!(parse_selector("::slotted(*):is(#discarded)").is_some());
    assert!(parse_selector("::slotted(i):not(.class)").is_none());
}

#[test]
fn virtual_host_and_logical_negation_never_leak_features_or_cross_native_documents() {
    for mode in [ShadowMode::Open,ShadowMode::Closed] {
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
        dom.set_attribute(html,"class","theme").unwrap();
        let host=dom.create_element(html,"section");dom.set_attribute(host,"class","host").unwrap();
        let root=attach(&mut dom,host,mode,SlotAssignment::Named);
        let child=dom.create_element(root,"span");dom.set_attribute(child,"class","child").unwrap();
        for source in [":host",":host(.host)",":host-context(.theme)",":is(:host, .missing)",":where(:host)",
            ":not(:host(.missing))",":not(:not(:host))",":host:has(> .child)"] {
            assert!(matching(&dom,host,root,source),"{source}");
            if source==":not(:host(.missing))" {
                assert!(matching(&dom,host,NodeIdx(0),source),"document/{source}");
            } else { assert!(!matching(&dom,host,NodeIdx(0),source),"document/{source}"); }
        }
        for source in ["*","section",".host",".host:host",":not(.missing)",":not(.missing:host)",
            ":not(:host > .child)",":has(.child)",":host(.missing)"] {
            assert!(!matching(&dom,host,root,source),"{source}");
        }
        assert!(matching(&dom,child,root,":host > .child"));
        assert!(matching(&dom,child,root,":host-context(.theme) .child"));
        assert!(!matching(&dom,child,root,"html .child"));
        assert!(!matching(&dom,child,NodeIdx(0),"span"));
        assert!(!matching(&dom,child,root,":root"));
        let nested=dom.create_element(root,"article");
        let deep_root=attach(&mut dom,nested,ShadowMode::Closed,SlotAssignment::Named);
        let deep=dom.create_element(deep_root,"b");
        assert!(matching(&dom,nested,deep_root,":host-context(.theme)"));
        assert!(!matching(&dom,deep,root,"b"));
    }
}

#[test]
fn host_encapsulation_order_precedes_inline_specificity_layers_and_inherits_actual_values() {
    for mode in [ShadowMode::Open,ShadowMode::Closed] {
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");
        let host=dom.create_element(html,"section");dom.set_attribute(host,"id","host").unwrap();
        dom.set_attribute(host,"style","color:red!important; width:30px; --theme:green").unwrap();
        let root=attach(&mut dom,host,mode,SlotAssignment::Named);
        let inner=dom.create_element(root,"span");
        let sheets=[parse_stylesheet("section{height:40px} span{color:purple}")];
        let shadows=[(root,vec![parse_stylesheet("@layer inner{:host{color:blue!important;height:20px!important}} :host(#host){width:10px;height:10px}span{background-color:var(--theme)}")])];
        let result=scoped(&dom,&sheets,&shadows);
        let style=result.get(host).unwrap();
        assert_eq!(style.get("color"),Some("blue"));
        assert_eq!(style.get("width"),Some("30px"));
        assert_eq!(style.get("height"),Some("20px"));
        assert_eq!(result.get(inner).unwrap().get("color"),Some("blue"));
        assert_eq!(result.get(inner).unwrap().get("background-color"),Some("green"));
    }
}

#[test]
fn slotted_aliases_preserve_slot_prefixes_real_assignment_pseudos_and_encapsulation() {
    for mode in [ShadowMode::Open,ShadowMode::Closed] {for assignment in [SlotAssignment::Named,SlotAssignment::Manual] {
        let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let host=dom.create_element(html,"section");
        let assigned=dom.create_element(host,"i");dom.set_attribute(assigned,"slot","chosen").unwrap();
        dom.set_attribute(assigned,"class","item").unwrap();dom.set_attribute(assigned,"style","color:red!important").unwrap();
        let descendant=dom.create_element(assigned,"b");
        let other=dom.create_element(host,"i");dom.set_attribute(other,"slot","unused").unwrap();
        let root=attach(&mut dom,host,mode,assignment);let wrapper=dom.create_element(root,"div");
        let slot=dom.create_element(wrapper,"slot");dom.set_attribute(slot,"name","chosen").unwrap();
        let fallback=dom.create_element(slot,"em");
        if assignment==SlotAssignment::Manual {dom.assign_slot(slot,&[assigned]).unwrap();}
        assert!(matching(&dom,assigned,root,"div > slot[name=chosen]::slotted(.item)"));
        assert!(matching(&dom,assigned,root,"::slotted(.item):not(:hover)"));
        assert!(!matching(&dom,assigned,root,"::slotted(.item):is(:hover,:focus)"));
        assert!(!matching(&dom,assigned,root,"::slotted(.item):is(#discarded)"));
        assert!(!matching(&dom,assigned,root,"slot[name=unused]::slotted(*)"));
        assert!(!matching(&dom,descendant,root,"::slotted(*)"));
        assert!(!matching(&dom,other,root,"::slotted(*)"));
        assert!(!matching(&dom,fallback,root,"::slotted(*)"));
        assert!(matching(&dom,slot,root,":has-slotted"));
        let sheets=[parse_stylesheet("i{height:40px;color:red!important}")];
        let shadows=[(root,vec![parse_stylesheet("slot{font-size:24px}slot::slotted(.item){height:10px;color:green!important}slot::slotted(.item)::before{content:'slot';color:blue}")])];
        let result=scoped(&dom,&sheets,&shadows);
        assert_eq!(result.get(assigned).unwrap().get("color"),Some("green"));
        assert_eq!(result.get(assigned).unwrap().get("height"),Some("40px"));
        assert_eq!(result.get(assigned).unwrap().get("font-size"),Some("24px"));
        assert_eq!(result.get_pseudo(assigned,PseudoElement::Before).unwrap().get("content"),Some("\"slot\""));
        assert!(result.get(other).is_none());assert!(result.get(fallback).is_none());
    }}
}

#[test]
fn has_slotted_includes_whitespace_text_but_not_ordinary_slot_or_fallback() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let host=dom.create_element(html,"section");
    let root=attach(&mut dom,host,ShadowMode::Open,SlotAssignment::Named);
    let slot=dom.create_element(root,"slot");let fallback=dom.create_element(slot,"span");
    assert!(!matching(&dom,slot,root,":has-slotted"));
    assert!(!matching(&dom,fallback,root,"::slotted(*)"));
    dom.create_text(host," ");
    assert!(matching(&dom,slot,root,":has-slotted"));
    let ordinary=dom.create_element(html,"slot");dom.create_text(ordinary,"text");
    assert!(!matching(&dom,ordinary,NodeIdx(0),":has-slotted"));
}

#[test]
fn nested_flattened_aliases_keep_immediate_slot_inheritance_and_real_context_rollback() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let host=dom.create_element(html,"section");
    let item=dom.create_element(host,"i");
    let root=attach(&mut dom,host,ShadowMode::Open,SlotAssignment::Named);
    let nested=dom.create_element(root,"article");let outer_slot=dom.create_element(nested,"slot");
    let deep_root=attach(&mut dom,nested,ShadowMode::Closed,SlotAssignment::Named);
    let inner_slot=dom.create_element(deep_root,"slot");
    assert!(matching(&dom,item,root,"slot::slotted(i)"));
    assert!(matching(&dom,item,deep_root,"slot::slotted(i)"));
    assert!(matching(&dom,inner_slot,deep_root,":has-slotted"));
    let sheets=[parse_stylesheet("i{color:red!important;width:40px}")];
    let shadows=[(root,vec![parse_stylesheet("slot{font-size:24px}@layer base,reset;@layer base{::slotted(i){color:green!important;width:20px}}@layer reset{::slotted(i){color:revert-layer!important}}")]),
        (deep_root,vec![parse_stylesheet("slot{font-size:30px}::slotted(i){color:blue!important;width:10px}")])];
    let result=scoped(&dom,&sheets,&shadows);
    let style=result.get(item).unwrap();
    assert_eq!(style.get("color"),Some("blue"));
    assert_eq!(style.get("width"),Some("40px"));
    assert_eq!(style.get("font-size"),Some("24px"));
    assert_eq!(result.get(outer_slot).unwrap().get("font-size"),Some("24px"));
    assert_eq!(result.get(inner_slot).unwrap().get("font-size"),Some("30px"));
}

#[test]
fn explicit_and_implicit_scopes_bind_the_featureless_host_and_slotted_originating_slot() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let host=dom.create_element(html,"section");
    let item=dom.create_element(host,"i");
    let root=attach(&mut dom,host,ShadowMode::Closed,SlotAssignment::Named);
    let slot=dom.create_element(root,"slot");
    let blocked=dom.create_element(root,"div");dom.set_attribute(blocked,"class","blocked").unwrap();
    let inner=dom.create_element(blocked,"span");
    let mut implicit=parse_stylesheet("@scope {:scope{background-color:green} slot::slotted(i){width:20px}}");
    implicit.scope_root=Some(root);
    let shadows=[(root,vec![parse_stylesheet("@scope (:host) to (.blocked){:scope{color:blue}:host{padding-left:5px}slot::slotted(i){height:30px}span{color:red}}"),implicit])];
    let result=scoped(&dom,&[],&shadows);
    assert_eq!(result.get(host).unwrap().get("color"),Some("blue"));
    assert_eq!(result.get(host).unwrap().get("background-color"),Some("green"));
    assert_eq!(result.get(host).unwrap().get("padding-left"),Some("5px"));
    assert_eq!(result.get(item).unwrap().get("width"),Some("20px"));
    assert_eq!(result.get(item).unwrap().get("height"),Some("30px"));
    assert_eq!(result.get(inner).unwrap().get("color"),Some("blue"));
    assert!(matching(&dom,slot,root,"slot"));
}
