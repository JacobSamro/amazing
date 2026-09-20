//! Read an unmodified WPT testharness.js on stdin; verify real assertion callbacks.
//! Supplies only the shell's global `self` alias. This is not a DOM WPT run.
use amazing_js::{parse_script, value::JsValue, Interpreter};
use std::io::Read;

const PROBE: &str = r#"
setup({explicit_done:true});
var js2_results=[];
var js2_completion=-1;
add_result_callback(function(t){js2_results.push(t.status);});
add_completion_callback(function(tests,status){js2_completion=status.status;});
test(function(){
    assert_true(/^a+b$/i.test('AAb'));
    assert_equals('a1a2'.replace(/a(\d)/g,'$1'),'12');
},'JS-2 regex pass');
test(function(){assert_true(false);},'JS-2 intentional failure');
done();
"#;

fn main() -> std::process::ExitCode {
    let mut source = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut source) {
        eprintln!("stdin: {error}");
        return std::process::ExitCode::FAILURE;
    }
    if let Err(error) = parse_script(&source) {
        eprintln!("parse: {error}");
        return std::process::ExitCode::FAILURE;
    }
    let mut interpreter = Interpreter::new();
    for script in ["var self=globalThis;", source.as_str(), PROBE] {
        if let Err(error) = interpreter.run_script(script) {
            eprintln!("execution: {error:?}");
            return std::process::ExitCode::FAILURE;
        }
    }
    match interpreter.run_script("js2_results.join(',')==='0,1' && js2_completion===0") {
        Ok(JsValue::Bool(true)) => {
            println!("parsed; initialized; subtests=PASS,FAIL; completion=OK");
            std::process::ExitCode::SUCCESS
        }
        outcome => {
            eprintln!("unexpected harness callbacks: {outcome:?}");
            std::process::ExitCode::FAILURE
        }
    }
}
