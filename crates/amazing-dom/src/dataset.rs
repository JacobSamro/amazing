//! HTML DOMStringMap name conversion and live attribute storage.
use crate::{namespace::valid_attribute_name, Dom, DomError, NodeIdx};

fn property_name(attribute: &str) -> Option<Vec<u16>> {
    let tail = attribute.strip_prefix("data-")?;
    if tail.bytes().any(|byte| byte.is_ascii_uppercase()) { return None; }
    let mut units = tail.encode_utf16().peekable();
    let mut name = Vec::new();
    while let Some(unit) = units.next() {
        if unit == u16::from(b'-') && units.peek().is_some_and(|next| (97..=122).contains(next)) {
            name.push(units.next().unwrap() - 32);
        } else { name.push(unit); }
    }
    Some(name)
}
fn attribute_name(name: &[u16], setter: bool) -> Result<String, DomError> {
    if setter && name.windows(2).any(|pair| pair[0] == 45 && (97..=122).contains(&pair[1])) {
        return Err(DomError::SyntaxError);
    }
    let mut units: Vec<u16> = "data-".encode_utf16().collect();
    for &unit in name {
        if (65..=90).contains(&unit) { units.extend([45, unit + 32]); }
        else { units.push(unit); }
    }
    String::from_utf16(&units).map_err(|_| DomError::InvalidCharacterError)
}
impl Dom {
    /// Attribute order is preserved. Only ASCII capitals are excluded; other
    /// Unicode characters and unusual valid names remain observable.
    pub fn dataset_pairs(&mut self, element: NodeIdx) -> Result<Vec<(Vec<u16>, Vec<u16>)>, DomError> {
        let attributes = self.attribute_nodes(element)?;
        let mut pairs = Vec::new();
        for attribute in attributes {
            let snapshot = self.attr_snapshot(attribute)?;
            if let Some(name) = property_name(&snapshot.name()) { pairs.push((name, snapshot.value)); }
        }
        Ok(pairs)
    }
    pub fn dataset_names(&mut self, element: NodeIdx) -> Result<Vec<Vec<u16>>, DomError> {
        Ok(self.dataset_pairs(element)?.into_iter().map(|(name, _)| name).collect())
    }
    pub fn dataset_value(&mut self, element: NodeIdx, name: &[u16]) -> Result<Option<Vec<u16>>, DomError> {
        Ok(self.dataset_pairs(element)?.into_iter().find(|(key, _)| key == name).map(|(_, value)| value))
    }
    pub fn set_dataset_value(&mut self, element: NodeIdx, name: &[u16], value: Vec<u16>) -> Result<(), DomError> {
        let name = attribute_name(name, true)?;
        if !valid_attribute_name(&name) { return Err(DomError::InvalidCharacterError); }
        self.set_attribute(element, &name, &String::from_utf16_lossy(&value))?;
        let attribute = self.get_attribute_node(element, None, &name).ok_or(DomError::InvalidStateError)?;
        self.store_attr_value_units(attribute, value)
    }
    pub fn delete_dataset_value(&mut self, element: NodeIdx, name: &[u16]) -> Result<(), DomError> {
        let name = attribute_name(name, false)?;
        self.remove_attribute(element, &name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn units(value: &str) -> Vec<u16> { value.encode_utf16().collect() }
    #[test]
    fn names_convert_only_ascii_hyphen_pairs_in_attribute_order() {
        let mut dom = Dom::new();
        let element = dom.create_element_ns(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "g").unwrap();
        for name in ["data-", "data-id-", "data--x", "data-foo-bar", "data-É", "data-X", "other"] {
            dom.set_attribute(element, name, name).unwrap();
        }
        assert_eq!(dom.dataset_names(element).unwrap(), ["", "id-", "X", "fooBar", "É"].map(units));
        dom.set_dataset_value(element, &units("middleName"), units("Ada")).unwrap();
        assert_eq!(dom.get_attribute_ns(element, None, "data-middle-name"), Some("Ada"));
        dom.delete_dataset_value(element, &units("fooBar")).unwrap();
        assert_eq!(dom.dataset_value(element, &units("fooBar")).unwrap(), None);
        dom.set_dataset_value(element, &units("fooBar"), units("last")).unwrap();
        assert_eq!(dom.dataset_names(element).unwrap().last(), Some(&units("fooBar")));
    }
    #[test]
    fn writes_preserve_code_units_and_validate_without_mutating() {
        let mut dom = Dom::new();
        let element = dom.create_element_ns(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "div").unwrap();
        let value = vec![0xd800, 0, 0xdc00];
        dom.set_dataset_value(element, &units("value"), value.clone()).unwrap();
        assert_eq!(dom.dataset_value(element, &units("value")).unwrap(), Some(value));
        let before = dom.dataset_pairs(element).unwrap();
        for name in ["bad-name", "-a"] {
            assert_eq!(dom.set_dataset_value(element, &units(name), units("bad")), Err(DomError::SyntaxError));
        }
        for name in [units("with space"), vec![0xd800], units("bad=key")] {
            assert_eq!(dom.set_dataset_value(element, &name, units("bad")), Err(DomError::InvalidCharacterError));
        }
        assert_eq!(dom.dataset_pairs(element).unwrap(), before);
        dom.delete_dataset_value(element, &units("absent")).unwrap();
    }
}
