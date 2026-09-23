# IA-JS4 fixed-base driver gate

This lane pinned dev base `c8c3c21b369d300e41009948aa11819e3c602e43`,
which contains JS-4 and DOM-6's real reference adapter proof. The driver source
is implementation revision `1f16fec356c957922fe5428a70fcf3f521268787`.
The co-integrator's later merged-head preflight is separate. None of the
synthetic traces or scaffold WPT cases earns upstream WPT credit.

## Entry and dispatch coverage

The shared `DriverScheduler` in `crates/amazing/src/js_scheduler.rs` owns one
`EventLoop`, the external-scheduler `Interpreter`, callback payloads, timer and
frame ID maps, and a retained FIFO of requests taken from the interpreter.
Its `enter` wrapper drains the complete outbox after both success and throw.
A failed EventLoop admission leaves the taken request in that FIFO and blocks
later JS entry. Each EventLoop step invokes one callback/job/native delivery,
drains newly emitted requests, then calls `complete(step.id)` even if the
callback threw. The driver advances time and requests rendering explicitly.

| Driver path | Interpreter entry and checkpoint |
| --- | --- |
| `cmd/js.rs::execute_inline_scripts` | inline handler installation, each classic script, document/body load events, then clock/render pump |
| `cmd/js.rs::cmd_run` | script and clock/render pump, including after a thrown script |
| `cmd/js.rs::cmd_domrun` | script against the parsed DOM and clock/render pump, including after a throw |
| `cmd/wpt_harness.rs::execute` | Window initialization, harness/page scripts, handler installation, callback installation script, document/body events, then repeated EventLoop pump |
| `gui.rs::render_page` | handler installation, scripts, document/body events, then initial pump |
| `gui.rs::GuiState::click` | click dispatch and checkpoint before navigation/default handling |
| `gui.rs::GuiState::pump_timers` | Windows timer tick advances the shared EventLoop clock and requests rendering |
| `js_scheduler.rs::invoke` | one Promise/queueMicrotask job, timer/frame callback with Window receiver and frame timestamp, or native mutation checkpoint |

`BrowserDomHost` forwards a pending native mutation after the DOM borrow ends;
the runtime acknowledges only successful outbox admission. The host's forwarded
bit and DOM pending flag stay set until the EventLoop's native checkpoint. The
native tests pre-register an observer in the DOM. No JS MutationObserver
constructor or BindingHost installer is claimed here. The checkpoint consumes
native delivery records; JS observer callback invocation remains for the
later binding installation.

## Evidence

`trace-1.json` and `trace-2.json` are two Windows nextest runs of the five
new adapter tests. Their eight named outcomes agree exactly after sorting by
name, projection SHA-256
`4c6d28065fe67184cef9a69456d45ab1f2d9be8ad10afddfe702a4c84904eb59`.
The order that nextest prints independent tests is excluded from the projection;
within each named trace, order is preserved. Named outcomes cover A,P,Q,R,B,
frame cancellation, P,M,Q, capacity-one catchable TypeError with P,M,
mutation batching/rearming, thrown script and callback, and retained requests
on downstream failure.

`gate-summary.json` records the Windows commands, counts, log hashes, target
directory and shared slot wrapper. Scoped driver nextest passed 122/122 with
3 skipped; `cargo test -p amazing --locked --doc` reports no library target for
the binary package. The final workspace gate passed 1414/1414 with 3 skipped;
separate doctests passed 2/2. The scaffold WPT worker preserved one pass and
one intentional fail, each with its expected subtest classification.

`head-source-manifest.json` contains SHA-256 of every 1109 tracked path at the
implementation revision. After both workspace commands, all 1109 paths in the
Windows lane tree matched that manifest; `source-verification.json` records
zero missing and zero mismatched files. The REFRESH-HOLD was then released.
