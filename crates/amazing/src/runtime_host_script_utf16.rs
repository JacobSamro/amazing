//! Lossless DOM Script text handoff. Source remains code units through execution.
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_js::value::JsString;

/// Script child text content includes direct Text and CDATA children, in order.
/// Scalar display caches are never a source of JavaScript text.
pub(crate) fn script_child_text(dom: &Dom, node: NodeIdx) -> Option<JsString> {
    let mut units = Vec::new();
    for child in dom.children(node) {
        if matches!(
            &dom.get(child).kind,
            NodeKind::Text { .. } | NodeKind::CDATASection { .. }
        ) {
            units.extend(dom.character_data_utf16(child)?);
        }
    }
    Some(JsString::from_units(units))
}

/// Retain the existing collector's whitespace-only admission decision without
/// converting source to a scalar display cache. Lone surrogates are non-whitespace.
pub(crate) fn has_script_source(source: &JsString) -> bool {
    source
        .units()
        .iter()
        .any(|&unit| char::from_u32(u32::from(unit)).is_none_or(|c| !c.is_whitespace()))
}

/// Metadata of the newly installed native Script text accessors.
pub(crate) fn script_text_accessor(
    operation: amazing_bindings::host::Operation,
) -> Option<(&'static str, u8)> {
    match operation {
        amazing_bindings::host::Operation::ScriptText => Some(("get text", 0)),
        amazing_bindings::host::Operation::ScriptSetText => Some(("set text", 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cmd::js::{ScriptKind, script_source_from_node},
        dom_host::BrowserDomHost,
        js_scheduler::{DriverScheduler, JS_CAPACITY},
    };
    use amazing_js::value::JsValue;
    use std::{cell::RefCell, rc::Rc};

    fn setup() -> (Rc<RefCell<BrowserDomHost>>, DriverScheduler, NodeIdx) {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><body><script id=exact></script>"),
            native_forwarded: false,
        }));
        let node = host
            .borrow()
            .dom
            .tree_order(NodeIdx(0))
            .into_iter()
            .find(|&node| host.borrow().dom.name(node) == Some("script"))
            .unwrap();
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/scripts/page.html");
        (host, scheduler, node)
    }
    fn source(prefix: &str, payload: &[u16], suffix: &str) -> JsString {
        JsString::from_units(
            prefix
                .encode_utf16()
                .chain(payload.iter().copied())
                .chain(suffix.encode_utf16())
                .collect::<Vec<_>>(),
        )
    }
    fn install(host: &Rc<RefCell<BrowserDomHost>>, node: NodeIdx, source: &JsString) {
        let mut host = host.borrow_mut();
        let text = host.dom.create_text(node, "");
        host.dom
            .replace_character_data(text, 0, 0, source.units())
            .unwrap();
    }
    #[test]
    fn actual_dom_child_units_reach_classic_script_and_retained_function_source() {
        for payload in [
            vec![0xd800],
            vec![0xdc00],
            vec![0xd800, 0xdc00],
            vec![0xfffd],
            vec![0, 0xe000],
        ] {
            let (host, mut scheduler, node) = setup();
            let expected = source("function retained(){return '", &payload, "'}");
            let initial = source("window.exact='", &payload, "'; ");
            let units = JsString::from_units(
                initial
                    .units()
                    .iter()
                    .chain(expected.units())
                    .copied()
                    .collect::<Vec<_>>(),
            );
            install(&host, node, &units);
            let record = script_source_from_node(&host.borrow().dom, node).unwrap();
            assert_eq!(record.body.units(), units.units());
            scheduler
                .run_page_script(
                    &record,
                    &record.body,
                    "https://example.test/scripts/inline.js",
                )
                .unwrap();
            assert_eq!(
                scheduler.run_script("exact").unwrap(),
                JsValue::Str(JsString::from_units(payload.clone()))
            );
            assert_eq!(
                scheduler.run_script("retained.toString()").unwrap(),
                JsValue::Str(expected)
            );
            assert_eq!(
                scheduler
                    .run_script("document.currentScript===null")
                    .unwrap(),
                JsValue::Bool(true)
            );
        }
    }
    #[test]
    fn prepared_source_is_an_owned_unit_snapshot_and_later_dom_text_is_distinct() {
        let (host, mut scheduler, node) = setup();
        let original = source(
            "window.exact='",
            &[0xd800],
            "';window.observedCurrent=document.currentScript.id;",
        );
        install(&host, node, &original);
        let first = script_source_from_node(&host.borrow().dom, node).unwrap();
        let child = host.borrow().dom.children(node).next().unwrap();
        let next = source("window.exact='", &[0xdc00], "';");
        host.borrow_mut()
            .dom
            .replace_character_data(child, 0, original.len(), next.units())
            .unwrap();
        scheduler
            .run_page_script(&first, &first.body, "https://example.test/scripts/first.js")
            .unwrap();
        assert_eq!(
            scheduler.run_script("exact.charCodeAt(0)").unwrap(),
            JsValue::num(0xd800 as f64)
        );
        assert_eq!(
            scheduler.run_script("observedCurrent").unwrap(),
            JsValue::str("exact")
        );
        let second = script_source_from_node(&host.borrow().dom, node).unwrap();
        assert_eq!(second.body.units(), next.units());
        assert_eq!(first.body.units(), original.units());
    }
    #[test]
    fn installed_dynamic_text_content_and_character_data_preserve_units_and_once_execution() {
        let (_host, mut scheduler, _node) = setup();
        scheduler.run_script(r#"var runs=0;var s=document.createElement('script');
            s.textContent="runs++;window.dynamic='"+String.fromCharCode(0xd800)+"';";
            document.body.appendChild(s);s.remove();document.body.appendChild(s);
            var t=document.createElement('script');var n=document.createTextNode('');t.appendChild(n);
            n.data="runs++;window.fromData='"+String.fromCharCode(0xdc00)+"';";document.body.appendChild(t);"#).unwrap();
        assert_eq!(scheduler.run_script("runs===2 && dynamic.charCodeAt(0)===0xd800 && fromData.charCodeAt(0)===0xdc00 && document.currentScript===null").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn inline_javascript_module_preserves_original_units_and_registry_identity() {
        for payload in [
            vec![0xd800],
            vec![0xdc00],
            vec![0xfffd],
            vec![0xd800, 0xdc00],
        ] {
            let (host, mut scheduler, node) = setup();
            let units = source(
                "window.moduleExact='",
                &payload,
                "';window.moduleRuns=(window.moduleRuns||0)+1;window.moduleCurrent=document.currentScript;export const value=7;",
            );
            install(&host, node, &units);
            let mut record = script_source_from_node(&host.borrow().dom, node).unwrap();
            record.kind = ScriptKind::Module;
            scheduler
                .run_page_script(
                    &record,
                    &record.body,
                    "https://example.test/scripts/page.html",
                )
                .unwrap();
            scheduler
                .run_page_script(
                    &record,
                    &record.body,
                    "https://example.test/scripts/page.html",
                )
                .unwrap();
            assert_eq!(
                scheduler.run_script("moduleExact").unwrap(),
                JsValue::Str(JsString::from_units(payload))
            );
            assert_eq!(
                scheduler
                    .run_script("moduleRuns===1 && moduleCurrent===null")
                    .unwrap(),
                JsValue::Bool(true)
            );
        }
    }
    #[test]
    fn invalid_original_unit_script_throws_without_running_replacement_text() {
        let (host, mut scheduler, node) = setup();
        let units = source(
            "window.beforeInvalid=true;",
            &[0xd800],
            ";window.afterInvalid=true;",
        );
        install(&host, node, &units);
        let record = script_source_from_node(&host.borrow().dom, node).unwrap();
        let error = scheduler
            .run_page_script(&record, &record.body, "https://example.test/scripts/bad.js")
            .unwrap_err();
        assert_eq!(
            scheduler.js.binding_get_property(&error.0, "name").unwrap(),
            JsValue::str("SyntaxError")
        );
        assert_eq!(scheduler.run_script("typeof beforeInvalid==='undefined' && typeof afterInvalid==='undefined' && document.currentScript===null").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn xml_text_and_cdata_concatenate_exact_direct_units_without_comments_or_descendants() {
        for kind in ["text/javascript", "module"] {
            let mut dom = amazing_html::xml::parse_xml(&format!(
                "<script xmlns='http://www.w3.org/1999/xhtml' type='{kind}'>left<![CDATA[middle]]><!--ignore--><span>nested</span>right</script>"
            )).unwrap();
            let script = dom.children(NodeIdx(0)).next().unwrap();
            let cdata = dom
                .children(script)
                .find(|&node| matches!(dom.get(node).kind, NodeKind::CDATASection { .. }))
                .unwrap();
            dom.replace_character_data(cdata, 0, 6, &[0xd800, 0xdc00, 0xdc00])
                .unwrap();
            let record = script_source_from_node(&dom, script).unwrap();
            let expected = source("left", &[0xd800, 0xdc00, 0xdc00], "right");
            assert_eq!(record.body.units(), expected.units());
        }
    }
    #[test]
    fn actual_async_queue_owns_original_source_until_its_registered_task_runs() {
        let (host, mut scheduler, node) = setup();
        let original = source(
            "window.asyncExact='",
            &[0xd800],
            "';window.asyncRuns=(window.asyncRuns||0)+1;",
        );
        install(&host, node, &original);
        let record = script_source_from_node(&host.borrow().dom, node).unwrap();
        let session = scheduler.begin_parser_session(NodeIdx(0)).unwrap();
        let queued = crate::cmd::js::HtmlParserScript {
            script: record,
            mode: amazing_html::ScriptMode::Async,
            session,
        };
        scheduler
            .run_parser_script(&queued, &original, "https://example.test/scripts/async.js")
            .unwrap();
        let child = host.borrow().dom.children(node).next().unwrap();
        host.borrow_mut()
            .dom
            .replace_character_data(
                child,
                0,
                original.len(),
                "window.asyncExact='replacement';"
                    .encode_utf16()
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert_eq!(
            scheduler
                .run_script("asyncExact.charCodeAt(0)===0xd800 && asyncRuns===1")
                .unwrap(),
            JsValue::Bool(true)
        );
    }
    #[test]
    fn actual_external_decode_and_final_url_feed_the_same_unit_execution_gateway() {
        let (host, mut scheduler, node) = setup();
        let mut record = script_source_from_node(&host.borrow().dom, node).unwrap_or(
            crate::cmd::js::PageScript {
                kind: ScriptKind::Classic,
                node,
                node_generation: host.borrow().dom.node_generation(node),
                src: None,
                body: "".into(),
            },
        );
        record.src = Some("external.js".into());
        scheduler.set_frame_resource_loader(Rc::new(|request| {
            let mut headers = amazing_net::Headers::new();
            headers
                .append(
                    "content-type",
                    "text/javascript; charset=windows-1252".into(),
                )
                .unwrap();
            let mut body = b"window.externalExact='".to_vec();
            body.push(0x80);
            body.extend_from_slice(b"';window.externalCurrent=document.currentScript.id;");
            Ok(amazing_net::Response {
                url: "https://example.test/final/external.js".into(),
                status: 200,
                reason: "OK".into(),
                headers,
                body,
            })
        }));
        let (units, url) = scheduler
            .load_classic_page_script(&record, "https://example.test/scripts/page.html")
            .unwrap();
        assert_eq!(url, "https://example.test/final/external.js");
        assert!(units.units().contains(&0x20ac));
        scheduler.run_page_script(&record, &units, &url).unwrap();
        assert_eq!(scheduler.run_script("externalExact.charCodeAt(0)===0x20ac && externalCurrent==='exact' && document.currentScript===null").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn dynamic_child_document_script_keeps_its_actual_window_and_current_script_owner() {
        let (_host, mut scheduler, _node) = setup();
        scheduler.run_script("var f=document.createElement('iframe');f.srcdoc='<body></body>';document.body.appendChild(f);").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.run_script(r#"var d=f.contentDocument;var s=d.createElement('script');
            s.textContent="window.childExact='"+String.fromCharCode(0xd800)+"';window.childOwner=document.currentScript.ownerDocument===document;";
            d.body.appendChild(s);"#).unwrap();
        assert_eq!(scheduler.run_script("typeof childExact==='undefined' && f.contentWindow.childExact.charCodeAt(0)===0xd800 && f.contentWindow.childOwner && document.currentScript===null").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn installed_script_text_domstring_setter_and_child_only_getter_keep_exact_units() {
        let (_host, mut scheduler, _node) = setup();
        scheduler.run_script(r#"var s=document.createElement('script');var calls=0;
            s.text={toString(){calls++;return "window.byText='"+String.fromCharCode(0xd800)+"';window.textRuns=(window.textRuns||0)+1;"}};
            var captured=s.text;document.body.appendChild(s);s.remove();document.body.appendChild(s);
            var descriptor=Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype,'text');var getter=descriptor.get;
            var metadata=descriptor.enumerable && descriptor.configurable && getter.name==='get text' && getter.length===0 && descriptor.set.name==='set text' && descriptor.set.length===1;
            var invalid=0;for(var receiver of [{},document.createElement('div'),document.createElementNS('http://www.w3.org/2000/svg','script')]){
                try{getter.call(receiver)}catch(e){if(e instanceof TypeError)invalid++}}
            var child=document.createElement('span');child.textContent='excluded';s.appendChild(child);
            var t=document.createElement('script');t.type='application/json';t.text=null;var nullText=t.text;t.text=undefined;var undefinedText=t.text;
            t.text=String.fromCharCode(0xdc00,0,0xfffd);var exactText=t.text;"#).unwrap();
        assert_eq!(scheduler.run_script("metadata && calls===1 && textRuns===1 && byText.charCodeAt(0)===0xd800 && s.text===captured && invalid===3 && nullText==='null' && undefinedText==='undefined' && exactText.length===3 && exactText.charCodeAt(0)===0xdc00 && exactText.charCodeAt(1)===0 && exactText.charCodeAt(2)===0xfffd").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn script_text_setter_uses_original_target_document_across_author_conversion() {
        let (_host, mut scheduler, _node) = setup();
        scheduler.run_script("var f=document.createElement('iframe');f.srcdoc='<body></body>';document.body.appendChild(f);").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        scheduler.run_script(r#"var d=f.contentDocument,w=f.contentWindow,s=d.createElement('script');s.type='application/json';var originalTextPrototype=w.Text.prototype;
            var setter=Object.getOwnPropertyDescriptor(HTMLScriptElement.prototype,'text').set;
            setter.call(s,{toString(){f.remove();return String.fromCharCode(0xd800)}});
            var originalOwner=s.firstChild.ownerDocument===d;
            var originalPrototype=Object.getPrototypeOf(s.firstChild)===originalTextPrototype;
            var retained=s.text;"#).unwrap();
        assert_eq!(scheduler.run_script("originalOwner && originalPrototype && retained.charCodeAt(0)===0xd800 && s.ownerDocument===d").unwrap(), JsValue::Bool(true));
    }
    #[test]
    fn connected_script_text_preparation_runs_once_and_abrupt_conversion_does_not_replace() {
        let (_host, mut scheduler, _node) = setup();
        scheduler.run_script(r#"var s=document.createElement('script');document.body.appendChild(s);
            s.text="window.connectedText='"+String.fromCharCode(0xdc00)+"';window.connectedRuns=(window.connectedRuns||0)+1;";
            var before=s.text;var sentinel={};var caught=false;
            try{s.text={toString(){throw sentinel}}}catch(e){caught=e===sentinel}
            var unchanged=s.text===before;s.text='window.connectedRuns=99';"#).unwrap();
        assert_eq!(scheduler.run_script("connectedRuns===1 && connectedText.charCodeAt(0)===0xdc00 && caught && unchanged && document.currentScript===null").unwrap(), JsValue::Bool(true));
    }
}
