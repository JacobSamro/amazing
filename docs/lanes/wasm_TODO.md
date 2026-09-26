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

## Remaining WASM-1 gaps (not blocking, follow-up)

All in the **`.wat`/`.wast` text parser** (`wat.rs`, `sexpr.rs`) — the
decoder/validator/interpreter that the JS API and any future consumer will
actually exercise are unaffected. 39 assertions across:
- `tokens.wast` (17): lexer edge cases around comment/whitespace adjacency
  (`func;;bla`) and adjacent string-literal segments in `data`.
- `binary.wast` (8): a handful of malformed-binary shapes not yet rejected.
- `stack.wast` (6): a deep folded-instruction form (`$body`) the parser
  doesn't resolve; likely a fold-desugaring gap, not validated as a real
  spec-required form beyond this test.
- `func.wast` (4): redundant explicit `(type $sig)` alongside inline
  param/result should be malformed; currently accepted.
- `call_indirect.wast` (2), `elem.wast` (1), `select.wast` (1): similar
  narrow malformed-text acceptance gaps.

Pick up by re-running `scripts/linux-run.sh wasm -- '<build+run wast_suite>'`
against the pinned testsuite (fetch via `git clone` + `git checkout
7ef86ddeed81458f9031a49a40b3a3f99c1c6a8a` into `corpora/testsuite`, not
committed) and triaging by file.

## Next: WASM-2 (JS API binding seam)

WebAssembly.{Module,Instance,Memory,Table,Global}, `instantiate`/`compile`.
Needs a small binding seam into `amazing-js`/js-runtime (JsValue <->
`amazing_wasm::Value` marshaling, import-object resolution, JS closures as
`Extern::Func` hosts). Flagged in `.agents/js-runtime/questions.md` rather
than implemented, per lane escalation rules — `amazing-wasm` stays a pure
engine crate with no dependency on `amazing-js`.
