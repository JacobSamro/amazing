//! Isolated, phase-aware Test262 process host. Protocol is documented in scripts/test262.py.
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, NativeFn, Property, Throw};
use amazing_js::{Interpreter, parse_script};
use amazing_js::interp::RealmHandle;
use std::collections::HashMap;
use std::{
    cell::RefCell,
    io::{self, Read},
    rc::Rc,
};

thread_local! {
    static UNSUPPORTED: RefCell<Option<String>> = const { RefCell::new(None) };
    static STRING: RefCell<HashMap<RealmHandle, JsValue>> = RefCell::new(HashMap::new());
    static ASYNC_DONE: RefCell<Option<Result<(), String>>> = const { RefCell::new(None) };
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(8192) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn emit(phase: &str, kind: &str, detail: &str) {
    let unsupported = UNSUPPORTED.with(|s| s.borrow().clone());
    println!(
        "\nTEST262_RESULT {{\"protocol\":1,\"phase\":{},\"error_type\":{},\"detail\":{},\"unsupported\":{}}}",
        quote(phase),
        quote(kind),
        quote(detail),
        unsupported.as_deref().map(quote).unwrap_or("null".into())
    );
}

fn native(i: &Interpreter, name: &str, call: NativeFn) -> JsValue {
    let mut obj = JsObject::function(JsFunction {
        name: name.into(),
        params: vec![],
        body: FunctionBody::Native(call),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        env: i.global_env.clone(),
        home: None,
        proto_for_new: None,
    });
    obj.proto = Some(i.function_proto.clone());
    obj.set_own("name", JsValue::str(name));
    JsValue::object(obj)
}
fn put(obj: &mut JsObject, name: &str, value: JsValue) {
    let mut p = Property::data(value);
    p.enumerable = false;
    obj.define(name, p).expect("ordinary host property");
}
fn unsupported(name: &str) -> Result<JsValue, Throw> {
    UNSUPPORTED.with(|s| {
        if s.borrow().is_none() {
            *s.borrow_mut() = Some(name.into());
        }
    });
    Err(Throw::type_error(format!(
        "unsupported Test262 host operation: {name}"
    )))
}
fn realm(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let global = i.binding_create_object(None)?;
    let realm = i.binding_create_realm(global)?;
    i.binding_with_realm(realm, |i| {
        install(i);
        i.binding_get_property(&i.binding_global_object(), "$262")
    })
}
fn detach(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.binding_detach_array_buffer(args.first().unwrap_or(&JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}
fn gc(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    unsupported("host.gc")
}
fn agent(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    unsupported("host.agent")
}
fn abstract_module(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    unsupported("host.AbstractModuleSource")
}
fn coerce(i: &mut Interpreter, v: JsValue) -> Result<String, Throw> {
    let string = STRING
        .with(|s| s.borrow().get(&i.binding_current_realm()).cloned())
        .expect("captured String intrinsic");
    match i.call_value(&string, JsValue::Undefined, vec![v])? {
        JsValue::Str(s) => Ok(s.to_string()),
        _ => Err(Throw::type_error("String did not return a string")),
    }
}
fn eval(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = coerce(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    i.binding_eval_in_realm(i.binding_current_realm(), &source)
}
fn print(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    // Prefix and JSON-escape output so test text cannot impersonate a result frame.
    let text = coerce(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    ASYNC_DONE.with(|done| {
        if text == "Test262:AsyncTestComplete" {
            let mut done = done.borrow_mut();
            *done = Some(if done.is_some() { Err("$DONE called more than once".into()) } else { Ok(()) });
        } else if let Some(detail) = text.strip_prefix("Test262:AsyncTestFailure:") {
            *done.borrow_mut() = Some(Err(detail.to_string()));
        }
    });
    println!("TEST262_PRINT {}", quote(&text));
    Ok(JsValue::Undefined)
}
fn install(i: &mut Interpreter) {
    STRING.with(|s| { s.borrow_mut().insert(i.binding_current_realm(), i.global_env.resolve("String").unwrap()); });
    let mut host = JsObject::ordinary();
    host.proto = Some(i.object_proto.clone());
    put(&mut host, "global", JsValue::Object(i.global_obj.clone()));
    for (name, call) in [
        ("evalScript", eval as NativeFn),
        ("createRealm", realm),
        ("detachArrayBuffer", detach),
        ("gc", gc),
        ("AbstractModuleSource", abstract_module),
    ] {
        put(&mut host, name, native(i, name, call));
    }
    let mut agents = JsObject::ordinary();
    agents.proto = Some(i.object_proto.clone());
    for name in [
        "start",
        "broadcast",
        "getReport",
        "sleep",
        "monotonicNow",
        "receiveBroadcast",
        "report",
        "leaving",
    ] {
        put(&mut agents, name, native(i, name, agent));
    }
    put(&mut agents, "canBlock", JsValue::Bool(false));
    put(&mut host, "agent", JsValue::object(agents));
    put(
        &mut i.global_obj.borrow_mut(),
        "$262",
        JsValue::object(host),
    );
    put(
        &mut i.global_obj.borrow_mut(),
        "print",
        native(i, "print", print),
    );
}
fn field(input: &mut &[u8]) -> io::Result<String> {
    let mut size = [0; 4];
    input.read_exact(&mut size)?;
    let n = u32::from_le_bytes(size) as usize;
    if n > 32 * 1024 * 1024 {
        return Err(io::Error::other("oversized source"));
    }
    let mut bytes = vec![0; n];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(io::Error::other)
}
fn describe(error: &Throw) -> String {
    let description = error.describe();
    if description == "[object]" {
        if let JsValue::Object(o) = &error.0 {
            if let Some(JsValue::Str(message)) = o.borrow().get_own("message") {
                return message.to_string();
            }
        }
    }
    description
}
fn error_type(i: &mut Interpreter, inspect: &JsValue, error: &Throw) -> String {
    // Test262 matches the thrown value's constructor name, never its mutable `name`.
    match i.call_value(inspect, JsValue::Undefined, vec![error.0.clone()]) {
        Ok(JsValue::Str(s)) => s.to_string(),
        _ => "<unknown constructor>".into(),
    }
}
fn emit_module(phase: &str, kind: &str, detail: &str, unsupported: Option<&str>) {
    println!(
        "\nTEST262_RESULT {{\"protocol\":2,\"phase\":{},\"error_type\":{},\"detail\":{},\"unsupported\":{}}}",
        quote(phase), quote(kind), quote(detail),
        unsupported.map(quote).unwrap_or("null".into())
    );
}

fn run_module_request() -> io::Result<()> {
    use amazing_js::module::{ModuleLinker, ModulePhase};
    use std::path::Path;
    let mut bytes = Vec::new();
    io::stdin().take(128 * 1024 * 1024).read_to_end(&mut bytes)?;
    let mut input = bytes.as_slice();
    if field(&mut input)? != "AMAZING_TEST262_2_MODULE" {
        return Err(io::Error::other("bad module protocol"));
    }
    let source = field(&mut input)?;
    let path = field(&mut input)?;
    let root = field(&mut input)?;
    let mut helpers = Vec::new();
    while !input.is_empty() {
        helpers.push((field(&mut input)?, field(&mut input)?));
    }
    if let Err(error) = amazing_js::parse_module(&source) {
        emit_module("parse", "SyntaxError", &error.to_string(), None);
        return Ok(());
    }
    let mut i = Interpreter::new();
    install(&mut i);
    let inspect = i.run_script("(function(e) { return e.constructor.name; })")
        .map_err(|error| io::Error::other(error.describe()))?;
    for (name, source) in helpers {
        let helper = match parse_script(&source) {
            Ok(program) => program,
            Err(error) => {
                emit_module("harness-parse", "SyntaxError", &format!("{name}: {error}"), None);
                return Ok(());
            }
        };
        if let Err(error) = i.run_program(&helper) {
            let kind = error_type(&mut i, &inspect, &error);
            emit_module("harness-runtime", &kind, &format!("{name}: {}", describe(&error)), None);
            return Ok(());
        }
    }
    let root_path = Path::new(&root).canonicalize()?;
    let root_path_for_resolver = root_path.clone();
    let path_for_resolver = path.clone();
    let resolve_specifier = move |specifier: &str, referrer: &str| -> Result<(String, String), String> {
        let referrer = if referrer == path_for_resolver { root_path_for_resolver.join(&path_for_resolver) }
            else { Path::new(referrer).to_path_buf() };
        let candidate = referrer.parent().unwrap_or(&root_path_for_resolver).join(specifier);
        let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(&root_path_for_resolver) {
            return Err(format!("module specifier escapes Test262 checkout: {specifier}"));
        }
        let source = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
        Ok((canonical.to_string_lossy().into_owned(), source))
    };
    let linker = Rc::new(RefCell::new(ModuleLinker::new(resolve_specifier)));
    // JS-19 item 1: install the dynamic-import loader hook onto the SAME linker the static
    // graph below links/evaluates through, so `import()` from inside a Test262 module test
    // resolves through the identical module registry (per-realm singleton instances,
    // `ModuleLinker::instances` now persists across `evaluate` calls -- see module.rs) instead
    // of unconditionally rejecting with "dynamic import is not supported by this host". This
    // was the harness gap named in questions.md for
    // `top-level-await/await-dynamic-import-resolution.js`,
    // `top-level-await/dynamic-import-of-waiting-module.js` and
    // `top-level-await/module-graphs-does-not-hang.js`.
    let linker_for_hook = Rc::clone(&linker);
    let root_path_for_hook = root_path.clone();
    let path_for_hook = path.clone();
    i.binding_set_module_loader_hook(Rc::new(
        move |interp: &mut amazing_js::Interpreter, specifier: &str, referrer: &str| -> Result<JsValue, String> {
            // The root module's own "\0module.id" is the raw (possibly non-canonical) `path`
            // the protocol sent, exactly like the static resolver above special-cases; every
            // other module's id is already the canonical path the resolver returned.
            let referrer_path = if referrer.is_empty() || referrer == path_for_hook {
                root_path_for_hook.join(&path_for_hook)
            } else {
                std::path::PathBuf::from(referrer)
            };
            let candidate = referrer_path.parent().unwrap_or(&root_path_for_hook).join(specifier);
            let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
            if !canonical.starts_with(&root_path_for_hook) {
                return Err(format!("module specifier escapes Test262 checkout: {specifier}"));
            }
            let id = canonical.to_string_lossy().into_owned();
            let source = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
            let mut linker = linker_for_hook.borrow_mut();
            linker.link(&id, &source).map_err(|failure| failure.message)?;
            linker.evaluate(&id, interp).map_err(|failure| failure.message)?;
            Ok(linker
                .namespace_object(&id)
                .expect("just-evaluated module has an instance"))
        },
    ));
    let evaluation = {
        let mut linker = linker.borrow_mut();
        linker.link(&path, &source).and_then(|_| linker.evaluate(&path, &mut i).map(|_| ()))
    };
    if let Err(failure) = evaluation {
        match failure.phase {
            ModulePhase::Parse => emit_module("parse", "SyntaxError", &failure.message, None),
            ModulePhase::Resolution => emit_module("resolution", "SyntaxError", &failure.message, None),
            ModulePhase::Async => emit_module("runtime", "", &failure.message, Some("module.async-evaluation")),
            ModulePhase::Runtime => {
                let kind = failure.thrown.as_ref().map(|error| error_type(&mut i, &inspect, error)).unwrap_or_else(|| "Error".into());
                emit_module("runtime", &kind, &failure.message, None);
            }
        }
        return Ok(());
    }
    emit_module("complete", "", "", None);
    Ok(())
}
fn run() -> io::Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(128 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    let mut input = bytes.as_slice();
    if field(&mut input)? != "AMAZING_TEST262_1" {
        return Err(io::Error::other("bad protocol"));
    }
    let source = field(&mut input)?;
    let mut helpers = Vec::new();
    while !input.is_empty() {
        helpers.push((field(&mut input)?, field(&mut input)?));
    }
    let async_test = helpers.iter().any(|(name, _)| name == "doneprintHandle.js");
    ASYNC_DONE.with(|done| *done.borrow_mut() = None);
    let program = match parse_script(&source) {
        Ok(p) => p,
        Err(e) => {
            emit("parse", "SyntaxError", &e.to_string());
            return Ok(());
        }
    };
    let mut i = Interpreter::new();
    install(&mut i);
    let inspect = i
        .run_script("(function(e) { return e.constructor.name; })")
        .map_err(|e| io::Error::other(e.describe()))?;
    for (name, source) in helpers {
        let helper = match parse_script(&source) {
            Ok(p) => p,
            Err(e) => {
                emit("harness-parse", "SyntaxError", &format!("{name}: {e}"));
                return Ok(());
            }
        };
        if let Err(e) = i.run_program(&helper) {
            let kind = error_type(&mut i, &inspect, &e);
            emit(
                "harness-runtime",
                &kind,
                &format!("{name}: {}", describe(&e)),
            );
            return Ok(());
        }
    }
    match i.run_program(&program) {
        Ok(_) => {
            if async_test {
                match ASYNC_DONE.with(|done| done.borrow().clone()) {
                    Some(Ok(())) => emit("complete", "", ""),
                    Some(Err(detail)) => emit("runtime", "Test262Error", &detail),
                    None => emit("runtime", "Test262Error", "$DONE was not called"),
                }
            } else {
                emit("complete", "", "");
            }
        }
        Err(e) => {
            let kind = error_type(&mut i, &inspect, &e);
            emit("runtime", &kind, &describe(&e));
        }
    }
    Ok(())
}
// Keep every host feature flag in one place. Protocol 1 predates module
// requests, so only that protocol's module field is masked below.
struct Capabilities {
    module: bool,
    asynchronous: bool,
    can_block: bool,
    create_realm: bool,
    detach_array_buffer: bool,
    gc: bool,
    agent: bool,
    is_html_dda: bool,
    abstract_module_source: bool,
    eval_script: bool,
    global: bool,
    print: bool,
    intl402: bool,
}

const CAPABILITIES: Capabilities = Capabilities {
    module: true,
    asynchronous: true,
    // Atomics.wait genuinely blocks the calling (sole) agent for the
    // requested timeout (js-binary JSB-10): this host can suspend, so
    // CanBlockIsTrue tests are included and the CanBlockIsFalse test
    // (which assumes a main-thread-style host that cannot suspend at all)
    // is correctly excluded instead of run-and-failed.
    can_block: true,
    create_realm: true,
    detach_array_buffer: true,
    gc: false,
    agent: false,
    is_html_dda: false,
    abstract_module_source: false,
    eval_script: true,
    global: true,
    print: true,
    intl402: true,
};

impl Capabilities {
    fn emit(&self, protocol: u8) {
        println!(
            "{{\"protocol\":{protocol},\"module\":{},\"async\":{},\"can_block\":{},\"createRealm\":{},\"detachArrayBuffer\":{},\"gc\":{},\"agent\":{},\"IsHTMLDDA\":{},\"AbstractModuleSource\":{},\"evalScript\":{},\"global\":{},\"print\":{},\"intl402\":{}}}",
            self.module && protocol >= 2,
            self.asynchronous,
            self.can_block,
            self.create_realm,
            self.detach_array_buffer,
            self.gc,
            self.agent,
            self.is_html_dda,
            self.abstract_module_source,
            self.eval_script,
            self.global,
            self.print,
            self.intl402,
        );
    }
}

fn main() -> std::process::ExitCode {
    if std::env::args().any(|arg| arg == "--capabilities-v2") {
        CAPABILITIES.emit(2);
        return std::process::ExitCode::SUCCESS;
    }
    if std::env::args().any(|arg| arg == "--module-request") {
        return match run_module_request() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => { eprintln!("Test262 module host protocol: {error}"); std::process::ExitCode::FAILURE }
        };
    }
    if std::env::args().any(|a| a == "--capabilities") {
        CAPABILITIES.emit(1);
        return std::process::ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Test262 host protocol: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
