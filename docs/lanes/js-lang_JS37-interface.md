# JS37 — async-generator Return/Await completion

Fixed initial dependency: 88f64b9f69c6f71037e02e44d7553cda2ed7f7a1 (JS36, VM24, Runtime52/53).
Initial baseline 44105ced6f05f5bb35950e871177fa514484fb1c: actual bytecode factories and equivalent forced Block bodies, each under both public entry backends. Five grouped tests failed exclusively on the coroutine consumer; ordinary async Return negative cases passed. Initial whole language24450 passing files/45223 variants, Promise732/1458, four generator selections unchanged61/122 +48/96 +23/46 +23/46. This initial diagnostic is not the final matched dependency proof. Supplemental baseline196f9126 adds bare/fallthrough quantum, checked constructor-get abrupt timing and borrowed-method realm distinctions.

Upstream: lucid-softworks/lumen, MIT, f6c9860d44fa52249cfd647122edea184d32c863; crates/lumen/src/eval.rs ReturnStatement and yield_one. Exact MIT copyright/license were verified in the cached upstream source. Copied/adapted Return operand Await and injected-yield Return Await use Amazing's own execution-frame detection, Flow/Throw and pending-return representation. The queue/continuation adapter is implemented on Amazing's existing pooled coroutine and internal Promise observation contracts, without dependencies.

## Ownership and execution

Only Return-specific Await can suspend a coroutine-backed async generator in this ticket. Segment.awaits remains unchanged, so other explicit-await/yield/delegation policy is not claimed fixed. An expression-bearing ReturnStatement checks the current own function (async && generator); bare return, fallthrough and nested ordinary/async calls do not inherit that grammar. Injected Return at a yield awaits its payload before setting the pending-return marker. Rejection remains catchable at the yield; fulfilled return is normalized by JS36 before cleanup.

ThreadGeneratorState has a private request queue, active capability, executing/awaiting/await-return flags and body realm. AsyncGeneratorPrototype methods create a capability in the invoked builtin's realm, enqueue, and drive only when idle. A body suspension at Return Await attaches internal reactions to the already prepared intrinsic Promise; it performs no observable then/constructor/species lookup and creates no result capability. Fulfillment/rejection resumes the retained coroutine. Done settles the iterator result directly, with no second Await. Suspended-start/completed Return uses checked PromiseResolve and blocks later queued requests until settlement; it executes no body.

Body segments resume inside the captured callee realm. First yielded iterator result is created in the previous caller realm; ordinary completion uses body realm. Suspended-start/completed Return normalization stays in its active caller/drain realm. This distinction follows AsyncGeneratorYield/CompleteStep/AwaitReturn and avoids blanket body-realm wrapping.

State, request capability, coroutine body environments and reaction callback environments are strongly Rc-owned. No RefCell borrow survives a worker resume or JS execution. Pending callback environments retain the state/capability; completed coroutine resources are released. No collector/tracing schema exists in the current engine. No public value/API, scheduler Job, VM opcode or factory/FDI fields change.

## Native companion

Native explicit Return already uses checked Await inside its saved context. Existing injected Return normalization happens unchecked before saved context installation; borrowed native methods also lack body-realm capture. Those gaps require separately coordinated VM owner changes, not a coroutine-only parity claim. Initial/completed normalization must retain caller/drain scope, while injected Return normalization uses saved callee context. Constructor-get abrupt must inject Throw immediately rather than enqueue an extra reaction. Completed native settlement must not Await again.

Synchronous ForOf IteratorRecord remains a separate parked ticket.

Final matched dependency: 175107c08d43a9fe746a4ee87fe9dd21aa5b72a0, adding VM25 suspended DynamicImport to the same88 source. Tests-only baseline2895ff5100283b73d7ded14c2ee2764a81b2a492 has identical engine/host/runner. Parent separately granted disposal result Await through the same checked ownership helper only when current own function is async-generator and the segment owns its depth; all other coroutine_await/await_value fallback behavior remains.
