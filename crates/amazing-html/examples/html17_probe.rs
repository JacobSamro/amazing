use amazing_dom::custom_elements::{CustomElementDefinition, CustomElementReaction, CustomElementState};
use amazing_dom::events::CallbackId;
use amazing_dom::shadow::{ShadowMode, SlotAssignment};
use amazing_dom::{Dom, NodeIdx};
use amazing_html::{parse, parse_fragment, parse_with_custom_element_host, ParserConstruction, ParserCustomElementHost};
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

#[derive(Default)]
struct Calls(Vec<(String, usize, bool, NodeIdx)>);
struct Host(Rc<RefCell<Calls>>);
impl ParserCustomElementHost for Host {
    fn construct(&mut self, dom: &mut Dom, request: ParserConstruction) -> bool {
        let attributes = dom.attrs_of(request.element).map_or(0, |a| a.len());
        let detached = dom.get(request.element).parent.is_none();
        let owner = dom.owner_document(request.element).unwrap();
        self.0.borrow_mut().0.push((request.local_name.clone(), attributes, detached, owner));
        if request.local_name == "x-bad" { return false; }
        dom.custom_element_super(request.definition) == Ok(request.element)
    }
}
fn define(dom: &mut Dom, name: &str, callback: u64) {
    let mut definition = CustomElementDefinition::new(NodeIdx(0), name.into(), name.into(), CallbackId(callback));
    definition.observed_attributes.push("data-k".into());
    definition.callbacks.attribute_changed = Some(CallbackId(callback + 100));
    definition.callbacks.connected = Some(CallbackId(callback + 200));
    dom.define_custom_element(definition).unwrap();
}
fn find(dom: &Dom, name: &str) -> Vec<NodeIdx> {
    (0..dom.node_count()).map(|n| NodeIdx(n as u32)).filter(|&n| dom.name(n) == Some(name)).collect()
}
fn id(dom: &Dom, value: &str) -> NodeIdx {
    (0..dom.node_count()).map(|n| NodeIdx(n as u32))
        .find(|&n| dom.get_attribute_ns(n, None, "id") == Some(value)).expect("id")
}
fn html(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}
fn fnv(data: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in data.bytes() { hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3); }
    format!("{hash:016x}")
}
fn check(case: &str, source: &str) -> (bool, String) {
    if case.starts_with("ce-") {
        let mut dom = Dom::new();
        define(&mut dom, "x-card", 1);
        if case == "ce-failure" { define(&mut dom, "x-bad", 2); }
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut dom = parse_with_custom_element_host(source, dom, Box::new(Host(calls.clone())));
        let seen = calls.borrow().0.clone();
        let cards = find(&dom, "x-card");
        let card = *cards.last().unwrap();
        let reactions = dom.take_custom_element_reactions();
        let pass = match case {
            "ce-empty" => {
                seen == [("x-card".into(), 0, true, NodeIdx(0))]
                    && dom.get_attribute_ns(card, None, "data-k") == Some("outer")
                    && html(&dom, card).contains("<span>child</span>")
                    && matches!(reactions.as_slice(),
                        [CustomElementReaction::AttributeChanged { .. }, CustomElementReaction::Connected { .. }])
            },
            "ce-owner" => {
                let inert = cards[0];
                seen == [("x-card".into(), 0, true, NodeIdx(0))]
                    && dom.custom_element_state(inert) == CustomElementState::Undefined
                    && dom.owner_document(inert) != Some(NodeIdx(0))
                    && dom.custom_element_state(card) == CustomElementState::Custom
            },
            "ce-failure" => {
                let bad = find(&dom, "x-bad")[0];
                seen.len() == 2 && seen.iter().all(|(_, attrs, detached, _)| *attrs == 0 && *detached)
                    && dom.custom_element_state(bad) == CustomElementState::Failed
                    && dom.custom_element_state(card) == CustomElementState::Custom
            },
            _ => false,
        };
        return (pass, format!("{seen:?}|{reactions:?}|{}", dom.dump(NodeIdx(0))));
    }
    let dom = if case == "dsd-fragment" { parse_fragment("div", source).unwrap() } else { parse(source) };
    let host = id(&dom, "host");
    let root = dom.shadow_root_of_host(host);
    let light = html(&dom, host);
    let shadow = root.map(|n| html(&dom, n)).unwrap_or_default();
    let templates = dom.children(host).filter(|&n| dom.name(n) == Some("template")).count();
    let pass = match case {
        "wpt-dsd-basic" => root.is_some() && templates == 0 && shadow.contains("<slot") && light.contains("id=\"c1\""),
        "dsd-closed" => root.is_some_and(|r| {
            let opts = dom.shadow_data(r).unwrap().options;
            opts.mode == ShadowMode::Closed && opts.delegates_focus && opts.clonable
                && opts.serializable && opts.slot_assignment == SlotAssignment::Manual
        }) && templates == 0 && shadow.contains("<b>shadow</b>") && light.contains("<i>light</i>"),
        "dsd-invalid" | "dsd-fragment" => root.is_none() && templates == 1 && light.contains("<template"),
        "dsd-repeated" => root.is_some() && templates == 1 && shadow.contains("<b>first</b>") && light.contains("<b>second</b>"),
        "dsd-nested" => root.is_some_and(|r| {
            let inner = dom.children(r).find(|&n| dom.get_attribute_ns(n, None, "id") == Some("inner"));
            inner.and_then(|n| dom.shadow_root_of_host(n)).is_some_and(|nested| html(&dom, nested).contains("<b>deep</b>"))
        }),
        _ => false,
    };
    (pass, format!("root={root:?}|templates={templates}|shadow={shadow}|light={light}"))
}
fn main() {
    let directory = std::env::args().nth(1).expect("evidence directory");
    for line in include_str!("../evidence/html17/selection.tsv").lines().skip(1) {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 4);
        let source = fs::read_to_string(format!("{directory}/{}", columns[1])).unwrap();
        let (pass, signature) = check(columns[0], &source);
        println!("{}\t{}\t{}", columns[0], if pass { "pass" } else { "fail" }, fnv(&signature));
    }
}
