use amazing_css::{compute, generated::ContentRun, parse_stylesheet, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn gradient_list_style_image_generates_an_image_marker() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let list = dom.create_element(root, "ul");
    let direct = dom.create_element(list, "li");
    let shorthand = dom.create_element(list, "li");
    dom.set_attrs(direct, vec![("class".into(), "direct".into())]);
    dom.set_attrs(shorthand, vec![("class".into(), "shorthand".into())]);
    let css = parse_stylesheet(
        "li.direct { list-style-image: linear-gradient(red, blue) } \
                                li.shorthand { list-style: repeating-linear-gradient(white, black) inside }",
    );
    let styled = compute(&dom, &[css]);
    let [ContentRun::Image(gradient)] = styled
        .generated_content(direct, PseudoElement::Marker)
        .unwrap()
        .runs
        .as_slice()
    else {
        panic!("missing gradient marker")
    };
    assert!(gradient.starts_with("linear-gradient("), "{gradient}");
    assert!(gradient.contains("red") && gradient.contains("blue"));
    let [ContentRun::Image(gradient)] = styled
        .generated_content(shorthand, PseudoElement::Marker)
        .unwrap()
        .runs
        .as_slice()
    else {
        panic!("missing shorthand gradient marker")
    };
    assert!(
        gradient.starts_with("repeating-linear-gradient("),
        "{gradient}"
    );
    assert_eq!(
        styled.get(shorthand).unwrap().get("list-style-position"),
        Some("inside")
    );
}

#[test]
fn invalid_image_function_falls_back_to_list_style_type() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let list = dom.create_element(root, "ul");
    let item = dom.create_element(list, "li");
    let styled = compute(
        &dom,
        &[parse_stylesheet(
            "ul { list-style-type: square; list-style-image: unsupported(red, blue) }",
        )],
    );
    assert_eq!(
        styled
            .generated_content(item, PseudoElement::Marker)
            .unwrap()
            .text(),
        "▪ "
    );
}
