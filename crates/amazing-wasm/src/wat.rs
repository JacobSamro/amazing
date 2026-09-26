//! The WebAssembly text-format (WAT) parser.
//!
//! Parses a `(module ...)` s-expression into a [`Module`], applying the text
//! format's abbreviations: inline imports and exports, inline type uses,
//! named index spaces with forward references, folded instructions, and the
//! `(memory (data ...))` / `(table reftype (elem ...))` shorthands. The
//! resulting module is encoded to binary and decoded again by the testsuite
//! runner, so the text path also exercises the decoder.

use std::collections::HashMap;

use crate::error::{Result, WasmError};
use crate::instr::Instr;
use crate::module::{
    Data, DataMode, Elem, ElemMode, Export, ExportDesc, Func, Global, Import, ImportDesc, Module,
};
use crate::numlit;
use crate::opname;
use crate::sexpr::Sexpr;
use crate::types::{BlockType, FuncType, GlobalType, Limits, MemArg, MemoryType, TableType, ValType};

/// A text-format module with its optional `$id`.
#[derive(Clone, Debug)]
pub struct ParsedModule {
    /// The decoded module.
    pub module: Module,
    /// The module identifier from `(module $id ...)`.
    pub id: Option<String>,
}

#[derive(Default)]
struct Names {
    types: HashMap<String, u32>,
    funcs: HashMap<String, u32>,
    tables: HashMap<String, u32>,
    memories: HashMap<String, u32>,
    globals: HashMap<String, u32>,
    elems: HashMap<String, u32>,
    datas: HashMap<String, u32>,
}

#[derive(Default)]
struct Reg {
    types: u32,
    funcs: u32,
    tables: u32,
    memories: u32,
    globals: u32,
    elems: u32,
    datas: u32,
    import_funcs: u32,
    import_tables: u32,
    import_memories: u32,
    import_globals: u32,
    any_def_seen: bool,
}

/// Parse a top-level `(module ...)` s-expression.
pub fn parse_module(expr: &Sexpr) -> Result<ParsedModule> {
    let fields = expr.list().ok_or_else(|| WasmError::malformed("expected (module ...)"))?;
    if fields.first().and_then(Sexpr::atom) != Some("module") {
        return Err(WasmError::malformed("expected (module ...)"));
    }
    let (id, rest) = split_id(&fields[1..])?;
    build(id, rest)
}

/// Parse the contents of a `(module quote ...)`: either a leading
/// `(module ...)` form or a bare sequence of module fields.
pub fn parse_quoted(text: &str) -> Result<ParsedModule> {
    let items = crate::sexpr::parse(text)?;
    if items.is_empty() {
        return Err(WasmError::malformed("empty quoted module"));
    }
    if items[0].head() == Some("module") {
        return parse_module(&items[0]);
    }
    let mut wrapped = vec![Sexpr::Atom("module".to_owned())];
    wrapped.extend(items);
    parse_module(&Sexpr::List(wrapped))
}

fn split_id(items: &[Sexpr]) -> Result<(Option<String>, &[Sexpr])> {
    if let Some(Sexpr::Atom(a)) = items.first() {
        if let Some(name) = a.strip_prefix('$') {
            return Ok((Some(name.to_owned()), &items[1..]));
        }
    }
    Ok((None, items))
}

fn build(id: Option<String>, rest: &[Sexpr]) -> Result<ParsedModule> {
    // Pass 1: assign every name its appearance index and count index spaces.
    let mut names = Names::default();
    let mut reg = Reg::default();
    for field in rest {
        register(field, &mut names, &mut reg)?;
    }
    if !imports_first(&reg) {
        return Err(WasmError::malformed("imports must precede definitions"));
    }

    let mut ctx = Ctx {
        module: Module::default(),
        names,
        func_cursor: reg.import_funcs,
        table_cursor: reg.import_tables,
        mem_cursor: reg.import_memories,
        global_cursor: reg.import_globals,
        elem_cursor: 0,
        data_cursor: 0,
        locals: HashMap::new(),
        nlocals: 0,
        labels: Vec::new(),
    };

    // Declared type fields define indices first, wherever they appear.
    for field in rest {
        if field.head() == Some("type") {
            ctx.parse_type_field(field)?;
        }
    }
    // Everything else, in source order.
    for field in rest {
        if field.head() == Some("type") {
            continue;
        }
        ctx.parse_field(field)?;
    }
    Ok(ParsedModule { module: ctx.module, id })
}

fn imports_first(reg: &Reg) -> bool {
    // The counters cannot detect ordering directly; the per-field check below
    // already rejects an import after a definition. This is a placeholder for
    // the case where a definition is the first field of a space and an import
    // follows.
    let _ = reg;
    true
}

fn optional_name(list: &[Sexpr]) -> Option<String> {
    match list.get(1) {
        Some(Sexpr::Atom(a)) => a.strip_prefix('$').map(str::to_owned),
        _ => None,
    }
}

fn has_list(list: &[Sexpr], head: &str) -> bool {
    list.iter().any(|x| x.head() == Some(head))
}

fn insert_name(map: &mut HashMap<String, u32>, name: &str, idx: u32) -> Result<()> {
    if map.insert(name.to_owned(), idx).is_some() {
        return Err(WasmError::malformed(format!("duplicate identifier ${name}")));
    }
    Ok(())
}

fn is_index(expr: &Sexpr) -> bool {
    match expr {
        Sexpr::Atom(a) => a.starts_with('$') || parse_index_text(a).is_ok(),
        _ => false,
    }
}

fn parse_index_text(a: &str) -> Result<u32> {
    numlit::parse_u32(a)
}

fn register(field: &Sexpr, names: &mut Names, reg: &mut Reg) -> Result<()> {
    let list = field.list().ok_or_else(|| WasmError::malformed("expected a module field"))?;
    let head = list.first().and_then(Sexpr::atom).unwrap_or("");
    match head {
        "type" => {
            if let Some(n) = optional_name(list) {
                insert_name(&mut names.types, &n, reg.types)?;
            }
            reg.types += 1;
        }
        "import" => {
            if reg.any_def_seen {
                return Err(WasmError::malformed("import after definition"));
            }
            let desc = list
                .last()
                .and_then(Sexpr::list)
                .ok_or_else(|| WasmError::malformed("malformed import"))?;
            let kind = desc.first().and_then(Sexpr::atom).unwrap_or("");
            let name = desc.get(1).and_then(Sexpr::atom).and_then(|a| a.strip_prefix('$')).map(str::to_owned);
            match kind {
                "func" => {
                    if let Some(n) = name {
                        insert_name(&mut names.funcs, &n, reg.funcs)?;
                    }
                    reg.funcs += 1;
                    reg.import_funcs += 1;
                }
                "table" => {
                    if let Some(n) = name {
                        insert_name(&mut names.tables, &n, reg.tables)?;
                    }
                    reg.tables += 1;
                    reg.import_tables += 1;
                }
                "memory" => {
                    if let Some(n) = name {
                        insert_name(&mut names.memories, &n, reg.memories)?;
                    }
                    reg.memories += 1;
                    reg.import_memories += 1;
                }
                "global" => {
                    if let Some(n) = name {
                        insert_name(&mut names.globals, &n, reg.globals)?;
                    }
                    reg.globals += 1;
                    reg.import_globals += 1;
                }
                _ => return Err(WasmError::malformed("malformed import kind")),
            }
        }
        "func" | "table" | "memory" | "global" => {
            let name = optional_name(list);
            let imported = has_list(list, "import");
            if imported && reg.any_def_seen {
                return Err(WasmError::malformed("import after definition"));
            }
            if !imported {
                reg.any_def_seen = true;
            }
            match head {
                "func" => {
                    if let Some(n) = name {
                        insert_name(&mut names.funcs, &n, reg.funcs)?;
                    }
                    reg.funcs += 1;
                }
                "table" => {
                    if let Some(n) = name {
                        insert_name(&mut names.tables, &n, reg.tables)?;
                    }
                    reg.tables += 1;
                }
                "memory" => {
                    if let Some(n) = name {
                        insert_name(&mut names.memories, &n, reg.memories)?;
                    }
                    reg.memories += 1;
                }
                _ => {
                    if let Some(n) = name {
                        insert_name(&mut names.globals, &n, reg.globals)?;
                    }
                    reg.globals += 1;
                }
            }
        }
        "elem" => {
            if let Some(n) = optional_name(list) {
                insert_name(&mut names.elems, &n, reg.elems)?;
            }
            reg.elems += 1;
        }
        "data" => {
            if let Some(n) = optional_name(list) {
                insert_name(&mut names.datas, &n, reg.datas)?;
            }
            reg.datas += 1;
        }
        "export" | "start" => {}
        _ => return Err(WasmError::malformed(format!("unknown module field {head}"))),
    }
    Ok(())
}

struct Ctx {
    module: Module,
    names: Names,
    func_cursor: u32,
    table_cursor: u32,
    mem_cursor: u32,
    global_cursor: u32,
    elem_cursor: u32,
    data_cursor: u32,
    locals: HashMap<String, u32>,
    nlocals: u32,
    labels: Vec<Option<String>>,
}

impl Ctx {
    fn parse_type_field(&mut self, field: &Sexpr) -> Result<()> {
        let list = field.list().expect("type field");
        let inner = list
            .get(2)
            .or_else(|| list.get(1))
            .and_then(Sexpr::list)
            .ok_or_else(|| WasmError::malformed("malformed type"))?;
        // `(type (func ...))` or `(type $t (func ...))`.
        if inner.first().and_then(Sexpr::atom) != Some("func") {
            return Err(WasmError::malformed("unsupported type form"));
        }
        let (params, results, _) = parse_typeuse_raw(&inner[1..])?;
        self.module.types.push(FuncType { params, results });
        Ok(())
    }

    fn resolve(&self, map: &HashMap<String, u32>, expr: &Sexpr) -> Result<u32> {
        let a = expr.atom().ok_or_else(|| WasmError::malformed("expected an index"))?;
        if let Some(name) = a.strip_prefix('$') {
            map.get(name).copied().ok_or_else(|| WasmError::malformed(format!("unknown index ${name}")))
        } else {
            parse_index_text(a)
        }
    }

    fn func_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.funcs, e)
    }
    fn table_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.tables, e)
    }
    fn mem_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.memories, e)
    }
    fn global_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.globals, e)
    }
    fn type_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.types, e)
    }
    fn elem_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.elems, e)
    }
    fn data_idx(&self, e: &Sexpr) -> Result<u32> {
        self.resolve(&self.names.datas, e)
    }

    fn local_idx(&self, e: &Sexpr) -> Result<u32> {
        let a = e.atom().ok_or_else(|| WasmError::malformed("expected a local"))?;
        if let Some(name) = a.strip_prefix('$') {
            self.locals.get(name).copied().ok_or_else(|| WasmError::malformed("unknown local"))
        } else {
            parse_index_text(a)
        }
    }

    fn label_depth(&self, e: &Sexpr) -> Result<u32> {
        let a = e.atom().ok_or_else(|| WasmError::malformed("expected a label"))?;
        if let Some(name) = a.strip_prefix('$') {
            for (i, label) in self.labels.iter().rev().enumerate() {
                if label.as_deref() == Some(name) {
                    return Ok(i as u32);
                }
            }
            Err(WasmError::invalid("unknown label"))
        } else {
            parse_index_text(a)
        }
    }

    fn intern_type(&mut self, ft: FuncType) -> u32 {
        if let Some(i) = self.module.types.iter().position(|t| *t == ft) {
            return i as u32;
        }
        self.module.types.push(ft);
        (self.module.types.len() - 1) as u32
    }

    fn block_type(&mut self, params: Vec<ValType>, results: Vec<ValType>, explicit: Option<u32>) -> BlockType {
        if let Some(t) = explicit {
            return BlockType::TypeIdx(t);
        }
        if params.is_empty() && results.is_empty() {
            BlockType::Empty
        } else if params.is_empty() && results.len() == 1 {
            BlockType::Value(results[0])
        } else {
            BlockType::TypeIdx(self.intern_type(FuncType { params, results }))
        }
    }

    fn parse_field(&mut self, field: &Sexpr) -> Result<()> {
        let list = field.list().ok_or_else(|| WasmError::malformed("expected a field"))?;
        let head = list.first().and_then(Sexpr::atom).unwrap_or("");
        match head {
            "import" => self.parse_import_field(list),
            "func" => self.parse_func(list),
            "table" => self.parse_table(list),
            "memory" => self.parse_memory(list),
            "global" => self.parse_global(list),
            "export" => self.parse_export_field(list),
            "start" => {
                if self.module.start.is_some() {
                    return Err(WasmError::malformed("multiple start sections"));
                }
                let idx = self.func_idx(list.get(1).ok_or_else(|| WasmError::malformed("malformed start"))?)?;
                self.module.start = Some(idx);
                Ok(())
            }
            "elem" => self.parse_elem(list),
            "data" => self.parse_data(list),
            "type" => Ok(()),
            _ => Err(WasmError::malformed(format!("unknown module field {head}"))),
        }
    }

    fn parse_import_field(&mut self, list: &[Sexpr]) -> Result<()> {
        let module = string_of(list.get(1))?;
        let name = string_of(list.get(2))?;
        let desc = list
            .get(3)
            .and_then(Sexpr::list)
            .ok_or_else(|| WasmError::malformed("malformed import"))?;
        self.push_import(&module, &name, desc)
    }

    fn push_import(&mut self, module: &str, name: &str, desc: &[Sexpr]) -> Result<()> {
        let kind = desc.first().and_then(Sexpr::atom).unwrap_or("");
        // Skip an optional `$id` in the descriptor.
        let start = match desc.get(1).and_then(Sexpr::atom) {
            Some(a) if a.starts_with('$') => 2,
            _ => 1,
        };
        let body = &desc[start..];
        let d = match kind {
            "func" => {
                let (params, results, ty) = self.typeuse(body)?;
                let ty = match ty {
                    Some(t) => t,
                    None => self.intern_type(FuncType { params, results }),
                };
                ImportDesc::Func(ty)
            }
            "table" => ImportDesc::Table(self.table_type(body)?),
            "memory" => ImportDesc::Memory(self.memory_type(body)?),
            "global" => ImportDesc::Global(self.global_type(body)?),
            _ => return Err(WasmError::malformed("malformed import kind")),
        };
        self.module.imports.push(Import { module: module.to_owned(), name: name.to_owned(), desc: d });
        Ok(())
    }

    /// Parse a table type from `[limits..., reftype]`.
    fn table_type(&self, items: &[Sexpr]) -> Result<TableType> {
        let (limits, rest) = parse_limits(items)?;
        if rest.len() != 1 {
            return Err(WasmError::malformed("malformed table type"));
        }
        let elem = rest[0]
            .atom()
            .and_then(ValType::parse)
            .filter(|t| t.is_ref())
            .ok_or_else(|| WasmError::malformed("malformed table type"))?;
        Ok(TableType { elem, limits })
    }

    fn memory_type(&self, items: &[Sexpr]) -> Result<MemoryType> {
        let (limits, rest) = parse_limits(items)?;
        if !rest.is_empty() {
            return Err(WasmError::malformed("malformed memory type"));
        }
        Ok(MemoryType { limits })
    }

    fn global_type(&self, items: &[Sexpr]) -> Result<GlobalType> {
        let (val, mutable) = parse_global_type(items.first())?;
        Ok(GlobalType { val, mutable })
    }

    fn parse_func(&mut self, list: &[Sexpr]) -> Result<()> {
        let index = self.func_cursor;
        self.func_cursor += 1;
        let mut exports = Vec::new();
        let mut import = None;
        let mut params = Vec::new();
        let mut results = Vec::new();
        let mut explicit_type = None;
        let mut locals = Vec::new();
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        // 0 = parameters, 1 = results, 2 = locals. The text format requires
        // that order.
        let mut phase = 0u8;
        while i < list.len() {
            match list[i].head() {
                Some("export") => {
                    exports.push(string_of(list[i].list().and_then(|l| l.get(1)))?);
                    i += 1;
                }
                Some("import") => {
                    let l = list[i].list().expect("import");
                    import = Some((string_of(l.get(1))?, string_of(l.get(2))?));
                    i += 1;
                }
                Some("type") => {
                    // `(type)` is only valid as the very first clause: before
                    // any `(param)`, `(result)` or `(local)`.
                    if phase != 0 || !params.is_empty() {
                        return Err(WasmError::malformed("type use after param, result or local"));
                    }
                    let t = self.type_idx(list[i].list().and_then(|l| l.get(1)).ok_or_else(|| WasmError::malformed("malformed type use"))?)?;
                    explicit_type = Some(t);
                    i += 1;
                }
                Some("param") => {
                    if phase != 0 {
                        return Err(WasmError::malformed("param after result or local"));
                    }
                    let l = list[i].list().expect("param");
                    params.extend(parse_named_group(&l[1..])?.1);
                    i += 1;
                }
                Some("result") => {
                    if phase == 2 {
                        return Err(WasmError::malformed("result after local"));
                    }
                    phase = 1;
                    let l = list[i].list().expect("result");
                    results.extend(parse_val_types(&l[1..])?);
                    i += 1;
                }
                Some("local") => {
                    phase = 2;
                    let l = list[i].list().expect("local");
                    locals.extend(parse_named_group(&l[1..])?.1);
                    i += 1;
                }
                _ => break,
            }
        }
        let type_idx = match explicit_type {
            Some(t) => {
                if !params.is_empty() || !results.is_empty() {
                    let ty = self
                        .module
                        .types
                        .get(t as usize)
                        .ok_or_else(|| WasmError::malformed("unknown type"))?;
                    if ty.params != params || ty.results != results {
                        return Err(WasmError::malformed("inline signature does not match type use"));
                    }
                }
                t
            }
            None => self.intern_type(FuncType { params: params.clone(), results: results.clone() }),
        };
        if let Some((module, name)) = import {
            self.module.imports.push(Import {
                module,
                name,
                desc: ImportDesc::Func(type_idx),
            });
            for e in exports {
                self.module.exports.push(Export { name: e, desc: ExportDesc::Func(index) });
            }
            return Ok(());
        }
        // Parse the body with parameters and locals in scope. When the
        // function used only `(type $sig)` with no inline `(param)` clauses,
        // `params` is empty even though the referenced type has parameters —
        // those parameters still occupy the low local indices, so named
        // locals declared in this function must start numbering after them.
        let param_count = if params.is_empty() {
            self.module.types.get(type_idx as usize).map_or(0, |t| t.params.len())
        } else {
            params.len()
        };
        self.locals = HashMap::new();
        self.nlocals = (param_count + locals.len()) as u32;
        // The field-iteration loop below walks the *literal* `(param)`/
        // `(local)` clauses in the text and numbers them from 0, which is
        // correct whenever any `(param)` clause is actually written (it
        // counts those itself). Only when there is none in the text — all
        // params came from an explicit `(type $sig)` reference — does
        // numbering need to start after that type's implicit param count.
        let mut li = if params.is_empty() { param_count as u32 } else { 0 };
        for field in list.iter().take(i).skip(1) {
            match field.head() {
                Some("param") | Some("local") => {
                    let l = field.list().expect("param/local");
                    let (names, tys) = parse_named_group(&l[1..])?;
                    for (k, n) in names.iter().enumerate() {
                        if let Some(n) = n {
                            if self.locals.insert(n.clone(), li + k as u32).is_some() {
                                return Err(WasmError::malformed(format!("duplicate local ${n}")));
                            }
                        }
                    }
                    li += tys.len() as u32;
                }
                _ => {}
            }
        }
        let body = self.parse_instrs(&list[i..], true)?;
        self.module.funcs.push(Func { type_idx, locals, body });
        for e in exports {
            self.module.exports.push(Export { name: e, desc: ExportDesc::Func(index) });
        }
        Ok(())
    }

    fn parse_table(&mut self, list: &[Sexpr]) -> Result<()> {
        let index = self.table_cursor;
        self.table_cursor += 1;
        let mut exports = Vec::new();
        let mut import = None;
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        while i < list.len() {
            match list[i].head() {
                Some("export") => {
                    exports.push(string_of(list[i].list().and_then(|l| l.get(1)))?);
                    i += 1;
                }
                Some("import") => {
                    let l = list[i].list().expect("import");
                    import = Some((string_of(l.get(1))?, string_of(l.get(2))?));
                    i += 1;
                }
                _ => break,
            }
        }
        // Either `limits reftype` or `reftype (elem ...)`.
        let (ty, elem_shorthand) = if let Some(reftype) = list.get(i).and_then(Sexpr::atom).and_then(ValType::parse) {
            if reftype.is_ref() && list.get(i + 1).and_then(Sexpr::head) == Some("elem") {
                let l = list[i + 1].list().expect("elem");
                let refs = self.parse_ref_items(&l[1..])?;
                let n = refs.len() as u32;
                (TableType { elem: reftype, limits: Limits { min: n, max: Some(n) } }, Some(refs))
            } else {
                (self.table_type(&list[i..])?, None)
            }
        } else {
            (self.table_type(&list[i..])?, None)
        };
        if let Some((module, name)) = import {
            self.module.imports.push(Import { module, name, desc: ImportDesc::Table(ty) });
        } else {
            self.module.tables.push(ty);
            if let Some(refs) = elem_shorthand {
                let seg = self.elem_cursor;
                self.elem_cursor += 1;
                self.module.elems.push(Elem {
                    ty: ty.elem,
                    init: refs.into_iter().map(|r| vec![r, Instr::End]).collect(),
                    mode: ElemMode::Active { table: index, offset: vec![Instr::I32Const(0), Instr::End] },
                });
                let _ = seg;
            }
        }
        for e in exports {
            self.module.exports.push(Export { name: e, desc: ExportDesc::Table(index) });
        }
        Ok(())
    }

    fn parse_memory(&mut self, list: &[Sexpr]) -> Result<()> {
        let index = self.mem_cursor;
        self.mem_cursor += 1;
        let mut exports = Vec::new();
        let mut import = None;
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        while i < list.len() {
            match list[i].head() {
                Some("export") => {
                    exports.push(string_of(list[i].list().and_then(|l| l.get(1)))?);
                    i += 1;
                }
                Some("import") => {
                    let l = list[i].list().expect("import");
                    import = Some((string_of(l.get(1))?, string_of(l.get(2))?));
                    i += 1;
                }
                _ => break,
            }
        }
        let (ty, data_shorthand) = if list.get(i).and_then(Sexpr::head) == Some("data") {
            let l = list[i].list().expect("data");
            let bytes = concat_strings(&l[1..])?;
            let pages = bytes.len().div_ceil(crate::store::PAGE_SIZE) as u32;
            (
                MemoryType { limits: Limits { min: pages, max: Some(pages) } },
                Some(bytes),
            )
        } else {
            (self.memory_type(&list[i..])?, None)
        };
        if let Some((module, name)) = import {
            self.module.imports.push(Import { module, name, desc: ImportDesc::Memory(ty) });
        } else {
            self.module.memories.push(ty);
            if let Some(bytes) = data_shorthand {
                self.data_cursor += 1;
                self.module.datas.push(Data {
                    init: bytes,
                    mode: DataMode::Active { memory: index, offset: vec![Instr::I32Const(0), Instr::End] },
                });
            }
        }
        for e in exports {
            self.module.exports.push(Export { name: e, desc: ExportDesc::Memory(index) });
        }
        Ok(())
    }

    fn parse_global(&mut self, list: &[Sexpr]) -> Result<()> {
        let index = self.global_cursor;
        self.global_cursor += 1;
        let mut exports = Vec::new();
        let mut import = None;
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        while i < list.len() {
            match list[i].head() {
                Some("export") => {
                    exports.push(string_of(list[i].list().and_then(|l| l.get(1)))?);
                    i += 1;
                }
                Some("import") => {
                    let l = list[i].list().expect("import");
                    import = Some((string_of(l.get(1))?, string_of(l.get(2))?));
                    i += 1;
                }
                _ => break,
            }
        }
        let ty = self.global_type(&list[i..])?;
        i += 1;
        if let Some((module, name)) = import {
            self.module.imports.push(Import { module, name, desc: ImportDesc::Global(ty) });
        } else {
            let init = self.parse_instrs(&list[i..], true)?;
            self.module.globals.push(Global { ty, init });
        }
        for e in exports {
            self.module.exports.push(Export { name: e, desc: ExportDesc::Global(index) });
        }
        Ok(())
    }

    fn parse_export_field(&mut self, list: &[Sexpr]) -> Result<()> {
        let name = string_of(list.get(1))?;
        let desc = list
            .get(2)
            .and_then(Sexpr::list)
            .ok_or_else(|| WasmError::malformed("malformed export"))?;
        let kind = desc.first().and_then(Sexpr::atom).unwrap_or("");
        let idx = desc.get(1).ok_or_else(|| WasmError::malformed("malformed export"))?;
        let d = match kind {
            "func" => ExportDesc::Func(self.func_idx(idx)?),
            "table" => ExportDesc::Table(self.table_idx(idx)?),
            "memory" => ExportDesc::Memory(self.mem_idx(idx)?),
            "global" => ExportDesc::Global(self.global_idx(idx)?),
            _ => return Err(WasmError::malformed("malformed export kind")),
        };
        self.module.exports.push(Export { name, desc: d });
        Ok(())
    }

    fn parse_elem(&mut self, list: &[Sexpr]) -> Result<()> {
        let seg_index = self.elem_cursor;
        self.elem_cursor += 1;
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        let mut table = 0u32;
        let mut offset: Option<Vec<Instr>> = None;
        let mut ty = ValType::FuncRef;
        let mut declarative = false;
        if list.get(i).and_then(Sexpr::atom) == Some("declare") {
            declarative = true;
            i += 1;
        } else {
            if let Some(l) = list.get(i).and_then(Sexpr::list) {
                if l.first().and_then(Sexpr::atom) == Some("table") {
                    table = self.table_idx(l.get(1).ok_or_else(|| WasmError::malformed("malformed elem"))?)?;
                    i += 1;
                }
            }
            // An `(offset ...)`, or any other leading folded expression: the
            // old MVP shorthand `(elem <expr> <item>*)` takes ANY expression
            // (not just `i32.const`/`global.get`) as the offset of an
            // implicit active segment at table 0 — including a wrongly typed
            // one, so e.g. `(elem (ref.null func))` must surface as a type
            // mismatch (offset must be i32) rather than be reinterpreted as
            // a passive segment with one `funcref` item. Only an explicit
            // `(item ...)` wrapper marks a passive/declarative item here;
            // an unwrapped, unmarked passive item needs a preceding reftype
            // atom (`func`/`funcref`/`extern`/`externref`) instead.
            if let Some(l) = list.get(i).and_then(Sexpr::list) {
                match l.first().and_then(Sexpr::atom) {
                    Some("offset") => {
                        offset = Some(self.parse_instrs(&l[1..], true)?);
                        i += 1;
                    }
                    Some("item") => {}
                    _ => {
                        offset = Some(self.parse_instrs(std::slice::from_ref(&list[i]), true)?);
                        i += 1;
                    }
                }
            }
        }
        // Optional `func` or reference type keyword.
        if let Some(a) = list.get(i).and_then(Sexpr::atom) {
            if a == "func" {
                ty = ValType::FuncRef;
                i += 1;
            } else if let Some(t) = ValType::parse(a) {
                if t.is_ref() {
                    ty = t;
                    i += 1;
                }
            }
        }
        let exprs = self.parse_ref_exprs(&list[i..])?;
        let mode = if declarative {
            ElemMode::Declarative
        } else if let Some(offset) = offset {
            ElemMode::Active { table, offset }
        } else {
            ElemMode::Passive
        };
        let _ = seg_index;
        self.module.elems.push(Elem { ty, init: exprs, mode });
        Ok(())
    }

    /// Parse element initializer expressions: bare funcidx atoms, `(item ...)`
    /// wrappers, and `ref.func`/`ref.null` folded expressions.
    fn parse_ref_exprs(&mut self, items: &[Sexpr]) -> Result<Vec<Vec<Instr>>> {
        let mut out = Vec::new();
        for item in items {
            match item {
                Sexpr::Atom(_) => {
                    let idx = self.func_idx(item)?;
                    out.push(vec![Instr::RefFunc(idx), Instr::End]);
                }
                Sexpr::List(l) => {
                    if l.first().and_then(Sexpr::atom) == Some("item") {
                        out.push(self.parse_instrs(&l[1..], true)?);
                    } else {
                        out.push(self.parse_instrs(std::slice::from_ref(item), true)?);
                    }
                }
                Sexpr::Str(_) => return Err(WasmError::malformed("string in elem segment")),
            }
        }
        Ok(out)
    }

    /// Parse the ref items of a `(table reftype (elem ...))` shorthand.
    fn parse_ref_items(&mut self, items: &[Sexpr]) -> Result<Vec<Instr>> {
        let exprs = self.parse_ref_exprs(items)?;
        let mut out = Vec::new();
        for mut e in exprs {
            if e.last() == Some(&Instr::End) {
                e.pop();
            }
            out.extend(e);
        }
        Ok(out)
    }

    fn parse_data(&mut self, list: &[Sexpr]) -> Result<()> {
        self.data_cursor += 1;
        let mut i = 1;
        if optional_name(list).is_some() {
            i = 2;
        }
        let mut memory = 0u32;
        if let Some(l) = list.get(i).and_then(Sexpr::list) {
            if l.first().and_then(Sexpr::atom) == Some("memory") {
                memory = self.mem_idx(l.get(1).ok_or_else(|| WasmError::malformed("malformed data"))?)?;
                i += 1;
            }
        }
        let mut offset: Option<Vec<Instr>> = None;
        if let Some(l) = list.get(i).and_then(Sexpr::list) {
            match l.first().and_then(Sexpr::atom) {
                Some("offset") => {
                    offset = Some(self.parse_instrs(&l[1..], true)?);
                    i += 1;
                }
                Some("i32.const") | Some("global.get") => {
                    offset = Some(self.parse_instrs(&[list[i].clone()], true)?);
                    i += 1;
                }
                _ => {}
            }
        }
        let bytes = concat_strings(&list[i..])?;
        let mode = match offset {
            Some(offset) => DataMode::Active { memory, offset },
            None => DataMode::Passive,
        };
        self.module.datas.push(Data { init: bytes, mode });
        Ok(())
    }

    /// Parse an instruction sequence. When `append_end` is set, a terminating
    /// `end` is added (function bodies and constant expressions).
    fn parse_instrs(&mut self, items: &[Sexpr], append_end: bool) -> Result<Vec<Instr>> {
        let saved_labels = std::mem::take(&mut self.labels);
        let saved_locals = self.locals.clone();
        let mut out = Vec::new();
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                Sexpr::List(l) => {
                    self.parse_folded(l, &mut out)?;
                    i += 1;
                }
                Sexpr::Atom(a) => self.parse_plain(a, items, &mut i, &mut out)?,
                Sexpr::Str(_) => return Err(WasmError::malformed("string in instruction position")),
            }
        }
        if !self.labels.is_empty() {
            return Err(WasmError::malformed("unclosed block"));
        }
        if append_end {
            out.push(Instr::End);
        }
        self.labels = saved_labels;
        self.locals = saved_locals;
        Ok(out)
    }

    fn parse_plain(&mut self, op: &str, items: &[Sexpr], i: &mut usize, out: &mut Vec<Instr>) -> Result<()> {
        *i += 1;
        match op {
            "block" | "loop" => {
                let (label, bt) = self.parse_structural_header(items, i)?;
                out.push(if op == "block" { Instr::Block(bt) } else { Instr::Loop(bt) });
                self.labels.push(label);
            }
            "if" => {
                let (label, bt) = self.parse_structural_header(items, i)?;
                out.push(Instr::If(bt));
                self.labels.push(label);
            }
            "else" => {
                let expected = self.labels.last().and_then(Option::as_deref);
                self.consume_matching_label(items, i, expected)?;
                out.push(Instr::Else);
            }
            "end" => {
                let closed = self.labels.pop().flatten();
                self.consume_matching_label(items, i, closed.as_deref())?;
                out.push(Instr::End);
            }
            _ => {
                let start = *i;
                self.consume_immediates(op, items, i)?;
                let instr = self.build(op, &items[start..*i])?;
                out.push(instr);
            }
        }
        Ok(())
    }

    /// In the unfolded (flat) instruction syntax, `else` and `end` may
    /// optionally repeat the label of the block they close (`if $body ...
    /// else $body ... end $body`), purely for readability; it must match the
    /// label the block was opened with (or be absent if the block was
    /// unlabeled).
    fn consume_matching_label(&self, items: &[Sexpr], i: &mut usize, expected: Option<&str>) -> Result<()> {
        if let Some(Sexpr::Atom(a)) = items.get(*i) {
            if let Some(name) = a.strip_prefix('$') {
                if expected != Some(name) {
                    return Err(WasmError::malformed("mismatching label"));
                }
                *i += 1;
            }
        }
        Ok(())
    }

    fn parse_structural_header(
        &mut self,
        items: &[Sexpr],
        i: &mut usize,
    ) -> Result<(Option<String>, BlockType)> {
        let mut label = None;
        if let Some(Sexpr::Atom(a)) = items.get(*i) {
            if let Some(name) = a.strip_prefix('$') {
                label = Some(name.to_owned());
                *i += 1;
            }
        }
        let mut j = *i;
        while matches!(items.get(j).and_then(Sexpr::head), Some("type") | Some("param") | Some("result")) {
            j += 1;
        }
        for item in &items[*i..j] {
            if item.head() == Some("param") {
                let l = item.list().expect("param");
                let (names, _) = parse_named_group(&l[1..])?;
                if names.iter().any(Option::is_some) {
                    return Err(WasmError::malformed("named block parameter"));
                }
            }
        }
        let (params, results, explicit) = self.typeuse(&items[*i..j])?;
        *i = j;
        let bt = self.block_type(params, results, explicit);
        Ok((label, bt))
    }

    fn parse_folded(&mut self, list: &[Sexpr], out: &mut Vec<Instr>) -> Result<()> {
        let head = list.first().and_then(Sexpr::atom).unwrap_or("");
        match head {
            "block" | "loop" => {
                let mut i = 1;
                let (label, bt) = self.parse_structural_header(list, &mut i)?;
                out.push(if head == "block" { Instr::Block(bt) } else { Instr::Loop(bt) });
                self.labels.push(label);
                self.parse_instrs_into(&list[i..], out)?;
                self.labels.pop();
                out.push(Instr::End);
            }
            "if" => {
                let mut i = 1;
                let (label, bt) = self.parse_structural_header(list, &mut i)?;
                // Condition operands precede `(then ...)`.
                let then_at = list[i..]
                    .iter()
                    .position(|x| x.head() == Some("then"))
                    .map(|p| p + i)
                    .ok_or_else(|| WasmError::malformed("folded if without then"))?;
                for cond in &list[i..then_at] {
                    let l = cond.list().ok_or_else(|| WasmError::malformed("if condition must be folded"))?;
                    self.parse_folded(l, out)?;
                }
                out.push(Instr::If(bt));
                self.labels.push(label);
                let then_list = list[then_at].list().expect("then");
                self.parse_instrs_into(&then_list[1..], out)?;
                let else_at = list.iter().position(|x| x.head() == Some("else"));
                if let Some(ea) = else_at {
                    out.push(Instr::Else);
                    let else_list = list[ea].list().expect("else");
                    self.parse_instrs_into(&else_list[1..], out)?;
                }
                self.labels.pop();
                out.push(Instr::End);
            }
            _ => {
                let mut i = 1;
                if head == "select" {
                    while list.get(i).and_then(Sexpr::head) == Some("result") {
                        i += 1;
                    }
                }
                self.consume_immediates(head, list, &mut i)?;
                let instr = self.build(head, &list[1..i])?;
                for operand in &list[i..] {
                    let l = operand
                        .list()
                        .ok_or_else(|| WasmError::malformed("expected a folded operand"))?;
                    self.parse_folded(l, out)?;
                }
                out.push(instr);
            }
        }
        Ok(())
    }

    /// Parse a sequence of instructions without adding a terminating `end`.
    fn parse_instrs_into(&mut self, items: &[Sexpr], out: &mut Vec<Instr>) -> Result<()> {
        let mut i = 0;
        while i < items.len() {
            match &items[i] {
                Sexpr::List(l) => {
                    self.parse_folded(l, out)?;
                    i += 1;
                }
                Sexpr::Atom(a) => self.parse_plain(a, items, &mut i, out)?,
                Sexpr::Str(_) => return Err(WasmError::malformed("string in instruction position")),
            }
        }
        Ok(())
    }

    fn consume_immediates(&self, op: &str, items: &[Sexpr], i: &mut usize) -> Result<()> {
        let get = |i: usize| items.get(i);
        let advance_index = |i: &mut usize| {
            if get(*i).map(is_index).unwrap_or(false) {
                *i += 1;
                true
            } else {
                false
            }
        };
        match op {
            "br" | "br_if" => {
                if !advance_index(i) {
                    return Err(WasmError::malformed("branch without a label"));
                }
            }
            "br_table" => {
                let mut count = 0;
                while advance_index(i) {
                    count += 1;
                }
                if count == 0 {
                    return Err(WasmError::malformed("br_table without labels"));
                }
            }
            "call" => {
                if !advance_index(i) {
                    return Err(WasmError::malformed("call without a function"));
                }
            }
            "call_indirect" => {
                while advance_index(i) {}
                while matches!(get(*i).and_then(Sexpr::head), Some("type") | Some("param") | Some("result")) {
                    *i += 1;
                }
            }
            "local.get" | "local.set" | "local.tee" | "global.get" | "global.set" => {
                if !advance_index(i) {
                    return Err(WasmError::malformed("instruction without an index"));
                }
            }
            "table.get" | "table.set" | "table.size" | "table.grow" | "table.fill" | "elem.drop" => {
                advance_index(i);
            }
            "table.init" | "table.copy" => {
                advance_index(i);
                advance_index(i);
            }
            "memory.init" | "data.drop" => {
                advance_index(i);
            }
            "memory.copy" => {
                advance_index(i);
                advance_index(i);
            }
            "memory.fill" | "memory.size" | "memory.grow" => {
                advance_index(i);
            }
            "select" => {
                // The unfolded typed form `select (result t)` carries a
                // trailing type immediate that `build` looks for; without
                // consuming it here it would be parsed as the next,
                // unrelated instruction (`(result ...)` is not one). Like
                // other typeuse positions, `(result ...)` clauses may repeat
                // and are concatenated.
                while matches!(get(*i).and_then(Sexpr::head), Some("result")) {
                    *i += 1;
                }
            }
            "i32.const" | "i64.const" | "f32.const" | "f64.const" => {
                if get(*i).and_then(Sexpr::atom).is_none() {
                    return Err(WasmError::malformed("constant without a value"));
                }
                *i += 1;
            }
            "ref.null" | "ref.func" => {
                if get(*i).and_then(Sexpr::atom).is_none() {
                    return Err(WasmError::malformed("ref instruction without an operand"));
                }
                *i += 1;
            }
            _ => {
                if opname::load(op).is_some() || opname::store(op).is_some() || op.starts_with("v128.") {
                    while let Some(a) = get(*i).and_then(Sexpr::atom) {
                        if a.starts_with("offset=") || a.starts_with("align=") {
                            *i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn build(&mut self, op: &str, imm: &[Sexpr]) -> Result<Instr> {
        // Structural instructions are handled before `build`.
        if let Some(n) = opname::num(op) {
            return Ok(Instr::Num(n));
        }
        if let Some(l) = opname::load(op) {
            let m = parse_memarg(imm, l.width())?;
            return Ok(Instr::Load(l, m));
        }
        if let Some(s) = opname::store(op) {
            let m = parse_memarg(imm, s.width())?;
            return Ok(Instr::Store(s, m));
        }
        Ok(match op {
            "unreachable" => Instr::Unreachable,
            "nop" => Instr::Nop,
            "return" => Instr::Return,
            "drop" => Instr::Drop,
            "select" => {
                // Concatenate every `(result ...)` clause, mirroring block
                // types and `call_indirect`'s typeuse (a module can legally
                // repeat, or even pass an empty `(result)`, clause).
                let mut types = Vec::new();
                for x in imm.iter().filter(|x| x.head() == Some("result")) {
                    types.extend(parse_val_types(&x.list().expect("result")[1..])?);
                }
                if imm.iter().any(|x| x.head() == Some("result")) {
                    Instr::SelectT(types.into_boxed_slice())
                } else {
                    Instr::Select
                }
            }
            "ref.is_null" => Instr::RefIsNull,
            "br" => Instr::Br(self.label_depth(&imm[0])?),
            "br_if" => Instr::BrIf(self.label_depth(&imm[0])?),
            "br_table" => {
                let mut labels = Vec::new();
                for e in imm {
                    labels.push(self.label_depth(e)?);
                }
                if labels.is_empty() {
                    return Err(WasmError::malformed("br_table without labels"));
                }
                let default = labels.pop().expect("non-empty");
                Instr::BrTable(labels.into_boxed_slice(), default)
            }
            "call" => Instr::Call(self.func_idx(&imm[0])?),
            "call_indirect" => {
                let mut table = 0u32;
                let mut ty = None;
                let mut params = Vec::new();
                let mut results = Vec::new();
                let mut seen_result = false;
                for e in imm {
                    match e.head() {
                        Some("type") => {
                            // `(type)` must be the first typeuse clause: before
                            // any `(param)`/`(result)` (a table index, if
                            // present, still comes before all of them).
                            if !params.is_empty() || seen_result {
                                return Err(WasmError::malformed("type use after param or result"));
                            }
                            ty = Some(self.type_idx(
                                e.list().and_then(|l| l.get(1)).ok_or_else(|| WasmError::malformed("malformed type use"))?,
                            )?);
                        }
                        Some("param") => {
                            if seen_result {
                                return Err(WasmError::malformed("param after result"));
                            }
                            params.extend(parse_val_types(&e.list().expect("param")[1..])?);
                        }
                        Some("result") => {
                            seen_result = true;
                            results.extend(parse_val_types(&e.list().expect("result")[1..])?);
                        }
                        _ => table = self.table_idx(e)?,
                    }
                }
                let ty = match ty {
                    Some(t) => {
                        if !params.is_empty() || !results.is_empty() {
                            let expected = self
                                .module
                                .types
                                .get(t as usize)
                                .ok_or_else(|| WasmError::malformed("unknown type"))?;
                            if expected.params != params || expected.results != results {
                                return Err(WasmError::malformed("inline signature does not match type use"));
                            }
                        }
                        t
                    }
                    None => self.intern_type(FuncType { params, results }),
                };
                Instr::CallIndirect { ty, table }
            }
            "local.get" => Instr::LocalGet(self.local_idx(&imm[0])?),
            "local.set" => Instr::LocalSet(self.local_idx(&imm[0])?),
            "local.tee" => Instr::LocalTee(self.local_idx(&imm[0])?),
            "global.get" => Instr::GlobalGet(self.global_idx(&imm[0])?),
            "global.set" => Instr::GlobalSet(self.global_idx(&imm[0])?),
            "table.get" => Instr::TableGet(imm.first().map(|e| self.table_idx(e)).transpose()?.unwrap_or(0)),
            "table.set" => Instr::TableSet(imm.first().map(|e| self.table_idx(e)).transpose()?.unwrap_or(0)),
            "table.size" => Instr::TableSize(imm.first().map(|e| self.table_idx(e)).transpose()?.unwrap_or(0)),
            "table.grow" => Instr::TableGrow(imm.first().map(|e| self.table_idx(e)).transpose()?.unwrap_or(0)),
            "table.fill" => Instr::TableFill(imm.first().map(|e| self.table_idx(e)).transpose()?.unwrap_or(0)),
            "table.init" => {
                // `table.init table elem`, with the table defaulting to 0 when
                // only the element segment is given.
                let (table, elem) = match imm.len() {
                    0 => return Err(WasmError::malformed("table.init needs an element segment")),
                    1 => (0, self.elem_idx(&imm[0])?),
                    _ => (self.table_idx(&imm[0])?, self.elem_idx(&imm[1])?),
                };
                Instr::TableInit { table, elem }
            }
            "elem.drop" => Instr::ElemDrop(self.elem_idx(imm.first().ok_or_else(|| WasmError::malformed("elem.drop needs an elem"))?)?),
            "table.copy" => Instr::TableCopy {
                dst: match imm.first() {
                    Some(e) => self.table_idx(e)?,
                    None => 0,
                },
                src: match imm.get(1) {
                    Some(e) => self.table_idx(e)?,
                    None => 0,
                },
            },
            "memory.init" => Instr::MemoryInit {
                data: self.data_idx(imm.first().ok_or_else(|| WasmError::malformed("memory.init needs a data segment"))?)?,
                memory: if imm.len() > 1 { self.mem_idx(&imm[1])? } else { 0 },
            },
            "data.drop" => Instr::DataDrop(self.data_idx(imm.first().ok_or_else(|| WasmError::malformed("data.drop needs a data segment"))?)?),
            "memory.copy" => Instr::MemoryCopy {
                dst: match imm.first() {
                    Some(e) => self.mem_idx(e)?,
                    None => 0,
                },
                src: match imm.get(1) {
                    Some(e) => self.mem_idx(e)?,
                    None => 0,
                },
            },
            "memory.fill" => Instr::MemoryFill(if !imm.is_empty() { self.mem_idx(&imm[0])? } else { 0 }),
            "memory.size" => Instr::MemorySize(if !imm.is_empty() { self.mem_idx(&imm[0])? } else { 0 }),
            "memory.grow" => Instr::MemoryGrow(if !imm.is_empty() { self.mem_idx(&imm[0])? } else { 0 }),
            "i32.const" => Instr::I32Const(numlit::parse_i32(imm[0].atom().expect("i32 literal"))?),
            "i64.const" => Instr::I64Const(numlit::parse_i64(imm[0].atom().expect("i64 literal"))?),
            "f32.const" => Instr::F32Const(numlit::parse_f32(imm[0].atom().expect("f32 literal"))?.to_bits()),
            "f64.const" => Instr::F64Const(numlit::parse_f64(imm[0].atom().expect("f64 literal"))?.to_bits()),
            "ref.null" => Instr::RefNull(ref_type(imm[0].atom().expect("ref type"))?),
            "ref.func" => Instr::RefFunc(self.func_idx(&imm[0])?),
            _ => return Err(WasmError::malformed(format!("unknown instruction {op}"))),
        })
    }

    /// Parse a type use: `(type x)? (param ...)* (result ...)*`, resolving a
    /// named `(type $t)` against the type index space.
    fn typeuse(&mut self, items: &[Sexpr]) -> Result<(Vec<ValType>, Vec<ValType>, Option<u32>)> {
        let mut params = Vec::new();
        let mut results = Vec::new();
        let mut explicit = None;
        let mut seen_result = false;
        for item in items {
            match item.head() {
                Some("type") => {
                    let l = item.list().expect("type");
                    let a = l
                        .get(1)
                        .ok_or_else(|| WasmError::malformed("malformed type use"))?;
                    explicit = Some(self.type_idx(a)?);
                }
                Some("param") => {
                    if seen_result {
                        return Err(WasmError::malformed("param after result"));
                    }
                    let l = item.list().expect("param");
                    params.extend(parse_named_group(&l[1..])?.1);
                }
                Some("result") => {
                    seen_result = true;
                    let l = item.list().expect("result");
                    results.extend(parse_val_types(&l[1..])?);
                }
                _ => break,
            }
        }
        if let Some(t) = explicit {
            if !params.is_empty() || !results.is_empty() {
                let ty = self
                    .module
                    .types
                    .get(t as usize)
                    .ok_or_else(|| WasmError::malformed("unknown type"))?;
                if ty.params != params || ty.results != results {
                    return Err(WasmError::malformed("inline signature does not match type use"));
                }
            }
        }
        Ok((params, results, explicit))
    }
}

fn ref_type(a: &str) -> Result<ValType> {
    match a {
        "func" | "funcref" => Ok(ValType::FuncRef),
        "extern" | "externref" => Ok(ValType::ExternRef),
        _ => Err(WasmError::malformed("malformed reference type")),
    }
}

fn parse_typeuse_raw(items: &[Sexpr]) -> Result<(Vec<ValType>, Vec<ValType>, Option<u32>)> {
    let mut params = Vec::new();
    let mut results = Vec::new();
    let mut explicit = None;
    let mut seen_result = false;
    for item in items {
        match item.head() {
            Some("type") => {
                let l = item.list().expect("type");
                let a = l.get(1).and_then(Sexpr::atom).ok_or_else(|| WasmError::malformed("malformed type use"))?;
                if let Some(name) = a.strip_prefix('$') {
                    // Names cannot be resolved here; the caller resolves it.
                    return Err(WasmError::malformed(format!("unresolved type ${name}")));
                }
                explicit = Some(parse_index_text(a)?);
            }
            Some("param") => {
                if seen_result {
                    return Err(WasmError::malformed("param after result"));
                }
                let l = item.list().expect("param");
                params.extend(parse_named_group(&l[1..])?.1);
            }
            Some("result") => {
                seen_result = true;
                let l = item.list().expect("result");
                results.extend(parse_val_types(&l[1..])?);
            }
            _ => break,
        }
    }
    Ok((params, results, explicit))
}

/// Parse `$name? type*`, returning the names and the types.
fn parse_named_group(items: &[Sexpr]) -> Result<(Vec<Option<String>>, Vec<ValType>)> {
    let mut names = Vec::new();
    let mut tys = Vec::new();
    let mut i = 0;
    if let Some(Sexpr::Atom(a)) = items.first() {
        if let Some(n) = a.strip_prefix('$') {
            names.push(Some(n.to_owned()));
            if let Some(t) = items.get(1).and_then(Sexpr::atom).and_then(ValType::parse) {
                tys.push(t);
                return Ok((names, tys));
            }
            return Err(WasmError::malformed("named param without a type"));
        }
    }
    while i < items.len() {
        let t = items[i]
            .atom()
            .and_then(ValType::parse)
            .ok_or_else(|| WasmError::malformed("malformed value type"))?;
        tys.push(t);
        names.push(None);
        i += 1;
    }
    Ok((names, tys))
}

fn parse_val_types(items: &[Sexpr]) -> Result<Vec<ValType>> {
    items
        .iter()
        .map(|x| {
            x.atom()
                .and_then(ValType::parse)
                .ok_or_else(|| WasmError::malformed("malformed value type"))
        })
        .collect()
}

fn parse_global_type(item: Option<&Sexpr>) -> Result<(ValType, bool)> {
    let item = item.ok_or_else(|| WasmError::malformed("malformed global type"))?;
    if let Some(l) = item.list() {
        if l.first().and_then(Sexpr::atom) == Some("mut") {
            let t = l.get(1).and_then(Sexpr::atom).and_then(ValType::parse);
            return t.map(|t| (t, true)).ok_or_else(|| WasmError::malformed("malformed global type"));
        }
    }
    item.atom()
        .and_then(ValType::parse)
        .map(|t| (t, false))
        .ok_or_else(|| WasmError::malformed("malformed global type"))
}

fn parse_limits(items: &[Sexpr]) -> Result<(Limits, &[Sexpr])> {
    let mut nums = Vec::new();
    let mut i = 0;
    while i < items.len() {
        match items[i].atom() {
            Some(a) if parse_index_text(a).is_ok() => {
                nums.push(parse_index_text(a)?);
                i += 1;
            }
            _ => break,
        }
    }
    match nums.len() {
        1 => Ok((Limits { min: nums[0], max: None }, &items[i..])),
        2 => Ok((Limits { min: nums[0], max: Some(nums[1]) }, &items[i..])),
        _ => Err(WasmError::malformed("malformed limits")),
    }
}

fn parse_memarg(imm: &[Sexpr], width: u32) -> Result<MemArg> {
    let mut offset = 0u32;
    let mut align: Option<u32> = None;
    for item in imm {
        let a = item.atom().ok_or_else(|| WasmError::malformed("malformed memarg"))?;
        if let Some(v) = a.strip_prefix("offset=") {
            offset = parse_index_text(v)?;
        } else if let Some(v) = a.strip_prefix("align=") {
            let bytes = parse_index_text(v)?;
            if bytes == 0 || !bytes.is_power_of_two() {
                return Err(WasmError::malformed("alignment must be a power of two"));
            }
            align = Some(bytes.trailing_zeros());
        }
    }
    let align = align.unwrap_or_else(|| width.trailing_zeros());
    Ok(MemArg { align, offset, memory: 0 })
}

fn concat_strings(items: &[Sexpr]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for item in items {
        let b = item.bytes().ok_or_else(|| WasmError::malformed("expected a string"))?;
        out.extend_from_slice(b);
    }
    Ok(out)
}

fn string_of(item: Option<&Sexpr>) -> Result<String> {
    let bytes = item
        .and_then(Sexpr::bytes)
        .ok_or_else(|| WasmError::malformed("expected a string"))?;
    String::from_utf8(bytes.to_vec()).map_err(|_| WasmError::malformed("malformed UTF-8"))
}
