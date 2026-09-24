//! Isolated, phase-aware Test262 process host. Protocol is documented in scripts/test262.py.
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, NativeFn, Property, Throw};
use amazing_js::{Interpreter, parse_script};
use std::{
    cell::RefCell,
    io::{self, Read},
};

thread_local! {
    static UNSUPPORTED: RefCell<Option<String>> = const { RefCell::new(None) };
    static STRING: RefCell<Option<JsValue>> = const { RefCell::new(None) };
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
fn realm(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    unsupported("host.createRealm")
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
        .with(|s| s.borrow().clone())
        .expect("captured String intrinsic");
    match i.call_value(&string, JsValue::Undefined, vec![v])? {
        JsValue::Str(s) => Ok(s.to_string()),
        _ => Err(Throw::type_error("String did not return a string")),
    }
}
fn eval(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = coerce(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    i.run_script(&source)
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
    STRING.with(|s| *s.borrow_mut() = i.global_env.resolve("String"));
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
    let resolver = |specifier: &str, referrer: &str| -> Result<(String, String), String> {
        let referrer = if referrer == path { root_path.join(&path) }
            else { Path::new(referrer).to_path_buf() };
        let candidate = referrer.parent().unwrap_or(&root_path).join(specifier);
        let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(&root_path) {
            return Err(format!("module specifier escapes Test262 checkout: {specifier}"));
        }
        let source = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
        Ok((canonical.to_string_lossy().into_owned(), source))
    };
    let mut linker = ModuleLinker::new(resolver);
    if let Err(failure) = linker.link(&path, &source).and_then(|_| linker.evaluate(&path, &mut i).map(|_| ())) {
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
fn main() -> std::process::ExitCode {
    if std::env::args().any(|arg| arg == "--capabilities-v2") {
        println!("{{\"protocol\":2,\"module\":true,\"async\":true,\"can_block\":false,\"createRealm\":false,\"detachArrayBuffer\":true,\"gc\":false,\"agent\":false,\"IsHTMLDDA\":false,\"AbstractModuleSource\":false,\"evalScript\":true,\"global\":true,\"print\":true,\"intl402\":true}}");
        return std::process::ExitCode::SUCCESS;
    }
    if std::env::args().any(|arg| arg == "--module-request") {
        return match run_module_request() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => { eprintln!("Test262 module host protocol: {error}"); std::process::ExitCode::FAILURE }
        };
    }
    if std::env::args().any(|a| a == "--capabilities") {
        println!(
            "{{\"protocol\":1,\"module\":false,\"async\":true,\"can_block\":false,\"createRealm\":false,\"detachArrayBuffer\":true,\"gc\":false,\"agent\":false,\"IsHTMLDDA\":false,\"AbstractModuleSource\":false,\"evalScript\":true,\"global\":true,\"print\":true,\"intl402\":true}}"
        );
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
