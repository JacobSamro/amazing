use amazing_dom::{Dom, Namespace, NodeIdx};
use amazing_html::{parse, parse_fragment};
use std::fs;

fn html(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}
fn visit(dom: &Dom, node: NodeIdx, found: &mut Vec<NodeIdx>) {
    for child in dom.children(node) {
        if dom.namespace(child) == Some(Namespace::Html) && dom.name(child) == Some("template") {
            found.push(child);
            if let Some(contents) = dom.template_contents(child) { visit(dom, contents, found); }
        } else { visit(dom, child, found); }
    }
}
fn fnv(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes { h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3); }
    format!("{h:016x}")
}
fn main() {
    let base = std::env::args().nth(1).expect("evidence directory");
    for line in include_str!("../evidence/html16/selection.tsv").lines().skip(1) {
        let cols: Vec<_> = line.split('\t').collect();
        assert_eq!(cols.len(), 6);
        let src = fs::read_to_string(format!("{base}/{}", cols[1])).unwrap();
        let result = if cols[2] == "document" { Ok(parse(&src)) } else { parse_fragment(cols[2], &src) };
        match result {
            Err(_) => println!("{}\tunsupported\t-", cols[0]),
            Ok(dom) => {
                let mut templates = Vec::new();
                visit(&dom, NodeIdx(0), &mut templates);
                let output = html(&dom, NodeIdx(0));
                let first = templates.first().and_then(|&t| dom.template_contents(t));
                let content = first.map(|f| html(&dom, f)).unwrap_or_default();
                let expected_count: usize = cols[3].parse().unwrap();
                let content_ok = cols[4] == "EMPTY" && content.is_empty()
                    || cols[4] == "WHITESPACE" && content.trim().is_empty()
                    || content.contains(cols[4]);
                let light_ok = templates.iter().all(|&t| dom.children(t).next().is_none());
                let pass = templates.len() == expected_count && content_ok && light_ok && output.contains(cols[5]);
                let classification = if pass { "pass" } else { "fail" };
                if !pass { eprintln!("{}: count={} content={content:?} output={output:?} light={light_ok}", cols[0], templates.len()); }
                let signature = format!("{}\n{}\n{}\n{}", templates.len(), content, output, light_ok);
                println!("{}\t{}\t{}", cols[0], classification, fnv(signature.as_bytes()));
            }
        }
    }
}
