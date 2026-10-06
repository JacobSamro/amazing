use super::*;
use crate::value::JsString;
use std::cell::Cell;

#[derive(Clone)]
struct DataResolver {
    reads: Rc<Cell<usize>>,
    sources: Rc<RefCell<HashMap<String, ModuleSource>>>,
}
impl DataResolver {
    fn new() -> Self { Self { reads: Rc::new(Cell::new(0)), sources: Rc::new(RefCell::new(HashMap::new())) } }
    fn put(&self, id: &str, source: ModuleSource) { self.sources.borrow_mut().insert(id.into(), source); }
}
impl ModuleResolver for DataResolver {
    fn cache_loaded_requests(&self) -> bool { true }
    fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> { Err("typed data resolver".into()) }
    fn resolve_request(&self, request: &ModuleRequest, _: &str) -> Result<LoadedModule, String> {
        self.reads.set(self.reads.get()+1);
        let id = String::from_utf16(request.specifier.units()).map_err(|e| e.to_string())?;
        let source = self.sources.borrow().get(&id).cloned().ok_or("missing data source")?;
        Ok(LoadedModule { id, source })
    }
}
fn request(id: &str, phase: ImportPhase) -> ModuleRequest { ModuleRequest::new(id, phase, vec![]) }

#[test]
fn json_text_bytes_are_real_default_records_with_no_evaluated_source() {
    let resolver = DataResolver::new();
    resolver.put("json", ModuleSource::JsonText("{\"__proto__\":{\"n\":1},\"a\":[1,2]}".into()));
    resolver.put("text", ModuleSource::Text(JsString::from_units(vec![0xd800,0x0a,0x3c,0x3e])));
    resolver.put("bytes", ModuleSource::Bytes(vec![0,255,128,42]));
    let mut linker = ModuleLinker::new(resolver);
    let mut i = Interpreter::new();
    linker.link("root", "import j from 'json';import t from 'text';import b from 'bytes';export {j,t,b};").unwrap();
    let out = linker.evaluate("root", &mut i).unwrap();
    assert!(i.binding_get_property(&out["j"], "__proto__").unwrap().is_object());
    let JsValue::Str(text) = &out["t"] else { panic!("text module") };
    assert_eq!(text.units(), &[0xd800,0x0a,0x3c,0x3e]);
    assert_eq!(i.binding_buffer_source_bytes(&out["b"]).unwrap().unwrap(), vec![0,255,128,42]);
    assert_eq!(i.binding_uint8_array_info(&out["b"]).unwrap(), Some(4));
    let buffer = i.binding_get_property(&out["b"], "buffer").unwrap();
    assert_eq!(i.binding_get_property(&buffer, "immutable").unwrap(), JsValue::Bool(true));
    assert!(i.binding_detach_array_buffer(&buffer).is_err());
    assert!(i.binding_write_uint8_array(&out["b"],0,&[7]).is_err());
}

#[test]
fn normal_request_cache_survives_source_change_and_phase_crossing() {
    let resolver = DataResolver::new();
    resolver.put("json", ModuleSource::JsonText("{\"value\":1}".into()));
    let mut linker = ModuleLinker::new(resolver.clone());
    let source = linker.load_request(&request("json", ImportPhase::Source), "root").unwrap();
    assert!(linker.instances.is_empty());
    resolver.put("json", ModuleSource::JsonText("invalid changed source".into()));
    let eval = linker.load_request(&request("json", ImportPhase::Evaluation), "root").unwrap();
    assert_eq!(source,eval);assert_eq!(resolver.reads.get(),2);
    linker.load_request(&request("json", ImportPhase::Evaluation), "root").unwrap();
    assert_eq!(resolver.reads.get(),2);
    let mut i = Interpreter::new();
    let first = linker.evaluate(&eval,&mut i).unwrap()["default"].clone();
    let namespace = linker.namespace_object(&eval).unwrap();
    linker.load_request(&request("json", ImportPhase::Defer),"root").unwrap();
    let again = linker.evaluate(&eval,&mut i).unwrap()["default"].clone();
    assert!(i.binding_same_value(&first,&again));
    assert!(i.binding_same_value(&namespace,&linker.namespace_object(&eval).unwrap()));
    assert_eq!(resolver.reads.get(),3);
}

#[test]
fn every_json_value_kind_keeps_only_default_export_and_mutable_objects() {
    for text in ["null","true","false","-0","1e400","\"x\"","[]","{}","[1,2]","{\"x\":1}"] {
        let resolver=DataResolver::new();resolver.put("json",ModuleSource::JsonText(text.into()));
        let mut linker=ModuleLinker::new(resolver);let mut i=Interpreter::new();
        linker.link("root","import * as ns from 'json';export {ns};").unwrap();
        let ns=linker.evaluate("root",&mut i).unwrap()["ns"].clone();
        assert_eq!(i.binding_own_property_keys(&ns).unwrap(), vec!["default"]);
        let value=i.binding_get_property(&ns,"default").unwrap();
        if let JsValue::Object(object)=value { assert!(object.borrow().extensible); }
    }
}

#[test]
fn invalid_json_is_cached_as_a_real_syntax_error_without_script_execution() {
    let resolver=DataResolver::new();resolver.put("bad",ModuleSource::JsonText("globalThis.changed=1".into()));
    let mut linker=ModuleLinker::new(resolver);
    let first=linker.load_request(&request("bad",ImportPhase::Evaluation),"root").unwrap_err();
    let second=linker.load_request(&request("bad",ImportPhase::Evaluation),"other").unwrap_err();
    assert_eq!(first.phase,ModulePhase::Resolution);assert_eq!(second.phase,ModulePhase::Resolution);
    let mut i=Interpreter::new();
    assert!(i.binding_same_value(&first.thrown.unwrap().0,&second.thrown.unwrap().0));
    assert!(linker.instances.is_empty());assert!(linker.data_modules.is_empty());
    assert_eq!(i.run_script("typeof changed").unwrap(),JsValue::str("undefined"));
}

#[test]
fn poisoned_global_constructors_do_not_change_byte_module_intrinsics() {
    let resolver=DataResolver::new();resolver.put("bytes",ModuleSource::Bytes(vec![1,2,3]));
    let mut linker=ModuleLinker::new(resolver);let mut i=Interpreter::new();
    let expected=i.run_script("[Uint8Array.prototype,ArrayBuffer.prototype]").unwrap();
    i.run_script("Object.defineProperty(globalThis,'Uint8Array',{get(){throw 'bad'},configurable:true});ArrayBuffer=function(){throw 'bad'}").unwrap();
    linker.link("root","import bytes from 'bytes';export default bytes;").unwrap();
    let bytes=linker.evaluate("root",&mut i).unwrap()["default"].clone();
    let buffer=i.binding_get_property(&bytes,"buffer").unwrap();
    for (value,index) in [(bytes,"0"),(buffer,"1")] {
        let JsValue::Object(value)=value else { panic!("real binary object") };
        let expected=i.binding_get_property(&expected,index).unwrap();
        assert!(i.binding_same_value(&JsValue::Object(value.borrow().proto.clone().unwrap()),&expected));
    }
}

#[test]
fn separate_actual_realm_registries_materialize_distinct_data_with_own_prototypes() {
    let mut i=Interpreter::new();let parent=i.binding_current_realm();
    let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
    let mut values=Vec::new();
    for realm in [parent,child] {
        let value=i.binding_with_realm(realm,|i| {
            let resolver=DataResolver::new();resolver.put("json",ModuleSource::JsonText("[1]".into()));
            let mut linker=ModuleLinker::new(resolver);linker.link("root","import value from 'json';export default value;").unwrap();
            let value=linker.evaluate("root",i).unwrap()["default"].clone();
            let prototype=i.run_script("Array.prototype")?;
            let JsValue::Object(object)=&value else { panic!("array") };
            assert!(i.binding_same_value(&JsValue::Object(object.borrow().proto.clone().unwrap()),&prototype));
            Ok(value)
        }).unwrap();values.push(value);
    }
    assert!(!i.binding_same_value(&values[0],&values[1]));assert_eq!(i.binding_current_realm(),parent);
}

#[test]
fn validated_json_payload_and_source_phase_keep_real_data_and_syntax_brand() {
    let resolver = DataResolver::new();
    let document = crate::json_data::JsonDocument::parse_utf16(&"{\"x\":1}".encode_utf16().collect::<Vec<_>>()).unwrap();
    resolver.put("validated", ModuleSource::Json(document));
    resolver.put("javascript", ModuleSource::JavaScriptUtf16("export const value=1".encode_utf16().collect()));
    let mut linker = ModuleLinker::new(resolver);
    let mut i = Interpreter::new();
    for id in ["validated", "javascript"] {
        linker.load_request(&request(id, ImportPhase::Source), "root").unwrap();
        let error = linker.module_source(id).unwrap_err();
        assert_eq!(error.phase, ModulePhase::Resolution);
        let error = i.binding_normalize_throw(error.thrown.unwrap());
        i.binding_define_data_property(&i.binding_global_object(), "sourceError", error.0,
            crate::interp::BindingPropertyFlags::METHOD).unwrap();
        assert_eq!(i.run_script("sourceError instanceof SyntaxError").unwrap(), JsValue::Bool(true));
    }
    linker.link("root", "import data from 'validated';export default data").unwrap();
    let value = linker.evaluate("root", &mut i).unwrap()["default"].clone();
    assert_eq!(i.binding_get_property(&value, "x").unwrap(), JsValue::Num(1.0));
}
