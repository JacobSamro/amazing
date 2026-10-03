// Portions ported from boa-dev/boa (core/engine/src/module/source.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Ordered requested-module walks and ResolveExport are adapted onto Amazing's
// canonical string identifiers, parsed records and std collections.

#[derive(Clone, PartialEq, Eq)]
enum ExportResolution {
    Binding(String, String),
    Namespace(String),
    Missing,
    Ambiguous,
}

impl<R: ModuleResolver> ModuleLinker<R> {
    fn ordered_dependencies(&self, id: &str) -> Vec<String> {
        self.requests
            .get(id)
            .into_iter()
            .flatten()
            .filter(|request| self.request_metadata.get(&(id.into(),(*request).clone())).is_none_or(|r| r.phase != ImportPhase::Source))
            .filter_map(|request| self.records[id].dependencies.get(request).cloned())
            .collect()
    }

    fn first_graph_failure(
        &self,
        id: &str,
        root: bool,
        visited: &mut HashSet<String>,
    ) -> Result<(), ModuleFailure> {
        if !visited.insert(id.into()) {
            return Ok(());
        }
        if let Some(cached) = self.load_failures.get(id) {
            let mut failure = cached.failure.clone();
            if cached.direct_parse {
                failure.phase = if root {
                    ModulePhase::Parse
                } else {
                    ModulePhase::Resolution
                };
            }
            return Err(failure);
        }
        let record = self
            .records
            .get(id)
            .ok_or_else(|| ModuleFailure::new(ModulePhase::Resolution, "module record missing"))?;
        for request in self.requests.get(id).into_iter().flatten() {
            if let Some(failure) = self.request_failures.get(&(id.into(), request.clone())) {
                return Err(failure.clone());
            }
            if self.request_metadata.get(&(id.into(),request.clone())).is_some_and(|r| r.phase == ImportPhase::Source) { continue; }
            self.first_graph_failure(&record.dependencies[request], false, visited)?;
        }
        Ok(())
    }

    fn graph_order(&self, id: &str, visited: &mut HashSet<String>, order: &mut Vec<String>) {
        if !visited.insert(id.into()) {
            return;
        }
        for dependency in self.ordered_dependencies(id) {
            self.graph_order(&dependency, visited, order);
        }
        order.push(id.into());
    }

    fn resolve_export(
        &self,
        id: &str,
        name: &str,
        resolve_set: &mut HashSet<(String, String)>,
    ) -> ExportResolution {
        if self.synthetic.contains_key(id) {
            return if name == "default" { ExportResolution::Binding(id.into(),"\0module.default".into()) }
                else { ExportResolution::Missing };
        }
        if !resolve_set.insert((id.into(), name.into())) {
            return ExportResolution::Missing;
        }
        let Some(record) = self.records.get(id) else {
            return ExportResolution::Missing;
        };
        for item in &record.module.items {
            match item {
                ModuleItem::ExportDefaultExpression(_)
                | ModuleItem::ExportDefaultDeclaration(_)
                    if name == "default" =>
                {
                    let binding = match item {
                        ModuleItem::ExportDefaultDeclaration(stmt) => match &stmt.kind {
                            StmtKind::Function(function) => {
                                function.name.as_ref().map(|name| name.name.clone())
                            }
                            StmtKind::Class(class) => {
                                class.name.as_ref().map(|name| name.name.clone())
                            }
                            _ => None,
                        },
                        _ => None,
                    }
                    .unwrap_or_else(|| "\0module.default".into());
                    return ExportResolution::Binding(id.into(), binding);
                }
                ModuleItem::ExportNamed { names, from, attributes, .. } => {
                    if let Some(entry) = names.iter().find(|entry| entry.exported == name) {
                        if let Some(from) = from {
                            return self.resolve_export(
                                &record.dependencies[&export_key(from, attributes)],
                                &entry.local,
                                resolve_set,
                            );
                        }
                        for item in &record.module.items {
                            if let ModuleItem::Import(import) = item {
                                if let Some(binding) = import
                                    .bindings
                                    .iter()
                                    .find(|binding| binding.local.name == entry.local)
                                {
                                    if import.phase.as_deref()==Some("source") || import.phase.as_deref()==Some("defer") {
                                        return ExportResolution::Binding(id.into(),entry.local.clone());
                                    }
                                    let dependency = &record.dependencies[&import_key(import)];
                                    return if binding.imported == "*" {
                                        ExportResolution::Namespace(dependency.clone())
                                    } else {
                                        self.resolve_export(
                                            dependency,
                                            &binding.imported,
                                            resolve_set,
                                        )
                                    };
                                }
                            }
                        }
                        return ExportResolution::Binding(id.into(), entry.local.clone());
                    }
                }
                ModuleItem::ExportAll {
                    as_name: Some(alias),
                    from,
                    attributes, ..
                } if alias == name => {
                    return ExportResolution::Namespace(record.dependencies[&export_key(from, attributes)].clone());
                }
                _ => {}
            }
        }
        if name == "default" {
            return ExportResolution::Missing;
        }
        let mut star = ExportResolution::Missing;
        for item in &record.module.items {
            if let ModuleItem::ExportAll {
                as_name: None,
                from,
                attributes, ..
            } = item
            {
                let resolution = self.resolve_export(&record.dependencies[&export_key(from, attributes)], name, resolve_set);
                if resolution == ExportResolution::Ambiguous {
                    return resolution;
                }
                if resolution != ExportResolution::Missing {
                    if star == ExportResolution::Missing {
                        star = resolution;
                    } else if star != resolution {
                        return ExportResolution::Ambiguous;
                    }
                }
            }
        }
        star
    }
}

#[cfg(test)]
mod graph_tests {
    use super::*;

    fn registry(
        sources: &'static [(&'static str, &'static str)],
    ) -> ModuleLinker<impl ModuleResolver> {
        ModuleLinker::new(move |specifier: &str, _: &str| {
            sources
                .iter()
                .find(|(id, _)| *id == specifier)
                .map(|(id, source)| ((*id).into(), (*source).into()))
                .ok_or_else(|| format!("missing {specifier}"))
        })
    }

    #[test]
    fn cyclic_roots_select_ordered_intrinsic_parse_errors_without_propagated_cache() {
        let mut linker = registry(&[
            ("a", "import 'b';import 'c';"),
            ("b", "import 'a';import 'd';"),
            ("c", "export const =;"),
            ("d", "%%%;"),
        ]);
        let a = linker.link("a", "import 'b';import 'c';").unwrap_err();
        let b = linker.link("b", "import 'a';import 'd';").unwrap_err();
        let c = linker.link("c", "export const =;").unwrap_err();
        let d = linker.link("d", "%%%;").unwrap_err();
        assert_ne!(a.thrown.as_ref().unwrap().0, b.thrown.as_ref().unwrap().0);
        assert_eq!(a.thrown.unwrap().0, d.thrown.unwrap().0);
        assert_eq!(b.thrown.unwrap().0, c.thrown.unwrap().0);
        assert_eq!(c.phase, ModulePhase::Parse);
        assert!(linker.instances.is_empty());
    }

    #[test]
    fn cyclic_root_link_errors_are_fresh_and_do_not_poison_dependencies() {
        let mut linker = registry(&[
            ("a", "import 'b';import {left} from 'leaf';"),
            ("b", "import 'a';import {right} from 'leaf';"),
            ("leaf", "export const value=42;"),
        ]);
        let a = linker
            .link("a", "import 'b';import {left} from 'leaf';")
            .unwrap_err();
        let b = linker
            .link("b", "import 'a';import {right} from 'leaf';")
            .unwrap_err();
        assert!(a.message.contains("right"));
        assert!(b.message.contains("left"));
        assert_ne!(a.thrown.as_ref().unwrap().0, b.thrown.as_ref().unwrap().0);
        assert_eq!(
            linker.link("a", "").unwrap_err().thrown.unwrap().0,
            a.thrown.unwrap().0
        );
        linker.link("leaf", "export const value=42;").unwrap();
        assert_eq!(
            linker.evaluate("leaf", &mut Interpreter::new()).unwrap()["value"],
            JsValue::num(42.0)
        );
    }

    #[test]
    fn evaluation_preserves_source_requests_and_deduplicates_canonical_records() {
        for shared in [false, true] {
            let mut linker = registry(&[
                ("z", "globalThis.order+='z';export const z=1;"),
                ("a", "globalThis.order+='a';export const a=1;"),
            ]);
            linker.link("root", "import 'z';import 'a';import 'z';globalThis.order+='r';export const done=true;").unwrap();
            let mut interpreter = Interpreter::new();
            interpreter.run_script("globalThis.order='';").unwrap();
            if shared {
                ModuleLinker::evaluate_shared(
                    &Rc::new(RefCell::new(linker)),
                    "root",
                    &mut interpreter,
                )
                .unwrap();
            } else {
                linker.evaluate("root", &mut interpreter).unwrap();
            }
            assert_eq!(
                interpreter.run_script("order").unwrap(),
                JsValue::str("zar")
            );
        }
    }

    #[test]
    fn star_exports_compare_terminal_bindings_and_omit_ambiguous_namespace_names() {
        let mut linker = registry(&[
            ("leaf", "export const value=42;"),
            ("other", "export const value=17;"),
            ("one", "export {value} from 'leaf';"),
            ("two", "export {value} from 'leaf';"),
            ("same", "export * from 'one';export * from 'two';"),
            ("ambiguous", "export * from 'leaf';export * from 'other';"),
            ("outer", "export * from 'ambiguous';export * from 'leaf';"),
            (
                "named-default",
                "export default function f(){return 42;}export {f};",
            ),
            (
                "default-alias",
                "export {default as call} from 'named-default';",
            ),
            ("named-alias", "export {f as call} from 'named-default';"),
            (
                "both-aliases",
                "export * from 'default-alias';export * from 'named-alias';",
            ),
            ("ns-one", "import * as ns from 'leaf';export {ns};"),
            ("ns-two", "export * as ns from 'leaf';"),
            ("ns-both", "export * from 'ns-one';export * from 'ns-two';"),
        ]);
        linker
            .link("valid", "import {value} from 'same';export {value};")
            .unwrap();
        let mut interpreter = Interpreter::new();
        assert_eq!(
            linker.evaluate("valid", &mut interpreter).unwrap()["value"],
            JsValue::num(42.0)
        );
        linker.link("aliases", "import {call} from 'both-aliases';import {ns} from 'ns-both';export const good=call()===ns.value;").unwrap();
        assert_eq!(
            linker.evaluate("aliases", &mut interpreter).unwrap()["good"],
            JsValue::Bool(true)
        );
        assert!(linker
            .link("bad", "import {value} from 'ambiguous';")
            .unwrap_err()
            .thrown
            .is_some());
        linker
            .link("ambiguous", "export * from 'leaf';export * from 'other';")
            .unwrap();
        assert!(!linker
            .evaluate("ambiguous", &mut interpreter)
            .unwrap()
            .contains_key("value"));
        linker
            .link("outer", "export * from 'ambiguous';export * from 'leaf';")
            .unwrap();
        assert!(!linker
            .evaluate("outer", &mut interpreter)
            .unwrap()
            .contains_key("value"));
    }
}
