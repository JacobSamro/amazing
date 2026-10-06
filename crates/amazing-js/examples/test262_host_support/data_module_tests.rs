use super::*;
use std::path::PathBuf;
use super::realm_modules::Registry;
use amazing_js::module::ModuleLinker;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_DATA: AtomicU64 = AtomicU64::new(0);
struct DataFiles(PathBuf);
impl DataFiles {
    fn new() -> Self {
        let root=std::env::temp_dir().join(format!("amazing-data-modules-{}-{}",std::process::id(),NEXT_DATA.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();Self(root)
    }
    fn put(&self,path:&str,bytes:&[u8]) {
        let file=self.0.join(path);std::fs::create_dir_all(file.parent().unwrap()).unwrap();std::fs::write(file,bytes).unwrap();
    }
    fn install(&self,i:&mut Interpreter)->(Registry,String) {
        install_test262_module_loader(i,self.0.to_str().unwrap(),"entry.js").unwrap()
    }
}
impl Drop for DataFiles {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
fn run(i:&mut Interpreter,source:&str,referrer:&str) {
    i.run_script_with_source_context(source,referrer).unwrap();i.drain_microtasks().unwrap();
}
fn truth(i:&mut Interpreter,source:&str) {assert_eq!(i.run_script(source).unwrap(),JsValue::Bool(true));}

#[test]
fn static_dynamic_json_namespace_default_and_reexports_reuse_actual_mutable_objects() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("value.json",br#"{"a":[1],"__proto__":{"x":2}}"#);
    files.put("reexport.js",b"export {default} from './value.json' with {type:'json'};");
    let mut i=Interpreter::new();let (linker,root)=files.install(&mut i);
    let source="import a from './value.json' with {type:'json'};import * as ns from './value.json' with {type:'json'};import b from './reexport.js';export {a,b,ns};";
    linker.borrow_mut().link(&root,source).unwrap();let out=ModuleLinker::evaluate_shared(&linker,&root,&mut i).unwrap();
    assert!(i.binding_same_value(&out["a"],&out["b"]));
    i.binding_define_data_property(&i.binding_global_object(),"staticValue",out["a"].clone(),amazing_js::interp::BindingPropertyFlags::METHOD).unwrap();
    run(&mut i,"var n1,n2;import('./value.json',{with:{type:'json'}}).then(v=>n1=v);import('./value.json',{with:{type:'json'}}).then(v=>n2=v)",&root);
    truth(&mut i,"n1===n2&&n1.default===staticValue&&Object.isExtensible(n1.default)&&Object.isExtensible(n1.default.a)&&Object.getPrototypeOf(n1.default)===Object.prototype&&Object.prototype.hasOwnProperty.call(n1.default,'__proto__')");
}

#[test]
fn text_and_bytes_preserve_real_input_and_type_qualified_record_identity() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("same",b"2.0");files.put("bom.txt",&[0xef,0xbb,0xbf,b'x',0xff]);files.put("raw",&[0xef,0xbb,0xbf,0,255,128]);
    let mut i=Interpreter::new();let (_,root)=files.install(&mut i);
    run(&mut i,"var j,t,b,tx,raw;import('./same',{with:{type:'json'}}).then(v=>j=v);import('./same',{with:{type:'text'}}).then(v=>t=v);import('./same',{with:{type:'bytes'}}).then(v=>b=v);import('./bom.txt',{with:{type:'text'}}).then(v=>tx=v.default);import('./raw',{with:{type:'bytes'}}).then(v=>raw=v.default)",&root);
    truth(&mut i,"j!==t&&t!==b&&j.default===2&&t.default==='2.0'&&b.default instanceof Uint8Array&&b.default.length===3&&b.default[0]===50&&tx.length===2&&tx.charCodeAt(1)===0xfffd&&raw.length===6&&raw[0]===239&&raw[3]===0&&raw[4]===255&&raw.buffer.immutable===true");
}

#[test]
fn bytes_are_immutable_real_intrinsic_views_and_no_module_text_executes() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("code.js",b"globalThis.shouldNotRun=1");
    let mut i=Interpreter::new();let (_,root)=files.install(&mut i);
    run(&mut i,"var bytes,text;var AP=ArrayBuffer.prototype,UP=Uint8Array.prototype;Uint8Array=function(){throw 'poison'};ArrayBuffer=function(){throw 'poison'};import('./code.js',{with:{type:'bytes'}}).then(v=>bytes=v.default);import('./code.js',{with:{type:'text'}}).then(v=>text=v.default)",&root);
    truth(&mut i,"typeof shouldNotRun==='undefined'&&text==='globalThis.shouldNotRun=1'&&Object.getPrototypeOf(bytes)===UP&&Object.getPrototypeOf(bytes.buffer)===AP&&bytes.buffer.immutable===true");
    let bytes=i.run_script("bytes").unwrap();let buffer=i.binding_get_property(&bytes,"buffer").unwrap();
    assert!(i.binding_detach_array_buffer(&buffer).is_err());assert!(i.binding_write_uint8_array(&bytes,0,&[99]).is_err());
}

#[test]
fn canonical_normal_result_survives_file_removal_and_json_parse_errors_are_cached() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("value",br#"{"n":1}"#);files.put("bad",b"[1,]");
    let mut i=Interpreter::new();let (_,root)=files.install(&mut i);
    run(&mut i,"var first,bad1,bad2;import('./value',{with:{type:'json'}}).then(v=>first=v);import('./bad',{with:{type:'json'}}).catch(e=>bad1=e)",&root);
    std::fs::remove_file(files.0.join("value")).unwrap();files.put("bad",b"2");
    run(&mut i,"var again;import('./value',{with:{type:'json'}}).then(v=>again=v);import('./bad',{with:{type:'json'}}).catch(e=>bad2=e)",&root);
    truth(&mut i,"first===again&&again.default.n===1&&bad1===bad2&&bad1 instanceof SyntaxError");
}

#[test]
fn admission_uses_actual_attribute_values_and_rejects_unsupported_keys_without_guessing_types() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("value",b"17");
    let mut i=Interpreter::new();let (_,root)=files.install(&mut i);
    run(&mut i,"var calls=0,ns,e1,e2,e3;var attrs={get type(){calls++;return 'json'}};import('./value',{with:attrs}).then(v=>ns=v);import('./value',{with:{type:'JSON'}}).catch(e=>e1=e);import('./value',{with:{type:'json',extra:'x'}}).catch(e=>e2=e);import('./value',{with:{type:'\\ud800'}}).catch(e=>e3=e)",&root);
    truth(&mut i,"calls===1&&ns.default===17&&e1 instanceof TypeError&&e2 instanceof TypeError&&e3 instanceof TypeError");
}

#[test]
fn each_real_realm_owns_json_and_byte_values_and_error_intrinsics() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("json",b"[1]");files.put("bytes",&[1,2]);files.put("bad",b"invalid");
    let mut i=Interpreter::new();let parent=i.binding_current_realm();let child=i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();let (_,root)=files.install(&mut i);
    let mut values=Vec::new();
    for realm in [parent,child] {
        i.binding_with_realm(realm,|i|{run(i,"var json,bytes,error;import('./json',{with:{type:'json'}}).then(v=>json=v.default);import('./bytes',{with:{type:'bytes'}}).then(v=>bytes=v.default);import('./bad',{with:{type:'json'}}).catch(e=>error=e)",&root);Ok(())}).unwrap();
        let ok=i.binding_eval_in_realm(realm,"json instanceof Array&&bytes instanceof Uint8Array&&bytes.buffer instanceof ArrayBuffer&&error instanceof SyntaxError").unwrap();assert_eq!(ok,JsValue::Bool(true));values.push(i.binding_eval_in_realm(realm,"json").unwrap());
    }
    assert!(!i.binding_same_value(&values[0],&values[1]));
}

#[test]
fn shadow_import_value_uses_typed_child_records_and_primitive_wrapping() {
    let files = DataFiles::new();
    files.put("entry.js", b""); files.put("number", b"42"); files.put("text", b"native text");
    files.put("object", b"{}"); files.put("bytes", &[1, 2]);
    let mut i = Interpreter::new(); let (_, root) = files.install(&mut i);
    // importValue has no attributes argument. Real child code issues the typed
    // request, so this exported function uses the child's genuine loader/jobs.
    files.put("typed.js", b"export function read(){return value}");
    run(&mut i, "var s=new ShadowRealm(),read;s.importValue('./typed.js','read').then(f=>read=f)", &root);
    // Returning a Promise from the wrapped callable is prohibited, so leave the
    // actual child Promise pending as a side effect and return undefined.
    files.put("start.js", b"export function start(type,path){import(path,{with:{type}}).then(ns=>globalThis.value=ns.default)}");
    run(&mut i, "var start;s.importValue('./start.js','start').then(f=>start=f)", &root);
    run(&mut i, "start('json','./number')", &root); truth(&mut i, "read()===42");
    run(&mut i, "start('text','./text')", &root); truth(&mut i, "read()==='native text'");
    for (kind, path) in [("json", "./object"), ("bytes", "./bytes")] {
        run(&mut i, &format!("start('{kind}','{path}')"), &root);
        truth(&mut i, "(()=>{try{read();return false}catch(e){return e instanceof TypeError}})()");
    }
}


#[test]
fn installed_module_source_resource_is_compiled_cached_and_reexported_across_diamonds() {
    let files=DataFiles::new();files.put("entry.js",b"");
    files.put("a.js",b"import source a from '<module source>';export {a as value};");
    files.put("b.js",b"import source b from '<module source>';export {b as value};");
    files.put("diamond.js",b"export * from './a.js';export * from './b.js';");
    let mut i=Interpreter::new();install(&mut i);let(linker,root)=files.install(&mut i);
    linker.borrow_mut().link(&root,"import {value} from './diamond.js';import * as ns from './diamond.js';import source direct from '<module source>';export {value,ns,direct};").unwrap();
    let output=ModuleLinker::evaluate_shared(&linker,&root,&mut i).unwrap();
    assert!(i.binding_same_value(&output["value"],&output["direct"]));
    let ns=i.binding_get_property(&output["ns"],"value").unwrap();assert!(i.binding_same_value(&ns,&output["direct"]));
    i.global_env.declare_var("actualSource",output["direct"].clone()).unwrap();
    truth(&mut i,"actualSource instanceof $262.AbstractModuleSource&&Object.prototype.toString.call(actualSource)==='[object CompiledModuleSource]'");
    run(&mut i,"var sourceAgain,evaluated;import.source('<module source>').then(v=>sourceAgain=v);import('<module source>').then(v=>evaluated=v)",&root);
    truth(&mut i,"sourceAgain===actualSource&&evaluated.nativeSource===true");
}

#[test]
fn installed_module_source_resource_respects_child_realms_attributes_and_real_file_absence() {
    let files=DataFiles::new();files.put("entry.js",b"");files.put("plain.js",b"export const value=1");
    let mut i=Interpreter::new();install(&mut i);let(_,root)=files.install(&mut i);
    run(&mut i,r#"var original,childSource,childConstructor;var child=$262.createRealm();childConstructor=child.AbstractModuleSource;import.source('<module source>').then(v=>original=v);child.evalScript("import.source('<module source>').then(v=>globalThis.moduleSource=v)")"#,&root);
    // Public cross-realm global access preserves the actual child's native brand.
    truth(&mut i,"original instanceof $262.AbstractModuleSource&&child.global.moduleSource instanceof childConstructor&&!(child.global.moduleSource instanceof $262.AbstractModuleSource)&&child.global.moduleSource!==original");
    run(&mut i,"var attrs,plain;import.source('<module source>',{with:{type:'text'}}).catch(e=>attrs=e);import.source('./plain.js').catch(e=>plain=e)",&root);
    truth(&mut i,"attrs instanceof TypeError&&plain instanceof SyntaxError");
}
