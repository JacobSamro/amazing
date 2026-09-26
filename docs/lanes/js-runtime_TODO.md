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
