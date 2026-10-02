# Dynamic import host contract

`Interpreter::binding_set_module_loader_hook(Rc<ModuleLoaderHook>)` retains the
string-error API; its callback now runs in a queued microtask. The additive
`binding_set_typed_module_loader_hook` takes a callback returning
`Result<JsValue, Throw>`, preserving the module's thrown object and error type.
Both callbacks receive the evaluated string specifier and module identifier
stored in the importing environment. Classic scripts supply an empty referrer
unless the host uses `run_script_with_source_context(source, referrer)` (or its
`run_script_with_source_context_and_backend` variant). The source locator is
immutable metadata on captured environment handles, while their binding frame
remains the same live global frame; later scripts cannot change a delayed
closure's originating referrer. Ordinary `run_script` retains the empty default.

The callback runs in the importing realm, even if the embedding dispatches its
scheduling token from a different realm. It returns the actual live namespace
object, and the import promise resolves through the normal thenable machinery
(including a namespace's exported `then`). Callback errors reject that promise.
Argument evaluation can throw synchronously; string conversion, options and
attribute validation reject instead. This host supports empty import attributes;
nonempty attributes reject as unsupported.

An embedding keeps one registry per realm and uses:

```rust
let linked = registry.borrow_mut().link(&canonical_id, &source);
linked?;
ModuleLinker::evaluate_shared(&registry, &canonical_id, interpreter)?;
let namespace = registry.borrow().namespace_object(&canonical_id).unwrap();
```

Do not chain `borrow_mut().link(...).and_then(...)` into evaluation: Rust may
retain the temporary guard for the entire statement. `evaluate_shared` releases
all registry guards before dependency/body execution or a job drain; a module
may therefore load another module through the same registry. The existing
`evaluate(&mut self, ...)` remains available for callers without reentrant host
loading.

The registry retains the original realm global environment for later module
instantiation, preventing imported modules from inheriting the currently running
module's private lexical scope. Namespace objects and binding cells remain
singletons, and runtime evaluation failures cache their original thrown object.
Use canonical identifiers for the root as well as dependencies so a self-import
reaches the same record.

Link failures expose parser and static import/export errors as
`ModuleFailure.thrown = Some(Throw)` containing a SyntaxError object. Host
resolver failures retain `thrown = None`, so the embedding can select its fetch
error type. Always prefer an existing `thrown` over constructing a new object
from `phase` or `message`; cloning it preserves identity. A dependency syntax
failure has Resolution phase and the same cached object as a direct Parse-phase
load of that canonical identifier. Registry identifiers are immutable module-map
entries: changing source bytes under a failed identifier does not invalidate it.

The linker caches failed loads and static link roots, and rolls back all newly
inserted records when a link transaction fails. This prevents partially loaded
cycles from poisoning a later graph or exposing missing dependency edges.
Successful pre-existing records and their evaluated instances remain intact.
These load/link caches complement the existing runtime evaluation error cache.

Browser resolvers may override `ModuleResolver::source_url(id) -> Option<String>`
to install `import.meta.url`. Its default is `None`, preserving hosts that do
not expose URL metadata. This URL may differ from the unique registry identity
used for inline module scripts. The resolver supplies it for both roots and
dependencies; each module keeps its own stable metadata object.

This remains a synchronous evaluator underneath the queued host callback. General
async-module sibling concurrency and waiting on already-evaluating async graphs
still require a resumable module graph coordinator. The classic Test262 protocol
also needs an additive filename/root field before its relative imports can use
this loader; JS-21 changes only the module host's existing source-context path.
