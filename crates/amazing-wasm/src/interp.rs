//! The WebAssembly interpreter.
//!
//! Execution is iterative rather than recursive: a call pushes a [`Frame`]
//! (its own program counter, locals and label stack) onto an explicit frame
//! stack, and the value stack is shared. That keeps deeply recursive modules
//! from exhausting the native Rust stack, and makes the specification's
//! `call stack exhausted` condition a simple depth check.
//!
//! Structured control is interpreted from the flat instruction body using a
//! precomputed table of matching `else`/`end` positions, exactly as the
//! validator walks it.

use std::rc::Rc;

use crate::error::{Result, WasmError};
use crate::instr::{Instr, LoadOp, NumOp, StoreOp};
use crate::module::Module;
use crate::store::{Extern, FuncInstance, Store};
use crate::types::{BlockType, MemArg, ValType};
use crate::value::{Ref, Value};

/// The maximum number of nested WebAssembly calls before exhaustion.
pub const MAX_CALL_DEPTH: usize = 1024;

/// A control label (the runtime counterpart of a validation control frame).
#[derive(Clone, Copy, Debug)]
struct Label {
    /// Number of values transferred by a branch to this label.
    arity: usize,
    /// Operand-stack height when the construct was entered.
    height: usize,
    /// Where a branch to this label resumes.
    target: usize,
    /// True for `loop` labels (a branch resumes the loop header).
    is_loop: bool,
}

/// An activation record.
struct Frame {
    /// The module instance this function belongs to.
    inst: usize,
    /// Function address.
    func: u32,
    /// The defining module.
    module: Rc<Module>,
    /// Index into `module.funcs`.
    defined: usize,
    /// Program counter into the body.
    pc: usize,
    /// Parameters followed by locals.
    locals: Vec<Value>,
    /// Control labels, innermost last.
    labels: Vec<Label>,
    /// Operand-stack height on entry.
    base: usize,
    /// Number of result values.
    results_arity: usize,
    /// Matching `end` index for each structured instruction.
    match_end: Vec<usize>,
    /// Matching `else` index for each `if` (or `usize::MAX`).
    match_else: Vec<usize>,
}

/// A WebAssembly interpreter over a mutable store.
pub struct Interpreter<'a> {
    store: &'a mut Store,
    stack: Vec<Value>,
    frames: Vec<Frame>,
}

impl<'a> Interpreter<'a> {
    /// Wrap a store.
    pub fn new(store: &'a mut Store) -> Self {
        Interpreter { store, stack: Vec::new(), frames: Vec::new() }
    }

    /// Invoke function address `func` inside instance `inst` with `args`.
    pub fn invoke(&mut self, inst: usize, func: u32, args: Vec<Value>) -> Result<Vec<Value>> {
        let expected = self
            .store
            .funcs
            .get(func as usize)
            .ok_or_else(|| WasmError::trap("unknown function"))?
            .ty()
            .clone();
        if args.len() != expected.params.len() {
            return Err(WasmError::trap("argument count mismatch"));
        }
        for (a, p) in args.iter().zip(&expected.params) {
            if !value_matches(a.ty(), *p) {
                return Err(WasmError::trap("argument type mismatch"));
            }
        }
        self.stack.clear();
        self.frames.clear();
        self.stack.extend(args);
        self.push_call(inst, func)?;
        loop {
            if self.frames.is_empty() {
                return Ok(std::mem::take(&mut self.stack));
            }
            self.step()?;
        }
    }

    /// One interpreter step.
    fn step(&mut self) -> Result<()> {
        let fidx = self.frames.len() - 1;
        let func = self.frames[fidx].func;
        if matches!(self.store.funcs[func as usize], FuncInstance::Host { .. }) {
            // Host frames are never pushed; handled in `push_call`.
            unreachable!("host functions do not get frames");
        }

        let module = self.frames[fidx].module.clone();
        let defined = self.frames[fidx].defined;
        let pc = self.frames[fidx].pc;
        let body = &module.funcs[defined].body;
        if pc >= body.len() {
            return self.finish_function();
        }
        self.frames[fidx].pc = pc + 1;
        let instr = &body[pc];
        let inst = self.frames[fidx].inst;

        match instr {
            Instr::Unreachable => return Err(WasmError::trap("unreachable executed")),
            Instr::Nop => {}
            Instr::Block(bt) => {
                let (params, results) = self.block_types(inst, *bt)?;
                self.enter_label(results.len(), params.len(), false, *bt, pc, false);
            }
            Instr::Loop(bt) => {
                let (params, results) = self.block_types(inst, *bt)?;
                let _ = results;
                self.enter_label(params.len(), params.len(), true, *bt, pc, false);
            }
            Instr::If(bt) => {
                let (params, results) = self.block_types(inst, *bt)?;
                let cond = self.pop_i32()?;
                let end = self.frames[fidx].match_end[pc];
                let height = self.stack.len() - params.len();
                self.frames[fidx].labels.push(Label {
                    arity: results.len(),
                    height,
                    target: end + 1,
                    is_loop: false,
                });
                if cond == 0 {
                    let els = self.frames[fidx].match_else[pc];
                    if els != usize::MAX {
                        self.frames[fidx].pc = els + 1;
                    } else {
                        self.frames[fidx].labels.pop();
                        self.frames[fidx].pc = end + 1;
                    }
                }
            }
            Instr::Else => {
                // Reached by falling out of the then arm: skip the else arm.
                self.frames[fidx].labels.pop();
                let end = self.frames[fidx].match_end[pc];
                self.frames[fidx].pc = end + 1;
            }
            Instr::End => {
                self.frames[fidx].labels.pop();
                if self.frames[fidx].labels.is_empty() {
                    self.frames[fidx].pc = self.frames[fidx].pc;
                    return self.finish_function();
                }
            }
            Instr::Br(depth) => self.branch(*depth)?,
            Instr::BrIf(depth) => {
                let c = self.pop_i32()?;
                if c != 0 {
                    self.branch(*depth)?;
                }
            }
            Instr::BrTable(targets, default) => {
                let i = self.pop_i32()? as u32 as usize;
                let depth = if i < targets.len() { targets[i] } else { *default };
                self.branch(depth)?;
            }
            Instr::Return => {
                let depth = (self.frames[fidx].labels.len() - 1) as u32;
                self.branch(depth)?;
            }
            Instr::Call(index) => {
                let addr = self.func_addr(inst, *index)?;
                self.push_call(inst, addr)?;
            }
            Instr::CallIndirect { ty, table } => {
                let i = self.pop_i32()? as u32;
                let taddr = self.table_addr(inst, *table)?;
                let table = &self.store.tables[taddr];
                let Some(r) = table.elems.get(i as usize).copied() else {
                    return Err(WasmError::trap("undefined element"));
                };
                let Ref::Func(addr) = r else {
                    return Err(WasmError::trap("uninitialized element"));
                };
                let expected = module
                    .types
                    .get(*ty as usize)
                    .ok_or_else(|| WasmError::invalid("unknown type"))?;
                if self.store.funcs[addr as usize].ty() != expected {
                    return Err(WasmError::trap("indirect call type mismatch"));
                }
                self.push_call(inst, addr)?;
            }
            Instr::Drop => {
                self.stack.pop();
            }
            Instr::Select | Instr::SelectT(_) => {
                let c = self.pop_i32()?;
                let b = self.stack.pop().ok_or_else(|| WasmError::trap("stack underflow"))?;
                let a = self.stack.pop().ok_or_else(|| WasmError::trap("stack underflow"))?;
                self.stack.push(if c != 0 { a } else { b });
            }
            Instr::LocalGet(i) => {
                let v = self.frames[fidx]
                    .locals
                    .get(*i as usize)
                    .copied()
                    .ok_or_else(|| WasmError::trap("unknown local"))?;
                self.stack.push(v);
            }
            Instr::LocalSet(i) => {
                let v = self.stack.pop().ok_or_else(|| WasmError::trap("stack underflow"))?;
                let slot = self.frames[fidx]
                    .locals
                    .get_mut(*i as usize)
                    .ok_or_else(|| WasmError::trap("unknown local"))?;
                *slot = v;
            }
            Instr::LocalTee(i) => {
                let v = *self.stack.last().ok_or_else(|| WasmError::trap("stack underflow"))?;
                let slot = self.frames[fidx]
                    .locals
                    .get_mut(*i as usize)
                    .ok_or_else(|| WasmError::trap("unknown local"))?;
                *slot = v;
            }
            Instr::GlobalGet(i) => {
                let a = self.global_addr(inst, *i)?;
                self.stack.push(self.store.globals[a].value);
            }
            Instr::GlobalSet(i) => {
                let a = self.global_addr(inst, *i)?;
                let v = self.stack.pop().ok_or_else(|| WasmError::trap("stack underflow"))?;
                self.store.globals[a].value = v;
            }
            Instr::TableGet(i) => {
                let ta = self.table_addr(inst, *i)?;
                let idx = self.pop_i32()? as u32;
                let Some(r) = self.store.tables[ta].elems.get(idx as usize).copied() else {
                    return Err(WasmError::trap("out of bounds table access"));
                };
                self.stack.push(Value::Ref(r));
            }
            Instr::TableSet(i) => {
                let ta = self.table_addr(inst, *i)?;
                let r = self.pop_ref()?;
                let idx = self.pop_i32()? as u32;
                let len = self.store.tables[ta].elems.len();
                if idx as usize >= len {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                self.store.tables[ta].elems[idx as usize] = r;
            }
            Instr::TableInit { elem, table } => {
                let n = self.pop_i32()? as u32 as usize;
                let s = self.pop_i32()? as u32 as usize;
                let d = self.pop_i32()? as u32 as usize;
                let ta = self.table_addr(inst, *table)?;
                let seg = self.elem_seg(inst, *elem)?;
                let src: &[Ref] =
                    if self.store.elems[seg].dropped { &[] } else { &self.store.elems[seg].elems };
                if s.checked_add(n).is_none_or(|e| e > src.len()) {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                let elems = src[s..s + n].to_vec();
                let table = &mut self.store.tables[ta];
                if d.checked_add(n).is_none_or(|e| e > table.elems.len()) {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                table.elems[d..d + n].copy_from_slice(&elems);
            }
            Instr::ElemDrop(e) => {
                let seg = self.elem_seg(inst, *e)?;
                self.store.elems[seg].dropped = true;
            }
            Instr::TableCopy { dst, src } => {
                let n = self.pop_i32()? as u32 as usize;
                let s = self.pop_i32()? as u32 as usize;
                let d = self.pop_i32()? as u32 as usize;
                let da = self.table_addr(inst, *dst)?;
                let sa = self.table_addr(inst, *src)?;
                let sdata = self.store.tables[sa].elems.clone();
                if s.checked_add(n).is_none_or(|e| e > sdata.len()) {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                let table = &mut self.store.tables[da];
                if d.checked_add(n).is_none_or(|e| e > table.elems.len()) {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                table.elems[d..d + n].clone_from_slice(&sdata[s..s + n]);
            }
            Instr::TableGrow(i) => {
                let ta = self.table_addr(inst, *i)?;
                let delta = self.pop_i32()? as u32;
                let init = self.pop_ref()?;
                let old = self.store.tables[ta].grow(delta, init);
                self.stack.push(Value::I32(old));
            }
            Instr::TableSize(i) => {
                let ta = self.table_addr(inst, *i)?;
                self.stack.push(Value::I32(self.store.tables[ta].elems.len() as i32));
            }
            Instr::TableFill(i) => {
                let ta = self.table_addr(inst, *i)?;
                let n = self.pop_i32()? as u32 as usize;
                let val = self.pop_ref()?;
                let d = self.pop_i32()? as u32 as usize;
                let table = &mut self.store.tables[ta];
                if d.checked_add(n).is_none_or(|e| e > table.elems.len()) {
                    return Err(WasmError::trap("out of bounds table access"));
                }
                table.elems[d..d + n].fill(val);
            }
            Instr::RefNull(t) => self.stack.push(Value::Ref(Ref::Null(*t))),
            Instr::RefIsNull => {
                let r = self.pop_ref()?;
                self.stack.push(Value::I32(i32::from(r.is_null())));
            }
            Instr::RefFunc(f) => {
                let a = self.func_addr(inst, *f)?;
                self.stack.push(Value::Ref(Ref::Func(a)));
            }
            Instr::Load(op, m) => self.exec_load(inst, *op, m)?,
            Instr::Store(op, m) => self.exec_store(inst, *op, m)?,
            Instr::MemorySize(i) => {
                let a = self.mem_addr(inst, *i)?;
                self.stack.push(Value::I32(self.store.memories[a].size_pages() as i32));
            }
            Instr::MemoryGrow(i) => {
                let a = self.mem_addr(inst, *i)?;
                let delta = self.pop_i32()? as u32;
                let old = self.store.memories[a].grow(delta);
                self.stack.push(Value::I32(old));
            }
            Instr::MemoryInit { data, memory } => {
                let n = self.pop_i32()? as u32 as usize;
                let s = self.pop_i32()? as u32 as usize;
                let d = self.pop_i32()? as u32 as usize;
                let ma = self.mem_addr(inst, *memory)?;
                let seg = self.data_seg(inst, *data)?;
                let src: &[u8] = if self.store.datas[seg].dropped { &[] } else { &self.store.datas[seg].data };
                if s.checked_add(n).is_none_or(|e| e > src.len()) {
                    return Err(WasmError::trap("out of bounds memory access"));
                }
                let bytes = src[s..s + n].to_vec();
                let mem = &mut self.store.memories[ma];
                if d.checked_add(n).is_none_or(|e| e > mem.data.len()) {
                    return Err(WasmError::trap("out of bounds memory access"));
                }
                mem.data[d..d + n].copy_from_slice(&bytes);
            }
            Instr::DataDrop(d) => {
                let seg = self.data_seg(inst, *d)?;
                self.store.datas[seg].dropped = true;
            }
            Instr::MemoryCopy { dst, src } => {
                let n = self.pop_i32()? as u32 as usize;
                let s = self.pop_i32()? as u32 as usize;
                let d = self.pop_i32()? as u32 as usize;
                let da = self.mem_addr(inst, *dst)?;
                let sa = self.mem_addr(inst, *src)?;
                let sdata = self.store.memories[sa].data.clone();
                if s.checked_add(n).is_none_or(|e| e > sdata.len()) {
                    return Err(WasmError::trap("out of bounds memory access"));
                }
                let mem = &mut self.store.memories[da];
                if d.checked_add(n).is_none_or(|e| e > mem.data.len()) {
                    return Err(WasmError::trap("out of bounds memory access"));
                }
                mem.data[d..d + n].copy_from_slice(&sdata[s..s + n]);
            }
            Instr::MemoryFill(i) => {
                let ma = self.mem_addr(inst, *i)?;
                let n = self.pop_i32()? as u32 as usize;
                let val = self.pop_i32()? as u8;
                let d = self.pop_i32()? as u32 as usize;
                let mem = &mut self.store.memories[ma];
                if d.checked_add(n).is_none_or(|e| e > mem.data.len()) {
                    return Err(WasmError::trap("out of bounds memory access"));
                }
                mem.data[d..d + n].fill(val);
            }
            Instr::I32Const(v) => self.stack.push(Value::I32(*v)),
            Instr::I64Const(v) => self.stack.push(Value::I64(*v)),
            Instr::F32Const(bits) => self.stack.push(Value::F32(f32::from_bits(*bits))),
            Instr::F64Const(bits) => self.stack.push(Value::F64(f64::from_bits(*bits))),
            Instr::Num(op) => self.exec_num(*op)?,
        }
        Ok(())
    }

    fn finish_function(&mut self) -> Result<()> {
        let frame = self.frames.pop().expect("frame");
        let arity = frame.results_arity;
        if self.stack.len() < frame.base + arity {
            return Err(WasmError::trap("stack underflow on return"));
        }
        self.stack.truncate(frame.base + arity);
        Ok(())
    }

    fn enter_label(
        &mut self,
        arity: usize,
        _params: usize,
        is_loop: bool,
        bt: BlockType,
        pc: usize,
        _is_if: bool,
    ) {
        let _ = bt;
        let fidx = self.frames.len() - 1;
        let target = if is_loop { pc + 1 } else { self.frames[fidx].match_end[pc] + 1 };
        let height = self.stack.len().saturating_sub(self.block_param_count(fidx, bt));
        self.frames[fidx].labels.push(Label { arity, height, target, is_loop });
    }

    fn block_param_count(&self, fidx: usize, bt: BlockType) -> usize {
        match bt {
            BlockType::Empty | BlockType::Value(_) => 0,
            BlockType::TypeIdx(i) => {
                let module = &self.frames[fidx].module;
                module.types.get(i as usize).map_or(0, |t| t.params.len())
            }
        }
    }

    fn branch(&mut self, depth: u32) -> Result<()> {
        let fidx = self.frames.len() - 1;
        let n = self.frames[fidx].labels.len();
        let di = depth as usize;
        if di >= n {
            return Err(WasmError::trap("unknown label"));
        }
        let label = self.frames[fidx].labels[n - 1 - di];
        // Move the branch operands down to the label's height.
        let keep = label.arity;
        let src = self.stack.len() - keep;
        if src < label.height {
            return Err(WasmError::trap("stack underflow on branch"));
        }
        for i in 0..keep {
            self.stack[label.height + i] = self.stack[src + i];
        }
        self.stack.truncate(label.height + keep);
        if label.is_loop {
            self.frames[fidx].labels.truncate(n - di);
        } else {
            self.frames[fidx].labels.truncate(n - 1 - di);
        }
        self.frames[fidx].pc = label.target;
        Ok(())
    }

    fn push_call(&mut self, inst: usize, addr: u32) -> Result<()> {
        let _ = inst; // the callee's own defining instance is used, not the caller's
        let func = self
            .store
            .funcs
            .get(addr as usize)
            .cloned()
            .ok_or_else(|| WasmError::trap("unknown function"))?;
        match func {
            FuncInstance::Host { ty, func, .. } => {
                let n = ty.params.len();
                if self.stack.len() < n {
                    return Err(WasmError::trap("stack underflow on call"));
                }
                let base = self.stack.len() - n;
                let args: Vec<Value> = self.stack.split_off(base);
                let results = func(&args)?;
                if results.len() != ty.results.len() {
                    return Err(WasmError::trap("host function returned the wrong number of values"));
                }
                self.stack.extend(results);
            }
            FuncInstance::Wasm { ty, module, index, locals, inst } => {
                if self.frames.len() >= MAX_CALL_DEPTH {
                    return Err(WasmError::exhaustion("call stack exhausted"));
                }
                let n = ty.params.len();
                if self.stack.len() < n {
                    return Err(WasmError::trap("stack underflow on call"));
                }
                let base = self.stack.len() - n;
                let mut local_values: Vec<Value> = self.stack.split_off(base);
                for lt in &locals {
                    local_values.push(Value::default_for(*lt));
                }
                let body_len = module.funcs[index].body.len();
                let (match_end, match_else) = compute_markers(&module.funcs[index].body);
                self.frames.push(Frame {
                    inst,
                    func: addr,
                    module,
                    defined: index,
                    pc: 0,
                    locals: local_values,
                    labels: vec![Label {
                        arity: ty.results.len(),
                        height: base,
                        target: body_len,
                        is_loop: false,
                    }],
                    base,
                    results_arity: ty.results.len(),
                    match_end,
                    match_else,
                });
            }
        }
        Ok(())
    }

    fn block_types(&self, inst: usize, bt: BlockType) -> Result<(Vec<ValType>, Vec<ValType>)> {
        match bt {
            BlockType::Empty => Ok((Vec::new(), Vec::new())),
            BlockType::Value(t) => Ok((Vec::new(), vec![t])),
            BlockType::TypeIdx(i) => {
                let module = &self.store.instances[inst].module;
                let t = module
                    .types
                    .get(i as usize)
                    .ok_or_else(|| WasmError::invalid("unknown type"))?;
                Ok((t.params.clone(), t.results.clone()))
            }
        }
    }

    fn func_addr(&self, inst: usize, index: u32) -> Result<u32> {
        self.store.instances[inst]
            .funcs
            .get(index as usize)
            .copied()
            .ok_or_else(|| WasmError::trap("unknown function"))
    }

    fn table_addr(&self, inst: usize, index: u32) -> Result<usize> {
        self.store.instances[inst]
            .tables
            .get(index as usize)
            .map(|a| *a as usize)
            .ok_or_else(|| WasmError::trap("unknown table"))
    }

    fn mem_addr(&self, inst: usize, index: u32) -> Result<usize> {
        self.store.instances[inst]
            .memories
            .get(index as usize)
            .map(|a| *a as usize)
            .ok_or_else(|| WasmError::trap("unknown memory"))
    }

    fn global_addr(&self, inst: usize, index: u32) -> Result<usize> {
        self.store.instances[inst]
            .globals
            .get(index as usize)
            .map(|a| *a as usize)
            .ok_or_else(|| WasmError::trap("unknown global"))
    }

    fn elem_seg(&self, inst: usize, index: u32) -> Result<usize> {
        self.store.instances[inst]
            .elems
            .get(index as usize)
            .copied()
            .ok_or_else(|| WasmError::trap("unknown elem segment"))
    }

    fn data_seg(&self, inst: usize, index: u32) -> Result<usize> {
        self.store.instances[inst]
            .datas
            .get(index as usize)
            .copied()
            .ok_or_else(|| WasmError::trap("unknown data segment"))
    }

    fn exec_load(&mut self, inst: usize, op: LoadOp, m: &MemArg) -> Result<()> {
        let a = self.mem_addr(inst, m.memory)?;
        let base = self.pop_i32()? as u32;
        let ea = u64::from(base) + u64::from(m.offset);
        let width = op.width() as u64;
        let mem = &self.store.memories[a];
        if ea + width > mem.data.len() as u64 {
            return Err(WasmError::trap("out of bounds memory access"));
        }
        let start = ea as usize;
        let bytes = &mem.data[start..start + width as usize];
        let mut raw = [0u8; 8];
        raw[..bytes.len()].copy_from_slice(bytes);
        let value = match op {
            LoadOp::I32 => Value::I32(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])),
            LoadOp::I64 => Value::I64(i64::from_le_bytes(raw)),
            LoadOp::F32 => Value::F32(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])),
            LoadOp::F64 => Value::F64(f64::from_le_bytes(raw)),
            LoadOp::I32_8S => Value::I32(raw[0] as i8 as i32),
            LoadOp::I32_8U => Value::I32(i32::from(raw[0])),
            LoadOp::I32_16S => Value::I32(i16::from_le_bytes([raw[0], raw[1]]) as i32),
            LoadOp::I32_16U => Value::I32(i32::from(u16::from_le_bytes([raw[0], raw[1]]))),
            LoadOp::I64_8S => Value::I64(raw[0] as i8 as i64),
            LoadOp::I64_8U => Value::I64(i64::from(raw[0])),
            LoadOp::I64_16S => Value::I64(i16::from_le_bytes([raw[0], raw[1]]) as i64),
            LoadOp::I64_16U => Value::I64(i64::from(u16::from_le_bytes([raw[0], raw[1]]))),
            LoadOp::I64_32S => Value::I64(i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as i64),
            LoadOp::I64_32U => Value::I64(i64::from(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))),
        };
        self.stack.push(value);
        Ok(())
    }

    fn exec_store(&mut self, inst: usize, op: StoreOp, m: &MemArg) -> Result<()> {
        let a = self.mem_addr(inst, m.memory)?;
        let value = self.stack.pop().ok_or_else(|| WasmError::trap("stack underflow"))?;
        let base = self.pop_i32()? as u32;
        let ea = u64::from(base) + u64::from(m.offset);
        let width = op.width() as u64;
        let mem = &mut self.store.memories[a];
        if ea + width > mem.data.len() as u64 {
            return Err(WasmError::trap("out of bounds memory access"));
        }
        let start = ea as usize;
        let bytes: Vec<u8> = match (op, value) {
            (StoreOp::I32, Value::I32(v)) => v.to_le_bytes().to_vec(),
            (StoreOp::I64, Value::I64(v)) => v.to_le_bytes().to_vec(),
            (StoreOp::F32, Value::F32(v)) => v.to_le_bytes().to_vec(),
            (StoreOp::F64, Value::F64(v)) => v.to_le_bytes().to_vec(),
            (StoreOp::I32_8, Value::I32(v)) => vec![v as u8],
            (StoreOp::I32_16, Value::I32(v)) => (v as u16).to_le_bytes().to_vec(),
            (StoreOp::I64_8, Value::I64(v)) => vec![v as u8],
            (StoreOp::I64_16, Value::I64(v)) => (v as u16).to_le_bytes().to_vec(),
            (StoreOp::I64_32, Value::I64(v)) => (v as u32).to_le_bytes().to_vec(),
            _ => return Err(WasmError::trap("store type mismatch")),
        };
        mem.data[start..start + bytes.len()].copy_from_slice(&bytes);
        Ok(())
    }

    fn exec_num(&mut self, op: NumOp) -> Result<()> {
        use crate::value as val;
        use NumOp::*;
        match op {
            I32Eqz => {
                let a = self.pop_i32()?;
                self.push(Value::I32(i32::from(a == 0)));
            }
            I32Eq => self.cmp_i32(|a, b| a == b)?,
            I32Ne => self.cmp_i32(|a, b| a != b)?,
            I32LtS => self.cmp_i32(|a, b| a < b)?,
            I32LtU => self.cmp_i32(|a, b| (a as u32) < (b as u32))?,
            I32GtS => self.cmp_i32(|a, b| a > b)?,
            I32GtU => self.cmp_i32(|a, b| (a as u32) > (b as u32))?,
            I32LeS => self.cmp_i32(|a, b| a <= b)?,
            I32LeU => self.cmp_i32(|a, b| (a as u32) <= (b as u32))?,
            I32GeS => self.cmp_i32(|a, b| a >= b)?,
            I32GeU => self.cmp_i32(|a, b| (a as u32) >= (b as u32))?,
            I64Eqz => {
                let a = self.pop_i64()?;
                self.push(Value::I32(i32::from(a == 0)));
            }
            I64Eq => self.cmp_i64(|a, b| a == b)?,
            I64Ne => self.cmp_i64(|a, b| a != b)?,
            I64LtS => self.cmp_i64(|a, b| a < b)?,
            I64LtU => self.cmp_i64(|a, b| (a as u64) < (b as u64))?,
            I64GtS => self.cmp_i64(|a, b| a > b)?,
            I64GtU => self.cmp_i64(|a, b| (a as u64) > (b as u64))?,
            I64LeS => self.cmp_i64(|a, b| a <= b)?,
            I64LeU => self.cmp_i64(|a, b| (a as u64) <= (b as u64))?,
            I64GeS => self.cmp_i64(|a, b| a >= b)?,
            I64GeU => self.cmp_i64(|a, b| (a as u64) >= (b as u64))?,
            F32Eq => self.cmp_f32(|a, b| a == b)?,
            F32Ne => self.cmp_f32(|a, b| a != b)?,
            F32Lt => self.cmp_f32(|a, b| a < b)?,
            F32Gt => self.cmp_f32(|a, b| a > b)?,
            F32Le => self.cmp_f32(|a, b| a <= b)?,
            F32Ge => self.cmp_f32(|a, b| a >= b)?,
            F64Eq => self.cmp_f64(|a, b| a == b)?,
            F64Ne => self.cmp_f64(|a, b| a != b)?,
            F64Lt => self.cmp_f64(|a, b| a < b)?,
            F64Gt => self.cmp_f64(|a, b| a > b)?,
            F64Le => self.cmp_f64(|a, b| a <= b)?,
            F64Ge => self.cmp_f64(|a, b| a >= b)?,
            I32Clz => self.un_i32(|a| a.leading_zeros() as i32)?,
            I32Ctz => self.un_i32(|a| a.trailing_zeros() as i32)?,
            I32Popcnt => self.un_i32(|a| a.count_ones() as i32)?,
            I32Add => self.bin_i32(|a, b| a.wrapping_add(b))?,
            I32Sub => self.bin_i32(|a, b| a.wrapping_sub(b))?,
            I32Mul => self.bin_i32(|a, b| a.wrapping_mul(b))?,
            I32DivS => {
                let b = self.pop_i32()?;
                let a = self.pop_i32()?;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                if a == i32::MIN && b == -1 {
                    return Err(WasmError::trap("integer overflow"));
                }
                self.push(Value::I32(a.wrapping_div(b)));
            }
            I32DivU => {
                let b = self.pop_i32()? as u32;
                let a = self.pop_i32()? as u32;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I32((a / b) as i32));
            }
            I32RemS => {
                let b = self.pop_i32()?;
                let a = self.pop_i32()?;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I32(if a == i32::MIN && b == -1 { 0 } else { a % b }));
            }
            I32RemU => {
                let b = self.pop_i32()? as u32;
                let a = self.pop_i32()? as u32;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I32((a % b) as i32));
            }
            I32And => self.bin_i32(|a, b| a & b)?,
            I32Or => self.bin_i32(|a, b| a | b)?,
            I32Xor => self.bin_i32(|a, b| a ^ b)?,
            I32Shl => self.bin_i32(|a, b| a.wrapping_shl(b as u32))?,
            I32ShrS => self.bin_i32(|a, b| a.wrapping_shr(b as u32))?,
            I32ShrU => self.bin_i32(|a, b| ((a as u32).wrapping_shr(b as u32)) as i32)?,
            I32Rotl => self.bin_i32(|a, b| a.rotate_left(b as u32 & 31))?,
            I32Rotr => self.bin_i32(|a, b| a.rotate_right(b as u32 & 31))?,
            I64Clz => self.un_i64(|a| a.leading_zeros() as i64)?,
            I64Ctz => self.un_i64(|a| a.trailing_zeros() as i64)?,
            I64Popcnt => self.un_i64(|a| a.count_ones() as i64)?,
            I64Add => self.bin_i64(|a, b| a.wrapping_add(b))?,
            I64Sub => self.bin_i64(|a, b| a.wrapping_sub(b))?,
            I64Mul => self.bin_i64(|a, b| a.wrapping_mul(b))?,
            I64DivS => {
                let b = self.pop_i64()?;
                let a = self.pop_i64()?;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                if a == i64::MIN && b == -1 {
                    return Err(WasmError::trap("integer overflow"));
                }
                self.push(Value::I64(a.wrapping_div(b)));
            }
            I64DivU => {
                let b = self.pop_i64()? as u64;
                let a = self.pop_i64()? as u64;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I64((a / b) as i64));
            }
            I64RemS => {
                let b = self.pop_i64()?;
                let a = self.pop_i64()?;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I64(if a == i64::MIN && b == -1 { 0 } else { a % b }));
            }
            I64RemU => {
                let b = self.pop_i64()? as u64;
                let a = self.pop_i64()? as u64;
                if b == 0 {
                    return Err(WasmError::trap("integer divide by zero"));
                }
                self.push(Value::I64((a % b) as i64));
            }
            I64And => self.bin_i64(|a, b| a & b)?,
            I64Or => self.bin_i64(|a, b| a | b)?,
            I64Xor => self.bin_i64(|a, b| a ^ b)?,
            I64Shl => self.bin_i64(|a, b| a.wrapping_shl(b as u32))?,
            I64ShrS => self.bin_i64(|a, b| a.wrapping_shr(b as u32))?,
            I64ShrU => self.bin_i64(|a, b| ((a as u64).wrapping_shr(b as u32)) as i64)?,
            I64Rotl => self.bin_i64(|a, b| a.rotate_left(b as u32 & 63))?,
            I64Rotr => self.bin_i64(|a, b| a.rotate_right(b as u32 & 63))?,
            F32Abs => self.un_f32(val::f32_abs)?,
            F32Neg => self.un_f32(val::f32_neg)?,
            F32Ceil => self.un_f32(|a| val::quiet_f32(a.ceil()))?,
            F32Floor => self.un_f32(|a| val::quiet_f32(a.floor()))?,
            F32Trunc => self.un_f32(|a| val::quiet_f32(a.trunc()))?,
            F32Nearest => self.un_f32(|a| val::quiet_f32(val::f32_nearest(a)))?,
            F32Sqrt => self.un_f32(|a| val::quiet_f32(a.sqrt()))?,
            F32Add => self.bin_f32(|a, b| val::quiet_f32(a + b))?,
            F32Sub => self.bin_f32(|a, b| val::quiet_f32(a - b))?,
            F32Mul => self.bin_f32(|a, b| val::quiet_f32(a * b))?,
            F32Div => self.bin_f32(|a, b| val::quiet_f32(a / b))?,
            F32Min => self.bin_f32(val::f32_min)?,
            F32Max => self.bin_f32(val::f32_max)?,
            F32Copysign => self.bin_f32(val::f32_copysign)?,
            F64Abs => self.un_f64(val::f64_abs)?,
            F64Neg => self.un_f64(val::f64_neg)?,
            F64Ceil => self.un_f64(|a| val::quiet_f64(a.ceil()))?,
            F64Floor => self.un_f64(|a| val::quiet_f64(a.floor()))?,
            F64Trunc => self.un_f64(|a| val::quiet_f64(a.trunc()))?,
            F64Nearest => self.un_f64(|a| val::quiet_f64(val::f64_nearest(a)))?,
            F64Sqrt => self.un_f64(|a| val::quiet_f64(a.sqrt()))?,
            F64Add => self.bin_f64(|a, b| val::quiet_f64(a + b))?,
            F64Sub => self.bin_f64(|a, b| val::quiet_f64(a - b))?,
            F64Mul => self.bin_f64(|a, b| val::quiet_f64(a * b))?,
            F64Div => self.bin_f64(|a, b| val::quiet_f64(a / b))?,
            F64Min => self.bin_f64(val::f64_min)?,
            F64Max => self.bin_f64(val::f64_max)?,
            F64Copysign => self.bin_f64(val::f64_copysign)?,
            I32WrapI64 => {
                let a = self.pop_i64()?;
                self.push(Value::I32(a as i32));
            }
            I32TruncF32S => {
                let a = self.pop_f32()?;
                self.push(Value::I32(val::i32_trunc_f32_s(a)?));
            }
            I32TruncF32U => {
                let a = self.pop_f32()?;
                self.push(Value::I32(val::i32_trunc_f32_u(a)? as i32));
            }
            I32TruncF64S => {
                let a = self.pop_f64()?;
                self.push(Value::I32(val::i32_trunc_f64_s(a)?));
            }
            I32TruncF64U => {
                let a = self.pop_f64()?;
                self.push(Value::I32(val::i32_trunc_f64_u(a)? as i32));
            }
            I64ExtendI32S => {
                let a = self.pop_i32()?;
                self.push(Value::I64(a as i64));
            }
            I64ExtendI32U => {
                let a = self.pop_i32()?;
                self.push(Value::I64(i64::from(a as u32)));
            }
            I64TruncF32S => {
                let a = self.pop_f32()?;
                self.push(Value::I64(val::i64_trunc_f32_s(a)?));
            }
            I64TruncF32U => {
                let a = self.pop_f32()?;
                self.push(Value::I64(val::i64_trunc_f32_u(a)? as i64));
            }
            I64TruncF64S => {
                let a = self.pop_f64()?;
                self.push(Value::I64(val::i64_trunc_f64_s(a)?));
            }
            I64TruncF64U => {
                let a = self.pop_f64()?;
                self.push(Value::I64(val::i64_trunc_f64_u(a)? as i64));
            }
            F32ConvertI32S => {
                let a = self.pop_i32()?;
                self.push(Value::F32(a as f32));
            }
            F32ConvertI32U => {
                let a = self.pop_i32()? as u32;
                self.push(Value::F32(a as f32));
            }
            F32ConvertI64S => {
                let a = self.pop_i64()?;
                self.push(Value::F32(a as f32));
            }
            F32ConvertI64U => {
                let a = self.pop_i64()? as u64;
                self.push(Value::F32(a as f32));
            }
            F32DemoteF64 => {
                let a = self.pop_f64()?;
                self.push(Value::F32(a as f32));
            }
            F64ConvertI32S => {
                let a = self.pop_i32()?;
                self.push(Value::F64(f64::from(a)));
            }
            F64ConvertI32U => {
                let a = self.pop_i32()? as u32;
                self.push(Value::F64(f64::from(a)));
            }
            F64ConvertI64S => {
                let a = self.pop_i64()?;
                self.push(Value::F64(a as f64));
            }
            F64ConvertI64U => {
                let a = self.pop_i64()? as u64;
                self.push(Value::F64(a as f64));
            }
            F64PromoteF32 => {
                let a = self.pop_f32()?;
                self.push(Value::F64(f64::from(a)));
            }
            I32ReinterpretF32 => {
                let a = self.pop_f32()?;
                self.push(Value::I32(a.to_bits() as i32));
            }
            I64ReinterpretF64 => {
                let a = self.pop_f64()?;
                self.push(Value::I64(a.to_bits() as i64));
            }
            F32ReinterpretI32 => {
                let a = self.pop_i32()?;
                self.push(Value::F32(f32::from_bits(a as u32)));
            }
            F64ReinterpretI64 => {
                let a = self.pop_i64()?;
                self.push(Value::F64(f64::from_bits(a as u64)));
            }
            I32Extend8S => self.un_i32(|a| a as i8 as i32)?,
            I32Extend16S => self.un_i32(|a| a as i16 as i32)?,
            I64Extend8S => self.un_i64(|a| a as i8 as i64)?,
            I64Extend16S => self.un_i64(|a| a as i16 as i64)?,
            I64Extend32S => self.un_i64(|a| a as i32 as i64)?,
            I32TruncSatF32S => {
                let a = self.pop_f32()?;
                self.push(Value::I32(val::i32_trunc_sat_f32_s(a)));
            }
            I32TruncSatF32U => {
                let a = self.pop_f32()?;
                self.push(Value::I32(val::i32_trunc_sat_f32_u(a) as i32));
            }
            I32TruncSatF64S => {
                let a = self.pop_f64()?;
                self.push(Value::I32(val::i32_trunc_sat_f64_s(a)));
            }
            I32TruncSatF64U => {
                let a = self.pop_f64()?;
                self.push(Value::I32(val::i32_trunc_sat_f64_u(a) as i32));
            }
            I64TruncSatF32S => {
                let a = self.pop_f32()?;
                self.push(Value::I64(val::i64_trunc_sat_f32_s(a)));
            }
            I64TruncSatF32U => {
                let a = self.pop_f32()?;
                self.push(Value::I64(val::i64_trunc_sat_f32_u(a) as i64));
            }
            I64TruncSatF64S => {
                let a = self.pop_f64()?;
                self.push(Value::I64(val::i64_trunc_sat_f64_s(a)));
            }
            I64TruncSatF64U => {
                let a = self.pop_f64()?;
                self.push(Value::I64(val::i64_trunc_sat_f64_u(a) as i64));
            }
        }
        Ok(())
    }

    fn cmp_i32(&mut self, f: impl Fn(i32, i32) -> bool) -> Result<()> {
        let b = self.pop_i32()?;
        let a = self.pop_i32()?;
        self.push(Value::I32(i32::from(f(a, b))));
        Ok(())
    }

    fn cmp_i64(&mut self, f: impl Fn(i64, i64) -> bool) -> Result<()> {
        let b = self.pop_i64()?;
        let a = self.pop_i64()?;
        self.push(Value::I32(i32::from(f(a, b))));
        Ok(())
    }

    fn cmp_f32(&mut self, f: impl Fn(f32, f32) -> bool) -> Result<()> {
        let b = self.pop_f32()?;
        let a = self.pop_f32()?;
        self.push(Value::I32(i32::from(f(a, b))));
        Ok(())
    }

    fn cmp_f64(&mut self, f: impl Fn(f64, f64) -> bool) -> Result<()> {
        let b = self.pop_f64()?;
        let a = self.pop_f64()?;
        self.push(Value::I32(i32::from(f(a, b))));
        Ok(())
    }

    fn un_i32(&mut self, f: impl Fn(i32) -> i32) -> Result<()> {
        let a = self.pop_i32()?;
        self.push(Value::I32(f(a)));
        Ok(())
    }

    fn un_i64(&mut self, f: impl Fn(i64) -> i64) -> Result<()> {
        let a = self.pop_i64()?;
        self.push(Value::I64(f(a)));
        Ok(())
    }

    fn bin_i32(&mut self, f: impl Fn(i32, i32) -> i32) -> Result<()> {
        let b = self.pop_i32()?;
        let a = self.pop_i32()?;
        self.push(Value::I32(f(a, b)));
        Ok(())
    }

    fn bin_i64(&mut self, f: impl Fn(i64, i64) -> i64) -> Result<()> {
        let b = self.pop_i64()?;
        let a = self.pop_i64()?;
        self.push(Value::I64(f(a, b)));
        Ok(())
    }

    fn un_f32(&mut self, f: impl Fn(f32) -> f32) -> Result<()> {
        let a = self.pop_f32()?;
        self.push(Value::F32(f(a)));
        Ok(())
    }

    fn un_f64(&mut self, f: impl Fn(f64) -> f64) -> Result<()> {
        let a = self.pop_f64()?;
        self.push(Value::F64(f(a)));
        Ok(())
    }

    fn bin_f32(&mut self, f: impl Fn(f32, f32) -> f32) -> Result<()> {
        let b = self.pop_f32()?;
        let a = self.pop_f32()?;
        self.push(Value::F32(f(a, b)));
        Ok(())
    }

    fn bin_f64(&mut self, f: impl Fn(f64, f64) -> f64) -> Result<()> {
        let b = self.pop_f64()?;
        let a = self.pop_f64()?;
        self.push(Value::F64(f(a, b)));
        Ok(())
    }

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn pop_i32(&mut self) -> Result<i32> {
        match self.stack.pop() {
            Some(Value::I32(v)) => Ok(v),
            _ => Err(WasmError::trap("type mismatch: expected i32 on stack")),
        }
    }

    fn pop_i64(&mut self) -> Result<i64> {
        match self.stack.pop() {
            Some(Value::I64(v)) => Ok(v),
            _ => Err(WasmError::trap("type mismatch: expected i64 on stack")),
        }
    }

    fn pop_f32(&mut self) -> Result<f32> {
        match self.stack.pop() {
            Some(Value::F32(v)) => Ok(v),
            _ => Err(WasmError::trap("type mismatch: expected f32 on stack")),
        }
    }

    fn pop_f64(&mut self) -> Result<f64> {
        match self.stack.pop() {
            Some(Value::F64(v)) => Ok(v),
            _ => Err(WasmError::trap("type mismatch: expected f64 on stack")),
        }
    }

    fn pop_ref(&mut self) -> Result<Ref> {
        match self.stack.pop() {
            Some(Value::Ref(r)) => Ok(r),
            _ => Err(WasmError::trap("type mismatch: expected a reference on stack")),
        }
    }
}

/// True when an actual value type can be used where `expected` is required.
fn value_matches(actual: ValType, expected: ValType) -> bool {
    actual == expected || (expected.is_ref() && actual.is_ref())
}

/// Precompute matching `else`/`end` positions for a flat body.
fn compute_markers(body: &[Instr]) -> (Vec<usize>, Vec<usize>) {
    let mut match_end = vec![usize::MAX; body.len()];
    let mut match_else = vec![usize::MAX; body.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, instr) in body.iter().enumerate() {
        match instr {
            Instr::Block(_) | Instr::Loop(_) | Instr::If(_) => stack.push(i),
            Instr::Else => {
                if let Some(&ifpc) = stack.last() {
                    match_else[ifpc] = i;
                }
            }
            Instr::End => {
                if let Some(open) = stack.pop() {
                    match_end[open] = i;
                    if match_else[open] != usize::MAX {
                        match_end[match_else[open]] = i;
                    }
                }
            }
            _ => {}
        }
    }
    (match_end, match_else)
}

/// A convenience wrapper: invoke a module export directly.
pub fn call_export(
    store: &mut Store,
    interp: &mut Interpreter<'_>,
    instance: usize,
    name: &str,
    args: Vec<Value>,
) -> Result<Vec<Value>> {
    let (addr, ty) = store.export_func(instance, name)?;
    let _ = ty;
    interp.invoke(instance, addr, args)
}

/// The `Extern` for a name, or an error.
pub fn export_extern(store: &Store, instance: usize, name: &str) -> Result<Extern> {
    store.instances[instance]
        .export(name)
        .ok_or_else(|| WasmError::trap(format!("no export named {name}")))
}
