//! Binary-format encoder.
//!
//! The WAT text parser builds a [`Module`], which is encoded here and then
//! handed to [`crate::decode::decode_module`]. That round trip means every
//! text-format testsuite case exercises the binary decoder too.

use crate::instr::{Instr, LoadOp, NumOp, StoreOp};
use crate::module::{Data, DataMode, Elem, ElemMode, ExportDesc, ImportDesc, Module};
use crate::types::{BlockType, FuncType, GlobalType, Limits, TableType, ValType};

/// Encode a module to the WebAssembly binary format.
pub fn encode_module(module: &Module) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\0asm");
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);

    if !module.types.is_empty() {
        section(&mut out, 1, |b| {
            write_u32(b, module.types.len() as u32);
            for t in &module.types {
                write_functype(b, t);
            }
        });
    }
    if !module.imports.is_empty() {
        section(&mut out, 2, |b| {
            write_u32(b, module.imports.len() as u32);
            for i in &module.imports {
                write_name(b, &i.module);
                write_name(b, &i.name);
                match &i.desc {
                    ImportDesc::Func(t) => {
                        b.push(0x00);
                        write_u32(b, *t);
                    }
                    ImportDesc::Table(t) => {
                        b.push(0x01);
                        write_table_type(b, t);
                    }
                    ImportDesc::Memory(t) => {
                        b.push(0x02);
                        write_limits(b, &t.limits);
                    }
                    ImportDesc::Global(t) => {
                        b.push(0x03);
                        write_global_type(b, t);
                    }
                }
            }
        });
    }
    if !module.funcs.is_empty() {
        section(&mut out, 3, |b| {
            write_u32(b, module.funcs.len() as u32);
            for f in &module.funcs {
                write_u32(b, f.type_idx);
            }
        });
    }
    if !module.tables.is_empty() {
        section(&mut out, 4, |b| {
            write_u32(b, module.tables.len() as u32);
            for t in &module.tables {
                write_table_type(b, t);
            }
        });
    }
    if !module.memories.is_empty() {
        section(&mut out, 5, |b| {
            write_u32(b, module.memories.len() as u32);
            for m in &module.memories {
                write_limits(b, &m.limits);
            }
        });
    }
    if !module.globals.is_empty() {
        section(&mut out, 6, |b| {
            write_u32(b, module.globals.len() as u32);
            for g in &module.globals {
                write_global_type(b, &g.ty);
                write_expr(b, &g.init);
            }
        });
    }
    if !module.exports.is_empty() {
        section(&mut out, 7, |b| {
            write_u32(b, module.exports.len() as u32);
            for e in &module.exports {
                write_name(b, &e.name);
                let (kind, idx) = match e.desc {
                    ExportDesc::Func(i) => (0x00, i),
                    ExportDesc::Table(i) => (0x01, i),
                    ExportDesc::Memory(i) => (0x02, i),
                    ExportDesc::Global(i) => (0x03, i),
                };
                b.push(kind);
                write_u32(b, idx);
            }
        });
    }
    if let Some(start) = module.start {
        section(&mut out, 8, |b| write_u32(b, start));
    }
    if !module.elems.is_empty() {
        section(&mut out, 9, |b| {
            write_u32(b, module.elems.len() as u32);
            for e in &module.elems {
                write_elem(b, e);
            }
        });
    }
    if needs_data_count(module) {
        section(&mut out, 12, |b| write_u32(b, module.datas.len() as u32));
    }
    if !module.funcs.is_empty() {
        section(&mut out, 10, |b| {
            write_u32(b, module.funcs.len() as u32);
            for f in &module.funcs {
                let mut body = Vec::new();
                write_locals(&mut body, &f.locals);
                write_expr(&mut body, &f.body);
                write_u32(b, body.len() as u32);
                b.extend_from_slice(&body);
            }
        });
    }
    if !module.datas.is_empty() {
        section(&mut out, 11, |b| {
            write_u32(b, module.datas.len() as u32);
            for d in &module.datas {
                write_data(b, d);
            }
        });
    }
    out
}

fn needs_data_count(module: &Module) -> bool {
    if module.data_count.is_some() {
        return true;
    }
    module.funcs.iter().any(|f| {
        f.body.iter().any(|i| matches!(i, Instr::MemoryInit { .. } | Instr::DataDrop(_)))
    })
}

fn section(out: &mut Vec<u8>, id: u8, write: impl FnOnce(&mut Vec<u8>)) {
    let mut body = Vec::new();
    write(&mut body);
    out.push(id);
    write_u32(out, body.len() as u32);
    out.extend_from_slice(&body);
}

fn write_u32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn write_signed(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v as u8) & 0x7f;
        v >>= 7;
        let sign = byte & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

fn write_name(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn write_functype(out: &mut Vec<u8>, t: &FuncType) {
    out.push(0x60);
    write_u32(out, t.params.len() as u32);
    for p in &t.params {
        out.push(p.byte());
    }
    write_u32(out, t.results.len() as u32);
    for r in &t.results {
        out.push(r.byte());
    }
}

fn write_limits(out: &mut Vec<u8>, l: &Limits) {
    match l.max {
        None => {
            out.push(0x00);
            write_u32(out, l.min);
        }
        Some(max) => {
            out.push(0x01);
            write_u32(out, l.min);
            write_u32(out, max);
        }
    }
}

fn write_table_type(out: &mut Vec<u8>, t: &TableType) {
    out.push(t.elem.byte());
    write_limits(out, &t.limits);
}

fn write_global_type(out: &mut Vec<u8>, t: &GlobalType) {
    out.push(t.val.byte());
    out.push(if t.mutable { 0x01 } else { 0x00 });
}

fn write_elem(out: &mut Vec<u8>, e: &Elem) {
    match &e.mode {
        ElemMode::Active { table, offset } => {
            out.push(0x06);
            write_u32(out, *table);
            write_expr(out, offset);
            out.push(e.ty.byte());
        }
        ElemMode::Passive => {
            out.push(0x05);
            out.push(e.ty.byte());
        }
        ElemMode::Declarative => {
            out.push(0x07);
            out.push(e.ty.byte());
        }
    }
    write_u32(out, e.init.len() as u32);
    for expr in &e.init {
        write_expr(out, expr);
    }
}

fn write_data(out: &mut Vec<u8>, d: &Data) {
    match &d.mode {
        DataMode::Active { memory, offset } => {
            out.push(0x02);
            write_u32(out, *memory);
            write_expr(out, offset);
        }
        DataMode::Passive => out.push(0x01),
    }
    write_u32(out, d.init.len() as u32);
    out.extend_from_slice(&d.init);
}

fn write_locals(out: &mut Vec<u8>, locals: &[ValType]) {
    let mut groups: Vec<(u32, ValType)> = Vec::new();
    for &ty in locals {
        match groups.last_mut() {
            Some((count, t)) if *t == ty => *count += 1,
            _ => groups.push((1, ty)),
        }
    }
    write_u32(out, groups.len() as u32);
    for (count, ty) in groups {
        write_u32(out, count);
        out.push(ty.byte());
    }
}

fn write_block_type(out: &mut Vec<u8>, bt: &BlockType) {
    match bt {
        BlockType::Empty => out.push(0x40),
        BlockType::Value(t) => out.push(t.byte()),
        BlockType::TypeIdx(i) => write_signed(out, i64::from(*i)),
    }
}

fn write_expr(out: &mut Vec<u8>, instrs: &[Instr]) {
    for instr in instrs {
        write_instr(out, instr);
    }
}

fn write_memarg(out: &mut Vec<u8>, align: u32, offset: u32) {
    write_u32(out, align);
    write_u32(out, offset);
}

fn write_instr(out: &mut Vec<u8>, instr: &Instr) {
    match instr {
        Instr::Unreachable => out.push(0x00),
        Instr::Nop => out.push(0x01),
        Instr::Block(bt) => {
            out.push(0x02);
            write_block_type(out, bt);
        }
        Instr::Loop(bt) => {
            out.push(0x03);
            write_block_type(out, bt);
        }
        Instr::If(bt) => {
            out.push(0x04);
            write_block_type(out, bt);
        }
        Instr::Else => out.push(0x05),
        Instr::End => out.push(0x0B),
        Instr::Br(i) => {
            out.push(0x0C);
            write_u32(out, *i);
        }
        Instr::BrIf(i) => {
            out.push(0x0D);
            write_u32(out, *i);
        }
        Instr::BrTable(targets, default) => {
            out.push(0x0E);
            write_u32(out, targets.len() as u32);
            for t in targets.iter() {
                write_u32(out, *t);
            }
            write_u32(out, *default);
        }
        Instr::Return => out.push(0x0F),
        Instr::Call(i) => {
            out.push(0x10);
            write_u32(out, *i);
        }
        Instr::CallIndirect { ty, table } => {
            out.push(0x11);
            write_u32(out, *ty);
            write_u32(out, *table);
        }
        Instr::Drop => out.push(0x1A),
        Instr::Select => out.push(0x1B),
        Instr::SelectT(tys) => {
            out.push(0x1C);
            write_u32(out, tys.len() as u32);
            for t in tys.iter() {
                out.push(t.byte());
            }
        }
        Instr::LocalGet(i) => {
            out.push(0x20);
            write_u32(out, *i);
        }
        Instr::LocalSet(i) => {
            out.push(0x21);
            write_u32(out, *i);
        }
        Instr::LocalTee(i) => {
            out.push(0x22);
            write_u32(out, *i);
        }
        Instr::GlobalGet(i) => {
            out.push(0x23);
            write_u32(out, *i);
        }
        Instr::GlobalSet(i) => {
            out.push(0x24);
            write_u32(out, *i);
        }
        Instr::TableGet(i) => {
            out.push(0x25);
            write_u32(out, *i);
        }
        Instr::TableSet(i) => {
            out.push(0x26);
            write_u32(out, *i);
        }
        Instr::Load(op, m) => {
            out.push(load_opcode(*op));
            write_memarg(out, m.align, m.offset);
        }
        Instr::Store(op, m) => {
            out.push(store_opcode(*op));
            write_memarg(out, m.align, m.offset);
        }
        Instr::MemorySize(_) => {
            out.push(0x3F);
            out.push(0x00);
        }
        Instr::MemoryGrow(_) => {
            out.push(0x40);
            out.push(0x00);
        }
        Instr::I32Const(v) => {
            out.push(0x41);
            write_signed(out, i64::from(*v));
        }
        Instr::I64Const(v) => {
            out.push(0x42);
            write_signed(out, *v);
        }
        Instr::F32Const(bits) => {
            out.push(0x43);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        Instr::F64Const(bits) => {
            out.push(0x44);
            out.extend_from_slice(&bits.to_le_bytes());
        }
        Instr::Num(op) => write_num(out, *op),
        Instr::RefNull(t) => {
            out.push(0xD0);
            out.push(t.byte());
        }
        Instr::RefIsNull => out.push(0xD1),
        Instr::RefFunc(i) => {
            out.push(0xD2);
            write_u32(out, *i);
        }
        Instr::MemoryInit { data, memory } => {
            out.push(0xFC);
            write_u32(out, 8);
            write_u32(out, *data);
            write_u32(out, *memory);
        }
        Instr::DataDrop(i) => {
            out.push(0xFC);
            write_u32(out, 9);
            write_u32(out, *i);
        }
        Instr::MemoryCopy { dst, src } => {
            out.push(0xFC);
            write_u32(out, 10);
            write_u32(out, *dst);
            write_u32(out, *src);
        }
        Instr::MemoryFill(i) => {
            out.push(0xFC);
            write_u32(out, 11);
            write_u32(out, *i);
        }
        Instr::TableInit { elem, table } => {
            out.push(0xFC);
            write_u32(out, 12);
            write_u32(out, *elem);
            write_u32(out, *table);
        }
        Instr::ElemDrop(i) => {
            out.push(0xFC);
            write_u32(out, 13);
            write_u32(out, *i);
        }
        Instr::TableCopy { dst, src } => {
            out.push(0xFC);
            write_u32(out, 14);
            write_u32(out, *dst);
            write_u32(out, *src);
        }
        Instr::TableGrow(i) => {
            out.push(0xFC);
            write_u32(out, 15);
            write_u32(out, *i);
        }
        Instr::TableSize(i) => {
            out.push(0xFC);
            write_u32(out, 16);
            write_u32(out, *i);
        }
        Instr::TableFill(i) => {
            out.push(0xFC);
            write_u32(out, 17);
            write_u32(out, *i);
        }
    }
}

fn write_num(out: &mut Vec<u8>, op: NumOp) {
    let byte = op.byte();
    if byte < 0x100 {
        out.push(byte as u8);
    } else {
        out.push(0xFC);
        write_u32(out, u32::from(byte & 0xFF));
    }
}

fn load_opcode(op: LoadOp) -> u8 {
    match op {
        LoadOp::I32 => 0x28,
        LoadOp::I64 => 0x29,
        LoadOp::F32 => 0x2A,
        LoadOp::F64 => 0x2B,
        LoadOp::I32_8S => 0x2C,
        LoadOp::I32_8U => 0x2D,
        LoadOp::I32_16S => 0x2E,
        LoadOp::I32_16U => 0x2F,
        LoadOp::I64_8S => 0x30,
        LoadOp::I64_8U => 0x31,
        LoadOp::I64_16S => 0x32,
        LoadOp::I64_16U => 0x33,
        LoadOp::I64_32S => 0x34,
        LoadOp::I64_32U => 0x35,
    }
}

fn store_opcode(op: StoreOp) -> u8 {
    match op {
        StoreOp::I32 => 0x36,
        StoreOp::I64 => 0x37,
        StoreOp::F32 => 0x38,
        StoreOp::F64 => 0x39,
        StoreOp::I32_8 => 0x3A,
        StoreOp::I32_16 => 0x3B,
        StoreOp::I64_8 => 0x3C,
        StoreOp::I64_16 => 0x3D,
        StoreOp::I64_32 => 0x3E,
    }
}
