use super::*;
use amazing_dom::legacy_document::LegacyColor as C;

#[test]
fn legacy_color_conversion_precedes_missing_body_and_preserves_utf16() {
    let mut h = Host::default();
    assert_eq!(invoke(&mut h, O::DocumentLegacyColorSet(C::Text), V::Node(NodeIdx(0)), &[V::Throw]),
        Err("coercion sentinel".into()));
    call(&mut h, O::DocumentLegacyColorSet(C::Text), V::Node(NodeIdx(0)), &[V::Coerce("ignored")]);
    assert_eq!(h.conversions, ["ignored"]);
    assert_eq!(call(&mut h, O::DocumentLegacyColorGet(C::Text), V::Node(NodeIdx(0)), &[]), str_value(""));
    let root = h.dom.create_element(NodeIdx(0), "html");
    let body = h.dom.create_element(root, "body");
    for color in [C::Text, C::Background, C::Link, C::VisitedLink, C::ActiveLink] {
        call(&mut h, O::DocumentLegacyColorSet(color), V::Node(NodeIdx(0)), &[V::Null]);
        assert_eq!(call(&mut h, O::BodyLegacyColorGet(color), V::Node(body), &[]), str_value(""));
        call(&mut h, O::BodyLegacyColorSet(color), V::Node(body), &[V::Undefined]);
        assert_eq!(call(&mut h, O::DocumentLegacyColorGet(color), V::Node(NodeIdx(0)), &[]), str_value("undefined"));
        let value = V::String(vec![0xd800, 65]);
        call(&mut h, O::DocumentLegacyColorSet(color), V::Node(NodeIdx(0)), &[value.clone()]);
        assert_eq!(call(&mut h, O::BodyLegacyColorGet(color), V::Node(body), &[]), value);
    }
    h.conversions.clear();
    assert!(invoke(&mut h, O::BodyLegacyColorSet(C::Text), V::Node(root), &[V::Coerce("must not run")]).is_err());
    assert!(h.conversions.is_empty());
    assert_eq!(call(&mut h, O::DocumentLegacyNoop, V::Node(NodeIdx(0)), &[V::Throw]), V::Undefined);
    assert!(invoke(&mut h, O::DocumentLegacyNoop, V::Node(body), &[]).is_err());
}
