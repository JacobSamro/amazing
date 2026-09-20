//! HTML parsing commands and shared source input.
use std::io::Read;
use std::process::ExitCode;
/// Reads a file (or stdin when `path` is None) as lossy UTF-8.
pub(crate) fn read_source(path: Option<&str>, what: &str) -> Result<String, ExitCode> {
    let mut bytes = Vec::new();
    match path {
        Some(p) => match std::fs::File::open(p) {
            Ok(mut f) => {
                if let Err(e) = f.read_to_end(&mut bytes) {
                    eprintln!("amazing {what}: cannot read {p}: {e}");
                    return Err(ExitCode::FAILURE);
                }
            }
            Err(e) => {
                eprintln!("amazing {what}: cannot open {p}: {e}");
                return Err(ExitCode::FAILURE);
            }
        },
        None => {
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("amazing {what}: cannot read stdin: {e}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Phase 1 demo artifact: parse HTML with the WHATWG tokenizer + tree
/// builder, dump the DOM tree to stdout.
///
/// Bytes are decoded as lossy UTF-8 (documented Phase 1 deviation).
pub(crate) fn cmd_parse(path: Option<&str>) -> ExitCode {
    match read_source(path, "parse") {
        Ok(src) => {
            match crate::document::parse_document(&src, path.unwrap_or(""), None) {
                Ok(document) => {
                    print!("{}", document.dom.dump(amazing_dom::NodeIdx(0)));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("amazing parse: {error}");
                    error.exit_code()
                }
            }
        }
        Err(code) => code,
    }
}

pub(crate) fn cmd_parse_fragment(context: &str, path: Option<&str>) -> ExitCode {
    match read_source(path, "parse-fragment") {
        Ok(src) => match amazing_html::parse_fragment(context, &src) {
            Ok(dom) => {
                print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("amazing parse-fragment: {error}");
                ExitCode::FAILURE
            }
        },
        Err(code) => code,
    }
}
