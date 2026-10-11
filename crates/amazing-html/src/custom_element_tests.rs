use std::cell::RefCell;
use std::rc::Rc;

use amazing_dom::custom_elements::{CustomElementDefinition, CustomElementReaction, CustomElementState};
use amazing_dom::events::CallbackId;
use amazing_dom::{Dom, Namespace, NodeIdx};

use crate::{parse, parse_fragment, parse_with_custom_element_host, ParserConstruction, ParserCustomElementHost};

#[derive(Default)]
struct Trace { calls: Vec<(String, NodeIdx, Option<String>, CustomElementState)>, fail: Option<String> }
struct MockHost(Rc<RefCell<Trace>>);
fn mock_create(dom: &mut Dom, request: ParserConstruction, mut construct: impl FnMut(&mut Dom, NodeIdx, usize) -> bool)
    -> Result<amazing_dom::render_mutation::NodeGeneration, amazing_dom::DomError> {
    use amazing_dom::custom_elements::{ElementCreationPlan as Plan, CustomElementCreationCompletion as Completion};
    let plan = dom.prepare_custom_element_creation(request.document, Some("http://www.w3.org/1999/xhtml"), None,
        &request.local_name, request.is_value.as_deref(), true)?;
    let selected = match plan {
        Plan::Element(owner) => Ok(owner),
        Plan::Upgrade { element, definition, .. } => {
            dom.begin_custom_element_upgrade(element.node, definition)?;
            let success = construct(dom, element.node, definition);
            // The fixture's actual super consumes the existing upgrade stack.
            let _ = dom.finish_custom_element_upgrade(element.node, definition, success);
            Ok(element)
        }
        Plan::Autonomous(context) => {
            let cap = dom.prepare_html_constructor_in_document(context.document(), context.document(), context.definition(), context.constructor())?;
            let owner = cap.element(); dom.consume_html_constructor(cap)?;
            let returned = construct(dom, owner.node, context.definition()).then_some(owner);
            match dom.complete_custom_element_creation(context, returned)? {
                Completion::Created(owner) => Ok(owner),
                Completion::Failed(failure) => dom.finish_custom_element_creation_failure(failure),
            }
        }
    }?;
    for (name, value) in request.attributes { dom.set_attribute(selected.node, &name, &value)?; }
    Ok(selected)
}
impl ParserCustomElementHost for MockHost {
    fn create(&mut self, dom: &mut Dom, request: ParserConstruction)
        -> Result<amazing_dom::render_mutation::NodeGeneration, amazing_dom::DomError> {
        let local = request.local_name.clone();
        mock_create(dom, request, |dom, element, definition| {
            assert_eq!(dom.name(element), Some(local.as_str()));
            assert_eq!(dom.get(element).parent, None);
            assert_eq!(dom.custom_element_definition_of(element), Some(definition));
            let attribute = dom.get_attribute_ns(element, None, "data-k").map(str::to_owned);
            assert!(dom.children(element).next().is_none());
            let state = dom.custom_element_state(element);
            self.0.borrow_mut().calls.push((local.clone(), element, attribute, state));
            let fail = self.0.borrow().fail.as_deref() == Some(local.as_str());
            if state == CustomElementState::Precustomized { assert_eq!(dom.custom_element_super(definition), Ok(element)); }
            !fail
        })
    }
}

fn define(dom: &mut Dom, name: &str, local: &str, constructor: u64) -> usize {
    let mut definition = CustomElementDefinition::new(NodeIdx(0), name.into(), local.into(), CallbackId(constructor));
    definition.callbacks.connected = Some(CallbackId(constructor + 100));
    definition.callbacks.disconnected = Some(CallbackId(constructor + 200));
    definition.callbacks.attribute_changed = Some(CallbackId(constructor + 300));
    definition.observed_attributes.push("data-k".into());
    dom.define_custom_element(definition).unwrap()
}
fn host(trace: &Rc<RefCell<Trace>>) -> Box<dyn ParserCustomElementHost> {
    Box::new(MockHost(trace.clone()))
}
fn element(dom: &Dom, name: &str) -> NodeIdx {
    (0..dom.node_count()).map(|n| NodeIdx(n as u32))
        .find(|&idx| dom.name(idx) == Some(name) && dom.get(idx).parent.is_some()).unwrap()
}

#[test]
fn defined_nested_elements_construct_before_children_with_attributes_and_reactions() {
    let source = "<x-card data-k=outer><x-leaf data-k=inner></x-leaf></x-card>";
    let before = parse(source);
    assert_eq!(before.dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <x-card data-k=\"outer\">\n        <x-leaf data-k=\"inner\">\n");
    let mut dom = Dom::new();
    let card_def = define(&mut dom, "x-card", "x-card", 1);
    let leaf_def = define(&mut dom, "x-leaf", "x-leaf", 2);
    let trace = Rc::new(RefCell::new(Trace::default()));
    dom = parse_with_custom_element_host(source, dom, host(&trace));
    assert_eq!(dom.dump(NodeIdx(0)), before.dump(NodeIdx(0)));
    let card = element(&dom, "x-card");
    let leaf = element(&dom, "x-leaf");
    assert_eq!(trace.borrow().calls, vec![
        ("x-card".into(), card, None, CustomElementState::Custom),
        ("x-leaf".into(), leaf, None, CustomElementState::Custom),
    ]);
    assert_eq!(dom.custom_element_state(card), CustomElementState::Custom);
    assert_eq!(dom.custom_element_state(leaf), CustomElementState::Custom);
    assert_eq!(dom.custom_element_definition_of(card), Some(card_def));
    assert_eq!(dom.custom_element_definition_of(leaf), Some(leaf_def));
    assert_eq!(dom.take_custom_element_reactions(), vec![
        CustomElementReaction::AttributeChanged { element: card, callback: CallbackId(301),
            name: "data-k".into(), old_value: None, new_value: Some("outer".encode_utf16().collect()), namespace: None },
        CustomElementReaction::Connected { element: card, callback: CallbackId(101) },
        CustomElementReaction::AttributeChanged { element: leaf, callback: CallbackId(302),
            name: "data-k".into(), old_value: None, new_value: Some("inner".encode_utf16().collect()), namespace: None },
        CustomElementReaction::Connected { element: leaf, callback: CallbackId(102) },
    ]);
    dom.detach(card);
    assert_eq!(dom.take_custom_element_reactions(), vec![
        CustomElementReaction::Disconnected { element: card, callback: CallbackId(201) },
        CustomElementReaction::Disconnected { element: leaf, callback: CallbackId(202) },
    ]);
}

#[test]
fn undefined_and_customized_built_in_paths_remain_distinct() {
    let late_src = "<x-late data-k=late></x-late>";
    let mut late = parse(late_src);
    assert_eq!(late.dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <x-late data-k=\"late\">\n");
    let late_node = element(&late, "x-late");
    assert_eq!(late.custom_element_state(late_node), CustomElementState::Undefined);
    let late_id = define(&mut late, "x-late", "x-late", 5);
    assert_eq!(late.take_custom_element_reactions(), vec![
        CustomElementReaction::Upgrade { element: late_node, definition: late_id },
    ]);

    let src = "<button is=x-fancy data-k=v>ok</button>";
    let before = parse(src);
    assert_eq!(before.dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <button is=\"x-fancy\" data-k=\"v\">\n        \"ok\"\n");
    let mut dom = Dom::new();
    let fancy_id = define(&mut dom, "x-fancy", "button", 8);
    let trace = Rc::new(RefCell::new(Trace::default()));
    dom = parse_with_custom_element_host(src, dom, host(&trace));
    let button = element(&dom, "button");
    assert_eq!(dom.dump(NodeIdx(0)), before.dump(NodeIdx(0)));
    assert_eq!(trace.borrow().calls[0].0, "button");
    assert_eq!(trace.borrow().calls[0].2.as_deref(), None);
    assert_eq!(dom.custom_element_state(button), CustomElementState::Custom);
    assert_eq!(dom.custom_element_definition_of(button), Some(fancy_id));
    assert!(!dom.take_custom_element_reactions().iter().any(|reaction|
        matches!(reaction, CustomElementReaction::Upgrade { .. })));

    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut no_match = Dom::new();
    define(&mut no_match, "x-fancy", "button", 8);
    no_match = parse_with_custom_element_host("<button is=x-other>ok</button>", no_match, host(&trace));
    assert!(trace.borrow().calls.is_empty());
    assert_eq!(no_match.custom_element_state(element(&no_match, "button")), CustomElementState::Undefined);
}

#[test]
fn failed_construction_does_not_stop_parser_or_queue_lifecycle() {
    let source = "<x-bad data-k=broken></x-bad><x-good data-k=ok></x-good><p>after</p>";
    let before = parse(source);
    assert_eq!(before.dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <x-bad data-k=\"broken\">\n      <x-good data-k=\"ok\">\n      <p>\n        \"after\"\n");
    let mut dom = Dom::new();
    define(&mut dom, "x-bad", "x-bad", 3);
    define(&mut dom, "x-good", "x-good", 4);
    let trace = Rc::new(RefCell::new(Trace { fail: Some("x-bad".into()), ..Trace::default() }));
    dom = parse_with_custom_element_host(source, dom, host(&trace));
    assert_eq!(dom.dump(NodeIdx(0)), before.dump(NodeIdx(0)));
    assert_eq!(dom.custom_element_state(element(&dom, "x-bad")), CustomElementState::Failed);
    assert_eq!(dom.custom_element_state(element(&dom, "x-good")), CustomElementState::Custom);
    assert_eq!(trace.borrow().calls.len(), 2);
    let bad = element(&dom, "x-bad");
    assert!(dom.take_custom_element_reactions().iter().all(|reaction| match reaction {
        CustomElementReaction::Connected { element, .. }
        | CustomElementReaction::AttributeChanged { element, .. } => *element != bad,
        _ => true,
    }));
}

#[test]
fn ordinary_parse_and_fragment_remain_unchanged() {
    let source = "<div><span>A</span></div>";
    let trace = Rc::new(RefCell::new(Trace::default()));
    let dom = parse_with_custom_element_host(source, Dom::new(), host(&trace));
    assert_eq!(dom.dump(NodeIdx(0)), parse(source).dump(NodeIdx(0)));
    assert!(trace.borrow().calls.is_empty());
    assert_eq!(parse_fragment("div", "<span>A</span>").unwrap().dump(NodeIdx(0)),
        "#document\n  <span>\n    \"A\"\n");
}

#[test]
fn host_can_mutate_dom_during_constructor_without_a_parser_borrow() {
    struct MutatingHost;
    impl ParserCustomElementHost for MutatingHost {
        fn create(&mut self, dom: &mut Dom, request: ParserConstruction)
            -> Result<amazing_dom::render_mutation::NodeGeneration, amazing_dom::DomError> {
            mock_create(dom, request, |dom, element, definition| {
                dom.set_attribute(element, "constructed", "yes").unwrap();
                let scratch = dom.create_element(NodeIdx(0), "aside");
                dom.detach(scratch);
                if dom.custom_element_state(element) == CustomElementState::Precustomized {
                    assert_eq!(dom.custom_element_super(definition), Ok(element));
                }
                true
            })
        }
    }
    let mut dom = Dom::new();
    define(&mut dom, "x-edit", "x-edit", 20);
    let dom = parse_with_custom_element_host("<x-edit></x-edit>", dom, Box::new(MutatingHost));
    let selected = element(&dom, "x-edit");
    let held = (0..dom.node_count()).map(|n| NodeIdx(n as u32)).find(|&n|
        dom.name(n) == Some("x-edit") && dom.get_attribute_ns(n, None, "constructed") == Some("yes")).unwrap();
    assert_ne!(held, selected);
    assert_eq!(dom.get_attribute_ns(held, None, "constructed"), Some("yes"));
    assert_eq!(dom.custom_element_state(held), CustomElementState::Custom);
    assert_eq!(dom.custom_element_state(selected), CustomElementState::Failed);
    assert!(dom.get_attribute_ns(selected, None, "constructed").is_none());
    // The same authored mutation succeeds for a genuine customized upgrade.
    let mut dom = Dom::new(); define(&mut dom, "x-edit-div", "div", 21);
    let dom = parse_with_custom_element_host("<div is=x-edit-div></div>", dom, Box::new(MutatingHost));
    let selected = element(&dom, "div");
    assert_eq!(dom.get_attribute_ns(selected, None, "constructed"), Some("yes"));
    assert_eq!(dom.custom_element_state(selected), CustomElementState::Custom);
}

#[test]
fn template_contents_use_inert_owner_registry_and_delay_upgrade() {
    let mut dom = Dom::new();
    define(&mut dom, "x-card", "x-card", 40);
    let trace = Rc::new(RefCell::new(Trace::default()));
    let dom = parse_with_custom_element_host(
        "<template><x-card data-k=inert></x-card></template><x-card data-k=live></x-card>",
        dom, host(&trace));
    let cards: Vec<_> = (0..dom.node_count()).map(|n| NodeIdx(n as u32))
        .filter(|&node| dom.name(node) == Some("x-card")).collect();
    assert_eq!(cards.len(), 2);
    assert_eq!(trace.borrow().calls.len(), 1);
    assert_eq!(dom.custom_element_state(cards[0]), CustomElementState::Undefined);
    assert_ne!(dom.owner_document(cards[0]), Some(NodeIdx(0)));
    assert_eq!(dom.custom_element_state(cards[1]), CustomElementState::Custom);
}

#[test]
fn streamed_constructor_runs_before_token_attributes_and_children() {
    let mut dom = Dom::new();
    define(&mut dom, "x-card", "x-card", 50);
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut parser = crate::IncrementalParser::with_custom_element_host(dom, host(&trace));
    for chunk in ["<x-ca", "rd data-", "k=value", "><span>", "child</span></x-card>"] {
        parser.push_chunk(chunk);
    }
    let mut dom = parser.finish();
    let card = element(&dom, "x-card");
    assert_eq!(trace.borrow().calls, vec![("x-card".into(), card, None,
        CustomElementState::Custom)]);
    assert_eq!(dom.get_attribute_ns(card, None, "data-k"), Some("value"));
    assert!(dom.dump(NodeIdx(0)).contains("<span>"));
    assert!(matches!(dom.take_custom_element_reactions().as_slice(),
        [CustomElementReaction::AttributeChanged { .. }, CustomElementReaction::Connected { .. }]));
}

#[test]
fn scoped_creation_releases_shared_parser_borrow_and_retains_terminal_failure() {
    struct Reject { parser: Rc<RefCell<crate::ResumableParser>>, calls: usize }
    impl crate::ParserElementCreationHost for Reject {
        fn create(&mut self, _: &mut Dom, request: crate::ParserElementCreation)
            -> Result<amazing_dom::render_mutation::NodeGeneration, amazing_dom::DomError> {
            assert_eq!(request.local_name, "x-rejected");
            // The actual persistent parser borrow is absent at this callback.
            assert!(self.parser.try_borrow_mut().is_ok());
            self.calls += 1;
            Err(amazing_dom::DomError::InvalidStateError)
        }
    }
    let parser = Rc::new(RefCell::new(crate::ResumableParser::new(Dom::new())));
    parser.borrow_mut().push_source("<body><x-rejected></x-rejected><p>never</p>");
    let mut host = Reject { parser: parser.clone(), calls: 0 };
    assert_eq!(crate::ResumableParser::run_with_creation_host(&parser, &mut host), Err(amazing_dom::DomError::InvalidStateError));
    assert_eq!(crate::ResumableParser::run_with_creation_host(&parser, &mut host), Err(amazing_dom::DomError::InvalidStateError));
    assert_eq!(host.calls, 1);
    assert!(!parser.borrow().dom().dump(NodeIdx(0)).contains("never"));
}

#[test]
fn retired_scoped_parser_restores_actual_frame_without_consuming_later_tokens() {
    struct Retire { parser: Rc<RefCell<crate::ResumableParser>> }
    impl crate::ParserElementCreationHost for Retire {
        fn create(&mut self, dom: &mut Dom, request: crate::ParserElementCreation)
            -> Result<amazing_dom::render_mutation::NodeGeneration, amazing_dom::DomError> {
            let node = mock_create(dom, request, |_, _, _| true)?;
            assert!(!self.parser.borrow().execution_frame_available());
            self.parser.borrow_mut().abort();
            Ok(node)
        }
    }
    let parser = Rc::new(RefCell::new(crate::ResumableParser::new(Dom::new())));
    parser.borrow_mut().push_source("<body><x-retired></x-retired><p>later input</p>");
    let mut host = Retire { parser: parser.clone() };
    assert_eq!(crate::ResumableParser::run_with_creation_host(&parser, &mut host), Ok(crate::ParseSignal::Aborted));
    assert!(parser.borrow().execution_frame_available());
    assert!(!parser.borrow().dom().dump(NodeIdx(0)).contains("later input"));
    assert_eq!(crate::ResumableParser::run_with_creation_host(&parser, &mut host), Ok(crate::ParseSignal::Aborted));
}
