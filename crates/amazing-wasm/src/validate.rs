//! The type-checking validator.
//!
//! Implements the specification's validation algorithm (appendix "Validation
//! Algorithm"): an operand stack of types plus a stack of control frames with
//! a `height` and an `unreachable` flag, so code after `unreachable`, `br` or
//! `return` is checked in the polymorphic bottom type. Module-level checks
//! (limits, exports, constant expressions, `ref.func` declaration) live in
//! [`validate_module`].

use std::collections::HashSet;

use crate::error::{Result, WasmError};
use crate::instr::{Instr, NumOp, NumSig};
use crate::module::{ExportDesc, ImportDesc, Module};
use crate::types::{BlockType, FuncType, GlobalType, MemoryType, TableType, ValType};

/// 65536 pages, the 32-bit linear-memory ceiling.
const MAX_PAGES: u32 = 65536;

/// An operand type, possibly the polymorphic bottom type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opd {
    Known(ValType),
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CtrlKind {
    Block,
    Loop,
    If,
    Else,
    Func,
}

struct Ctrl {
    kind: CtrlKind,
    start: Vec<ValType>,
    end: Vec<ValType>,
    height: usize,
    unreachable: bool,
}

struct FnValidator<'a> {
    types: &'a [FuncType],
    funcs: &'a [u32],
    tables: &'a [TableType],
    memories: &'a [MemoryType],
    globals: &'a [GlobalType],
    refs: &'a HashSet<u32>,
    data_count: Option<u32>,
    elem_types: Vec<ValType>,
    locals: Vec<ValType>,
    stack: Vec<Opd>,
    ctrls: Vec<Ctrl>,
}

impl<'a> FnValidator<'a> {
    fn push(&mut self, t: ValType) {
        self.stack.push(Opd::Known(t));
    }

    fn push_opd(&mut self, o: Opd) {
        self.stack.push(o);
    }

    fn push_vals(&mut self, ts: &[ValType]) {
        for &t in ts {
            self.push(t);
        }
    }

    fn pop(&mut self) -> Result<Opd> {
        let frame = self.ctrls.last().expect("control stack is never empty");
        if self.stack.len() == frame.height {
            if frame.unreachable {
                return Ok(Opd::Unknown);
            }
            return Err(WasmError::invalid("type mismatch: expected a value but the stack is empty"));
        }
        Ok(self.stack.pop().expect("stack above frame height"))
    }

    fn pop_expect(&mut self, t: ValType) -> Result<()> {
        match self.pop()? {
            Opd::Unknown => Ok(()),
            Opd::Known(u) if u == t => Ok(()),
            Opd::Known(u) => Err(WasmError::invalid(format!(
                "type mismatch: expected {}, found {}",
                t.name(),
                u.name()
            ))),
        }
    }

    fn pop_vals(&mut self, ts: &[ValType]) -> Result<()> {
        for &t in ts.iter().rev() {
            self.pop_expect(t)?;
        }
        Ok(())
    }

    /// Check that `ts` would be poppable without mutating the stack.
    ///
    /// Used by `br_table`'s non-default targets: each target must be
    /// independently compatible with the operand stack, but per the
    /// specification's "meet" semantics for polymorphic (unreachable) code,
    /// checking one target must not leave residue that corrupts the check for
    /// the next target when two labels happen to have the same arity but
    /// different concrete value types (only possible when the actual operand
    /// is the unknown bottom type, so no mismatch can ever be observed at
    /// run time).
    fn check_vals(&self, ts: &[ValType]) -> Result<()> {
        let frame = self.ctrls.last().expect("control stack is never empty");
        let mut idx = self.stack.len();
        for &t in ts.iter().rev() {
            if idx == frame.height {
                if frame.unreachable {
                    continue;
                }
                return Err(WasmError::invalid("type mismatch: expected a value but the stack is empty"));
            }
            idx -= 1;
            match self.stack[idx] {
                Opd::Unknown => {}
                Opd::Known(u) if u == t => {}
                Opd::Known(u) => {
                    return Err(WasmError::invalid(format!(
                        "type mismatch: expected {}, found {}",
                        t.name(),
                        u.name()
                    )))
                }
            }
        }
        Ok(())
    }

    fn push_ctrl(&mut self, kind: CtrlKind, start: Vec<ValType>, end: Vec<ValType>) {
        let height = self.stack.len();
        self.ctrls.push(Ctrl { kind, start: start.clone(), end, height, unreachable: false });
        self.push_vals(&start);
    }

    fn pop_ctrl(&mut self) -> Result<Ctrl> {
        let frame = self.ctrls.last().ok_or_else(|| WasmError::invalid("type mismatch"))?;
        let (height, end) = (frame.height, frame.end.clone());
        self.pop_vals(&end)?;
        if self.stack.len() != height {
            return Err(WasmError::invalid("type mismatch: values remaining on stack at end of block"));
        }
        let frame = self.ctrls.pop().expect("checked non-empty");
        Ok(frame)
    }

    fn unreachable(&mut self) {
        let height = self.ctrls.last().expect("frame").height;
        self.stack.truncate(height);
        self.ctrls.last_mut().expect("frame").unreachable = true;
    }

    fn label_types(&self, depth: u32) -> Result<Vec<ValType>> {
        let idx = self
            .ctrls
            .len()
            .checked_sub(depth as usize + 1)
            .ok_or_else(|| WasmError::invalid("unknown label"))?;
        let frame = &self.ctrls[idx];
        Ok(if frame.kind == CtrlKind::Loop { frame.start.clone() } else { frame.end.clone() })
    }

    fn block_type(&self, bt: BlockType) -> Result<(Vec<ValType>, Vec<ValType>)> {
        match bt {
            BlockType::Empty => Ok((Vec::new(), Vec::new())),
            BlockType::Value(t) => Ok((Vec::new(), vec![t])),
            BlockType::TypeIdx(i) => {
                let ty = self
                    .types
                    .get(i as usize)
                    .ok_or_else(|| WasmError::invalid("unknown type"))?;
                Ok((ty.params.clone(), ty.results.clone()))
            }
        }
    }

    fn local(&self, i: u32) -> Result<ValType> {
        self.locals.get(i as usize).copied().ok_or_else(|| WasmError::invalid("unknown local"))
    }

    fn global(&self, i: u32) -> Result<GlobalType> {
        self.globals.get(i as usize).copied().ok_or_else(|| WasmError::invalid("unknown global"))
    }

    fn table(&self, i: u32) -> Result<TableType> {
        self.tables.get(i as usize).copied().ok_or_else(|| WasmError::invalid("unknown table"))
    }

    fn memory(&self, i: u32) -> Result<MemoryType> {
        self.memories.get(i as usize).copied().ok_or_else(|| WasmError::invalid("unknown memory"))
    }

    fn func_type(&self, i: u32) -> Result<FuncType> {
        let ti = *self.funcs.get(i as usize).ok_or_else(|| WasmError::invalid("unknown function"))?;
        self.types.get(ti as usize).cloned().ok_or_else(|| WasmError::invalid("unknown type"))
    }

    fn align_ok(align: u32, width: u32) -> bool {
        let natural = width.trailing_zeros();
        align <= natural
    }

    fn check_mem(&self, align: u32, width: u32) -> Result<()> {
        if !Self::align_ok(align, width) {
            return Err(WasmError::invalid("alignment must not be larger than natural"));
        }
        Ok(())
    }

    fn visit(&mut self, instr: &Instr) -> Result<()> {
        match instr {
            Instr::Unreachable => self.unreachable(),
            Instr::Nop => {}
            Instr::Block(bt) => {
                let (params, results) = self.block_type(*bt)?;
                self.pop_vals(&params)?;
                self.push_ctrl(CtrlKind::Block, params, results);
            }
            Instr::Loop(bt) => {
                let (params, results) = self.block_type(*bt)?;
                self.pop_vals(&params)?;
                self.push_ctrl(CtrlKind::Loop, params, results);
            }
            Instr::If(bt) => {
                let (params, results) = self.block_type(*bt)?;
                self.pop_expect(ValType::I32)?;
                self.pop_vals(&params)?;
                self.push_ctrl(CtrlKind::If, params, results);
            }
            Instr::Else => {
                let frame = self.pop_ctrl()?;
                if frame.kind != CtrlKind::If {
                    return Err(WasmError::invalid("else without matching if"));
                }
                self.push_ctrl(CtrlKind::Else, frame.start, frame.end);
            }
            Instr::End => {
                let frame = self.pop_ctrl()?;
                if frame.kind == CtrlKind::If && frame.start != frame.end {
                    return Err(WasmError::invalid(
                        "type mismatch: if without else must have matching parameters and results",
                    ));
                }
                self.push_vals(&frame.end);
            }
            Instr::Br(l) => {
                let types = self.label_types(*l)?;
                self.pop_vals(&types)?;
                self.unreachable();
            }
            Instr::BrIf(l) => {
                self.pop_expect(ValType::I32)?;
                let types = self.label_types(*l)?;
                self.pop_vals(&types)?;
                self.push_vals(&types);
            }
            Instr::BrTable(targets, default) => {
                self.pop_expect(ValType::I32)?;
                let default_types = self.label_types(*default)?;
                for &target in targets.iter() {
                    let types = self.label_types(target)?;
                    if types.len() != default_types.len() {
                        return Err(WasmError::invalid("type mismatch: br_table target arity differs"));
                    }
                    self.check_vals(&types)?;
                }
                self.pop_vals(&default_types)?;
                self.unreachable();
            }
            Instr::Return => {
                let results = self.ctrls.first().expect("function frame").end.clone();
                self.pop_vals(&results)?;
                self.unreachable();
            }
            Instr::Call(f) => {
                let ft = self.func_type(*f)?;
                self.pop_vals(&ft.params)?;
                self.push_vals(&ft.results);
            }
            Instr::CallIndirect { ty, table } => {
                let t = self.table(*table)?;
                if t.elem != ValType::FuncRef {
                    return Err(WasmError::invalid("type mismatch: call_indirect requires a funcref table"));
                }
                let ft = self
                    .types
                    .get(*ty as usize)
                    .cloned()
                    .ok_or_else(|| WasmError::invalid("unknown type"))?;
                self.pop_expect(ValType::I32)?;
                self.pop_vals(&ft.params)?;
                self.push_vals(&ft.results);
            }
            Instr::Drop => {
                self.pop()?;
            }
            Instr::Select => {
                self.pop_expect(ValType::I32)?;
                let a = self.pop()?;
                let b = self.pop()?;
                let t = match (a, b) {
                    (Opd::Known(x), Opd::Known(y)) if x == y => Opd::Known(x),
                    (Opd::Known(x), Opd::Unknown) | (Opd::Unknown, Opd::Known(x)) => Opd::Known(x),
                    (Opd::Unknown, Opd::Unknown) => Opd::Unknown,
                    _ => return Err(WasmError::invalid("type mismatch: select operands differ")),
                };
                if let Opd::Known(x) = t {
                    if !x.is_num() && !x.is_vec() {
                        return Err(WasmError::invalid(
                            "type mismatch: select without a type requires numeric operands",
                        ));
                    }
                }
                self.push_opd(t);
            }
            Instr::SelectT(tys) => {
                if tys.len() != 1 {
                    return Err(WasmError::invalid("invalid result arity"));
                }
                let t = tys[0];
                self.pop_expect(ValType::I32)?;
                self.pop_expect(t)?;
                self.pop_expect(t)?;
                self.push(t);
            }
            Instr::LocalGet(i) => {
                let t = self.local(*i)?;
                self.push(t);
            }
            Instr::LocalSet(i) => {
                let t = self.local(*i)?;
                self.pop_expect(t)?;
            }
            Instr::LocalTee(i) => {
                let t = self.local(*i)?;
                self.pop_expect(t)?;
                self.push(t);
            }
            Instr::GlobalGet(i) => {
                let g = self.global(*i)?;
                self.push(g.val);
            }
            Instr::GlobalSet(i) => {
                let g = self.global(*i)?;
                if !g.mutable {
                    return Err(WasmError::invalid("global is immutable"));
                }
                self.pop_expect(g.val)?;
            }
            Instr::TableGet(i) => {
                let t = self.table(*i)?;
                self.pop_expect(ValType::I32)?;
                self.push(t.elem);
            }
            Instr::TableSet(i) => {
                let t = self.table(*i)?;
                self.pop_expect(t.elem)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::TableInit { elem, table } => {
                let et = self.elem_type(*elem)?;
                let t = self.table(*table)?;
                if et != t.elem {
                    return Err(WasmError::invalid("type mismatch: element segment type differs from table"));
                }
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::ElemDrop(e) => {
                self.elem_type(*e)?;
            }
            Instr::TableCopy { dst, src } => {
                let d = self.table(*dst)?;
                let s = self.table(*src)?;
                if d.elem != s.elem {
                    return Err(WasmError::invalid("type mismatch: table.copy element types differ"));
                }
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::TableGrow(i) => {
                let t = self.table(*i)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(t.elem)?;
                self.push(ValType::I32);
            }
            Instr::TableSize(i) => {
                self.table(*i)?;
                self.push(ValType::I32);
            }
            Instr::TableFill(i) => {
                let t = self.table(*i)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(t.elem)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::RefNull(t) => self.push(*t),
            Instr::RefIsNull => {
                let o = self.pop()?;
                match o {
                    Opd::Known(t) if t.is_ref() => {}
                    Opd::Known(_) => {
                        return Err(WasmError::invalid("type mismatch: ref.is_null expects a reference"))
                    }
                    Opd::Unknown => {}
                }
                self.push(ValType::I32);
            }
            Instr::RefFunc(f) => {
                if !self.refs.contains(f) {
                    return Err(WasmError::invalid("undeclared function reference"));
                }
                if *f as usize >= self.funcs.len() {
                    return Err(WasmError::invalid("unknown function"));
                }
                self.push(ValType::FuncRef);
            }
            Instr::Load(op, m) => {
                self.memory(m.memory)?;
                self.check_mem(m.align, op.width())?;
                self.pop_expect(ValType::I32)?;
                self.push(op.result());
            }
            Instr::Store(op, m) => {
                self.memory(m.memory)?;
                self.check_mem(m.align, op.width())?;
                self.pop_expect(op.operand())?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::MemorySize(_) => {
                self.memory(0)?;
                self.push(ValType::I32);
            }
            Instr::MemoryGrow(_) => {
                self.memory(0)?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Instr::MemoryInit { data, memory } => {
                self.memory(*memory)?;
                let count = self
                    .data_count
                    .ok_or_else(|| WasmError::invalid("data count section required"))?;
                if *data >= count {
                    return Err(WasmError::invalid("unknown data segment"));
                }
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::DataDrop(data) => {
                let count = self
                    .data_count
                    .ok_or_else(|| WasmError::invalid("data count section required"))?;
                if *data >= count {
                    return Err(WasmError::invalid("unknown data segment"));
                }
            }
            Instr::MemoryCopy { dst, src } => {
                self.memory(*dst)?;
                self.memory(*src)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::MemoryFill(m) => {
                self.memory(*m)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Instr::I32Const(_) => self.push(ValType::I32),
            Instr::I64Const(_) => self.push(ValType::I64),
            Instr::F32Const(_) => self.push(ValType::F32),
            Instr::F64Const(_) => self.push(ValType::F64),
            Instr::Num(op) => self.visit_num(*op)?,
        }
        Ok(())
    }

    fn visit_num(&mut self, op: NumOp) -> Result<()> {
        match op.signature() {
            NumSig::Test(t) => {
                self.pop_expect(t)?;
                self.push(ValType::I32);
            }
            NumSig::Cmp(t) => {
                self.pop_expect(t)?;
                self.pop_expect(t)?;
                self.push(ValType::I32);
            }
            NumSig::Unary(t) => {
                self.pop_expect(t)?;
                self.push(t);
            }
            NumSig::Binary(t) => {
                self.pop_expect(t)?;
                self.pop_expect(t)?;
                self.push(t);
            }
            NumSig::Convert(from, to) => {
                self.pop_expect(from)?;
                self.push(to);
            }
        }
        Ok(())
    }

    fn elem_type(&self, i: u32) -> Result<ValType> {
        self.elem_types.get(i as usize).copied().ok_or_else(|| WasmError::invalid("unknown elem segment"))
    }
}

// Add the element-type table to the validator context.
struct Context {
    types: Vec<FuncType>,
    funcs: Vec<u32>,
    tables: Vec<TableType>,
    memories: Vec<MemoryType>,
    globals: Vec<GlobalType>,
    refs: HashSet<u32>,
    data_count: Option<u32>,
}

/// Validate a whole module; returns an error classified as
/// [`ErrorKind::Invalid`](crate::ErrorKind::Invalid) on failure.
pub fn validate_module(module: &Module) -> Result<()> {
    let ctx = Context {
        types: module.types.clone(),
        funcs: collect_func_types(module)?,
        tables: collect_tables(module),
        memories: collect_memories(module),
        globals: collect_globals(module),
        refs: collect_refs(module),
        data_count: module.data_count,
    };

    // Imports reference declared types.
    for import in &module.imports {
        if let ImportDesc::Func(t) = import.desc {
            if t as usize >= ctx.types.len() {
                return Err(WasmError::invalid("unknown type"));
            }
        }
    }

    if ctx.memories.len() > 1 {
        return Err(WasmError::invalid("multiple memories"));
    }
    for m in &ctx.memories {
        check_limits(&m.limits, MAX_PAGES, "memory size")?;
    }
    for t in &ctx.tables {
        check_limits(&t.limits, u32::MAX, "table size")?;
    }

    if let Some(dc) = module.data_count {
        if dc as usize != module.datas.len() {
            return Err(WasmError::invalid("data count and data section have inconsistent lengths"));
        }
    }

    validate_globals(module, &ctx)?;
    validate_elems(module, &ctx)?;
    validate_datas(module, &ctx)?;
    validate_exports(module, &ctx)?;

    if let Some(start) = module.start {
        let ft = ctx
            .funcs
            .get(start as usize)
            .and_then(|t| ctx.types.get(*t as usize))
            .ok_or_else(|| WasmError::invalid("unknown function"))?;
        if !ft.params.is_empty() || !ft.results.is_empty() {
            return Err(WasmError::invalid("start function must have type [] -> []"));
        }
    }

    for func in &module.funcs {
        let ty = ctx
            .types
            .get(func.type_idx as usize)
            .ok_or_else(|| WasmError::invalid("unknown type"))?;
        let mut locals = ty.params.clone();
        locals.extend_from_slice(&func.locals);
        let mut v = FnValidator {
            types: &ctx.types,
            funcs: &ctx.funcs,
            tables: &ctx.tables,
            memories: &ctx.memories,
            globals: &ctx.globals,
            refs: &ctx.refs,
            data_count: ctx.data_count,
            locals,
            stack: Vec::new(),
            ctrls: Vec::new(),
            elem_types: module.elems.iter().map(|e| e.ty).collect(),
        };
        v.push_ctrl(CtrlKind::Func, Vec::new(), ty.results.clone());
        for instr in &func.body {
            v.visit(instr)?;
        }
        if !v.ctrls.is_empty() {
            return Err(WasmError::invalid("type mismatch: function body did not end"));
        }
        if v.stack.len() != ty.results.len() {
            return Err(WasmError::invalid("type mismatch: function results remain"));
        }
    }
    Ok(())
}

fn collect_func_types(module: &Module) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for import in &module.imports {
        if let ImportDesc::Func(t) = import.desc {
            out.push(t);
        }
    }
    for f in &module.funcs {
        out.push(f.type_idx);
    }
    Ok(out)
}

fn collect_tables(module: &Module) -> Vec<TableType> {
    let mut out = Vec::new();
    for import in &module.imports {
        if let ImportDesc::Table(t) = import.desc {
            out.push(t);
        }
    }
    out.extend_from_slice(&module.tables);
    out
}

fn collect_memories(module: &Module) -> Vec<MemoryType> {
    let mut out = Vec::new();
    for import in &module.imports {
        if let ImportDesc::Memory(t) = import.desc {
            out.push(t);
        }
    }
    out.extend_from_slice(&module.memories);
    out
}

fn collect_globals(module: &Module) -> Vec<GlobalType> {
    let mut out = Vec::new();
    for import in &module.imports {
        if let ImportDesc::Global(t) = import.desc {
            out.push(t);
        }
    }
    for g in &module.globals {
        out.push(g.ty);
    }
    out
}

/// The set of functions a module declares for `ref.func`: those mentioned in
/// global initializers, element segments, or exports (everything except function
/// bodies and the start function).
fn collect_refs(module: &Module) -> HashSet<u32> {
    let mut refs = HashSet::new();
    for g in &module.globals {
        collect_ref_funcs(&g.init, &mut refs);
    }
    for e in &module.elems {
        for expr in &e.init {
            collect_ref_funcs(expr, &mut refs);
        }
    }
    for e in &module.exports {
        if let ExportDesc::Func(f) = e.desc {
            refs.insert(f);
        }
    }
    refs
}

fn collect_ref_funcs(exprs: &[Instr], refs: &mut HashSet<u32>) {
    for instr in exprs {
        if let Instr::RefFunc(f) = instr {
            refs.insert(*f);
        }
    }
}

fn check_limits(l: &crate::types::Limits, ceiling: u32, what: &str) -> Result<()> {
    if l.min > ceiling {
        return Err(WasmError::invalid(format!("{} minimum must be at most {}", what, ceiling)));
    }
    if let Some(max) = l.max {
        if max > ceiling {
            return Err(WasmError::invalid(format!("{} maximum must be at most {}", what, ceiling)));
        }
        if max < l.min {
            return Err(WasmError::invalid("size minimum must not be greater than maximum"));
        }
    }
    Ok(())
}

fn validate_globals(module: &Module, ctx: &Context) -> Result<()> {
    for g in &module.globals {
        let imported = ctx.globals.len() - module.globals.len();
        check_const_expr(&g.init, g.ty.val, ctx, imported)?;
    }
    Ok(())
}

fn validate_elems(module: &Module, ctx: &Context) -> Result<()> {
    let imported_globals = ctx.globals.len() - module.globals.len();
    for e in &module.elems {
        for expr in &e.init {
            check_const_expr(expr, e.ty, ctx, imported_globals)?;
        }
        if let crate::module::ElemMode::Active { table, offset } = &e.mode {
            let t = ctx.tables.get(*table as usize).ok_or_else(|| WasmError::invalid("unknown table"))?;
            if t.elem != e.ty {
                return Err(WasmError::invalid("type mismatch: element segment type differs from table"));
            }
            check_const_expr(offset, ValType::I32, ctx, imported_globals)?;
        }
    }
    Ok(())
}

fn validate_datas(module: &Module, ctx: &Context) -> Result<()> {
    let imported_globals = ctx.globals.len() - module.globals.len();
    for d in &module.datas {
        if let crate::module::DataMode::Active { memory, offset } = &d.mode {
            ctx.memories.get(*memory as usize).ok_or_else(|| WasmError::invalid("unknown memory"))?;
            check_const_expr(offset, ValType::I32, ctx, imported_globals)?;
        }
    }
    Ok(())
}

fn validate_exports(module: &Module, ctx: &Context) -> Result<()> {
    let mut seen = HashSet::new();
    for e in &module.exports {
        if !seen.insert(e.name.as_str()) {
            return Err(WasmError::invalid("duplicate export name"));
        }
        match e.desc {
            ExportDesc::Func(i) => {
                if i as usize >= ctx.funcs.len() {
                    return Err(WasmError::invalid("unknown function"));
                }
            }
            ExportDesc::Table(i) => {
                if i as usize >= ctx.tables.len() {
                    return Err(WasmError::invalid("unknown table"));
                }
            }
            ExportDesc::Memory(i) => {
                if i as usize >= ctx.memories.len() {
                    return Err(WasmError::invalid("unknown memory"));
                }
            }
            ExportDesc::Global(i) => {
                if i as usize >= ctx.globals.len() {
                    return Err(WasmError::invalid("unknown global"));
                }
            }
        }
    }
    Ok(())
}

/// Validate a constant expression producing exactly `expected`.
fn check_const_expr(
    expr: &[Instr],
    expected: ValType,
    ctx: &Context,
    imported_globals: usize,
) -> Result<()> {
    let mut stack: Vec<ValType> = Vec::new();
    for instr in expr {
        match instr {
            Instr::I32Const(_) => stack.push(ValType::I32),
            Instr::I64Const(_) => stack.push(ValType::I64),
            Instr::F32Const(_) => stack.push(ValType::F32),
            Instr::F64Const(_) => stack.push(ValType::F64),
            Instr::RefNull(t) => stack.push(*t),
            Instr::RefFunc(f) => {
                if !ctx.refs.contains(f) || *f as usize >= ctx.funcs.len() {
                    return Err(WasmError::invalid("undeclared function reference"));
                }
                stack.push(ValType::FuncRef);
            }
            Instr::GlobalGet(g) => {
                if *g as usize >= imported_globals {
                    return Err(WasmError::invalid("unknown global"));
                }
                let gt = ctx.globals[*g as usize];
                if gt.mutable {
                    return Err(WasmError::invalid("constant expression required"));
                }
                stack.push(gt.val);
            }
            Instr::End => break,
            _ => return Err(WasmError::invalid("constant expression required")),
        }
    }
    if stack.len() != 1 {
        return Err(WasmError::invalid("type mismatch: constant expression must produce one value"));
    }
    if stack[0] != expected {
        return Err(WasmError::invalid(format!(
            "type mismatch: constant expression expected {}, found {}",
            expected.name(),
            stack[0].name()
        )));
    }
    Ok(())
}
