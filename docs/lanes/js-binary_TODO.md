# js-binary lane decision map

## JSB-1 — binary data built-ins

Base: `db0a8e7`. The byte store and view state live in
`crates/amazing-js/src/builtins_binary.rs`, keyed by weak object identity.
This keeps the shared `JsObject` representation stable while exposing live
ArrayBuffer, SharedArrayBuffer, DataView, TypedArray and Atomics operations.
The VM reaches the same property hooks through `Interpreter` (ADR 0003).

The DOM-17 host facade is `Interpreter::binding_create_array_buffer` and
`Interpreter::binding_create_uint8_array`; both make fresh fixed buffers.

Shared runtime gaps resolved for these suites:

- Integer-indexed get/set/has/delete/define/descriptor/key hooks in
  `interp.rs` and `interp_builtins.rs` (ECMA-262 §10.4.5).
- Internal `Throw` normalization to realm Error prototypes (§20.5), native
  function name/length descriptors (§10.2.8, §20.2.4), and object-literal
  accessor/data property replacement (§13.2.5).
- `StringNumericLiteral` ToNumber prefixes and grammar (§7.1.4.1.1),
  `Array.from` array-like fallback (§23.1.2.1), and `Array.prototype.splice`
  (§23.1.3.30), needed by pinned Test262 helpers.

Pinned Test262 revision: `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`.
Linux iteration on identical inputs: full Test262 files with 11,967 baseline
passes and 16,070 after passes; zero lost passing files or variants. Directory
after pass counts: ArrayBuffer 186, SharedArrayBuffer 92, DataView 361,
TypedArray 695, TypedArrayConstructors 329, Atomics 163. Windows final pass
counts match those six figures exactly. Full Windows Test262 passed 11,967
baseline files and 16,070 after files, with zero lost passing files or variants.
The pinned Windows input hash was
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`;
the after classification hash was
`01672f42eb5472a9ead7057ef842c35b691173075e19d75dbec7aba01008ee1c`.

Remaining JSB-1 conformance gaps include constructor `newTarget` timing pending
JSRT-7, TypedArray callback/species behavior under resize or detachment,
Atomics.waitAsync, and pruning dead weak side-table entries. BigInt binary
operations wait for JSB-2 rather than using lossy Number conversions.

## JSB-2 — BigInt

Implemented signed base-2^32 limb arithmetic, BigInt literals, constructor,
prototype, coercion, operators, comparisons, `asIntN`/`asUintN`, BigInt
TypedArrays, DataView accessors and Atomics. The Test262 host now calls the
binary module's real ArrayBuffer detach operation, so BigInt TypedArray
detachment tests run instead of becoming unsupported when they reach the host.

Pinned Linux Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`,
53,872 sources, input hash
`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`:
JSB-1 base `2a2f9b2` had 16,070 pass / 27,281 fail / 15 timeout /
10,212 unsupported / 294 skipped. JSB-2 had 17,112 pass / 26,449 fail /
14 timeout / 10,003 unsupported / 294 skipped, with **zero lost passing
files and zero lost passing variants**. After classification hash:
`d67aa6fff95083484fd39b48cc75e911a61bb829a1cf93803b7ba8fcc54b5cf7`.
Selected pass counts: `built-ins/BigInt` 1→67,
`TypedArrayConstructors/ctors-bigint` 20→75, `DataView` 361→434,
`Atomics` 163→207. Linux amazing-js nextest passed 225/225 and its doctest
passed 1/1.

The authoritative win-002 final gate used the same pinned revision and 53,872
sources, input hash
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`.
JSB-1 base: 16,070 pass / 27,283 fail / 14 timeout / 10,211 unsupported /
294 skipped. JSB-2: 17,112 pass / 26,451 fail / 13 timeout /
10,002 unsupported / 294 skipped. **Zero lost passing files and zero lost
passing variants**. After classification hash:
`401976ddd429ed3ce27974abcd6cd9956cd414c68cdbda8c898f55a41cb9f351`.
win-002 amazing-js nextest passed 225/225, crate doctests 1/1; workspace
nextest passed 1,779/1,779 with four skipped, workspace doctests 2/2.

Remaining gaps include BigInt Test262 cases blocked by user-thrown error
constructor identity, global descriptor handling, and BigInt wrapper property
overrides. The bigint limb implementation caps enormous exponent/shift/width
results at 1,048,576 bits to avoid unbounded allocation. VM unary BigInt
fast paths are routed to js-vm in `.agents/js-binary/questions.md`.

The VM delegates binary operators to `Interpreter::binary`, but its unary
Neg and BitNot instructions use Number fast paths in `vm/mod.rs`. Route that
hook through js-vm; this lane does not edit `vm/`.
