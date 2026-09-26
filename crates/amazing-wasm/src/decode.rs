//! The binary-format decoder.
//!
//! Reads a complete module from a byte slice, classifying every structural
//! problem as [`ErrorKind::Malformed`]. Section order follows the grammar
//! (`type`, `import`, `func`, `table`, `memory`, `global`, `export`, `start`,
//! `elem`, `datacount`, `code`, `data`); the data-count section is given the
//! grammar's position even though its id is `12`.

use crate::error::{Result, WasmError};
use crate::instr::{Instr, LoadOp, NumOp, StoreOp};
use crate::module::{Data, DataMode, Elem, ElemMode, Export, ExportDesc, Func, Global, Import, ImportDesc, Module};
use crate::reader::Reader;
use crate::types::{BlockType, FuncType, GlobalType, Limits, MemArg, MemoryType, TableType, ValType};

/// Decode a complete WebAssembly module from its binary encoding.
pub fn decode_module(bytes: &[u8]) -> Result<Module> {
    let mut r = Reader::new(bytes);
    if r.bytes_n(4)? != b"\0asm" {
        return Err(WasmError::malformed("magic header not detected"));
    }
    let version = r.bytes_n(4)?;
    if version != [0x01, 0x00, 0x00, 0x00] {
        return Err(WasmError::malformed("unknown binary version"));
    }

    let mut module = Module::default();
    let mut func_types: Vec<u32> = Vec::new();
    let mut codes: Vec<(Vec<ValType>, Vec<Instr>)> = Vec::new();
    let mut last_priority = 0u8;

    while !r.is_empty() {
        let id = r.u8_raw("section id")?;
        let size = r.var_u32()? as usize;
        let body = r.bytes_n(size)?;
        if id == 0 {
            // Custom section: its name must be valid UTF-8; contents are
            // opaque to the core format.
            let mut cr = Reader::new(body);
            let _name = cr.name()?;
            continue;
        }
        let priority = section_priority(id)?;
        if priority <= last_priority {
            return Err(WasmError::malformed("unexpected section id"));
        }
        last_priority = priority;

        let mut sr = Reader::new(body);
        match id {
            1 => decode_types(&mut sr, &mut module)?,
            2 => decode_imports(&mut sr, &mut module)?,
            3 => {
                func_types = sr.vec("function", |r| r.var_u32())?;
            }
            4 => {
                module.tables = sr.vec("table", decode_table_type)?;
            }
            5 => {
                module.memories = sr.vec("memory", decode_memory_type)?;
            }
            6 => {
                module.globals = sr.vec("global", decode_global)?;
            }
            7 => {
                module.exports = sr.vec("export", decode_export)?;
            }
            8 => {
                module.start = Some(sr.var_u32()?);
            }
            9 => {
                module.elems = sr.vec("element", decode_elem)?;
            }
            10 => {
                codes = sr.vec("code", decode_code)?;
            }
            11 => {
                module.datas = sr.vec("data", decode_data)?;
            }
            12 => {
                module.data_count = Some(sr.var_u32()?);
            }
            _ => return Err(WasmError::malformed("malformed section id")),
        }
        if !sr.is_empty() {
            return Err(WasmError::malformed("section size mismatch"));
        }
    }

    if func_types.len() != codes.len() {
        return Err(WasmError::malformed("function and code section have inconsistent lengths"));
    }
    for (type_idx, (locals, body)) in func_types.into_iter().zip(codes) {
        module.funcs.push(Func { type_idx, locals, body });
    }
    Ok(module)
}

fn section_priority(id: u8) -> Result<u8> {
    Ok(match id {
        1..=9 => id,
        12 => 10,
        10 => 11,
        11 => 12,
        _ => return Err(WasmError::malformed("malformed section id")),
    })
}

fn decode_types(r: &mut Reader<'_>, module: &mut Module) -> Result<()> {
    module.types = r.vec("type", |r| {
        let form = r.u8_raw("functype")?;
        if form != 0x60 {
            return Err(WasmError::malformed("integer representation too long"));
        }
        let params = r.vec("param", decode_val_type)?;
        let results = r.vec("result", decode_val_type)?;
        Ok(FuncType { params, results })
    })?;
    Ok(())
}

fn decode_imports(r: &mut Reader<'_>, module: &mut Module) -> Result<()> {
    module.imports = r.vec("import", |r| {
        let module_name = r.name()?;
        let field = r.name()?;
        let kind = r.u8_raw("import kind")?;
        let desc = match kind {
            0x00 => ImportDesc::Func(r.var_u32()?),
            0x01 => ImportDesc::Table(decode_table_type(r)?),
            0x02 => ImportDesc::Memory(decode_memory_type(r)?),
            0x03 => ImportDesc::Global(decode_global_type(r)?),
            _ => return Err(WasmError::malformed("malformed import kind")),
        };
        Ok(Import { module: module_name, name: field, desc })
    })?;
    Ok(())
}

fn decode_val_type(r: &mut Reader<'_>) -> Result<ValType> {
    let b = r.u8_raw("value type")?;
    ValType::from_byte(b).ok_or_else(|| WasmError::malformed("malformed value type"))
}

fn decode_ref_type(r: &mut Reader<'_>) -> Result<ValType> {
    let b = r.u8_raw("reference type")?;
    match ValType::from_byte(b) {
        Some(t) if t.is_ref() => Ok(t),
        _ => Err(WasmError::malformed("malformed reference type")),
    }
}

fn decode_limits(r: &mut Reader<'_>) -> Result<Limits> {
    let flags = r.u8_raw("limits flags")?;
    match flags {
        0x00 => Ok(Limits { min: r.var_u32()?, max: None }),
        0x01 => {
            let min = r.var_u32()?;
            let max = r.var_u32()?;
            Ok(Limits { min, max: Some(max) })
        }
        _ => Err(WasmError::malformed("integer too large")),
    }
}

fn decode_table_type(r: &mut Reader<'_>) -> Result<TableType> {
    let elem = decode_ref_type(r)?;
    Ok(TableType { elem, limits: decode_limits(r)? })
}

fn decode_memory_type(r: &mut Reader<'_>) -> Result<MemoryType> {
    Ok(MemoryType { limits: decode_limits(r)? })
}

fn decode_global_type(r: &mut Reader<'_>) -> Result<GlobalType> {
    let val = decode_val_type(r)?;
    let mutable = match r.u8_raw("global mutability")? {
        0x00 => false,
        0x01 => true,
        _ => return Err(WasmError::malformed("malformed mutability")),
    };
    Ok(GlobalType { val, mutable })
}

fn decode_global(r: &mut Reader<'_>) -> Result<Global> {
    let ty = decode_global_type(r)?;
    let init = decode_expr(r)?;
    Ok(Global { ty, init })
}

fn decode_export(r: &mut Reader<'_>) -> Result<Export> {
    let name = r.name()?;
    let kind = r.u8_raw("export kind")?;
    let idx = r.var_u32()?;
    let desc = match kind {
        0x00 => ExportDesc::Func(idx),
        0x01 => ExportDesc::Table(idx),
        0x02 => ExportDesc::Memory(idx),
        0x03 => ExportDesc::Global(idx),
        _ => return Err(WasmError::malformed("malformed export kind")),
    };
    Ok(Export { name, desc })
}

fn decode_elem(r: &mut Reader<'_>) -> Result<Elem> {
    let flags = r.var_u32()?;
    match flags {
        0 => {
            let offset = decode_expr(r)?;
            let funcs = r.vec("elem", |r| r.var_u32())?;
            let init = funcs.into_iter().map(|f| vec![Instr::RefFunc(f), Instr::End]).collect();
            Ok(Elem { ty: ValType::FuncRef, init, mode: ElemMode::Active { table: 0, offset } })
        }
        1 => {
            expect_elem_kind(r)?;
            let funcs = r.vec("elem", |r| r.var_u32())?;
            let init = funcs.into_iter().map(|f| vec![Instr::RefFunc(f), Instr::End]).collect();
            Ok(Elem { ty: ValType::FuncRef, init, mode: ElemMode::Passive })
        }
        2 => {
            let table = r.var_u32()?;
            let offset = decode_expr(r)?;
            expect_elem_kind(r)?;
            let funcs = r.vec("elem", |r| r.var_u32())?;
            let init = funcs.into_iter().map(|f| vec![Instr::RefFunc(f), Instr::End]).collect();
            Ok(Elem { ty: ValType::FuncRef, init, mode: ElemMode::Active { table, offset } })
        }
        3 => {
            expect_elem_kind(r)?;
            let funcs = r.vec("elem", |r| r.var_u32())?;
            let init = funcs.into_iter().map(|f| vec![Instr::RefFunc(f), Instr::End]).collect();
            Ok(Elem { ty: ValType::FuncRef, init, mode: ElemMode::Declarative })
        }
        4 => {
            let offset = decode_expr(r)?;
            let init = r.vec("elem", decode_expr)?;
            Ok(Elem { ty: ValType::FuncRef, init, mode: ElemMode::Active { table: 0, offset } })
        }
        5 => {
            let ty = decode_ref_type(r)?;
            let init = r.vec("elem", decode_expr)?;
            Ok(Elem { ty, init, mode: ElemMode::Passive })
        }
        6 => {
            let table = r.var_u32()?;
            let offset = decode_expr(r)?;
            let ty = decode_ref_type(r)?;
            let init = r.vec("elem", decode_expr)?;
            Ok(Elem { ty, init, mode: ElemMode::Active { table, offset } })
        }
        7 => {
            let ty = decode_ref_type(r)?;
            let init = r.vec("elem", decode_expr)?;
            Ok(Elem { ty, init, mode: ElemMode::Declarative })
        }
        _ => Err(WasmError::malformed("malformed elements segment kind")),
    }
}

fn expect_elem_kind(r: &mut Reader<'_>) -> Result<()> {
    if r.u8_raw("elem kind")? != 0x00 {
        return Err(WasmError::malformed("malformed element kind"));
    }
    Ok(())
}

fn decode_data(r: &mut Reader<'_>) -> Result<Data> {
    let flags = r.var_u32()?;
    match flags {
        0 => {
            let offset = decode_expr(r)?;
            let init = r.byte_vec()?;
            Ok(Data { init, mode: DataMode::Active { memory: 0, offset } })
        }
        1 => {
            let init = r.byte_vec()?;
            Ok(Data { init, mode: DataMode::Passive })
        }
        2 => {
            let memory = r.var_u32()?;
            let offset = decode_expr(r)?;
            let init = r.byte_vec()?;
            Ok(Data { init, mode: DataMode::Active { memory, offset } })
        }
        _ => Err(WasmError::malformed("malformed data segment kind")),
    }
}

fn decode_code(r: &mut Reader<'_>) -> Result<(Vec<ValType>, Vec<Instr>)> {
    let size = r.var_u32()? as usize;
    let body = r.bytes_n(size)?;
    let mut br = Reader::new(body);
    let local_groups = br.vec("local", |r| {
        let count = r.var_u32()?;
        let ty = decode_val_type(r)?;
        Ok((count, ty))
    })?;
    let mut locals = Vec::new();
    for (count, ty) in local_groups {
        // A bogus count must not allocate unbounded memory; validation also
        // rejects a total over 2^32-1, but cap here so we fail fast.
        let count = count as usize;
        if count > 10_000_000 {
            return Err(WasmError::malformed("too many locals"));
        }
        locals.extend(std::iter::repeat_n(ty, count));
    }
    let instrs = decode_expr(&mut br)?;
    if !br.is_empty() {
        return Err(WasmError::malformed("section size mismatch"));
    }
    Ok((locals, instrs))
}

/// Decode a constant or function expression: instructions up to the matching
/// closing `end`, with no trailing bytes.
pub fn decode_expr(r: &mut Reader<'_>) -> Result<Vec<Instr>> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    loop {
        if r.is_empty() {
            return Err(WasmError::malformed("unexpected end of section or function"));
        }
        let instr = decode_instr(r)?;
        match &instr {
            Instr::Block(_) | Instr::Loop(_) | Instr::If(_) => depth += 1,
            Instr::End => {
                if depth == 0 {
                    out.push(instr);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(instr);
    }
    Ok(out)
}

fn decode_block_type(r: &mut Reader<'_>) -> Result<BlockType> {
    let v = r.var_s33()?;
    if v == -64 {
        return Ok(BlockType::Empty);
    }
    if v < 0 {
        return ValType::from_s33(v)
            .map(BlockType::Value)
            .ok_or_else(|| WasmError::malformed("malformed block type"));
    }
    Ok(BlockType::TypeIdx(v as u32))
}

/// Decode one instruction (including its immediates).
pub fn decode_instr(r: &mut Reader<'_>) -> Result<Instr> {
    let op = r.u8_raw("instruction")?;
    Ok(match op {
        0x00 => Instr::Unreachable,
        0x01 => Instr::Nop,
        0x02 => Instr::Block(decode_block_type(r)?),
        0x03 => Instr::Loop(decode_block_type(r)?),
        0x04 => Instr::If(decode_block_type(r)?),
        0x05 => Instr::Else,
        0x0B => Instr::End,
        0x0C => Instr::Br(r.var_u32()?),
        0x0D => Instr::BrIf(r.var_u32()?),
        0x0E => {
            let targets = r.vec("br_table", |r| r.var_u32())?;
            let default = r.var_u32()?;
            Instr::BrTable(targets.into_boxed_slice(), default)
        }
        0x0F => Instr::Return,
        0x10 => Instr::Call(r.var_u32()?),
        0x11 => {
            let ty = r.var_u32()?;
            let table = r.var_u32()?;
            Instr::CallIndirect { ty, table }
        }
        0x1A => Instr::Drop,
        0x1B => Instr::Select,
        0x1C => {
            let tys = r.vec("select", decode_val_type)?;
            Instr::SelectT(tys.into_boxed_slice())
        }
        0x20 => Instr::LocalGet(r.var_u32()?),
        0x21 => Instr::LocalSet(r.var_u32()?),
        0x22 => Instr::LocalTee(r.var_u32()?),
        0x23 => Instr::GlobalGet(r.var_u32()?),
        0x24 => Instr::GlobalSet(r.var_u32()?),
        0x25 => Instr::TableGet(r.var_u32()?),
        0x26 => Instr::TableSet(r.var_u32()?),
        0x28..=0x3E => {
            let memarg = decode_memarg(r)?;
            decode_mem_op(op, memarg)?
        }
        0x3F => decode_reserved_memory(r, Instr::MemorySize(0))?,
        0x40 => decode_reserved_memory(r, Instr::MemoryGrow(0))?,
        0x41 => Instr::I32Const(r.var_i32()?),
        0x42 => Instr::I64Const(r.var_i64()?),
        0x43 => Instr::F32Const(r.f32_bits()?),
        0x44 => Instr::F64Const(r.f64_bits()?),
        0x45..=0xC4 => Instr::Num(NumOp::from_byte(op).expect("range is fully mapped")),
        0xD0 => Instr::RefNull(decode_ref_type(r)?),
        0xD1 => Instr::RefIsNull,
        0xD2 => Instr::RefFunc(r.var_u32()?),
        0xFC => {
            let sub = r.var_u32()?;
            if sub <= 7 {
                let op = NumOp::from_fc(sub as u8).expect("0..=7 mapped");
                Instr::Num(op)
            } else {
                match sub {
                    8 => Instr::MemoryInit { data: r.var_u32()?, memory: r.var_u32()? },
                    9 => Instr::DataDrop(r.var_u32()?),
                    10 => Instr::MemoryCopy { dst: r.var_u32()?, src: r.var_u32()? },
                    11 => Instr::MemoryFill(r.var_u32()?),
                    12 => Instr::TableInit { elem: r.var_u32()?, table: r.var_u32()? },
                    13 => Instr::ElemDrop(r.var_u32()?),
                    14 => Instr::TableCopy { dst: r.var_u32()?, src: r.var_u32()? },
                    15 => Instr::TableGrow(r.var_u32()?),
                    16 => Instr::TableSize(r.var_u32()?),
                    17 => Instr::TableFill(r.var_u32()?),
                    _ => return Err(WasmError::malformed("illegal opcode")),
                }
            }
        }
        _ => return Err(WasmError::malformed("illegal opcode")),
    })
}

fn decode_reserved_memory(r: &mut Reader<'_>, make: Instr) -> Result<Instr> {
    let idx = r.var_u32()?;
    if idx != 0 {
        return Err(WasmError::malformed("zero byte expected"));
    }
    Ok(make)
}

fn decode_memarg(r: &mut Reader<'_>) -> Result<MemArg> {
    let align = r.var_u32()?;
    let offset = r.var_u32()?;
    if align > 31 {
        return Err(WasmError::malformed("malformed memop flags"));
    }
    Ok(MemArg { align, offset, memory: 0 })
}

fn decode_mem_op(op: u8, memarg: MemArg) -> Result<Instr> {
    let load = |l| Instr::Load(l, memarg);
    let store = |s| Instr::Store(s, memarg);
    Ok(match op {
        0x28 => load(LoadOp::I32),
        0x29 => load(LoadOp::I64),
        0x2A => load(LoadOp::F32),
        0x2B => load(LoadOp::F64),
        0x2C => load(LoadOp::I32_8S),
        0x2D => load(LoadOp::I32_8U),
        0x2E => load(LoadOp::I32_16S),
        0x2F => load(LoadOp::I32_16U),
        0x30 => load(LoadOp::I64_8S),
        0x31 => load(LoadOp::I64_8U),
        0x32 => load(LoadOp::I64_16S),
        0x33 => load(LoadOp::I64_16U),
        0x34 => load(LoadOp::I64_32S),
        0x35 => load(LoadOp::I64_32U),
        0x36 => store(StoreOp::I32),
        0x37 => store(StoreOp::I64),
        0x38 => store(StoreOp::F32),
        0x39 => store(StoreOp::F64),
        0x3A => store(StoreOp::I32_8),
        0x3B => store(StoreOp::I32_16),
        0x3C => store(StoreOp::I64_8),
        0x3D => store(StoreOp::I64_16),
        0x3E => store(StoreOp::I64_32),
        _ => return Err(WasmError::malformed("illegal opcode")),
    })
}
