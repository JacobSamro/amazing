// Portions adapted from Amazing's Boa port (boa-dev/boa,
// core/engine/src/builtins/json/mod.rs) @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Copyright (c) 2019 Jason Williams. Allocation uses the actual active realm.

impl Interpreter {
    /// Materialize validated JSON data in this interpreter's active realm.
    /// This performs no source evaluation, property lookup or author callback.
    pub fn binding_materialize_json(
        &mut self,
        document: &crate::json_data::JsonDocument,
    ) -> Result<JsValue, Throw> {
        self.materialize_json_document(document, &mut HashMap::new())
    }

    /// Parse exact JSON source units and allocate in this interpreter's realm.
    pub fn binding_parse_json_utf16(&mut self, source: &[u16]) -> Result<JsValue, Throw> {
        let document = crate::json_data::JsonDocument::parse_utf16(source)
            .map_err(|error| self.normalize_throw(Throw::syntax_error(error.to_string())))?;
        self.binding_materialize_json(&document)
    }

    fn materialize_json_document(
        &mut self,
        document: &crate::json_data::JsonDocument,
        sources: &mut HashMap<(usize, crate::value::JsString), (crate::value::JsString, JsValue)>,
    ) -> Result<JsValue, Throw> {
        self.materialize_json_node(&document.root, &document.source, sources)
    }

    fn materialize_json_node(
        &mut self,
        node: &crate::json_data::JsonNode,
        source: &[u16],
        sources: &mut HashMap<(usize, crate::value::JsString), (crate::value::JsString, JsValue)>,
    ) -> Result<JsValue, Throw> {
        use crate::json_data::JsonValue;
        match &node.value {
            JsonValue::Null => Ok(JsValue::Null),
            JsonValue::Bool(value) => Ok(JsValue::Bool(*value)),
            JsonValue::Number(value) => Ok(JsValue::num(*value)),
            JsonValue::String(units) => Ok(JsValue::str(crate::value::JsString::from_units(units.clone()))),
            JsonValue::Array(elements) => {
                let mut array = JsObject::array();
                array.proto = Some(self.array_proto.clone());
                let array = JsValue::object(array);
                let JsValue::Object(object) = &array else { unreachable!() };
                for (index, child) in elements.iter().enumerate() {
                    let value = self.materialize_json_node(child, source, sources)?;
                    let key = crate::value::JsString::from(index.to_string());
                    // Fresh own data descriptors bypass poisoned inherited setters.
                    object.borrow_mut().define(key.clone(), Property::data(value.clone())).unwrap();
                    sources.insert((std_weak_id(object), key),
                        (crate::value::JsString::from_units(source[child.span.clone()].to_vec()), value));
                }
                let mut length = object.borrow().own_prop("length").cloned().unwrap();
                length.value = JsValue::num(elements.len() as f64);
                object.borrow_mut().define("length", length).unwrap();
                Ok(array)
            }
            JsonValue::Object(members) => {
                let object_value = self.object_value();
                let JsValue::Object(object) = &object_value else { unreachable!() };
                for (key, child) in members {
                    let value = self.materialize_json_node(child, source, sources)?;
                    let key = crate::value::JsString::from_units(key.clone());
                    // Duplicate names replace data in-place; __proto__ is ordinary data.
                    object.borrow_mut().define(key.clone(), Property::data(value.clone())).unwrap();
                    sources.insert((std_weak_id(object), key),
                        (crate::value::JsString::from_units(source[child.span.clone()].to_vec()), value));
                }
                Ok(object_value)
            }
        }
    }
}
