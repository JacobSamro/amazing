# DOM bindings decision map — DOM-1

## Scope and decisions

DOM-1 is an experimental runner for classic-script HTML WPT tests. The
integrator explicitly permits the driver module under `crates/amazing/src/cmd/`
and one dispatch line in `main.rs`. No JS engine files are owned by this lane.

The 2026-09-20 scope amendment permits a minimal local harness shim solely to
prove plumbing while JS-2 implements regex. Scaffold output is always labeled
`suite: "scaffolding-only"` with `wpt_revision: null`. It earns **zero WPT
credit**. Known-passing and known-failing upstream gates remain mandatory.

## Runner design

```
amazing wpt-harness [--scaffold] <root> <relative-test.html> <width> <height> <timeout-ms>
```

- The supervisor creates a fresh Amazing worker process for each page. Its
  wall-clock deadline includes parsing, script execution and event-loop work,
  so a non-cooperative infinite JS loop cannot defeat the timeout.
- WPT mode requires checkout HEAD
  `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Every executed HTML/JS file is also
  compared byte-for-byte with that revision's git object. Local edits or a
  substituted harness cannot produce a passing WPT result.
- Load UTF-8 HTML through `amazing_html::parse`, retain the live arena DOM in
  `BrowserDomHost`, then execute classic scripts in document order using the
  real `Interpreter`. Root-relative and page-relative local scripts resolve
  within the supplied root. Install the viewport and document/window host,
  dispatch load events and pump engine timers/microtasks.
- Upstream provides `test`, `async_test`, assertions and lifecycle semantics.
  After its script runs, install native Rust result/completion callbacks and
  request `setup({output:false})`. The visual
  `/resources/testharnessreport.js` reporter is omitted in favor of these
  callbacks. The upstream harness itself is never rewritten.
- Native callbacks immediately stream escaped result records to the parent.
  Results observed before an exception or infinite loop survive. The parent
  kills and reaps timed-out workers. Output is bounded to 16 MiB.
- One JSON record reports the suite/revision, test, viewport, timeout, file
  classification and counts, harness status, and individually named subtest
  results/counts. WPT codes 0/1/2/3 map to pass/fail/timeout/notrun; code 4
  (precondition failure) and unknown codes map to subtest error.
- File-level errors/timeouts never fabricate subtests. A harness-complete file
  with any non-passing subtest is classified `fail`. Harness status 2 or the
  supervisor deadline gives file `timeout`; other harness errors give `error`.
  Exit zero requires a passing file. CLI usage errors also emit JSON.
- XML, generated worker/any.js variants, URL query/fragment expansion,
  async/defer/module scripts, WPT HTTP handlers, navigation, and full browser
  parsing/event scheduling are not implemented. Unsupported script modes
  are rejected explicitly. This is not a general `wpt run` replacement.

## Verification

All builds and execution run on win-001, never macOS. Use the lane server tree
`C:\Users\server\src\wt\dom-bindings` and target
`C:\Users\server\target\dom-bindings`.

`crates/amazing/src/cmd/verify-wpt-harness.ps1` runs the scaffold fixtures twice,
asserts exact file and subtest classifications, validates JSON/provenance/exit
codes, and compares SHA-256 hashes of both JSONL artifacts. With `-WptRoot`,
it also runs the explicitly selected three-file `dom/nodes/` baseline twice.

## Pending gates

- [x] Finish scaffold verification and record measured artifacts below.
- [x] Execute unmodified pinned harness after JS-2; route subsequent engine or
  browser-host gaps through lane questions rather than adding substitute APIs.
- [x] Known-passing and known-failing **upstream** harness tests.
- [x] Repeated upstream proof classifications with stable hashes.
- [x] Full workspace green on measured base d8437f3; rebase refresh pending.

First follow-up: rerun the pinned CharacterData selection using the runner
against JS-2. The initial upstream script was rejected at line 581, column 43,
inside a regex literal, before any tests could register. Regex support may
expose further missing ECMAScript or host behavior; it is not evidence that
all remaining upstream requirements will work.

## Measured checkpoint — 2026-09-20

Rebased onto origin/dev `e9c1e21` before the final build. No commit or push was
made for this checkpoint. `cargo build -p amazing --locked` passed on win-001.
`cargo test -p amazing-dom --locked`: **7 passed, 0 failed**. The attempted
binary unit-test name filter discovered zero runner tests (the one-line
integration defines a local module), so the real subprocess verifier is the
runner evidence, not that empty unit-test run. Unreachable unit-test code was
removed. `git diff --check` passed. All authored files remain below 4,000 lines.

### Scaffold evidence — zero WPT credit

The verifier ran **14 cases twice**, with all 28 executions matching expected
file classifications, per-subtest counts, exit status and suite labeling.
Each run deliberately produced these file classifications:

| Pass | Fail | Timeout | Error | Unsupported |
|---:|---:|---:|---:|---:|
| 3 | 3 | 2 | 5 | 1 |

Each run observed 10 scaffold subtests: 6 pass, 1 fail, 1 timeout, 1 notrun,
1 error. Cases cover DOM mutation/viewport, async timer completion, assertion
failure, subtest timeout, an infinite loop after a passing result, uncaught
error after a passing result, escaped result names, all remaining status codes,
harness timeout/error, ignored data scripts, rejected module scripts, missing
script/harness and path escape. A zero viewport also produced a machine-readable
CLI error. The final check found **0 remaining harness worker processes**.

Both scaffold JSONL files SHA-256:
`F8DB03A6109855078313208AC15DC229B663729F9F8B7C2A970C46DAA5475519`.

### Pinned DOM smoke baseline

Revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Selection (exactly three files, not all of `dom/`):

- `dom/nodes/CharacterData-data.html`
- `dom/nodes/CharacterData-appendData.html`
- `dom/nodes/CharacterData-deleteData.html`

Viewport 800×600; timeout 10,000 ms; two runs. Each run reports:

| File pass | File fail | File timeout | File error | File unsupported | Registered subtests |
|---:|---:|---:|---:|---:|---:|
| 0 | 0 | 0 | 3 | 0 | 0 |

Every subtest count is zero. All three stop while parsing unmodified upstream
`/resources/testharness.js`: `SyntaxError: 581:43: unexpected character \`.
This measures harness-loading failures, **not DOM assertion failures**.
Before this ticket there was no testharness runner/baseline; no numerical
before/after conformance improvement is claimed.

Both DOM JSONL files SHA-256:
`67BE351C845AA1C332039311B085709C610C0498142D9BA68F700C00CB6C7DF5`.

Raw artifacts live under
`C:\Users\server\target\dom-bindings\diagnostics\dom-1\` and the
orchestration copy `.agents/dom-bindings/dom-1/` in the main local tree.
Reproduce on win-001 with:

```powershell
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\dom-bindings'
Set-Location 'C:\Users\server\src\wt\dom-bindings'
cargo build -p amazing --locked
# Invoke from a PowerShell process with -ExecutionPolicy Bypass.
& crates\amazing\src\cmd\verify-wpt-harness.ps1 `
  -Executable C:\Users\server\target\dom-bindings\debug\amazing.exe `
  -OutputDirectory C:\Users\server\target\dom-bindings\diagnostics\dom-1 `
  -WptRoot C:\Users\server\target\dom-bindings-wpt
```

DOM-1 remains **incomplete**. The independent plumbing proof is ready; the
upstream known-pass/known-fail proof awaits JS-2 and any subsequently exposed
host gaps. Full-workspace validation is deferred to that final gate, as the
brief reserves it for completion. No PR has been opened.

# DOM-2 — node algorithms, events, and mutation observation

DOM-1 is accepted as a scaffold checkpoint and **parked/open** by the
integrator. Its shim remains non-WPT evidence. DOM-2 adds a std-only model
without JS or GPUI coupling. Reference algorithms: [DOM Standard
§4.2.3](https://dom.spec.whatwg.org/#mutation-algorithms),
[§2.9](https://dom.spec.whatwg.org/#dispatching-events), and
[§4.3](https://dom.spec.whatwg.org/#mutation-observers), consulted 2026-09-20
(the published living-standard header was dated 2026-08-25).

## Node model

- `Dom` retains the arena; `NodeIdx` identity survives removal and adoption.
  Multiple document roots can occupy one arena. `create_document`,
  `create_node_in`, and `create_document_fragment` carry owner-document
  identity; adoption updates the entire subtree without copying it.
- `ensure_pre_insert_validity`, `pre_insert`, `pre_remove`, `replace`, and
  `replace_all` are the checked binding surface. Validation happens before
  any mutation. Internal insertion/removal apply observer suppression and
  fragment batching as specified. Errors retain distinct DOMException names.
- `DocumentType` is needed as a node: validity checks depend on doctype
  ordering, uniqueness, and replacement identity. The preexisting
  `Dom::doctype` name metadata cannot express those properties.
- `DocumentFragment` insertion moves its children in order and empties the
  fragment. Its removal record is emitted even when the target operation
  suppresses individual records. Empty fragments perform no insertion.
- Tree order and ancestry/sibling/connected predicates are available directly.
  `compare_document_position` supplies standard masks and a stable,
  implementation-specific arena-index order for disconnected trees.
- Existing parser attach/detach/attribute primitives retain their role and
  bypass DOM hierarchy validity; they now feed mutation observation and owner
  bookkeeping. They must not become the JS Node mutation entry points. Bulk
  parser attribute order remains unchanged.

## Event delivery without borrowing through a callback

`EventTargets` owns listener lists and event objects. Nodes and non-node host
objects use `EventTarget::Node` / `External`; callback identities are opaque
`CallbackId`s. No Rust or JS callbacks are stored in the DOM core.

1. Create an `Event` with flags and a monotonic timestamp relative to the
   driver's time origin, then store it to get an `EventId`.
2. `begin_dispatch` builds an owned path before callbacks. The `EventTree`
   trait supplies parents, retargeting and default-passive policy.
3. Call `next_listener` to receive an owned `EventInvocation`, release the
   registry borrow, invoke the binding callback, then resume. This allows
   tree/listener mutation and nested dispatches without aliasing the arena.
4. Callback exception reporting records the exception and permits dispatch to
   continue. `dispatch_result` returns false when cancellation occurred.
   An abandoned dispatch must use `abort_dispatch` to clear transient state.

Capture, both at-target groups, and bubbling use separate per-invocation
listener snapshots. Removals take effect against existing snapshots; new
listeners can run in later phases. Identity is type/callback/capture, so
re-adding does not replace once/passive options. Once listeners are removed
before invocation. Passive listeners cannot cancel defaults; ordinary
cancelable listeners can. Stop propagation and immediate stop have distinct
behavior. Completion clears phase/currentTarget/path/stop flags but retains
cancellation. Dispatch of an already-dispatching event fails. Script dispatch
sets isTrusted false; only the embedding's user-agent path sets it true.

DOM-2 implements light-tree dispatch. Composed-dependent parent and retarget
hooks are exercised with an external-target adapter. Actual shadow roots,
closed-tree composedPath filtering, related/touch-target retargeting, slots,
and HTML activation behaviors require the future corresponding host types;
they are not claimed as implemented here. A UA-trusted event alone must never
be treated as a user-activation permission by bindings.

## Mutation observation

- `create_mutation_observer` stores an opaque callback identity. `observe`
  validates presence-sensitive options, including implicit attributes/data
  enablement when old-value/filter options are present, even if false.
- Child-list, attributes and character-data records preserve mutation order,
  sibling context, namespace and requested old values. Overlapping ancestor
  registrations deliver one record per observer, taking the union of old-value
  requests. Attribute filters exclude non-null namespaces.
- Removal installs transient subtree registrations. Delivery removes them;
  disconnect and re-observe implement their corresponding cleanup rules.
- `mutation_microtask_pending` is the scheduling seam. The driver calls
  `begin_mutation_checkpoint` at its microtask checkpoint and resumes
  `next_mutation_delivery` until exhausted. The notify set is frozen, while
  each observer's records are drained only when its turn arrives. Callback A
  can therefore add records delivered to B later in the same checkpoint;
  mutations during delivery also queue the next checkpoint. The driver must
  not nest checkpoints. `take_mutation_records` does not cancel scheduling.
- No scheduler, JS types, Rc, RefCell, unsafe, or third-party crates are added.

## DOM-3 integration obligations

- Bind Node methods to checked operations, resolve callback identities, expose
  event flags/methods via `EventId`, resume dispatch after every callback, report
  callback exceptions, and schedule mutation notifications at real microtask
  checkpoints. Wire driver default actions to the cancellation result.
- Retire shell host implementations only in the separately authorized binding
  ticket. This ticket makes **no GPUI edits**. The integrator will add the new
  enum arms to `engine_viewport.rs` and `dom_runtime.rs` at merge.
- Arrange shared arenas for documents that can exchange adopted nodes. A raw
  NodeIdx from another Dom is not an adoptable identity. Existing HTML parsers
  still record a doctype name in `Dom::doctype`; the parser integration must
  create a correctly positioned DocumentType node when building a bindable
  document. Fragment parsing must target the destination document/arena.
- Attribute methods accept normalized namespace/local-name inputs. Element
  name validation/WebIDL conversion is a binding concern. Namespaced attributes
  have separate storage from the legacy parser's unqualified attribute vector.
- Existing CharacterData strings remain Rust UTF-8 strings. UTF-16 code-unit
  and lone-surrogate operations require a deliberate string-model extension
  before exposing the complete CharacterData WebIDL surface. No range/string
  slicing conformance is claimed by the full-value mutation setter.
- Live Range/NodeIterator adjustment, custom-element reactions, and Shadow DOM
  side effects must attach when those types exist. Current tree operations
  implement the core light-tree model, not those future subsystems.

## Cross-crate compatibility grant

Only new-variant match arms were added outside amazing-dom:

- `crates/amazing-html/src/tree.rs`: test tree dumper.
- `crates/amazing-layout/src/box_tree.rs`: four exhaustive layout matches;
  fragment/doctype nodes themselves produce no boxes.
- `crates/amazing/src/main.rs`: serialization and clone-insertion matches.

No CSS file required an edit; its existing matches already have fallbacks.
No existing arms or other code in those files were changed for DOM-2. The
DOM-1 dispatch line belongs to its separate checkpoint commit.

## DOM-2 final measured gate — 2026-09-20

Rebased immediately before gating to origin/dev **26c64de**. After the
integrator's server refresh, remeasured the base crate, re-synced the Mac
changes and verified **153** source/manifest/fixture SHA-256 hashes on win-001.
Pre-refresh results are not final gate evidence.

| Windows measurement | Pass | Fail | Ignored | Repeats |
|---|---:|---:|---:|---:|
| Base amazing-dom | 7 | 0 | 0 | 2 |
| DOM-2 amazing-dom | 65 | 0 | 0 | 2 |
| Full workspace, including doctests | 902 | 0 | 3 | 1 |

Commands: `cargo test -p amazing-dom --locked -- --test-threads=1` and
`cargo test --workspace --locked -- --test-threads=1`, with the lane's dedicated
Windows `CARGO_TARGET_DIR`. Both base and final runs have stable SHA-256 over
sorted `test ... ... ok/FAILED/ignored` lines (elapsed timings excluded):

- Before: `f751d8aca01d1ea708f3f7a603a41ff62e85fba52aa77a34a0437b6540aa9b6e`
- After: `45236395e4d9203355d569c9ae45d312c67fb026f07aa9db510b2a2d4f648f8f`

The three preexisting ignored tests require external HTTPS or real D3D11
hardware. They were not newly ignored. All executed workspace suites pass;
no separate sibling browser/WPT corpus was rerun or credited. **DOM WPT:
not measured.** The isolated GPUI workspace was not built or modified; its two
new-variant match sites remain the integrator's explicitly retained merge task.

Logs and machine-readable totals: main-tree orchestration directory
`.agents/dom-bindings/dom-2/`, `dom-2-before.log`,
`dom-2-source-hashes.json`, and `dom-2-validation.json`. Source modules and
tests are all below 4,000 lines. DOM-1's accepted scaffold is preserved in a
separate checkpoint commit; this ticket does not close its upstream gate.


## DOM-1 upstream gate and baseline — resumed after JS-2

Integration base: `d8437f3` (JS-2, LAYOUT-2, CSS-2 and RC-2 merged).
DOM-3 is parked on local `checkpoint/dom-3`; its unmerged code is absent from
this baseline. The integrator refreshed the Windows lane tree with current
mtimes and first-party artifact invalidation. Earlier gates do not substitute
for this fresh run.

Runner integration changes:

- Bind bare Window listener functions to their actual Window receiver; install
  the top-level `parent`/`top` aliases. Existing JS/native DOM machinery runs
  the callbacks; no harness assertions or outcomes are implemented in the runner.
- Disable visual output with the upstream `setup({output:false})` API. Keep
  default message callbacks: disabling them needs Array.splice, which the
  current engine lacks. This is runner configuration, not an upstream edit.
- Replace the visual reporter with native callbacks for both absolute and
  relative reporter URLs. All executed test and harness sources still require
  exact pinned git-object bytes.

`measure-wpt-dom.py` first runs two unmodified upstream self-tests twice:
`resources/test/tests/functional/order.html` (2 PASS), and
`resources/test/tests/functional/assert-array-equals.html` (2 PASS, 14 intentional
FAIL). It compares every subtest name/status against the upstream embedded
`expected` JSON, requires harness status OK, checks file status and exit code,
and requires repeat outcome hashes to agree. Assertion diagnostic strings are
retained but not asserted equal to upstream expectations: current JS string
replacement does not fully format the harness's messages.

The measured unit is **test source files, not expanded WPT URLs**. Inventory
comes from pinned tracked `dom/` paths: markup loading external testharness.js
and `.any.js`/`.window.js`/`.worker.js` sources, excluding resources/support.
Selection includes 627 sources; 192 other paths are excluded with recorded
reasons. Of those selected, 495 are eligible for execution. The other 132
require generated JS wrappers (64), XML/SVG parsing (55), URL variants (8),
server template substitution (3), or HTTPS origins (2). Unsupported sources
remain in the file denominator and have zero fabricated subtests. Script
scheduling restrictions can also produce runtime `unsupported` results.

Normal parent deadlines are 10000ms; `<meta name=timeout content=long>` selects
60000ms. Viewport is 800×600. Each file has a fresh process; three execute at a
time. JSONL retains each actual callback, status, name and message, including
callbacks completed before a later error or timeout. Registered-subtest totals
do not estimate tests that never registered. Outcome hashes include all file
and subtest identities/statuses/counts; raw JSONL hashes also retain diagnostics.

Reproduce on win-001 after building the matching runner:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\server\target\dom-bindings'
python crates/amazing/src/cmd/measure-wpt-dom.py `
  --executable C:/Users/server/target/dom-bindings/debug/amazing.exe `
  --wpt-root C:/Users/server/target/dom-bindings-wpt `
  --output C:/Users/server/target/dom-bindings/dom-1-upstream --jobs 3
```

Do not rebuild `amazing.exe` concurrently with this measurement: Windows locks
the running executable. Run workspace tests sequentially after the runner exits.
The upstream harness SHA-256 is
`d2399236c2a09c429804ff2299ad6629e17e2b53f17a74dd341e936adb11ae3e`.
Proof raw JSONL SHA-256 (both runs):
`fda888792ad20e12b69ab4909e69896b98916b170c7ecf259cd79854ebe404bd`.
Proof outcome SHA-256 (both runs):
`955a95d1fd7db2790c52d61aeddc9019c5840a4825a6cfb11722bb6e499409c2`.
Measured baseline on d8437f3, identical in both runs:

| Unit | Pass | Fail | Timeout | Error | Unsupported / notrun |
|---|---:|---:|---:|---:|---:|
| Source files | 2 | 241 | 30 | 221 | 133 unsupported |
| Registered subtests | 83 | 2584 | 0 | 0 | 0 notrun |

Two passing source files: `dom/events/event-disabled-dynamic.html` and
`dom/nodes/getElementsByClassName-empty-set.html`. Raw JSONL SHA-256 (both runs):
`3601e251023d0f34e0668c7e9b8d1b24df1fbca43391e0cff123ab51ee719f4c`.
Outcome SHA-256 (both runs):
`327f8456d54edf7b306759439ffda04763a1f6c0ffc705ebdb7d8bc2cf0aa4d4`.
Selection SHA-256:
`9787de028efe5dc83f491ce2a818e2a549b4a07a236e6538c1e6165bb437b0b5`.
Measured executable SHA-256:
`ccc9cef3f2b806f42b575d7594f9b49746c7e6890b164ae5a74f5a6078212dd9`.
159 Rust source/manifests/measurement-script files matched Mac. Full Windows
workspace: 992 passed, zero failed, three existing ignored across 52 targets.
A prior concurrent build hit Windows' executable lock before testing; the
successful workspace gate ran sequentially after measurement.

The earlier three CharacterData probes went from three harness-load errors and
zero registered subtests to three completed failing files: appendData 14 FAIL,
data 16 FAIL, deleteData 18 FAIL. This is measurement infrastructure progress,
not a claimed DOM implementation improvement.

The largest error group is 76 testdriver.js parse errors (2451:27, `subscribe`);
30 additional files fail in dom/common.js object-to-primitive conversion. These
are measured follow-up inputs for JS/binding work, not hidden exclusions.
Module/async/defer scheduling adds one runtime-unsupported source to the 132
inventory-unsupported sources. No overall expanded-WPT-URL pass rate is claimed.

Durable evidence lives in `crates/amazing/src/cmd/wpt_harness_results/`:
summary, one complete baseline JSONL (the second is byte-identical), upstream
proof JSONL, and selected/excluded source inventory JSONL. Full two-run files,
logs and original selection.json also live in `.agents/dom-bindings/dom-1-upstream/`
and `C:/Users/server/target/dom-bindings/dom-1-upstream/`; the latter includes
the preserved measured `amazing.exe`. Scaffolding remains separate and earns
zero WPT credit.
