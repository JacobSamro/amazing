# JS language conformance decision map

Frontier: **JS-27 — reference creation, assignment ordering and strict PutValue** (cluster analysis; shared execution ownership being coordinated).
JS-25 parser API is ready; Runtime35 owns its native eval consumer. JS-26 is complete.
JS-12 bounded regex folding is complete; its Linux gates and evidence appear below.

Ownership: js-lang owns `lexer.rs`, `parser.rs`, `ast.rs`, `tokens.rs` and
`amazing-regex` historically; RegExp moved to js-date/JSD-2 on 2026-09-26. `ast.rs` is the front-end-owned contract consumed by js-runtime;
AST change requests come back to this lane. Execution, builtins, values,
environments, VM and GC belong to js-runtime. JS-3 finishes its explicitly
granted runner and host work before the next language ticket.

**Measurement correction:** every JS-1/JS-2 Test262 count below is a historical
process-exit measurement under a weakened harness, withdrawn as conformance
evidence. JS-3 replaces that ruler with real upstream helpers, strict variants,
phase/type matching and explicit unsupported capabilities. Its results appear
at the end of this decision map.

## JS-1: Full-tree Test262 baseline

Type: Research. Engine changes: none.

### Evidence contract

These are full-tree discovery results from the existing, unmodified `scripts/test262.py`, not a complete ECMA-262 conformance score. Its exclusions, simplified assertions, omitted includes, and missing strict variants remain in effect. A pass means the process exited successfully under this harness.

Test262 revision: `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`. Both runs select every `test/**/*.js` with no `--only` or `--limit`. The shared checkout at `C:\Users\server\src\test262` was sparse (641 files); a separate complete pinned checkout was fetched into the lane scratch directory. Full discovery is **53872 files**.

Baseline source: `c50ee3f71608fd5adbea922a02e04f4c1e8c1b19`; all 100 Rust/manifest/harness source files matched the local tree before compilation. The lane subsequently rebased to `7dcd51cf02d2e1941aa09cd789f42feb1c084ed8` for integrator-provided crate skeletons; both measurements retain the original binary.

- Binary SHA-256: `d8f9ba9a4a3245a55590d9fd2c7a35657c1f3ff64070505c85fc92060b530399`.
- Harness SHA-256: `26d666f7529a1788fbbfe7c119f7f6f149cf32c0e0914a604017592cda8ab79c`.
- Observer SHA-256: `9ea34640b35c7af7988bf9fb748c31cbb64862a12899cf95fff5c7f40cb971f1`.
- Selected normalized-source manifest SHA-256: `9c8b925634f19587ffe94a4c07748e730c936cbeb33c4cb422860df704f5e595`.
- Run 1 outcome/cluster SHA-256: `5ea2668006e278f17409342831d560dfd20bf0acf2cb74e2cb78c9205e6c6f80`.
- Run 2 outcome/cluster SHA-256: `5ea2668006e278f17409342831d560dfd20bf0acf2cb74e2cb78c9205e6c6f80`.
- Identical per-file status and cluster: **True**; status changes: **0**.
- Raw path/status/error-reason hashes identical: **False**.

The raw error-reason hashes differ only because crash diagnostics embed Windows thread IDs; the stable per-file outcome/cluster hashes are identical.

### Measured baseline

Timeouts are separated from ordinary failures below; the original harness includes timeouts in its failed total. Skipped means unsupported or excluded by harness policy, not demonstrated engine failure. Crashes, if present, remain counted in failed and are broken out in the cluster table.

| Selection | Discovered | Passed | Failed | Timeout | Skipped |
|---|---:|---:|---:|---:|---:|
| Whole tree, run 1 | 53872 | 5438 | 23165 | 7 | 25262 |
| Whole tree, run 2 | 53872 | 5438 | 23165 | 7 | 25262 |
| annexB | 1086 | 252 | 762 | 0 | 72 |
| built-ins | 23815 | 2067 | 15682 | 6 | 6060 |
| harness | 116 | 13 | 65 | 0 | 38 |
| intl402 | 3357 | 77 | 2918 | 0 | 362 |
| language | 24007 | 2869 | 2947 | 0 | 18191 |
| staging | 1491 | 160 | 791 | 1 | 539 |

This is a before/repeat comparison, not an engine improvement: no engine implementation changed in JS-1.

### Per-directory results

Directory rows below are at depth two (top-level-only directories remain above). Complete counts for every ancestor directory, including skip reasons per directory, are retained in each `analysis.json`.

| Directory | Discovered | Passed | Failed | Timeout | Skipped |
|---|---:|---:|---:|---:|---:|
| annexB/built-ins | 241 | 24 | 169 | 0 | 48 |
| annexB/language | 845 | 228 | 593 | 0 | 24 |
| built-ins/AbstractModuleSource | 8 | 0 | 0 | 0 | 8 |
| built-ins/AggregateError | 25 | 0 | 17 | 0 | 8 |
| built-ins/Array | 3082 | 554 | 2219 | 6 | 303 |
| built-ins/ArrayBuffer | 221 | 13 | 159 | 0 | 49 |
| built-ins/ArrayIteratorPrototype | 27 | 0 | 1 | 0 | 26 |
| built-ins/AsyncDisposableStack | 104 | 1 | 55 | 0 | 48 |
| built-ins/AsyncFromSyncIteratorPrototype | 38 | 0 | 1 | 0 | 37 |
| built-ins/AsyncFunction | 18 | 1 | 15 | 0 | 2 |
| built-ins/AsyncGeneratorFunction | 23 | 0 | 15 | 0 | 8 |
| built-ins/AsyncGeneratorPrototype | 48 | 0 | 12 | 0 | 36 |
| built-ins/AsyncIteratorPrototype | 13 | 0 | 0 | 0 | 13 |
| built-ins/Atomics | 389 | 0 | 0 | 0 | 389 |
| built-ins/BigInt | 77 | 0 | 0 | 0 | 77 |
| built-ins/Boolean | 51 | 24 | 26 | 0 | 1 |
| built-ins/DataView | 561 | 0 | 418 | 0 | 143 |
| built-ins/Date | 594 | 18 | 512 | 0 | 64 |
| built-ins/DisposableStack | 93 | 1 | 67 | 0 | 25 |
| built-ins/Error | 93 | 3 | 70 | 0 | 20 |
| built-ins/FinalizationRegistry | 47 | 0 | 30 | 0 | 17 |
| built-ins/Function | 509 | 64 | 397 | 0 | 48 |
| built-ins/GeneratorFunction | 23 | 0 | 22 | 0 | 1 |
| built-ins/GeneratorPrototype | 61 | 3 | 54 | 0 | 4 |
| built-ins/Infinity | 6 | 2 | 4 | 0 | 0 |
| built-ins/Iterator | 654 | 47 | 367 | 0 | 240 |
| built-ins/JSON | 165 | 45 | 77 | 0 | 43 |
| built-ins/Map | 204 | 42 | 101 | 0 | 61 |
| built-ins/MapIteratorPrototype | 11 | 0 | 2 | 0 | 9 |
| built-ins/Math | 327 | 52 | 272 | 0 | 3 |
| built-ins/NaN | 6 | 2 | 4 | 0 | 0 |
| built-ins/NativeErrors | 94 | 8 | 79 | 0 | 7 |
| built-ins/Number | 340 | 60 | 268 | 0 | 12 |
| built-ins/Object | 3411 | 403 | 2818 | 0 | 190 |
| built-ins/Promise | 732 | 39 | 225 | 0 | 468 |
| built-ins/Proxy | 311 | 0 | 0 | 0 | 311 |
| built-ins/Reflect | 153 | 25 | 92 | 0 | 36 |
| built-ins/RegExp | 1879 | 91 | 1347 | 0 | 441 |
| built-ins/RegExpStringIteratorPrototype | 17 | 0 | 0 | 0 | 17 |
| built-ins/Set | 383 | 130 | 178 | 0 | 75 |
| built-ins/SetIteratorPrototype | 11 | 0 | 2 | 0 | 9 |
| built-ins/ShadowRealm | 67 | 0 | 46 | 0 | 21 |
| built-ins/SharedArrayBuffer | 104 | 0 | 0 | 0 | 104 |
| built-ins/String | 1223 | 238 | 824 | 0 | 161 |
| built-ins/StringIteratorPrototype | 7 | 0 | 0 | 0 | 7 |
| built-ins/SuppressedError | 22 | 0 | 17 | 0 | 5 |
| built-ins/Symbol | 98 | 0 | 0 | 0 | 98 |
| built-ins/Temporal | 4603 | 95 | 3098 | 0 | 1410 |
| built-ins/ThrowTypeError | 14 | 0 | 14 | 0 | 0 |
| built-ins/TypedArray | 1446 | 5 | 922 | 0 | 519 |
| built-ins/TypedArrayConstructors | 738 | 9 | 494 | 0 | 235 |
| built-ins/Uint8Array | 70 | 9 | 60 | 0 | 1 |
| built-ins/WeakMap | 141 | 0 | 0 | 0 | 141 |
| built-ins/WeakRef | 29 | 0 | 20 | 0 | 9 |
| built-ins/WeakSet | 85 | 0 | 0 | 0 | 85 |
| built-ins/decodeURI | 55 | 0 | 55 | 0 | 0 |
| built-ins/decodeURIComponent | 56 | 0 | 56 | 0 | 0 |
| built-ins/encodeURI | 31 | 0 | 31 | 0 | 0 |
| built-ins/encodeURIComponent | 31 | 0 | 31 | 0 | 0 |
| built-ins/eval | 10 | 1 | 8 | 0 | 1 |
| built-ins/global | 29 | 19 | 10 | 0 | 0 |
| built-ins/isFinite | 15 | 3 | 5 | 0 | 7 |
| built-ins/isNaN | 15 | 3 | 5 | 0 | 7 |
| built-ins/parseFloat | 54 | 28 | 26 | 0 | 0 |
| built-ins/parseInt | 55 | 25 | 30 | 0 | 0 |
| built-ins/undefined | 8 | 4 | 4 | 0 | 0 |
| intl402/Array | 2 | 0 | 2 | 0 | 0 |
| intl402/BigInt | 11 | 0 | 0 | 0 | 11 |
| intl402/Collator | 65 | 1 | 59 | 0 | 5 |
| intl402/Date | 12 | 0 | 12 | 0 | 0 |
| intl402/DateTimeFormat | 244 | 7 | 203 | 0 | 34 |
| intl402/DisplayNames | 57 | 14 | 32 | 0 | 11 |
| intl402/DurationFormat | 110 | 0 | 101 | 0 | 9 |
| intl402/FallbackSymbol | 2 | 0 | 2 | 0 | 0 |
| intl402/Intl | 66 | 1 | 57 | 0 | 8 |
| intl402/ListFormat | 81 | 0 | 44 | 0 | 37 |
| intl402/Locale | 168 | 2 | 136 | 0 | 30 |
| intl402/Number | 7 | 1 | 5 | 0 | 1 |
| intl402/NumberFormat | 249 | 5 | 214 | 0 | 30 |
| intl402/PluralRules | 53 | 1 | 46 | 0 | 6 |
| intl402/RelativeTimeFormat | 80 | 0 | 65 | 0 | 15 |
| intl402/Segmenter | 79 | 3 | 56 | 0 | 20 |
| intl402/String | 19 | 4 | 12 | 0 | 3 |
| intl402/Temporal | 2029 | 38 | 1849 | 0 | 142 |
| intl402/TypedArray | 1 | 0 | 1 | 0 | 0 |
| language/arguments-object | 263 | 18 | 140 | 0 | 105 |
| language/asi | 102 | 66 | 1 | 0 | 35 |
| language/block-scope | 145 | 40 | 3 | 0 | 102 |
| language/comments | 52 | 13 | 8 | 0 | 31 |
| language/computed-property-names | 48 | 13 | 20 | 0 | 15 |
| language/destructuring | 19 | 0 | 0 | 0 | 19 |
| language/directive-prologue | 62 | 26 | 25 | 0 | 11 |
| language/eval-code | 347 | 65 | 227 | 0 | 55 |
| language/export | 3 | 0 | 0 | 0 | 3 |
| language/expressions | 11164 | 1560 | 1170 | 0 | 8434 |
| language/function-code | 217 | 159 | 50 | 0 | 8 |
| language/future-reserved-words | 55 | 22 | 6 | 0 | 27 |
| language/global-code | 42 | 7 | 19 | 0 | 16 |
| language/identifier-resolution | 14 | 5 | 6 | 0 | 3 |
| language/identifiers | 268 | 26 | 96 | 0 | 146 |
| language/import | 191 | 9 | 14 | 0 | 168 |
| language/keywords | 25 | 0 | 0 | 0 | 25 |
| language/line-terminators | 41 | 17 | 0 | 0 | 24 |
| language/literals | 534 | 136 | 59 | 0 | 339 |
| language/module-code | 755 | 9 | 21 | 0 | 725 |
| language/punctuators | 11 | 0 | 1 | 0 | 10 |
| language/reserved-words | 27 | 1 | 9 | 0 | 17 |
| language/rest-parameters | 11 | 2 | 8 | 0 | 1 |
| language/source-text | 1 | 0 | 1 | 0 | 0 |
| language/statementList | 80 | 8 | 0 | 0 | 72 |
| language/statements | 9350 | 564 | 1007 | 0 | 7779 |
| language/types | 113 | 67 | 31 | 0 | 15 |
| language/white-space | 67 | 36 | 25 | 0 | 6 |
| staging/Temporal | 2 | 0 | 2 | 0 | 0 |
| staging/Uint8Array | 1 | 0 | 1 | 0 | 0 |
| staging/built-ins | 8 | 0 | 5 | 0 | 3 |
| staging/decorators | 3 | 0 | 0 | 0 | 3 |
| staging/explicit-resource-management | 53 | 1 | 4 | 0 | 48 |
| staging/set-methods | 3 | 0 | 2 | 0 | 1 |
| staging/sm | 1409 | 159 | 773 | 1 | 476 |
| staging/source-phase-imports | 4 | 0 | 0 | 0 | 4 |
| staging/top-level-await | 4 | 0 | 1 | 0 | 3 |

### Why files were skipped

Reasons are mutually exclusive, using the harness’s first matching exclusion. Path exclusions run before metadata checks.

| First exclusion | Count |
|---|---:|
| path exclusion: private/accessor/dstr/destructuring | 11785 |
| deferred runtime feature | 6181 |
| negative test | 3369 |
| module/async/raw flag | 2839 |
| accessor semantics | 434 |
| module syntax | 354 |
| module/async metadata | 223 |
| tail-call harness hook | 35 |
| private syntax | 33 |
| async harness hook | 9 |

### Failure clusters

These are disjoint first-observed failure classifications. Missing include helpers are attributed only when the reported unresolved identifier is declared in a helper explicitly named by that test’s metadata. Broad TypeError/assertion buckets require further triage and are not claimed to have one root cause.

| Observed cluster | Count |
|---|---:|
| runtime: TypeError (requires triage) | 5825 |
| harness: missing declared include helper | 4256 |
| parser: other syntax rejection | 3887 |
| runtime: missing builtin Temporal | 2889 |
| assertion/other runtime failure | 2768 |
| regex: literal slash rejection or missing RegExp constructor | 747 |
| runtime: other unresolved identifier | 624 |
| runtime: missing builtin Intl | 541 |
| runtime: missing builtin ArrayBuffer | 507 |
| runtime: missing builtin Function | 357 |
| runtime: generators unimplemented | 238 |
| harness: missing $262 host hooks | 109 |
| runtime: missing builtin Reflect | 77 |
| runtime: missing builtin DataView | 68 |
| process: RefCell borrow panic | 54 |
| runtime: missing builtin Uint8Array | 49 |
| runtime: missing builtin Int32Array | 32 |
| runtime: missing builtin Int8Array | 25 |
| runtime: missing builtin FinalizationRegistry | 15 |
| runtime: missing builtin AggregateError | 13 |
| runtime: missing builtin Float32Array | 12 |
| runtime: missing builtin Uint8ClampedArray | 12 |
| runtime: missing builtin Float64Array | 11 |
| runtime: missing builtin Int16Array | 11 |
| runtime: missing builtin Uint16Array | 11 |
| runtime: missing builtin Uint32Array | 11 |
| runtime: missing builtin WeakRef | 11 |
| timeout | 7 |
| process: stack overflow | 5 |

The largest established coherent failure cluster is **4,256 missing declared include helpers**. The larger 5,825-TypeError bucket contains mixed causes and is not a single root cause. Missing Temporal accounts for 2,889 failures. Regex is the authorized next ticket because it blocks the upstream WPT harness.

### Regex critical path

The conservative, directly attributable regex count is **747 failures (3.22% of 23165 ordinary failures)**: 445 literal slash rejection, 302 missing RegExp constructor. These are exact missing-constructor errors or parser rejection of `/` or `/=` at an operand boundary. This is a lower bound: escaped patterns may fail lexing first, and skipped tests are not counted.

| Broader regex-directory scope | Discovered | Passed | Failed | Timeout | Skipped |
|---|---:|---:|---:|---:|---:|
| Path contains /RegExp/ or /regexp/ | 2285 | 111 | 1482 | 0 | 692 |

The broader directory total is not attributed wholesale to regex: included tests can fail on other engine or harness gaps. `regex_direct_paths` in the analysis artifact records the exact conservative set.

### Harness limitations

- The full glob includes 294 `_FIXTURE.js` companion files; these are recorded because the existing runner discovers them, but they are not all standalone conformance tests.
- 9296 eligible files declare includes that this harness never loads; 274 eligible files declare `onlyStrict`, which it never enforces.
- Ordinary tests run once, without the mandated default/non-strict and strict variants. Negative expectations, modules, async/raw tests and several runtime/syntax features are skipped.
- `sameValue` uses `===`, so NaN and signed zero are wrong; `throws` checks only that something threw and does not validate the error constructor. These can create both false failures and false passes.
- Metadata is parsed heuristically and lexical/path exclusions can overmatch. Reported skipped reasons reflect that implementation, not a standards capability detector.
- Therefore neither the former 196/196 slice nor these process-exit counts establish full Test262 compliance.

### Reproduction and artifacts

Windows lane root: `C:\Users\server\src\wt\js-lang`. Scratch: `.agents\js-lang`. Local evidence mirror: `/Users/jMac/Projects/OpenSource/amazing/.agents/js-lang/`.

Build on Windows with `CARGO_TARGET_DIR=C:\Users\server\target\js-lang` and `cargo build -p amazing --locked`. Preserve the executable hash above. The observer imports the unchanged harness and calls its `main()`, `metadata()` and `run()`; it only records outcomes. It does not add tests, parallelize, alter prelude, change timeout, or override eligibility.

```powershell
python .agents\js-lang\observe_test262.py --harness scripts\test262.py --root .agents\js-lang\test262-full --binary .agents\js-lang\baseline-amazing.exe --output .agents\js-lang\run1
# Repeat with --output .agents\js-lang\run2.
python .agents\js-lang\inventory_test262.py .agents\js-lang\test262-full scripts\test262.py .agents\js-lang\inventory.json
python .agents\js-lang\analyze_test262.py .agents\js-lang\run1
python .agents\js-lang\analyze_test262.py .agents\js-lang\run2
```

Each run retains `records.json`, `summary.json`, `analysis.json`. Scratch also retains the observer, inventory/analyzer sources, selected-source manifest, source-parity record, frozen executable on Windows, comparison result and build/test logs. JSON record artifacts are compact to keep every file under 4,000 lines.

## JS-2: RegExp support — authorized next

Type: Implementation. Blocked by: no workspace-registration blocker (integrator supplied the std-only amazing-regex skeleton).

Question: Can the engine correctly lex and execute the bounded RegExp subset required by the unmodified pinned WPT harness, without confusing division and regex literals?

Exit criteria:

- Parser-guided regex literal handling and a std-only amazing-regex implementation for character classes, quantifiers, groups, alternation, anchors and g/i/m/s/u/y flags; explicit unsupported-feature errors and resource bounds.
- RegExp constructor/exec/test plus regex-aware String.match and String.replace needed by testharness.js; include noncapturing groups and negative lookahead used by the pinned harness.
- Windows crate regressions, pinned measured Test262 selection repeated with stable classifications, and an upstream testharness.js parse/execution probe that reports the next blocker honestly.
- Document remaining unsupported RegExp features; coordinate cross-lane changes through questions.md.

The integrator explicitly selected regex as the next ticket because two lanes are blocked behind it, regardless of the largest numeric cluster. No JS-1 engine fix is included.

### JS-1 validation

Windows `cargo test -p amazing-js --locked`: 143 unit tests + 1 doctest passed. Final `cargo test --workspace --locked` passed: 725 passed, 0 failed, 3 ignored across 46 test-result groups. Initial workspace attempts were blocked by incomplete integrator skeleton sync and then by the in-use baseline executable; the final run completed after both measurements.

## JS-2 delivered: bounded RegExp and upstream harness unblock

The **unmodified pinned testharness.js now parses**. It also initializes and
executes real upstream assertion callbacks with only the shell host alias
`var self=globalThis;`. The diagnostic reports one PASS, one intentional FAIL,
and harness completion OK. This proves the parser/runtime unblock; it is not
a DOM WPT conformance result. The file was not patched or replaced with a shim.

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
UTF-8/LF harness source SHA-256:
`d2399236c2a09c429804ff2299ad6629e17e2b53f17a74dd341e936adb11ae3e`.
Without a host-provided `self`, the standalone CLI correctly reports that name
missing; the DOM harness host already supplies it.

### Scope and why the additional fixes are required

The new std-only amazing-regex core implements classes/ranges, greedy and lazy
quantifiers, groups/captures, alternation, anchors, backreferences, positive and
negative lookahead, UTF-16 indexing and flags g/i/m/s/u/y. Parser-directed slash
lexing preserves division. The interpreter supplies constructor/exec/test,
source and flag getters, lastIndex and regex-aware String match/search/replace.
Unicode case folding uses pinned Unicode 17.0.0 C/S data with its license.

The pinned harness itself exposed these prerequisites, all fixed in amazing-js:

- Its dynamically constructed escape regex uses a template substitution inside
  a function: record each substitution's enclosing brace depth.
- Its escape map calls String.fromCharCode: implement conversion for strings
  representable by the current UTF-8 JS value model.
- Its API is exported as global object properties: unresolved identifiers and
  typeof consult that object; assignments to such properties keep the reference.
- Its setup overload checks actual arguments.length and instanceof Function;
  supply the actual count and Function constructor/prototype identity.
- Its property setup calls Object.prototype.hasOwnProperty, and its step wrapper
  calls Array.prototype.slice.call: supply the method and correct Array prototype.
- Its assertion wrapper forwards rest parameters: bind the remaining arguments
  as an array rather than taking only the first value.
- Its completion predicate calls Array.prototype.some: implement the short-circuit
  callback with hole skipping and a bounded length.
- RegExp inherited getters preserve the original receiver and drop object borrows
  before calling JS, including a getter that mutates its receiver.

`crates/amazing-js/examples/probe_harness.rs` is the runnable shell proof. It
parses the supplied unchanged script, initializes it, executes regex assertions,
checks both real result callbacks, and checks the completion callback after jobs
are drained. No debug instrumentation remains in engine code.

### Measured gate

Build base: `d8437f3f1693436ecdff574aba971db939f288e7` (refreshed Windows tree).
All owned changes were re-synced after refresh. 159 Rust/manifest files matched
Mac SHA-256 values immediately before and after the final workspace gate.
Earlier results affected by refresh are superseded by these final measurements.

Test262 revision remains `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`. Selection is the union of
the original 747 directly attributed failures, all `/RegExp/` or `/regexp/`
directory files, and built-ins/String/prototype match, replace, search and split
files. Exact membership is recorded in scratch `js2-selection.json`.

| Run | Discovered | Pass | Fail | Timeout | Skip |
|---|---:|---:|---:|---:|---:|
| JS-1 baseline, same selection | 2714 | 127 | 1856 | 0 | 731 |
| JS-2 final run 1 | 2714 | 798 | 1185 | 0 | 731 |
| JS-2 final run 2 | 2714 | 798 | 1185 | 0 | 731 |

671 fail-to-pass transitions; zero pass-to-fail regressions; zero classification
or reason changes between final runs. Of the original 747 directly regex-attributed
failures, 391 now pass and 356 still fail (including other exposed engine/harness
limitations). These gains include the required harness prerequisites above.

| Directory/selection group | Files | Before pass | Before fail | After pass | After fail | Skip | After pass / eligible |
|---|---:|---:|---:|---:|---:|---:|---:|
| Annex B regex directories | 168 | 2 | 101 | 33 | 70 | 65 | 32.04% |
| Other direct-regex failures | 149 | 0 | 149 | 69 | 80 | 0 | 46.31% |
| built-ins/RegExp | 1879 | 91 | 1347 | 586 | 852 | 441 | 40.75% |
| built-ins/String/prototype/endsWith | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/includes | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/match | 51 | 2 | 40 | 20 | 22 | 9 | 47.62% |
| built-ins/String/prototype/matchAll | 2 | 0 | 2 | 2 | 0 | 0 | 100.00% |
| built-ins/String/prototype/replace | 55 | 7 | 39 | 33 | 13 | 9 | 71.74% |
| built-ins/String/prototype/replaceAll | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/search | 43 | 2 | 30 | 11 | 21 | 11 | 34.38% |
| built-ins/String/prototype/split | 120 | 5 | 105 | 6 | 104 | 10 | 5.45% |
| built-ins/String/prototype/startsWith | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/toLocaleLowerCase | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/toLocaleUpperCase | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/toLowerCase | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/toUpperCase | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| built-ins/String/prototype/trim | 1 | 0 | 1 | 0 | 1 | 0 | 0.00% |
| language/literals/regexp | 238 | 18 | 34 | 38 | 14 | 186 | 73.08% |

All rows have zero timeouts. Percentages describe this harness's eligible files,
not complete ECMA-262 conformance. JS-1's caveats remain: missing includes, strict
variants not run, incomplete sameValue/throws assertions, and metadata skips.
For example, two matchAll exception tests pass under this permissive throws
helper even though matchAll is not implemented. No full-tree post-JS-2 result
is claimed; the historical full-tree baseline remains 53872/5438/23165/7/25262.

Final Windows validation:

- `cargo test -p amazing-js -p amazing-regex --locked`: 156 JS unit tests,
  10 regex unit tests, 1 JS doctest passed; no failures.
- `cargo test --workspace --locked`: 992 passed, 0 failed,
  3 ignored across 52 result groups; exit 0.
- CLI and both examples build with --locked. The unchanged upstream probe exits
  0 and prints `parsed; initialized; subtests=PASS,FAIL; completion=OK`.
- Every owned file is below 4000 lines; git diff --check passes.

Reproducibility hashes:

| Artifact | SHA-256 |
|---|---|
| Final CLI | `4f3a18f91d6491a02fa9d934973c8ce59e411715a02e64da24a8a843134b1bba` |
| Both final classifications | `952ab71398b351d130aeea88c5a7f18cc5e9b1e6d67b18011806bd5a17b10338` |
| Selection paths | `fbaaf7f7d0e897133a0d32fe439f79e1ef91fc4f46380d01350f3e4965d4a353` |
| Selected Test262 sources | `3425ae5f4b28a9b51472d0edf46ca8af60e3f6b5762b71e753f4577a5a27422d` |
| Unchanged scripts/test262.py | `26d666f7529a1788fbbfe7c119f7f6f149cf32c0e0914a604017592cda8ab79c` |
| Selection observer | `5baccd96598cb68db46ac45e29eeacc0f7e4fc007c3506e6607425544b18ee62` |
| 159-file source manifest | `0b9ed5f690073bfbe1ac61ae6a50ec9a7aff5205fa5aec9e1ebe41c30b3ce5bb` |
| Final Cargo.lock | `3feaf8130f2c60230292a4d70311c6e480d848a2ffdebc036fd09a603581ba7d` |

### Revalidation after rebuilding artifacts

The earlier JS-2 post-refresh gates were treated as suspect because archive
mtimes could permit stale first-party artifacts. These results supersede them.
The implementation was already merged in `29294fc`; validation was repeated
on integrator-confirmed stable dev `d8437f3f1693436ecdff574aba971db939f288e7` after re-sync.

The lane created a separate, initially **absent** Cargo target:
`C:/Users/server/target/js-lang/js2-fresh-20260920T180036Z`. A pre-build existence guard and
`fresh-artifact-origin.json` record that no prior artifacts existed there.
Cargo rebuilt first-party crates and third-party adapters; build parallelism
was limited to two jobs. Source hashes are supporting evidence, not the basis
for claiming artifact freshness. All 159 Rust/manifest files matched the
pinned source snapshot before and after the gates.

Fresh Windows results: 156 JS unit tests, 10 regex unit tests and 1 doctest pass;
workspace **992 passed / 0 failed / 3 ignored across 52 targets**. The workspace
count also agrees with the integrator's independently reported clean gate.
Both fresh Test262 runs remain **798 pass / 1185 fail / 731 skip / 0 timeout**;
there are zero status/reason changes from the earlier classification and zero
repeat differences. The unchanged upstream harness again parses, initializes,
emits PASS/FAIL and completes OK in both the CLI and standalone example.

Evidence: scratch `fresh-config.json`, `fresh-artifact-origin.json`,
`fresh-source-manifest.json`, `fresh-final-parity.json`, `fresh-check.log`,
`fresh-crate-tests.log`, `fresh-workspace-tests.log`, `fresh-workspace-summary.json`,
`fresh-cli-build.log`, `fresh-examples-build.log`, `fresh-run1/`, `fresh-run2/`,
`upstream-fresh-proof.json`, `upstream-fresh-example.json`, and `fresh-validation.json`.
Root Cargo.lock still differs from the refreshed `57f0e69` version by exactly
the authorized amazing-js -> amazing-regex edge; the original full diff in the lane report
remains accurate. No implementation changes were necessary for revalidation.

### Unsupported surface and bounds

Still missing: named captures/backreferences, lookbehind, property escapes,
inline modifiers, d/v flags and Unicode sets; Annex B octal/decimal fallbacks,
incomplete escapes and quantified lookahead; Symbol protocols, species/subclass
semantics, custom exec dispatch in String methods, regex split, matchAll,
replaceAll, RegExp.escape, compile and legacy RegExp static properties. Existing
strict-mode, descriptor and built-in gaps still apply. String.fromCharCode and
regex capture/replacement output cannot represent lone surrogates in the current
UTF-8 JS string model and throw explicitly; the regex core supports lossless u16.

Bounds: 16384 pattern bytes, 64 nesting levels, 128 captures, 32768 compilation
work/instruction units, 1048576 input code units, 1000000 shared matching steps,
8192 backtracking frames and 1048576 saved slots. Global collection caps at
100000 matches, replacement capture storage at 1048576 slots, and replacement
output at 16777216 bytes. Execution limit failures are RangeErrors; regex literal
compilation failures are positioned parse errors. See the regex README for detail.

### Lockfile exception and reproduction

Cargo regenerated root Cargo.lock on Windows using `cargo check -p amazing-js
--offline`. An exact comparison with the pre-build file and a final comparison
against origin/dev confirmed that the **only** change is adding amazing-regex
to amazing-js's dependency list. No manual lockfile editing or other dependency
change occurred. The full diff is included in the lane report.

```powershell
$env:CARGO_TARGET_DIR='C:/Users/server/target/js-lang/js2-fresh-20260920T180036Z'
$env:CARGO_BUILD_JOBS='2'
cargo test -p amazing-js -p amazing-regex --locked
cargo build -p amazing --locked
cargo build -p amazing-js --examples --locked
python .agents/js-lang/observe_js2.py --harness scripts/test262.py --root .agents/js-lang/test262-full --binary C:/Users/server/target/js-lang/js2-fresh-20260920T180036Z/debug/amazing.exe --output .agents/js-lang/fresh-run1 --selection .agents/js-lang/js2-selection.json
# Repeat with --output .agents/js-lang/fresh-run2.
python .agents/js-lang/probe_upstream_fresh.py
cargo test --workspace --locked
```

For a fresh upstream checkout, feed the unchanged resources/testharness.js bytes
to `debug/examples/probe_harness.exe` under that target directory on Windows. The scratch
probe does this from the pinned source JSON and records source/executable hashes.

### Next boundary

JS-2 is merged and freshly revalidated. Resume DOM-1 against the real upstream
harness. No DOM conformance percentage is claimed here. Do not silently broaden
JS-2 into full UTF-16 strings, Symbol protocols or harness repair: choose those
as separate measured tickets. The largest JS-1 helper-loading cluster remains
a candidate for an integrator-owned scripts/test262.py follow-up. Stop here.

## JS-3 — Test262 runner contract (implemented)

The integrator explicitly granted `scripts/test262.py` for this ticket. All
historical JS-1/JS-2 Test262 totals above are withdrawn as conformance evidence:
they measured exit codes under a weakened harness. They remain historical
measurements only and must not be compared as a before/after conformance rate.

The replacement follows the pinned suite's [INTERPRETING.md](https://github.com/tc39/test262/blob/3655e7464de3d52643ecddd4b5f9f4f3e7f62398/INTERPRETING.md).
Python runner tooling uses pinned PyYAML 6.0.3 to parse full YAML, including
block/flow collections, multiline values and anchors/merges. Engine crates
remain std-only. Metadata records include includes, flags, features, negative
phase/type, esid and locale. Invalid input and missing helpers are failures
with explicit reasons, never implicit exclusions.

Each variant executes in a fresh `amazing-js` example `test262_host` process.
The driver passes length-prefixed UTF-8 fields; the host parses the test
separately, loads each helper as a separate script and reports parse,
harness-parse, harness-runtime, runtime or complete. Non-raw tests receive
unchanged `assert.js`, then `sta.js`, then ordered includes. No helper is
replaced or weakened. Raw test bytes receive no prefix or helpers, including
explicit includes. Default tests have non-strict and strict variants; onlyStrict
and noStrict have one; modules have a single module variant. The strict prefix
is exactly `"use strict";\n`, applied only to the test body. Engine strict-mode
and early-error gaps therefore appear as failures, not runner exclusions.

Negative tests require both the specified phase and thrown constructor name.
Runtime `SyntaxError` cannot satisfy a parse negative, and a harness failure
cannot satisfy any negative. The engine's currently missing Error prototype
constructor wiring produces `<unknown constructor>`, which fails type matching;
`error.name` is never accepted as evidence of type. The real `assert.throws`
independently tests constructor identity and exposes the same engine limitation.

The host supplies `$262.global`, `$262.evalScript`, `print`, and reports
`agent.canBlock=false`. Modules (including resolution), async completion,
blocking agents, additional realms, ArrayBuffer detachment, GC, concurrent
agents, AbstractModuleSource and IsHTMLDDA remain unsupported. Module, async,
CanBlockIsTrue and IsHTMLDDA metadata receive explicit unsupported reasons;
CanBlockIsFalse runs. Absent ECMA-402 support excludes intl402/staging/intl402
with `host.intl402`. Unknown flags are unsupported. Generated and
non-deterministic are informational flags and do not suppress execution.
Unavailable host calls set an out-of-band unsupported marker, even if test
code catches the exception. Missing ordinary language features or builtins
are engine failures; no source-text heuristics skip them. Locale/features
metadata is retained without inventing support or silently filtering tests.

The report covers every discovered file. `_FIXTURE` names are skipped with
`fixture`; user filters/limits use `selection.excluded`. All other exclusions
are explicit unsupported capabilities. Each test records every variant and
reason. File aggregation prioritizes fail, then timeout, unsupported, pass;
a file passes only when all of its variants pass. Both file and variant
denominators, per-mode totals, two-level directories, reason counts and exact
failure diagnostic clusters are retained. Timing/diagnostic text is excluded
from the classification hash; paths, variant names, statuses and reasons are
included. `--compare` checks this hash plus source/host/runner hashes and run
configuration. Timeouts and abnormal process exits never count as passes.

Windows checkout uses `core.autocrlf=false` and restores pinned Git blob bytes;
Python reads sources as bytes before UTF-8 decoding, preserving CRLF/BOM/raw
input. The report records a sorted test/harness content manifest hash. Sources are
snapshotted once; that hash covers the exact bytes subsequently executed. The
full-tree run must use the complete lane-local checkout, not the older
641-file sparse shared checkout.

```powershell
$env:CARGO_TARGET_DIR='C:/Users/server/target/js-lang'
python -m venv .agents/js-lang/js3-venv
& ./.agents/js-lang/js3-venv/Scripts/python.exe -m pip install PyYAML==6.0.3
cargo build -p amazing-js --example test262_host --locked
& ./.agents/js-lang/js3-venv/Scripts/python.exe scripts/test262.py .agents/js-lang/test262-full --binary C:/Users/server/target/js-lang/debug/examples/test262_host.exe --self-test
& ./.agents/js-lang/js3-venv/Scripts/python.exe scripts/test262.py .agents/js-lang/test262-full --binary C:/Users/server/target/js-lang/debug/examples/test262_host.exe --jobs 8 --timeout 60 --output .agents/js-lang/js3-run1.json
& ./.agents/js-lang/js3-venv/Scripts/python.exe scripts/test262.py .agents/js-lang/test262-full --binary C:/Users/server/target/js-lang/debug/examples/test262_host.exe --jobs 8 --timeout 60 --output .agents/js-lang/js3-run2.json --compare .agents/js-lang/js3-run1.json
```

Closing integration gates use the project-standard runner and explicitly retain
doctests, which nextest does not execute:

```powershell
cargo nextest run --workspace --locked
cargo test -p amazing-js --locked --doc
cargo test -p amazing-html --locked --doc
```

Contract probes exercise YAML forms, flag/variant expansion, exact source
framing, helper order, phase/type mismatch rejection, real SameValue and throws,
realm isolation, unsupported calls caught by test code, host eval/global and
process deadlines. The positive Error-constructor probe explicitly wires its
own constructor inside that disposable probe; production test sources and
harness files receive no such repair.

Full-tree validation found a protocol bug that small probes missed: Python's
`splitlines()` treats U+0085/U+2028/U+2029 inside valid JSON strings as line
boundaries. The driver now splits only the wire LF delimiter. Regression probes
cover all three characters, and the five affected pinned files (ten variants)
now yield valid engine-failure records. The provisional sweep is retained as
`js3-shakedown1.json`; both final full-tree runs are restarted after this fix.


The first corrected full sweep at a 10-second wall deadline measured **10,294
pass / 33,259 fail / 69 timeout / 9,956 unsupported**, plus 294 fixture files,
out of 53,872 discovered files. This is nearly twice the old lenient runner's
5,438 passing files. We expected the honest count to be worse; it is better.
The old runner excluded 25,262 files, including many that actually pass. This
is a corrected measurement, not an engine improvement or a comparable old/new
conformance rate. No production engine code changes in JS-3.

The 10-second repeat exposed three files switching between failure and timeout;
six focused variants finished in 5.6–7.5 seconds and exposed the engine's
iterator limit or constructor-identity failure. Final full measurements use
60 seconds per variant and eight workers for both runs. The rejected 10-second
pair is retained as diagnostic evidence and does not satisfy the stable gate.

## JS-3 — Stable full-tree baseline

Test262 `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`; Python 3.12.10 / PyYAML 6.0.3; Windows, 8 process workers, 60-second wall deadline per variant, no selection filters or limits. Both runs use unchanged pinned source bytes and the same frozen host/runner.

Discovered **53,872 files**, yielding **102,918 variants**. Every file and variant has an identical status and reason in both runs. Run durations: 886.672 and 1852.109 seconds.

| Measurement | Pass | Fail | Timeout | Unsupported | Skipped |
|---|---:|---:|---:|---:|---:|
| Files, run 1 | 10314 | 33292 | 16 | 9956 | 294 |
| Files, run 2 | 10314 | 33292 | 16 | 9956 | 294 |
| Variants, both runs | 19820 | 64238 | 32 | 18828 | 0 |
| module | 0 | 0 | 0 | 843 | 0 |
| non-strict | 10086 | 32833 | 16 | 9092 | 0 |
| raw | 17 | 13 | 0 | 0 | 0 |
| strict | 9717 | 31392 | 16 | 8893 | 0 |

The corrected passing-file count is **10,314**, versus the historical lenient runner's **5,438**. We expected the honest number to be worse; it is better, nearly twice as high. The old runner skipped 25,262 files, including many that actually pass. No production engine code changed in JS-3. These counts describe different runner contracts, so the difference is not an engine improvement or a like-for-like conformance-rate gain.

The 294 skipped files are upstream `_FIXTURE` files. Unsupported variants are accounted for below; a file is unsupported only if no variant fails or times out. Mixed fail/timeout files count as failed, so file and variant timeout totals differ.

| Unsupported reason | Variants |
|---|---:|
| `host.async` | 10815 |
| `host.intl402` | 6714 |
| `host.module` | 843 |
| `host.createRealm` | 352 |
| `host.IsHTMLDDA` | 82 |
| `host.CanBlockIsTrue` | 14 |
| `host.gc` | 8 |

### First observed failure clusters

Counts below are failing variants, not unique files. They are diagnostic clusters: the first observable blocker can mask later defects. The broad non-callable bucket includes missing Object/Array/String APIs and computed-method lookup failures; it is not attributed to one defect. Missing constructor wiring is exposed both by real `assert.throws` and negative-type checks. Private fields, early errors and regex diagnostics belong to the front end; runtime APIs and constructor wiring belong to js-runtime.

| First diagnostic | Failing variants |
|---|---:|
| runtime:&lt;unknown constructor&gt; \| TypeError: value is not callable | 14236 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: Temporal is not defined | 7724 |
| parse:SyntaxError \| unexpected character \`#\` | 4523 |
| harness-runtime:&lt;unknown constructor&gt; \| testTypedArray.js: ReferenceError: Float64Array is not defined | 4108 |
| runtime:&lt;unknown constructor&gt; \| Expected a TypeError but got a undefined | 2595 |
| negative.phase:parse!=runtime \| Test262: This statement should not be evaluated. | 1771 |
| runtime:&lt;unknown constructor&gt; \| Expected a Test262Error but got a undefined | 1453 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: ArrayBuffer is not defined | 1198 |
| runtime:&lt;unknown constructor&gt; \| Expected a ReferenceError but got a undefined | 1117 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: Proxy is not defined | 1023 |
| runtime:&lt;unknown constructor&gt; \| Expected a TypeError to be thrown but no exception was thrown at all | 1008 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: Iterator is not defined | 950 |
| parse:SyntaxError \| Unicode properties/named references are unsupported at pattern offset 3 | 886 |
| parse:SyntaxError \| unexpected character \`\\` | 828 |
| runtime:&lt;unknown constructor&gt; \| TypeError: generators are not implemented | 803 |
| runtime:&lt;unknown constructor&gt; \| TypeError: BigInt is not implemented | 553 |
| runtime:&lt;unknown constructor&gt; \| Expected a Test262Error to be thrown but no exception was thrown at all | 542 |
| parse:SyntaxError \| unexpected \`)\` | 494 |
| runtime:&lt;unknown constructor&gt; \| isConstructor invoked with a non-function value | 462 |
| runtime:&lt;unknown constructor&gt; \| Expected SameValue(«"undefined"», «"function"») to be true | 411 |
| harness-runtime:&lt;unknown constructor&gt; \| resizableArrayBufferUtils.js: ReferenceError: Uint8Array is not defined | 376 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: DataView is not defined | 352 |
| runtime:&lt;unknown constructor&gt; \| ReferenceError: Reflect is not defined | 313 |
| parse:SyntaxError \| unexpected keyword \`with\` | 310 |
| runtime:&lt;unknown constructor&gt; | 302 |

Next-ticket evidence: constructor/prototype wiring and absent builtin APIs affect large runtime clusters; private-field syntax and missing early errors are front-end candidates. The counts above identify observed blockers, not the number that would pass after one fix. No next ticket is started here.

### Reproducibility and validation

- `input_sha256`: `a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`.
- `host_sha256`: `0418bac1becb790052c82b163359e1164a843c8a07cc1435d4c9de8ec1606e15`.
- `runner_sha256`: `bdb36351facc0475e661bea7ef928ebcc63e06e77d0a2740d897d77b3476d595`.
- `classification_sha256`: `c0b747c840bc51e5977b4355a5e6d4715ad8dd39c8b21b85904810e5a375a04f`.
- Pinned provenance: 53,915 test/helper JavaScript blobs, zero Git-blob mismatches and zero extra JavaScript files.
- Runner protocol/input failures: **0**. Real-helper and host contract probes passed on Windows.
- Fresh Windows workspace at dev `b5d1b99` (lane base `622012d95d8228b932250a6b3a362f7991800912` plus JS-3 changes): nextest **1229 pass / 0 fail / 3 skipped**. Separate doctests: amazing-js **1 pass / 0 fail**; amazing-html **1 pass / 0 fail**.
- Frozen measurement host was rebuilt after the 79464ca artifact-invalidating refresh; JS/regex source is unchanged through the closing dev head. Closing self-refresh also invalidated first-party artifacts. Source parity: 374 inputs identical before and after the gate.
- Root lock SHA-256: `f4ff57271aa44d67cd1a9d2eee65074b7fb885d8d181cf71b8f738e4a4159fb3`. **Cargo.lock diff for JS-3: empty**; the JS → regex edge was already merged in JS-2.
- Artifacts: `.agents/js-lang/js3-run1.json`, `js3-run2.json`, `js3-analysis.json`, `js3-test262-provenance.json`, `js3-source-manifest.json`, and Windows workspace logs in the lane reporting directory. Per-file reports retain complete classifications and metadata; this table is the durable summary.

### Per-directory file counts

Directories are grouped by the first two directory components (never by filename). The JSON reports also retain variant totals for each directory. P/F/T/U/S mean pass/fail/timeout/unsupported/skipped; all rows together partition the full discovery set.

| Directory | Files | Pass | Fail | Timeout | Unsupported | Skipped |
|---|---:|---:|---:|---:|---:|---:|
| annexB/built-ins | 241 | 4 | 221 | 0 | 16 | 0 |
| annexB/language | 845 | 193 | 626 | 0 | 26 | 0 |
| built-ins/AbstractModuleSource | 8 | 0 | 0 | 0 | 8 | 0 |
| built-ins/AggregateError | 25 | 0 | 24 | 0 | 1 | 0 |
| built-ins/Array | 3082 | 829 | 2137 | 10 | 106 | 0 |
| built-ins/ArrayBuffer | 221 | 0 | 220 | 0 | 1 | 0 |
| built-ins/ArrayIteratorPrototype | 27 | 0 | 27 | 0 | 0 | 0 |
| built-ins/AsyncDisposableStack | 104 | 0 | 74 | 0 | 30 | 0 |
| built-ins/AsyncFromSyncIteratorPrototype | 38 | 0 | 1 | 0 | 37 | 0 |
| built-ins/AsyncFunction | 18 | 0 | 17 | 0 | 1 | 0 |
| built-ins/AsyncGeneratorFunction | 23 | 0 | 15 | 0 | 8 | 0 |
| built-ins/AsyncGeneratorPrototype | 48 | 0 | 13 | 0 | 35 | 0 |
| built-ins/AsyncIteratorPrototype | 13 | 0 | 9 | 0 | 4 | 0 |
| built-ins/Atomics | 389 | 0 | 317 | 0 | 72 | 0 |
| built-ins/BigInt | 77 | 0 | 76 | 0 | 1 | 0 |
| built-ins/Boolean | 51 | 16 | 34 | 0 | 1 | 0 |
| built-ins/DataView | 561 | 0 | 559 | 0 | 2 | 0 |
| built-ins/Date | 594 | 12 | 579 | 0 | 3 | 0 |
| built-ins/DisposableStack | 93 | 0 | 92 | 0 | 1 | 0 |
| built-ins/Error | 93 | 1 | 89 | 0 | 3 | 0 |
| built-ins/FinalizationRegistry | 47 | 0 | 46 | 0 | 1 | 0 |
| built-ins/Function | 509 | 130 | 366 | 0 | 13 | 0 |
| built-ins/GeneratorFunction | 23 | 0 | 21 | 0 | 2 | 0 |
| built-ins/GeneratorPrototype | 61 | 0 | 61 | 0 | 0 | 0 |
| built-ins/Infinity | 6 | 2 | 4 | 0 | 0 | 0 |
| built-ins/Iterator | 654 | 0 | 653 | 0 | 1 | 0 |
| built-ins/JSON | 165 | 31 | 132 | 0 | 2 | 0 |
| built-ins/Map | 204 | 24 | 179 | 0 | 1 | 0 |
| built-ins/MapIteratorPrototype | 11 | 0 | 11 | 0 | 0 | 0 |
| built-ins/Math | 327 | 59 | 268 | 0 | 0 | 0 |
| built-ins/NaN | 6 | 2 | 4 | 0 | 0 | 0 |
| built-ins/NativeErrors | 94 | 8 | 80 | 0 | 6 | 0 |
| built-ins/Number | 340 | 100 | 239 | 0 | 1 | 0 |
| built-ins/Object | 3411 | 666 | 2744 | 0 | 1 | 0 |
| built-ins/Promise | 732 | 23 | 293 | 0 | 416 | 0 |
| built-ins/Proxy | 311 | 0 | 273 | 0 | 38 | 0 |
| built-ins/Reflect | 153 | 0 | 153 | 0 | 0 | 0 |
| built-ins/RegExp | 1879 | 683 | 1193 | 0 | 3 | 0 |
| built-ins/RegExpStringIteratorPrototype | 17 | 0 | 17 | 0 | 0 | 0 |
| built-ins/Set | 383 | 39 | 343 | 0 | 1 | 0 |
| built-ins/SetIteratorPrototype | 11 | 0 | 11 | 0 | 0 | 0 |
| built-ins/ShadowRealm | 67 | 0 | 60 | 0 | 4 | 3 |
| built-ins/SharedArrayBuffer | 104 | 0 | 103 | 0 | 1 | 0 |
| built-ins/String | 1223 | 252 | 968 | 0 | 3 | 0 |
| built-ins/StringIteratorPrototype | 7 | 0 | 7 | 0 | 0 | 0 |
| built-ins/SuppressedError | 22 | 0 | 21 | 0 | 1 | 0 |
| built-ins/Symbol | 98 | 0 | 81 | 0 | 17 | 0 |
| built-ins/Temporal | 4603 | 0 | 4603 | 0 | 0 | 0 |
| built-ins/ThrowTypeError | 14 | 0 | 13 | 0 | 1 | 0 |
| built-ins/TypedArray | 1446 | 0 | 1446 | 0 | 0 | 0 |
| built-ins/TypedArrayConstructors | 738 | 0 | 738 | 0 | 0 | 0 |
| built-ins/Uint8Array | 70 | 0 | 70 | 0 | 0 | 0 |
| built-ins/WeakMap | 141 | 0 | 140 | 0 | 1 | 0 |
| built-ins/WeakRef | 29 | 0 | 28 | 0 | 1 | 0 |
| built-ins/WeakSet | 85 | 0 | 84 | 0 | 1 | 0 |
| built-ins/decodeURI | 55 | 0 | 55 | 0 | 0 | 0 |
| built-ins/decodeURIComponent | 56 | 0 | 56 | 0 | 0 | 0 |
| built-ins/encodeURI | 31 | 0 | 31 | 0 | 0 | 0 |
| built-ins/encodeURIComponent | 31 | 0 | 31 | 0 | 0 | 0 |
| built-ins/eval | 10 | 1 | 9 | 0 | 0 | 0 |
| built-ins/global | 29 | 16 | 13 | 0 | 0 | 0 |
| built-ins/isFinite | 15 | 2 | 13 | 0 | 0 | 0 |
| built-ins/isNaN | 15 | 2 | 13 | 0 | 0 | 0 |
| built-ins/parseFloat | 54 | 45 | 9 | 0 | 0 | 0 |
| built-ins/parseInt | 55 | 47 | 8 | 0 | 0 | 0 |
| built-ins/undefined | 8 | 3 | 5 | 0 | 0 | 0 |
| harness | 116 | 15 | 82 | 1 | 18 | 0 |
| intl402 | 22 | 0 | 0 | 0 | 22 | 0 |
| intl402/Array | 2 | 0 | 0 | 0 | 2 | 0 |
| intl402/BigInt | 11 | 0 | 0 | 0 | 11 | 0 |
| intl402/Collator | 65 | 0 | 0 | 0 | 65 | 0 |
| intl402/Date | 12 | 0 | 0 | 0 | 12 | 0 |
| intl402/DateTimeFormat | 244 | 0 | 0 | 0 | 244 | 0 |
| intl402/DisplayNames | 57 | 0 | 0 | 0 | 57 | 0 |
| intl402/DurationFormat | 110 | 0 | 0 | 0 | 110 | 0 |
| intl402/FallbackSymbol | 2 | 0 | 0 | 0 | 2 | 0 |
| intl402/Intl | 66 | 0 | 0 | 0 | 66 | 0 |
| intl402/ListFormat | 81 | 0 | 0 | 0 | 81 | 0 |
| intl402/Locale | 168 | 0 | 0 | 0 | 168 | 0 |
| intl402/Number | 7 | 0 | 0 | 0 | 7 | 0 |
| intl402/NumberFormat | 249 | 0 | 0 | 0 | 249 | 0 |
| intl402/PluralRules | 53 | 0 | 0 | 0 | 53 | 0 |
| intl402/RelativeTimeFormat | 80 | 0 | 0 | 0 | 80 | 0 |
| intl402/Segmenter | 79 | 0 | 0 | 0 | 79 | 0 |
| intl402/String | 19 | 0 | 0 | 0 | 19 | 0 |
| intl402/Temporal | 2029 | 0 | 0 | 0 | 2029 | 0 |
| intl402/TypedArray | 1 | 0 | 0 | 0 | 1 | 0 |
| language/arguments-object | 263 | 23 | 180 | 0 | 60 | 0 |
| language/asi | 102 | 100 | 2 | 0 | 0 | 0 |
| language/block-scope | 145 | 45 | 100 | 0 | 0 | 0 |
| language/comments | 52 | 38 | 13 | 0 | 1 | 0 |
| language/computed-property-names | 48 | 15 | 33 | 0 | 0 | 0 |
| language/destructuring | 19 | 13 | 6 | 0 | 0 | 0 |
| language/directive-prologue | 62 | 28 | 34 | 0 | 0 | 0 |
| language/eval-code | 347 | 53 | 241 | 0 | 53 | 0 |
| language/export | 3 | 0 | 0 | 0 | 3 | 0 |
| language/expressions | 11164 | 3441 | 5400 | 1 | 2260 | 62 |
| language/function-code | 217 | 165 | 52 | 0 | 0 | 0 |
| language/future-reserved-words | 55 | 39 | 16 | 0 | 0 | 0 |
| language/global-code | 42 | 15 | 27 | 0 | 0 | 0 |
| language/identifier-resolution | 14 | 6 | 8 | 0 | 0 | 0 |
| language/identifiers | 268 | 139 | 129 | 0 | 0 | 0 |
| language/import | 191 | 6 | 0 | 0 | 123 | 62 |
| language/keywords | 25 | 24 | 1 | 0 | 0 | 0 |
| language/line-terminators | 41 | 32 | 9 | 0 | 0 | 0 |
| language/literals | 534 | 417 | 117 | 0 | 0 | 0 |
| language/module-code | 755 | 1 | 0 | 0 | 598 | 156 |
| language/punctuators | 11 | 10 | 1 | 0 | 0 | 0 |
| language/reserved-words | 27 | 20 | 6 | 0 | 1 | 0 |
| language/rest-parameters | 11 | 8 | 3 | 0 | 0 | 0 |
| language/source-text | 1 | 0 | 1 | 0 | 0 | 0 |
| language/statementList | 80 | 79 | 1 | 0 | 0 | 0 |
| language/statements | 9350 | 2101 | 4723 | 3 | 2520 | 3 |
| language/types | 113 | 75 | 36 | 0 | 2 | 0 |
| language/white-space | 67 | 51 | 16 | 0 | 0 | 0 |
| staging | 4 | 0 | 4 | 0 | 0 | 0 |
| staging/Temporal | 2 | 0 | 2 | 0 | 0 | 0 |
| staging/Uint8Array | 1 | 0 | 1 | 0 | 0 | 0 |
| staging/built-ins | 8 | 0 | 8 | 0 | 0 | 0 |
| staging/decorators | 3 | 0 | 3 | 0 | 0 | 0 |
| staging/explicit-resource-management | 53 | 0 | 15 | 0 | 38 | 0 |
| staging/set-methods | 3 | 0 | 3 | 0 | 0 | 0 |
| staging/sm | 1409 | 165 | 1193 | 1 | 47 | 3 |
| staging/source-phase-imports | 4 | 0 | 0 | 0 | 2 | 2 |
| staging/top-level-await | 4 | 0 | 0 | 0 | 1 | 3 |

### Runtime crash evidence

35 failing variants across 19 files terminate abnormally. They remain failures in the tables above. Counts by observed cause: RefCell borrow panic: 22, stack overflow: 13. Full stderr and exit codes remain in the per-variant JSON.

JS-3 stops here. The next front-end or runtime ticket is selected from this baseline; none is started by this lane.


## JS-4 — External scheduling seam (local design; gates not run)

Integrator authorized this ticket after merging JS-3. Two integration lanes
need the JS timers, intervals, animation frames, queueMicrotask callbacks and
Promise jobs to participate in the driver-stepped DOM event loop. Execution
files now belong to js-runtime. This lane defines/reviews the contract and
owns the preserved Test262 measurement; implementation is routed to js-runtime
in its own worktree, separated from JSRT-1's VM/GC slice. No AST change is needed.

### Audited coupling

At dev `9bf1224`, `run_program`, `dispatch_dom_event` and `pump_timers` call
`drain_jobs`; the latter runs the entire private Promise queue. Four enqueue
sites in interp_builtins.rs push PromiseJob directly. Timers own Instant
expiries and rearm internally. requestAnimationFrame is a 16ms timer with
constant timestamp zero. queueMicrotask is not installed. Exposing only
pump_timers/drain_microtasks would preserve the bug instead of supplying the
DOM loop with independently invocable work.

### Proposed contract sent to the execution owner

Use an opt-in external-mode constructor before any page code, retaining
Interpreter::new() as the standalone default. Types can live under the existing
public amazing_js::interp module; no JS-to-DOM dependency or root lock edit is
needed. Exact Rust names await the owner's acknowledgment.

A synchronous scheduling sink receives ordered registration/cancellation
requests with scalar metadata and opaque rooted tokens. Its only job is to
register work; it cannot call JS reentrantly. A post-script outbox alone is
insufficient: native mutation-observer enqueue notifications must interleave
with Promise and queueMicrotask notifications at their original enqueue points.
All Promise enqueue sites must use this same emission path.

Requests distinguish timers (registration, token, signed converted delay,
repeat), timer cancellation, animation frames, frame cancellation and individual
microtasks. JS roots callback values/arguments/reactions behind tokens; the
embedding maps registrations to DOM TimerId/AnimationFrameId. The future GC
must trace these roots. One-shot roots are released on consumption, intervals
on cancellation; all on interpreter teardown. Stale tokens cannot alias newer
work after handle reuse. No registry/object borrow spans a JS invocation or
synchronous sink notification.

External time belongs to the embedding's EventLoop::advance_to(Duration).
JS emits relative delays and never reads Instant, sleeps, independently computes
expiry or rearms intervals in external mode. DOM owns timer nesting/clamping,
active-time eligibility, task selection and interval repetition. Frame callbacks
receive caller-supplied monotonic Duration converted to milliseconds; rAF is
not a timer. The embedding supplies the Window receiver and preserves timer
extra arguments. Timer/interval cancellation shares its namespace; rAF is
separate.

**One owned callback/job invocation runs exactly one item.** New requests remain
observable in enqueue order. No hidden drain is allowed on return, throw,
nested eval or Promise settlement. External run_script/run_program and DOM
entrypoints also omit automatic drains; explicit legacy pump/drain APIs cannot
bypass external scheduling. A thrown callback leaves other work intact. The
adapter reports the error and completes the DOM StepId before continuing.

The driver controls checkpoints and rendering opportunities, including explicit
per-callback boundaries inside an enclosing task where appropriate. A single
final dispatch drain is not a resumable multi-listener dispatch seam. Required
additional DOM/driver continuation hooks must be routed to their owners and
remain unclaimed until implemented.

### Acceptance traces and failure cases

| Case | Required observable result |
|---|---|
| External script registers work | Registration notifications only; no callbacks execute |
| Timer A queues Promise P, then microtask Q; P queues R | Invoke A alone: A; host checkpoint invokes P,Q,R; next task B sees A,P,Q,R,B |
| Two pending Promise jobs | Invoking first leaves second untouched; any chained reaction is a new request |
| Promise P, native mutation M, queueMicrotask Q | Same shared enqueue order P,M,Q; post-return polling is insufficient |
| Callback schedules Q then throws | Q remains observable; later work remains invocable; host reports error and completes current step |
| A cancels already-due timer B | B token cannot invoke; cancellation reaches the DOM timer map |
| rAF A queues P which cancels frame B | Host's checkpoint runs P before continuing frame snapshot; B does not execute |
| Interval clears itself | Callback roots released; no private rearm/reappearance |
| Nested eval from one callback | It cannot trigger another queued job |
| Stale/consumed/foreign or wrong-kind token | Defined rejection/no-op; cannot execute unrelated work |
| Frame time and callback payloads | Supplied monotonic timestamp, Window receiver and timer arguments preserved |

Runtime-owned focused tests use a recording sink and explicit invocations, with
no sleeps. Driver/DOM owners must also connect the seam to actual EventLoop
steps for the task/microtask/frame ordering proof; this request is recorded in
questions.md. Contract packet: `.agents/js-lang/js4-contract.md` in the central
lane reporting directory.

### Gate plan and current limits

JS-4 has **no build/test results yet**. The integrator's fleet-wide win-001
SSH/SCP stand-down is active; no new connection attempts are permitted. All
work in this checkpoint is local reading, rebasing, design and coordination.
The prior JS-3 numbers are the comparison target, not a JS-4 pass claim.

After recovery and implementation handoff: review the runtime-owned diff;
rebase/own-tree refresh/re-sync; verify all compiled inputs around the Windows
gates; run focused scheduling tests with nextest and the JS doctest; run the
unchanged full pinned Test262 measurement at 60s/jobs=8 against the JS-3
classification hash; finish with workspace nextest and separate JS/HTML
doctests. Required Test262 files remain 10314/33292/16/9956 plus 294 fixtures,
with 102918 variants. No VM/GC work or broad Promise fixes are part of JS-4.

## JS-5 — Class private names (front half)

JS-3's largest front-end-owned first diagnostic is `parse:SyntaxError |
unexpected character #`: **4,523 failing variants**. The next ranked
front-end candidates are the mixed negative-phase bucket (1,771 variants),
unsupported Unicode RegExp properties/named references (886), and a broad
backslash rejection bucket (828). JS-5 therefore targets class private-name
syntax and early errors. The 4,523 count is an observed first blocker, not a
promise that 4,523 variants will pass once parsing works.

The lexer emits a distinct private identifier token. The AST keeps private
class keys, private member accesses, and the left operand of private-brand
`in` separate from public property strings. The parser checks lexical class
scope, forward references, duplicate declarations (with one getter/setter
pair allowed only when both have the same static placement), forbidden
`#constructor`, private deletion and `super.#x`, bare `#x` misuse, and private access outside a class. A nested
class can resolve an outer class's private name if it does not redeclare it.

The integrator granted four additive interpreter match arms so this front-end
change compiles without giving private names public-property behavior. Each
arm explicitly rejects private runtime evaluation. Private brands, fields,
methods, accessors and membership tests remain execution-owned work after
JS-4 and BindingHost. Full-tree results must split parse/early-error cases
from the runtime-semantic failures that remain.

### JS-5 measured gate (fixed base `2021707`)

On win-001, the final five changed source files matched the Mac hashes before
and after the gate. `cargo nextest run -p amazing-js --locked` passed **158/158**;
the separate JS doctest passed **1/1**. The slot-wrapped workspace nextest
passed **1,319** tests with **3 skipped** and no failures. Separate workspace
doctests passed **1 amazing-js** and **1 amazing-html**, with no failures.

Pinned Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398` was run
twice over **53,872 files / 102,918 variants**, with 8 workers and a 60-second
per-variant deadline. Both runs returned the same per-file and per-variant
classification hash:
`257aba54cd36b8d40561b3bb98c7598b03c7d680dd2dac2786b2fbe56c202e13`.
The exact input, frozen host and runner hashes matched across both runs.

| Files | Pass | Fail | Timeout | Unsupported | Fixture skipped |
|---|---:|---:|---:|---:|---:|
| Pre-JS-5 baseline | 10,314 | 33,292 | 16 | 9,956 | 294 |
| JS-5, each final run | 10,503 | 33,103 | 16 | 9,956 | 294 |

At the variant level, **375 failed variants became passes and zero passing
variants regressed**. The gains split into **337 parse-negative early-error
variants** across 170 files and **38 syntax-valid variants** across 19 files.
Among the baseline `unexpected character #` cluster, all **1,366** already
passing parse-negative variants still pass. The other **4,573** variants split
into 38 syntax-valid passes and 4,535 failures. **2,597** failures explicitly
say private names are unsupported by the runtime; no private operation is
aliased to a public `"#x"` property. Runtime brand, field, method and accessor
semantics remain with js-runtime.

The provisional sweeps exposed early-error and parser-context regressions;
those are corrected in the final source. A focused Windows probe of the 84
affected files / 168 variants passes, including the nested-function parse-error
unwind that previously crashed. Final reports and complete records live in the
central `.agents/js-lang/js5-before.json`, `js5-after1.json`,
`js5-after2.json`, and `js5-analysis.json` scratch artifacts.

## Queued after JS-5

**JS-6** is the additive lossless UTF-16 string field in the lexer and AST.
Decode string and template escapes into code units, preserving lone surrogates;
keep the existing string field alongside the new field during the js-runtime
`JsString` migration. Record the exact field names here when JS-6 merges.
Test lone surrogates, paired escapes, code-point escapes, and cooked versus raw
template text. **JS-7**, briefed in `.agents/js-lang/queued-js7.md`, follows
only after JS-6 is reported.

## JS-6 handoff

Code commit: `12e5deb` (based on `0871d92`). js-runtime can build directly
against the field names below.

The lexer now carries lossless UTF-16 code units in `Token.cooked_utf16` for
string literals and template chunks, and `Token.raw_utf16` for template chunks.
Both fields have type `Option<Vec<u16>>`; non-string tokens contain `None`.

The AST exposes `Expr.string_utf16: Option<Vec<u16>>` beside
`ExprKind::Str(String)`. `TemplateLit.quasis_utf16` and
`TemplateLit.raw_quasis_utf16` are `Vec<Vec<u16>>`, aligned one-for-one with the
existing `TemplateLit.quasis: Vec<String>`. Quoted object and class keys retain
their units in `Property.key_utf16`, `ClassMethod.key_utf16`, and
`ClassField.key_utf16`, each `Option<Vec<u16>>` beside the existing `PropKey`.
The legacy Rust strings may contain U+FFFD for a lone surrogate; runtime string
operations must read the UTF-16 fields during the `JsString` migration.

Identifier escapes remain deferred in the lexer. When implemented, they must
resolve to valid identifier code points; arbitrary UTF-16 code units are for
string and template values only.

## JS-7 — WPT `dom/` parser barriers

Initial fixed base: `0244456e725f0e97755df756c062fca31aa6bea6`. The lane
and win-001 build trees were refreshed to this base. Later `dev` movement
touched `amazing-js`, so the final branch gate was repeated on fixed base
`bc8aa45523d156f7e2f92128df7f585539509993`; the integrator owns the
merged-head gate. This ticket changes only the JS parser. A method's `async`
modifier must have no newline *after* it; a newline
before it is legal. `await` is likewise legal at the start of a line in an
async body. Calls permit one trailing argument comma. These changes are paired
with method-formal early errors so newly recognized async methods retain
required parse rejections: duplicate bound names, `await` in formals, strict
`eval`/`arguments` formals, and a `use strict` body with non-simple parameters.
Non-static class constructors also reject async and generator modifiers; the
full Test262 sweep found four parse-negative class files that exposed this
early error after the newline fix.

The before ruler is the full pinned Test262 revision
`3655e7464de3d52643ecddd4b5f9f4f3e7f62398`: 53,872 files and 102,918
variants, 8 workers, 60-second variant deadline. On the fixed base it measured
**10,594 pass / 33,011 fail / 17 timeout / 9,956 unsupported / 294 fixture
skips**. The full classification hash was
`d80b8fd8eb5e086aa4a95b7c2619d0ab04e636ee372ff2c2a0f5967c3479c8ea`.

Focused directories were each run twice before and twice after with identical
per-file and per-variant classifications in each pair. File results:

| Directory | Before pass | After pass | Before fail | After fail | Unsupported |
|---|---:|---:|---:|---:|---:|
| `language/expressions/object/method-definition` | 92 | 105 | 110 | 97 | 101 |
| `language/expressions/await` | 8 | 8 | 1 | 1 | 13 |
| `language/expressions/call` | 41 | 42 | 50 | 49 | 1 |

The 13 object-method gains are parse-negative early-error cases, and
`language/expressions/call/trailing-comma.js` now passes in both variants.
The four `grammar-special-meth-ctor-async` class expression/declaration files
also pass in both variants, measured before and after twice each with stable
classifications.
The pinned WPT `dom/` before pair used revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the unmodified upstream
harness, and the 627-source selection hash
`9787de028efe5dc83f491ce2a818e2a549b4a07a236e6538c1e6165bb437b0b5`.
Both rounds had **115 zero-subtest SyntaxErrors**, including 76 in shared
`testdriver.js`; file totals were 2 pass / 251 fail / 30 timeout / 211 error /
133 unsupported. The before outcome hash was
`b040b4290a0fae08866dd2cc17895f483febf662b999f8742352e21261c37ad4`.

The after pair kept the same WPT revision, harness hash, 627-source selection
hash, viewport, and deadlines. Both rounds had **zero** zero-subtest
SyntaxErrors; all 115 previously blocked source files moved beyond parsing.
Their later classifications were 48 fail, 48 timeout, and 19 other error.
Overall file totals became **2 pass / 299 fail / 78 timeout / 115 error / 133
unsupported**; registered subtests changed from 83 pass / 2,672 fail to 86
pass / 2,854 fail. The after outcome hash, identical in both rounds, was
`2ab0eb54e6aa4814e170dc48eb7e5b454ee5000a344eca6c5576236053491393`.
The exact 115 moved paths and later diagnostics are recorded in the lane's
`.agents/js-lang/js7-wpt-moves.json` scratch artifact. The 76 shared
`testdriver.js` cases now reach 24 failures, 46 timeouts, and 6 other errors;
their execution/driver follow-up is routed to the owning lanes in
`.agents/js-lang/questions.md`.

The initial-base Windows workspace gate passed `cargo nextest run
--workspace --locked`: **1,502 tests passed, 3 skipped, none failed**. A
separate `cargo test --workspace --locked --doc` ran 19 doctest suites with
**2 passed and none failed** (the JS and HTML doctests). The parser crate's
focused gate passed **189/189** nextest tests and its separate **1/1** doctest.

The final-base `bc8aa45` Test262 sweep used the same pin, input hash and runner
hash as the initial gate. It measured 53,872 files and 102,918 variants before
and after. Before: **10,594 file pass / 33,012 fail / 16 timeout / 9,956
unsupported / 294 fixture skips**, with 20,377 passing variants and
classification hash
`2cc7eea5f4b32c9a4d88d2be5ccf97160f11493494b51f1cd7c9f3e3d2ff3ebb`.
After: **10,694 file pass / 32,906 fail / 17 timeout / 9,961 unsupported /
294 fixture skips**, with 20,572 passing variants. Both after sweeps had
identical per-file and per-variant classifications, hash
`5d9e472f8f37b3c1063f50e8dbfdceb0524a8caff7c8cb78e3fba7894a17c113`.
No previously passing variant regressed. The 100 newly passing files came from
failures; five previously failing files became unsupported and one timed out.
The three focused directory pairs were rerun twice on the final base and kept
the same counts shown above. The parser crate passed **194/194** nextest tests
and its separate **1/1** doctest on that base.

The final-base WPT `dom/` pair used the same pin, upstream harness, 627-source
selection, viewport and deadlines as the initial gate. Both before rounds had
**115 zero-subtest SyntaxErrors**, 76 from shared `testdriver.js`, and file
totals **2 pass / 251 fail / 30 timeout / 211 error / 133 unsupported**. The
before outcome hash was
`598344c1a1f182a28e8e7df181f1542f4c2d47a6320e2e791450ffc0d7c1a34d`.
Both after rounds had **zero** zero-subtest SyntaxErrors; all 115 sources moved
beyond parsing, with 48 later failures, 48 timeouts and 19 other errors. The
after file totals were **2 pass / 299 fail / 78 timeout / 115 error / 133
unsupported**, and the outcome hash was
`6d6c69bf22a1ee919645350b9e05778c83c08d399ebc5019b28b3bb7a934853b`.
Registered subtests changed from 85 pass / 2,670 fail to 88 pass / 2,852
fail. The exact 115 moved paths, unchanged from the initial-base pair, are in
`.agents/js-lang/js7-bc8-wpt-moved-files.md`; the matching JSON includes each
file's old and later diagnostics.

The final-base Windows workspace gate passed `cargo nextest run --workspace
--locked`: **1,609 passed, 3 skipped, none failed**. Its separate `cargo test
--workspace --locked --doc` completed 19 doctest suites with **2 passed and
none failed**. The win-001 build tree was refreshed once to `bc8aa45`, then
the parser was re-synced; no subsequent refresh altered the gate tree.

## JS-8 — parser split and lossless RegExp pattern input

Initial fixed base: `6de1d9348b8048336b2d3d8b5204398d4dca0123` on
`origin/dev`. The final re-gate base is
`8f2695d92b31962982bb9c389f4d3b4a784ba13a`, selected because dev
drift since the initial base touched `amazing-js`. The mechanical parser
split is rebased commit `c4360a0`. It moved unchanged method
bodies out of `parser.rs` into `parser_statements.rs`, `parser_functions.rs`,
and `parser_expressions.rs`; the parser and each new module remain below 4,000
lines. The frozen split host completed two full Test262 sweeps with identical
classification `5d9e472f8f37b3c1063f50e8dbfdceb0524a8caff7c8cb78e3fba7894a17c113`.
A fixed-base frozen before-host rerun produced the same classification,
confirming direct equality. The first before sweep had one deadline-sensitive
strict variant and is retained as a diagnostic artifact.

`amazing-regex` now offers `Regex::new_utf16(pattern: &[u16], flags: &str)` and
`Regex::pattern_utf16()`. A pattern's exact units are retained for matching and
equality. In non-Unicode mode, each unit is a separate atom; in `u` or `v`
mode, paired surrogates combine into a code point and lone surrogates remain
literal units. The older `Regex::new(&str, &str)` and `Flags::parse` continue to
reject `v`, preserving the JS binding's parse-error behavior until full Unicode
sets are implemented. The new UTF-16 constructor accepts `v` for ordinary
patterns and rejects unsupported set operations and string properties.

The pinned Test262 `built-ins/RegExp` pair selected 1,962 files. Before and
corrected after each repeated twice with the same classification:
**691 pass / 1,261 fail / 10 unsupported**, hash
`67daa8e1189616f6694a7a0853073ef2805dc31bc0d3e360e62ed00b356abc93`.
The regex crate passed **12/12** nextest tests with **0** doctests, and the JS
crate passed **194/194** nextest tests with **1/1** doctest on win-001.

On final fixed base `8f2695d`, the `built-ins/RegExp` before and after pairs
each repeated twice with **693 pass / 1,259 fail / 10 unsupported** across
1,962 selected files and identical classification hash
`c440abf372a2f1abd18aca90e59763d0729cf49769d173d4e6632675bfc6e9b5`.
There were no passing file or variant losses. The two owned crates passed
**209/209** combined nextest tests (197 JS, 12 regex), with **1/1** JS
doctest and zero regex doctests. The lossless regex change is in the
separate second JS-8 commit.

The final-base full Test262 before and after sweeps used upstream pin
`3655e7464de3d52643ecddd4b5f9f4f3e7f62398`, the same 53,872-file input
hash `a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`,
and runner hash `bdb36351facc0475e661bea7ef928ebcc63e06e77d0a2740d897d77b3476d595`.
Both completed with **10,965 pass / 32,636 fail / 16 timeout / 9,961
unsupported / 294 fixture skips**, and **21,114 passing variants**. Their
classification hash was identical,
`bfdcb4a0cc96fa1eafe5ac14af911860d6c00b741be5426917a228d5ae0382c5`;
every file and variant classification matched, with no pass losses. The
complete JSONs are in the lane's `.agents/js-lang/` scratch area as
`js8-newbase-full-before.json` and `js8-newbase-full-after.json`.

The final fixed-base Windows workspace gate passed `cargo nextest run
--workspace --locked`: **1,709 passed, 3 skipped, none failed**. A separate
`cargo test --workspace --locked --doc` completed **21 doctest suites**, with
**2 passed and none failed**. The re-gate used the js-lang build tree refreshed
to `8f2695d` (Cargo.lock hash `BF3037D0`) and re-synced lane files; no later
refresh changed it. The final drift check against local `origin/dev` at
`582a6af` had no changes under `crates/amazing-js` or `crates/amazing-regex`,
so this fixed-base gate stands and the integrator will gate the merged head.

## JS-9 — RegExp UnicodeSets (`v`)

Initial base: `d8a228dc73eb5f83c9dd4bb95fd7c6b1b17cb79d` on
`origin/dev`. The JS-8 parser split and lossless pattern commits were rebased
onto it as `e8a54df` and `bfd52a8`; they had not yet appeared on `origin/dev`
when JS-9 began. The final fixed base is
`2c1a3e82160333425ce10e97d2a84d4e550bc26b`, after the required drift
audit found that JS-8 and JSVM-1 had changed the owned crates. JS-9 is commit
`3fda674` on that base. The win-001 js-lang tree was refreshed to each base
and lane-owned files were re-synced before the corresponding gate.

`amazing-regex` now accepts `v` through both constructors and parses nested
UnicodeSets classes, union, `&&`, `--`, character ranges and `\q{...}` class
strings. The VM tries longer string members first and preserves backtracking
when a later atom fails. The pinned Unicode 17.0 emoji data supplies all six
properties of strings and their `RGI_Emoji` union; `ASCII_Hex_Digit` is also
available. Regex input and pattern units remain lossless UTF-16.

The focused pinned Test262 selection includes both `built-ins/RegExp/unicodeSets`
and `built-ins/RegExp/prototype/unicodeSets`: 152 files. The frozen BEFORE host
passed **29** and failed **123** in two identical runs, classification
`1b992fe993d8ce61efdf5431f241ae2a4a5908d2449c9ca307b1825a6e09f0c6`.
The frozen AFTER host passed **143** and failed **9** in two identical runs,
classification
`102e90a74ba00165cb2ab819ad7c24711a8f40fe2b2c63481dc1efbc56ba1d9e`.
All 114 files in `built-ins/RegExp/unicodeSets` moved from fail to pass;
there were zero passing file or variant losses. The nine remaining failures
are in the runtime-owned `RegExp.prototype.unicodeSets` getter and constructor
surface; the exact work is filed to js-runtime in the lane questions. The
focused pair was rerun twice on final base `2c1a3e8` with the same before and
after counts, classifications and 114-file gain.

The initial-base full Test262 BEFORE sweep used upstream pin
`3655e7464de3d52643ecddd4b5f9f4f3e7f62398`, input hash
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`
and runner hash
`bdb36351facc0475e661bea7ef928ebcc63e06e77d0a2740d897d77b3476d595`.
Its frozen host hash was
`b7aa1e7b82687f7f03d5343bbf93313c0147d6e4f5e47fbcdba24c2d610103eb`;
it measured **10,965 pass / 32,636 fail / 16 timeout / 9,961 unsupported /
294 fixture skips**, classification
`bfdcb4a0cc96fa1eafe5ac14af911860d6c00b741be5426917a228d5ae0382c5`.
The final-base full BEFORE sweep repeated these counts and classification
exactly, with frozen host hash
`43e7d093b7e025282487c50a59ef608b8826efa5beaffb0c0fe20d435d88d424`.

The first final-base full AFTER sweep exposed seven formerly passing `/u`
negative tests for properties of strings. Commit `e657e96` restores the
required `v`-only guard; the targeted 28-file property selection is now
**28/28** pass, including all seven negative `/u` cases. The final-base
UnicodeSets pair was repeated after the fix and retained the same **29 →
143** counts and classification hashes above. The corrected full AFTER sweep
measured **11,086 pass / 32,515 fail / 16 timeout / 9,961 unsupported / 294
fixture skips**, classification
`1a2cb758320cebf9fe1f53e18d822cbda17a54de7336eb00076fb8f1fa82e8df`.
Compared with the final-base BEFORE sweep, 121 files and 242 variants moved
to pass, with zero passing file or variant losses. The gains are the 114
UnicodeSets files and seven property-of-strings files.

The two owned crates passed **210/210** nextest tests on the initial base and
**222/222** on the final base on win-001. The separate doctest runs passed
**1/1** for amazing-js and **0/0** for amazing-regex on both bases. The
final-base Windows workspace gate passed **1,739/1,739** nextest tests with
three skips; 21 separate doctest suites completed with **2 passed, 0 failed**.
The final drift check found no changes under `crates/amazing-js` or
`crates/amazing-regex` between fixed base `2c1a3e8` and `origin/dev` at
`b42383a`, so the fixed-base gate stands. The integrator's merged-head gate
will prove the combination.
## JS-10 part 1 — front end and BigInt split

Part 1 is the front-end-only series on fixed base
`c5192e6364a96439df52d843c749536ebb8c6e04`. It adds normalized
`ExprKind::BigInt(String)` literals, script and module grammar/early errors,
cover grammar handling, and the additive module AST. The module runtime,
host capability, and Test262 module runner are in the follow-on series.
The bytecode compiler's sole new fallback is
`PropValue::CoverInitializedName { .. }` (atomic interpreter fallback).

Full pinned Test262 (`3655e7464de3d52643ecddd4b5f9f4f3e7f62398`),
53,872 sources, identical runner and input hashes before/after:

| | Before | After |
| --- | ---: | ---: |
| Passing files | 21,249 | 22,729 |
| Passing variants | 41,438 | 44,030 |
| Classification SHA-256 | `a6e46a79f7743553dcc1c7b301f2617643fa405257bdc38738d6f0688a27dbef` | `7c5e39249ce68b6bac6b3356cd857d43cf726e1a04d6df9e9264df34169fda8c` |

There are 1,480 fail-to-pass file and 2,592 variant transitions and **zero
pass-to-nonpass transitions**. The before and after records are
`/tmp/js10-part1-base-c5192e6.json` and
`/tmp/js10-part1-after-82be02e.json`. Windows win-002: amazing-js nextest
256/256 (1 skipped); workspace nextest 2,057/2,057 (5 skipped); separate
crate and workspace doctest commands passed.

The module syntax contract exposed to the follow-on linker is
`ast::Module { items, span, has_top_level_await }`, with
`ModuleItem::{Statement, Import, ExportNamed, ExportAll,
ExportDefaultExpression, ExportDefaultDeclaration}`;
`ImportDeclaration { from, phase, bindings, attributes }`,
`ImportBinding { imported, local }`, `ExportName { local, exported }`,
and `ImportAttribute { key, value }`. `parse_module` is publicly exported.

## JS-10 part 2 — synchronous ES modules

Fixed base `7c61b9fcc33078685d2e552e648f2e486f93492b`, module code
`af57d53a8e2cf8883ece77e79f0ccb2f166ad7a4`. `module::ModuleLinker` exposes
`link` and `evaluate` with a host `ModuleResolver`; imported bindings alias the
exporter's cells, preserving live updates and TDZ through cycles. Module namespace
objects use the `Exotic::ModuleNamespace` property path. The Test262 host and
runner now execute synchronous module tests; dynamic `import()` and top-level
await evaluation remain with js-vm.

Pinned Test262 `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`, 53,872 sources,
same input hash and 60-second timeout on venus-001: before **24,762 files /
48,074 variants pass** (classification `a07a0061664bc09ba404c5358a9d66fbd249b3a115feb50ee67d21a567ce3421`),
after **25,057 files / 48,370 variants pass** (classification
`dd94095047cc6b5efb7088b561bad982cf7d7fb19cd7d13d8ef02ed07518a84d`).
There are **295 file and 296 variant gains, zero passing losses**. Of 843 module
variants, 295 pass, 286 fail, and 262 are explicitly unsupported for async
evaluation. The only classic classification difference is a pre-existing long
TypedArray test's strict variant moving from 60-second timeout to pass; all
other classic status/reason pairs are identical. Windows win-002: amazing-js
nextest **273 passed, 1 skipped**, workspace nextest **2,154 passed**, separate
JS and workspace doctests passed. No new VM fallback arms were needed in this
series.

## JS-10 front-end follow-up

Fixed base `5860d66` plus the module binding-cell compatibility fix
`c775d02`; front-end commit `8cd37e6`. The pinned full Test262 pair ran
53,872 sources with identical input and runner hashes: 25,295 to 25,371
passing files and 48,824 to 48,954 passing variants. There were 76 file and
130 variant gains with zero passing losses. Windows win-002 passed 292 JS
tests and 2,226 workspace tests; separate JS and workspace doctests passed.
The fixed-base classification hashes are `4c618ad3` before and `4446e075`
after. The changes tighten early errors, `super` and `await` contexts, lexical
declaration placement, and Annex B HTML comment parsing.

## Test262 host capability consistency

Fixed local `dev` base `ef925d7`; commit `d351fd47`. Both
`--capabilities` and `--capabilities-v2` serialize one `Capabilities`
value. Protocol 1 alone masks `module`; all shared flags, including
`intl402`, are set once. The emitted v1 and v2 JSON lines are byte-identical
to the fixed base. Pinned Test262 `intl402/` selection: 805 passing files and
1,610 passing variants before and after, zero losses, identical classification
SHA-256 `f9bcf0576e6247ca2fc2cae6b3e7a3ff5ddd875a2f0605ea8c8b27f24583dd69`.
The full-tree pair on earlier base `2cc7347` retained 720 Intl passes and
had no passing losses at 60 seconds; four staging variants moved out of
timeouts. The 120-second pair had one staging string timeout only; that test's
two variants both passed when selected alone. Win-002 ran 295 JS and 2,279
workspace nextest cases; both commands and separate JS/workspace doctests
passed.

## JS-11 — class methods, binding patterns, and async iteration

Code head `f9a7eba640d45f23eece100347b0ca22a841b898`, frozen JS base
`59492359e4a267885e8822f0f66941fa60720f9a`. The later scripts-only merge
`6e01e7b2` does not change engine inputs. Local commits only; no push.

The implementation adds private class elements and field initialization,
compiles method/generator bodies through the VM function path, binds default
and destructured parameters, advances array binding iterators lazily, and keeps
`for await` state in suspended VM frames with iterator closing. Merge corrections
preserve class method names/descriptors, inherited strictness and generator
function name descriptors. Direct eval (including parameter initializers) and
unsupported class elements retain atomic interpreter fallback.

Full pinned Test262 `3655e7464de3d52643ecddd4b5f9f4f3e7f62398` on
venus-001 Linux: 53,872 sources, 102,918 variants, release hosts, 10 workers,
60-second deadline, identical input and runner hashes before/after:

| Mode | Passing files before → after | Passing variants before → after | Lost passes |
| --- | ---: | ---: | ---: |
| Treewalk | 30,379 → 38,065 | 58,728 → 73,918 | 0 files, 0 variants |
| `AMAZING_JS_VM=1` | 30,344 → 38,047 | 58,687 → 73,885 | 0 files, 0 variants |

Treewalk target directories: `statements/class` 1,127 → 3,625 passes;
`expressions/class` 951 → 3,384; `for-await-of` 94 → 893. The first merged
sweep exposed 16 file / 19 variant losses in eval scope, strict method execution,
and function name descriptors; all were fixed and all 16 files / 19 variants
passed a focused replay before the corrected full zero-loss sweeps.

Final Windows gate on **win-001**, explicitly authorized as the win-002 outage
fallback: owned nextest **331 passed, 1 skipped** (318 JS, 13 regex); separate
owned doctests **1 passed**. Full workspace nextest completed with
**2,447 passed, 2 failed, 7 skipped**. The two failures are the known compositor
`paint_view_owner_mapping_binds_dom_properties_to_tree_nodes` and
`retained_real_page_property_trace` base failures, fixed in local dev `fba3b71`
and explicitly accepted for frozen-base reporting by the user. Separate workspace
doctests completed **28 suites, 2 passed, 0 failed**. All 574 Rust sources,
Cargo manifests and lockfiles checked on Windows match the exact gated commit.
This is not a claim of an all-green workspace.

Machine-readable provenance, full classification and binary hashes, per-directory
counts and zero-loss comparisons are in [js-lang_JS11-evidence.json](js-lang_JS11-evidence.json).
Full records remain `/tmp/js11-final-before-{tree,vm}.json` and
`/tmp/js11-corrected-after-{tree,vm}.json`, mirrored under `~/amazing/out` on
venus-001. Logs and the 16-file replay are in lane scratch. Remaining target
nonpasses include iterator error identity, class element edge cases and async
job ordering; these measurements do not claim complete class or async conformance.

## JS-12 — bounded character-class folding

Fixed base `063a0105b018735550b64c844f55f174cd3be163`. The JSB-6 Intl helper
blocker reproduces with 32 consecutive `[a-z]` classes under ignore-case flags:
the compiler charges a complete Unicode folding-table scan for each class and
exhausts its instruction/work budget. A shared sorted index of the same table
endpoints lets each range visit only relevant entries. Class members, range
lookups and visited endpoints remain budgeted; no resource limit is raised.

The regression covers legacy/Unicode/UnicodeSets class sequences, non-ASCII
folding, inversion, predefined classes, lookaheads, and rejection of expensive
wide-range sequences. Code `7e7edf84`, Linux gate head `1de643c8` (the merge changes only gate
instructions/scripts). Boat owned nextest: **345 passed, 1 skipped**; owned
doctests: **1 passed**. Workspace: **2,530 passed, 10 skipped**; separate
workspace doctests: **31 suites, 2 passed, 0 failed**. Windows is deferred.

Full pinned Test262 (53,872 files / 102,918 variants, 4 workers, 60-second
variant deadline): **39,938 → 39,953 passing files**, **77,661 → 77,691 passing
variants**, **zero passing file or variant losses**. All 33 files previously
blocked by class-compilation limits advance; 15 now pass and 18 expose other
failures. The input and runner hashes match. Baseline/after classification
hashes are `dc657e09` / `57200b23`. Evidence and exact hashes are in
[js-lang_JS12-evidence.json](js-lang_JS12-evidence.json); compressed full
records and gate logs are retained in lane scratch.

Next: the measured frontier contains 64 front-end failures, including 15 missed
parse-negative tests. JS-13 handles parser context, strictness and template
escape errors; decorator and Annex B RegExp clusters remain separate areas.

## JS-13 — callable grammar contexts and early errors

Parser source `e0e9d64a`, fixed base `bbe7bec9`, integration repair `1de87f21`. Function and method
parameter/body parsing now replaces enclosing Yield/Await/control contexts;
function-body directives apply strict early errors during parsing. Assignment
expressions own Yield parsing, argument expressions permit `in`, and contextual
`await using`, `let yield`, labels and bare `async` for-of heads follow their
productions. Tagged templates preserve invalid cooked entries through
`TemplateLit.cooked_valid`; untagged templates reject them. Runtime consumers
must turn invalid tagged cooked entries into `undefined`.

Owned Linux nextest: **348 passed, 1 skipped**; doctests: **1 passed**.
Full pinned Test262, 53,872 files / 102,918 variants: **39,953 → 39,999 passing
files**, **77,691 → 77,762 passing variants**, **zero passing file or variant
losses**. Input/runner hashes match; four workers and 60-second deadline.
Of the 64 measured front-end failures, 32 now pass and four advance to runtime
failures; remaining parser failures are 20 decorators and eight RegExp cases.
The runtime merge activated the async-body VM bridge; repair `1de87f21`
preserves the original loop-body throw across iterator return-method failures,
including rejected awaits. Twenty backend/case combinations cover throw/return
completion precedence.

The retained pooled target was stale and its 2,740-test result was discarded.
Final joint Linux gate at source `04f89683` (also includes JS-14): **2,750 passed,
10 skipped**, **31 doctest suites / 4 passed**. Full Test262 from the parser-only
head to this merged source: **39,999 → 40,132 files**, **77,762 → 77,958 variants**,
zero losses. Those additional gains include incoming runtime and handler work;
the parser-only improvement remains the 46 files / 71 variants reported above.
Evidence: [js-lang_JS13-evidence.json](js-lang_JS13-evidence.json).

Next, user-routed JS-14: public event-handler FunctionBody compiler with live
object environments ordered document → optional form → element. The caller
keeps lazy compilation, event registration and forwarding. RegExp Annex B and
named-capture parsing remain queued after this cross-lane dependency.


## JS-14 — public event-handler compilation and live object environments

Source `04f89683` exposes `Interpreter::compile_event_handler(name, parameters,
body, scope_chain)` and `parser::parse_event_handler`. The body has an independent
FunctionBody goal; scope objects are supplied outermost first (document, optional
form, element). The function is nonconstructible, uses its own `this`, and starts
non-strict unless its directive prologue enables strictness. Window handlers use
an empty scope slice; Window onerror accepts its five named parameters.

Live object lookup, accessors, unscopables, nested closures, identifier-call
receivers, deletion and setter-only assignments work through interpreter and VM
paths. Direct parsing prevents wrapper injection and rejects hashbang prefixes.
DOM retains lazy compilation, listener registration and event-return semantics.
The runtime has no per-function ScriptOrModule field or dynamic import execution;
the required null provenance is documented for that future implementation.

Final Linux source `04f89683`, fresh dedicated target: **2,750 workspace passed,
10 skipped**, **369 owned passed / 1 skipped**, **4 workspace doctests** (1 owned).
Pinned full Test262: **40,096 → 40,132 passing files**, **77,919 → 77,958 variants**,
**zero lost file or variant passes**. This comparison also includes the separately
committed async-close repair and deadline recoveries, listed in the evidence;
it is not an isolated count of new HTML API cases.

[Interface](js-lang_JS14-interface.md) · [Evidence](js-lang_JS14-evidence.json).
Next: Annex B decimal/octal and identity escapes, legacy class-range fallbacks,
and quantified lookahead/group boundaries; then Unicode property escape tables.

## JS-15 — Annex B RegExp grammar

Source `bcdd8fa0` implements decimal backreference versus legacy octal selection,
incomplete hex and control escapes, identity escapes, legacy class-range unions,
and quantified lookahead with noncapturing-group Atom boundaries. Capture counts
include forward references and exclude escaped parentheses and class contents.

Linux: **2,753 workspace passed / 10 skipped**, **372 owned passed / 1 skipped**,
**4 doctests** (1 owned). Full pinned Test262: **40,132 → 40,146 files**,
**77,958 → 77,986 variants**, **zero lost passes**. All 14 file gains belong to
the targeted RegExp grammar area. [Evidence](js-lang_JS15-evidence.json).

Next: Unicode property escapes, exact aliases, categories/scripts/extensions,
and Unicode mode complement/case-fold semantics.

## JS-20 — immutable Object.prototype and legacy __proto__

Fixed base `2eb1229a`. `default_set_prototype` now preserves every realm's
immutable Object.prototype and the module namespace's fixed prototype. Its
ordinary cycle check stops at nonordinary GetPrototypeOf methods without
calling Proxy traps, as required by OrdinarySetPrototypeOf. Previously a
self-returning trap could hang the walk; mutating Object.prototype underneath a
Proxy could also make a handler inherit its own Proxy and recursively look up
traps. Neither path is reached after the fix.

Object.prototype.__proto__ is installed as a configurable, nonenumerable
accessor, with call-only getter/setter functions and lengths 0/1. The getter
boxes primitives then delegates GetPrototypeOf; the setter requires a coercible
receiver, ignores invalid prototypes and primitive receivers, then delegates
SetPrototypeOf and throws if it returns false. Both interpreter backends cover
proxy boundaries, same-prototype nonextensible writes, primitive boxing, nullish
receivers, trap refusal and descriptor metadata.

Reference algorithms: [OrdinarySetPrototypeOf](https://tc39.es/ecma262/2024/multipage/ordinary-and-exotic-objects-behaviours.html#sec-ordinarysetprototypeof)
and [Object.prototype.__proto__](https://tc39.es/ecma262/multipage/fundamental-objects.html#sec-object.prototype.__proto__). No upstream code ported.

Next: dynamic-import host loading must release the ModuleLinker borrow before
executing module code and draining jobs. Queuing the hook alone cannot solve
reentrancy: ordinary run_program and top-level-await evaluation both drain the
queue inside evaluate's borrow. A prepared-step evaluator is the likely seam.

Linux Boat gates: **471 owned passed / 1 skipped**, **1 owned doctest**;
**3517 workspace passed / 17 skipped**, **31 doctest suites / 4 passed**.
Full pinned Test262: **49679 → 49727 passing files**, **95710 → 95801 passing
variants**, **zero lost passing files or variants**. The full sweep used a
10-second deadline; its six file / sixteen variant deadline flips all recovered
in 60-second whole-directory decodeURI/decodeURIComponent/Date replays with
unchanged input, runner and host hashes. Of the 48 file / 91 variant gains,
**44 files / 87 variants** are feature gains and four Date files/variants are
additional baseline deadline recoveries. All **15/15 __proto__** files pass;
the accepted mutable-proto-special-form loss is recovered. Raw and combined
classification hashes, exact provenance and gain lists are retained in
[js-lang_JS20-evidence.json](js-lang_JS20-evidence.json).

## JS-21 — queued module loading and a reentrant shared registry

Fixed base `6a81bf73`. The host uses `ModuleLinker::evaluate_shared` to release
all registry RefCell borrows before running JS or draining its jobs; queuing
import alone would still reenter the earlier evaluator's live borrow. New module
scopes retain the original realm global rather than inheriting the running
importer's lexical bindings, and failed evaluations preserve their thrown
object without running the body again. Canonical root identity unifies static,
dynamic and self-import paths in the Test262 module host.

A queued ModuleImport job captures the importing realm and typed loader, then
resolves the namespace through the existing promise thenable machinery. The
string-error loader API remains compatible; an additive typed hook preserves
module syntax/resolution/runtime errors. Import arguments execute in order;
ToString and attribute-validation failures reject the promise. Empty attributes
work; nonempty attributes remain unsupported by this host. Five new regressions
cover nested imports, namespace identity/live cells, scope isolation, cached
errors, deferred order, namespace then exports and realm restoration.

Initial Boat owned gate: **476 passed / 1 skipped**, **1 doctest**. Focused
Test262 dynamic-import **661 → 670**, module-code **543 → 564**, zero lost
passing files or variants. Final Boat workspace: **3522 passed / 17 skipped**,
**31 doctest suites / 4 passed**. Full pinned Test262: **49727 → 49758 passing
files**, **95801 → 95841 passing variants**, **31 file / 40 variant gains**,
**zero lost passing files or variants**. The raw 10-second sweep's eleven file /
twenty-one variant deadline flips recovered in whole-directory 60-second
decodeURI, decodeURIComponent, Date and TypedArray replays with identical input,
runner and host hashes. Source and target were copied to native `/tmp` storage
for that replay; no engine edits occurred between runs. Exact classification
hashes, gain paths and replay provenance are in
[js-lang_JS21-evidence.json](js-lang_JS21-evidence.json).

The classic-script wire protocol still has no filename/root. Its fixture
resolution needs a separately approved additive source-context extension;
legacy classic framing is unchanged. General async-module sibling concurrency
and in-progress async import graphs remain beyond this synchronous evaluator.

## JS-22 — optional-chain references and browser source metadata

Fixed base `680822e95c355415bd0541f7caf57780af95dbb1` after clean JS-21 merged
ancestry. Ordinary calls and optional chains now share a reference evaluator:
a short circuit stays distinct from an actual `undefined` property value,
propagates across the remaining ungrouped links, skips computed keys/arguments,
and terminates at parentheses. References preserve method receivers through
parentheses and tagged calls; delete acts on the final property without reading
its getter, returns true on a short circuit and throws on a grouped nullish base.
The VM owner granted exactly one `Paren` discovery arm in its existing whole-chain
TreeEval dispatch; no AST variant, direct-eval/FDI or Promise implementation edits.

The chain/receiver/delete control flow is adapted as a whole from MIT
`boa-dev/boa`, `core/engine/src/bytecompiler/mod.rs`, commit
`39cd11214ecb366200157dab96b3919e61e9a391`, functions
`compile_optional_preserve_this`, `compile_optional_delete` and their link
operations. Its VM jumps/registers map onto a tree-walker reference/short-circuit
result; all property, private-name, call, realm and delete operations use Amazing's
own gateways. License verified at the pinned commit; header and appended notice
are in the same optional-chain commit. Specification:
[Optional Chains](https://tc39.es/ecma262/multipage/ecmascript-language-expressions.html#sec-optional-chains).

Browser engine APIs are a separate commit: immutable source locator metadata on
EnvHandle shares the same live binding Rc, inherited by child/captured scopes.
`run_script_with_source_context` and its backend variant preserve delayed import
referrers after other classic scripts run; ordinary run_script retains an empty
locator. The parent also granted `ModuleResolver::source_url(id)->Option<String>`
(default None), allowing opt-in browser import.meta.url for roots/dependencies
while unique inline registry IDs stay separate and Test262 metadata stays unchanged.
Driver adoption remains with integrator-assist; classic Test262 framing is unchanged.

Four new regressions (including both backends) cover skipped work, grouping,
receivers and final-getter-free deletion, delayed closure origins with shared
global bindings, failed-script restoration, and opt-in URL/inline record identity.
One shared final Boat gate for the API and optional-chain commits: owned
**482 → 486 passed / one skipped**, **one doctest**; workspace **3551 passed /
17 skipped**, **31 doctest suites / four passed**. Whole language including
Annex B (24590 selected files), matching 60-second deadlines: **23353 → 23363
passing files**, **43461 → 43481 passing variants**, **10 file / 20 variant
gains**, **zero losses**, **zero timeouts**. Input and runner hashes match;
exact host/classification hashes and gain lists are in
[js-lang_JS22-evidence.json](js-lang_JS22-evidence.json). The API dependency is
`f86b814d6f74e9ffc1eff29dece4d8f6456da539`. The integrator explicitly owns the
combined full-tree Test262 gate; duplicate lane full sweeps were cancelled before
completion. Source and target ran on native `/tmp` storage through the main wrapper.


## JS-23 — object method home objects and canonical module failures

Fixed dependency base `7e3c07ba` (JSVM-12 eager generator frames included).
The initial `d20f78dd` base lacked eager generator initialization, so the parent
authorized adopting the existing owner fix. A fresh bulk then showed that eager
initialization alone did not retain the resumable call context: the VM owner
granted a narrow captured CallFrame/strictness field and resume push/restore
wrapper, with no FunctionCode.call or FDI edits. Object-literal ordinary,
getter/setter, generator and async methods receive their object as HomeObject,
exact UTF-16/symbol names, leading-required parameter length and standard
name/length descriptors. Arrows retain lexical HomeObject. Super references
carry both the home prototype and actual this receiver through property reads,
calls, assignment and updates; plain assignment avoids reading its getter.
Grouped references retain these semantics. Plain writes to ordinary, super and
private members evaluate base/key without reading the property; compound/update
references read explicitly. The VM owner granted one lexical home copy,
non-suspending super-reference TreeEval dispatch, and plain public-member
ResolveMemberWrite lowering. Suspended frames restore the original call context
and strictness on every resume/result path. Post-yield arrows and post-await
methods/new.target therefore keep their original context. Captured values use
strong Rc ownership; no tracing collector hook exists in this tree. Suspending
super property operations remain a separately coordinated VM follow-up.

Method-definition metadata operations are adapted from MIT `boa-dev/boa`,
`core/engine/src/bytecompiler/expression/object_literal.rs`, pinned commit
`39cd11214ecb366200157dab96b3919e61e9a391`. Upstream property-key naming and
SetHomeObject operations map onto Amazing's existing function factory and
property gateways. License verified at this commit; the new helper's header and
an appended third-party notice accompany the code.

The separate browser module API follow-up preserves canonical SyntaxError
object identity across direct and nested parse failures, distinguishes typed
parser/static-link errors from host-owned resolver errors, and rolls back all
records newly inserted by a failed link transaction. Completed earlier records
remain intact. Two module and three both-backend method regressions cover these
contracts. Embedding error conversion must prefer `failure.thrown`, as documented
in [js-lang_JS21-interface.md](js-lang_JS21-interface.md); browser and Test262
adapters remain owned by their respective integrators.

The first 60-second baseline was interrupted by the sandbox's finite TTL and
is incomplete evidence. Its replacement and the shared after gate use the same
fixed base and native `/tmp` source/target on the renewed Boat sandbox.

Final shared Linux Boat gate on fixed `7e3c07ba`: owned **489 → 494 passed /
one skipped**, **one doctest**; workspace **3594 passed / 17 skipped**, **four
doctests**. Whole language including Annex B (24590 selected files), matching
10-second deadlines: **23363 → 23445 passing files**, **43481 → 43641 passing
variants**, **82 file / 160 variant gains**, **zero losses**, **zero timeouts**.
Host/input/runner/classification provenance and exact gain lists are in
[js-lang_JS23-evidence.json](js-lang_JS23-evidence.json). The source-phase
lookahead dependency `600ce2a6` was committed after this native snapshot froze;
the root gates that separate minimal parser fix. The API's driver WPT follow-up
exposed a nondeterministic root-link error choice, tracked as JS-24; its failure
is preserved and the parent holds module-containing pushes until ordered graph
linking restores stable passes.


### JS-24 — ordered module graphs and root-specific linking failures

Port the ordered requested-module walks and ResolveExport from MIT Boa
`core/engine/src/module/source.rs` at `39cd11214ecb366200157dab96b3919e61e9a391`,
adapted to canonical string IDs and std collections in `module_graph.rs`.
Keep the public dependencies map as a lookup, with a private ordered request
list. Retain complete parsed graph records even when another node fails parsing.
Cache intrinsic parse errors by canonical node and linking errors by graph root;
never propagate a root's linking error into a dependency's intrinsic cache.
Validate and instantiate only reachable records, in ordered dependency postorder.
Resolve star exports by terminal binding origin: equal origins are unambiguous,
distinct origins are omitted from namespaces and rejected by named imports.
The parent gates the installed browser driver's whole 111-page module corpus,
including repeated cycle-root error choices; the lane gates whole language with
a fixed before/after source pair and workspace plus doctests on Boat.

JS24 source `a44bf171`, fixed before `df0cb8ed`, Linux Boat: owned
**526 passed / 1 skipped** (JS499, regex27), **1 doctest**; workspace
**3599 passed / 17 skipped**, **4 doctests**. Matching whole-language/
including AnnexB (24572 selected files), 10s deadlines, 8 workers:
**23428 → 23438 passing files**, **43607 → 43617 passing variants**,
**+10 / +10**, **zero losses**, **zero timeouts**. The initial workspace
controller SIGBUS after all test PASS lines is retained as incomplete; the
unchanged source repeat with native nextest completed successfully. Exact
hashes, selection and gains are in [js-lang_JS24-evidence.json](js-lang_JS24-evidence.json).
The integrator independently gates the installed111-page module corpus.


### JS-25 — contextual eval parsing

Add public `parser::EvalContext` / `parse_eval` for lexical super, new.target,
private names, strictness and class-initializer arguments restrictions. Keep
Script grammar controls independent of caller function kind. Restore ordinary
function/method lexical boundaries before parameter initializers as well as
bodies. Reject initializer arguments shorthand consistently with references.
Runtime owns native-eval context capture/execution after source locator34;
VM17 owns invocation identity and factory/caller metadata. No shared callee or
environment edits in this parser series. Interface and grouped source tests:
[js-lang_JS25-interface.md](js-lang_JS25-interface.md).

Parser dependency source `aefe3397`, fixed base `f878c84c` (executable source
identical to measured before `a44bf171`): owned**529/1skip +1doc**; workspace
**3602/17skip +4docs**. Matching whole language/ including AnnexB24572 selected
files,10s/8 workers: **23438 passing files /43617 passing variants unchanged**,
**zero losses/timeouts**. [js-lang_JS25-evidence.json](js-lang_JS25-evidence.json)
records exact hashes. API/grammar dependency is ready; runtime contextual eval
consumer follows source locator34 and retains ownership of callee/env metadata.


### JS-26 — decorators and class initialization

Port decorator grammar and evaluator algorithms from MIT Lumen
`f6c9860d44fa52249cfd647122edea184d32c863`, preserving source-ordered
expression evaluation, receivers, reverse application, replacement validation,
private access brands, auto-accessor storage and initializer order. Keep class-local
bindings uninitialized until decoration completes. Static initializers use the final
replacement as this and retain the original super HomeObject. Private methods
precede method extra initializers and fields. Field creation uses the existing
DefineOwnProperty gateway, preserving Proxy effects and frozen-object failures.
VM class fallback is atomic and suspension discovery visits all decorator vectors.

Source series `8255d80f` + `19052d3c`, fixed base `bd3bff1a`:
Linux workspace **3708 passed /17 skipped**, **four doctests**. Whole language/
including AnnexB 24572 selected files: **23803 → 23843 files**, **44234 → 44307
variants**, **+40/+73**, **zero lost files or variants**, **zero timeouts**.
All 20 decorator corpus paths pass. Thirteen grouped owned tests exercise both
backends and grammar/export boundaries. Details and provenance:
[js-lang_JS26-interface.md](js-lang_JS26-interface.md),
[js-lang_JS26-evidence.json](js-lang_JS26-evidence.json).

Next cluster analysis: strict unresolvable PutValue, reference capture before
assignment defaults/conversions, and destructuring target ordering. Native eval
context remains Runtime35; invocation/factory metadata remains VM17. No shared
execution hunks are changed before an explicit owner grant.

### JS-27 — reference capture, assignment and destructuring

Completed `ecfe581d` on coherent VM17 base `0d5963fe`. Captured declarative
frames/object records/unresolvable realm globals, deferred computed write-key
conversion, prepared destructuring targets before Get/default/iterator operations,
primitive receiver setters, numeric conversion ordering and postfix numeric values.
Linux workspace **3759/17 skipped +4 docs**; owned **575 passed**, eight grouped
regressions. Whole language/ **23865→23982 files (+117)** and **44351→44516
variants (+165)**, **zero losses/timeouts**.
[js-lang_JS27-interface.md](js-lang_JS27-interface.md).

Next JS28 audit: whole module import phases and typed attributes, faithful deferred
namespace triggers and graph evaluation. Source/defer argument effects currently
short-circuit; static phase/attributes currently ignored. Exact shared property/value
hooks and host adoption need parent/owner grants; Runtime35 retains contextual eval.


### JS-28 — typed module requests, asynchronous SCC evaluation and deferred namespaces

Completed source series `d9449fa7` + `52bacee5` + `8c4f1b8e` + `da8b0498`,
matched fixed JS27 dependency with integrator host14 on both sides. Additive phase
and attribute loader contract, genuine host source/synthetic records, source-only
parse boundary, ordered asynchronous siblings/SCC ancestors, cached cycle-root
errors, faithful deferred readiness and namespace triggers, internal asynchronous
completion observer, active coroutine async disposal and standalone/external
legacy checkpoint compatibility. No driver type support is claimed by the API.

Linux workspace **3777/17 skipped +4 docs**, explicit host **1**, owned JS/regex
**593/1 skipped**, eighteen new grouped tests. Whole language **23982→24134 files
(+152)** and **44516→44721 variants (+205)**, zero losses. Whole Object3338/6662,
Reflect151/302 and Promise627/1248 unchanged, zero losses. Four existing RegExp
literal files/eight variant10s deadlines on both sides. Exact proof and API:
[js-lang_JS28-evidence.json](js-lang_JS28-evidence.json),
[js-lang_JS28-interface.md](js-lang_JS28-interface.md).

Next JS29: faithful callable name/length/property order metadata, lossless
symbol/UTF16 SetFunctionName and precise assignment NamedEvaluation. Parent and
VM owner granted narrow descriptor order and plain/logical identifier companion
emission; preserve raw reference evaluation order, FDI/factory/context metadata
and broad nonsuspending class fallback guards. Gate whole language/Function/
Object/Proxy and both-backend grouped metadata cases.
