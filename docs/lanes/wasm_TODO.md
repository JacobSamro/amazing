# wasm lane TODO / decision map

## WASM-1 (done, commit follows in log.md)

Std-only WebAssembly core engine in `crates/amazing-wasm`: binary decoder,
encoder, validator (full type-checking algorithm incl. polymorphic/unreachable
stack), interpreter (MVP + sign-extension, non-trapping float-to-int,
multi-value, bulk memory, reference types), and a `.wast` spec-testsuite
runner. Testsuite pinned at `WebAssembly/testsuite@7ef86ddeed81458f9031a49a40b3a3f99c1c6a8a`
(2023-03-08, pre-SIMD-at-root; SIMD and `skip-stack-guard-page.wast` are out of
scope for WASM-1 and skipped by name).

Linux gate (Boat sandbox): 27858/27897 testsuite assertions (99.86%), up from
27178/27897 at the start of this session, across 89 non-skipped `.wast` files.
Full workspace `cargo nextest run --workspace --locked --no-fail-fast`: 2813
passed, 0 failed, 10 skipped. `cargo test --workspace --locked --doc`: all
green (amazing-js/amazing-html/amazing-wasm/amazing-webrtc-crypto doctests
included).

Bugs found and fixed this session (all in `crates/amazing-wasm/src`):
- `value.rs` `quiet_f32`/`quiet_f64`: matched on the exponent bits alone, so
  it turned legitimate infinities into NaN (any `is_infinite` value has the
  same all-1s exponent as NaN). Fixed to gate on `a.is_nan()`. This was the
  single largest cluster (~600 float-arithmetic assertions).
- `validate.rs` `pop_ctrl`: skipped the operand-stack-height check entirely
  when the current control frame was already unreachable, so code that
  pushed extra unconsumed values after `unreachable`/`br`/`return` wrongly
  validated (19 `unreached-invalid.wast` cases).
- `validate.rs` `Instr::BrTable`: required every target label's result types
  to equal the default label's types by list equality. Per spec only the
  *arity* is a static requirement; per-target type compatibility is checked
  against the (possibly polymorphic) operand stack. Added `check_vals`, a
  non-mutating peek so checking one target doesn't leave residue that
  corrupts the check for the next target when two labels share an arity but
  differ in concrete type under `unreachable` ("meet-bottom" semantics).
- `interp.rs` `TableCopy`/`MemoryCopy`: cloned the source table/memory into a
  scratch buffer but then called `copy_within` on the *destination*'s own
  slice with the source's indices — correct by accident when src and dst are
  the same table/memory (self overlap), silently wrong data across two
  different tables. Now writes from the cloned source buffer directly (18
  `table_copy.wast` failures; `memory_copy.wast` happened to already pass
  because the pinned corpus only exercises single-memory copies).
- `interp.rs`/`store.rs` `push_call`: pushed the new call frame with the
  *caller's* module instance rather than the callee's own defining instance,
  so an imported function's `call_indirect`/table/memory/global accesses
  resolved against the calling module's index space instead of its own.
  Added `FuncInstance::Wasm::inst` (the home instance, computed once at
  `Store::instantiate` time) and made `push_call` use it. Fixed all 18
  `linking.wast` cross-module table/function-index failures.
- `store.rs` `check_table_import`/`check_memory_import`: matched the
  import's declared minimum against the exporter's *original* declared type
  instead of its *live* current size, so a memory/table grown after export
  (and re-imported by a third module) was wrongly rejected as unlinkable (4
  `imports.wast` failures).
- `numlit.rs`: decimal/hex float literals that round to infinity from a
  finite source (`1e39`, `0x1p128`, ...) were silently accepted as `inf`
  instead of rejected as malformed per the text-format grammar; `.0`/`.0e0`
  (missing the mandatory leading digit before `.`) were also accepted. Fixed
  both (20 `const.wast` failures).

## WASM-1b (done, commit follows in log.md)

Fixed the 39 remaining WASM-1 testsuite gaps, all in the WAT text parser
(`wat.rs`) and its s-expression lexer (`sexpr.rs`), plus one decoder
strictness fix (`decode.rs`). None touch the binary decoder's happy path,
validator, or interpreter.

Linux gate (Boat sandbox): **27898/27898 testsuite assertions (100%)**, up
from 27858/27897, across the same 89 non-skipped `.wast` files. Full
workspace `cargo nextest run --workspace --locked --no-fail-fast`: 2882
passed, 0 failed, 10 skipped. `cargo test --workspace --locked --doc`: all
green.

Fixes (all in `crates/amazing-wasm/src`):
- `sexpr.rs`: a string literal glued directly onto a preceding word/string
  with no separating white space or comment (`data"a"`, `"a""b"`) is
  malformed even though the `"` makes the boundary unambiguous — added
  `prev_word_or_str_end` tracking to reject it. A lone `;` (not `;;` or
  `(;`) is now an explicit malformed error instead of falling through to an
  empty atom (`tokens.wast`, 17 assertions).
- `decode.rs` `decode_reserved_memory`: `memory.size`/`memory.grow`'s
  reserved byte must be exactly one raw byte; was reading a full LEB128
  varint, which happened to accept multi-byte encodings of zero
  (`binary.wast`, part of 8 assertions).
- `wat.rs` `parse_func` / `call_indirect` immediates: an explicit
  `(type $sig)` clause after inline `(param)`/`(result)` fields is
  malformed per the text-format typeuse grammar (type must come first);
  added the ordering check (`func.wast` 4, `call_indirect.wast` 2).
- `wat.rs` `parse_func`: fixed local-index numbering for functions whose
  params come entirely from a `(type $sig)` reference with no literal
  `(param)` fields — locals now start numbering after the referenced type's
  param count. Caught a self-introduced regression (double-counting when
  literal `(param)` fields are *also* present, since those are numbered
  independently by the existing field walk) before it was committed, via
  the very next corpus rerun (13308/27898 -> traced to one line -> back to
  27897/27898).
- `wat.rs` `parse_elem`: any list head other than `offset` (explicit) or
  `item` (explicit passive marker) counts as an implicit offset expression,
  matching the spec's active/passive/declarative disambiguation rule
  instead of special-casing `i32.const`/`global.get` heads (`elem.wast`, 1).
- `wat.rs` select: `consume_immediates` and `build()` now consume and
  concatenate *all* repeated `(result t)` clauses instead of just the
  first, for chained forms like `select (result i32) (result)`
  (`select.wast`, 1).
- `wat.rs` `parse_plain`: consume an optional repeated `$label` atom after
  `else`/`end`, matching the currently open block's label
  (`stack.wast`, 6 — the "deep folded-instruction form" noted below turned
  out to be this).

Diagnosis note: mapping a wast-runner failure index back to source text by
hand-rolling a comment-stripping line counter is unreliable (`;;` inside a
string literal, multi-line block comments) — use the crate's own
`amazing_wasm::sexpr::parse` from a throwaway example instead for ground
truth, then delete the example before committing.

## WASM-2a (done, commit follows in log.md): embedder API

Added `crates/amazing-wasm/src/embed.rs`: the seam js-runtime calls to build
the WebAssembly JS API, with **no dependency on `amazing-js`** in either
direction — every type here is store-address/handle-based, never a
`JsValue`. Full API surface (also re-exported from crate root):

- **Compile:** `embed::compile(bytes: &[u8]) -> Result<CompiledModule>` —
  decode + validate. `CompiledModule::imports()` / `::exports()` return
  `&[ImportType]` / `&[ExportType]`, each an `(module, name)` (imports only)
  plus an `ExternKind` — `Func(FuncType)`, `Table(TableType)`,
  `Memory(MemoryType)`, or `Global(GlobalType)` — i.e. exactly the shape of
  `WebAssembly.Module.imports()`/`.exports()`.
- **Instantiate:** `Engine::instantiate(&mut self, &CompiledModule, &mut dyn
  ImportResolver) -> Result<InstanceHandle>`. `ImportResolver::resolve(&mut
  self, module: &str, name: &str, expected: &ExternKind) -> Result<ExternValue>`
  is the trait the embedder implements over its own import-object lookup;
  it returns a handle (see below), not a raw value, so js-runtime never
  touches a `Store` address directly. `instantiate` runs the module's
  `start` afterward, exactly like `wast.rs`'s `Runner::instantiate`. A
  resolver `Err` (use `WasmError::unlinkable` for "no such import") is
  propagated as-is; a type-incompatible resolved value is caught by the
  same linking checks `Store::instantiate` already runs and comes back as
  `ErrorKind::Unlinkable`.
- **Handles:** `FuncHandle`/`TableHandle`/`MemoryHandle`/`GlobalHandle`
  (opaque, `Copy`) stand in for `WebAssembly.{Function,Table,Memory,Global}`.
  `Engine::create_func`/`create_table`/`create_memory`/`create_global` mint
  fresh ones — `create_func` takes a `store::HostFn`
  (`Rc<dyn Fn(&[Value]) -> Result<Vec<Value>>>`), i.e. the JS-closure-as-host-
  function case. `Engine::get_export` reads a handle off an
  `InstanceHandle` by export name. `Engine::call_export`/`call_func` invoke;
  `table_grow`/`memory_grow` are `.grow()`; `memory_bytes`/`memory_bytes_mut`
  back a `WebAssembly.Memory`'s `buffer`; `global_get`/`global_set` are the
  `.value` getter/setter (setter rejects immutable or type-mismatched).
- **Error classification:** `ErrorKind::js_error_class(self) -> JsErrorClass`
  (`Compile`/`Link`/`Runtime`) — `Malformed`/`Invalid`/`Unsupported` ->
  `Compile` (`WebAssembly.CompileError`), `Unlinkable` -> `Link`
  (`LinkError`), `Trap`/`Exhaustion` -> `Runtime` (`RuntimeError`). One
  lookup instead of js-runtime re-deriving the mapping.

4 new unit tests in `embed.rs` build a module by hand (import + call +
export + memory + global), round-trip it through `encode_module`/`compile`,
instantiate through a resolver backed by a `create_func` host closure, call
the export, grow the memory, and read/write the global — plus the
unlinkable-import and error-class-mapping paths. Full workspace gate: 2886
nextest passed/0 failed/10 skipped (up from 2882: the 4 new tests), doctests
all green, corpus unaffected at 27898/27898.

## Next: WASM-2b (JS-side binding, owned by js-runtime)

`WebAssembly.{Module,Instance,Memory,Table,Global}` JS constructors/
prototypes and `WebAssembly.instantiate`/`.compile` built on the `embed`
seam above: `JsValue <-> amazing_wasm::Value` marshaling, building an
`ImportResolver` over a JS import object, and surfacing `ErrorKind::
js_error_class` as the matching JS error constructor. Not amazing-wasm's
work — see the updated request in the main checkout's
`.agents/js-runtime/questions.md`.
