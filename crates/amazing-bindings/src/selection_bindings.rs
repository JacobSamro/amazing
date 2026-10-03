//! Selection wrappers validate an append-only native slot and its associated document generation.
use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{DomError, NodeIdx, range::{BoundaryPoint, Range}, selection::{SelectionId, SelectionDirection}};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::GetSelection | O::WindowGetSelection | O::SelectionAnchorNode | O::SelectionAnchorOffset
        | O::SelectionFocusNode | O::SelectionFocusOffset | O::SelectionIsCollapsed
        | O::SelectionRangeCount | O::SelectionGetRangeAt | O::SelectionAddRange
        | O::SelectionRemoveRange | O::SelectionRemoveAllRanges | O::SelectionCollapse
        | O::SelectionSelectAllChildren | O::SelectionToString | O::SelectionType | O::SelectionDirection
        | O::SelectionCollapseToStart | O::SelectionCollapseToEnd | O::SelectionExtend
        | O::SelectionSetBaseAndExtent | O::SelectionDeleteFromDocument | O::SelectionGetComposedRanges
        | O::SelectionEventHandlerGet(_) | O::SelectionEventHandlerSet(_))
}
fn key<H: BindingHost>(host: &H, slot: usize) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena, slot: slot as u64, generation: 1 }
}
fn required_node<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<NodeIdx, H::Error> {
    value.and_then(|value| host.node_brand(value)).ok_or_else(|| host.type_error("Expected a Node"))
}
fn unsigned_long<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<usize, H::Error> {
    let number = value.map(|value| host.to_number(value)).transpose()?.unwrap_or(0.);
    Ok(if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.) as usize } else { 0 })
}
fn range_from_value<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<Range, H::Error> {
    let value = value.and_then(|value| host.platform_object(value, InterfaceBrand::RANGE))
        .ok_or_else(|| host.type_error("Expected a Range"))?;
    if value.arena != key(host, 0).arena || value.generation != 1 || value.slot > usize::MAX as u64 {
        return Err(host.type_error("Invalid Range"));
    }
    let range = Range::from_slot(value.slot as usize);
    host.with_dom(|dom| dom.range(range).map(|_| ()))?; Ok(range)
}

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if let O::SelectionEventHandlerGet(name) | O::SelectionEventHandlerSet(name) = op {
        let target = if let Some(node) = host.node_brand(&receiver) {
            host.with_dom(|dom| if matches!(dom.get(node).kind, amazing_dom::NodeKind::Document | amazing_dom::NodeKind::Element { .. }) { Ok(()) } else { Err(DomError::TypeError) })?;
            amazing_dom::events::EventTarget::Node(node)
        } else {
            let slot = host.window_event_target_slot(&receiver).ok_or_else(|| host.type_error("Expected a Window, Document or Element"))?;
            amazing_dom::events::EventTarget::External(slot)
        };
        if matches!(op, O::SelectionEventHandlerGet(_)) {
            let root = host.with_dom(|dom| Ok(dom.events().event_handler(target, name)))?;
            return Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()));
        }
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let object = host.is_object_value(value)?;
        let callback = object.then(|| host.retain_value(value.clone()));
        let old = host.with_dom(|dom| Ok(dom.events_mut().set_event_handler(target, name, callback)))?;
        if let Some(old) = old { host.release_value(old); }
        return Ok(host.undefined());
    }
    if matches!(op, O::GetSelection | O::WindowGetSelection) {
        let document = if op == O::GetSelection {
            required_node(host, Some(&receiver))?
        } else {
            // WebIDL normal-operation receiver substitution uses the actual callee global.
            let receiver = if host.is_nullish(&receiver) { host.global_object()? } else { receiver };
            host.selection_document_for_window(&receiver)?
        };
        host.with_dom(|dom| dom.selection_range(document).map(|_| ()))?;
        if !host.document_has_browsing_context(document)? { return Ok(host.null()); }
        let id = host.with_dom(|dom| dom.document_selection(document))?;
        return host.wrap_platform_object(key(host, id.slot()), InterfaceBrand::SELECTION);
    }
    let handle = host.platform_object(&receiver, InterfaceBrand::SELECTION)
        .ok_or_else(|| host.type_error("Expected a Selection"))?;
    if handle.arena != key(host, 0).arena || handle.generation != 1 || handle.slot > usize::MAX as u64 {
        return Err(host.type_error("Invalid Selection"));
    }
    let id = SelectionId::from_slot(handle.slot as usize);
    let previous_range = host.with_dom(|dom| dom.selection(id).map(|state| state.range))?;
    match op {
        O::SelectionAddRange | O::SelectionRemoveRange => {
            let range = range_from_value(host, args.first())?;
            host.with_dom(|dom| if op == O::SelectionAddRange { dom.selection_add(id, range) } else { dom.selection_remove(id, range) })?;
            if op == O::SelectionAddRange && host.with_dom(|dom| dom.selection(id).map(|state| state.range))? == Some(range) {
                host.selection_set_range_value(id, range, args.first().unwrap())?;
            }
        }
        O::SelectionRemoveAllRanges => { host.with_dom(|dom| dom.selection_clear(id))?; }
        O::SelectionCollapse => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let node = if host.is_nullish(value) { None } else { Some(required_node(host, Some(value))?) };
            let offset = unsigned_long(host, args.get(1))?;
            host.with_dom(|dom| dom.selection_collapse_at(id, node.map(|node| BoundaryPoint::new(node, offset))))?;
        }
        O::SelectionCollapseToStart | O::SelectionCollapseToEnd => {
            host.with_dom(|dom| dom.selection_collapse_to(id, op == O::SelectionCollapseToStart))?;
        }
        O::SelectionExtend => {
            let node = required_node(host, args.first())?;
            let offset = unsigned_long(host, args.get(1))?;
            host.with_dom(|dom| dom.selection_extend(id, BoundaryPoint::new(node, offset)))?;
        }
        O::SelectionSetBaseAndExtent => {
            if args.len() < 4 { return Err(host.type_error("Not enough arguments")); }
            // Convert in IDL argument order, before any native mutation/arena borrow.
            let anchor = required_node(host, args.first())?;
            let anchor_offset = unsigned_long(host, args.get(1))?;
            let focus = required_node(host, args.get(2))?;
            let focus_offset = unsigned_long(host, args.get(3))?;
            host.with_dom(|dom| dom.selection_set_base_and_extent(id, BoundaryPoint::new(anchor, anchor_offset), BoundaryPoint::new(focus, focus_offset)))?;
        }
        O::SelectionSelectAllChildren => {
            let node = required_node(host, args.first())?;
            host.with_dom(|dom| dom.selection_select_children(id, node))?;
        }
        O::SelectionDeleteFromDocument => { host.with_dom(|dom| dom.selection_delete_from_document(id))?; }
        O::SelectionGetComposedRanges => {
            let mut roots = Vec::new();
            if let Some(options) = args.first().filter(|value| !host.is_nullish(value)) {
                let value = host.get_property(options, "shadowRoots")?;
                if !host.is_undefined(&value) { roots = host.sequence_shadow_roots(&value)?; }
            }
            let ranges = host.with_dom(|dom| dom.selection_composed_ranges(id, &roots))?;
            let mut values = Vec::new();
            for range in ranges { values.push(host.wrap_selection_static_range(id, range)?); }
            return host.create_array(values);
        }
        _ => {
            if op == O::SelectionGetRangeAt {
                if args.is_empty() { return Err(host.type_error("Not enough arguments")); }
                let index = unsigned_long(host, args.first())?;
                // Conversion can reenter and change the actual retained Selection.
                let (state, points) = host.with_dom(|dom| Ok((dom.selection(id)?, dom.selection_visible_points(id)?)))?;
                if index != 0 || points.is_none() { return host.with_dom::<H::Value>(|_| Err(DomError::IndexSizeError)); }
                let range = state.range.unwrap();
                if let Some(value) = host.selection_range_value(id, range)? { return Ok(value); }
                let value = host.wrap_platform_object(key(host, range.slot()), InterfaceBrand::RANGE)?;
                host.selection_set_range_value(id, range, &value)?;
                return Ok(value);
            }
            let state = host.with_dom(|dom| dom.selection(id))?;
            if op == O::SelectionDirection {
                let direction = if state.range.is_none() { "none" } else { match state.direction {
                    SelectionDirection::Forwards => "forward", SelectionDirection::Backwards => "backward", SelectionDirection::Directionless => "none" } };
                return host.string(&direction.encode_utf16().collect::<Vec<_>>());
            }
            let points = host.with_dom(|dom| dom.selection_visible_points(id))?;
            if op == O::SelectionRangeCount { return Ok(host.number(if points.is_some() { 1. } else { 0. })); }
            if op == O::SelectionIsCollapsed { return Ok(host.boolean(points.is_none_or(|(anchor, focus)| anchor == focus))); }
            if op == O::SelectionType {
                let value = match points { None => "None", Some((a,f)) if a == f => "Caret", Some(_) => "Range" };
                return host.string(&value.encode_utf16().collect::<Vec<_>>());
            }
            if op == O::SelectionToString {
                // Existing embedding compatibility: unrendered Range text; rendered projection remains unimplemented.
                let text = host.with_dom(|dom| state.range.map_or(Ok(vec![]), |range| dom.range_to_utf16_string(range)))?;
                return host.string(&text);
            }
            let point = points.map(|(anchor, focus)| if matches!(op, O::SelectionAnchorNode | O::SelectionAnchorOffset) { anchor } else { focus });
            return match op {
                O::SelectionAnchorNode | O::SelectionFocusNode => match point { Some(point) => host.wrap_node(point.node), None => Ok(host.null()) },
                O::SelectionAnchorOffset | O::SelectionFocusOffset => Ok(host.number(point.map_or(0, |point| point.offset) as f64)),
                _ => unreachable!(),
            };
        }
    }
    let current_range = host.with_dom(|dom| dom.selection(id).map(|state| state.range))?;
    if current_range != previous_range {
        // addRange accepts an existing platform object; every other association creates a fresh Range.
        if !matches!(op, O::SelectionAddRange | O::SelectionRemoveRange | O::SelectionRemoveAllRanges) {
            if let Some(range) = current_range { host.range_created(range, crate::host::RangeCreationContext::Selection(id))?; }
        }
        // addRange already retained its actual value. Replacements/clears release the old strong root.
        if op != O::SelectionAddRange { host.selection_release_range_value(id)?; }
    }
    Ok(host.undefined())
}
