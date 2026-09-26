//! The decoded module structure.
//!
//! This is the shared representation produced both by the binary decoder and
//! by the WAT text parser (which encodes it back to binary so the testsuite
//! exercises the decoder). Index spaces keep the specification's ordering:
//! imported functions/tables/memories/globals come before defined ones.

use crate::instr::Instr;
use crate::types::{FuncType, GlobalType, MemoryType, TableType, ValType};

/// An import's external descriptor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportDesc {
    /// Imported function with a declared type index.
    Func(u32),
    /// Imported table.
    Table(TableType),
    /// Imported memory.
    Memory(MemoryType),
    /// Imported global.
    Global(GlobalType),
}

/// A single import.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Import {
    /// Module name.
    pub module: String,
    /// Field name.
    pub name: String,
    /// What is imported.
    pub desc: ImportDesc,
}

/// An export's descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportDesc {
    /// Export of function at the given index.
    Func(u32),
    /// Export of table at the given index.
    Table(u32),
    /// Export of memory at the given index.
    Memory(u32),
    /// Export of global at the given index.
    Global(u32),
}

/// A single export.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Export {
    /// Exported name.
    pub name: String,
    /// What is exported.
    pub desc: ExportDesc,
}

/// A defined function.
#[derive(Clone, PartialEq, Debug)]
pub struct Func {
    /// Index into the module's type section.
    pub type_idx: u32,
    /// Local types beyond the parameters.
    pub locals: Vec<ValType>,
    /// Flat instruction body, including the terminating `end`.
    pub body: Vec<Instr>,
}

/// A defined global.
#[derive(Clone, PartialEq, Debug)]
pub struct Global {
    /// Global type.
    pub ty: GlobalType,
    /// Constant initializer expression, including the terminating `end`.
    pub init: Vec<Instr>,
}

/// How an element segment is applied.
#[derive(Clone, PartialEq, Debug)]
pub enum ElemMode {
    /// Not applied; only usable via `table.init`.
    Passive,
    /// Applied to `table` at `offset` during instantiation.
    Active {
        /// Target table.
        table: u32,
        /// Constant offset expression.
        offset: Vec<Instr>,
    },
    /// Not applied; only declares `ref.func` targets.
    Declarative,
}

/// An element segment.
#[derive(Clone, PartialEq, Debug)]
pub struct Elem {
    /// Element reference type.
    pub ty: ValType,
    /// One initializer expression per element.
    pub init: Vec<Vec<Instr>>,
    /// Activation mode.
    pub mode: ElemMode,
}

/// How a data segment is applied.
#[derive(Clone, PartialEq, Debug)]
pub enum DataMode {
    /// Not applied; only usable via `memory.init`.
    Passive,
    /// Copied into `memory` at `offset` during instantiation.
    Active {
        /// Target memory.
        memory: u32,
        /// Constant offset expression.
        offset: Vec<Instr>,
    },
}

/// A data segment.
#[derive(Clone, PartialEq, Debug)]
pub struct Data {
    /// Raw bytes.
    pub init: Vec<u8>,
    /// Activation mode.
    pub mode: DataMode,
}

/// A fully decoded WebAssembly module.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Module {
    /// Type section.
    pub types: Vec<FuncType>,
    /// Imports, in order.
    pub imports: Vec<Import>,
    /// Defined functions, in order.
    pub funcs: Vec<Func>,
    /// Defined tables, in order.
    pub tables: Vec<TableType>,
    /// Defined memories, in order.
    pub memories: Vec<MemoryType>,
    /// Defined globals, in order.
    pub globals: Vec<Global>,
    /// Exports, in order.
    pub exports: Vec<Export>,
    /// Start function index, if any.
    pub start: Option<u32>,
    /// Element segments, in order.
    pub elems: Vec<Elem>,
    /// Data segments, in order.
    pub datas: Vec<Data>,
    /// Declared data-segment count from the data-count section, if present.
    pub data_count: Option<u32>,
}

/// Aggregate counts of an index space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Counts {
    /// Number of imported functions.
    pub imported_funcs: u32,
    /// Number of imported tables.
    pub imported_tables: u32,
    /// Number of imported memories.
    pub imported_memories: u32,
    /// Number of imported globals.
    pub imported_globals: u32,
}

impl Module {
    /// Import counts by kind.
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for import in &self.imports {
            match import.desc {
                ImportDesc::Func(_) => c.imported_funcs += 1,
                ImportDesc::Table(_) => c.imported_tables += 1,
                ImportDesc::Memory(_) => c.imported_memories += 1,
                ImportDesc::Global(_) => c.imported_globals += 1,
            }
        }
        c
    }

    /// Total function count (imported + defined).
    pub fn func_count(&self) -> u32 {
        self.counts().imported_funcs + self.funcs.len() as u32
    }

    /// Total table count.
    pub fn table_count(&self) -> u32 {
        self.counts().imported_tables + self.tables.len() as u32
    }

    /// Total memory count.
    pub fn memory_count(&self) -> u32 {
        self.counts().imported_memories + self.memories.len() as u32
    }

    /// Total global count.
    pub fn global_count(&self) -> u32 {
        self.counts().imported_globals + self.globals.len() as u32
    }

    /// The type index of function `idx`, whether imported or defined.
    pub fn func_type_index(&self, idx: u32) -> Option<u32> {
        let imported = self.counts().imported_funcs;
        if idx < imported {
            let mut seen = 0;
            for import in &self.imports {
                if let ImportDesc::Func(t) = import.desc {
                    if seen == idx {
                        return Some(t);
                    }
                    seen += 1;
                }
            }
            None
        } else {
            self.funcs.get((idx - imported) as usize).map(|f| f.type_idx)
        }
    }

    /// The imported table type at `idx` (< imported table count).
    pub fn imported_table(&self, idx: u32) -> Option<TableType> {
        let mut seen = 0;
        for import in &self.imports {
            if let ImportDesc::Table(t) = import.desc {
                if seen == idx {
                    return Some(t);
                }
                seen += 1;
            }
        }
        None
    }

    /// The imported memory type at `idx`.
    pub fn imported_memory(&self, idx: u32) -> Option<MemoryType> {
        let mut seen = 0;
        for import in &self.imports {
            if let ImportDesc::Memory(t) = import.desc {
                if seen == idx {
                    return Some(t);
                }
                seen += 1;
            }
        }
        None
    }

    /// The imported global type at `idx`.
    pub fn imported_global(&self, idx: u32) -> Option<GlobalType> {
        let mut seen = 0;
        for import in &self.imports {
            if let ImportDesc::Global(t) = import.desc {
                if seen == idx {
                    return Some(t);
                }
                seen += 1;
            }
        }
        None
    }

    /// The full type of any function index.
    pub fn func_type(&self, idx: u32) -> Option<&FuncType> {
        let ti = self.func_type_index(idx)?;
        self.types.get(ti as usize)
    }

    /// The type of any table index.
    pub fn table_type(&self, idx: u32) -> Option<TableType> {
        let imported = self.counts().imported_tables;
        if idx < imported {
            self.imported_table(idx)
        } else {
            self.tables.get((idx - imported) as usize).copied()
        }
    }

    /// The type of any memory index.
    pub fn memory_type(&self, idx: u32) -> Option<MemoryType> {
        let imported = self.counts().imported_memories;
        if idx < imported {
            self.imported_memory(idx)
        } else {
            self.memories.get((idx - imported) as usize).copied()
        }
    }

    /// The type of any global index.
    pub fn global_type(&self, idx: u32) -> Option<GlobalType> {
        let imported = self.counts().imported_globals;
        if idx < imported {
            self.imported_global(idx)
        } else {
            self.globals.get((idx - imported) as usize).map(|g| g.ty)
        }
    }
}
