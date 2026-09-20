# JS-4 execution seam — planning checkpoint

JSRT-1 remains committed at 8a04aed; its report is preserved as jsrt1-checkpoint-report.md.
Local-only work; no SSH/SCP/build/test while fleet stand-down remains in force.

Proposed API direction (not finalized or implemented) in amazing_js::interp: SchedulingHost::schedule(request)
-> Result<(), String>; ordered SchedulingRequest enum; opaque interpreter-owned
SchedulingToken; Interpreter::new_with_scheduler(host) -> Result<Self, Throw>;
invoke_one_job(token); invoke_one_callback(token, receiver, frame_timestamp).
Callback timestamp is Option<Duration>: Some only for frames, None for timers.
Both invocation methods run one token and never drain. Constructor alone opts in.
Timer registrations remain u32 JS handles, callback/job tokens cannot be forged
or reused across interpreters. Legacy public timer helpers are standalone-only
and inert in external mode; browser native functions use fallible registrations.
Sink errors remove newly retained work and become JS TypeErrors. Sink must not
synchronously execute JS or accept work and then return Err.

Stopped before implementation. Exact signatures must be published and coordinated before a driver adapter is written.
Per-listener resumable DOM dispatch remains an integration-layer follow-up.

## End-of-day checkpoint

JS-4 is isolated on `lane/js-runtime-js4`, based on merged dev `a042677`
which includes JS-3 merge `493e3d0`. No execution source edits were made.
JSRT-1 stays separately committed at `8a04aed` on `lane/js-runtime`; no VM
or GC changes are carried on the JS-4 branch.

First resume action: read `docs/RESUME.md` and the routed contract at
`/Users/jMac/Projects/OpenSource/amazing/.agents/js-lang/js4-contract.md`,
then finalize/publish the exact API with js-lang before driver adapter work.
Implement only the execution-owned scheduling seam after that handoff.
Hard invariant: one token invocation, no hidden queue drain, including nested
eval, Promise settlement, errors, script completion and DOM dispatch.

No JS-4 build or test has run. SSH/SCP stand-down remains in force.
The locally inspected JS-3 reports both record classification
`c0b747c840bc51e5977b4355a5e6d4715ad8dd39c8b21b85904810e5a375a04f`,
10314 pass / 33292 fail / 16 timeout / 9956 unsupported / 294 skipped
over 53872 files and 102918 variants. Those measurements predate the outage;
reading their files today is not a new run or proof of JS-4 correctness.
Future Windows gates require nextest and separate doctests with both counts.
