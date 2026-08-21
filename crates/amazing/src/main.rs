//! amazing — browser driver binary.
//!
//! Phase 0 artifact: builds and runs, prints the component pipeline status.
//! Each phase replaces one line of "not started" with a real capability
//! plus its demo subcommand (e.g. `amazing parse <file>` in Phase 1).

use std::process::ExitCode;

const PIPELINE: &[(&str, &str)] = &[
    ("amazing-common", "byte streams + text utils"),
    ("amazing-html", "Phase 1: WHATWG tokenizer/tree builder — not started"),
    ("amazing-css", "Phase 2: syntax 3 + selectors + cascade — not started"),
    ("amazing-layout", "Phase 3: block/inline FC — not started"),
    ("amazing-paint", "Phase 4: software rasterizer — not started"),
    ("amazing-net", "Phase 5: HTTP/1.1 — not started"),
    ("amazing-js", "Phase 6: ECMA-262 interpreter — not started"),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("version") | None => {
            println!("amazing v{}", env!("CARGO_PKG_VERSION"));
            println!("zero-dependency browser engine — pipeline status:");
            for (crate_name, status) in PIPELINE {
                println!("  {crate_name:<15} {status}");
            }
            println!("dom: arena skeleton present (u32 indices, no Rc<RefCell>)");
            ExitCode::SUCCESS
        },
        Some(other) => {
            eprintln!("unknown command: {other}\nusage: amazing [version]");
            ExitCode::FAILURE
        },
    }
}
