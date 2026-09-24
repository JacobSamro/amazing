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
fixed `dev` base `d136fdd` had 19,687 pass / 23,658 fail / 16 timeout /
10,217 unsupported / 294 skipped. JSB-2 had 20,999 pass / 22,557 fail /
15 timeout / 10,007 unsupported / 294 skipped, with **zero lost passing
files and zero lost passing variants**. After classification hash:
`d6dad40499b358008e52a69ae8d82e91441dcee111f1de3822ce320cb8ddc254`.
Selected pass counts: `built-ins/BigInt` 1→68,
`TypedArrayConstructors/ctors-bigint` 20→80, `DataView` 368→442,
`Atomics` 163→207. Linux amazing-js nextest passed 235/235 (one skipped),
workspace nextest 1,927/1,927 (three skipped), and the crate doctest passed
1/1. The updated Test262 runner self-test passed.

The authoritative win-002 final gate used the same pinned revision and 53,872
sources, input hash
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`.
Fixed `d136fdd` base: 19,687 pass / 23,659 fail / 16 timeout /
10,216 unsupported / 294 skipped. JSB-2: 20,999 pass / 22,558 fail /
15 timeout / 10,006 unsupported / 294 skipped. **Zero lost passing files and zero lost
passing variants**. After classification hash:
`d34e6f1a04def59753898bd8e519031f07e5f5eb249a5635c5c0737f885fa51c`.
win-002 amazing-js nextest passed 235/235 (one skipped), crate doctests
1/1; workspace nextest passed 1,930/1,930 (five skipped), workspace doctests
2/2. The updated Test262 runner self-test passed on Windows too.

Remaining gaps include BigInt Test262 cases blocked by user-thrown error
constructor identity, global descriptor handling, and BigInt wrapper property
overrides. The bigint limb implementation caps enormous exponent/shift/width
results at 1,048,576 bits to avoid unbounded allocation. VM unary BigInt
fast paths are routed to js-vm in `.agents/js-binary/questions.md`.

The VM delegates binary operators to `Interpreter::binary`, but its unary
Neg and BitNot instructions use Number fast paths in `vm/mod.rs`. Route that
hook through js-vm; this lane does not edit `vm/`.

## JSB-3 — Intl from pinned CLDR data

Fixed base: `d9b253e78e040cb243671871975f9c04db86c2fd`. Implementation:
`0aafabc004287268bfaeb837195e00ca4257cab7` (including the earlier
BufferSource host binding). The checked-in `scripts/generate_intl_data.py`
consumes Unicode CLDR JSON 48.2.0, archive SHA-256
`4f28298550dea43439986cd9ffb0a69cb73ba7d0b34a3c474760905ce8ce9e70`.
Regeneration on venus-003 produced byte-identical tables. The locale set is
`en`, `en-GB`, `de`, `fr`, `es`, `ja`, `zh-Hans`, `ar`, `hi`, and `ru`, with
English data as the fallback.

The new Intl namespace includes locale canonicalization and `Intl.Locale`,
NumberFormat, PluralRules, Gregorian DateTimeFormat, RelativeTimeFormat,
ListFormat, Collator, Segmenter, and DisplayNames. It also installs
locale-sensitive methods on String, Number, BigInt, Date, and Array. The
Test262 host now advertises `intl402`.

On pinned Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`
and Linux input SHA-256
`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`,
the fixed base had 24,762 pass / 24,307 fail / 22 timeout / 4,487
unsupported / 294 skipped files. JSB-3 had 25,601 pass / 26,815 fail /
22 timeout / 1,140 unsupported / 294 skipped: **839 gained passes and zero
lost passing files or variants**. `intl402/` had 720 pass / 2,627 fail /
10 unsupported files. Two repeated selections had identical file and variant
classifications, SHA-256
`60c12f029ba4f6f5ce4d6902d5f1f6a11457f0ebfb5198f3a5796860bb4980c4`.
The Test262 runner self-test passed.

The authoritative Windows run on win-001 used the same 53,872-file revision,
with input SHA-256
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`.
The fixed base had 24,762 pass / 24,308 fail / 22 timeout / 4,486
unsupported / 294 skipped files; JSB-3 had 25,601 pass / 26,816 fail /
22 timeout / 1,139 unsupported / 294 skipped. The Windows comparison also
found **839 gained passes, zero lost passing files, and zero lost passing
variants**. Base and after classification SHA-256 values were
`cd3d6418cd2b3417515fbcac83345ee68128cd9866d9a1186b0db8b7b17fa523`
and `c48b86dd1264ee7ea949a251af77d9ba63e4854c3938bf6e13e0298fcd18b770`.
On win-002, amazing-js nextest passed 280/280 (one skipped) and its doctest
passed 1/1; workspace nextest passed 2,121/2,121 (five skipped) and
workspace doctests passed 2/2.

Remaining ECMA-402 gaps include IANA time zones, full Unicode collation
tailoring and segmentation rules, NumberFormat option/range corner cases,
and broader locale data. `Temporal` and `DurationFormat` Intl cases are
outside this ticket's implementation. The regex class-compilation resource
limit affecting two language-tag tests is routed to js-lang in the lane's
`questions.md`.

## JSB-4 — Intl depth and pinned IANA time zones (in progress)

The current branch adds decimal tie and increment rounding, scientific and
engineering notation, compact notation for the bounded locales, DateTimeFormat
option validation and defaults, DisplayNames code validation, and correct
Segmenter `isWordLike` property presence. Its DateTimeFormat accepts IANA zone
IDs and computes offsets per instant, including DST and historical seconds.

`scripts/generate_intl_tzdata.py` consumes the official IANA
`tzdata2026d.tar.gz` archive, SHA-256
`0cb2aa8e333c3dc049badc42a0c61f21987b8cd44e107fa900bad764aacc7767`.
It invokes `zic` with the explicit 1800–2500 range and packs 597 zone/link
names into `builtins_intl_tzdata.bin` (2,237,090 bytes, SHA-256
`0bd177c49e4a83cf96d3c85be7c159fc12e50f479642782df18cfe17ade66744`).
The `tz_canonicalize`, `tz_offset_ns`, `tz_possible_epoch_ns`, and transition
queries are shared nanosecond APIs for js-stdlib's Temporal implementation.
The API has exact instants for ordinary wall times, gaps, and folds. Zone rules
outside the generated 1800–2500 interval currently use the first/last stored
offset and need a future extension for distant Temporal instants.

The pinned Linux Test262 selection moved from 720 to 751 passing `intl402/`
files, with zero lost passing files. The full 53,872-file Linux comparison on
JSB-3 base `3494c85` moved from 25,601 to 25,632 passing files: 31 gained,
zero lost files. One previously passing strict variant of the unrelated
`staging/sm/TypedArray/sort_modifications.js` test exceeded the 60-second
limit during both high-load full runs; isolated 120-second runs passed both
variants. The authoritative win-001 full Test262 comparison used input SHA-256
`a2ef5e43ff95da279324b6df9a37740a505b9f8b6c7a11406408af70f453b075`
and moved from 25,601 to 25,632 passing files, with **31 gained and zero lost
passing files or variants**. The exact final-source rerun had the same
classification SHA-256 as the first run:
`c39a42a25e4604dd9dac4db96ed08ba3b0be96541331663ae55292d93b06f537`.
Win-002 nextest passed amazing-js 283/283 (one skipped) and workspace
2,124/2,124 (five skipped); doctests passed 1/1 and 2/2.

`Intl.supportedValuesOf("timeZone")` still enumerates UTC only. Enumerating
all canonical IANA IDs before js-stdlib migrates Temporal.ZonedDateTime lost
three previously passing Test262 files, which construct a ZonedDateTime for
every enumerated ID. The full list can be exposed after that integration and
must be gated together with it.
