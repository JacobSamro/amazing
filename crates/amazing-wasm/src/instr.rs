//! Instructions and the numeric opcode table.
//!
//! Function bodies are stored *flat*: structured control (`block`, `loop`,
//! `if`, `else`, `end`) is kept as ordinary instructions. The validator and
//! interpreter maintain their own control/label stacks, which is exactly the
//! shape the specification's algorithms expect.

use crate::types::{BlockType, MemArg, ValType};

/// The operand/result shape of a numeric operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumSig {
    /// `[t] -> [i32]`.
    Test(ValType),
    /// `[t, t] -> [i32]`.
    Cmp(ValType),
    /// `[t] -> [t]`.
    Unary(ValType),
    /// `[t, t] -> [t]`.
    Binary(ValType),
    /// `[from] -> [to]`.
    Convert(ValType, ValType),
}

macro_rules! num_ops {
    ($( $variant:ident = $byte:expr => $sig:ident $($ty:ident),+ ; )*) => {
        /// A numeric instruction (integer, float, conversion, sign-extension,
         /// or a `0xFC` saturating float-to-int conversion).
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum NumOp { $( $variant, )* }

        impl NumOp {
            /// Decode a single-byte numeric opcode (`0x45..=0xC4`).
            pub fn from_byte(b: u8) -> Option<NumOp> {
                Some(match u16::from(b) {
                    $( $byte => NumOp::$variant, )*
                    _ => return None,
                })
            }

            /// Decode a `0xFC`-prefixed numeric opcode.
            pub fn from_fc(sub: u8) -> Option<NumOp> {
                Some(match 0xFC00u16 | u16::from(sub) {
                    $( $byte => NumOp::$variant, )*
                    _ => return None,
                })
            }

            /// The defining byte(s): low byte alone, or `0xFC00 | sub`.
            pub fn byte(self) -> u16 {
                match self { $( NumOp::$variant => $byte, )* }
            }

            /// Operand and result types.
            pub fn signature(self) -> NumSig {
                match self { $( NumOp::$variant => NumSig::$sig($(ValType::$ty),+), )* }
            }
        }
    };
}

num_ops! {
    I32Eqz = 0x45 => Test I32;
    I32Eq = 0x46 => Cmp I32;
    I32Ne = 0x47 => Cmp I32;
    I32LtS = 0x48 => Cmp I32;
    I32LtU = 0x49 => Cmp I32;
    I32GtS = 0x4A => Cmp I32;
    I32GtU = 0x4B => Cmp I32;
    I32LeS = 0x4C => Cmp I32;
    I32LeU = 0x4D => Cmp I32;
    I32GeS = 0x4E => Cmp I32;
    I32GeU = 0x4F => Cmp I32;
    I64Eqz = 0x50 => Test I64;
    I64Eq = 0x51 => Cmp I64;
    I64Ne = 0x52 => Cmp I64;
    I64LtS = 0x53 => Cmp I64;
    I64LtU = 0x54 => Cmp I64;
    I64GtS = 0x55 => Cmp I64;
    I64GtU = 0x56 => Cmp I64;
    I64LeS = 0x57 => Cmp I64;
    I64LeU = 0x58 => Cmp I64;
    I64GeS = 0x59 => Cmp I64;
    I64GeU = 0x5A => Cmp I64;
    F32Eq = 0x5B => Cmp F32;
    F32Ne = 0x5C => Cmp F32;
    F32Lt = 0x5D => Cmp F32;
    F32Gt = 0x5E => Cmp F32;
    F32Le = 0x5F => Cmp F32;
    F32Ge = 0x60 => Cmp F32;
    F64Eq = 0x61 => Cmp F64;
    F64Ne = 0x62 => Cmp F64;
    F64Lt = 0x63 => Cmp F64;
    F64Gt = 0x64 => Cmp F64;
    F64Le = 0x65 => Cmp F64;
    F64Ge = 0x66 => Cmp F64;
    I32Clz = 0x67 => Unary I32;
    I32Ctz = 0x68 => Unary I32;
    I32Popcnt = 0x69 => Unary I32;
    I32Add = 0x6A => Binary I32;
    I32Sub = 0x6B => Binary I32;
    I32Mul = 0x6C => Binary I32;
    I32DivS = 0x6D => Binary I32;
    I32DivU = 0x6E => Binary I32;
    I32RemS = 0x6F => Binary I32;
    I32RemU = 0x70 => Binary I32;
    I32And = 0x71 => Binary I32;
    I32Or = 0x72 => Binary I32;
    I32Xor = 0x73 => Binary I32;
    I32Shl = 0x74 => Binary I32;
    I32ShrS = 0x75 => Binary I32;
    I32ShrU = 0x76 => Binary I32;
    I32Rotl = 0x77 => Binary I32;
    I32Rotr = 0x78 => Binary I32;
    I64Clz = 0x79 => Unary I64;
    I64Ctz = 0x7A => Unary I64;
    I64Popcnt = 0x7B => Unary I64;
    I64Add = 0x7C => Binary I64;
    I64Sub = 0x7D => Binary I64;
    I64Mul = 0x7E => Binary I64;
    I64DivS = 0x7F => Binary I64;
    I64DivU = 0x80 => Binary I64;
    I64RemS = 0x81 => Binary I64;
    I64RemU = 0x82 => Binary I64;
    I64And = 0x83 => Binary I64;
    I64Or = 0x84 => Binary I64;
    I64Xor = 0x85 => Binary I64;
    I64Shl = 0x86 => Binary I64;
    I64ShrS = 0x87 => Binary I64;
    I64ShrU = 0x88 => Binary I64;
    I64Rotl = 0x89 => Binary I64;
    I64Rotr = 0x8A => Binary I64;
    F32Abs = 0x8B => Unary F32;
    F32Neg = 0x8C => Unary F32;
    F32Ceil = 0x8D => Unary F32;
    F32Floor = 0x8E => Unary F32;
    F32Trunc = 0x8F => Unary F32;
    F32Nearest = 0x90 => Unary F32;
    F32Sqrt = 0x91 => Unary F32;
    F32Add = 0x92 => Binary F32;
    F32Sub = 0x93 => Binary F32;
    F32Mul = 0x94 => Binary F32;
    F32Div = 0x95 => Binary F32;
    F32Min = 0x96 => Binary F32;
    F32Max = 0x97 => Binary F32;
    F32Copysign = 0x98 => Binary F32;
    F64Abs = 0x99 => Unary F64;
    F64Neg = 0x9A => Unary F64;
    F64Ceil = 0x9B => Unary F64;
    F64Floor = 0x9C => Unary F64;
    F64Trunc = 0x9D => Unary F64;
    F64Nearest = 0x9E => Unary F64;
    F64Sqrt = 0x9F => Unary F64;
    F64Add = 0xA0 => Binary F64;
    F64Sub = 0xA1 => Binary F64;
    F64Mul = 0xA2 => Binary F64;
    F64Div = 0xA3 => Binary F64;
    F64Min = 0xA4 => Binary F64;
    F64Max = 0xA5 => Binary F64;
    F64Copysign = 0xA6 => Binary F64;
    I32WrapI64 = 0xA7 => Convert I64, I32;
    I32TruncF32S = 0xA8 => Convert F32, I32;
    I32TruncF32U = 0xA9 => Convert F32, I32;
    I32TruncF64S = 0xAA => Convert F64, I32;
    I32TruncF64U = 0xAB => Convert F64, I32;
    I64ExtendI32S = 0xAC => Convert I32, I64;
    I64ExtendI32U = 0xAD => Convert I32, I64;
    I64TruncF32S = 0xAE => Convert F32, I64;
    I64TruncF32U = 0xAF => Convert F32, I64;
    I64TruncF64S = 0xB0 => Convert F64, I64;
    I64TruncF64U = 0xB1 => Convert F64, I64;
    F32ConvertI32S = 0xB2 => Convert I32, F32;
    F32ConvertI32U = 0xB3 => Convert I32, F32;
    F32ConvertI64S = 0xB4 => Convert I64, F32;
    F32ConvertI64U = 0xB5 => Convert I64, F32;
    F32DemoteF64 = 0xB6 => Convert F64, F32;
    F64ConvertI32S = 0xB7 => Convert I32, F64;
    F64ConvertI32U = 0xB8 => Convert I32, F64;
    F64ConvertI64S = 0xB9 => Convert I64, F64;
    F64ConvertI64U = 0xBA => Convert I64, F64;
    F64PromoteF32 = 0xBB => Convert F32, F64;
    I32ReinterpretF32 = 0xBC => Convert F32, I32;
    I64ReinterpretF64 = 0xBD => Convert F64, I64;
    F32ReinterpretI32 = 0xBE => Convert I32, F32;
    F64ReinterpretI64 = 0xBF => Convert I64, F64;
    I32Extend8S = 0xC0 => Unary I32;
    I32Extend16S = 0xC1 => Unary I32;
    I64Extend8S = 0xC2 => Unary I64;
    I64Extend16S = 0xC3 => Unary I64;
    I64Extend32S = 0xC4 => Unary I64;
    I32TruncSatF32S = 0xFC00 => Convert F32, I32;
    I32TruncSatF32U = 0xFC01 => Convert F32, I32;
    I32TruncSatF64S = 0xFC02 => Convert F64, I32;
    I32TruncSatF64U = 0xFC03 => Convert F64, I32;
    I64TruncSatF32S = 0xFC04 => Convert F32, I64;
    I64TruncSatF32U = 0xFC05 => Convert F32, I64;
    I64TruncSatF64S = 0xFC06 => Convert F64, I64;
    I64TruncSatF64U = 0xFC07 => Convert F64, I64;
}

/// A linear-memory load instruction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LoadOp {
    /// `i32.load`.
    I32,
    /// `i64.load`.
    I64,
    /// `f32.load`.
    F32,
    /// `f64.load`.
    F64,
    /// `i32.load8_s`.
    I32_8S,
    /// `i32.load8_u`.
    I32_8U,
    /// `i32.load16_s`.
    I32_16S,
    /// `i32.load16_u`.
    I32_16U,
    /// `i64.load8_s`.
    I64_8S,
    /// `i64.load8_u`.
    I64_8U,
    /// `i64.load16_s`.
    I64_16S,
    /// `i64.load16_u`.
    I64_16U,
    /// `i64.load32_s`.
    I64_32S,
    /// `i64.load32_u`.
    I64_32U,
}

impl LoadOp {
    /// The value pushed on the stack.
    pub fn result(self) -> ValType {
        match self {
            LoadOp::I32 | LoadOp::I32_8S | LoadOp::I32_8U | LoadOp::I32_16S | LoadOp::I32_16U => ValType::I32,
            LoadOp::I64
            | LoadOp::I64_8S
            | LoadOp::I64_8U
            | LoadOp::I64_16S
            | LoadOp::I64_16U
            | LoadOp::I64_32S
            | LoadOp::I64_32U => ValType::I64,
            LoadOp::F32 => ValType::F32,
            LoadOp::F64 => ValType::F64,
        }
    }

    /// Accessed byte width.
    pub fn width(self) -> u32 {
        match self {
            LoadOp::I32 | LoadOp::F32 | LoadOp::I64_32S | LoadOp::I64_32U => 4,
            LoadOp::I64 | LoadOp::F64 => 8,
            LoadOp::I32_8S | LoadOp::I32_8U | LoadOp::I64_8S | LoadOp::I64_8U => 1,
            LoadOp::I32_16S | LoadOp::I32_16U | LoadOp::I64_16S | LoadOp::I64_16U => 2,
        }
    }

    /// Whether the loaded value is sign extended before widening.
    pub fn signed(self) -> bool {
        matches!(
            self,
            LoadOp::I32_8S | LoadOp::I32_16S | LoadOp::I64_8S | LoadOp::I64_16S | LoadOp::I64_32S
        )
    }
}

/// A linear-memory store instruction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StoreOp {
    /// `i32.store`.
    I32,
    /// `i64.store`.
    I64,
    /// `f32.store`.
    F32,
    /// `f64.store`.
    F64,
    /// `i32.store8`.
    I32_8,
    /// `i32.store16`.
    I32_16,
    /// `i64.store8`.
    I64_8,
    /// `i64.store16`.
    I64_16,
    /// `i64.store32`.
    I64_32,
}

impl StoreOp {
    /// The value consumed from the stack.
    pub fn operand(self) -> ValType {
        match self {
            StoreOp::I32 | StoreOp::I32_8 | StoreOp::I32_16 => ValType::I32,
            StoreOp::I64 | StoreOp::I64_8 | StoreOp::I64_16 | StoreOp::I64_32 => ValType::I64,
            StoreOp::F32 => ValType::F32,
            StoreOp::F64 => ValType::F64,
        }
    }

    /// Stored byte width.
    pub fn width(self) -> u32 {
        match self {
            StoreOp::I32 | StoreOp::F32 | StoreOp::I64_32 => 4,
            StoreOp::I64 | StoreOp::F64 => 8,
            StoreOp::I32_8 | StoreOp::I64_8 => 1,
            StoreOp::I32_16 | StoreOp::I64_16 => 2,
        }
    }
}

/// A decoded instruction.
#[derive(Clone, PartialEq, Debug)]
pub enum Instr {
    /// Trap unconditionally.
    Unreachable,
    /// Do nothing.
    Nop,
    /// Enter a block.
    Block(BlockType),
    /// Enter a loop.
    Loop(BlockType),
    /// Enter a conditional.
    If(BlockType),
    /// The else arm of an `if`.
    Else,
    /// End the innermost block.
    End,
    /// Branch to a label.
    Br(u32),
    /// Branch if the top of the stack is non-zero.
    BrIf(u32),
    /// Branch through a table of labels.
    BrTable(Box<[u32]>, u32),
    /// Return from the function.
    Return,
    /// Call a function by index.
    Call(u32),
    /// Call a function through a table.
    CallIndirect {
        /// Required function type index.
        ty: u32,
        /// Table to index.
        table: u32,
    },
    /// Discard the top of the stack.
    Drop,
    /// Select one of two numeric or reference operands.
    Select,
    /// Typed `select` from the reference-types proposal.
    SelectT(Box<[ValType]>),
    /// Read a local.
    LocalGet(u32),
    /// Write a local.
    LocalSet(u32),
    /// Write a local and keep the value.
    LocalTee(u32),
    /// Read a global.
    GlobalGet(u32),
    /// Write a global.
    GlobalSet(u32),
    /// Read a table element.
    TableGet(u32),
    /// Write a table element.
    TableSet(u32),
    /// Copy from an element segment into a table.
    TableInit {
        /// Source element segment.
        elem: u32,
        /// Target table.
        table: u32,
    },
    /// Drop an element segment.
    ElemDrop(u32),
    /// Copy between tables.
    TableCopy {
        /// Destination table.
        dst: u32,
        /// Source table.
        src: u32,
    },
    /// Grow a table.
    TableGrow(u32),
    /// Current table size.
    TableSize(u32),
    /// Fill a table range.
    TableFill(u32),
    /// Push a null reference.
    RefNull(ValType),
    /// Whether the top reference is null.
    RefIsNull,
    /// Push a reference to a function.
    RefFunc(u32),
    /// Load from linear memory.
    Load(LoadOp, MemArg),
    /// Store to linear memory.
    Store(StoreOp, MemArg),
    /// Current memory size in pages.
    MemorySize(u32),
    /// Grow memory by a page count.
    MemoryGrow(u32),
    /// Copy from a data segment into memory.
    MemoryInit {
        /// Source data segment.
        data: u32,
        /// Target memory.
        memory: u32,
    },
    /// Drop a data segment.
    DataDrop(u32),
    /// Copy within/between memories.
    MemoryCopy {
        /// Destination memory.
        dst: u32,
        /// Source memory.
        src: u32,
    },
    /// Fill a memory range with a byte.
    MemoryFill(u32),
    /// `i32.const`.
    I32Const(i32),
    /// `i64.const`.
    I64Const(i64),
    /// `f32.const`, stored as bits to keep `Eq`.
    F32Const(u32),
    /// `f64.const`, stored as bits.
    F64Const(u64),
    /// A numeric operator.
    Num(NumOp),
}
