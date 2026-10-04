//! Whole official Unicode17 conformance corpus over actual installed APIs.
#[path = "../tests/support/unicode_normalization17.rs"]
mod corpus;
use amazing_js::{Interpreter, interp::ExecutionBackend};
use std::{fs, fmt::Write};

fn main() {
    let output = std::env::args().nth(1).expect("usage: unicode_normalization17 <report.tsv>");
    let mut report = String::from("# Unicode17 NormalizationTest: all20 invariants per row, both entry drivers\n");
    let mut failed = 0;
    for (name, backend) in [("tree", ExecutionBackend::TreeWalk), ("bytecode", ExecutionBackend::Bytecode)] {
        let mut interpreter = Interpreter::new();
        let result = corpus::run_rows(&mut interpreter, backend);
        for (line, passed) in &result.row_statuses {
            writeln!(report, "{name}\t{line}\t{}", if *passed { "pass" } else { "fail" }).unwrap();
        }
        let (unlisted, failures) = corpus::run_unlisted(&mut interpreter, backend);
        writeln!(report, "{name}\tunlisted\t{}", if failures.is_empty() { "pass" } else { "fail" }).unwrap();
        eprintln!("{name}: {} rows, {} row checks, {} failed checks, {unlisted} unlisted points, {} failed unlisted forms",
            result.row_statuses.len(), result.checks, result.failed_checks, failures.len());
        for diagnostic in result.diagnostics.iter().chain(failures.iter()) { eprintln!("{diagnostic}"); }
        failed += result.failed_checks + failures.len();
    }
    fs::write(output, report).unwrap();
    if failed != 0 { std::process::exit(1); }
}
