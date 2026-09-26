# JavaScript standard library lane

## JSS-1, areas 1–3: Reflect, keyed collections, Iterator

Fixed base: `6146edbb6596a34c5c192f0ef6428920f4a09e2e`.
Pinned Test262: `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`.

The lane implements all 13 Reflect methods, Map/Set and weak collection
constructors and methods, Map.groupBy, ES2025 Set operations, and the Iterator
constructor and helper methods. Collection iterators are live and inherit the
shared Iterator prototype. FinalizationRegistry cleanup is asynchronous and
currently never scheduled by this host.

The full pinned Linux Test262 run moved from **11,967 pass / 31,622 fail /
15 timeout / 9,974 unsupported / 294 skipped** to **12,691 pass / 30,894
fail / 15 timeout / 9,978 unsupported / 294 skipped**. At the variant level,
1,443 newly pass and **zero formerly passing variants are lost**. The input
snapshot hashes match. The final classification hash is
`fcef8dec5121291e273650486a4a45aff7d8e31a1476259c161d880b26758413`.

| Selection | Before pass | After pass | After fail |
| --- | ---: | ---: | ---: |
| Reflect | 17 | 82 | 71 |
| Map | 24 | 92 | 111 |
| Set | 39 | 182 | 200 |
| WeakMap | 0 | 38 | 102 |
| WeakSet | 0 | 32 | 52 |
| WeakRef | 0 | 17 | 11 |
| FinalizationRegistry | 0 | 28 | 18 |
| Iterator | 0 | 138 | 515 |

Remaining gaps include Proxy internal methods, exact accessor function
identity in the existing `Property` model, weak ephemeron reachability,
iterator abrupt completion and newer Iterator methods. Error constructor
realm identity is being addressed by JSB-1. These failures stay visible in
the pinned corpus and are not counted as unsupported.

## JSS-1, area 4: Temporal ISO surface

The lane has installed the ISO method and accessor surfaces for Duration,
Instant, PlainDate, PlainTime, PlainDateTime, PlainYearMonth, PlainMonthDay,
ZonedDateTime, and Temporal.Now. Instant and ZonedDateTime retain exact epoch
nanoseconds in private `i128` slots. UTC and minute-precision fixed offsets
are supported. Plain values also use private slots, so own-key reflection
does not expose their fields.

The implementation includes ISO parsing and formatting, calendar fields,
plain-type conversions, arithmetic, differences, rounding, Duration totals,
and the UTC/fixed-offset Now factories. The pinned `built-ins/Temporal`
selection rose from 612/4,603 at the first constructor checkpoint to
**1,449/4,603** in the final full-corpus run. Each selected snapshot retained
all previously passing files.

Against the exact areas 1–3 commit `441df906ee261f22f4fef882cbdb420785585ff7`,
the full pinned Linux Test262 run moved from **12,691 pass / 30,894 fail /
15 timeout / 9,978 unsupported / 294 skipped** to **14,142 pass / 29,443
fail / 15 timeout / 9,978 unsupported / 294 skipped**. The input SHA-256 is
identical (`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`),
with **2,902 new passing variants and zero lost variants**. The final
classification SHA-256 is
`06a0b700d7a3279be72d95f050242e437b128a3674ea59942c53a4be2b9e2ee3`.
Linux `amazing-js` nextest passed 216/216, and its doctest passed 1/1.

JSB-2 owns arbitrary-precision BigInt. Until it lands, constructors that
require BigInt epoch nanoseconds and BigInt-returning epoch getters throw;
the internal `i128` representation and string/Now factories allow the other
methods to run. JSB-1 owns realm error prototypes; many Test262 negative
assertions currently observe the wrong thrown-object prototype. About 750
Temporal files still fail for other reasons, including parser, option-order,
calendar arithmetic, and host JavaScript gaps; this is a measured conformance
tranche, not complete Test262 conformance. Named IANA zones and non-ISO
calendars remain outside this host's scope. All remaining failures stay
visible in the pinned corpus rather than being classified away.

## JSS-1 priority interrupt: core built-ins

Added the missing String case conversion, padding, repeat, concatenation,
search/replacement, Annex B HTML methods, and static `fromCodePoint`/`raw`;
Number formatting, predicates, constructor and constants; Math constants and
functions; JSON parse/stringify with reviver, replacer, and indentation; and
global numeric and URI functions. Case conversion uses Rust's Unicode mappings
with Turkish/Azeri locale tailoring. Strings and JSON preserve UTF-16 code
units, including isolated surrogates. `Object.defineProperty` now uses the
existing `define_own_property` path so descriptors preserve omitted fields;
the old path overwrote a JSON reviver's nonconfigurable property value.

Full pinned Linux Test262 on fixed Temporal base `b257461` and identical input
SHA-256 `91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`:

| Run | Pass | Fail | Timeout | Unsupported | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before | 14,142 | 29,443 | 15 | 9,978 | 294 |
| After | 15,218 | 28,367 | 15 | 9,978 | 294 |

At `(path, variant)` granularity, 2,134 variants newly pass and **zero pass
variants are lost**. Final classification SHA-256 is
`0bc07b17895600776cbc5889769fc9b3527f41f47efa0ce1185e31fe3a7761fb`.
Final selected passes: String 748/1,223, Number 221/340, Math 253/327,
JSON 65/165, and global 19/29. Linux `amazing-js` nextest passed 216/216 and
its doctest passed 1/1.

This installs the missing members but does not close every semantic gap.
`String.prototype.matchAll` still delegates to the existing match result and
returns an array iterator rather than spec match records. Number radix
conversion is bounded and does not yet use the shortest-round-trip algorithm.
Locale casing implements Turkish/Azeri tailoring; full ECMA-402 locale
selection is outside this host. Most remaining selected failures expose
underlying engine behavior such as realm error prototypes, boxing, Proxy,
RegExp, or parser support and remain counted as failures.

## JSS-1 Temporal BigInt epoch integration

After JSB-2 landed, `Temporal.Instant` and `Temporal.ZonedDateTime` constructors
now convert their epoch argument with `ToBigInt`, enforce the Temporal epoch
range, and retain exact `i128` nanoseconds. Their `epochNanoseconds` getters
return a BigInt. `Instant.fromEpochNanoseconds` and
`Instant.fromEpochMilliseconds` are installed with BigInt and integral Number
input respectively. ZonedDateTime continues to support UTC and fixed offsets;
its optional ISO calendar argument rejects non-string values with TypeError.

The rebased core built-ins preserve JSB-2 behavior: `Number(BigInt)` explicitly
converts the primitive, JSON.stringify throws for BigInt unless `toJSON`
replaces it, and TypedArray index descriptors dispatch through the binary hook
inside `define_own_property`. The `matchAll` path observes RegExp `flags`, and
repeated definitions of a function's `length` retain its configurable
descriptor. These fixes close the losses found when the earlier core commit was
combined with JSB-2.

Fixed base `5e837a4830d0c049b5c5883dcd7c7c0ab799dfca`; pinned Test262
revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`; identical full-input
SHA-256 `91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`:

| Run | Pass | Fail | Timeout | Unsupported | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Base | 20,999 | 22,557 | 15 | 10,007 | 294 |
| Integrated branch | 23,036 | 20,513 | 22 | 10,007 | 294 |

At `(path, variant)` granularity, **4,053 new passes and zero lost passes**.
Final full classification SHA-256:
`8e9283e49199539fff07b602b99501d7b67b8f1ff2d6b1bd82c6387957e6303a`.
The named `built-ins/Temporal/` selection passes 3,288/4,603 files; two
consecutive runs have identical classification SHA-256
`93c39a225d0a1b8efe13fbc096185da8c9c97f7fdc7fc8faa67636e0eb4c6bb1`.
Linux `amazing-js` nextest passed 235/235 with one skipped; the doctest passed
1/1. The exact-commit Windows workspace gate on win-002 passed 1,995/1,995
nextest tests with five skipped and 2/2 doctests.

Remaining Temporal failures include calendar/time-zone behavior beyond the
UTC/fixed-offset ISO scope, Proxy behavior, and other shared engine semantics.
They remain failures in the pinned corpus.

## JSB-4 IANA time-zone integration

`Temporal.ZonedDateTime` now retains a canonical time-zone identifier and uses
JSB-4's pinned resolver at each instant for local fields, offset accessors,
formatting, and arithmetic. Local-to-instant conversion handles gaps and folds
with `compatible`, `earlier`, `later`, and `reject` disambiguation. String and
property-bag construction honor offset choices, including historical offsets
with seconds. `startOfDay`, `hoursInDay`, and `getTimeZoneTransition` use the
zone's rules. Fixed offsets and UTC still use the same resolver interface.

The fixed source base is JSB-4 commit `4c5c62d0f0b5afd184267a96a6eb9dc44de44443`.
Pinned Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398` and full-input
SHA-256 `91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`
were identical in both Linux release-host runs:

| Run | Pass | Fail | Timeout | Unsupported | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| JSB-4 base | 25,632 | 26,784 | 22 | 1,140 | 294 |
| IANA integration | 25,704 | 26,712 | 22 | 1,140 | 294 |

At `(path, variant)` granularity, **144 variants newly pass and zero previously
passing variants are lost**. The final full classification SHA-256 is
`5688f1b751255b50ab3a794941512da7569e62175318e11c677ff5a08eb310c4`.
The `built-ins/Temporal/` selection passes 3,319/4,603 files on two
consecutive runs with identical classification SHA-256
`e2ec39b1c51d7bbf4efb68e7d48f78d18c2d0b0676e90b69e6016eb84f4d79b1`.
The new full-suite passes include IANA DST disambiguation, offset getters,
transition queries, day lengths, and zone-aware date arithmetic.

Linux `amazing-js` nextest passed 283/283 tests with one skipped; its doctest
passed 1/1. The final Windows workspace gate is recorded in the lane report.

## JSS-next checkpoint: Temporal and Iterator

Fixed base: local `dev` `18a54d31c350c99a96e24dd72b6c49cdfe0e118c`.
The three largest owned failing Test262 built-ins directories on that base were
Temporal (1,284 failures), Iterator (437), and String (298). Temporal parsing,
calendar conversion, IANA offset parsing, and PlainYearMonth differences now
cover more ISO forms and enforce the tested range and rounding rules. Iterator
adds `includes`, `chunks`, `windows`, `zip`, and `zipKeyed` with lazy state,
argument validation, and iterator closing.

Pinned Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`:

| Full Linux comparison | Pass | Fail | Timeout | Unsupported | Skipped | New passing variants | Lost variants |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Fixed base | 26,206 | 26,791 | 22 | 559 | 294 | — | — |
| After Temporal | 26,545 | 26,452 | 22 | 559 | 294 | 678 | 0 |
| After Iterator | 26,615 | 26,382 | 22 | 559 | 294 | 140 | 0 |

String and the requested win-002 gate remain for JSS-next after the priority
structured-clone insert. These Linux measurements are cluster comparisons,
not the final Windows gate.

## JSS-SC structured clone priority insert

On rebased fixed base `484ea8799b1629aacecd79c654d387b86d5459ea`, the
`structured_clone.rs` module implements an owned `SerializedValue` graph with
node indices for cycles and repeated object identity. It carries primitives,
BigInt, boxed primitives, Date, RegExp, ArrayBuffer, TypedArray, DataView, Map,
Set, Error, arrays, and ordinary objects across realms. The public
`StructuredCloneHost` hook stores a host type name and owned bytes for later
Blob, File, and ImageData integration. `Interpreter::structured_serialize`,
`structured_serialize_for_storage`, `structured_serialize_with_transfer`,
`structured_serialize_with_host`, `structured_deserialize`, and
`structured_deserialize_with_host` expose the IPC and storage seam; the global
`structuredClone(value, { transfer })` uses it. Transfer detaches reachable
ArrayBuffers only after successful serialization. Functions, symbols,
duplicate/unreachable transfer entries, and unsupported host objects raise
DataCloneError. The graph is `Send` and does not retain interpreter objects.

A small shared array write correction preserves the non-enumerable `length`
descriptor while array literals and indexed writes grow an array. This fixed
nested array WPT assertions. An `EvalError` constructor completes the native
Error family used by the structured clone WPT battery.

Pinned Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`,
identical input SHA-256
`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`:

| Full Linux comparison | Pass | Fail | Timeout | Unsupported | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Fixed dev base | 26,206 | 26,791 | 22 | 559 | 294 |
| Structured clone branch, including JSS-next checkpoint | 26,659 | 26,337 | 23 | 559 | 294 |

At `(path, variant)` granularity, 903 variants newly pass and **zero previously
passing variants are lost**. Relative to the JSS-next Temporal/Iterator
checkpoint on the same JS base, this insert adds 85 passing variants with zero
losses. The final classification SHA-256 is
`d967e1b651e329b585febb37fab85dbda2847d793e3e1015d827d371b67de33a`.
The one extra timed-out file was not reclassified as a pass or unsupported.
Linux `amazing-js` nextest passed 297/297, with one skipped; its doctest run
passed.

Pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`:
`html/webappapis/structured-clone/structured-clone.any.js` passed **112/152**
subtests, with 40 failures, no harness error, and no subtest timeout. The two
other source tests in this directory classified as one timeout (cross-realm
window method) and one harness error (detached-window script). Raw runner
records are in `js-stdlib-sc-wpt-battery.json`, `js-stdlib-sc-wpt-cross-realm.json`,
and `js-stdlib-sc-wpt-detached-window.json` beside this file. Most battery
failures concern platform objects that need the host hook (Blob, File,
FileList, ImageBitmap, MessagePort, OffscreenCanvas); Object.prototype and
SharedArrayBuffer cases remain measured failures.

IPC and storage integration signatures were posted in their lane questions.
The exact-commit Windows workspace gate is recorded in the lane report.

## JSS-next String core semantics

Fixed base: local `dev` `a0105c17` (`lane/js-stdlib` head `712927c0`). Pinned
Test262 revision `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`, full-input SHA-256
`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`.

Spec-exact String core methods that blocked the String directory:
`split` (`@@split` dispatch, `ToUint32(limit)` ordering, RegExp split via the
internal engine, string `SplitMatch`, empty-separator/source cases), `matchAll`
(RegExp match records with empty-match advance, `@@matchAll` dispatch, IsRegExp
and global validation before `ToString(this)`), `replaceAll` (proper
`IsRegExp(@@match)`, flags/global checks before `ToString(this)`, `@@replace`
dispatch), `replace` (`@@replace` dispatch for object search values only, `$`
substitution, callback `(match, offset, string)`), `includes`/`startsWith`/
`endsWith` (position/endPosition clamping, IsRegExp TypeError), `charAt`/
`charCodeAt`/`codePointAt` (out-of-range results), `trim` (full ECMAScript
WhiteSpace+LineTerminator set), String exotic objects (own `length` and
per-code-unit index properties) and `Object.prototype.toString` boxed tags.

| built-ins/String | Pass | Fail | Unsupported |
| --- | ---: | ---: | ---: |
| Base | 1,154 | 178 | 9 |
| After | 1,278 | 54 | 9 |

At `(path, variant)` granularity, **248 variants newly pass and zero previously
passing variants are lost**. Final selection classification SHA-256 is
`8779bf57d4b600afa1aab51064e2839d3222b500210898732101dceb7d4d75f1`. Linux
`amazing-js` nextest passed 317/317 with one skipped; its doctest passed 1/1.
Remaining failures are tagged templates (`String.raw` tag), RegExp engine gaps
(Unicode property escapes, named captures, `d` flag), Unicode normalization
tables, `String.prototype[Symbol.iterator]` receiver checking (in
`interp_symbol.rs`, outside this lane's grant) and global-variable shadowing in
`String(this)`.

## JSS-next Temporal annotation and Instant coercion

ISO string annotations for `Instant` and `ZonedDateTime` are now validated
syntactically while remaining semantically opaque for `Instant`: multiple time
zone annotations, repeated critical calendar annotations, uppercase unknown
keys, critical unknown annotations, offset annotations with seconds, and
trailing junk are RangeErrors, but `Instant` accepts and ignores any zone or
calendar name. `ToTemporalInstant` accepts a `ZonedDateTime` instance and
coerces other objects through `ToString`. `Instant.prototype.round` rejects
day/week/year/month units. `Duration.prototype.round` accepts year/month units
for an exactly blank duration and keeps the relativeTo requirement otherwise.

| built-ins/Temporal | Pass | Fail |
| --- | ---: | ---: |
| Base | 3,698 | 905 |
| After | 3,745 | 858 |

At `(path, variant)` granularity, **94 variants newly pass and zero previously
passing variants are lost**; `built-ins/String` is unchanged at 1,278/1,341 with
zero losses. Final selection classification SHA-256 is
`a6e0af52a317083041bcc7ffa2b98d8037054472cbfeac19c72fba43be2bd905`.

The largest remaining Temporal blocker is the absent `Proxy` (about 250 failing
variants, 125 files); it is `js-runtime`'s and recorded in this lane's
`questions.md`, along with tagged templates, RegExp engine gaps and
`String.prototype[Symbol.iterator]`. The rest are calendar/time-zone arithmetic
beyond the ISO surface and shared engine semantics, all left visible as
measured failures.

## JSS-next String symbol dispatch and RegExp prototype methods

`String.prototype.matchAll` now always builds `RegExpCreate(searchValue, "g")`
and reads `@@matchAll` from that copy; `match`/`search` dispatch `@@match`/
`@@search` on the argument and on the internally created RegExp;
`replaceAll` reads `@@replace` before coercing this/replaceValue and coerces
`ToString(replaceValue)` exactly once; `split` reads `@@split` only for object
separators and reports length 2; `substring` defaults an undefined end to the
length. `RegExp.prototype[Symbol.match/search/matchAll/replace/split]` are
installed, delegating to the internal regexp engine, with SpeciesConstructor
validation for `@@matchAll`.

built-ins/String: 1,278 -> 1,307 / 1,341 pass (+58 variants, zero lost;
classification SHA-256
`b3e0e8a7fbb41d1d554b8d1c1a73e6051bdabd28cbdb155b08a03bcfb91f86bd`).
built-ins/RegExp: 1,031 -> 1,135 pass (+212 variants); six previously-passing
variants call a symbol method that used to be absent, so their vacuous
`TypeError` passes now reach RegExpExec details owned by js-runtime (recorded
in `questions.md`).

## JSS-next Temporal option validation

Temporal string precision validates `roundingMode` even when the call does not
round; `Instant.prototype.since`/`until` reject day/week/year/month
smallestUnit; plain-type `from` validates its options bag. A duplicated
`native_string` tail left by the dev merge was removed so `amazing-js`
compiles.

built-ins/Temporal: 3,745 -> 3,765 / 4,603 pass (+40 variants, zero lost;
classification SHA-256
`61a3fe6c127c7de1183be0d5383ff09f0e4201c8fae5cb65723b9f7cf16b5d8b`).

## Final Linux gate

Workspace `cargo nextest run --workspace --locked --no-fail-fast` passed
2,701/2,701 with 10 skipped; workspace doctests passed. String 1,307/1,341,
Temporal 3,765/4,603 and RegExp 1,135/1,941 were all measured on the lane
Boat sandbox before and after each group.


