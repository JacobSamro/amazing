//! Run the WebAssembly spec testsuite `.wast` corpus and report counts.
//!
//! Usage:
//!   wast_suite <testsuite-dir> [--skip TOKEN]... [--output FILE]
//!
//! Every file is run on a thread with a large stack because the
//! `assert_exhaustion` cases recurse deeply before the interpreter's own depth
//! limit fires.

use std::path::PathBuf;

use amazing_wasm::{run_directory, WastStats};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut dir = PathBuf::from("corpora/testsuite");
    let mut skip: Vec<String> = vec!["simd_".to_owned(), "skip-stack-guard-page".to_owned()];
    let mut output: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--skip" => {
                i += 1;
                if let Some(tok) = args.get(i) {
                    skip.push(tok.clone());
                }
            }
            "--output" => {
                i += 1;
                output = args.get(i).map(PathBuf::from);
            }
            other => dir = PathBuf::from(other),
        }
        i += 1;
    }

    let dir_for_thread = dir.clone();
    let skip_for_thread = skip.clone();
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let skip_refs: Vec<&str> = skip_for_thread.iter().map(String::as_str).collect();
            run_directory(&dir_for_thread, &skip_refs)
        })
        .expect("spawn runner");
    let stats = handle.join().expect("runner panicked");

    report(&stats, output.as_deref());
}

fn report(stats: &[WastStats], output: Option<&std::path::Path>) {
    let mut total = 0usize;
    let mut passed = 0usize;
    for s in stats {
        total += s.total;
        passed += s.passed;
        println!("{:<34} {:>5}/{:<5}", s.file, s.passed, s.total);
        for f in &s.failures {
            eprintln!("{}: {f}", s.file);
        }
    }
    println!("TOTAL {passed}/{total} across {} files", stats.len());

    if let Some(path) = output {
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str(&format!("  \"total\": {total},\n  \"passed\": {passed},\n"));
        json.push_str(&format!("  \"failed\": {},\n", total - passed));
        json.push_str("  \"files\": [\n");
        for (i, s) in stats.iter().enumerate() {
            json.push_str(&format!(
                "    {{\"file\": \"{}\", \"total\": {}, \"passed\": {}, \"failed\": {}}}",
                s.file,
                s.total,
                s.passed,
                s.failed()
            ));
            if i + 1 != stats.len() {
                json.push(',');
            }
            json.push('\n');
        }
        json.push_str("  ]\n}\n");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, json) {
            eprintln!("failed to write {}: {e}", path.display());
        }
    }
}
