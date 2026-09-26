//! Synchronous source-text modules. Resolution and linking are separate from
//! evaluation so a missing module/export is reported in the resolution phase.
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::{
    Binding, Declarator, Expr, ExprKind, Ident, Module, ModuleItem, Program, Stmt, StmtKind,
    VarDecl, VarKind,
};
use crate::env::{BindingCell, EnvHandle, EnvKind};
use crate::interp::Interpreter;
use crate::tokens::Span;
use crate::value::{Exotic, JsObject, JsValue, Property, Throw, WellKnownSymbol};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulePhase {
    Parse,
    Resolution,
    Runtime,
    Async,
}

#[derive(Debug, Clone)]
pub struct ModuleFailure {
    pub phase: ModulePhase,
    pub message: String,
    pub thrown: Option<Throw>,
}

impl ModuleFailure {
    fn new(phase: ModulePhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            thrown: None,
        }
    }
    fn runtime(error: Throw) -> Self {
        Self {
            phase: ModulePhase::Runtime,
            message: error.describe(),
            thrown: Some(error),
        }
    }
}

/// The host supplies an identifier and source for each module specifier.
/// Identifiers must be stable: they are the keys used to break dependency cycles.
pub trait ModuleResolver {
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String>;
}

impl<F> ModuleResolver for F
where
    F: Fn(&str, &str) -> Result<(String, String), String>,
{
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String> {
        self(specifier, referrer)
    }
}

pub type ModuleNamespace = BTreeMap<String, JsValue>;

#[derive(Debug, Clone)]
pub struct ModuleRecord {
    pub id: String,
    pub module: Module,
    pub dependencies: BTreeMap<String, String>,
}

pub struct ModuleLinker<R: ModuleResolver> {
    resolver: R,
    records: HashMap<String, ModuleRecord>,
}

impl<R: ModuleResolver> ModuleLinker<R> {
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            records: HashMap::new(),
        }
    }

    /// Parse the root and all transitive dependencies, then validate imports.
    pub fn link(&mut self, id: &str, source: &str) -> Result<(), ModuleFailure> {
        self.load(id.to_owned(), source.to_owned(), true)?;
        let mut cache = HashMap::new();
        for record in self.records.values() {
            for item in &record.module.items {
                if let ModuleItem::Import(import) = item {
                    for binding in &import.bindings {
                        if binding.imported != "*"
                            && !self.has_export(
                                &record.dependencies[&import.from],
                                &binding.imported,
                                &mut HashSet::new(),
                                &mut cache,
                            )?
                        {
                            return Err(ModuleFailure::new(
                                ModulePhase::Resolution,
                                format!("missing export {} from {}", binding.imported, import.from),
                            ));
                        }
                    }
                }
                if let ModuleItem::ExportNamed {
                    names,
                    from: Some(from),
                    ..
                } = item
                {
                    for name in names {
                        if !self.has_export(
                            &record.dependencies[from],
                            &name.local,
                            &mut HashSet::new(),
                            &mut cache,
                        )? {
                            return Err(ModuleFailure::new(
                                ModulePhase::Resolution,
                                format!("missing re-export {} from {from}", name.local),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn load(&mut self, id: String, source: String, root: bool) -> Result<(), ModuleFailure> {
        if self.records.contains_key(&id) {
            return Ok(());
        }
        let module = crate::parse_module(&source).map_err(|error| {
            ModuleFailure::new(
                if root {
                    ModulePhase::Parse
                } else {
                    ModulePhase::Resolution
                },
                error.to_string(),
            )
        })?;
        // Insert before recursing, so an import cycle terminates the load walk.
        self.records.insert(
            id.clone(),
            ModuleRecord {
                id: id.clone(),
                module,
                dependencies: BTreeMap::new(),
            },
        );
        let requests: Vec<String> = self.records[&id]
            .module
            .items
            .iter()
            .filter_map(|item| match item {
                ModuleItem::Import(import) => Some(import.from.clone()),
                ModuleItem::ExportNamed {
                    from: Some(from), ..
                } => Some(from.clone()),
                ModuleItem::ExportAll { from, .. } => Some(from.clone()),
                _ => None,
            })
            .collect();
        for request in requests {
            if self.records[&id].dependencies.contains_key(&request) {
                continue;
            }
            let (dep_id, dep_source) = self
                .resolver
                .resolve(&request, &id)
                .map_err(|message| ModuleFailure::new(ModulePhase::Resolution, message))?;
            self.records
                .get_mut(&id)
                .unwrap()
                .dependencies
                .insert(request, dep_id.clone());
            self.load(dep_id, dep_source, false)?;
        }
        Ok(())
    }

    fn has_export(
        &self,
        id: &str,
        name: &str,
        visiting: &mut HashSet<String>,
        cache: &mut HashMap<(String, String), bool>,
    ) -> Result<bool, ModuleFailure> {
        let key = (id.to_owned(), name.to_owned());
        if let Some(found) = cache.get(&key) {
            return Ok(*found);
        }
        if !visiting.insert(id.to_owned()) {
            return Ok(false);
        }
        let record = self
            .records
            .get(id)
            .ok_or_else(|| ModuleFailure::new(ModulePhase::Resolution, "module record missing"))?;
        let mut direct = false;
        let mut star_count = 0;
        for item in &record.module.items {
            match item {
                ModuleItem::ExportDefaultExpression(_)
                | ModuleItem::ExportDefaultDeclaration(_)
                    if name == "default" =>
                {
                    direct = true
                }
                ModuleItem::ExportNamed { names, .. }
                    if names.iter().any(|entry| entry.exported == name) =>
                {
                    direct = true
                }
                ModuleItem::ExportAll {
                    as_name: Some(alias),
                    ..
                } if alias == name => direct = true,
                ModuleItem::ExportAll {
                    as_name: None,
                    from,
                    ..
                } if name != "default" => {
                    if self.has_export(&record.dependencies[from], name, visiting, cache)? {
                        star_count += 1;
                    }
                }
                _ => {}
            }
        }
        let found = direct || star_count == 1;
        visiting.remove(id);
        cache.insert(key, found);
        Ok(found)
    }

    pub fn evaluate(
        &self,
        id: &str,
        root_interpreter: &mut Interpreter,
    ) -> Result<ModuleNamespace, ModuleFailure> {
        let mut instances = HashMap::new();
        let realm_global = root_interpreter.global_env.clone();
        // All environments and function declarations exist before any module
        // evaluates. A dependency cycle can therefore read hoisted functions
        // and observes TDZ for lexical declarations.
        for (record_id, record) in &self.records {
            let (program, default_binding) = module_program(record);
            let engine = &mut *root_interpreter;
            let module_env = EnvHandle::new(EnvKind::Module, Some(realm_global.clone()));
            module_env.bind_this(JsValue::Undefined);
            let mut meta = JsObject::ordinary();
            meta.proto = Some(engine.object_proto.clone());
            module_env
                .declare_lexical("\0import.meta", JsValue::object(meta), false)
                .map_err(ModuleFailure::runtime)?;
            predeclare_lexical(&module_env, &program).map_err(ModuleFailure::runtime)?;
            let previous = std::mem::replace(&mut engine.global_env, module_env.clone());
            let prepared = engine.prepare_module(&program);
            engine.global_env = previous;
            prepared.map_err(ModuleFailure::runtime)?;
            let mut object = JsObject::ordinary();
            object.proto = None;
            object.extensible = false;
            instances.insert(
                record_id.clone(),
                ModuleInstance {
                    env: module_env,
                    program,
                    default_binding,
                    namespace: JsValue::object(object),
                    exports: BTreeMap::new(),
                    state: ModuleState::Linked,
                },
            );
        }
        // Exported namespace aliases can refer to namespaces in a cycle.
        for (record_id, record) in &self.records {
            for item in &record.module.items {
                if let ModuleItem::ExportAll {
                    as_name: Some(name),
                    from,
                    ..
                } = item
                {
                    let dependency = &record.dependencies[from];
                    let namespace = instances[dependency].namespace.clone();
                    instances[record_id]
                        .env
                        .declare_lexical(&namespace_binding(name), namespace, false)
                        .map_err(ModuleFailure::runtime)?;
                }
            }
        }
        // Resolve aliases after every local cell exists. This follows export
        // chains without evaluating a module and detects ambiguous star exports.
        for (record_id, record) in &self.records {
            for item in &record.module.items {
                if let ModuleItem::Import(import) = item {
                    let dependency = &record.dependencies[&import.from];
                    for binding in &import.bindings {
                        let cell = if binding.imported == "*" {
                            BindingCell::new_value(instances[dependency].namespace.clone())
                        } else {
                            self.export_cell(
                                dependency,
                                &binding.imported,
                                &instances,
                                &mut HashSet::new(),
                            )?
                            .ok_or_else(|| {
                                ModuleFailure::new(
                                    ModulePhase::Resolution,
                                    format!("missing export {}", binding.imported),
                                )
                            })?
                        };
                        instances[record_id]
                            .env
                            .declare_import(&binding.local.name, cell)
                            .map_err(ModuleFailure::runtime)?;
                    }
                }
            }
        }
        for record_id in self.records.keys() {
            let names = self.export_names(record_id, &mut HashSet::new());
            let mut exports = BTreeMap::new();
            for name in names {
                if let Some(cell) =
                    self.export_cell(record_id, &name, &instances, &mut HashSet::new())?
                {
                    exports.insert(name, cell);
                }
            }
            let instance = instances.get_mut(record_id).unwrap();
            fill_namespace(&instance.namespace, &exports);
            instance.exports = exports;
        }
        self.evaluate_instance(id, root_interpreter, &mut instances)?;
        let mut result = BTreeMap::new();
        for (name, cell) in &instances[id].exports {
            result.insert(name.clone(), cell.read().map_err(ModuleFailure::runtime)?);
        }
        Ok(result)
    }

    fn evaluate_instance(
        &self,
        id: &str,
        root: &mut Interpreter,
        instances: &mut HashMap<String, ModuleInstance>,
    ) -> Result<(), ModuleFailure> {
        match instances[id].state {
            ModuleState::Evaluated | ModuleState::Evaluating => return Ok(()),
            ModuleState::Linked => {}
        }
        if self.records[id].module.has_top_level_await {
            return Err(ModuleFailure::new(
                ModulePhase::Async,
                "top-level await requires async module evaluation",
            ));
        }
        instances.get_mut(id).unwrap().state = ModuleState::Evaluating;
        for dependency in self.records[id].dependencies.values() {
            self.evaluate_instance(dependency, root, instances)?;
        }
        let instance = instances.get_mut(id).unwrap();
        let previous = std::mem::replace(&mut root.global_env, instance.env.clone());
        let evaluated = root.run_program(&instance.program);
        root.global_env = previous;
        evaluated.map_err(ModuleFailure::runtime)?;
        instance.state = ModuleState::Evaluated;
        Ok(())
    }

    fn export_names(&self, id: &str, visiting: &mut HashSet<String>) -> HashSet<String> {
        if !visiting.insert(id.to_owned()) {
            return HashSet::new();
        }
        let record = &self.records[id];
        let mut names = HashSet::new();
        for item in &record.module.items {
            match item {
                ModuleItem::ExportNamed {
                    names: exported, ..
                } => {
                    names.extend(exported.iter().map(|name| name.exported.clone()));
                }
                ModuleItem::ExportDefaultExpression(_)
                | ModuleItem::ExportDefaultDeclaration(_) => {
                    names.insert("default".into());
                }
                ModuleItem::ExportAll {
                    as_name: Some(name),
                    ..
                } => {
                    names.insert(name.clone());
                }
                ModuleItem::ExportAll {
                    as_name: None,
                    from,
                    ..
                } => {
                    names.extend(
                        self.export_names(&record.dependencies[from], visiting)
                            .into_iter()
                            .filter(|name| name != "default"),
                    );
                }
                _ => {}
            }
        }
        visiting.remove(id);
        names
    }

    fn export_cell(
        &self,
        id: &str,
        name: &str,
        instances: &HashMap<String, ModuleInstance>,
        visiting: &mut HashSet<(String, String)>,
    ) -> Result<Option<BindingCell>, ModuleFailure> {
        if !visiting.insert((id.to_owned(), name.to_owned())) {
            return Ok(None);
        }
        let record = &self.records[id];
        let mut direct = None;
        let mut stars = Vec::new();
        for item in &record.module.items {
            match item {
                ModuleItem::ExportNamed { names, from, .. } => {
                    for entry in names.iter().filter(|entry| entry.exported == name) {
                        direct = if let Some(from) = from {
                            self.export_cell(
                                &record.dependencies[from],
                                &entry.local,
                                instances,
                                visiting,
                            )?
                        } else if let Some(import) =
                            record.module.items.iter().find_map(|item| match item {
                                ModuleItem::Import(import) => import
                                    .bindings
                                    .iter()
                                    .find(|binding| binding.local.name == entry.local)
                                    .map(|binding| (import, binding)),
                                _ => None,
                            })
                        {
                            let (import, binding) = import;
                            if binding.imported == "*" {
                                Some(BindingCell::new_value(
                                    instances[&record.dependencies[&import.from]]
                                        .namespace
                                        .clone(),
                                ))
                            } else {
                                self.export_cell(
                                    &record.dependencies[&import.from],
                                    &binding.imported,
                                    instances,
                                    visiting,
                                )?
                            }
                        } else {
                            instances[id].env.local_cell(&entry.local)
                        };
                    }
                }
                ModuleItem::ExportDefaultExpression(_)
                | ModuleItem::ExportDefaultDeclaration(_)
                    if name == "default" =>
                {
                    direct = instances[id]
                        .default_binding
                        .as_ref()
                        .and_then(|binding| instances[id].env.local_cell(binding));
                }
                ModuleItem::ExportAll {
                    as_name: Some(alias),
                    ..
                } if alias == name => {
                    direct = instances[id].env.local_cell(&namespace_binding(alias));
                }
                ModuleItem::ExportAll {
                    as_name: None,
                    from,
                    ..
                } if name != "default" => {
                    if let Some(cell) =
                        self.export_cell(&record.dependencies[from], name, instances, visiting)?
                    {
                        stars.push(cell);
                    }
                }
                _ => {}
            }
        }
        visiting.remove(&(id.to_owned(), name.to_owned()));
        if direct.is_some() {
            return Ok(direct);
        }
        let first = stars.first().cloned();
        if let Some(first) = &first {
            if stars.iter().skip(1).any(|cell| !cell.same_target(first)) {
                return Err(ModuleFailure::new(
                    ModulePhase::Resolution,
                    format!("ambiguous export {name}"),
                ));
            }
        }
        Ok(first)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleState {
    Linked,
    Evaluating,
    Evaluated,
}

struct ModuleInstance {
    env: EnvHandle,
    program: Program,
    default_binding: Option<String>,
    namespace: JsValue,
    exports: BTreeMap<String, BindingCell>,
    state: ModuleState,
}

fn namespace_binding(name: &str) -> String {
    format!("\0module.namespace.{name}")
}

fn module_program(record: &ModuleRecord) -> (Program, Option<String>) {
    let mut body = Vec::new();
    let mut default_binding = None;
    for item in &record.module.items {
        match item {
            ModuleItem::Statement(stmt) => body.push(stmt.clone()),
            ModuleItem::ExportNamed {
                declaration: Some(stmt),
                ..
            } => body.push(stmt.clone()),
            ModuleItem::ExportDefaultDeclaration(stmt) => {
                let named = match &stmt.kind {
                    StmtKind::Function(function) => {
                        function.name.as_ref().map(|name| name.name.clone())
                    }
                    StmtKind::Class(class) => class.name.as_ref().map(|name| name.name.clone()),
                    _ => None,
                };
                if let Some(name) = named {
                    default_binding = Some(name);
                    body.push(stmt.clone());
                } else {
                    let expr = match &stmt.kind {
                        StmtKind::Function(function) => {
                            Expr::new(ExprKind::Function(function.clone()), stmt.span)
                        }
                        StmtKind::Class(class) => {
                            Expr::new(ExprKind::Class(class.clone()), stmt.span)
                        }
                        _ => unreachable!(),
                    };
                    default_binding = Some("\0module.default".into());
                    body.push(default_declaration(expr, stmt.span));
                }
            }
            ModuleItem::ExportDefaultExpression(expr) => {
                default_binding = Some("\0module.default".into());
                body.push(default_declaration(expr.clone(), expr.span));
            }
            _ => {}
        }
    }
    (
        Program {
            body,
            span: record.module.span,
        },
        default_binding,
    )
}

fn predeclare_lexical(env: &EnvHandle, program: &Program) -> Result<(), Throw> {
    for statement in &program.body {
        match &statement.kind {
            StmtKind::Var(decl) if decl.kind != VarKind::Var => {
                for declarator in &decl.decls {
                    for name in binding_declared_names(&declarator.binding) {
                        env.declare_uninitialized(&name, decl.kind == VarKind::Let)?;
                    }
                }
            }
            StmtKind::Class(class) => {
                if let Some(name) = &class.name {
                    env.declare_uninitialized(&name.name, true)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Namespace properties are sorted and immutable. The exotic internal slot
/// holds exporter cells; the ordinary descriptors below supply own-key order
/// while property operations read the cells through the runtime hooks.
fn fill_namespace(namespace: &JsValue, exports: &BTreeMap<String, BindingCell>) {
    let JsValue::Object(object) = namespace else {
        unreachable!()
    };
    let mut namespace = object.borrow_mut();
    namespace.exotic = Exotic::ModuleNamespace(exports.clone());
    for name in exports.keys() {
        namespace
            .define(
                name.as_str(),
                Property {
                    value: JsValue::Undefined,
                    writable: true,
                    enumerable: true,
                    configurable: false,
                    getter: None,
                    setter: None,
                },
            )
            .expect("namespace property");
    }
    namespace
        .define(
            WellKnownSymbol::ToStringTag.symbol(),
            Property {
                value: JsValue::str("Module"),
                writable: false,
                enumerable: false,
                configurable: false,
                getter: None,
                setter: None,
            },
        )
        .expect("namespace toStringTag");
}

fn binding_declared_names(binding: &Binding) -> Vec<String> {
    match binding {
        Binding::Ident(name) => vec![name.name.clone()],
        Binding::Array(items) => items
            .iter()
            .flatten()
            .flat_map(|item| binding_declared_names(item))
            .collect(),
        Binding::Object(items) => items
            .iter()
            .flat_map(|(_, value)| binding_declared_names(value))
            .collect(),
        Binding::Rest(inner) | Binding::Default(inner, _) => binding_declared_names(inner),
    }
}

fn default_declaration(expr: Expr, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Var(VarDecl {
            kind: VarKind::Const,
            resource_kind: None,
            decls: vec![Declarator {
                binding: Binding::Ident(Ident::new("\0module.default", span)),
                init: Some(expr),
                span,
            }],
        }),
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_link_and_evaluate_import_export() {
        let resolver = |specifier: &str, _: &str| match specifier {
            "./dep.js" => Ok(("dep".into(), "export const answer = 42;".into())),
            _ => Err("unresolved".into()),
        };
        let mut linker = ModuleLinker::new(resolver);
        linker
            .link(
                "root",
                "import { answer } from './dep.js'; export const result = answer + 1;",
            )
            .unwrap();
        let namespace = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(namespace["result"], JsValue::num(43.0));
    }

    #[test]
    fn module_top_level_this_is_undefined_with_global_this_bound() {
        let mut linker = ModuleLinker::new(|_: &str, _: &str| Err("unresolved".into()));
        linker.link("root", "export const result = this === undefined;").unwrap();
        let namespace = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(namespace["result"], JsValue::Bool(true));
    }

    #[test]
    fn missing_module_and_export_are_resolution_errors() {
        let mut linker = ModuleLinker::new(|_: &str, _: &str| Err("missing module".into()));
        assert_eq!(
            linker
                .link("root", "import './missing.js';")
                .unwrap_err()
                .phase,
            ModulePhase::Resolution
        );
        let mut linker =
            ModuleLinker::new(|_: &str, _: &str| Ok(("dep".into(), "export const x = 1;".into())));
        assert_eq!(
            linker
                .link("root", "import { y } from './dep.js';")
                .unwrap_err()
                .phase,
            ModulePhase::Resolution
        );
        let mut linker =
            ModuleLinker::new(|_: &str, _: &str| Ok(("dep".into(), "export const = ;".into())));
        assert_eq!(
            linker.link("root", "import './dep.js';").unwrap_err().phase,
            ModulePhase::Resolution
        );
    }

    #[test]
    fn live_import_and_namespace_follow_exporter_writes() {
        let resolver = |specifier: &str, _: &str| match specifier {
            "./dep.js" => Ok((
                "dep".into(),
                "export let x = 1; export function bump() { x = 2; }".into(),
            )),
            _ => Err("unresolved".into()),
        };
        let mut linker = ModuleLinker::new(resolver);
        linker.link("root", "import { x, bump } from './dep.js'; import * as ns from './dep.js'; bump(); export const direct = x; export const reflected = ns.x;").unwrap();
        let namespace = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(namespace["direct"], JsValue::num(2.0));
        assert_eq!(namespace["reflected"], JsValue::num(2.0));
    }

    #[test]
    fn cyclic_function_linking_and_lexical_tdz() {
        let resolver = |specifier: &str, _: &str| {
            match specifier {
            "./dep.js" => Ok(("dep".into(), "import { value } from './root.js'; export function run() { return value(); }".into())),
            "./root.js" => Ok(("root".into(), "import { run } from './dep.js'; export function value() { return 7; } export const result = run();".into())),
            _ => Err("unresolved".into()),
        }
        };
        let mut linker = ModuleLinker::new(resolver);
        linker.link("root", "import { run } from './dep.js'; export function value() { return 7; } export const result = run();").unwrap();
        let namespace = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(namespace["result"], JsValue::num(7.0));

        let resolver = |specifier: &str, _: &str| match specifier {
            "./dep.js" => Ok((
                "dep".into(),
                "import { value } from './root.js'; export const early = value;".into(),
            )),
            "./root.js" => Ok((
                "root".into(),
                "import './dep.js'; export let value = 1;".into(),
            )),
            _ => Err("unresolved".into()),
        };
        let mut linker = ModuleLinker::new(resolver);
        linker
            .link("root", "import './dep.js'; export let value = 1;")
            .unwrap();
        let error = linker
            .evaluate("root", &mut Interpreter::new())
            .unwrap_err();
        assert_eq!(error.phase, ModulePhase::Runtime);
        assert!(error.message.contains("uninitialized"));
    }

    #[test]
    fn namespace_exotic_reports_live_data_descriptor_and_rejects_writes() {
        let resolver = |_: &str, _: &str| Ok(("dep".into(), "export let x = 1;".into()));
        let mut linker = ModuleLinker::new(resolver);
        linker.link("root", "import * as ns from './dep.js'; const d = Object.getOwnPropertyDescriptor(ns, 'x'); export const writable = d.writable; export const getter = d.get; export const extensible = Object.isExtensible(ns); export const proto = Object.getPrototypeOf(ns); export const set = Reflect.set(ns, 'x', 2); export const deleted = Reflect.deleteProperty(ns, 'x'); export const unchanged = ns.x;").unwrap();
        let result = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(result["writable"], JsValue::Bool(true));
        assert_eq!(result["getter"], JsValue::Undefined);
        assert_eq!(result["extensible"], JsValue::Bool(false));
        assert_eq!(result["proto"], JsValue::Null);
        assert_eq!(result["set"], JsValue::Bool(false));
        assert_eq!(result["deleted"], JsValue::Bool(false));
        assert_eq!(result["unchanged"], JsValue::num(1.0));
    }

    #[test]
    fn import_meta_is_stable_within_each_module_and_distinct_across_modules() {
        let resolver =
            |_: &str, _: &str| Ok(("dep".into(), "export const meta = import.meta;".into()));
        let mut linker = ModuleLinker::new(resolver);
        linker.link("root", "import { meta } from './dep.js'; export const same = import.meta === import.meta; export const distinct = import.meta !== meta;").unwrap();
        let result = linker.evaluate("root", &mut Interpreter::new()).unwrap();
        assert_eq!(result["same"], JsValue::Bool(true));
        assert_eq!(result["distinct"], JsValue::Bool(true));
    }
}
