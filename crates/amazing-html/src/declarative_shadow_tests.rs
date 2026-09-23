use amazing_dom::shadow::{ShadowMode, SlotAssignment};
use amazing_dom::{Dom, NodeIdx};
use crate::{parse, parse_fragment, IncrementalParser};

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
