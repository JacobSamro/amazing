// Portions ported from lucid-softworks/lumen (crates/lumen/src/interpreter.rs,
// run_program GlobalDeclarationInstantiation) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.

#[derive(Debug, Clone)]
struct GlobalDeclarationPlan {
    lexicals: Vec<(String, bool)>,
    variables: Vec<String>,
    functions: Vec<Rc<ast::FunctionItem>>,
    annex: Vec<(usize, String, bool)>,
}

impl GlobalDeclarationPlan {
    fn new(body: &[ast::Stmt], strict: bool) -> Self {
        let eligible = vm::global_annex_candidates(body, strict);
        let mut plan = Self { lexicals: Vec::new(), variables: Vec::new(), functions: Vec::new(), annex: Vec::new() };
        for statement in body {
            match &statement.kind {
                StmtKind::Var(declaration) if declaration.kind != ast::VarKind::Var => {
                    for item in &declaration.decls {
                        plan.lexicals.extend(binding_names(&item.binding).into_iter().map(|name| (name, declaration.kind == ast::VarKind::Let)));
                    }
                }
                StmtKind::Class(class) => {
                    if let Some(name) = &class.name { plan.lexicals.push((name.name.clone(), true)); }
                }
                _ => {}
            }
            if let Some(function) = labeled_function_decl(statement) {
                plan.functions.push(Rc::new(function.clone()));
            } else { plan.walk(statement, &eligible); }
        }
        plan
    }

    fn variable(&mut self, declaration: &ast::VarDecl) {
        if declaration.kind != ast::VarKind::Var { return; }
        for item in &declaration.decls {
            for name in binding_names(&item.binding) {
                if !self.variables.contains(&name) { self.variables.push(name); }
            }
        }
    }

    fn walk(&mut self, statement: &ast::Stmt, eligible: &HashSet<usize>) {
        match &statement.kind {
            StmtKind::Var(declaration) => self.variable(declaration),
            StmtKind::Function(function) => {
                if let Some(name) = &function.name {
                    let key = function.as_ref() as *const ast::FunctionItem as usize;
                    self.annex.push((key, name.name.clone(), eligible.contains(&key)));
                }
            }
            StmtKind::Block(block) => { for statement in &block.body { self.walk(statement, eligible); } }
            StmtKind::If { then, els, .. } => {
                self.walk(then, eligible);
                if let Some(els) = els { self.walk(els, eligible); }
            }
            StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
            | StmtKind::With { body, .. } | StmtKind::Labeled { body, .. } => self.walk(body, eligible),
            StmtKind::For { init, body, .. } => {
                if let Some(ForInit::Decl(declaration)) = init { self.variable(declaration); }
                self.walk(body, eligible);
            }
            StmtKind::ForIn { left, body, .. } | StmtKind::ForOf { left, body, .. } | StmtKind::ForAwaitOf { left, body, .. } => {
                if let ForLeft::Decl(declaration) = left { self.variable(declaration); }
                self.walk(body, eligible);
            }
            StmtKind::Switch { cases, .. } => {
                for case in cases { for statement in &case.body { self.walk(statement, eligible); } }
            }
            StmtKind::Try(statement) => {
                for child in &statement.block.body { self.walk(child, eligible); }
                if let Some(catch) = &statement.catch { for child in &catch.body.body { self.walk(child, eligible); } }
                if let Some(finally) = &statement.finally { for child in &finally.body { self.walk(child, eligible); } }
            }
            _ => {}
        }
    }
}

impl Interpreter {
    fn copy_annex_binding(&mut self, scope: &EnvHandle, name: &str, value: JsValue) -> Result<(), Throw> {
        if scope.kind() == EnvKind::Global {
            let reference = self.name_reference(scope, name)?;
            self.put_reference(reference, value, scope)
        } else {
            if !scope.assign(name, value.clone())? { scope.declare_var(name, value)?; }
            Ok(())
        }
    }

    fn can_declare_global_function(&mut self, object: &Obj, name: &str) -> Result<bool, Throw> {
        Ok(match self.default_get_own_property(object, &PropertyKey::from(name))? {
            Some(property) => property.configurable || (property.is_data() && property.writable && property.enumerable),
            None => self.default_is_extensible(object)?,
        })
    }

    fn global_declaration_descriptor(&mut self, object: &Obj, name: &str, value: JsValue, full: bool) -> Result<(), Throw> {
        let mut descriptor = JsObject::ordinary();
        descriptor.define("value", Property::data(value)).unwrap();
        if full {
            descriptor.define("writable", Property::data(JsValue::Bool(true))).unwrap();
            descriptor.define("enumerable", Property::data(JsValue::Bool(true))).unwrap();
            descriptor.define("configurable", Property::data(JsValue::Bool(false))).unwrap();
        }
        if !self.define_own_property(object, &PropertyKey::from(name), &JsValue::object(descriptor))? {
            return Err(Throw::type_error(format!("cannot declare global binding '{name}'")));
        }
        Ok(())
    }

    fn instantiate_global_declarations(&mut self, plan: &GlobalDeclarationPlan, env: &EnvHandle) -> Result<HashSet<String>, Throw> {
        let object = self.global_obj.clone();
        let mut lexicals = HashSet::new();
        for (name, _) in &plan.lexicals {
            if !lexicals.insert(name.clone()) || env.has_script_var(name) || env.has_global_lexical(name)
                || self.default_get_own_property(&object, &PropertyKey::from(name.as_str()))?.is_some_and(|property| !property.configurable) {
                return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
            }
        }
        for name in plan.variables.iter().chain(plan.functions.iter().filter_map(|function| function.name.as_ref().map(|name| &name.name))) {
            if lexicals.contains(name) || env.has_global_lexical(name) {
                return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
            }
        }
        let mut function_names = HashSet::new();
        let mut functions = Vec::new();
        for function in plan.functions.iter().rev() {
            let Some(name) = &function.name else { continue; };
            if !function_names.insert(name.name.clone()) { continue; }
            if !self.can_declare_global_function(&object, &name.name)? {
                return Err(Throw::type_error(format!("cannot declare global function '{}'", name.name)));
            }
            functions.push(function);
        }
        functions.reverse();
        for name in &plan.variables {
            if !function_names.contains(name) && self.default_get_own_property(&object, &PropertyKey::from(name.as_str()))?.is_none()
                && !self.default_is_extensible(&object)? {
                return Err(Throw::type_error(format!("cannot declare global variable '{name}'")));
            }
        }
        let mut annex_names = Vec::new();
        let mut annex_keys = HashSet::new();
        for (key, name, eligible) in &plan.annex {
            if *eligible && !lexicals.contains(name) && !env.has_global_lexical(name) && self.can_declare_global_function(&object, name)? {
                annex_keys.insert(*key);
                if !annex_names.contains(name) { annex_names.push(name.clone()); }
            }
        }
        // All declaration validation precedes any lexical/cell/property mutation.
        for (name, mutable) in &plan.lexicals { env.reserve_script_lexical(name, *mutable)?; }
        for function in functions {
            let name = &function.name.as_ref().unwrap().name;
            let value = self.make_function(function, env);
            let full = self.default_get_own_property(&object, &PropertyKey::from(name.as_str()))?.is_none_or(|property| property.configurable);
            self.global_declaration_descriptor(&object, name, value.clone(), full)?;
            env.install_script_var_cell(name, value);
            env.register_script_var(name);
        }
        for name in plan.variables.iter().chain(annex_names.iter()) {
            if self.default_get_own_property(&object, &PropertyKey::from(name.as_str()))?.is_none() && self.default_is_extensible(&object)? {
                let value = env.global_overlay_data_value(name).unwrap_or(JsValue::Undefined);
                self.global_declaration_descriptor(&object, name, value, true)?;
            }
            env.install_script_var_cell(name, JsValue::Undefined);
            env.register_script_var(name);
        }
        {
            let mut actor = self.annex_b_eligible.borrow_mut();
            for (key, _, _) in &plan.annex { actor.remove(key); }
            actor.extend(annex_keys);
        }
        Ok(annex_names.into_iter().collect())
    }
}

#[cfg(test)]
mod global_declaration_tests {
    use super::*;

    #[test]
    fn script_instantiation_matrix_on_both_backends() {
        let cases: &[(&[&str], &str)] = &[
            (&["var x=3"], "x===3&&globalThis.x===3"),
            (&["var x=3", "var x"], "x===3&&globalThis.x===3"),
            (&["let x=3"], "x===3&&!Object.hasOwn(globalThis,'x')"),
            (&["const x=3"], "x===3&&!Object.hasOwn(globalThis,'x')"),
            (&["let Array=3"], "Array===3&&typeof globalThis.Array==='function'"),
            (&["let parseInt=3"], "parseInt===3&&typeof globalThis.parseInt==='function'"),
            (&["class Array{}"], "typeof Array==='function'&&Array!==globalThis.Array"),
            (&["var capture=()=>x;let x=3"], "capture()===3"),
            (&["var capture=()=>x", "let x=3"], "capture()===3"),
            (&["function f(){return 1}function f(){return 2}"], "f()===2&&globalThis.f===f"),
            (&["var f=3;function f(){}"], "f===3&&globalThis.f===3"),
            (&["if(false){function f(){}}"], "f===undefined&&Object.hasOwn(globalThis,'f')"),
            (&["if(true){function f(){return 4}}"], "f()===4&&globalThis.f===f"),
            (&["{function f(){return 4}}let f=5"], "f===5&&!Object.hasOwn(globalThis,'f')"),
            (&["try{function f(){return 4}}finally{}"], "f()===4"),
            (&["try{throw 1}catch(e){function f(){return e}}"], "f()===1"),
            (&["try{}finally{function f(){return 4}}"], "f()===4"),
            (&["var completion=eval('try{3}finally{}')"], "completion===3"),
            (&["Object.defineProperty(globalThis,'x',{value:4,writable:false});var x=5"], "x===4"),
            (&["var Infinity=3"], "Infinity===1/0"),
            (&["var [x,y]=[3,4]"], "x===3&&y===4&&globalThis.x===3"),
            (&["var {x,y}={x:3,y:4}"], "x===3&&y===4&&globalThis.y===4"),
            (&["for(var x of [3,4]){}"], "x===4&&globalThis.x===4"),
            (&["eval('var x=3')"], "x===3&&globalThis.x===3"),
            (&["var x=3"], "Object.getOwnPropertyDescriptor(globalThis,'x').configurable===false"),
            (&["eval('var x=3');var capture=()=>x", "delete x", "let x=4"], "capture()===4&&!Object.hasOwn(globalThis,'x')"),
            (&["eval('var x=3');var capture=()=>x", "delete x"], "typeof x==='undefined'&&(()=>{try{capture();return false}catch(e){return e instanceof ReferenceError}})()"),
            (&["var saved;Object.defineProperty(globalThis,'f',{get(){return 1},set(value){saved=value},configurable:true})", "{function f(){return 3}}"], "f===1&&saved()===3"),
            (&["var f=1;{let f=2;if(true)function f(){return 3}}"], "f===1"),
            (&["Object.defineProperty(globalThis,'length',{get(){return 0},configurable:true})", "function length(){return ['0px','1px','2px']}"], "typeof length==='function'&&length().join(',')==='0px,1px,2px'&&globalThis.length===length&&!Object.getOwnPropertyDescriptor(globalThis,'length').get"),
            (&["Object.defineProperty(Object.getPrototypeOf(globalThis),'length',{get(){return 0},configurable:true})", "function length(){return ['0px','1px','2px']}"], "length().join(',')==='0px,1px,2px'&&Object.hasOwn(globalThis,'length')"),
        ];
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for (setup, check) in cases {
                let mut interpreter = Interpreter::new();
                for source in *setup {
                    interpreter.run_script_with_backend(source, backend).unwrap_or_else(|error| panic!("{backend:?} setup {source}: {error:?}"));
                }
                assert_eq!(interpreter.run_script_with_backend(check, backend).unwrap(), JsValue::Bool(true), "{backend:?}: {setup:?} -> {check}");
            }
        }
    }

    #[test]
    fn rejected_scripts_create_no_bindings_and_lexicals_share_across_overlays() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for (setup, rejected, absent) in [
                ("var x=1", "var absent;let x=2", "absent"),
                ("let x=1", "var absent;var x=2", "absent"),
                ("let x=1", "var absent;let x=2", "absent"),
                ("Object.defineProperty(globalThis,'x',{value:1,configurable:false})", "var absent;let x=2", "absent"),
                ("Object.preventExtensions(globalThis)", "var absent;function f(){}", "absent"),
                ("Object.defineProperty(globalThis,'x',{value:1,writable:false,configurable:false})", "var absent;function x(){}", "absent"),
                ("eval('var x=3');Reflect.deleteProperty(globalThis,'x')", "var absent;let x=4", "absent"),
            ] {
                let mut interpreter = Interpreter::new();
                interpreter.run_script_with_backend(setup, backend).unwrap();
                assert!(interpreter.run_script_with_backend(rejected, backend).is_err(), "{backend:?}: {rejected}");
                assert!(interpreter.global_env.local_cell(absent).is_none());
                assert!(interpreter.global_obj.borrow().own_prop(absent).is_none());
            }
            let mut interpreter = Interpreter::new();
            interpreter.run_script_with_backend("var capture=()=>x", backend).unwrap();
            interpreter.global_env = interpreter.binding_create_global_environment_overlay();
            interpreter.run_script_with_backend("let x=3", backend).unwrap();
            assert_eq!(interpreter.run_script_with_backend("capture()===3&&!Object.hasOwn(globalThis,'x')", backend).unwrap(), JsValue::Bool(true));
        }
    }

    #[test]
    fn global_records_are_realm_owned_and_survive_forced_workers() {
        let mut interpreter = Interpreter::new();
        interpreter.run_script("let x=1;var parentCapture=()=>x").unwrap();
        let foreign = interpreter.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
        interpreter.binding_eval_in_realm(foreign, "let x=2;var childCapture=()=>x").unwrap();
        assert_eq!(interpreter.run_script("parentCapture()===1").unwrap(), JsValue::Bool(true));
        assert_eq!(interpreter.binding_eval_in_realm(foreign, "childCapture()===2").unwrap(), JsValue::Bool(true));
        assert!(interpreter.binding_eval_in_realm(foreign, "var x").is_err());
        assert_eq!(interpreter.run_script("parentCapture()===1").unwrap(), JsValue::Bool(true));

        let mut interpreter = Interpreter::new();
        interpreter.run_script("var capture,out;async function f(){with({}){}capture=()=>x;await 0;return capture()}").unwrap();
        interpreter.global_env = interpreter.binding_create_global_environment_overlay();
        interpreter.run_script("let x=7;f().then(v=>out=v)").unwrap();
        assert_eq!(interpreter.run_script("out===7&&capture()===7&&!Object.hasOwn(globalThis,'x')").unwrap(), JsValue::Bool(true));
        for source in [
            "function f(){with({}){}let x=1;try{{function x(){}}}finally{}return x===1}f()",
            "function f(){with({}){}try{throw 1}catch(x){{function x(){return 2}}}return x()===2}f()",
            "function f(){with({}){}try{throw {x:1}}catch({x}){{function x(){}}}return typeof x==='undefined'}f()",
            "function f(){with({}){}try{function x(){return 1}return x()}finally{function y(){}}}f()===1",
            "async function f(){with({}){}let x=1;await 0;try{{function x(){}}}finally{}return x===1}var out;f().then(v=>out=v);out===true",
        ] {
            let mut interpreter = Interpreter::new();
            let result = interpreter.run_script(source).unwrap_or_else(|error| panic!("{source}: {error:?}"));
            if source.starts_with("async") {
                assert_eq!(interpreter.run_script("out===true").unwrap(), JsValue::Bool(true));
            } else { assert_eq!(result, JsValue::Bool(true), "{source}"); }
        }
    }
}
