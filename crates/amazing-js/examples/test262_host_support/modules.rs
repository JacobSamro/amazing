//! Test262 filesystem transport with independently instantiated realm registries.
use super::*;
use amazing_js::interp::RealmHandle;
use amazing_js::module::{LoadedModule, ModuleFailure, ModuleLinker, ModulePhase, ModuleRequest, ModuleResolver, ModuleSource};
use std::{collections::HashMap, path::{Path, PathBuf}};

#[derive(Clone)]
pub(super) struct Test262Resolver {
    root: PathBuf,
    entry: String,
    normal_requests: RefCell<HashMap<(String, amazing_js::value::JsString, Vec<(amazing_js::value::JsString, amazing_js::value::JsString)>), LoadedModule>>,
}
impl ModuleResolver for Test262Resolver {
    fn cache_loaded_requests(&self) -> bool { true }
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String> {
        let canonical = self.canonical_path(specifier, referrer)?;
        let source = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
        Ok((Self::path_id(&canonical)?, source))
    }
    fn resolve_request(&self, request: &ModuleRequest, referrer: &str) -> Result<LoadedModule, String> {
        let kind = match request.attributes.as_slice() {
            [] => "javascript",
            [(key, value)] if key == &"type" => {
                if value == &"json" { "json" } else if value == &"text" { "text" }
                else if value == &"bytes" { "bytes" }
                else { return Err("unsupported module type".into()); }
            },
            _ => return Err("unsupported import attribute".into()),
        };
        let key = (referrer.to_owned(), request.specifier.clone(), request.attributes.clone());
        if let Some(loaded) = self.normal_requests.borrow().get(&key) { return Ok(loaded.clone()); }
        // Host filesystem paths are Unicode scalar strings. Reject unrepresentable
        // requests; never coalesce lone-surrogate specifiers through replacement.
        let specifier = String::from_utf16(request.specifier.units()).map_err(|_| "module path has unpaired UTF16 units")?;
        let canonical = self.canonical_path(&specifier, referrer)?;
        let bytes = std::fs::read(&canonical).map_err(|error| error.to_string())?;
        let path = Self::path_id(&canonical)?;
        let id = if kind == "javascript" { path } else {
            format!("\0data-module:{}:{}:{}", path.len(), path, kind)
        };
        let source = match kind {
            "bytes" => ModuleSource::Bytes(bytes),
            "text" => ModuleSource::Text(Self::decode_utf8(&bytes).into()),
            "json" => ModuleSource::JsonText(Self::decode_utf8(&bytes).into()),
            _ => ModuleSource::JavaScript(Self::decode_utf8(&bytes)),
        };
        let loaded = LoadedModule { id, source };
        self.normal_requests.borrow_mut().insert(key, loaded.clone());
        Ok(loaded)
    }
    fn resolve_request_error(&self, _request: &ModuleRequest, _referrer: &str, message: &str) -> Option<Throw> {
        Some(Throw::type_error(message))
    }
}
impl Test262Resolver {
    fn decode_utf8(bytes: &[u8]) -> String {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        String::from_utf8_lossy(bytes).into_owned()
    }
    fn path_id(path: &Path) -> Result<String, String> {
        path.to_str().map(str::to_owned).ok_or_else(|| "module canonical path is not Unicode".into())
    }
    fn canonical_path(&self, specifier: &str, referrer: &str) -> Result<PathBuf, String> {
        let referrer = if referrer.is_empty() || referrer == self.entry { self.root.join(&self.entry) }
            else { Path::new(referrer).to_path_buf() };
        let candidate = referrer.parent().unwrap_or(&self.root).join(specifier);
        let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(&self.root) {
            return Err(format!("module specifier escapes Test262 checkout: {specifier}"));
        }
        Ok(canonical)
    }
}

pub(super) type Registry = Rc<RefCell<ModuleLinker<Test262Resolver>>>;
struct Test262ModuleRuntime {
    resolver: Test262Resolver,
    realms: RefCell<HashMap<RealmHandle, Registry>>,
}
impl Test262ModuleRuntime {
    fn registry(&self, i: &Interpreter) -> Registry {
        self.realms.borrow_mut().entry(i.binding_current_realm()).or_insert_with(||
            Rc::new(RefCell::new(ModuleLinker::new(self.resolver.clone())))
        ).clone()
    }
    fn failure(&self, i: &Interpreter, failure: ModuleFailure) -> Throw {
        let Some(original) = failure.thrown else {
            // Transport/unsupported-request failures have no cached module
            // error value. Each request obtains its own realm TypeError.
            return i.binding_normalize_throw(match failure.phase {
                ModulePhase::Parse => Throw::syntax_error(failure.message),
                _ => Throw::type_error(failure.message),
            });
        };
        // Core normalization attaches the active realm prototype in place,
        // preserving this linker-cached Throw and arbitrary user error identity.
        i.binding_normalize_throw(original)
    }
}

pub(super) fn install_test262_module_loader(
    i: &mut Interpreter, root: &str, path: &str,
) -> io::Result<(Registry, String)> {
    let root = Path::new(root).canonicalize()?;
    let root_id = Test262Resolver::path_id(&root.join(path).canonicalize()?).map_err(io::Error::other)?;
    if !Path::new(&root_id).starts_with(&root) {
        return Err(io::Error::other("root module escapes Test262 checkout"));
    }
    let runtime = Rc::new(Test262ModuleRuntime {
        resolver: Test262Resolver { root, entry: path.to_owned(), normal_requests: RefCell::new(HashMap::new()) },
        realms: RefCell::new(HashMap::new()),
    });
    // Static root execution and subsequent imports share this exact root entry.
    let linker = runtime.registry(i);
    i.binding_set_phase_module_loader_hook(Rc::new(move |i, request, referrer| {
        let linker = runtime.registry(i);
        // Release the registry-map borrow before parsing, evaluation, jobs or
        // any user code. importValue uses this same genuine typed host hook.
        ModuleLinker::import_shared(&linker, request, referrer, i)
            .map_err(|failure| runtime.failure(i, failure))
    }));
    Ok((linker, root_id))
}
