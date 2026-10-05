//! Shared observable static-request matrix: uses the existing text link API
//! and typed resolver on both baseline and candidate, without a fake UTF16 API.
use super::*;
use crate::value::JsString;

#[derive(Clone)]
struct CaptureResolver {
    seen: Rc<RefCell<Vec<ModuleRequest>>>,
    shared_id: bool,
}
impl ModuleResolver for CaptureResolver {
    fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
        Err("typed resolver unexpectedly fell through to legacy transport".into())
    }
    fn resolve_request(&self, request: &ModuleRequest, _: &str) -> Result<LoadedModule, String> {
        self.seen.borrow_mut().push(request.clone());
        let id = if self.shared_id {
            "shared".into()
        } else {
            request.edge_key()
        };
        let source = if request.phase == ImportPhase::Source {
            ModuleSource::Synthetic {
                default_export: JsValue::num(42.0),
                module_source: Some(JsValue::num(7.0)),
            }
        } else {
            ModuleSource::JavaScript("export const x=41;export default 42;".into())
        };
        Ok(LoadedModule { id, source })
    }
}
fn capture(source: &str) -> (ModuleLinker<CaptureResolver>, Vec<ModuleRequest>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut linker = ModuleLinker::new(CaptureResolver {
        seen: seen.clone(),
        shared_id: false,
    });
    linker
        .link("root", source)
        .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    let requests = seen.borrow().clone();
    (linker, requests)
}
fn units(value: &[u16]) -> JsString {
    JsString::from_units(value.to_vec())
}

#[test]
fn every_static_request_form_preserves_specifier_code_units() {
    let spellings: &[(&str, &[u16])] = &[
        (r"\uD800", &[0xD800]),
        (r"\uDC00", &[0xDC00]),
        (r"\uFFFD", &[0xFFFD]),
        (r"\uD800\uDC00", &[0xD800, 0xDC00]),
        (r"\u{10000}", &[0xD800, 0xDC00]),
        (r"\0", &[0]),
        (r"a\\b", &[0x61, 0x5C, 0x62]),
    ];
    let forms = [
        "import '{s}';",
        "import x from '{s}';",
        "import {x} from '{s}';",
        "import * as ns from '{s}';",
        "export {x} from '{s}';",
        "export * from '{s}';",
        "export * as ns from '{s}';",
        "import source x from '{s}';",
        "import defer * as ns from '{s}';",
    ];
    let mut failures = Vec::new();
    for (spelling, expected) in spellings {
        for form in forms {
            let source = form.replace("{s}", spelling);
            let seen = Rc::new(RefCell::new(Vec::new()));
            let mut linker = ModuleLinker::new(CaptureResolver {
                seen: seen.clone(),
                shared_id: false,
            });
            let result = linker.link("root", &source);
            let requests = seen.borrow();
            if result.is_err() || requests.len() != 1 || requests[0].specifier.units() != *expected
            {
                failures.push(format!("{source}: {result:?}, {requests:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn import_attribute_keys_values_and_sorting_are_exact_utf16() {
    let sources = [
        r"import 'dep' with {'\uD800':'\uDC00','\uE000':'\uD800',type:'\uFFFD'};",
        r"import x from 'dep' with {'\uD800':'\uDC00','\uE000':'\uD800',type:'\uFFFD'};",
        r"export {x} from 'dep' with {'\uD800':'\uDC00','\uE000':'\uD800',type:'\uFFFD'};",
        r"export * from 'dep' with {'\uD800':'\uDC00','\uE000':'\uD800',type:'\uFFFD'};",
    ];
    let expected = vec![
        ("type".into(), units(&[0xFFFD])),
        (units(&[0xD800]), units(&[0xDC00])),
        (units(&[0xE000]), units(&[0xD800])),
    ];
    let mut failures = Vec::new();
    for source in sources {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut linker = ModuleLinker::new(CaptureResolver {
            seen: seen.clone(),
            shared_id: false,
        });
        let result = linker.link("root", source);
        if result.is_err() || seen.borrow().len() != 1 || seen.borrow()[0].attributes != expected {
            failures.push(format!("{source}: {result:?}, {:?}", seen.borrow()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let (_, requests) =
        capture(r"import 'dep' with {'\uD800\uDC00':'pair','\uE000':'bmp','\uD800':'high'};");
    assert_eq!(
        requests[0]
            .attributes
            .iter()
            .map(|(k, _)| k.clone())
            .collect::<Vec<_>>(),
        vec![units(&[0xD800]), units(&[0xD800, 0xDC00]), units(&[0xE000])]
    );
}

#[test]
fn exact_attribute_duplicate_errors_do_not_reject_distinct_surrogates() {
    for source in [
        r"import 'dep' with {type:'a','type':'b'};",
        r"import 'dep' with {type:'a','\u0074ype':'b'};",
        r"import 'dep' with {'\uD800':'a','\uD800':'b'};",
        r"export * from 'dep' with {'\uD800\uDC00':'a','\u{10000}':'b'};",
    ] {
        assert!(crate::parse_module(source).is_err(), "{source}");
    }
    for source in [
        r"import 'dep' with {'\uD800':'a','\uDC00':'b','\uFFFD':'c'};",
        r"import 'dep' with {default:'a',await:'b','\u0000':'c'};",
    ] {
        assert!(crate::parse_module(source).is_ok(), "{source}");
    }
    // ModuleExportName has its independent well-formed-Unicode early error.
    for source in [
        r"import {'\uD800' as x} from 'dep';",
        r"export {'\uDC00' as x} from 'dep';",
        r"export * as '\uD800' from 'dep';",
    ] {
        assert!(crate::parse_module(source).is_err(), "{source}");
    }
}

#[test]
fn request_dedup_keeps_first_order_and_does_not_merge_lossy_collisions() {
    let (_, requests) = capture(
        r"import '\uD800';import '\uFFFD';import '\uDC00';import '\uD800';import '\uD800\uDC00';import '\u{10000}';",
    );
    assert_eq!(
        requests
            .iter()
            .map(|r| r.specifier.clone())
            .collect::<Vec<_>>(),
        vec![
            units(&[0xD800]),
            units(&[0xFFFD]),
            units(&[0xDC00]),
            units(&[0xD800, 0xDC00])
        ]
    );
    let (_, requests) = capture(
        r"import 'dep' with {a:'\uD800',b:'x'};import 'dep' with {b:'x',a:'\uD800'};import 'dep' with {a:'\uFFFD',b:'x'};",
    );
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].attributes[0].1, units(&[0xD800]));
    assert_eq!(requests[1].attributes[0].1, units(&[0xFFFD]));
}

#[test]
fn typed_request_edges_and_host_selected_canonical_identity_stay_distinct() {
    for actor in [false, true] {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut linker = ModuleLinker::new(CaptureResolver {
            seen: seen.clone(),
            shared_id: true,
        });
        linker
            .link(
                "root",
                r"import a from '\uD800';import b from '\uFFFD';export const result=a===b&&a===42;",
            )
            .unwrap();
        assert_eq!(seen.borrow().len(), 2, "host must see both exact requests");
        assert_eq!(
            linker.records.len(),
            2,
            "host chose one shared canonical module"
        );
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        let result =
            ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), "root", &mut i).unwrap();
        assert_eq!(result["result"], JsValue::Bool(true));
    }
    let (_, requests) = capture(r"import * as a from '\uD800';import defer * as b from '\uD800';");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].phase, ImportPhase::Evaluation);
    assert_eq!(requests[1].phase, ImportPhase::Defer);
    assert_eq!(requests[0].specifier, requests[1].specifier);
}

#[test]
fn legacy_resolver_remains_explicitly_lossy_and_rejects_attributes() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let observer = seen.clone();
    let mut linker = ModuleLinker::new(move |specifier: &str, _: &str| {
        observer.borrow_mut().push(specifier.to_owned());
        Ok((specifier.to_owned(), "export const x=1;".into()))
    });
    linker
        .link("root", r"import '\uD800';import '\uFFFD';")
        .unwrap();
    assert_eq!(
        &*seen.borrow(),
        &["\u{FFFD}".to_owned(), "\u{FFFD}".to_owned()]
    );
    assert_eq!(
        linker.records.len(),
        2,
        "legacy host selected the same canonical ID"
    );
    let error = linker
        .link("attrs", "import 'dep' with {type:'json'};")
        .unwrap_err();
    assert_eq!(error.phase, ModulePhase::Resolution);
    assert!(error.message.contains("does not support import attributes"));
    assert_eq!(
        seen.borrow().len(),
        2,
        "reject attributes before the legacy callback"
    );
}

struct TypedGraph(Vec<(JsString, String, String)>);
impl ModuleResolver for TypedGraph {
    fn resolve(&self, _: &str, _: &str) -> Result<(String, String), String> {
        Err("typed graph only".into())
    }
    fn resolve_request(&self, request: &ModuleRequest, _: &str) -> Result<LoadedModule, String> {
        let (_, id, source) = self
            .0
            .iter()
            .find(|(key, _, _)| key == &request.specifier)
            .ok_or_else(|| "missing exact graph specifier".to_owned())?;
        Ok(LoadedModule {
            id: id.clone(),
            source: ModuleSource::JavaScript(source.clone()),
        })
    }
}

#[test]
fn exact_static_requests_preserve_cycles_tla_and_deferred_live_namespace() {
    for actor in [false, true] {
        let mut linker = ModuleLinker::new(TypedGraph(vec![
            (
                units(&[0xD800]),
                "leaf".into(),
                "import {saved} from 'root';export default function(){return saved()};".into(),
            ),
            ("root".into(), "root".into(), String::new()),
        ]));
        linker.link("root", r"import read from '\uD800';export function saved(){return 7};export const result=read()===7;").unwrap();
        let mut i = Interpreter::new();
        i.test_force_ast_callables(actor);
        let result =
            ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), "root", &mut i).unwrap();
        assert_eq!(result["result"], JsValue::Bool(true), "cycle actor={actor}");

        let mut linker = ModuleLinker::new(TypedGraph(vec![(
            units(&[0xDC00]),
            "leaf".into(),
            "export const value=await Promise.resolve(42);".into(),
        )]));
        linker
            .link(
                "root",
                r"import {value} from '\uDC00';export const result=value===42;",
            )
            .unwrap();
        let linker = Rc::new(RefCell::new(linker));
        let completion = ModuleLinker::evaluate_async_shared(&linker, "root", &mut i).unwrap();
        i.drain_microtasks().unwrap();
        assert!(matches!(
            i.binding_module_evaluation_state(&completion).unwrap(),
            PromiseFate::Fulfilled(_)
        ));
        let result = ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
        assert_eq!(result["result"], JsValue::Bool(true), "TLA actor={actor}");

        i.run_script("globalThis.loads=0;").unwrap();
        let mut linker = ModuleLinker::new(TypedGraph(vec![(
            units(&[0xD800]),
            "leaf".into(),
            "loads++;export let x=42;export function bump(){x++};".into(),
        )]));
        linker.link("root", r"import defer * as ns from '\uD800';export function read(){return ns.x};export function bump(){ns.bump()};").unwrap();
        let linker = Rc::new(RefCell::new(linker));
        let result = ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
        assert_eq!(i.run_script("loads").unwrap(), JsValue::num(0.0));
        assert_eq!(
            i.call_value(&result["read"], JsValue::Undefined, vec![])
                .unwrap(),
            JsValue::num(42.0)
        );
        i.call_value(&result["bump"], JsValue::Undefined, vec![])
            .unwrap();
        assert_eq!(
            i.call_value(&result["read"], JsValue::Undefined, vec![])
                .unwrap(),
            JsValue::num(43.0)
        );
        assert_eq!(i.run_script("loads").unwrap(), JsValue::num(1.0));
    }
}

#[test]
fn static_source_phase_keeps_genuine_synthetic_policy_and_no_eager_expansion() {
    let (linker, requests) =
        capture(r"import source sourceObject from '\uD800';export const result=sourceObject===7;");
    assert_eq!(requests[0].phase, ImportPhase::Source);
    let mut i = Interpreter::new();
    let result =
        ModuleLinker::evaluate_shared(&Rc::new(RefCell::new(linker)), "root", &mut i).unwrap();
    assert_eq!(result["result"], JsValue::Bool(true));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let observer = seen.clone();
    let linker = Rc::new(RefCell::new(ModuleLinker::new(
        move |name: &str, _: &str| {
            observer.borrow_mut().push(name.to_owned());
            Ok((
                name.to_owned(),
                "import 'unvisited';export default 1;".into(),
            ))
        },
    )));
    let request = ModuleRequest::new("text", ImportPhase::Source, vec![]);
    let error = ModuleLinker::import_shared(&linker, &request, "", &mut i).unwrap_err();
    assert!(error.message.contains("source-text modules"));
    assert_eq!(
        &*seen.borrow(),
        &["text".to_owned()],
        "source load must not visit nested imports"
    );
}

#[test]
fn cached_direct_parser_failures_use_the_requesting_boundary_phase() {
    for root_first in [false, true] {
        let mut linker = ModuleLinker::new(|_: &str, _: &str| {
            Ok(("canonical".into(), "export const broken = %;".into()))
        });
        let request = ModuleRequest::new("bad", ImportPhase::Evaluation, vec![]);
        let first = if root_first {
            linker
                .link("canonical", "export const broken = %;")
                .unwrap_err()
        } else {
            linker.load_request(&request, "caller").unwrap_err()
        };
        let dependency = linker.load_request(&request, "caller").unwrap_err();
        let root = linker
            .link("canonical", "export const repaired = 1;")
            .unwrap_err();
        assert_eq!(
            dependency.phase,
            ModulePhase::Resolution,
            "root_first={root_first}"
        );
        assert_eq!(root.phase, ModulePhase::Parse, "root_first={root_first}");
        assert_eq!(first.message, dependency.message);
        assert_eq!(first.message, root.message);
        assert_eq!(
            first.thrown.as_ref().unwrap().0,
            dependency.thrown.as_ref().unwrap().0
        );
        assert_eq!(
            first.thrown.as_ref().unwrap().0,
            root.thrown.as_ref().unwrap().0
        );
    }
}

#[test]
fn canonical_first_source_wins_after_later_host_source_changes() {
    let calls = Rc::new(RefCell::new(0));
    let counter = calls.clone();
    let mut linker = ModuleLinker::new(move |_: &str, _: &str| {
        let mut count = counter.borrow_mut();
        *count += 1;
        Ok((
            "canonical".into(),
            if *count == 1 {
                "globalThis.runs++;export default 41;".into()
            } else {
                "% malformed later source".into()
            },
        ))
    });
    linker
        .load_request(
            &ModuleRequest::new("first", ImportPhase::Evaluation, vec![]),
            "",
        )
        .unwrap();
    linker
        .load_request(
            &ModuleRequest::new("second", ImportPhase::Evaluation, vec![]),
            "",
        )
        .unwrap();
    assert_eq!(*calls.borrow(), 2);
    let mut i = Interpreter::new();
    i.run_script("globalThis.runs=0;").unwrap();
    let linker = Rc::new(RefCell::new(linker));
    for _ in 0..2 {
        let result = ModuleLinker::evaluate_shared(&linker, "canonical", &mut i).unwrap();
        assert_eq!(result["default"], JsValue::num(41.0));
    }
    assert_eq!(i.run_script("runs").unwrap(), JsValue::num(1.0));
}
