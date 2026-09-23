//! Ported WPT html5lib tree-construction selection for HTML-19.
use amazing_dom::NodeIdx;
use amazing_html::{parse, parse_fragment};
use std::fs;

fn canonical_dump(dump: &str) -> String {
    let mut lines = dump.lines();
    if lines.next() != Some("#document") {
        return format!("unexpected dump: {dump:?}\n");
    }
    let mut result = String::new();
    let mut in_text = false;
    for line in lines {
        if in_text {
            result.push_str(line);
            result.push('\n');
            if line.matches('"').count() % 2 == 1 {
                in_text = false;
            }
            continue;
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        let content = line.trim_start_matches(' ');
        if content.starts_with("<!DOCTYPE") {
            continue;
        }
        let depth = (spaces / 2).saturating_sub(1);
        result.push_str(&format!("{depth}\t{content}\n"));
        if content.starts_with('"') && content.matches('"').count() % 2 == 1 {
            in_text = true;
        }
    }
    result
}

fn fnv(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn main() {
    let root = std::env::args().nth(1).expect("evidence directory");
    let details = std::env::args().any(|arg| arg == "--details");
    std::panic::set_hook(Box::new(|_| {}));
    let selection = fs::read_to_string(format!("{root}/selection.tsv")).unwrap();
    for row in selection.lines().skip(1) {
        let fields: Vec<_> = row.split('\t').collect();
        assert_eq!(fields.len(), 6);
        let source = fs::read_to_string(format!("{root}/{}", fields[1])).unwrap();
        let expected = fs::read_to_string(format!("{root}/{}", fields[2])).unwrap();
        let parsed = std::panic::catch_unwind(|| {
            if fields[3] == "document" {
                Ok(parse(&source))
            } else {
                parse_fragment(fields[3], &source)
            }
        });
        let (status, actual) = match parsed {
            Ok(Ok(dom)) => {
                let actual = canonical_dump(&dom.dump(NodeIdx(0)));
                (if actual == expected { "pass" } else { "fail" }, actual)
            },
            Ok(Err(error)) => ("unsupported", error),
            Err(_) => ("panic", "parser panic".to_owned()),
        };
        println!("{}\t{}\t{}", fields[0], status, fnv(actual.as_bytes()));
        if details && status != "pass" {
            eprintln!("FAIL {}\nEXPECTED:\n{}ACTUAL:\n{}", fields[0], expected, actual);
        }
    }
}
