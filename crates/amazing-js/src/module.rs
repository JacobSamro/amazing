// Declaration-instantiation portions adapted from lucid-softworks/lumen
// (crates/lumen/src/interpreter.rs, build_hoist_ops/apply_hoist_ops)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//! Synchronous source-text modules. Resolution and linking are separate from
//! evaluation so a missing module/export is reported in the resolution phase.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{
    Binding, Block, Declarator, Expr, ExprKind, Ident, Module, ModuleItem, Program, Stmt,
    StmtKind, VarDecl, VarKind,
};
use crate::env::{BindingCell, EnvHandle, EnvKind};
use crate::interp::Interpreter;
use crate::tokens::Span;
use crate::value::{
    Exotic, FunctionBody, JsObject, JsValue, PromiseFate, PromiseState, Property, Throw,
    WellKnownSymbol,
};

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
    fn syntax(phase: ModulePhase, message: impl Into<String>) -> Self {
        let message = message.into();
        Self { phase, thrown: Some(Throw::syntax_error(message.clone())), message }
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
    /// Browser hosts can expose a URL distinct from an inline record's unique identifier.
    /// The default leaves import.meta unchanged for non-browser embeddings.
    fn source_url(&self, _id: &str) -> Option<String> { None }
    /// Typed, phase-aware host loading. Legacy resolvers retain their source API.
    /// Legacy compatibility uses the scalar callback. Lossless hosts must
    /// override this method; the default does not supply UTF-16 transport.
    fn resolve_request(&self, request: &ModuleRequest, referrer: &str) -> Result<LoadedModule, String> {
        if !request.attributes.is_empty() { return Err("host does not support import attributes".into()); }
        let (id, source) = self.resolve(&request.specifier.to_utf8_lossy(), referrer)?;
        Ok(LoadedModule { id, source: ModuleSource::JavaScript(source) })
    }
    /// Opt into callback-result memoization for hosts with replayable requests.
    /// Legacy callbacks retain their per-request invocation contract; canonical
    /// records still keep their first source regardless of this capability.
    fn cache_loaded_requests(&self) -> bool { false }
    /// Hosts distinguish specifier-resolution exceptions from fetch failure.
    /// A fetch failure has no module-script error value and remains `None`.
    fn resolve_request_error(&self, _request: &ModuleRequest, _referrer: &str, _message: &str) -> Option<Throw> { None }
}

impl<F> ModuleResolver for F
where
    F: Fn(&str, &str) -> Result<(String, String), String>,
{
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String> {
        self(specifier, referrer)
    }
}

include!("module_requests.rs");
include!("module_data.rs");
include!("module_native_source.rs");

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
    load_failures: HashMap<String, CachedLoadFailure>,
    root_failures: HashMap<String, ModuleFailure>,
    requests: HashMap<String, Vec<String>>,
    request_failures: HashMap<(String, String), ModuleFailure>,
    /// Instances persist across `evaluate` calls (JS-19 item 1) so that a dynamic `import()`
    /// made through `Interpreter::binding_set_module_loader_hook` that reuses this same linker
    /// -- the normal case for an embedding with one linker per realm -- reaches an
    /// already-evaluated module by its per-realm singleton record rather than re-instantiating
    /// and re-running its top-level code (16.2.1.5.1's LoadedModules reuse). Without this, a
    /// shared or cyclic dynamic-import graph re-evaluates unboundedly
    /// (`module-graphs-does-not-hang.js`).
    instances: HashMap<String, ModuleInstance>,
    realm_global: Option<EnvHandle>,
    async_sequence: u64,
    request_metadata: HashMap<(String,String), ModuleRequest>,
    expanded: HashSet<String>,
    synthetic: HashMap<String, (JsValue, Option<JsValue>)>,
    data_modules: HashMap<String, ModuleData>,
    compiled_sources: HashMap<String, Rc<CompiledModuleSource>>,
    source_objects: HashMap<String, JsValue>,
    loaded_host_requests: HashMap<(String, String), String>,
}

#[derive(Clone)]
struct CachedLoadFailure {
    failure: ModuleFailure,
    direct_parse: bool,
}

impl<R: ModuleResolver> ModuleLinker<R> {
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            records: HashMap::new(),
            load_failures: HashMap::new(),
            root_failures: HashMap::new(),
            requests: HashMap::new(),
            request_failures: HashMap::new(),
            instances: HashMap::new(),
            realm_global: None, async_sequence: 0,
            request_metadata: HashMap::new(), expanded: HashSet::new(), synthetic: HashMap::new(), data_modules: HashMap::new(), loaded_host_requests: HashMap::new(), compiled_sources: HashMap::new(), source_objects: HashMap::new(),
        }
    }

    /// Parse the root and all transitive dependencies, then validate imports.
    pub fn link(&mut self, id: &str, source: &str) -> Result<(), ModuleFailure> {
        self.link_utf16(id, &source.encode_utf16().collect::<Vec<_>>())
    }

    /// Parse and link exact UTF-16 source units. Canonical IDs and legacy
    /// resolver locator strings remain host policy; request values stay lossless.
    pub fn link_utf16(&mut self, id: &str, source: &[u16]) -> Result<(), ModuleFailure> {
        if let Some(failure) = self.root_failures.get(id) { return Err(failure.clone()); }
        let result = self.link_inner(id, source);
        if let Err(failure) = &result { self.root_failures.insert(id.into(), failure.clone()); }
        result
    }

    fn link_inner(&mut self, id: &str, source: &[u16]) -> Result<(), ModuleFailure> {
        self.load_text_record(id, source, true)?;
        self.expand_record(id)?;
        self.first_graph_failure(id, true, &mut HashSet::new())?;
        let mut order = Vec::new();
        self.graph_order(id, &mut HashSet::new(), &mut order);
        for record_id in order {
            let record = &self.records[&record_id];
            for item in &record.module.items {
                if let ModuleItem::Import(import) = item {
                    if import.phase.as_deref() == Some("source") { self.require_module_source(&record.dependencies[&import_key(import)])?; continue; }
                    for binding in &import.bindings {
                        if binding.imported != "*"
                            && !self.has_export(
                                &record.dependencies[&import_key(import)],
                                &binding.imported,
                            )?
                        {
                            return Err(ModuleFailure::syntax(
                                ModulePhase::Resolution,
                                format!("missing export {} from {}", binding.imported, import.from),
                            ));
                        }
                    }
                }
                if let ModuleItem::ExportNamed {
                    names,
                    from: Some(from),
                    attributes, ..
                } = item
                {
                    for name in names {
                        if !self.has_export(
                            &record.dependencies[&export_key(from, attributes)],
                            &name.local,
                        )? {
                            return Err(ModuleFailure::syntax(
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

    fn cached_load_failure(&self, id: &str, root: bool) -> Option<ModuleFailure> {
        self.load_failures.get(id).map(|cached| {
            let mut failure = cached.failure.clone();
            if cached.direct_parse {
                failure.phase = if root { ModulePhase::Parse } else { ModulePhase::Resolution };
            }
            failure
        })
    }

    // Both source transport variants share this parse/cache insertion path.
    // Expansion is deliberately separate: source-phase imports must not visit
    // dependencies until an evaluation/defer request actually needs them.
    fn load_text_record(&mut self, id: &str, source: &[u16], root: bool) -> Result<(), ModuleFailure> {
        if let Some(failure) = self.cached_load_failure(id, root) { return Err(failure); }
        if self.records.contains_key(id) { return Ok(()); }
        let module = match crate::parse_module_utf16(source) {
            Ok(module) => module,
            Err(error) => {
                let failure = ModuleFailure::syntax(
                    if root { ModulePhase::Parse } else { ModulePhase::Resolution },
                    error.to_string(),
                );
                self.load_failures.insert(id.to_owned(), CachedLoadFailure {
                    failure: failure.clone(), direct_parse: true,
                });
                return Err(failure);
            }
        };
        // Insert before recursing, so an import cycle terminates the load walk.
        self.records.insert(
            id.to_owned(),
            ModuleRecord {
                id: id.to_owned(),
                module,
                dependencies: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn expand_record(&mut self, id: &str) -> Result<(), ModuleFailure> {
        if !self.expanded.insert(id.into()) { return Ok(()); }
        let requests: Vec<_> = self.records[id].module.items.iter().filter_map(item_request).collect();
        let mut seen = HashSet::new();
        let requests: Vec<_> = requests.into_iter().filter(|r| seen.insert(r.edge_key())).collect();
        self.requests.insert(id.into(), requests.iter().map(ModuleRequest::edge_key).collect());
        for request in requests {
            let key = request.edge_key();
            self.request_metadata.insert((id.into(),key.clone()),request.clone());
            match self.load_host_request_record(&request, id) {
                Ok(dep_id) => {
                    self.records.get_mut(id).unwrap().dependencies.insert(key, dep_id);
                }
                Err(failure) => {
                    self.request_failures.insert((id.into(),key),failure);
                }
            }
        }
        Ok(())
    }

    fn has_export(
        &self, id: &str, name: &str,
    ) -> Result<bool, ModuleFailure> {
        Ok(matches!(self.resolve_export(id, name, &mut HashSet::new()), ExportResolution::Binding(..) | ExportResolution::Namespace(..) | ExportResolution::Source(..)))
    }

    pub fn evaluate(
        &mut self,
        id: &str,
        root_interpreter: &mut Interpreter,
    ) -> Result<ModuleNamespace, ModuleFailure> {
        self.instantiate_pending(id, root_interpreter)?;
        self.evaluate_instance(id, root_interpreter)?;
        let mut result = BTreeMap::new();
        for (name, cell) in &self.instances[id].exports {
            result.insert(name.clone(), cell.read().map_err(ModuleFailure::runtime)?);
        }
        Ok(result)
    }

    /// Evaluate a host-owned shared registry without holding its RefCell borrow
    /// while JavaScript runs. A module body (or a job it drains) may invoke the
    /// same registry's dynamic-import loader recursively.
    pub fn evaluate_shared(
        linker: &Rc<RefCell<Self>>, id: &str, root: &mut Interpreter,
    ) -> Result<ModuleNamespace, ModuleFailure> where R: 'static {
        let promise = Self::evaluate_async_shared(linker,id,root)?;
        root.module_legacy_checkpoint().map_err(ModuleFailure::runtime)?;
        match promise_fate(&promise) {
            Some(PromiseFate::Rejected(value)) => return Err(ModuleFailure::runtime(Throw(value))),
            Some(PromiseFate::Fulfilled(_)) => {},
            _ => return Err(ModuleFailure::new(ModulePhase::Async,"module evaluation is pending; use evaluate_async_shared")),
        }
        let registry = linker.borrow();
        registry.instances[id].exports.iter().map(|(name, cell)| {
            Ok((name.clone(), cell.read().map_err(ModuleFailure::runtime)?))
        }).collect()
    }

    /// The live namespace exotic object for an already-evaluated (or at least instantiated)
    /// module, keyed by the same id `link`/`evaluate` use. Unlike `evaluate`'s returned
    /// `ModuleNamespace` (a flattened, one-shot snapshot), this is the actual per-realm
    /// singleton object `fill_namespace` built, so repeat lookups of the same `id` -- e.g. from
    /// two separate dynamic `import()` calls -- observe the identical object
    /// (`await-dynamic-import-resolution.js`'s `import(x) === import(x)` shape). `None` means
    /// `id` was never linked into this linker.
    pub fn namespace_object(&self, id: &str) -> Option<JsValue> {
        self.instances.get(id).map(|instance| instance.namespace.clone())
    }

    /// Instantiate the root's reachable records (module environments, hoisted
    /// bindings, import/re-export wiring, the namespace object), leaving any
    /// already-instantiated module completely untouched. Splitting this out of `evaluate` is
    /// what makes calling `evaluate` again after further `link`s -- the shape a dynamic
    /// `import()` loader hook needs -- safe: re-running the original single-shot version for
    /// every record on every call would re-execute already-evaluated top-level code and, for a
    /// widely shared or cyclic graph reached through repeated dynamic imports, never terminate.
    fn instantiate_pending(&mut self, id: &str, root_interpreter: &mut Interpreter) -> Result<(), ModuleFailure> {
        self.prepare_source_objects(root_interpreter);
        // A dynamic import may run with its importer's module environment
        // installed, but new modules belong to the original realm global.
        let realm_global = self.realm_global.get_or_insert_with(|| root_interpreter.global_env.clone()).clone();
        if let Some(failure) = self.root_failures.get(id) { return Err(failure.clone()); }
        self.first_graph_failure(id, true, &mut HashSet::new())?;
        let mut order = Vec::new();
        self.graph_order(id, &mut HashSet::new(), &mut order);
        let new_ids: Vec<String> = order.into_iter().filter(|id| !self.instances.contains_key(id)).collect();
        if new_ids.is_empty() {
            return Ok(());
        }
        // All environments and function declarations for the newly linked subgraph exist before
        // any of it evaluates. A dependency cycle can therefore read hoisted functions and
        // observes TDZ for lexical declarations.
        for record_id in &new_ids {
            let (mut program, default_binding) = {
                let record = &self.records[record_id];
                let (program, default) = module_program(record);
                (program, if self.is_synthetic_record(record_id) { Some("\0module.default".into()) } else { default })
            };
            let engine = &mut *root_interpreter;
            let module_env = EnvHandle::new(EnvKind::Module, Some(realm_global.clone()));
            module_env.bind_this(JsValue::Undefined);
            let mut meta = JsObject::ordinary();
            meta.proto = Some(engine.object_proto.clone());
            if let Some(url) = self.resolver.source_url(record_id) {
                let _ = meta.define("url", Property::data(JsValue::str(url)));
            }
            module_env
                .declare_lexical("\0import.meta", JsValue::object(meta), false)
                .map_err(ModuleFailure::runtime)?;
            // The referrer a dynamic `import()` made from this module's own top-level or nested
            // function code reports to `Interpreter::binding_set_module_loader_hook`'s hook (see
            // `interp_module_host.rs`), so the driver's own registry can resolve a relative
            // specifier against the SAME module identity this linker used for `record_id`.
            module_env
                .declare_lexical("\0module.id", JsValue::str(record_id.as_str()), false)
                .map_err(ModuleFailure::runtime)?;
            predeclare_lexical(&module_env, &program).map_err(ModuleFailure::runtime)?;
            if let Some(data) = self.data_modules.get(record_id) {
                let value = data.materialize(engine).map_err(ModuleFailure::runtime)?;
                module_env.declare_lexical("\0module.default",value,false).map_err(ModuleFailure::runtime)?;
            } else if let Some((value,_)) = self.synthetic.get(record_id) {
                module_env.declare_lexical("\0module.default",value.clone(),false).map_err(ModuleFailure::runtime)?;
            }
            let previous = std::mem::replace(&mut engine.global_env, module_env.clone());
            let prepared = engine.prepare_module(&program);
            engine.global_env = previous;
            prepared.map_err(ModuleFailure::runtime)?;
            // ModuleDeclarationInstantiation created these objects exactly once.
            // Their evaluation is empty; omit them from the execution adapter so
            // legacy run_program entry cannot replace a function already observed
            // by an importing module in a cycle. Nested declarations remain in
            // their original statements and instantiate in their own block scope.
            program.body.retain(|statement| !matches!(statement.kind, StmtKind::Function(_)));
            let mut object = JsObject::ordinary();
            object.proto = None;
            object.extensible = false;
            self.instances.insert(
                record_id.clone(),
                ModuleInstance {
                    env: module_env,
                    program,
                    default_binding,
                    namespace: JsValue::object(object),
                    exports: BTreeMap::new(),
                    state: ModuleState::Linked,
                    error: None,
                    deferred_namespace: JsValue::object(JsObject::ordinary()),
                    evaluation_promise: None, dfs_index: 0, dfs_ancestor: 0,
                    on_stack: false, cycle_root: None, async_order: None,
                    pending_async: 0, async_parents: Vec::new(), body_started: false,
                },
            );
        }
        // Exported namespace aliases can refer to namespaces in a cycle within the new subgraph.
        for record_id in &new_ids {
            let aliases: Vec<(String, String)> = {
                let record = &self.records[record_id];
                record
                    .module
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        ModuleItem::ExportAll {
                            as_name: Some(name),
                            from,
                            attributes, ..
                        } => Some((name.clone(), record.dependencies[&export_key(from, attributes)].clone())),
                        _ => None,
                    })
                    .collect()
            };
            for (name, dependency) in aliases {
                let namespace = self.instances[&dependency].namespace.clone();
                self.instances[record_id]
                    .env
                    .declare_lexical(&namespace_binding(&name), namespace, false)
                    .map_err(ModuleFailure::runtime)?;
            }
        }
        // Resolve import aliases after every local cell in the new subgraph exists. This follows
        // export chains without evaluating a module and detects ambiguous star exports.
        for record_id in &new_ids {
            let imports: Vec<(String, ImportPhase, Vec<(String, String)>)> = {
                let record = &self.records[record_id];
                record
                    .module
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        ModuleItem::Import(import) => {
                            let dependency = record.dependencies[&import_key(import)].clone();
                            let bindings = import
                                .bindings
                                .iter()
                                .map(|binding| {
                                    (binding.imported.clone(), binding.local.name.clone())
                                })
                                .collect();
                            Some((dependency, ModuleRequest::ast(&import.from,import.phase.as_deref(),&import.attributes).phase, bindings))
                        }
                        _ => None,
                    })
                    .collect()
            };
            for (dependency, phase, bindings) in imports {
                for (imported, local) in bindings {
                    let cell = if phase == ImportPhase::Source {
                        BindingCell::new_value(self.module_source(&dependency)?)
                    } else if imported == "*" {
                        BindingCell::new_value(if phase == ImportPhase::Defer { self.instances[&dependency].deferred_namespace.clone() }
                            else { self.instances[&dependency].namespace.clone() })
                    } else {
                        self.export_cell(&dependency, &imported, &mut HashSet::new())?
                            .ok_or_else(|| {
                                ModuleFailure::syntax(
                                    ModulePhase::Resolution,
                                    format!("missing export {imported}"),
                                )
                            })?
                    };
                    self.instances[record_id]
                        .env
                        .declare_import(&local, cell)
                        .map_err(ModuleFailure::runtime)?;
                }
            }
        }
        for record_id in &new_ids {
            let names = self.export_names(record_id, &mut HashSet::new());
            let mut exports = BTreeMap::new();
            for name in names {
                if let Some(cell) = self.export_cell(record_id, &name, &mut HashSet::new())? {
                    exports.insert(name, cell);
                }
            }
            let instance = self.instances.get_mut(record_id).unwrap();
            fill_namespace(&instance.namespace, &exports);
            instance.exports = exports;
        }
        Ok(())
    }

    fn evaluate_instance(&mut self, id: &str, root: &mut Interpreter) -> Result<(), ModuleFailure> {
        if let Some(error) = &self.instances[id].error { return Err(error.clone()); }
        match self.instances[id].state {
            ModuleState::Evaluated | ModuleState::Evaluating | ModuleState::EvaluatingAsync => return Ok(()),
            ModuleState::Linked => {}
        }
        self.instances.get_mut(id).unwrap().state = ModuleState::Evaluating;
        let dependencies = self.ordered_dependencies(id);
        let outcome = (|| {
            for dependency in dependencies { self.evaluate_instance(&dependency, root)?; }
            execute_module_body(root, &self.instances[id], self.records[id].module.has_top_level_await)
        })();
        match &outcome {
            Ok(()) => self.instances.get_mut(id).unwrap().state = ModuleState::Evaluated,
            Err(error) => self.instances.get_mut(id).unwrap().error = Some(error.clone()),
        }
        outcome
    }

    fn export_names(&self, id: &str, visiting: &mut HashSet<String>) -> HashSet<String> {
        if !visiting.insert(id.to_owned()) {
            return HashSet::new();
        }
        let record = &self.records[id];
        let mut names = HashSet::new();
        if self.is_synthetic_record(id) { names.insert("default".into()); }
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
                    attributes, ..
                } => {
                    names.extend(
                        self.export_names(&record.dependencies[&export_key(from, attributes)], visiting)
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
        visiting: &mut HashSet<(String, String)>,
    ) -> Result<Option<BindingCell>, ModuleFailure> {
        if self.is_synthetic_record(id) && name == "default" {
            return Ok(self.instances[id].env.local_cell("\0module.default"));
        }
        match self.resolve_export(id, name, &mut HashSet::new()) {
            ExportResolution::Missing | ExportResolution::Ambiguous => return Ok(None),
            // Source records deliberately have no evaluated ModuleInstance.
            // Follow their terminal source value directly, including through
            // cyclic/star forwarding, rather than a forwarding local cell
            // that may not have been initialized yet in this graph walk.
            ExportResolution::Source(target) => {
                return Ok(Some(BindingCell::new_value(self.module_source(&target)?)));
            }
            _ => {}
        }
        if !visiting.insert((id.to_owned(), name.to_owned())) {
            return Ok(None);
        }
        let record = &self.records[id];
        let mut direct = None;
        let mut stars = Vec::new();
        for item in &record.module.items {
            match item {
                ModuleItem::ExportNamed { names, from, attributes, .. } => {
                    for entry in names.iter().filter(|entry| entry.exported == name) {
                        direct = if let Some(from) = from {
                            self.export_cell(&record.dependencies[&export_key(from, attributes)], &entry.local, visiting)?
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
                            if import.phase.as_deref()==Some("source") || import.phase.as_deref()==Some("defer") {
                                self.instances[id].env.local_cell(&binding.local.name)
                            } else if binding.imported == "*" {
                                Some(BindingCell::new_value(
                                    self.instances[&record.dependencies[&import_key(import)]]
                                        .namespace
                                        .clone(),
                                ))
                            } else {
                                self.export_cell(
                                    &record.dependencies[&import_key(import)],
                                    &binding.imported,
                                    visiting,
                                )?
                            }
                        } else {
                            self.instances[id].env.local_cell(&entry.local)
                        };
                    }
                }
                ModuleItem::ExportDefaultExpression(_)
                | ModuleItem::ExportDefaultDeclaration(_)
                    if name == "default" =>
                {
                    direct = self.instances[id]
                        .default_binding
                        .as_ref()
                        .and_then(|binding| self.instances[id].env.local_cell(binding));
                }
                ModuleItem::ExportAll {
                    as_name: Some(alias),
                    ..
                } if alias == name => {
                    direct = self.instances[id].env.local_cell(&namespace_binding(alias));
                }
                ModuleItem::ExportAll {
                    as_name: None,
                    from,
                    attributes, ..
                } if name != "default" => {
                    if let Some(cell) = self.export_cell(&record.dependencies[&export_key(from, attributes)], name, visiting)?
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
        // ResolveExport already compared terminal bindings, including namespace
        // aliases whose wrapper cells need not have identical allocations.
        Ok(stars.first().cloned())
    }
}

include!("module_graph.rs");
include!("module_async.rs");

/// ExecuteAsyncModule (16.2.1.5.2), simplified for a host with no genuine
/// cross-module concurrency: the module's top-level statements run on a
/// coroutine exactly like an ordinary async function body (via
/// `start_thread_async_function`, the same machinery `interp_coroutine.rs`
/// uses for `async function` bodies the bytecode VM didn't compile), so a
/// top-level `await` genuinely suspends until its operand settles rather
/// than degrading to `undefined` immediately.
///
/// A real engine lets an async module's own dependents keep making progress
/// while it's pending (its `[[AsyncEvaluatingOrder]]`); this host instead
/// blocks by draining the job queue once the coroutine parks, since
/// `ModuleLinker::evaluate` is an ordinary synchronous Rust call with
/// nowhere to return control to in the meantime. This is transparent for the
/// overwhelmingly common case (a linear or tree-shaped module graph: each
/// module either has no top-level await or awaits something that itself
/// resolves once the job queue is pumped -- a resolved/rejected Promise, a
/// `setTimeout`-free microtask chain, another module's already-evaluated
/// export). It is observably different only for a module explicitly written
/// to prove non-blocking concurrency between unrelated sibling modules
/// (e.g. `top-level-await/async-module-does-not-block-sibling-modules.js`),
/// and for genuine import cycles between async modules, which need the full
/// `[[AsyncEvaluatingOrder]]` bookkeeping this does not implement.
fn evaluate_async_module_body(root: &mut Interpreter, instance: &ModuleInstance) -> Result<(), ModuleFailure> {
    let body = FunctionBody::Block(Block {
        body: instance.program.body.clone(),
        span: instance.program.span,
    });
    // All module code is strict (10.2.1 IsStrict for a Module ScriptOrModule).
    let Some(promise) = root.start_thread_async_function(body, instance.env.clone(), true) else {
        return Err(ModuleFailure::new(
            ModulePhase::Async,
            "top-level await requires OS thread support, unavailable in this host",
        ));
    };
    if promise_fate(&promise).is_none() {
        // The coroutine already ran up to its first `await` (or finished) by the
        // time `start_thread_async_function` returned; draining the job queue
        // once runs every reaction that first `await` unblocked, including the
        // module's own resumption(s), to a fixpoint. If the promise is still
        // pending after that, nothing left in the queue can ever settle it
        // synchronously (no real host-level concurrency here), so this reports
        // that as a failure rather than looping forever.
        if root.drain_microtasks().is_err() {
            return Err(ModuleFailure::new(
                ModulePhase::Async,
                "top-level await needs the job queue, unavailable under external scheduling",
            ));
        }
    }
    match promise_fate(&promise) {
        Some(PromiseFate::Fulfilled(_)) => Ok(()),
        Some(PromiseFate::Rejected(error)) => Err(ModuleFailure::runtime(Throw(error))),
        _ => Err(ModuleFailure::new(
            ModulePhase::Async,
            "top-level await did not settle synchronously (no real host-level module concurrency)",
        )),
    }
}

fn promise_fate(value: &JsValue) -> Option<PromiseFate> {
    match value {
        JsValue::Object(o) => match &o.borrow().exotic {
            Exotic::Promise(PromiseState { state, .. }) if !matches!(state, PromiseFate::Pending) => {
                Some(state.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleState {
    Linked,
    Evaluating,
    EvaluatingAsync,
    Evaluated,
}

#[derive(Clone)]
struct ModuleInstance {
    env: EnvHandle,
    program: Program,
    default_binding: Option<String>,
    namespace: JsValue,
    exports: BTreeMap<String, BindingCell>,
    state: ModuleState,
    error: Option<ModuleFailure>,
    deferred_namespace: JsValue,
    evaluation_promise: Option<JsValue>,
    dfs_index: usize,
    dfs_ancestor: usize,
    on_stack: bool,
    cycle_root: Option<String>,
    async_order: Option<u64>,
    pending_async: usize,
    async_parents: Vec<String>,
    body_started: bool,
}

fn execute_module_body(root: &mut Interpreter, instance: &ModuleInstance, has_await: bool) -> Result<(), ModuleFailure> {
    let previous = std::mem::replace(&mut root.global_env, instance.env.clone());
    let outcome = if has_await {
        root.with_execution_context(None, |root| evaluate_async_module_body(root, instance))
    } else {
        root.run_program(&instance.program).map(|_| ()).map_err(ModuleFailure::runtime)
    };
    root.global_env = previous;
    outcome
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
                } else if let StmtKind::Function(function) = &stmt.kind {
                    // Anonymous default functions are HoistableDeclarations,
                    // including async/generator forms, rather than expressions.
                    // `default` cannot be a source BindingIdentifier, so this
                    // reserved adapter binding is collision-free and gives the
                    // existing function factory the observable name "default".
                    let mut function = (**function).clone();
                    function.name = Some(Ident::new("default", stmt.span));
                    default_binding = Some("default".into());
                    body.push(Stmt::new(StmtKind::Function(Box::new(function)), stmt.span));
                } else {
                    let StmtKind::Class(class) = &stmt.kind else { unreachable!() };
                    let expr = Expr::new(ExprKind::Class(class.clone()), stmt.span);
                    default_binding = Some("default".into());
                    body.push(default_declaration(expr, stmt.span));
                }
            }
            ModuleItem::ExportDefaultExpression(expr) => {
                default_binding = Some("default".into());
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
                    accessor: false,
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
                accessor: false,
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
                binding: Binding::Ident(Ident::new("default", span)),
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
    fn syntax_failures_keep_type_identity_and_phase_without_poisoning_later_graphs() {
        let resolver = |specifier: &str, _: &str| match specifier {
            "cycle" => Ok(("cycle".into(), "import 'bad-root';".into())),
            "bad-root" => Ok(("bad-root".into(), "import 'cycle';import 'broken';".into())),
            "broken" => Ok(("broken".into(), "export const =;".into())),
            "good" => Ok(("good".into(), "export const value=42;".into())),
            _ => Err("fetch failed".into()),
        };
        let mut linker = ModuleLinker::new(resolver);
        let nested = linker.link("bad-root", "import 'cycle';import 'broken';").unwrap_err();
        assert_eq!(nested.phase, ModulePhase::Resolution);
        assert!(nested.thrown.as_ref().unwrap().describe().starts_with("SyntaxError"));
        assert!(linker.instances.is_empty());
        let direct = linker.link("broken", "export const =;").unwrap_err();
        assert_eq!(direct.phase, ModulePhase::Parse);
        assert_eq!(nested.thrown.as_ref().unwrap().0, direct.thrown.as_ref().unwrap().0);
        let repeated = linker.link("bad-root", "import 'cycle';import 'broken';").unwrap_err();
        assert_eq!(nested.thrown.as_ref().unwrap().0, repeated.thrown.as_ref().unwrap().0);
        linker.link("good-root", "import {value} from 'good';export {value};").unwrap();
        assert_eq!(linker.evaluate("good-root", &mut Interpreter::new()).unwrap()["value"], JsValue::num(42.0));
    }

    #[test]
    fn linking_syntax_errors_are_typed_but_host_fetch_failures_remain_host_owned() {
        let mut missing = ModuleLinker::new(|_: &str, _: &str| Err("fetch failed".into()));
        let fetched = missing.link("root", "import 'missing';").unwrap_err();
        assert_eq!(fetched.phase, ModulePhase::Resolution);
        assert!(fetched.thrown.is_none());
        let mut missing_export = ModuleLinker::new(|_: &str, _: &str| Ok(("dep".into(), "export const present=1;".into())));
        let failure = missing_export.link("root", "import {absent} from 'dep';").unwrap_err();
        assert_eq!(failure.phase, ModulePhase::Resolution);
        assert!(failure.thrown.as_ref().unwrap().describe().starts_with("SyntaxError"));
        let repeated = missing_export.link("root", "import {absent} from 'dep';").unwrap_err();
        assert_eq!(failure.thrown.unwrap().0, repeated.thrown.unwrap().0);
    }

    #[test]
    fn source_url_metadata_is_opt_in_and_distinct_from_record_identity() {
        struct BrowserResolver;
        impl ModuleResolver for BrowserResolver {
            fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
                Ok(("external-record".into(), "export const url=import.meta.url;".into()))
            }
            fn source_url(&self, id: &str) -> Option<String> {
                Some(match id {
                    "external-record" => "https://example.test/dep.js",
                    _ => "https://example.test/document.html",
                }.into())
            }
        }
        let mut linker = ModuleLinker::new(BrowserResolver);
        let mut root = Interpreter::new();
        let source = "import {url} from 'dep'; export const own=import.meta.url; export const dependency=url; export const meta=import.meta;";
        linker.link("inline-record-1", source).unwrap();
        linker.link("inline-record-2", "export const meta=import.meta;").unwrap();
        let first = linker.evaluate("inline-record-1", &mut root).unwrap();
        let second = linker.evaluate("inline-record-2", &mut root).unwrap();
        assert_eq!(first["own"], JsValue::str("https://example.test/document.html"));
        assert_eq!(first["dependency"], JsValue::str("https://example.test/dep.js"));
        assert_ne!(first["meta"], second["meta"]);
        let mut plain = ModuleLinker::new(|_: &str, _: &str| Err("unresolved".into()));
        plain.link("plain", "export const absent=!('url' in import.meta);").unwrap();
        assert_eq!(plain.evaluate("plain", &mut root).unwrap()["absent"], JsValue::Bool(true));
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

    #[test]
    fn shared_registry_allows_nested_imports_without_borrows_or_scope_leaks() {
        let sources = Rc::new(HashMap::from([
            ("dep", "const privateValue=99; globalThis.loads++; import('leaf').then(ns=>{globalThis.leaf=ns.value;globalThis.leaked=ns.leaked;}); export let value=1; export function bump(){value++;}"),
            ("leaf", "export const value=41; export const leaked=typeof privateValue;"),
        ]));
        let resolver_sources = sources.clone();
        let linker = Rc::new(RefCell::new(ModuleLinker::new(move |specifier: &str, _: &str| {
            resolver_sources.get(specifier).map(|source| (specifier.to_owned(), (*source).to_owned()))
                .ok_or_else(|| "missing module".to_owned())
        })));
        let mut root = Interpreter::new();
        root.run_script("globalThis.loads=0;").unwrap();
        let loader_linker = linker.clone();
        root.binding_set_typed_module_loader_hook(Rc::new(move |i, specifier, _| {
            let source = sources.get(specifier).ok_or_else(|| Throw::type_error("missing module"))?;
            loader_linker.borrow_mut().link(specifier, source).map_err(|e| Throw::type_error(e.message))?;
            ModuleLinker::evaluate_shared(&loader_linker, specifier, i)
                .map_err(|e| e.thrown.unwrap_or_else(|| Throw::type_error(e.message)))?;
            Ok(loader_linker.borrow().namespace_object(specifier).unwrap())
        }));
        linker.borrow_mut().link("root", "import * as stat from 'dep'; import('dep').then(ns=>{globalThis.same=ns===stat;ns.bump();globalThis.finalValue=stat.value;}); export const done=true;").unwrap();
        assert_eq!(ModuleLinker::evaluate_shared(&linker, "root", &mut root).unwrap()["done"], JsValue::Bool(true));
        assert_eq!(root.run_script("same && finalValue===2 && loads===1 && leaf===41 && leaked==='undefined'").unwrap(), JsValue::Bool(true));
        ModuleLinker::evaluate_shared(&linker, "root", &mut root).unwrap();
        assert_eq!(root.run_script("loads").unwrap(), JsValue::num(1.0));
    }

    #[test]
    fn shared_registry_rethrows_cached_module_error_without_evaluating_again() {
        let linker = Rc::new(RefCell::new(ModuleLinker::new(|_: &str, _: &str| Err("unresolved".into()))));
        let mut root = Interpreter::new();
        root.run_script("globalThis.count=0;globalThis.marker={};").unwrap();
        linker.borrow_mut().link("bad", "globalThis.count++;throw globalThis.marker;").unwrap();
        let first = ModuleLinker::evaluate_shared(&linker, "bad", &mut root).unwrap_err().thrown.unwrap();
        let second = ModuleLinker::evaluate_shared(&linker, "bad", &mut root).unwrap_err().thrown.unwrap();
        assert_eq!(first.0, second.0);
        assert_eq!(root.run_script("count").unwrap(), JsValue::num(1.0));
        assert!(linker.borrow_mut().evaluate("bad", &mut root).is_err());
        assert_eq!(root.run_script("count").unwrap(), JsValue::num(1.0));
    }
}

#[cfg(test)]
#[path = "module_namespace_tests.rs"]
mod namespace_operation_tests;

#[cfg(test)]
#[path = "module_declarations_tests.rs"]
mod declaration_instantiation_tests;

#[cfg(test)]
#[path = "module_utf16_tests.rs"]
mod utf16_request_tests;

#[cfg(test)]
#[path = "module_utf16_source_tests.rs"]
mod utf16_source_tests;

#[cfg(test)]
#[path = "module_data_tests.rs"]
mod data_tests;

#[cfg(test)]
#[path = "module_source_record_tests.rs"]
mod source_record_tests;
