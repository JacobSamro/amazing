# JS28 module request and evaluation interface

Fixed dependency: JS27 evidence `5f6191c7d76dff9a32462344339fa44f53975a22` on coherent `0d5963fe7abda88ec6ea8100efd6e853f278bf4b`.

Module algorithms adapted from lucid-softworks/lumen, `crates/lumen/src/modules.rs`, commit `f6c9860d44fa52249cfd647122edea184d32c863`, MIT, Copyright (c) 2026 Lucid Softworks Limited. New request, asynchronous evaluation and namespace files carry their exact source headers and the notices append is in the same series. Engine code stays std-only.

## Host boundary

`ModuleRequest` preserves a dynamic specifier's UTF-16 units and sorted UTF-16 attribute pairs. `ImportPhase` is Evaluation, Source or Defer. Static AST strings still use the parser's existing Rust string representation.

`ModuleResolver::resolve_request` is additive: the default delegates the existing `resolve(specifier, referrer)` for requests without attributes and rejects unsupported nonempty attribute sets. Existing closure resolvers and `source_url` remain source-compatible. A host override returns `LoadedModule { id, source }`. The host supplies a stable canonical ID that includes type policy and remains identical across phases. `ModuleSource::JavaScript` contains source text; `Synthetic` contains a host-parsed default value and an optional genuine source object. The engine does not treat JSON as executable JavaScript or claim that a synthetic API installs JSON/WebAssembly/text/bytes fetch support. Host adoption is separate.

`binding_set_phase_module_loader_hook` is additive; legacy string-error and typed-error setters remain available. Its callback receives the complete request and referrer and may return a Promise. Argument expressions execute synchronously, followed by specifier coercion, options/attribute validation and canonical sorting before dispatch/rejection. Attribute enumeration/getters preserve abrupt error identity. Legacy loaders reject requests they cannot faithfully handle.

## Evaluation boundary

`ModuleLinker::import_shared` loads the requested phase through the resolver. Source phase parses only the root; a source-text record rejects with SyntaxError because it has no module source object. Source phase does not fetch/link/evaluate its transitive requests.

`evaluate_async_shared` returns an evaluation Promise without pumping jobs. Hosts observe it with `binding_observe_module_evaluation`, whose internal reaction receives `Result<(), Throw>` in the registering realm without reading then/constructor/species. `binding_module_evaluation_state` clones the PromiseFate without observable operations. The host can keep its event loop active. Evaluation performs source-ordered DFS/SCC discovery, starts independent siblings when a body parks at top-level await, tracks pending async dependencies and ordered ancestors, and caches evaluation errors. `evaluate_shared` preserves the standalone synchronous convenience checkpoint by draining to a fixpoint. Under an external scheduler it never pumps: settled evaluation returns its result, while pending evaluation reports an Async failure. Callers requiring asynchronous completion use the Promise boundary. The older borrowed `evaluate` remains a compatibility path; browser/module hosts should use shared entry points.

Deferred namespace objects have separate identity from ordinary namespaces, live exporter cells, a `Deferred Module` tag and symbol-like `then`. String Get/Has/GetOwnProperty/Define/Delete and OwnKeys synchronously trigger evaluation, including missing string names. Symbols/then and Set do not trigger. ReadyForSyncExecution traverses the complete requested graph: pending async execution or unexecuted top-level await rejects before any deferred body runs. Deferred graph loading starts the async subgraph eagerly and leaves synchronous-only bodies deferred. Namespace string keys sort by UTF-16 rather than ordinary array-index order.

## Ownership and lifetime

Namespace state strongly owns live BindingCells and an Rc callback that owns the shared module registry and captured realm. Internal continuation functions own their callback state through their hidden home object. Every callback is cloned and all RefCell borrows released before invoking JavaScript. Current engine ownership is strong Rc/RefCell with no tracing collector seam, confirmed by the VM owner. Registry/namespace cycles follow the existing engine ownership model; a future tracing collector will need to visit these callbacks and cells. Ordinary inline caches already exclude all non-Ordinary exotics.

Shared hook grants cover only namespace Get/Has/OwnProperty/Define/Delete/OwnKeys and exhaustive exotic dispatch, plus the additive loader field/setter and internal Promise observer adapter. The separately granted asynchronous disposal consumer uses the active `coroutine_await` boundary and retains `await_value` outside a coroutine. No native callee, coroutine implementation or Job enum changes.


## Fixed Linux gate

Final source `da8b0498e900420274e667ac09e2b5d6998c6f6b` consists of
`d9449fa7` + `52bacee5` + compatibility corrections `8c4f1b8e` + `da8b0498`.
Matched baseline `58596d6d22e8d8aa291fb12084bf208f9ece24b6` and final source both
include only the integrator's Test262 host fix `14a0a09d` on fixed JS27 dependency.
The host fix captures String privately per realm and shares completion/protocol
flags across coroutine threads; it is a dependency, not an owned engine change.

Workspace **3777 passed /17 skipped**, explicit host **one passed**, and **four
doctests**. Whole language gained **152 files /205 variants**, zero lost passes.
Whole Object, Reflect and Promise classifications were unchanged. The four
existing RegExp literal deadline files/eight variants at ten seconds occur on
both sides. Exact inventories, runner/input hashes and classification digests
are recorded in `js-lang_JS28-evidence.json`. Browser phase-loader/scheduler
adoption and the integrator's combined full corpus remain separate gates.
