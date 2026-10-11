//! DOM create-element: synchronous autonomous construction has no stack seed.
use super::*;
use crate::render_mutation::NodeGeneration;

#[derive(Debug)]
pub struct AutonomousCreationContext {
    arena: std::sync::Arc<()>,
    document: NodeGeneration,
    definition: usize,
    constructor: CallbackId,
    local_name: String,
    prefix: Option<String>,
}
impl AutonomousCreationContext {
    pub fn document(&self) -> NodeGeneration { self.document }
    pub fn definition(&self) -> usize { self.definition }
    pub fn constructor(&self) -> CallbackId { self.constructor }
}
#[derive(Debug)]
pub enum ElementCreationPlan {
    Element(NodeGeneration),
    Upgrade { element: NodeGeneration, definition: usize, constructor: CallbackId },
    Autonomous(AutonomousCreationContext),
}
#[derive(Debug)]
pub struct CustomElementCreationFailure { context: AutonomousCreationContext, reason: DomError }
impl CustomElementCreationFailure { pub fn reason(&self) -> DomError { self.reason } }
#[derive(Debug)]
pub enum CustomElementCreationCompletion { Created(NodeGeneration), Failed(CustomElementCreationFailure) }

impl Dom {
    /// The receiving document and validated native creation arguments are
    /// explicit. No autonomous node is allocated before actual Construct.
    pub fn prepare_custom_element_creation(&mut self, document: NodeGeneration,
        namespace_uri: Option<&str>, prefix: Option<&str>, local_name: &str,
        is_value: Option<&[u16]>, synchronous: bool) -> Result<ElementCreationPlan, DomError> {
        if self.node_generation(document.node) != Some(document.generation)
            || !matches!(self.get(document.node).kind, NodeKind::Document) { return Err(DomError::InvalidStateError); }
        let namespace_uri = namespace_uri.filter(|uri| !uri.is_empty());
        let definition = if namespace_uri == Some("http://www.w3.org/1999/xhtml") {
            self.custom_element_definitions(document.node).into_iter().find(|(_, def)| def.local_name == local_name
                && (def.name == def.local_name || is_value == Some(def.name.encode_utf16().collect::<Vec<_>>().as_slice())))
                .map(|(id, def)| (id, def.constructor, def.name == def.local_name))
        } else { None };
        if let Some((definition, constructor, true)) = definition.filter(|_| synchronous) {
            return Ok(ElementCreationPlan::Autonomous(AutonomousCreationContext {
                arena: self.token_list_arena.clone(), document, definition, constructor,
                local_name: local_name.to_owned(), prefix: prefix.map(str::to_owned),
            }));
        }
        let element = self.create_element_expanded_in(document.node, namespace_uri, prefix, local_name)?;
        self.initialize_custom_element_is_value(element, self.node_generation(element).unwrap(), is_value)?;
        let owner = self.render_handle(element);
        if let Some((definition, constructor, _)) = definition {
            if synchronous { return Ok(ElementCreationPlan::Upgrade { element: owner, definition, constructor }); }
            self.queue_custom_element_upgrade(element);
        }
        Ok(ElementCreationPlan::Element(owner))
    }
    pub fn prepare_dom_custom_element_creation(&mut self, document: NodeGeneration,
        namespace_uri: Option<&str>, prefix: Option<&str>, local_name: &str, is_value: Option<&[u16]>)
        -> Result<ElementCreationPlan, DomError> {
        let plan = self.prepare_custom_element_creation(document, namespace_uri, prefix, local_name, is_value, true)?;
        if let ElementCreationPlan::Element(owner) | ElementCreationPlan::Upgrade { element: owner, .. } = &plan {
            self.init_force_async(owner.node);
        }
        Ok(plan)
    }
    /// Consumes original settings after actual Construct. Failure is returned as
    /// a move-only plan so the host reports in the constructor realm BEFORE
    /// allocating the standards-required fresh fallback.
    pub fn complete_custom_element_creation(&mut self, context: AutonomousCreationContext,
        returned: Option<NodeGeneration>) -> Result<CustomElementCreationCompletion, DomError> {
        self.validate_autonomous_creation_context(&context)?;
        let reason = match returned {
            None => Some(DomError::TypeError),
            Some(owner) if self.node_generation(owner.node) != Some(owner.generation)
                || self.namespace(owner.node) != Some(Namespace::Html) => Some(DomError::TypeError),
            Some(owner) if self.owner_document(owner.node) != Some(context.document.node)
                || self.local_name(owner.node) != Some(context.local_name.as_str())
                || self.get(owner.node).parent.is_some() || self.get(owner.node).first_child.is_some()
                || !self.attributes_expanded(owner.node).is_some_and(|attrs| attrs.is_empty()) => Some(DomError::NotSupportedError),
            Some(_) => None,
        };
        if let Some(reason) = reason {
            return Ok(CustomElementCreationCompletion::Failed(CustomElementCreationFailure { context, reason }));
        }
        let owner = returned.expect("Validated actual constructor result");
        self.nodes[owner.node.0 as usize].prefix = context.prefix;
        let slot = owner.node.0 as usize;
        self.custom_elements.is_values.resize_with(self.custom_elements.is_values.len().max(slot + 1), || None);
        self.custom_elements.is_values[slot] = Some((owner.generation, None));
        Ok(CustomElementCreationCompletion::Created(owner))
    }
    pub fn finish_custom_element_creation_failure(&mut self, failure: CustomElementCreationFailure)
        -> Result<NodeGeneration, DomError> {
        self.validate_autonomous_creation_context(&failure.context)?;
        let context = failure.context;
        let node = self.create_element_expanded_in(context.document.node, Some("http://www.w3.org/1999/xhtml"),
            context.prefix.as_deref(), &context.local_name)?;
        let owner = self.render_handle(node);
        let slot = node.0 as usize;
        self.custom_elements.failed_creation_nodes.resize_with(self.custom_elements.failed_creation_nodes.len().max(slot + 1), || None);
        self.custom_elements.failed_creation_nodes[slot] = Some(owner.generation);
        self.initialize_custom_element_is_value(node, owner.generation, None)?;
        Ok(owner)
    }
    fn validate_autonomous_creation_context(&self, context: &AutonomousCreationContext) -> Result<(), DomError> {
        if !std::sync::Arc::ptr_eq(&context.arena, &self.token_list_arena)
            || self.custom_element_definition_document(context.definition) != Some(context.document) {
            return Err(DomError::InvalidStateError);
        }
        self.html_constructor_definition(context.document, context.definition, context.constructor)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(local: &str) -> (Dom, usize) {
        let mut dom = Dom::new();
        let id = dom.define_custom_element(CustomElementDefinition::new(NodeIdx(0),
            "x-plan".into(), local.into(), CallbackId(42))).unwrap();
        (dom, id)
    }
    fn autonomous(dom: &mut Dom, prefix: Option<&str>) -> AutonomousCreationContext {
        let ElementCreationPlan::Autonomous(context) = dom.prepare_custom_element_creation(dom.render_handle(NodeIdx(0)),
            Some("http://www.w3.org/1999/xhtml"), prefix, "x-plan", None, true).unwrap() else { panic!("Real autonomous plan"); };
        context
    }
    fn construct(dom: &mut Dom, id: usize) -> NodeGeneration {
        let cap = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), id, CallbackId(42)).unwrap();
        dom.consume_html_constructor(cap).unwrap()
    }
    #[test]
    fn autonomous_plan_allocates_no_node_and_nested_direct_constructions_stay_distinct() {
        let (mut dom, id) = fixture("x-plan"); let count = dom.node_count();
        let outer = autonomous(&mut dom, None); assert_eq!(dom.node_count(), count);
        let inner = autonomous(&mut dom, None); assert_eq!(dom.node_count(), count);
        assert_eq!(dom.custom_element_super(id), Err(DomError::InvalidStateError));
        let first = construct(&mut dom, id); let second = construct(&mut dom, id);
        assert_ne!(first, second);
        assert!(matches!(dom.complete_custom_element_creation(inner, Some(first)).unwrap(), CustomElementCreationCompletion::Created(owner) if owner == first));
        assert!(matches!(dom.complete_custom_element_creation(outer, Some(second)).unwrap(), CustomElementCreationCompletion::Created(owner) if owner == second));
    }
    #[test]
    fn completion_selects_returned_node_and_applies_requested_namespace_prefix() {
        let (mut dom, id) = fixture("x-plan"); let context = autonomous(&mut dom, Some("p"));
        let owner = construct(&mut dom, id);
        assert_eq!(dom.prefix(owner.node), None);
        assert!(matches!(dom.complete_custom_element_creation(context, Some(owner)).unwrap(), CustomElementCreationCompletion::Created(actual) if actual == owner));
        assert_eq!(dom.prefix(owner.node), Some("p"));
    }
    #[test]
    fn missing_or_invalid_return_issues_failure_before_any_fallback_allocation() {
        let (mut dom, id) = fixture("x-plan"); let context = autonomous(&mut dom, None);
        let held = construct(&mut dom, id); dom.set_attribute(held.node, "id", "held").unwrap();
        let count = dom.node_count();
        let CustomElementCreationCompletion::Failed(failure) = dom.complete_custom_element_creation(context, Some(held)).unwrap() else { panic!("Attributes invalidate synchronous result"); };
        assert_eq!(failure.reason(), DomError::NotSupportedError); assert_eq!(dom.node_count(), count);
        assert_eq!(dom.custom_element_state(held.node), CustomElementState::Custom);
        let fallback = dom.finish_custom_element_creation_failure(failure).unwrap();
        assert_ne!(fallback, held); assert_eq!(dom.node_count(), count + 1);
        assert_eq!(dom.custom_element_state(fallback.node), CustomElementState::Failed);
        assert_eq!(dom.custom_element_definition_of(fallback.node), None);
        assert_eq!(dom.local_name(fallback.node), Some("x-plan"));
        assert_eq!(dom.custom_element_is_value(fallback.node).unwrap(), None);
        assert!(dom.attributes_expanded(fallback.node).unwrap().is_empty());
    }
    #[test]
    fn constructor_callee_document_is_distinct_from_registry_document() {
        let (mut dom, id) = fixture("x-plan"); let context = autonomous(&mut dom, None);
        let other = dom.create_document();
        let cap = dom.prepare_html_constructor_in_document(dom.render_handle(NodeIdx(0)), dom.render_handle(other), id, CallbackId(42)).unwrap();
        let owner = dom.consume_html_constructor(cap).unwrap();
        assert_eq!(dom.owner_document(owner.node), Some(other));
        let CustomElementCreationCompletion::Failed(failure) = dom.complete_custom_element_creation(context, Some(owner)).unwrap() else { panic!("Foreign document result fails original creation validation"); };
        assert_eq!(failure.reason(), DomError::NotSupportedError);
        let fallback = dom.finish_custom_element_creation_failure(failure).unwrap();
        assert_eq!(dom.owner_document(fallback.node), Some(NodeIdx(0)));
    }
    #[test]
    fn builtin_plan_uses_real_existing_element_upgrade_and_private_is() {
        let (mut dom, id) = fixture("button"); let units: Vec<_> = "x-plan".encode_utf16().collect();
        let ElementCreationPlan::Upgrade { element, definition, constructor } = dom.prepare_custom_element_creation(dom.render_handle(NodeIdx(0)),
            Some("http://www.w3.org/1999/xhtml"), None, "button", Some(&units), true).unwrap() else { panic!("Real builtin upgrade"); };
        assert_eq!(definition, id); assert_eq!(constructor, CallbackId(42));
        assert_eq!(dom.get_attribute_ns(element.node, None, "is"), None);
        assert_eq!(dom.custom_element_is_value(element.node).unwrap(), Some(units.as_slice()));
        dom.begin_custom_element_upgrade(element.node, id).unwrap();
        assert_eq!(dom.custom_element_super(id), Ok(element.node));
        dom.set_attribute(element.node, "title", "constructor changes allowed").unwrap();
        dom.finish_custom_element_upgrade(element.node, id, true).unwrap();
        assert_eq!(dom.custom_element_state(element.node), CustomElementState::Custom);
    }
    #[test]
    fn foreign_arena_or_retired_original_registry_cannot_complete_context() {
        let (mut dom, _) = fixture("x-plan"); let context = autonomous(&mut dom, None);
        let (mut other, _) = fixture("x-plan");
        assert!(matches!(other.complete_custom_element_creation(context, None), Err(DomError::InvalidStateError)));
        let doc = dom.create_document();
        dom.define_custom_element(CustomElementDefinition::new(doc, "x-plan".into(), "x-plan".into(), CallbackId(42))).unwrap();
        let ElementCreationPlan::Autonomous(context) = dom.prepare_custom_element_creation(dom.render_handle(doc), Some("http://www.w3.org/1999/xhtml"), None, "x-plan", None, true).unwrap() else { panic!("Registered document plan"); };
        dom.retire_unreachable_node(doc).unwrap();
        assert!(matches!(dom.complete_custom_element_creation(context, None), Err(DomError::InvalidStateError)));
    }
    #[test]
    fn native_dom_script_creation_keeps_force_async_distinct_from_parser_creation() {
        let mut dom = Dom::new(); let document = dom.render_handle(NodeIdx(0));
        let ElementCreationPlan::Element(imperative) = dom.prepare_dom_custom_element_creation(document, Some("http://www.w3.org/1999/xhtml"), None, "script", None).unwrap() else { panic!("Ordinary script"); };
        let ElementCreationPlan::Element(parser) = dom.prepare_custom_element_creation(document, Some("http://www.w3.org/1999/xhtml"), None, "script", None, false).unwrap() else { panic!("Actual parser path"); };
        assert!(dom.force_async(imperative.node));
        assert!(!dom.force_async(parser.node));
    }
}

#[cfg(test)]
mod observed_names_tests {
    use super::*;
    #[test]
    fn exact_observed_names_never_collapse_surrogates_to_replacement_character() {
        let mut dom = Dom::new();
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-exact-name".into(), "x-exact-name".into(), CallbackId(43));
        definition.callbacks.attribute_changed = Some(CallbackId(44));
        definition.set_observed_attribute_units(vec![vec![0xd800]]);
        let id = dom.define_custom_element(definition).unwrap();
        let cap = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), id, CallbackId(43)).unwrap();
        let owner = dom.consume_html_constructor(cap).unwrap();
        dom.set_attribute(owner.node, "\u{fffd}", "actual replacement-name").unwrap();
        assert!(dom.take_custom_element_reactions().is_empty());
        let mut exact = CustomElementDefinition::new(NodeIdx(0), "x-real-name".into(), "x-real-name".into(), CallbackId(45));
        exact.callbacks.attribute_changed = Some(CallbackId(46));
        exact.set_observed_attribute_units(vec![vec![0xfffd]]);
        let id = dom.define_custom_element(exact).unwrap();
        let cap = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), id, CallbackId(45)).unwrap();
        let owner = dom.consume_html_constructor(cap).unwrap();
        dom.set_attribute(owner.node, "\u{fffd}", "actual").unwrap();
        assert_eq!(dom.take_custom_element_reactions().len(), 1);
    }
}
