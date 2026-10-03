# JS41: module declaration instantiation and default exports

Fixed dependency: ready numeric proof1c7e45a3278c22d17c3bab3aa4b386fbac849378. Tests-only baselinee6246d90. No moving integration dependency during paired measurement.

Anonymous default ordinary/generator/async/async-generator functions are actual HoistableDeclarations. The adapter lowers them into the existing strict prepare_module hoister with reserved internal binding name `default`, which cannot collide with a source BindingIdentifier. Their function objects and names exist before dependency evaluation. Named top-level declarations are instantiated once; evaluation adapter omits only top-level Function statements after successful prepare_module, preventing legacy script entry from replacing functions already captured by a cyclic dependency. Original module records and nested block declarations are unchanged.

Anonymous default classes and default expressions retain existing const initialization/TDZ, with the reserved `default` binding providing proper anonymous NamedEvaluation before class static elements run. Comma/conditional/member expressions do not gain names. Named default declarations retain their local names. Synthetic host-provided exports retain the separate NUL default binding and phase/source policy unchanged. No factory, AST/public schema, VM, job or coroutine changes.

Current async graph and TLA coroutine execute statements directly without re-hoisting; the duplicate hoist existed in the legacy synchronous evaluate entry. The shared evaluation program avoids that duplicate while preserving actual graph scheduling and TLA detection from the original module record.

Adapted verified MIT Lumen declaration-instantiation plan: lucid-softworks/lumen crates/lumen/src/interpreter.rs build_hoist_ops/apply_hoist_ops @f6c9860d44fa52249cfd647122edea184d32c863; Copyright(c)2026 Lucid Softworks limited. Hoisting synthetic anonymous default callables and observable default name is reused through our existing architecture. Upstream evaluation fallback that constructs another function is not copied. Mandatory source header and appended notice included.

Primary reference: [ECMA-262 InitializeEnvironment](https://tc39.es/ecma262/2025/multipage/ecmascript-language-scripts-and-modules.html#sec-source-text-module-record-initialize-environment), step24 initializes all four callable declaration kinds; ExecuteModule evaluates code after declarations are instantiated. ExportDeclaration evaluation applies NamedEvaluation only to anonymous default expressions.

Validation pending: eight grouped actual-native/forced-Block × legacy/shared tests covering four callable kinds, cycle identity/reassignment, expression/class TDZ, static name timing/negative forms, nested declarations, TLA early identity and synthetic host values. Whole paired release60 language/module-code/Promise default+VM, workspace nextest and doctests. Root owns installed111-module WPT gate at integration.
