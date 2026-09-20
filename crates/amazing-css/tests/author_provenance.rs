use amazing_css::{ComputedStyle, compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn fixture(css: &str, inline: &str) -> (ComputedStyle, ComputedStyle) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let parent = dom.create_element(html, "div");
    let child = dom.create_element(parent, "p");
    dom.set_attrs(
        child,
        vec![
            ("style".into(), inline.into()),
            ("dir".into(), "rtl".into()),
        ],
    );
    let styles = compute(&dom, &[parse_stylesheet(css)]);
    (
        styles.get(parent).unwrap().clone(),
        styles.get(child).unwrap().clone(),
    )
}

#[test]
fn initial_values_and_inherited_values_do_not_imply_local_author_declarations() {
    let (parent, child) = fixture("div {color: red; background: transparent}", "");
    assert!(parent.is_author_specified("background-color"));
    assert_eq!(
        parent.get("background-color"),
        child.get("background-color")
    );
    assert!(!child.is_author_specified("background-color"));
    assert_eq!(child.get("color"), Some("red"));
    assert!(!child.is_author_specified("color"));
    assert!(!child.is_author_specified("display"));
    assert!(!child.is_author_specified("direction")); // presentational dir hint
}

#[test]
fn explicit_initial_inherit_unset_and_currentcolor_count_locally() {
    for keyword in ["initial", "inherit", "unset"] {
        let (_, child) = fixture(
            "div {background-color: red}",
            &format!("background-color:{keyword}"),
        );
        assert!(child.is_author_specified("BACKGROUND-COLOR"), "{keyword}");
        assert_eq!(
            child.get("background-color"),
            Some(if keyword == "inherit" {
                "red"
            } else {
                "transparent"
            })
        );
    }
    let (_, child) = fixture("div{color:red} p{color:currentcolor}", "");
    assert_eq!(child.get("color"), Some("red"));
    assert!(child.is_author_specified("color"));
}

#[test]
fn origin_and_layer_rollback_report_the_restored_winner() {
    for rollback in ["revert", "var(--missing, revert)"] {
        let (_, child) = fixture("p{background:red}", &format!("background-color:{rollback}"));
        assert!(!child.is_author_specified("background-color"));
        assert_eq!(child.get("background-color"), Some("transparent"));
    }
    for rollback in ["revert-layer", "var(--missing, revert-layer)"] {
        let (_, child) = fixture(
            &format!(
                "@layer a,b; @layer a{{p{{background:transparent}}}} @layer b{{p{{background-color:{rollback}}}}}"
            ),
            "",
        );
        assert!(child.is_author_specified("background-color"));
        let (_, child) = fixture(&format!("@layer b{{p{{background-color:{rollback}}}}}"), "");
        assert!(!child.is_author_specified("background-color"));
    }
}

#[test]
fn invalid_parse_time_declarations_do_not_count_but_invalid_computed_winners_do() {
    let (_, child) = fixture("p{background-color:banana; border-top:solid solid}", "");
    assert!(!child.is_author_specified("background-color"));
    assert!(!child.is_author_specified("border-top-style"));
    for value in ["var(--missing)", "var(--missing, banana)"] {
        let (_, child) = fixture(&format!("p{{background:red;background-color:{value}}}"), "");
        assert_eq!(child.get("background-color"), Some("transparent"));
        assert!(child.is_author_specified("background-color"));
    }
}

#[test]
fn shorthands_and_all_mark_only_their_expanded_properties() {
    let (_, child) = fixture("p{background:transparent;border:none}", "");
    assert!(child.is_author_specified("background-color"));
    assert!(!child.is_author_specified("background"));
    for side in ["top", "right", "bottom", "left"] {
        for part in ["width", "style", "color"] {
            assert!(child.is_author_specified(&format!("border-{side}-{part}")));
        }
    }
    let (_, child) = fixture("p{all:initial}", "");
    assert!(child.is_author_specified("background-color"));
    assert!(child.is_author_specified("border-left-style"));
    assert!(!child.is_author_specified("direction"));
    assert!(!child.is_author_specified("unicode-bidi"));
}

#[test]
fn custom_properties_keep_case_and_do_not_inherit_provenance() {
    let (parent, child) = fixture(
        "div{--Theme:red} p{--local:initial;unknown-property:hello}",
        "",
    );
    assert!(parent.is_author_specified("--Theme"));
    assert!(!parent.is_author_specified("--theme"));
    assert!(!child.is_author_specified("--Theme"));
    assert!(child.is_author_specified("--local"));
    assert!(child.is_author_specified("unknown-property"));
}

#[test]
fn synthetic_mutations_clear_only_the_replaced_properties_provenance() {
    let (_, mut child) = fixture(
        "p{background:transparent; border-left:solid; color:red}",
        "",
    );
    child.set("background-color", "transparent");
    assert!(!child.is_author_specified("background-color"));
    assert!(child.is_author_specified("color"));
    child.set("border-left", "dashed blue");
    for part in ["width", "style", "color"] {
        assert!(!child.is_author_specified(&format!("border-left-{part}")));
    }
    let synthetic: ComputedStyle = [("color".into(), "red".into())].into_iter().collect();
    assert!(!synthetic.is_author_specified("color"));
    assert!(!ComputedStyle::default().is_author_specified("background-color"));
}

#[test]
fn logical_mapping_and_important_rollback_preserve_winner_provenance() {
    let (_, child) = fixture(
        "p{margin-inline-start:3px!important}",
        "margin-right:revert",
    );
    assert_eq!(child.get("margin-right"), Some("3px"));
    assert!(child.is_author_specified("margin-right"));
    let (_, child) = fixture(
        "p{background:red!important}",
        "background-color:revert!important",
    );
    assert!(!child.is_author_specified("background-color"));
}
