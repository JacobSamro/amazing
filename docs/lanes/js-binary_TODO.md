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

Next: arbitrary-precision std-only limb arithmetic and BigInt built-ins in
`builtins_binary_bigint*.rs`, then the shared value/coercion/operator paths,
BigInt TypedArrays, DataView accessors and Atomics. Measure the pinned BigInt
Test262 selections and full-suite zero lost passes before its own gate.

The VM delegates binary operators to `Interpreter::binary`, but its unary
Neg and BitNot instructions use Number fast paths in `vm/mod.rs`. Route that
hook through js-vm; this lane does not edit `vm/`.
