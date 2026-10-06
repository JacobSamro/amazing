use super::*;
use crate::interp::ExecutionBackend;

#[derive(Clone)]
struct Sources(HashMap<String, ModuleSource>);
impl Sources {
    fn new() -> Self {
        let compiled = CompiledModuleSource::compile("native", &"globalThis.executed=(globalThis.executed||0)+1;export const value=42".encode_utf16().collect::<Vec<_>>()).unwrap();
        Self(HashMap::from([("native".into(), ModuleSource::CompiledSource(Rc::new(compiled)))]))
    }
    fn js(&mut self, id: &str, source: &str) { self.0.insert(id.into(), ModuleSource::JavaScript(source.into())); }
}
impl ModuleResolver for Sources {
    fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> { Err("typed source resolver required".into()) }
    fn resolve_request(&self, request: &ModuleRequest, _: &str) -> Result<LoadedModule, String> {
        let id = request.specifier.to_utf8_lossy();
        self.0.get(&id).cloned().map(|source| LoadedModule {id, source}).ok_or_else(|| "missing source".into())
    }
}
fn engine(backend: ExecutionBackend) -> Interpreter {
    let mut i=Interpreter::new();
    i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);i
}

#[test]
fn compiled_source_retains_original_units_and_real_module_parse() {
    for unit in [0xd800,0xdc00,0xfffd,0] {
        let source="export const value='".encode_utf16().chain([unit]).chain("';".encode_utf16()).collect::<Vec<_>>();
        let record=CompiledModuleSource::compile("native",&source).unwrap();
        assert_eq!(record.source_units(),source);assert_eq!(record.id(),"native");
        assert_eq!(record.module.items.len(),1);assert_eq!(record.kind(),"CompiledModuleSource");
    }
    for source in ["export const = 1", "return 1", "export {"] {
        assert!(CompiledModuleSource::compile("bad",&source.encode_utf16().collect::<Vec<_>>()).is_err());
    }
}

#[test]
fn source_phase_has_real_cached_object_without_instantiation_or_evaluation() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut i=engine(backend);let linker=Rc::new(RefCell::new(ModuleLinker::new(Sources::new())));
        let request=ModuleRequest::new("native",ImportPhase::Source,vec![]);
        let a=ModuleLinker::import_shared(&linker,&request,"root",&mut i).unwrap();
        let b=ModuleLinker::import_shared(&linker,&request,"other",&mut i).unwrap();
        assert!(i.binding_same_value(&a,&b));assert!(linker.borrow().instances.is_empty());
        assert!(!linker.borrow().expanded.contains("native"));
        assert_eq!(i.run_script("typeof executed").unwrap(),JsValue::str("undefined"));
        let JsValue::Object(object)=a else {panic!("native source")};
        assert!(matches!(&object.borrow().exotic,Exotic::ModuleSource(_)));
        let evaluated=linker.borrow_mut().load_request(&ModuleRequest::new("native",ImportPhase::Evaluation,vec![]),"root").unwrap();
        let output=ModuleLinker::evaluate_shared(&linker,&evaluated,&mut i).unwrap();
        assert_eq!(output["value"],JsValue::num(42.0));
        assert_eq!(i.run_script("executed").unwrap(),JsValue::num(1.0));
        assert!(i.binding_same_value(&JsValue::Object(object),&linker.borrow().module_source("native").unwrap()));
    }
}

#[test]
fn named_and_namespace_source_reexports_share_terminal_record_before_local_initialization() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut resolver=Sources::new();
        resolver.js("left","import source left from 'native';export {left as shared};");
        resolver.js("right","import source right from 'native';export {right as shared};");
        resolver.js("diamond","export * from 'left';export * from 'right';");
        resolver.js("cycle","export * from 'diamond';export * from 'other';");
        resolver.js("other","export * from 'cycle';");
        let mut linker=ModuleLinker::new(resolver);let mut i=engine(backend);
        linker.link("root","import {shared as a} from 'diamond';import * as ns from 'cycle';import source b from 'native';export {a,b,ns};").unwrap();
        let output=ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)),"root",&mut i).unwrap();
        let ns=i.binding_get_property(&output["ns"],"shared").unwrap();
        assert!(i.binding_same_value(&output["a"],&output["b"]));assert!(i.binding_same_value(&ns,&output["b"]));
        assert_eq!(i.run_script("typeof executed").unwrap(),JsValue::str("undefined"));
    }
}

#[test]
fn distinct_sources_are_ambiguous_and_source_records_require_canonical_identity() {
    let mut resolver=Sources::new();
    let other=CompiledModuleSource::compile("second",&"export const value=2".encode_utf16().collect::<Vec<_>>()).unwrap();
    resolver.0.insert("second".into(),ModuleSource::CompiledSource(Rc::new(other)));
    resolver.js("a","import source x from 'native';export {x};");resolver.js("b","import source x from 'second';export {x};");
    resolver.js("diamond","export * from 'a';export * from 'b';");
    let mut linker=ModuleLinker::new(resolver);
    assert!(linker.link("root","import {x} from 'diamond';").is_err());
    let wrong=CompiledModuleSource::compile("wrong",&"export const x=1".encode_utf16().collect::<Vec<_>>()).unwrap();
    assert!(linker.load_host_record(LoadedModule{id:"different".into(),source:ModuleSource::CompiledSource(Rc::new(wrong))},true).is_err());
}

#[test]
fn ordinary_javascript_and_data_modules_keep_missing_source_syntax_errors() {
    for source in [ModuleSource::JavaScript("export const value=1".into()),ModuleSource::JavaScriptUtf16("export const value=1".encode_utf16().collect()),ModuleSource::Text("text".into()),ModuleSource::JsonText("{}".into()),ModuleSource::Bytes(vec![1])] {
        let mut resolver=Sources::new();resolver.0.insert("plain".into(),source);
        let mut linker=ModuleLinker::new(resolver);let mut i=Interpreter::new();
        let request=ModuleRequest::new("plain",ImportPhase::Source,vec![]);
        let failure=ModuleLinker::import_shared(&Rc::new(RefCell::new(linker)),&request,"root",&mut i).unwrap_err();
        assert!(failure.thrown.unwrap().0!=JsValue::Undefined);
        let mut resolver=Sources::new();resolver.js("plain","export const value=1");
        linker=ModuleLinker::new(resolver);
        assert!(linker.link("root","import source x from 'plain';").is_err());
    }
}

#[test]
fn source_object_prototypes_belong_to_the_actual_registry_realm() {
    let mut i=Interpreter::new();let parent=i.binding_current_realm();
    let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
    let mut values=Vec::new();let mut constructors=Vec::new();
    for realm in [parent,child] {
        i.binding_with_realm(realm,|i| {
            let linker=Rc::new(RefCell::new(ModuleLinker::new(Sources::new())));
            values.push(ModuleLinker::import_shared(&linker,&ModuleRequest::new("native",ImportPhase::Source,vec![]),"root",i).unwrap());
            constructors.push(i.binding_abstract_module_source_constructor());Ok(JsValue::Undefined)
        }).unwrap();
    }
    assert!(!i.binding_same_value(&values[0],&values[1]));assert!(!i.binding_same_value(&constructors[0],&constructors[1]));
    for index in 0..2 {
        let JsValue::Object(source)=&values[index] else {panic!("source")};
        let prototype=source.borrow().proto.clone().unwrap();let parent=prototype.borrow().proto.clone().unwrap();
        let expected=i.binding_get_property(&constructors[index],"prototype").unwrap();
        assert!(i.binding_same_value(&JsValue::Object(parent),&expected));
    }
}


#[test]
fn source_loading_defers_missing_dependencies_and_top_level_await_until_evaluation() {
    let mut resolver=Sources::new();
    let record=CompiledModuleSource::compile("native",&"import {missing} from 'absent';await 0;export const value=1".encode_utf16().collect::<Vec<_>>()).unwrap();
    resolver.0.insert("native".into(),ModuleSource::CompiledSource(Rc::new(record)));
    let linker=Rc::new(RefCell::new(ModuleLinker::new(resolver)));let mut i=Interpreter::new();
    let source=ModuleLinker::import_shared(&linker,&ModuleRequest::new("native",ImportPhase::Source,vec![]),"root",&mut i).unwrap();
    assert!(matches!(source,JsValue::Object(_)));assert!(linker.borrow().instances.is_empty());
    assert!(linker.borrow_mut().load_request(&ModuleRequest::new("native",ImportPhase::Evaluation,vec![]),"root").is_err());
    let again=ModuleLinker::import_shared(&linker,&ModuleRequest::new("native",ImportPhase::Source,vec![]),"root",&mut i).unwrap();
    assert!(i.binding_same_value(&source,&again));
}

#[test]
fn source_aliases_remain_immutable_and_star_default_exclusion_stays_ordinary() {
    for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
        let mut resolver=Sources::new();
        resolver.js("forward","import source x from 'native';export {x as default,x};");
        resolver.js("star","export * from 'forward';");
        let mut linker=ModuleLinker::new(resolver);let mut i=engine(backend);
        linker.link("root","import source source from 'native';import * as star from 'star';export {source,star};export function reject(){try{source=1;return false}catch(e){return e instanceof TypeError}}").unwrap();
        let output=ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)),"root",&mut i).unwrap();
        assert_eq!(i.call_value(&output["reject"],JsValue::Undefined,vec![]).unwrap(),JsValue::Bool(true));
        assert_eq!(i.binding_own_property_keys(&output["star"]).unwrap(),vec!["x"]);
        let x=i.binding_get_property(&output["star"],"x").unwrap();assert!(i.binding_same_value(&x,&output["source"]));
    }
}
