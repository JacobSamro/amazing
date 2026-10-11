# JS engine performance baseline (r2-jsperf ticket 1)

Base `bfb2c57ad`. Measurement only: no engine behaviour change. Evidence:
[js-perf-baseline-evidence.json](js-perf-baseline-evidence.json) (raw 71 MB trace stays on the Boat box,
sha256 in the evidence file). Box: Boat 4 vCPU, release profile (thin LTO), Node v24.18.1.

## How to reproduce

- Counting mode: `AMAZING_JS_VM_STATS=<file>` (off by default; one cached `OnceLock` read when off).
  Source `crates/amazing-js/src/vm/stats.rs`. Per process it appends program decisions (`P`), callable
  compile attempts (`F`) and executed JS-function calls by backend (`C`), each with the first fallback
  reason. Summarise with `scripts/js-vm-coverage-summary.py`.
- Benchmarks: `crates/amazing-js/benches/programs/*.js` (12 programs written for this suite, each prints a
  checksum), host `crates/amazing-js/examples/js_bench_host.rs`, runner `scripts/js-bench.py`
  (`--runs N`, median in-process time, peak RSS via wait4, checksum agreement across modes and Node).
- Profile: `perf record -g` on a build with `--config profile.profiling.*` (inherits release, strip off,
  debug=1, frame pointers); the box needs `sudo perf` (perf_event_paranoid). Release profile unchanged.

## Finding 0: which paths the VM actually runs

- Callables (functions, arrows, methods, class constructors) are compiled to bytecode by default.
- Top-level scripts run on the **tree walker** unless `AMAZING_JS_VM=1` is set (opt-in, plus a 10M
  instruction fuel limit, `DEFAULT_INSTRUCTION_LIMIT`). Embedders that call `run_program` get the tree
  walker at top level. With the opt-in on, long top-level loops die with
  `RangeError: bytecode instruction limit exceeded` (seen on the first, larger array_loop and typed_array).

## Test262 VM coverage (full suite, `AMAZING_JS_VM=1`, 94,258 host processes)

Files 53,260 pass / 318 fail (variants 102,295 / 623) with the VM program path on.

| Level | VM | Fallback / tree |
|---|---|---|
| Test body (last program per process) | 88,103 (93.47%) | 6,155 (6.53%) |
| All program executions (harness + eval + test) | 411,642 (27.47%) | 1,087,152 (72.53%) |
| Callable compile attempts | 1,117,134 (100%) | 0 |
| JS function calls executed | 119,116,299 (100%) | 0 |

All-programs fallback is dominated by direct-eval code (1,081,327 of 1,087,152), which deliberately
bypasses the VM. Top fallback reasons (no more than 8 distinct reasons exist; there are zero callable ones):

| # | Reason | All programs | Test bodies |
|---|---|---|---|
| 1 | direct-eval-context (code run by direct eval) | 1,081,327 | 2,049 |
| 2 | ineligible: class field | 3,467 | 3,348 |
| 3 | ineligible: direct eval call in the script | 1,855 | 256 |
| 4 | compile error: statement kind | 222 | 222 |
| 5 | ineligible: catch binding pattern | 197 | 196 |
| 6 | ineligible: class static block | 76 | 76 |
| 7 | compile error: compiler resource limit | 6 | 6 |
| 8 | compile error: register limit | 2 | 2 |

Caveat: callable compile attempts are counted at instantiation, executed calls at call time;
generator and async-generator body compiles are not logged. Calls to native functions are not counted.

## Benchmarks (median of 5, in-process ms; RSS is peak of the process)

Modes: default = shipped behaviour (top-level tree walker, callables on VM); wrapped = default, but the
program body sits inside an IIFE so its loops run as bytecode; vm = `AMAZING_JS_VM=1` (top-level on VM);
ast = `AMAZING_JS_AST_FUNCTIONS=1` (function declarations/expressions/arrows stay on the tree walker;
methods and class code still compile). All checksums agree across modes and with Node.

| Program | default | wrapped | vm | ast | node --jitless | node | RSS MB (ours) | RSS MB (node) |
|---|---|---|---|---|---|---|---|---|
| array_loop | 235.7 | 111.6 | 232.5 | 241.1 | 5.7 | 3.4 | 17.8 | 50.7 |
| class_inherit | 228.8 | 167.4 | 220.5 | 229.2 | 6.8 | 3.4 | 15.1 | 52.3 |
| closures | 264.5 | 165.5 | 257.0 | 269.5 | 6.3 | 3.9 | 102.4 | 51.8 |
| fib | 329.3 | 268.0 | 329.1 | 389.6 | 7.2 | 3.0 | 15.4 | 50.1 |
| json_roundtrip | 110.4 | 107.7 | 109.9 | 108.9 | 10.0 | 7.3 | 15.1 | 46.7 |
| map_set | 105.2 | 64.7 | 103.2 | 106.0 | 5.9 | 3.1 | 15.1 | 45.7 |
| method_calls | 413.0 | 263.6 | 396.2 | 420.6 | 6.6 | 2.8 | 15.1 | 50.6 |
| prop_access | 272.5 | 181.9 | 310.2 | 273.2 | 5.5 | 2.8 | 15.1 | 50.6 |
| regexp | 209.2 | 164.1 | 208.5 | 213.2 | 9.6 | 4.5 | 15.4 | 52.8 |
| scheduler | 281.9 | 279.1 | 277.9 | 319.7 | 7.0 | 3.4 | 15.4 | 51.7 |
| string_ops | 220.8 | 88.7 | 210.4 | 220.8 | 6.4 | 3.8 | 15.4 | 53.3 |
| typed_array | 451.0 | 146.1 | 447.4 | 454.3 | 7.0 | 3.7 | 15.4 | 52.6 |

Node times are in-process (exit hook minus preload start), so they exclude process startup (~40 ms).
We are 11x (json_roundtrip) to 63x (method_calls) slower than `node --jitless` on the default path
(fib 46x, prop_access 50x, typed_array 65x, array_loop 41x). Findings:

- Bytecode inside a function is 1.0x to 3.1x faster than the same code at top level (typed_array 3.1x,
  array_loop 2.1x, string_ops 2.5x, map_set 1.6x), but still 20x to 40x slower than `node --jitless`.
- Moving the top level onto the VM (`vm` column) gains nothing: global variable access (the global object's
  property table plus the script scope chain) dominates, and it is identical on both backends.
- json_roundtrip and regexp barely change when wrapped: they spend their time in native builtins,
  which pay the same hashing and key-conversion costs.
- `ast` is within noise of default except fib (+18%) and scheduler (+13%) where calls dominate.
- `closures` holds 102 MB: closure environments form `Rc` cycles that are never reclaimed.

## Profile (default mode, flat, top 15 of the two slowest: typed_array 464 ms, method_calls 411 ms)

Symbols are shortened; full list in the evidence file. Sampling at 2 kHz (about 900 samples each).

| typed_array | % | method_calls | % |
|---|---|---|---|
| SipHasher13::write | 7.37 | hash_one<&PropertyKey> (SipHash) | 6.52 |
| Vec<u16>::from_iter(encode_utf16) | 6.38 | SipHasher13::write | 5.72 |
| Vec<u16>::extend_desugared(encode_utf16) | 5.31 | Vec<u16>::from_iter(encode_utf16) | 5.35 |
| Interpreter::scoped_reference | 4.55 | malloc | 4.63 |
| malloc | 4.33 | Vec<u16>::extend_desugared(encode_utf16) | 3.35 |
| hashbrown find (PropertyKey, BindingCell) | 4.24 | vm resume_frame_inner closure | 3.08 |
| hash_one<&PropertyKey> | 3.81 | hashbrown find (PropertyKey, BindingCell) | 3.08 |
| interp::binary_state | 3.58 | free | 2.61 |
| hash_one<u32> | 2.92 | libc 0x1a1544 | 2.14 |
| libc 0x1a1544 | 2.92 | Interpreter::scoped_reference | 2.13 |
| Interpreter::eval_expr | 2.61 | String::from_iter(decode_utf16) | 2.13 |
| Interpreter::identifier_value | 2.28 | RawVec::finish_grow | 2.02 |
| RawVec::finish_grow | 2.04 | libc 0x1a14d3 | 1.78 |
| realloc | 1.95 | realloc | 1.66 |
| libc 0x1a155b | 1.95 | Interpreter::bind_params | 1.66 |

Roughly 12-14% of samples are SipHash (std `RandomState`) and 20-23% are string-to-`Vec<u16>` conversion
plus the malloc/realloc/free it causes. Interpretation work (eval_expr, VM dispatch) is under 10%.

## Ranked changes expected to pay off most

1. **Allocation-free property and identifier keys.** `PropertyKey::from(&str)` runs `encode_utf16` into a
   fresh `Rc<[u16]>` on every named access, and `String` conversions run on hot paths (`from_utf16_lossy`).
   Evidence: 20-23% of samples in both profiles (encode_utf16 collect/extend, malloc, realloc, free).
   Intern AST names and engine literals once (thread-local interner or cached `JsString` in AST/bytecode
   constants).
2. **Replace SipHash with a cheap deterministic hasher** on property tables (`prop_index`), scope maps
   (`HashMap<String, BindingCell>`), and integer keys. Evidence: SipHasher13::write plus hash_one is 14.1%
   (typed_array) and 12.2% (method_calls) of samples. Iteration order is already unspecified with
   `RandomState`, so this is behaviour-neutral. Small, whole-engine, low risk.
3. **Slot-resolved scopes.** Identifier reads go `scoped_reference` then hashbrown `find` on a
   string-keyed scope chain. Evidence: scoped_reference + find + identifier_value is 11% (typed_array)
   and 5% (method_calls) before their hashing. Top level on the VM equals the tree walker (see `vm` column)
   and wrapping the same code in a function is up to 3.1x faster, so global/scope lookup is the cost to remove.
4. **Call path cost.** fib is 2.5 us per call (138k calls in 347 ms); `call_value_with_new_target_inner`
   clones the `JsFunction`, builds a `CallFrame`, `bind_params`, and re-checks async/derived state per
   call. Evidence: fib/method_calls/scheduler are the call-bound programs; bind_params, call_admitted and
   call_outcome_once appear in method_calls' profile.
5. **Make VM top-level the default, with fuel scaled to the program.** 93.5% of Test262 test bodies compile
   to bytecode and callables are 100% covered, but the 10M instruction fuel turns long loops into
   RangeErrors, so it cannot be enabled today. Gains only follow items 1-3 (global access). Memory: closure cycles
   (`closures` 102 MB vs 15 MB) need a cycle collector or weak environments.

Remaining class-field and static-block scripts (3,424 test bodies) are the only sizeable program-level
fallback worth removing for coverage; it does not affect speed.
