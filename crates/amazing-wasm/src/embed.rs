//! The embedder API: the seam a host (js-runtime's WebAssembly JS API,
//! or any other embedder) drives without depending on `amazing-wasm`'s
//! internal `Store`/`Interpreter` representation.
//!
//! This module intentionally has **no dependency on `amazing-js`** — it is
//! the trait/handle boundary the other side of that dependency is built
//! against. The pieces map directly onto the WebAssembly JS API surface:
//!
//! - [`compile`] — `WebAssembly.compile`/`new WebAssembly.Module`: decode +
//!   validate bytes into a [`CompiledModule`], whose [`CompiledModule::imports`]
//!   and [`CompiledModule::exports`] are `WebAssembly.Module.imports()`/
//!   `.exports()`.
//! - [`Engine::instantiate`] — `WebAssembly.instantiate`/`new
//!   WebAssembly.Instance`: resolves each import through the embedder's own
//!   [`ImportResolver`] (its `resolve` sees the same `(module, name, kind)`
//!   triple the JS API reads off the import object) rather than a
//!   pre-built address map, then runs `start` if present.
//! - [`FuncHandle`]/[`TableHandle`]/[`MemoryHandle`]/[`GlobalHandle`] —
//!   opaque store addresses standing in for `WebAssembly.{Function,Table,
//!   Memory,Global}`. `Engine::create_*` makes fresh ones (a host function
//!   for an import object, a `WebAssembly.Memory` constructed from JS, ...);
//!   `Engine::get_export` reads one off an instance; `Engine::memory_grow`/
//!   `table_grow` are `.grow()`.
//! - [`crate::ErrorKind::js_error_class`] — which of `CompileError`,
//!   `LinkError`, `RuntimeError` an error should surface as.
//!
//! Everything here is built on the same [`crate::Store`]/[`crate::Interpreter`]
//! the `.wast` runner uses; it only adds a handle-based, name-resolution-free
//! layer on top so the embedder never touches store addresses directly.

use std::collections::HashMap;
use std::rc::Rc;

use crate::error::{Result, WasmError};
use crate::interp::Interpreter;
use crate::module::{ExportDesc, ImportDesc, Module};
use crate::store::{
    Extern, FuncInstance, GlobalInstance, HostFn, MemoryInstance, Store, TableInstance,
};
use crate::types::{FuncType, GlobalType, MemoryType, TableType};
use crate::value::{Ref, Value};

/// Which of the three WebAssembly JS-API error constructors an error kind
/// surfaces as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JsErrorClass {
    /// `WebAssembly.CompileError`: malformed bytes or a validation failure.
    Compile,
    /// `WebAssembly.LinkError`: valid but could not be instantiated.
    Link,
    /// `WebAssembly.RuntimeError`: a trap or resource exhaustion.
    Runtime,
}

impl crate::error::ErrorKind {
    /// The WebAssembly JS-API error constructor this kind maps onto.
    pub fn js_error_class(self) -> JsErrorClass {
        use crate::error::ErrorKind::*;
        match self {
            Malformed | Invalid | Unsupported => JsErrorClass::Compile,
            Unlinkable => JsErrorClass::Link,
            Trap | Exhaustion => JsErrorClass::Runtime,
        }
    }
}

/// The kind and type of one import or export, independent of any store
/// address — what `WebAssembly.Module.imports()`/`.exports()` report.
#[derive(Clone, PartialEq, Debug)]
pub enum ExternKind {
    /// A function and its signature.
    Func(FuncType),
    /// A table and its element type/limits.
    Table(TableType),
    /// A linear memory and its page limits.
    Memory(MemoryType),
    /// A global and its value type/mutability.
    Global(GlobalType),
}

/// One entry of [`CompiledModule::imports`].
#[derive(Clone, PartialEq, Debug)]
pub struct ImportType {
    /// The two-level import's module name.
    pub module: String,
    /// The two-level import's field name.
    pub name: String,
    /// What must be supplied to satisfy it.
    pub kind: ExternKind,
}

/// One entry of [`CompiledModule::exports`].
#[derive(Clone, PartialEq, Debug)]
pub struct ExportType {
    /// The exported name.
    pub name: String,
    /// What the export provides.
    pub kind: ExternKind,
}

/// A module that has been decoded and validated, ready to introspect and
/// instantiate. Corresponds to `WebAssembly.Module`.
pub struct CompiledModule {
    module: Rc<Module>,
    imports: Vec<ImportType>,
    exports: Vec<ExportType>,
}

impl CompiledModule {
    /// This module's imports, in declaration order — `WebAssembly.Module.imports()`.
    pub fn imports(&self) -> &[ImportType] {
        &self.imports
    }

    /// This module's exports, in declaration order — `WebAssembly.Module.exports()`.
    pub fn exports(&self) -> &[ExportType] {
        &self.exports
    }
}

/// Decode and validate a module from bytes. `WebAssembly.compile`/
/// `new WebAssembly.Module`.
///
/// Any error's [`crate::ErrorKind`] is [`crate::ErrorKind::Malformed`],
/// [`crate::ErrorKind::Invalid`] or [`crate::ErrorKind::Unsupported`] — all
/// [`JsErrorClass::Compile`].
pub fn compile(bytes: &[u8]) -> Result<CompiledModule> {
    let module = crate::decode::decode_module(bytes)?;
    crate::validate::validate_module(&module)?;
    let imports = module
        .imports
        .iter()
        .map(|imp| ImportType {
            module: imp.module.clone(),
            name: imp.name.clone(),
            kind: import_kind(&module, &imp.desc),
        })
        .collect();
    let exports = module
        .exports
        .iter()
        .map(|exp| ExportType { name: exp.name.clone(), kind: export_kind(&module, exp.desc) })
        .collect();
    Ok(CompiledModule { module: Rc::new(module), imports, exports })
}

fn import_kind(module: &Module, desc: &ImportDesc) -> ExternKind {
    match desc {
        ImportDesc::Func(t) => ExternKind::Func(
            module.types.get(*t as usize).cloned().expect("validated module has valid type index"),
        ),
        ImportDesc::Table(t) => ExternKind::Table(*t),
        ImportDesc::Memory(t) => ExternKind::Memory(*t),
        ImportDesc::Global(t) => ExternKind::Global(*t),
    }
}

fn export_kind(module: &Module, desc: ExportDesc) -> ExternKind {
    match desc {
        ExportDesc::Func(i) => ExternKind::Func(
            module.func_type(i).cloned().expect("validated module has valid export index"),
        ),
        ExportDesc::Table(i) => {
            ExternKind::Table(module.table_type(i).expect("validated module has valid export index"))
        }
        ExportDesc::Memory(i) => ExternKind::Memory(
            module.memory_type(i).expect("validated module has valid export index"),
        ),
        ExportDesc::Global(i) => ExternKind::Global(
            module.global_type(i).expect("validated module has valid export index"),
        ),
    }
}

/// An opaque handle to a function in an [`Engine`]'s store — either a
/// defined WebAssembly function or a host function created with
/// [`Engine::create_func`]. Stands in for `WebAssembly.Function`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FuncHandle(u32);

/// An opaque handle to a table. Stands in for `WebAssembly.Table`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TableHandle(u32);

/// An opaque handle to a linear memory. Stands in for `WebAssembly.Memory`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemoryHandle(u32);

/// An opaque handle to a global. Stands in for `WebAssembly.Global`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlobalHandle(u32);

/// An opaque handle to a module instance. Stands in for `WebAssembly.Instance`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InstanceHandle(usize);

/// A handle to whatever kind of extern value one import or export is —
/// the resolved counterpart of [`ExternKind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExternValue {
    /// A function.
    Func(FuncHandle),
    /// A table.
    Table(TableHandle),
    /// A linear memory.
    Memory(MemoryHandle),
    /// A global.
    Global(GlobalHandle),
}

impl ExternValue {
    fn to_extern(self) -> Extern {
        match self {
            ExternValue::Func(h) => Extern::Func(h.0),
            ExternValue::Table(h) => Extern::Table(h.0),
            ExternValue::Memory(h) => Extern::Memory(h.0),
            ExternValue::Global(h) => Extern::Global(h.0),
        }
    }

    fn from_extern(ext: Extern) -> ExternValue {
        match ext {
            Extern::Func(a) => ExternValue::Func(FuncHandle(a)),
            Extern::Table(a) => ExternValue::Table(TableHandle(a)),
            Extern::Memory(a) => ExternValue::Memory(MemoryHandle(a)),
            Extern::Global(a) => ExternValue::Global(GlobalHandle(a)),
        }
    }
}

/// Supplies a value for one of a module's imports during instantiation.
///
/// The embedder (js-runtime, for the WebAssembly JS API) implements this
/// over its own import-object lookup; `amazing-wasm` never depends on
/// `amazing-js`. `expected` is the import's declared kind/type, exactly as
/// reported by [`CompiledModule::imports`], so the resolver can validate a
/// JS value against it (or just build a fresh host function/memory/table/
/// global on the spot via `Engine::create_*` and return its handle).
///
/// A resolver that cannot satisfy an import should return
/// [`WasmError::unlinkable`] so the failure surfaces as
/// [`JsErrorClass::Link`].
pub trait ImportResolver {
    /// Resolve one two-level import by name and expected kind.
    fn resolve(&mut self, module: &str, name: &str, expected: &ExternKind) -> Result<ExternValue>;
}

/// The embedder's handle onto a [`Store`]: creates host-side extern values,
/// instantiates [`CompiledModule`]s against an [`ImportResolver`], and calls
/// into instances by handle. Corresponds to a WebAssembly JS API realm's
/// module/instance registry.
#[derive(Default)]
pub struct Engine {
    store: Store,
}

impl Engine {
    /// A fresh engine with an empty store.
    pub fn new() -> Self {
        Engine { store: Store::new() }
    }

    /// Instantiate a compiled module, resolving each import through
    /// `imports`. `WebAssembly.instantiate`/`new WebAssembly.Instance`.
    ///
    /// Runs the module's `start` function, if any, before returning. An
    /// error from `imports.resolve` is propagated as-is (its `ErrorKind`
    /// drives `js_error_class`); a resolved value whose kind is
    /// incompatible with the import's declared type is caught by the same
    /// linking checks the `.wast` runner uses and surfaces as
    /// [`crate::ErrorKind::Unlinkable`].
    pub fn instantiate(
        &mut self,
        compiled: &CompiledModule,
        imports: &mut dyn ImportResolver,
    ) -> Result<InstanceHandle> {
        let mut map: HashMap<(String, String), Extern> = HashMap::new();
        for imp in &compiled.imports {
            let ev = imports.resolve(&imp.module, &imp.name, &imp.kind)?;
            map.insert((imp.module.clone(), imp.name.clone()), ev.to_extern());
        }
        let inst = self
            .store
            .instantiate(compiled.module.clone(), &|m, n, _| map.get(&(m.to_owned(), n.to_owned())).copied())?;
        if let Some(start) = self.store.instances[inst].module.start {
            let addr = self.store.instances[inst].funcs[start as usize];
            let mut interp = Interpreter::new(&mut self.store);
            interp.invoke(inst, addr, Vec::new())?;
        }
        Ok(InstanceHandle(inst))
    }

    /// Look up an export by name. `Reflect.ownKeys`/property access on a
    /// `WebAssembly.Instance.exports` object.
    pub fn get_export(&self, inst: InstanceHandle, name: &str) -> Option<ExternValue> {
        self.store.instances[inst.0].export(name).map(ExternValue::from_extern)
    }

    /// Call an exported function by name with the given arguments.
    pub fn call_export(&mut self, inst: InstanceHandle, name: &str, args: Vec<Value>) -> Result<Vec<Value>> {
        let (addr, _ty) = self.store.export_func(inst.0, name)?;
        let mut interp = Interpreter::new(&mut self.store);
        interp.invoke(inst.0, addr, args)
    }

    /// Call a function by handle (e.g. one already resolved through
    /// [`Engine::get_export`]) inside `inst`'s index space.
    pub fn call_func(&mut self, inst: InstanceHandle, func: FuncHandle, args: Vec<Value>) -> Result<Vec<Value>> {
        let mut interp = Interpreter::new(&mut self.store);
        interp.invoke(inst.0, func.0, args)
    }

    /// The signature of a function handle.
    pub fn func_type(&self, h: FuncHandle) -> FuncType {
        self.store.funcs[h.0 as usize].ty().clone()
    }

    /// Create a host function, callable from WebAssembly and usable to
    /// satisfy a `Func` import. `new WebAssembly.Function`, or a JS
    /// function placed in an import object.
    pub fn create_func(&mut self, ty: FuncType, name: impl Into<String>, call: HostFn) -> FuncHandle {
        let addr = self.store.funcs.len() as u32;
        self.store.funcs.push(FuncInstance::Host { ty, name: name.into(), func: call });
        FuncHandle(addr)
    }

    /// Create a fresh table. `new WebAssembly.Table`.
    pub fn create_table(&mut self, ty: TableType) -> TableHandle {
        let addr = self.store.tables.len() as u32;
        self.store.tables.push(TableInstance::new(ty));
        TableHandle(addr)
    }

    /// This table's declared type.
    pub fn table_type(&self, h: TableHandle) -> TableType {
        self.store.tables[h.0 as usize].ty
    }

    /// Current element count. `WebAssembly.Table.prototype.length`.
    pub fn table_len(&self, h: TableHandle) -> u32 {
        self.store.tables[h.0 as usize].elems.len() as u32
    }

    /// Grow by `delta` elements initialised to `init`, returning the
    /// previous size or `-1` on failure. `WebAssembly.Table.prototype.grow`.
    pub fn table_grow(&mut self, h: TableHandle, delta: u32, init: Ref) -> i32 {
        self.store.tables[h.0 as usize].grow(delta, init)
    }

    /// Read one element. `WebAssembly.Table.prototype.get`.
    pub fn table_get(&self, h: TableHandle, idx: u32) -> Result<Ref> {
        self.store.tables[h.0 as usize]
            .elems
            .get(idx as usize)
            .copied()
            .ok_or_else(|| WasmError::trap("out of bounds table access"))
    }

    /// Write one element. `WebAssembly.Table.prototype.set`.
    pub fn table_set(&mut self, h: TableHandle, idx: u32, val: Ref) -> Result<()> {
        let slot = self.store.tables[h.0 as usize]
            .elems
            .get_mut(idx as usize)
            .ok_or_else(|| WasmError::trap("out of bounds table access"))?;
        *slot = val;
        Ok(())
    }

    /// Create a fresh linear memory. `new WebAssembly.Memory`.
    pub fn create_memory(&mut self, ty: MemoryType) -> MemoryHandle {
        let addr = self.store.memories.len() as u32;
        self.store.memories.push(MemoryInstance::new(ty));
        MemoryHandle(addr)
    }

    /// This memory's declared type.
    pub fn memory_type(&self, h: MemoryHandle) -> MemoryType {
        self.store.memories[h.0 as usize].ty
    }

    /// Current size in 64 KiB pages.
    pub fn memory_size(&self, h: MemoryHandle) -> u32 {
        self.store.memories[h.0 as usize].size_pages()
    }

    /// Grow by `delta` pages, returning the previous size or `-1` on
    /// failure. `WebAssembly.Memory.prototype.grow`.
    pub fn memory_grow(&mut self, h: MemoryHandle, delta: u32) -> i32 {
        self.store.memories[h.0 as usize].grow(delta)
    }

    /// The memory's bytes — the backing store for a `WebAssembly.Memory`'s
    /// `buffer` (an `ArrayBuffer`/typed array view in the JS API).
    pub fn memory_bytes(&self, h: MemoryHandle) -> &[u8] {
        &self.store.memories[h.0 as usize].data
    }

    /// Mutable access to the memory's bytes.
    pub fn memory_bytes_mut(&mut self, h: MemoryHandle) -> &mut [u8] {
        &mut self.store.memories[h.0 as usize].data
    }

    /// Create a fresh global. `new WebAssembly.Global`.
    pub fn create_global(&mut self, ty: GlobalType, value: Value) -> Result<GlobalHandle> {
        if value.ty() != ty.val {
            return Err(WasmError::invalid("global initializer type mismatch"));
        }
        let addr = self.store.globals.len() as u32;
        self.store.globals.push(GlobalInstance { ty, value });
        Ok(GlobalHandle(addr))
    }

    /// This global's declared type.
    pub fn global_type(&self, h: GlobalHandle) -> GlobalType {
        self.store.globals[h.0 as usize].ty
    }

    /// Current value. `WebAssembly.Global.prototype.value` getter.
    pub fn global_get(&self, h: GlobalHandle) -> Value {
        self.store.globals[h.0 as usize].value
    }

    /// Set the value, rejecting a type mismatch or an immutable global.
    /// `WebAssembly.Global.prototype.value` setter.
    pub fn global_set(&mut self, h: GlobalHandle, v: Value) -> Result<()> {
        let g = &mut self.store.globals[h.0 as usize];
        if !g.ty.mutable {
            return Err(WasmError::invalid("cannot set an immutable global"));
        }
        if v.ty() != g.ty.val {
            return Err(WasmError::invalid("global value type mismatch"));
        }
        g.value = v;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::encode_module;
    use crate::instr::{Instr, NumOp};
    use crate::module::{Export, Global, Import};
    use crate::types::ValType;

    /// A module importing `env.add_one: [i32] -> [i32]`, defining
    /// `apply(x) = add_one(x) + 41`, and exporting that function, a memory
    /// and an immutable global.
    fn test_module() -> Module {
        let sig = FuncType::new(vec![ValType::I32], vec![ValType::I32]);
        Module {
            types: vec![sig],
            imports: vec![Import {
                module: "env".to_owned(),
                name: "add_one".to_owned(),
                desc: ImportDesc::Func(0),
            }],
            funcs: vec![crate::module::Func {
                type_idx: 0,
                locals: vec![],
                body: vec![
                    Instr::LocalGet(0),
                    Instr::Call(0),
                    Instr::I32Const(41),
                    Instr::Num(NumOp::I32Add),
                    Instr::End,
                ],
            }],
            memories: vec![MemoryType { limits: crate::types::Limits { min: 1, max: Some(2) } }],
            globals: vec![Global {
                ty: GlobalType { val: ValType::I32, mutable: false },
                init: vec![Instr::I32Const(7), Instr::End],
            }],
            exports: vec![
                Export { name: "apply".to_owned(), desc: ExportDesc::Func(1) },
                Export { name: "mem".to_owned(), desc: ExportDesc::Memory(0) },
                Export { name: "g".to_owned(), desc: ExportDesc::Global(0) },
            ],
            ..Default::default()
        }
    }

    struct FixedResolver(FuncHandle);

    impl ImportResolver for FixedResolver {
        fn resolve(&mut self, module: &str, name: &str, expected: &ExternKind) -> Result<ExternValue> {
            assert_eq!(module, "env");
            assert_eq!(name, "add_one");
            assert!(matches!(expected, ExternKind::Func(_)));
            Ok(ExternValue::Func(self.0))
        }
    }

    #[test]
    fn compile_reports_import_and_export_descriptors() {
        let bytes = encode_module(&test_module());
        let compiled = compile(&bytes).unwrap();
        assert_eq!(compiled.imports().len(), 1);
        assert_eq!(compiled.imports()[0].module, "env");
        assert_eq!(compiled.imports()[0].name, "add_one");
        assert!(matches!(&compiled.imports()[0].kind, ExternKind::Func(t) if t.params == [ValType::I32]));
        assert_eq!(compiled.exports().len(), 3);
        assert_eq!(compiled.exports()[0].name, "apply");
        assert!(matches!(compiled.exports()[1].kind, ExternKind::Memory(_)));
        assert!(matches!(&compiled.exports()[2].kind, ExternKind::Global(g) if !g.mutable));
    }

    #[test]
    fn instantiate_resolves_imports_and_runs_calls() {
        let bytes = encode_module(&test_module());
        let compiled = compile(&bytes).unwrap();
        let mut engine = Engine::new();
        let host = engine.create_func(
            FuncType::new(vec![ValType::I32], vec![ValType::I32]),
            "add_one",
            Rc::new(|args: &[Value]| Ok(vec![Value::I32(args[0].as_i32()? + 1)])),
        );
        let inst = engine.instantiate(&compiled, &mut FixedResolver(host)).unwrap();

        let result = engine.call_export(inst, "apply", vec![Value::I32(10)]).unwrap();
        assert_eq!(result, vec![Value::I32(52)]);

        let ExternValue::Memory(mem) = engine.get_export(inst, "mem").unwrap() else {
            panic!("expected a memory export");
        };
        assert_eq!(engine.memory_size(mem), 1);
        assert_eq!(engine.memory_grow(mem, 1), 1);
        assert_eq!(engine.memory_size(mem), 2);
        assert_eq!(engine.memory_bytes(mem).len(), 2 * crate::store::PAGE_SIZE);

        let ExternValue::Global(g) = engine.get_export(inst, "g").unwrap() else {
            panic!("expected a global export");
        };
        assert_eq!(engine.global_get(g), Value::I32(7));
        assert!(engine.global_set(g, Value::I32(8)).is_err());
    }

    #[test]
    fn missing_import_is_unlinkable() {
        struct NoneResolver;
        impl ImportResolver for NoneResolver {
            fn resolve(&mut self, module: &str, name: &str, _expected: &ExternKind) -> Result<ExternValue> {
                Err(WasmError::unlinkable(format!("unknown import: {module}::{name}")))
            }
        }
        let bytes = encode_module(&test_module());
        let compiled = compile(&bytes).unwrap();
        let mut engine = Engine::new();
        let err = engine.instantiate(&compiled, &mut NoneResolver).unwrap_err();
        assert_eq!(err.kind, crate::ErrorKind::Unlinkable);
        assert_eq!(err.kind.js_error_class(), JsErrorClass::Link);
    }

    #[test]
    fn error_kinds_map_onto_js_error_classes() {
        assert_eq!(crate::ErrorKind::Malformed.js_error_class(), JsErrorClass::Compile);
        assert_eq!(crate::ErrorKind::Invalid.js_error_class(), JsErrorClass::Compile);
        assert_eq!(crate::ErrorKind::Unsupported.js_error_class(), JsErrorClass::Compile);
        assert_eq!(crate::ErrorKind::Unlinkable.js_error_class(), JsErrorClass::Link);
        assert_eq!(crate::ErrorKind::Trap.js_error_class(), JsErrorClass::Runtime);
        assert_eq!(crate::ErrorKind::Exhaustion.js_error_class(), JsErrorClass::Runtime);
    }
}
