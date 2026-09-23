//! Next bulk ported html5lib tree-construction probe. See evidence/html22/README.md.
use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};
use amazing_html::{parse, parse_fragment};
use std::{fs, panic};

const FILES: &[&str] = &[
    "foreign-fragment.dat", "tests2.dat", "tests4.dat", "tests10.dat", "tests16.dat",
];
const RETAINED_FILES: &[&str] = &[
    "adoption01.dat", "blocks.dat", "comments01.dat", "tables01.dat",
    "tests1.dat", "tests6.dat", "tests19.dat", "tests_innerHTML_1.dat", "webkit01.dat",
];

fn fnv(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Default)]
struct Case {
    data: String,
    expected: String,
    context: String,
    script_off: bool,
    script_on: bool,
}

fn cases(source: &str) -> Vec<Case> {
    let mut result = Vec::new();
    for block in source.split("#data\n").skip(1) {
        let mut case = Case::default();
        let mut section = "data";
        for line in block.split_inclusive('\n') {
            let marker = line.trim_end_matches('\n').trim_end_matches('\r');
            match marker {
                "#errors" | "#new-errors" => section = "errors",
                "#document" => section = "document",
                "#document-fragment" => section = "context",
                "#script-off" => case.script_off = true,
                "#script-on" => case.script_on = true,
                _ => match section {
                    "data" => case.data.push_str(line),
                    "document" => case.expected.push_str(line),
                    "context" => case.context.push_str(line),
                    _ => {},
                },
            }
        }
        // The final LF in a section is the html5lib separator, not input.
        case.data.pop();
        case.context = case.context.trim().to_owned();
        result.push(case);
    }
    result
}

#[derive(Debug)]
struct Entry {
    depth: usize,
    kind: String,
    value: String,
    attrs: Vec<String>,
}

fn encode(entries: &[Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let mut attrs = entry.attrs.clone();
        attrs.sort();
        out.push_str(&format!("{}\t{}\t{:?}\t{:?}\n", entry.depth, entry.kind, entry.value, attrs));
    }
    out
}

fn expected_tree(source: &str) -> Result<String, String> {
    let mut rows: Vec<(usize, String)> = Vec::new();
    for line in source.trim_end_matches('\n').lines() {
        if let Some(body) = line.strip_prefix("| ") {
            let indent = body.len() - body.trim_start_matches(' ').len();
            if indent % 2 != 0 { return Err("odd tree indentation".into()); }
            rows.push((indent / 2, body[indent..].to_owned()));
        } else if let Some((_, last)) = rows.last_mut() {
            last.push('\n');
            last.push_str(line);
        }
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (depth, raw) in rows {
        if !raw.starts_with('"') && raw.contains("=\"") && raw.ends_with('"') {
            if let Some(element) = entries.last_mut() {
                if element.kind == "element" && depth == element.depth + 1 {
                    element.attrs.push(raw);
                    continue;
                }
            }
        }
        let (kind, value) = if raw.starts_with("<!DOCTYPE ") {
            ("doctype", raw.trim_start_matches("<!DOCTYPE ").trim_end_matches('>'))
        } else if raw.starts_with("<?") && raw.ends_with('>') {
            ("processing-instruction", raw.trim_start_matches("<?").trim_end_matches('>'))
        } else if raw.starts_with("<!-- ") {
            ("comment", raw.trim_start_matches("<!-- ").trim_end_matches(" -->"))
        } else if raw.starts_with('"') && raw.ends_with('"') {
            ("text", &raw[1..raw.len() - 1])
        } else if raw.starts_with('<') && raw.ends_with('>') {
            ("element", &raw[1..raw.len() - 1])
        } else {
            return Err(format!("unsupported expected row: {raw:?}"));
        };
        entries.push(Entry { depth, kind: kind.into(), value: value.into(), attrs: Vec::new() });
    }
    Ok(encode(&entries))
}

fn walk(dom: &Dom, node: NodeIdx, depth: usize, entries: &mut Vec<Entry>) {
    let kind = &dom.get(node).kind;
    let (label, value, attrs) = match kind {
        NodeKind::Document | NodeKind::DocumentFragment => return,
        NodeKind::DocumentType { name, public_id, system_id } => {
            let mut value = name.clone();
            if !public_id.is_empty() || !system_id.is_empty() {
                value.push_str(&format!(" \"{public_id}\" \"{system_id}\""));
            }
            ("doctype", value, Vec::new())
        },
        NodeKind::ProcessingInstruction { target, data } => {
            ("processing-instruction", format!("{target} {data}"), Vec::new())
        },
        NodeKind::Element { name, namespace, attrs } => {
            let prefix = match namespace {
                Namespace::Html | Namespace::Null => "",
                Namespace::Svg => "svg ",
                Namespace::MathMl => "math ",
                Namespace::Other(_) => "other ",
            };
            ("element", format!("{prefix}{name}"), attrs.iter().map(|(k, v)| format!("{k}=\"{v}\"")).collect())
        },
        NodeKind::Text { data } => ("text", data.clone(), Vec::new()),
        NodeKind::Comment { data } => ("comment", data.clone(), Vec::new()),
        other => ("other", format!("{other:?}"), Vec::new()),
    };
    entries.push(Entry { depth, kind: label.into(), value, attrs });
    for child in dom.children(node) { walk(dom, child, depth + 1, entries); }
}

fn actual_tree(dom: &Dom) -> String {
    let mut entries = Vec::new();
    for child in dom.children(NodeIdx(0)) { walk(dom, child, 0, &mut entries); }
    encode(&entries)
}

fn main() {
    let root = std::env::args().nth(1).expect("evidence directory");
    let details = std::env::args().any(|arg| arg == "--details");
    let retained = std::env::args().any(|arg| arg == "--retained");
    panic::set_hook(Box::new(|_| {}));
    println!("source\tcase\tcontext\tstatus\traw_fnv64");
    for file in if retained { RETAINED_FILES } else { FILES } {
        let source = fs::read_to_string(format!("{root}/{file}")).expect("pinned data");
        for (index, case) in cases(&source).into_iter().enumerate() {
            let context = if case.context.is_empty() { "document" } else { &case.context };
            let expected = expected_tree(&case.expected);
            let result = if case.script_off {
                Ok(Err("scripting-disabled input mode".to_owned()))
            } else if case.script_on {
                Ok(Err("scripted mutation input mode".to_owned()))
            } else if case.context.contains(' ') {
                Ok(Err("namespaced fragment context".to_owned()))
            } else {
                panic::catch_unwind(|| {
                    if case.context.is_empty() { Ok(parse(&case.data)) }
                    else { parse_fragment(&case.context, &case.data) }
                })
            };
            let (status, raw) = match (expected, result) {
                (Ok(want), Ok(Ok(dom))) => {
                    let got = actual_tree(&dom);
                    (if want == got { "pass" } else { "fail" }, got)
                },
                (_, Err(_)) => ("crash", "parser panic".to_owned()),
                (Err(reason), _) | (_, Ok(Err(reason))) => ("unsupported", reason),
            };
            println!("{file}\t{}\t{context}\t{status}\t{}", index + 1, fnv(raw.as_bytes()));
            if details && status != "pass" {
                eprintln!("{file}:{} {context} {status}\nEXPECTED:\n{:?}\nACTUAL:\n{}", index + 1, expected_tree(&case.expected), raw);
            }
        }
    }
}
