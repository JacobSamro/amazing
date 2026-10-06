# Executing ECMAScript contexts

Recorded base: 5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58. Source-only until native paired gate.

Expose binding_execution_context_stack_is_empty from explicit executing-context identities, independent of entry_realm, CallFrame count and resource depth. Each Script/Eval/Module entry and ordinary/builtin call installs its real identity; the same initial VM callable body reuses that identity. Direct public Program.run enters separately; internal script VM dispatch uses run_in_current_context to avoid duplicate context construction. Resumption restores the captured callable identity while executing. Coroutine segment enter saves the driver's scalar identity stack and leave/park restores it; parked bodies do not remain marked executing. Job and module declaration-instantiation boundaries are covered. No realm ABI/worker/channel/managed graph change.

HTML create-an-element-for-token step9.2 requires the actual emptiness query before its checkpoint: https://html.spec.whatwg.org/multipage/parsing.html#create-an-element-for-the-token. ECMAScript context execution/suspension: https://tc39.es/ecma262/multipage/executable-code-and-execution-contexts.html#sec-execution-contexts. This is an independently authored bookkeeping adaptation, no code port or new dependency.

Ten after-only groups observe genuine host WindowProxy property hooks (including Script and direct compiled Program paths before any function call), native and HostNative entries, actual AST/Bytecode callable allocations, reentrant abrupt callbacks, direct/indirect Eval, default/explicit construction/foreign realm, Promise/microtask work, generator return/throw and async/asyncgen parking, synchronous/TLA modules. The absent public API is not fabricated in baseline. Whole workspace/doctests and language/Function/Promise/ShadowRealm/module/async-generator corpus run before and after.


Final whole closure includes modern module_start_body and legacy TLA entry. Segment captures the executing module's original scalar context identity rather than a physical native caller frame identity. The same ten groups exercise async-shared module evaluation and real timer/animation-frame dispatch. Existing realm, environment, source, resource admission, channels and strong graph slots remain unchanged.

Job entry preparation follows ECMA9.5 host preparation; resumption and driver restoration follow9.4.7/9.4.8. Primary read2026-10-06: https://tc39.es/ecma262/multipage/executable-code-and-execution-contexts.html#sec-jobs and #sec-runsuspendedcontext.

Tested source f2055b769bec40f30c8ba0313105eaf53e9cb2d2; ready shipping Rust bytes are identical. Original c97 and timer-controls ab4 source freezes remain in scratch. Baseline uses actual published source without the absent API, not a compatibility substitute.
