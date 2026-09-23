//! Ported declarative-shadow parser probes from pinned WPT resources.
use amazing_dom::shadow::{ShadowMode, ShadowOptions};
use amazing_dom::{Dom, NodeIdx};
use amazing_html::{parse, parse_fragment};
use std::collections::BTreeMap;
use std::fs;

fn by_id(dom: &Dom, id: &str) -> NodeIdx {
    (0..dom.node_count()).map(|i| NodeIdx(i as u32))
        .find(|&node| dom.get_attribute_ns(node, None, "id") == Some(id)).unwrap()
}
fn markup(data: Vec<u16>) -> String { String::from_utf16(&data).unwrap() }
fn fnv(data: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in data.bytes() { hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3); }
    format!("{hash:016x}")
}
fn observe(context: &str, source: &str) -> BTreeMap<String, String> {
    let mut dom = if context == "fragment-div" {
        parse_fragment("div", source).unwrap()
    } else { parse(source) };
    let host = by_id(&dom, "host");
    let root = dom.shadow_root_of_host(host);
    let mut values = BTreeMap::new();
    values.insert("root".into(), if root.is_some() { "yes" } else { "no" }.into());
    values.insert("template_count".into(), dom.children(host)
        .filter(|&node| dom.name(node) == Some("template")).count().to_string());
    values.insert("light".into(), markup(dom.serialize_children(host).unwrap()));
    values.insert("serialized".into(),
        markup(dom.serialize_children_with_shadow(host, true, &[]).unwrap()));
    if let Some(root) = root {
        let data = dom.shadow_data(root).unwrap();
        let options = data.options;
        values.insert("mode".into(), match options.mode {
            ShadowMode::Open => "open", ShadowMode::Closed => "closed",
        }.into());
        values.insert("declarative".into(), data.declarative.to_string());
        values.insert("internals".into(), data.available_to_element_internals.to_string());
        values.insert("registry".into(), data.keep_custom_element_registry_null.to_string());
        values.insert("serializable".into(), options.serializable.to_string());
        values.insert("delegates_focus".into(), options.delegates_focus.to_string());
        values.insert("visible".into(), if dom.visible_shadow_root(host).is_some() { "yes" } else { "no" }.into());
        values.insert("shadow".into(), markup(dom.serialize_children(root).unwrap()));
        let opposite = ShadowOptions { mode: if options.mode == ShadowMode::Open {
            ShadowMode::Closed
        } else { ShadowMode::Open }, ..options };
        values.insert("attach_opposite".into(), if dom.attach_shadow(host, opposite).is_err() {
            "rejected"
        } else { "accepted" }.into());
        let same = dom.attach_shadow(host, options);
        values.insert("attach_same".into(), if same == Ok(root)
            && dom.children(root).next().is_none() { "cleared" } else { "rejected" }.into());
    }
    values
}
fn main() {
    let directory = std::env::args().nth(1).expect("evidence directory");
    let selection = fs::read_to_string(format!("{directory}/selection.tsv")).unwrap();
    for row in selection.lines().skip(1) {
        let columns: Vec<_> = row.split('\t').collect();
        assert_eq!(columns.len(), 5);
        let source = fs::read_to_string(format!("{directory}/{}", columns[1])).unwrap();
        let expected = fs::read_to_string(format!("{directory}/{}", columns[2])).unwrap();
        let values = observe(columns[3], &source);
        let pass = expected.lines().all(|line| {
            let (key, wanted) = line.split_once('\t').unwrap();
            values.get(key).is_some_and(|actual| actual == wanted)
        });
        let actual: String = values.iter().map(|(key, value)| format!("{key}\t{value}\n")).collect();
        if std::env::var_os("HTML20_DETAILS").is_some() && !pass {
            eprintln!("{}\nEXPECTED:\n{}ACTUAL:\n{}", columns[0], expected, actual);
        }
        println!("{}\t{}\t{}", columns[0], if pass { "pass" } else { "fail" }, fnv(&actual));
    }
}
