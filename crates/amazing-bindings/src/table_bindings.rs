//! HTMLTableElement, HTMLTableSectionElement, HTMLTableRowElement and cellIndex.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{collection::CollectionFilter, DomError};

pub(crate) fn handles(op: O) -> bool { matches!(op,
    O::TableCaption | O::TableSetCaption | O::TableCreateCaption | O::TableDeleteCaption |
    O::TableHead | O::TableSetHead | O::TableCreateHead | O::TableDeleteHead |
    O::TableFoot | O::TableSetFoot | O::TableCreateFoot | O::TableDeleteFoot |
    O::TableBodies | O::TableCreateBody | O::TableRows | O::TableInsertRow | O::TableDeleteRow |
    O::SectionRows | O::SectionInsertRow | O::SectionDeleteRow |
    O::RowIndex | O::RowSectionIndex | O::RowCells | O::RowInsertCell | O::RowDeleteCell | O::CellIndex) }

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a table platform object"))?;
    let names: &[&str] = match op {
        O::SectionRows | O::SectionInsertRow | O::SectionDeleteRow => &["thead", "tbody", "tfoot"],
        O::RowIndex | O::RowSectionIndex | O::RowCells | O::RowInsertCell | O::RowDeleteCell => &["tr"],
        O::CellIndex => &["td", "th"], _ => &["table"],
    };
    host.with_dom(|dom| {
        if dom.node_generation(node).is_some() && names.iter().any(|name| dom.is_html_element_named(node, name)) { Ok(()) }
        else { Err(DomError::TypeError) }
    })?;
    if matches!(op, O::TableBodies | O::TableRows | O::SectionRows | O::RowCells) {
        let filter = match op {
            O::TableRows => CollectionFilter::TableRows,
            O::TableBodies => CollectionFilter::HtmlChildren(vec!["tbody".into()]),
            O::SectionRows => CollectionFilter::HtmlChildren(vec!["tr".into()]),
            _ => CollectionFilter::HtmlChildren(vec!["td".into(), "th".into()]),
        };
        let slot = host.with_dom(|dom| dom.collection(node, filter))?;
        let mut key = host.node_platform_key(node); key.slot = slot as u64;
        return host.wrap_platform_object(key, InterfaceBrand::COLLECTION);
    }
    if matches!(op, O::RowIndex | O::RowSectionIndex | O::CellIndex) {
        let index = host.with_dom(|dom| if op == O::CellIndex { dom.table_cell_index(node) }
            else { dom.table_row_index(node, op == O::RowSectionIndex) })?;
        return Ok(host.number(index as f64));
    }
    if op == O::TableCreateBody { let body = host.with_dom(|dom| dom.table_create_body(node))?; return host.wrap_node(body); }
    if matches!(op, O::TableInsertRow | O::TableDeleteRow | O::SectionInsertRow | O::SectionDeleteRow | O::RowInsertCell | O::RowDeleteCell) {
        let inserting = matches!(op, O::TableInsertRow | O::SectionInsertRow | O::RowInsertCell);
        let value = if let Some(value) = args.first().filter(|value| !inserting || !host.is_undefined(value)) { host.to_number(value)? }
            else if inserting { -1.0 } else { return Err(host.type_error("Not enough arguments")) };
        let index = if value.is_finite() { value.trunc().rem_euclid(4_294_967_296.0) as u32 as i32 } else { 0 };
        let cells = matches!(op, O::RowInsertCell | O::RowDeleteCell);
        if inserting { let item = host.with_dom(|dom| dom.table_insert_item(node, index, cells))?; return host.wrap_node(item); }
        host.with_dom(|dom| dom.table_delete_item(node, index, cells))?;
        return Ok(host.undefined());
    }
    let name = match op {
        O::TableCaption | O::TableSetCaption | O::TableCreateCaption | O::TableDeleteCaption => "caption",
        O::TableHead | O::TableSetHead | O::TableCreateHead | O::TableDeleteHead => "thead", _ => "tfoot",
    };
    if matches!(op, O::TableSetCaption | O::TableSetHead | O::TableSetFoot) {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let child = if host.is_nullish(value) { None } else {
            let child = host.node_brand(value).ok_or_else(|| host.type_error("Expected a table part"))?;
            // WebIDL converts caption to HTMLTableCaptionElement? and head/foot
            // to HTMLTableSectionElement? before the HTML setter validates tag.
            host.with_dom(|dom| {
                let valid = dom.node_generation(child).is_some() && if name == "caption" {
                    dom.is_html_element_named(child, "caption")
                } else { ["thead", "tbody", "tfoot"].iter().any(|tag| dom.is_html_element_named(child, tag)) };
                if valid { Ok(()) } else { Err(DomError::TypeError) }
            })?; Some(child)
        };
        host.with_dom(|dom| dom.table_set_part(node, name, child))?; return Ok(host.undefined());
    }
    if matches!(op, O::TableDeleteCaption | O::TableDeleteHead | O::TableDeleteFoot) {
        host.with_dom(|dom| dom.table_delete_part(node, name))?; return Ok(host.undefined());
    }
    let part = host.with_dom(|dom| if matches!(op, O::TableCreateCaption | O::TableCreateHead | O::TableCreateFoot) {
        dom.table_create_part(node, name).map(Some)
    } else { dom.table_part(node, name) })?;
    match part { Some(part) => host.wrap_node(part), None => Ok(host.null()) }
}
