//! Native-document computation with explicit shadow stylesheet identities.
//! CSS Shadow 1: selector trees remain separate; inheritance follows hosts/slots.
use super::*;
use crate::font_metric_context::FontMetricContext;
use amazing_dom::NodeKind;

/// Ordinary sheets apply only in their native tree. Each shadow entry names an
/// actual attached ShadowRoot (open or closed) and its own ordered sheet list.
/// Duplicate entries concatenate in input order; foreign/detached roots are
/// ignored. Nodes excluded from the flattened tree have no computed style.
/// Inline declarations remain local to their element. Active browsing
/// contexts and sheet collection/URL provenance are the driver's responsibility.
///
/// This extends computed styles, not layout's flat-tree box traversal. Existing
/// selector support is retained: :host/::slotted and tree-scoped named resources
/// need their own interfaces. Generated shadow counters are not synthesized.
pub fn compute_document_with_shadow_font_metrics(
    dom: &Dom, document: NodeIdx, author_sheets: &[Stylesheet],
    shadow_sheets: &[(NodeIdx, Vec<Stylesheet>)], environment: &MediaEnvironment,
    animated: Option<&AnimatedValues>, context: &dyn FontMetricContext,
) -> StyledDom {
    if document.0 as usize >= dom.node_count()
        || !matches!(dom.get(document).kind, NodeKind::Document) {
        return StyledDom::default();
    }
    let rules = Rules::new(dom, document, author_sheets, shadow_sheets, environment);
    let body_link = hints::body_link_in(dom, document);
    let root = dom.children(document).find(|node| dom.name(*node).is_some());
    let mut computation = Computation {
        dom, document, root, rules: &rules, environment, animated, context,
        body_link: body_link.as_deref(), out: StyledDom::default(), visiting: HashSet::new(),
        root_font_px: 16.0, flat: flat_members(dom, document),
    };
    if let Some(root) = root { computation.element(root); }
    // Arena identities are retained. Membership excludes template contents,
    // detached roots and nested native Documents without remapping any index.
    for node in computation.flat.clone() {
        if dom.name(node).is_some() { computation.element(node); }
    }
    crate::generated::resolve_document_at(dom, document, &mut computation.out, author_sheets, environment);
    computation.out
}

/// Query the actual pseudo cascade in the originating element's selector tree,
/// including hidden/non-generated pseudos. Native document membership crosses
/// real shadow host metadata, never ownerDocument or a principal substitution.
pub fn compute_document_pseudo_with_shadow_font_metrics(
    dom: &Dom, document: NodeIdx, node: NodeIdx, pseudo: PseudoElement,
    author_sheets: &[Stylesheet], shadow_sheets: &[(NodeIdx, Vec<Stylesheet>)],
    environment: &MediaEnvironment, animated: Option<&AnimatedValues>,
    context: &dyn FontMetricContext,
) -> Option<ComputedStyle> {
    if node.0 as usize >= dom.node_count() || dom.name(node).is_none()
        || native_document(dom, node) != Some(document) { return None; }
    let principal = compute_document_with_shadow_font_metrics(
        dom, document, author_sheets, shadow_sheets, environment, animated, context);
    let origin = principal.get(node)?;
    let rules = Rules::new(dom, document, author_sheets, shadow_sheets, environment);
    let author = rules.author(tree_root(dom, node)?);
    let mut ua = compile_sheets(&[parse_stylesheet(UA_SHEET), parse_stylesheet("slot {display:contents}")], true, environment);
    if pseudo == PseudoElement::Backdrop {
        ua.extend(compile_sheets(&[parse_stylesheet("::backdrop {position:fixed;inset:0}")], true, environment));
    }
    let body_link = hints::body_link_in(dom, document);
    let root = dom.children(document).find(|child| dom.name(*child).is_some());
    let root_font_px = if Some(node) == root {
        origin.get("font-size").and_then(|value| value.strip_suffix("px"))
            .and_then(|value| value.parse().ok()).unwrap_or(origin.root_font_size_px())
    } else { origin.root_font_size_px() };
    let (mut result, _) = compute_one(dom, node, Some(origin), root_font_px,
        &ua, author, body_link.as_deref(), Some(pseudo), None, environment);
    crate::font_metric_context::resolve(&mut result, node, Some(pseudo), Some(context));
    Some(result)
}

// Value processing applies to the selected document's flattened tree only.
// Keep immediate assigned slots (not flattened assignedNodes) for inheritance.
fn flat_members(dom: &Dom, document: NodeIdx) -> HashSet<NodeIdx> {
    let mut seen = HashSet::new();
    let mut pending: Vec<_> = dom.children(document).collect();
    while let Some(node) = pending.pop() {
        if matches!(dom.get(node).kind, NodeKind::Document) || !seen.insert(node) { continue; }
        let children: Vec<_> = if let Some(root) = dom.shadow_root_of_host(node) {
            dom.children(root).collect()
        } else if dom.is_html_slot(node) && tree_root(dom, node).is_some_and(|root| dom.is_shadow_root(root)) {
            let assigned = dom.assigned_nodes(node, false, false).unwrap_or_default();
            if assigned.is_empty() { dom.children(node).collect() } else { assigned }
        } else { dom.children(node).collect() };
        pending.extend(children);
    }
    seen
}
fn native_document(dom: &Dom, node: NodeIdx) -> Option<NodeIdx> {
    let mut current = Some(node);
    let mut seen = HashSet::new();
    while let Some(node) = current {
        if node.0 as usize >= dom.node_count() || !seen.insert(node) { return None; }
        if matches!(dom.get(node).kind, NodeKind::Document) { return Some(node); }
        current = dom.get(node).parent.or_else(|| dom.shadow_data(node).map(|data| data.host));
    }
    None
}
fn tree_root(dom: &Dom, node: NodeIdx) -> Option<NodeIdx> {
    let mut current = node;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(current) { return None; }
        if matches!(dom.get(current).kind, NodeKind::Document) || dom.is_shadow_root(current) {
            return Some(current);
        }
        current = dom.get(current).parent?;
    }
}
struct Rules {
    ua: Vec<CompiledRule>,
    author: BTreeMap<NodeIdx, Vec<CompiledRule>>,
}
impl Rules {
    fn new(dom: &Dom, document: NodeIdx, sheets: &[Stylesheet],
        shadows: &[(NodeIdx, Vec<Stylesheet>)], environment: &MediaEnvironment) -> Self {
        let mut grouped = BTreeMap::<NodeIdx, Vec<Stylesheet>>::new();
        grouped.insert(document, sheets.to_vec());
        for (root, sheets) in shadows {
            if (root.0 as usize) < dom.node_count() && dom.is_shadow_root(*root)
                && native_document(dom, *root) == Some(document) {
                grouped.entry(*root).or_default().extend(sheets.iter().cloned());
            }
        }
        Self { ua: compile_sheets(&[parse_stylesheet(UA_SHEET),
            parse_stylesheet("slot {display:contents}")], true, environment),
            author: grouped.into_iter().map(|(root, sheets)|
                (root, compile_sheets(&sheets, false, environment))).collect() }
    }
    fn author(&self, root: NodeIdx) -> &[CompiledRule] {
        self.author.get(&root).map(Vec::as_slice).unwrap_or(&[])
    }
}
struct Computation<'a> {
    dom: &'a Dom, document: NodeIdx, root: Option<NodeIdx>, rules: &'a Rules,
    environment: &'a MediaEnvironment, animated: Option<&'a AnimatedValues>,
    context: &'a dyn FontMetricContext, body_link: Option<&'a str>,
    out: StyledDom, visiting: HashSet<NodeIdx>, root_font_px: f32, flat: HashSet<NodeIdx>,
}
impl Computation<'_> {
    fn element(&mut self, node: NodeIdx) -> bool {
        if !self.flat.contains(&node) { return false; }
        if self.out.get(node).is_some() { return true; }
        if !self.visiting.insert(node) { return false; }
        let parent = self.dom.find_slot(node, false).or_else(|| {
            let parent = self.dom.get(node).parent?;
            if self.dom.is_shadow_root(parent) { self.dom.shadow_data(parent).map(|data| data.host) }
            else if self.dom.name(parent).is_some() { Some(parent) } else { None }
        });
        if let Some(parent) = parent {
            if native_document(self.dom, parent) != Some(self.document) || !self.element(parent) {
                self.visiting.remove(&node); return false;
            }
        }
        let Some(tree) = tree_root(self.dom, node) else {
            self.visiting.remove(&node); return false;
        };
        let parent_style = parent.and_then(|parent| self.out.get(parent)).cloned();
        let author = self.rules.author(tree);
        let pseudos = PseudoElement::STYLED.map(|pseudo|
            self.rules.ua.iter().chain(author).any(|rule| rule.targets(pseudo)));
        let (style, sources, _) = style_element(self.dom, node, parent_style.as_ref(),
            self.root_font_px, &self.rules.ua, author, self.body_link, &pseudos,
            self.animated, self.environment, Some(self.context), &mut self.out, Some(node) == self.root);
        if Some(node) == self.root {
            self.root_font_px = style.get("font-size").and_then(|value| value.strip_suffix("px"))
                .and_then(|value| value.parse().ok()).unwrap_or(16.0);
        }
        self.out.styles.insert(node.0, style);
        self.out.sources.insert(node.0, sources);
        self.visiting.remove(&node);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    use crate::font_size_adjust::FontSizeAdjustMetric;
    struct Metrics;
    impl FontMetricContext for Metrics {
        fn aspect_ratio(&self, _: NodeIdx, _: Option<PseudoElement>, _: &ComputedStyle,
            _: FontSizeAdjustMetric) -> Option<f64> { Some(0.7) }
    }
    fn attach(dom: &mut Dom, host: NodeIdx, mode: ShadowMode, slot_assignment: SlotAssignment) -> NodeIdx {
        dom.attach_shadow(host, ShadowOptions {mode, slot_assignment, delegates_focus:false,
            clonable:false, serializable:false}).unwrap()
    }
    fn compute_scoped(dom: &Dom, document: NodeIdx, sheets: &[Stylesheet],
        shadows: &[(NodeIdx,Vec<Stylesheet>)]) -> StyledDom {
        compute_document_with_shadow_font_metrics(dom,document,sheets,shadows,
            &MediaEnvironment::default(),None,&Metrics)
    }
    #[test]
    fn open_closed_and_nested_scopes_preserve_host_inheritance_and_native_identity() {
        for mode in [ShadowMode::Open,ShadowMode::Closed] {
            let mut dom=Dom::new();
            let html=dom.create_element(NodeIdx(0),"html");
            let host=dom.create_element(html,"section");
            let light=dom.create_element(html,"span");
            let root=attach(&mut dom,host,mode,SlotAssignment::Named);
            let inner=dom.create_element(root,"span");
            let nested=dom.create_element(root,"article");
            let nested_root=attach(&mut dom,nested,ShadowMode::Closed,SlotAssignment::Named);
            let deep=dom.create_element(nested_root,"span");
            let sheets=[parse_stylesheet("html{font-size:30px}html::before{font-size:2rem}section{color:red;--tone:green;font-size-adjust:from-font}span{color:blue}span::before{color:blue}")];
            let shadows=[(root,vec![parse_stylesheet("span{color:var(--tone);font-size:2rem}article{color:yellow}span::before{content:'shadow';color:purple}")]),
                (nested_root,vec![parse_stylesheet("span{font-size:3rem}span::before{content:'nested'}")])];
            let result=compute_scoped(&dom,NodeIdx(0),&sheets,&shadows);
            assert_eq!(result.get(light).unwrap().get("color"),Some("blue"));
            assert_eq!(result.get(inner).unwrap().get("color"),Some("green"));
            assert_eq!(result.get(inner).unwrap().get("font-size"),Some("60px"));
            assert_eq!(result.get(deep).unwrap().get("color"),Some("yellow"));
            assert_eq!(result.get(deep).unwrap().get("font-size"),Some("90px"));
            assert_eq!(result.get(deep).unwrap().get("font-size-adjust"),Some("0.7"));
            let root_pseudo=compute_document_pseudo_with_shadow_font_metrics(&dom,NodeIdx(0),html,
                PseudoElement::Before,&sheets,&shadows,&MediaEnvironment::default(),None,&Metrics).unwrap();
            assert_eq!(root_pseudo.get("font-size"),Some("60px"));
            let pseudo=compute_document_pseudo_with_shadow_font_metrics(&dom,NodeIdx(0),inner,
                PseudoElement::Before,&sheets,&shadows,&MediaEnvironment::default(),None,&Metrics).unwrap();
            assert_eq!(pseudo.get("color"),Some("purple"));
            assert_eq!(pseudo.get("content"),Some("\"shadow\""));
            assert!(compute_document_with_font_metrics(&dom,NodeIdx(0),&sheets,
                &MediaEnvironment::default(),None,&Metrics).get(inner).is_none());
        }
    }
    #[test]
    fn named_and_manual_closed_slots_supply_inheritance_and_generated_parent_display() {
        for assignment in [SlotAssignment::Named,SlotAssignment::Manual] {
            let mut dom=Dom::new();
            let host=dom.create_element(NodeIdx(0),"section");
            let assigned=dom.create_element(host,"span");
            let unassigned=dom.create_element(host,"b");
            dom.set_attrs(assigned,vec![("slot".into(),"chosen".into())]);
            dom.set_attrs(unassigned,vec![("slot".into(),"absent".into())]);
            let root=attach(&mut dom,host,ShadowMode::Closed,assignment);
            let wrapper=dom.create_element(root,"div");
            let slot=dom.create_element(wrapper,"slot");
            dom.set_attrs(slot,vec![("name".into(),"chosen".into())]);
            let fallback_slot=dom.create_element(wrapper,"slot");
            dom.set_attrs(fallback_slot,vec![("name".into(),"fallback".into())]);
            let fallback=dom.create_element(fallback_slot,"em");
            let unused_fallback=dom.create_element(slot,"i");
            if assignment==SlotAssignment::Manual {dom.assign_slot(slot,&[assigned]).unwrap();}
            assert!(dom.assigned_slot(assigned).is_none()); // closed WebIDL accessor
            let sheets=[parse_stylesheet("section{color:red}span{display:inline-flex}")];
            let shadows=[(root,vec![parse_stylesheet("div{display:flex}slot{color:green;font-size:24px;font-size-adjust:from-font;--slot:chosen}")])];
            let result=compute_scoped(&dom,NodeIdx(0),&sheets,&shadows);
            for node in [assigned,fallback] {
                let style=result.get(node).unwrap();
                assert_eq!(style.get("color"),Some("green"));
                assert_eq!(style.get("font-size"),Some("24px"));
                assert_eq!(style.get("font-size-adjust"),Some("0.7"));
                assert_eq!(style.cssom_property_value("--slot"),Some("chosen".into()));
            }
            assert_eq!(result.get(slot).unwrap().get("display"),Some("contents"));
            assert_eq!(result.get(assigned).unwrap().get("display"),Some("flex"));
            assert!(result.get(unassigned).is_none());
            assert!(result.get(unused_fallback).is_none());
            assert!(compute_document_pseudo_with_shadow_font_metrics(&dom,NodeIdx(0),unassigned,
                PseudoElement::Before,&sheets,&shadows,&MediaEnvironment::default(),None,&Metrics).is_none());
        }
    }
    #[test]
    fn foreign_native_documents_detached_hosts_and_wrong_scope_entries_never_leak() {
        let mut dom=Dom::new();
        let host=dom.create_element(NodeIdx(0),"section");
        let root=attach(&mut dom,host,ShadowMode::Open,SlotAssignment::Named);
        let inner=dom.create_element(root,"span");
        let other=dom.append_child(NodeIdx(0),NodeKind::Document);
        let foreign=dom.create_element(other,"section");
        let foreign_root=attach(&mut dom,foreign,ShadowMode::Closed,SlotAssignment::Named);
        let foreign_inner=dom.create_element(foreign_root,"span");
        let detached=dom.create_element(NodeIdx(0),"section");
        let detached_root=attach(&mut dom,detached,ShadowMode::Open,SlotAssignment::Named);
        let detached_inner=dom.create_element(detached_root,"span");
        dom.detach(detached);
        let sheets=[parse_stylesheet("section{color:red}")];
        let scopes=[(root,vec![parse_stylesheet("span{color:green}")]),
            (foreign_root,vec![parse_stylesheet("span{color:blue}")]),
            (detached_root,vec![parse_stylesheet("span{color:purple}")]),
            (NodeIdx(u32::MAX),vec![parse_stylesheet("*{color:yellow}")]),
            (host,vec![parse_stylesheet("*{color:yellow}")])];
        let main=compute_scoped(&dom,NodeIdx(0),&sheets,&scopes);
        assert_eq!(main.get(inner).unwrap().get("color"),Some("green"));
        assert!(main.get(foreign).is_none());assert!(main.get(foreign_inner).is_none());
        assert!(main.get(detached_inner).is_none());
        assert_eq!(compute_scoped(&dom,other,&sheets,&scopes).get(foreign_inner).unwrap().get("color"),Some("blue"));
        for (document,node) in [(NodeIdx(0),foreign_inner),(NodeIdx(0),detached_inner),(other,inner),
            (host,inner),(NodeIdx(u32::MAX),inner),(NodeIdx(0),NodeIdx(u32::MAX))] {
            assert!(compute_document_pseudo_with_shadow_font_metrics(&dom,document,node,
                PseudoElement::Before,&sheets,&scopes,&MediaEnvironment::default(),None,&Metrics).is_none());
        }
    }
}
