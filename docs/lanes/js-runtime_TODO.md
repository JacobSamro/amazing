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

## BindingHost accepted contract — fixed base d42609d

Branch `lane/js-runtime-bindinghost` begins at merged JS-4 commit `d42609d`.
This records method vocabulary that dom-bindings drafted first in its owned
`amazing_bindings::host::BindingHost` trait. The trait, `InterfaceDefinition`,
`Member` and `Operation` are declared in `amazing-bindings`, which depends only
on `amazing-dom`. js-runtime accepts the active trait text, including
`string(&[u16]) -> Value` and fallible `to_dom_string() -> Vec<u16>`, as the
initial synchronous DOM contract. The driver-local `RuntimeBindingHost` implementation
belongs to integrator-assist; the driver imports both crates and lends its
existing parsed `Dom`. Neither JS nor bindings adds a dependency on the other.

The DOM lane's current draft has backend-neutral methods: `install_interface`,
`install_node_constant`, `current_document`, `with_dom`, `node_brand`,
`wrap_node`, `child_nodes`, `is_html_document`, value constructors and checks,
`to_dom_string`, and `type_error`. Keep `type Value: Clone` and `type Error`
opaque, with no `Interpreter`, `Obj`, `NativeFn`, AST or VM register type in the
trait. Its `NodeIdx` and `Dom` mentions are from its permitted DOM dependency.
The method vocabulary covers the first synchronous node APIs. Get/set/call and
broader WebIDL conversion can be added when a binding algorithm actually needs
them, using observable JS semantics rather than representation access.

On the `amazing-js` side, js-runtime will expose matching backend-neutral
capabilities on the Interpreter-facing facade: create an object with a chosen
prototype; define own data/accessor properties with explicit flags; define a
native function from a per-interpreter dispatch ID, name, length and
call/construct policy; get/set/call/construct with proper receiver and
`newTarget`; perform fallible JS conversion to UTF-16 DOMString; create and
inspect private branded wrapper identities; and preserve the original thrown
value. These operations must run through either execution backend without
exposing function body representation to the driver.

Property flags carry writable/enumerable/configurable bits. Definitions are
own-property operations and never call inherited setters. `native_function`
installs a function object with a runtime-owned `NativeId`; a per-interpreter
dispatcher receives that ID, rooted receiver and arguments, call versus
construct, and `newTarget`. The dispatcher is instance-owned and its callable
handle is cloned before invocation so no mutable dispatcher borrow survives a
reentrant getter or callback. No function object stores an `Operation` pointer
through a global/thread-local registry, and no JS value is hidden in a Rust
closure without a declared root. The VM can call the same `NativeId` dispatcher
without executing an AST body or reading a tree-walker frame.

`Value: Clone` in `BindingHost` must eventually represent a short-lived rooted
local token. Under the future collector, cloning cannot extend it beyond the
native entry; use after scope end or on another heap must be rejected. The
runtime must trace receiver, arguments, returned value and original thrown
value until each native entry closes; nested entry adds a new scope and
promotes its result/error into the caller scope. The current `Rc` value model
does not claim these collector guarantees. Long-lived listeners,
jobs and callbacks use separate persistent roots with explicit release. The
wrapper cache is weak and does not root every DOM node. JS-4 scheduler tokens
remain interpreter-owned roots and dispatch one job at a time.

`Identity` includes realm, arena, node slot and reuse generation. Today's
`NodeIdx` is arena-local and non-reused, so the driver supplies stable arena
identity and a generation of zero until reuse exists. A brand is engine-private,
not an own JS property. `wrap_host` atomically checks the weak canonical cache,
validates the identity, and returns the existing wrapper or creates one with
the correct prototype. `host_identity` rejects foreign realms, foreign arenas,
stale slots and wrong brands. The existing document wrapper, query results and
legacy createElement must go through this same cache. `child_nodes` uses a
separate branded live NodeList identity keyed by parent, not a snapshot array.

The adapter maps the concrete `BindingHost` methods as follows:

| BindingHost methods | Owner and rule |
|---|---|
| `install_interface`, `install_node_constant` | Driver translates definitions to runtime object, prototype, native function and descriptor primitives. `None` constructor is illegal to call/construct; actual constructors require `new` and preserve `newTarget`. Migrate legacy own properties that shadow installed prototype members without deleting unrelated APIs. |
| `current_document`, `with_dom`, `is_html_document` | Driver supplies the same parsed arena and current document. `with_dom` holds no JS value borrow and runs no JS; native result data is converted or wrapped only after release. Map `DomError` to JS TypeError or genuine DOMException as specified. `DocumentConstructor` creates an XML document. |
| `node_brand`, `wrap_node`, `child_nodes` | Runtime private brand and canonical weak cache, with driver validation of arena-local `NodeIdx`. Same-object live childNodes wrapper is stable per parent. |
| value constructors/checks, `to_dom_string`, `type_error` | Runtime owns JS conversion and thrown value identity; binding algorithms own WebIDL policy. DOMString conversion returns UTF-16 code units including lone surrogates and may reenter or throw. |

`with_dom` also observes JS-4 native mutation forwarding: after releasing the
arena borrow, forward a pending MutationObserver descriptor into the bounded
outbox, even when the DOM closure returned an error. Acknowledge only successful
outbox admission. The DOM queued flag remains set until checkpoint. The driver
owns accepted requests after `take_scheduling_requests()` and appends them to
its event loop before another JS entry.

The remaining string gap is `JsValue::Str(Rc<str>)`: it cannot represent lone
UTF-16 surrogates, so `string(&[u16])` and `to_dom_string` cannot yet fulfill
their WebIDL contract. The runtime needs lossless `JsString` shared with both
backends, after js-lang adds the additive AST field. Do not claim string
methods work by replacing invalid code units.

The active trait text has js-runtime's acceptance for this first synchronous
slice. The additional commented installation vocabulary in the DOM draft still
needs one change before becoming active methods: `construct_only: bool` on
`create_native_function` cannot distinguish call-only methods from functions
that permit both call and construction. Use an explicit `CallOnly`,
`ConstructOnly`, `CallAndConstruct` policy. The other proposed installation
methods (`global_object`, `create_object`, `define_data_property`,
`define_accessor_property`, `get_property`, `set_property`) are accepted with
ordinary JS receiver, descriptor and exception semantics. Confirm the driver can identify
the same parsed arena and its document wrapper and can provide per-instance
native dispatch. The initial acceptance cases are a single installer used by
both execution backends, stable Node/NodeList identity, constructor `newTarget`,
reentrant DOMString conversion without an arena borrow, preserved thrown-object
identity, and P/native-M/Q scheduling order through the adapter. These are
contract tests, not claimed WPT gains.

### Non-string runtime capability slice

The first runtime slice adds `FunctionBody::HostNative(HostFunctionId,
HostCallPolicy)` and a per-interpreter dispatcher. Native entry receives rooted
receiver, arguments and optional `newTarget`; nested entry clones the callable
handle before dispatch, so no dispatcher slot borrow spans reentry. The public
binding facade creates objects and native functions, defines data and accessor
descriptors, performs get/set/call/construct, non-string boolean/number/nullish/
callability checks, and preserves thrown JS value identity. There is no
`Operation`, AST or VM frame type in that facade. Existing built-in `NativeFn`
continues to work, but a VM can dispatch the host ID through the same facade.

`HostHandle` combines arena, slot, generation and private brand. The runtime
adds realm identity in an internal `JsObject` slot and uses a weak per-interpreter
canonical wrapper cache. The driver must validate arena/slot/generation against
its live DOM before using a returned handle. No BindingHost string method or
driver installer is claimed by this slice. Windows amazing-js gate on fixed
base `d42609d`: nextest 174/174, separate doctest 1/1.

### Lossless JS string migration decision

The integrator requires an opaque `JsString` in runtime-owned `value.rs` with
lossless UTF-16 code-unit storage and method-only access; `JsValue::Str`
migrates to it. Length, indexing, `charCodeAt`, concatenation, comparison and
ToString use code units, and regex execution uses amazing-regex's existing
`exec_utf16` path. The representation may later optimize one-byte strings
without changing callers. js-lang first adds a UTF-16 field beside existing
lexer/AST string fields, preserving additive `ast.rs` compatibility. Only then
does js-runtime migrate the runtime value representation. No BindingHost string
method is implemented before `JsString` lands; object, function, property and
private identity capabilities may proceed meanwhile. Exit evidence is full
Test262 before/after with no regression from 10,314 passing files and a JS →
DOM → JS lone-surrogate round-trip test.

### JSRT-2 testharness scaffolding builtins (merged)

On fixed base `c82ca69`, runtime added string-hint OrdinaryToPrimitive and
Error.prototype.toString, Array.prototype.toString through generic join, and
per-interpreter seedable Math.random. Windows amazing-js nextest finished
178/178 with one doctest. Twice-stable pinned Test262 directory counts moved:
Error.prototype.toString 0→8/17, Array.prototype.toString 0→4/11,
Array.prototype.join 8→13/23, Math.random 0→1/5. The full pinned sweep moved
10,314→10,386 passes across 53,872 files, with 72 fail→pass transitions and
zero lost file or variant passes. Classification hash after JSRT-2:
`b7d67bc195c033d623b3cf5b7549d7b23599b05c81a2ba8b006ee5ba72c73ef3`.
The 41 dom-bindings conversion diagnostics are a follow-up measurement; their
raw count is not a pass promise.

### JSRT-3 facade and interpreter split

Fixed base `133db58` combines JSRT-2, JS-5 private-name parsing, and the
Throw::describe hotfix. Its measured pinned Test262 baseline is 10,575 passes
with classification hash
`d2251207a676957b84fb0d8556859f96146dd027c9e45ab2cd0ce65c05cd8b96`.
The mechanical `interp.rs` split into `interp_install.rs` reconstructs the
base file byte-for-byte, and the Windows full sweep preserves that exact hash.
It was committed separately as `7222c62`.

The subsequent facade adds explicit-receiver `binding_call`, realm
`binding_create_array`, and ECMAScript `binding_same_value`. `HostHandle` slot
and generation are u64 to map DOM-7 `PlatformKey` without narrowing. The
extended DOM-7 BindingHost trait is accepted: runtime supplies call, arrays,
SameValue, private wrapper identity and conversions; the driver owns native
key validation, callback retention, interface constants, and installation.
Cloned JsValue values are strong Rc references in the current runtime, so a
driver registry holding a clone keeps its callback alive. A callback capturing
its own target can form an Rc cycle until the future collector traces and
releases those registry roots. Windows amazing-js nextest passed 182/182 with
one doctest. The full Test262 classification remained byte-identical to the
10,575-pass combined baseline, with zero file or variant transitions. The
workspace gate passed 1,438/1,438 nextest tests (3 skipped) and 2/2 doctests.

### JSRT-4 legacy platform object and iterable decision

Fixed base: `0244456` (includes JS-6 additive UTF-16 lexer/AST fields).
The DOM-8 draft adds `BindingHost::define_legacy_platform_object` and
`define_iterable`. Runtime accepts these entry points. The integrator accepted
four extra declarations: `override_builtins`, optional indexed and named
setters, and an optional named deleter. The host reports a native `u32`
supported index count and `Vec<Vec<u16>>` supported names. The embedding
converts DOM `Operation` tokens into the runtime's per-interpreter callbacks;
amazing-js does not depend on amazing-bindings or borrow the DOM. Callbacks run
without an active JavaScript object borrow, allowing a short DOM borrow and
reentrant value wrapping.

Runtime matches hooks using private realm and interface brand. Indexed and
named properties remain virtual: getters and own-key enumeration query the
current host state rather than materializing own properties. The named
visibility check respects ordinary own/prototype properties unless
`LegacyOverrideBuiltIns` applies; `LegacyUnenumerableNamedProperties` filters
enumerable key consumers. Indexed value iterators keep a live target and index
and read current values at each `next`. Their prototype has `next` and
`@@iterator`; the interface prototype receives `@@iterator`, `keys`, `values`,
`entries`, and `forEach`. `forEach` is the same function object as
`Array.prototype.forEach`; its generic callback path uses the live host count
and indexed presence checks only for legacy wrappers.

Runtime values still store Unicode scalar strings as `Rc<str>`. A supported
name with an unpaired surrogate cannot become a JavaScript property key until
the next JsString ticket; the bridge returns an explicit TypeError rather than
silently replacing code units. This limits DOM-8 coverage until that migration.

JSRT-4 fixed-base Test262 before/after both pass 10,594 of 53,872 files
(33,011 fail, 17 timeout, 9,956 unsupported, 294 skipped). There are zero
lost passing files or variants. Classification SHA-256 changes from
`d80b8fd8eb5e086aa4a95b7c2619d0ab04e636ee372ff2c2a0f5967c3479c8ea`
to `7554e7619859569ae012ef12c3cdb5f23744b85ce58548aa9934ca5d40af0365`
for exactly one fail-to-fail diagnostic in both variants of
`built-ins/JSON/stringify/property-order.js`. Frozen-host probes reproduce a
base `RefCell already borrowed` panic at `value.rs:67`; the candidate reaches
the test's existing JSON key-order assertion and fails normally. The
integrator accepted this explained hash difference on 2026-09-23. Windows
amazing-js nextest passes 190/190 with one doctest; fixed-base workspace
nextest passes 1,503/1,503 with three skipped and doctests pass 2/2. No
`crates/amazing-js` drift from fixed `0244456` to current `origin/dev` at final
review.

### JSRT-5 lossless UTF-16 JavaScript strings

Fixed base: `7e0ec7f` (JSRT-4), with a measured 10,594/53,872 pinned
Test262 baseline. `JsValue::Str` now holds opaque `JsString(Rc<[u16]>)`.
Literals, template quasis, concatenation, ToString, property keys, Map string
keys, primitive string indices, and String prototype methods carry exact UTF-16
code units. The BindingHost facade accepts and returns code units; a Windows
fake-host test verifies a JS → DOMString → JS lone-surrogate round trip. String
length and indexing are code-unit based, and regex input and captures preserve
those units. `normalize` uses a deliberately bounded Latin table;
`localeCompare` uses code-unit ordering, pending a locale collation layer.

`amazing-regex::Regex::new` still takes `&str`. Well-formed patterns continue
through that constructor, while a pattern containing a lone surrogate raises a
typed SyntaxError stating the temporary limit. The integrator queued a
`Regex::new_utf16` constructor as js-lang JS-8 after JS-7. Runtime never
replaces pattern code units to make them compile. The integrator also authorized
mechanical JsString adapters in `amazing` CLI/WPT code and the GPUI storage
test. The WPT JSON encoder escapes an unpaired surrogate as `\\uXXXX`; valid
Unicode strings retain their previous JSON output.

JSRT-5 fixed-base evidence on 2026-09-23: Windows amazing-js nextest passes 193/193
and its doctest passes 1/1. Fixed-base workspace nextest passes 1,560/1,560
with three skipped; workspace doctests pass 2/2. The first complete after
Test262 run on frozen host SHA-256
`8d49350067252aa00fbe96e2c8cae68271a6a7dfc401097737e0f1c9fcb25c56`
passes 10,865/53,872 files (32,741 fail, 16 timeout, 9,956 unsupported,
294 skipped), compared with the 10,594-pass baseline. `built-ins/String`
passes 481 versus 259. There are 270 fail-to-pass and one timeout-to-pass
file transitions, with zero lost baseline file or variant passes. After #1
classification SHA-256 is
`e78552fd9f7d7e17bfabe9b0e2e73aa1ff40b92c3641ff055076aad78531085f`.
The second complete after run reproduced all 53,872 per-file statuses and
reasons, the same counts, and the exact classification hash above with frozen
host SHA-256
`8d49350067252aa00fbe96e2c8cae68271a6a7dfc401097737e0f1c9fcb25c56`.
The branch was rebased without conflicts onto integration base `8151f5c`;
its updated scheduler adapters and the JsString conversions coexist in the
resulting source. The integrator-authorized driver adapter now uses
`binding_string`, `binding_string_units`, and `binding_to_dom_string`, so a DOM
Text node round trips an unpaired surrogate without replacement. The refreshed
Windows integration tree passes 1,671/1,671 workspace nextest cases (three
skipped), 2/2 doctests, and 129/129 amazing crate cases. An eager iterator
safety bound of 100,000 entries avoids
unstable runner deadline races in four iterator-close tests pending lazy
consumption.

### JSRT-6 real Symbols and property keys

Fixed base: `cb3064b`. `JsValue::Symbol` has identity separate from its
optional UTF-16 description. Thirteen well-known symbols have stable identities;
ordinary symbols use a distinct ID range, and `Symbol.for`/`keyFor` use an
interpreter registry. Object property storage now holds a `PropertyKey` enum so
computed access, assignment, deletion, descriptors, object spread, own-key
enumeration, and the binding facade never stringify a Symbol. Own-key order is
array indices ascending, remaining strings in insertion order, then symbols
in insertion order; string-only consumers such as `Object.keys` and `for...in`
exclude symbols. `@@toPrimitive`, `@@hasInstance`, and `@@toStringTag` enter their
respective runtime operations. JSRT-4's iterable string stand-ins are replaced
by the real iterator symbol. The integrator granted a method-only change to
the driver `install_interface_class_string` hook; DOMException's Error
prototype hook was already installed.

The pinned full Test262 BEFORE sweep on win-001 at fixed base `cb3064b`
finished with 10,965 pass, 32,636 fail, 16 timeout, 9,961 unsupported, and
294 skipped across 53,872 files. Frozen host SHA-256 is
`b12757b8d86319f8dd05631bc406c5516384f5d0c99017cb9d7f9738ad84493f`;
classification SHA-256 is
`bfdcb4a0cc96fa1eafe5ac14af911860d6c00b741be5426917a228d5ae0382c5`.
The measured baseline is 100 passes above JSRT-5's prior 10,865; use this
fixed-base measurement for JSRT-6. The first corrected full AFTER sweep on
frozen host SHA-256
`e125b6f25ab2f740f2f28807cd955977781987e260a33de2791a04c5119c8731`
passes 11,965 files, with 31,624 fail, 16 timeout, 9,973 unsupported, and
294 skipped. Its classification SHA-256 is
`8461750b5e6a7d8f1dcacf040e057ad92a3bb4147551e56a1e4538ed30be0798`.
There are 1,000 fail-to-pass transitions, 12 fail-to-unsupported transitions,
and zero lost file or variant passes. The 12 unsupported files reach the
existing `host.createRealm` limit: eleven cross-realm Error/RegExp files and
`staging/sm/Symbol/toPrimitive.js`. `built-ins/Symbol` rises from 0 to 52
passes; its remaining 29 failures and 17 unsupported files are separately
reported. The fixed-base Windows workspace gate passes 1,710 nextest cases
with three skipped and both doctests. The second full AFTER sweep on the
same frozen host reproduces all 53,872 file statuses and reasons, all 102,918
variant statuses and reasons, the same counts, and the exact classification
hash above. js-vm's `FunctionBody::Bytecode`
value.rs addition is coordinated as an additive sibling change.

The final integration base is `2c1a3e82160333425ce10e97d2a84d4e550bc26b`
(JSVM-1), with this ticket at `147643c7b3edc26896a064629695404dbf80ff64`.
After the clean rebase, the win-001 lane tree was refreshed to that base and
this ticket's files re-synced. The final Windows workspace gate completed:
`cargo nextest run --workspace --locked` passed 1,745 tests, with zero failures
and three skipped; `cargo test --workspace --locked --doc` passed both doctests.
The `amazing-js` scoped gate passed 216/216 on this integration base. The
intervening `origin/dev` advance to `7c6824b8a12a50ea6ba99688678b98617bd27f26`
did not touch this ticket's owned or granted files, so the final gate remains
on this recorded fixed base; the integrator gates the merged head.

### JSRT-7 derived construction and binding facades

Fixed integration base: `f8bdb330a5c16c48ebe17acd9d0d348461ddd2cb`.
Derived constructors now leave `this` uninitialized until `super()` constructs
the parent with the original `newTarget`. They reject early `this`, repeated
`super()`, and invalid primitive returns; default derived constructors forward
arguments. Native parents honor `newTarget.prototype`, while ConstructOnly host
parents can return their canonical wrapper. Arrow closures retain lexical
`super()` and `new.target`. `Reflect.construct` and the native callback
`current_new_target()` facade expose the same construction path.

RegExp construction accepts UTF-16 patterns through `Regex::new_utf16`; the
binding facade has a full-match operation that distinguishes invalid patterns.
The `unicodeSets` getter and `v` flag are exposed. A WebIDL pair-iterable
facade provides live entries, keys, values, and forEach over host count/pair
hooks, including the iterator symbol.

On Linux, pinned Test262 revision `3655e746` over 53,872 files and 102,918
variants moved from 12,088 to 12,127 passing files, with 39 fail-to-pass
transitions and **zero lost file or variant passes**. One existing timeout in
`staging/sm/String/string-upper-lower-mapping.js` became a normal failure.
`language/statements/class/subclass` rose from 33 to 42 passes; the expression
subclass and statement super selections stayed at 16 and 5 passes;
`built-ins/RegExp/prototype/unicodeSets` rose from 29 to 31. The before and
after classification SHA-256 values are
`1cb29841f3337109ecd73bb1dfb1931590622ffcc1a6abacbf5cbd47e95c0cf2`
and `95745cec8a4f22a67ef0396724642fd0f25173ef8d7dd7a31485461903b3b524`.
The corrected Linux workspace gate passed 1,788 tests with two skipped and
both doctests; `amazing-js` passed 221 tests and its doctest.

The final Windows crate/workspace gate ran on `win-002` from the fresh JSRT-7
worktree: `amazing-js` nextest passed 221/221; workspace nextest passed
1,791/1,791 with four skipped; both workspace doctests passed. The Windows
Test262 BEFORE sweep on the same fixed base had 12,088 passing files; after
win-002 routing replaced win-001's queued candidate job, the complete pinned
Linux before/after corpus is the zero-loss evidence for this ticket. The
integrator rechecks the merged head.

### JSRT-8 priority built-in interrupt

Fixed Test262 comparison base: `82d273a` (JSB-2). The full pinned `3655e746`
corpus moved from 20,999 to 23,664 passing files and from 41,000 to 46,285
passing variants: 2,665 fail-to-pass files and no lost file or variant passes.
Before and after classification hashes are `d6dad40499b358008e52a69ae8d82e91441dcee111f1de3822ce320cb8ddc254`
and `0001b78eb38467a21e4f87bbefee39eaf4a6f7ff730838f4e1b5f5057d01382a`.

`for...in` now walks own string keys and prototypes with descriptor checks,
shadowing, deletion awareness, and non-enumerable exclusion. Array methods and
statics include stable `sort`/`toSorted` and the missing §23.1.3 surface;
NativeError constructors inherit from `%Error%`. The Object, Function,
Boolean, and Symbol surfaces were audited. Class prototype methods and
`constructor` have non-enumerable descriptors. `Array.fromAsync` covers
synchronous iterable inputs and settled values; true asynchronous iterators
and pending input promises still take an explicit unsupported path.

The pinned Test262 file `built-ins/Object/freeze/typedarray-backed-by-resizable-buffer.js`
asserts TypeError even for zero-length resizable-backed TypedArrays despite
comments saying they should freeze successfully. The runtime currently follows
those assertions; revisit this when the corpus is updated or the integer-indexed
integrity model is made complete.

## Queued JSRT-8 internal-slot cleanup

Move `[[...]]` pseudo-properties such as `[[NumberData]]`, `[[StringData]]`, and `[[DateValue]]` out of ordinary property storage into a real `JsObject` internal-slot field. Reflection through `getOwnPropertyNames`, `for...in`, JSON, and `Object.create` must never reveal them. Coordinate the call-site migration with js-stdlib and js-binary. The current Number constructor has a narrow non-enumerable descriptor fix to preserve Object.create conformance until this migration.
## JSRT-9 — multiple realms in one agent (after built-in audit)

Give each realm its own intrinsics, global object, and global environment in the shared agent heap. Switch to a callable's [[Realm]] during a call, allow shared cross-realm objects, and expose host creation with a custom Window global for HTML-25 iframes. Add `$262.createRealm` to both Test262 hosts and report the resulting full-corpus gain; preserve `intl402: true`.

Accepted seam: `binding_create_realm(global) -> RealmHandle` plus realm-scoped eval and call. Include a host-created WindowProxy exotic whose target global swaps on navigation while identity remains stable; one agent-wide microtask/job queue; current, entry, and incumbent realm accessors; realm-specific Error prototypes; and Test262 `$262.createRealm` through this same API. Ask html for any remaining hooks before locking.

HTML-25 additionally needs scoped realm entry/restoration for realm-specific `binding_global_object`, constructor/prototype installation, and `binding_wrap_host`; WindowProxy get/set hooks with entry/incumbent realm and target origin for cross-origin checks after navigation; and host-native dispatch/`HostHandle` identity across parent-to-child calls, with child-realm error intrinsics.

## JSRT-8 Object property collection and iterable consumption (2026-09-26)

Collection code `e0d61eb8` snapshots keys while rechecking each property's
current enumerability after earlier getters, snapshots converted descriptors
before any defineProperties writes, and creates own descriptor-result keys
without invoking inherited setters. It also requires Object.create's prototype
argument, reflects UTF-16 primitive string descriptors, and exposes host
collection descriptors through the existing legacy hooks.

On fixed base `4b8555a2`, pinned full Test262 moved from 30,404 to 30,415
passing files and 58,777 to 58,799 passing variants, with no lost file or variant
passes. Windows integration `6936733c` passed 2,469 workspace tests, skipped
seven, and passed both doctests. The earlier compositor failures were resolved
by the merged COMP-13b changes. Evidence: `js-runtime_object_collection_evidence.json`.

Iterator follow-up `33d7cf43` makes Object.fromEntries and Object.groupBy
consume one item at a time, closes iterators on consumer failures while
preserving the original exception, and propagates iterator-step failures
without closing. Live array growth during groupBy callbacks is observable.
Its complete fixed-base corpus moved from 30,415 to 30,418 passing files and
58,799 to 58,805 variants, again with no lost passes. Linux crate tests passed
327 with one skipped, plus the doctest. Windows passed 2,473 tests with seven skipped plus both doctests. Fresh
integration d33ee2f9 (including upstream standard-library changes) passed
31,537 Test262 files / 61,040 variants; Windows passed 2,482 tests with eight
skipped plus both doctests. No prior passing file or variant was lost.
Evidence: `js-runtime_object_iterator_evidence.json`.

Next Object audit findings: boxed String entries still lack indexed exotic
properties (`Object.fromEntries([Object('ab')])`). Five fromEntries assertions
were blocked by the missing ordinary constructor prototype backlink; local dev
now supplies the tree-walker backlink, so the new integration corpus will
measure that combination. VM prototype construction and descriptor attributes
need the corresponding audit. Internal-slot cleanup remains queued above,
with call-site migration requests recorded for js-stdlib, js-binary and js-vm.


## JSRT-9 same-agent realms (completed, 2026-09-26)

One Interpreter owns the agent job queue, scheduler tokens, timers, and Symbol.for registry.
RealmHandle identifies a global plus its intrinsics and host dispatcher. Cross-realm calls
enter the callable's creation realm and restore the caller on either return or throw.
The embedding installs each document's bindings with binding_with_realm.

Public entry points in interp_realms.rs: binding_create_realm(global),
binding_with_realm, binding_eval_in_realm, binding_call_in_realm, binding_realm_global,
binding_current_realm, binding_entry_realm, and binding_incumbent_realm.
Use binding_brand_global for Window host receiver branding, and binding_global_this
for the script-visible global (binding_global_object remains the installation target).
WindowProxy entry points: binding_create_window_proxy, binding_retarget_window_proxy,
binding_set_realm_origin, binding_set_window_proxy_access_hook. Navigation retains
proxy identity while replacing its target realm. Origin policy is supplied by the host;
unconfigured cross-origin property access is rejected.

Reference: https://html.spec.whatwg.org/multipage/nav-history-apis.html#the-windowproxy-exotic-object
WindowProxy has an immutable prototype, stays extensible, and refuses PreventExtensions.
Test262 createRealm is enabled while retaining intl402.

Host installation order: allocate global → binding_create_realm(global) →
binding_create_window_proxy(realm) → binding_with_realm(realm, install bindings and
binding_brand_global(window_handle)). Window getters return binding_global_this().
Expose the returned proxy as iframe contentWindow; on navigation, prepare the next
realm's bindings and use binding_retarget_window_proxy with the existing proxy.
The host supplies origin policy and each document's native dispatcher.

Constructor fallback records intrinsic constructor/prototype identities at realm
initialization. Foreign newTarget fallback survives reassignment of global
constructor names. Modules share the root agent and restore its environment.
Eval declarations are validated before bindings are created, and strict indirect
eval retains its own scope.

Linux gate on fixed base 500bf39f: 2,744 workspace tests passed, 10 skipped,
124.639 seconds; four doctests and ten realm regressions passed. Full Test262:
31,650 → 31,989 passing files (+339), 61,229 → 61,829 passing variants; zero
lost files, one strict TypedArray sort timeout. A complete 96-file TypedArray
directory recheck with both frozen binaries at the same 60-second deadline
passed both sort modes and lost zero variants (33 → 37 files, 66 → 74 variants).
Replacing all 96 directory records in both sweeps gives 31,651 → 31,990 files
and 61,230 → 61,831 variants, zero lost passes. These are combined measurements,
not a claim that the original single full sweep had zero lost variants.
The recheck also passed 350 crate tests (one skipped) and its doctest.
Evidence: js-runtime_realms_evidence.json. DOM/iframe WPT gains remain for
adopting lanes to measure; embedding code supplies cross-origin policy.

## JSRT-10 runtime intrinsic and Error-family audit (completed, 2026-09-26)

Audit runtime-owned constructor lengths/prototype descriptors and Promise method
attributes, class [[Call]] rejection, and caller-realm derived-constructor
completion errors. Complete Error message/cause descriptors, AggregateError,
SuppressedError, Error.isError, and intrinsic AggregateError rejections from
Promise.any. Specification: ECMA-262 §20.5.

Nullish GetV correction is tracked separately. A diagnostic corpus with the
correction exposed ArrayBuffer's optional-options lookup and previously masked
missing Error.prototype.stack accessors (12 lost variants in the first 13,602
completed files). The core correction remains necessary; js-binary has the
options request, and Error-stack support needs a proper stack representation.
Those false passes are recorded as debt, not claimed as working accessors.
The current audit excludes that correction while its dependencies are fixed.

Final Linux gate on fixed base 7766dbfe: 2,748 workspace tests passed,
10 skipped, 67.601 seconds; four doctests passed. Full pinned Test262:
31,990 → 32,182 passing files (+192), 61,831 → 62,215 passing variants,
zero lost files or variants at the same 60-second deadline. The baseline is
JSRT-9's documented full sweep plus complete directory recheck; this candidate
result is one full sweep, with no recheck needed. Evidence:
js-runtime_intrinsic_audit_evidence.json.

Next: dynamic code must use the realm global environment during module
execution, and bound function exotica must forward calls/construction without
an extra realm entry, prototype allocation, or observable argument iteration.

## JSRT-11 dynamic code and bound-function realm execution (completed, 2026-09-26)

Indirect eval, Function, and host realm eval use the realm's global environment
even while a module environment is active. Host eval restores the prior environment
on return or throw. Eval returns non-String inputs without coercion. Bound calls
and construction forward to their target without an extra realm entry or prototype
allocation, and consume stored arguments as a private List.

On fixed base af6364b1, Linux passed 2,751 workspace tests (10 skipped) in
59.342 seconds plus four doctests. Full pinned Test262: 32,182 → 32,189 passing
files and 62,215 → 62,224 variants, with zero lost passes at timeout60/jobs4.
Evidence: js-runtime_realm_execution_evidence.json.

References: ECMA-262 §19.2.1.1 PerformEval and §10.4.1 Bound Function Exotic Objects.
Next bulk area: generic Array object operations, callback boxing, species results,
sparse holes, and throwing Set/Delete semantics.

## JSRT-12 generic Array object protocol (completed, 2026-09-26)

Array callbacks box primitive receivers, read ordinary length before callback
validation, and preserve live HasProperty/Get observations. Map, filter, slice,
concat and splice allocate through ArraySpeciesCreate, retain sparse holes, and
create result data properties without invoking inherited setters. A foreign
realm's intrinsic Array constructor defaults to the calling realm's Array.
Mutating methods use throwing Set/Delete; splice follows observable operation
order. Object.prototype.toString recognizes primitive wrapper brands using the
existing internal-slot representation.

The WebIDL value-iterable test fixture now installs its interface length getter;
forEach snapshots that ordinary length once while indexed values remain live.
The complete Array directory gained 237 passing files with no lost variants;
360 crate tests and its doctest passed. Final Linux gate on fixed base a8b75377:
2,792 workspace tests passed (10 skipped) in 61.639 seconds, plus four doctests.
Full Test262: 32,189 → 32,459 passing files (+270),
62,224 → 62,764 passing variants, zero lost passes.
Evidence: js-runtime_array_protocol_evidence.json. Existing resource limits,
missing Proxy support, and property-based primitive slot storage remain separate debt.

Next: synchronous Array.from/of construction, mapping and iterator-close order;
then Array.fromAsync suspension and proper private primitive slots with owner
coordination for stdlib/binary/structured-clone callers.

## JSRT-13 synchronous Array factories (completed, 2026-09-26)

Array.from/of construct through the generic constructor path: they invoke the
receiver (or bound newTarget) with the specified argument list rather than
always allocating an ordinary Array. Array.from caches the iterator's `next`
method once and steps it lazily, so a mapper function observes live mutation
of the source between steps. Mapper and own-data-property-definition errors
close the iterator while preserving the original error as the thrown value;
failures from the iterator's own `next`/`done`/`value` steps propagate without
an extra `IteratorClose`. Both factories create own data properties (never
invoking an inherited setter) and finish with the spec-required throwing
`Set` of `length`. Array-like (non-iterable) input still boxes a primitive
receiver and reads live indices at each step.

On fixed base 6683ceaa, Linux workspace: 2,825 tests passed (10 skipped) in
65.852 seconds, plus four doctests. The raw single `--jobs 4` full pinned
Test262 sweep at the 60s deadline moved 33,092 → 33,112 passing files
(64,030 → 64,070 variants), but recorded 4 lost variants across two
`staging/sm` files unrelated to Array.from/of:
`staging/sm/TypedArray/sort_modifications.js` (timeout, both variants) and
`staging/sm/regress/regress-610026.js` (host.abnormal-exit, exit -9, both
variants).

Diagnosis (both retained, not silently reclassified): direct single-process
probes of both frozen before/after hosts show `regress-610026.js` completes
in ~4.5s using ~3.9GB RSS on both binaries in isolation — no timeout, no
abnormal exit — so the exit -9 under the full sweep is this sandbox's OOM
killer reacting to several `--jobs 4-8` concurrent Test262 workers competing
for the 8GB box, not a fault introduced by this change. `sort_modifications.js`
completes in ~49-54s wall time on both hosts in isolation (11 TypedArray
constructors x 5 size classes x 3 sorts each), leaving only 6-11s of margin
under the pinned 60s deadline in both old and new code; it is a pre-existing
marginal/flaky timing case on this hardware (the same file caused JSRT-9's
one documented timing loss). A complete recheck of both directories
(`staging/sm/TypedArray/`, 96 files, and `staging/sm/regress/`, 106 files)
with both frozen hosts at the identical 60s deadline but `--jobs 1`
(serialized, no concurrent workers) lost zero variants. Substituting those
202 directory records into the raw sweep (the JSRT-9 combined-measurement
method) gives 33,092 → 33,114 passing files (64,030 → 64,074 variants),
zero lost passes; this is a combined measurement, not a claim that the
original single `--jobs 4` sweep itself had zero lost variants.
`built-ins/Array` gained 19 files (2,931 → 2,950), `staging/sm/Array` gained
3. Evidence, including the raw 4-loss comparison and the directory-recheck
numbers, is retained in full at
`docs/lanes/js-runtime_array_factories_evidence.json`.

Not implemented: Proxy support and Array.fromAsync suspension for pending
input remain separate areas (Array.fromAsync's synchronous-iterable path was
already covered by JSRT-8).

## JSRT-14 private and public class elements (completed, 2026-09-26)

Implements §15.7's per-class private environment and instance/static element
templates: `PrivateName` identity (`value.rs`, `Rc<str>` + `Rc::ptr_eq`, so
textually identical `#x` in different classes never collide),
`EnvHandle::class_private`/`resolve_private` (§9.2's NewPrivateEnvironment,
chained through parents so a nested class's own `#x` shadows an enclosing
one), and `crates/amazing-js/src/interp_class_private.rs`
(`define_field`/`initialize_instance_elements`/`run_static_elements`/
`private_field_get`/`set`/`has`). `eval_class_inner` now builds one
`ClassElements` template per class (instance fields, private instance
methods/accessors merged by `PrivateName` identity regardless of getter/setter
declaration order) and one static-element list (fields, private static
methods/accessors, static blocks, run once with `this` = the constructor).
Instance elements replay at all three construction sites: base-class
allocation, the implicit default derived constructor's `super(...args)`, and
an explicit `super()` call (`CallFrame` gained `own_function` to find the
running class's own template from the `super()` call site). Member access
(`obj.#x` read/write/call, `#x in obj` brand check, `delete obj.#x` early
SyntaxError) routes through a new `Reference::Private` variant instead of
`PropertyKey`, since a private name is never a property key. Bonus fix in the
same area: `new.target` now resolves from `CallFrame.new_target` instead of
throwing "not implemented" (~85 Test262 occurrences).

While scoping this ticket's failure cluster (5,727 Test262 failures tagged
"private names are not yet supported"), found and fixed two more bugs
in-lane, both pre-existing and unrelated to private fields:
- `eval_class_inner` installed every method — including `static` ones — onto
  the prototype; only instance methods belong there (static methods belong on
  the constructor object itself). Public/private instance and static fields
  were also previously unhandled (`ClassMember::Field` was a no-op, so public
  fields silently did nothing).
- `bind_pattern_mode`'s `Binding::Array` arm (`interp_helpers.rs`) swallowed
  every error from array-pattern destructuring via `.unwrap_or_default()`,
  masking any `TypeError`/`RangeError` a bad iterable should raise, and
  collected via `Interpreter::iterate`'s eager collect-everything, which is
  wrong for a pattern without a trailing rest element (§8.6.3 pulls only as
  many values as the pattern declares, then performs IteratorClose — an
  iterator that legitimately never signals `done` must not be drained first).
  Replaced with `PatternIter`, a lazily-stepped iterator (index fast path for
  an array whose `[Symbol.iterator]` is still the untouched intrinsic via new
  `Interpreter::has_original_array_iterator`, also applied to `iterate`'s and
  `custom_iterator`'s own array fast paths so a deleted/overridden
  `Array.prototype[Symbol.iterator]` is honored). While fixing this, also
  found that both `iterate`'s generic protocol loop and `exec_for_of`'s custom
  iterator loop read a `next()` result's `done`/`value` with `get_own`
  (raw own-property probe) instead of the full `[[Get]]` — a `value` defined
  as a throwing getter was silently read as absent instead of propagating,
  which for an iterator that never sets `done` looped until the 100,000-step
  guard instead of throwing immediately; fixed to go through `get_prop`
  (§7.4.6/§7.4.7 IteratorComplete/IteratorValue).

On fixed base 69ea3ea7, Linux workspace: 2,825 tests passed (10 skipped) in
51.961 seconds, plus doctests all `ok` (including `amazing_js`, `amazing_html`).
`amazing-js` crate: 363 passed (1 skipped). Full pinned Test262 sweep:
33,103 → 35,606 passing files (+2,503), 64,052 → 69,011 passing variants
(+4,959). Repeated with `--compare` against the same after-run: identical
classification hash, zero flakiness.

126 variants (63 distinct files) lost relative to the raw before/after diff —
fully diagnosed, not a regression from this ticket's own code. Every one
requires a real generator or async-generator object (`features` tagged
`generators` or `async-iteration`: `function*(){...}()` as a custom iterable,
or an async generator method), which this interpreter does not implement at
all (`ExprKind::Yield` already throws "generators are not implemented", a
known, tracked gap — see ARCHITECTURE.md's locked single-threaded/tree-walking
decisions). These files previously passed only because two of this ticket's
own bug fixes had been masking that gap: the static-method-placement bug made
`C.method(...)` throw "not a function" (accidentally matching a test's
expected `TypeError` for the wrong reason), and the destructuring
error-swallow bug silently no-op'd a failed generator-backed destructure
(accidentally matching a test that expected quiet completion). Fixing both
bugs correctly is what exposed the real, pre-existing "no generators" gap as
the genuine failure it always was. Evidence, including the full lost-variant
list with each one's `features` tag and observed error, is retained at
`docs/lanes/js-runtime_class_private_elements_evidence.json`.

Not implemented (at the time): real generators/async-generators (separate,
large, pre-existing gap — see ADR-locked single-threaded tree-walking
decisions; a cross-lane request is filed against js-vm, which already has
resumable VM generator frames from JSVM-5), Proxy support, and the `accessor`
keyword proposal for class fields (parsed but not given distinct semantics
from an ordinary field).

## JSRT-15 Proxy support (2026-09-26)

Implemented `Proxy` (revocable and non-revocable) end to end: a new
`Exotic::Proxy(Rc<ProxyState>)` variant, all 13 fundamental traps (`get`,
`set`, `has`, `deleteProperty`, `ownKeys`, `getOwnPropertyDescriptor`,
`defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
`preventExtensions`, `apply`, `construct`), and the `Proxy`/`Proxy.revocable`
globals, in a new `crates/amazing-js/src/interp_proxy.rs`. Follows the
`interp_window_proxy.rs` hook pattern exactly: every fundamental object
operation gets a `proxy_*` check (`Option<Result<T, Throw>>`, `None` = not a
proxy, proceed ordinarily) alongside its existing `window_proxy_*` check, at
every site that already had one, plus the two new [[Call]]/[[Construct]]
interception points in `call_value_with_new_target_inner` and
`construct_with_new_target` (added before either function touches
`.func`/`.constructible`, since a Proxy's own storage never carries real
callable state — a dummy `FunctionBody::Native` is set only so
`typeof`/IsCallable/IsConstructor checks that read `.func.is_some()` /
`.constructible` directly still see a callable-target proxy correctly;
invocation itself is always intercepted before that body could run).
`Reflect.*`'s duplicated ad hoc WindowProxy-only logic
(`isExtensible`/`preventExtensions`/`getPrototypeOf`/`setPrototypeOf`/`has`/
`deleteProperty`) was collapsed to call the new shared `default_get_prototype`
/ `default_set_prototype` / `default_is_extensible` / `default_prevent_extensions`
/ `default_has_property` / `default_delete_property` helpers, which are
themselves proxy-aware and used identically by `Object.*`; this also gave
`Object.freeze`/`seal`/`isFrozen`/`isSealed` real (if not fully spec-exhaustive)
Proxy support essentially for free, by routing their per-key mutation through
`define_own_property` instead of raw storage writes.

Two more pre-existing bugs were found and fixed while gating (both raw
`.proto`/`.has_property()` walks that predate this ticket and bypass every
exotic-object hook, not just Proxy's):
- `ordinary_has_instance` read `b.borrow().get_own("prototype")` (raw own-storage)
  instead of `Get(b, "prototype")`, so `x instanceof someProxy` on a
  Proxy-wrapped constructor could never observe a `get` trap on `"prototype"`.
  Fixed to call `self.get_prop(b, "prototype")`.
- Unqualified global identifier resolution (`identifier_value` and
  `resolve_reference`'s `ExprKind::Ident` arm) checked
  `self.global_obj.borrow().has_property(name)` — `JsObject::has_property`'s
  own prototype-chain walk, which has no exotic-object awareness at all.
  Fixed both call sites to `self.default_has_property(...)`. That in turn
  exposed a second layer of the same bug one level down: `legacy_has_property`
  (`interp_legacy.rs`)'s own chain-walking loop checked `window_proxy_has` at
  each step but never `proxy_has`, so a Proxy discovered while walking a
  prototype chain (e.g. `Object.setPrototypeOf(globalThis, someProxy)`) was
  silently skipped. Fixed by adding the missing `proxy_has` check beside the
  existing `window_proxy_has` one in that loop.

Known, accepted gaps (documented rather than chased, per the same "diagnose
transparently, don't chase every corner" standard as JSRT-14's generator
gap): invariant enforcement covers the highest-value checks per trap
(revocation, non-callable traps, non-configurable/non-extensible-target
agreement) but not the complete ECMA-262 invariant list — e.g. the `ownKeys`
"extensible target must report every non-configurable target key" superset
rule is unenforced. A Proxy inserted mid-prototype-chain is honored at the
well-known entry points fixed above (`instanceof`, unqualified identifiers,
`Object`/`Reflect` getPrototypeOf/setPrototypeOf, direct property access) but
a handful of other raw `.proto` walks elsewhere (e.g. `set_with_receiver`'s
receiver-shadowing walk was upgraded; `ordinary_has_instance`'s own chain
walk past the first hop was not) still do not call through it.

Gated on Linux (fixed base `76af29d4`): `amazing-js` crate 384 passed (1
skipped) + 1 doctest; workspace `cargo nextest run --workspace --locked` 3122
passed (14 skipped) in 53.929s; `cargo test --workspace --locked --doc` all
`ok`. Full pinned Test262 sweep: 36,335 → 36,821 passing files (+486),
70,468 → 71,427 passing variants (+959), **zero lost files, zero lost
variants** (verified against the pre-Proxy baseline at the same base commit,
then re-verified after both `legacy_has_property` fixes eliminated the one
transient regression found mid-gate — see
`docs/lanes/js-runtime_proxy_evidence.json`).

Not implemented: real generators/async-generators (cross-lane request filed
to js-vm), the `ownKeys`/invariant gaps above, and full defineProperty-trap
routing for `Object.freeze`/`seal` (extensibility and per-key attribute
changes are trap-routed; the TypedArray/array-length special cases are not
re-verified against a Proxy target).

## TypedArray regression fix (2026-09-27, blocking origin/dev push)

The JSB-6/7/8 merge landed on `dev` alongside JSRT-15 and regressed 11
Test262 files in `crates/amazing-js/src/builtins_binary.rs`, all pre-existing
bugs in that lane's new TypedArray code, unmasked rather than caused by
Proxy. Fixed in `builtins_binary.rs`:

- `binary_typed_iterator` (backs `%TypedArray%.prototype.{entries,keys,values}`)
  set `[[Prototype]]` to `Object.prototype` instead of the shared
  `%ArrayIteratorPrototype%` that `CreateArrayIterator` uses for both Array
  and TypedArray iterators — now reuses
  `std_sequence_iterator_proto(SequenceIteratorKind::ArrayValues)`, the same
  object `Array.prototype.values/keys/entries` share.
- `binary_construct_typed_object` (shared by `%TypedArray%.from` and `.of`)
  never checked the constructed instance was at least as long as requested;
  `.of`'s call site had its own redundant check, `.from`'s did not and
  silently dropped tail elements instead of throwing. Moved the check into
  the shared helper so every caller gets it once.
- `includes`/`indexOf`/`lastIndexOf` re-read `values`/`length` from the view
  *after* coercing `fromIndex`, so a `fromIndex` `valueOf` that grows a
  resizable `ArrayBuffer` let the search see the newly grown region. The
  spec reads `TypedArrayLength` once, before `fromIndex` coercion, and
  searches that fixed window regardless of a later resize; now reuses the
  `values`/`length` already captured at the top of `binary_typed_method`.
- `with()` resolved a negative index against the *current*
  (post-value-coercion) length instead of the length captured before any
  coercion, and used `@@species` for the result contrary to the current
  spec text (which always constructs the TypedArray's own intrinsic kind,
  ignoring species).

Verified via a full unsharded Test262 run (`linux-run.sh js-runtime --ref
9767ca91`, not `test262-boat.sh` — see `questions.md` for a stale-shard bug
found in that script) diffed against `.agents/test262/baseline.json`: the
`built-ins/TypedArray*` tree loses only one pre-existing timeout
(`staging/sm/TypedArray/sort_modifications.js`), all 11 originally-reported
files pass. Full details in
`docs/lanes/js-runtime_typedarray_regression_evidence.json`. Workspace
nextest 3285 passed/15 skipped, doctests all `ok`.

## JSRT-16 exotic-object and reference correctness (2026-09-27)

Combined ticket, based on dev `9767ca91` (the TypedArray fix merge above):

- `put_reference` (`interp.rs`): strict-mode assignment to a non-writable
  var-like global binding (`undefined`, `NaN`, `Infinity`) now throws instead
  of silently succeeding. Root cause: `Env::declare_var` always passes
  `mutable: true`, and `Env::assign_raw` syncs the write to the global
  object's mirrored property only when it's writable but reports success
  regardless. Fix routes the write through `set_with_receiver` (the object's
  own correct per-property `[[Set]]`) whenever `Env::global_binding_object`
  says the name is a var-like global binding, applying PutValue's
  strict/sloppy handling of an unsuccessful `[[Set]]`. Target:
  `built-ins/global/10.2.1.1.3-4-18-s.js`.
- `legacy_own_property`/`legacy_has_property` (`interp_legacy.rs`): an
  array-index-shaped key is never treated as a named property, in range or
  not (previously an out-of-range index fell through to the named-getter
  check). An out-of-range index now falls straight through to ordinary
  own-property storage. Target: `dom/collections/
  HTMLCollection-supported-property-indices.html` (5 subtests, WPT-only, no
  Test262 coverage for legacy DOM platform objects).
- `legacy_set`/`legacy_define` (`interp_legacy.rs`): gated purely on whether
  the interface has an indexed setter at all — matching WebIDL's actual
  `[[Set]]`/`[[DefineOwnProperty]]` algorithms — not on
  `supported_index_count`. An initial attempt gated on the supported range
  instead and broke the pre-existing
  `optional_setters_and_deleter_receive_native_keys` unit test, whose
  `indexed_setter` legitimately grows the collection when `index ==
  values.len()`; that test is exactly the evidence the range-gate was wrong.
  No setter now falls through to an ordinary `[[Set]]`/`[[DefineOwnProperty]]`
  (`Ok(false)`) instead of unconditionally throwing.
- `legacy_delete` (`interp_legacy.rs`): a supported index now correctly
  reports non-configurable delete failure (`Some(false)`); an unsupported
  index falls through to an ordinary `[[Delete]]` of any expando actually
  stored there (`None`) instead of claiming vacuous success without ever
  deleting anything.
- `native_array_join` (`interp_builtins.rs`): cycle protection for
  `Array.prototype.join`/`toString` via a thread-local identity stack
  (`ARRAY_JOIN_STACK`, keyed by `std_weak_id`) and an RAII guard
  (`ArrayJoinGuard`) that pops on every exit path including early `?`
  returns; a self-referential array's re-entrant `join` call now contributes
  an empty string instead of overflowing the real Rust call stack.
- `ordinary_has_instance` (`interp.rs`), `set_prop` (`interp_property_access.rs`)
  and `binding_set_property` (`interp_bindings.rs`): their multi-hop
  prototype-chain walks now observe a Proxy/WindowProxy's `getPrototypeOf`
  trap past the first hop, closing the specific gaps JSRT-15's own TODO
  notes flagged as not yet upgraded.

Gated on Linux: `amazing-js` crate 391 passed (1 skipped, two pre-existing
unit tests updated to match the corrected, spec-accurate behavior); workspace
`cargo nextest run --workspace --locked` 3285 passed (15 skipped) in
129.745s; `cargo test --workspace --locked --doc` all `ok` (`amazing_js` and
`amazing_html` included). Full unsharded Test262 on both `9767ca91` (parent)
and `04b0f373` (this ticket): **zero lost, zero gained** — expected, since
JSRT-16's scope (legacy DOM platform objects, global-write strictness, Proxy
prototype hops, `Array.join` cycles) has no Test262 coverage. A secondary
diff against the older, independently-flagged-stale
`.agents/test262/baseline.json` shows 162 apparent variant losses, all
pre-existing drift unrelated to this diff (destructuring rest patterns,
async-arrow-function, intl402, async-gen-meth, and a handful of pre-existing
timeouts) — confirmed absent from the parent-vs-implementation diff. Full
details in `docs/lanes/js-runtime_jsrt16_evidence.json`.

Not implemented/out of scope: an exhaustive sweep of every remaining raw
`.proto.clone()` occurrence in the crate (several single-hop reads and
internal proxy-trap-implementation reads are correct as-is;
`named_visible`'s own chain walk was assessed lower-priority and left
unfixed). Generators/async-generators remain blocked on js-vm (cross-lane
request standing in `questions.md`).

## JSRT-25 native constructibility (2026-10-03)

At base `2eb1229a`, `native_arity` inherited `JsObject::ordinary`'s
constructible flag and manufactured a prototype for every builtin callback.
Native functions now have only [[Call]] by default; only the audited spec
constructors opt into [[Construct]] with `native_constructor(_arity)`.
The raw `JsObject::function` path also defaults `FunctionBody::Native` to
nonconstructible, covering internal async callbacks. Host-native call policies
are preserved. Ordinary native methods and global functions have no fabricated
own prototype, and bound functions retain their target's constructibility.

The constructor allowlist includes core constructors, Error subtypes, the
three dynamic function kinds, binary constructors, collections, Iterator,
DisposableStack/AsyncDisposableStack, Date/RegExp, Proxy, Intl and Temporal.
Symbol and BigInt explicitly retain [[Construct]] for use as newTarget/extends,
while their constructor bodies reject direct construction. The old survey
incorrectly classified them as nonconstructors; the pinned BigInt
`is-a-constructor.js` test exposed this distinction. No upstream code was copied.

Linux Boat validation: workspace nextest **3516 passed, 17 skipped** (76.702s);
workspace doctests **4 passed**. Crate iteration **470 passed, 1 skipped**,
with **1 doctest passed**. Full pinned Test262: baseline **49,679** passing
files to **49,695 confirmed passes**, **16 gained, zero lost**. The 10s/jobs4
sweep reported 49,687 passes; all eight pass-to-timeout flips passed together
at 60s/jobs4 on the same binary and sandbox. Evidence:
`js-runtime_jsrt25_evidence.json`.

Spec references: [builtin function objects](https://tc39.es/ecma262/multipage/ordinary-and-exotic-objects-behaviours.html#sec-built-in-function-objects),
[Symbol](https://tc39.es/ecma262/multipage/fundamental-objects.html#sec-symbol-constructor),
[BigInt](https://tc39.es/ecma262/multipage/numbers-and-dates.html#sec-bigint-constructor).

## JSRT-26 try/catch/finally resource disposal (2026-10-03)

Based on committed JSRT-25 `d9029d63`. js-vm's parity probe showed that the
AST evaluator returned `undefined:undefined` instead of `dispose:body` when
a try-body resource disposal and the body both threw. Try/catch/finally
called `exec_stmts` directly, skipping the disposal boundary already used by
ordinary blocks. The shared `exec_stmts_with_disposal` now evaluates every
lexical body with its own resource frame, disposing before entering the
following catch/finally clause. Catch binding/hoist errors also become the
completion processed by finally rather than returning early.

Nine grouped TreeWalk regressions cover the original suppressed error,
try/catch/finally ordering, LIFO cleanup, nested boundaries, disposal throws
and abrupt catch binding. Bulk before run reproduced the original symptom
and all eight initial disposal scenarios. The final Linux Boat workspace
passed **3517 nextest tests, 17 skipped**, plus **4 doctests**. Pinned Test262
using **76/80 → 77/80**, await-using **93/98 → 94/98**, and try **198/206 →
198/206**: **384 cases, 367 → 369 passes, 2 gained, zero lost**. Evidence:
`js-runtime_jsrt26_evidence.json`.

This factors the existing attributed Lumen disposal implementation; no new
upstream code was copied. A single lexical-body entry point prevents the
ordinary-block and try/catch/finally paths from drifting again.

## JSRT-27 Promise construction capabilities (2026-10-03)

Based on frozen JSRT-26 `d8c33bd2`. Static resolve/reject and all four
combinators now construct their receiver with a real NewPromiseCapability
executor rather than substituting an internal promise. The executor captures
and validates both callbacks and rejects duplicate initialization. Reactions
dispatch through those callbacks, so custom constructor results retain their
identity and subclass branding. `then` selects the species constructor; the
intrinsic default is stored and restored with its realm. `catch` and combinator
attachment invoke the observable `then` property.

Native Promise construction requires new, preserves the allocated prototype,
and shares an already-resolved gate between resolving callbacks. Resolving
with a pending promise cannot be overridden by a later executor rejection.
Promise.resolve compares the input's constructor before returning it unchanged.
The capability/PromiseResolve algorithms are ported from Boa's
`core/engine/src/builtins/promise/mod.rs` at
`39cd11214ecb366200157dab96b3919e61e9a391` (MIT, copyright Jason Williams),
adapted to native callback home objects and the existing interpreter construction
API. No dependencies or value-model fields were added. The upstream notice is
appended in THIRD_PARTY_NOTICES.md.

Grouped regressions cover custom callbacks, subclass identity, species,
intrinsic default preservation, executor validation, pending adoption and
generic then invocation on both interpreter backends. Two prior tests used
dummy constructors whose returned ordinary objects were incorrectly treated
as promises; they now use actual Promise subclasses while retaining their
resolve-observation/rejection assertions. Whole Promise corpus iteration:
**357/732 → 465/732, 108 gained, zero lost**, crate **472 passed/1 skipped**
and **1 doctest**. Final evidence: `js-runtime_jsrt27_evidence.json`.

Final Boat workspace: **3518 passed, 17 skipped, 85.286 seconds**, **4
doctests**. Promise repeated the same **465/732** classification; await
expressions remained **21/22**. Combined **754 selected cases**, **378 → 486
passes**, **108 gained**, **zero lost**, no corpus timeouts. The pinned revision
and full input manifest matched the frozen JSRT-25 full-suite baseline; JSRT-26
only changed disposal classifications outside these selections.

Next clusters: generic thenable assimilation/jobs, finally callbacks,
streaming iterator semantics and element already-called guards in combinators.
Promise.try/withResolvers and keyed combinators remain unimplemented.

## JSRT-28 Promise thenable resolution (2026-10-03)

Based on frozen JSRT-27 `cd3863fd`. Native resolving callbacks now implement
the complete observable resolution sequence: reject self-resolution with a
TypeError object; read an object's `then` immediately; reject getter errors;
fulfill non-callable then properties; otherwise enqueue a thenable job and
invoke it with the original receiver. Each job creates fresh resolving
functions sharing a private already-resolved gate. Nested thenables can thus
resolve independently, while rejection or a thrown exception after the first
resolution cannot override a still-pending adopted promise. Even an internal
Promise's observable `then` participates in resolution.

Ported Boa CreateResolvingFunctions/Promise Resolve Functions/
NewPromiseResolveThenableJob from
`core/engine/src/builtins/promise/mod.rs` at MIT commit
`39cd11214ecb366200157dab96b3919e61e9a391`, using existing interpreter-owned
Microtask jobs and native callback home objects. No Job enum or value-model
fields changed. A new THIRD_PARTY_NOTICES.md row identifies the new source.

Ten grouped resolution scenarios run on both backends. A separate external
scheduler proof checks synchronous getter observation, deferred then invocation
and a separately scheduled reaction. Crate **474 passed/1 skipped**, **1
doctest**. Final Boat workspace **3520 passed/17 skipped, 68.624 seconds**,
**4 doctests**. Pinned Promise **465/732 → 520/732**, await expressions
**21/22 unchanged**, async-function statements **71/74 unchanged**. Combined
**828 cases, 557 → 612 passes, 55 gained, zero lost**, no timeouts. Promise
classification and binary hash repeat identically between iteration and final.
Evidence: `js-runtime_jsrt28_evidence.json`.

VM async consumers retain their earlier callback construction/fast path; the
VM lane has the helper signature `promise_resolving_functions(&Interpreter,
&Obj) -> (JsValue, JsValue)` and owns migrating those consumers. Runtime next:
separate function parameter-expression and body variable environments, in
coordination with JSVM-12 eager generator initialization.

## JSRT-29 Parameter-expression/body variable environments (2026-10-03)

Frozen base `284dce79`. Functions with parameter expressions now create a
separate body variable environment after binding parameters. Default closures
retain the parameter environment; body var declarations that share formal
names start with copied values. Computed destructuring keys, nested defaults,
rest bindings, concise arrows, ordinary/async functions and generators use the
same helper. The new Var environment owns direct-eval declarations and inherits
this/super from the call frame. Annex B block functions exclude formal names
before reserving placeholders. Switch declarations participate in collection.
The VM consumer is one additive call, coordinated with JSVM-12 eager generator
initialization and its independent removal of parameter-eval compilation guards.

Ported Lumen ContainsExpression/body-environment algorithms from
`crates/lumen/src/interpreter.rs` at MIT
`f6c9860d44fa52249cfd647122edea184d32c863`; the source header and a new
THIRD_PARTY_NOTICES row retain attribution. Normative reference:
[FunctionDeclarationInstantiation](https://tc39.es/ecma262/multipage/ordinary-and-exotic-objects-behaviours.html#sec-functiondeclarationinstantiation).

Crate **477 passed/1 skipped**, **1 doctest**. Final Boat workspace
**3523 passed/17 skipped, 67.585 seconds**, **4 doctests**. Pinned functions:
AST **2432 → 2437/2615**, VM mode **2319 → 2324/2615**; generators:
AST **565 → 569/586**, VM **564 → 568/586**; async generators unchanged
AST **1046/1054**, VM **1045/1054**. Unique union **4232 cases**, AST
**4024 → 4033**, VM **3909 → 3918**, **9 gained and zero lost in each mode**.
Grouped regressions cover closures, copied bindings, arguments, derived this,
eval and Annex B. Direct parameter eval uses AST on this fixed base; the VM
compiler acceptance change belongs to JSVM-12. Evidence:
`js-runtime_jsrt29_evidence.json`.

Initial workspace linking failed on a shared HOME ICU cache. The coordinator
fixed MAIN linux-run to use native source/target directories with incremental
disabled; the unchanged-source workspace retry passed without deleting caches.
Next coordinated Promise cluster: internal await reaction attachment should
bypass observable `then` and species, using the native reaction machinery.

### JSRT-29 recovery: parameter lexical environment

The combined `bc9e8388` whole-language gate exposed thirty lost direct-eval
`arguments` cases that the original function-directory selection missed.
The original JSRT-29 implementation separated the body environment but omitted
FunctionDeclarationInstantiation step 20: non-strict functions with parameter
expressions need a parameter lexical environment before default evaluation.
Formal names and the implicit arguments binding remain lexical in that frame;
eval-created vars belong to the callee's outer variable environment. Declaration
validation checks these parameter bindings atomically before hoisting, so
`function f(p=eval('var arguments')){let arguments}` throws during initialization.
The earlier method regression expectation that allowed this declaration was
incorrect and is restored to SyntaxError. Both AST calls and the coordinated
VM consumer prepare the parameter environment before bind_params while retaining
the body split, generator argument suppression and eager generator frame setup.
The new frame inherits source metadata and transparently resolves this/super.

An initial broadened eval lexical check rejected an existing Annex B simple-catch
case. The new check is therefore confined to parameter frames; ordinary block
and catch declaration handling retains its existing behavior. Recovery evidence
uses the fixed combined base and the full language selection, direct eval and
all Function constructor families, plus workspace nextest and doctests.
Final Boat workspace **3638 passed/17 skipped, 74.430 seconds**, **4 doctests**.
Whole language **23574 → 23694/24572**, direct eval **452 → 572/595**:
**120 gained and zero lost**, including all thirty reported regressions.
Function constructor selection **619/653**, unchanged with zero losses.
Both explicit interpreter backends pass the grouped initialization regressions.
Evidence: `js-runtime_jsrt29_recovery_evidence.json`.

## JSRT-30 Internal Promise reactions (2026-10-03)

Frozen base `8d3a4eca`. `perform_promise_then(&mut Interpreter, &Obj,
Option<JsValue>, Option<JsValue>, Option<Obj>) -> Result<(), Throw>` attaches
native fulfillment/rejection reactions directly. Native Promise.prototype.then
passes its actual capability record; internal AST coroutine await passes no
result capability. Pending and settled promises share the helper and existing
scheduler, preserving reaction order without reading observable then/species.
Empty-capability jobs ignore normal handler results instead of assimilating
them into an unnecessary chained promise. An unexpected abrupt host callback
completion propagates through the existing job error path.

Ported Boa PerformPromiseThen and optional reaction-capability completion at
MIT `39cd11214ecb366200157dab96b3919e61e9a391`, with headers and a new notices
row. Normative reference:
[PerformPromiseThen](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-performpromisethen).
Parent granted the exact optional PendingReaction field; the VM owner approved
conditional tracing if needed. No reaction tracing consumer exists on this
fixed base, so no GC files changed. VM await drivers are reserved for its next
consumer ticket. AST tests force a coroutine with a with statement because
eligible async bodies compile independently of the script backend.

Crate **480 passed/1 skipped**, **1 doctest**; final native Boat workspace
**3526 passed/17 skipped, 67.079 seconds**, **4 doctests**. Whole pinned Promise
**520/732**, await expressions **21/22**, async-function selection **328/382**
remain unchanged with zero lost passes. Internal tests cover pending/settled
fulfillment/rejection order, ignored handler return thenables, poisoned await
then getters and species. Evidence: `js-runtime_jsrt30_evidence.json`.

VM-owned async_resolve_value constructor-get abrupt timing remains a separate
follow-up. Promise rejection tracking is pre-existing host work. Runtime next:
port the whole Promise.prototype.finally algorithm (generic Invoke, species,
deferred callbacks and PromiseResolve-based cleanup settlement).

## JSRT-31 Promise.prototype.finally (2026-10-03)

Frozen base `692f9ff8`. Replaced the synchronous/pending-identity shortcut with
the whole generic finally algorithm: obtain species, Invoke the receiver's
then method, create deferred fulfillment/rejection cleanup closures, resolve
cleanup results with the captured constructor, then preserve the original
value or reason after cleanup. Cleanup rejection and thrown callbacks override
the original completion. Non-callable callbacks pass through unchanged.

Ported Boa finally and ThenFinally/CatchFinally closures from the same MIT
`39cd11214ecb366200157dab96b3919e61e9a391` commit into
`interp_promise_finally.rs`; header and new notices row identify the port.
Normative reference:
[Promise.prototype.finally](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-promise.prototype.finally).
The last finally failure exposed VM assignment naming of passed native
callbacks. The shared inference helper now preserves explicit native names,
including empty names, without changing its VM consumer signature.

Before grouped regression: **480 existing crate passes plus one failure**,
cleanup ran before the synchronous marker. After **481 passed/1 skipped**,
**1 doctest**; nine grouped scenarios run on both backends. Final native Boat
workspace **3527 passed/17 skipped, 70.507 seconds**, **4 doctests**. Pinned
Promise **520 → 540/732**, **20 gained, zero lost**; finally **14 → 29/29**.
Additional gains cover native callback names and reactions that await finally
cleanup. Await and async-function selections remain unchanged.
Evidence: `js-runtime_jsrt31_evidence.json`.

The first final gate was green; the native-name correction warranted one
unchanged-base gate repeat. Runtime next: whole allKeyed/allSettledKeyed area,
the largest remaining unimplemented Promise cluster (87 pinned cases).

## JSRT-33 Constant-time tail property deletion (2026-10-03)

Frozen base `7785c295`. Generic `JsObject::delete_own` now pops the final
property and removes its hash index entry when no remaining slot moves.
Interior deletion keeps its previous shift/rebuild behavior. Both successful
paths preserve configurable checks, insertion order and shape-version bumps,
so VM cached slots still invalidate. Descending dense-array truncation and
generic object tail deletion avoid rebuilding every surviving index entry.
No dense array representation, VM default policy or harness recognition was
introduced. A grouped regression covers tail/interior deletion, lookup,
reinsertion order, empty objects and cached-slot invalidation.

Final Boat workspace **3639 passed/17 skipped, 69.741 seconds**, **4 doctests**.
Array selection **3226/3332**, Object **3338/3414**, deletion **414/452**,
fromCodePoint **12/12**, apply **48/48**, all unchanged with zero lost passes.
Full RegExp at the same **5-second timeout/8 jobs**: **1590 → 1694/2063**,
**104 gained, zero lost**; timeouts **447 → 343**, failures unchanged at 26.
Unique union **9305 cases**, **8614 → 8718**, **104 gained, zero lost**.

The unchanged pinned regExpUtils helper builds the full 1,114,112-code-point
range in **90,590 → 4,255 ms** (about **21.3×** faster), producing the same
2,162,688 UTF-16 units; Any/nonASCII scans remain 19/17 ms. The smaller 65,536
range improves **5,014 → 247 ms**. Independent reference diagnostics also show
three 10,000-element truncations **2286 → 93 ms** and generic 10,000-key tail
deletion **751 → 33 ms**; these use the VM lane's different fixed reference
base and are diagnostics, while the full helper comparison uses this ticket's
same frozen base. Remaining 343 deadline cases need further general work.
Evidence: `js-runtime_jsrt33_evidence.json`; runnable diagnostic:
`js-runtime_array_profile.py`. Next priority: dynamic Function and indirect
eval active source-location capture. Keyed Promise WIP remains preserved.

## JSRT-34 Dynamic compilation source locations (2026-10-03)

Frozen base `f120de608169358b37e7cf2686c38ef9e8bdaa10`. Dynamic Function
constructors and indirect eval now capture the active script/module location
while retaining their realm's global binding frame. All four constructor
families share this behavior. Promise reactions capture their registration
location and restore it around the handler, including an explicit absent
location that masks an unrelated checkpoint script. Cross-realm invocation
preserves the calling script location without changing realm bindings.
This follows [HTML HostMakeJobCallback](https://html.spec.whatwg.org/multipage/webappapis.html#hostmakejobcallback).

The implementation reads existing saved callable frame metadata; it changes
no VM code or invocation tokens. Adopting the coherent base supplied the
previously landed resumed-frame context support. Three grouped regressions
exercise both backends, delayed dynamic constructors, resumed generators,
indirect eval, Promise callbacks, foreign realms, module locations and absent
locations. An initial explicit-Bytecode fixture used unsupported `with`;
seeding that AST fallback separately corrected the fixture before the final
gate, without changing production behavior.

Final Boat workspace **3685 passed/17 skipped, 71.567 seconds**, **4 doctests**;
the workspace includes **516 amazing-js tests**. Matched 15-second/4-job
selections: whole language **23803/24572**, Function **619/653**, Promise
**540/732**, all unchanged with **zero lost passes and zero timeouts**.
Unique union **25945 cases**, **24950 passing before and after**, zero losses.
Evidence: `js-runtime_jsrt34_evidence.json`. Next: resume the preserved whole
allKeyed/allSettledKeyed Promise area on this coherent base.

## JSRT-32 Keyed Promise combinators (resumed 2026-10-03)

Frozen base `addab721147a7e79fb2049cece42dafda56c2802`. The preserved whole
`Promise.allKeyed` and `Promise.allSettledKeyed` implementation was transplanted
onto the coherent FDI, VM-await and source-context base. The algorithms are
ported from Boa's Promise implementation at
`39cd11214ecb366200157dab96b3919e61e9a391` (MIT); the source header and appended
`THIRD_PARTY_NOTICES.md` row record the port. No dependencies were introduced.

The methods use actual constructor capabilities and cache the constructor's
resolve method. They snapshot own keys, then read each current descriptor and
enumerable value, preserving symbols and property getter order. Per-element
native closures share an already-called flag; a sentinel counter tolerates
synchronous callbacks while enumeration continues. Results have a null
prototype, and settled entries retain ordinary status/value or status/reason
records. Abrupt operations reject through the capability. A grouped regression
covers both backends, subclasses, duplicate callbacks, removed keys, symbols,
`__proto__`, invalid inputs and empty dictionaries. Normative reference:
[Await Dictionary proposal](https://tc39.es/proposal-await-dictionary/).

Final Boat workspace **3686 passed/17 skipped, 69.348 seconds**, **4 doctests**.
Matched 15-second/4-job Promise substring selection **550 → 637/742**,
**87 gained, zero lost**: allKeyed **44/44**, allSettledKeyed **43/43**.
The selection includes ten passing language Promise cases in addition to the
732 builtin files. Await-expression **9/9** and async-function **354/382**
remain unchanged. Unique union **1133 cases**, **913 → 1000 passing**,
**87 gained, zero lost**, no timeouts. Evidence:
`js-runtime_jsrt32_evidence.json`. Next: contextual direct-eval parser consumer,
coordinated with the VM invocation metadata and decorators lane.

## JSRT-36 WindowProxy receiver seams (2026-10-03)

Frozen base `d7fd1d5e65dc2a6e3a9492728247655bc456fe9f`. HTML52's real-root
Window tests exposed two interpreter bypasses. OrdinaryHasInstance read the
left operand's first prototype directly, observing exotic hooks only on later
links. It now uses the existing WindowProxy/Proxy prototype operations at that
first link as well. The native Window location getter received the WindowProxy
but read an internal slot directly from the proxy. It now resolves the current
authorized target and reads that Window's location slot. Existing PutForwards,
location replacement and setter paths are unchanged.

Two grouped regressions cover both execution backends, root EventTarget
inheritance, location identity and forwarding, same-origin realm retargeting,
observable initial Proxy prototype traps and revocation. This changes no
factory policy, VM metadata or value-model shape. Normative reference:
[OrdinaryHasInstance](https://tc39.es/ecma262/multipage/abstract-operations.html#sec-ordinaryhasinstance).
Final Boat workspace **3688 passed/17 skipped, 71.475 seconds**, **4 doctests**.
Matched 15-second/4-job full Proxy **278 → 280/311**, instanceof **67 → 69/70**,
Function **619 → 620/653**. Unique union **1032 cases**, **964 → 967 passing**,
**3 gained, zero lost**, no timeouts. The gained cases exercise initial proxy
prototype traps, non-extensible target invariants and abrupt prototype lookup
through Function.prototype's hasInstance. Evidence:
`js-runtime_jsrt36_evidence.json`. HTML owns driver and WPT validation of this
standalone dependency; contextual eval remains the next coordinated area.

## JSRT-35 Contextual PerformEval (2026-10-03)

Frozen coherent dependency `0d5963fe7abda88ec6ea8100efd6e853f278bf4b` includes
parser25, decorators26 and VM17. PerformEval context flags, lexical scope and
restoration are ported from Boa's eval implementation at
`39cd11214ecb366200157dab96b3919e61e9a391` (MIT), adapted to our EnvHandle,
parser API and native frames; the header and appended notices row record it.
Normative reference: [PerformEval](https://tc39.es/ecma262/multipage/global-object.html#sec-performeval).

Direct eval receives syntactic new.target, super property/call, initializer and
visible-private-name permissions from its lexical caller. Arrows retain scalar
permissions independently of actual new.target values. Indirect eval resets
these permissions and masks ambient method metadata when creating arrows;
ordinary functions invoked inside eval establish their own contexts. Eval
uses a fresh lexical scope with TDZ cells for let/const/destructuring/class
declarations; sloppy vars retain the caller's variable scope, and strict vars
stay local in a transparent variable frame. Captured private identities and
source locations retain the existing environment chain.

The native eval CallFrame is popped and restored whole, preserving VM17's
context_id while super operations and caller introspection see the lexical
caller. VM changes are confined to the agreed arrow metadata capture block;
factory policy and invocation tokens are unchanged. Four grouped regressions
cover both execution modes, including deliberately seeded AST callable
fallbacks, indirect eval isolation, initializer arrows, constructors, private
names, super, TDZ, closures, caller restoration and Annex B promotion.

The first bulk owned gate passed **542 tests/1 skipped and 1 doctest**, but the
wide comparison found **80 gains/195 losses**, all losses in Annex B eval.
Fresh lexical scopes exposed raw variable hoists targeting the lexical frame;
removing native eval also exposed caller formals to an FDI-only exclusion.
Promotion now creates eval variable bindings in the variable scope, including
configurable global properties; eval's Script body has no formal parameters.
The final run explicitly restores **all 195** former passes.

Final Boat workspace **3755 passed/17 skipped, 75.423 seconds**, **4 doctests**,
including **542 amazing-js tests**. Matched 15-second/4-job language substring
selection **23882 → 23977/24590**, **95 gained, zero lost**; Function
**620/653** and Promise **627/732** unchanged. Unique union **25963 cases**,
**25117 → 25212 passing**, **95 gained, zero lost**, no timeouts. Gains cover
Annex B eval16, direct eval8, indirect eval6, class expressions24, super4 and
class statements37. Evidence: `js-runtime_jsrt35_evidence.json`.

## JSRT-38 WindowProxy caller-realm descriptors (2026-10-03)

Frozen base `bf59a0cddb0443154fc53b8b8af7b0e8bbf6040d`. HTML52 exposed
that permitting a safe cross-origin property through the access hook forwarded
the target's original descriptor, including its Replaceable setter. The new
additive `WindowProxyPropertyHook` selects `Target` or
`Descriptor(Option<Property>)` after authorization in the caller realm.
`Descriptor(None)` explicitly masks a property. Get invokes a synthesized
getter with the original receiver; GetOwnProperty returns the same host-cached
descriptor. Navigation during either hook restarts authorization and synthesis
against the replacement target. The existing access hook and setter paths
retain their interfaces. Internal target resolution does not invoke the
synthesis hook, avoiding discarded descriptors during native getter operations.

Two grouped regressions cover both backends, caller realm and receiver,
stable getter identity, absent setters, hidden target getters/properties,
navigation revalidation and denial before synthesis. HTML owns the cached
cross-origin safe property descriptors and the real installed driver/WPT gate.
Reference: [CrossOriginGetOwnPropertyHelper](https://html.spec.whatwg.org/multipage/nav-history-apis.html#crossorigingetownpropertyhelper).
Final measurements are recorded in `js-runtime_jsrt38_evidence.json`.

Final Boat workspace **3757 passed/17 skipped, 75.759 seconds**, **4 doctests**.
Matched 15-second/4-job whole selections: Proxy **280/311**, Object
**3338/3414**, Reflect **151/154**, Function **502/515**, all unchanged.
Unique union **4394 cases**, **4271 passing before and after**, zero losses
and zero timeouts. HTML owns the installed cross-origin descriptor gate.

## JSRT-37 Mapped and unmapped Arguments objects (2026-10-03)

Frozen base `a157bdb407e4899a396e8511a14d7c4bf44671c5`. Arguments creation
and exotic algorithms are ported from Boa's
`core/engine/src/builtins/function/arguments.rs` at
`39cd11214ecb366200157dab96b3919e61e9a391` (MIT), adapted to shared
`BindingCell` ownership. The mapped object's parameter map retains individual
cells, not a whole environment. Escaping arguments preserves live aliases;
successful deletion, accessor conversion and non-writable definitions release
only the corresponding alias. The current engine uses strong Rc ownership and
has no tracing collector; this ticket adds no collector or GC contract.

Only sloppy simple parameter lists map indices, and the rightmost duplicate
formal claims its name even when no corresponding argument was supplied.
Strict and non-simple lists create ordinary objects carrying the Arguments
brand. Both variants retain the realm's saved original Array iterator.
Unmapped callee uses a saved per-realm ThrowTypeError intrinsic, independent of
later mutation of Function.prototype's configurable restricted properties.

Mapped Get and own descriptors read the live formal cell. Set writes the map
before ordinary receiver semantics only when the receiver is the same object;
another receiver shadows without writing the original map. Ordinary descriptor
validation completes before mapped Define updates or detaches the alias.
Freezing snapshots the live value before detachment; sealing preserves writable
aliases. Successful low-level deletion also removes its map entry, covering
both captured-reference and existing raw deletion consumers. Mapped exotics
already bypass VM ordinary-object property caches; no VM metadata changes.

Five grouped regressions cover both entry backends plus an AST-fallback
callable, escaped aliases, duplicate and missing formals, reflective receiver
semantics, Proxy forwarding, seal/freeze, accessor and non-writable detachment,
abrupt descriptor conversion, brand and intrinsic mutation. The final gate
covers workspace/doctests and whole language plus Arguments, Function, Object,
Reflect and Proxy corpora. Measurements follow after the immutable run.

Final Boat workspace **3762 passed/17 skipped, 74.979 seconds**, **4 doctests**;
**549 amazing-js tests** include five new grouped regressions. Matched whole
language **23977 → 24029/24590** (+52), including arguments-object
**240 → 263/263** (+23). Object **3338 → 3347/3414** (+9); Function
**502/515**, Reflect **151/154**, Proxy **280/311** unchanged. Unique union
**28984 cases**, **28248 → 28309 passing** (+61), **82 gained variants**,
zero lost files or variants, zero timeouts. The empty built-ins/Arguments
filter is explicitly recorded; actual arguments-object coverage is derived
from the complete language records. Initial iteration's only failure was
the explicit Bytecode unsupported-fallback declaration fixture, corrected
by AST seeding before both-backend calls; all production/native cases pass
in the final repeat. Evidence: `js-runtime_jsrt37_evidence.json`.
