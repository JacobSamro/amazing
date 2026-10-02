//! HTML table parts, ordered row collections, and checked row/cell mutations.
//! Algorithms: HTML Standard §4.9 (tables), reviewed 2026-10-03.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

impl Dom {
    pub fn is_html_element_named(&self, node: NodeIdx, name: &str) -> bool {
        matches!(&self.get(node).kind, NodeKind::Element { namespace: Namespace::Html, name: actual, .. } if actual == name)
    }
    fn table_check(&self, node: NodeIdx, names: &[&str]) -> Result<(), DomError> {
        self.check_node(node)?;
        if names.iter().any(|name| self.is_html_element_named(node, name)) { Ok(()) }
        else { Err(DomError::TypeError) }
    }
    pub fn html_children_named(&self, node: NodeIdx, names: &[&str]) -> Vec<NodeIdx> {
        self.children(node).filter(|child| names.iter().any(|name| self.is_html_element_named(*child, name))).collect()
    }
    pub fn table_rows(&self, table: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.table_check(table, &["table"])?;
        let mut head = Vec::new(); let mut body = Vec::new(); let mut foot = Vec::new();
        for child in self.children(table) {
            if self.is_html_element_named(child, "tr") { body.push(child); }
            else if self.is_html_element_named(child, "thead") { head.extend(self.html_children_named(child, &["tr"])); }
            else if self.is_html_element_named(child, "tbody") { body.extend(self.html_children_named(child, &["tr"])); }
            else if self.is_html_element_named(child, "tfoot") { foot.extend(self.html_children_named(child, &["tr"])); }
        }
        head.extend(body); head.extend(foot); Ok(head)
    }
    pub fn table_part(&self, table: NodeIdx, name: &str) -> Result<Option<NodeIdx>, DomError> {
        self.table_check(table, &["table"])?;
        Ok(self.html_children_named(table, &[name]).first().copied())
    }
    fn table_part_reference(&self, table: NodeIdx, name: &str) -> Option<NodeIdx> {
        match name {
            "caption" => self.get(table).first_child,
            "thead" => self.children(table).find(|child| matches!(self.get(*child).kind, NodeKind::Element { .. })
                && !self.is_html_element_named(*child, "caption") && !self.is_html_element_named(*child, "colgroup")),
            _ => None,
        }
    }
    pub fn table_set_part(&mut self, table: NodeIdx, name: &str, value: Option<NodeIdx>) -> Result<(), DomError> {
        self.table_check(table, &["table"])?;
        if let Some(value) = value {
            self.check_node(value)?;
            if !self.is_html_element_named(value, name) { return Err(DomError::HierarchyRequestError); }
        }
        if let Some(old) = self.table_part(table, name)? { self.pre_remove(old, table)?; }
        if let Some(value) = value { let reference = self.table_part_reference(table, name); self.pre_insert(value, table, reference)?; }
        Ok(())
    }
    fn create_table_node(&mut self, owner: NodeIdx, name: &str) -> Result<NodeIdx, DomError> {
        self.create_element_local_in(self.owner_document(owner).ok_or(DomError::InvalidNode)?, Some("http://www.w3.org/1999/xhtml"), name)
    }
    pub fn table_create_part(&mut self, table: NodeIdx, name: &str) -> Result<NodeIdx, DomError> {
        if let Some(part) = self.table_part(table, name)? { return Ok(part); }
        let part = self.create_table_node(table, name)?;
        let reference = self.table_part_reference(table, name);
        self.pre_insert(part, table, reference)?; Ok(part)
    }
    pub fn table_delete_part(&mut self, table: NodeIdx, name: &str) -> Result<(), DomError> {
        if let Some(part) = self.table_part(table, name)? { self.pre_remove(part, table)?; } Ok(())
    }
    pub fn table_create_body(&mut self, table: NodeIdx) -> Result<NodeIdx, DomError> {
        self.table_check(table, &["table"])?;
        let reference = self.html_children_named(table, &["tbody"]).last().and_then(|last| self.get(*last).next_sibling);
        let body = self.create_table_node(table, "tbody")?; self.pre_insert(body, table, reference)?; Ok(body)
    }
    fn table_list(&self, node: NodeIdx, cells: bool) -> Result<Vec<NodeIdx>, DomError> {
        if cells { self.table_check(node, &["tr"])?; Ok(self.html_children_named(node, &["td", "th"])) }
        else if self.is_html_element_named(node, "table") { self.table_rows(node) }
        else { self.table_check(node, &["thead", "tbody", "tfoot"])?; Ok(self.html_children_named(node, &["tr"])) }
    }
    pub fn table_insert_item(&mut self, node: NodeIdx, index: i32, cells: bool) -> Result<NodeIdx, DomError> {
        let items = self.table_list(node, cells)?;
        if index < -1 || index as i64 > items.len() as i64 { return Err(DomError::IndexSizeError); }
        let item = self.create_table_node(node, if cells { "td" } else { "tr" })?;
        let at_end = index == -1 || index as usize == items.len();
        let (parent, reference) = if !cells && self.is_html_element_named(node, "table") {
            if items.is_empty() {
                let parent = match self.html_children_named(node, &["tbody"]).last().copied() {
                    Some(body) => body,
                    None => { let body = self.create_table_node(node, "tbody")?; self.pre_insert(body, node, None)?; body }
                }; (parent, None)
            } else if at_end { (self.get(*items.last().unwrap()).parent.unwrap(), None) }
            else { let row = items[index as usize]; (self.get(row).parent.unwrap(), Some(row)) }
        } else { (node, if at_end { None } else { Some(items[index as usize]) }) };
        self.pre_insert(item, parent, reference)?; Ok(item)
    }
    pub fn table_delete_item(&mut self, node: NodeIdx, index: i32, cells: bool) -> Result<(), DomError> {
        let items = self.table_list(node, cells)?;
        if index < -1 || index as i64 >= items.len() as i64 { return Err(DomError::IndexSizeError); }
        let item = if index == -1 { items.last().copied() } else { Some(items[index as usize]) };
        if let Some(item) = item { self.pre_remove(item, self.get(item).parent.unwrap())?; } Ok(())
    }
    pub fn table_row_index(&self, row: NodeIdx, section: bool) -> Result<i32, DomError> {
        self.table_check(row, &["tr"])?;
        let Some(parent) = self.get(row).parent else { return Ok(-1) };
        let section_parent = ["thead", "tbody", "tfoot"].iter().any(|name| self.is_html_element_named(parent, name));
        let rows = if self.is_html_element_named(parent, "table") { self.table_rows(parent)? }
        else if section_parent && section { self.html_children_named(parent, &["tr"]) }
        else if section_parent {
            let Some(table) = self.get(parent).parent.filter(|table| self.is_html_element_named(*table, "table")) else { return Ok(-1) };
            self.table_rows(table)?
        } else { return Ok(-1) };
        Ok(rows.iter().position(|candidate| *candidate == row).map_or(-1, |index| index as i32))
    }
    pub fn table_cell_index(&self, cell: NodeIdx) -> Result<i32, DomError> {
        self.table_check(cell, &["td", "th"])?;
        let Some(row) = self.get(cell).parent.filter(|row| self.is_html_element_named(*row, "tr")) else { return Ok(-1) };
        Ok(self.html_children_named(row, &["td", "th"]).iter().position(|candidate| *candidate == cell).map_or(-1, |index| index as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::CollectionFilter;
    #[test]
    fn table_rows_order_liveness_namespace_and_nested_exclusion() {
        let mut dom = Dom::new(); let doc = NodeIdx(0); let table = dom.create_element(doc, "table");
        let foot = dom.create_element(table, "tfoot"); let f = dom.create_element(foot, "tr");
        let body = dom.create_element(table, "tbody"); let b = dom.create_element(body, "tr");
        let direct = dom.create_element(table, "tr"); let head = dom.create_element(table, "thead"); let h = dom.create_element(head, "tr");
        let nested = dom.create_element(b, "table"); dom.create_element(nested, "tr");
        let foreign = dom.create_element_ns_in(doc, Some("urn:test"), "tr").unwrap(); dom.pre_insert(foreign, body, None).unwrap();
        assert_eq!(dom.table_rows(table).unwrap(), vec![h,b,direct,f]);
        let rows = dom.collection(table, CollectionFilter::TableRows).unwrap(); assert_eq!(rows, dom.collection(table, CollectionFilter::TableRows).unwrap());
        assert_eq!(dom.collection_item(rows,0).unwrap(),Some(h));
        dom.pre_remove(h,head).unwrap(); assert_eq!(dom.collection_item(rows,0).unwrap(),Some(b));
        let added = dom.table_insert_item(table,-1,false).unwrap(); assert_eq!(dom.get(added).parent,Some(foot));
        assert_eq!(dom.table_row_index(added,false).unwrap(),3);
        assert_eq!(dom.table_row_index(added,true).unwrap(),1);
        dom.table_delete_item(table,-1,false).unwrap(); assert_eq!(dom.collection_length(rows).unwrap(),3);
    }
    #[test]
    fn parts_order_validation_and_empty_row_cell_edges() {
        let mut dom = Dom::new();let table = dom.create_element(NodeIdx(0),"table");
        let text = dom.append_child(table,NodeKind::Text{data:"x".into()});
        let col = dom.create_element(table,"colgroup"); let body = dom.table_create_body(table).unwrap();
        let foot = dom.table_create_part(table,"tfoot").unwrap();let head = dom.table_create_part(table,"thead").unwrap();let cap = dom.table_create_part(table,"caption").unwrap();
        assert_eq!(dom.children(table).collect::<Vec<_>>(),vec![cap,text,col,head,body,foot]);
        assert_eq!(dom.table_create_part(table,"thead").unwrap(),head);
        assert_eq!(dom.table_set_part(table,"thead",Some(body)),Err(DomError::HierarchyRequestError));
        assert_eq!(dom.table_part(table,"thead").unwrap(),Some(head));
        dom.table_delete_part(table,"thead").unwrap();assert_eq!(dom.get(head).parent,None);
        dom.table_delete_item(table,-1,false).unwrap();
        assert_eq!(dom.table_delete_item(table,0,false),Err(DomError::IndexSizeError));
        let row = dom.table_insert_item(table,-1,false).unwrap();assert_eq!(dom.get(row).parent,Some(body));
        let cell = dom.table_insert_item(row,-1,true).unwrap();let th = dom.create_element(row,"th");
        assert_eq!(dom.table_cell_index(th).unwrap(),1);dom.table_delete_item(row,0,true).unwrap();
        assert_eq!(dom.table_cell_index(cell).unwrap(),-1);assert_eq!(dom.table_cell_index(th).unwrap(),0);
        assert_eq!(dom.table_insert_item(row,-2,true),Err(DomError::IndexSizeError));
    }
}
