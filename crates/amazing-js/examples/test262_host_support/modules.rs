//! Test262 filesystem transport with independently instantiated realm registries.
use super::*;
use amazing_js::interp::RealmHandle;
use amazing_js::module::{LoadedModule, ModuleFailure, ModuleLinker, ModulePhase, ModuleRequest, ModuleResolver, ModuleSource};
use std::{collections::HashMap, path::{Path, PathBuf}};

#[derive(Clone)]
pub(super) struct Test262Resolver {
    root: PathBuf,
    entry: String,
}
impl ModuleResolver for Test262Resolver {
    fn resolve_request(&self, request: &ModuleRequest, referrer: &str) -> Result<LoadedModule, String> {
        // This installed filesystem host supplies JavaScript records only,
        // never a module-source object or a typed JSON/text/bytes record.
        // Source-phase JavaScript still resolves the genuine record: the core
        // rejects its empty ModuleSource with the required SyntaxError.
        // Unsupported attributes are a separate host admission TypeError.
        if !request.attributes.is_empty() {
            return Err("Test262 JavaScript host does not support this typed/source request".into());
        }
        let (id, source) = self.resolve(&request.specifier.to_utf8_lossy(), referrer)?;
        Ok(LoadedModule { id, source: ModuleSource::JavaScript(source) })
    }
    fn resolve_request_error(&self, request: &ModuleRequest, _: &str, message: &str) -> Option<Throw> {
        (!request.attributes.is_empty())
            .then(|| Throw::type_error(message))
    }
    fn resolve(&self, specifier: &str, referrer: &str) -> Result<(String, String), String> {
        let referrer = if referrer.is_empty() || referrer == self.entry { self.root.join(&self.entry) }
            else { Path::new(referrer).to_path_buf() };
        let candidate = referrer.parent().unwrap_or(&self.root).join(specifier);
        let canonical = candidate.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(&self.root) {
            return Err(format!("module specifier escapes Test262 checkout: {specifier}"));
        }
        let source = std::fs::read_to_string(&canonical).map_err(|error| error.to_string())?;
        Ok((canonical.to_string_lossy().into_owned(), source))
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
    let root_id = root.join(path).canonicalize()?.to_string_lossy().into_owned();
    if !Path::new(&root_id).starts_with(&root) {
        return Err(io::Error::other("root module escapes Test262 checkout"));
    }
    let runtime = Rc::new(Test262ModuleRuntime {
        resolver: Test262Resolver { root, entry: path.to_owned() },
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

