//! The spec-testsuite `.wast` script runner.
//!
//! A `.wast` file is a sequence of script commands: module definitions,
//! `register`, `invoke`/`get` actions and `assert_*` checks. This module
//! interprets those commands against the [`Store`], classifying each failure
//! by the spec's taxonomy so conformance can be counted per file.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::rc::Rc;

use crate::decode::decode_module;
use crate::encode::encode_module;
use crate::error::{ErrorKind, Result, WasmError};
use crate::interp::Interpreter;
use crate::module::Module;
use crate::sexpr::{self, Sexpr};
use crate::store::{Extern, FuncInstance, GlobalInstance, Instance, MemoryInstance, Store, TableInstance};
use crate::types::{FuncType, GlobalType, Limits, MemoryType, TableType, ValType};
use crate::validate::validate_module;
use crate::value::{
    is_arithmetic_f32, is_arithmetic_f64, is_canonical_f32, is_canonical_f64, Ref, Value,
};
use crate::{numlit, wat};

/// The result of running one `.wast` file.
#[derive(Clone, Debug, Default)]
pub struct WastStats {
    /// File name.
    pub file: String,
    /// Number of checked commands.
    pub total: usize,
    /// Number of passing commands.
    pub passed: usize,
    /// Human-readable failures (command index and reason).
    pub failures: Vec<String>,
}

impl WastStats {
    /// Number of failing commands.
    pub fn failed(&self) -> usize {
        self.total - self.passed
    }
}

/// Run every `.wast` file in `dir` except those whose name contains a skip
/// token, returning one [`WastStats`] per file.
pub fn run_directory(dir: &Path, skip: &[&str]) -> Vec<WastStats> {
    let mut files: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(_) => return Vec::new(),
    };
    files.retain(|p| p.extension().is_some_and(|e| e == "wast"));
    files.retain(|p| {
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        !skip.iter().any(|s| name.contains(s))
    });
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let source = fs::read_to_string(&p).unwrap_or_default();
            let file = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            run_wast_source(&file, &source)
        })
        .collect()
}

/// Run a single `.wast` source, counting commands in `file`.
pub fn run_wast_source(file: &str, source: &str) -> WastStats {
    let mut stats = WastStats { file: file.to_owned(), ..Default::default() };
    let commands = match sexpr::parse(source) {
        Ok(c) => c,
        Err(e) => {
            stats.total += 1;
            stats.failures.push(format!("script parse error: {e}"));
            return stats;
        }
    };
    let mut runner = Runner::new();
    for (i, cmd) in commands.iter().enumerate() {
        if runner.run_command(cmd, i, &mut stats).is_err() {
            // Individual commands record their own failure; this only catches
            // runner-level surprises.
        }
    }
    stats
}

struct Runner {
    store: Store,
    registry: Vec<(String, usize)>,
    named: HashMap<String, usize>,
    last: Option<usize>,
}

impl Runner {
    fn new() -> Self {
        let mut runner = Runner {
            store: Store::new(),
            registry: Vec::new(),
            named: HashMap::new(),
            last: None,
        };
        let spectest = runner.make_spectest();
        runner.registry.push(("spectest".to_owned(), spectest));
        runner
    }

    /// The standard `spectest` host module.
    fn make_spectest(&mut self) -> usize {
        let mut exports = BTreeMap::new();
        let void: Rc<dyn Fn(&[Value]) -> Result<Vec<Value>>> = Rc::new(|_| Ok(Vec::new()));
        let simples: &[(&str, &[ValType])] = &[
            ("print", &[]),
            ("print_i32", &[ValType::I32]),
            ("print_i64", &[ValType::I64]),
            ("print_f32", &[ValType::F32]),
            ("print_f64", &[ValType::F64]),
            ("print_i32_f32", &[ValType::I32, ValType::F32]),
            ("print_f64_f64", &[ValType::F64, ValType::F64]),
        ];
        let mut funcs = Vec::new();
        for (name, params) in simples {
            let addr = self.store.funcs.len() as u32;
            self.store.funcs.push(FuncInstance::Host {
                ty: FuncType { params: params.to_vec(), results: Vec::new() },
                name: (*name).to_owned(),
                func: void.clone(),
            });
            funcs.push(addr);
            exports.insert((*name).to_owned(), Extern::Func(addr));
        }
        let table_addr = self.store.tables.len() as u32;
        self.store.tables.push(TableInstance::new(TableType {
            elem: ValType::FuncRef,
            limits: Limits { min: 10, max: Some(20) },
        }));
        exports.insert("table".to_owned(), Extern::Table(table_addr));
        let mem_addr = self.store.memories.len() as u32;
        self.store.memories.push(MemoryInstance::new(MemoryType {
            limits: Limits { min: 1, max: Some(2) },
        }));
        exports.insert("memory".to_owned(), Extern::Memory(mem_addr));
        let mut globals = Vec::new();
        for (name, ty) in [
            ("global_i32", GlobalType { val: ValType::I32, mutable: false }),
            ("global_i64", GlobalType { val: ValType::I64, mutable: false }),
            ("global_f32", GlobalType { val: ValType::F32, mutable: false }),
            ("global_f64", GlobalType { val: ValType::F64, mutable: false }),
        ] {
            let value = match ty.val {
                ValType::I32 => Value::I32(666),
                ValType::I64 => Value::I64(666),
                ValType::F32 => Value::F32(666.6),
                _ => Value::F64(666.6),
            };
            let addr = self.store.globals.len() as u32;
            self.store.globals.push(GlobalInstance { ty, value });
            globals.push(addr);
            exports.insert(name.to_owned(), Extern::Global(addr));
        }
        self.store.instances.push(Instance {
            module: Rc::new(Module::default()),
            exports,
            funcs,
            tables: vec![table_addr],
            memories: vec![mem_addr],
            globals,
            elems: Vec::new(),
            datas: Vec::new(),
            name: Some("spectest".to_owned()),
        });
        self.store.instances.len() - 1
    }

    fn resolve_map(&self) -> HashMap<(String, String), Extern> {
        let mut map = HashMap::new();
        for (name, inst) in &self.registry {
            if let Some(instance) = self.store.instances.get(*inst) {
                for (field, ext) in &instance.exports {
                    map.insert((name.clone(), field.clone()), *ext);
                }
            }
        }
        map
    }

    fn compile_text(&self, expr: &Sexpr) -> Result<Module> {
        let parsed = wat::parse_module(expr)?;
        let bytes = encode_module(&parsed.module);
        let decoded = match decode_module(&bytes) {
            Ok(d) => d,
            Err(e) => {
                if std::env::var_os("WASM_DEBUG").is_some() {
                    eprintln!("decode failed: {e}\nhex: {}", to_hex(&bytes));
                }
                return Err(e);
            }
        };
        validate_module(&decoded)?;
        Ok(decoded)
    }

    fn compile_quoted(&self, text: &str) -> Result<Module> {
        let parsed = wat::parse_quoted(text)?;
        let bytes = encode_module(&parsed.module);
        let decoded = decode_module(&bytes)?;
        validate_module(&decoded)?;
        Ok(decoded)
    }

    fn instantiate(&mut self, module: Module, name: Option<String>) -> Result<usize> {
        let map = self.resolve_map();
        let module = Rc::new(module);
        let inst = self.store.instantiate(module, &|m, n, _| {
            map.get(&(m.to_owned(), n.to_owned())).copied()
        })?;
        let start = self.store.instances[inst].module.start;
        if let Some(start) = start {
            let addr = self.store.instances[inst].funcs[start as usize];
            let mut interp = Interpreter::new(&mut self.store);
            interp.invoke(inst, addr, Vec::new())?;
        }
        if let Some(name) = name {
            self.named.insert(name, inst);
        }
        self.last = Some(inst);
        Ok(inst)
    }

    fn module_form(expr: &Sexpr) -> Option<ModuleForm> {
        let list = expr.list()?;
        if list.first().and_then(Sexpr::atom) != Some("module") {
            return None;
        }
        let mut i = 1;
        if matches!(list.get(1).and_then(Sexpr::atom), Some(a) if a.starts_with('$')) {
            i = 2;
        }
        match list.get(i).and_then(Sexpr::atom) {
            Some("binary") => Some(ModuleForm::Binary(concat_bytes(&list[i + 1..]))),
            Some("quote") => Some(ModuleForm::Quote(concat_text(&list[i + 1..]))),
            _ => Some(ModuleForm::Text(i)),
        }
    }

    fn run_command(&mut self, cmd: &Sexpr, index: usize, stats: &mut WastStats) -> Result<()> {
        let head = cmd.head().unwrap_or("");
        match head {
            "module" => {
                stats.total += 1;
                match self.instantiate_command(cmd).and_then(|_| Ok(())) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} module: {e}")),
                }
            }
            "register" => {
                let list = cmd.list().expect("register");
                let name = string_of(list.get(1))?;
                let inst = match list.get(2).and_then(Sexpr::atom) {
                    Some(a) if a.starts_with('$') => {
                        self.named.get(&a[1..]).copied().ok_or_else(|| WasmError::trap("unknown module"))?
                    }
                    _ => self.last.ok_or_else(|| WasmError::trap("no module to register"))?,
                };
                self.registry.retain(|(n, _)| n != &name);
                self.registry.push((name, inst));
            }
            "invoke" => {
                stats.total += 1;
                match self.run_action(cmd) {
                    Ok(_) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} invoke: {e}")),
                }
            }
            "get" => {
                stats.total += 1;
                match self.run_action(cmd) {
                    Ok(_) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} get: {e}")),
                }
            }
            "assert_return" => {
                stats.total += 1;
                match self.assert_return(cmd) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_return: {e}")),
                }
            }
            "assert_trap" => {
                stats.total += 1;
                match self.assert_error(cmd, &[ErrorKind::Trap]) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_trap: {e}")),
                }
            }
            "assert_exhaustion" => {
                stats.total += 1;
                match self.assert_error(cmd, &[ErrorKind::Exhaustion, ErrorKind::Trap]) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_exhaustion: {e}")),
                }
            }
            "assert_invalid" => {
                stats.total += 1;
                match self.assert_invalid(cmd) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_invalid: {e}")),
                }
            }
            "assert_malformed" => {
                stats.total += 1;
                match self.assert_malformed(cmd) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_malformed: {e}")),
                }
            }
            "assert_unlinkable" => {
                stats.total += 1;
                match self.assert_error(cmd, &[ErrorKind::Unlinkable, ErrorKind::Trap]) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_unlinkable: {e}")),
                }
            }
            "assert_return_canonical_nan" => {
                stats.total += 1;
                match self.assert_legacy_nan(cmd, true) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_return_canonical_nan: {e}")),
                }
            }
            "assert_return_arithmetic_nan" => {
                stats.total += 1;
                match self.assert_legacy_nan(cmd, false) {
                    Ok(()) => stats.passed += 1,
                    Err(e) => stats.failures.push(format!("#{index} assert_return_arithmetic_nan: {e}")),
                }
            }
            "input" | "output" | "script" => {}
            _ => {}
        }
        Ok(())
    }

    fn instantiate_command(&mut self, cmd: &Sexpr) -> Result<usize> {
        match Self::module_form(cmd).ok_or_else(|| WasmError::malformed("not a module"))? {
            ModuleForm::Text(i) => {
                let list = cmd.list().expect("module");
                let name = match list.get(1).and_then(Sexpr::atom) {
                    Some(a) if a.starts_with('$') => Some(a[1..].to_owned()),
                    _ => None,
                };
                let synthesized = synthesize_module(&list[i..])?;
                let module = self.compile_text(&synthesized)?;
                self.instantiate(module, name)
            }
            ModuleForm::Binary(bytes) => {
                let module = decode_module(&bytes)?;
                validate_module(&module)?;
                self.instantiate(module, module_name(cmd))
            }
            ModuleForm::Quote(text) => {
                let module = self.compile_quoted(&text)?;
                self.instantiate(module, module_name(cmd))
            }
        }
    }

    fn run_action(&mut self, action: &Sexpr) -> Result<Vec<Value>> {
        let list = action.list().ok_or_else(|| WasmError::trap("malformed action"))?;
        let head = list.first().and_then(Sexpr::atom).unwrap_or("");
        let mut i = 1;
        let inst = match list.get(1).and_then(Sexpr::atom) {
            Some(a) if a.starts_with('$') => {
                i = 2;
                self.named.get(&a[1..]).copied().ok_or_else(|| WasmError::trap("unknown module"))?
            }
            _ => self.last.ok_or_else(|| WasmError::trap("no module instantiated"))?,
        };
        let name = string_of(list.get(i))?;
        match head {
            "invoke" => {
                let (addr, _) = self.store.export_func(inst, &name)?;
                let args = list[i + 1..]
                    .iter()
                    .map(|e| const_value(e))
                    .collect::<Result<Vec<_>>>()?;
                let mut interp = Interpreter::new(&mut self.store);
                interp.invoke(inst, addr, args)
            }
            "get" => {
                let g = self.store.export_global(inst, &name)?;
                Ok(vec![self.store.globals[g as usize].value])
            }
            _ => Err(WasmError::trap("malformed action")),
        }
    }

    fn assert_return(&mut self, cmd: &Sexpr) -> Result<()> {
        let list = cmd.list().expect("assert_return");
        let action = list.get(1).ok_or_else(|| WasmError::trap("missing action"))?;
        let actual = self.run_action(action)?;
        let expected: Vec<Expected> = list[2..]
            .iter()
            .map(expected_value)
            .collect::<Result<Vec<_>>>()?;
        if actual.len() != expected.len() {
            return Err(WasmError::trap(format!(
                "result count: expected {}, got {}",
                expected.len(),
                actual.len()
            )));
        }
        for (a, e) in actual.iter().zip(&expected) {
            if !expected_matches(*a, e) {
                return Err(WasmError::trap(format!("expected {e:?}, got {a:?}")));
            }
        }
        Ok(())
    }

    fn assert_error(&mut self, cmd: &Sexpr, kinds: &[ErrorKind]) -> Result<()> {
        let list = cmd.list().expect("assert");
        let action = list.get(1).ok_or_else(|| WasmError::trap("missing action"))?;
        let result = if action.head() == Some("module") {
            self.try_instantiate(action)
        } else {
            self.run_action(action).map(|_| ())
        };
        match result {
            Ok(()) => Err(WasmError::trap("expected a failure, but it succeeded")),
            Err(e) if kinds.contains(&e.kind) => Ok(()),
            Err(e) => Err(WasmError::trap(format!("expected {kinds:?}, got {e}"))),
        }
    }

    fn try_instantiate(&mut self, cmd: &Sexpr) -> Result<()> {
        match Self::module_form(cmd).ok_or_else(|| WasmError::malformed("not a module"))? {
            ModuleForm::Text(i) => {
                let list = cmd.list().expect("module");
                let synthesized = synthesize_module(&list[i..])?;
                let module = self.compile_text(&synthesized)?;
                self.instantiate(module, None).map(|_| ())
            }
            ModuleForm::Binary(bytes) => {
                let module = decode_module(&bytes)?;
                validate_module(&module)?;
                self.instantiate(module, None).map(|_| ())
            }
            ModuleForm::Quote(text) => {
                let module = self.compile_quoted(&text)?;
                self.instantiate(module, None).map(|_| ())
            }
        }
    }

    fn assert_invalid(&mut self, cmd: &Sexpr) -> Result<()> {
        let list = cmd.list().expect("assert_invalid");
        let module = list.get(1).ok_or_else(|| WasmError::trap("missing module"))?;
        let result = self.try_instantiate(module);
        match result {
            Ok(()) => {
                let text = format!("{module:?}");
                let snippet = &text[..text.len().min(500)];
                Err(WasmError::trap(format!("expected invalid, but it validated: {snippet}")))
            }
            Err(e) if matches!(e.kind, ErrorKind::Invalid | ErrorKind::Malformed) => Ok(()),
            Err(e) => Err(WasmError::trap(format!("expected invalid, got {e}"))),
        }
    }

    fn assert_malformed(&mut self, cmd: &Sexpr) -> Result<()> {
        let list = cmd.list().expect("assert_malformed");
        let module = list.get(1).ok_or_else(|| WasmError::trap("missing module"))?;
        match Self::module_form(module).ok_or_else(|| WasmError::trap("not a module"))? {
            ModuleForm::Binary(bytes) => match decode_module(&bytes) {
                Ok(m) => match validate_module(&m) {
                    Ok(()) => Err(WasmError::trap("expected malformed binary")),
                    Err(_) => Ok(()),
                },
                Err(_) => Ok(()),
            },
            ModuleForm::Quote(text) => match self.compile_quoted(&text) {
                Ok(_) => Err(WasmError::trap(format!("expected malformed text: {text}"))),
                Err(_) => Ok(()),
            },
            ModuleForm::Text(_) => match self.try_instantiate(module) {
                Ok(()) => Err(WasmError::trap("expected malformed")),
                Err(_) => Ok(()),
            },
        }
    }

    fn assert_legacy_nan(&mut self, cmd: &Sexpr, canonical: bool) -> Result<()> {
        let list = cmd.list().expect("assert nan");
        let action = list.get(1).ok_or_else(|| WasmError::trap("missing action"))?;
        let actual = self.run_action(action)?;
        match actual.as_slice() {
            [Value::F32(v)] => {
                let bits = v.to_bits();
                if (canonical && is_canonical_f32(bits)) || (!canonical && is_arithmetic_f32(bits)) {
                    Ok(())
                } else {
                    Err(WasmError::trap("wrong NaN"))
                }
            }
            [Value::F64(v)] => {
                let bits = v.to_bits();
                if (canonical && is_canonical_f64(bits)) || (!canonical && is_arithmetic_f64(bits)) {
                    Ok(())
                } else {
                    Err(WasmError::trap("wrong NaN"))
                }
            }
            _ => Err(WasmError::trap("expected a float result")),
        }
    }
}

enum ModuleForm {
    Text(usize),
    Binary(Vec<u8>),
    Quote(String),
}

/// Wrap bare module fields in a `(module ...)` form (for `(assert_invalid
/// (module ...))` the list is already a module, but `ModuleForm::Text` points
/// at the first field).
fn synthesize_module(fields: &[Sexpr]) -> Result<Sexpr> {
    let mut wrapped = vec![Sexpr::Atom("module".to_owned())];
    wrapped.extend_from_slice(fields);
    Ok(Sexpr::List(wrapped))
}

fn module_name(cmd: &Sexpr) -> Option<String> {
    match cmd.list().and_then(|l| l.get(1)).and_then(Sexpr::atom) {
        Some(a) if a.starts_with('$') => Some(a[1..].to_owned()),
        _ => None,
    }
}

fn concat_bytes(items: &[Sexpr]) -> Vec<u8> {
    let mut out = Vec::new();
    for item in items {
        if let Some(b) = item.bytes() {
            out.extend_from_slice(b);
        }
    }
    out
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn concat_text(items: &[Sexpr]) -> String {
    let mut out = String::new();
    for item in items {
        if let Some(b) = item.bytes() {
            out.push_str(&String::from_utf8_lossy(b));
        }
    }
    out
}

fn string_of(item: Option<&Sexpr>) -> Result<String> {
    let b = item
        .and_then(Sexpr::bytes)
        .ok_or_else(|| WasmError::trap("expected a name string"))?;
    String::from_utf8(b.to_vec()).map_err(|_| WasmError::trap("malformed UTF-8 name"))
}

/// Parse a constant argument expression.
fn const_value(expr: &Sexpr) -> Result<Value> {
    let list = expr.list().ok_or_else(|| WasmError::trap("malformed argument"))?;
    let head = list.first().and_then(Sexpr::atom).unwrap_or("");
    let arg = || list.get(1).and_then(Sexpr::atom).ok_or_else(|| WasmError::trap("missing literal"));
    Ok(match head {
        "i32.const" => Value::I32(numlit::parse_i32(arg()?)?),
        "i64.const" => Value::I64(numlit::parse_i64(arg()?)?),
        "f32.const" => Value::F32(numlit::parse_f32(arg()?)?),
        "f64.const" => Value::F64(numlit::parse_f64(arg()?)?),
        "ref.null" => Value::Ref(Ref::Null(ref_type(arg()?)?)),
        "ref.extern" | "ref.host" => Value::Ref(Ref::Extern(numlit::parse_i64(arg()?)? as u64)),
        _ => return Err(WasmError::trap(format!("unsupported argument {head}"))),
    })
}

fn ref_type(a: &str) -> Result<ValType> {
    match a {
        "func" | "funcref" => Ok(ValType::FuncRef),
        "extern" | "externref" => Ok(ValType::ExternRef),
        _ => Err(WasmError::trap("unknown reference type")),
    }
}

#[derive(Clone, Debug)]
enum Expected {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    F32Canonical,
    F32Arithmetic,
    F64Canonical,
    F64Arithmetic,
    Ref(Ref),
    AnyRef,
}

fn expected_value(expr: &Sexpr) -> Result<Expected> {
    let list = expr.list().ok_or_else(|| WasmError::trap("malformed expected value"))?;
    let head = list.first().and_then(Sexpr::atom).unwrap_or("");
    let arg = || list.get(1).and_then(Sexpr::atom);
    Ok(match head {
        "i32.const" => Expected::I32(numlit::parse_i32(arg().ok_or_else(|| WasmError::trap("literal"))?)?),
        "i64.const" => Expected::I64(numlit::parse_i64(arg().ok_or_else(|| WasmError::trap("literal"))?)?),
        "f32.const" => match arg() {
            Some("nan:canonical") => Expected::F32Canonical,
            Some("nan:arithmetic") => Expected::F32Arithmetic,
            Some(a) => Expected::F32(numlit::parse_f32(a)?),
            None => return Err(WasmError::trap("literal")),
        },
        "f64.const" => match arg() {
            Some("nan:canonical") => Expected::F64Canonical,
            Some("nan:arithmetic") => Expected::F64Arithmetic,
            Some(a) => Expected::F64(numlit::parse_f64(a)?),
            None => return Err(WasmError::trap("literal")),
        },
        "ref.null" => Expected::Ref(Ref::Null(ref_type(arg().ok_or_else(|| WasmError::trap("ref type"))?)?)),
        "ref.extern" | "ref.host" => match arg() {
            Some(a) => Expected::Ref(Ref::Extern(numlit::parse_i64(a)? as u64)),
            None => Expected::AnyRef,
        },
        "ref.func" => match arg() {
            Some(_) => Expected::AnyRef,
            None => Expected::AnyRef,
        },
        _ => return Err(WasmError::trap(format!("unsupported expected value {head}"))),
    })
}

fn expected_matches(actual: Value, expected: &Expected) -> bool {
    match (actual, expected) {
        (Value::I32(a), Expected::I32(e)) => a == *e,
        (Value::I64(a), Expected::I64(e)) => a == *e,
        (Value::F32(a), Expected::F32(e)) => a.to_bits() == e.to_bits(),
        (Value::F64(a), Expected::F64(e)) => a.to_bits() == e.to_bits(),
        (Value::F32(a), Expected::F32Canonical) => is_canonical_f32(a.to_bits()),
        (Value::F32(a), Expected::F32Arithmetic) => is_arithmetic_f32(a.to_bits()),
        (Value::F64(a), Expected::F64Canonical) => is_canonical_f64(a.to_bits()),
        (Value::F64(a), Expected::F64Arithmetic) => is_arithmetic_f64(a.to_bits()),
        (Value::Ref(a), Expected::Ref(e)) => a == *e,
        (Value::Ref(a), Expected::AnyRef) => !a.is_null(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(source: &str) {
        let stats = run_wast_source("inline", source);
        assert_eq!(stats.failed(), 0, "failures: {:?}", stats.failures);
        assert!(stats.total > 0);
    }

    #[test]
    fn arithmetic_and_locals() {
        check(
            r#"(module
                 (func (export "add") (param i32 i32) (result i32)
                   local.get 0 local.get 1 i32.add))"#,
        );
        check(
            r#"(module
                 (func (export "add") (param i32 i32) (result i32)
                   local.get 0 local.get 1 i32.add))
               (assert_return (invoke "add" (i32.const 1) (i32.const 2)) (i32.const 3))"#,
        );
    }

    #[test]
    fn folded_and_blocks() {
        check(
            r#"(module
                 (func (export "f") (param i32) (result i32)
                   (if (result i32) (local.get 0)
                     (then (i32.const 10))
                     (else (i32.const 20)))))
               (assert_return (invoke "f" (i32.const 1)) (i32.const 10))
               (assert_return (invoke "f" (i32.const 0)) (i32.const 20))"#,
        );
    }

    #[test]
    fn memory_load_store() {
        check(
            r#"(module
                 (memory 1)
                 (func (export "rw") (result i32)
                   (i32.store (i32.const 0) (i32.const 42))
                   (i32.load (i32.const 0))))
               (assert_return (invoke "rw") (i32.const 42))"#,
        );
    }

    #[test]
    fn traps_and_invalid_and_malformed() {
        check(
            r#"(module (func (export "div") (param i32 i32) (result i32)
                   local.get 0 local.get 1 i32.div_s))
               (assert_trap (invoke "div" (i32.const 1) (i32.const 0)) "integer divide by zero")"#,
        );
        check(r#"(assert_invalid (module (func (result i32) (i64.const 0))) "type mismatch")"#);
        check(r#"(assert_malformed (module binary "\00asm\01\00\00") "unexpected end")"#);
    }

    #[test]
    fn globals_and_multi_value() {
        check(
            r#"(module
                 (global $g (mut i32) (i32.const 7))
                 (func (export "inc") (result i32 i32)
                   (global.set $g (i32.add (global.get $g) (i32.const 1)))
                   (global.get $g)
                   (i32.const 1)))
               (assert_return (invoke "inc") (i32.const 8) (i32.const 1))"#,
        );
    }

    #[test]
    fn bulk_memory_and_tables() {
        check(
            r#"(module
                 (memory 1)
                 (data $d "hello")
                 (func (export "init")
                   (memory.init $d (i32.const 0) (i32.const 0) (i32.const 5)))
                 (func (export "peek") (param i32) (result i32)
                   (i32.load8_u (local.get 0))))
               (invoke "init")
               (assert_return (invoke "peek" (i32.const 0)) (i32.const 104))"#,
        );
    }
}

