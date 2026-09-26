//! The runtime store: function, table, memory, global and segment instances.
//!
//! Instantiation follows the specification's order: resolve and type-check
//! imports, allocate the defined instances, evaluate global initializers,
//! build the module instance and its exports, apply active element and data
//! segments, run `start`, then drop the active segments (bulk-memory
//! semantics). An out-of-bounds active segment is a runtime failure and is
//! classified as [`ErrorKind::Trap`](crate::ErrorKind::Trap), which is what
//! the reference testsuite asserts.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use crate::error::{Result, WasmError};
use crate::instr::Instr;
use crate::module::{ElemMode, ExportDesc, ImportDesc, Module};
use crate::types::{FuncType, GlobalType, Limits, MemoryType, TableType, ValType};
use crate::value::{Ref, Value};

/// A 64 KiB linear-memory page.
pub const PAGE_SIZE: usize = 65536;

/// The 32-bit linear-memory ceiling in pages.
pub const MAX_PAGES: u32 = 65536;

/// A host function callable from WebAssembly.
pub type HostFn = Rc<dyn Fn(&[Value]) -> Result<Vec<Value>>>;

/// A runtime function: a defined WebAssembly body or a host closure.
#[derive(Clone)]
pub enum FuncInstance {
    /// A function defined by a module.
    Wasm {
        /// Declared signature.
        ty: FuncType,
        /// The defining module (shared; bodies are immutable).
        module: Rc<Module>,
        /// Index into `module.funcs`.
        index: usize,
        /// Local types beyond the parameters.
        locals: Vec<ValType>,
        /// The module instance this function was defined in. Local index
        /// operands inside its body (tables, memories, globals, other
        /// functions) always resolve against this instance, never against
        /// whichever instance happened to call it.
        inst: usize,
    },
    /// A host function.
    Host {
        /// Declared signature.
        ty: FuncType,
        /// Debug name (`module.field`).
        name: String,
        /// The closure.
        func: HostFn,
    },
}

impl FuncInstance {
    /// The signature of this function.
    pub fn ty(&self) -> &FuncType {
        match self {
            FuncInstance::Wasm { ty, .. } | FuncInstance::Host { ty, .. } => ty,
        }
    }
}

impl fmt::Debug for FuncInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FuncInstance::Wasm { ty, index, .. } => {
                f.debug_struct("WasmFunc").field("ty", ty).field("index", index).finish()
            }
            FuncInstance::Host { ty, name, .. } => {
                f.debug_struct("HostFunc").field("ty", ty).field("name", name).finish()
            }
        }
    }
}

/// A linear memory.
#[derive(Clone, Debug)]
pub struct MemoryInstance {
    /// Declared type.
    pub ty: MemoryType,
    /// Raw bytes, `len == size_pages * PAGE_SIZE`.
    pub data: Vec<u8>,
}

impl MemoryInstance {
    /// Allocate a memory with `min` pages.
    pub fn new(ty: MemoryType) -> Self {
        MemoryInstance { ty, data: vec![0; ty.limits.min as usize * PAGE_SIZE] }
    }

    /// Current size in pages.
    pub fn size_pages(&self) -> u32 {
        (self.data.len() / PAGE_SIZE) as u32
    }

    /// Grow by `delta` pages, returning the previous size or `-1`.
    pub fn grow(&mut self, delta: u32) -> i32 {
        let old = self.size_pages();
        let ceiling = self.ty.limits.max.unwrap_or(MAX_PAGES).min(MAX_PAGES);
        let Some(new) = old.checked_add(delta) else {
            return -1;
        };
        if new > ceiling {
            return -1;
        }
        self.data.resize(new as usize * PAGE_SIZE, 0);
        old as i32
    }
}

/// A table of references.
#[derive(Clone, Debug)]
pub struct TableInstance {
    /// Declared type.
    pub ty: TableType,
    /// Elements.
    pub elems: Vec<Ref>,
}

impl TableInstance {
    /// Allocate a table with `min` null elements.
    pub fn new(ty: TableType) -> Self {
        let null = Ref::Null(ty.elem);
        TableInstance { ty, elems: vec![null; ty.limits.min as usize] }
    }

    /// Grow by `delta` elements initialised to `init`, returning the previous
    /// size or `-1`.
    pub fn grow(&mut self, delta: u32, init: Ref) -> i32 {
        let old = self.elems.len();
        let ceiling = self.ty.limits.max.unwrap_or(u32::MAX);
        let Some(new) = old.checked_add(delta as usize) else {
            return -1;
        };
        if new as u64 > u64::from(ceiling) {
            return -1;
        }
        self.elems.resize(new, init);
        old as i32
    }
}

/// A global variable instance.
#[derive(Clone, Debug)]
pub struct GlobalInstance {
    /// Declared type.
    pub ty: GlobalType,
    /// Current value.
    pub value: Value,
}

/// An element segment instance.
#[derive(Clone, Debug)]
pub struct ElemInstance {
    /// Element reference type.
    pub ty: ValType,
    /// The references.
    pub elems: Vec<Ref>,
    /// Whether the segment has been dropped.
    pub dropped: bool,
}

/// A data segment instance.
#[derive(Clone, Debug)]
pub struct DataInstance {
    /// The bytes.
    pub data: Vec<u8>,
    /// Whether the segment has been dropped.
    pub dropped: bool,
}

/// An exportable external value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extern {
    /// A function address.
    Func(u32),
    /// A table address.
    Table(u32),
    /// A memory address.
    Memory(u32),
    /// A global address.
    Global(u32),
}

/// A module instance.
#[derive(Clone, Debug)]
pub struct Instance {
    /// The instantiated module.
    pub module: Rc<Module>,
    /// Export name to address.
    pub exports: BTreeMap<String, Extern>,
    /// Function address space (imports first).
    pub funcs: Vec<u32>,
    /// Table address space.
    pub tables: Vec<u32>,
    /// Memory address space.
    pub memories: Vec<u32>,
    /// Global address space.
    pub globals: Vec<u32>,
    /// Element segment indices.
    pub elems: Vec<usize>,
    /// Data segment indices.
    pub datas: Vec<usize>,
    /// Registered name, if the script named it.
    pub name: Option<String>,
}

impl Instance {
    /// Look up an export by name.
    pub fn export(&self, name: &str) -> Option<Extern> {
        self.exports.get(name).copied()
    }
}

/// The whole runtime store.
#[derive(Default)]
pub struct Store {
    /// All function instances.
    pub funcs: Vec<FuncInstance>,
    /// All table instances.
    pub tables: Vec<TableInstance>,
    /// All memory instances.
    pub memories: Vec<MemoryInstance>,
    /// All global instances.
    pub globals: Vec<GlobalInstance>,
    /// All element segments.
    pub elems: Vec<ElemInstance>,
    /// All data segments.
    pub datas: Vec<DataInstance>,
    /// All module instances.
    pub instances: Vec<Instance>,
}

impl Store {
    /// An empty store.
    pub fn new() -> Self {
        Store::default()
    }

    /// Instantiate `module`, resolving imports through `resolve`.
    pub fn instantiate(
        &mut self,
        module: Rc<Module>,
        resolve: &dyn Fn(&str, &str, &ImportDesc) -> Option<Extern>,
    ) -> Result<usize> {
        let types = module.types.clone();
        let inst_id = self.instances.len();

        // 1. Resolve and type-check imports.
        let mut funcs: Vec<u32> = Vec::new();
        let mut tables: Vec<u32> = Vec::new();
        let mut memories: Vec<u32> = Vec::new();
        let mut globals: Vec<u32> = Vec::new();
        for import in &module.imports {
            let ext = resolve(&import.module, &import.name, &import.desc).ok_or_else(|| {
                WasmError::unlinkable(format!("unknown import: {}::{}", import.module, import.name))
            })?;
            match (&import.desc, ext) {
                (ImportDesc::Func(t), Extern::Func(a)) => {
                    let expected = types
                        .get(*t as usize)
                        .ok_or_else(|| WasmError::invalid("unknown type"))?;
                    let actual = self
                        .funcs
                        .get(a as usize)
                        .ok_or_else(|| WasmError::unlinkable("unknown function import"))?;
                    if actual.ty() != expected {
                        return Err(WasmError::unlinkable("incompatible import type"));
                    }
                    funcs.push(a);
                }
                (ImportDesc::Table(t), Extern::Table(a)) => {
                    let actual = self
                        .tables
                        .get(a as usize)
                        .ok_or_else(|| WasmError::unlinkable("unknown table import"))?;
                    // Matching uses the table's current live size, not the
                    // size it was declared with: it may have grown since.
                    let live = TableType {
                        elem: actual.ty.elem,
                        limits: Limits { min: actual.elems.len() as u32, max: actual.ty.limits.max },
                    };
                    check_table_import(t, &live)?;
                    tables.push(a);
                }
                (ImportDesc::Memory(t), Extern::Memory(a)) => {
                    let actual = self
                        .memories
                        .get(a as usize)
                        .ok_or_else(|| WasmError::unlinkable("unknown memory import"))?;
                    // Same live-size matching as tables above.
                    let live = MemoryType {
                        limits: Limits { min: actual.size_pages(), max: actual.ty.limits.max },
                    };
                    check_memory_import(t, &live)?;
                    memories.push(a);
                }
                (ImportDesc::Global(t), Extern::Global(a)) => {
                    let actual = self
                        .globals
                        .get(a as usize)
                        .ok_or_else(|| WasmError::unlinkable("unknown global import"))?;
                    if !t.matches(&actual.ty) {
                        return Err(WasmError::unlinkable("incompatible import type"));
                    }
                    globals.push(a);
                }
                _ => return Err(WasmError::unlinkable("incompatible import kind")),
            }
        }

        // 2. Allocate defined functions.
        for (index, f) in module.funcs.iter().enumerate() {
            let ty = types
                .get(f.type_idx as usize)
                .cloned()
                .ok_or_else(|| WasmError::invalid("unknown type"))?;
            let addr = self.funcs.len() as u32;
            self.funcs.push(FuncInstance::Wasm {
                ty,
                module: module.clone(),
                index,
                locals: f.locals.clone(),
                inst: inst_id,
            });
            funcs.push(addr);
        }

        // 3. Allocate tables and memories.
        for t in &module.tables {
            let addr = self.tables.len() as u32;
            self.tables.push(TableInstance::new(*t));
            tables.push(addr);
        }
        for m in &module.memories {
            let addr = self.memories.len() as u32;
            self.memories.push(MemoryInstance::new(*m));
            memories.push(addr);
        }

        // 4. Allocate globals, evaluating initializers against imports only.
        for g in &module.globals {
            let value = eval_const(&g.init, &globals, &funcs, &self.globals)?;
            let addr = self.globals.len() as u32;
            self.globals.push(GlobalInstance { ty: g.ty, value });
            globals.push(addr);
        }

        // 5. Build the instance.
        let mut exports = BTreeMap::new();
        for e in &module.exports {
            let ext = match e.desc {
                ExportDesc::Func(i) => Extern::Func(addr_in(&funcs, i, "unknown function")?),
                ExportDesc::Table(i) => Extern::Table(addr_in(&tables, i, "unknown table")?),
                ExportDesc::Memory(i) => Extern::Memory(addr_in(&memories, i, "unknown memory")?),
                ExportDesc::Global(i) => Extern::Global(addr_in(&globals, i, "unknown global")?),
            };
            exports.insert(e.name.clone(), ext);
        }
        debug_assert_eq!(inst_id, self.instances.len());
        self.instances.push(Instance {
            module: module.clone(),
            exports,
            funcs,
            tables,
            memories,
            globals,
            elems: Vec::new(),
            datas: Vec::new(),
            name: None,
        });

        // 6. Element segments.
        let inst_funcs = self.instances[inst_id].funcs.clone();
        let inst_globals = self.instances[inst_id].globals.clone();
        let inst_tables = self.instances[inst_id].tables.clone();
        for e in &module.elems {
            let refs = e
                .init
                .iter()
                .map(|expr| {
                    eval_const(expr, &inst_globals, &inst_funcs, &self.globals).and_then(|v| v.as_ref())
                })
                .collect::<Result<Vec<Ref>>>()?;
            let seg = self.elems.len();
            self.elems.push(ElemInstance { ty: e.ty, elems: refs, dropped: false });
            self.instances[inst_id].elems.push(seg);
            match &e.mode {
                ElemMode::Declarative => self.elems[seg].dropped = true,
                ElemMode::Passive => {}
                ElemMode::Active { table, offset } => {
                    let off = eval_const_i32(offset, &inst_globals, &inst_funcs, &self.globals)?;
                    let table_addr = addr_in(&inst_tables, *table, "unknown table")?;
                    apply_elems(&mut self.tables[table_addr as usize], off, &self.elems[seg].elems)?;
                    self.elems[seg].dropped = true;
                }
            }
        }

        // 7. Data segments.
        let inst_memories = self.instances[inst_id].memories.clone();
        for d in &module.datas {
            let seg = self.datas.len();
            self.datas.push(DataInstance { data: d.init.clone(), dropped: false });
            self.instances[inst_id].datas.push(seg);
            match &d.mode {
                crate::module::DataMode::Passive => {}
                crate::module::DataMode::Active { memory, offset } => {
                    let off = eval_const_i32(offset, &inst_globals, &inst_funcs, &self.globals)?;
                    let mem_addr = addr_in(&inst_memories, *memory, "unknown memory")?;
                    apply_data(&mut self.memories[mem_addr as usize], off, &self.datas[seg].data)?;
                    self.datas[seg].dropped = true;
                }
            }
        }

        Ok(inst_id)
    }

    /// The address and signature of an exported function in `instance`.
    pub fn export_func(&self, instance: usize, name: &str) -> Result<(u32, FuncType)> {
        match self.instances[instance].export(name) {
            Some(Extern::Func(a)) => Ok((a, self.funcs[a as usize].ty().clone())),
            Some(_) => Err(WasmError::trap("not a function export")),
            None => Err(WasmError::trap(format!("no export named {name}"))),
        }
    }

    /// The address of an exported global in `instance`.
    pub fn export_global(&self, instance: usize, name: &str) -> Result<u32> {
        match self.instances[instance].export(name) {
            Some(Extern::Global(a)) => Ok(a),
            Some(_) => Err(WasmError::trap("not a global export")),
            None => Err(WasmError::trap(format!("no export named {name}"))),
        }
    }
}

fn addr_in(addrs: &[u32], index: u32, what: &str) -> Result<u32> {
    addrs.get(index as usize).copied().ok_or_else(|| WasmError::invalid(what))
}

fn check_table_import(expected: &TableType, actual: &TableType) -> Result<()> {
    if expected.elem != actual.elem || !actual.limits.matches(&expected.limits) {
        return Err(WasmError::unlinkable("incompatible import type"));
    }
    Ok(())
}

fn check_memory_import(expected: &MemoryType, actual: &MemoryType) -> Result<()> {
    if !actual.limits.matches(&expected.limits) {
        return Err(WasmError::unlinkable("incompatible import type"));
    }
    Ok(())
}

fn apply_elems(table: &mut TableInstance, offset: i32, elems: &[Ref]) -> Result<()> {
    let off = offset as usize;
    let end = off
        .checked_add(elems.len())
        .ok_or_else(|| WasmError::trap("out of bounds table access"))?;
    if end > table.elems.len() {
        return Err(WasmError::trap("out of bounds table access"));
    }
    table.elems[off..end].copy_from_slice(elems);
    Ok(())
}

fn apply_data(memory: &mut MemoryInstance, offset: i32, data: &[u8]) -> Result<()> {
    let off = offset as usize;
    let end = off
        .checked_add(data.len())
        .ok_or_else(|| WasmError::trap("out of bounds memory access"))?;
    if end > memory.data.len() {
        return Err(WasmError::trap("out of bounds memory access"));
    }
    memory.data[off..end].copy_from_slice(data);
    Ok(())
}

/// Evaluate a constant expression, returning its single value.
///
/// `globals` and `funcs` are the instance's address spaces; `store_globals`
/// supplies the values for `global.get`.
pub fn eval_const(
    expr: &[Instr],
    globals: &[u32],
    funcs: &[u32],
    store_globals: &[GlobalInstance],
) -> Result<Value> {
    let mut stack: Vec<Value> = Vec::new();
    for instr in expr {
        match instr {
            Instr::I32Const(v) => stack.push(Value::I32(*v)),
            Instr::I64Const(v) => stack.push(Value::I64(*v)),
            Instr::F32Const(bits) => stack.push(Value::F32(f32::from_bits(*bits))),
            Instr::F64Const(bits) => stack.push(Value::F64(f64::from_bits(*bits))),
            Instr::RefNull(t) => stack.push(Value::Ref(Ref::Null(*t))),
            Instr::RefFunc(f) => {
                let a = funcs
                    .get(*f as usize)
                    .copied()
                    .ok_or_else(|| WasmError::invalid("unknown function"))?;
                stack.push(Value::Ref(Ref::Func(a)));
            }
            Instr::GlobalGet(g) => {
                let a = globals
                    .get(*g as usize)
                    .copied()
                    .ok_or_else(|| WasmError::invalid("unknown global"))?;
                let gi = store_globals
                    .get(a as usize)
                    .ok_or_else(|| WasmError::invalid("unknown global"))?;
                stack.push(gi.value);
            }
            Instr::End => break,
            _ => return Err(WasmError::invalid("constant expression required")),
        }
    }
    if stack.len() != 1 {
        return Err(WasmError::invalid("type mismatch: constant expression must produce one value"));
    }
    Ok(stack[0])
}

fn eval_const_i32(
    expr: &[Instr],
    globals: &[u32],
    funcs: &[u32],
    store_globals: &[GlobalInstance],
) -> Result<i32> {
    eval_const(expr, globals, funcs, store_globals)?.as_i32()
}
