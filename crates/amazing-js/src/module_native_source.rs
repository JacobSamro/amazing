// Host-defined compiled Module Source Objects (source-phase imports, §28.1).
// Original UTF-16 and the actual Module parse are retained together. This is a
// native data record, not a SourceTextModule's optional source representation.

/// Validated host module source. No JavaScript objects, realm environments or
/// callback captures are hidden in this record; allocation belongs to the linker realm.
#[derive(Debug)]
pub struct CompiledModuleSource {
    id: String,
    source: std::sync::Arc<[u16]>,
    module: Module,
}
impl CompiledModuleSource {
    pub fn compile(id: impl Into<String>, source: &[u16]) -> Result<Self, crate::errors::SyntaxError> {
        let module = crate::parse_module_utf16(source)?;
        Ok(Self { id: id.into(), source: source.into(), module })
    }
    pub fn id(&self) -> &str { &self.id }
    pub fn source_units(&self) -> &[u16] { &self.source }
    pub fn kind(&self) -> &'static str { "CompiledModuleSource" }
}
impl<R: ModuleResolver> ModuleLinker<R> {
    fn require_module_source(&self, id: &str) -> Result<(), ModuleFailure> {
        if self.compiled_sources.contains_key(id) { return Ok(()); }
        self.module_source(id).map(|_| ())
    }
    fn prepare_source_objects(&mut self, interpreter: &mut Interpreter) {
        for (id, record) in &self.compiled_sources {
            self.source_objects.entry(id.clone()).or_insert_with(||
                interpreter.binding_create_compiled_module_source(record.clone()));
        }
    }
}
