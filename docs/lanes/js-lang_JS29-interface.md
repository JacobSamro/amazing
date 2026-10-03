# JS29 callable metadata and NamedEvaluation

Fixed dependency: JS28 evidence `b03ae45b096042347d97af935647deaf9118458d`.
Source series: `0c1f3e31e39946fe7a4931414ba87955a99d21a4` +
`4d7a759dde83b10bc3545f3463b47e85335a31d9`.

`JsObject::function` creates the configurable, nonenumerable `length` property
before `name`. Redefining either property retains its original insertion position,
including class static methods that overwrite constructor name or length.

Property-based SetFunctionName uses `JsString` units throughout. Symbols without
a description produce the empty name; described symbols produce bracketed
names. Getter/setter prefixes precede the exact units, including lone surrogates.
Class methods, field initializers and object property initializers preserve this
observable name while the existing internal Rust-string diagnostic name remains
lossy. The existing string helper remains available.

Plain and logical assignment name a value only when the source is an anonymous
function definition and the target is an identifier reference. Parentheses retain
anonymous-definition status on the right. A grouped assignment target resolves
the same binding but does not satisfy syntactic IsIdentifierRef, as checked by the
[pinned Test262 fixture](https://raw.githubusercontent.com/tc39/test262/3655e7464de3d52643ecddd4b5f9f4f3e7f62398/test/language/expressions/assignment/fn-name-lhs-cover.js).
Comma expressions, existing callable values and
member targets do not acquire an inferred name. The VM emits explicit
NamedEvaluation at the same source/target boundary, and stores no longer rename
arbitrary values. Reference capture and computed write-key conversion order stay
unchanged.

An additive NamedPropertyEvaluation instruction applies only to object-literal
Init properties. It receives the already-converted property key; it does not
reevaluate a key expression or invoke conversion again. Literal __proto__,
shorthand properties and method-definition paths retain their own operations.
Suspending class evaluation uses the same method name/length helper, while all
existing nonsuspending class fallback guards and HomeObject/prototype wiring
remain intact.

Five new grouped integration tests run both backends. The earlier assignment
heuristic regression is replaced with spec-positive identifier and spec-negative
value/comma/member/logical cases. Whole language/Function/Object/Proxy measurements
and fixed-source provenance belong in `js-lang_JS29-evidence.json` after the gate.

## Fixed Linux gate

Workspace **3782 passed /17 skipped**, explicit host **one passed**, and **four
doctests**. Whole language **24134→24146 files /44721→44745 variants (+12/+24)**,
Function **501→503 /878→882 (+2/+4)**, Proxy **280→281 /545→547 (+1/+2)**,
and Object **3338/6662 unchanged**. Every selection has zero lost files or variants.
The four existing RegExp literal deadline files/eight variants at ten seconds are
unchanged. Five new grouped integration tests plus the revised existing naming
regression cover both execution backends. The final source is fixed4d7a759d on
JS28 evidence b03ae45b; exact hashes/inventories are in JS29 evidence.
