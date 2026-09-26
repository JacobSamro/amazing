//! A Selection is a document-owned view of one live Range.
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{DomError, NodeIdx, NodeKind, range::{BoundaryPoint, Range}};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::GetSelection | O::SelectionAnchorNode | O::SelectionAnchorOffset
        | O::SelectionFocusNode | O::SelectionFocusOffset | O::SelectionIsCollapsed
        | O::SelectionRangeCount | O::SelectionGetRangeAt | O::SelectionAddRange
        | O::SelectionRemoveRange | O::SelectionRemoveAllRanges | O::SelectionCollapse
        | O::SelectionSelectAllChildren | O::SelectionToString)
}

fn range_from_value<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<Range, H::Error> {
    let key = value.and_then(|value| host.platform_object(value, InterfaceBrand::RANGE))
        .ok_or_else(|| host.type_error("Expected a Range"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid Range"));
    }
    let range = Range::from_slot(key.slot as usize);
    host.with_dom(|dom| dom.range(range).map(|_| ()))?;
    Ok(range)
}

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::GetSelection {
        let document = host.node_brand(&receiver).unwrap_or_else(|| host.current_document());
        host.with_dom(|dom| if matches!(dom.get(document).kind, NodeKind::Document) { Ok(()) }
            else { Err(DomError::TypeError) })?;
        let key = host.node_platform_key(document);
        return host.wrap_platform_object(PlatformKey { arena: key.arena, slot: document.0 as u64,
            generation: key.generation }, InterfaceBrand::SELECTION);
    }
    let key = host.platform_object(&receiver, InterfaceBrand::SELECTION)
        .ok_or_else(|| host.type_error("Expected a Selection"))?;
    let document = NodeIdx(key.slot as u32);
    host.with_dom(|dom| dom.selection_range(document).map(|_| ()))?;
    match op {
        O::SelectionAddRange | O::SelectionRemoveRange => {
            let range = range_from_value(host, args.first())?;
            host.with_dom(|dom| if op == O::SelectionAddRange {
                dom.selection_add_range(document, range)
            } else { dom.selection_remove_range(document, range) })?;
            return Ok(host.undefined());
        }
        O::SelectionRemoveAllRanges => {
            host.with_dom(|dom| dom.selection_remove_all_ranges(document))?;
            return Ok(host.undefined());
        }
        O::SelectionCollapse => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let node = if host.is_nullish(value) { None } else { Some(host.node_brand(value)
                .ok_or_else(|| host.type_error("Expected a Node"))?) };
            let offset = match args.get(1) { Some(value) => host.to_number(value)?, None => 0. };
            let offset = if offset.is_finite() { offset.trunc().rem_euclid(4_294_967_296.) as usize } else { 0 };
            host.with_dom(|dom| dom.selection_collapse(document, node.map(|node| BoundaryPoint::new(node, offset))))?;
            return Ok(host.undefined());
        }
        O::SelectionSelectAllChildren => {
            let node = args.first().and_then(|value| host.node_brand(value))
                .ok_or_else(|| host.type_error("Expected a Node"))?;
            host.with_dom(|dom| dom.selection_select_all_children(document, node))?;
            return Ok(host.undefined());
        }
        _ => {}
    }
    let range = host.with_dom(|dom| dom.selection_range(document))?;
    if op == O::SelectionGetRangeAt {
        let index = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let index = host.to_number(index)?;
        if index != 0. || range.is_none() {
            return host.with_dom::<H::Value>(|_| Err(DomError::IndexSizeError));
        }
        let range = range.unwrap();
        let document_key = host.node_platform_key(document);
        return host.wrap_platform_object(PlatformKey { arena: document_key.arena,
            slot: range.slot() as u64, generation: 1 }, InterfaceBrand::RANGE);
    }
    if op == O::SelectionRangeCount { return Ok(host.number(if range.is_some() { 1. } else { 0. })); }
    if op == O::SelectionIsCollapsed {
        let collapsed = host.with_dom(|dom| Ok(range.map_or(true, |range| dom.range(range).unwrap().collapsed())))?;
        return Ok(host.boolean(collapsed));
    }
    if op == O::SelectionToString {
        let value = host.with_dom(|dom| range.map_or(Ok(vec![]), |range| dom.range_to_utf16_string(range)))?;
        return host.string(&value);
    }
    let point = host.with_dom(|dom| range.map(|range| dom.range(range).unwrap())
        .map(|range| if matches!(op, O::SelectionAnchorNode | O::SelectionAnchorOffset) { range.start } else { range.end }).ok_or(DomError::NotFoundError));
    match point {
        Ok(point) => match op {
            O::SelectionAnchorNode | O::SelectionFocusNode => host.wrap_node(point.node),
            O::SelectionAnchorOffset | O::SelectionFocusOffset => Ok(host.number(point.offset as f64)),
            _ => unreachable!(),
        },
        Err(_) => match op {
            O::SelectionAnchorNode | O::SelectionFocusNode => Ok(host.null()),
            O::SelectionAnchorOffset | O::SelectionFocusOffset => Ok(host.number(0.)),
            _ => unreachable!(),
        },
    }
}
