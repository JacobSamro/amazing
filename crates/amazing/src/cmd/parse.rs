//! HTML parsing commands and shared source input.
use std::io::Read;
use std::process::ExitCode;
/// Reads document bytes without choosing an encoding before document mode.
pub(crate) fn read_bytes(path: Option<&str>, what: &str) -> Result<Vec<u8>, ExitCode> {
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
    Ok(bytes)
}

/// Caller-decoded text for scripts, stylesheets, and HTML fragments.
pub(crate) fn read_source(path: Option<&str>, what: &str) -> Result<String, ExitCode> {
    read_bytes(path, what).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// Phase 1 demo artifact: parse HTML with the WHATWG tokenizer + tree
/// builder, dump the DOM tree to stdout.
///
pub(crate) fn cmd_parse(path: Option<&str>) -> ExitCode {
    match read_bytes(path, "parse") {
        Ok(bytes) => {
            match crate::document::parse_document_bytes(&bytes, path.unwrap_or(""), None) {
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

/// html5lib/WPT `#document-fragment` context lines are a bare local name for
/// an HTML context (`"div"`) or `"<namespace> <local name>"` for a foreign
/// one (`"svg title"`, `"math ms"`) -- split that convention here so this
/// CLI can drive `parse_fragment_with_namespace` for both, rather than only
/// ever parsing in the HTML namespace.
fn split_fragment_context(context: &str) -> (amazing_dom::Namespace, &str) {
    match context.split_once(' ') {
        Some(("svg", name)) => (amazing_dom::Namespace::Svg, name),
        Some(("math", name)) => (amazing_dom::Namespace::MathMl, name),
        _ => (amazing_dom::Namespace::Html, context),
    }
}

pub(crate) fn cmd_parse_fragment(context: &str, path: Option<&str>) -> ExitCode {
    let (namespace, local_context) = split_fragment_context(context);
    match read_source(path, "parse-fragment") {
        Ok(src) => match amazing_html::parse_fragment_with_namespace(local_context, namespace, &src) {
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
