//! WebAssembly for Amazing.
//!
//! A from-scratch, `std`-only implementation of the Wasm 2.0 core: the binary
//! decoder and encoder, the text (`.wat`) parser, the type-checking validator,
//! a bytecode-free structured interpreter with linear memory and traps, and a
//! runner for the WebAssembly spec testsuite `.wast` scripts.
//!
//! The testsuite path always encodes parsed text back to binary and decodes it
//! again, so the binary decoder is exercised by every text module.
#![forbid(unsafe_code)]

pub mod decode;
pub mod embed;
pub mod encode;
pub mod error;
pub mod instr;
pub mod interp;
pub mod module;
pub mod numlit;
pub mod opname;
pub mod reader;
pub mod sexpr;
pub mod store;
pub mod types;
pub mod validate;
pub mod value;
pub mod wast;
pub mod wat;

pub use decode::decode_module;
pub use embed::{
    compile, CompiledModule, Engine, ExportType, ExternKind, ExternValue, FuncHandle,
    GlobalHandle, ImportResolver, ImportType, InstanceHandle, JsErrorClass, MemoryHandle,
    TableHandle,
};
pub use encode::encode_module;
pub use error::{ErrorKind, Result, WasmError};
pub use interp::Interpreter;
pub use module::Module;
pub use store::{Extern, Store};
pub use types::{
    BlockType, FuncType, GlobalType, Limits, MemArg, MemoryType, TableType, ValType,
};
pub use validate::validate_module;
pub use value::{Ref, Value};
pub use wast::{run_directory, run_wast_source, WastStats};
pub use wat::{parse_module, ParsedModule};
