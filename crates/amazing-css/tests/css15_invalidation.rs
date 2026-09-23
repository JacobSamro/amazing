use amazing_css::selectors::PseudoElement;
use amazing_css::{
    compute, parse_stylesheet, recompute_roots, MutationKind, RenderMutation, StyleInvalidator,
};
use amazing_dom::{Dom, NodeIdx};

fn fixture(
    css: &str,
) -> (
    Dom,
    amazing_css::parser::Stylesheet,
    NodeIdx,
    NodeIdx,
    NodeIdx,
) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let first = dom.create_element(html, "div");
    let child = dom.create_element(first, "span");
    let second = dom.create_element(html, "p");
    (dom, parse_stylesheet(css), first, child, second)
}

#[test]
fn class_mutation_recomputes_inherited_descendants_and_reports_delta() {
    let (mut dom, sheet, first, child, second) =
        fixture(".hot { color:red } .hot span { font-weight:bold }");
    let old = compute(&dom, &[sheet.clone()]);
    dom.set_attribute(first, "class", "hot").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet], 7);
    let roots = invalidator.collect(
        &[RenderMutation::attribute(first, "class", None, Some("hot"))],
        7,
    );
    assert!(!roots.full);
    let delta = recompute_roots(&dom, &old, roots);
    assert!(!delta.full_rebuild);
    assert!(delta.recalculated.contains(&first));
    assert!(delta.recalculated.contains(&child));
    assert!(!delta.recalculated.contains(&second));
    assert!(delta
        .changed
        .get(&child)
        .unwrap()
        .contains(&"font-weight".into()));
    assert_eq!(delta.styles.get(child).unwrap().get("color"), Some("red"));
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();
}

#[test]
fn sibling_selector_recalculates_following_sibling() {
    let (mut dom, sheet, first, _, second) = fixture(".hot + p { color:green }");
    let old = compute(&dom, &[sheet.clone()]);
    dom.set_attribute(first, "class", "hot").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet], 2);
    let roots = invalidator.collect(
        &[RenderMutation::attribute(first, "class", None, Some("hot"))],
        2,
    );
    assert_eq!(roots.following_siblings, vec![first]);
    let delta = recompute_roots(&dom, &old, roots);
    assert!(delta.recalculated.contains(&second));
    assert_eq!(
        delta.styles.get(second).unwrap().get("color"),
        Some("green")
    );
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();
}

#[test]
fn id_and_attribute_mutations_use_old_and_new_tokens() {
    let (mut dom, sheet, first, _, _) =
        fixture("#one { color:red } #two { color:blue } [title=yes] { display:none }");
    dom.set_attribute(first, "id", "one").unwrap();
    let old = compute(&dom, &[sheet.clone()]);
    dom.set_attribute(first, "id", "two").unwrap();
    dom.set_attribute(first, "title", "yes").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet], 1);
    let roots = invalidator.collect(
        &[
            RenderMutation::attribute(first, "id", Some("one"), Some("two")),
            RenderMutation::attribute(first, "title", None, Some("yes")),
        ],
        1,
    );
    assert_eq!(roots.roots, vec![first]);
    let delta = recompute_roots(&dom, &old, roots);
    assert_eq!(delta.styles.get(first).unwrap().get("color"), Some("blue"));
    assert_eq!(
        delta.styles.get(first).unwrap().get("display"),
        Some("none")
    );
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();
}

#[test]
fn relational_tree_and_epoch_changes_fall_back_to_full() {
    let (dom, sheet, first, _, _) = fixture("div:has(span) { color:red }");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 3);
    let old = compute(&dom, &[sheet]);
    for (mutations, epoch) in [
        (
            vec![RenderMutation::attribute(first, "class", None, Some("x"))],
            3,
        ),
        (
            vec![RenderMutation {
                node: first,
                generation: 0,
                kind: MutationKind::Tree,
            }],
            3,
        ),
        (vec![], 4),
    ] {
        let roots = invalidator.collect(&mutations, epoch);
        assert!(roots.full);
        let delta = recompute_roots(&dom, &old, roots);
        assert!(delta.full_rebuild);
        invalidator.compare_to_full(&dom, &delta.styles).unwrap();
    }
}

#[test]
fn oracle_reports_first_differing_node_and_property() {
    let (mut dom, sheet, first, _, _) = fixture(".hot { color:red }");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 1);
    let old = compute(&dom, &[sheet]);
    dom.set_attribute(first, "class", "hot").unwrap();
    let mismatch = invalidator.compare_to_full(&dom, &old).unwrap_err();
    assert_eq!(mismatch.node, first);
    assert_eq!(mismatch.property, "color");
}

#[test]
fn pseudo_generation_updates_and_recycled_generation_falls_back() {
    let (mut dom, sheet, first, _, _) = fixture(".hot::before { content:'new' }");
    let old = compute(&dom, &[sheet.clone()]);
    dom.set_attribute(first, "class", "hot").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet], 1);
    let mutation = RenderMutation::attribute(first, "class", None, Some("hot"));
    let delta = recompute_roots(&dom, &old, invalidator.collect(&[mutation.clone()], 1));
    assert!(delta
        .changed
        .get(&first)
        .unwrap()
        .contains(&"::Before".into()));
    assert!(delta
        .styles
        .get_pseudo(first, PseudoElement::Before)
        .is_some());
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();
    let mut recycled = mutation;
    recycled.generation = 1;
    assert!(invalidator.collect(&[recycled], 1).full);
}

#[test]
fn root_font_change_recomputes_rem_from_initial_root_basis() {
    let (mut dom, sheet, first, child, _) =
        fixture("html.hot { font-size:20px } span { width:2rem }");
    let html = dom.get(first).parent.unwrap();
    let old = compute(&dom, &[sheet.clone()]);
    dom.set_attribute(html, "class", "hot").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet], 1);
    let roots = invalidator.collect(
        &[RenderMutation::attribute(html, "class", None, Some("hot"))],
        1,
    );
    let delta = recompute_roots(&dom, &old, roots);
    assert_eq!(delta.styles.get(child).unwrap().used_length_px("width"), Some(40.0));
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();
}
