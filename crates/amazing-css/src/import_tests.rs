use super::*;
use crate::parser::LayerName;
fn loaded(css: &str, url: &str) -> LoadedStylesheet {
    let mut sheet = parse_stylesheet(css); sheet.base_url=Some(url.into());
    LoadedStylesheet {sheet,imports:vec![]}
}
#[test]
fn import_url_layer_supports_and_media_grammar_is_atomic_and_ordered() {
    let sheet=parse_stylesheet(r#"@charset "UTF-8"; @layer base,theme;
        @import "a.css" layer(base.inner) supports(display:grid) screen and (min-width:100px);
        @import url('b.css') layer supports(not (display:bad));
        @import url(c.css) supports((display:flex) or selector(:host));
        @import "d.css" print,screen; p{color:red}"#);
    assert_eq!(sheet.imports().len(),4);assert_eq!(sheet.rules.len(),3);
    assert_eq!(sheet.imports()[0].rule_index,2);assert_eq!(sheet.imports()[0].url,"a.css");
    assert_eq!(sheet.imports()[0].layer,Some(ImportLayer::Named(vec!["base".into(),"inner".into()])));
    assert!(sheet.imports()[0].conditions_match(&MediaEnvironment::default()));
    assert_eq!(sheet.imports()[1].layer,Some(ImportLayer::Anonymous));
    assert!(sheet.imports()[1].supports_matches());assert!(sheet.imports()[3].conditions_match(&MediaEnvironment::default()));
    for css in ["@import none;","@import url(bad path);","@import url('a' 'b');","@import 'a' layer();",
        "@import 'a' layer(a .b);","@import 'a' layer(initial);","@import 'a' layer(a,b);",
        "@import 'a' supports();","@import 'a' supports(not);","@import 'a' supports((display:flex) and);",
        "@import 'a' supports((display:flex) or (display:grid) and (color:red));",
        "@import 'a' supports(display:flex) layer(x);","@import 'a' layer(x) layer(y);",
        "@import 'a' {p{color:red}}"] {
        assert!(parse_stylesheet(css).imports().is_empty(),"{css}");
    }
    assert_eq!(parse_stylesheet("@import 'a'; @import 'broken' layer(); @import 'b';").imports().len(),2);
}
#[test]
fn import_prefix_ignores_statements_and_invalid_rules_but_not_empty_blocks() {
    for css in ["p{} @import 'late';","p{color:red} @import 'late';","@layer empty{} @import 'late';",
        "@media print{} @import 'late';","@supports (display:bad){} @import 'late';",
        "@scope (.x){} @import 'late';","@font-face{} @import 'late';",
        "@media all{@import 'nested';}","@layer x{@import 'nested';}"] {
        assert!(parse_stylesheet(css).imports().is_empty(),"{css}");
    }
    for css in ["@layer x,y; @import 'ok';","@unknown whatever{} @import 'ok';",
        "???{} @import 'ok';","@import bad; @import 'ok';", "@supports not{} @import 'ok';", "@layer a,b{} @import 'ok';", "@scope invalid{} @import 'ok';"] {
        assert_eq!(parse_stylesheet(css).imports().len(),1,"{css}");
    }
}
#[test]
fn typed_supports_rejects_malformed_logic_without_turning_not_invalid_into_true() {
    for (css,yes) in [("(display:flex)",true),("(display:nonsense)",false),
        ("not (display:nonsense)",true),("(display:flex) and (color:red)",true),
        ("(display:nonsense) or (display:grid)",true),("selector(:host(.x))",true),
        ("selector(??? invalid)",false),("future(unknown)",false),("(future feature)",false),
        ("not future(unknown)",true),("at-rule(@import)",true),("at-rule(@nonsense)",false)] {
        assert_eq!(SupportsCondition::parse(css).unwrap().evaluate(),yes,"{css}");
    }
    for css in ["","not","not not (display:flex)","(display:flex) and","(display:flex) nonsense",
        "(display:flex) or (display:grid) and (color:red)","(display:flex))","(display:flex","()"] {
        assert!(SupportsCondition::parse(css).is_none(),"{css}");
    }
    let sheet=parse_stylesheet("@supports not {p{color:red}} @supports not (display:no){p{color:blue}}");
    assert_eq!(sheet.rules.len(),1);assert_eq!(sheet.rules[0].declarations[0].value,"blue");
}
#[test]
fn projection_preserves_layer_order_important_reversal_source_url_and_native_owner() {
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let p=dom.create_element(html,"p");
    let mut root=loaded("@layer first,last; @import 'a' layer(last); @import 'b' layer(first); p{width:3px}","https://root.test/main.css");
    root.sheet.scope_root=Some(html);
    root.imports.push((0,loaded("p{color:red!important;background-image:url(shared.png)}","https://a.test/theme.css")));
    root.imports.push((1,loaded("p{color:blue!important;background-image:url(shared.png)}","https://b.test/theme.css")));
    let result=project_imports(&root,&MediaEnvironment::default()).unwrap();
    assert!(result.stylesheets.iter().all(|sheet|sheet.scope_root==Some(html)));
    let styles=compute(&dom,&result.stylesheets);let style=styles.get(p).unwrap();
    assert_eq!(style.get("color"),Some("blue"));assert_eq!(style.get("width"),Some("3px"));
    assert_eq!(style.url_base("background-image"),Some("https://a.test/theme.css"));
    // Unlayered imported content occurs before local content at its own slot.
    let mut plain=loaded("@import 'a'; p{color:green}","https://root.test/main.css");
    plain.imports.push((0,loaded("p{color:red}","https://a.test/theme.css")));
    assert_eq!(compute(&dom,&project_imports(&plain,&MediaEnvironment::default()).unwrap().stylesheets).get(p).unwrap().get("color"),Some("green"));
}
#[test]
fn repeated_and_nested_anonymous_import_occurrences_do_not_merge_or_collide() {
    let mut root=loaded("@import 'same' layer; @import 'same' layer; @layer local{p{color:green!important}}","https://root.test/main.css");
    for index in 0..2 {
        let mut child=loaded("@import 'inner'; p{color:red!important}","https://same.test/same.css");
        child.imports.push((0,loaded("@layer inside{p{height:7px}}","https://inner.test/inner.css")));
        root.imports.push((index,child));
    }
    let result=project_imports(&root,&MediaEnvironment::default()).unwrap();
    let mut anonymous=Vec::new();
    for sheet in &result.stylesheets { for rule in &sheet.rules {
        if let Some(LayerName::Anonymous(id))=rule.layer.first() { if !anonymous.contains(id) {anonymous.push(*id);} }
    }}
    assert_eq!(anonymous.len(),2);
    let mut dom=Dom::new();let html=dom.create_element(NodeIdx(0),"html");let p=dom.create_element(html,"p");
    let styles=compute(&dom,&result.stylesheets);assert_eq!(styles.get(p).unwrap().get("color"),Some("red"));
    assert_eq!(styles.get(p).unwrap().get("height"),Some("7px"));
    let another=project_imports(&root,&MediaEnvironment::default()).unwrap();
    assert_ne!(another.stylesheets[0].rules[0].layer,result.stylesheets[0].rules[0].layer);
    // IDs are internal and fresh per projection; every split fragment agrees.
}
#[test]
fn missing_imports_register_only_matching_layers_and_fonts_use_actual_child_base() {
    let mut root=loaded("@import 'missing' layer(first); @import 'blocked' layer(blocked) supports(display:no); @import 'print' layer(print) print; @import 'font' layer(fonts); @layer last;","https://root.test/main.css");
    let child=loaded("@font-face{font-family:One;src:url(one.woff2)} @media print{@font-face{font-family:Print;src:url(print.woff2)}} @supports (display:no){@font-face{font-family:Never;src:url(no.woff2)}} @keyframes imported{to{opacity:0}}","https://cdn.test/fonts/actual.css");
    root.imports.push((3,child));let env=MediaEnvironment::default();let result=project_imports(&root,&env).unwrap();
    let layers:Vec<_>=result.stylesheets.iter().flat_map(|s|&s.rules).flat_map(|r|&r.layer).collect();
    assert!(layers.contains(&&LayerName::Named("first".into())));assert!(!layers.contains(&&LayerName::Named("blocked".into())));assert!(!layers.contains(&&LayerName::Named("print".into())));
    assert_eq!(result.font_faces.len(),1);assert_eq!(result.font_faces[0].font_face.family,"One");
    assert_eq!(result.font_faces[0].base_url.as_deref(),Some("https://cdn.test/fonts/actual.css"));
    assert_eq!(keyframes_rules(&result.stylesheets)[0].name,"imported");
    let print=MediaEnvironment{media_type:MediaType::Print,..env};let result=project_imports(&root,&print).unwrap();
    assert_eq!(result.font_faces.len(),2);assert!(result.stylesheets.iter().flat_map(|s|&s.rules).any(|r|r.layer==vec![LayerName::Named("print".into())]));
}
#[test]
fn actual_ancestor_cycles_and_graph_bounds_are_explicit_errors_not_global_url_dedup() {
    let mut root=loaded("@import 'next';","https://a.test/a.css");
    root.imports.push((0,loaded("p{color:red}","https://a.test/a.css")));
    assert_eq!(project_imports(&root,&MediaEnvironment::default()).unwrap_err(),ImportProjectionError::Cycle("https://a.test/a.css".into()));
    root.imports.clear();root.imports.push((1,loaded("","b")));
    assert_eq!(project_imports(&root,&MediaEnvironment::default()).unwrap_err(),ImportProjectionError::InvalidChildIndex(1));
    root.imports=vec![(0,loaded("","b")),(0,loaded("","c"))];
    assert_eq!(project_imports(&root,&MediaEnvironment::default()).unwrap_err(),ImportProjectionError::DuplicateChildIndex(0));
    let mut deep=loaded("p{color:red}","https://end.test/end.css");
    for index in 0..130 {let mut next=loaded("@import 'next';",&format!("https://deep.test/{index}"));next.imports.push((0,deep));deep=next;}
    assert_eq!(project_imports(&deep,&MediaEnvironment::default()).unwrap_err(),ImportProjectionError::DepthLimit);
    let mut many=loaded(&"@import 'same';".repeat(4096),"https://root.test/root.css");
    for index in 0..4096 {many.imports.push((index,loaded("","https://same.test/same.css")));}
    assert_eq!(project_imports(&many,&MediaEnvironment::default()).unwrap_err(),ImportProjectionError::InstanceLimit);
}
