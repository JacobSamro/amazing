use amazing_css::selectors::{matches, parse_selector};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn selectors_four_language_ranges_use_extended_filtering() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let french = dom.create_element(root, "span");
    dom.set_attrs(french, vec![("lang".into(), "fr-Latn-FR".into())]);
    let swiss = dom.create_element(root, "span");
    dom.set_attrs(swiss, vec![("lang".into(), "fr-CH".into())]);
    let extension = dom.create_element(root, "span");
    dom.set_attrs(extension, vec![("lang".into(), "fr-x-FR".into())]);

    for selector in [":lang(\"fr-FR\")", ":lang(\"*-FR\")", ":lang(\\*-FR)"] {
        assert!(
            matches(&dom, french, &parse_selector(selector).unwrap()),
            "{selector}"
        );
        assert!(
            !matches(&dom, swiss, &parse_selector(selector).unwrap()),
            "{selector}"
        );
        assert!(
            !matches(&dom, extension, &parse_selector(selector).unwrap()),
            "{selector}"
        );
    }
    assert!(matches(
        &dom,
        swiss,
        &parse_selector(":lang(\"*-CH\")").unwrap()
    ));
    assert!(matches(
        &dom,
        french,
        &parse_selector(":lang(\"*-Latn\")").unwrap()
    ));
    assert!(matches(
        &dom,
        french,
        &parse_selector(":lang(\"*\")").unwrap()
    ));
    assert!(!matches(
        &dom,
        french,
        &parse_selector(":lang(\"åå\")").unwrap()
    ));
}
