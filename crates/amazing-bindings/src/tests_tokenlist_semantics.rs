use super::*;

#[test]
fn token_list_queries_and_updates_preserve_attribute_presence_and_order() {
    let mut h = Host::default();
    let node = element(&mut h);
    let list = call(&mut h, O::ElementClassList, V::Node(node), &[]);
    for value in ["", " ", "a b"] {
        assert_eq!(call(&mut h, O::TokenListContains, list.clone(), &[str_value(value)]), V::Bool(false));
        assert!(invoke(&mut h, O::TokenListSupports, list.clone(), &[str_value(value)])
            .unwrap_err().starts_with("TypeError"));
    }
    call(&mut h, O::TokenListRemove, list.clone(), &[str_value("missing")]);
    call(&mut h, O::TokenListAdd, list.clone(), &[]);
    assert!(h.dom.get_attribute_node(node, None, "class").is_none());
    for op in [O::TokenListAdd, O::TokenListRemove] {
        call(&mut h, O::TokenListSetValue, list.clone(), &[str_value("a  b a ")]);
        call(&mut h, op, list.clone(), &[]);
        assert_eq!(call(&mut h, O::TokenListValue, list.clone(), &[]), str_value("a b"));
    }
    for (original, old, new, expected) in [
        ("c b a", "c", "a", "a b"), ("a b c", "c", "a", "a b"),
        ("a a b", "a", "a", "a b"), ("a a b", "x", "y", "a a b")
    ] {
        call(&mut h, O::TokenListSetValue, list.clone(), &[str_value(original)]);
        call(&mut h, O::TokenListReplace, list.clone(), &[str_value(old), str_value(new)]);
        assert_eq!(call(&mut h, O::TokenListValue, list.clone(), &[]), str_value(expected));
    }
}

#[test]
fn token_list_converts_all_arguments_before_validation_and_prioritizes_empty_replacement() {
    let mut h = Host::default();
    let node = element(&mut h);
    let list = call(&mut h, O::ElementClassList, V::Node(node), &[]);
    assert_eq!(invoke(&mut h, O::TokenListReplace, list.clone(), &[str_value(" "), str_value("")]),
        Err("SyntaxError".into()));
    for op in [O::TokenListAdd, O::TokenListRemove, O::TokenListReplace] {
        h.conversions.clear();
        assert!(invoke(&mut h, op, list.clone(), &[str_value(""), V::Coerce("later")]).is_err());
        assert_eq!(h.conversions, ["later"]);
    }
}
