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
  request `setup({output:false, message_events:[]})`. The visual
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
- [ ] Execute unmodified pinned harness after JS-2; route subsequent engine or
  browser-host gaps through lane questions rather than adding substitute APIs.
- [ ] Known-passing and known-failing **upstream** harness tests.
- [ ] Repeated upstream classifications with stable hashes.
- [ ] Full workspace green before final completion/PR.

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
