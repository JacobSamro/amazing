use amazing_css::{compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn fixture() -> (Dom, [NodeIdx; 5]) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let section = dom.create_element(body, "section");
    let p = dom.create_element(section, "p");
    let span = dom.create_element(p, "span");
    dom.set_attrs(p, vec![("id".into(), "target".into())]);
    (dom, [html, body, section, p, span])
}
fn value(css: &str, property: &str) -> String {
    let (dom, nodes) = fixture();
    compute(&dom, &[parse_stylesheet(css)])
        .get(nodes[3])
        .unwrap()
        .get(property)
        .unwrap()
        .to_owned()
}

#[test]
fn layer_order_precedes_specificity_and_source_order() {
    assert_eq!(
        value(
            "@layer a, b; @layer b { p {color: green} } @layer a { #target {color: red} }",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "p {color: green} @layer a { #target {color: red} }",
            "color"
        ),
        "green"
    );
}
#[test]
fn important_reverses_layers_including_unlayered() {
    assert_eq!(
        value(
            "@layer a,b; @layer a { p {color: green!important} } @layer b { #target {color:red!important} } #target {color:blue!important}",
            "color"
        ),
        "green"
    );
}

#[test]
fn important_revert_layer_excludes_weaker_important_layers() {
    let css = "p { background-color: blue } \
        @layer { p { background-color: green } } \
        @layer { p { background-color: red; background-color: red !important; \
                    background-color: revert-layer !important } } \
        @layer { p { background-color: red; background-color: red !important } }";
    assert_eq!(value(css, "background-color"), "green");
}
#[test]
fn nested_layers_group_under_parent_and_direct_rules_win() {
    assert_eq!(
        value(
            "@layer a,b; @layer a.deep {p{color:red}} @layer b {p{color:green}} @layer a.later {p{color:blue}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a {p{color:green} @layer b {#target{color:red}}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a {p{color:red!important} @layer b {p{color:green!important}}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn layer_order_and_names_span_stylesheets_anonymous_layers_do_not() {
    let (dom, nodes) = fixture();
    for sheets in [
        vec![
            "@layer b,a;",
            "@layer a{p{color:green}}",
            "@layer b{p{color:red}}",
        ],
        vec!["@layer {#target{color:red}}", "@layer {p{color:green}}"],
        vec![
            "@layer A{p{color:red}}",
            "@layer a{p{color:green}}",
            "@layer A{p{color:blue}}",
        ],
    ] {
        let sheets: Vec<_> = sheets.into_iter().map(parse_stylesheet).collect();
        assert_eq!(
            compute(&dom, &sheets).get(nodes[3]).unwrap().get("color"),
            Some("green")
        );
    }
}
#[test]
fn revert_restores_ua_instead_of_inheriting_or_reviving_author() {
    assert_eq!(value("p{display:flex;display:revert}", "display"), "block");
    assert_eq!(
        value("p{display:flex;display:revert!important}", "display"),
        "block"
    );
    assert_eq!(
        value("section{color:green} p{color:red;color:revert}", "color"),
        "green"
    );
    assert_eq!(value("p{margin:90px;all:revert}", "margin-top"), "1em");
}
#[test]
fn revert_layer_rolls_back_whole_layer_and_chains() {
    assert_eq!(
        value(
            "@layer a,b,c; @layer a{p{color:green}} @layer b{p{color:blue;color:revert-layer}} @layer c{p{color:revert-layer}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a{p{color:green}} p{color:red;color:revert-layer}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value("@layer a{p{display:flex;display:revert-layer}}", "display"),
        "block"
    );
    assert_eq!(
        value(
            "@layer a{p{margin:2px}} @layer b{p{all:revert-layer}}",
            "margin-top"
        ),
        "2px"
    );
}
#[test]
fn important_layer_rollback_removes_normal_interval() {
    assert_eq!(
        value(
            "@layer a,b; @layer a{p{color:green}} @layer b{p{color:revert-layer!important}} p{color:red}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a,b; @layer a{p{color:revert-layer!important}} @layer b{p{color:green!important}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a{p{display:flex;display:revert-layer!important}} p{display:grid}",
            "display"
        ),
        "block"
    );
}
#[test]
fn inline_precedence_and_rollback() {
    let (mut dom, nodes) = fixture();
    for (inline, css, expected) in [
        (
            "color:green!important",
            "@layer a{p{color:red!important}}",
            "green",
        ),
        (
            "color:revert-layer!important",
            "@layer a{p{color:green!important}}",
            "green",
        ),
        ("color:revert-layer", "p{color:green}", "green"),
        (
            "display:revert!important",
            "p{display:flex!important}",
            "block",
        ),
    ] {
        dom.set_attrs(nodes[3], vec![("style".into(), inline.into())]);
        let property = if inline.starts_with("display") {
            "display"
        } else {
            "color"
        };
        assert_eq!(
            compute(&dom, &[parse_stylesheet(css)])
                .get(nodes[3])
                .unwrap()
                .get(property),
            Some(expected)
        );
    }
}
#[test]
fn custom_properties_roll_back_before_substitution() {
    assert_eq!(
        value(
            "@layer a{p{--x:green}} @layer b{p{--x:revert-layer;color:var(--x)}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "section{--x:green} p{--x:red;--x:revert;color:var(--x)}",
            "color"
        ),
        "green"
    );
}
#[test]
fn scope_proximity_follows_specificity_and_precedes_source_order() {
    assert_eq!(
        value(
            "@scope(section){p{color:green}} @scope(body){p{color:red}} p{color:blue}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@scope(section){p{color:red}} @scope(body){#target{color:green}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@scope(section){p{color:red}} @scope(section){p{color:green}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a,b; @layer b{@scope(body){p{color:green}}} @layer a{@scope(section){p{color:red}}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn scope_root_limit_and_inheritance_are_separate() {
    let (dom, n) = fixture();
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "@scope(section) to (p){:scope{color:green;background-color:green} p{background-color:red} span{background-color:red}}",
        )],
    );
    assert_eq!(
        styles.get(n[2]).unwrap().get("background-color"),
        Some("green")
    );
    for node in &n[3..] {
        assert_eq!(
            styles.get(*node).unwrap().get("background-color"),
            Some("transparent")
        );
        assert_eq!(styles.get(*node).unwrap().get("color"), Some("green"));
    }
    assert_eq!(styles.get(n[1]).unwrap().get("color"), Some("black"));
}
#[test]
fn scope_pseudo_class_binds_in_compounds_and_limits() {
    assert_eq!(
        value(
            "@scope(section) to (:scope > span){:is(:scope) > p{color:green}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value("@scope(section) to (:scope > p){p{color:red}}", "color"),
        "black"
    );
    assert_eq!(value(":scope{color:green}", "color"), "green");
    assert_eq!(value("@scope(section){> p{color:green}}", "color"), "green");
}
#[test]
fn nested_scopes_keep_outer_limits_and_bind_inner_start_to_outer_root() {
    assert_eq!(
        value(
            "@scope(body){@scope(:scope > section){:scope > p{color:green}}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@scope(body) to (p){@scope(section){p{color:red}}}",
            "color"
        ),
        "black"
    );
}
#[test]
fn overlapping_scopes_consider_all_roots_before_selecting_proximity() {
    assert_eq!(
        value(
            "@scope(body,section){:scope > section > p{color:green}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn implicit_scope_uses_explicit_stylesheet_owner() {
    let (dom, n) = fixture();
    let mut sheet = parse_stylesheet("@scope{:scope{background-color:green} p{color:green}}");
    sheet.scope_root = Some(n[2]);
    let styles = compute(&dom, &[sheet]);
    assert_eq!(
        styles.get(n[2]).unwrap().get("background-color"),
        Some("green")
    );
    assert_eq!(styles.get(n[3]).unwrap().get("color"), Some("green"));
    assert_eq!(
        styles.get(n[1]).unwrap().get("background-color"),
        Some("transparent")
    );
}
#[test]
fn grouping_recovery_does_not_activate_invalid_or_false_rules() {
    for prefix in [
        "@layer a,b {p{color:red}}",
        "@scope(p::before){p{color:red}}",
        "@scope(section) garbage{p{color:red}}",
        "@layer initial{p{color:red}}",
    ] {
        assert_eq!(
            value(&format!("p{{color:green}} {prefix}"), "color"),
            "green"
        );
    }
}
#[test]
fn logical_properties_rollback_with_physical_candidates() {
    assert_eq!(
        value(
            "@layer a{p{margin-left:7px}} @layer b{p{margin-inline-start:revert-layer}}",
            "margin-left"
        ),
        "7px"
    );
    assert_eq!(
        value(
            "@layer a{p{margin-inline-start:7px}} @layer b{p{margin-left:revert-layer}}",
            "margin-left"
        ),
        "7px"
    );
}
#[test]
fn substituted_rollback_and_invalid_substitution_are_distinct() {
    assert_eq!(
        value("p{display:flex;display:var(--missing,revert)}", "display"),
        "block"
    );
    assert_eq!(
        value(
            "@layer a{p{color:green}} @layer b{p{color:var(--missing,revert-layer)}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a{p{color:green}} @layer b{p{color:var(--missing)}}",
            "color"
        ),
        "black"
    );
    assert_eq!(
        value(
            "p{--keyword:var(--missing,revert);display:var(--keyword)}",
            "display"
        ),
        "block"
    );
}
#[test]
fn deferred_shorthands_accept_substituted_rollback() {
    assert_eq!(
        value(
            "@layer a{p{overflow:hidden}} @layer b{p{overflow:var(--missing,revert-layer)}}",
            "overflow-x"
        ),
        "hidden"
    );
    assert_eq!(
        value(
            "@layer a{p{border-radius:7px}} @layer b{p{border-radius:var(--missing,revert-layer)}}",
            "border-top-left-radius"
        ),
        "7px"
    );
}
#[test]
fn relative_scope_limits_and_nested_starts() {
    assert_eq!(
        value("@scope(body){@scope(> section){p{color:green}}}", "color"),
        "green"
    );
    assert_eq!(
        value("@scope(section) to (> p){p{color:red}}", "color"),
        "black"
    );
}
#[test]
fn all_substitution_rolls_back_and_excludes_direction_and_custom_properties() {
    assert_eq!(
        value("p{all:var(--missing,revert);color:green}", "display"),
        "block"
    );
    assert_eq!(
        value(
            "@layer a{p{margin:7px}} @layer b{p{all:var(--missing,revert-layer)}}",
            "margin-left"
        ),
        "7px"
    );
    assert_eq!(
        value("p{color:red;all:var(--missing,green)}", "color"),
        "black"
    );
    assert_eq!(
        value(
            "p{--x:green;direction:rtl;all:revert;color:var(--x)}",
            "color"
        ),
        "green"
    );
    assert_eq!(value("p{direction:rtl;all:revert}", "direction"), "rtl");
}
#[test]
fn defaulting_keywords_work_after_layer_rollback() {
    assert_eq!(
        value(
            "section{color:green}@layer a{p{color:inherit}}@layer b{p{color:revert-layer}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "section{color:red}@layer a{p{color:initial}}@layer b{p{color:revert-layer}}",
            "color"
        ),
        "black"
    );
    assert_eq!(
        value(
            "section{color:green}@layer a{p{color:unset}}@layer b{p{color:revert-layer}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn layer_name_grammar_preserves_adjacency_and_allows_default() {
    assert_eq!(value("@layer default{p{color:green}}", "color"), "green");
    for invalid in ["a .b", "a. b", "a . b", "a b", "a,", ",a", "a.initial"] {
        assert_eq!(
            value(&format!("@layer {invalid} {{p{{color:red}}}}"), "color"),
            "black"
        );
    }
    assert_eq!(
        value(
            "@layer a, b; @layer b{p{color:green}} @layer a{p{color:red}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn anonymous_layer_children_are_private_and_reopened_names_keep_order() {
    assert_eq!(
        value(
            "@layer{@layer x{#target{color:red}}} @layer{@layer x{p{color:green}}}",
            "color"
        ),
        "green"
    );
    assert_eq!(
        value(
            "@layer a{p{color:red}} @layer b{p{color:green}} @layer a{p{color:blue}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn oversized_dotted_layer_names_are_rejected_without_recursion() {
    let name = vec!["a"; 129].join(".");
    assert_eq!(
        value(&format!("@layer {name}{{p{{color:red}}}}"), "color"),
        "black"
    );
}
#[test]
fn invalid_scope_does_not_reserve_layer_order() {
    assert_eq!(
        value(
            "@scope(> body){@layer b;} @layer a,b; @layer b{p{color:green}} @layer a{p{color:red}}",
            "color"
        ),
        "green"
    );
}
#[test]
fn matching_selector_specificity_is_chosen_before_scope_proximity() {
    assert_eq!(
        value(
            "@scope(body,section){p,#target{color:green}} @scope(section){p{color:red}}",
            "color"
        ),
        "green"
    );
}
