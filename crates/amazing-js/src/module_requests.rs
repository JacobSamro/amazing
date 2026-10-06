// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Module request phases and canonical attribute keys adapted to Amazing's host API.

/// The phase controls linking/evaluation, never the identity selected by HostLoadImportedModule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportPhase {
    Evaluation,
    Source,
    Defer,
}

/// Lossless static or dynamic import request. Attributes are sorted by their UTF-16 keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleRequest {
    pub specifier: crate::value::JsString,
    pub phase: ImportPhase,
    pub attributes: Vec<(crate::value::JsString, crate::value::JsString)>,
}
impl ModuleRequest {
    pub fn new(
        specifier: impl Into<crate::value::JsString>,
        phase: ImportPhase,
        mut attributes: Vec<(crate::value::JsString, crate::value::JsString)>,
    ) -> Self {
        attributes.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            specifier: specifier.into(),
            phase,
            attributes,
        }
    }
    pub fn attribute(&self, name: &str) -> Option<&crate::value::JsString> {
        self.attributes
            .iter()
            .find(|(key, _)| key == &name)
            .map(|(_, value)| value)
    }
    fn ast(
        specifier: &crate::value::JsString,
        phase: Option<&str>,
        attributes: &[crate::ast::ImportAttribute],
    ) -> Self {
        Self::new(
            specifier,
            match phase {
                Some("source") => ImportPhase::Source,
                Some("defer") => ImportPhase::Defer,
                _ => ImportPhase::Evaluation,
            },
            attributes
                .iter()
                .map(|a| (a.key.clone(), a.value.clone()))
                .collect(),
        )
    }
    // Length-prefixed UTF-16 prevents delimiter/specifier collisions. Phase is
    // an edge distinction, while the resolver's canonical ID is phase-independent.
    fn edge_key(&self) -> String {
        let mut key = format!("\0{:?}:{}:", self.phase, self.specifier.len());
        for unit in self.specifier.units() {
            key.push_str(&format!("{unit:04x}"));
        }
        for (name, value) in &self.attributes {
            key.push_str(&format!(":{}:", name.len()));
            for unit in name.units() {
                key.push_str(&format!("{unit:04x}"));
            }
            key.push_str(&format!(":{}:", value.len()));
            for unit in value.units() {
                key.push_str(&format!("{unit:04x}"));
            }
        }
        key
    }
}

/// A host chooses canonical identity including its module type. Reusing an ID
/// returns the same record across source/evaluation/defer requests.
#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub id: String,
    pub source: ModuleSource,
}
#[derive(Debug, Clone)]
pub enum ModuleSource {
    /// A genuine host-defined parsed source record; ordinary JavaScript stays source-less.
    CompiledSource(Rc<CompiledModuleSource>),
    JavaScript(String),
    /// Exact source code units supplied by a typed host, including lone surrogates.
    JavaScriptUtf16(Vec<u16>),
    /// Validated data-only JSON; objects are allocated once in the module realm.
    Json(crate::json_data::JsonDocument),
    /// Unparsed exact JSON text; validation is cached at canonical record loading.
    JsonText(crate::value::JsString),
    /// Decoded exact text source, without JavaScript evaluation.
    Text(crate::value::JsString),
    /// Original bytes; a genuine realm Uint8Array is allocated at instantiation.
    Bytes(Vec<u8>),
    /// Hosts parse JSON/text/bytes against their actual MIME/type policy, then
    /// supply the immutable synthetic default binding. No JavaScript parsing of JSON.
    Synthetic {
        default_export: JsValue,
        module_source: Option<JsValue>,
    },
}
fn import_key(import: &crate::ast::ImportDeclaration) -> String {
    ModuleRequest::ast(&import.from, import.phase.as_deref(), &import.attributes).edge_key()
}
fn export_key(from: &crate::value::JsString, attrs: &[crate::ast::ImportAttribute]) -> String {
    ModuleRequest::ast(from, None, attrs).edge_key()
}
fn item_request(item: &ModuleItem) -> Option<ModuleRequest> {
    match item {
        ModuleItem::Import(import) => Some(ModuleRequest::ast(
            &import.from,
            import.phase.as_deref(),
            &import.attributes,
        )),
        ModuleItem::ExportNamed {
            from: Some(from),
            attributes,
            ..
        }
        | ModuleItem::ExportAll {
            from, attributes, ..
        } => Some(ModuleRequest::ast(from, None, attributes)),
        _ => None,
    }
}

impl<R: ModuleResolver> ModuleLinker<R> {
    fn load_host_record(
        &mut self,
        loaded: LoadedModule,
        source_only: bool,
    ) -> Result<(), ModuleFailure> {
        let id = loaded.id;
        if let Some(failure) = self.cached_load_failure(&id, false) { return Err(failure); }
        if !self.records.contains_key(&id) {
            match loaded.source {
                ModuleSource::CompiledSource(source) => {
                    if source.id() != id {
                        return Err(ModuleFailure::syntax(ModulePhase::Resolution, "compiled module canonical identity mismatch"));
                    }
                    self.records.insert(id.clone(), ModuleRecord {
                        id: id.clone(), module: source.module.clone(), dependencies: BTreeMap::new(),
                    });
                    self.compiled_sources.insert(id.clone(), source);
                }
                ModuleSource::JavaScript(source) => {
                    self.load_text_record(&id, &source.encode_utf16().collect::<Vec<_>>(), false)?;
                }
                ModuleSource::JavaScriptUtf16(source) => {
                    self.load_text_record(&id, &source, false)?;
                }
                ModuleSource::JsonText(source) => {
                    let document = match crate::json_data::JsonDocument::parse_utf16(source.units()) {
                        Ok(document) => document,
                        Err(error) => {
                            let failure = ModuleFailure::syntax(ModulePhase::Resolution, error.to_string());
                            self.load_failures.insert(id, CachedLoadFailure { failure: failure.clone(), direct_parse: true });
                            return Err(failure);
                        }
                    };
                    self.insert_data_record(&id, ModuleData::Json(document));
                }
                ModuleSource::Json(document) => {
                    self.insert_data_record(&id, ModuleData::Json(document));
                }
                ModuleSource::Text(text) => {
                    self.insert_data_record(&id, ModuleData::Text(text));
                }
                ModuleSource::Bytes(bytes) => {
                    self.insert_data_record(&id, ModuleData::Bytes(bytes));
                }
                ModuleSource::Synthetic {
                    default_export,
                    module_source,
                } => {
                    self.synthetic
                        .insert(id.clone(), (default_export, module_source));
                    let module = crate::parse_module("").expect("empty module");
                    self.records.insert(
                        id.clone(),
                        ModuleRecord {
                            id: id.clone(),
                            module,
                            dependencies: BTreeMap::new(),
                        },
                    );
                }
            }
        }
        if !source_only {
            self.expand_record(&id)?;
        }
        Ok(())
    }
    fn module_source(&self, id: &str) -> Result<JsValue, ModuleFailure> {
        if let Some(source) = self.source_objects.get(id) { return Ok(source.clone()); }
        self.synthetic
            .get(id)
            .and_then(|(_, source)| source.clone())
            .ok_or_else(|| {
                ModuleFailure::syntax(
                    ModulePhase::Resolution,
                    "source-text modules do not expose a module source object",
                )
            })
    }
    // Static and dynamic callers share normal-result identity and source-only
    // expansion. A cycle can reuse the record before walking its dependencies.
    fn load_host_request_record(&mut self, request: &ModuleRequest, referrer: &str) -> Result<String, ModuleFailure> {
        let key = (referrer.to_owned(), request.edge_key());
        let memoize = self.resolver.cache_loaded_requests();
        let cached = if memoize { self.loaded_host_requests.get(&key) } else { None };
        let id = if let Some(id) = cached {
            id.clone()
        } else {
            let loaded = self.resolver.resolve_request(request, referrer)
                .map_err(|message| ModuleFailure {
                    thrown: self.resolver.resolve_request_error(request, referrer, &message),
                    phase: ModulePhase::Resolution,
                    message,
                })?;
            let id = loaded.id.clone();
            // Validate and register first, without expanding. Publish normal
            // request identity before recursion so cycles do not reload it.
            self.load_host_record(loaded, true)?;
            if memoize { self.loaded_host_requests.insert(key, id.clone()); }
            id
        };
        if request.phase != ImportPhase::Source { self.expand_record(&id)?; }
        Ok(id)
    }
    /// Load a dynamic request, preserving phase-independent canonical identity.
    pub fn load_request(
        &mut self,
        request: &ModuleRequest,
        referrer: &str,
    ) -> Result<String, ModuleFailure> {
        let id = self.load_host_request_record(request, referrer)?;
        if request.phase != ImportPhase::Source {
            self.expand_record(&id)?;
            self.first_graph_failure(&id, true, &mut HashSet::new())?;
            // Reuse the complete export validation and root error identity.
            self.link(&id, "")?;
        }
        Ok(id)
    }
}
