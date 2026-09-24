use crate::host::{BindingHost, CallbackId, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{
    traversal::{NodeIterator, TraversalId, TraversalStep, Traverse, TreeWalker},
    DomError, NodeIdx, NodeKind,
};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::CreateNodeIterator | O::CreateTreeWalker | O::TraversalRoot
        | O::TraversalWhatToShow | O::TraversalFilter | O::IteratorReferenceNode
        | O::IteratorPointerBeforeReferenceNode | O::IteratorNextNode
        | O::IteratorPreviousNode | O::IteratorDetach | O::WalkerCurrentNode
        | O::WalkerSetCurrentNode | O::WalkerParentNode | O::WalkerFirstChild
        | O::WalkerLastChild | O::WalkerPreviousSibling | O::WalkerNextSibling
        | O::WalkerPreviousNode | O::WalkerNextNode)
}
#[derive(Clone, Copy)]
enum Handle { Iterator(NodeIterator), Walker(TreeWalker) }
impl Handle {
    fn id(self) -> TraversalId {
        match self { Self::Iterator(id) => id.into(), Self::Walker(id) => id.into() }
    }
}
fn key<H: BindingHost>(host: &H, slot: usize) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: slot as u64, generation: 1 }
}
fn unsigned_long<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<u32, H::Error> {
    let number = host.to_number(value)?;
    Ok(if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 })
}
fn receiver<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Handle, H::Error> {
    let (key, handle) = if let Some(key) = host.platform_object(value, InterfaceBrand::NODE_ITERATOR) {
        (key, Handle::Iterator(NodeIterator::from_slot(key.slot as usize)))
    } else if let Some(key) = host.platform_object(value, InterfaceBrand::TREE_WALKER) {
        (key, Handle::Walker(TreeWalker::from_slot(key.slot as usize)))
    } else { return Err(host.type_error("Expected a traversal object")); };
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid traversal object"));
    }
    host.with_dom(|dom| dom.traversal_root(handle.id()).map(|_| ()))?;
    Ok(handle)
}
fn filter_call<H: BindingHost>(host: &mut H, callback: CallbackId, node: NodeIdx) -> Result<u16, H::Error> {
    let filter = host.root_value(callback).ok_or_else(|| host.type_error("Released NodeFilter"))?;
    let (function, this) = if host.is_callable(&filter) {
        (filter, host.undefined())
    } else {
        let function = host.get_property(&filter, "acceptNode")?;
        if !host.is_callable(&function) { return Err(host.type_error("NodeFilter.acceptNode is not callable")); }
        (function, filter)
    };
    let node_value = host.wrap_node(node)?;
    let value = host.call(&function, &this, &[node_value])?;
    Ok(unsigned_long(host, &value)? as u16)
}
fn run<H: BindingHost>(host: &mut H, handle: Handle, direction: Traverse) -> Result<H::Value, H::Error> {
    let mut step = host.with_dom(|dom| dom.begin_traversal(handle.id(), direction))?;
    loop {
        match step {
            TraversalStep::Done(Some(node)) => return host.wrap_node(node),
            TraversalStep::Done(None) => return Ok(host.null()),
            TraversalStep::Filter { traversal, callback, node } => {
                let result = filter_call(host, CallbackId(callback), node);
                match result {
                    Ok(value) => step = host.with_dom(|dom| dom.resume_traversal(traversal, Ok(value)))?,
                    Err(error) => {
                        let _ = host.with_dom(|dom| dom.cancel_traversal(traversal));
                        return Err(error);
                    }
                }
            }
        }
    }
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver_value: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if matches!(op, O::CreateNodeIterator | O::CreateTreeWalker) {
        let document = host.node_brand(&receiver_value).ok_or_else(|| host.type_error("Expected a Document"))?;
        host.with_dom(|dom| if matches!(dom.get(document).kind, NodeKind::Document) { Ok(()) }
            else { Err(DomError::TypeError) })?;
        let root = args.first().and_then(|v| host.node_brand(v))
            .ok_or_else(|| host.type_error("Expected a root Node"))?;
        let show = if let Some(value) = args.get(1).filter(|value| !host.is_undefined(value)) {
            unsigned_long(host, value)?
        } else { u32::MAX };
        let filter = args.get(2).filter(|v| !host.is_nullish(v)).cloned();
        let callback = filter.map(|value| host.retain_value(value));
        let result = host.with_dom(|dom| if op == O::CreateNodeIterator {
            dom.create_node_iterator(root, show, callback.map(|id| id.0))
                .map(|id| (id.slot(), InterfaceBrand::NODE_ITERATOR))
        } else {
            dom.create_tree_walker(root, show, callback.map(|id| id.0))
                .map(|id| (id.slot(), InterfaceBrand::TREE_WALKER))
        });
        return match result {
            Ok((slot, brand)) => host.wrap_platform_object(key(host, slot), brand),
            Err(error) => {
                if let Some(callback) = callback { host.release_value(callback); }
                Err(error)
            }
        };
    }
    let handle = receiver(host, &receiver_value)?;
    if matches!(op, O::IteratorReferenceNode | O::IteratorPointerBeforeReferenceNode
        | O::IteratorNextNode | O::IteratorPreviousNode | O::IteratorDetach)
        && !matches!(handle, Handle::Iterator(_)) {
        return Err(host.type_error("Expected a NodeIterator"));
    }
    if matches!(op, O::WalkerCurrentNode | O::WalkerSetCurrentNode | O::WalkerParentNode
        | O::WalkerFirstChild | O::WalkerLastChild | O::WalkerPreviousSibling
        | O::WalkerNextSibling | O::WalkerPreviousNode | O::WalkerNextNode)
        && !matches!(handle, Handle::Walker(_)) {
        return Err(host.type_error("Expected a TreeWalker"));
    }
    match op {
        O::TraversalRoot => {
            let node = host.with_dom(|dom| dom.traversal_root(handle.id()))?;
            host.wrap_node(node)
        }
        O::TraversalWhatToShow => {
            let show = host.with_dom(|dom| dom.traversal_what_to_show(handle.id()))?;
            Ok(host.number(show as f64))
        }
        O::TraversalFilter => {
            let callback = host.with_dom(|dom| dom.traversal_filter(handle.id()))?;
            Ok(callback.and_then(|id| host.root_value(CallbackId(id))).unwrap_or_else(|| host.null()))
        }
        O::IteratorReferenceNode => {
            let Handle::Iterator(id) = handle else { unreachable!() };
            let node = host.with_dom(|dom| dom.iterator_reference(id).map(|p| p.node))?;
            host.wrap_node(node)
        }
        O::IteratorPointerBeforeReferenceNode => {
            let Handle::Iterator(id) = handle else { unreachable!() };
            let before = host.with_dom(|dom| dom.iterator_reference(id).map(|p| p.before))?;
            Ok(host.boolean(before))
        }
        O::IteratorDetach => {
            let Handle::Iterator(id) = handle else { unreachable!() };
            host.with_dom(|dom| dom.iterator_detach(id))?;
            Ok(host.undefined())
        }
        O::WalkerCurrentNode => {
            let Handle::Walker(id) = handle else { unreachable!() };
            let node = host.with_dom(|dom| dom.walker_current(id))?;
            host.wrap_node(node)
        }
        O::WalkerSetCurrentNode => {
            let Handle::Walker(id) = handle else { unreachable!() };
            let node = args.first().and_then(|v| host.node_brand(v))
                .ok_or_else(|| host.type_error("Expected a Node"))?;
            host.with_dom(|dom| dom.walker_set_current(id, node))?;
            Ok(host.undefined())
        }
        _ => {
            let direction = match op {
                O::IteratorNextNode | O::WalkerNextNode => Traverse::Next,
                O::IteratorPreviousNode | O::WalkerPreviousNode => Traverse::Previous,
                O::WalkerParentNode => Traverse::Parent,
                O::WalkerFirstChild => Traverse::FirstChild,
                O::WalkerLastChild => Traverse::LastChild,
                O::WalkerPreviousSibling => Traverse::PreviousSibling,
                O::WalkerNextSibling => Traverse::NextSibling,
                _ => unreachable!(),
            };
            run(host, handle, direction)
        }
    }
}
