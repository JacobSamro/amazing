//! WebAssembly value, function, table, memory and global types.

/// A value type. `FuncRef`/`ExternRef` require the reference-types proposal;
/// `V128` is decoded but no SIMD instruction is implemented.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ValType {
    /// 32-bit integer.
    I32,
    /// 64-bit integer.
    I64,
    /// 32-bit float.
    F32,
    /// 64-bit float.
    F64,
    /// 128-bit vector (decoded, not executed).
    V128,
    /// Reference to a function.
    FuncRef,
    /// Opaque host reference.
    ExternRef,
}

impl ValType {
    /// The binary encoding byte.
    pub fn byte(self) -> u8 {
        match self {
            ValType::I32 => 0x7F,
            ValType::I64 => 0x7E,
            ValType::F32 => 0x7D,
            ValType::F64 => 0x7C,
            ValType::V128 => 0x7B,
            ValType::FuncRef => 0x70,
            ValType::ExternRef => 0x6F,
        }
    }

    /// Decode a single value-type byte.
    pub fn from_byte(b: u8) -> Option<ValType> {
        Some(match b {
            0x7F => ValType::I32,
            0x7E => ValType::I64,
            0x7D => ValType::F32,
            0x7C => ValType::F64,
            0x7B => ValType::V128,
            0x70 => ValType::FuncRef,
            0x6F => ValType::ExternRef,
            _ => return None,
        })
    }

    /// Map a negative block-type `s33` value onto its value type.
    pub fn from_s33(v: i64) -> Option<ValType> {
        match v {
            -1 => Some(ValType::I32),
            -2 => Some(ValType::I64),
            -3 => Some(ValType::F32),
            -4 => Some(ValType::F64),
            -5 => Some(ValType::V128),
            -16 => Some(ValType::FuncRef),
            -17 => Some(ValType::ExternRef),
            _ => None,
        }
    }

    /// True for `i32`/`i64`/`f32`/`f64`.
    pub fn is_num(self) -> bool {
        matches!(self, ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64)
    }

    /// True for `funcref`/`externref`.
    pub fn is_ref(self) -> bool {
        matches!(self, ValType::FuncRef | ValType::ExternRef)
    }

    /// True for `v128`.
    pub fn is_vec(self) -> bool {
        matches!(self, ValType::V128)
    }

    /// The WAT spelling (`i32`, `funcref`, ...).
    pub fn name(self) -> &'static str {
        match self {
            ValType::I32 => "i32",
            ValType::I64 => "i64",
            ValType::F32 => "f32",
            ValType::F64 => "f64",
            ValType::V128 => "v128",
            ValType::FuncRef => "funcref",
            ValType::ExternRef => "externref",
        }
    }

    /// Parse a WAT value-type spelling.
    pub fn parse(name: &str) -> Option<ValType> {
        Some(match name {
            "i32" => ValType::I32,
            "i64" => ValType::I64,
            "f32" => ValType::F32,
            "f64" => ValType::F64,
            "v128" => ValType::V128,
            "funcref" | "anyfunc" => ValType::FuncRef,
            "externref" => ValType::ExternRef,
            _ => return None,
        })
    }
}

/// A function signature.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct FuncType {
    /// Parameter types in order.
    pub params: Vec<ValType>,
    /// Result types in order (multi-value allowed).
    pub results: Vec<ValType>,
}

impl FuncType {
    /// Build a signature.
    pub fn new(params: Vec<ValType>, results: Vec<ValType>) -> Self {
        FuncType { params, results }
    }
}

/// Resizable limits shared by tables and memories.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Limits {
    /// Minimum size.
    pub min: u32,
    /// Optional maximum size.
    pub max: Option<u32>,
}

impl Limits {
    /// Build limits.
    pub fn new(min: u32, max: Option<u32>) -> Self {
        Limits { min, max }
    }

    /// True when these limits are contained in `other` (an import is usable
    /// where `other` is expected).
    pub fn matches(&self, other: &Limits) -> bool {
        self.min >= other.min
            && match (self.max, other.max) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some(a), Some(b)) => a <= b,
            }
    }
}

/// A table type: element reference type plus limits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TableType {
    /// The reference type stored in the table.
    pub elem: ValType,
    /// Table size limits.
    pub limits: Limits,
}

/// A linear-memory type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MemoryType {
    /// Page limits.
    pub limits: Limits,
}

/// A global variable type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlobalType {
    /// Value type.
    pub val: ValType,
    /// Whether the global is mutable.
    pub mutable: bool,
}

impl GlobalType {
    /// True when a global of this type can be used where `other` is expected.
    pub fn matches(&self, other: &GlobalType) -> bool {
        self.val == other.val && self.mutable == other.mutable
    }
}

/// The type of a block, loop or `if`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BlockType {
    /// No parameters, no results (`0x40`).
    Empty,
    /// A single result, or no parameters and a single result.
    Value(ValType),
    /// A full function type from the type section.
    TypeIdx(u32),
}

/// A memory access immediate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MemArg {
    /// Alignment as a power-of-two exponent.
    pub align: u32,
    /// Static byte offset.
    pub offset: u32,
    /// Memory index (0 unless multi-memory).
    pub memory: u32,
}
