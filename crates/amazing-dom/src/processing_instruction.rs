//! Processing instruction pseudo-attributes (DOM §4.13 / XML stylesheet §3).
use crate::{Dom, DomError, NodeIdx, NodeKind};
pub(crate) type Attributes = Vec<(Vec<u16>, Vec<u16>)>;
fn valid_attribute_name(name: &[u16]) -> bool {
    !name.is_empty() && !name.iter().any(|c| matches!(*c, 0 | 9 | 10 | 12 | 13 | 32 | 47 | 61 | 62))
}
fn space(c: u16) -> bool { matches!(c, 9 | 10 | 13 | 32) }
fn entity(value: &[u16]) -> Option<Vec<u16>> {
    let value = String::from_utf16(value).ok()?;
    let named = match value.as_str() {
        "amp" => Some('&'), "lt" => Some('<'), "gt" => Some('>'),
        "quot" => Some('"'), "apos" => Some('\''), _ => None,
    };
    let ch = if let Some(ch) = named { ch } else {
        let (digits, radix) = if let Some(v) = value.strip_prefix("#x") { (v, 16) }
            else { (value.strip_prefix('#')?, 10) };
        if digits.is_empty() || !digits.chars().all(|c| if radix == 16 { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }) { return None; }
        let number = u32::from_str_radix(digits, radix).ok()?;
        if !matches!(number, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff) { return None; }
        char::from_u32(number)?
    };
    Some(ch.encode_utf16(&mut [0; 2]).to_vec())
}
fn parse(data: &[u16]) -> Option<Attributes> {
    let mut attributes: Attributes = Vec::new();
    let mut cursor = 0;
    while cursor < data.len() {
        let before_space = cursor;
        while data.get(cursor).copied().is_some_and(space) { cursor += 1; }
        if cursor == data.len() { break; }
        if !attributes.is_empty() && cursor == before_space { return None; }
        let start = cursor;
        while data.get(cursor).is_some_and(|c| !space(*c) && *c != b'=' as u16) { cursor += 1; }
        let name = data[start..cursor].to_vec();
        if !crate::namespace::valid_processing_instruction_target(&String::from_utf16(&name).ok()?)
            || attributes.iter().any(|(key, _)| key == &name) { return None; }
        while data.get(cursor).copied().is_some_and(space) { cursor += 1; }
        if data.get(cursor) != Some(&(b'=' as u16)) { return None; }
        cursor += 1;
        while data.get(cursor).copied().is_some_and(space) { cursor += 1; }
        let quote = *data.get(cursor)?;
        if !matches!(quote, 34 | 39) { return None; }
        cursor += 1;
        let mut value = Vec::new();
        loop {
            let ch = *data.get(cursor)?;
            cursor += 1;
            if ch == quote { break; }
            if ch == b'<' as u16 { return None; }
            if ch == b'&' as u16 {
                let start = cursor;
                while data.get(cursor).is_some_and(|c| *c != b';' as u16) { cursor += 1; }
                data.get(cursor)?;
                value.extend(entity(&data[start..cursor])?);
                cursor += 1;
            } else { value.push(ch); }
        }
        attributes.push((name, value));
    }
    Some(attributes)
}
impl Dom {
    pub fn processing_instruction_attributes(&self, node: NodeIdx) -> Result<Attributes, DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::ProcessingInstruction { .. }) { return Err(DomError::TypeError); }
        Ok(self.get(node).processing_instruction_attributes.clone().unwrap_or_else(||
            parse(&self.character_data_utf16(node).unwrap()).unwrap_or_default()))
    }
    fn write_processing_instruction_attributes(&mut self, node: NodeIdx, attributes: Attributes) -> Result<(), DomError> {
        let mut data = Vec::new();
        for (name, value) in &attributes {
            if !data.is_empty() { data.push(32); }
            data.extend_from_slice(name);
            data.extend_from_slice(&[61, 34]);
            for &ch in value {
                let escape = match ch { 38 => Some("&amp;"), 60 => Some("&lt;"), 62 => Some("&gt;"), 34 => Some("&quot;"), _ => None };
                if let Some(escape) = escape { data.extend(escape.encode_utf16()); } else { data.push(ch); }
            }
            data.push(34);
        }
        self.replace_character_data(node, 0, self.node_length(node), &data)?;
        self.nodes[node.0 as usize].processing_instruction_attributes = Some(attributes);
        Ok(())
    }
    pub fn set_processing_instruction_attribute(&mut self, node: NodeIdx, name: &[u16], value: &[u16]) -> Result<(), DomError> {
        let mut attributes = self.processing_instruction_attributes(node)?;
        if !valid_attribute_name(name) { return Err(DomError::InvalidCharacterError); }
        if let Some((_, old)) = attributes.iter_mut().find(|(key, _)| key == name) { *old = value.to_vec(); }
        else { attributes.push((name.to_vec(), value.to_vec())); }
        self.write_processing_instruction_attributes(node, attributes)
    }
    pub fn remove_processing_instruction_attribute(&mut self, node: NodeIdx, name: &[u16]) -> Result<(), DomError> {
        let mut attributes = self.processing_instruction_attributes(node)?;
        attributes.retain(|(key, _)| key != name);
        self.write_processing_instruction_attributes(node, attributes)
    }
    pub fn toggle_processing_instruction_attribute(&mut self, node: NodeIdx, name: &[u16], force: Option<bool>) -> Result<bool, DomError> {
        let attributes = self.processing_instruction_attributes(node)?;
        if !valid_attribute_name(name) { return Err(DomError::InvalidCharacterError); }
        let present = attributes.iter().any(|(key, _)| key == name);
        let desired = force.unwrap_or(!present);
        if present != desired {
            if desired { self.set_processing_instruction_attribute(node, name, &[])?; }
            else { self.remove_processing_instruction_attribute(node, name)?; }
        }
        Ok(desired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn units(s: &str) -> Vec<u16> { s.encode_utf16().collect() }
    #[test]
    fn pseudo_attributes_parse_atomically_and_decode_only_xml_references() {
        let pairs = parse(&units(" a='&amp;&lt;&gt;&quot;&apos;&#65;&#x1F600;'\t_z = \"x\n y\" ")).unwrap();
        assert_eq!(pairs, vec![(units("a"), units("&<>\"'A😀")), (units("_z"), units("x\n y"))]);
        for input in ["a=x", "a='1'a='2'", "a='1' a='2'", "a='&unknown;'", "a='&#0;'",
            "a='&#xD800;'", "a='&#xFFFE;'", "a='&#1114112;'", "a='&#x+41;'", "a='<'",
            "a='&amp'", "0='x'", "a='1' b", "a='1'\u{c}b='2'"] {
            assert!(parse(&units(input)).is_none(), "{input}");
        }
        assert_eq!(parse(&units("\t \r\n")), Some(vec![]));
    }
    #[test]
    fn attributes_preserve_order_utf16_and_mutate_character_data_with_observers() {
        let mut dom = Dom::new();
        let node = dom.create_processing_instruction_in(NodeIdx(0), "t", &units(" a='one' b='two' ")).unwrap();
        let observer = dom.create_mutation_observer(1);
        dom.observe(observer, node, crate::mutation::ObserveOptions {
            character_data: Some(true), character_data_old_value: Some(true), ..Default::default()
        }).unwrap();
        dom.set_processing_instruction_attribute(node, &units("a"), &units("&<\">" )).unwrap();
        assert_eq!(dom.character_data(node), Some("a=\"&amp;&lt;&quot;&gt;\" b=\"two\""));
        let records = dom.take_mutation_records(observer);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].old_value.as_deref(), Some(" a='one' b='two' "));
        dom.set_processing_instruction_attribute(node, &[0xd800], &[0xdc00]).unwrap();
        let clone = dom.clone_node(node, false).unwrap();
        assert_eq!(dom.processing_instruction_attributes(clone).unwrap(), dom.processing_instruction_attributes(node).unwrap());
        assert_eq!(dom.processing_instruction_attributes(node).unwrap()[2], (vec![0xd800], vec![0xdc00]));
        dom.set_character_data(node, "a='new'".into()).unwrap();
        assert_eq!(dom.processing_instruction_attributes(node).unwrap(), vec![(units("a"), units("new"))]);
        dom.set_character_data(node, "a='new' broken".into()).unwrap();
        assert!(dom.processing_instruction_attributes(node).unwrap().is_empty());
    }
    #[test]
    fn toggle_noops_keep_raw_data_and_removal_always_normalizes() {
        let mut dom = Dom::new();
        let node = dom.create_processing_instruction_in(NodeIdx(0), "xml", &units(" a='one' ")).unwrap();
        assert!(dom.toggle_processing_instruction_attribute(node, &units("a"), Some(true)).unwrap());
        assert!(!dom.toggle_processing_instruction_attribute(node, &units("absent"), Some(false)).unwrap());
        assert_eq!(dom.character_data(node), Some(" a='one' "));
        dom.remove_processing_instruction_attribute(node, &units("missing")).unwrap();
        assert_eq!(dom.character_data(node), Some("a=\"one\""));
        assert!(!dom.toggle_processing_instruction_attribute(node, &units("a"), None).unwrap());
        assert!(dom.toggle_processing_instruction_attribute(node, &units("$"), None).unwrap());
        assert_eq!(dom.processing_instruction_attributes(node).unwrap(), vec![(units("$"), vec![])]);
        let raw = dom.character_data_utf16(node).unwrap();
        dom.replace_character_data(node, 0, raw.len(), &raw).unwrap();
        assert!(dom.processing_instruction_attributes(node).unwrap().is_empty());
        for name in ["", "a b", "a=", "a/", "a>", "a\0"] {
            assert_eq!(dom.toggle_processing_instruction_attribute(node, &units(name), None), Err(DomError::InvalidCharacterError));
        }
    }
}
