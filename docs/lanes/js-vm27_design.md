# JSVM27: suspended ClassDefinitionEvaluation proposal

Status: API review only. No engine changes. Fixed predecessor is ready
JSVM26 `2857df0a`. Ordinary nonsuspending classes retain the whole AST
adapter; the eight class-initialization compile fallbacks remain atomic
until the complete area is implemented and gated.

## Sources and current seams

Checked current [ECMA-262 ClassDefinitionEvaluation](https://tc39.es/ecma262/multipage/ecmascript-language-functions-and-classes.html#sec-runtime-semantics-classdefinitionevaluation)
and existing adapted `interp_class_eval.rs`, `interp_decorators.rs`, and
`interp_class_private.rs`. The permissive reference is
[lucid-softworks/lumen eval.rs](https://github.com/lucid-softworks/lumen/blob/f6c9860d44fa52249cfd647122edea184d32c863/crates/lumen/src/eval.rs)
at `f6c9860d44fa52249cfd647122edea184d32c863`.
Its [license](https://github.com/lucid-softworks/lumen/blob/f6c9860d44fa52249cfd647122edea184d32c863/LICENSE)
is MIT, Copyright (c) 2026 Lucid Softworks limited, checked at that commit.
Existing class/decorator algorithms already carry this attribution. Any
new adapted file receives its own header and a new notice row.

The class name is uninitialized during heritage. Heritage uses outer private
names; member evaluation uses fresh class private identities. Computed names
run before static initialization. Private methods are installed before fields.
Abrupt completion restores the outer environments. These constraints rule out
pre-evaluating keys in the outer scope and replaying a substituted AST.

The current compiler's `EnterStrict` uses Interpreter `vm_strict_stack`,
whereas frame resume restores only `VmFrame.strict_context`. A suspended
class expression therefore needs frame-owned dynamic strictness, including
abrupt unwinding, rather than extra EnterStrict/ExitStrict pairs.

## Shared opaque evaluation record

Proposed owned file: `interp_class_evaluation_state.rs`, declared by one module
line. Keep the record parent-private, visible to child `vm`; it is not a
JsValue variant, exotic object, public API or callback home.

```rust
struct ClassEvaluation {
    class: Rc<ast::ClassItem>,
    outer_env: EnvHandle,
    class_env: Option<EnvHandle>,
    inferred_name: Option<JsString>,
    decorators: Vec<DecoratorDefinition>,
    member_decorators: Vec<DecoratorDefinition>,
    original: Option<JsValue>,
    prototype: Option<Obj>,
    class_name: JsString,
    definitions: Vec<ClassDefinitionElement>,
    phase: ClassEvaluationPhase,
}
enum ClassEvaluationPhase {
    OuterDecorators,
    Heritage,
    Elements { next: usize },
    Finish,
}
```

The record keeps existing DecoratorDefinition value/receiver pairs; values
alone would lose decorator method receivers. The compiler reuses reference
pair lowering from JSVM26 for decorator expression evaluation. Calls and comma
expressions clear their receiver; grouped property expressions retain it.

Exact proposed parent-private methods:

```rust
fn begin_class_evaluation(
    &self, class: Rc<ast::ClassItem>, outer: &EnvHandle,
    inferred_name: Option<JsString>,
) -> Result<ClassEvaluation, Throw>;
fn append_class_decorator(
    &self, state: &mut ClassEvaluation, member: Option<usize>,
    value: JsValue, receiver: JsValue,
) -> Result<(), Throw>;
fn enter_class_heritage(
    &self, state: &mut ClassEvaluation,
) -> Result<EnvHandle, Throw>;
fn prepare_class_elements(
    &mut self, state: &mut ClassEvaluation, superclass: Option<JsValue>,
) -> Result<EnvHandle, Throw>;
fn append_class_element(
    &mut self, state: &mut ClassEvaluation, member: usize,
    computed_key: Option<PropertyKey>,
) -> Result<(), Throw>;
fn finish_class_evaluation(
    &mut self, state: ClassEvaluation,
) -> Result<JsValue, Throw>;
```

Begin retains the outer environment without exposing an inner class name to
class decorator expressions. Enter-heritage creates the immutable inner name
and returns the real class environment. Its private map is still inherited.
Prepare-elements runs superclass constructor/prototype validation with the
outer lexical/private environment active, then registers the fresh class
private map, builds the existing constructor/prototype and returns class_env.
Each append consumes previously evaluated member decorators and the canonical
computed key, or resolves a static/private key from the original AST. It does
not evaluate expressions. Finish reuses the existing decoration, private
installation, initializer metadata, replacement/name binding and static
initialization algorithm, including completion preservation in field scopes.

`interp_class_eval.rs` becomes the ordinary AST driver of these same methods;
`interp_stmt.rs::eval_class_named` keeps its existing strict wrapper. This
avoids independent VM class semantics. Initializers and static blocks remain
their own existing execution scopes, rather than being lowered into the
enclosing resumable function.

## VM phases and roots

Proposed instructions (slot allocation is Program metadata):

```rust
ClassBegin(slot, Rc<ast::ClassItem>, Option<KeyOperand>)
ClassDecorator(slot, Option<member_index>, value_reg, receiver_reg)
ClassEnterHeritage(slot)
ClassPrepareElements(slot, Option<superclass_reg>)
ClassAppendElement(slot, member_index, Option<KeyOperand>)
ClassFinish(result_reg, slot)
```

ClassBegin opens a restoration boundary before class decorators, retaining the
same outer EnvHandle and switching dynamic strictness to true. EnterHeritage
changes only the current environment; it does not create another scope-stack
entry. ClassPrepareElements temporarily restores the outer environment for
observable superclass prototype access, then activates the class environment.
Member decorator expressions and computed keys compile in source order, with
ToPropertyKey once before append. Static blocks append their original nodes.
ClassFinish completes the shared algorithm and closes the restoration boundary.

VmFrame adds `class_evaluations: Vec<Option<VmClassEvaluation>>`, whose wrapper
contains the opaque state and its boundary depth. A helper takes the record
out of its slot before invoking JavaScript, releases all RefCell borrows and
returns it afterward. Nested/reentrant classes use different frame/slot records.
Suspension occurs only in emitted expression instructions, never inside a
record helper. Registers and records remain in the frame across Await/Yield
and debugger quantum stops.

Current Obj is `Rc<RefCell<JsObject>>`; JsValue, EnvHandle, PrivateName and
ClassElements retain strong ownership. The record's values, constructor,
prototype, decorator callbacks and environment are therefore roots without a
new tracing API. No tracing collector exists in this crate. This preserves
existing strong-Rc lifetime semantics and does not claim cycle reclamation.
Records drop on finish or when their boundary is unwound. Escaped closures
retain their actual class EnvHandle; they are never rebound to a copied scope.
Each class evaluation mints private identities once, shared by its methods,
field templates and decorator access functions.

## Exact context/unwind inventory

Replace frame `scopes: Vec<EnvHandle>` with snapshots containing outer EnvHandle,
dynamic strictness and per-frame strict-stack depth. Convert existing ordinary
scope/With/catch/copy-scope operations mechanically; their semantic environment
creation stays unchanged. Move EnterStrict/ExitStrict storage from Interpreter
to `VmFrame.strict_stack`; keep `VmFrame.strict_context` as the actual current
strictness, updating it at strict enter/exit and snapshot restoration.

TryHandler saves strictness and strict-stack depth beside its existing env/scope
depth. `leave_scopes_to` and `route_abrupt` restore environment and strictness
together and discard class records whose boundary was exited. Scope snapshots
remain class evaluation metadata, with no new declaration, FDI or Reference API.

| Outcome | Required retained/restored state |
|---|---|
| Await/Yield/quantum | Retain record, actual env/private map and dynamic strictness; restore ambient caller on return from resume; reinstall saved state on next resume |
| Normal ClassFinish | Complete under class strictness, then restore exact outer scope/With and strictness |
| Throw into outer catch/finally | Restore handler env and strict-depth snapshot; drop exited class slots before catch/finally runs |
| Injected generator Return | Unwind class boundary before outer finally/disposal; keep existing Return/Await handling unchanged |
| Break/continue ExitTo | Restore target scope snapshot and discard exited slots; ordinary control-flow lowering stays unchanged |
| Terminal uncaught Throw/Return/fuel error | Drop frame-owned records; caller context restoration remains existing resume wrapper responsibility |

Captured CallFrame invocation IDs, body realms, native async Return context,
With object identity, resource-disposal ordering, Option completion and
per-realm intrinsics are preserved. Class evaluation is not a callable and
does not invent an own-function frame.

## Proof and ownership boundary

Parent review must approve the six shared methods and exact scope/strict unwind
conversion before engine edits. Coordinate Lang's current async-delegation
adapter, which touches only reexports/delegate paths, and Runtime collection
work, which does not touch class records or frame scopes.

Before/candidate must use one coherent fixed source and identical release
Test262 host profiles, pins, capabilities, deadlines and worker/job counts.
Run full workspace/doctests plus whole language/Function/Object/Reflect/Proxy,
four callable-kind, Promise/Iterator/module/decorator selections in both modes.
Native/forced-AST matrices must cover name TDZ/escaped closures, outer-vs-inner
private identities, source-order decorator receivers/computed keys, subclass
prototype errors, private method/field partial installation, static ordering,
property names/metadata, repeated/nested/reentrant class evaluations, foreign
realms, With restoration, synchronous/asynchronous suspension, injected
Throw/Return with catch/finally/disposal, and debugger quantum stops.
No pass losses or new host crashes; existing eight fallback removals must be
measured, not inferred from the opcode inventory.

Node **v24.18.1** on Boat confirms seven reference cases: heritage class-name
TDZ before keys, retained computed-key closures and per-evaluation private
brands, outer private names during heritage, outer sloppy catch/finally after
injected Throw/Return, and strictness after Await in heritage and keys. See
[js-vm27_reference.json](js-vm27_reference.json). These are reference results,
not Amazing before/after improvements.

Review refinement: heritage's last Reference must survive separately from its
value until the outer environment has been restored. A class-private
Value-or-existing-Reference operand can use the current name/capture/key/GetValue
gateways without adding a Reference variant. The same six phase methods remain;
ClassPrepareElements resolves that operand before prototype validation. Do not
substitute eager expression lowering for this ordering.
