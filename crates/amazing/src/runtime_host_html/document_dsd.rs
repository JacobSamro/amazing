//! Actual destination Document declarative-shadow-root parser policy.
//! HTML opening-the-input-stream and HTML parsing methods use this state independently
//! of scripting. Inert APIs default false; unsafe parsing explicitly chooses true.
use super::*;

impl RuntimeBindingHost<'_> {
    pub(crate) fn html_create_inert_document(&mut self, bytes: &[u8], content_type: &str,
        url: &str, charset: Option<&str>, allow_shadow_roots: bool) -> Result<NodeIdx, DomError> {
        self.current_document_for_nodes().map_err(|_| DomError::TypeError)?;
        let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let (parsed, detected_encoding) = if mime == "text/html" {
            let decoded = amazing_html::encoding::decode(bytes, charset);
            (amazing_html::parse_with_document_policy(&decoded.text, amazing_html::ScriptingMode::Disabled, allow_shadow_roots), decoded.encoding)
        } else if matches!(mime.as_str(), "text/xml" | "application/xml" | "application/xhtml+xml" | "image/svg+xml")
            || mime.ends_with("+xml") {
            let (source, encoding) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                (String::from_utf8(bytes[3..].to_vec()).map_err(|_| DomError::SyntaxError)?,
                    amazing_html::encoding::Encoding::Utf8)
            } else if !bytes.starts_with(&[0xFE, 0xFF]) && !bytes.starts_with(&[0xFF, 0xFE])
                && charset.is_none_or(|label| label.eq_ignore_ascii_case("UTF-8")) {
                (String::from_utf8(bytes.to_vec()).map_err(|_| DomError::SyntaxError)?,
                    amazing_html::encoding::Encoding::Utf8)
            } else {
                let decoded = amazing_html::encoding::decode(bytes, charset);
                if charset.is_some() && !matches!(decoded.source,
                    amazing_html::encoding::Source::Transport | amazing_html::encoding::Source::Bom) {
                    return Err(DomError::SyntaxError);
                }
                (decoded.text, decoded.encoding)
            };
            (amazing_html::parse_xml(&source).map_err(|_| DomError::SyntaxError)?, encoding)
        } else { return Err(DomError::NotSupportedError); };
        let character_set = amazing_html::encoding::canonical_name(detected_encoding);
        let host = self.state.borrow().host.clone();
        let document = host.borrow_mut().dom.import_inert_document(&parsed, url, &mime,
            character_set)?;
        self.dom_register_document_realm(document, self.js.binding_current_realm())
            .map_err(|_| DomError::TypeError)?;
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};

    #[test]
    fn inert_unsafe_clone_and_reopened_documents_use_actual_shadow_policy() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><body></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/shadow-policy");
        assert!(host.borrow().dom.document_allows_declarative_shadow_roots(NodeIdx(0)).unwrap());
        assert_eq!(scheduler.run_script(r#"
            (function() {
                let checkpoint="ordinary parser";
                try {
                const markup='<div id="host"><template shadowrootmode="open"><span>shadow</span></template></div>';
                const ordinary=new DOMParser().parseFromString(markup,'text/html');
                checkpoint="unsafe parser";
                const unsafe=Document.parseHTMLUnsafe(markup);
                checkpoint="fresh document";
                const fresh=document.implementation.createHTMLDocument('fresh');
                checkpoint="unsafe clone";
                const cloned=unsafe.cloneNode(true);
                checkpoint="ordinary clone";
                const ordinaryClone=ordinary.cloneNode(true);
                const docs=[ordinary,unsafe,fresh,cloned,ordinaryClone];
                const expected=[false,true,false,true,false];
                for(let i=0;i<docs.length;i++) {
                    checkpoint="open "+i; docs[i].open();
                    checkpoint="write "+i; docs[i].write(markup);
                    checkpoint="close "+i; docs[i].close();
                    const element=docs[i].getElementById('host');
                    if(!element || Boolean(element.shadowRoot)!==expected[i]) return false;
                    if(!expected[i] && element.querySelector('template')===null) return false;
                }
                return true;
                } catch(error) { return checkpoint+": "+error.name+": "+error.message; }
            })()
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn byte_response_inert_default_and_explicit_unsafe_policy_precede_parse() {
        let host=Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><body></body>"), native_forwarded:false,
        }));
        let mut js=Interpreter::new();
        js.set_dom_host(host.clone());
        let bindings=crate::runtime_binding_host::install(&mut js,host.clone()).unwrap();
        bindings.set_document_url(&mut js,"https://example.test/root");
        let markup=b"<div><template shadowrootmode=open><span>shadow</span></template></div>";
        let inert=bindings.create_inert_document(&mut js,markup,"text/html","https://example.test/response",None).unwrap();
        let unsafe_doc={
            let mut adapter=RuntimeBindingHost {js:&mut js,state:bindings.state.clone()};
            adapter.html_create_inert_document(markup,"text/html","https://example.test/unsafe",None,true).unwrap()
        };
        let borrowed=host.borrow();
        let dom=&borrowed.dom;
        for (document,allowed,url) in [(inert,false,"https://example.test/response"),(unsafe_doc,true,"https://example.test/unsafe")] {
            assert_eq!(dom.document_allows_declarative_shadow_roots(document).unwrap(),allowed);
            assert!(!dom.document_scripting_enabled(document).unwrap());
            assert_eq!(dom.document_metadata(document).unwrap().unwrap().url,url);
            let element=dom.tree_order(document).into_iter().find(|&node| dom.name(node)==Some("div")).unwrap();
            assert_eq!(dom.shadow_root_of_host(element).is_some(),allowed);
        }
    }

    #[test]
    fn sandboxed_navigation_shadow_policy_is_independent_of_scripts() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<!doctype html><iframe id='child' sandbox='allow-same-origin' srcdoc='<div id=h><template shadowrootmode=open><b>shadow</b></template></div>'></iframe>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/shadow-policy");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script(r#"
            (function() {
                const doc=document.getElementById('child').contentDocument;
                if(!doc.getElementById('h').shadowRoot) return false;
                doc.open(); doc.write('<div id=h><template shadowrootmode=open><b>shadow</b></template></div>'); doc.close();
                return Boolean(doc.getElementById('h').shadowRoot);
            })()
        "#).unwrap(), JsValue::Bool(true));
        let state=scheduler.test_bindings().state.borrow();
        let document=state.frames.documents.values().find_map(|entry| entry.document).unwrap();
        assert!(!host.borrow().dom.document_scripting_enabled(document).unwrap());
        assert!(host.borrow().dom.document_allows_declarative_shadow_roots(document).unwrap());
    }
}
