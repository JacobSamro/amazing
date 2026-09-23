# JS-4 execution seam and BindingHost handoff

Branch: `lane/js-runtime-js4`, rebased onto `origin/dev` at `474251e`.
JSRT-1 VM/GC remains parked on `lane/js-runtime` at `8a04aed`.

## Published JS-4 public API

The types below live in `amazing_js::interp`; there is no JS-to-DOM
dependency or shared borrow of the arena event loop. The driver opts in before
page code and takes a bounded outbox after each entry/invocation:

```rust,ignore
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SchedulingToken { /* interpreter identity + monotonic sequence */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulingRequest {
    Timer { registration: u32, callback_token: SchedulingToken,
            delay_ms: i32, repeat: bool },
    CancelTimer { registration: u32 },
    AnimationFrame { registration: u32, callback_token: SchedulingToken },
    CancelAnimationFrame { registration: u32 },
    Microtask { job_token: SchedulingToken },
    NativeMicrotask { global: u64, kind: NativeMicrotaskKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMicrotaskKind { MutationObserver }

impl Interpreter {
    pub fn new_with_external_scheduler(js_capacity: usize)
        -> Result<Self, Throw>;
    pub fn take_scheduling_requests(&mut self) -> Vec<SchedulingRequest>;
    pub fn enqueue_native_microtask(&mut self, global: u64,
        kind: NativeMicrotaskKind) -> Result<(), Throw>;
    pub fn invoke_one_job(&mut self, token: SchedulingToken) -> Result<(), Throw>;
    pub fn invoke_one_callback(&mut self, token: SchedulingToken,
        receiver: JsValue, frame_timestamp: Option<Duration>) -> Result<(), Throw>;
    pub fn cancel_animation_frame(&mut self, registration: u32) -> Result<(), Throw>;
}
```

The outbox is FIFO and bounded. `js_capacity` counts only JS-originated
requests; one additional slot is reserved for the flag-gated MutationObserver
microtask of this interpreter/agent. Promise, queueMicrotask, timer and frame
requests use only the JS slots. JS exhaustion throws `TypeError` synchronously
and drops the new token. The one native delivery cannot fail for capacity in a
conforming agent. The driver drains requests after each entry or token
invocation before stepping the DOM loop. The outbox owns no DOM value or event-loop borrow. A sink cannot
reject a request after the JS call returns: admission is decided by capacity
inside the runtime.

Rejected alternative: `Rc<RefCell<dyn SchedulingHost>>` with synchronous
callbacks into the driver's event loop. If the loop holds a mutable borrow while
invoking a JS token, JS enqueue would call `borrow_mut()` on the same loop and
panic. Context-passing `&mut dyn SchedulingHost` through every interpreter entry
and nested execution path could prove exclusive access, but it requires a broad
signature migration through the tree walker and its native callbacks before the
VM and driver are settled. The bounded outbox is the smaller stable seam.

`invoke_one_job` accepts only Promise/queueMicrotask tokens. Callback invocation
accepts timers with `None` timestamp and animation frames with `Some(timestamp)`;
the caller supplies the Window/global receiver. Timer arguments remain rooted
in the registry and are passed to the callback. Frame timestamp is converted to
milliseconds for its sole argument. Unknown, consumed, cancelled, wrong-kind and
foreign-interpreter tokens return `TypeError` and invoke nothing. Interval tokens
stay rooted after an invocation until cancelled; one-shot tokens are consumed
before callback entry, so self-cancellation/reentrancy is safe. IDs never reuse
within an interpreter, and tokens carry an interpreter-specific identity.

Each enqueue/cancel appends in order to the internal outbox. It holds only
scalar metadata and opaque tokens; callback/Promise values stay in the
interpreter. JS capacity failure discards the new registration/root and
propagates as `TypeError`. Timer nesting, clamping, eligibility, rearm and clock
belong to the driver's `amazing_dom::event_loop::EventLoop`. In external mode,
`run_script`, `run_program`, `dispatch_dom_event`, and each token invocation run
without draining any private queue. Legacy explicit `pump_timers` and
`drain_microtasks` reject external mode. Standalone `Interpreter::new()` retains
its existing drain convention and timers.

`queueMicrotask` and `cancelAnimationFrame` are exposed to JS. A separate
per-listener DOM dispatch continuation may still be required; the final drain
removal alone does not supply it. The future GC must trace the scheduler's owned
job/callback/argument registry. No VM/GC work is part of JS-4.

## Gate plan

Focused deterministic outbox tests establish registration order, one
job/callback at a time, Promise chaining, nested eval, throws, cancellation,
interval self-clear, frame timestamp and receiver/arguments. Windows crate
nextest and JS doctests run separately; before PR, workspace nextest with a
build slot and workspace doctests run. Full Test262 default-mode baseline at
60s/jobs=8 must match merged JS-3 classification
`c0b747c840bc51e5977b4355a5e6d4715ad8dd39c8b21b85904810e5a375a04f`.
All pre-outage measurements are historical and not a JS-4 gate.

### Native mutation ordering boundary

`DomHost::pending_native_microtasks()` returns host-owned descriptors generated
by the just-completed native operation. The runtime releases the DOM borrow,
appends each `SchedulingRequest::NativeMicrotask { global, kind }` immediately,
then calls `DomHost::acknowledge_native_microtask` only after successful outbox
admission. That acknowledgement marks *forwarded*, not delivered: the DOM's
mutation observer queued flag remains set until the EventLoop checkpoint runs.
`BindingHost::with_dom` must apply the same rule on its future adapter, including
when its native closure mutated and then returned an error. Nothing inside the
DOM borrow calls JS or drains the loop. The driver owns an accepted request after
`take_scheduling_requests()` and must queue it in EventLoop before another JS
entry; it cannot drop an accepted native request on a downstream failure.

The HTML mutation observer queued flag permits at most one outstanding native
delivery per surrounding agent until its checkpoint. The runtime reserves one
native slot on that premise. A capacity-one JS outbox admits P in its JS slot
and M in the reserved slot; a subsequent Q throws a catchable `TypeError`, with
P,M still ordered. With two JS slots, P,M,Q is admitted in that order. A
future native kind without a flag-derived bound cannot use this reserved slot
without a new bound; it needs a poison/fail-closed protocol before any JS entry
can proceed. A second native request before the first is acknowledged is a
contract error, not normal capacity pressure.
