// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// DFS/SCC asynchronous evaluation and deferred readiness adapted to shared registries.

impl<R: ModuleResolver + 'static> ModuleLinker<R> {
    /// Begin evaluation without pumping jobs. Siblings can start while another
    /// body is suspended; the returned capability settles when the graph completes.
    pub fn evaluate_async_shared(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
    ) -> Result<JsValue, ModuleFailure> {
        linker.borrow_mut().instantiate_pending(id, i)?;
        Self::install_deferred_namespaces(linker, i);
        let evaluation_root = {
            let registry = linker.borrow();
            let instance = &registry.instances[id];
            if matches!(
                instance.state,
                ModuleState::EvaluatingAsync | ModuleState::Evaluated
            ) {
                instance
                    .cycle_root
                    .clone()
                    .unwrap_or_else(|| id.to_string())
            } else {
                id.to_string()
            }
        };
        let promise = Self::ensure_capability(linker, &evaluation_root, i)?;
        Self::inner_evaluate(linker, &evaluation_root, i, &mut Vec::new(), &mut 0)?;
        Ok(promise)
    }
    /// Host dynamic-import entry point. The result is a Promise; evaluation
    /// requests resolve its live namespace, defer requests resolve a separate
    /// namespace without evaluating its synchronous body, source requests expose
    /// only a genuine host-provided source object.
    pub fn import_shared(
        linker: &Rc<RefCell<Self>>,
        request: &ModuleRequest,
        referrer: &str,
        i: &mut Interpreter,
    ) -> Result<JsValue, ModuleFailure> {
        let id = linker.borrow_mut().load_request(request, referrer)?;
        if request.phase == ImportPhase::Source {
            return linker.borrow().module_source(&id);
        }
        linker.borrow_mut().instantiate_pending(&id, i)?;
        Self::install_deferred_namespaces(linker, i);
        let promise = i.module_promise();
        let completion = if request.phase == ImportPhase::Defer {
            let targets = linker.borrow().async_subgraph(&id, &mut HashSet::new());
            let gate = i.module_promise();
            Self::await_modules(linker, targets, gate.clone(), i)?;
            gate
        } else {
            Self::evaluate_async_shared(linker, &id, i)?
        };
        let registry = linker.clone();
        let target = promise.clone();
        let phase = request.phase;
        let fulfilled = Rc::new(move |i: &mut Interpreter, _: JsValue| {
            let value = {
                let registry = registry.borrow();
                let instance = &registry.instances[&id];
                if phase == ImportPhase::Defer {
                    instance.deferred_namespace.clone()
                } else {
                    instance.namespace.clone()
                }
            };
            i.module_resolve(&target, value)
        });
        let rejected_target = promise.clone();
        i.module_observe(
            &completion,
            fulfilled,
            Rc::new(move |i, value| i.module_settle(&rejected_target, Some(Throw(value)))),
        )
        .map_err(ModuleFailure::runtime)?;
        Ok(promise)
    }
    fn ensure_capability(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
    ) -> Result<JsValue, ModuleFailure> {
        let (promise, terminal, error, new) = {
            let mut registry = linker.borrow_mut();
            let instance = registry.instances.get_mut(id).unwrap();
            let new = instance.evaluation_promise.is_none();
            let promise = instance
                .evaluation_promise
                .get_or_insert_with(|| i.module_promise())
                .clone();
            (
                promise,
                instance.state == ModuleState::Evaluated,
                instance.error.clone(),
                new,
            )
        };
        if terminal && new {
            i.module_settle(
                &promise,
                error.map(|error| {
                    error
                        .thrown
                        .unwrap_or_else(|| Throw::type_error(error.message))
                }),
            )
            .map_err(ModuleFailure::runtime)?;
        }
        Ok(promise)
    }
    fn install_deferred_namespaces(linker: &Rc<RefCell<Self>>, i: &Interpreter) {
        let ids: Vec<_> = linker.borrow().instances.keys().cloned().collect();
        let realm = i.binding_current_realm();
        for id in ids {
            let (object, exports) = {
                let registry = linker.borrow();
                let instance = &registry.instances[&id];
                (
                    instance.deferred_namespace.clone(),
                    instance.exports.clone(),
                )
            };
            let JsValue::Object(object) = object else {
                unreachable!()
            };
            if matches!(object.borrow().exotic, Exotic::DeferredModuleNamespace(_)) {
                continue;
            }
            let registry = linker.clone();
            let evaluate = Rc::new(move |i: &mut Interpreter| {
                i.binding_with_realm(realm, |i| {
                    if !registry.borrow().ready_for_sync(&id, &mut HashSet::new()) {
                        return Err(Throw::type_error(
                            "deferred module graph is not ready for synchronous execution",
                        ));
                    }
                    Self::evaluate_sync(&registry, &id, i)
                        .map_err(|f| f.thrown.unwrap_or_else(|| Throw::type_error(f.message)))
                })
            });
            let mut object = object.borrow_mut();
            object.proto = None;
            object.extensible = false;
            for name in exports.keys().filter(|name| name.as_str() != "then") {
                let _ = object.define(
                    name.as_str(),
                    Property {
                        value: JsValue::Undefined,
                        writable: true,
                        enumerable: true,
                        configurable: false,
                        getter: None,
                        setter: None,
                    },
                );
            }
            let _ = object.define(
                WellKnownSymbol::ToStringTag.symbol(),
                Property {
                    value: JsValue::str("Deferred Module"),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    getter: None,
                    setter: None,
                },
            );
            object.exotic =
                Exotic::DeferredModuleNamespace(Rc::new(crate::value::DeferredNamespaceState {
                    exports,
                    evaluate,
                }));
        }
    }
    fn ready_for_sync(&self, id: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(id.into()) {
            return true;
        }
        let instance = &self.instances[id];
        let root = instance.cycle_root.as_deref().unwrap_or(id);
        if self.instances[root].state == ModuleState::Evaluated {
            return true;
        }
        if matches!(
            instance.state,
            ModuleState::Evaluating | ModuleState::EvaluatingAsync
        ) {
            return false;
        }
        if self.records[id].module.has_top_level_await {
            return false;
        }
        self.ordered_dependencies(id)
            .iter()
            .all(|dep| self.ready_for_sync(dep, seen))
    }
    fn evaluate_sync(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
    ) -> Result<(), ModuleFailure> {
        // ReadyForSyncExecution guarantees this same SCC evaluator returns an
        // already-settled capability; do not invent a separate cycle/error walk.
        let completion = Self::evaluate_async_shared(linker, id, i)?;
        match promise_fate(&completion) {
            Some(PromiseFate::Fulfilled(_)) => Ok(()),
            Some(PromiseFate::Rejected(value)) => Err(ModuleFailure::runtime(Throw(value))),
            _ => Err(ModuleFailure::new(
                ModulePhase::Async,
                "deferred synchronous evaluation unexpectedly remained pending",
            )),
        }
    }
    fn evaluation_dependencies(&self, id: &str) -> Vec<(String, ImportPhase)> {
        // A mixed eager/deferred target is visited at its first eager request.
        let edges: Vec<_> = self
            .requests
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|key| {
                let request = self.request_metadata.get(&(id.into(), key.clone()))?;
                (request.phase != ImportPhase::Source)
                    .then(|| (self.records[id].dependencies[key].clone(), request.phase))
            })
            .collect();
        let eager: HashSet<_> = edges
            .iter()
            .filter(|(_, phase)| *phase == ImportPhase::Evaluation)
            .map(|(id, _)| id.clone())
            .collect();
        let mut seen = HashSet::new();
        edges
            .into_iter()
            .filter(|(id, phase)| {
                (!eager.contains(id) || *phase == ImportPhase::Evaluation)
                    && seen.insert(id.clone())
            })
            .collect()
    }
    fn async_subgraph(&self, id: &str, seen: &mut HashSet<String>) -> Vec<String> {
        if !seen.insert(id.into()) {
            return Vec::new();
        }
        let mut result = Vec::new();
        for dep in self.ordered_dependencies(id) {
            result.extend(self.async_subgraph(&dep, seen));
        }
        if self.records[id].module.has_top_level_await {
            result.push(id.into());
        }
        result
    }
    fn inner_evaluate(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
        stack: &mut Vec<String>,
        index: &mut usize,
    ) -> Result<(), ModuleFailure> {
        Self::ensure_capability(linker, id, i)?;
        let deps = {
            let mut registry = linker.borrow_mut();
            let instance = registry.instances.get_mut(id).unwrap();
            if instance.state != ModuleState::Linked {
                return Ok(());
            }
            instance.state = ModuleState::Evaluating;
            instance.dfs_index = *index;
            instance.dfs_ancestor = *index;
            instance.on_stack = true;
            *index += 1;
            stack.push(id.into());
            registry.evaluation_dependencies(id)
        };
        for (dep, phase) in deps {
            let targets = if phase == ImportPhase::Defer {
                linker.borrow().async_subgraph(&dep, &mut HashSet::new())
            } else {
                vec![dep]
            };
            for dep in targets {
                Self::inner_evaluate(linker, &dep, i, stack, index)?;
                let (on_stack, ancestor, cycle, error, done, asynchronous) = {
                    let registry = linker.borrow();
                    let instance = &registry.instances[&dep];
                    let cycle = instance.cycle_root.clone().unwrap_or(dep.clone());
                    let required = if instance.on_stack {
                        instance
                    } else {
                        &registry.instances[&cycle]
                    };
                    (
                        instance.on_stack,
                        instance.dfs_ancestor,
                        cycle,
                        required.error.clone(),
                        required.state == ModuleState::Evaluated,
                        required.async_order.is_some(),
                    )
                };
                if let Some(error) = error {
                    Self::rejected(linker, id, error, i)?;
                    Self::finish_scc(linker, id, stack);
                    return Ok(());
                }
                let mut registry = linker.borrow_mut();
                if on_stack {
                    let instance = registry.instances.get_mut(id).unwrap();
                    instance.dfs_ancestor = instance.dfs_ancestor.min(ancestor);
                }
                if asynchronous && !done {
                    let waiting = if on_stack { dep } else { cycle };
                    if !registry.instances[&waiting]
                        .async_parents
                        .iter()
                        .any(|parent| parent == id)
                    {
                        registry.instances.get_mut(id).unwrap().pending_async += 1;
                        registry
                            .instances
                            .get_mut(&waiting)
                            .unwrap()
                            .async_parents
                            .push(id.into());
                    }
                }
            }
        }
        let execute = {
            let mut registry = linker.borrow_mut();
            let asynchronous = registry.instances[id].pending_async > 0
                || registry.records[id].module.has_top_level_await;
            if asynchronous {
                registry.async_sequence += 1;
                let order = registry.async_sequence;
                registry.instances.get_mut(id).unwrap().async_order = Some(order);
            }
            registry.instances[id].pending_async == 0
        };
        if execute {
            Self::execute_async(linker, id, i)?;
        }
        Self::finish_scc(linker, id, stack);
        Ok(())
    }
    fn finish_scc(linker: &Rc<RefCell<Self>>, id: &str, stack: &mut Vec<String>) {
        let mut registry = linker.borrow_mut();
        let instance = &registry.instances[id];
        if !instance.on_stack || instance.dfs_index != instance.dfs_ancestor {
            return;
        }
        while let Some(member) = stack.pop() {
            let instance = registry.instances.get_mut(&member).unwrap();
            instance.on_stack = false;
            instance.cycle_root = Some(id.into());
            if instance.state == ModuleState::Evaluating && instance.async_order.is_some() {
                instance.state = ModuleState::EvaluatingAsync;
            }
            if member == id {
                break;
            }
        }
    }
    fn execute_async(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
    ) -> Result<(), ModuleFailure> {
        let (instance, has_await) = {
            let mut registry = linker.borrow_mut();
            let has_await = registry.records[id].module.has_top_level_await;
            let instance = registry.instances.get_mut(id).unwrap();
            if instance.body_started || instance.state == ModuleState::Evaluated {
                return Ok(());
            }
            instance.body_started = true;
            (instance.clone(), has_await)
        };
        match i.module_start_body(&instance.program, instance.env, has_await) {
            Ok(Some(promise)) => {
                let registry = linker.clone();
                let key = id.to_string();
                let fulfilled = Rc::new(move |i: &mut Interpreter, _: JsValue| {
                    Self::fulfilled(&registry, &key, i).map_err(|failure| {
                        failure
                            .thrown
                            .unwrap_or_else(|| Throw::type_error(failure.message))
                    })
                });
                let registry = linker.clone();
                let key = id.to_string();
                i.module_observe(
                    &promise,
                    fulfilled,
                    Rc::new(move |i, value| {
                        Self::rejected(&registry, &key, ModuleFailure::runtime(Throw(value)), i)
                            .map_err(|failure| {
                                failure
                                    .thrown
                                    .unwrap_or_else(|| Throw::type_error(failure.message))
                            })
                    }),
                )
                .map_err(ModuleFailure::runtime)?;
                Ok(())
            }
            Ok(None) => Self::fulfilled(linker, id, i),
            Err(error) => Self::rejected(linker, id, ModuleFailure::runtime(error), i),
        }
    }
    fn fulfilled(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        i: &mut Interpreter,
    ) -> Result<(), ModuleFailure> {
        let (promise, parents) = {
            let mut registry = linker.borrow_mut();
            let instance = registry.instances.get_mut(id).unwrap();
            if instance.state == ModuleState::Evaluated {
                return Ok(());
            }
            instance.state = ModuleState::Evaluated;
            (
                instance.evaluation_promise.clone(),
                instance.async_parents.clone(),
            )
        };
        if let Some(promise) = promise {
            i.module_settle(&promise, None)
                .map_err(ModuleFailure::runtime)?;
        }
        let mut ready = Vec::new();
        linker
            .borrow_mut()
            .gather_ready_ancestors(parents, &mut ready);
        ready.sort_by_key(|(order, _)| *order);
        for (_, parent) in ready {
            Self::execute_async(linker, &parent, i)?;
        }
        Ok(())
    }
    fn gather_ready_ancestors(&mut self, parents: Vec<String>, ready: &mut Vec<(u64, String)>) {
        for parent in parents {
            let instance = self.instances.get_mut(&parent).unwrap();
            if instance.state == ModuleState::Evaluated || instance.pending_async == 0 {
                continue;
            }
            instance.pending_async -= 1;
            if instance.pending_async != 0 {
                continue;
            }
            ready.push((instance.async_order.unwrap_or(u64::MAX), parent.clone()));
            let ancestors = instance.async_parents.clone();
            if !self.records[&parent].module.has_top_level_await {
                self.gather_ready_ancestors(ancestors, ready);
            }
        }
    }
    fn rejected(
        linker: &Rc<RefCell<Self>>,
        id: &str,
        error: ModuleFailure,
        i: &mut Interpreter,
    ) -> Result<(), ModuleFailure> {
        let (promise, parents) = {
            let mut registry = linker.borrow_mut();
            let instance = registry.instances.get_mut(id).unwrap();
            if instance.state == ModuleState::Evaluated {
                return Ok(());
            }
            instance.state = ModuleState::Evaluated;
            instance.error = Some(error.clone());
            (
                instance.evaluation_promise.clone(),
                instance.async_parents.clone(),
            )
        };
        if let Some(promise) = promise {
            i.module_settle(
                &promise,
                Some(
                    error
                        .thrown
                        .clone()
                        .unwrap_or_else(|| Throw::type_error(error.message.clone())),
                ),
            )
            .map_err(ModuleFailure::runtime)?;
        }
        for parent in parents {
            Self::rejected(linker, &parent, error.clone(), i)?;
        }
        Ok(())
    }
    fn await_modules(
        linker: &Rc<RefCell<Self>>,
        targets: Vec<String>,
        promise: JsValue,
        i: &mut Interpreter,
    ) -> Result<(), ModuleFailure> {
        if targets.is_empty() {
            i.module_settle(&promise, None)
                .map_err(ModuleFailure::runtime)?;
            return Ok(());
        }
        let remaining = Rc::new(std::cell::Cell::new(targets.len()));
        // Start the entire batch before any continuation can run.
        let mut completions = Vec::new();
        for id in targets {
            completions.push(Self::evaluate_async_shared(linker, &id, i)?);
        }
        for completion in completions {
            let count = remaining.clone();
            let target = promise.clone();
            let fulfilled = Rc::new(move |i: &mut Interpreter, _: JsValue| {
                count.set(count.get() - 1);
                if count.get() == 0 {
                    i.module_settle(&target, None)?;
                }
                Ok(())
            });
            let target = promise.clone();
            i.module_observe(
                &completion,
                fulfilled,
                Rc::new(move |i, value| i.module_settle(&target, Some(Throw(value)))),
            )
            .map_err(ModuleFailure::runtime)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod asynchronous_graph_tests {
    use super::*;
    fn registry(
        sources: &'static [(&'static str, &'static str)],
    ) -> Rc<RefCell<ModuleLinker<impl ModuleResolver>>> {
        Rc::new(RefCell::new(ModuleLinker::new(
            move |name: &str, _: &str| {
                sources
                    .iter()
                    .find(|(id, _)| *id == name)
                    .map(|(id, source)| (id.to_string(), source.to_string()))
                    .ok_or_else(|| format!("missing {name}"))
            },
        )))
    }
    #[test]
    fn suspended_module_does_not_block_sibling_and_parent_waits() {
        let linker = registry(&[
            (
                "a",
                "order.push('a');await blocked;order.push('resume');export const a=1;",
            ),
            ("b", "order.push('b');release();export const b=2;"),
        ]);
        let mut i = Interpreter::new();
        i.run_script("var order=[];var release;var blocked=new Promise(r=>release=r);")
            .unwrap();
        linker
            .borrow_mut()
            .link(
                "root",
                "import 'a';import 'b';order.push('root');export const done=1;",
            )
            .unwrap();
        let completion = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        assert!(promise_fate(&completion).is_none());
        let JsValue::Object(order) = i.global_env.resolve("order").unwrap() else {
            unreachable!()
        };
        assert_eq!(order.borrow().get_own("length"), Some(JsValue::num(2.0)));
        i.drain_microtasks().unwrap();
        assert_eq!(
            i.run_script("order.join(',')").unwrap(),
            JsValue::str("a,b,resume,root")
        );
        assert!(matches!(
            promise_fate(&completion),
            Some(PromiseFate::Fulfilled(_))
        ));
    }
    #[test]
    fn cycle_error_is_cached_and_propagates_to_every_async_parent() {
        let linker = registry(&[
            ("a", "import 'b';await Promise.resolve();throw marker;"),
            ("b", "import 'a';export function b(){}"),
        ]);
        let mut i = Interpreter::new();
        i.run_script("var marker={};").unwrap();
        linker
            .borrow_mut()
            .link("root", "import 'a';export const untouched=1;")
            .unwrap();
        let promise = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        i.drain_microtasks().unwrap();
        let expected = i.run_script("marker").unwrap();
        assert!(
            matches!(promise_fate(&promise),Some(PromiseFate::Rejected(value)) if value==expected)
        );
        let again = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        assert_eq!(promise, again);
        assert!(ModuleLinker::evaluate_shared(&linker, "root", &mut i).is_err());
    }
    #[test]
    fn deferred_namespace_symbols_then_and_set_do_not_evaluate_but_all_string_operations_do() {
        for operation in [
            "ns.missing",
            "'missing' in ns",
            "Object.getOwnPropertyDescriptor(ns,'missing')",
            "Reflect.defineProperty(ns,'missing',{value:1})",
            "delete ns.missing",
            "Reflect.ownKeys(ns)",
        ] {
            let linker = registry(&[(
                "leaf",
                "count++;export const value=7;export function then(){throw 98}",
            )]);
            let mut i = Interpreter::new();
            i.run_script("var count=0;").unwrap();
            linker
                .borrow_mut()
                .link(
                    "root",
                    "import defer * as ns from 'leaf';globalThis.saved=ns;export const done=1;",
                )
                .unwrap();
            ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
            i.run_script("var ns=saved;ns[Symbol.toStringTag];'then' in ns;Object.getOwnPropertyDescriptor(ns,'then');Reflect.set(ns,'value',99);Reflect.defineProperty(ns,Symbol.toStringTag,{value:'Deferred Module'});ns.then;").unwrap();
            assert_eq!(
                i.run_script("count").unwrap(),
                JsValue::num(0.0),
                "{operation}"
            );
            i.run_script(operation).unwrap();
            assert_eq!(
                i.run_script("count").unwrap(),
                JsValue::num(1.0),
                "{operation}"
            );
            assert_eq!(i.run_script("ns.value").unwrap(), JsValue::num(7.0));
            let ordinary = linker.borrow().namespace_object("leaf").unwrap();
            assert_ne!(ordinary, i.run_script("ns").unwrap());
        }
    }
    #[test]
    fn deferred_readiness_rejects_before_any_body_and_eager_async_subgraph_completes_first() {
        let linker = registry(&[
            ("leaf", "import 'async';count++;export const value=7;"),
            ("async", "await pending;order.push('async');"),
        ]);
        let mut i = Interpreter::new();
        i.run_script("var count=0,order=[],release;var pending=new Promise(r=>release=r);")
            .unwrap();
        linker
            .borrow_mut()
            .link(
                "root",
                "import defer * as ns from 'leaf';order.push('root');globalThis.saved=ns;",
            )
            .unwrap();
        linker
            .borrow_mut()
            .instantiate_pending("root", &mut i)
            .unwrap();
        ModuleLinker::install_deferred_namespaces(&linker, &i);
        let namespace = linker.borrow().instances["leaf"].deferred_namespace.clone();
        i.global_env.declare_var("ns", namespace).unwrap();
        assert_eq!(
            i.run_script(
                "var notReady=false;try{ns.value}catch(e){notReady=e instanceof TypeError}notReady"
            )
            .unwrap(),
            JsValue::Bool(true)
        );
        assert_eq!(i.run_script("count").unwrap(), JsValue::num(0.0));
        let completion = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        assert!(promise_fate(&completion).is_none());
        i.run_script("release()").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(
            i.run_script("order.join(',')").unwrap(),
            JsValue::str("async,root")
        );
        assert_eq!(i.run_script("ns.value").unwrap(), JsValue::num(7.0));
        assert_eq!(i.run_script("count").unwrap(), JsValue::num(1.0));
    }
    #[test]
    fn deferred_failure_identity_and_namespace_key_order_are_stable() {
        let linker = registry(&[
            ("leaf", "throw marker;export const value=7;"),
            (
                "keys",
                "const a=1,b=2,c=3;export {a as '10',b as '2',c as '1'};",
            ),
        ]);
        let mut i = Interpreter::new();
        i.run_script("var marker={};").unwrap();
        linker
            .borrow_mut()
            .link("root", "import defer * as ns from 'leaf';globalThis.ns=ns;")
            .unwrap();
        ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
        assert_eq!(i.run_script("var first,second;try{ns.value}catch(e){first=e}try{'missing' in ns}catch(e){second=e}first===marker&&second===marker").unwrap(),JsValue::Bool(true));
        linker
            .borrow_mut()
            .link(
                "keys",
                "const a=1,b=2,c=3;export {a as '10',b as '2',c as '1'};",
            )
            .unwrap();
        ModuleLinker::evaluate_shared(&linker, "keys", &mut i).unwrap();
        i.global_env
            .declare_var("keys", linker.borrow().namespace_object("keys").unwrap())
            .unwrap();
        assert_eq!(
            i.run_script("Object.getOwnPropertyNames(keys).join(',')")
                .unwrap(),
            JsValue::str("1,10,2")
        );
    }
    #[test]
    fn source_phase_loads_only_root_and_rejects_source_text_without_executing() {
        struct Sources;
        impl ModuleResolver for Sources {
            fn resolve(&self, name: &str, _: &str) -> Result<(String, String), String> {
                match name {
                    "leaf" => Ok((name.into(), "import 'not-fetched';throw 99;".into())),
                    _ => Err("unexpected fetch".into()),
                }
            }
        }
        let mut linker = ModuleLinker::new(Sources);
        let request = ModuleRequest::new("leaf", ImportPhase::Source, Vec::new());
        assert_eq!(linker.load_request(&request, "").unwrap(), "leaf");
        assert!(linker.records["leaf"].dependencies.is_empty());
        assert!(linker.module_source("leaf").unwrap_err().thrown.is_some());
        let request = ModuleRequest::new("leaf", ImportPhase::Evaluation, Vec::new());
        assert!(
            linker
                .load_request(&request, "")
                .unwrap_err()
                .message
                .contains("unexpected fetch")
        );
    }
    #[test]
    fn typed_attribute_sets_select_distinct_synthetic_bindings_and_preserve_phase_identity() {
        struct Typed;
        impl ModuleResolver for Typed {
            fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
                Err("legacy unexpected".into())
            }
            fn resolve_request(
                &self,
                request: &ModuleRequest,
                _: &str,
            ) -> Result<LoadedModule, String> {
                let kind = request
                    .attribute("type")
                    .ok_or("missing type")?
                    .to_utf8_lossy();
                let value = JsValue::str(kind.as_str());
                Ok(LoadedModule {
                    id: format!("same#{kind}"),
                    source: ModuleSource::Synthetic {
                        default_export: value.clone(),
                        module_source: Some(value),
                    },
                })
            }
        }
        let linker = Rc::new(RefCell::new(ModuleLinker::new(Typed)));
        let mut i = Interpreter::new();
        linker.borrow_mut().link("root","import json from 'same' with {type:'json'};import text from 'same' with {type:'text'};export {json,text};").unwrap();
        let exports = ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
        assert_eq!(exports["json"], JsValue::str("json"));
        assert_eq!(exports["text"], JsValue::str("text"));
        let request = ModuleRequest::new(
            "same",
            ImportPhase::Source,
            vec![("type".into(), "text".into())],
        );
        assert_eq!(
            ModuleLinker::import_shared(&linker, &request, "", &mut i).unwrap(),
            JsValue::str("text")
        );
        assert_eq!(linker.borrow().records.len(), 3);
    }
}

#[cfg(test)]
mod module_phase_order_tests {
    use super::*;
    #[test]
    fn mixed_eager_defer_visits_at_first_eager_position_and_preserves_exported_namespace() {
        let sources = [
            ("leaf", "order.push('leaf');export const value=42;"),
            ("middle", "order.push('middle');"),
        ];
        let linker = Rc::new(RefCell::new(ModuleLinker::new(
            move |name: &str, _: &str| {
                sources
                    .iter()
                    .find(|(id, _)| *id == name)
                    .map(|(id, source)| (id.to_string(), source.to_string()))
                    .ok_or_else(|| format!("missing {name}"))
            },
        )));
        let mut i = Interpreter::new();
        i.run_script("var order=[]").unwrap();
        linker.borrow_mut().link("root","import defer * as lazy from 'leaf';import 'middle';import 'leaf';export {lazy};order.push('root')").unwrap();
        let exports = ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
        i.global_env
            .declare_var("lazy", exports["lazy"].clone())
            .unwrap();
        assert_eq!(
            i.run_script("order.join(',')").unwrap(),
            JsValue::str("middle,leaf,root")
        );
        assert_eq!(i.run_script("lazy.value").unwrap(), JsValue::num(42.0));
        assert!(
            matches!(exports["lazy"],JsValue::Object(ref object) if matches!(object.borrow().exotic,Exotic::DeferredModuleNamespace(_)))
        );
    }
    #[test]
    fn one_async_leaf_releases_diamond_ancestors_in_async_evaluation_order() {
        let sources = [
            ("leaf", "await pending;order.push('leaf');"),
            ("left", "import 'leaf';order.push('left');"),
            ("right", "import 'leaf';order.push('right');"),
            ("both", "import 'left';import 'right';order.push('both');"),
        ];
        let linker = Rc::new(RefCell::new(ModuleLinker::new(
            move |name: &str, _: &str| {
                sources
                    .iter()
                    .find(|(id, _)| *id == name)
                    .map(|(id, source)| (id.to_string(), source.to_string()))
                    .ok_or_else(|| format!("missing {name}"))
            },
        )));
        let mut i = Interpreter::new();
        i.run_script("var order=[],release;var pending=new Promise(r=>release=r)")
            .unwrap();
        linker
            .borrow_mut()
            .link("root", "import 'both';import 'right';order.push('root');")
            .unwrap();
        let completion = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        assert!(promise_fate(&completion).is_none());
        i.run_script("release()").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(
            i.run_script("order.join(',')").unwrap(),
            JsValue::str("leaf,left,right,both,root")
        );
    }
}

#[cfg(test)]
mod cycle_root_completion_tests {
    use super::*;
    #[test]
    fn evaluating_completed_cycle_member_returns_pending_cycle_root_capability() {
        let linker = Rc::new(RefCell::new(ModuleLinker::new(
            |name: &str, _: &str| match name {
                "b" => Ok(("b".into(), "import 'a';export const b=1;".into())),
                "a" => Ok(("a".into(), "import 'b';await pending;throw marker;".into())),
                _ => Err("missing".into()),
            },
        )));
        let mut i = Interpreter::new();
        i.run_script("var release,marker={};var pending=new Promise(r=>release=r)")
            .unwrap();
        linker
            .borrow_mut()
            .link("a", "import 'b';await pending;throw marker;")
            .unwrap();
        let a = ModuleLinker::evaluate_async_shared(&linker, "a", &mut i).unwrap();
        let b = ModuleLinker::evaluate_async_shared(&linker, "b", &mut i).unwrap();
        assert_eq!(a, b);
        assert!(promise_fate(&b).is_none());
        i.run_script("release()").unwrap();
        i.drain_microtasks().unwrap();
        assert!(
            matches!(promise_fate(&b),Some(PromiseFate::Rejected(value)) if value==i.global_env.resolve("marker").unwrap())
        );
        let again = ModuleLinker::evaluate_async_shared(&linker, "b", &mut i).unwrap();
        assert_eq!(again, a);
    }
}
