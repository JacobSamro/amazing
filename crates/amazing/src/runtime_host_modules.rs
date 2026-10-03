//! Browser-owned source-text module registries and transport adapters.
use super::*;
use amazing_js::module::{ModuleFailure, ModuleLinker, ModulePhase, ModuleResolver};
use amazing_js::value::JsObject;

pub(crate) type ModuleSourceLoader = dyn Fn(&str) -> Result<String, String>;
type Registry = Rc<RefCell<ModuleLinker<BrowserModuleResolver>>>;
type SourceCache = Rc<RefCell<HashMap<String, Result<String, String>>>>;

#[derive(Clone)]
pub(super) struct ModuleRuntime {
    sources: Rc<RefCell<Rc<ModuleSourceLoader>>>,
    registries: Rc<RefCell<HashMap<RealmHandle, Registry>>>,
    inline_urls: Rc<RefCell<HashMap<String, String>>>,
    caches: Rc<RefCell<HashMap<RealmHandle, SourceCache>>>,
    failures: Rc<RefCell<HashMap<(RealmHandle, String), Throw>>>,
    current_scripts: Rc<RefCell<HashMap<NodeIdx, Option<NodeIdx>>>>,
    fetch_failures: Rc<RefCell<HashSet<(RealmHandle, String)>>>,
    bases: Rc<RefCell<HashMap<RealmHandle, String>>>,
}

#[derive(Clone)]
struct BrowserModuleResolver {
    sources: Rc<RefCell<Rc<ModuleSourceLoader>>>,
    inline_urls: Rc<RefCell<HashMap<String, String>>>,
    fallback: String,
    cache: Rc<RefCell<HashMap<String, Result<String, String>>>>,
}

impl BrowserModuleResolver {
    fn url(&self, id: &str) -> String {
        self.inline_urls.borrow().get(id).cloned().unwrap_or_else(|| id.to_owned())
    }
    fn locate(&self, specifier: &str, referrer: &str) -> Result<String, String> {
        // Import maps are a separate driver feature. Bare names must not
        // silently turn into relative filesystem or network paths.
        if !specifier.starts_with('/') && !specifier.starts_with("./")
            && !specifier.starts_with("../")
            && amazing_net::url::ParsedUrl::parse(specifier).is_err()
        {
            return Err(format!("unmapped bare module specifier: {specifier}"));
        }
        let base = self.url(if referrer.is_empty() { &self.fallback } else { referrer });
        amazing_net::url::ParsedUrl::parse(&base)
            .and_then(|base| base.resolve(specifier))
            .map(|url| url.serialize()).map_err(|error| error.to_string())
    }
}

impl ModuleResolver for BrowserModuleResolver {
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String> {
        let id = self.locate(specifier, referrer)?;
        let cached = self.cache.borrow().get(&id).cloned();
        let source = if let Some(source) = cached { source } else {
            let loader = self.sources.borrow().clone();
            let source = loader(&id);
            self.cache.borrow_mut().insert(id.clone(), source.clone());
            source
        }?;
        Ok((id, source))
    }
    fn source_url(&self, id: &str) -> Option<String> { Some(self.url(id)) }
    fn resolve_request_error(&self, request: &amazing_js::module::ModuleRequest, referrer: &str, message: &str) -> Option<Throw> {
        self.locate(&request.specifier.to_utf8_lossy(), referrer).err()
            .map(|_| Throw::type_error(message))
    }
}

pub(crate) fn source_url(source: &str) -> String {
    if let Ok(url) = amazing_net::url::ParsedUrl::parse(source) { return url.serialize(); }
    let path = std::path::Path::new(source);
    let path = if path.is_absolute() { path.to_owned() }
        else { std::env::current_dir().unwrap_or_default().join(path) };
    // URL parsing removes dot segments and escapes spaces. Canonical module
    // identifiers must be independent of the path's spelling.
    let path = path.to_string_lossy().replace('\\', "/");
    let url = format!("file://{}{}", if path.starts_with('/') { "" } else { "/" }, path);
    amazing_net::url::ParsedUrl::parse(&url).map(|url| url.serialize()).unwrap_or(url)
}

pub(crate) fn decode_module(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)).into_owned()
}

pub(crate) fn fetch_module(url: &str) -> Result<String, String> {
    let canonical = source_url(url);
    let url = canonical.as_str();
    if let Ok(parsed) = amazing_net::url::ParsedUrl::parse(url) && parsed.scheme() == "data" {
        let response = amazing_fetch::process_data_url(&parsed).ok_or_else(|| "invalid module data URL".to_owned())?;
        if !amazing_html::is_javascript_mime_type(&response.mime_type.essence()) {
            return Err("module data URL is not JavaScript".into());
        }
        return Ok(decode_module(&response.body));
    }
    let response = amazing_net::fetch(url, &amazing_net::FetchOptions {
        max_response_bytes: 16 * 1024 * 1024, ..Default::default()
    }).map_err(|error| error.to_string())?;
    if !response.is_success() { return Err(format!("module fetch failed: {url}")); }
    if url.starts_with("http:") || url.starts_with("https:") {
        let mime = response.content_type().unwrap_or("").split(';').next().unwrap_or("").trim();
        if !crate::cmd::js::is_classic_javascript_type(Some(mime)) || mime.is_empty() {
            return Err(format!("module response is not JavaScript: {mime}"));
        }
    }
    Ok(decode_module(&response.body))
}

fn module_failure(error: ModuleFailure) -> Throw {
    if let Some(thrown) = error.thrown { return thrown; }
    match error.phase {
        ModulePhase::Parse => Throw::syntax_error(error.message),
        _ => Throw::type_error(error.message),
    }
}

impl ModuleRuntime {
    pub(super) fn new() -> Self {
        Self { sources: Rc::new(RefCell::new(Rc::new(fetch_module))),
            registries: Rc::new(RefCell::new(HashMap::new())),
            inline_urls: Rc::new(RefCell::new(HashMap::new())),
            caches: Rc::new(RefCell::new(HashMap::new())),
            failures: Rc::new(RefCell::new(HashMap::new())),
            current_scripts: Rc::new(RefCell::new(HashMap::new())),
            fetch_failures: Rc::new(RefCell::new(HashSet::new())),
            bases: Rc::new(RefCell::new(HashMap::new())) }
    }
    fn resolver(&self, js: &Interpreter) -> BrowserModuleResolver {
        let cache = self.caches.borrow_mut().entry(js.binding_current_realm())
            .or_insert_with(|| Rc::new(RefCell::new(HashMap::new()))).clone();
        BrowserModuleResolver { sources: self.sources.clone(), inline_urls: self.inline_urls.clone(),
            fallback: self.bases.borrow().get(&js.binding_current_realm()).cloned()
                .unwrap_or_else(|| source_url(js.binding_location_href())), cache }
    }
    fn registry(&self, js: &Interpreter) -> Registry {
        let realm = js.binding_current_realm();
        self.registries.borrow_mut().entry(realm).or_insert_with(|| {
            Rc::new(RefCell::new(ModuleLinker::new(self.resolver(js))))
        }).clone()
    }
    fn evaluate(&self, js: &mut Interpreter, id: &str, source: &str) -> Result<(JsValue, Option<JsValue>), Throw> {
        let key = (js.binding_current_realm(), id.to_owned());
        if let Some(error) = self.failures.borrow().get(&key).cloned() { return Err(error); }
        let registry = self.registry(js);
        self.resolver(js).cache.borrow_mut().entry(id.to_owned()).or_insert_with(|| Ok(source.to_owned()));
        let result = (|| {
            let linked = registry.borrow_mut().link(id, source);
            if linked.as_ref().is_err_and(|error| error.phase == ModulePhase::Resolution && error.thrown.is_none()) {
                self.fetch_failures.borrow_mut().insert(key.clone());
            }
            linked.map_err(module_failure)?;
            let completion = ModuleLinker::evaluate_async_shared(&registry, id, js).map_err(module_failure)?;
            let pending = match js.binding_module_evaluation_state(&completion)? {
                amazing_js::value::PromiseFate::Rejected(reason) => return Err(Throw(reason)),
                amazing_js::value::PromiseFate::Pending => Some(completion),
                amazing_js::value::PromiseFate::Fulfilled(_) => None,
            };
            let namespace = registry.borrow().namespace_object(id);
            namespace.map(|namespace| (namespace, pending))
                .ok_or_else(|| Throw::type_error("module namespace missing"))
        })();
        if let Err(error) = &result { self.failures.borrow_mut().insert(key, js.binding_normalize_throw(error.clone())); }
        result
    }
    fn install(&self, js: &mut Interpreter) {
        let runtime = self.clone();
        // Dynamic imports retain their phase/attributes and settle through the
        // engine's evaluation capability. A pending top-level await is ordinary
        // progress, never a synchronous failure cached by the page-script path.
        js.binding_set_phase_module_loader_hook(Rc::new(move |js, request, referrer| {
            let registry = runtime.registry(js);
            // Evaluation imports share their canonical module's cached error.
            // Cache a realm Error object, rather than a native marker that would
            // materialize a different object every time the job rejects.
            let key = if request.phase == amazing_js::module::ImportPhase::Evaluation
                && request.attributes.is_empty()
            {
                runtime.resolver(js).locate(&request.specifier.to_utf8_lossy(), referrer)
                    .ok().map(|id| (js.binding_current_realm(), id))
            } else { None };
            if let Some(key) = &key {
                if let Some(error) = runtime.failures.borrow().get(key).cloned() {
                    return Err(error);
                }
            }
            ModuleLinker::import_shared(&registry, request, referrer, js).map_err(|failure| {
                let has_module_error = failure.thrown.is_some();
                let error = js.binding_normalize_throw(module_failure(failure));
                if has_module_error && let Some(key) = key { runtime.failures.borrow_mut().insert(key, error.clone()); }
                error
            })
        }));
    }
}

impl RuntimeBindings {
    pub(crate) fn page_script_url(&self, js: &Interpreter, script: &crate::cmd::js::PageScript, page: &str) -> String {
        let runtime = self.state.borrow().modules.clone();
        let mut base = if matches!(page, "about:blank" | "about:srcdoc") { runtime.resolver(js).fallback } else { source_url(page) };
        let href = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            let document = if (script.node.0 as usize) < host.dom.node_count() { host.dom.root(script.node) } else { NodeIdx(0) };
            host.dom.document_base_href(document).ok().flatten()
        };
        if let Some(href) = href && let Ok(url) = amazing_net::url::ParsedUrl::parse(&base)
            .and_then(|url| url.resolve(&String::from_utf16_lossy(&href))) { base = url.serialize(); }
        script.src.as_deref().map(|src| source_url(&crate::cmd::js::resolve_script_url(&base, src))).unwrap_or(base)
    }
    pub(crate) fn finish_module_script(&self, js: &mut Interpreter, script: &crate::cmd::js::PageScript,
        error: Option<&Throw>, url: &str, fetched: bool) -> Result<(), Throw> {
        let fetched = fetched && !self.module_graph_fetch_failed(js, script, url);
        if fetched && let Some(error) = error { self.report_module_exception(js, error, url)?; }
        self.dispatch_module_script_event(js, script.node, if fetched { "load" } else { "error" })
    }
    pub(crate) fn execute_document_script(&self, js: &mut Interpreter, script: &crate::cmd::js::PageScript,
        source: &str, url: &str) -> Result<(), Throw> {
        let result = self.execute_page_script(js, script, source, url);
        if script.kind == crate::cmd::js::ScriptKind::Module {
            self.finish_module_script(js, script, result.as_ref().err(), url, true)
        } else { result.map(|_| ()) }
    }
    pub(crate) fn set_module_document_base(&self, js: &Interpreter, url: &str) {
        self.state.borrow().modules.bases.borrow_mut().insert(js.binding_current_realm(), source_url(url));
    }
    pub(crate) fn module_graph_fetch_failed(&self, js: &Interpreter, script: &crate::cmd::js::PageScript, url: &str) -> bool {
        let id = if script.src.is_some() { source_url(url) } else {
            format!("amazing:inline-module:{:?}:{}", js.binding_current_realm(), script.node.0)
        };
        self.state.borrow().modules.fetch_failures.borrow().contains(&(js.binding_current_realm(), id))
    }
    pub(crate) fn report_module_exception(&self, js: &mut Interpreter, error: &Throw, url: &str) -> Result<(), Throw> {
        let global = JsValue::Object(js.global_obj.clone());
        if let JsValue::Object(object) = &error.0 {
            let name = {
                let object = object.borrow();
                if matches!(object.exotic, Exotic::Error) && object.proto.is_none() {
                    object.get_own("name")
                } else { None }
            };
            if let Some(JsValue::Str(name)) = name {
                let constructor = js.binding_get_property(&global, &name.to_utf8_lossy())?;
                if let JsValue::Object(constructor) = constructor {
                    let prototype = constructor.borrow().get_own("prototype");
                    if let Some(JsValue::Object(prototype)) = prototype { object.borrow_mut().proto = Some(prototype); }
                }
            }
        }
        let constructor = js.binding_get_property(&global, "ErrorEvent")?;
        let mut init = JsObject::ordinary();
        init.set_own("error", error.0.clone());
        init.set_own("message", JsValue::str(error.describe()));
        init.set_own("filename", JsValue::str(url));
        let event = js.binding_construct(&constructor, vec![JsValue::str("error"), JsValue::object(init)])?;
        let dispatch = js.binding_get_property(&global, "dispatchEvent")?;
        js.binding_call(&dispatch, global, vec![event]).map(|_| ())
    }
    pub(crate) fn dispatch_module_script_event(&self, js: &mut Interpreter, node: NodeIdx, name: &str) -> Result<(), Throw> {
        js.dispatch_dom_event(node.0, name)?;
        let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
        amazing_bindings::dispatch_user_agent_event(&mut adapter,
            amazing_dom::events::EventTarget::Node(node), name,
            amazing_dom::events::EventInit { bubbles: false, cancelable: false, composed: false })
    }
    pub(crate) fn load_module_source(&self, js: &Interpreter, url: &str) -> Result<String, String> {
        let runtime = self.state.borrow().modules.clone();
        let url = source_url(url);
        runtime.resolver(js).resolve(&url, &url).map(|(_, source)| source)
    }
    pub(crate) fn set_module_source_loader(&self, loader: Rc<ModuleSourceLoader>) {
        let runtime = self.state.borrow().modules.clone();
        *runtime.sources.borrow_mut() = loader;
    }
    pub(crate) fn execute_page_script(&self, js: &mut Interpreter,
        script: &crate::cmd::js::PageScript, source: &str, url: &str) -> Result<JsValue, Throw> {
        let runtime = self.state.borrow().modules.clone();
        let url = if matches!(url, "about:blank" | "about:srcdoc") {
            runtime.resolver(js).fallback
        } else { source_url(url) };
        let document = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            ((script.node.0 as usize) < host.dom.node_count()).then(|| host.dom.root(script.node))
        };
        let previous = document.map(|document| {
            let current = if script.kind == crate::cmd::js::ScriptKind::Classic {
                let state = self.state.borrow();
                let current = (state.host.borrow().dom.name(script.node) == Some("script")).then_some(script.node);
                current
            } else { None };
            runtime.current_scripts.borrow_mut().insert(document, current).flatten()
        });
        if script.kind == crate::cmd::js::ScriptKind::Classic {
            let result = js.run_script_with_source_context(source, &url);
            if let Some(document) = document { runtime.current_scripts.borrow_mut().insert(document, previous.flatten()); }
            return result;
        }
        let id = if script.src.is_some() { url.clone() } else {
            let id = format!("amazing:inline-module:{:?}:{}", js.binding_current_realm(), script.node.0);
            runtime.inline_urls.borrow_mut().insert(id.clone(), url);
            id
        };
        let result = runtime.evaluate(js, &id, source).and_then(|(namespace, pending)| {
            if let Some(promise) = pending {
                let bindings = self.clone();
                let url = runtime.inline_urls.borrow().get(&id).cloned().unwrap_or_else(|| id.clone());
                // HTML run-a-module-script reports asynchronous rejection in
                // the registering realm. Script load and parser progress do
                // not wait for top-level await to settle.
                js.binding_observe_module_evaluation(&promise, Rc::new(move |js, result| {
                    if let Err(error) = result { bindings.report_module_exception(js, &error, &url)?; }
                    Ok(())
                }))?;
            }
            Ok(namespace)
        });
        if let Some(document) = document { runtime.current_scripts.borrow_mut().insert(document, previous.flatten()); }
        result
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_module_loader(&mut self) -> Result<(), Throw> {
        let runtime = self.state.borrow().modules.clone();
        runtime.install(self.js);
        if let Some(prototype) = self.prototype(Interface::Document) {
            let getter = self.native("get currentScript", 0, NativeEntry::CurrentScript, HostCallPolicy::CallOnly);
            self.define_accessor_property(&prototype, "currentScript", Some(getter), None, true, true)?;
        }
        Ok(())
    }
    pub(super) fn module_current_script(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let key = self.platform_object(&receiver, InterfaceBrand::NODE).ok_or_else(|| Throw::type_error("Expected a Document"))?;
        let document = NodeIdx(key.slot as u32);
        let current = {
            let state = self.state.borrow();
            if !matches!(state.host.borrow().dom.get(document).kind, NodeKind::Document) {
                return Err(Throw::type_error("Expected a Document"));
            }
            let current = state.modules.current_scripts.borrow().get(&document).copied().flatten();
            current
        };
        match current { Some(node) => self.wrap_node(node), None => Ok(JsValue::Null) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::js::{PageScript, ScriptKind};

    fn scheduler() -> crate::js_scheduler::DriverScheduler {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/page.html");
        scheduler
    }
    fn script(kind: ScriptKind, node: u32, external: bool) -> PageScript {
        PageScript { kind, node: NodeIdx(node), src: external.then(|| "root.js".into()), body: String::new() }
    }
    #[test]
    fn static_and_delayed_imports_share_live_namespace_and_fetch_once() {
        let mut scheduler = scheduler();
        let fetches = Rc::new(RefCell::new(Vec::new()));
        let calls = fetches.clone();
        scheduler.set_module_source_loader(Rc::new(move |url| {
            calls.borrow_mut().push(url.to_owned());
            match url {
                "https://example.test/lib/counter.js" => Ok("export let count=1; export function bump(){count++}".into()),
                _ => Err(format!("missing {url}")),
            }
        }));
        scheduler.run_page_script(&script(ScriptKind::Module, 1, true),
            "import * as ns from './counter.js'; window.ns=ns; ns.bump(); window.meta=import.meta.url;",
            "https://example.test/lib/root.js").unwrap();
        scheduler.run_page_script(&script(ScriptKind::Classic, 2, true),
            "var delayed=()=>import('./counter.js');", "https://example.test/lib/classic.js").unwrap();
        scheduler.run_page_script(&script(ScriptKind::Classic, 3, true),
            "delayed().then(x=>{window.same=x===ns;window.count=x.count;});", "https://example.test/other/script.js").unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(scheduler.run_script("same && count===2 && meta==='https://example.test/lib/root.js'").unwrap(), JsValue::Bool(true)));
        assert_eq!(&*fetches.borrow(), &["https://example.test/lib/counter.js"]);
    }
    #[test]
    fn inline_modules_have_distinct_lexicals_and_document_url_metadata() {
        let mut scheduler = scheduler();
        for (node, value) in [(1, "first"), (2, "second")] {
            scheduler.run_page_script(&script(ScriptKind::Module, node, false),
                &format!("let privateName='{value}';window.{value}=privateName;window.url{node}=import.meta.url;"),
                "https://example.test/page.html").unwrap();
        }
        assert!(matches!(scheduler.run_script("first==='first' && second==='second' && typeof privateName==='undefined' && url1===url2 && url1==='https://example.test/page.html'").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn dynamic_import_preserves_cached_thrown_object_and_rejects_bare_names() {
        let mut scheduler = scheduler();
        scheduler.set_module_source_loader(Rc::new(|_| Ok("throw window.original;".into())));
        scheduler.run_script("window.original={marker:1};var errors=[];import('./fail.js').catch(x=>errors.push(x));import('./fail.js').catch(x=>errors.push(x));import('unmapped').catch(x=>window.bare=x instanceof TypeError);").unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(scheduler.run_script("errors.length===2 && errors[0]===original && errors[1]===original && bare").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn browser_dynamic_import_waits_for_async_graph_and_preserves_namespace_identity() {
        let mut driver = scheduler();
        driver.set_module_source_loader(Rc::new(|url| match url {
            "https://example.test/async.js" => Ok("await new Promise(resolve=>setTimeout(resolve,1)); export const value=7;".into()),
            _ => Err(format!("missing {url}")),
        }));
        driver.run_script("var namespaces=[];import('./async.js').then(ns=>namespaces.push(ns));import('./async.js').then(ns=>namespaces.push(ns));").unwrap();
        assert!(matches!(driver.run_script("namespaces.length===0").unwrap(), JsValue::Bool(true)));
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("namespaces.length===0").unwrap(), JsValue::Bool(true)));
        driver.pump_at(std::time::Duration::from_millis(1), false).unwrap();
        assert!(matches!(driver.run_script("namespaces.length===2 && namespaces[0]===namespaces[1] && namespaces[0].value===7").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn browser_phase_import_keeps_deferred_body_lazy_and_source_dependencies_unfetched() {
        let mut driver = scheduler();
        let fetched = Rc::new(RefCell::new(Vec::new()));
        let requests = fetched.clone();
        driver.set_module_source_loader(Rc::new(move |url| {
            requests.borrow_mut().push(url.to_owned());
            match url {
                "https://example.test/deferred.js" => Ok("window.deferredRuns=(window.deferredRuns||0)+1; export const value=9;".into()),
                "https://example.test/source.js" => Ok("import './missing.js'; export default 1;".into()),
                _ => Err(format!("missing {url}")),
            }
        }));
        driver.run_script("var lazy,sourceError,attributeError;import.defer('./deferred.js').then(ns=>lazy=ns);import.source('./source.js').catch(e=>sourceError=e);import('./attributes.js',{with:{type:'json'}}).catch(e=>attributeError=e);").unwrap();
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("typeof deferredRuns==='undefined' && sourceError instanceof SyntaxError && attributeError instanceof TypeError").unwrap(), JsValue::Bool(true)));
        assert_eq!(&*fetched.borrow(), &["https://example.test/deferred.js", "https://example.test/source.js"]);
        assert!(matches!(driver.run_script("lazy.value===9 && deferredRuns===1").unwrap(), JsValue::Bool(true)));
        driver.run_script("var evaluated;import('./deferred.js').then(ns=>evaluated=ns);").unwrap();
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("evaluated.value===9 && evaluated!==lazy && deferredRuns===1").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn host_lexical_overlays_keep_script_globals_connected_to_window() {
        let mut driver = scheduler();
        driver.run_script("var marker={};function fn(){return marker;}let privateMarker=3;").unwrap();
        assert!(matches!(driver.run_script("window.marker===marker && window.fn===fn && window.fn()===marker && !Object.prototype.hasOwnProperty.call(window,'privateMarker')").unwrap(), JsValue::Bool(true)));
        driver.run_script("window.marker=17;").unwrap();
        assert!(matches!(driver.run_script("marker===17 && fn()===17 && privateMarker===3").unwrap(), JsValue::Bool(true)));
        driver.run_script("marker=19;").unwrap();
        assert!(matches!(driver.run_script("window.marker===19").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn pending_page_modules_resume_without_blocking_load_and_share_import_namespace() {
        let mut driver = scheduler();
        driver.run_script("window.order=[];").unwrap();
        let module = script(ScriptKind::Module, 0, true);
        driver.run_page_script(&module,
            "order.push('start');await new Promise(resolve=>setTimeout(resolve,1));order.push('end');export const value=7;",
            "https://example.test/root.js").unwrap();
        driver.finish_module_script(&module, None, "https://example.test/root.js", true).unwrap();
        driver.run_script("var ns;import('./root.js').then(value=>ns=value);order.push('next');").unwrap();
        assert!(matches!(driver.run_script("order.join(',')==='start,next' && typeof ns==='undefined' && document.currentScript===null").unwrap(), JsValue::Bool(true)));
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        driver.pump_at(std::time::Duration::from_millis(1), false).unwrap();
        assert!(matches!(driver.run_script("order.join(',')==='start,next,end' && ns.value===7 && document.currentScript===null").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn pending_page_module_errors_report_original_value_after_external_load() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<script id=root></script>"), native_forwarded: false }));
        let node = host.borrow_mut().query_selector("script").unwrap();
        let mut driver = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        driver.set_location_url("https://example.test/page.html");
        driver.run_script("window.original={};var errors=[],events=[];window.addEventListener('error',e=>{errors.push(e.error);events.push('error');});document.getElementById('root').addEventListener('load',()=>events.push('load'));").unwrap();
        let module = script(ScriptKind::Module, node, true);
        driver.run_page_script(&module, "await new Promise(resolve=>setTimeout(resolve,1));throw window.original;", "https://example.test/root.js").unwrap();
        driver.finish_module_script(&module, None, "https://example.test/root.js", true).unwrap();
        assert!(matches!(driver.run_script("events.join(',')==='load' && errors.length===0").unwrap(), JsValue::Bool(true)));
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        driver.pump_at(std::time::Duration::from_millis(1), false).unwrap();
        assert!(matches!(driver.run_script("events.join(',')==='load,error' && errors.length===1 && errors[0]===window.original").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn browser_dynamic_import_caches_materialized_dependency_errors() {
        let mut driver = scheduler();
        driver.set_module_source_loader(Rc::new(|url| match url {
            "https://example.test/bad.js" => Ok("import 'unmapped';".into()),
            _ => Err(format!("missing {url}")),
        }));
        driver.run_script("var errors=[];import('./bad.js').catch(e=>errors.push(e));import('./bad.js').catch(e=>errors.push(e));").unwrap();
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(driver.run_script("errors.length===2 && errors[0]===errors[1] && errors[0] instanceof TypeError").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn browser_async_import_rejections_keep_the_original_error_across_requests() {
        let mut driver = scheduler();
        driver.set_module_source_loader(Rc::new(|_| Ok("await Promise.resolve(); throw window.original;".into())));
        driver.run_script("var original={},errors=[];window.original=original;import('./reject.js').catch(e=>errors.push(e));import('./reject.js').catch(e=>errors.push(e));").unwrap();
        assert!(matches!(driver.run_script("window.original===original").unwrap(), JsValue::Bool(true)));
        driver.pump_at(std::time::Duration::ZERO, false).unwrap();
        let actual = driver.run_script("JSON.stringify([errors.length,errors.map(e=>e===original),errors.map(e=>String(e))])").unwrap();
        assert!(matches!(driver.run_script("errors.length===2 && errors[0]===original && errors[1]===original").unwrap(), JsValue::Bool(true)), "async import rejection identity: {actual:?}");
    }
    #[test]
    fn module_text_is_always_utf8_and_bom_is_removed() {
        assert_eq!(decode_module(&[0xef,0xbb,0xbf,b'a',0xe9]), "a\u{fffd}");
        assert_eq!(source_url("https://example.test/a/../b.js"), "https://example.test/b.js");
        assert_eq!(fetch_module("data:text/javascript;base64,ZXhwb3J0IGRlZmF1bHQgMTs=").unwrap(), "export default 1;");
        assert!(fetch_module("data:text/plain,export%20default%201").is_err());
    }
    #[test]
    fn parser_defers_module_until_following_classic_script_and_skips_nomodule() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: Dom::new(), native_forwarded: false }));
        let mut scheduler = crate::cmd::js::prepare_page_scripts(&host, "https://example.test/page.html",
            "<script>var order=[];window.addEventListener('DOMContentLoaded',()=>order.push('ready'));</script><script type=module>order.push('module');</script><script nomodule>throw 'fallback';</script><script>order.push('classic');</script>", true).unwrap().unwrap();
        assert!(matches!(scheduler.run_script("order.join(',')==='classic,module,ready'").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn reentrant_dynamic_import_during_module_evaluation_releases_registry_borrows() {
        let mut scheduler = scheduler();
        scheduler.set_module_source_loader(Rc::new(|url| match url {
            "https://example.test/child.js" => Ok("export const value=7;".into()),
            _ => Err(format!("missing {url}")),
        }));
        scheduler.run_page_script(&script(ScriptKind::Module, 1, true),
            "window.loaded=import('./child.js').then(ns=>window.result=ns.value);", "https://example.test/root.js").unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert!(matches!(scheduler.run_script("result===7").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn page_module_errors_report_original_object_and_fire_load_without_aborting() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<script id=root></script>"), native_forwarded: false }));
        let node = host.borrow_mut().query_selector("script").unwrap();
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/page.html");
        scheduler.run_script("var original={},errors=[],events=[];window.addEventListener('error',e=>errors.push(e.error));document.getElementById('root').addEventListener('load',()=>events.push('load'));").unwrap();
        let module = script(ScriptKind::Module, node, true);
        for _ in 0..2 {
            let error = scheduler.run_page_script(&module, "throw original", "https://example.test/root.js").unwrap_err();
            scheduler.finish_module_script(&module, Some(&error), "https://example.test/root.js", true).unwrap();
        }
        assert!(matches!(scheduler.run_script("errors.length===2 && errors[0]===original && errors[1]===original && events.join(',')==='load,load'").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn current_script_tracks_classic_elements_and_restores_null_after_module_execution() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<script id=root></script>"), native_forwarded: false }));
        let node = host.borrow_mut().query_selector("script").unwrap();
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.run_page_script(&script(ScriptKind::Classic, node, false), "window.classic=document.currentScript.id;", "https://example.test/page.html").unwrap();
        scheduler.run_page_script(&script(ScriptKind::Module, node, false), "window.moduleCurrent=document.currentScript;", "https://example.test/page.html").unwrap();
        assert!(matches!(scheduler.run_script("classic==='root' && moduleCurrent===null && document.currentScript===null").unwrap(), JsValue::Bool(true)));
    }
    #[test]
    fn iframe_registries_are_distinct_and_srcdoc_inherits_parent_module_base() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<iframe srcdoc=\"<script type='module'>import './counter.js';window.moduleURL=import.meta.url;</script>\"></iframe>"),
            native_forwarded: false,
        }));
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        let fetches = Rc::new(RefCell::new(0));
        let calls = fetches.clone();
        scheduler.set_module_source_loader(Rc::new(move |url| {
            assert_eq!(url, "https://example.test/counter.js");
            *calls.borrow_mut() += 1;
            Ok("window.calls=(window.calls||0)+1;export const count=window.calls;".into())
        }));
        scheduler.set_location_url("https://example.test/page.html");
        scheduler.run_page_script(&script(ScriptKind::Module, 1000, true),
            "import './counter.js';", "https://example.test/root.js").unwrap();
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert!(matches!(scheduler.run_script("var child=document.querySelector('iframe').contentWindow;calls===1 && child.calls===1 && child.moduleURL==='https://example.test/page.html' && child.document.currentScript===null").unwrap(), JsValue::Bool(true)));
        assert_eq!(*fetches.borrow(), 2);
    }
    #[test]
    fn inline_source_url_captures_document_base_before_later_base_changes() {
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<base href='/alpha/'><script id=root></script>"), native_forwarded: false,
        }));
        let node = host.borrow_mut().query_selector("script").unwrap();
        let mut scheduler = crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/page.html");
        scheduler.set_module_source_loader(Rc::new(|url| {
            assert_eq!(url, "https://example.test/alpha/dep.js");
            Ok("export const value=3".into())
        }));
        let inline = script(ScriptKind::Classic, node, false);
        let url = scheduler.page_script_url("https://example.test/page.html", &inline);
        scheduler.run_page_script(&inline, "var delayed=()=>import('./dep.js');", &url).unwrap();
        scheduler.run_script("document.querySelector('base').setAttribute('href','/beta/');delayed().then(ns=>window.value=ns.value)").unwrap();
        scheduler.pump_at(std::time::Duration::ZERO, false).unwrap();
        assert_eq!(scheduler.run_script("value").unwrap(), JsValue::num(3.0));
        assert_eq!(scheduler.page_script_url("https://example.test/page.html", &inline), "https://example.test/beta/");
    }
}
