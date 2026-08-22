//! amazing — browser driver binary.
//!
//! Phase 0 artifact: builds and runs, prints the component pipeline status.
//! Each phase replaces one line of "not started" with a real capability
//! plus its demo subcommand (e.g. `amazing parse <file>` in Phase 1).

use std::io::Read;
use std::process::ExitCode;

const PIPELINE: &[(&str, &str)] = &[
    ("amazing-common", "byte streams + text utils"),
    (
        "amazing-html",
        "Phase 1: WHATWG tokenizer/tree builder — `parse` subcommand",
    ),
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
        Some("parse") => cmd_parse(args.get(2).map(String::as_str)),
        Some(other) => {
            eprintln!("unknown command: {other}\nusage: amazing [version|parse [file]]");
            ExitCode::FAILURE
        },
    }
}

/// Phase 1 demo artifact: read HTML from a file (or stdin), parse it with
/// the WHATWG tokenizer + tree builder, dump the DOM tree to stdout.
///
/// Bytes are decoded as lossy UTF-8 (documented Phase 1 deviation).
fn cmd_parse(path: Option<&str>) -> ExitCode {
    let mut bytes = Vec::new();
    match path {
        Some(p) => match std::fs::File::open(p) {
            Ok(mut f) => {
                if let Err(e) = f.read_to_end(&mut bytes) {
                    eprintln!("amazing parse: cannot read {p}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            Err(e) => {
                eprintln!("amazing parse: cannot open {p}: {e}");
                return ExitCode::FAILURE;
            },
        },
        None => {
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("amazing parse: cannot read stdin: {e}");
                return ExitCode::FAILURE;
            }
        },
    }

    let src = String::from_utf8_lossy(&bytes);
    let dom = amazing_html::parse(&src);
    print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
    ExitCode::SUCCESS
}
