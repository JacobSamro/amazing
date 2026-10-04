//! Decoded-input html5lib probe. Execution uses the installed parser/scheduler,
//! without testharness nodes or fixture-specific tree mutations.
use std::{cell::RefCell, io::{self, Read}, process::ExitCode, rc::Rc};
use amazing_dom::{Dom, Namespace, NodeIdx};
use amazing_html::ScriptingMode;
use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
use super::js::{run_interleaved_parse, ScriptKind};

const PAGE_URL: &str = "https://html5lib.test/tree";

fn static_tree(source: &str, mode: ScriptingMode, context: Option<&str>) -> Result<Dom, String> {
    let Some(context) = context else {
        return Ok(amazing_html::parse_with_scripting_mode(source, mode));
    };
    let (namespace, name) = match context.split_once(' ') {
        Some(("svg", name)) => (Namespace::Svg, name),
        Some(("math", name)) => (Namespace::MathMl, name),
        _ => (Namespace::Html, context),
    };
    amazing_html::parse_fragment_with_scripting_mode(name, namespace, source, mode)
}

fn executed_tree(source: &str) -> Result<(Dom, Vec<String>), String> {
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
    let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY)
        .map_err(|error| error.describe())?;
    scheduler.set_location_url(PAGE_URL);
    // The probe has no external resource embedding. Nested written Classic
    // scripts and iframe navigations must not silently fall back to transport.
    scheduler.set_frame_resource_loader(Rc::new(|_| Err("external resources are unsupported by the tree probe".into())));
    scheduler.set_module_source_loader(Rc::new(|_| Err("modules are unsupported by the tree probe".into())));
    let mut errors = Vec::new();
    let deferred = run_interleaved_parse(&host, &mut scheduler, source, |scheduler, script| {
        if script.kind != ScriptKind::Classic || script.src.is_some() {
            errors.push("only inline blocking Classic scripts are supported by the tree probe".into());
        } else if let Err(error) = scheduler.run_page_script(script, &script.body, PAGE_URL) {
            errors.push(error.describe());
        }
        // Script exceptions do not stop the HTML parser, but remain a failed
        // probe observation. The real arena is swapped back before resuming.
        Ok(())
    })?;
    if !deferred.is_empty() {
        errors.push("deferred, async and module scripts are unsupported by the tree probe".into());
    }
    let dom = std::mem::take(&mut host.borrow_mut().dom);
    Ok((dom, errors))
}

pub(crate) fn cmd_html5lib_tree(args: &[String]) -> ExitCode {
    let mode = match args.first().map(String::as_str) {
        Some("enabled") => ScriptingMode::Enabled,
        Some("disabled") => ScriptingMode::Disabled,
        _ => { eprintln!("html5lib-tree: expected enabled or disabled"); return ExitCode::FAILURE; },
    };
    let execute = match args.get(1).map(String::as_str) {
        Some("static") => false,
        Some("execute") if mode == ScriptingMode::Enabled && args.len() == 2 => true,
        _ => { eprintln!("html5lib-tree: expected static [context], or enabled execute document"); return ExitCode::FAILURE; },
    };
    if args.len() > 3 { eprintln!("html5lib-tree: too many arguments"); return ExitCode::FAILURE; }
    let mut source = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut source) {
        eprintln!("html5lib-tree: decoded UTF-8 input: {error}");
        return ExitCode::FAILURE;
    }
    let result = if execute { executed_tree(&source) }
        else { static_tree(&source, mode, args.get(2).map(String::as_str)).map(|dom| (dom, Vec::new())) };
    match result {
        Ok((dom, errors)) => {
            print!("{}", dom.dump(NodeIdx(0)));
            for error in &errors { eprintln!("html5lib-tree: {error}"); }
            if errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        },
        Err(error) => { eprintln!("html5lib-tree: {error}"); ExitCode::FAILURE },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_tree_uses_real_nested_writes_and_live_formatting_mutations() {
        let source = r#"<p><b id=A><script>document.getElementById('A').id='B';document.write('<script>document.write("W")</scr'+'ipt>')</script></p>T</b>"#;
        let (dom, errors) = executed_tree(source).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let dump = dom.dump(NodeIdx(0));
        assert!(dump.contains("<b id=\"B\">"), "{dump}");
        assert!(dump.contains("<b id=\"A\">"), "{dump}");
        assert!(dump.contains("\"W\""), "{dump}");
        assert!(dump.contains("\"T\""), "{dump}");
    }

    #[test]
    fn script_errors_remain_failures_and_parser_continues_to_real_eof() {
        let (dom, errors) = executed_tree("<script>throw new Error('actual failure')</script><p id=tail>tail").unwrap();
        assert_eq!(errors.len(), 1);
        assert!(dom.dump(NodeIdx(0)).contains("<p id=\"tail\">"));
        let (_, errors) = executed_tree("<script src='https://invalid.test/never-fetch'></script><script type=module>0</script>").unwrap();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn static_mode_and_foreign_fragment_are_actual_core_inputs() {
        let enabled = static_tree("<noscript><b>X</b></noscript>", ScriptingMode::Enabled, Some("body")).unwrap();
        let disabled = static_tree("<noscript><b>X</b></noscript>", ScriptingMode::Disabled, Some("body")).unwrap();
        assert!(!enabled.tree_order(NodeIdx(0)).into_iter().any(|node| enabled.name(node) == Some("b")));
        assert!(disabled.tree_order(NodeIdx(0)).into_iter().any(|node| disabled.name(node) == Some("b")));
        let foreign = static_tree("<circle/>", ScriptingMode::Enabled, Some("svg svg")).unwrap();
        assert!(foreign.tree_order(NodeIdx(0)).into_iter().any(|node|
            foreign.local_name(node) == Some("circle") && foreign.namespace(node) == Some(Namespace::Svg)));
    }
}
