//! CSS Shadow 1 selector-tree contexts and Selectors 4 featureless subjects.
//! Tree identities are actual DOM roots, never flattened synthetic parents.
use super::*;
use amazing_dom::NodeKind;

#[derive(Clone, Copy)]
struct Context { tree: NodeIdx, scope: Option<NodeIdx>, host: Option<NodeIdx> }

/// Match a principal in an explicitly selected actual DOM selector tree.
/// Slotted aliases apply to assigned elements; ordinary selectors stay local.
pub fn matches_in_tree(dom: &Dom, node: NodeIdx, selector: &Selector,
    scope: Option<NodeIdx>, tree: NodeIdx) -> bool {
    selector.pseudo_element().is_none() && matching(dom, node, selector, scope, tree)
}

/// Match a tree-abiding pseudo on its actual principal, including slotted ones.
pub fn matches_pseudo_in_tree(dom: &Dom, node: NodeIdx, selector: &Selector,
    pseudo: PseudoElement, scope: Option<NodeIdx>, tree: NodeIdx) -> bool {
    selector.pseudo_element() == Some(pseudo) && matching(dom, node, selector, scope, tree)
}

fn matching(dom: &Dom, node: NodeIdx, selector: &Selector, scope: Option<NodeIdx>, tree: NodeIdx) -> bool {
    if tree.0 as usize >= dom.node_count() || node.0 as usize >= dom.node_count()
        || selector.compounds.is_empty() || dom.name(node).is_none() { return false; }
    let context = Context {tree, scope, host:dom.shadow_data(tree).map(|data|data.host)};
    let position = selector.compounds.len()-1;
    let compound = &selector.compounds[position];
    if let Some(index) = compound.simple.iter().position(|simple| matches!(simple, Simple::Slotted(_))) {
        let Simple::Slotted(argument) = &compound.simple[index] else { unreachable!() };
        if context.host.is_none() || !matches_compound(dom,node,argument) { return false; }
        if !compound.simple[index+1..].iter().all(|simple|simple_in_scope(dom,node,simple,context.scope)) { return false; }
        // A slot's own fallback is ordinary shadow content. Flattening an
        // actual assigned slot may expose nested fallback elements.
        let mut pending:Vec<_> = dom.children(tree).collect();
        while let Some(slot) = pending.pop() {
            if dom.is_html_slot(slot)
                && dom.assigned_nodes(slot,false,false).is_ok_and(|nodes| !nodes.is_empty())
                && dom.assigned_nodes(slot,true,true).is_ok_and(|nodes|nodes.contains(&node)) {
                let mut originating = selector.clone();
                originating.compounds[position].simple.truncate(index);
                if rightward(dom,slot,&originating,position,context) { return true; }
            }
            pending.extend(dom.children(slot));
        }
        return false;
    }
    rightward(dom,node,selector,position,context)
}

fn tree_root(dom:&Dom, mut node:NodeIdx)->NodeIdx {
    while let Some(parent)=dom.get(node).parent { node=parent; }
    node
}
pub(crate) fn selector_origins(dom:&Dom,node:NodeIdx,selector:&Selector,tree:NodeIdx)->Vec<NodeIdx> {
    if !selector.compounds.last().is_some_and(|compound|compound.simple.iter().any(|simple|matches!(simple,Simple::Slotted(_)))) { return vec![node]; }
    if dom.shadow_data(tree).is_none() { return Vec::new(); }
    let mut found=Vec::new();
    let mut pending:Vec<_>=dom.children(tree).collect();
    while let Some(slot)=pending.pop() {
        if dom.is_html_slot(slot)
            && dom.assigned_nodes(slot,false,false).is_ok_and(|nodes|!nodes.is_empty())
            && dom.assigned_nodes(slot,true,true).is_ok_and(|nodes|nodes.contains(&node)) {found.push(slot);}
        pending.extend(dom.children(slot));
    }
    found
}
pub(crate) fn selector_parent(dom:&Dom,node:NodeIdx,tree:NodeIdx)->Option<NodeIdx> {
    parent(dom,node,Context {tree,scope:None,host:dom.shadow_data(tree).map(|data|data.host)})
}
fn parent(dom:&Dom,node:NodeIdx,context:Context)->Option<NodeIdx> {
    if context.host==Some(node) { return None; }
    let parent=dom.get(node).parent?;
    if parent==context.tree { context.host.or_else(||dom.name(parent).map(|_|parent)) }
    else { dom.name(parent).map(|_|parent) }
}
fn children(dom:&Dom,node:NodeIdx,context:Context)->Vec<NodeIdx> {
    dom.children(if context.host==Some(node) {context.tree} else {node}).collect()
}
fn rightward(dom:&Dom,node:NodeIdx,selector:&Selector,position:usize,context:Context)->bool {
    if !compound(dom,node,&selector.compounds[position],context) { return false; }
    if position==0 { return true; }
    match selector.combinators[position-1] {
        Combinator::Child => parent(dom,node,context).is_some_and(|parent|rightward(dom,parent,selector,position-1,context)),
        Combinator::Descendant => {
            let mut ancestor=parent(dom,node,context);
            while let Some(node)=ancestor {
                if rightward(dom,node,selector,position-1,context) { return true; }
                ancestor=parent(dom,node,context);
            }
            false
        }
        Combinator::NextSibling | Combinator::LaterSibling => {
            if context.host==Some(node) { return false; }
            let mut sibling=dom.get(node).prev_sibling;
            while let Some(node)=sibling {
                if dom.name(node).is_some() {
                    if rightward(dom,node,selector,position-1,context) { return true; }
                    if selector.combinators[position-1]==Combinator::NextSibling { return false; }
                }
                sibling=dom.get(node).prev_sibling;
            }
            false
        }
    }
}

// Allowed-to-match is independent of actual matching. In particular,
// :not(.foo:host) cannot select a featureless host by negating a false match.
fn allowed_compound(compound:&Compound,scope_host:bool)->bool {
    let anchor=compound.simple.iter().any(|simple| !matches!(simple,Simple::Has(_) | Simple::PseudoElement(_)) && allowed_simple(simple,scope_host));
    compound.simple.iter().all(|simple| match simple {
        Simple::Has(_) | Simple::PseudoElement(_) => anchor,
        _=>allowed_simple(simple,scope_host),
    }) && anchor
}
fn allowed_simple(simple:&Simple,scope_host:bool)->bool {
    match simple {
        Simple::Host(_) | Simple::HostContext(_) => true,
        Simple::Pseudo(Pseudo::Scope)=>scope_host,
        Simple::Negation(args) | Simple::Matches(args) | Simple::Where(args) => args.iter().any(|argument|allowed_compound(argument,scope_host)),
        Simple::ComplexNegation(args) | Simple::ComplexMatches(args) | Simple::ComplexWhere(args) =>
            args.iter().any(|selector|selector.compounds.last().is_some_and(|argument|allowed_compound(argument,scope_host))),
        _=>false,
    }
}
fn compound(dom:&Dom,node:NodeIdx,compound:&Compound,context:Context)->bool {
    if context.host==Some(node) {
        if !allowed_compound(compound,context.scope==Some(node)) { return false; }
    } else if tree_root(dom,node)!=context.tree { return false; }
    compound.simple.iter().all(|simple|matching_simple(dom,node,simple,context))
}
fn matching_simple(dom:&Dom,node:NodeIdx,simple:&Simple,context:Context)->bool {
    match simple {
        Simple::Host(argument) => context.host==Some(node) && argument.as_ref().is_none_or(|argument|matches_compound(dom,node,argument)),
        Simple::HostContext(argument) => {
            if context.host!=Some(node) { return false; }
            let mut ancestor=Some(node);
            while let Some(node)=ancestor {
                if dom.name(node).is_some() && matches_compound(dom,node,argument) { return true; }
                if matches!(dom.get(node).kind,NodeKind::Document) { break; }
                ancestor=dom.get(node).parent.or_else(||dom.shadow_data(node).map(|data|data.host));
            }
            false
        }
        Simple::Slotted(_)=>false,
        Simple::Negation(args)=>!args.iter().any(|argument|compound(dom,node,argument,context)),
        Simple::Matches(args) | Simple::Where(args)=>args.iter().any(|argument|compound(dom,node,argument,context)),
        Simple::ComplexNegation(args)=>!args.iter().any(|argument|rightward(dom,node,argument,argument.compounds.len()-1,context)),
        Simple::ComplexMatches(args) | Simple::ComplexWhere(args)=>args.iter().any(|argument|rightward(dom,node,argument,argument.compounds.len()-1,context)),
        Simple::Has(args)=>args.iter().any(|(relation,argument)| {
            let mut pending=children(dom,node,context);
            while let Some(child)=pending.pop() {
                if dom.name(child).is_some() && compound(dom,child,argument,context) { return true; }
                if *relation==Combinator::Descendant { pending.extend(children(dom,child,context)); }
            }
            false
        }),
        _=>simple_in_scope(dom,node,simple,context.scope),
    }
}
