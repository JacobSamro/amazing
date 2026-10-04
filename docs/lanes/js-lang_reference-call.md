# JS49 — Ordinary property Reference/GetValue coherence

Original source-only base: `196151944624bd84c13f5cd977d099d3e39d5a45`.
Independent delivery base: published `00e6045c9d506762781392037132880869a9f58f`.
The first fixed pair tested source `491ee2c1` against tests-only `14c7b692`.
It remains unqualified: native 4410 PASS / 6 new FAIL / 17 SKIP became
4413 PASS / 3 new FAIL / 17 SKIP, with four doctests passing on both sides.
No existing PASS or SKIP identity was lost. The strict native guard stopped
the candidate before release/corpus; the sixteen baseline corpus receipts
are evidence of the baseline only.

## Algorithm and boundaries

Current primary [property access evaluation](https://tc39.es/ecma262/multipage/ecmascript-language-expressions.html#sec-property-accessors-runtime-semantics-evaluation)
and [GetValue](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-getvalue)
were reviewed on 2026-10-04. A computed expression evaluates and produces its
raw value before the property Reference is consumed. GetValue checks/boxes the
base before ToPropertyKey. Therefore `null[raw()]` must evaluate `raw`, but must
reject null before invoking a returned object's coercion hook. A thrown raw
expression retains precedence and identity. Plain assignment retains its
separate RHS-before-PutValue/key-coercion order.

Ordinary AST Member reads now use the existing chain-reference consumer,
already shared by calls, tags, optional links, private members and Super reads.
The ordinary VM Member read invokes the existing `member_key(..., true)` gateway:
evaluate raw expression, RequireObjectCoercible, ToKey, GetMember. No new opcode,
Reference variant, source metadata, Env/frame field, worker transfer, lifetime
or scheduling contract is introduced. No upstream code was copied; existing
ported reference helpers and notices are unchanged.

Root granted these two production seams. VM owner confirmed that the ordinary
Member compiler arm is disjoint from frozen Delete38 on 2026-10-04. Generic
Delete helpers, raw writes, compound/update consumers, body/FDI metadata and
module host policy are unchanged.

## Authored whole-area matrix

Eight interpreter groups exercise actual forced-AST and native callable
factories, null/undefined/static/computed/grouped/nested reads, raw-expression
and coercion abrupt identity, primitives, symbols/lone UTF16 keys, Proxy
receiver/Get, private access, optional grouping and short circuit, methods and
tags before argument evaluation, delete/plain-write/compound/update/destructuring
controls. The suspended matrix covers three callable kinds in both actual
factory modes; generator replacement proves the original base remains captured
across raw-key suspension.

Two VM-private groups prove actual RequireObjectCoercible→ToKey→GetMember
instruction order with no TreeEval, and single-instruction quantum retention of
the original base with a raw key mutated before its later coercion.

These are authored fixtures, not native results. No local build/test, new Boat
mission, corpus cut, accepted-pass waiver, fake host ModuleSource or claim of
AST/VM parity was made. Planned matched gates retain whole language, Function,
Object, Proxy, Reflect and callable async/generator selections, workspace and
doctests, genuine factory assertions and all original abrupt diagnostics.

Delivery closure: all ten original authored groups remain byte-identical. The existing private test actor flag receives a cfg(test)-only setter, identically in before/after fixtures; no production API or field is introduced. Fresh current primary GetValue3.a and property-expression13.3.3 were reviewed on 2026-10-04: raw expression GetValue precedes base ToObject; base ToObject precedes property ToPropertyKey. Existing MIT-ported Reference and chain gateways are reused whole, with their headers/notices unchanged; no new upstream code is copied. Current Super dispatcher, raw writes, deletion helpers, frame/FDI and caller/source metadata are untouched. Before outcomes are observed, not predetermined or waived.

## Whole Call producer/consumer closure successor

Root subsequently granted the genuine ordinary Call closure after the first
gate demonstrated lost receivers through parenthesized member callees in
native ordinary/generator/async/async-generator execution. The whole previously
qualified VM28 adapter is being integrated: ordinary calls use the rooted
chain reference pair, contextual eval keeps the captured callable and evaluated
argv, and its VM consumer delegates to the existing interpreter call gateway.
All ten original JS49 groups remain unchanged; all three original VM28 groups
are retained, covering parenthesized/indirect/optional/direct eval, With names,
private/Super receivers, callee capture and abrupt suspension.

The reused call lowering is adapted from boa-dev/boa
`core/engine/src/bytecompiler/mod.rs::call` at
`39cd11214ecb366200157dab96b3919e61e9a391` (MIT), preserving original headers
and adding its exact notice. VM ownership ACK confirms no overlap with the
separate physical call-resource admission area; ordinary call_value remains
the invocation boundary after argument evaluation.

The independent successor is assembled on published base
`7b5a62c10ea9d2588356d83be982a8792eba639c`. Genuine prerequisites are now
published: lossless Array.join `71cbb461` and corrected Super read `01abc0ec`.
The three-way assembly preserves the current Super dispatch and first-link
LoadThis → raw computed expression → LoadSuperHome order. All ten original JS49
and three VM28 groups remain byte-identical. This assembly is unbuilt and
ungated; historical first-pair diagnostics remain historical evidence only.

## Qualified independent delivery

The fixed Linux pair tested `6e5bc198ed542dd846ebd2d49f190ea1db4336c1`
(tests only) against `b37e4c491abf647d7740482f3421444284023d99` on published
`7b5a62c10ea9d2588356d83be982a8792eba639c`. Workspace: 4,497 PASS / eight
new authored FAIL / 17 SKIP became **4,505 PASS / zero FAIL / 17 SKIP**; all
original PASS and SKIP identities retained. Four doctests passed on both sides.
All ten original Reference groups and three original Call groups pass.

Eight complete selectors in each of default and VM mode cover language, Object,
Proxy, Reflect, Function, AsyncFunction, GeneratorFunction and
AsyncGeneratorFunction. Each mode retained all 29,324 selected file identities:
28,913 → 28,914 passing files and 53,985 → 53,987 passing variants;
**+one file / +two variants, zero lost files or variants**. The gain is
`language/expressions/member-expression/computed-reference-null-or-undefined.js`.
All 32 corpus controllers returned normal 0/1; workspace/docs/release phases,
compact fetches and shipping binary preservation completed with actual exits.
Both sources ran on the same baremetal four-vCPU/eight-GiB assigned sandbox.
Corpus modes select ordinary callable factories; forced-AST/native coverage is
claimed for the authored groups that assert those actual factory choices.

Compact proof: `js-lang_reference-call-gate.json` (61,384 bytes); raw logs,
reports and both exact shipping binaries remain on Boat. No private recovery
controller, corpus waiver, fabricated prerequisite or local build was used.
