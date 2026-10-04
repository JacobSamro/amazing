# Function.prototype.bind

Recorded published base: `1710e4d39a460d0208f7ed2ffda33c4995458fa0`. One independent ready squash; no prerequisite histories imported.

Bind follows current ECMA-262 §20.2.3.2 and BoundFunctionCreate: callable check, actual GetPrototypeOf before metadata, semantic HasOwn length/conditional Get, Number-only ToIntegerOrInfinity, exact UTF-16 name, and complete fresh metadata descriptors. Private BoundArguments List storage performs no Array Set or iteration. Existing forwarding, NewTarget substitution and target realm dispatch remain unchanged.

Port: boa-dev/boa `core/engine/src/builtins/function/{mod,bound}.rs` @ `39cd11214ecb366200157dab96b3919e61e9a391`, verified MIT (Copyright 2019 Jason Williams). No dependencies added.

Own Linux gate: BEFORE 4463 pass / 5 new protocol failures / 17 skips; AFTER 4468 pass / 0 failures / 17 skips. Four doctests passed on both. All ten groups retain genuine AST/Bytecode carrier controls. Fourteen whole Test262 selectors in both release modes retain 35,583 passing files / 67,257 passing variants per mode, zero lost files or variants. Fresh final release binaries match the completed corpus hosts exactly. Root's full 16-shard merge gate is still required before publication.

The initial compile error and authored wrong constructor prototype oracle remain diagnostics. [Bound [[Construct]] step 5](https://tc39.es/ecma262/multipage/ordinary-and-exotic-objects-behaviours.html#sec-bound-function-exotic-objects-construct-argumentslist-newtarget) substitutes NewTarget only when identical to that Bound function. Node 24.18.1 confirms the outer Proxy survives mixed forwarding and yields Object.prototype fallback. The corrected fixture additionally checks actual `new.target === wrapped`; original resource controls remain unchanged and held until this foundation publishes.

Compact evidence is in `evidence/function-bind-current.compact.json`; full raw reports/logs remain on Boat.
