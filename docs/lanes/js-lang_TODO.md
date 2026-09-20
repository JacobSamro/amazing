# JS language conformance decision map

Frontier: **JS-2 — RegExp literals and a std-only regex engine**, explicitly prioritized by the integrator because upstream WPT testharness.js cannot parse.

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
