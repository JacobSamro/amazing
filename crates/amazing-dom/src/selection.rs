//! Document selection keeps a reference to one live Range, so mutations of
//! selected content follow the same boundary adjustment as ordinary ranges.
use crate::{Dom, DomError, NodeIdx, NodeKind, range::{BoundaryPoint, Range}};

impl Dom {
    pub fn selection_range(&self, document: NodeIdx) -> Result<Option<Range>, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return Err(DomError::TypeError);
        }
        Ok(self.selections.iter().find(|(doc, _)| *doc == document).and_then(|(_, range)| *range))
    }

    pub fn selection_add_range(&mut self, document: NodeIdx, range: Range) -> Result<(), DomError> {
        if self.selection_range(document)?.is_some() { return Ok(()); }
        let start = self.range(range)?.start.node;
        if self.owner_document(start).unwrap_or(start) != document { return Ok(()); }
        if let Some((_, selected)) = self.selections.iter_mut().find(|(doc, _)| *doc == document) {
            *selected = Some(range);
        } else {
            self.selections.push((document, Some(range)));
        }
        Ok(())
    }

    pub fn selection_remove_all_ranges(&mut self, document: NodeIdx) -> Result<(), DomError> {
        self.selection_range(document)?;
        if let Some((_, selected)) = self.selections.iter_mut().find(|(doc, _)| *doc == document) {
            *selected = None;
        }
        Ok(())
    }

    pub fn selection_remove_range(&mut self, document: NodeIdx, range: Range) -> Result<(), DomError> {
        if self.selection_range(document)? != Some(range) { return Err(DomError::NotFoundError); }
        self.selection_remove_all_ranges(document)
    }

    pub fn selection_collapse(&mut self, document: NodeIdx, point: Option<BoundaryPoint>) -> Result<(), DomError> {
        let Some(point) = point else { return self.selection_remove_all_ranges(document); };
        if self.owner_document(point.node).unwrap_or(point.node) != document {
            return Err(DomError::WrongDocumentError);
        }
        let range = self.create_range(document)?;
        self.range_set_start(range, point)?;
        self.range_set_end(range, point)?;
        self.selection_remove_all_ranges(document)?;
        self.selection_add_range(document, range)
    }

    pub fn selection_select_all_children(&mut self, document: NodeIdx, node: NodeIdx) -> Result<(), DomError> {
        if self.owner_document(node).unwrap_or(node) != document { return Err(DomError::WrongDocumentError); }
        let range = self.create_range(document)?;
        self.range_select_node_contents(range, node)?;
        self.selection_remove_all_ranges(document)?;
        self.selection_add_range(document, range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_retains_live_range_and_rejects_foreign_document() {
        let mut dom = Dom::new();
        let parent = dom.create_element(NodeIdx(0), "p");
        let text = dom.create_text(parent, "abc");
        let range = dom.create_range(NodeIdx(0)).unwrap();
        dom.range_set_start(range, BoundaryPoint::new(text, 1)).unwrap();
        dom.range_set_end(range, BoundaryPoint::new(text, 3)).unwrap();
        dom.selection_add_range(NodeIdx(0), range).unwrap();
        assert_eq!(dom.selection_range(NodeIdx(0)), Ok(Some(range)));
        dom.replace_character_data(text, 0, 0, &['x' as u16]).unwrap();
        assert_eq!(dom.range(range).unwrap().start.offset, 2);
        dom.selection_remove_all_ranges(NodeIdx(0)).unwrap();
        assert_eq!(dom.selection_range(NodeIdx(0)), Ok(None));
        let foreign = dom.create_document();
        dom.selection_add_range(foreign, range).unwrap();
        assert_eq!(dom.selection_range(foreign), Ok(None));
    }
}
