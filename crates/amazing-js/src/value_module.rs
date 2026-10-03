// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Rooted module namespace triggers and internal Promise continuations.

/// Strong ownership keeps the registry, realm and live cells alive while a
/// namespace is reachable. Invoke a cloned callback outside all object borrows.
pub struct DeferredNamespaceState {
    pub exports: std::collections::BTreeMap<String, crate::env::BindingCell>,
    pub evaluate: Rc<dyn Fn(&mut crate::interp::Interpreter) -> Result<(), Throw>>,
}
impl fmt::Debug for DeferredNamespaceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeferredNamespaceState")
    }
}
pub struct ModuleCallbackState {
    pub callback: Rc<dyn Fn(&mut crate::interp::Interpreter, JsValue) -> Result<(), Throw>>,
}
impl fmt::Debug for ModuleCallbackState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ModuleCallbackState")
    }
}
