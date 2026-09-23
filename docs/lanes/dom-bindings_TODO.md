# DOM bindings decision map

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

## DOM-1 exit gates

- [x] Finish scaffold verification and record measured artifacts below.
- [x] Execute unmodified pinned harness after JS-2; route subsequent engine or
  browser-host gaps through lane questions rather than adding substitute APIs.
- [x] Known-passing and known-failing **upstream** harness tests.
- [x] Repeated upstream proof classifications with stable hashes.
- [x] Final refreshed workspace green on base 0e298c1: 1017 passed, 0 failed, 3 ignored.

Historical blocker (resolved by JS-2 and the final upstream gate below):
the initial upstream script was rejected at line 581, column 43,
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

## Binding integration obligations (recorded after DOM-2)

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
- DOM-3 adds exact UTF-16 CharacterData access/replacement, with a private
  sidecar for lone surrogates. Bindings must use that surface and observer
  `old_value_utf16`; legacy Rust String fields are display projections.
- DOM-3 attaches live Range/NodeIterator adjustment to tree/data mutation.
  Custom-element reactions and Shadow DOM side effects still belong to their
  future subsystems; the current tree remains the core light-tree model.

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

Final integration base: `0e298c1` (including NET-4); initial measurement base
was `d8437f3` (JS-2, LAYOUT-2, CSS-2 and RC-2 merged).
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
Final measured baseline on 0e298c1, identical in both runs and unchanged from
the two earlier d8437f3 runs:

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
`6882b3534422bc3c91de071249134b99527e5a6df70dffe6536fe4e63155476c`.
188 source/manifest/fixture files matched Mac. First-party compilation is
recorded after the artifact-invalidating refresh. Final Windows workspace:
1017 passed, zero failed, three existing ignored across 52 targets.
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
logs and original selection.json also live in `.agents/dom-bindings/dom-1-upstream-final/`
and `C:/Users/server/target/dom-bindings/dom-1-upstream-final/`; the latter includes
the preserved measured `amazing.exe`. Scaffolding remains separate and earns
zero WPT credit.


Final scaffold regression is separate: 14 cases twice, stable
`F8DB03A6109855078313208AC15DC229B663729F9F8B7C2A970C46DAA5475519`,
zero WPT credit. Zero remaining lane workers were verified. DOM-1 is complete;
DOM-3 remains on its committed checkpoint for the next integration boundary.



Implementation follows [DOM Standard ranges](https://dom.spec.whatwg.org/#ranges),
[traversal](https://dom.spec.whatwg.org/#traversal), and the shared mutation,
replace-data, split-text, and normalize algorithms (reviewed 2026-09-20).

- `Range` is a non-reused arena-local live handle. `AbstractRange` is an owned
  boundary snapshot; `StaticRange` construction rejects forbidden node types
  but permits invalid offsets/order/roots, with a separate validity query.
- Insert/remove/adopt/replace/fragment operations update all live ranges;
  suppressed observer records do not suppress those updates. CharacterData
  replacement, splitText and normalize maintain UTF-16 boundary offsets.
- Content cloning/extraction keeps partial ancestors, clones partial data,
  moves whole contained nodes only during extraction, and checks contained
  doctypes before changing the source. Delete/extract collapse before mutations
  as specified. Surround preserves the specified late-error side effects.
- A private code-unit sidecar preserves ill-formed UTF-16 without changing the
  public NodeKind String payloads used by sibling crates. String access is a
  lossy compatibility projection only when lone surrogates exist. Exact data
  and observer old values are available to future DOMString bindings.
- NodeIterator and TreeWalker use owned `TraversalStep::Filter` requests with
  callback identity and node handles. Resume exactly once with the converted
  unsigned-short result; cancel native state before propagating a binding
  exception. No callback holds an arena borrow. The iterator's reference and
  in-flight candidate both adjust during pre-removal. TreeWalker currentNode
  is unrestricted and does not relocate automatically.
- `release_range`/`release_traversal` are explicit binding-GC seams; legacy
  detach methods are no-ops. Handles must never cross Dom arenas.
- No JS binding, custom-element/shadow-tree extension, WPT result, or broad
  browser conformance is claimed. DOM-1 is now merged as 156cc13; these native
  APIs are not yet exposed to script, so its baseline is not relabeled.

Fresh-artifact Windows gate on base 57f0e69: 107 DOM tests passed twice (65
preexisting plus 42 new), zero failed/ignored; canonical result hash
`f9f87bb833ccdf30425e279abc5372671a5190526bb97eaeec3739b63c63a8e9`.
Full workspace: 957 passed, zero failed, three existing ignored, twice;
canonical result hash `05385f87dfb95722b7af7ef06b03da37ba8662eadac4e066bd513ee656b267dd`.
144 Rust source/manifests matched Mac; logs confirm first-party recompilation
following the integrator's artifact-invalidation refresh. These gates completed
as the integrator redirected the lane to DOM-1; DOM-3 is checkpointed for return.


### DOM-3 resumption and XML namespace seam

Resumed the checkpoint on integrated DOM-1 (`156cc13`). The integrator added
the html lane's XML namespace request to DOM-3. `Namespace` keeps its existing
copyable Html/Svg/MathMl values, adding Null and Other(NamespaceId). Arbitrary
URI strings are interned in the Dom arena; equality compares namespace identity
within that arena, including across document adoption. NamespaceId, like NodeIdx,
is arena-local and must not cross arenas. No leaked/static URI allocation.

- `create_element_ns(parent, Option<&str>, qualified_name)` validates namespace
  and name, then uses checked insertion; returns Result<NodeIdx, DomError>.
- `create_element_ns_in(document, ...)` creates an unattached element for sinks
  and future bindings. Legacy HTML/parser constructors retain their behavior.
- `namespace_uri`, `prefix`, `local_name` (alias of name), and `qualified_name`
  expose the separate values. Empty URI normalizes to null. Known URI constants
  map exactly to existing enum values; other URI strings are case-sensitive.
- Prefix metadata survives deep cloning, range content cloning and adoption.
  Local names remain case-preserving NodeKind::Element::name values.
- Name validation follows the current [DOM validate-and-extract algorithm](https://dom.spec.whatwg.org/#validate-and-extract) in element context, reviewed
  2026-09-21. XML parsers enforce stricter QName syntax before this seam; the
  DOM API intentionally accepts some names XML syntax rejects.

Seven additional tests cover null/empty/custom/known namespaces, URI equality,
reserved prefixes, error ordering, DOM name validation, clone/adopt identity,
partially-contained range cloning and detached/checked construction. Current
refreshed Windows gates are pending. The one new exhaustive-match arm required
in amazing-html's scope check has been routed through questions.md.

Resumed gate on `156cc13` + lane `9229280`: 170 source/manifest hashes
matched Mac; 114 passed / 0 failed / 0 ignored twice. Canonical result hash
`0aa71774e90e39694ffc25b8b0e5e7a4aa78003f7a42cc998ed648222286ccc6`.
Workspace exited 101 during compilation: missing Null/Other arms in
`amazing-html/src/tree.rs:568`. Second workspace gate not run. DOM-3 is
not complete; sibling compatibility is routed through questions.md.


### DOM-3 final integrated gate — complete

Base `79464ca` (NET-5 and RC-3 included), measured lane head `d39904f`.
HTML owner commits `7ddf65e` and `a1f03f5` are included as `44aea24`/`d39904f`;
their combined delta is exactly +1/-0 in `amazing-html/src/tree.rs`, handling
Null/Other namespaces as non-boundaries. The DOM lane authored no HTML logic.
The earlier E0004 blocker is resolved.

After rebase, own-tree refresh used current mtimes and first-party artifact
invalidation (lock 3FEAF813), followed by source re-sync. All 177 Rust source
and manifest hashes matched Mac; fresh first-party compilation appears in logs.
All builds/tests ran on win-001, isolated dom-bindings tree and target.

| Gate (each run, twice) | Passed | Failed | Ignored | Targets |
|---|---:|---:|---:|---:|
| amazing-dom | 114 | 0 | 0 | 2 |
| Workspace | 1119 | 0 | 3 | 54 |

Native DOM coverage went from 65 existing tests to 114: 42 range/traversal
tests and seven namespace tests. Canonical test-name/status lines (sorted,
UTF-8 with LF endings, timing excluded) match between repeats:

- DOM: `0aa71774e90e39694ffc25b8b0e5e7a4aa78003f7a42cc998ed648222286ccc6`.
- Workspace: `cca4d3fb50e1f27e894278ff7fcacff01c7c6e257c1c49e3ddecf203592259f2`.

The initial PowerShell Start-Process wrapper stalled after Cargo exited; only
that wrapper was stopped and the gate restarted with direct Cargo invocation.
The final four direct-invocation gates all exited zero.
Evidence: `.agents/dom-bindings/dom-3-integration-results.json`,
`dom-3-final-source-manifest.json`, `dom-3-final-gate-direct.log`, and
`dom-3-final/` raw logs; corresponding Windows artifacts are under
`C:/Users/server/target/dom-bindings/`. No DOM-3 WPT improvement is claimed: these
range/traversal APIs still need JS bindings. DOM-1's measured baseline stands.

HTML follow-ups for qualified attribute prefixes and a parser-only expanded-name
constructor (e.g. valid XML element `xmlns`) are recorded by that owning lane
for a later assigned scope; create_element_ns retains DOM API validation.
DOM-3 is complete and stopped at its review boundary.

## DOM-4 — driver-stepped event loop (in progress)

Ticket branch: `lane/dom-bindings-dom4` only. Based on current dev after corrected
DOM-3 namespace merge c8c8264. DOM-3 sibling branches are not part of DOM-4.

Implementation is `amazing_dom::event_loop`, std-only and independent of JS:

- `EventLoop` owns task queues by `TaskSource`, a separate FIFO microtask queue,
  timer state and animation callback maps. Source selection is deterministic
  round-robin; each queue selects its oldest runnable task, bypassing inactive
  globals rather than allowing them to block active documents.
- The driver supplies monotonic `Duration` with `advance_to`, registers globals,
  and updates activity/renderability at the corresponding clock transition.
  Timer-active time excludes inactive document/suspended worker intervals.
- All callback payloads remain rooted in the host behind `CallbackId(u64)`.
  `step()` yields owned `Step { id, action }`; the host may enqueue/cancel work
  while executing it and must `complete(id)` once, including after reporting a
  callback exception. Reentrant stepping and stale completions are rejected.
- Task callbacks and each rAF callback lead into a guarded microtask checkpoint.
  It drains newly queued microtasks, then yields `CheckpointCleanup` for rejected
  promises, transaction cleanup and ClearKeptObjects. Nested checkpoint requests
  are no-ops. Mutation-observer notification is a microtask: enqueue its opaque
  token at the original queueing point, not after all Promise jobs have drained.
- `set_timer(global, callback, i32 delay_ms, repeat)` returns an opaque `TimerId`.
  Bindings own WebIDL conversion, callback this/arguments, string compilation and
  security checks. Negative delay becomes zero; nesting beyond five clamps to
  4ms. Interval initialization runs after the callback checkpoint, honors
  cancellation from its microtasks, and does not replay missed ticks. Timer IDs
  never reuse; clearing a timer invalidates an already queued task too.
- `request_animation_frame` is separate from timers. Driver rendering
  opportunities enqueue/coalesce a Rendering-source task. Renderability is
  rechecked at execution. Pre/post-animation render actions are embedding hooks;
  layout, resize/scroll delivery, observer phases, painting and document ordering
  are driver responsibilities. The core snapshots callbacks after pre-render
  cleanup, removes each before invocation, checkpoints between callbacks, honors
  in-frame cancellation, and defers callbacks registered by an rAF callback.
  The driver supplies the frame timestamp relative to the target global's origin.

Reference algorithms reviewed 2026-09-21:
[HTML event loops](https://html.spec.whatwg.org/multipage/webappapis.html#event-loops),
[timers](https://html.spec.whatwg.org/multipage/timers-and-user-prompts.html#timers),
[animation callbacks](https://html.spec.whatwg.org/multipage/imagebitmap-and-animations.html#animation-frames).
Current HTML uses a rendering task source; rAF is not a 16ms timer shim.
Nested event-loop spinning, worker lifetime/idle callbacks, and the rendering
subsystems themselves are not implemented by this core.

Fresh Windows iteration on base 3d0c2b0: 133 DOM tests passed / 0 failed
(114 existing plus 19 focused event-loop tests). Formatted only on win-001.
The core is not wired to JS yet. JS/driver integration is routed via questions.md:
Interpreter currently owns Instant-based timer/rAF and private Promise jobs, and
WPT driver calls pump_timers/drain_microtasks. A native scheduler alone cannot
move the pinned baseline. Current before-measurement is running with that
unchanged driver; upstream known-pass/known-fail proof verified twice so far.
No DOM-4 WPT result or completion claim yet.

### JS scheduler integration contract (routed after JS-3)

The driver owns EventLoop and payload roots. The JS crate must expose a host mode
with these semantics (names are suggestions; behavior is the contract):

1. Enable external scheduling before any page code. `run_script` and
   `dispatch_dom_event` in this mode execute their requested script/event only;
   they must not call `drain_jobs`/`drain_microtasks`/`pump_timers` implicitly.
2. Emit ordered host scheduling requests at enqueue time: ScheduleTimer with a
   stable host registration ID, owned handler/arguments, converted delay and
   repeat flag; CancelTimer; Request/CancelAnimationFrame; and QueueMicrotask.
   Promise jobs and queueMicrotask callbacks must share this same emission order.
   Do not expose an "all pending promises" batch drain as the only operation.
3. Each microtask request carries a unique rooted owned job/token. Expose
   `invoke_one_job(token)` to run exactly that job, and
   `invoke_one_callback(callback, this_value, arguments)` to run exactly that
   callback. **Neither may auto-drain another private queue**, on success or
   throw. Newly generated jobs/requests remain observable in enqueue order for
   the driver to append to the shared FIFO before completing the current step.
4. Synchronous native DOM hooks must notify the adapter at the original mutation
   observer enqueue point, interleaved with Promise and queueMicrotask requests.
   Polling mutation_pending only after returning from script loses this order.
   Existing native DOM checkpoint/delivery seams remain the notification payload.
5. The adapter maps JS-visible timer/rAF IDs to opaque core TimerId/AnimationFrameId,
   retains callback/argument roots through cancellation/execution, supplies Window
   receiver and rAF timestamp, catches/reports callback errors, then completes the
   scheduler StepId. A thrown callback must not abandon the checkpoint or prevent
   later callbacks. Expired/cancelled registrations release host roots safely.
6. A CheckpointCleanup action invokes rejected-promise reporting, transaction
   cleanup and ClearKeptObjects hooks without recursively draining jobs. Driver
   clock updates and rendering opportunities remain embedding responsibilities.

Contract tests for JS/driver owners: timer A queues Promise P and microtask Q,
then timer B observes P/Q already run; P queues R behind Q; callback A cancels B
that was already due; rAF A queues P which cancels rAF B; invoking one job with
another already pending leaves that other job untouched; throwing A still permits
B after the mandatory checkpoint. Native core already tests the scheduling side.
Legacy standalone Interpreter behavior may remain behind its default mode; host
mode must opt out of implicit drains explicitly, without changing Test262's
intentional standalone execution convention.

For host-triggered multi-listener dispatch and rendering hooks, the driver may
call `checkpoint_after_callback(enclosing_step_id)` when the JS stack becomes
empty between callbacks. Retain the enclosing task continuation, drain yielded
microtasks and cleanup until `pending_step()` equals that original ID, then
resume it. The scheduler restores the original task/frame continuation and timer
nesting without selecting another task. Recursive checkpoint requests are no-ops.
This is callback cleanup within a task, not nested event-loop spinning.

### DOM-4 scope amendment — XML parser seams

Integrator routed two additional html-lane requests to DOM-4:

- `create_element_expanded(parent, namespace_uri, prefix, local_name)` and
  detached `create_element_expanded_in(document, ...)` accept parser-validated
  expanded names. Caller enforces XML QName grammar, namespace resolution and
  reserved-name constraints. The DOM still checks tree insertion/document
  ownership. This admits an unprefixed XML element named xmlns without weakening
  `create_element_ns`, which retains DOM API validation.
- `set_attribute_qualified_ns(node, namespace_uri, qualified_name, value)` does
  DOM attribute-context validation. Parser primitive `set_attribute_expanded`
  accepts already-validated URI/prefix/local name. `attribute_ns(node, uri, local)`
  returns AttributeRef { namespace_uri, prefix, local_name, value }, with
  qualified_name(). Existing get_attribute_ns remains URI/local-name lookup.
  Null namespace attributes do not inherit their element's default namespace.
- Existing expanded-identity updates change the value and retain the original
  prefix, following DOM's set-an-attribute-value algorithm. Remove then create
  selects a new prefix. Mutation records still carry local name/namespace and
  old value; clone/range clone/adoption preserve the prefix metadata.

Five focused XML tests cover parser xmlns construction, ownership, qualified
attribute identity/update/removal, validation, namespace declarations, mutation
records and clone/adoption. Along with two additional callback-continuation
checks, Windows native suite is now 140 passed / 0 failed (21 scheduler tests,
5 new XML tests, 114 previously merged). Initial XML iteration had one test
fixture failure: root construction had already queued a mutation checkpoint;
the test now drains that setup checkpoint and observes invalid attribute edits
independently. No implementation failure was hidden or credited as a pass.

### DOM-4 before measurement (legacy driver queues)

On base `3d0c2b0`, after artifact-invalidating own-tree refresh and 181 matching
source/manifest hashes, the unchanged WPT driver measured pinned revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` twice. Known-pass/known-fail upstream
proof passed twice first. This executable contains the native core but does not
invoke it; it is **before integration**, not evidence of scheduler effectiveness.

Both runs, 627 source files: 2 pass / 241 fail / 30 timeout / 221 error /
133 unsupported; registered subtests 83 PASS / 2584 FAIL. Raw results are
byte-identical both to each other and to DOM-1. Before-baseline shift is zero.
No after result exists until the routed JS/driver adapter runs the scheduler.

- Raw DOM JSONL SHA256: `3601e251023d0f34e0668c7e9b8d1b24df1fbca43391e0cff123ab51ee719f4c`.
- Outcome SHA256: `327f8456d54edf7b306759439ffda04763a1f6c0ffc705ebdb7d8bc2cf0aa4d4`.
- Executable SHA256: `907482bcd7e6abd85e4fb63e533d0641c7b55c9b3be999bf1a6358ff29e86be3`.
- Proof SHA256: `fda888792ad20e12b69ab4909e69896b98916b170c7ecf259cd79854ebe404bd`.
- Durable summary: `crates/amazing-dom/tests/wpt-dom4-before.json`; unchanged
  raw baseline/proof/selection bytes are already committed with DOM-1 under
  the driver's wpt_harness_results directory. Both new raw runs and the preserved
  executable are in `.agents/dom-bindings/dom-4-before/` and Windows target's
  `dom-4-before/`. Source parity and measurement logs are in lane scratch.


### DOM-4 native-core/XML checkpoint gate

Branch: **lane/dom-bindings-dom4 only**. Core/XML implementation `e6f8396`,
rebased on `784dc59`. Own-tree refresh invalidated first-party artifacts and
matched lock 23CEF873; source re-sync followed. All 183 source/manifest hashes
matched Mac. Windows-only gates ran twice with stable canonical results:

| Gate, each run | Passed | Failed | Ignored | Targets |
|---|---:|---:|---:|---:|
| amazing-dom | 140 | 0 | 0 | 2 |
| Workspace | 1160 | 0 | 3 | 58 |

- DOM SHA256: `076bd659b67d5d33647e4f3e32ff746107b3435a602132fecfa2e0ccce4d1657`.
- Workspace SHA256: `da66d34339190626c91c089edb0e24a8d879b7d0313910dd2e42f25cc730b354`.

Canonicalization is sorted test-name/status lines, UTF-8 with LF and a trailing
LF; timing excluded. Native delta is 114 → 140 (+21 scheduler, +5 XML).
Evidence: lane scratch `dom-4-checkpoint-results.json`,
`dom-4-checkpoint-source-manifest.json`, `dom-4-checkpoint-gate.log`, and
`dom-4-checkpoint/` raw logs, mirrored under the Windows lane target.

This is a validated implementation checkpoint, **not DOM-4 completion**. The
integrator routed JS external scheduling after JS-3 and driver adoption to
integrator-assist. Their contract is above. Once those seams are ready, integrate
through their owners, rebase/refresh/re-sync, repeat gates and measure the pinned
WPT after baseline twice. Report its actual delta, including zero/regressions.


## Authorized DOM-4 core/XML delivery

The integrator explicitly split delivery from the remaining integration gate.
Merge **lane/dom-bindings-dom4-core only** for this delivery. It packages the
existing core and both XML seams as one reviewable commit, superseding the
equivalent multi-commit lane/dom-bindings-dom4 history. Do not merge both branches.
The original ab79ebc checkpoint is preserved in history.

Native core/XML delivery is ready for review independently of JS scheduling.
JavaScript queue adoption, driver wiring and the pinned WPT after measurement are
explicitly deferred; neither the before baseline nor native tests claim them.
The external scheduling contract (one owned job/callback, no hidden drains)
remains above for the routed runtime/driver owners.

HTML sink usage for the two independently requested seams:

```rust
// Parser has already checked XML names and namespace bindings.
let node = dom.create_element_expanded(document, None, None, "xmlns")?;
dom.set_attribute_expanded(node, Some("urn:p"), Some("p"), "a", "value")?;
assert_eq!(dom.get_attribute_ns(node, Some("urn:p"), "a"), Some("value"));
assert_eq!(dom.attribute_ns(node, Some("urn:p"), "a").unwrap().qualified_name(), "p:a");
```

`create_element_expanded_in` is the detached counterpart. The DOM API entry
points `create_element_ns` and `set_attribute_qualified_ns` keep their validation;
the parser primitives deliberately accept already-validated expanded names.
The example's unprefixed element xmlns is valid XML and is not treated as a DOM
API namespace error. Five focused XML tests cover these behaviors.

Fresh native-delivery gate on base `bb79669`, measured code `2f2328d`: own-tree
refresh with artifact invalidation, lock E4D68B10 matched, source re-sync and
190 matching source/manifest hashes. Windows DOM **140 passed / 0 failed**,
twice; workspace **1210 passed / 0 failed / 3 ignored** across 59 targets, twice.
Repeat hashes match:

- DOM: `076bd659b67d5d33647e4f3e32ff746107b3435a602132fecfa2e0ccce4d1657`.
- Workspace: `08d1e52cb0a2ea7664a5e71fcbb566d1ddc5bf67bb7f0acb36d8681e730ef7c4`.

Logs/manifest/results: lane scratch `dom-4-core/`, `dom-4-core-gate.log`,
`dom-4-core-source-manifest.json`, `dom-4-core-results.json`, mirrored under
Windows lane target. Native implementation is unchanged from the checkpoint;
the higher workspace count reflects newer sibling code in the integration base.
The authorized core/XML delivery is complete and ready for integrator merge.
The broader DOM-4 JS/WPT integration gate remains deferred and unclaimed.


## DOM-5 — full dom/ conformance push (in progress)

Assignment: implement high-impact error/timeout clusters without waiting for the
DOM-4 scheduling integration. Branch: `lane/dom-bindings-dom5`; starting dev
`11dd8a8` includes merged DOM-4 core `74e3c50`. The orchestration brief still
names DOM-3; the integrator's explicit DOM-5 assignment supersedes it.

### Evidence and selection discipline

A fresh BEFORE pair is running on Windows after an artifact-invalidating own-lane
refresh and source-hash verification. The upstream known-pass/known-fail proof
already passed twice. WPT revision remains
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; selection is the same 627 tracked
source files, not expanded WPT URLs. No AFTER or improvement is claimed yet.

The historical DOM-1/DOM-4-before baseline is 2 pass / 241 fail / 30 timeout /
221 error / 133 unsupported; 83 PASS / 2584 FAIL subtests. The following
**diagnostic grouping**, not a proven causal ranking, accounts for all 251
error/timeout sources. “Observed subtests” means callbacks recorded before the
source ended; it is neither the source's complete potential count nor an
estimate of passes a fix would produce. Zero means registration did not yield
results, not that fixing the cluster has no value.

| Diagnostic group | Sources | Observed subtests |
|---|---:|---:|
| Duplicate names / JS stringification | 7 | 744 |
| Missing callable (mixed DOM APIs) | 35 | 129 |
| Deadline (requires per-source analysis) | 30 | 78 |
| Missing global/interface | 14 | 1 |
| Conversion diagnostic (may mask setup exception) | 41 | 0 |
| Shared testdriver parser failure | 76 | 0 |
| Other JavaScript parser failures | 42 | 0 |
| Unavailable upstream resource | 3 | 0 |
| Other runtime error | 3 | 0 |

The 76-source shared parser failure is `testdriver.js` at `subscribe`; another
42 sources fail parsing other scripts. These require JS-owner work, not a DOM
algorithm change. The 41 conversion diagnostics include 30 in `dom/common.js`;
that shared file calls `setup(setupRangeTests)`, which needs document creation,
node insertion, and multiple node kinds. Exception stringification may mask the
original missing API, so treating all 41 as one proven JS coercion bug would be
incorrect.

The duplicate-name group contains 596 recorded subtests in
`Document-createElementNS.html`, 100 in `passive-by-default.html`, and 48 in five
shadow-range files. The first uses array stringification for names and XML/XHTML
iframe documents. A namespace core fix alone cannot honestly claim to unlock
596 tests. The callable group spans document factories, ChildNode/ParentNode
methods, collection interfaces, and events and needs subdivision by the first
missing operation. The 30 deadlines likewise include iframe/document loading,
constructors, events, and selectors; none is automatically assigned to the
deferred event-loop seam merely because it timed out.

### Binding integration prerequisite (routed in questions.md)

`amazing-bindings` does not yet exist or appear in the workspace. Current live
wrappers are private methods/native functions in `amazing-js/src/interp.rs` and
`interp_builtins.rs`; `BrowserDomHost` in the driver owns the parsed `Dom`.
The existing `DomHost` trait has no general node-kind, namespace, comment,
fragment, range, traversal, or Event object surface. Adding native algorithms
alone cannot change WPT-visible behavior. Registration belongs to the integrator;
JS hooks and driver adoption belong to their respective owners.

Proposed additive binding route, independent of scheduling:

- The runtime provides an owned native callback closure, or equivalent per-
  interpreter extension state and invocation hook, so `amazing-bindings` can
  install constructors, methods, and accessors using its own DOM state without
  process-global/thread-local registries. Current `NativeFn` is a bare function
  pointer and the interpreter's DOM host is private.
- Expose the runtime's ordinary get/set and WebIDL-required conversion hooks to
  the binding layer. Conversions must preserve thrown exceptions and observable
  getter/coercion order, rather than using Rust debug formatting.
- The driver supplies access to its **same parsed Dom**, installs bindings before
  page scripts, and retains canonical wrappers keyed by arena node identity. A
  second disconnected Dom would only manufacture apparent conformance. Native
  borrows must be released before any callback/getter/coercion can re-enter JS.
- Register `amazing-bindings` with first-party `amazing-dom` / `amazing-js`
  dependencies and wire the driver dependency/install call. Alternatively route
  a narrowly scoped bridge grant; no edits to another lane are assumed.

Final validation remains two stable full-selection AFTER runs with the same
upstream proof and revision, measured BEFORE/AFTER counts and cluster impact,
Windows `cargo nextest run -p amazing-dom --locked` (and amazing-bindings once
registered), separate doctests, then refreshed/re-synced workspace nextest plus
workspace doctests. Report the two runner counts separately.

### Diagnostic probes (not WPT results)

Windows driver `--scaffold` probes on base `11dd8a8` confirm that
`String(new Error("sentinel"))` throws “cannot convert object to primitive”,
array stringification is incorrect, and `Math.random` is absent. Consequently
the six NodeList tampering scripts in the callable diagnostic bucket first hit
the JS-owned `Math.random()` interpolation in their shared support script; they
are not six demonstrated NodeList implementation defects. Template interpolation
of a number itself works.

The same probes confirm missing Event/Node/Comment/Text/Document constructors;
`document.createEvent/createComment/createDocumentFragment/createElementNS/
createRange/createAttribute`, `document.implementation`, and element
`cloneNode/insertBefore/querySelectorAll/nodeType`. `document.body` and element
`append` do exist. In `setupRangeTests`, the earliest unsupported call after
creating its div is `document.body.insertBefore`; its error is then liable to be
masked by the confirmed Error stringification failure. Thirty shared-resource
errors therefore require both setup APIs and usable exception reporting, not
merely another Range core implementation. The probe output explicitly carries
`suite: scaffolding-only`; it contributes zero WPT results.

### DOM-5 concrete runtime contract (supersedes proposed import direction above)

Integrator seeded `amazing-bindings` in `62f8487`, depending only on amazing-dom.
The concrete trait is `crates/amazing-bindings/src/host.rs::BindingHost`; runtime
implements it and calls the binding crate, never the reverse dependency. Please
use the file's exact types: `InterfaceDefinition`, `Member`, `Operation`, and
`BindingHost`. Installation registers descriptors and prototypes; native entry
points forward `Operation`, receiver and arguments to the forthcoming `invoke`.
`with_dom` executes a non-reentrant native closure on the driver's same arena;
all JS coercion and wrapper creation occur outside it. `node_brand` must use a
private brand; `wrap_node` must share identity with existing wrappers, including
the existing document object. `child_nodes` supplies a live same-object NodeList.
DOMString conversion takes/returns UTF-16 code units and propagates throws. The
initial operations cover node factories/base properties and ParentNode/ChildNode
mutations, not callbacks or scheduling. `is_html_document` controls the existing
HTML element nodeName casing rule and must reflect document kind.

Native mutation work is checkpointed on this branch. The ChildNode before/after/
replaceWith source helpers have respectively 15, 15, and 11 test declarations
called for Comment, Element and Text: 123 source-declared registrations currently
blocked before any result. This is a bounded synchronous candidate alongside the
ParentNode family, not a claim that 123 tests will pass. Source-declared potential
is kept distinct from measured registered results.

Runtime install notes: new constructors require `new`; `None` constructors are
illegal constructors with real interface/prototype objects. Install members
without deleting unrelated legacy APIs, but migrate legacy **own** properties
that would shadow the new prototype methods/getters. Canonical wrapping applies
to query results and legacy createElement as well as new factories. Map
`DomError::TypeError` to a JS TypeError and hierarchy/not-found errors to genuine
DOMException-shaped exceptions. `is_undefined` distinguishes constructor default
empty strings from explicit null (`"null"`). `DocumentConstructor` makes a new
XML document; `is_html_document` must return false for it. New `NodeOrString::Utf16`
preserves lone surrogate arguments through native fragment conversion.

Targeted Windows validation on the refreshed 62f8487 tree: 149 DOM tests passed
(140 existing + 9 mutation tests); eight binding tests passed (157 combined).
Separate doctests: amazing-dom 0, amazing-bindings 0. These are native and host-
contract tests, not evidence that the runtime has adopted the contract. No
workspace or WPT AFTER claim is made at this checkpoint.

Fresh BEFORE pair completed on 11dd8a8: both raw JSONL streams are byte-identical
to each other and to DOM-1/DOM-4-before. 2 pass / 241 fail / 30 timeout / 221 error /
133 unsupported, 83 PASS / 2584 FAIL. Summary/provenance is committed at
`crates/amazing-dom/tests/wpt-dom5-before.json`; raw artifact SHA-256 is
`3601e251023d0f34e0668c7e9b8d1b24df1fbca43391e0cff123ab51ee719f4c`.

Timeout inspection: 24 of 30 source bodies mention iframe/frame access; the
Comment/Text constructor pair additionally loads iframe work from their shared
support script. Thus 26 have explicit frame dependencies in source or support;
this is dependency evidence, not proof of a single runtime cause. The remaining
four are two click activation tests, EventListener-handleEvent, and the SVG
window.onload smoke test. Constructors can gain synchronous subtest passes while
their cross-realm part still times out; do not claim the whole file is fixed.

### DOM-5 end-of-day triage checkpoint (2026-09-20 UTC)

Standing down by explicit user instruction during the win-001 SSH outage. No
post-outage gate ran and no further SSH/scp connections are permitted until the
integrator clears the stand-down. New durable evidence is in
`crates/amazing-dom/tests/dom5-triage/README.md` and its adjacent artifacts.

The instrumented copied harness recovered the original exception in all 41
conversion-error sources. All 30 common.js cases reached body.insertBefore and
threw “value is not callable” before the next statement; the harness's String(e)
then masked it. This is a confirmed missing binding, not a native Range failure.
A pre-outage Edge registration census measured 49,514 subtests behind those 30
sources (34,505 Range boundaries/queries; 5,626 mutation response; 4,179 content;
3,652 node properties/relationships; 1,552 traversal). These are reference counts,
not Amazing results or a promise that one missing method unlocks all of them.
The remaining fixture dependencies include CDATA and DOMImplementation APIs.

All 118 parser failures have source-location attribution: 76 async object-method,
33 await-expression, 6 trailing-call-comma, 3 surrogate-escape failures. Full
causal ranking is still partial: the additional call-site tracer was NOT run
because SSH failed. Remaining mixed callable/duplicate-name/deadline groups and
the full failed/unsupported inventory need completion. The current binding/core
implementation is unchanged; no runtime adoption or WPT AFTER result is claimed.

FIRST resume action: check the integrator's current restoration/stand-down notice.
Do not open SSH/scp until explicitly cleared. Then retrieve the reference-census
server metadata and clean up only task-owned reference-browser profiles, review
and run the draft call tracer, inspect routed runtime/driver adoption, and follow
fresh rebase/refresh/re-sync gates. The report lists every remaining item.

### DOM-5 resumed triage and native microtask contract (2026-09-23)

The integrator restored SSH and refreshed the lane tree to `origin/dev`
`ffb143b`; the lane rebased and re-synced its owned files. The 30 reference
census records were retrieved, and 30 task-owned headless-browser profiles were
removed after confirming none was in use. The copied-resource call tracer ran
for 46 historical callable/conversion error sources. Its results, the complete
627-source historical ranking and their diagnostic limitations are under
`crates/amazing-dom/tests/dom5-triage/`. None of that changes the pre-outage WPT
baseline; a fresh current-base full selection is queued in a shared build slot.

JS-4's draft scheduling host is being revised to a bounded interpreter outbox.
Native MutationObserver delivery must interleave with JS jobs **during** an
interpreter entry: script queues Promise P, mutates the DOM (M), then calls
queueMicrotask Q, so the outbox must preserve P,M,Q when it drains into the
EventLoop after the entry. `BindingHost::with_dom` observes the DOM's mutation
delivery flag across the native closure, drops the DOM borrow, and appends an
opaque `NativeMicrotask::MutationObserver` request before the next JS statement.
It does this even when the closure mutates and then returns an error. The native
request neither invokes JS nor steps EventLoop synchronously. The DOM flag is
cleared only by `begin_mutation_checkpoint` when that queued EventLoop microtask
runs, not when admission is attempted or acknowledged. Mutations with no
interested observer leave the flag clear; multiple records while it is set
produce one pending delivery per surrounding agent.

The integrator's capacity decision reserves one native outbox slot per agent
for this flag-gated request. JS Promise, queueMicrotask, timer and frame requests
use only unreserved capacity, so native admission cannot fail. Acknowledgement
means successful M admission. At JS capacity one, P takes the JS slot, M takes
the native slot and Q throws catchable TypeError without being enqueued: the
resulting order is P,M. With enough JS capacity for P and Q, the order is
P,M,Q. A future native kind without a bounded flag would require a different
failure policy, potentially a poison state; that policy is not part of this
MutationObserver contract. js-runtime owns the capacity split and exact outbox
API, js-lang reviews it, and this lane owns the DOM flag semantics. The real
EventLoop adapter proof must include these two traces plus script A, Promise P,
queueMicrotask Q, chained R, timer B and frame cancellation after a checkpoint.
No driver adapter will be built against the draft SchedulingHost signature.

js-runtime answered the final proposed API as
`Interpreter::enqueue_native_microtask(global: u64, kind: NativeMicrotaskKind)
-> Result<(), Throw>` and
`SchedulingRequest::NativeMicrotask { global, kind }`, with
`NativeMicrotaskKind::MutationObserver`. The adapter's forwarded bit is set
only after successful outbox admission and prevents duplicate true-to-true
posts. A test-only real Interpreter/DOM/EventLoop proof is drafted under
`crates/amazing-dom/tests/dom5-triage/`; the integrator granted exactly one new
driver integration test file at
`crates/amazing/tests/js4_event_loop_adapter.rs`. Move and compile the draft
there only after JS-4 merges to dev. No `amazing-dom` → `amazing-js` test
dependency or root lockfile change is authorized.

### BindingHost placement and lossless string boundary (2026-09-23 decision)

`amazing_bindings::host::BindingHost` is declared by amazing-bindings, which
continues to depend only on amazing-dom. The driver-local `RuntimeBindingHost`
will implement it with backend-neutral capabilities exposed by amazing-js.
js-runtime accepted the object/function/property capability vocabulary in
`crates/amazing-bindings/src/host.rs`. `create_native_function` takes
`HostCallPolicy::{CallOnly, ConstructOnly, CallAndConstruct}` and an opaque
`Operation` token; the driver maps that token to a runtime-owned,
per-interpreter `HostFunctionId`. No AST or VM type crosses the trait.
The active opaque `install_interface` entry point is retained until the driver
adapter and real installer are ready. Integrator-assist owns the driver file.

The existing `string(&[u16]) -> Value` and
`to_dom_string(&Value) -> Result<Vec<u16>, Error>` signatures are accepted as
lossless UTF-16 contracts. In particular, lone surrogates must round-trip
unchanged, without U+FFFD substitution. js-lang first adds UTF-16 literal data
to the AST without removing the existing field. js-runtime then migrates
`JsValue::Str` to an opaque `JsString` storing code units, verifies Test262
before/after against the 10,314 baseline, and proves JS→DOM→JS lone-surrogate
round-trip. Runtime string methods for BindingHost wait for `JsString`;
independent object/function/property capabilities can proceed before it.

### DOM-5 native fixed-base final gate (2026-09-23)

On recorded base `2021707`, the Windows lane tree was refreshed with first-party
artifact invalidation and owned files re-synced. The pinned 627-source `dom/`
AFTER pair at WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`
completed with two workers per round. The unmodified upstream known-pass and
known-fail proof passed twice. Both rounds were byte-identical: 2 pass, 241
fail, 30 timeout, 221 error and 133 unsupported files; 83 pass and 2,584 fail
registered subtests, with no other subtest statuses. Each raw SHA-256 was
`3601e251023d0f34e0668c7e9b8d1b24df1fbca43391e0cff123ab51ee719f4c`;
each outcome SHA-256 was
`327f8456d54edf7b306759439ffda04763a1f6c0ffc705ebdb7d8bc2cf0aa4d4`.
These equal the BEFORE gate; no ranked cluster closed in the unadopted driver.
The durable summary is `crates/amazing-dom/tests/wpt-dom5-after.json`.

The final fixed-base Windows workspace gate verified 24 owned file hashes,
including the accepted `HostCallPolicy` trait signatures. `cargo nextest run
--workspace --locked` passed 1,335/1,335 tests with 3 skipped across 43
binaries. Separate `cargo test --workspace --locked --doc` passed 2/2
doctests; amazing-dom and amazing-bindings each had zero doctests. The JS-4
adapter proof and driver installer remain follow-up integration work after
native DOM-5 merge.

### DOM-6 Adapter recipe (JS-4 EventLoop proof)

The runnable reference is `crates/amazing/tests/js4_event_loop_adapter.rs`.
Construct `Interpreter::new_with_external_scheduler(js_capacity)` before any
page code, attach the same parsed DOM through `set_dom_host`, create one
`EventLoop`, register the global, and retain maps from JS timer/frame
registration IDs to core IDs plus opaque callback payloads.

At every interpreter entry, in this order:

1. Run exactly one script, event dispatch, timer/frame callback via
   `invoke_one_callback`, or Promise/queueMicrotask job via `invoke_one_job`.
   If it throws, record the exception but still perform the remaining steps.
   A native DOM mutation is observed inside the host call: after releasing its
   DOM borrow and before the next JS statement, the host exposes one pending
   `MutationObserver` request. The interpreter admits it into its ordered
   outbox, then acknowledges admission; only then set the adapter's forwarded
   bit. Keep the DOM queued flag set until the checkpoint executes.
2. Immediately call `take_scheduling_requests()` and process its vector in
   order. For timer/frame registration, retain the JS token in a driver-owned
   callback payload and map the returned core ID by JS registration ID.
   Apply cancellations against those maps. Queue JS and native microtask
   requests in the shared EventLoop FIFO in the order received. A native
   request stores only its kind/global and a driver-owned delivery payload.
3. Advance the driver clock with `EventLoop::advance_to`; request rendering
   opportunities when the embedding has one. Repeatedly take `step()`, invoke
   its one callback/job/native delivery, drain the newly emitted interpreter
   requests **before** `complete(step.id)`, then complete the step even after a
   throw. Handle render and checkpoint-cleanup actions without recursively
   draining a private JS queue. Frame callbacks receive the core timestamp;
   timers receive no frame timestamp. Retain the Window receiver for JS
   callbacks. Cancellation removes the driver mapping and core registration.
4. For native delivery, call `Dom::begin_mutation_checkpoint()` only when the
   EventLoop reaches M, clear the forwarded bit, then consume
   `next_mutation_delivery` until empty. A second mutation while the pending
   flag stays true must not queue another M; a mutation after this checkpoint
   rearms it. JS-originated requests use only the configured JS capacity; the
   reserved native slot admits M in order even when a later Q throws a
   catchable TypeError.

The reference test checks A,P,Q,R,B; frame cancellation at the checkpoint;
P,M,Q with capacity two; P,M and catchable TypeError with capacity one; one M
for repeated true-to-true mutations; admission acknowledgment only after
success; M rearming after its checkpoint; and outbox draining after both a thrown
callback and a thrown script. The driver must use the same sequence at *every*
interpreter entry, including error exits.

### DOM-7 first-installation coverage audit (BindingHost extension accepted)

Normative source pinned to WHATWG DOM Standard `dom.bs` at commit
[`a2331a45360129e8645ef7e0a04740241b6e3726`](https://github.com/whatwg/dom/blob/a2331a45360129e8645ef7e0a04740241b6e3726/dom.bs),
SHA-256 `d98c6542f35f2aa976cfbf037f8cd1f556c09a704700f19a0a163100935098ff`.
This is an IDL-to-native/binding inventory on fixed engine base `c8c3c21`,
not a conformance or WPT result. "Implemented" means the existing binding
operation is present, even though the product driver has not installed it.
"Needs host" means the native concept exists but the accepted `BindingHost`
contract cannot privately brand/wrap or invoke the required JS value.
"Deferred native" means the named standard behavior has no matching native
model yet. The DOM-7 brief requires a stop and a question when a host primitive
is missing; the request is in `.agents/dom-bindings/questions.md`.

| Interface | Members (IDL names) | State | First missing piece |
|---|---|---|---|
| Document | `createTextNode`, `createComment`, `createDocumentFragment` | Implemented | Existing Node wrappers; no driver installation yet. |
| Document | `createCDATASection`, `createAttribute`, `createAttributeNS`, `createProcessingInstruction` | Implemented | Native factories and private CDATASection/Attr wrappers; driver installation pending. |
| Document | `createRange` | Implemented | Generic private Range wrapper and native dispatch; driver installation pending. |
| Document | `createNodeIterator`, `createTreeWalker` | Implemented | Callback conversion and invocation release the DOM borrow; driver installation pending. |
| Document | `createEvent` | Implemented | Event registry now belongs to Dom; driver installation pending. |
| Document | `implementation` `[SameObject]` | Implemented | Associated document key supplies canonical private wrapper; driver installation pending. |
| DOMImplementation | `createDocumentType`, `createDocument`, `createHTMLDocument`, `hasFeature` | Implemented | Native factory dispatch and mock proof; document metadata remains incomplete. |
| CDATASection : Text | inherited Text/CharacterData/Node members | Implemented | Native node kind and factory; driver installation pending. |
| Attr : Node | `namespaceURI`, `prefix`, `localName`, `name`, `value`, `ownerElement`, `specified` | Implemented | Attached/detached Attr identity and UTF-16 value sidecar; driver installation pending. |
| NamedNodeMap | `length`, `item`, `getNamedItem`, `getNamedItemNS`, `setNamedItem`, `setNamedItemNS`, `removeNamedItem`, `removeNamedItemNS` | Implemented | Live map through element attributes; driver installation pending. |
| NodeList | `length`, `item` | Implemented | Live `childNodes` and immutable mutation snapshot wrappers; shared private identity. |
| NodeList | indexed getter, iterable | Runtime hook draft | Live/static NodeList indexed hooks and `define_iterable(NodeList, Values)` declare dynamic indexed access and `@@iterator`, `keys`, `values`, `entries`, `forEach`; runtime implementation pending. |
| HTMLCollection | `children` on Document, DocumentFragment, Element; `getElementsByTagName(NS)`, `getElementsByClassName`; `length`, `item`, `namedItem`, indexed/named getters | Native and binding dispatch implemented | Live mutation-generation cache and canonical owner/filter key; `define_legacy_platform_object` is a draft runtime hook, so JS property access awaits js-runtime. |
| HTMLElement : Element | `id`, `className`, `title`, `lang`, `dir`, `hidden`, `tabIndex`, `accessKey`, `translate` | Native binding dispatch implemented | Shared reflection helpers, specialized empty-prototype registrations, known/unknown interface lookup; driver prototype selection pending. |
| HTMLUnknownElement : HTMLElement | inherited members | Interface registered | Fallback from local-name lookup; driver prototype selection pending. |
| Element | `innerHTML`, `outerHTML`, `insertAdjacentHTML` | Native and binding dispatch implemented | Fragment parser is a new driver host seam; DOM owns serialization and cross-arena import. HTML-6/7 context coverage remains a parser prerequisite. |
| Document, DocumentFragment, Element | `querySelector`, `querySelectorAll` | Native and binding dispatch implemented | New selector engine seam parses once and matches outside DOM borrows; `querySelectorAll` snapshots a static NodeList. |
| Element | `matches`, `closest`, `webkitMatchesSelector` | Native and binding dispatch implemented | Parse errors must surface as SyntaxError from the driver seam. |
| HTMLElement | `innerText` | DOM-only approximation | Reads text and BR line breaks without computed style, visibility or layout; setter replaces content with a text node. |
| DOMTokenList | `length`, `item`, `contains`, `add`, `remove`, `toggle`, `replace`, `supports`, `value`, stringifier | Implemented | Live `classList` uses the element's class attribute and canonical wrapper. |
| DOMTokenList | indexed getter, iterable | Runtime hook draft | Dynamic indexed hooks and `define_iterable(DOMTokenList, Values)` registered; runtime implementation pending. |
| AbstractRange | `startContainer`, `startOffset`, `endContainer`, `endOffset`, `collapsed` | Implemented | Private Range/StaticRange wrappers return live or immutable boundaries respectively. |
| StaticRange : AbstractRange | constructor with `StaticRangeInit` dictionary | Implemented | Immutable native snapshot in an append-only registry; inherited getters accept its private brand. |
| Range : AbstractRange | constructor, `commonAncestorContainer`, `setStart`, `setEnd`, `setStartBefore`, `setStartAfter`, `setEndBefore`, `setEndAfter`, `collapse`, `selectNode`, `selectNodeContents` | Implemented | Live native Range handle; `unsigned long` coercion runs before DOM borrow. |
| Range : AbstractRange | `START_TO_START`, `START_TO_END`, `END_TO_END`, `END_TO_START`, `compareBoundaryPoints` | Implemented | Constants installed on interface/prototype through host primitive; comparison dispatch written. |
| Range : AbstractRange | `deleteContents`, `extractContents`, `cloneContents`, `insertNode`, `surroundContents`, `cloneRange`, `detach`, `intersectsNode` | Implemented | Native algorithms dispatched; fragments use shared Node wrappers. |
| Range : AbstractRange | `isPointInRange`, `comparePoint`, stringifier | Implemented | Native point checks and text projection dispatch. |
| NodeFilter callback interface | `FILTER_ACCEPT`, `FILTER_REJECT`, `FILTER_SKIP`; `SHOW_ALL`, `SHOW_ELEMENT`, `SHOW_ATTRIBUTE`, `SHOW_TEXT`, `SHOW_CDATA_SECTION`, `SHOW_ENTITY_REFERENCE`, `SHOW_ENTITY`, `SHOW_PROCESSING_INSTRUCTION`, `SHOW_COMMENT`, `SHOW_DOCUMENT`, `SHOW_DOCUMENT_TYPE`, `SHOW_DOCUMENT_FRAGMENT`, `SHOW_NOTATION` | Implemented | Constants installed through the generic interface constant hook. |
| NodeFilter callback interface | `acceptNode` | Implemented | Host retains opaque callback roots; traversal suspends native borrow, calls `acceptNode`, then resumes. |
| NodeIterator | `root` `[SameObject]`, `referenceNode`, `pointerBeforeReferenceNode`, `whatToShow`, `filter`, `nextNode`, `previousNode`, `detach` | Implemented | Private traversal wrapper, retained filter identity and resumable invocation tested; GC release hook still needed in driver. |
| TreeWalker | `root` `[SameObject]`, `whatToShow`, `filter`, `currentNode`, `parentNode`, `firstChild`, `lastChild`, `previousSibling`, `nextSibling`, `previousNode`, `nextNode` | Implemented | Same callback and root model as NodeIterator; GC release hook still needed in driver. |
| EventTarget | constructor, `addEventListener`, `removeEventListener`, `dispatchEvent` | Implemented | Dom-owned registry, callback identity and resumable listener dispatch tested; driver still uses legacy separate registry. |
| EventListener callback interface | `handleEvent` | Implemented | Function or object method resolved and called after DOM borrow. |
| Event | constructor, `type`, `target`, `srcElement`, `currentTarget`, `composedPath`, `NONE`, `CAPTURING_PHASE`, `AT_TARGET`, `BUBBLING_PHASE`, `eventPhase` | Implemented with UTF-16 gap | Private Event wrapper and array path; native event type remains Rust String and rejects lone surrogate. |
| Event | `stopPropagation`, `cancelBubble`, `stopImmediatePropagation`, `bubbles`, `cancelable`, `returnValue`, `preventDefault`, `defaultPrevented`, `composed`, `isTrusted`, `timeStamp`, `initEvent` | Implemented | Native methods and lifecycle; mock dispatch/cancellation tested. |
| CustomEvent : Event | constructor, `detail`, `initCustomEvent` | Implemented | Native Event stores an opaque driver-owned value root; replacement releases the old root. Release on Event slot collection remains driver work. |
| MutationObserver | constructor, `observe`, `disconnect`, `takeRecords` | Implemented | Callback root survives disconnect and observe again; mock test covers record snapshot and reconnect. |
| MutationCallback | callback `(mutations, observer)` | Implemented | `deliver_mutation_checkpoint` runs post-borrow and takes native checkpoint only when JS-4 invokes M. |
| MutationRecord | `type`, `target` `[SameObject]`, `addedNodes` `[SameObject]`, `removedNodes` `[SameObject]`, `previousSibling`, `nextSibling`, `attributeName`, `attributeNamespace`, `oldValue` | Implemented | Native record registry and two immutable NodeList slots per record preserve identity. |

Integrator accepted the backend-neutral host extension on 2026-09-23. The
trait now uses `PlatformKey { arena, slot, generation }` and a private
`InterfaceBrand` family for canonical wrappers, with Node methods expressed
through that generic identity. It also declares explicit-this `call`,
`is_callable`, `same_value`, `create_array`, `to_number`, and `throw_value`.
The additive `install_interface_constant` hook places non-Node constants on
both interface and prototype while the opaque installer remains active.
The 2026-09-23 final decision generalized retention to `retain_value`,
`root_value`, and `release_value` for callbacks and CustomEvent detail, and moved
`EventTargets` into `Dom`. The driver must route its legacy separate event
registry to `Dom::events()` / `events_mut()` before binding installation. The
current `Rc` runtime can leak a callback that captures its own target wrapper
until the collector exists; `release_value` handles explicit listener removal
and `once`. MutationObserver callbacks survive `disconnect()` so `observe()`
can run again, and release only when the observer slot is freed. The future
collector must trace those edges. Traversal filter roots need release when JS
wrappers are collected.
The driver must map these to the interpreter facade; js-runtime still owes
explicit-this call, array construction, and SameValue. Binding tests use a
mock host in the meantime. Runtime review should confirm the exact `u64`
brand family values and the node key mapping before driver installation.

The InterfaceDefinition list now covers every implemented row above; HTMLCollection,
indexed/named property access and iterability remain explicit
installation gaps. `BindingHost::string` returns `Result<Value, Error>` and
propagates lone-surrogate rejection without inserting replacement characters.
The driver must route native `DomError` values through
`amazing_bindings::dom_exception::throw_dom_error` so exposed failures carry
the specified DOMException name and legacy code. The mock host currently
asserts the mapping separately from its `with_dom` error conversion.

### DOM-7 final Windows gate (2026-09-23)

The local lane branch was rebased onto `origin/dev` `a3d9001` (later than the
required `133db58`). The Windows lane tree was refreshed from that exact dev
commit with first-party artifact invalidation; all changed source files were
re-synced, including the granted sibling match arms. Those four sibling files
have additions only. The exact-source `cargo nextest run --workspace --locked`
gate passed **1,452/1,452** tests with 3 skipped across 50 binaries. The
separate `cargo test -p amazing-dom -p amazing-bindings --locked --doc` gate
passed with 0 doctests in each crate. No local build or test ran.

The pinned `dom/` AFTER selection at WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` passed the unmodified
upstream known-pass/known-fail proof twice. Both 627-source rounds were
byte-identical: **2 pass, 248 fail, 30 timeout, 214 error, 133 unsupported**
files; **83 pass and 2,590 fail** registered subtests, no other subtest
statuses. Raw SHA-256:
`0243a4ea04619f7c2125185bd3aa31b9d0520c9a4a13f127be3146666d04fba8`;
outcome SHA-256:
`7f939b5fd55441c27861e1cde846283ee4ae76d02add7765a0c4a3a01a10343b`.
The durable summary is `crates/amazing-dom/tests/wpt-dom7-after.json`.
Compared with the DOM-5 fixed-base gate, seven files moved from error to fail
and six additional failing subtests registered. `dev` changed between those
measurements, and the DOM-7 installer is not yet in the driver, so this is
not attributed as a DOM-7 conformance improvement.

### DOM-8 legacy platform object host draft (2026-09-23)

`HTMLCollection` uses the native owner/filter collection slot as a private key. The runtime must consult the registered hooks during `[[Get]]`, `[[GetOwnProperty]]` and `[[OwnPropertyKeys]]`, applying WebIDL named-property visibility and the `LegacyUnenumerableNamedProperties` rule. It must not materialize keys as ordinary own properties. Supported names are returned in tree order, with each element’s ID before its HTML `name`; `namedItem` finds the first element in tree order whose ID or HTML name matches. Indexed getters use currently supported indices. Every hook call runs outside `with_dom`, then binds a short native borrow. `NodeList` iterable registration needs runtime iterator state with live indexed reads. The mock exercises dynamic key resolution after mutation. This is an additive proposal for js-runtime, not a runtime implementation.

The js-runtime review is incorporated: all four registered legacy objects set `override_builtins: false` and expose optional `indexed_setter`, `named_setter`, and `named_deleter` slots (currently `None` for these read-only objects). The supported index hook dispatches through `supported_index_count(host, operation, receiver) -> Result<u32, H::Error>`; the runtime treats `0..count` as the supported indices. The names hook dispatches through `supported_names(host, operation, receiver) -> Result<Vec<Vec<u16>>, H::Error>`, preserving UTF-16 units and spec order. These typed functions return native values; the generic `invoke` entry point handles getter operations returning JS values, and the runtime converts keys only for `[[OwnPropertyKeys]]`. Every callback starts outside a DOM borrow and acquires only its own short borrow. Optional setter/deleter operations remain available for future mutable legacy interfaces.

Exact `host.rs` diff on the fixed base:

```diff
diff --git a/crates/amazing-bindings/src/host.rs b/crates/amazing-bindings/src/host.rs
index 81848d9..6d21c52 100644
--- a/crates/amazing-bindings/src/host.rs
+++ b/crates/amazing-bindings/src/host.rs
@@ -51,6 +51,72 @@ pub enum Interface {
     DocumentFragment,
     DocumentType,
     Element,
+    HTMLElement,
+    HTMLHtmlElement,
+    HTMLHeadElement,
+    HTMLTitleElement,
+    HTMLBaseElement,
+    HTMLLinkElement,
+    HTMLMetaElement,
+    HTMLStyleElement,
+    HTMLBodyElement,
+    HTMLHeadingElement,
+    HTMLParagraphElement,
+    HTMLHRElement,
+    HTMLPreElement,
+    HTMLQuoteElement,
+    HTMLOListElement,
+    HTMLUListElement,
+    HTMLLIElement,
+    HTMLDListElement,
+    HTMLDivElement,
+    HTMLAnchorElement,
+    HTMLImageElement,
+    HTMLBRElement,
+    HTMLModElement,
+    HTMLTimeElement,
+    HTMLDataElement,
+    HTMLTableElement,
+    HTMLTableCaptionElement,
+    HTMLTableSectionElement,
+    HTMLTableRowElement,
+    HTMLTableCellElement,
+    HTMLTableColElement,
+    HTMLFormElement,
+    HTMLInputElement,
+    HTMLButtonElement,
+    HTMLSelectElement,
+    HTMLOptionElement,
+    HTMLOptGroupElement,
+    HTMLTextAreaElement,
+    HTMLLabelElement,
+    HTMLFieldSetElement,
+    HTMLLegendElement,
+    HTMLDataListElement,
+    HTMLOutputElement,
+    HTMLProgressElement,
+    HTMLMeterElement,
+    HTMLVideoElement,
+    HTMLMediaElement,
+    HTMLAudioElement,
+    HTMLSourceElement,
+    HTMLTrackElement,
+    HTMLCanvasElement,
+    HTMLScriptElement,
+    HTMLTemplateElement,
+    HTMLSlotElement,
+    HTMLIFrameElement,
+    HTMLEmbedElement,
+    HTMLObjectElement,
+    HTMLParamElement,
+    HTMLDetailsElement,
+    HTMLDialogElement,
+    HTMLMapElement,
+    HTMLAreaElement,
+    HTMLPictureElement,
+    HTMLMenuElement,
+    HTMLUnknownElement,
+    HTMLCollection,
     CharacterData,
     Text,
     Comment,
@@ -112,8 +178,26 @@ pub enum Operation {
     ElementClassList,
     ElementClassName,
     ElementSetClassName,
+    ElementId,
+    ElementSetId,
+    HTMLTitle,
+    HTMLSetTitle,
+    HTMLLang,
+    HTMLSetLang,
+    HTMLDir,
+    HTMLSetDir,
+    HTMLHidden,
+    HTMLSetHidden,
+    HTMLTabIndex,
+    HTMLSetTabIndex,
+    HTMLAccessKey,
+    HTMLSetAccessKey,
+    HTMLTranslate,
+    HTMLSetTranslate,
     TokenListLength,
     TokenListItem,
+    TokenListSupportedIndexCount,
+    TokenListIndexedGetter,
     TokenListContains,
     TokenListAdd,
     TokenListRemove,
@@ -195,6 +279,24 @@ pub enum Operation {
     CreateElementNS,
     NodeListLength,
     NodeListItem,
+    NodeListSupportedIndexCount,
+    NodeListIndexedGetter,
+    ElementChildren,
+    DocumentChildren,
+    FragmentChildren,
+    ElementGetElementsByTagName,
+    ElementGetElementsByTagNameNS,
+    ElementGetElementsByClassName,
+    DocumentGetElementsByTagName,
+    DocumentGetElementsByTagNameNS,
+    DocumentGetElementsByClassName,
+    HTMLCollectionLength,
+    HTMLCollectionItem,
+    HTMLCollectionNamedItem,
+    HTMLCollectionSupportedIndexCount,
+    HTMLCollectionSupportedNames,
+    HTMLCollectionIndexedGetter,
+    HTMLCollectionNamedGetter,
     CreateNodeIterator,
     CreateTreeWalker,
     TraversalRoot,
@@ -294,6 +396,28 @@ pub enum HostCallPolicy {
     CallAndConstruct,
 }

+/// Runtime-owned internal methods for WebIDL legacy platform objects. The
+/// runtime asks bindings for the currently supported keys and values during
+/// [[Get]], [[GetOwnProperty]] and [[OwnPropertyKeys]], after ordinary WebIDL
+/// visibility rules. No indexed or named properties are preinstalled.
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub struct LegacyPlatformHooks {
+    /// Routed through `supported_index_count`, returning a native count.
+    pub supported_index_count: Operation,
+    /// Routed through `supported_names`, returning UTF-16 names in spec order.
+    pub supported_names: Option<Operation>,
+    pub indexed_getter: Operation,
+    pub indexed_setter: Option<Operation>,
+    pub named_getter: Option<Operation>,
+    pub named_setter: Option<Operation>,
+    pub named_deleter: Option<Operation>,
+    pub override_builtins: bool,
+    pub unenumerable_names: bool,
+}
+
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub enum IterableKind { Values }
+
 /// The binding layer declares this trait. The driver implements it using the
 /// runtime's backend-neutral capabilities and its same parsed DOM arena.
 ///
@@ -354,6 +478,12 @@ pub trait BindingHost: Sized {
         enumerable: bool,
         configurable: bool,
     ) -> Result<(), Self::Error>;
+    fn define_legacy_platform_object(
+        &mut self, brand: InterfaceBrand, hooks: LegacyPlatformHooks,
+    ) -> Result<(), Self::Error>;
+    /// Installs @@iterator, keys, values, entries and forEach on the interface
+    /// prototype with runtime-owned iterator state and live indexed reads.
+    fn define_iterable(&mut self, interface: Interface, kind: IterableKind) -> Result<(), Self::Error>;
     fn get_property(
         &mut self,
         object: &Self::Value,
@@ -443,6 +573,9 @@ pub trait BindingHost: Sized {
     fn is_undefined(&self, value: &Self::Value) -> bool;
     fn is_nullish(&self, value: &Self::Value) -> bool;
     fn to_boolean(&self, value: &Self::Value) -> bool;
+    /// Returns code units only when the runtime value is a primitive String.
+    /// Used before WebIDL union conversion for `HTMLElement.hidden`.
+    fn string_units(&self, value: &Self::Value) -> Option<Vec<u16>>;
     /// WebIDL DOMString conversion, preserving code units and thrown coercions.
     fn to_dom_string(&mut self, value: &Self::Value) -> Result<Vec<u16>, Self::Error>;
     fn type_error(&mut self, message: &'static str) -> Self::Error;
```

### DOM-9 host seams and installation checklist (2026-09-23)

`FragmentParser` receives a context local name, namespace URI, and UTF-16 markup after the binding releases its DOM borrow. It returns a parser-owned `Dom` whose document children are copied into the receiver arena, preserving element namespaces and tree order. The driver must implement it with `amazing_html::parse_fragment` and map unsupported contexts to a typed error until HTML-6/7 provides the remaining context rules. Parsing and conversion finish before any receiver tree mutation. The parser must handle WebIDL DOMString code units, including lone-surrogate replacement required by HTML tokenization.

`SelectorEngine` parses UTF-16 selector text once to an opaque driver handle and reports invalid syntax as SyntaxError. It matches elements against the current DOM using `amazing_css::selectors::{parse_selector, matches}`. The binding releases every DOM borrow before calling this seam; the driver may briefly borrow the arena for each match. `querySelectorAll` stores a static NodeList of matching node handles in tree order; `querySelector` returns the first. `matches` tests only the receiver; `closest` tests its inclusive element ancestors.

Integrator-assist installation: implement both new traits on the driver host; connect HTML/CSS parser errors to DOMException names; add the new `Operation` cases to the installed interface dispatch; expose static NodeList wrappers and the `NoModificationAllowedError` mapping. The HTML parser's bounded fragment context list currently limits real-page coverage, and the CSS parser's supported selector subset limits matching. `innerText` is a DOM-only approximation: it includes text and BR newlines, without computed style, visibility, layout, or CSS white-space processing. Template content is represented by the current DOM child tree, so fragment serialization serializes those children; a separate template-content node is future HTML/DOM work. Driver adoption and WPT measurement remain separate tickets.


## DOM-10 surface and installation checklist (2026-09-23)

Fixed base: `c2d3012`. DOM-10 is implemented in the native DOM and mock binding host. Production driver adoption and pinned WPT measurement are separate integrator work.

| Surface | Coverage |
|---|---|
| AbortController `signal`, `abort(reason)` | Done; default AbortError, idempotent abort |
| AbortSignal `aborted`, `reason`, `throwIfAborted`, `onabort`, abort event | Done; native algorithm callbacks precede events |
| AbortSignal static `abort`, `any`, `timeout` | Done; dependent source graph, host timer fire seam |
| Document `title` HTML/SVG, `head`, `body` get/set, `documentElement`, `readyState` | Done; host advances readyState |
| Document `URL`, `documentURI`, `characterSet`, `contentType`, `compatMode`, `referrer` | Done via DocumentHost |
| Document/Fragment `getElementById`; Document `getElementsByName` | Done; name result is live NodeList |
| Document `forms`, `images`, `links`, `scripts` | Done; live HTMLCollections |
| Document `activeElement`, `hasFocus` | Done via DocumentHost |
| Anchor/Area `href`, `target`, `rel`, `relList`, `download` | Done; href resolves through UrlReflection, relList is live |
| Image `src`, `alt`, `width`, `height`, `loading` | Done; unsigned long conversion and loading enum |
| Input `type`, `name`, `disabled`, `placeholder` | Done; `value`/`checked` state deferred to form work |
| Script `src`, `type`, `async`, `defer` | Done |
| Link `href`, `rel`, `relList`, `media`, `type`, `disabled` | Done |
| Meta `name`, `content`, `httpEquiv`, `charset` | Done |
| TableCell `colSpan`, `rowSpan` | Done; clamp to specified limits |
| OList `start`, `reversed`; LI `value` | Done |
| Remaining element-specific reflected attributes outside ticket list | Deferred |

Integrator installation steps:

1. Implement `AbortHost` with a driver timer queue and call `fire_abort_timeout` when its timer fires. Keep signal reason roots alive and expose an embedding callback registry for native abort algorithms. For NET-13 `fetch_api` and NET-14 `xhr`, register each request through `Dom::add_abort_algorithm`; if it returns an existing reason, reject immediately. On abort, the driver maps that typed reason to each NET cancellation path, then unregisters on settlement.
2. Implement `DocumentHost` using the active document, parser readiness, navigation metadata, and focus state. Call `set_document_ready_state` at parser and load transitions. Implement `UrlReflection::resolve_url` against the document base URL and URL parser.
3. Install AbortController/AbortSignal constructors and AbortSignal static methods with `BindingHost::install_static_method`. Dispatch `Operation` additions, the `FILTERED_NODE_LIST` brand, and its legacy indexed hooks. Preserve one shared DOM arena and wrapper identity.
4. Retain JS roots for signal reasons and onabort handlers until the runtime collector can trace/release the native signal graph. Wire timeout task cancellation when the signal is no longer observable.

Mock-host coverage includes source/dependent abort algorithm and event order, default/explicit reasons, timeout fire idempotence, HTML/SVG Document behavior, live named NodeList/collections, invalid enums, negative unsigned long, URL seam resolution, relList mutation and supported-token lookup, and cell clamping. Production driver adoption and WPT `html/dom/reflection-*` measurement are deferred; no WPT credit is claimed for mock tests.
