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


## DOM-11 pinned WPT native assertion ports (2026-09-23)

Pinned WPT checkout was verified at `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` on win-001. These are native Rust assertion ports, **not WPT execution**. Each source below contributes one or more concrete expected results or error-order assertions to `crates/amazing-dom/tests/wpt_assertion_ports.rs`, `src/events_tests.rs`, or the bindings mock tests.

| Directory | Ported source sites | Source files |
|---|---:|---|
| `dom/ranges/` | 9 | `Range-comparePoint.html`, `Range-isPointInRange.html`, `Range-set.html`, `Range-cloneContents.html`, `Range-extractContents.html`, `Range-deleteContents.html`, `Range-surroundContents.html`, `Range-mutations-splitText.html`, `Range-mutations.js` |
| `dom/nodes/` | 6 | `pre-insertion-validation-notfound.js`, `Node-compareDocumentPosition.html`, `Node-normalize.html`, `Node-cloneNode.html`, `Element-closest.html`, `Element-matches.js` |
| `dom/traversal/` | 2 | `NodeIterator-removal.html`, `TreeWalker-acceptNode-filter.html` |
| `dom/events/` | 2 | `Event-stopImmediatePropagation.html`, `AddEventListenerOptions-once.any.js` |
| **Total** | **19** | Native expected-pass candidates only |

One defect was found and fixed: `range_is_point_in_range` and `range_compare_point` called `check_boundary` before comparing roots. On a foreign-root doctype or out-of-range offset, they returned `InvalidNodeTypeError` or `IndexSizeError`; pinned WPT expects `false` or `WrongDocumentError` first. DOM Standard §5.5 `isPointInRange` step 1 and `comparePoint` step 1 require root comparison before doctype and offset checks. The implementation now validates the node handle, compares roots, then validates the boundary. The native regression test includes both foreign-root offset 99 and a foreign doctype.

No defect was found in the ported pre-insert, clone, normalization, splitText live-range, surround partial-node, iterator fix-up, TreeWalker filter cleanup, or event once/immediate-stop cases. The `Element-closest` and `matches` ports use only tag selectors because the mock selector seam intentionally supports tag names; CSS selector conformance and driver installation remain separate. Broader WPT parameter matrices, Shadow DOM, iframe unload mutation, and full event retargeting are not claimed by this native selection.

The host-contract rule in `.agents/dom-bindings/decision-dom7-host.md` applies from DOM-11 onward: new `BindingHost` methods and seam traits need typed Unsupported defaults so the production driver keeps compiling. DOM-11 adds no host methods or seam traits. The decision explicitly leaves existing required methods unchanged.

## DOM-12 WebIDL class strings and pinned `dom/common.js` setup (2026-09-23)

Pinned WebIDL source: [whatwg/webidl `6a53497380036fc4a25b82be889a88e0de7cf448`](https://github.com/whatwg/webidl/blob/6a53497380036fc4a25b82be889a88e0de7cf448/index.bs), `index.bs` sections “Class strings”, “Interface prototype object”, “Stringifiers”, and “DOMException custom bindings”. Pinned WPT checkout: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; inspected `dom/common.js` lines 18–350 in that checkout. This is a binding/fixture audit, not a claim of passing WPT. The 30 shared sources currently register zero subtests after `TypeError: cannot convert object to primitive`; `DOMImplementation-createDocument.html` has four duplicate generated names after `String(arg)`.

`BindingHost::install_interface_class_string` now runs for every `INTERFACES` descriptor. It requests a real `%Symbol.toStringTag%` data property on each interface prototype: value is the qualified interface name, writable false, enumerable false, configurable true. The interface object remains a Function object; the prototype object has its own class string. `HTMLAnchorElement` and `HTMLAreaElement` both install their `href` attribute stringifier as `toString` (length 0); `DOMTokenList` and `Range` already install their own `toString` stringifiers. `DOMException` does not get an own `toString`: its prototype must inherit `%Error.prototype%`, whose `toString` uses `name` and `message`. Location is absent from these bindings. Both new host hooks default to typed `NotSupportedError`; runtime/driver Symbol support is queued in `questions.md`.

`setupRangeTests()` runs before test registration. Every operation executed during its fixture construction is accounted for here; the large range/point/node arrays contain **strings**, so their embedded DOM expressions run later, outside setup.

| Executed setup operation | Binding status |
|---|---|
| `document.querySelector`, optional `parentNode.removeChild` | Bound through `SelectorEngine` and `Node.removeChild`; driver selector adoption remains external. |
| `document.createElement`, `createTextNode`, `createComment`, `createDocumentFragment`, `createCDATASection`, `createProcessingInstruction` | Bound, including `new Document()` for the XML fixture. |
| `testDiv.id =`, `setAttribute(id/style)`, `textContent =` | Bound setters. |
| `document.body`, `firstChild`, `insertBefore`, `appendChild`, `append` | Bound tree/document members. |
| `document.implementation.createHTMLDocument`, `createDocumentType`, `createDocument` | Bound DOMImplementation members. |
| `document.doctype` and `foreignDoc.doctype` | Missing before DOM-12; now bound through direct-child native getter. |
| `paras.push`, array literals, `concat` for range/point/node lists | JS Array runtime operations; not DOM bindings. |
| `"setup" in window`, `setup(setupRangeTests)` | WPT harness/runtime operations; not DOM bindings. |

The setup does not call `Node.compareDocumentPosition` or `isEqualNode`; helpers below it call those only when the registered tests run. The production runtime currently lacks real Symbol-keyed properties and platform-object ToPrimitive dispatch, so the WPT blocker cannot be declared fixed by the mock-host gate. IA-BINDINGS-2 must install these hooks and rerun the pinned selection.

## DOM-13 native Shadow DOM and slot bindings (2026-09-23)

Fixed base `6481c90`. Source pins: [DOM Standard `a2331a45360129e8645ef7e0a04740241b6e3726`](https://github.com/whatwg/dom/blob/a2331a45360129e8645ef7e0a04740241b6e3726/dom.bs), §4.2.2 shadow trees/slots, §4.8 Element/ShadowRoot, and [HTML Standard `cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d`](https://github.com/whatwg/html/blob/cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d/source), valid custom element names and HTMLSlotElement. Pinned WPT checkout on win-001: `53bc812da2ed81179ebd1ba76b2d197cc4747d12`.

A ShadowRoot is a DocumentFragment arena node with private host/options data, so light-tree parent/child operations remain separate. `attach_shadow` validates the HTML namespace, built-in safelist or valid custom name, and duplicate attachment. Open roots are exposed by `Element.shadowRoot`; closed roots remain available to native algorithms. `getRootNode({composed})`, shadow-including ancestry/order, connectedness, host adoption, and clonable root copying are native. The binding surfaces `ShadowRoot` (including `innerHTML` through DOM-9's fragment parser), Element/Text slot members, and HTMLSlotElement methods.

Slot assignment supports first matching named slot, manual ordered assignment, re-assignment on tree/attribute changes, flat and flattened assigned nodes/elements, fallback content, and closed-root hiding for `assignedSlot`. Changes queue a deduplicated `slotchange` signal through the existing MutationObserver native-microtask flag; the checkpoint dispatches slotchange after observer callbacks. Native event parentage follows assigned slots and composed shadow boundaries; per-listener target/relatedTarget retargeting and frozen closed-tree membership govern event paths and `composedPath()`.

Pinned WPT source assertions ported into native/mock tests (these are **not** unmodified WPT passes):

| Source | Ported expected result |
|---|---|
| `shadow-dom/Element-interface-attachShadow.html` | Safelist, duplicate attachment, open/closed return and hiding |
| `shadow-dom/ShadowRoot-interface.html` | Host/mode and root identity; DOM-9 serialization seam |
| `dom/nodes/rootNode.html` | Composed root crosses to document, ordinary root stays ShadowRoot |
| `shadow-dom/slotchange.html` | Assignment changes and fallback changes signal once per checkpoint |
| `shadow-dom/manual-slot-assignment-no-wrong-unassign.html` | Manual order and de-duplication |
| `shadow-dom/event-composed-path.html` and `event-inside-shadow-tree.html` | Retargeted path, closed-tree hiding, non-composed boundary |
| `shadow-dom/event-inside-slotted-node.html` and `event-composed-path-after-dom-mutation.html` | Slotted path crosses shadow; path membership remains frozen during callbacks |
| `shadow-dom/event-with-related-target.html` | Retargeted equal target/relatedTarget suppresses outside listener |
| `shadow-dom/shadow-root-clonable.html` | Clonable shadow subtree copies even when host clone is shallow |
| `shadow-dom/Node-prototype-cloneNode.html` | Cloning a ShadowRoot itself throws NotSupportedError |

The existing `isConnected` and event-parent algorithms were light-tree-only; both were defects for shadow descendants and are corrected. The HTML parser owns declarative shadow DOM, including template attributes and tree-builder attachment; this ticket does not implement it. Custom-element-definition `disable shadow`, scoped registries, CSS layout/rendering of slots, ShadowRoot.activeElement/styleSheets, and MouseEvent's JS-facing relatedTarget remain outside the currently available lower-crate model. Driver wrapper selection must distinguish ShadowRoot from ordinary DocumentFragment via `Dom::is_shadow_root`; the cross-lane request is in `.agents/dom-bindings/questions.md`.

## Optional binding installation hooks (2026-09-23)

On base `9cd9e24` (merged DOM-13), the DOM-12 class-string and DOMException Error-prototype hooks no longer abort installation when an older host lacks those capabilities. Both return the typed `OptionalHookInstall::NotSupported` by default. `interfaces::install` records each skipped capability in `InstallReport::not_installed` and continues installing the rest of the interface surface; actual `Err` values remain fatal. Mock tests cover complete installation with both hooks skipped and propagation of a real hook failure. The runtime/driver can override each hook when it supports the required Symbol key or Error prototype behavior. A skipped hook is not WebIDL conformance credit.

## DOM-14 native Custom Elements and Node.baseURI (2026-09-23)

Fixed base `7801627` (includes OPTIONAL-HOOKS). Pinned normative sources: [HTML Standard `cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d`](https://github.com/whatwg/html/blob/cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d/source), §2.4.1 document base URLs and §4.13 custom elements; [DOM Standard `a2331a45360129e8645ef7e0a04740241b6e3726`](https://github.com/whatwg/dom/blob/a2331a45360129e8645ef7e0a04740241b6e3726/dom.bs), moveBefore and shadow-including tree algorithms. Pinned WPT checkout `53bc812da2ed81179ebd1ba76b2d197cc4747d12`.

`Node.baseURI` selects the first HTML `base[href]` in the owner document's tree, resolves it via `UrlReflection`, and otherwise returns the driver's `DocumentField::URL`. It works for connected and detached nodes, fragments, and Attr nodes. The current URL seam has one input and resolves against the embedding's base; a driver with a relative `<base href>` must ensure it resolves that href against the document's fallback URL, not against itself. The pinned `Node-baseURI.html` selection covers only the document-URL fallback.

The native `CustomElements` registry stores definitions per document, constructor/callback root IDs, observed attributes, disabled features, form association, per-element upgrade state, construction stacks with the already-constructed marker, and reaction frames plus a backup queue. Native insertion/removal/adoption/attribute hooks enqueue lifecycle reactions. `ParentNode.moveBefore` preserves custom element connection and enqueues `connectedMoveCallback`, falling back to disconnected then connected callbacks when absent. Defining a name upgrades matching shadow-including document descendants; `CustomElementRegistry.upgrade(root)` queues disconnected candidates. `Document.createElement` and `createElementNS` synchronously deliver matching upgrades; `createElement(name, {is})` supports customized built-ins. Constructor errors in synchronous creation propagate. Binding calls flush reactions after the DOM borrow, one upgraded element's callbacks before the next element's upgrade. Parser tasks must call the exported `deliver_custom_element_reactions` after native tree mutations.

The binding descriptor exposes `CustomElementRegistry.define/get/getName/whenDefined/upgrade`. `define` validates constructor, name, duplicate name/constructor, and `extends`; uses a definition-running guard while extracting prototype callbacks, conditional `observedAttributes`, `disabledFeatures`, `formAssociated`, and form lifecycle callbacks. `whenDefined` returns one pending promise per name, rejects invalid names, and resolves pending/fresh promises with the constructor. Constructors and callbacks are held through `retain_value`. New host methods default to typed NotSupported. The optional global installer hook records `CustomElementsGlobal` as not installed if the driver does not expose `window.customElements`; its absence never aborts other interface installation.

Pinned WPT **native/mock assertion ports**, not unmodified WPT passes:

| Source file | Ported assertion cases |
|---|---:|
| `dom/nodes/Node-baseURI.html` | 5 fallback cases: connected/detached/fragment element, detached/attached Attr |
| `custom-elements/CustomElementRegistry.html` | 13 validation, lookup, promise, and upgrade-order cases |
| `custom-elements/CustomElementRegistry-getName.html` | 2 known/unknown constructor cases |
| `custom-elements/attribute-changed-callback.html` | 3 add/remove payload and one-callback-per-set cases |
| `custom-elements/connected-callbacks.html` | 1 insertion case |
| `custom-elements/disconnected-callbacks.html` | 1 removal case |
| `custom-elements/adopted-callback.html` | 1 adoption reaction case |
| `custom-elements/custom-element-reaction-queue.html` | 1 per-element upgrade/reaction ordering case |
| `custom-elements/Document-createElement-customized-builtins.html` | 1 `{is}` construction case |
| `custom-elements/customized-built-in-constructor-exceptions.html` | 1 synchronous constructor failure case |
| **Total** | **29 cases from 10 files** |

The assertion ports found two defects: `Element.setAttribute` changed the same attached Attr twice to preserve UTF-16 sidecar units, which queued two `attributeChangedCallback`s; it now updates the sidecar without a second mutation. Flat reaction delivery ran all upgrades before any of their callbacks; it now drains each upgraded element's queued callbacks first. The older `Node.baseURI` missing getter caused IA-BINDINGS-2's `Node-baseURI.html` pass-to-fail regression.

Installation limits: the production driver must install the optional global, implement constructor/object/sequence conversion, construction and promise hooks, call `Dom::custom_element_super` from the HTMLElement constructor, and report callback exceptions. Until then the feature is proven natively and in the mock host only. Scoped `CustomElementRegistry()` construction/`initialize`, custom element registry association with scoped shadow roots, form-associated internals/callback dispatch, and parser-side synchronous construction hooks are beyond this ticket's named methods. `observedAttributes` presently passes through the DOM's scalar `String` attribute storage, so lone-surrogate names need a future UTF-16 attribute model. No real `custom-elements/` WPT pass number is claimed.

## DOM-15 form controls (2026-09-23)

Fixed implementation base: local `dev` merge `d887b17c91dd61f598f2dc94edb547bbefa9174e` (the fetched `origin/dev` lagged the merged DOM-14 prerequisite). Windows lane tree refresh lock: `8262C312`. Pinned normative source: [HTML Standard §4.10 at `cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d`](https://github.com/whatwg/html/blob/cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d/source), especially the input value-mode and dirty flags, form-owner reset, select/option selectedness, and constraint-validation algorithms. Pinned WPT source revision: [`53bc812da2ed81179ebd1ba76b2d197cc4747d12`](https://github.com/web-platform-tests/wpt/tree/53bc812da2ed81179ebd1ba76b2d197cc4747d12/html/semantics/forms).

Native form state is attached to DOM arena nodes. Input value modes, checkedness, radio grouping by form owner/name/tree root, type sanitation, temporal number/date conversion, textarea API values, option selectedness, selection data, live form owner association, and validity flags live in `forms.rs`. Form and select collection wrappers use DOM-8 legacy hooks; `RadioNodeList` is returned for duplicate form-control names. `reset()` fires a cancelable bubbling reset event before resetting controls. `checkValidity()` and `reportValidity()` fire a cancelable nonbubbling invalid event, with no UI. New optional pattern-regex and Date host methods default to typed `NotSupportedError`, so the older driver compiles and installation continues.

Native/mock assertion ports cite these pinned WPT files: `the-input-element/valueMode.html`, `the-input-element/input-type-change-value.html`, `the-input-element/radio.html`, `the-input-element/input-valueasnumber.html`, `the-input-element/input-valueasdate.html`, `the-select-element/selected-index.html`, `constraints/form-validation-validity-valueMissing.html`, `constraints/form-validation-willValidate.html`, and `constraints/form-validation-validity-patternMismatch.html`. These are selected assertion ports, not execution of unmodified WPT. The driver `cargo check -p amazing` passes, but runtime use of new brands and hooks needs the integrator adapter changes listed in `.agents/dom-bindings/questions.md`; no browser WPT pass count is claimed.

Remaining form work is explicitly outside this ticket: constructing a submission entry list, `FormData`, submit/requestSubmit, encodings, navigation/fetch, file selection and uploads, focus/UI validation bubbles, user editing events, and interactive selection painting. `badInput` is false without an editing UI or failed user parse state. The pattern matcher and Date objects require the driver host seams; the mock host tests them with a small deterministic fixture. Full email/URL validation, calendar edge cases, and step rounding still need broader WPT coverage before conformance can be claimed.


## DOM-16 installed WPT `dom/` reduction (2026-09-23)

Fixed base: fetched `origin/dev` merge `8f2695d` (includes the separately delivered inert-template fragment). Local DOM-16 commits: `6cf4302`, `8ff2977`, `7284511`, `c4c3d92`; primary granted one runner-file exception, committed as `0aea9c6`. The Windows lane tree was refreshed to this exact base, lock SHA-256 prefix `BF3037D0`, and all changed files were re-synced. Pinned WPT revision: [`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`](https://github.com/web-platform-tests/wpt/tree/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/dom); selection SHA-256 `9787de028efe5dc83f491ce2a818e2a549b4a07a236e6538c1e6165bb437b0b5`, 627 selected source files and 192 excluded, viewport 800×600, upstream known-pass/known-fail proof verified twice for each pair. The installed runner's new `--timeout-multiplier 2` option was used for both pairs and recorded in each summary: normal deadline 20,000 ms, long deadline 120,000 ms. Its default remains 1. The prior fixed 10-second deadline crossed the large `Document-createElementNS.html` source's 10.83–11.15 second execution time and caused unstable partial subtest output on this base; the two 10-second attempts are diagnostic only.

| Stable two-round WPT gate | BEFORE on `8f2695d` | AFTER on `0aea9c6` |
|---|---:|---:|
| Passing files | 77 | 84 |
| Failing / timeout / error / unsupported files | 259 / 83 / 75 / 133 | 254 / 83 / 73 / 133 |
| Passing / failing subtests | 1,775 / 2,715 | 2,203 / 2,287 |
| Outcome SHA-256 (both rounds) | `226bcb106411a1402d3a9392068ddf248e53422aaba22d43daf461c9a827e4bc` | `b3ca4a40d25ae25f702dc8e3dd928a3231a1f92fba10599e9457b506490ccc6e` |
| Raw SHA-256 (both rounds) | `24aae5ea18173fb864357726a5dba6940a887b3128ede97bc41af63fefb5fbe0` | `dec28f92b4061bd8307c81af0c9c67fe3a0b61a8a2731e4fb58cfa4bb0b1a029` |

File transitions: seven fail→pass (`CharacterData-surrogates`, `Document-getElementsByTagNameNS`, `DocumentType-literal`, `Element-getElementsByTagNameNS`, `MutationObserver-childList`, `Node-isSameNode`, `Node-lookupNamespaceURI`); two error→fail (`DOMTokenList-coverage-for-attributes`, `DOMImplementation-createDocument`); zero pass→fail. Subtest net gain: **428**. By exact file/name/occurrence there are 433 fail→pass, 1,638 pass→pass, and zero pass→fail for unchanged names. In `DOMTokenList-coverage-for-attributes.html`, 137 former passes had generated names saying `undefined` tag and namespace because those Element getters were absent. The correct getters activate real tag/namespace names; 132 of the newly named cases pass and five fail (two foreign-namespace anchors awaiting driver interface adoption, three HTML token-list class-string assertions awaiting production class-string behavior). These are newly exercised assertions, not evidence of a formerly correct behavior regressing. The file has 175 assertion positions in both runs; 137 formerly passing positions received corrected generated names. The native bindings now return live canonical lists for the three HTML attributes.

Bindings install Element namespaceURI/prefix/localName/tagName, DocumentType name/publicId/systemId, CharacterData UTF-16 indexed methods and `Text.splitText`, and Node namespace lookup/default check/normalization/identity methods. Native DOM namespace lookup follows element names/declarations and ancestors; doctype-name validation follows the permissive DOM rule. `Document.createElement` now validates a local name without treating colons as QName separators and lowercases ASCII in HTML documents. Empty attribute namespace strings normalize to null. The new HTML `htmlFor`, `sandbox`, and `sizes` getters return attribute-backed DOMTokenLists. Native/mock tests assert these operations and errors. The unchanged pinned WPT sources gained full-file passes in `Node-lookupNamespaceURI` (0→75) and `Node-isSameNode` (0→9), and substantial subtest gains in doctype, CharacterData and creation cases.

Remaining measured clusters are routed in `.agents/dom-bindings/questions.md` with first-failure counts. In particular, XML/XHTML iframe document setup, constructed XMLDocument metadata, DOMTokenList class strings, Window globals, event interfaces, JS ToPrimitive/constructor behavior, CSS selectors, and tentative OpaqueRange geometry need their owners. `Document.adoptNode` still lacks an installed binding, so the fixed `createElement(":good:times:")` validation advances that file to its next blocker without producing a file pass. The owner-ranked table and raw transition details are retained in the lane report and `.agents/dom-bindings/dom16-*-x2/` evidence.

Fixed-base Windows gate after source sync: owned nextest **259/259** passed with no skips; owned doctests **0/0** in each of two suites. Slot-protected workspace nextest **1,716/1,716** passed with 3 skipped; workspace doctests **2/2** passed. These are `8f2695d` plus DOM-16 source results; the workspace count includes sibling changes in that base and is not attributed solely to DOM-16. A later fetch of `origin/dev` at `2c1a3e8` showed no own-path drift, so no moving-head regate was needed.

## DOM-17 URL, binary body, form reflection, and FontFaceSet bindings (2026-09-24)

Fixed base `f8bdb33`; implementation commits `ea958fb` and `9c955be`. Pinned WPT revision [`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`](https://github.com/web-platform-tests/wpt/tree/c2a2fce69e5c2509d9dc28440ec8b42532e5c293). The lane installed URL/URLSearchParams, Blob/File, FormData and synchronous `document.fonts` bindings, native state and wrapper reuse in the driver, form-control attribute reflection, and wptserve-compatible window wrappers for `.any.js`. The WebIDL host hooks default to typed NotSupported when their JS facade is absent. ArrayBuffer/Uint8Array construction is routed to JSB-1; pair iteration is routed to JSRT-7's `binding_register_pair_iterable_hooks(brand, PairIterableHooks { count, pair_at })` and `binding_define_pair_iterable`. Font loads currently finish synchronously, so an absent pending-load hook reports `loaded` and resolves `ready`.

Pinned URL selection: 14 files, selection SHA-256 `21a3a253580f75efa4e5c4a209d7c591a134021d6765388ca8c1ba346622bc49`. On the fixed base, both rounds had **0 pass, 13 fail, 1 error; 0 passing/92 failing subtests** (outcome `9ab54e3dcf7e4521e606423c1cb9aa6af13b0709f2d8a99f8a0abb8095ec0aac`). With DOM-17, both rounds had **8 pass, 5 fail, 1 error; 59 passing/33 failing subtests** (outcome `52b9ce05488a37c5c4519ae0c9916677d0d063b8697c17b075944f5a739a3506`). There were no prior URL passes to lose. Remaining URL failures include generic iterable/byte-buffer facades and some unhandled data-loading rejections in constructor/setter fixtures. The shared net model correctly removes the empty query after deleting all URLSearchParams entries.

Pinned installed `dom/` selection: 627 files, selection SHA-256 `5b835d055856a62ba3aa8f52836e65d0ccd42356b1444bfa738029e8b4181bd8`, timeout multiplier 2. Base and DOM-17 each produced the same result twice: **87 pass, 252 fail, 84 timeout, 71 error, 133 unsupported; 2,252 passing/2,239 failing subtests**, outcome `be9321d322008d3a09f23495af75b0667c157bc37de656a2568f75418cd1dc31`. Exact file and subtest-name/occurrence comparison found **zero losses** and zero classification changes.

The eight pinned `css-fonts` reftests in text's FONT-FACE-1 evidence moved from six `document.fonts.ready.then` crashes plus two mismatches to **one pass, seven rendered mismatches, zero crashes**. Thus all six requested cases reached image comparison; the five remaining mismatches belong to text/rendering. The win-002 fixed-commit final gate passed owned nextest **263/263** (0 skipped), owned doctests **0/0** in both crates, workspace nextest **1,792/1,792** (4 skipped), and workspace doctests **2/2**. Windows Cargo.lock SHA-256 was `C47ADE258AB0D41FE91704AE2C58F8BFE78863268FBD2D8FC0C8773FD75D3F76`.

## DOM-18 declarative shadow root model and serialization (2026-09-24)

Fixed base `f8bdb33` plus DOM-17 (`48adffe`). Pinned [DOM Standard](https://dom.spec.whatwg.org/) §4.8 and [HTML Standard](https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html) `getHTML`/fragment serialization. `ShadowRootData` now records declarative, ElementInternals availability, and keep-null custom-registry flags alongside the existing mode, slot assignment, delegatesFocus, clonable, and serializable options. `attach_declarative_shadow` is the DOM-owned parser seam; a later same-mode `attachShadow()` consumes and clears a declarative root. Cloning and parser-arena fragment import preserve its flags and children. Element and ShadowRoot expose `getHTML({serializableShadowRoots, shadowRoots})`; serialization emits selected roots as `<template shadowrootmode>` before host light children, including the specified option attributes. Native/mock tests cover the flags, replacement, clone/import, own-host selection, explicit closed roots, and ordering. The `shadowRoots` sequence host hook defaults to typed NotSupported until driver adoption.

Pinned WPT revision [`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`](https://github.com/web-platform-tests/wpt/tree/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/shadow-dom/declarative), 20 top-level `shadow-dom/declarative/*.html` files (excluding support and tentative subdirectories), selection SHA-256 `0d4427d4038b9ed7d7207352be1e8ddfd54e8ad0cc0f9c7fe8317c454c317ec2`, 800×600, 20-second deadline. The DOM-17 executable `1910a660…` and DOM-18 executable `04e4eb11…` both classified **0 pass, 8 fail, 11 error, 1 timeout**, with **2 passing/38 failing subtests**. Exact file/subtest outcome SHA-256 was `3cea3b7d671d65a3722c09a6ac5cb14117c1492f35a1d009c406b12b51e10a51` for both. One `gethtml-ordering.html` inline-script error detail advanced from `value is not callable` to `[object]`, but its error classification and zero subtests were unchanged. No unmodified WPT pass is claimed until HTML-17 calls the parser seam and the production driver adopts `shadowRoots` sequence conversion; both requests are recorded in `.agents/dom-bindings/questions.md`.

The win-002 final gate on the lane worktree passed owned nextest **267/267** with no skips and owned doctests **0/0** in both crates. Workspace nextest passed **1,796/1,796** with four skipped (1,800 listed tests); workspace doctests **2/2** passed. The win-001 WPT measurement built the changed driver at source SHA-256 `9250f5c4d79205eb105fc91160c598e4375735ba76490358faaf0aff5a3ebcf4` and pinned WPT revision above. No classification loss was observed.

## DOM-19 document adoption/import and NamedNodeMap legacy properties (2026-09-24)

Fixed base `f8bdb33` plus DOM-17/18; implementation `f50603e`. `Document.adoptNode` and `Document.importNode` are installed through WebIDL bindings. Native import clones into the destination document without adopted custom-element reactions, including template contents and clonable shadow roots. Adoption updates attached Attr nodes' owner documents and rejects ShadowRoot with HierarchyRequestError. The production driver registers indexed and named NamedNodeMap hooks using the binding crate's native attribute-node source. Added driver code is confined to new lines in `define_legacy_platform_object`, as authorized. Mock/native tests cover these paths.

Pinned WPT revision [`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`](https://github.com/web-platform-tests/wpt/tree/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/dom), 627 selected `dom/` sources, selection SHA-256 `5b835d055856a62ba3aa8f52836e65d0ccd42356b1444bfa738029e8b4181bd8`, 800×600, normal timeout multiplier 2. The DOM-18 base executable `04e4eb11…` twice classified **87 pass, 252 fail, 84 timeout, 71 error, 133 unsupported; 2,252 passing/2,239 failing subtests**, outcome `be9321d322008d3a09f23495af75b0667c157bc37de656a2568f75418cd1dc31`. The DOM-19 executable SHA-256 `3122CE16052D6CCAFA6663C51092800F58E2E88002E8D8C2A0ED6B708A0B601C` twice classified **92 pass, 248 fail, 83 timeout, 71 error, 133 unsupported; 2,291 passing/2,201 failing subtests**, outcome `07d8372f07f8094ef70fc6b5e3cbe70159d1a465ca117bbbb7ed7eb4afbd18a5` and raw `16cc1928a15ff1b80e4436ba3a09b8977f7f57dffa4f4093c2b6afd257761f21`. The two candidate rounds were byte-identical in both hashes.

Exact file/name/occurrence comparison found **five fail→pass files, 38 fail→pass subtests, one newly introduced passing subtest, and zero lost file or subtest passes**. New full-file passes: `Document-importNode.html`, `Element-removeAttributeNS.html`, `Element-tagName.html`, `Node-mutation-adoptNode.html`, `attributes-namednodemap.html`. `MutationObserver-attributes.html` moved timeout→fail while its subtests advanced 26/15→28/14, with no lost pass. Focused adoption/import cases: import 0/5→5/5, mutation adoption 1/2→2/2, direct adoption 0/4→3/4. The last direct adoption failure is `assert_throws_dom` reporting “value is not callable”; the TypeError-chain issue is routed to js-runtime in lane questions.

The win-002 final fixed-source gate passed owned nextest **269/269**, owned doctests **0/0** in both crates, workspace nextest **1,798/1,798** with four skipped, and workspace doctests **2/2**. The candidate WPT executable was built from the same final DOM-19 source before DOM-20 edits began. No push.

## DOM-20 Element attribute queries and classList assignment (2026-09-24)

Fixed base `f8bdb33` plus DOM-17/18/19; implementation `3d95636`. Element now installs `hasAttributes`, `getAttributeNames`, `hasAttribute`, `hasAttributeNS`, and `toggleAttribute`. The qualified-name query checks attributes regardless of namespace, while the namespace query uses expanded identity. `setAttribute` validates its name and updates the first matching qualified-name attribute without changing that attribute's namespace. HTML ASCII name folding is restricted to HTML elements in HTML documents, so SVG attributes retain case. `classList` has a setter forwarding to the existing DOMTokenList value path, preserving its canonical live object. Mock/native tests cover namespace matches, toggle force, invalid names, SVG case, and classList setter metadata and effect.

Pinned WPT revision [`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`](https://github.com/web-platform-tests/wpt/tree/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/dom), 627 selected `dom/` sources, selection SHA-256 `5b835d055856a62ba3aa8f52836e65d0ccd42356b1444bfa738029e8b4181bd8`, 800×600, timeout multiplier 2. The DOM-19 fixed baseline executable `3122CE16052D6CCAFA6663C51092800F58E2E88002E8D8C2A0ED6B708A0B601C` had **92 pass, 248 fail, 83 timeout, 71 error, 133 unsupported; 2,291 passing/2,201 failing subtests**, measured twice. The DOM-20 executable `E06FF1D971852A063592A003DA8797A473324345BCED5B89B81848FE0596EFBC` twice classified **96 pass, 244 fail, 83 timeout, 71 error, 133 unsupported; 2,315 passing/2,177 failing subtests**. Both candidate rounds had identical outcome hash `bd4a0eafc76a1fc4522c0ce1a3519bd9fcf8901212869c6d7abdc4d5d497ed73` and raw hash `06706b2c639e999742a454f552000e41be6ed01f80f6a8fd07297eb50a603b1b`.

Exact file/name/occurrence comparison found **four fail→pass files, 24 fail→pass subtests, and zero lost file or subtest passes**. New full-file passes: `DocumentFragment-getElementById.html`, `Element-hasAttribute.html`, `Element-hasAttributes.html`, `Element-setAttribute.html`. `Element-classlist.html` gained the pinned “Assigning to classList (HTML node)” assertion (fail→pass); its remaining `.supports()` TypeError subtype failure belongs to the already routed JS error-chain issue. `attributes.html` gained 17 passing subtests. Focused `hasAttribute` and `hasAttributes` went 0/2→2/2 each, and `setAttribute` went 1/1→2/0. Existing focused NamedNodeMap and removeAttributeNS passes remained green.

The win-002 final gate at `3d95636` passed owned nextest **270/270**, owned doctests **0/0** in both crates, workspace nextest **1,799/1,799** with four skipped, and workspace doctests **2/2**. Linux owned nextest 270/270, doctests, and `cargo check -p amazing` also passed. `getAttributeNames` currently reflects the native split attribute storage's order; cross-namespace insertion order is a native model follow-up. No push.

## DOM-21 attribute insertion order (2026-09-24)

Fixed base `f8bdb33` plus DOM-17 through DOM-20; implementation `f2441dd`. Native Elements now preserve one insertion order across plain and namespaced attributes for `getAttributeNames`, NamedNodeMap, cloning, and serialization. Replacing an attribute retains its position; removing and readding puts it last. Two native tests cover mixed namespaces and removal. Pinned `dom/nodes/attributes.html` gained “First set attribute is returned with mapped attribute set later” (47 pass/20 fail to 48 pass/19 fail).

The pinned 627-file `dom/` selection, revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, selection hash `5b835d055856a62ba3aa8f52836e65d0ccd42356b1444bfa738029e8b4181bd8`, measured twice at 800×600 and timeout multiplier 2. Compared with the twice-stable DOM-20 baseline, both candidate rounds classified **96 pass, 244 fail, 83 timeout, 71 error, 133 unsupported files; 2,316 passing/2,176 failing subtests**. Both rounds had outcome hash `51163f77cb01445494c3e6b02e34050394d085c176019929ce6fbc8144402e45` and raw hash `2e1f98fcff71accd6581d4713160e5d5d05e5ef8d96badc325ffb57b1ec20881`. Exact file/name/occurrence comparison found one fail→pass subtest, zero lost passes, and unchanged file classifications. Candidate executable SHA-256 `17F6ABDBF01F59D8FBD3F37C8FF3F32A74D9F3199D76525C9538C9CA4965D5B8`.

Win-002 final gate on `f2441dd`: owned nextest **272/272**, owned doctests **0/0**, workspace nextest **1,801/1,801** with four skips, workspace doctests **2/2**. No push.

## Priority WebIDL [PutForwards] and [Global] receiver fixes (2026-09-24)

Implementation `6ef5729` on fixed base `f8bdb33` plus prior DOM tickets. Token-list assignment forwards through `value` for classList, relList, sandbox, sizes, and htmlFor. A generic amazing-js `binding_install_put_forwards` facade forwards Element.style to cssText and Window.location to href. Nullish receivers for the legacy Window EventTarget native methods use the Window global. Added bindings and driver tests cover these cases. Win-002 final gate: **623 owned** tests, **1,804 workspace listed** with four skips, and **2/2 doctests**, all passing.

The pinned 627-file `dom/` selection at WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, selection SHA-256 `5b835d055856a62ba3aa8f52836e65d0ccd42356b1444bfa738029e8b4181bd8`, ran twice at 800×600 with timeout multiplier 2. Both candidate rounds classified **96 pass, 244 fail, 83 timeout, 71 error, 133 unsupported files; 2,316 passing/2,176 failing subtests**, unchanged from the twice-stable DOM-21 baseline. Both candidate rounds had outcome SHA-256 `51163f77cb01445494c3e6b02e34050394d085c176019929ce6fbc8144402e45` and byte-identical raw SHA-256 `980b3da7895fb586eed1f449520d629819cc6354950b1d10a04c8c5a22aec460`. Exact file/name/occurrence comparison found zero lost passes. Executable SHA-256 `7E89DFD9266B3D781B10D10E6DF192E0D2CFDB637F122A3E8EE84A09ADD992B3`. No push.

Diagnostic follow-up: the pinned css-fonts `font-variant-ligatures.html` parses and `document.getElementById("content")` succeeds before its inline script. Its table builder uses `document.createElement("td").style.cssText`; the detached cell's `style` is nullish because `RuntimeBindingHost::preserve_legacy_element_extensions` skips disconnected elements. The exact driver function and probe are routed to integrator-assist in lane questions.

## DOM-20/21 inert documents, mutation journal, detached style, and Location follow-up (2026-09-24)

Fixed-base implementation commits `70d6c98` (inert documents), `9fdf663` (ADR 0010 render mutation journal), and `d10e758` (Location, detached-element extensions, constructed-Document metadata). The lane was then rebased onto `e9ddbcc`; corresponding final head is `5140196`. The journal records tree, attribute, text, state, and shadow-slot mutations by epoch with node generations and a whole-document fallback. `DOMParser.parseFromString` and the shared-arena inert-document hook expose metadata without a browsing context. `Location` exposes URL components, setters, navigation methods and an empty top-level `ancestorOrigins` DOMStringList; Window and Document assignments forward to `href`. Disconnected `createElement('td').style` now retains a live legacy style object. A constructed XML `Document` receives `about:blank`, `application/xml`, and `UTF-8` metadata. Native and driver regressions cover these paths.

The fixed-base `d10e758` WPT gate used pinned `dom/` revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the installed 627-source selection, 800×600 viewport, and timeout multiplier 2. Both candidate rounds were byte-identical: **99 pass, 236 fail, 82 timeout, 77 error, 133 unsupported files; 2,548 passing and 1,935 failing subtests**. Outcome SHA-256 `e60154ece465dd8b4dba641a09f50ac62c16b7fa251d8a4d5f4f120a68df0a98`; raw SHA-256 `419ecea330fa92752afac658cf60a9e85cd24ca5f2175cb608e2f917b22aa7b0`; executable SHA-256 `4e93796c8e4c40311cd70c6615fb9ec07b81d21ac6c68d29fa8c95d2c0ea83f3`. Against the twice-stable priority-WebIDL baseline (96 passing files, 2,316 passing subtests), exact file/name/occurrence comparison found **232 fail-to-pass subtests, three fail-to-pass files, zero lost file or subtest passes**. Five zero-pass files changed fail→error and one zero-pass file timeout→error as execution advanced; both candidate rounds agreed. `Document-constructor.html` retained its passing URL-parsing and children assertions and gained its metadata assertion. The diagnostic `70d6c98` sweep had crashed this file from a RefCell borrow in `RuntimeBindingHost::document_field`; `d10e758` fixes the borrow and the missing constructor metadata.

Final `5140196` win-002 gate passed owned crate and workspace nextest plus owned and workspace doctests; the rebased inventory lists **437 owned / 2,117 workspace tests**, with **2/2 workspace doctests**. Linux owned-area nextest passed. Rebased exact `font-variant-ligatures.html` still stops at `support/js/font-variant-features.js:209` `cp.toString(16)`: `cp` is a number, `typeof cp.toString` is `undefined`, while `Number.prototype.toString` is a function. `Interpreter::get_prop_receiver` does not look up Number prototype properties for numeric primitives. Bypassing only that call reaches `span.innerHTML = ...`, where `RuntimeBindingHost::parse_fragment` passes unsupported `span` context to `amazing_html::parse_fragment`. Both exact handoffs are in lane questions for js-runtime and html/integrator-assist. The six font reftests remain blocked before rendered comparison. No push.

## Document constructor interface and charset aliases (2026-09-24)

Fixed base `e9ddbcc`; implementation `5c0651a`. A constructed `Document` now retains the `Document` brand even though its default content type is `application/xml`; `DOMImplementation.createDocument` retains `XMLDocument`. `Document.charset` and `inputEncoding` alias `characterSet`. The pinned `dom/nodes/Document-constructor.html` improved from 3/5 to **5/5** passing assertions, with driver tests for both document brands.

The pinned 627-file `dom/` selection at WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600 and timeout multiplier 2, ran twice on win-001 at exact `5c0651a`. Both rounds were byte-identical: **117 pass, 227 fail, 85 timeout, 65 error, 133 unsupported files; 2,958 passing and 1,671 failing subtests**. Outcome SHA-256 `909ffc75e6455ba0cf88608e89e225a43a01fce321a9c8f0c47e4cb4fd55d79e`; raw SHA-256 `4db219751370b51f6ea8014ac9a5b29b2c257a63628776b0c7c186954d6a2ad7`; executable SHA-256 `88b375487d1c7e0b1f70245af9cafc3602ad6c36153080a6aa4dcaf75d4edcb2`. The earlier `d10e758` gate was on a pre-rebase base: 99 passing files and 2,548 passing subtests. Exact file comparison shows 18 more passing files and zero lost file passes. Assertion-position comparison shows 404 newly passing positions and zero lost positions. The JS base changed generated names at 24 still-passing positions (mostly U+FFFF escape formatting), so name-based matching alone reports 24 missing names. The before/after delta includes upstream changes from the rebase and is not attributable solely to this constructor fix.

Win-002 owned-area and workspace nextest/doctests passed on `5c0651a`; the first workspace run encountered a transient `amazing-process` sandbox `host.ping` `UnknownInterface`, which passed both targeted retry and full workspace retry. Linux owned-area nextest passed. No push.

## ADR 0010 node slot generations for RC-19 (2026-09-24)

Implementation `42a1e19` exposes `Dom::node_generation(NodeIdx) -> Option<u64>`. Each live slot starts at generation 1; explicit retirement makes the slot return `None`, and reuse increments that slot's generation. `RenderMutation.target` already stores `NodeGeneration`, so queued entries retain their original identity across reuse. Reclamation is an internal GC seam and requires the caller to prove no DOM side table or script wrapper can still reach the node; ordinary detach never retires it. The reuse test checks the old and new journal identities and the free-slot `None` state.

Linux and win-002 `amazing-dom` nextest passed **211/211**, doctests **0/0**. Win-002 exact-commit workspace nextest and doctests passed. The generation change does not alter script-visible DOM behavior; the twice-stable constructor corpus above is the preceding fixed-commit render baseline. No push.

## ParentNode and element sibling navigation getters (2026-09-24)

Implementation `daf202d` installs `firstElementChild`, `lastElementChild`, and `childElementCount` on Document, DocumentFragment, and Element, plus `previousElementSibling` and `nextElementSibling` on Element and CharacterData. Native traversal skips non-elements and reflects tree mutations. Native/mock tests cover mixed SVG/HTML children, text siblings, detach, and null results. Linux owned nextest passed **280/280**, owned doctests **0/0**, and all 11 pinned focused HTML files in the element-navigation cluster passed (each failed on the prior baseline).

The exact-commit win-001 pinned `dom/` gate used WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 627 sources, 800×600, timeout multiplier 2, and executable SHA-256 `5AD2A260B84BF480001E5D83A9D3F8B27D7522DA24F4452EB0CE692DF3A97593`. One complete candidate corpus round classified **130 pass, 214 fail, 85 timeout, 65 error, 133 unsupported files; 2,972 passing and 1,657 failing subtests**. Outcome SHA-256 `dd3227be929c299af64a5b54443c6802e640c049cc777eadf3b96bc30df6011e`; raw SHA-256 `33c945696ce327e0c1e7e8c14b372d622336e9d66348c7dd95f0ca57664ac3c7`. Against the twice-stable preceding constructor gate, there are **13 fail→pass files, 14 fail→pass subtests, zero lost file or subtest passes**. Eleven file gains are the named element-navigation cluster. `Node-cloneNode-svg.html` and `Range-intersectsNode-shadow.html` also pass because their setup uses the newly installed getters. The runner began a second round automatically; it was stopped after the complete first round under the new one-run bulk gate convention.

Win-002 exact-commit owned nextest **280/280**, owned doctests **0/0**, workspace nextest **2,120 listed** with its normal skips, and workspace doctests **2/2** all passed. No push.

## Node.normalize exclusive Text semantics (2026-09-24)

Implementation `97750f4` changes `Node.normalize()` to merge/remove only exclusive Text nodes. The shared `is_text` predicate also includes CDATASection for other DOM operations, but DOM normalization must leave CDATASection, comments, and processing instructions intact. A native regression checks mixed nodes and boundaries. Linux owned nextest passed **281/281**, owned doctests **0/0**; pinned `dom/nodes/Node-normalize.html` improved **3/4→4/4**.

The win-001 exact-commit DOM corpus ran one complete 627-file candidate round at WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600, timeout multiplier 2. Candidate: **131 pass, 213 fail, 85 timeout, 65 error, 133 unsupported files; 2,973 passing and 1,656 failing subtests**. Outcome SHA-256 `7bf29ba607a893391d0def4393c946349a7f182ce575bfba6bf77f7d416cc75c`, raw SHA-256 `9567c1699768dfde94abce4b48437215d28c6e1707c21901567e8d8679c4f3e4`, executable SHA-256 `F41C6524ADC621FD329F1114636F4A1358B391B9F3C1BB095C1F556D1489879E`. Compared with preceding `daf202d`, exactly one file and one subtest changed fail→pass; zero lost passes. The automatically started second runner round was stopped after the completed first round under the one-run bulk gate convention.

Win-002 exact-commit owned nextest **281/281**, owned doctests **0/0**, workspace nextest **2,121 listed** with normal skips, and workspace doctests **2/2** passed. No push.

## MutationObserver callback, takeRecords, and targeted attribute writes (2026-09-24)

Implementation `b9f2220`, `734ddba`, `1b7ee92` passes the observer as callback `this`, makes DOMTokenList writes queue one attribute record, and changes the authorized `BrowserDomHost::set_attribute` path to set only the requested attribute. The final driver regression checks that writing the same `id` queues a record with its old value and that a subsequent `class` write queues no unrelated `id` record. The intermediate DOM `set_attrs` skip was reverted because it suppressed a required same-value record.

At pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the exact-commit win-001 627-source `dom/` corpus (800×600, timeout multiplier 2) changed from **131 pass, 213 fail, 85 timeout, 65 error, 133 unsupported; 2,973 passing/1,656 failing subtests** to **133 pass, 211 fail, 85 timeout, 65 error, 133 unsupported; 2,987 passing/1,642 failing subtests**. `MutationObserver-inner-outer.html` and `MutationObserver-takeRecords.html` changed fail→pass; 14 subtests changed fail→pass, with **zero file or subtest pass losses**. The passing `attributes Element.id: same value mutation` assertion remained passing. Candidate raw SHA-256 `eddfa2cc795ceea3ec8fc25a52401ead0d4ce1177cd6cc898e74e7f3e127cd20`, outcome SHA-256 `4599c66ca825bda27444366e44d595fff05c9d397d333ff8f0d6c3a977f556ef`, executable SHA-256 `F99D7B6DAB73D3FD0D17BB5C98AD4DD2E119B372153E04318670C689E4EF7399`. The runner's second round was stopped after one completed round under the bulk gate convention.

Linux owned-area nextest passed **440/440** (including amazing driver tests), owned doctests **0/0**. Win-002 exact-commit owned nextest passed **282/282**, owned doctests **0/0**; workspace nextest and doctests passed. `MutationObserver-callback-arguments.html` remains blocked by amazing-js arrow capture of `arguments`, routed to JSRT-7 in `.agents/dom-bindings/questions.md`. No push.

## Forced DOMTokenList.toggle no-op mutation (2026-09-24)

Implementation `bb36af6` returns from `DOMTokenList.toggle(token, force)` when force requests the token's existing state, avoiding a spurious attribute mutation. A native binding test observes both forced no-op cases. Pinned `dom/nodes/MutationObserver-attributes.html` now passes all 42 subtests (previously 40/42). In the fixed 627-source WPT `dom/` selection (revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600, timeout multiplier 2), files changed **133 pass/211 fail → 134 pass/210 fail** with 85 timeout, 65 error, 133 unsupported unchanged; subtests changed **2,987 pass/1,642 fail → 2,989 pass/1,640 fail**. Exactly one file and two subtests changed fail→pass, with **zero losses**. Candidate raw SHA-256 `954e61cee4e728a417925e72991b12ec35f7742eb011070f634454c8df7ca6a7`, outcome SHA-256 `d35e96bd4012518ec3a842039bf8efdbc5541fa983a63388a4454bf5274dafe4`, executable SHA-256 `284A45AC87F0E170B22BC0B19F359BB4A3E648E908E139C10E229277777D0F92`. One complete candidate round; the automatic second round was stopped under the bulk gate convention. Linux owned-area nextest **441/441** and doctests passed; win-002 exact-ref owned nextest **283/283**, workspace nextest and doctests passed. No push.

## ProcessingInstruction.target getter (2026-09-24)

Implementation `f8527f9` installs the readonly WebIDL `ProcessingInstruction.target` getter with receiver validation and a binding regression. The pinned `dom/nodes/Node-textContent.html` improved **80/81→81/81**. In the fixed 627-source DOM WPT selection above, files changed **134 pass/210 fail → 135 pass/209 fail** with 85 timeout, 65 error, and 133 unsupported unchanged; subtests changed **2,989 pass/1,640 fail → 2,993 pass/1,636 fail**. The four gains are the `Node-textContent` assertion and three `Document-createProcessingInstruction` target assertions; **zero losses**. Candidate raw SHA-256 `9eaa805cd5b686b2e1e4570ed5391abd20c382f9f61b79f2caeb6c3a9960d933`, outcome SHA-256 `67bd10990097a3ebd89922d8899e0c2c873540aec2e45fa4c6685d8c04f0f29b`, executable SHA-256 `31451E92E8207E1D8630673077B7E2568DE42573C0F2AA4BE75FA9B3584BC9D8`. One complete corpus round; the automatic second round was stopped. Linux owned-area nextest **441/441**, owned doctests and focused WPT passed. Win-002 exact-ref owned nextest **283/283**, workspace nextest **2,124 listed**, and workspace doctests passed. No push.

## DOM-B bulk nodes, events, and ranges (2026-09-24)

Implementation `6384359` covers three highest failing pinned `dom/` clusters by subtest count (nodes, events, ranges): Node equality/position and qualified-name reflection, event subclasses and legacy initialization, Window's native EventTarget path and default-passive listeners, Selection backed by a live Range, and image/source reflection with loader-state and relevant-mutation seams. It also exposes `Dom::has_blocking_listener(EventTarget, &[&str])` and `EventTargets::listener_generation()` for compositor scroll caches. The image loader hooks default to unsupported until replaced-content adopts them.

One complete pinned DOM WPT round at revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` (627 sources, 800×600, timeout multiplier 2) changed **135→175 passing files** and **2,993→37,250 passing subtests**. The large subtest increase comes from previously aborting/timeout Range fixtures now running thousands of assertions. There were **zero lost passing files or distinct passing subtests**. Eight shadow Range files had duplicate names for the same passing assertions in the baseline; those duplicates disappeared when the tests completed and are not assertion losses. `dom/events/passive-by-default.html` reached **100/100** from **80/100**. Candidate raw SHA-256 `31cd8880f8664971b126bb0b02f30f57a2f3139a670b3a9fd7d00be35766f5f8`, outcome SHA-256 `5cb6804c6e9a2c472f02e0cc34f8fa5ae0782c4f80e11720a6179b00612189cf`. The automatic second runner round was stopped after the first completed round.

### DOM-B continuation: Range, traversal, and node reflection

The continuation completes `Document`/`Element.moveBefore`, `insertAdjacentElement` and `insertAdjacentText`, XML document cloning metadata, XHTML `createElement` namespace selection, CharacterData's null-to-empty conversion, explicit-undefined traversal defaults, and primary-document `compatMode`. It also installs `EventTarget.prototype` in Window's prototype chain using js-runtime's `binding_set_global_prototype` facade, paired with js-runtime's legacy-native removal `df1ceb06`.

One pinned `dom/` round on the DOM continuation before the JS runtime pairing: **175→189 passing files; 37,250→43,041 passing subtests**, across 627 sources, with zero passing file or subtest losses. Raw SHA-256 `4a25d0ca5858f008390bcef92c6fbc4c68e8db821df703580f2adb48f90088c3`; outcome SHA-256 `f1652eef913073c5b2aee475f62387f289f563b7c30580c52f81fcdbbde71173`. The combined JS runtime and DOM tree passed **39/169** pinned `dom/events/` files versus 35/169 on the DOM-only tree, with zero file losses; its outcome SHA-256 is `215bea08e75a806008e16b4c1443d9390122662292bb67438b928e0a68fba555`. In `passive-by-default.html`, all 100 assertions still pass: 20 assertion labels changed from `undefined` to `EventTarget` because Window now inherits that prototype. Remaining near-pass Range/traversal files mostly fail the JS Error-subtype assertion path; iframe realm/setup and legacy platform object own-key enumeration are separate routed gaps.

Final exact-ref win-002 gate for `e68b7685`: owned nextest **488/488** (four skipped), workspace nextest **2,422/2,422** (six skipped), owned doctests **0/0**, workspace doctests **2/2**.

Merge `640a4bed` incorporates dev `b5c5bcbd` and unifies storage event delivery with Window's binding EventTarget slot. It removes storage's separate Window listener wrapper; `storage` events dispatch through the native registry and then `onstorage`. Linux merged-tree owned nextest passed **479/479** (2 skipped), including cross-document storage delivery/removal; owned doctests passed. Win-002 exact-ref `640a4bed` owned crate nextest and doctests, workspace nextest and doctests all passed (482 owned tests listed, 2,398 workspace tests listed, no failures; workspace doctests 2/2). The paired js-runtime combined `dom/events` run remains pending. No push.

### DOM-B follow-up: WebIDL unscopables (checkpoint)

The DOM Standard's `[Unscopable]` members are now exposed through null-prototype `Symbol.unscopables` dictionaries on Document, DocumentFragment, Element, CharacterData, and DocumentType prototypes. The pinned `remove-unscopable.html` reaches its six inline-handler assertions; js-runtime must make inline-handler object-environment lookup consult those dictionaries. Linux owned nextest passed 486/486 (two skipped) with doctests green. A combined-tree full `dom/` sweep gained 20 passing files but found one real pass→fail assertion in inherited HTMLCollection named-property assignment from JSRT `df1ceb06`; it is routed with an exact reproduction. Heavy Range timeouts were repeated in isolation and recovered complete passes; `Node-compareDocumentPosition.html` still times out under the combined runtime and needs a same-host old-ref comparison before a zero-loss claim. This is a checkpoint, not a final gate.

## Focus-state handoff — 2026-09-26

Focus implementation `9a79112d` plus disabled-fieldset/first-legend follow-up `8cb9e5cb` merged by primary into local dev `9d2e5c27`. Lane infrastructure-only merge: `62750442`.

Exact-source Windows gate (`8cb9e5cb`, win-001 fallback): owned nextest **495 passed, 4 skipped**, owned doctests **0**; complete workspace nextest **2,454 passed, 2 failed, 7 skipped**, workspace doctests **2 passed**. The two failures are compositor `paint_view_owner_mapping_binds_dom_properties_to_tree_nodes` (4 vs 3 bindings) and `retained_real_page_property_trace` (2 vs 0 scroll uploads); compositor fix `1d7b991d` is in its lane gate. This is not a green workspace claim. Logs: `focus-windows-gate.log`, `focus-windows-completion.log`.

Linux iteration: owned 492 passed/2 skipped, doctests 0. Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293, isolated 120-second deadline/fresh storage: Node-compareDocumentPosition **1444/1444**, fieldset disabled-001 **5/5**, HTMLCollection-as-prototype **1/2** (JS owner notified), disabled-003 **0/2** (dynamic disabling of an already-focused descendant remains). These are Linux diagnostics, not Windows conformance gates.

Next DOM-B cluster: StaticRange required dictionary members and conversion order; historical pinned constructor case is 16/17. CSS and HTML can consume the merged focus-state APIs now.

## DOM-B StaticRange dictionary conversion — 2026-09-26

Implementation `1d507ddb`; merged fixed gate source `ae17e533` incorporates local dev `fba3b71b` and the compositor/JS-owner fixes. StaticRange now rejects missing and explicit-undefined required offsets, reads/converts members in WebIDL lexicographic order, and preserves null-to-zero numeric conversion. Native binding coverage checks all four required members, null dictionaries, read order, and null offsets. Sources: [DOM StaticRange](https://dom.spec.whatwg.org/#interface-staticrange), [WebIDL dictionary conversion](https://webidl.spec.whatwg.org/#es-dictionary).

Windows final gate through win-run's win-001 fallback: owned nextest **496/496** (4 skipped), owned doctests **0**, workspace nextest **2,464/2,464** (7 skipped), workspace doctests **2/2**; command exit **0**. No remaining compositor failures on this combined source. Linux native iteration passed **309/309**, owned doctests **0**.

Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293 targeted Linux iteration, 800×600, 120-second deadline, fresh storage per invocation: StaticRange-constructor **16/17→17/17** (baseline is the historical DOM-B3 sweep); HTMLCollection-as-prototype **1/2→2/2** after the JS-owner fix joined dev. The earlier isolated Node-compareDocumentPosition run passed **1444/1444**, closing the reproduced timeout concern without claiming a full-corpus zero-loss gate. Raw targeted records and Windows totals: `crates/amazing-bindings/evidence/staticrange-20260926.json`.

Remaining DOM-B work: Range bulk-operation fixtures still have timeout clusters; cross-realm traversal requires iframe/realm host support; tentative OpaqueRange APIs are unimplemented. Dynamic disabling of an already-focused fieldset descendant remains a focus follow-up (disabled-003: 0/2 in Linux diagnostics). No push.

## Queued: IPC-11 MessageEvent and messaging EventTargets

Order: after the DOM-B Range bulk-operation timeout cluster.

Add `EventClass::Message` in amazing-dom and the MessageEvent interface in amazing-bindings: constructor, data, origin, lastEventId, source, ports, and legacy initMessageEvent. MessagePort and BroadcastChannel inherit the common EventTarget methods and use the DOM listener registry.

Planned seam (not implemented yet): generalize the Window adapter pattern with optional `BindingHost::external_event_target_slot(value) -> Option<u64>` and `external_event_target_value(slot) -> Option<Value>` hooks, defaulting to None. The host validates wrapper identity/realm/liveness, assigns a stable slot from the common DOM external-target allocation domain, and maps the slot back to the canonical messaging wrapper. Existing Window/AbortSignal mappings remain supported. EventTarget receiver conversion and target/currentTarget wrapping use these hooks; ports/channels keep their own brands without a duplicate listener list.

Expose a typed trusted message-delivery helper in amazing-bindings so IPC can supply data, origin, lastEventId, source and ports and dispatch through the same native registry. DOM stores only native message metadata and opaque retained-value handles; bindings/host own JS value rooting and cleanup. IPC retains transport, structured clone/transfer, inbox scheduling and lifecycle ownership. Publish exact helper signatures with the implementation; these names are the proposed adapter contract, not callable APIs yet.

Exit checks: native/binding coverage of constructor defaults/conversions, legacy initialization, retained payload identity, target/currentTarget identity, and shared capture/once/passive/removal/dispatch behavior; pinned messaging WPT selection; Windows owned/workspace nextest and doctests.

## Deferred: iframe-dependent Range bulk fixtures (2026-09-26)

On merged source `6a029fa3`, Linux Boat reproduces five pinned bulk timeouts at
120 seconds each: cloneContents/extractContents/deleteContents each complete
one detached assertion; insertNode/surroundContents complete none. A minimized
three-second scaffold passes direct clone/extract, fails iframe contentDocument
and contentWindow presence, and never completes iframe load. No Range algorithm
change. Raw evidence and reproducible diagnostic:
`crates/amazing-bindings/evidence/range-timeouts/`.

User-directed dependency: **JSRT-9 child realms after the await fix, then HTML-25
part 2**. Deferred, not blocked. Continue independent nodes, traversal, events,
collections, and queued IPC-11 MessageEvent/shared EventTarget work. Windows
source upload failed; this is Linux diagnostic evidence, not a Windows gate.

## IPC-11 DOM-side MessageEvent and shared targets ready (2026-09-26)

Implementation `2f5ca997` + `aa2024f3`, fixed base `536d0bdc`. Native MessageEvent,
retained payload/source/ports, frozen arrays, legacy initialization, trusted
message dispatch, and optional external target mapping now share the DOM event
registry. Exact API contract and IPC adoption request are in
`crates/amazing-bindings/evidence/messageevent/README.md` and lane questions.
IPC still owns switching its temporary MessageEvent/global/listener adapter to
this contract; this commit does not claim installed messaging gains.

Linux Boat final gate: workspace **2531 passed, 10 skipped**, docs **2/2**;
owned/driver iteration **514 passed, 4 skipped**, owned docs **0**. Seven-source
pinned selection before/after: **2 pass / 3 fail / 2 unsupported**, **234 passing /
206 failing / 6 error subtests**, zero losses or gains. Fixed-source raw evidence
is in `crates/amazing-bindings/evidence/messageevent/`.

Next independent DOM-B event cluster: UIEvent dictionary Window validation,
conversion order, signed-long detail, and related-target validation/root cleanup.

## DOM-B UI event dictionary conversion complete (2026-09-26)

`01e52255` validates Window views and related EventTargets, reads supported
inherited dictionary fields in WebIDL order, converts UIEvent.detail as signed
long, and roots values only after fallible conversion completes. Native tests
cover all six UI event constructors, nullable/default fields, numeric edges,
conversion ordering, invalid dictionaries, and root cleanup.

Linux Boat exact-source workspace **2534 passed / 10 skipped**, docs **2/2**;
owned/driver **517 passed / 4 skipped**, owned docs **0**. Pinned event subclass
constructors improve **48/49→49/49**; the seven-source selection gains one file
and one assertion with **zero losses**. Evidence:
`crates/amazing-bindings/evidence/event-dictionaries/`.

## DOM-B legacy event factory complete (2026-09-26)

Legacy event factory complete: implementation `03856580`, fixed base `f14340c8`.
Document.createEvent selects the matching UI event class, preserves ASCII-case
aliases, and throws DOMException NotSupportedError for unsupported names.
Linux final gate: **2536 passed / 10 skipped**, doctests **2/2**; owned/driver
iteration **519 passed / 4 skipped**, owned doctests **0**.
Pinned Document-createEvent improves **75→249 passing subtests**, with **zero
lost passes** across the seven-source selection (409 pass / 31 fail / 6 errors).
Evidence: `crates/amazing-bindings/evidence/legacy-events/`.

Next: routed DOMException conformance and event-handler content attributes.

# DOM-B DOMException conformance ready

Implementation `82bb3868`, fixed base `129cde71`. Explicit undefined uses the
constructor defaults; native name/message/code accessors remain inherited.
The existing constructor, 25 legacy constants and native thrown exceptions
are covered by the full pinned WebIDL exception directory.

Linux Boat gate: **2709 passed / 10 skipped**, workspace doctests **4/4**.
Baseline owned/driver tests: **533 passed / 4 skipped**. Whole-directory WPT
subtests **112→123 passed, 19→8 failed**, **11 gains / zero lost passes**.
Files: **2→4 pass**, **4→2 fail**, **1 timeout**. The remaining eight failures
need JS Error.isError/stack support; the iframe portion of exceptions.html is
deferred for JSRT-9 → HTML-25 part 2. Its six local assertions now pass.

Evidence: `crates/amazing-bindings/evidence/dom-exception/`.
Windows deferred. Next: DOMStringMap, with numeric named-property hooks routed
to JS runtime; handler scope/compiler facade also routed there.

# DOM-B DOMStringMap native area ready

Implementation `07a885b5`, fixed base `c7b0c3c2`. Live dataset reads/writes/deletes,
ASCII name conversion, UTF-16 values, attribute-order names and SameObject wrappers
now use native DOMStringMap. HTML, SVG and MathML expose the getter; arbitrary
namespaces no longer receive the old legacy dataset extension.

Linux isolated-target gate: **2733 passed / 10 skipped**, docs **4/4**;
baseline owned/driver **537 passed / 4 skipped**. The full 55-source collection
and global-attribute testharness selection improves **140→168 passing subtests**,
**135→107 failing**, **28 gains / zero lost passes**. Files **15→22 pass**,
**39→32 fail**, one harness error unchanged. All native dataset files pass except
the two prototype-accessor setter assertions in dataset-binding.window.js.
Those and numeric named-only set/define/delete discrimination are routed to JS.

Evidence: `crates/amazing-bindings/evidence/dataset/` (includes excluded reftest
inventory). The interrupted namespace gate was discarded; identical source was
rerun through linux-run recovery and completed. Windows remains deferred.
Next: inherited collection property surfaces; generic intrinsic array iteration
needs a routed JS LengthOfArrayLike fix. Iframe Range remains deferred.

# DOM-B collection own-property surfaces ready

Implementation `12737f79`, fixed base `7b6678a2`. Collection length/value accessors
remain inherited across HTMLCollection, form/options/radio collections, live/static
NodeLists, NamedNodeMap, DOMTokenList and DOMStringList. An installed regression
covers live key order, named visibility and mutation updates.

Linux isolated-target workspace **2734 passed / 10 skipped**, docs **4/4**.
Full55-source collection/global-attribute corpus: **168→175 passing subtests**,
**107→100 failing**, **7 gains / zero lost passes**. Files **22→24 pass**,
**32→30 fail**, one harness error unchanged. Baseline is dataset/after.json;
evidence is `crates/amazing-bindings/evidence/collection-properties/`.

Generic intrinsic array iteration and strict legacy assignment/deletion remain
routed to JS. Continuing legacy document collections (embeds/plugins, anchors,
applets). Windows and iframe-dependent Range remain deferred.

# DOM-B document legacy collections ready

Implementation `d8f5ac84`, fixed base `1f219daa`. Native document.embeds/plugins
share the same live collection; anchors matches HTML a elements with a name
attribute (including empty); applets remains an empty collection. Tests cover
namespace exclusion, attribute mutation, canonical identity and document roots.

Linux isolated-target workspace **2736 passed / 10 skipped**, docs **4/4**.
The complete71-source document-accessor, obsolete-API and collection corpus
improves **166→172 passing subtests**, **148→142 failing**, **6 gains / zero
lost passes**. Files **27→29 pass**, **32→30 fail**, nine unsupported and three
harness errors unchanged. Baseline owned/driver **543 passed / 4 skipped**.
Evidence: `crates/amazing-bindings/evidence/document-collections/`.

Continuing obsolete document/body color reflection and legacy no-op methods.
Iframe Range and JS-dependent collection dispatch/iteration remain deferred.

# DOM-B legacy document/body reflection ready

Implementation `b73d7bcd`, fixed base `1122ea30`. The five obsolete document and
body color IDL attributes reflect the same live body content attributes, preserve
UTF-16, distinguish null from undefined, and ignore absent bodies/framesets after
argument conversion. Document clear/captureEvents/releaseEvents and Window
captureEvents/releaseEvents implement the specified no-op behavior.

Linux isolated-target workspace **2739 passed / 10 skipped**, docs **4/4**.
The complete 71-source document-accessor/obsolete/collection selection improves
**172→197 passing subtests**, **142→117 failing**, **25 gains / zero lost passes**.
Files **29→33 pass**, **30→26 fail**, nine unsupported and three harness errors
unchanged. Three color cases still need iframe documents; native coverage proves
the corresponding missing-body/frameset behavior without iframe dependencies.
Evidence: `crates/amazing-bindings/evidence/legacy-document/`.

Continuing a fresh bulk nodes/traversal/events/collections baseline. Iframe Range,
JS-dependent named-property dispatch and generic array iteration remain deferred.

# DOM-B listener registration and AbortSignal lifetime ready

Implementation `f9fde9d7`, fixed base `93156830`. Listener callbacks and the
complete options dictionary convert before null/duplicate/aborted early returns.
Abort removal shares the signal's ordered algorithm list, preserves a duplicate
registration's original signal, and releases callback roots only when removed.
The recursive abort fixture no longer crashes.

Linux isolated-target workspace **2744 passed /10 skipped**, docs **4/4**.
All **510** testharness sources in dom/nodes, dom/traversal, dom/events and
 dom/collections: **9812→9823 passing assertions**, **1263→1255 failing**,
**11 gains /zero lost passes**. Files **214→217 pass**, **141→139 fail**,
**28→27 errors**;100 timeouts and27 unsupported unchanged. Subtest errors6
unchanged. Signal11/11, capture4/4 and passive5/5 now pass.

Continuing DOMTokenList's140-failure classList cluster. Traversal's1600 passing
assertions and7 cross-realm failures confirm that remaining traversal work and
iframe Range remain deferred to JSRT-9/HTML-25 part2.

# DOM-B DOMTokenList semantics ready

Implementation `d6613150`, fixed base `54e681e4`. Token-list arguments convert
before validation and live reads; contains/supports use their own validation
rules. Empty add/remove calls normalize existing attributes, absent empty
attributes stay absent, and replacement preserves ordered-set position and
mutation/no-op semantics.

Linux workspace **2747 passed /10 skipped**, docs **4/4**. Complete dom/nodes
and dom/collections directories (**324 sources**): **7864→8004 passing**,
**1007→867 failing assertions**, **140 gains /zero lost passes**. classList is
**1420/1420**, previously1280/1420. Files151→152 pass,102→101 fail;
37 timeouts,27 unsupported,7 harness errors and6 subtest errors unchanged.
Baseline is dom-b-audit/after.json restricted to those two directories.

Continuing document/Window association reflection: missing defaultView is the
next non-iframe prerequisite in createElementNS's exception checks. Child realms
and iframe-dependent Range remain deferred.

# DOM-B document defaultView ready

Implementation `cc808abd`, fixed base `56035411`. Document.defaultView uses
an embedding hook to return its associated WindowProxy, or null for inert
DOMImplementation/DOMParser/cloned documents. The current driver associates its
single window with the active document; child-realm embedders can supply their
own association through the default-Unsupported hook.

Linux workspace **2749 passed /10 skipped**, doctests **4/4**. Complete
node/collection corpus (**324 sources**): **8004→8114 passing assertions**,
**867→757 failing**, **110 gains /zero lost passes**. File classifications
unchanged:152 pass,101 fail,37 timeouts,27 unsupported,7 harness errors;
6 subtest errors unchanged. All110 gains are createElementNS exception checks
that access document.defaultView.DOMException. Remaining390 assertions in that
file are XML/XHTML iframe cases and remain deferred.

Continuing shadow event dispatch retargeting/cleanup. Inline handler compilation,
iframe Range, and CSS selector escape/scoping dependencies remain deferred.

# DOM-B shadow event retargeting ready

Implementation `a4f60416`, fixed base `9e68980b`. FocusEvent/MouseEvent
relatedTarget now uses native dispatch state; retargeting works against non-node
targets, boundary suppression truncates the path, and cleanup uses shadow
membership captured before callbacks. Adjusted related targets survive ordinary
dispatch and redispatch instead of reverting to the original private node.

Linux workspace **2753 passed /10 skipped**, doctests **4/4**. Fresh complete
**168-source dom/events** corpus: **359→360 passing assertions**, **241→240
failing**, **one gain /zero lost passes**. Files unchanged:51 pass,35 fail,
20 harness errors and62 timeouts. Four added native/installed-driver regressions
cover external targets, callback reparenting, redispatch, and identical original
target/relatedTarget handling.

Remaining relatedTarget fixture cases include net's XHR facade missing native
EventTarget adoption, activation behavior, and a pinned-WPT/current-DOM early
return discrepancy recorded in questions.md. Child realms/Range remain deferred.
Continuing existing UI event modifier and numeric dictionary semantics.

# DOM-B UI event modifiers ready

Implementation `7e014f57`, fixed base `88f37385`. MouseEvent, WheelEvent and
KeyboardEvent now retain all fourteen modifier flags and expose case-sensitive
getModifierState. Numeric dictionary conversion applies the specified signed and
unsigned widths, preserves fractional coordinates, rejects non-finite doubles,
and includes inherited UIEvent.which and WheelEvent.momentum.

Linux workspace **2758 passed /10 skipped**, doctests **4/4**. Complete
**170-source dom/events + uievents/constructors** corpus: **360→376 passing
assertions**, **260→244 failing**, **16 gains /zero lost passes**. Files:
51→52 pass,37→36 fail,20 harness errors and62 timeouts unchanged.
The modifier-state constructor fixture is now **16/16**.

InputEvent target-range sequence conversion awaits a JS iterator/closing seam;
iframe Range remains deferred. Continuing independent legacy element reflection.

# DOM-B legacy element reflection ready

Implementation `1cb4dbcf`, fixed base `e5f1a8c5`. Directory, font, frame and
frameset elements now use their specified interfaces and fourteen reflected
attributes, including boolean removal, USVString URL conversion and legacy
null-to-empty-string setters. Installed-driver coverage checks cloning, live
attributes, foreign-namespace/receiver rejection and UTF-16 conversion.

Linux workspace **2759 passed /10 skipped**, doctests **4/4**. Complete
node/collection/obsolete API directories (**333 sources**): **8144→8148 passes**,
**767→763 failures**, **four gains /zero lost passes**. Files158→159 pass,
104→103 fail;37 timeouts,27 unsupported,7 harness errors and6 subtest errors
unchanged. Node-cloneNode.html now **135/135**.

Initial gate exposed a test setup omission (no document URL for URL resolution);
corrected fixture and full rerun passed. Frame child navigables and frameset
Window handler forwarding remain deferred with their existing dependencies.
Continuing ProcessingInstruction constructor and attribute APIs.

# DOM-B ProcessingInstruction ready

Implementation `ab38fc57`, fixed base `ca5e01fe`. Added the constructor and
complete attribute API, with ordered UTF-16 values, XML pseudo-attribute parsing,
reference decoding, serialization and reparsing after character-data changes.
Attribute mutations use existing character-data observer/range machinery;
no-op toggles preserve data. Target validation now permits xml per current DOM.

Linux workspace **2763 passed /10 skipped**, doctests **4/4**. Complete
**314-source dom/nodes** corpus: **8077→8082 passing assertions**, **741→736
failing**, **five gains /zero lost passes**. Files unchanged:148 pass,95 fail,
37 timeouts,27 unsupported,7 harness errors;6 subtest errors unchanged.
Baseline is legacy-elements/after.json restricted to dom/nodes.

The attribute fixture's first five assertions now pass. Its pre-existing
`ReferenceError: source is not defined` still prevents later registrations;
that JS lexical-scope issue is routed to js-runtime. Native and installed-driver
regressions independently cover parsing, mutation records, ordering, UTF-16,
conversion order and branding. Continuing navigation event payload interfaces.

# DOM-B navigation event payloads ready

Implementation `9d13ddc7`, fixed base `d50cd1bf`. HashChangeEvent,
PopStateEvent, PageTransitionEvent and BeforeUnloadEvent now have native payloads
and interface brands. Constructors convert inherited dictionaries first, keep
state-object identity and convert URL fields to USVString. BeforeUnloadEvent's
string returnValue remains independent of Event's boolean cancellation flag;
legacy factory aliases select the correct interfaces.

Linux workspace **2767 passed /10 skipped**, doctests **4/4**. Fresh complete
**482-source dom/nodes + dom/events** corpus: **8442→8448 passing assertions**,
**1019→1013 failing**, **six gains /zero lost passes**. Files unchanged:199 pass,
131 fail,99 timeouts,27 unsupported,26 harness errors;6 subtest errors unchanged.
Document-createEvent improves **249→255 passes**.

Navigation scheduling and checking unload cancellation remain embedding work.
Compared with the older events corpus, the fresh baseline registers43 additional
body/frameset handler failures after HTMLFrameSetElement became available;
no passes were lost, and inline handler compilation remains deferred. Continuing
lossless UTF-16 event types across constructors, initializers and listener keys.

## Lossless UTF-16 event names — complete

Lossless UTF-16 event types complete: `268c5a4e`, fixed base `ac89b733`.

Constructors, legacy initializers and listener registration/removal preserve lone
surrogates without conflating them with replacement characters. Normal Unicode
names retain the existing native String representation. Abort/once listener
identity uses the same exact keys. Four new native/binding/driver regressions pass.

Linux workspace **2771 passed /10 skipped**, doctests **4/4**. Complete
482-source dom/nodes + dom/events corpus remains **8448 pass /1013 fail**,
**zero gained or lost passes**. File classifications unchanged (199 pass,131 fail,
99 timeout,27 unsupported,26 error); six subtest errors unchanged.
Baseline reuses the immediately preceding navigation-events after corpus on the
exact base. Frozen candidate binary and source hashes recorded in gate.json.

Next: merge local dev for JS realms, then remaining legacy reflected attributes.
Iframe-dependent Range remains deferred pending HTML-25 part 2. WebSocket is
owned by net and removed from this lane's queue.

## Remaining obsolete reflection — complete

Completed remaining obsolete HTML reflection: `e78c8e4c`, fixed base `270e9735` (includes JSRT realms). Adds 98 attributes on 32 interfaces, aliases and correct receiver checks, legacy-null/boolean/numeric/URL conversions. Driver regression includes conversion boundaries, UTF16, aliasing and cloning.

Linux workspace **2874 passed /10 skipped**, doctests **4/4**. Whole333-source nodes/collections/obsolete API corpus: **8159→8165 pass**, **752→746 fail**, **six gains /zero lost passes**. Five script event/htmlFor assertions and one moveBefore name-map assertion improve. Files158→160 pass,103→102 fail,38→37 timeout,27 unsupported and7 harness errors unchanged;6 subtest errors unchanged. Node-compareDocumentPosition changed timeout→pass with the same1444 passing assertions on both runs; this is deadline variation, not additional conformance credit.

Evidence: crates/amazing-bindings/evidence/obsolete-reflection/. Next: ErrorEvent and PromiseRejectionEvent payloads. Iframe Range remains deferred for HTML-25 part2; JSRT-9..12 are merged. WebSocket belongs to net.

## Error and promise-rejection event payloads — complete

ErrorEvent and PromiseRejectionEvent complete: `8c0efe84`, fixed base `131fe706`.

Native payloads preserve object identity and arbitrary error/reason values,
including undefined versus null. Dictionary conversion order, required object
validation, numeric wrapping, UTF16/USV strings and interface checks follow HTML.
Value roots are retained only after all conversions succeed. Constructor lengths
and native dispatch use the installed interfaces; three new regression tests pass.

Linux workspace **2877 passed /10 skipped**, doctests **4/4**. Whole228-source
dom/events, HTML scripting/events and unhandled-promise-rejections corpus:
**399→410 pass**, **345→334 fail**, **11 gains /zero lost passes**.
Files59→61 pass,56→54 fail,78 timeouts and35 harness errors unchanged.
Ten ErrorEvent payload/handler assertions and one PromiseRejectionEvent
constructor assertion improve. Runtime error and rejection scheduling remain
embedding/JS event-loop work; no automatic-reporting claim is made.

Spec: https://html.spec.whatwg.org/multipage/webappapis.html#the-errorevent-interface
Next: checkbox/radio activation. Iframe Range remains deferred for HTML-25 part2;
JSRT realms are merged. WebSocket belongs to net.

# DOM-B ticket-2 pivot: WebSocket already landed by net; diagnosed the next dom/ cluster instead

Merged `origin/dev` onto `lane/dom-bindings` (175 commits behind; two real conflicts,
both resolved by combining both sides: `interfaces.rs`'s iframe/embed/object/param
D{} entries now use the legacy-attribute wrappers (`LEGACY_IFRAME`/`LEGACY_EMBED`/
`LEGACY_OBJECT`/`LEGACY_PARAM`) built on top of the upstream HTML-25/26 member sets
(`IFRAME`, `OBJ`) instead of either side's version alone — `LEGACY_OBJECT` now
builds on `OBJ` (which gained a `form` getter upstream) rather than the empty base
it used before, so neither ticket's members were lost; `forms.rs`'s
`clone_form_state` keeps both this lane's `click_in_progress` reset and upstream's
selection/custom-error/user-edited/last-type/submission-guard resets). Post-merge
gate on Linux Boat (`9cabc736`): workspace nextest 3074 passed / 13 skipped / 0
failed, `cargo test --workspace --locked --doc` exit 0.

Checked `git log dev -- crates/amazing/src/runtime_host_net.rs` per the brief:
net already landed the WebSocket JS interface (`7a657d9d`, "NET-34: install the
WebSocket JS interface via net's own driver adapter") — full `WebSocketOp` surface
(constructor, url/protocol/extensions/readyState/bufferedAmount/binaryType,
send/close, event handlers, add/removeEventListener/dispatchEvent). Nothing to do
here; not re-verified beyond confirming the merged workspace still builds/gates
clean with it in place.

Per the brief's fallback ("otherwise the largest WPT dom/ failure cluster"),
triaged `dom/nodes/Document-createElement.html` / `-createElementNS.html`, which
html's questions.md thread flagged as needing a fresh per-subtest look now that
contentWindow exists. Finding: 100% of the remaining 98+390 failing subtests in
these two files are the "in XML document"/"in XHTML document" parametrizations
(every "in HTML document" case passes); the common WPT idiom of
`iframe.contentWindow.document.createElement(...)` then checking `.localName`
returns `undefined` instead of the correct string, while `instanceof win.Node`/
`win.Element` both pass and the identical call on the top window works. Read
through the dom-bindings-owned dispatch path (`invoke.rs`'s `ElementLocalName`,
`runtime_binding_host.rs`'s `prototype()`/`materialize_accessors`) and found
nothing there that can produce this — DOM interface prototypes are process-global,
not per-realm, and the native getter body has no undefined-producing path short of
throwing (and nothing throws). This is very likely a js-runtime/js-vm cross-realm
property/getter dispatch bug, filed to `questions.md` with full evidence rather
than guessed at blindly across a lane boundary. Also filed a smaller, non-blocking
note: WPT's own `testharness.js` assertion-message template substitution
(`${expected}`/`${actual}`) silently no-ops in this engine, so every failing
assertion corpus-wide reports its raw un-substituted template — likely
`String.prototype.split` with a capturing-group regex, `amazing-js`-owned.

No dom-bindings source changed for this second item (diagnosis only, per AGENTS.md
"blocked item goes to questions.md and you move on" — the fix crosses into
another lane's files). Commit is the merge plus the sparse-checkout hygiene fix
below.

# DOM-B checkbox/radio native activation and onclick ready

Implementation `4993b82c`, evidence `d502fcd1`, fixed base `47bcf7f0`. Finished
and landed the previous agent's checkbox/radio activation + onclick WIP after
two correctness fixes found while gating:

1. `onclick`'s setter threw `TypeError` on non-callable objects, and dispatch
   fell back to reading `handleEvent` when a stored handler was not callable.
   `EventHandler` is WebIDL `EventHandlerNonNull?` where `EventHandlerNonNull`
   carries `[LegacyTreatNonObjectAsNull]`: any object (callable or not) is
   stored without throwing; a non-object primitive (or null/undefined) clears
   the handler; invoking a non-callable handler is a silent no-op that must
   never read `handleEvent` (unlike `EventListener`'s callback-interface
   fallback used by `addEventListener`). Fixed in `event_bindings.rs`
   (`callback_call` branches on a new `handler` flag before the `handleEvent`
   fallback) and `input_activation_bindings.rs` (onclick setter no longer
   checks callability).
2. Canceled radio activation re-derived the clicked node's *live* type/name to
   decide whether to recheck the previously-checked peer, which regressed
   `dom/events/Event-dispatch-click.tentative.html`'s "radio morphed into
   another type" case (a handler that changes `type` then calls
   `preventDefault()`). Fixed by snapshotting the peer's own identity
   (name/form owner/root) at prepare time and validating against that
   snapshot at completion, never against the clicked node's current state;
   the clicked node's own checkedness is now explicitly reverted first,
   independent of whether it still classifies as a radio.

Linux workspace **2883 passed /10 skipped**, doctests all green. WPT
`dom/events` + `html/semantics/forms` + `uievents` (whole directories):
**2472→2597 passing subtests (+125), zero lost passes**. Evidence in
`crates/amazing-bindings/evidence/input-activation/`.

Queued/deferred from this ticket: PointerEvent (click still constructs
`MouseEvent`), non-input activation targets (submit/reset, `a`/`area`
navigation, popovers, file pickers), event-handler content attributes (JS
compiler seam). Continuing with the WebSocket JS interface on net's
already-landed transport seam (NET-8/NET-19; see questions.md 2026-09-26T19:40Z).

## DOM-B legacy createEvent aliases: DeviceMotionEvent/DeviceOrientationEvent/DragEvent/StorageEvent/TextEvent

WebSocket (NET-8/NET-19) was already landed by `net`, so this ticket picked
the next-largest in-lane WPT `dom/` failure cluster instead, per the bulk
corpus scan across `dom/ranges`, `dom/traversal`, `dom/nodes` and
`dom/collections` (408 files). Two candidate clusters were routed away, not
fixed here, because the actual bug sites are outside `amazing-dom`/
`amazing-bindings` (see `questions.md`, 2026-09-26T18:05Z entries):

- `ParentNode-querySelector-escapes.html` (40 fails) — CSS hex/punctuation
  escape decoding lives in `amazing_css::selectors::parse_selector`, which
  `RuntimeBindingHost::SelectorEngine::parse_selector` calls after only
  splitting the selector list on quote/paren/bracket depth. Routed to `css`.
- `Document-createEvent.https.html`'s `MessageEvent` alias mismatch (3 of its
  24 original fails) — `message_event_bindings::create_legacy` and the real
  `new MessageEvent()` constructor both route through the identical
  `wrap_platform_object(..., InterfaceBrand::EVENT)` →
  `interface_for`/`dom_event_interface` → `self.prototype(Interface::MessageEvent)`
  lookup that every other legacy alias (including `CustomEvent`) resolves
  correctly through, but `Object.getPrototypeOf(document.createEvent("MessageEvent"))`
  is confirmed (via an `amazing domrun` probe) to NOT be reference-equal to
  `window.MessageEvent.prototype`, unlike the constructor path. No
  dom-bindings-owned divergence between the two call sites was found; routed
  to js-runtime/js-vm as a likely `binding_wrap_host` host-object identity
  issue in `amazing-js`.

The fixable, in-lane part of the same cluster: `document.createEvent()`'s
legacy alias table (`event_bindings.rs`) already *validated* the aliases
`DeviceMotionEvent`, `DeviceOrientationEvent`, `DragEvent`, `StorageEvent` and
`TextEvent` (real interfaces per the DOM Standard's create() legacy factory
table that this engine doesn't otherwise implement) but then fell through to
`_ => event.class()`, leaving the created event with the *generic* `Event`
class — so `Object.getPrototypeOf(ev)` was `Event.prototype`, not the named
interface's own prototype, and none of those five interfaces were even
registered as `window.X` globals. Fixed by:

- Five new `EventClass` variants (`amazing-dom/src/events.rs`):
  `DeviceMotion`, `DeviceOrientation`, `Drag`, `Storage`, `Text`.
- Five new `Interface` variants (`amazing-bindings/src/host.rs`) and matching
  `D{}` descriptors (`interfaces.rs`), each `constructor: None` (matching the
  existing `BeforeUnloadEvent` pattern — WPT only checks prototype identity,
  never `new X()`, for these) with empty `members` (no IDL surface beyond the
  inherited base is exercised by any pinned test).
- `event_bindings.rs`'s class-mapping switch now maps all five lowercase
  alias names to their new `EventClass`, and
  `crates/amazing/src/runtime_host_dom_bindings.rs::dom_event_interface`
  (additive-grant file) maps each new `EventClass` to its new `Interface`, so
  the existing dynamic `wrap_platform_object` dispatch picks the right
  prototype the same way it already does for `HashChangeEvent`/
  `CompositionEvent`/etc.
- Extended `tests_legacy_events.rs`'s alias-preservation test with the five
  new class/name pairs (exact/lower/upper case, per its existing loop shape).

`amazing-dom` 363/363, `amazing-bindings` 117/117 (both include the extended
test). WPT `dom/nodes/Document-createEvent.https.html`: 270 pass / 21 fail+error
→ 270 pass / 3 fail + 6 precondition-skips (TouchEvent's "expose legacy touch
event APIs" `assert_implements_optional` gate — expected/optional since this
engine doesn't implement touch events; not a regression or a bug). Full
4-directory corpus (`dom/ranges`+`dom/traversal`+`dom/nodes`+`dom/collections`,
408 files) before/after, **zero lost passes** (confirmed after excluding the
handful of hard-timeout-truncated combinatorial `Range-*` files — same
"timeout, partial subtests retained" status before and after, unrelated to
this change — where a fixed wall-clock deadline makes the exact retained
subtest count non-deterministic across runs on a shared sandbox), **+15
gained** (exactly the 5 interfaces × 3 case variants). Evidence:
`crates/amazing-bindings/evidence/dom-b-audit/` (`before-createevent` =
`cluster-scan`, `after-createevent`).

### Next-cluster scoping (same session, no further code landed)

Widened the corpus scan to `dom/abort`, `dom/lists` (clean) and `dom/observable`
(245 fails, 0% passing, one missing interface) looking for the next bulk
cluster. Deliberately did not start implementing `Observable` this ticket:
reading the real WPT source (`observable-constructor.any.js`,
`observable-map.any.js`, `observable-take.any.js`) shows its actual model is a
*shared, ref-counted producer* (one initializer invocation multicasts to every
concurrent `subscribe()` call; teardown only runs after the last unsubscribe;
`addTeardown()` is LIFO at real teardown time but synchronous FIFO if the
subscription is already inactive; `next()` iterates a snapshot of observers),
not a plain cold/Rx-style Observable — getting that core exactly right is the
whole ticket, since every operator is just a composition on top of it. Also
found `RuntimeBindingHost::run_abort_algorithm` (`crates/amazing/src/runtime_binding_host.rs`)
is currently a no-op stub, so nothing registered via `Dom::add_abort_algorithm`
(fetch/XHR cancellation, and Observable's per-subscription unsubscribe if built
the obvious way) actually fires yet. Full writeup and proposed native-registry
shape in `questions.md` (2026-09-26T19:05Z); proposing `Observable` as its own
dedicated next ticket rather than rushing it into this one.

## DOM-B-OBS: `dom/observable` shared-producer core, operators, and abort algorithms

Base: local `dev` @ `4f27b9c4`.

Implemented the model scoped above, exactly as read from the WPT source, in
one bulk pass:

- **`crates/amazing-dom/src/observable.rs`** (new): a JS-free native registry
  (`Observables`/`ObservableEntry`/`Run`) mirroring `abort.rs`'s shape. One
  `ObservableId` can have at most one active `Run`; the first concurrent
  `subscribe()` starts it (bumping a `generation` counter used to detect a
  Subscriber reference from a torn-down run), later ones just add another
  `ObserverEntry`. Teardown is LIFO and runs only once the observer list empties
  (last individual unsubscribe, or `complete()`/`error()`). `Producer` is an
  enum naming how a run starts (`Js` initializer, or an operator's upstream:
  `Map`/`Filter`/`Take`/`Drop`/`FlatMap`/`SwitchMap`/`TakeUntil`/`FromValues`/
  `FromPromise`/`EventTargetWhen`). `Sink` is `Js` (a normalized observer),
  `Relay(ObservableId, RelayKind)` (an operator's downstream), or
  `Terminal(Terminal)` (a promise-returning operator's bookkeeping —
  accumulator slots, current index). All values are opaque `CallbackId`/`u64`
  tokens; this crate never calls into JS.
- **`crates/amazing-bindings/src/observable_bindings.rs`** (new, ~750 lines):
  the driving loop that dereferences those tokens and applies transforms,
  always outside any `Dom` borrow. `subscribe_internal`/`start_producer` is the
  shared subscribe core every operator and the constructor route through;
  `relay_forward` is one big match over `(RelayKind, RelayEvent)` implementing
  every stream operator uniformly on the same relay+teardown-cascade mechanism,
  including `flatMap`/`switchMap`'s dynamically spawned inner subscriptions
  (`flat_map_spawn`, tracked via `open_inner_count`/`outer_complete`); every
  terminal operator (`forEach`/`toArray`/`first`/`last`/`find`/`some`/`every`/
  `reduce`) is a direct native subscription on the source
  (`Sink::Terminal`) driving a `Promise`. `EventTarget.prototype.when()` and
  `Observable.from(thenable)` reuse the *existing* generic
  `BindingHost::create_native_function` (already dynamic/callable-anytime, not
  bootstrap-only) via one new `Operation::ObservableInternalThunk(u64)`
  variant dispatched through the normal `invoke()` chain — no new host-trait
  primitive was needed for either.
- **Platform-key packing**: the driver's `platform_object` enforces
  `generation == 1` for every brand (a shared, brand-agnostic liveness
  invariant), so a Subscriber's run-generation is packed into `PlatformKey.slot`
  alongside its `ObservableId` (`pack_subscriber_slot`) instead of using
  `PlatformKey.generation` — the obvious first design broke that shared
  invariant and was changed before any driver code was touched.
- **Abort algorithms**: `RuntimeBindingHost::run_abort_algorithm` (previously a
  permanent `NotSupportedError` stub) is now a one-line hook —
  `observable_handle_unsubscribe(self, algorithm)` — delegating into this
  lane's own `observable_bindings::handle_unsubscribe`. `Dom::add_abort_algorithm`
  tokens are self-describing (`pack_unsub(ObservableId, call_id)`, bit 63 kept
  clear for a future subsystem such as net's fetch/XHR); nothing else in the
  tree called `add_abort_algorithm` before this, confirmed by grep. This also
  makes fetch/XHR cancellation infrastructure functional for the first time,
  though no net-owned code was touched.
- **New minimal host primitives**: `BindingHost::sequence_values` (array-like
  read without the final `to_dom_string`, for `Observable.from()`'s fallback
  path) and `BindingHost::range_error_value` (a constructed, not thrown,
  `RangeError` — `first()`/`last()`/`reduce()` reject with this on an empty
  source, per spec, not a `DOMException`).
- `event_bindings::target` and `abort_bindings::propagate`/`default_reason`
  widened from private to `pub(crate)` for reuse by the new module.

**Known, deliberate scope cuts** (all confirmed against the pinned WPT source,
not guessed): `Observable.from()` handles Observable/thenable/array-like only,
no iterator-protocol/async-iterable support; a user callback throwing
(initializer, map/filter predicate, or a terminal's visitor/reducer) currently
propagates via `?` instead of being caught and routed to the observable's
`error()`/the terminal's rejected promise — **a `call_catching` +
`BindingHost::error_value` fix was attempted and reverted this ticket**: it
regressed `first`/`last`/`toArray`/`forEach`/`some`/`every`/`reduce`/`find`
from clean fast fails back to hard timeouts without even fixing its target
(`observable-takeUntil.any.js` stayed an "error" status), so the interpreter is
doing something on a caught throw (likely leaving some VM/call-stack state
unwound incorrectly) that plain `?`-propagation does not hit — filed as a
follow-up needing its own investigation, not blind retry; `catch`/`finally`/
`inspect` are not implemented (correctly excluded from the ticket's operator
list). `moveBefore` was left alone per standing instruction.

**Known bug, not fixed this ticket**: `observable-takeUntil.any.js` (one file)
has WPT status `"error"` with an opaque `[object]` detail (0/0 subtest counts —
the whole file aborts before any test runs), present identically in the very
first Observable build this ticket produced and in every later measurement,
regardless of the takeUntil-ordering fix below or the reverted exception-catch
attempt. Root cause: its 4th test has a notifier `Observable` whose initializer
`throw`s synchronously, which currently propagates all the way out of
`.subscribe()` as an uncaught exception instead of being caught — the same
uncaught-initializer-throw gap noted above. **Not a zero-loss violation**:
`dom/observable`'s pass total was `0` in every prior baseline (the interface
did not exist), so this file contributed zero passes before and zero after;
nothing regressed. Filed for a follow-up ticket alongside the exception-catch
fix above, since a correct fix likely needs both together.

**Fixed in the same pass** (found via the corpus diff, both confirmed against
WPT source before any code changed): `takeUntil()` was subscribing to `source`
before `notifier` (`start_producer`'s `Producer::TakeUntil` arm); the spec
subscribes to `notifier` first and must never subscribe to `source` at all if
`notifier` synchronously emits/completes — `observable-takeUntil.any.js`'s own
"source never subscribed to when notifier synchronously emits" tests assert
this directly. `first()`/`last()`/`reduce()` on an empty source rejected with a
`DOMException` (`InvalidStateError`); the spec (and `observable-first.any.js`'s
own test) requires a real `RangeError` — fixed via the new
`range_error_value` primitive above. All eight terminal operators
(`forEach`/`toArray`/`first`/`last`/`find`/`some`/`every`/`reduce`) previously
ignored an `options.signal` argument entirely, which meant an aborted-signal
terminal call hung forever instead of rejecting — `observable-first/last/
toArray.any.js`'s `{signal: controller.signal}` tests exercise exactly this.
Fixed by threading `own_signal` through `start_terminal`/`subscribe_internal`
(the existing generic "first-call pre-check" and `add_abort_algorithm`
registration already worked for any `Sink` kind once `Sink::Terminal` was
added to both the pre-abort check and `Dom::observable_remove_observer`'s
now-richer `RemoveOutcome` — which had to start returning the removed
`Sink` itself, not just its teardowns, so a Terminal's promise can be rejected
with the signal's real abort reason on the async-abort path, not just the
already-aborted-at-call-time path).

**Measured** (`cargo nextest run -p amazing-dom -p amazing-bindings -p amazing
--locked`: 666/666, no regressions, before and after every fix in this
ticket). `dom/observable` (29 files, `dom/observable/tentative/**`):
`0 pass / ~245 fail` (prior scoping-scan baseline, interface did not exist) →
**68 pass / 161 fail**, 4 files with WPT-level `timeout` status (the
`.window.js` variants plus `idlharness.html`'s missing `WebIDLParser.js`
resource — both pre-existing runner gaps, unrelated to this feature) and 1
file (`observable-takeUntil.any.js`) with `error` status per the known bug
above. Wider `dom/` sweep (`dom/abort`+`dom/lists`+`dom/nodes`+`dom/traversal`+
`dom/events`+`dom/collections`+`dom/ranges`, re-run twice to separate signal
from shared-sandbox flakiness): `fail` counts identical across repeated runs
in every directory; only `pass` counts on `dom/nodes`/`dom/ranges`/
`dom/traversal` fluctuated run-to-run (confirmed pre-existing timing flakiness
on a shared, CPU-contended sandbox, not caused by this change — none of those
crates' source was touched). Workspace gate (this host, `k-test`/venus-class):
`3267 tests run: 3256 passed, 11 failed, 14 skipped` — the 11 failures are all
in `amazing-ipc-platform`/`amazing-process` sealed-memory tests, a known
host-specific `EPERM`/`os error 1` issue unrelated to this lane (ipc is fixing
as IPC-12, per fleet notice). Re-run clean on `venus-001`:
**3267/3267 passed, 14 skipped, zero failures**; doctests `amazing_js` 1/1,
`amazing_html` 1/1, all others `ok`.

Evidence: `crates/amazing-bindings/evidence/dom-b-audit/` (`after-observable`
= first bulk build; `after-observable-fix4` = final, after the takeUntil-order
and signal/RangeError fixes and the exception-catch revert).

## DOM-B-ACT: :scope in closest(), trusted driver clicks, label activation forwarding

Base: `dev` @ `75a5a3ce`. Ticket commits: `d589e992` (feature), `ed4028e0`
(fixup, see below).

**1. `:scope` for `Element.closest()`.** css's CSS-29 widened
`amazing_css::selectors::matches_in_scope`/`matches_pseudo_in_scope` to `pub`
specifically for this. Added `SelectorEngine::matches_selector_scoped`
(`amazing-bindings/src/host.rs`, default body falls back to
`matches_selector` so every other implementor/call site is unaffected) and
wired it into `closest()`'s ancestor walk only
(`amazing-bindings/src/markup_bindings.rs`), binding `:scope` to the
*original* receiver element throughout, per the DOM spec. The production
path (`amazing/src/runtime_binding_host.rs`'s `SelectorEngine` impl)
delegates straight to `matches_in_scope`. The old tree-walking interpreter
has an independent `closest()`/`matches()` implementation
(`amazing_js::DomHost` + `native_dom_closest` in `interp_builtins.rs`,
driven by `amazing/src/dom_host.rs::BrowserDomHost`); gave it the same
`matches_selector_scoped` method and threaded the original `this` node
through `native_dom_closest`'s walk, matching the coordinator's named wiring
point exactly.

**2. Trusted driver clicks.** `Operation::HTMLElementClick` (unit) became
`HTMLElementClick(bool)`: `false` for script's `element.click()` (unchanged,
still dispatches via `O::DispatchEvent`/`DispatchOrigin::Script`), `true` for
driver-initiated clicks, which now dispatch via the existing
`event_bindings::dispatch_created_event` (`DispatchOrigin::UserAgent`) so
`Event.isTrusted` reads true. IA-TESTDRIVER's `TdClick` and the WebDriver
Actions `pointerUp` handler (`amazing/src/runtime_host_integrator_assist.rs`)
now pass `HTMLElementClick(true)`.

**3. `<label>` click activation forwarding.** `Dom::prepare_input_activation`
(`amazing-dom/src/input_activation.rs`) resolves a label's associated control
via `Dom::label_control` and recurses to prepare activation on that control,
so a label's activation behavior toggles its checkbox/radio exactly as the
HTML Standard requires (`label_control` never resolves back to a `<label>`,
so the recursion terminates). **Found via this ticket's own corpus diff and
fixed before commit `ed4028e0`:** the first version forwarded unconditionally
and regressed `dom/events/Event-dispatch-click.tentative.html`'s "disabled
checkbox/radio should not be checked from label click(By dispatchEvent)"
(4 subtests, pass -> fail) by toggling a *disabled* control's label. Added a
`form_control_disabled(control)` guard before forwarding (returns `None`,
i.e. no activation, for a disabled control) — re-measured clean, see below.

Flipped IA-TESTDRIVER's two regression tests in
`runtime_host_integrator_assist.rs` that asserted the prior (wrong) behavior:
`td_click_toggles_a_checkbox_via_the_real_click_default_action` now expects
`trusted_clicks == 1`; the label test (renamed
`td_click_on_a_label_forwards_to_its_associated_checkbox`) now expects the
checkbox to end up checked.

**Measured** (`k-test` host). Crate tests
(`cargo nextest run -p amazing-dom -p amazing-bindings -p amazing-js -p
amazing --locked`): **1106/1106 passed, 7 skipped**, stable across the fixup.
Workspace gate (`cargo nextest run --workspace --locked --no-fail-fast`):
**3362/3362 passed, 17 skipped, 0 failed** (this run did not reproduce the
host's earlier known `amazing-ipc-platform`/`amazing-process` sealed-memory
EPERM failures reported for DOM-B-OBS). Doctests: `cargo test --workspace
--locked --doc` all `ok`; `amazing_js` and `amazing_html` explicitly
re-checked at 1/1 each.

WPT before/after (`crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`,
before = `75a5a3ce`, after = `ed4028e0`; `dom/nodes`+`dom/traversal`+
`dom/events`+`dom/collections`+`dom/abort`+`dom/lists`+`dom/ranges`+
`dom/observable`+`html/semantics/forms`, 1084 testharness files):
before **18564 pass / 1595 fail** subtests (403 pass files / 411 fail / 178
timeout / 62 error / 30 unsupported) -> after **18550 pass / 1591 fail**
(405 pass files / 408 fail / 179 timeout). File-status gains:
`dom/nodes/Element-closest.html` fail -> pass (the new `:scope` subtests:
`':scope'`, `'select > :scope'`, `':has(> :scope)'`) and
`dom/events/Event-dispatch-redispatch.html` fail -> pass. **Zero lost
passes** on any test this ticket touches (`Event-dispatch-click.tentative.html`'s
6 subtests, including all 4 disabled-checkbox/radio-from-label assertions,
pass identically before and after). The apparent ~14-subtest net drop in
raw pass count is entirely shared-sandbox timing flakiness already
documented under DOM-B-OBS: `dom/nodes/Element-classlist.html`,
`Node-compareDocumentPosition.html`, `Node-contains.html` and
`dom/ranges/Range-compareBoundaryPoints.html` all report file status
`timeout` with `fail: 0` in *both* runs — their `pass` counts differ only
because a CPU-contended sandbox lets a different number of subtests execute
before the harness's wall-clock deadline hits, confirmed by identical `fail`
counts (0) across both runs. Verified by rerunning `after` twice.
`dom/nodes/Document-createElementNS.html` flipped `fail` -> `timeout` with
identical `pass: 376, fail: 220` in both — the same pre-existing
createElementNS DOMException-shape bug already queued as a follow-up in
`status.json`'s blockers, unrelated to this ticket, now occasionally timing
out instead of finishing before the deadline (also flakiness, not a
regression).

Evidence: `crates/amazing-bindings/evidence/dom-b-audit/` (`before-domact`,
`after-domact` — the latter regenerated once, after the disabled-control
fixup).

## DOM-B-REFLECT (2fec1d2a): ARIAMixin reflection, HTMLInputElement.switch, Element.animate(), navigator.clipboard

Base `0654b5d3`. Four routed items in one ticket.

**1. ARIAMixin reflection.** New `crates/amazing-bindings/src/aria_reflection.rs`:
`Operation::AriaGet`/`AriaSet(&'static str)` bake the content-attribute name
into the operation and gate only on "is an Element" -- unlike
`element_reflection.rs`'s `LegacyReflectGet`/`Set`, which is tag/interface
gated and cannot express "every Element, any namespace, any tag" without
either hacking that shared gate or wrongly excluding SVG/MathML elements.
`role` plus all 41 `aria-*` `DOMString?` attributes are installed on
`ELEMENT`'s member table. Getter returns `null` for an absent attribute (not
`""`); setter removes the attribute on `null`/`undefined`, otherwise reuses
`html_reflection::set()` for CEReactions-safe writes. **Scope cut:** the
element-reference attributes (`ariaLabelledByElements` and friends,
`FrozenArray<Element>?`/`Element?`) are not implemented -- they need an
explicitly-set-element override that the accessible name/description
computation (`amazing-a11y`, the a11y lane) must consult ahead of any ID-ref
fallback; an ID-ref-only getter would silently diverge from that algorithm.
Routed to a11y coordination in `questions.md` rather than half-implemented.

**2. `HTMLInputElement.switch`.** Reuses the existing generic
`LegacyReflectGet`/`LegacyReflectSet(Interface, &'static str,
LegacyReflectionKind)` boolean-reflection mechanism (the same one `align`/
`useMap` already use), gated on `html_element_interface("input") ==
I::HTMLInputElement`.

**3. `Element.prototype.animate()` / minimal `Animation`.** New
`crates/amazing-dom/src/animation.rs`: a `Dom`-owned, id-keyed `Animations`
registry mirroring `abort.rs`/`observable.rs`'s pattern (an opaque
`AnimationId`, a private `Vec<AnimationData>`, checked `impl Dom` accessors).
New `crates/amazing-bindings/src/animation_bindings.rs` implements
`Element.animate()` and the `Animation` method/getter/setter set
(`finished`/`ready`/`cancel`/`finish`/`play`/`pause`/`onfinish`). Deliberately
**not** the full Web Animations timing model: no timeline, no keyframe/effect
application, no real pending-then-later-resolve scheduling -- an animation is
created already `Finished`, with `finished`/`ready` resolved synchronously
(at creation) to the `Animation` itself. `cancel()` swaps in a freshly
rejected (`AbortError`) promise for the getter to return afterwards and is a
no-op once already idle; `finish()` invokes `onfinish` synchronously (via
`host.call`) on every call, matching a real re-dispatched `finish` event.
This is enough that real pages calling `.animate(...)` and using the returned
object's promises/methods stop throwing "value is not callable"
(`app-kanban.html` in IA's real-pages corpus; also closes the
`Element.prototype.animate` gap IA-LIVE-PAGES flagged as `undefined` on
37/37 probed real pages).

**4. `navigator.clipboard`.** New `crates/amazing-bindings/src/clipboard_bindings.rs`
plus `Dom::clipboard_text`/`set_clipboard_text` (a plain `Vec<u16>` field on
`Dom`): a `Clipboard` singleton per document arena (`PlatformKey { slot: 0,
.. }`), `readText`/`writeText` round-tripping through that field. Not real OS
clipboard integration, just same-page/same-session round trips -- what real
pages exercise (also closes the `navigator.clipboard` IA-LIVE-PAGES gap).
Wired via a new `install_navigator_clipboard` in dom-bindings' own
`crates/amazing/src/runtime_host_dom_bindings.rs` adapter file (mirroring the
storage lane's lazy `navigator.<singleton>` getter pattern in
`runtime_host_storage.rs`, since `navigator` is an ad-hoc plain object, not a
WebIDL interface installed through `interfaces.rs`), plus one new call line
in `runtime_binding_host.rs`'s shared `install()`.

Native tests: `crates/amazing-bindings/src/tests_dom_b_reflect.rs` (5 new
tests, at the binding-op level against this crate's mock `Host`) cover all
four features, including reading back `Promise` resolution values
(`finished`/`ready` resolving to the `Animation`; `readText` resolving to the
written text) -- something the JS-level test harness in `crates/amazing`
cannot observe without a real microtask-pumping scheduler (those tests use
`Interpreter::new_with_external_scheduler`, which does not auto-drain jobs
at script end).

**Measured** (`k-test` host). Workspace gate (`cargo nextest run --workspace
--locked --no-fail-fast`): **3378/3378 passed, 17 skipped, 0 failed**.
Doctests (`cargo test --workspace --locked --doc`): all `ok`; `amazing_js`
and `amazing_html` explicitly re-checked at 1/1 each. `amazing-bindings`
crate: 122/122 including the 5 new tests; combined `amazing-dom` +
`amazing-bindings` + `amazing` crate run: 707/707.

WPT before/after (`crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`,
before = `0654b5d3` via `--ref`, after = this ticket;
`html/dom`+`wai-aria`+`web-animations/interfaces/Animatable`, whole
directories): before **10902 pass / 3561 fail** subtests -> after **10946
pass / 3380 fail**. Raw Counter diff: 178 gained, 134 lost. **Every one of
the 134 "lost" subtests is page-timing noise, not a regression**: all 134
sit in five pre-existing, untouched `html/dom/reflection-*.html` pages
(auto-generated `reflection.js` harnesses with hundreds to thousands of
subtests, run under the WPT runner's 20s per-page wall-clock budget) --
each such page reports **identical fail counts** before and after (e.g.
`reflection-embedded.html` 721/388 -> 701/388, `reflection-grouping.html`
1180/114 -> 1142/114), both `status: "timeout"` /
`"worker deadline exceeded; completed subtests retained"`; only the number
of subtests that finished executing before the deadline differs. None of
the lost subtests' attributes (`img.decoding`, `input.size`, `legend.lang`,
`blockquote.title`, `pre.width`, `noscript.className`, ...) are touched by
this ticket, and the 178 gained show the identical pattern in the other
direction on different, also-untouched pages. **Zero of the 134 lost
subtests touch anything this ticket changed; 158 of the 178 gained do.**
File-status flips: `html/dom/aria-attribute-reflection.html` and
`html/dom/aria-attribute-reflection.tentative.html`, both `fail -> pass`.
`input.switch`/`navigator.clipboard` have no automated coverage in this
pinned WPT checkout (covered by the new native tests instead);
`web-animations/interfaces/Animatable` mostly fails/errors identically
before and after (missing vendored resources, or the full timing/query
model this ticket doesn't implement) -- not a regression.

Evidence: `crates/amazing-bindings/evidence/dom-b-reflect/`.

## DOM-B-NEXT + HTML-35 (a3b471f8)

Fixed base `b72bc231`. Coordinator's ordered list: (1) fix the noisy
`html/dom/reflection-*.html` WPT timeouts, (2) HTML-35 item 1: `window.<id>`
named access (HTML SS7.2.2.3), (3) HTML-35 item 2: `HTMLScriptElement.async`
reflecting the "force async" flag.

**1. Reflection timeouts**: three compounding bugs in
`crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`, not an engine
perf regression -- `wai-aria/manual/*.html` (233 files, human-interaction
pages that never call `done()`) wrongly selected by a naive
`'testharness.js' in text` rule; the harness copied the *debug* binary
(4-5x slower than release for this CPU-bound work); the per-test deadline
(20s) was simply too small for pages with thousands of subtests even in
release (worst measured 84.2s). Fixed: exclude `/manual/` paths, copy
`release/amazing`, and split a `--heavy-timeout-ms 150000 --heavy-workers 1`
budget for the nine `reflection-*.html` pages specifically. Zero of them
time out on either side of the before/after diff now.

**2. Window named access**: `crates/amazing-dom/src/window_names.rs` (new)
computes the spec's supported-names list (`name` of
embed/form/img/object, `id` of any element) in tree order; a `Dom`-side
dirty flag (`named_objects_dirty`, set by the four real mutation choke
points: `set_attribute_ns`/`remove_attribute_ns` for id/name,
`insert_internal`/`remove_internal` for tree membership) lets
`crates/amazing/src/runtime_binding_host.rs`'s `with_dom` -- the one choke
point every DOM-mutating binding call passes through -- recompute and
redefine the global object's properties only when something relevant
actually changed. Scope cut: no nested-navigable/iframe-by-name support (no
content navigables in this engine yet); the multi-match case uses a
`STATIC_NODE_LIST`, not a true live `HTMLCollection`. **Found by the corpus
gate, not by inspection**: the first version unconditionally defined the
property, silently clobbering any pre-existing global with the same name
(`<div id="test">`, routine in WPT fixtures, overwrote testharness.js's own
`test()` function -- 55 lost subtests measured). Fixed by tracking which
keys the mechanism itself owns from the previous refresh and refusing to
(re)define a name that newly collides with something it doesn't already
own -- approximates the spec's "`[[Get]]` fallback, never shadows" rule
without an engine-level hook (out of this lane's granted paths). Re-verified
after the fix: 0 lost subtests.
- HTML-35 item 1's originally-cited motivating test,
  `dom/nodes/insertion-removing-steps/Node-appendChild-script-in-script.html`,
  turned out **not** to use `window.<id>` at all (confirmed by direct
  execution) -- its real gap is the HTML "children-changed steps"
  script-execution-model algorithm (whatwg/html#10188), a materially larger,
  separate feature needing an "already started" flag and mutation-triggered
  script preparation. Not started; routed to `questions.md` for html/
  js-runtime coordination. Implemented window named access anyway since
  it's independently spec-required and was already in scope as HTML-35
  item 1.

**3. `HTMLScriptElement.async`**: new `Node.force_async: bool` in
`amazing-dom`, initialized `true` by `create_element_local_in`/
`create_element_ns_in` (i.e. `createElement`/`createElementNS`) via a new
`init_force_async`; parser-inserted `<script>` elements go through
`amazing-html`'s structurally separate `tree.rs::insert_element` and so
correctly default to `false` with no html-lane change needed. Getter in
`element_reflection.rs` returns true if the flag is set OR the content
attribute is present; setter clears the flag before applying the boolean
attribute logic. New test:
`tests.rs::html_35_script_async_reflects_force_async_flag`.

**Measured** (venus-001; the k-test host died silently mid-gate -- see
`questions.md`). Workspace: **3466 passed / 17 skipped / 0 failed**
(`cargo nextest run --workspace --locked --no-fail-fast`); `cargo test
--workspace --locked --doc` all `ok` (`amazing_js`/`amazing_html` 1/1 each).
`amazing-bindings` crate: 123/123. Combined `amazing-dom` +
`amazing-bindings` + `amazing`: 736/736 passed, 6 skipped.

WPT before/after (`run-corpus.py`, before = `b72bc231` via `--ref` using
*that commit's own unfixed script*, after = this candidate;
`html/dom`+`wai-aria`+`web-animations/interfaces/Animatable`, whole
directories): **502 files/262 timeouts -> 269 files/21 timeouts** (exact
accounting: 233 manual-excluded + 8 reflection-fixed + 21 unchanged
pre-existing unimplemented cluster = 262). Subtests: 35336/9678 ->
47347/12528. Counter diff: **12011 gained, 0 lost**.

Also found and fixed in passing: venus-001's shared `/data` disk was at
100% full, crashing the workspace-test linker with SIGBUS on unrelated
crates. Freed 9.4 GB by deleting only this lane's own rebuildable
`debug/incremental` cache; flagged the underlying cross-lane disk-fullness
in `questions.md` rather than touching other lanes' directories.

Evidence: `crates/amazing-bindings/evidence/dom-b-next/`.

## DOM-18 follow-up: `sequence_shadow_roots` + `runtime_binding_host.rs` file-move (fixed base 5cd043b2)

**File-move (coordinator request)**: `runtime_binding_host.rs` had reached
2058 lines. Moved `refresh_window_named_properties` (HTML-35's
window-named-access mechanism) into this lane's own adapter file,
`runtime_host_dom_bindings.rs`, renamed `dom_refresh_window_named_properties`,
`pub(super)`, per the per-lane adapter convention in `AGENTS.md`. Pure move,
no behavior change; the shared file keeps one call site
(`self.dom_refresh_window_named_properties()?` in `with_dom`). After:
`runtime_binding_host.rs` 2013 lines, `runtime_host_dom_bindings.rs` 1164
lines.

**`BindingHost::sequence_shadow_roots`**: implemented (in
`runtime_host_dom_bindings.rs`, as `dom_sequence_shadow_roots`), replacing
the unconditional `NotSupportedError` default. WebIDL `sequence<ShadowRoot>`
conversion for `Element.getHTML({shadowRoots})` (DOM-18), same array-like-walk
shape as `sequence_values`/`sequence_dom_strings`; resolves each item to a
`NodeIdx` and leaves the actual `Dom::is_shadow_root` check to
`markup_bindings::invoke`'s existing `O::ElementGetHTML` handler (already
re-validates the whole list and throws `TypeError` if any item isn't a
shadow root).

**Measured**: `run-corpus.py --directory shadow-dom/declarative` (all 56
files under that directory) at fixed base `5cd043b2` vs this candidate:
**zero diffs** -- every file's status/counts identical. Root cause: the
only file in this pinned corpus exercising the `shadowRoots` option,
`gethtml.html`, errors in both runs before reaching any assertion on a
missing-resource harness gap (`html/resources/common.js` not in
`run-corpus.py`'s sparse checkout list), unrelated to this change. The fix
is still correct and needed (a real `getHTML({shadowRoots})` call would
previously throw unconditionally); it just isn't observable in this
snapshot yet. Routed the sparse-checkout gap, plus a found-in-passing
`HTMLTemplateElement.shadowRootMode`/`shadowRootDelegatesFocus`/
`shadowRootClonable` reflection gap (entirely unimplemented, driving several
`declarative-shadow-dom-basic.html` failures), to `questions.md` rather than
implementing them this ticket.

**Gate** (`venus-001`, base `5cd043b2` + this diff): workspace
`cargo nextest run --workspace --locked --no-fail-fast` -- **3482 passed /
17 skipped / 0 failed**. `cargo test --workspace --locked --doc` -- all
`ok` (`amazing_js`/`amazing_html` 1/1 each).

Evidence: `crates/amazing-bindings/evidence/dom-18-shadow-roots/`.

## Coordinator follow-up: run-corpus.py sparse-checkout gap + HTMLTemplateElement reflection (fixed base 4c512a79)

Both gaps flagged unaddressed in the previous ticket's evidence, fixed as one
ticket per coordinator instruction:

1. **`run-corpus.py` sparse checkout**: extracted the inlined
   `['resources', 'common', ...]` list at both `sparse_checkout` call sites
   into a new `SHARED_RESOURCE_DIRECTORIES` constant, adding `html/resources`
   and `webrtc` (the two cross-directory helper dirs found missing via
   `shadow-dom/declarative/gethtml.html` and
   `html/dom/usvstring-reflection.https.html`).
2. **`HTMLTemplateElement` reflection**: `shadowRootMode` (new
   `LegacyReflectionKind::LimitedKnownValues(&'static [&'static str])` --
   getter-only value limiting per the WebIDL reflection algorithm, default
   `""`), `shadowRootDelegatesFocus`/`shadowRootClonable`/
   `shadowRootSerializable` (existing `LegacyReflectionKind::Boolean`,
   unchanged). All four wired through the existing generic
   `O::LegacyReflectGet`/`LegacyReflectSet(Interface, attribute, kind)`
   pattern -- no new bespoke `Operation` variants.

**Measured** (venus-001, `--directory shadow-dom --directory html/dom`,
whole directories, 509 files; both runs on a freshly built
`cargo build -p amazing --release --locked` binary -- `run-corpus.py` only
copies the existing release binary, it does not build it): **1621 subtests
gained, 0 lost**; 34 files `error -> pass`, 29 `error -> fail`, 2
`error -> timeout` (all from the sparse-checkout fix).
`declarative-shadow-dom-basic.html` (the reflection fix's direct target):
`fail {pass: 7, fail: 15}` -> `fail {pass: 15, fail: 7}`. `gethtml.html`:
`error` (missing resource) -> `timeout` with 828/2062 real subtests
(auto-generated page, thousands of subtests, hits the per-test deadline;
still a real improvement over erroring before any assertion ran).

**Gate**: workspace `cargo nextest run --workspace --locked --no-fail-fast`
-- **3482 passed / 17 skipped / 0 failed**; `cargo test --workspace
--locked --doc` -- all `ok`. Crate: `amazing-bindings` + `amazing-dom`
372/372.

Found in passing, not implemented (routed to `questions.md`):
`innerhtml-on-ordinary-template.html`'s `ReferenceError: ordinarytemplate is
not defined` (a declarative-shadow-attach-skip or window-named-access gap
specific to `<template>`, not triaged further).

Evidence: `crates/amazing-bindings/evidence/dom-18-shadow-roots/combined-followup/`.

## innerhtml-on-ordinary-template.html ReferenceError: root cause + partial fix (fixed base ffdde5a3)

Root-caused via a minimal repro (two standalone probe pages run directly
through `amazing wpt-harness` against a scratch root, not committed).
Neither of the two hypotheses from the prior escalation was the actual
shape of the bug:

1. Declarative-shadow-attach-skip-when-host-already-has-a-shadow-root
   already works correctly (confirmed empirically) -- not the bug.
2. Not `<template>`-specific at all: a bare `<div id=x>` followed
   immediately by a sibling `<script>` with **zero** intervening DOM API
   calls never sees `window.x`, full stop.

**Real root cause**: `with_dom`'s dirty-flag refresh
(`runtime_binding_host.rs`) is the only place window's named properties
(HTML SS7.2.2.3) get recomputed, and it only runs when some DOM-mutating
JS *binding call* happens. A parser-driven insertion sets the dirty flag
without ever passing through `with_dom`; if the next thing that runs is a
script whose first statement is a bare global read with no DOM call
before it, the refresh never fires in time.

**Fixed here**: added `RuntimeBindingHost::sync_window_named_access`
(`pub(crate)`, one-line wrapper around `with_dom(|_| Ok(()))` -- reuses the
existing dirty-check, no duplicated logic), ready to call.
**Not fixed here, escalated**: the actual call site. Every place that runs
a parser-encountered `<script>` lives in `runtime_host_html.rs` (html
lane's file) and `amazing-html`'s tree builder is outside this lane's
granted paths; wiring one call to `sync_window_named_access()` immediately
before each `run_script`/`i.run_script` call there (9 call sites, listed
with line numbers in `questions.md`) needs html lane or the integrator.

**Gate**: workspace `cargo nextest run --workspace --locked --no-fail-fast`
-- **3482 passed / 17 skipped / 0 failed**; doctests all `ok`. Crate:
`amazing` + `amazing-bindings` + `amazing-dom` **736/736 passed, 6
skipped**. No WPT corpus re-measurement this ticket -- the fix has no call
site yet, so the target file's outcome is unchanged; a re-measurement
before the wiring lands would show zero movement and be misleading
evidence, not real signal.

Evidence: `crates/amazing-bindings/evidence/window-named-access-script-timing/`.

## setHTMLUnsafe: implement the missing Element/ShadowRoot method (base fa514dc9)

`Element`/`ShadowRoot.setHTMLUnsafe(html, options?)` (HTML Sanitizer API) was
entirely absent -- no `Operation` variant, no interface member -- so every WPT
test calling it crashed with `TypeError: value is not callable`, aborting the
whole test file before any assertion ran. Next actionable item from the
`declarative-shadow-dom-basic.html` remaining-failures queue entry.

**Fix**: added `O::ElementSetHTMLUnsafe` (`host.rs`), routed in
`markup_bindings.rs` into the exact same fragment-replacement body as
`O::ElementSetInnerHTML` (this engine has no sanitizer either way, so "skip
sanitizing" and the ordinary setter currently behave identically), declared
`method("setHTMLUnsafe", O::ElementSetHTMLUnsafe, 1)` on both `ELEMENT` and
`SHADOW_ROOT` in `interfaces.rs`. Options argument is accepted but ignored (no
sanitizer to apply against).

**Escalated, not fixed here**: `amazing_html::parse_fragment` has no way to
enable declarative shadow roots (`build_fragment` in `tree.rs` hardcodes
`allow_declarative_shadow_roots = false` for every fragment parse), so
`<template shadowrootmode=open>` markup passed to `setHTMLUnsafe` still
becomes an ordinary leftover template rather than an attached shadow root.
Routed to `questions.md` with a proposed non-breaking sibling function
(`parse_fragment_with_shadow_roots`) -- `amazing-html` is outside this lane's
granted paths. Once it lands, this lane's `FragmentParser` trait in `host.rs`
needs a second method/parameter to expose it to `markup_bindings.rs`.

**Measured** (venus-001, freshly rebuilt release binary both sides, fixed base
`fa514dc9`): `declarative-shadow-dom-basic.html` fail{pass:15,fail:7} ->
fail{pass:17,fail:5} (the 7 crashes gone; 5 remaining fail for the exact
escalated reason above). `declarative-shadow-dom-attachment.html`: 0/654 pass
-> 586/68 pass -- large genuine improvement, this file exercises
`setHTMLUnsafe` heavily. Whole-`shadow-dom/declarative` (56 files) naive diff
showed 588 gained/608 lost; the entire "lost" figure traces to `gethtml.html`
alone, an auto-generated page whose retained-pass count is a function of a
hard wall-clock worker-deadline timeout, not correctness (three consecutive
runs of the same unchanged binary against that one file gave 1216, 608, and
530 passes) -- not usable as a before/after signal, not a regression.

**Gate**: workspace `cargo nextest run --workspace --locked --no-fail-fast` --
**3482 passed / 17 skipped / 0 failed**; doctests all `ok`. Crate:
`amazing` + `amazing-bindings` + `amazing-dom` **736/736 passed, 6 skipped**.

Evidence: `crates/amazing-bindings/evidence/set-html-unsafe/`.

## observable-takeUntil.any.js: investigated, root cause found, fix reverted (base d7103c5b)

Took this from the queued-tickets backlog under "keep batching". Two findings, no code landed
(working tree is clean, back to `d7103c5b`) -- this section exists so the investigation isn't
repeated.

**Finding 1 (why the target file can't even be measured)**: `observable-takeUntil.any.js` errors
as a whole page (`status: error, harness_status: null, detail: "[object]"`) before any subtest
runs -- confirmed identical on base and on every variant tried. `"[object]"` traces to
`Throw::describe()` (`crates/amazing-js/src/value.rs:454`) formatting a thrown plain object with no
string `name`/`message`. Something throws a non-Error object during this file's setup/execution,
and `cmd/wpt_harness.rs`'s page driver treats it as fatal despite the file's own
`setup({allow_uncaught_exception: true})`. Not triaged further (no panic, no backtrace -- a real JS
throw, not a Rust crash). `dom/observable` corpus can't validate anything about `takeUntil`
specifically until this is fixed first.

**Finding 2 (the obvious per-spec fix regresses 9 other files, reverted)**: implemented the WICG
Observable spec's SS4.1 step 4 exactly ("if the initializer's abrupt completion, invoke
Subscriber's error() with the value") -- a new `BindingHost::caught_value` trait method plus a
`catch_to_error` helper wired into `Producer::Js`. Crate/workspace gates stayed green (no unit test
anywhere exercises this path). Measured `dom/observable` (29 files), base `d7103c5b`, two
independent runs per side to rule out host-load noise: base 100% stable both runs; the fix
reproducibly (both its own runs agree) flips 9 files from real `fail{pass:N,...}` counts to a
full-file `timeout{pass:0,fail:0}` hang (`observable-every/-find/-first/-forEach/-forEach.window/
-last/-reduce/-some/-toArray/-map.window`), while gaining 3 real passes on
`observable-constructor.any.js` and leaving the actual target file (`takeUntil.any.js`) completely
unaffected (blocked by finding 1). Root cause of the hang not found within budget. **Reverted**
rather than trade one broken file for nine newly broken ones under a gate that can't see it (no
Observable unit tests exist to catch this at the crate level -- a gap worth closing before anyone
touches `observable_bindings.rs` again).

**Finding 3 (a larger, separate spec-conformance gap, not attempted)**: `notify_terminate`
(`observable_bindings.rs`) forwards to observers *before* running teardowns and aborting its own
signal; the live WICG spec's "close a subscription" (invoked by `Subscriber#error()`/`#complete()`
*before* forwarding to observers) does the opposite order: abort signal, run teardowns LIFO, *then*
forward. Also, `relay_forward`'s `RelayKind::TakeUntilNotifier` arm maps notifier `Error` to an
outer *error* and notifier's own `Complete` to an outer *complete*; spec has notifier's error steps
call the outer's *complete()* (not error), and leaves notifier's own complete steps unspecified (a
no-op, source keeps mirroring uninterrupted) -- confirmed by the WPT file's own asserted results
array. Both are real, but reordering `notify_terminate` changes every operator's callback ordering
project-wide and needs a full `dom/observable` before/after gate to attempt safely -- out of this
ticket's budget. Full detail, including the exact spec quotes and reproduction commands, is in
`.agents/dom-bindings/questions.md` (not git-tracked; ask the lane agent or re-derive from this
entry plus the WICG Observable spec at https://wicg.github.io/observable/ if picking this up later).

## Observable unit tests + notify_terminate ordering + takeUntil mapping fix (e80581b3)

Coordinator-assigned, in order: (1) add crate-level Observable unit tests covering `takeUntil` and
`notify_terminate` ordering (closing the gap that let the earlier initializer-throw regression
through both gates undetected), (2) fix the ordering and mapping against those tests, with a full
`dom/observable` WPT before/after in one batch.

Added a `V::Script` mock-host callable (`tests.rs`) -- a boxed Rust closure run against the live
`Host` via the ordinary `BindingHost::call` path, so a test's "user callback" can perform real side
effects (`subscriber.next()`/`.error()`/`.complete()`/`.addTeardown()` on the exact `Subscriber` it
was invoked with) without a JS engine. Five tests in the new `tests_observable.rs`. Verified 4 of 5
fail against the unmodified `680d7659` code (temporarily swapped in via `git show HEAD:... >
observable_bindings.rs`, restored after -- no `git stash`), confirming they're real tests, not
vacuously true.

Fixed both bugs found in the prior ticket's investigation, against the live WICG Observable spec
text: `notify_terminate` now aborts its own signal and runs teardowns *before* forwarding to
observers (was backwards); `takeUntil()`'s `RelayKind::TakeUntilNotifier` arm now maps notifier
`Next`/`Error` to the outer completing (was: `Error` incorrectly mapped to outer *error*) and
notifier `Complete` to a no-op (was incorrectly mapped to outer *complete*).

Measured: workspace 3487/3487 passed (+5 new tests), 17 skipped, doctests ok; crate 741/741 passed,
6 skipped. Full `dom/observable/tentative` (29 files, two independent `after` runs agree exactly,
zero flakiness unlike the reverted initializer-throw attempt): **+6 subtests gained, 0 lost, 0 new
timeouts/errors** -- `observable-constructor.any.js` 24/20->26/18,
`observable-filter.any.js` 3/3->4/2, `observable-map.any.js` 3/3->4/2,
`observable-take.any.js` 2/4->4/2; every other file byte-for-byte unchanged.
`observable-takeUntil.any.js` itself remains unaffected (still blocked by the separate whole-page
`"[object]"` crash documented in the prior ticket, not attempted here).

Evidence: `crates/amazing-bindings/evidence/observable-notify-terminate-ordering/`.

## `parse_fragment_with_shadow_roots` wired into setHTMLUnsafe/Document.parseHTMLUnsafe (1da899a5)

Base `1da899a5` (local `dev`; Observable ordering/`takeUntil` fix, and html lane's
`amazing_html::parse_fragment_with_shadow_roots` for the nested-ancestor shadow shape, both
already in). Coordinator-assigned ticket: wire that new parser function into
`Element`/`ShadowRoot.setHTMLUnsafe` and `Document.parseHTMLUnsafe`, then remeasure
`shadow-dom/declarative`.

Fetched the live HTML spec (dynamic-markup-insertion.html + parsing.html) to confirm the exact
algorithms before writing code. Findings: (1) `setHTMLUnsafe` always parses with
`allowDeclarativeShadowRoots = true`, and per the fragment-parsing algorithm's own "adjusted
current node" definition, a *top-level* `<template shadowrootmode>` should attach to the real
fragment context element itself when the parser's stack of open elements has only its synthetic
root -- exactly the shape html lane's `parse_fragment_with_shadow_roots(context: &str, ...)`
can't do, since it only ever has a tag-name string for `context`, never a real `NodeIdx`; that gap
is why it was scoped to the nested-ancestor shape and left to this lane (`questions.md`). (2)
`Document.parseHTMLUnsafe` is a *full-document* parse, and every full-document parse in this
engine already defaults `allow_declarative_shadow_roots: true` unconditionally
(`TreeBuilder::new()`) with real ancestors genuinely on the open-elements stack by the time body
content starts -- so it needed no new parser support at all, just a static-method entry point onto
the existing `create_inert_document` primitive `DOMParser.parseFromString` already uses.

Added `FragmentParser::parse_fragment_with_shadow_roots` (`host.rs`, default falls back to plain
`parse_fragment`) and its `RuntimeBindingHost` impl calling `amazing_html::parse_fragment_with_
shadow_roots`. `markup_bindings.rs`'s `O::ElementSetHTMLUnsafe` alone now calls it; after parsing,
`top_level_declarative_shadow_template` detects the fragment's first top-level child being a
`<template shadowrootmode>`, attempts `Dom::attach_declarative_shadow` on the already-computed real
host `context_node`, and on success moves the template's `template_contents` into the new shadow
root (`Dom::import_subtree_children`, new in `amazing-dom/src/markup.rs` -- `import_fragment` can't
import an arbitrary subtree into an arbitrary existing target) and detaches the template
(`Dom::pre_remove`) before the unmodified `import_fragment` path imports the rest; on failure
(already a shadow host, or an invalid host element name) nothing is detached, so the template falls
through as ordinary content, matching the spec's exception-catch step. Added static
`O::DocumentParseHTMLUnsafe` (`interfaces.rs`: `I::Document`), dispatched in `parser_bindings.rs`
via the existing `create_inert_document("text/html", ...)` primitive.

Four new crate tests (`tests_set_html_unsafe.rs` + one alongside the existing `DOMParser` coverage
in `tests.rs`), via a `Host::parse_fragment_with_shadow_roots` sentinel-markup mock override (no
real HTML parser needed for this plumbing -- `amazing-html`'s own tests and the WPT gate cover real
parsing). Verified red before green by temporarily reverting the six production files to `HEAD`
(`git show HEAD:path > path`, fix kept safe in `/tmp`, no `git stash`), then restored.

Measured: workspace 3500/3500 passed (run twice, stable), 17 skipped, doctests ok every crate;
`shadow-dom/declarative` (56 files, base `1da899a5`, excluding the pre-existing-noisy
`gethtml.html`): pass 727->764, fail 120->84, **+37 subtests gained, 0 lost**, 0 new
timeouts/errors. `declarative-shadow-dom-basic.html` 17/5->22/0 (now fully passing);
`declarative-shadow-dom-slot-assignment-serialization.html` 0/3->3/0 (now fully passing);
`declarative-shadow-dom-slot-assignment.html` 0/8->5/3; `declarative-shadow-dom-attachment.html`
586/68->610/44. Every other file in the directory unchanged.

Evidence: `crates/amazing-bindings/evidence/set-html-unsafe-shadow/`.

## createElementNS DOMException-shape bug: root-caused to two amazing-js/html gaps, routed (base 678f1bfb)

Re-checked `dom/nodes/Document-createElementNS.html`'s 220-subtest failure cluster against current
`dev` (`678f1bfb`, includes js-runtime's JSRT-20..23 value-model changes -- confirmed unaffected,
same 220/376 split before and after that merge). Coordinator asked to nail the exact shape rather
than the prior session's guess ("DOMException object's construction path").

All 220 failures are exactly one shape: `"...in XML document..."` / `"...in XHTML document..."`
(110 each; every `"in HTML document"` case passes), each failing with `TypeError: value is not
callable` where a `DOMException` was expected. Diagnosed with three temporary probe `<script>`
blocks appended to a private WPT copy under `$OUT` (never committed, never touching the shared
`~/corpora/wpt` checkout), run through `amazing wpt-harness`:

1. **`xmlIframe.contentDocument.defaultView` (and the XHTML iframe's) is exactly `null`** --
   verified directly (`assert_equals(xmlDoc.defaultView, null)` passes). A broader probe across
   `DOMException`/`Node`/`Element`/`Event`/`Document`/`Error`/`TypeError`/`Object` confirms this
   isn't DOMException-specific -- with `defaultView` null, every global constructor is
   unreachable through it. Root cause is almost certainly `runtime_host_html.rs`'s iframe/child-
   realm setup (html lane's file) never creating or never wiring a Window realm as an XML/XHTML-
   typed iframe document's `defaultView` -- outside dom-bindings' granted paths (confirmed:
   nothing in `amazing-bindings`/`amazing-dom` populates `defaultView`, only reads a stored field).
2. **Reading a property off `undefined`/`null` does not throw in this engine** -- verified
   directly (`var u = undefined; u.foo` returns `undefined`, no throw). Per ECMA-262 this must
   always throw a `TypeError`. This is why the visible symptom is "not callable" two reads and one
   call later rather than an immediate crash at the real fault site: `null.DOMException` silently
   yields `undefined`, testharness.js's own `assert_throws_dom` (`resources/testharness.js`
   ~line 2279, `funcOrConstructor.name === "DOMException"`, no guard) then mis-routes internally
   because `undefined.name` also doesn't throw, ending in `undefined.call(this)` -- a core amazing-
   js `GetValue`/property-access gap, independent of DOM/iframes, likely with a much wider blast
   radius across other WPT selections wherever code expects an immediate throw on a null/undefined
   property read.

Neither root cause is in dom-bindings' granted paths (html's realm plumbing for #1, amazing-js's
interpreter for #2); not fixed here, written up in full (with the exact probe results) in
`questions.md` for js-runtime/html to route and pick up. No code changed; this entry is
documentation-only.

Tooling note for whoever reruns this: a private `wpt-harness` root under `$OUT` needs an empty
`.pinned-c2a2fce` marker file at its root (present in the shared `~/corpora/wpt`, not produced by
a plain `git clone`/`cp`) or the harness errors `"No such file or directory"` before running
anything at all.

## DOM-B iframe Document.defaultView (resume base 2eb1229a)

The HTML lane isolated the remaining XML/XHTML createElementNS failure cause to
`dom_document_default_view`: the driver only associated the root document with
its window. Its HTML-owned `iframe_document_window` helper maps live child
document IDs onto their cached stable WindowProxy. The DOM adapter now consults
that mapping before preserving root-window and inert-document behavior. Added a
srcdoc identity/inert-null installed regression; HTML's XML regression checks
child WindowProxy, document and DOMException constructor identity.

Reference: HTML [defaultView getter](https://html.spec.whatwg.org/multipage/nav-history-apis.html#dom-document-defaultview),
consulted 2026-10-02: null for documents without browsing contexts, otherwise the
browsing context's WindowProxy. No upstream code ported.

Measurements pending: fixed-base full dom/nodes corpus plus owned tests/docs,
then candidate workspace nextest/doctests + full same corpus with zero-loss
comparison. All runs use Boat after user steering; the preliminary k-test build
was cancelled before tests and is not evidence.

Completed on source `20b74057`: Boat native checkout/target workspace nextest
3520 passed/17 skipped, doctests4/4. Full314-file dom/nodes corpus11385->12028
passing assertions,1593->951 failing,643 gained/zero lost. Of these642 come
from iframe defaultView association (createElementNS+220,querySelectorAll+284,
matches+69,webkitMatchesSelector+69); one XHR assertion recovery is not credited
to this fix. Evidence `amazing-bindings/evidence/iframe-default-view/`. Earlier
HOME-FUSE missingrlib/SIGBUS attempts are excluded from proof.

### HTML-requested namespace attribute dump fidelity

The HTML lane measured ten foreign-content mismatches whose expanded attributes
were stored correctly but omitted by `Dom::dump`. The dump previously read only
`NodeKind::Element.attrs`, so namespaced attributes were invisible. It now reads
`attributes_expanded`, preserving cross-namespace insertion order, and prints
canonical `xml`, `xlink`, and `xmlns` namespace labels before local names. A native
regression mixes null, XLink, XML and XMLNS attributes. This is a diagnostic
fidelity correction; any html5lib measurement gain must be distinguished from
parser behavior changes. HTML's corpus measures it; the current DOM workspace
and node gate includes its native test.

### Document URL query encoding and hyperlink components

Adopted NET43's existing `ParsedUrl` query-encoder seam and driver helper;
no URL parser or codec algorithm is duplicated. Reflected element URLs use
the owner document's URL, first base href, and character set. HTML a/area now
expose all hyperlink components, with USV conversion before reinitialization,
opaque-path/credential early returns, and existing URL component setters.

The installed WPT runner previously rejected non-UTF-8 page bytes and assigned
UTF-8 metadata even to explicitly legacy-encoded HTML. It now verifies the raw
fixture bytes before decoding through the existing HTML decoder; BOM/meta
encodings supply both parser text and document metadata. The existing UTF-8
fixture default is preserved. Template substitutions keep their existing UTF-8
contract. Generated any/window wrappers still declare UTF-8.

Baseline: fixed `20b74057`, 451 selected legacy encoding assertions fail;
unsupported-encoding XHR assertions pass14/14. Initial hyperlink-only candidate
passed127/451, exposing324 failures caused by the runner's UTF-8 metadata.
Final native Boat workspace3523passed/17skipped, doctests4/4. Full314nodes
12028passing/951failing/6subtesterrors, zero gained/lost assertions against
the previous ticket. Legacy encoding0->451passing/451->0failing; XHR14/14
unchanged. Evidence under `amazing-bindings/evidence/document-url-query/`.

Reference: HTML [hyperlink component API](https://html.spec.whatwg.org/multipage/links.html#htmlhyperlinkelementutils)
and [encoding-parsing URLs](https://html.spec.whatwg.org/multipage/urls-and-fetching.html#encoding-parsing-a-url),
consulted2026-10-03. Component search mutation uses the URL parser's ordinary
UTF-8 query override; resolving href uses the document character encoding.

### Parser-owned document mode storage

HTML43/44's doctype classifier needs durable DOM mode storage. Exported
`QuirksMode::{NoQuirks,LimitedQuirks,Quirks}`, `document_quirks_mode(document)`
and `set_document_quirks_mode(document,mode)`. New documents default NoQuirks;
clones preserve mode; generation-tagged side-table entries cannot affect
recycled node slots. `document.compatMode` now reads this state for every
document, including inert documents without driver metadata. Only Quirks
reports BackCompat. HTML owns parser classification and its integration.
Fixed4ce0126d Boat native gate3525passed/17skipped, doctests4/4; full314nodes
12028pass/951fail/6subtesterrors, zero lost/gained, encoding451/451 andXHR14/14.
Initial corpus run timed out the known intermittent XHR responseType case;
complete314-file rerun restored it. Evidence/document-mode/gate.json records
both outcomes. Native tests cover cloning/reuse and installed root/inert
compatMode. No upstream algorithm copied.
Reference: DOM [compatMode](https://dom.spec.whatwg.org/#dom-document-compatmode),
consulted2026-10-03: document mode distinguishes no, limited, and full quirks;
compatMode exposes BackCompat only for full quirks.

### Pinned character-set normalization fixture protocol

Parent grants narrow `dom/nodes/encoding.py` server support. The largest
remaining dom/nodes cluster is654 nullish contentDocument assertions in the two
characterSet normalization tests. The runner currently sends the Python source
as application/octet-stream. The pinned Python endpoint actually takes the
first label query parameter, isomorphically decodes its bytes, escapes it in a
meta charset declaration, and returns a Unicode HTML body. WPT's default
FunctionHandler response is200 with no Content-Type; missing required label is
500/application-json. The new backend preserves this behavior (environment
traceback text is omitted). Real HTML MIME detection and canonical charset
names belong to HTML; document encoding is selected from actual response bytes,
never injected from the expected WPT label. No fixture file or assertion is
rewritten; no upstream algorithm is copied.

Baseline fixed20a85c17:315+339fail/0pass. Complete314nodes12028pass951fail6errors.
HTML46 adopted342a09d5. Final Boat native workspace3,531pass17skip and four
doctests. Complete314nodes12,028→12,682passes (+654),951→298fails,6errors;
zero lost passes by multiplicity. Both charset pages315/315+339/339;451 legacy
encoding and14XHR assertions preserved. The TTL-interrupted attempt is excluded;
full exact-source rerun completed. Durable evidence/charset-fixture records
raw upstream protocol outputs, fixture/source hashes and compact whole-node gate.

### HTML table DOM interfaces

Parent granted whole table/section/row collection and parts area plus cellIndex.
Fixed base8a816d91; table29-file baseline10pass144fail, whole314nodes12682pass
298fail6errors. Ordered live filters, parts setters/create/delete methods,
row/cell mutations and indices written in bulk. No parser or CSS changes.
Native source hashes and pinned before report in evidence/table-interfaces;
Final Boat native workspace3534pass17skip,doctests4; table29files10→146pass
(+136),144→9fail, zero lost; nodes31412682→12686pass (+4),298→293fail,6err,
zero lost after complete fixed-source343-file recheck restored knownXHRtiming.
Caption cross-realm wrapper identity remains an externalJS gap. 65,534-row
span-limits geometry fixture baseline timeout→candidate/recheckSIGKILL with
no prior passing assertions; preserved honestly in evidence.
Next: granted HTML serialization audit and whole HTMLBaseElement reflection.

### HTML serialization and document context

Fixed combinedbc9e8388; complete serialization8 baseline352pass23fail and314nodes
12686pass293fail6err. Parent granted HTML algorithm audit plus explicit
scripting-context DOM API and FragmentParser bool seam. Defaults no-context
false, active root explicitlytrue, Document clonefalse, generation-safe reuse.
Normal/shadow fragment parsing captures actual owner document bool; HTMLDOMParser
parses Disabled. HTML owner initializes configured parser/iframe context.
Attribute escaping, namespace names, UTF16raw data, CDATA/Text and PI ?> written
in bulk. Meaningful native tests cover context and serialization semantics.
Template contents appropriate inert owner-document model and full XML serializer
are separate gaps; no guessed template bool or relaxed XML validity. Final Linux3641pass17skip/docs4; serialization352/23→372/3 (+20), nodes12686/293→12692/288 (+6), zero loss after whole322 unchanged-source recheck restored known XHR deadline assertion. Evidence: amazing-bindings/evidence/html-serialization.
Queued parent-granted follow-ups: whole HTMLBaseElement reflection/fallback URL;
then faithful pinned single-byte-raw.py fixture protocol (actual body/header).
