//! Benchmark host: `js_bench_host <program.js>` runs one script with a global
//! `print`, then reports `BENCH_MS <elapsed>` on stderr (script run only, not
//! process startup or parse). Set AMAZING_JS_VM=1 to run top-level code on the
//! VM and AMAZING_JS_VM_STATS=<file> to record VM/fallback coverage.
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, Property, Throw};
use amazing_js::{parse_script, Interpreter};
use std::time::Instant;

fn print(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let units = i.binding_to_dom_string(args.first().unwrap_or(&JsValue::Undefined))?;
    println!("{}", String::from_utf16_lossy(&units));
    Ok(JsValue::Undefined)
}

fn main() -> std::process::ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: js_bench_host <program.js>");
        return std::process::ExitCode::from(2);
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => { eprintln!("{path}: {error}"); return std::process::ExitCode::from(2); }
    };
    let program = match parse_script(&source) {
        Ok(program) => program,
        Err(error) => { eprintln!("{path}: {error}"); return std::process::ExitCode::FAILURE; }
    };
    let mut interpreter = Interpreter::new();
    let mut function = JsObject::function(JsFunction {
        source: Default::default(),
        name: "print".into(),
        params: vec![],
        body: FunctionBody::Native(print),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        env: interpreter.global_env.clone(),
        home: None,
        proto_for_new: None,
    });
    function.proto = Some(interpreter.function_proto.clone());
    let mut property = Property::data(JsValue::object(function));
    property.enumerable = false;
    interpreter.global_obj.borrow_mut().define("print", property).expect("print");
    let start = Instant::now();
    let result = interpreter.run_program(&program);
    let elapsed = start.elapsed();
    match result {
        Ok(_) => {
            eprintln!("BENCH_MS {:.3}", elapsed.as_secs_f64() * 1000.0);
            std::process::ExitCode::SUCCESS
        }
        Err(error) => { eprintln!("uncaught: {}", error.describe()); std::process::ExitCode::FAILURE }
    }
}
