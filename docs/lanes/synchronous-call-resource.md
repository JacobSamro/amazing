# Synchronous callable resource boundary

Prepared on published `601d78162b8075b5f0b40682cf245c472e4a8bc8`, including genuine Bind and TemplateSite foundations.
The fixed-base Linux qualification is recorded in `evidence/synchronous-call-resource.compact.json`. Root combined-head/full16-shard qualification remains separate.

The existing JavaScript call-depth limit remains 512. It misses transparent
Proxy dispatch before ordinary entry and native accessor recursion consuming
Rust stack before that logical limit. The historical semantic Error.stack
setter crash is a motivating diagnostic on an older branch; this published
base instead writes a raw own Proxy property and its exact call must remain a
nonexhausting control. Current native reentry/wrapper chains establish the
actual resource fault independently. The new independent physical admission
uses bounds of the actual executing OS thread and a volatile local marker.
A small wrapper admits before Proxy/Bound dispatch, realm bookkeeping and
construction prototype lookup. A separate small property-read wrapper also
admits recursive Proxy-handler trap lookup and ordinary prototype traversal
inside callable metadata Get, without changing the existing lookup body.
Direct VM call/resume, asynchronous body bridge
and class-element entry also check. Worker threads initialize their own TLS
bounds before jobs execute. Interpreter policy remains agent-owned across
realm changes; no realm, frame or coroutine schema changes.

Exhaustion creates a RangeError using the active realm's saved intrinsic
prototype, without invoking authored globals or accessors. Admission adds no
JavaScript execution context or logical-depth increment. Existing cleanup and
user Throw identity remain unchanged. The policy can increase required
headroom, but cannot decrease the candidate floor.

## Embedding contract and qualification boundary

An embedding supplies an ordinary OS thread with a stack allocation request
of at least 2 MiB. The matrix uses exactly `Builder::stack_size(2 * 1024 *
1024)`; platform guard exclusion means its usable bounds are smaller than that
request. Qualification must report the actual usable bounds, not call them
2 MiB of usable stack. Supported bound discovery is GNU Linux x86_64, 64-bit macOS, and 64-bit
Windows. Failure to query actual bounds, or a marker outside the cached bounds,
rejects entry. Fibers, alternate signal stacks, unsupported targets, and host
callbacks consuming arbitrary unbounded stack without engine reentry are not
covered. Host callbacks must budget their own stack or raise admission
headroom. The parent authorized actual Linux qualification of this contract; no universal stack-safety claim is made.

The 256 KiB reserve passed the listed GNU Linux debug and release matrix; this
is not a universal headroom bound. Both artifacts passed the minimum-2-MiB native,
AST, HostNative, getter/setter/coercion, constructor/class, transparent 10,000
Proxy/Bound/mixed-chain and coroutine resume matrices. The release receipt's retained diagnostics report 2,093,056 usable bytes and
2,082,983 remaining bytes at tested entry from the 2,097,152-byte allocation
request. The complete matrix exercised exhaustion/error/unwind and repeated
caught calls without host aborts. It does not separately establish a maximum
unchecked interval for every possible host callback. Future artifacts that
exceed the reserve need a whole-policy correction and the complete matrix;
lowering logical recursion depth or increasing test stacks would conceal the
fault.

Alternatives requiring broader architecture are a stack-switching trampoline
with stable interpreter continuation ownership, or fully iterative dispatch
and property/constructor traversal. The former needs a supported stack-switch
ABI; the latter is substantially broader than this boundary. Neither is
implemented or claimed by this ticket.

## Sources and bulk gate

MIT ports are pinned and attributed in `call_stack.rs` and notices:
`rust-lang/stacker` d3ac2df79a409c681329006e9b31c96ab9c5fcae and
`rust-lang/libc` b739c733e06a5f184bae7e296919c15d6ccd8ec8.
Linux pthread attribute APIs are documented at
https://man7.org/linux/man-pages/man3/pthread_getattr_np.3.html and
https://man7.org/linux/man-pages/man3/pthread_attr_getstack.3p.html.
Windows thread bounds use
https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getcurrentthreadstacklimits.
No engine crate dependency or new Send implementation is added.

The supported fixed-base mission ran workspace nextest and docs, all six
minimum-stack groups in debug and release, and 19 whole selectors in each of
default and VM modes before/after. BEFORE passed 4,561 workspace tests; AFTER
passed 4,567, with 17 skips and four doctests on each side. Genuine callable
body assertions distinguish compiled and original AST functions. The compact
evidence preserves every passing file/variant and raw report hashes. Raw stays
on Boat; the integrator owns the separate full16-shard merge gate.
