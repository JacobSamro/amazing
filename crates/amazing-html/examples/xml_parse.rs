//! Runnable XML parser artifact. Strict UTF-8 file input; never executes scripts.
use std::{env, fs, process::ExitCode};
fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: xml_parse <document.xml|document.xhtml>");
        return ExitCode::FAILURE;
    };
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match amazing_html::parse_xml(&source) {
        Ok(dom) => {
            print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{path}: {error}");
            ExitCode::FAILURE
        }
    }
}
