use amazing_css::selectors::PseudoElement;
use amazing_css::{
    compute, parse_stylesheet, recompute_roots, AttributeStyleUpdate, MutationKind, RenderMutation,
    CachedBoxSize, StyleChangeKind, StyleInvalidator,
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

#[test]
fn classified_attribute_update_covers_class_style_inheritance_and_rebuild() {
    let (mut dom, sheet, first, child, _) = fixture(
        ".paint { color:red } .rebuild { display:none }",
    );
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 1);
    let old = compute(&dom, &[sheet]);
    dom.set_attribute(first, "class", "paint").unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "class", None, Some("paint"), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::PaintOnly);
    assert_eq!(update.change_kind(child), StyleChangeKind::PaintOnly);
    let AttributeStyleUpdate::Recomputed(delta) = update else { panic!("class changed selector matches") };
    assert!(!delta.full_rebuild);
    invalidator.compare_to_full(&dom, &delta.styles).unwrap();

    dom.set_attribute(first, "class", "rebuild").unwrap();
    let update = invalidator.update_attribute(&dom, &delta.styles, first, "class", Some("paint"), Some("rebuild"), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::Rebuild);

    dom.set_attribute(first, "class", "").unwrap();
    let width_sheet = parse_stylesheet(".wide { width:20px }");
    let invalidator = StyleInvalidator::new(&[width_sheet.clone()], 1);
    let old = compute(&dom, &[width_sheet]);
    dom.set_attribute(first, "class", "wide").unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "class", Some(""), Some("wide"), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::Layout);
}

#[test]
fn compositor_values_need_stable_inline_winner_and_no_selector_dependency() {
    let (mut dom, sheet, _, _, first) = fixture("div { color:blue }");
    dom.set_attribute(first, "style", "opacity:.4; transform:translate(2px, 3px)").unwrap();
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 1);
    let mut old = compute(&dom, &[sheet]);
    let new_style = "opacity:.6; transform:translate(4px, 3px)";
    dom.set_attribute(first, "style", new_style).unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "style",
        Some("opacity:.4; transform:translate(2px, 3px)"), Some(new_style), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::CompositorOnly);
    let AttributeStyleUpdate::CompositorOnly(values) = update else { panic!("expected value-only delta") };
    assert_eq!(values.opacity, Some(0.6));
    assert_eq!(values.transform_matrix.as_ref().map(|matrix| (matrix[0][3], matrix[1][3])), Some((4.0, 3.0)));
    assert!(values.apply_to_styles(&mut old));
    assert_eq!(old.get(first).unwrap().get("opacity"), Some("0.6"));
    assert_eq!(old.get(first).unwrap().get("transform"), Some("translate(4px, 3px)"));
    let next_style = "opacity:.7; transform:translate(6px, 3px)";
    dom.set_attribute(first, "style", next_style).unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "style", Some(new_style), Some(next_style), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::CompositorOnly);

    let old = compute(&dom, &[parse_stylesheet("")]);
    dom.set_attribute(first, "style", "opacity:1; transform:none").unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "style", Some(next_style),
        Some("opacity:1; transform:none"), 1);
    assert_eq!(update.change_kind(first), StyleChangeKind::Rebuild);

    let dependent = parse_stylesheet("[style] { color:red }");
    let invalidator = StyleInvalidator::new(&[dependent.clone()], 1);
    dom.set_attribute(first, "style", "opacity:.4; transform:none").unwrap();
    let old = compute(&dom, &[dependent]);
    dom.set_attribute(first, "style", "opacity:.5; transform:none").unwrap();
    let update = invalidator.update_attribute(&dom, &old, first, "style",
        Some("opacity:.4; transform:none"), Some("opacity:.5; transform:none"), 1);
    assert!(matches!(update, AttributeStyleUpdate::Recomputed(_)));
}

#[test]
fn transform_matrix_uses_cached_box_for_origin_and_percentages() {
    let (mut dom, sheet, _, _, node) = fixture("");
    let old_style = "transform:rotate(0deg)";
    let new_style = "transform:translate(10%, 0px) rotate(90deg) scale(2)";
    dom.set_attribute(node, "style", old_style).unwrap();
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 1);
    let old = compute(&dom, &[sheet]);
    dom.set_attribute(node, "style", new_style).unwrap();
    let fallback = invalidator.update_attribute(&dom, &old, node, "style", Some(old_style), Some(new_style), 1);
    assert_eq!(fallback.change_kind(node), StyleChangeKind::Layout);

    let update = invalidator.update_attribute_with_box(&dom, &old, node, "style", Some(old_style),
        Some(new_style), 1, Some(CachedBoxSize { width: 100.0, height: 40.0 }));
    assert_eq!(update.change_kind(node), StyleChangeKind::CompositorOnly);
    let AttributeStyleUpdate::CompositorOnly(delta) = update else { panic!("expected matrix") };
    let matrix = delta.transform_matrix.unwrap();
    let x = matrix[0][0] * 50.0 + matrix[0][1] * 20.0 + matrix[0][3];
    let y = matrix[1][0] * 50.0 + matrix[1][1] * 20.0 + matrix[1][3];
    assert!((x - 60.0).abs() < 0.001);
    assert!((y - 20.0).abs() < 0.001);
}
