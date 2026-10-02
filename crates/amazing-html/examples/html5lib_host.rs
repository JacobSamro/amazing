//! html5lib tree-construction probe, including explicit script-mode variants.
use std::io::{self, Read};
use amazing_dom::{Namespace, NodeIdx};

fn main() {
    let mut args = std::env::args().skip(1);
    let scripting = match args.next().as_deref() {
        Some("enabled") => amazing_html::ScriptingMode::Enabled,
        Some("disabled") => amazing_html::ScriptingMode::Disabled,
        _ => panic!("enabled or disabled script mode"),
    };
    let context = args.next();
    let mut source = String::new();
    io::stdin().read_to_string(&mut source).expect("decoded HTML input");
    let dom = match context {
        Some(context) => {
            let (namespace, name) = match context.split_once(' ') {
                Some(("svg", name)) => (Namespace::Svg, name),
                Some(("math", name)) => (Namespace::MathMl, name),
                _ => (Namespace::Html, context.as_str()),
            };
            amazing_html::parse_fragment_with_scripting_mode(name, namespace, &source, scripting).expect("fragment context")
        },
        None => amazing_html::parse_with_scripting_mode(&source, scripting),
    };
    print!("{}", dom.dump(NodeIdx(0)));
}
