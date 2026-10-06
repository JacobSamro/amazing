
use super::*;
use std::path::PathBuf;
use amazing_js::interp::RealmHandle;
use amazing_js::module::{ModuleLinker, ModuleResolver};
    use amazing_js::value::PromiseFate;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("amazing-test262-realms-{}-{}",
                std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn put(&self, path: &str, source: &str) {
            std::fs::create_dir_all(self.0.join(path).parent().unwrap()).unwrap();
            std::fs::write(self.0.join(path), source).unwrap();
        }
        fn install(&self, i: &mut Interpreter) -> (Rc<RefCell<ModuleLinker<impl ModuleResolver + 'static + use<>>>>, String) {
            install_test262_module_loader(i, self.0.to_str().unwrap(), "entry.js").unwrap()
        }
    }
    impl Drop for Files {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn child(i: &mut Interpreter) -> RealmHandle {
        i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap()
    }
    fn eval(i: &mut Interpreter, realm: RealmHandle, source: &str, context: &str) -> JsValue {
        i.binding_with_realm(realm, |i| i.run_script_with_source_context(source, context)).unwrap()
    }
    fn get(i: &mut Interpreter, realm: RealmHandle, name: &str) -> JsValue {
        i.binding_eval_in_realm(realm, name).unwrap()
    }

    #[test]
    fn static_root_and_dynamic_import_share_only_the_same_realm_instance() {
        let files = Files::new();
        let source = "globalThis.count=(globalThis.count||0)+1;export const object={};export function realm(){return globalThis}";
        files.put("entry.js", source);
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let a = child(&mut i); let b = child(&mut i);
        let (linker, root) = files.install(&mut i);
        linker.borrow_mut().link(&root, source).unwrap();
        ModuleLinker::evaluate_shared(&linker, &root, &mut i).unwrap();
        let namespace = linker.borrow().namespace_object(&root).unwrap();
        for realm in [parent, a, b] {
            eval(&mut i, realm, "var ns,again;import('./entry.js').then(v=>ns=v);import('./entry.js').then(v=>again=v)", &root);
        }
        i.drain_microtasks().unwrap();
        assert_eq!(get(&mut i, parent, "ns"), namespace);
        let mut exports = Vec::new();
        for realm in [parent, a, b] {
            assert_eq!(get(&mut i, realm, "count===1&&ns===again&&ns.realm()===globalThis"), JsValue::Bool(true));
            exports.push(get(&mut i, realm, "ns.object"));
        }
        assert_ne!(exports[0], exports[1]); assert_ne!(exports[1], exports[2]);
    }

    #[test]
    fn cycles_and_cached_parse_and_evaluation_errors_are_realm_local() {
        let files = Files::new(); files.put("entry.js", "");
        files.put("a.js", "import {b} from './b.js';export function a(){return 11};export const result=b()");
        files.put("b.js", "import {a} from './a.js';export function b(){return a()}");
        files.put("bad.js", "export const =");
        files.put("throw.js", "globalThis.marker={};throw marker");
        let mut i=Interpreter::new(); let parent=i.binding_current_realm(); let other=child(&mut i);
        let (_, root)=files.install(&mut i);
        for realm in [parent,other] {
            eval(&mut i,realm,"var ns,p1,p2,e1,e2;import('./a.js').then(v=>ns=v);import('./bad.js').catch(e=>p1=e);import('./bad.js').catch(e=>p2=e);import('./throw.js').catch(e=>e1=e);import('./throw.js').catch(e=>e2=e)",&root);
        }
        i.drain_microtasks().unwrap();
        for realm in [parent,other] {
            assert_eq!(get(&mut i,realm,"ns.result===11&&p1===p2&&p1 instanceof SyntaxError&&e1===e2&&e1===marker"),JsValue::Bool(true));
        }
        assert_ne!(get(&mut i,parent,"p1"),get(&mut i,other,"p1"));
        assert_ne!(get(&mut i,parent,"e1"),get(&mut i,other,"e1"));
    }

    #[test]
    fn top_level_await_capabilities_and_completion_stay_with_their_registry() {
        let files=Files::new(); files.put("entry.js","");
        files.put("wait.js","globalThis.count=(globalThis.count||0)+1;await new Promise(r=>globalThis.release=r);export const object={};export const bodyRealm=globalThis");
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let other=child(&mut i);
        let (_,root)=files.install(&mut i);
        for realm in [parent,other] {
            let promise=eval(&mut i,realm,"var ns,again;import('./wait.js').then(v=>ns=v)",&root);
            assert!(matches!(i.binding_module_evaluation_state(&promise).unwrap(),PromiseFate::Pending));
        }
        i.drain_microtasks().unwrap();
        for realm in [parent,other] { assert_eq!(get(&mut i,realm,"count===1&&ns===undefined&&typeof release==='function'"),JsValue::Bool(true)); }
        eval(&mut i,parent,"release()",&root);i.drain_microtasks().unwrap();
        assert_eq!(get(&mut i,parent,"ns.bodyRealm===globalThis"),JsValue::Bool(true));
        assert_eq!(get(&mut i,other,"ns"),JsValue::Undefined);
        eval(&mut i,other,"release();import('./wait.js').then(v=>again=v)",&root);i.drain_microtasks().unwrap();
        assert_eq!(get(&mut i,other,"count===1&&ns===again&&ns.bodyRealm===globalThis"),JsValue::Bool(true));
        assert_ne!(get(&mut i,parent,"ns.object"),get(&mut i,other,"ns.object"));
    }

    #[test]
    fn filesystem_boundary_and_unsupported_typed_requests_reject_in_requesting_realm() {
        let files=Files::new(); files.put("entry.js",""); files.put("value.js","export default 1");
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let other=child(&mut i);
        let (_,root)=files.install(&mut i);
        for realm in [parent,other] {
            eval(&mut i,realm,"var missing,attrs,source,escape,malformed;import('./missing.js').catch(e=>missing=e);import('../').catch(e=>escape=e);import('./value.js',{with:{type:'unsupported'}}).catch(e=>attrs=e);import('./value.js',{with:{type:'json'}}).catch(e=>malformed=e);import.source('./value.js').catch(e=>source=e)",&root);
        }
        i.drain_microtasks().unwrap();
        for realm in [parent,other] { assert_eq!(get(&mut i,realm,"missing instanceof TypeError&&escape instanceof TypeError&&attrs instanceof TypeError&&source instanceof SyntaxError&&malformed instanceof SyntaxError"),JsValue::Bool(true)); }
    }

    #[test]
    fn first_canonical_record_and_retained_loader_lifetime_are_per_realm() {
        let files=Files::new();files.put("entry.js","");
        files.put("value.js","export const value=1;export function read(){return value}");
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let other=child(&mut i);
        let (root_linker,root)=files.install(&mut i);
        drop(root_linker); // The installed hook owns the root registry.
        eval(&mut i,parent,"var first,alias;import('./value.js').then(v=>first=v)",&root);
        i.drain_microtasks().unwrap();
        files.put("value.js","export const value=2;export function read(){return value}");
        eval(&mut i,parent,"import('././value.js').then(v=>alias=v)",&root);
        eval(&mut i,other,"var first;import('./value.js').then(v=>first=v)",&root);
        i.drain_microtasks().unwrap();
        assert_eq!(get(&mut i,parent,"first===alias&&first.read()===1"),JsValue::Bool(true));
        assert_eq!(get(&mut i,other,"first.read()===2"),JsValue::Bool(true));
        let retained=get(&mut i,parent,"first.read");
        // Replacing a host hook does not revoke genuine exported closures.
        let (_, _)=files.install(&mut i);
        assert_eq!(i.call_value(&retained,JsValue::Undefined,Vec::new()).unwrap(),JsValue::num(1.0));
    }

    #[test]
    fn arbitrary_thrown_values_and_intrinsic_error_brand_are_preserved() {
        let files=Files::new();files.put("entry.js","");
        files.put("error.js","globalThis.error=Object.create(null);error.name='TypeError';throw error");
        files.put("syntax.js","export let =");
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let other=child(&mut i);
        let (_,root)=files.install(&mut i);
        for realm in [parent,other] {
            eval(&mut i,realm,"var seen,bad,OriginalSyntax=SyntaxError;SyntaxError=function(){throw 'poisoned'};import('./error.js').catch(v=>seen=v);import('./syntax.js').catch(v=>bad=v)",&root);
        }
        i.drain_microtasks().unwrap();
        for realm in [parent,other] {
            assert_eq!(get(&mut i,realm,"seen===error&&Object.getPrototypeOf(seen)===null&&bad instanceof OriginalSyntax"),JsValue::Bool(true));
        }
    }

    #[test]
    fn absent_referrer_uses_the_real_entry_directory_without_eval_source_guessing() {
        let files=Files::new();files.put("nested/entry.js","");
        files.put("nested/value.js","export default 19");files.put("value.js","export default 99");
        let mut i=Interpreter::new();let parent=i.binding_current_realm();let other=child(&mut i);
        install_test262_module_loader(&mut i,files.0.to_str().unwrap(),"nested/entry.js").unwrap();
        for realm in [parent,other] {
            i.binding_with_realm(realm,|i|i.run_script("var found;import('./value.js').then(v=>found=v.default)")).unwrap();
        }
        i.drain_microtasks().unwrap();
        assert_eq!(get(&mut i,parent,"found"),JsValue::num(19.0));
        assert_eq!(get(&mut i,other,"found"),JsValue::num(19.0));
    }

    // Genuine whole-family composition control. This uses Runtime's actual
    // ShadowRealm/importValue implementation; no substitute host API exists.
    #[test]
    fn shadow_import_value_uses_real_child_registries_and_pending_module_completion() {
        let files=Files::new();files.put("nested/entry.js","");
        files.put("nested/value.js","globalThis.count=(globalThis.count||0)+1;export const object={};export const primitive=23;export function read(){return count}");
        files.put("nested/wait.js","await new Promise(r=>globalThis.release=r);export function read(){return 41}");
        let mut i=Interpreter::new();
        install_test262_module_loader(&mut i,files.0.to_str().unwrap(),"nested/entry.js").unwrap();
        i.run_script("var a=new ShadowRealm,b=new ShadowRealm,ar,ar2,br,n,objectError,missingError,waiting; a.importValue('./value.js','read').then(v=>ar=v);a.importValue('./value.js','read').then(v=>ar2=v);b.importValue('./value.js','read').then(v=>br=v);a.importValue('./value.js','primitive').then(v=>n=v);a.importValue('./value.js','object').catch(e=>objectError=e);a.importValue('./value.js','missing').catch(e=>missingError=e);a.importValue('./wait.js','read').then(v=>waiting=v)").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ar()===1&&ar2()===1&&br()===1&&n===23&&a.evaluate('count')===1&&b.evaluate('count')===1&&objectError instanceof TypeError&&missingError instanceof TypeError&&waiting===undefined").unwrap(),JsValue::Bool(true));
        i.run_script("a.evaluate('release()')").unwrap();i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("waiting()===41").unwrap(),JsValue::Bool(true));
    }
