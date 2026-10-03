use super::*;

// Inspect the actual instantiated callable, then replace only its code carrier
// to exercise native frames and the pooled-worker AST consumer independently.
fn matrix(sources: &[(&str, &str)], root: &str, expected: bool) {
    let mut failures = Vec::new();
    for shared in [false, true] {
        for actor in [false, true] {
            let sources: HashMap<String, String> = sources.iter().map(|(id, text)|
                (id.to_string(), text.to_string())).collect();
            let resolver_sources = sources.clone();
            let resolver = move |specifier: &str, _: &str| resolver_sources.get(specifier)
                .map(|source| (specifier.to_string(), source.clone()))
                .ok_or_else(|| "missing fixture".to_string());
            let mut linker = ModuleLinker::new(resolver);
            linker.link(root, &sources[root]).unwrap();
            let mut i = Interpreter::new();
            i.run_script("var saved, calls=0, marker={};").unwrap();
            linker.instantiate_pending(root, &mut i).unwrap();
            if actor {
                for (id, record) in &linker.records {
                    let instance = &linker.instances[id];
                    for item in &record.module.items {
                        let stmt = match item {
                            ModuleItem::Statement(stmt)
                            | ModuleItem::ExportNamed { declaration: Some(stmt), .. }
                            | ModuleItem::ExportDefaultDeclaration(stmt) => stmt,
                            _ => continue,
                        };
                        let StmtKind::Function(function) = &stmt.kind else { continue };
                        let name = function.name.as_ref().map(|n| n.name.as_str())
                            .or(instance.default_binding.as_deref()).unwrap();
                        let Some(cell) = instance.env.local_cell(name) else { continue };
                        let Ok(JsValue::Object(value)) = cell.read() else { continue };
                        assert!(matches!(value.borrow().func.as_ref().unwrap().body,
                            FunctionBody::Bytecode(_)), "fixture must instantiate native code");
                        value.borrow_mut().func.as_mut().unwrap().body =
                            FunctionBody::Block(function.body.clone());
                    }
                }
            }
            let outcome = if shared {
                ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), root, &mut i)
            } else { linker.evaluate(root, &mut i) };
            if !matches!(outcome, Ok(ref ns) if ns.get("ok") == Some(&JsValue::Bool(expected))) {
                failures.push(format!("shared={shared} actor={actor}: {root}: {outcome:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn anonymous_default_four_callable_kinds_exist_before_evaluation() {
    for declaration in ["function(){return 7}", "function*(){yield 7}",
        "async function(){return 7}", "async function*(){yield 7}"] {
        let source = format!("import f from 'root';export const ok=typeof f==='function'&&f.name==='default';export default {declaration}");
        matrix(&[("root", &source)], "root", true);
    }
}

#[test]
fn default_functions_in_dependency_cycles_keep_instantiation_identity() {
    for declaration in ["function(){return 7}", "function*(){yield 7}",
        "async function(){return 7}", "async function*(){yield 7}"] {
        let source = format!("import {{before}} from 'dep';import f from 'root';export const ok=before===f&&f.name==='default';export default {declaration}");
        matrix(&[("root", &source), ("dep", "import f from 'root';export const before=f;")], "root", true);
    }
}

#[test]
fn named_top_level_function_identity_and_reassignment_are_once_only() {
    matrix(&[("root", "import {before} from 'dep';export function f(){return 7}export const ok=f===before&&f()===7;"),
        ("dep", "import {f} from 'root';export const before=f;")], "root", true);
    matrix(&[("root", "import 'dep';export function f(){return 7}export function change(){f=marker}export const ok=f===marker;"),
        ("dep", "import {change} from 'root';change();")], "root", true);
}

#[test]
fn default_expression_and_class_remain_tdz_in_cycles() {
    for declaration in ["class {}", "42", "(function(){})", "(()=>7)"] {
        let source = format!("import {{before}} from 'dep';export default {declaration};export const ok=before;");
        matrix(&[("root", &source), ("dep", "import f from 'root';var before=false;try{f}catch(e){before=e instanceof ReferenceError}export {before};")], "root", true);
    }
}

#[test]
fn default_named_evaluation_preserves_class_static_timing_and_negatives() {
    for expression in ["function(){}", "()=>7", "class {static {saved=this.name}}"] {
        let source = format!("export default {expression};import f from 'root';export const ok=f.name==='default'&&(saved===undefined||saved==='default');");
        matrix(&[("root", &source)], "root", true);
    }
    for expression in ["(0,function(){})", "true?function(){}:function(){}", "({f:function(){}}).f"] {
        let source = format!("export default {expression};import f from 'root';export const ok=f.name!=='default';");
        matrix(&[("root", &source)], "root", true);
    }
}

#[test]
fn named_default_declarations_and_nested_block_functions_are_preserved() {
    matrix(&[("root", "export default function original(){return 7}import f from 'root';var inner;{function nested(){return 9}inner=nested()}export const ok=f.name==='original'&&f()===7&&inner===9;")], "root", true);
    matrix(&[("root", "export default class Original {static {saved=this.name}}import C from 'root';export const ok=C.name==='Original'&&saved==='Original';")], "root", true);
}

#[test]
fn tla_default_callable_exists_early_and_retains_identity_after_await() {
    matrix(&[("root", "import {before} from 'dep';import f from 'root';await Promise.resolve();export const ok=f===before&&f()===7;export default function(){return 7}"),
        ("dep", "import f from 'root';export const before=f;")], "root", true);
}
