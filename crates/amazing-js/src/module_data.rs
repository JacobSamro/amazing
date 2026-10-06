// Real non-JavaScript module source records. Parsing is independent of realms;
// each registry's actual interpreter allocates the default export once.

#[derive(Debug, Clone)]
enum ModuleData {
    Json(crate::json_data::JsonDocument),
    Text(crate::value::JsString),
    Bytes(Vec<u8>),
}
impl ModuleData {
    fn materialize(&self, interpreter: &mut Interpreter) -> Result<JsValue, Throw> {
        match self {
            Self::Json(document) => interpreter.binding_materialize_json(document),
            Self::Text(text) => Ok(JsValue::Str(text.clone())),
            Self::Bytes(bytes) => interpreter.binding_create_module_bytes(bytes.clone()),
        }
    }
}
impl<R: ModuleResolver> ModuleLinker<R> {
    fn is_synthetic_record(&self, id: &str) -> bool {
        self.synthetic.contains_key(id) || self.data_modules.contains_key(id)
    }
    fn insert_data_record(&mut self, id: &str, data: ModuleData) {
        self.data_modules.insert(id.to_owned(), data);
        self.records.insert(id.to_owned(), ModuleRecord {
            id: id.to_owned(), module: crate::parse_module("").expect("empty data module"),
            dependencies: BTreeMap::new(),
        });
    }
}
