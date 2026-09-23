use crate::host::{BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{range::{AbstractRange, BoundaryPoint, Range, StaticRangeId}, DomError, NodeIdx, NodeKind};
use std::cmp::Ordering;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::RangeConstructor | O::StaticRangeConstructor | O::CreateRange | O::RangeStartContainer
        | O::RangeStartOffset | O::RangeEndContainer | O::RangeEndOffset
        | O::RangeCollapsed | O::RangeCommonAncestorContainer | O::RangeSetStart
        | O::RangeSetEnd | O::RangeSetStartBefore | O::RangeSetStartAfter
        | O::RangeSetEndBefore | O::RangeSetEndAfter | O::RangeCollapse
        | O::RangeSelectNode | O::RangeSelectNodeContents | O::RangeDeleteContents
        | O::RangeExtractContents | O::RangeCloneContents | O::RangeInsertNode
        | O::RangeSurroundContents | O::RangeCloneRange | O::RangeDetach
        | O::RangeIntersectsNode | O::RangeCompareBoundaryPoints
        | O::RangeIsPointInRange | O::RangeComparePoint | O::RangeStringifier)
}

fn unsigned_long<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<u32, H::Error> {
    let number = host.to_number(value)?;
    Ok(if number.is_finite() {
        number.trunc().rem_euclid(4_294_967_296.0) as u32
    } else { 0 })
}

fn required_node<H: BindingHost>(host: &mut H, arg: Option<&H::Value>) -> Result<NodeIdx, H::Error> {
    arg.and_then(|value| host.node_brand(value))
        .ok_or_else(|| host.type_error("Expected a Node"))
}

fn range_key<H: BindingHost>(host: &H, range: Range) -> PlatformKey {
    PlatformKey {
        arena: host.node_platform_key(host.current_document()).arena,
        slot: range.slot() as u64,
        generation: 1, // Dom range slots are append-only; released slots are invalid.
    }
}
fn static_range_key<H: BindingHost>(host: &H, range: StaticRangeId) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: range.slot() as u64, generation: 1 }
}

fn static_boundaries<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<Option<AbstractRange>, H::Error> {
    let Some(key) = host.platform_object(receiver, InterfaceBrand::STATIC_RANGE) else { return Ok(None) };
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid StaticRange"));
    }
    host.with_dom(|dom| dom.static_range(StaticRangeId::from_slot(key.slot as usize)).map(|r| Some(r.boundaries())))
}

fn receiver_range<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<Range, H::Error> {
    let key = host.platform_object(receiver, InterfaceBrand::RANGE)
        .ok_or_else(|| host.type_error("Expected a Range"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena
        || key.generation != 1 || key.slot > usize::MAX as u64 {
        return Err(host.type_error("Invalid Range"));
    }
    let range = Range::from_slot(key.slot as usize);
    host.with_dom(|dom| dom.range(range).map(|_| ()))?;
    Ok(range)
}

enum Output { Void, Node(NodeIdx), Number(f64), Boolean(bool), Range(Range), String(Vec<u16>) }

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::StaticRangeConstructor {
        let dictionary = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let start_node_value = host.get_property(dictionary, "startContainer")?;
        let start_node = required_node(host, Some(&start_node_value))?;
        let start_offset_value = host.get_property(dictionary, "startOffset")?;
        let start_offset = unsigned_long(host, &start_offset_value)? as usize;
        let end_node_value = host.get_property(dictionary, "endContainer")?;
        let end_node = required_node(host, Some(&end_node_value))?;
        let end_offset_value = host.get_property(dictionary, "endOffset")?;
        let end_offset = unsigned_long(host, &end_offset_value)? as usize;
        let range = host.with_dom(|dom| dom.register_static_range(
            BoundaryPoint::new(start_node, start_offset),
            BoundaryPoint::new(end_node, end_offset),
        ))?;
        return host.wrap_platform_object(static_range_key(host, range), InterfaceBrand::STATIC_RANGE);
    }
    if matches!(op, O::RangeConstructor | O::CreateRange) {
        let document = if op == O::CreateRange {
            let node = required_node(host, Some(&receiver))?;
            host.with_dom(|dom| {
                if matches!(dom.get(node).kind, NodeKind::Document) { Ok(()) }
                else { Err(DomError::TypeError) }
            })?;
            node
        } else { host.current_document() };
        let range = host.with_dom(|dom| dom.create_range(document))?;
        return host.wrap_platform_object(range_key(host, range), InterfaceBrand::RANGE);
    }
    if matches!(op, O::RangeStartContainer | O::RangeStartOffset | O::RangeEndContainer
        | O::RangeEndOffset | O::RangeCollapsed) {
        if let Some(boundaries) = static_boundaries(host, &receiver)? {
            return Ok(match op {
                O::RangeStartContainer => host.wrap_node(boundaries.start.node)?,
                O::RangeStartOffset => host.number(boundaries.start.offset as f64),
                O::RangeEndContainer => host.wrap_node(boundaries.end.node)?,
                O::RangeEndOffset => host.number(boundaries.end.offset as f64),
                O::RangeCollapsed => host.boolean(boundaries.collapsed()),
                _ => unreachable!(),
            });
        }
    }
    let range = receiver_range(host, &receiver)?;
    let node = if matches!(op, O::RangeSetStart | O::RangeSetEnd
        | O::RangeSetStartBefore | O::RangeSetStartAfter | O::RangeSetEndBefore
        | O::RangeSetEndAfter | O::RangeSelectNode | O::RangeSelectNodeContents
        | O::RangeInsertNode | O::RangeSurroundContents | O::RangeIntersectsNode
        | O::RangeIsPointInRange | O::RangeComparePoint) {
        Some(required_node(host, args.first())?)
    } else { None };
    let offset = if matches!(op, O::RangeSetStart | O::RangeSetEnd
        | O::RangeIsPointInRange | O::RangeComparePoint) {
        let arg = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
        Some(unsigned_long(host, arg)? as usize)
    } else { None };
    let how = if op == O::RangeCompareBoundaryPoints {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        Some(unsigned_long(host, arg)? as u16)
    } else { None };
    let other = if op == O::RangeCompareBoundaryPoints {
        let value = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
        Some(receiver_range(host, value)?)
    } else { None };
    let collapse = op == O::RangeCollapse && args.first().is_some_and(|v| host.to_boolean(v));
    let result = host.with_dom(|dom| {
        let value = match op {
            O::RangeStartContainer => Output::Node(dom.range(range)?.start.node),
            O::RangeStartOffset => Output::Number(dom.range(range)?.start.offset as f64),
            O::RangeEndContainer => Output::Node(dom.range(range)?.end.node),
            O::RangeEndOffset => Output::Number(dom.range(range)?.end.offset as f64),
            O::RangeCollapsed => Output::Boolean(dom.range(range)?.collapsed()),
            O::RangeCommonAncestorContainer => Output::Node(dom.range_common_ancestor(range)?),
            O::RangeSetStart => { dom.range_set_start(range, BoundaryPoint::new(node.unwrap(), offset.unwrap()))?; Output::Void }
            O::RangeSetEnd => { dom.range_set_end(range, BoundaryPoint::new(node.unwrap(), offset.unwrap()))?; Output::Void }
            O::RangeSetStartBefore => { dom.range_set_start_before(range, node.unwrap())?; Output::Void }
            O::RangeSetStartAfter => { dom.range_set_start_after(range, node.unwrap())?; Output::Void }
            O::RangeSetEndBefore => { dom.range_set_end_before(range, node.unwrap())?; Output::Void }
            O::RangeSetEndAfter => { dom.range_set_end_after(range, node.unwrap())?; Output::Void }
            O::RangeCollapse => { dom.range_collapse(range, collapse)?; Output::Void }
            O::RangeSelectNode => { dom.range_select_node(range, node.unwrap())?; Output::Void }
            O::RangeSelectNodeContents => { dom.range_select_node_contents(range, node.unwrap())?; Output::Void }
            O::RangeDeleteContents => { dom.range_delete_contents(range)?; Output::Void }
            O::RangeExtractContents => Output::Node(dom.range_extract_contents(range)?),
            O::RangeCloneContents => Output::Node(dom.range_clone_contents(range)?),
            O::RangeInsertNode => { dom.range_insert_node(range, node.unwrap())?; Output::Void }
            O::RangeSurroundContents => { dom.range_surround_contents(range, node.unwrap())?; Output::Void }
            O::RangeCloneRange => Output::Range(dom.clone_range(range)?),
            O::RangeDetach => { dom.detach_range(range)?; Output::Void }
            O::RangeIntersectsNode => Output::Boolean(dom.range_intersects_node(range, node.unwrap())?),
            O::RangeIsPointInRange => Output::Boolean(dom.range_is_point_in_range(range,
                BoundaryPoint::new(node.unwrap(), offset.unwrap()))?),
            O::RangeComparePoint => Output::Number(dom.range_compare_point(range,
                BoundaryPoint::new(node.unwrap(), offset.unwrap()))? as f64),
            O::RangeStringifier => Output::String(dom.range_to_utf16_string(range)?),
            O::RangeCompareBoundaryPoints => Output::Number(match dom.range_compare_boundary_points(range, how.unwrap(), other.unwrap())? {
                Ordering::Less => -1., Ordering::Equal => 0., Ordering::Greater => 1.,
            }),
            _ => unreachable!("range dispatcher called for a different operation"),
        };
        Ok(value)
    })?;
    Ok(match result {
        Output::Void => host.undefined(),
        Output::Node(node) => host.wrap_node(node)?,
        Output::Number(number) => host.number(number),
        Output::Boolean(boolean) => host.boolean(boolean),
        Output::Range(other) => host.wrap_platform_object(range_key(host, other), InterfaceBrand::RANGE)?,
        Output::String(units) => host.string(&units)?,
    })
}
