use amazing_dom::shadow::{ShadowMode, SlotAssignment};
use amazing_dom::{Dom, NodeIdx};
use crate::{parse, parse_fragment, parse_fragment_with_shadow_roots, IncrementalParser};

fn by_id(dom: &Dom, id: &str) -> NodeIdx {
    (0..dom.node_count()).map(|n| NodeIdx(n as u32))
        .find(|&node| dom.get_attribute_ns(node, None, "id") == Some(id)).unwrap()
}
fn html(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}

#[test]
fn declarative_template_attaches_to_host_and_keeps_light_dom_separate() {
    let dom = parse("<div id=host><template shadowrootmode=open><span>shadow</span></template><b>light</b></div>");
    let host = by_id(&dom, "host");
    let root = dom.shadow_root_of_host(host).unwrap();
    let data = dom.shadow_data(root).unwrap();
    assert!(data.declarative && data.available_to_element_internals);
    assert!(!data.keep_custom_element_registry_null);
    assert_eq!(dom.shadow_data(root).unwrap().options.mode, ShadowMode::Open);
    assert_eq!(html(&dom, root), "<span>shadow</span>");
    assert_eq!(html(&dom, host), "<b>light</b>");
    assert_eq!(dom.visible_shadow_root(host), Some(root));
}

#[test]
fn declarative_options_closed_root_and_nested_attachment() {
    let dom = parse("<div id=host><template shadowrootmode=CLOSED shadowrootdelegatesfocus shadowrootserializable shadowrootclonable shadowrootslotassignment=manual><span id=inner><template shadowrootmode=open><b>deep</b></template></span></template></div>");
    let host = by_id(&dom, "host");
    let root = dom.shadow_root_of_host(host).unwrap();
    let options = dom.shadow_data(root).unwrap().options;
    assert!(dom.shadow_data(root).unwrap().declarative);
    assert_eq!(options.mode, ShadowMode::Closed);
    assert_eq!(options.slot_assignment, SlotAssignment::Manual);
    assert!(options.delegates_focus && options.serializable && options.clonable);
    assert_eq!(dom.visible_shadow_root(host), None);
    let inner = by_id(&dom, "inner");
    assert_eq!(dom.get(inner).parent, Some(root));
    let nested = dom.shadow_root_of_host(inner).unwrap();
    assert_eq!(html(&dom, nested), "<b>deep</b>");
}

#[test]
fn invalid_duplicate_and_fragment_declarations_stay_templates() {
    let dom = parse("<div id=host><template shadowrootmode=invalid><b>inert</b></template></div>");
    let host = by_id(&dom, "host");
    assert_eq!(dom.shadow_root_of_host(host), None);
    assert!(html(&dom, host).contains("<template shadowrootmode=\"invalid\"><b>inert</b></template>"));

    let dom = parse("<div id=host><template shadowrootmode=open><b>first</b></template><template shadowrootmode=closed><b>second</b></template></div>");
    let host = by_id(&dom, "host");
    assert_eq!(html(&dom, dom.shadow_root_of_host(host).unwrap()), "<b>first</b>");
    assert!(html(&dom, host).contains("<template shadowrootmode=\"closed\"><b>second</b></template>"));

    let fragment = parse_fragment("div", "<div id=host><template shadowrootmode=open><b>inert</b></template></div>").unwrap();
    let host = by_id(&fragment, "host");
    assert_eq!(fragment.shadow_root_of_host(host), None);
    assert!(html(&fragment, host).contains("<template shadowrootmode=\"open\">"));
}

#[test]
fn declarative_shadow_recovers_at_eof_and_chunk_boundaries() {
    let source = "<div id=host><template shadowrootmode=open><span>split</span>";
    let expected = parse(source);
    let mut stream = IncrementalParser::new();
    for chunk in source.as_bytes().chunks(2) {
        stream.push_chunk(std::str::from_utf8(chunk).unwrap());
    }
    let streamed = stream.finish();
    let expected_root = expected.shadow_root_of_host(by_id(&expected, "host")).unwrap();
    let streamed_root = streamed.shadow_root_of_host(by_id(&streamed, "host")).unwrap();
    assert_eq!(html(&expected, expected_root), "<span>split</span>");
    assert_eq!(html(&streamed, streamed_root), html(&expected, expected_root));
}

#[test]
fn parser_sets_registry_null_and_dom_owned_shadow_flags() {
    let source = "<div id=host><template shadowrootmode=open shadowrootcustomelementregistry shadowrootserializable shadowrootclonable shadowrootdelegatesfocus shadowrootslotassignment=manual><b>shadow</b></template><i>light</i></div>";
    let dom = parse(source);
    let host = by_id(&dom, "host");
    let root = dom.shadow_root_of_host(host).unwrap();
    let data = dom.shadow_data(root).unwrap();
    assert!(data.declarative && data.available_to_element_internals);
    assert!(data.keep_custom_element_registry_null);
    assert_eq!(data.options.mode, ShadowMode::Open);
    assert_eq!(data.options.slot_assignment, SlotAssignment::Manual);
    assert!(data.options.delegates_focus && data.options.clonable && data.options.serializable);
    assert_eq!(html(&dom, root), "<b>shadow</b>");
    assert_eq!(html(&dom, host), "<i>light</i>");
    let serialized = String::from_utf16(&dom.serialize_children_with_shadow(host, true, &[]).unwrap()).unwrap();
    assert_eq!(serialized, "<template shadowrootmode=\"open\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\"><b>shadow</b></template><i>light</i>");
}

#[test]
fn closed_and_nonserializable_roots_use_dom_serialization_contract() {
    let dom = parse("<div id=host><template shadowrootmode=closed shadowrootcustomelementregistry><b>hidden</b></template><i>light</i></div>");
    let host = by_id(&dom, "host");
    let root = dom.shadow_root_of_host(host).unwrap();
    assert_eq!(dom.visible_shadow_root(host), None);
    let data = dom.shadow_data(root).unwrap();
    assert!(data.declarative && data.available_to_element_internals);
    assert!(data.keep_custom_element_registry_null);
    assert!(!data.options.serializable);
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, true, &[]).unwrap()).unwrap(),
        "<i>light</i>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, false, &[root]).unwrap()).unwrap(),
        "<template shadowrootmode=\"closed\" shadowrootcustomelementregistry=\"\"><b>hidden</b></template><i>light</i>");
}

#[test]
fn nested_duplicate_and_fragment_registry_declarations_keep_their_places() {
    let source = "<div id=host><template shadowrootmode=open><span id=inner><template shadowrootmode=closed shadowrootcustomelementregistry shadowrootserializable><b>deep</b></template></span></template><template shadowrootmode=open shadowrootcustomelementregistry><b>inert</b></template></div>";
    let dom = parse(source);
    let host = by_id(&dom, "host");
    let outer = dom.shadow_root_of_host(host).unwrap();
    let inner = by_id(&dom, "inner");
    let nested = dom.shadow_root_of_host(inner).unwrap();
    assert!(dom.shadow_data(outer).unwrap().declarative);
    let nested_data = dom.shadow_data(nested).unwrap();
    assert!(nested_data.declarative && nested_data.available_to_element_internals);
    assert!(nested_data.keep_custom_element_registry_null);
    assert_eq!(nested_data.options.mode, ShadowMode::Closed);
    assert!(html(&dom, host).contains("<template shadowrootmode=\"open\" shadowrootcustomelementregistry=\"\"><b>inert</b></template>"));

    let fragment = parse_fragment("div", "<div id=host><template shadowrootmode=open shadowrootcustomelementregistry><b>inert</b></template></div>").unwrap();
    let fragment_host = by_id(&fragment, "host");
    assert_eq!(fragment.shadow_root_of_host(fragment_host), None);
    assert!(html(&fragment, fragment_host).contains("<template shadowrootmode=\"open\" shadowrootcustomelementregistry=\"\">"));
}

#[test]
fn shadow_roots_fragment_parser_attaches_nested_declarative_templates() {
    // setHTMLUnsafe/parseHTMLUnsafe's fragment entry point, gated separately
    // from the always-off `parse_fragment`/`parse_fragment_with_namespace`
    // (see `invalid_duplicate_and_fragment_declarations_stay_templates`
    // above, whose `parse_fragment("div", ...)` case with this exact markup
    // stays an inert template). A nested ancestor (`<div id=host>`, itself
    // ordinary content of the "body" fragment context) is a real, already-
    // open element within the fragment, so it's a valid shadow host and the
    // shadow content -- and its clone into the standalone fragment `Dom` --
    // both work exactly as they would in a full document parse.
    let fragment = parse_fragment_with_shadow_roots(
        "body",
        "<div id=host><template shadowrootmode=open><span>shadow</span></template><b>light</b></div>",
    ).unwrap();
    let host = by_id(&fragment, "host");
    let root = fragment.shadow_root_of_host(host).unwrap();
    let data = fragment.shadow_data(root).unwrap();
    assert!(data.declarative);
    assert_eq!(data.options.mode, ShadowMode::Open);
    assert_eq!(html(&fragment, root), "<span>shadow</span>");
    assert_eq!(html(&fragment, host), "<b>light</b>");
}

#[test]
fn shadow_roots_fragment_parser_still_leaves_a_top_level_template_as_a_leftover() {
    // The fragment parser's synthetic root standing in for the context
    // element is never a valid shadow host, and this function has no real
    // host `NodeIdx` to attach to even if it were -- only the caller does
    // (see this function's own doc comment). A template with no real
    // element ancestor already open in the fragment -- `setHTMLUnsafe`'s
    // single most common shape, `el.setHTMLUnsafe('<template
    // shadowrootmode="open">...')` -- comes back as an ordinary top-level
    // template child for the binding layer to detect and re-home itself.
    let fragment = parse_fragment_with_shadow_roots(
        "div",
        "<template shadowrootmode=open><span>shadow</span></template><b>light</b>",
    ).unwrap();
    assert_eq!(
        fragment.dump(NodeIdx(0)),
        "#document\n  <template shadowrootmode=\"open\">\n    content\n      <span>\n        \"shadow\"\n  <b>\n    \"light\"\n",
    );
}

#[test]
fn registry_null_attachment_survives_every_scalar_chunk_boundary_and_eof() {
    let source = "<div id=host><template shadowrootmode=closed shadowrootcustomelementregistry shadowrootserializable><span>split</span>";
    let expected = parse(source);
    let expected_host = by_id(&expected, "host");
    let expected_root = expected.shadow_root_of_host(expected_host).unwrap();
    let expected_serialized = String::from_utf16(&expected.serialize_children_with_shadow(expected_host, true, &[]).unwrap()).unwrap();
    assert!(expected.shadow_data(expected_root).unwrap().keep_custom_element_registry_null);
    for split in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut parser = IncrementalParser::new();
        parser.push_chunk(&source[..split]);
        parser.push_chunk(&source[split..]);
        let dom = parser.finish();
        let host = by_id(&dom, "host");
        let root = dom.shadow_root_of_host(host).unwrap();
        assert!(dom.shadow_data(root).unwrap().declarative, "split at {split}");
        assert!(dom.shadow_data(root).unwrap().keep_custom_element_registry_null, "split at {split}");
        assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, true, &[]).unwrap()).unwrap(), expected_serialized,
            "split at {split}");
    }
}
