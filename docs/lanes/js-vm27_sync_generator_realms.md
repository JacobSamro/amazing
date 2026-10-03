# Synchronous generator realm companion

This companion is independent of suspended class lowering and coroutine strict
capture. It uses the already retained body realm; no new generator state,
realm field, or resumer-realm scalar is added.

The primary algorithms distinguish body execution and yielded results from
completion results. [Yield](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-yield)
creates its iterator result while the generator body context is active, before
GeneratorYield runs the caller. [GeneratorStart](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-generatorstart)
removes the body context before constructing a completed result.
[GeneratorResume](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-generatorresume)
and [GeneratorResumeAbrupt](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-generatorresumeabrupt)
create initial/completed short-circuit results in the actual invoking context.

Boat Node v24.18.1 confirms these distinctions for both plain and suspended
class generators. Borrowed parent next methods produce child-realm body
arrays, constructors/prototypes, and yielded result objects; completed and
after-completed result objects belong to the invoking parent. The original
Amazing diagnostic uses WIP `5647102f` release binary SHA256
`5400de0b8036178ec8403ca04989d9c55df63610d12653e37a82a32ef44fb15b`:
body allocations and ordinary yielded results incorrectly belonged to the
parent. See the retained raw reference artifact. This is a pre-existing
general generator-context defect, not a class-specific realm workaround.

Native and actor sync drivers scope actual body resume in the stored body
realm. Ordinary Yield constructs its result there; delegated raw result
identity is retained. Body error normalization also happens there. Completion
and initial/completed abrupt results remain outside that scope. The async
generator queue, Await policy, With environments and class private identities
are unchanged. Grouped native/forced-AST proof covers borrowed methods,
class-key suspension, With/finally injected Return, body ReferenceError realm,
raw delegated results, user Throw identity and invalid receivers. Readiness
requires the separate matched release corpus and workspace/doctest gate.

Final source `fe2690f0` on exact `3832dbd2` passes **4220 workspace tests**
(17 skipped), versus 4217 before, and **four doctests**. All three grouped
native/forced-AST borrowed-method matrices pass. The 26 matched release
reports retain **30305 passing files / 56764 variants** among **30442 files /
56960 variants** in both modes, with identical classifications and failure
reasons, zero lost passes and no new host crashes. Callable compilation
attempts/accepted remain 1428421/1428421 in default mode and 802936/802936 in
VM-program mode; both have zero fallbacks. The whole class27 final reports
are reused as the exact production-identical before source, with matching
pins, capability map, jobs, deadlines and release profile. See
[js-vm27_sync_generator_evidence.json](js-vm27_sync_generator_evidence.json).
