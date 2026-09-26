//! Repeatable, dependency-free JS microbenchmarks. Run with
//! `cargo bench -p amazing-js --bench vm_performance --locked`.
use amazing_js::interp::ExecutionBackend;
use amazing_js::{parse_script, Interpreter};
use std::hint::black_box;
use std::time::{Duration, Instant};

const CASES: &[(&str, &str)] = &[
    ("property_get", "var o={x:7,y:9}; var s=0; for(var i=0;i<4000;i++) s+=o.x; s"),
    ("property_set", "var o={x:0,y:9}; for(var i=0;i<4000;i++) o.x=i; o.x"),
    ("calls", "function f(x){return x+1} var s=0; for(var i=0;i<2000;i++) s=f(s); s"),
    ("closures", "function make(x){return function(y){return x+y}} var f=make(3); var s=0; for(var i=0;i<2000;i++) s=f(i); s"),
    ("array_loop", "var a=[1,2,3,4,5,6,7,8]; var s=0; for(var i=0;i<4000;i++) s+=a[i%8]; s"),
    ("string_build", "var s=''; for(var i=0;i<600;i++) s+='ab'; s.length"),
    ("json", "var o={a:1,b:[2,3,4],c:'hello'}; var s=''; for(var i=0;i<150;i++) s=JSON.stringify(o); s.length"),
    ("integer_math", "var s=0; for(var i=0;i<4000;i++) if(i<3000) s+=i*2-1; s"),
];

fn sample(source: &str, backend: ExecutionBackend, samples: usize) -> (String, Duration) {
    let program = parse_script(source).expect("valid benchmark");
    let mut durations = Vec::with_capacity(samples);
    let mut result = String::new();
    for _ in 0..samples {
        let mut interpreter = Interpreter::new();
        let start = Instant::now();
        let value = interpreter.run_program_with_backend(black_box(&program), backend)
            .expect("benchmark execution");
        durations.push(start.elapsed());
        let value = format!("{value:?}");
        if !result.is_empty() { assert_eq!(result, value); }
        result = value;
    }
    durations.sort_unstable();
    (result, durations[samples / 2])
}

fn main() {
    let samples = std::env::var("AMAZING_BENCH_SAMPLES").ok()
        .and_then(|value| value.parse::<usize>().ok()).filter(|value| *value > 0)
        .unwrap_or(9);
    println!("case,tree_us,vm_us,speedup,result");
    for &(name, source) in CASES {
        let (tree_result, tree) = sample(source, ExecutionBackend::TreeWalk, samples);
        let (vm_result, vm) = sample(source, ExecutionBackend::Bytecode, samples);
        assert_eq!(tree_result, vm_result, "backend result mismatch for {name}");
        println!("{name},{},{},{:.3},{}", tree.as_micros(), vm.as_micros(),
            tree.as_secs_f64() / vm.as_secs_f64(), vm_result);
    }
}
