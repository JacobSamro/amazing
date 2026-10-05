//! Genuine new UTF16 entry points; these groups are candidate-only.
use super::*;

struct UnitResolver {
    source: Vec<u16>,
    seen: Rc<RefCell<Vec<ModuleRequest>>>,
}
impl ModuleResolver for UnitResolver {
    fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
        Err("UTF16 typed source must not use legacy text".into())
    }
    fn resolve_request(&self, request: &ModuleRequest, _: &str) -> Result<LoadedModule, String> {
        self.seen.borrow_mut().push(request.clone());
        Ok(LoadedModule {
            id: "dep".into(),
            source: ModuleSource::JavaScriptUtf16(self.source.clone()),
        })
    }
}
fn raw_source(prefix: &str, payload: &[u16], suffix: &str) -> Vec<u16> {
    prefix
        .encode_utf16()
        .chain(payload.iter().copied())
        .chain(suffix.encode_utf16())
        .collect()
}

#[test]
fn utf16_root_and_dependency_sources_keep_real_literals_and_function_source() {
    for actor in [false, true] {
        for unit in [0xD800, 0xDC00, 0xFFFD] {
            let seen = Rc::new(RefCell::new(Vec::new()));
            let dependency = raw_source(
                "export const value='",
                &[unit],
                "';export function spelling(){return '",
            );
            let mut dependency = dependency;
            dependency.extend([unit]);
            dependency.extend("'}".encode_utf16());
            let mut root_source = raw_source(
                "import {value,spelling} from '",
                &[unit],
                "';export const result=value.charCodeAt(0)===",
            );
            root_source.extend(format!("{unit}&&spelling().charCodeAt(0)==={unit}&&spelling.toString().charCodeAt(spelling.toString().indexOf(\"return '\")+8)==={unit};").encode_utf16());
            let mut linker = ModuleLinker::new(UnitResolver {
                source: dependency,
                seen: seen.clone(),
            });
            linker.link_utf16("root", &root_source).unwrap();
            assert_eq!(seen.borrow()[0].specifier.units(), &[unit]);
            let mut i = Interpreter::new();
            i.test_force_ast_callables(actor);
            let exports =
                ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), "root", &mut i)
                    .unwrap();
            assert_eq!(
                exports["result"],
                JsValue::Bool(true),
                "actor={actor}, unit={unit:x}"
            );
        }
    }
}

#[test]
fn raw_and_escaped_module_values_clone_without_losing_units() {
    for unit in [0xD800, 0xDC00, 0xFFFD] {
        let raw = raw_source("import '", &[unit], "' with {'");
        let mut raw = raw;
        raw.extend([unit]);
        raw.extend("':'".encode_utf16());
        raw.extend([unit]);
        raw.extend("'};".encode_utf16());
        let escaped = format!("import '\\u{unit:04X}' with {{'\\u{unit:04X}':'\\u{unit:04X}'}};");
        let parsed = crate::parse_module_utf16(&raw).unwrap();
        let cloned = parsed.clone();
        let escaped = crate::parse_module(&escaped).unwrap();
        assert_eq!(
            item_request(&parsed.items[0]),
            item_request(&escaped.items[0])
        );
        drop(raw);
        drop(parsed);
        let ModuleItem::Import(import) = &cloned.items[0] else {
            panic!("import")
        };
        assert_eq!(import.from.units(), &[unit]);
        assert_eq!(import.attributes[0].key.units(), &[unit]);
        assert_eq!(import.attributes[0].value.units(), &[unit]);
        let request = item_request(&cloned.items[0]).unwrap();
        drop(cloned);
        assert_eq!(request.specifier.units(), &[unit]);
        assert_eq!(request.attributes[0].1.units(), &[unit]);
    }
}

#[test]
fn utf16_load_errors_keep_cached_phase_and_repeated_throw_identity() {
    let mut linker = ModuleLinker::new(|_: &str, _: &str| Err("unexpected resolver".into()));
    let invalid = raw_source("export const x='", &[0xD800], "'; %");
    let first = linker.link_utf16("bad", &invalid).unwrap_err();
    let second = linker.link("bad", "export const corrected=1;").unwrap_err();
    assert_eq!(first.phase, ModulePhase::Parse);
    assert_eq!(first.message, second.message);
    assert_eq!(first.thrown.unwrap().0, second.thrown.unwrap().0);
    let source = raw_source("export const x='", &[0xDC00], "'; %");
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut linker = ModuleLinker::new(UnitResolver { source, seen });
    let failure = linker.link("root", "import {x} from 'dep';").unwrap_err();
    assert_eq!(failure.phase, ModulePhase::Resolution);
    assert!(failure.thrown.is_some());
}

#[test]
fn utf16_source_phase_defers_expansion_then_reuses_the_first_canonical_source() {
    struct Phases {
        calls: Rc<RefCell<Vec<String>>>,
    }
    impl ModuleResolver for Phases {
        fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
            Err("typed only".into())
        }
        fn resolve_request(
            &self,
            request: &ModuleRequest,
            _: &str,
        ) -> Result<LoadedModule, String> {
            let name = request.specifier.to_utf8_lossy();
            let repeated = self.calls.borrow().contains(&name);
            self.calls.borrow_mut().push(name.clone());
            let source = match (name.as_str(), repeated) {
                ("main", false) => raw_source(
                    "import {x} from 'leaf';export const value='",
                    &[0xD800],
                    "';export const result=x===42;",
                ),
                ("leaf", _) => "export const x=42;".encode_utf16().collect(),
                _ => "% invalid later source".encode_utf16().collect(),
            };
            Ok(LoadedModule {
                id: name,
                source: ModuleSource::JavaScriptUtf16(source),
            })
        }
    }
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut linker = ModuleLinker::new(Phases {
        calls: calls.clone(),
    });
    let source = ModuleRequest::new("main", ImportPhase::Source, vec![]);
    linker.load_request(&source, "").unwrap();
    assert_eq!(&*calls.borrow(), &["main".to_owned()]);
    let evaluation = ModuleRequest::new("main", ImportPhase::Evaluation, vec![]);
    linker.load_request(&evaluation, "").unwrap();
    linker.load_request(&evaluation, "").unwrap();
    assert_eq!(
        &*calls.borrow(),
        &[
            "main".to_owned(),
            "main".to_owned(),
            "leaf".to_owned(),
            "main".to_owned()
        ]
    );
    let mut i = Interpreter::new();
    let exports =
        ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), "main", &mut i).unwrap();
    assert_eq!(exports["result"], JsValue::Bool(true));
    let JsValue::Str(value) = &exports["value"] else {
        panic!("exact string")
    };
    assert_eq!(value.units(), &[0xD800]);
}

#[test]
fn typed_utf16_graph_retains_host_canonical_id_and_explicit_source_url() {
    struct Located;
    impl ModuleResolver for Located {
        fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
            panic!("typed graph must not use legacy resolution")
        }
        fn source_url(&self, id: &str) -> Option<String> {
            match id {
                "inline-record" => Some("https://example.test/page.html".into()),
                "accepted-final-record" => Some("https://cdn.test/final.js".into()),
                _ => None,
            }
        }
        fn resolve_request(&self, request: &ModuleRequest, referrer: &str) -> Result<LoadedModule, String> {
            assert_eq!(request.specifier.units(), &[0xd800]);
            assert_eq!(referrer, "inline-record");
            Ok(LoadedModule { id: "accepted-final-record".into(), source: ModuleSource::JavaScriptUtf16(
                "export const url=import.meta.url;".encode_utf16().collect()) })
        }
    }
    let mut linker = ModuleLinker::new(Located);
    let root = raw_source("import {url} from '", &[0xd800], "';export const urls=[import.meta.url,url];");
    linker.link_utf16("inline-record", &root).unwrap();
    let mut i = Interpreter::new();
    let registry = Rc::new(RefCell::new(linker));
    ModuleLinker::evaluate_shared(&registry, "inline-record", &mut i).unwrap();
    let namespace = registry.borrow().namespace_object("inline-record").unwrap();
    i.global_env.declare_var("located", namespace).unwrap();
    assert_eq!(i.run_script("located.urls.join(',')").unwrap(),
        JsValue::str("https://example.test/page.html,https://cdn.test/final.js"));
}
