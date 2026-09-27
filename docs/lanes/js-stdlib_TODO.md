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

## JSS-next: Error.prototype.stack, String/matchAll/replace fixes, Temporal ZonedDateTime string parsing

Fixed base `c2104209` (the prior JSS-next commit). Pinned Test262
`3655e7464de3d52643ecddd4b5f9f4f3e7f62398`.

**`Error.prototype.stack`** is a real pinned Test262 directory this revision
(`built-ins/Error/prototype/stack/`, proposal `error-stack-accessor`), not a
de-facto extension. Implemented the getter (`get Error.prototype.stack`:
TypeError on a non-object receiver, `undefined` when the receiver lacks an
`[[ErrorData]]` internal slot even if `Error.prototype` is on its chain, else
an implementation-defined header string) and the setter
(`SetterThatIgnoresPrototypeProperties`: TypeError on a non-object receiver
or a non-String value; TypeError when the receiver is `%Error.prototype%`
itself (SameValue against the home object); otherwise `CreateDataPropertyOrThrow`
when the receiver has no own `"stack"`, or an ordinary `[[Set]]` -- preserving
an existing data property's attributes, or invoking/rejecting an existing
accessor -- when it does). Added a stored `Interpreter::error_proto: Obj`
field (mirroring `map_proto`/`set_proto`) so the constructor's `"prototype"`
own property IS that object, giving the setter a stable identity for the
SameValue check. The accessor pair is installed via `symbol_method`
(non-constructible, no own `"prototype"`) rather than `Self::native`, fixing
`getter`/`setter-not-a-constructor.js`; `native_error_value`'s shared
`"message"` own-property install was changed from enumerable-by-default to
`{writable:true, enumerable:false, configurable:true}` per section 20.5.1.1, fixing
`instance-not-enumerable.js`. `built-ins/Error/prototype/stack` selection (35
files, 2 `host.createRealm`-unsupported): 0 -> 27 pass (the files existed but
nothing implemented the feature before this). 6 remaining fail: 5 need Proxy
(`ReferenceError: Proxy is not defined`) and 1 (`getter-foreign-new-target.js`)
needs `Reflect.construct` foreign-new-target `[[ErrorData]]` semantics --
both js-runtime's core-interpreter territory, tracked below.

**String failure-cluster fixes** (25 failing files -> 21): `String.prototype.matchAll`
no longer falls back to an internal match-record implementation when the
freshly `RegExpCreate`d copy's own `@@matchAll` has been deleted from
`RegExp.prototype` -- step 5's `Invoke` is unconditional and throws
`TypeError` for a non-callable method, matching
`matchAll/regexp-prototype-has-no-matchAll.js`. `String.prototype.replace`
now `ToString`s a non-callable `replaceValue` unconditionally before
searching (step 6), observable via a `toString` side effect even when the
search string is never found (`replace/replaceValue-evaluation-order*.js`).
`String.prototype[Symbol.iterator]` now `RequireObjectCoercible`s `this`
before `ToString` (`Symbol.iterator/this-val-non-obj-coercible.js`) -- this
one line lives in `interp_symbol.rs` (js-runtime's file; `native_string_iterator`
called a bare `to_js_string` with no coercibility check), fixed under the
lane's standing grant for shared gaps blocking its own directories.
`built-ins/String`: 1,307 -> 1,311/1,341 pass (+8 variants, zero lost).

**Temporal ZonedDateTime string parsing** (`builtins_temporal_epoch.rs`):
`Temporal.ZonedDateTime.from`/`compare`/`equals`/`since`/`until` etc. now
accept a date-time string whose zone comes only from a bracketed annotation
with no `Z` and no numeric offset (`"1970-01-01T00:00[+01:00]"` --
`OffsetBehaviour: wall` in the spec, resolved through disambiguation alone;
previously `std_epoch_parse` required a `Z` or offset unconditionally and
threw `RangeError: invalid Temporal instant string` for every annotated
ZonedDateTime string lacking one, a ~29-file cluster).

Added `std_epoch_zone_from_string` (`ParseTemporalTimeZoneString`) so a
`timeZone` option value that is itself a full ISO date-time string --
optionally with a bracketed annotation -- resolves to a zone id instead of
being handed whole to `tz_canonicalize` (which only parses a bare
identifier). The date-time is always parsed and validated in full even when
an annotation ultimately supplies the zone (an early version skipped date
validation whenever a bracket was present, regressing the
`timezone-string-year-zero.js` negative-zero-year check -- caught and fixed
before this gate). A bare numeric offset with no annotation is only valid at
minute precision as a *time zone identifier* (`TimeZoneNumericUTCOffset`),
unlike the *exact time* offset in an Instant/ZonedDateTime string, which may
carry seconds -- `timezone-string-datetime.js`'s sub-minute-offset rejections.

Fixed-base full pinned Test262: **32,056 -> 32,147 pass** (files), **+181
passing variants, zero lost** (identical input SHA-256
`91109b8248e3825d658903f2496535b9500c8a776794355da7f32b6942ceddc5`).
`built-ins/Temporal` selection unchanged by the last two fixes at
3,813/4,603 pass (+48 files over this ticket's base). `built-ins/String`
selection unchanged at 1,311/1,341 pass. `built-ins/Error/prototype/stack`
selection: 27/35 pass (see above). Linux `amazing-js` crate nextest:
343/343 passed, 1 skipped; its doctest 1/1 passed -- re-verified after the
`symbol_method`/message-enumerable fixes.

Commit `dcfcd9d6` was then merged with `origin/dev` (commit `04b835de`) at
the orchestrator's request, to pick up a Boat sandbox SSH-endpoint fallback
fix needed to run the gate at all. The merge conflicted in
`interp_install.rs` (Error.isError install: kept this lane's `stack`
accessor block, took dev's `runtime_method_attributes`-based isError
install) and `interp_builtins.rs` (`native_error_value`: took dev's
`runtime_error_object`/`runtime_install_error_cause` refactor wholesale, since
it already installs `"message"` via `symbol_data(..., true, true)` --
`{enumerable:false}` -- superseding this ticket's hand-rolled fix with the
same result plus §20.5.7.1 `cause` support; `Object.prototype.toString`'s
exotic-tag chain: merged both sides' checks, since dev's added `[[DateValue]]`
-> "Date" but silently dropped this ticket's `[[BigIntData]]` -> "BigInt"
check, which is still exercised elsewhere in the crate). The merge also
surfaced one build break outside those two conflicts: dev's own Error-family
refactor (commit `05262079`, js-runtime's) had independently added a second,
weaker `native_error_is_error` (no `host_error_brand` hook) in the new
`interp_errors.rs`, name-colliding with this ticket's fuller one in
`interp_builtins.rs`; removed the weaker duplicate (fixup commit `a7fa4a92`)
since nothing referenced it and the installed one already subsumes it
(`Exotic::Error` OR `host_error_brand`).

Post-merge (`a7fa4a92`) re-verification on the same Boat sandbox: Linux
workspace `cargo nextest run --workspace --locked --no-fail-fast`: **2,934/2,934
passed** (1 slow, 11 skipped); `cargo test --workspace --locked --doc`: all
green, including `amazing_js` and `amazing_html`. Targeted directories
improved slightly from dev's unrelated changes, confirming zero loss:
`built-ins/Error/prototype/stack` 27 -> **29/35** pass (the 2
`host.createRealm`-unsupported cases are gone, now otherwise resolved);
`built-ins/Temporal` 3,813 -> **3,815/4,603** pass; `built-ins/String` 1,311
-> **1,315/1,341** pass.

Remaining `built-ins/Error/prototype/stack` gaps (5 Proxy-dependent files, 1
`getter-foreign-new-target.js` needing `Reflect.construct` foreign-new-target
`[[ErrorData]]` semantics) and the largest Temporal cluster (`ReferenceError:
Proxy is not defined`, ~125 files) stay js-runtime's, per `questions.md`.
String's remaining ~21 failures are mostly Unicode NFC/NFD normalization edge
cases (no external Unicode data crate available) and RegExp `/v` flag /
named-group parsing (js-lang's regex engine), left as measured failures
rather than chased for a small tail. `JsObject::error_obj` (`value.rs`,
internal engine-thrown errors) still installs `"message"` as enumerable,
unlike the constructor path fixed here; flagged in `questions.md` as a
deferred parity follow-up for whoever owns `value.rs`.

## JSS-fix: RegExpExec dispatch for @@match/@@replace/String.prototype.match

Fixed base: `93ed3aa8` (dev's merge of JSS-next). Priority regression fix
requested by the orchestrator ahead of the Temporal ticket below: dev's full
Test262 run showed +722 gained but 3 RegExp files newly failing relative to
the pre-merge baseline (`built-ins/RegExp/prototype/Symbol.match/{builtin-
success-g-set-lastindex-err,g-match-empty-set-lastindex-err}.js`,
`built-ins/RegExp/prototype/Symbol.replace/result-coerce-groups-err.js`).

Root cause: `RegExp.prototype[@@match]`'s global loop, `RegExp.prototype
[@@replace]`, and `String.prototype.match` all matched through the internal
`amazing_regex` matcher directly, bypassing spec `RegExpExec(R, S)`'s
`Get(R, "exec")` + `Call` dispatch entirely -- so an overridden `exec`, a
non-writable `lastIndex` set mid-match, or a `groups`/`0` getter with a side
effect never took effect. Added `regexp_exec_dispatch`/`regexp_exec_call`
(the spec `RegExpExec` algorithm, `interp_regexp.rs`) and rewired all three
call sites through it, plus a `groups`-`ToObject` coercion step in
`@@replace`'s string-substitution branch (the actual fix for
`result-coerce-groups-err.js`, reusing `std_legacy_to_object` from
`builtins_std_object_model.rs`, which already throws `TypeError` for
null/undefined and boxes primitives).

Since `RegExp.prototype.exec` is installed by default and essentially never
overridden in practice, both hot loops special-case the unmodified-`exec`
case (`regexp_exec_default`, a `std::ptr::fn_addr_eq` check against the
known native function) to skip a JS result-object round trip per match --
added after an initial version of this fix looked like it introduced 6
`CharacterClassEscapes` timeouts; `git stash`-based comparison against the
unmodified tree proved those were pre-existing, not a regression, but the
fast path removes the theoretical per-match overhead risk anyway. A real,
self-introduced 7-file `String`/`replace` regression (a captures-indexing
off-by-one: slot 0 must be the whole match per `regexp_substitution`'s and
`amazing_regex::Match.captures`'s existing convention, not left for capture
group 1, which caused both a Rust panic and wrong `$1` substitution) was
caught via before/after `.jsonl` diffing and fixed before landing.

All 3 target regressions now pass. `built-ins/RegExp`: 1,144 -> **1,176 /
1,962** pass (+32 files, 0 lost). `built-ins/String`: **1,315 / 1,341** pass,
unaffected (0 lost). `amazing-js` crate nextest 380/380 pass (1 skipped),
doctest 1/1. Workspace: `cargo nextest run --workspace --locked
--no-fail-fast` 3,105/3,105 passed (13 skipped); `cargo test --workspace
--locked --doc` all green. Committed `57dcf24b`.

## JSS-Temporal: Duration.compare relativeTo/options + string-form range checks

Fixed base: `57dcf24b`. Continuing the largest remaining js-stdlib Test262
cluster (Temporal), in bulk, skipping Proxy-dependent files (js-runtime's),
with zero lost passes, per the orchestrator's standing instruction.

Fixed three spec gaps in `Temporal.Duration.compare` (`builtins_temporal.rs`,
`std_temporal_compare`):

- `GetOptionsObject`/`ToRelativeTemporalObject` (spec steps 2-3) only ran on
  the calendar-units branch, so a non-object `options` argument or a
  malformed `relativeTo` string/property bag never threw when the two
  duration arguments happened to be field-wise identical. Restructured so
  `options`/`relativeTo` are read and parsed unconditionally first (throwing
  as needed), and the identical-duration equality check (step 4) now only
  skips the calendar-required `RangeError` and the day-based total
  computation, not the validation that must precede it.
- The "calendar units require relativeTo" check compared `left[0..2]`
  (years/months only), missing weeks at index 2 -- a week-only duration
  compared with no `relativeTo` should also throw; extended to `left[0..3]`.
- `std_temporal_parse`'s `Duration` branch returned raw parsed fields
  without running `std_temporal_validate`, unlike the property-bag path
  (`std_temporal_bag`, which already validates unconditionally), so a
  string-form `"P4294967296Y"` or an ISO duration string mixing a positive
  and negative field skipped the sign/range check entirely. Added the
  missing validate call.

A first attempt at the first fix introduced a 2-file regression
(`options-wrong-type.js`, `relativeto-no-fractional-minutes-hours.js`) by
short-circuiting on `left == right` before `options`/`relativeTo` were read
at all; caught via a full before/after `.jsonl` fail-path diff against
`js-stdlib-temporal-baseline2.jsonl` and fixed by hoisting the read+parse
above the equality check.

`built-ins/Temporal`: 3,815 -> **3,820 / 4,603** pass (+5 files / +10
variants), zero lost. Fixed: `Duration/compare/{calendar-possibly-required,
instances-identical, relativeto-propertybag-invalid, relativeto-string-
invalid, relativeto-undefined-throw-on-calendar-units}.js` (all variants).
Workspace: `cargo nextest run --workspace --locked --no-fail-fast`
3,105/3,105 passed (13 skipped, 0 failed); `cargo test --workspace --locked
--doc` all green, including `amazing_js` (1/1) and `amazing_html` (1/1).
Committed `20159ad5`.

Did not extend the same weeks-range fix to the analogous checks in
`builtins_temporal_arithmetic.rs` (`std_temporal_add_values`,
`std_temporal_duration_total`, `std_temporal_round`): no failing test
evidenced the same bug there, and weeks in simple arithmetic balancing are a
fixed 7-day unit needing no calendar, unlike `compare`'s relative-day
computation.

Remaining non-Proxy Temporal clusters, none started: `Duration.prototype
.round`'s `relativeTo`/calendar-unit balancing (~38 files -- needs a real
`RoundDuration`/`BalanceDateDurationRelative` implementation, the largest
single cluster and the most effort); `ZonedDateTime.prototype.{since,until}`
with `smallestUnit`/`roundingMode` (~24 files) and `PlainDateTime.prototype
.{since,until}` with `largestUnit` (~16 files), which likely share the same
difference/rounding gap as `round()`; "duration field must be an integer" /
`float64`-representable-integer precision-exact arithmetic (~23 files);
`PlainDate`/`PlainTime` ISO string validation edge cases ("invalid ISO
date"/"invalid ISO time", ~38 files combined); and a ~98-variant mixed
"Expected RangeError but no exception thrown" bucket. The `round()`/
since/until rounding cluster is flagged as the next ticket -- it is the
largest lever left in Temporal but is a substantial dedicated piece of work,
not a quick follow-on to this commit.

## JSS-round: Duration.prototype.round relativeTo/calendar-unit balancing

Implemented a real `RoundDuration`/`BalanceDateDurationRelative` for
`Duration.prototype.round` (`builtins_temporal_arithmetic.rs`,
`std_temporal_round_duration`, rewritten from scratch) and folded in the
`ZonedDateTime.prototype.{since,until}` (`builtins_temporal_epoch.rs`,
`std_epoch_difference`) and `PlainDateTime.prototype.{since,until}`
(`builtins_temporal_arithmetic.rs`, `std_temporal_difference`) rounding
clusters, which shared the same underlying gap.

New machinery in `builtins_temporal_arithmetic.rs`:
- `std_temporal_unit_rank`/`std_temporal_unit_name`/`std_temporal_larger_unit`/
  `std_temporal_singular_unit`: a year=0..nanosecond=9 rank matching the
  Duration field-index convention, used throughout for unit defaulting and
  coarseness comparisons.
- `std_temporal_epoch_breakdown(base, target_ns, largest)`: exact
  calendar+time rebalance of an epoch-ns span from `base` to a Duration at a
  given largestUnit granularity. Contains a "day borrow" step for when the
  calendar date-difference and the sub-day time remainder disagree in sign
  (e.g. base + 2 days - 1 hour must borrow a day from the date part and hand
  it to the time part so both end up sign-consistent).
- `std_temporal_calendar_step_ns`/`std_temporal_round_calendar_unit`: a
  bracket search for rounding to a variable-length calendar unit (year/month
  have no fixed ns length), computing real calendar arithmetic for candidate
  integer counts and applying floor/ceil/trunc/expand/half* logic within the
  bracket that contains the true value.
- `std_temporal_round_to_calendar_unit(base, target_ns, smallest, largest, ...)`:
  the hierarchical fix for `largestUnit != smallestUnit` -- rounding "the
  whole span in smallestUnit's own units from base" is wrong in that case
  (e.g. a span that's exactly 1 month but not a whole number of weeks must
  not spuriously pick up a week); it gets the exact breakdown at `largest`
  granularity first, then rounds only the sub-`largest` remainder to
  `smallest` granularity.

Also fixed along the way: `std_epoch_difference`'s default largestUnit
ignored an explicit/coarser `smallestUnit` (a `smallestUnit: "year"` with no
`largestUnit` must default `largestUnit` to "year" too, not the kind's usual
"hour"/"second"); `std_epoch_difference`'s calendar branch was missing
`"week"` and only did an exact `calendar_difference`, never true rounding.

Two intermediate regressions surfaced and were fixed before landing:
- A first pass at the `relativeTo`-string offset/zone-consistency check (for
  `relativeto-string-wrong-offset.js` /
  `throws-on-wrong-offset-for-zoned-date-time-relative-to.js`) was added
  directly inside the general `std_temporal_parse`, which also validates
  every plain-type constructor/parsing call -- but a mismatched offset must
  be *ignored* there (only `ToRelativeTemporalObject`'s string-relativeTo
  resolution needs the strict check), so it broke ~53 previously-passing
  files across `PlainDate`/`PlainDateTime`/`PlainMonthDay`/`PlainTime`/
  `PlainYearMonth`/`ZonedDateTime` parsing (`argument-string-date-with-utc-
  offset.js`, `argument-string-time-zone-annotation.js`,
  `PlainDateTime/from/parser.js` and more). Reverted the general check and
  reimplemented it as a new, narrowly-scoped
  `std_temporal_validate_relative_to_offset` helper
  (`builtins_temporal.rs`), called only from `std_temporal_round_duration`
  when `relativeTo` is itself a string.
- `std_temporal_add_calendar`/`std_temporal_epoch_breakdown` only ever
  rejected an i64 day-count overflow, not Temporal's actual (much narrower)
  ISO date range, so a duration with an enormous time component (e.g.
  `Number.MIN_SAFE_INTEGER` seconds) rounded to a calendar unit relative to a
  plain date silently bracket-searched to a wildly out-of-range civil date
  instead of throwing. Added `std_temporal_validate(PlainDate, ...)` calls at
  the two points that compute an intermediate civil date from an arbitrary
  epoch-ns offset (`std_temporal_calendar_step_ns`'s bracket-search candidate,
  and `std_temporal_epoch_breakdown`'s adjusted target before calendar-
  difference) -- this is what `Duration/prototype/round/relativeto-
  {plaindate,zoneddatetime}-large-time-component-out-of-range.js` need.

`built-ins/Temporal`: **7,640 -> 7,820 pass variants (+180), zero lost**,
diffed by (path, variant) against `js-stdlib-temporal2.jsonl` (the
post-JSS-Temporal baseline; counts are strict+non-strict variants, not
files -- 4,603 files discovered, 3,908 pass in strict mode alone in the
final run). Workspace: `cargo nextest run
--workspace --locked --no-fail-fast` 3,267/3,267 passed (14 skipped, 0
failed); `cargo test --workspace --locked --doc` all green, including
`amazing_js` (1/1) and `amazing_html` (1/1). Base: local `dev` at `4f27b9c4`
(the merge that landed JSS-Temporal) -- `dev` has since advanced to
`9e1f1aaf`, but the only lane-owned drift since `4f27b9c4`
(`builtins_std_object_model.rs`, `builtins_std_reflect.rs`, from JSRT-15)
does not touch any file this ticket changed, so no rebase was needed per the
"do not chase dev" rule.

Remaining Temporal clusters, none started this round: "duration field must
be an integer" / `float64`-precision-exact arithmetic (~23 files);
`PlainDate`/`PlainTime` ISO string validation edge cases (~38 files
combined); the mixed "Expected RangeError but no exception thrown" bucket;
`Duration.prototype.total`'s own `relativeTo`-string offset/zone-consistency
gap (same class of bug as the round() fix above, but out of this ticket's
scope -- `Duration/compare` and `Duration/prototype/total`'s
`relativeto-string-wrong-offset.js`-style tests are still failing, unchanged
from before this ticket).

## JSS-total: `.from()`/`.with()` overflow regulation, ZonedDateTime.round, toLocaleString

Fixed base: local `dev` `df152f3c` (JSS-round `72dbd5c6` already merged).

Applied the JSS-round `relativeTo` timeZone/offset-consistency machinery to
`Duration.prototype.total` and `Duration.compare` (both now resolve
`relativeTo` unconditionally, matching `round()`'s existing pattern).

Rewrote `ZonedDateTime.prototype.round`: Instant's day-adjacent increments
(hour:24, minute:1440, second:86400) use the simple "evenly divides a fixed
86400s solar day" rule, but `ZonedDateTime`'s "day" unit needs GetStartOfDay-
style variable-length-day bracketing (`std_epoch_start_of_day_ns`, new) to
handle DST, and its sub-day units round the local wall-clock time-of-day
(always non-negative) rather than raw, possibly hugely negative, epoch
nanoseconds. Rewrote `ZonedDateTime.prototype.with` similarly: calendar/
timeZone rejection, alphabetical `PrepareTemporalFields`-order field reads
with a two-tier validation model (unconditional lower bound vs. overflow-
dependent upper bound), deferred monthCode resolution, and full offset-option
(`prefer`/`use`/`ignore`/`reject`) resolution against the real timezone
database (`builtins_intl_timezone.rs`).

Root cause for most of the remaining `PlainYearMonth`/`PlainMonthDay` (and,
it turned out, `PlainDate`/`PlainDateTime`/`PlainTime`) `.from()` "overflow"/
"limits" failures: `std_temporal_from` computed the overflow option's
boolean purely to validate its type, then discarded it -- the object-bag
path always ran the unconditional (reject-only) ISO validity check, so
`overflow: "constrain"` never actually constrained an out-of-range month or
day. Split `std_temporal_bag` into raw field extraction plus a new
`std_temporal_regulate(kind, fields, reject)` (RegulateISODate/
RegulateISOYearMonth/RegulateISOMonthDay/RegulateTime), applied once
overflow is known. Threaded a `reject` parameter through
`std_temporal_from_fields` to all 13 call sites (constrain by default,
matching `ToTemporalDate`'s un-forwarded-options behavior -- only `.from()`
itself reads the real option). `.from()`'s three input shapes have three
different, spec-fixed, Proxy-observable read orders, confirmed against
`options-wrong-type.js`/`observable-get-overflow-argument-string-invalid.js`:
an object bag reads its own fields before options is touched; a string is
parsed before options is touched; anything already a Temporal record or
ZonedDateTime validates options before converting. Also added the
unconditional (overflow-independent) rejection of month/day < 1 -- confirmed
against `out-of-range.js`/`negative-month-or-day.js`, this is
`ToPositiveIntegerWithTruncation`, not `RegulateISODate`'s clamp.

The same overflow-discard gap, plus a missing
`RejectObjectWithCalendarOrTimeZone` check and wrong (non-alphabetical)
field read order, affected the shared `std_temporal_with` used by
`PlainDate`/`PlainDateTime`/`PlainTime`/`PlainYearMonth`/
`PlainMonthDay.prototype.with()`. Rewrote it to reject a bag that is itself
any recognized Temporal object (without touching its properties -- they may
be throwing getters on a real instance), or that carries a `calendar`/
`timeZone` property, before reading anything else; read fields in
`PrepareCalendarFields`' per-kind alphabetical order (`PlainYearMonth`/
`PlainMonthDay` now correctly skip fields outside their own `fieldNames`,
e.g. `PlainYearMonth.prototype.with` no longer reads "day" at all); defer
monthCode's calendar-format validation until after options is read; and
give `PlainMonthDay`'s `with` its own regulation path so a supplied `year`
can decide Feb-29 constrain/reject without itself being range-checked (it's
never stored -- the result's reference year is always 1972, per
`iso-year-used-only-for-overflow.js`).

Also fixed along the way: `ZonedDateTime.from`'s own bag path gained the
same overflow-option support it was previously missing entirely; and
`toLocaleString` on `Instant`/`ZonedDateTime` no longer misinterprets its
`(locale, options)` arguments as `toString`'s own (`calendarName`/`offset`/
`timeZoneName`) options bag (it now has its own trivial pass-through
matching the Plain* types' existing `std_temporal_to_locale_string`
pattern).

`built-ins/Temporal`: **7,862 -> 8,086 pass variants (+224), zero lost**,
diffed by (path, variant) against `js-stdlib-total-baseline.jsonl` (the
JSS-round-merged baseline on this host). Workspace: `cargo nextest run
--workspace --locked --no-fail-fast` 3,298/3,298 passed (16 skipped, 0
failed); `cargo test --workspace --locked --doc` all green, including
`amazing_js`, `amazing_html`, `amazing_common` (1/1 each). Committed
`4c759ccf`.

Remaining Temporal clusters, none started this round (largest first):
`Duration.prototype.total`/`toString`/`round`/`compare`/`add`/`subtract`
(~136 files combined -- distinct bugs from the relativeTo gap fixed this
round and JSS-round, likely rounding-mode/formatting-precision issues, not
yet triaged); `PlainDate`/`PlainDateTime.prototype.{since,until}` (~76 files
combined); `ZonedDateTime.prototype.{from,since,until,equals,add,subtract}`
(~83 files combined); `PlainDate.prototype.toZonedDateTime` (~17 files);
`Instant.prototype.toString` (~11 files); `PlainDateTime/PlainMonthDay/
PlainYearMonth.prototype.from` (~27 files, likely the remaining
`observable-get-overflow-argument-primitive.js`/`order-of-operations.js`-
style Proxy property-order tests, a deliberate diminishing-returns tradeoff
this round, same as ZonedDateTime.with's 2 residual order tests);
`PlainDateTime.prototype.with/throws-if-combined-date-time-outside-valid-
iso-range.js` (1 known-remaining edge case -- a combined date+time ISO
range check at nanosecond precision that `std_temporal_validate` doesn't
do today, low value for its investigation cost).

## JSS-duration: Duration total/toString/round/compare/add/subtract rounding, formatting and range-check bugs

Fixed base: `fdda620f` (dev, with JSS-total merged).

Four separate bugs, found by triaging the ~136-file cluster the previous
round surveyed but didn't start.

**`Duration.prototype.toString()` had no real formatting algorithm.** The
old `std_temporal_duration_text` ignored `smallestUnit`/`fractionalSecondDigits`/
`roundingMode` entirely and used an approximate "show seconds" heuristic.
Replaced with `std_temporal_duration_to_string`, implementing
`TemporalDurationToString`'s two mechanisms: (A) when a precision was
explicitly requested (`step.is_some()`), sum exactly the fields at rank
`balance_rank..=9` (nanoseconds) -- `balance_rank` is the highest-ranked
nonzero field, clamped to at least "day" (years/months/weeks can't
redistribute without a `relativeTo`, confirmed by `round-cross-unit-
boundary.js`'s 1Y11M30D23:59:59.999999999 rounding up into 31D, never into
the months) -- round that sum to the requested increment/mode, then re-split
starting at `balance_rank` only; everything coarser is left untouched
(confirmed by `fractionalseconddigits-number.js`, where weeks/days stay put
while only the seconds fraction rounds, and by the "no balancing if
smallest unit is largest unit" case, where a bare `seconds` field rounds
past 60 without promoting into minutes). (B) Otherwise, only a mandatory
ns -> us -> ms -> seconds overflow cascade runs; minutes/hours/days are
never touched. Also fixed `std_temporal_string_precision`'s
`fractionalSecondDigits` reader: it rejected any non-integer Number instead
of flooring it (`GetStringOrNumberOption` step 3.b), and called `ToNumber`
unconditionally instead of dispatching on the *actual* type of the option
value (only a primitive Number takes the `ToNumber` branch; boolean, null,
bigint, object and Symbol all go through `ToString` instead, confirmed by
`fractionalseconddigits-wrong-type.js`). Gained an `allow_minute: bool`
parameter (Duration's `smallestUnit` excludes "minute"/"hour", unlike
`PlainTime`/`Instant`/`ZonedDateTime`). Last fix: `Duration.prototype.
toString` doesn't have a `calendarName` option at all (a Duration carries
no calendar) -- `std_temporal_to_string` was still reading it for every
kind including Duration, an observable-order bug caught by
`order-of-operations.js`.

**The Duration validity check had the wrong bounds for the day-through-
nanosecond fields.** `std_temporal_validate` capped `days` together with
years/months/weeks at 2^32-1, and hours-through-nanoseconds each
individually at 2^53-1. Both are wrong: days is unrelated to the calendar
fields and has no fixed cap of its own (`Duration/max.js`'s "max days" case
is ~1.04e11 days, nowhere near 2^32); the real constraint (`IsValidDuration`)
is that the *exact total* of days..nanoseconds, expressed in seconds, must
fit the float64-safe-integer range (with up to .999999999 of a further
second, since nanosecond precision is kept exactly) -- confirmed by
`Duration/out-of-range.js`'s thresholds being derived from
`ceil(2**53-1 / unit-in-seconds)`, not from each field's own magnitude.
Added that total-nanoseconds bound (`STD_TEMPORAL_DURATION_MAX_NS`)
alongside (not instead of -- reverting the per-field cap regressed 2
already-passing tests, `round/out-of-range-when-converting-from-normalized-
duration.js`, where the ENTIRE span collapses into a single requested
`largestUnit` and that one surviving field must independently be
representable even though the total-in-seconds is still in range) the
existing per-field cap on hours..nanoseconds, with `days` now excluded from
both caps and governed by the total only.

**`Duration.prototype.add`/`subtract` neither rejected calendar units nor
ever balanced the result.** Per `AddDurations`, a nonzero year/month/week on
*either* operand always throws (there's no `relativeTo` to redistribute
through) -- unconditionally, regardless of whether the raw sum would still
be in range; the old code only checked calendar units as a fallback *after*
attempting the addition, so e.g. `new Temporal.Duration(1).add(blank)`
silently succeeded instead of throwing (`no-calendar-units.js`). Separately,
the day-through-nanosecond result was only ever rebalanced (nanosecond-exact
sum, re-split from a computed largest rank) as an out-of-range *fallback* --
never unconditionally -- so e.g. adding a duration to itself where
milliseconds overflowed past 1000 came back with `milliseconds: 1000`
instead of carrying into seconds, even though the raw per-field sum was
itself safe-integer-range (`basic.js`'s "balancing positive" case,
`balance-negative-result.js`). Rewrote the Duration arm of
`std_temporal_add_values` to always throw on any calendar unit first, then
always fully rebalance via exact nanosecond total (the surviving largest
rank is the coarser -- i.e. smaller rank number -- of the two operands' own
largest nonzero field, clamped to at least "day"), then validate.

`built-ins/Temporal`: **8,086 -> 8,218 pass variants (+132), zero lost**,
diffed by (path, variant) against a fresh `js-stdlib-duration-baseline.jsonl`
taken at this round's start. `Duration.prototype.toString`: 0 failing files
remaining (was ~25). `Duration.prototype.add`/`subtract`: 6+6 failing files
remaining each (was 13+12) -- all in the float64-representable-integer /
precision-exact-mathematical-values / result-out-of-range sub-cluster, which
requires replicating `BalanceDuration`'s literal float64 (ℝ(𝔽(x))) rounding
at specific intermediate steps rather than exact bigint math (confirmed by
`add/float64-representable-integer.js`'s explicit "should not use more
precise internal representation than the spec prescribes" assertion) --
not attempted this round, pre-existing failures, not regressions.
`Duration.from`/`compare`: `argument-duration-max.js`/`-out-of-range.js` and
`Duration/max.js`/`out-of-range.js` fixed by the validate-bounds fix above.
Workspace and doctest gate: see `report.md` for this round's numbers.

Remaining Temporal clusters, none started or only partially covered this
round (largest first): `Duration.prototype.round`/`total`'s `relativeTo`
handling (~21+29 files -- mostly `ZonedDateTime`-as-`relativeTo` property-
bag read-order and offset/timezone-string edge cases, e.g.
`relativeto-propertybag-timezone-string-datetime.js`, plus a handful of
`order-of-operations.js`/`options-read-before-algorithmic-validation.js`
tests requiring the full `GetTemporalRelativeToOption`+`ToRelativeTemporalObject`
Proxy-observable read order for a `ZonedDateTime`-shaped bag -- not
triaged in depth, likely substantial); `Duration.compare`'s remaining ~11
files (same `relativeTo` family); `PlainDate`/`PlainDateTime.prototype.
{since,until}` (~76 files, untouched -- no time remained after Duration);
the float64-exact-precision sub-cluster in `add`/`subtract`/`from`/`compare`
noted above (~6 files each).

## JSS-since-until: PlainDate/PlainDateTime since/until direction+rounding, Duration.total's calendar fraction, relativeTo weeks gate

Fixed base: local `dev` `0654b5d3`, fast-forwarded mid-ticket (no lane-owned
drift) to `2248f9bd`, which carries the orchestrator's new porting policy
(`AGENTS.md` "Porting from other engines", `THIRD_PARTY_NOTICES.md`). Per that
policy, this round ports (copies and adapts onto our own `[i64; 10]`
field/f64-free model, with attribution) two algorithms from
`boa-dev/temporal` (temporal_rs) `@ eadf8039d2577e4375493a0eb3f46a66f25616ec`
(Apache-2.0): the since/until direction+rounding-mode convention
(`src/builtins/core/plain_date.rs` `diff_date`) and `DifferenceISODate`'s
month/year bracket search (`src/iso.rs` `IsoDate::diff_iso_date`). Both are
recorded in `THIRD_PARTY_NOTICES.md` against
`crates/amazing-js/src/builtins_temporal_arithmetic.rs`, which also carries a
file-header attribution comment. No crate dependency was added.

**Root cause 1 (since/until direction and rounding mode, `std_temporal_difference`):**
every branch computed the exact delta self -> other (unrounded), rounded it
directly, and then negated the WHOLE thing for `since` -- which looks
plausible but is provably wrong for a directional roundingMode: rounding a
value and its negation with the same `ceil`/`floor`/`halfCeil`/`halfFloor`
does not generally negate (`ceil(-3) = -2`, but `-ceil(3) = -4`). Confirmed by
hand against `roundingmode-ceil.js` and traced against the actual ported
algorithm: the correct behavior is to always compute the exact delta as
receiver -> argument (never swapped), always bracket-search/round relative to
the RECEIVER (never the argument -- calendar units are variable-length, so
which endpoint anchors the month/year bracket is observable, not just a sign
convention; confirmed by `rounding-relative.js`, `round-relative-to-receiver.js`,
`roundingmode-half-boundary.js`), swap `ceil`<->`floor` and
`halfCeil`<->`halfFloor` (the only modes without a sign-symmetric
`mode(-x) = -mode(x)` identity) before that one rounding step when the
operation is `since`, and negate the fully-rounded result at the very end.
Added `std_temporal_swap_directional_mode`. Also fixed a related, independent
bug: an absent/`"auto"` `largestUnit` must default to the larger of the
operation's own default unit and `smallestUnit` (`GetDifferenceSettings`), not
always the operation's own default -- so `smallestUnit: "week"` with no
`largestUnit` resolves to `largestUnit: "week"`, not `"day"`, and does not
spuriously throw "largestUnit is smaller than smallestUnit"
(`smallestunit-plurals-accepted.js`).

**Root cause 2 (`std_temporal_calendar_difference`, the shared exact
year/month/day breakdown): the day-of-month clamp anchor was wrong.** The old
code swapped `first`/`second` to always anchor at whichever endpoint is
chronologically earlier and clamped month/year steps using THAT endpoint's
day-of-month. The ported `diff_iso_date` never swaps: it always anchors at
the literal `first` argument (walking in the `sign` direction) and always
clamps using `first`'s own day-of-month, so which argument is `first` is
observable in the day remainder whenever a month is shorter than `first`'s
day -- e.g. `2019-03-01` minus `2019-01-29` at `largestUnit: "month"` is
1 month + 3 days (correct, per `rounding-relative.js`), not 1 month + 1 day
(the old, wrong answer). Rewrote as a faithful port: an unclamped
`(year, month, day)` tuple comparison (`std_temporal_calendar_surpasses`,
matching the spec's `IsoDate::new_unchecked`-based compare exactly, including
at the day-31-vs-shorter-month boundary) drives the bracket search, and only
the FINAL result is clamped via the existing `std_temporal_add_calendar`. This
one fix alone resolved `basic-arithmetic.js`, `largestunit-higher-units.js`,
`leap-year-arithmetic.js`, and the `rounding-relative.js`/
`roundingmode-half-boundary.js` cluster for both `since` and `until` on both
`PlainDate` and `PlainDateTime`.

**Two regressions surfaced by the `calendar_difference` rewrite and fixed
before landing** (both in `std_temporal_round_to_calendar_unit`, used by
`Duration.prototype.round`'s `relativeTo` handling as well as since/until):
the `largestUnit == smallestUnit` branch and the `smallestUnit: "month"`/
`largestUnit: "year"` branch each re-derived their result by adding the
already-correct rounded count back onto `base` and then diffing that date
against `base` again through `calendar_difference` -- redundant, and lossy at
an end-of-month boundary (adding 11 months to a day-31 `base` lands on a
clamped day-30 date, and diffing that clamped date back against `base`'s own
unclamped day 31 reads one month short: `end-of-month-round-up.js`). Fixed by
constructing the result directly from the values the bracket search already
computed, with the month/year branch folding the combined month count back
into years using TRUNCATING (not Euclidean) division, so the sign matches the
overall span's own sign rather than always landing in `0..12`
(`round-cross-unit-boundary.js`'s cross-unit-carry cases, in both the positive
and negative direction).

**`Duration.prototype.total`'s year/month fraction divided by the wrong
unit's length.** It computed the exact span's total nanoseconds correctly,
then divided by the length of exactly ONE year/month starting at `relativeTo`
-- wrong whenever the span crosses into a differently-sized year/month, e.g.
1 year + 183 days counted from a non-leap year divides by that (365-day) first
year's length instead of the (366-day) leap year the 183 remaining days
actually fall inside, giving `1.5013698630136987` instead of the exact `1.5`
(`roundingmode-half-boundary.js`'s tie-break setup assertion, which needs an
exact `0.5` boundary to even be meaningful). Added
`std_temporal_total_calendar_unit`, the fractional counterpart of the
existing integer bracket search: find the two adjacent whole-unit-count
candidates that bracket the total, then interpolate linearly within that
SPECIFIC bracket's own length.

**`relativeTo` was not required often enough, in both `total` and `round`.**
Both gated only on the duration's own nonzero years/months and the requested
unit being year/month, missing weeks entirely on both sides: a week-only
duration always needs `relativeTo` (weeks are fixed-length in isolation, but
converting between a week-carrying duration and another unit is treated as
calendar-relative by spec), and requesting `unit`/`smallestUnit`/`largestUnit`
`"week"` requires `relativeTo` even for a duration with no calendar fields at
all (confirmed by `calendar-possibly-required.js`'s week-only case and
`relativeto-undefined-throw-on-calendar-units.js`'s full per-unit sweep over
every duration shape). `Duration.compare` already had this right (it already
checked `left[0..3]`, from the JSS-Temporal round); `total`/`round` did not.

`built-ins/Temporal`: **8,218 -> 8,400 pass variants (+182), zero lost**,
diffed by (path, variant) against a fresh `js-stdlib-sinceuntil-baseline.jsonl`
taken at this round's start. Final classification SHA-256 is
`e78d0849587f683b63436bcce4ca84eac09c2124bfe191a0a8e66eb674d44b00`. File-level,
before -> after:

| Selection | Before pass | After pass | Total files |
| --- | ---: | ---: | ---: |
| `PlainDate/prototype/since` | 61 | 84 | 87 |
| `PlainDate/prototype/until` | 60 | 83 | 86 |
| `PlainDateTime/prototype/since` | 73 | 88 | 95 |
| `PlainDateTime/prototype/until` | 74 | 90 | 98 |
| `Duration/prototype/total` | 49 | 57 | 78 |
| `Duration/prototype/round` | 105 | 107 | 126 |

Workspace: `cargo nextest run --workspace --locked --no-fail-fast`
**3,381/3,381 passed** (3 slow, 17 skipped, 0 failed); `cargo test --workspace
--locked --doc` all green.

Remaining `PlainDate`/`PlainDateTime` since/until failures (25 files) are a
different, cross-cutting bug affecting many Temporal methods beyond
since/until: `options-read-before-algorithmic-validation.js`/
`order-of-operations.js` need the exact spec read order (`largestUnit` ->
`roundingIncrement` -> `roundingMode` -> `smallestUnit`, all read and cast
BEFORE any validation can throw), but `std_temporal_rounding_options`
currently reads/validates smallestUnit's increment before largestUnit is even
read -- a `std_temporal_rounding_options`-level fix, not attempted this round
(it is shared by Duration.round/PlainTime.round/etc., so touches more than
since/until). The rest are unrelated: string-parsing edge cases
(`argument-string-invalid.js`, `argument-string-limits.js`), a
`float64-representable-integer.js`/`largestunit-plurals-accepted.js`
precision-exact sub-cluster (same class of bug noted for `add`/`subtract` in
JSS-duration), and `rounding-near-minimum-date.js`/`units-changed.js` (date
range edge cases). `Duration.prototype.round`/`total`'s remaining ~19+21
files are almost entirely `ZonedDateTime`-as-`relativeTo` property-bag/
timezone-string edge cases and the same options-read-order bug, not the
calendar-arithmetic bugs fixed this round. `Duration.prototype.round`/`total`/
`compare`'s `relativeTo` handling and the float64-exact `BalanceDuration`
sub-cluster (this ticket's stated next items) were only partially reached
(the two bugs shared with since/until above); no dedicated round started on
either this session.

## JSS-temporal-port, item 1: option-read-order fix + real since/until bugs (checkpoint, not fully closed)

Fixed base: `edca8055` (`localdev/dev`, fast-forwarded onto cleanly -- one
sibling-lane one-line drift in `builtins_std_collections.rs`, unrelated).
Pinned Test262: `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`.

Scope actually completed this session, in order:

1. **The cross-cutting option-read-order bug**, confirmed via the real
   `options-read-before-algorithmic-validation.js`/`order-of-operations.js`
   expectation lists: `largestUnit` must be read (and cast) first, then
   `roundingIncrement`, then `roundingMode`, then `smallestUnit`, and no
   algorithmic validation (unit-name-for-this-operation, `largestUnit` <
   `smallestUnit`, the `roundingIncrement` bound implied by the now-known
   `smallestUnit`) may throw until all four reads complete. The old
   `std_temporal_rounding_options` read smallest/increment/mode first and
   read `largestUnit` afterward, deep inside each per-kind branch. Replaced
   with `std_temporal_difference_reads` (raw four-tuple read, no validation)
   plus `std_temporal_resolve_largest`/`std_temporal_validate_rounding_
   increment` (per-kind default/validation, called after all four reads),
   in both `std_temporal_difference` (PlainDate/PlainDateTime/PlainTime/
   PlainYearMonth since/until, `builtins_temporal_arithmetic.rs`) and
   `std_epoch_difference` (Instant/ZonedDateTime since/until,
   `builtins_temporal_epoch.rs`). Fixed all four
   `options-read-before-algorithmic-validation.js` failures and all three
   `order-of-operations.js` failures (PlainDate/PlainDateTime/PlainTime).
   Ported ordering from boa-dev/temporal (temporal_rs)
   @ `eadf8039d2577e4375493a0eb3f46a66f25616ec`, `src/options.rs`'s
   `DifferenceSettings` parsing order, Apache-2.0 (same upstream/commit as
   JSS-since-until; no new `THIRD_PARTY_NOTICES.md` row needed).
2. **Found, while auditing `std_epoch_difference`, that ZonedDateTime's own
   calendar-unit since/until branch never applied the directional-
   roundingMode swap** (`ceil`/`floor`/`halfCeil`/`halfFloor`) that
   PlainDate/PlainDateTime's calendar branch already got from JSS-since-
   until -- fixed in the same edit (`std_temporal_swap_directional_mode`
   applied conditionally, only for the calendar branch; the fixed-unit
   branches correctly compute the signed delta directly per direction and
   need no swap, confirmed by re-deriving the math rather than assumed).
3. **The alphabetical-field-read-order bug** in `std_temporal_bag`
   (`builtins_temporal.rs`), the property-bag reader shared by every
   `from()`/constructor/since-until-`other`-argument conversion for
   PlainDate/PlainDateTime/PlainTime/PlainYearMonth/PlainMonthDay: it read
   `calendar, year, month, monthCode, day, [hour..nanosecond]` (a fixed
   natural order) instead of the alphabetical order
   PrepareTemporalFields/PrepareCalendarFields specifies (confirmed by the
   *real* assertion mismatch behind `order-of-operations.js`, after the
   read-order fix above turned out not to be sufficient by itself). `with()`
   already had this right, via its `StdWithField` enum and per-kind
   `(&str, StdWithField)` table; extracted that table into
   `std_temporal_field_table` and added `std_temporal_read_calendar_fields`
   (the shared alphabetical-read loop, each field's own immediate validation
   inlined, `monthCode` cross-check deferred), and rewrote `std_temporal_bag`
   to use them. Incidentally also fixed a spurious extra "day" property read
   for `PlainYearMonth.from()` (not in spec's field set for that kind, and
   not in `with()`'s existing table either).
4. **Duration validity's per-field float64-safe-integer cap was too
   strict for since/until's own output.** `std_temporal_validate`'s Duration
   branch rejected any hour-through-nanosecond field over 2**53-1, added in
   an earlier ticket to satisfy `Duration.prototype.round`'s own
   `round/out-of-range-when-converting-from-normalized-duration.js` --  but
   that is a narrower rule specific to `round()`'s BalanceTimeDuration-style
   conversion, not general `IsValidDuration` (confirmed by since/until's own
   `largestunit-plurals-accepted.js`/`units-changed.js`, which legitimately
   collapse a multi-year span into a single `nanoseconds` field well past
   2**53-1 while the *total*, in seconds, stays far inside
   `STD_TEMPORAL_DURATION_MAX_NS`). Removed the per-field cap from the
   general validator (keeping years/months/weeks' 2**32-1 cap and the
   total-based bound) and re-added the narrower field-level check locally in
   `std_temporal_round_duration`'s non-calendar fast path, where the
   round-conversion test actually lives. Verified both directions: the
   round-conversion test still passes (it was never lost against baseline)
   and the previously-blocked since/until tests now pass.
5. **A real rounding-carry bug, not an option/order issue:**
   `std_temporal_difference`'s PlainDateTime branch had an "exact, unrounded
   breakdown" shortcut whenever `largestUnit` was a calendar unit
   (year/month/week), taken unconditionally regardless of what `smallestUnit`
   actually was -- its own comment claimed `smallestUnit` was always
   `"nanosecond"` here ("the only case reaching here"), which was simply
   wrong: any fixed unit (day..nanosecond) reaches this branch once
   `largestUnit` is calendar-coarser. A real rounding request (e.g.
   `smallestUnit: "microseconds", roundingMode: "expand"`) was silently
   dropped, which matters exactly when the rounding carries across the
   `largestUnit` boundary (confirmed by `round-cross-unit-boundary.js`'s
   "rounding down 1 ns balances to -2 years" case, and this same fix
   resolved `rounding-near-minimum-date.js` as a side effect). Fixed by
   always rounding the exact receiver-argument delta to `smallestUnit`
   first, then re-expressing the rounded instant via
   `std_temporal_epoch_breakdown` at `largestUnit` -- an identity when
   `smallestUnit` is `"nanosecond"` with increment 1, so no separate
   unrounded-shortcut branch is needed at all.
6. **A discarded time part was never range-checked.** `std_temporal_parse`'s
   `PlainDate`/`PlainYearMonth`/`PlainMonthDay` branches parse (and then
   discard) a trailing time-of-day when the input string has one, but
   `std_temporal_parse_time` itself only checks digit shape/count, not
   range (hour <= 23, etc.) -- range checking is normally deferred to
   `std_temporal_validate` on the *kept* fields, which never sees the
   discarded ones. Added an explicit `std_temporal_validate(PlainTime, &
   ignored)` after each discard-parse. Fixes PlainDate's own since/until
   `argument-string-invalid.js` (`"2020-01-01T25:00:00"` must still throw).

Measured, `built-ins/Temporal` (fresh baseline taken at this ticket's start,
same fixed-selection classification hash `e78d0849587f683b63436bcce4ca84eac09c2124bfe191a0a8e66eb674d44b00`
as JSS-since-until's *end* state, confirming no drift touched this
selection): **8,400 -> 8,510 pass variants (+110)**, **4,200 -> 4,255 pass
files (+55)**, out of 9,206 variants / 4,603 files selected; **0 lost**
(diffed by `(path, variant)` against a fresh
`js-stdlib-temporalport-baseline.jsonl`).

Workspace: `cargo nextest run --workspace --locked --no-fail-fast`
**3,381/3,381 pass** (3 slow, 17 skipped, 0 failed, unchanged from
JSS-since-until's end state); `cargo test --workspace --locked --doc`: 31
crates, all green (`amazing_js`: 1 passed; `amazing_html`: 1 passed).

### What item 1 did NOT finish

A fresh survey after all the above puts PlainDate/PlainDateTime/PlainTime
since/until at **14 remaining failures** (down from 25, several of which
turned out to already be fixed by the field-order/rounding-carry fixes
above rather than needing individual attention):

- `PlainDate`: 2 (`argument-string-invalid.js` x since/until -- see below)
- `PlainDateTime`: 4 (`argument-string-limits.js`,
  `float64-representable-integer.js` x since/until)
- `PlainTime`: 8 (`argument-string-calendar-annotation.js`,
  `argument-string-minus-sign.js`,
  `argument-string-time-designator-required-for-disambiguation.js`,
  `year-zero.js` x since/until)

Two distinct root causes remain, neither attempted this round:

- **`float64-representable-integer.js`** (PlainDateTime only): the Duration
  getter already returns the correct float64-rounded value for a field past
  2**53-1 (`result.microseconds` itself passes), but `Duration.prototype.
  toString`/`.add`/`Duration.compare` read the internal exact `i64` value
  instead of first canonicalizing it through the same f64 rounding the
  getter applies -- needs an audit of every Duration-consuming operation to
  canonicalize each field through `as f64 as i64`-style rounding at read
  time, not just the getter. Cross-cutting, not since/until-specific.
- **PlainTime's parse path never validates (or even looks at) the *date*
  portion of a full date-time string it is about to discard the date half
  of.** `std_temporal_parse`'s `PlainTime` arm locates the substring after
  `T`/`t`/a space and parses *only* that, so a malformed or ambiguous date
  prefix (`-000000` extended year, a U+2212 minus sign instead of ASCII
  `-`, a bare `YYYY-MM` requiring a `T` prefix per
  `plainTimeStringsAmbiguous`, an invalid calendar annotation) is never
  even looked at. Fixing this properly means restructuring PlainTime's
  parse arm to validate the full ISO date-time grammar (like
  `std_temporal_parse_date` already does for the other kinds) before
  discarding the date half, not a quick patch -- deferred as its own item
  rather than rushed.

### Items 2 and 3 (not started)

A fresh full-corpus cluster survey (same run as the numbers above) for
item 2's remaining `built-ins/Temporal` failures by type:

| Type | Fail files |
| --- | ---: |
| `ZonedDateTime` | 92 |
| `Duration` | 63 |
| `PlainDateTime` | 45 |
| `PlainDate` | 42 |
| `PlainTime` | 39 |
| `PlainYearMonth` | 29 |
| `Instant` | 22 |
| `PlainMonthDay` | 11 |
| `Now` | 8 |

`ZonedDateTime.from` (the single largest method-level cluster, 24 files) was
spot-checked: it is **not** the same field-read-order bug (ZonedDateTime's
own property-bag reader is separate code in `builtins_temporal_epoch.rs`,
not `std_temporal_bag`) -- the failures sampled are wrong-exception-type bugs
(`TypeError` expected, `RangeError` thrown, e.g. an empty property bag, a
`timeZone` of the wrong type, an invalid offset string used with an
offset-aware disambiguation option), a distinct area from anything fixed
this session. `ZonedDateTime.since`/`.until` (11 + 10 files),
`Duration.total`/`.round` (21 + 19), `PlainYearMonth`/`PlainMonthDay.from`
(7 + 8), `Instant.toString` (9) and the rest were not individually
triaged. Item 2's actual porting from temporal_rs/lumen (`eadf8039`/
`f6c9860d`) has not started -- everything fixed this session was an
existing-code bug (wrong read order, wrong validation bound, a dropped
rounding step, a missing range check), not a missing algorithm. Item 3
(the 4,000-line file cap) needed no action: `builtins_temporal.rs` (1,415
lines), `builtins_temporal_arithmetic.rs` (1,262 lines) and
`builtins_temporal_epoch.rs` (1,107 lines) are all comfortably under it.

Next session should start from the item-2 table above, triage
`ZonedDateTime`/`Duration` (the two largest clusters) by exact test-name
pattern the way this session did for PlainDate/PlainDateTime, and decide
per-cluster whether the fix is a local bug (as everything this session
turned out to be) or an actual missing port from temporal_rs/lumen.

## JSS-temporal-port, item 2: ZonedDateTime.from wrong-exception-type cluster (checkpoint)

Fixed base: `136dcb33` (`dev`, fast-forwarded onto cleanly). Pinned Test262:
same as item 1. Gate selections: `built-ins/Temporal` and `intl402/Temporal`
(the latter added per the coordinator's note that 92% of intl402's failures
are under `intl402/Temporal/*`, so this lane's fixes are measured there too).

Worked the coordinator's largest-first cluster, `ZonedDateTime.from`
wrong-exception-type (24 files), in `builtins_temporal_epoch.rs`'s
`std_epoch_from_value_with_options`. All fixes below are local bug fixes
(wrong read order or a missing type/range check); nothing needed porting
from temporal_rs/lumen this session.

1. **`std_epoch_zone_value`'s generic ToString fallback.** For any value
   that wasn't already a ZonedDateTime instance or a string, the old code
   called `i.to_string(value)` and fed the (often garbage) result to zone
   parsing, producing a RangeError instead of the spec's TypeError
   (ToTemporalTimeZoneIdentifier: TypeError for anything that isn't a
   String or a `[[InitializedTemporalZonedDateTime]]` object, with no
   generic ToString fallback). Rewrote to match directly on the value.
2. **The `offset` field's missing type gate.** Added
   `std_epoch_string_field_value` (String or non-null Object -> ToString;
   else TypeError, matching `argument-propertybag-invalid-offset-string.js`
   or the wrong-type observer's expectations) and used it for the
   ZonedDateTime object-bag branch's `offset` field.
3. **Options were skipped entirely when `item` is already a ZonedDateTime.**
   `ZonedDateTime.from(existingZDT, options)` clones without applying
   overflow/disambiguation/offset, but per spec still reads and validates
   them first (steps 2.a-2.c). The old early-return path returned before
   ever touching `options`, so a bad-type `options.overflow`/
   `.disambiguation`/`.offset` (or a bad `options` primitive) silently
   didn't throw. Added the reads (using the existing
   `std_temporal_overflow`/`std_temporal_option_string` helpers, which
   already have the right object/ToString semantics) plus their enum
   validation (new `std_epoch_validate_disambiguation`, mirroring the
   existing offset-option enum check) before the early return.
4. **A string argument's options were read before the string was parsed.**
   `ZonedDateTime.from("bad string", badOptions)` must surface the string's
   own RangeError before ever validating `options`'s type -- the opposite
   of the object/instance cases, which validate `options` first.  Split
   `std_epoch_parse` into `std_epoch_parse_syntax` (pure syntax, no options)
   and `std_epoch_resolve_parsed` (options-dependent resolution), so the
   String branch of `std_epoch_from_value_with_options` parses first and
   only reads `options` (with the same enum validation as point 3) once
   parsing succeeds.
5. **The offset field's format was validated only when actually used.**
   `{ offset: "00:00", ... }` (missing sign) with `offsetOption: "ignore"`
   must still throw RangeError for the malformed string, but the old code
   only called `std_epoch_parse_offset` in the branch that *uses* the
   parsed value, skipping it whenever `given_offset` was ignored. Now
   parses/validates the field as soon as it's read, regardless of
   `offsetOption`.
6. **Missing Temporal-instant range check on the disambiguated paths.**
   `std_epoch_resolve_parsed`'s offset-absent and "ignore"/"prefer"-mismatch
   branches, plus the object-bag branch's own disambiguation calls, returned
   `std_epoch_disambiguate`'s result directly without the
   ±8.64e21 ns bounds check that the exact/offset-present path already had.
   Added a shared `std_epoch_check_range` and applied it at all four call
   sites (fixes e.g. the `invalidStrings`/most of `argument-string-limits.js`
   cases; the `offset: prefer`/`reject` wall-clock-range variant of that
   same file needs GetPossibleEpochNanoseconds' local-time-bounds check,
   which is a different, not-yet-implemented check -- deferred).
7. **Calendar was validated after `timeZone`, not before.** For the object-
   bag branch, `std_temporal_bag` (which validates the `calendar` property,
   e.g. rejecting unsupported ids like `"islamic"` with RangeError) was
   called *after* reading/validating `timeZone` -- so `ZonedDateTime.from(
   {calendar: "islamic", ...})` with no `timeZone` at all threw the
   wrong-type TypeError for the missing `timeZone` before ever reaching the
   calendar check. This one was caught by the `intl402/Temporal` gate
   (`ZonedDateTime/from/islamic.js` regressed under the item 3/4 fixes
   above, since the old TypeError-masking-as-RangeError ToString fallback
   had been accidentally passing it). Reordered to validate the calendar
   first, matching spec (calendar resolution precedes PrepareCalendarFields,
   which is where `timeZone` is read).

**Reverted, not kept**: a `monthCode` strict-string-type gate (TypeError for
any non-string, even an object with `toString`), matching the 2026-dated
`ZonedDateTime/from/month-code-wrong-type.js`/`PlainDate|PlainDateTime|
PlainYearMonth/from/month-code-wrong-type.js` (4 files). Applying it in the
shared `std_temporal_read_calendar_fields`/`std_temporal_with` regressed 13
long-established (2020-dated) `order-of-operations.js` files across
PlainDate/PlainDateTime/PlainMonthDay/PlainYearMonth's `from`/`with`/
`since`/`until`, which observe `monthCode` being ToString-coerced (via
`propertyBagObserver`-wrapped values) exactly like every other field. The
pinned Test262 revision has both an old (ToString-coercion) and a new
(strict-type) test for the same PrepareCalendarFields step; kept the old
behavior since it has far more passing coverage (13 files vs 4) and zero-
loss is the hard gate. Left as a known corpus inconsistency, not filed to
`questions.md` since it isn't cross-lane.

**Measured (zero-loss, hashes stable across two consecutive runs of
`built-ins/Temporal`):**

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` variants | 8510 pass / 696 fail | 8598 pass / 608 fail |
| `built-ins/Temporal` files | 4255 pass / 348 fail | 4299 pass / 304 fail |
| `intl402/Temporal` variants | 274 pass / 3784 fail | 274 pass / 3784 fail (unchanged) |
| `intl402/Temporal` files | 137 pass / 1892 fail | 137 pass / 1892 fail (unchanged) |

+88 built-ins/Temporal pass variants (+44 pass files), zero lost (verified by
a full pass-set diff against a true "before" build at the exact pre-session
commit, not just totals). `intl402/Temporal` is unaffected either way (the
92%-under-Temporal js-intl finding is about intl402-specific Intl.
DateTimeFormat/calendar-locale plumbing this lane's fixes don't touch, not
about the exception-type bugs fixed here). Workspace `cargo nextest run
--workspace --locked --no-fail-fast`: 3394/3394 passed, 17 skipped.
`cargo test --workspace --locked --doc`: all green (31 crates,
`amazing_js` 1/1, `amazing_html` 1/1).

12 of the original 24 `ZonedDateTime/from` files still fail: 2 need the
full alphabetical field-read interleave with `offset`/`timeZone`
(`order-of-operations.js`, `calendarresolvefields-error-ordering.js`); 3 need
ISO string grammar extensions this lane's parser doesn't have yet
(hour-only time, hour-only numeric offset, date-only-with-zone-annotation --
`argument-string-optional-parts.js`, and partially `argument-string-limits.js`
/`argument-string-calendar-case-insensitive.js`); `monthcode-invalid.js`
needs the syntax-vs-suitability-validation two-phase ordering noted above but
reverted; `subclassing-ignored.js` is an unrelated Instant/ZonedDateTime tag
mixup not yet root-caused; the rest
(`argument-propertybag-calendar-year-zero.js`,
`argument-propertybag-calendar-invalid-iso-string.js`,
`argument-propertybag-timezone-string-datetime.js`, `offset-string-invalid.js`,
`observable-get-overflow-argument-primitive.js`) were not individually
triaged this session. The coordinator's "next clusters" (Duration 63 fail
files, PlainDateTime 45, PlainDate 42, PlainTime 39, PlainYearMonth 29,
Instant 22, PlainMonthDay 11, Now 8, from the item-2 survey table) were not
started.



## JSS-temporal-port item 2 continued: Duration cluster (checkpoint)

Fixed base: `d62e6d9e` (item 2's ZonedDateTime.from checkpoint, merged into
`dev`). Pinned Test262 unchanged: `3655e7464de3d52643ecddd4b5f9f4f3e7f62398`.

Coordinator's instruction: work the rest of item 2's cluster table, largest
first -- Duration (63/60 measured), then PlainDateTime (45), then PlainDate
(42) -- porting from `boa-dev/temporal` (temporal_rs) or `lucid-softworks/lumen`
where that's faster, checkpointing and committing after each cluster.

**Fixed (6 root-cause bug classes, all local fixes, no porting needed):**

1. `std_temporal_make` (the single Duration-creation choke point) stored the
   exact `i64` for every Duration field instead of canonicalizing to the
   nearest float64-representable integer -- added an `(f as f64) as i64`
   round-trip for Duration fields there.
2. `Duration.compare`/`.round`/`.total` each had their own separately-written,
   partial `relativeTo` (ToRelativeTemporalObject) implementation -- unified
   into one shared `std_temporal_resolve_relative_to` reusing the
   already-correct ZonedDateTime/PlainDateTime bag machinery; deleted the
   now-dead `std_temporal_validate_relative_to_offset`.
3. That resolver's no-zone-annotation string branch parsed via
   `std_temporal_parse(PlainDateTime, ...)`, which wrongly *requires* a
   time-of-day component (a bare date string like `"2023-02-21"` failed) --
   fixed by parsing as `PlainDate` instead, matching ToRelativeTemporalObject
   discarding any time-of-day on a non-zoned string.
4. `std_epoch_parse_syntax` (shared by `ZonedDateTime.from` and the
   resolver's zoned-string branch) unconditionally required a `T`/`t`/` `
   separator, rejecting a valid bare `Date TimeZoneAnnotation` string
   (`"2000-01-01[UTC]"`) -- relaxed to allow an empty time-of-day for
   ZonedDateTime kind, defaulting to 0.
5. Same function was also missing `InterpretISODateTimeOffset`'s
   ~option~/~reject~ `CheckISODaysRange` gate: an explicit non-`Z` numeric
   offset resolved against a zone must reject a *local* date more than 10^8
   days from the epoch even when the offset would shift the final instant
   back into Instant's own valid range. Pinned down by fetching the real
   tc39/proposal-temporal spec text directly (`abstractops.html`,
   `zoneddatetime.html`, `instant.html`, `plaindatetime.html`) after a
   hand-calculation contradiction. Added the local-date-epoch-day check
   ahead of offset resolution.
6. `Duration.prototype.round`/`.total` each read+resolved `relativeTo` only
   inside a fast-path branch gated on whether the duration/unit combination
   needs calendar units at all, so a malformed `relativeTo` bag silently
   never got parsed when the duration was calendar-unit-free. Fetched the
   real `options-read-before-algorithmic-validation.js` source for the exact
   expected option-read order (round: largestUnit, relativeTo,
   roundingIncrement, roundingMode, smallestUnit; total: relativeTo, unit)
   and reordered both to read+resolve `relativeTo` unconditionally, before
   any algorithmic validation, at that exact position.

**Reverted, not kept**: relaxing `std_epoch_zone_from_string`'s `T`-separator
requirement, attempting to fix `relativeto-propertybag-timezone-string-
datetime.js` (a full ISO date-time string as a `timeZone` property-bag value
resolving via its bracket annotation with no `Z`/offset). Measured as 0
gains / 1 regression (`ZonedDateTime/timezone-iso-string.js`, whose
constructor path needs the raw `timeZone` argument to reject a full
date-time string outright) -- reverted cleanly, verified byte-identical
against the pre-attempt pass-set. The intended test's real blocker is still
unidentified.

**Measured (zero-loss at every one of 6 intermediate steps, not just
totals; final state re-verified byte-identical after the revert above):**

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` variants | 8598 pass / 608 fail | 8654 pass / 552 fail |
| `built-ins/Temporal` files | 4299 pass / 304 fail | 4327 pass / 276 fail |

+56 pass variants (+28 pass files), zero lost. Workspace `cargo nextest run
--workspace --locked`: 3399/3399 passed, 17 skipped, 0 failed. `cargo test
--workspace --locked --doc`: all green (31 crates, `amazing_js` 1/1,
`amazing_html` 1/1).

15 of the original 60 Duration fail files fixed. 45 remain, in four rough
sub-clusters: a ~15-file relativeTo out-of-range/large-time-component group
(partly blocked on the reverted timezone-string item, rest untriaged); an
11-file `argument-duration-precision-exact-numerical-values.js`/
`precision-exact-mathematical-values*.js` group (likely a genuine
i128-precision or algorithm gap -- the strongest temporal_rs/lumen porting
candidate found so far); a 4-file round balancing/rounding-algorithm group
(`balances-up-to-weeks.js`, `case-where-relativeto-affects-rounding-mode-
half-even.js`, `round-and-balance-calendar-units-with-increment-
disallowed.js`, `float64-representable-integer.js` -- second porting
candidate); 4 `result-out-of-range-{1,3}.js` files (add/subtract); 6
`order-of-operations.js` files (deprioritized per established pattern); and
`total/options-wrong-type.js` (untriaged).

No temporal_rs/lumen porting was actually needed for any Duration fix this
session -- every bug found was an existing-code defect (wrong order, missing
gate, missing check), not a missing algorithm. PlainDateTime (45 files) and
PlainDate (42 files) are next per the coordinator's ordering, not yet
started.

## JSS-temporal-port item 2 continued: PlainDateTime cluster (checkpoint)

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` variants | 8218 pass / 988 fail | 8704 pass / 502 fail |
| `built-ins/Temporal` files | 4109 pass / 494 fail | 4352 pass / 251 fail |

+486 pass variants (+243 pass files), zero lost, verified by a full
(path, variant) pass-set diff against the Duration-cluster commit's baseline
(`js-stdlib-sinceuntil-baseline.jsonl`, captured at 1aeaf16e). Workspace
`cargo nextest run --workspace --locked --no-fail-fast`: 3399/3399 passed (3
slow, 17 skipped, 0 failed). `cargo test --workspace --locked --doc`: all
green.

Root causes fixed (all local bug fixes; no temporal_rs/lumen porting needed):

- `std_temporal_validate`'s PlainDate/PlainDateTime range check used a crude
  day-count-only cutoff instead of the spec's `ISODateTimeWithinLimits`/
  `ISODateWithinLimits` (fetched from the real tc39/proposal-temporal spec
  text to resolve a hand-calculation contradiction between the two
  algorithms). Replaced with the precise ns-based check: PlainDateTime uses
  its own literal time-of-day; PlainDate defers to the same check with a
  noon (hour 12) reference time, both padded by a full extra day
  (`nsPerDay`) beyond Instant's own `nsMinInstant`/`nsMaxInstant` range.
- `std_temporal_resolve_relative_to`'s no-zone string branch parsed via
  `std_temporal_parse(PlainDateTime, ...)`, wrongly requiring a
  time-of-day -- same bug class as the Duration-cluster fix, third
  occurrence found this segment. Fixed to parse as `PlainDate`.
- That same resolver's property-bag-without-`timeZone` branch validated the
  resolved fields as `PlainDateTime` (midnight-anchored), but
  `ToRelativeTemporalObject` actually constructs a `PlainDate` (time fields
  are read/range-checked then discarded) -- fixed to validate as
  `PlainDate` (noon-anchored), which is what a boundary date like
  `{year:-271821, month:4, day:19}` needs to pass.
- `PlainDateTime.from`'s own string parser also mandated time-of-day (same
  bug class, a separate code path from the resolver) -- made optional.
- `std_temporal_round` (the bare PlainTime/PlainDateTime `.round()`, not
  Duration.round) read `smallestUnit` before `roundingIncrement`/
  `roundingMode`, backwards from spec's alphabetical-property-read order;
  it also never called the divisibility-into-next-unit check at all (only
  a bare 1..1e9 numeric range check existed). Reordered the reads and added
  the missing `std_temporal_validate_rounding_increment` call.
- `smallestUnit: "week"` didn't throw for PlainTime/PlainDateTime.round
  (only year/month were rejected; week is a real duration unit elsewhere,
  just not a legal `round()` smallestUnit) -- added to the rejected set.
- `smallestUnit: "day"` didn't throw for PlainTime.round at all (per spec,
  `ValidateTemporalUnitValue` for PlainTime.round excludes "day" outright,
  unlike PlainDateTime.round which allows it with `roundingIncrement`
  pinned to exactly 1) -- added a PlainTime-only day rejection alongside
  the week/year/month check.
- First attempt at the increment-must-be-1-for-day rule put it inside the
  *shared* `std_temporal_validate_rounding_increment`, used by
  Duration.round (with relativeTo) and by the since/until difference-
  rounding machinery for PlainDate/PlainDateTime -- both of which
  legitimately allow day increments > 1 (confirmed by
  `Duration/prototype/round/roundingincrement-days-large.js` and
  `PlainDate(Time)/prototype/since|until/roundingincrement.js`). That
  first attempt caused an 18-file regression (-30 net). Reverted the
  shared-function change and instead added the day-increment-must-be-1
  rule locally inside `std_temporal_round` only, per spec's
  `PlainDateTime.prototype.round` algorithm (`smallestUnit` is `day` =>
  `maximum` is 1, `inclusive` is `true`) -- Duration.round's and
  since/until's calls to the shared validator are untouched. Re-verified
  zero-loss after the fix.

## JSS-temporal-port item 2 continued: PlainDate cluster (checkpoint)

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` variants | 8704 pass / 502 fail | 8794 pass / 412 fail |
| `built-ins/Temporal` files | 4352 pass / 251 fail | 4397 pass / 206 fail |

+90 pass variants (+45 pass files), zero lost, verified by a full (path,
variant) pass-set diff against the PlainDateTime-cluster commit's baseline
(`js-stdlib-pdt-round-fix.jsonl`, captured at 084c0fd1/3a5a1b2e). Workspace
`cargo nextest run --workspace --locked --no-fail-fast`: 3399/3399 passed (17
skipped, 0 failed). `cargo test --workspace --locked --doc`: all green.

Root causes fixed (all local bug fixes; no temporal_rs/lumen porting needed):

- `std_temporal_plain_to_zoned`'s PlainDate branch required `item` to be an
  object with a `timeZone` property, but per spec
  (`PlainDate.prototype.toZonedDateTime`) `item` may be a bare
  TimeZoneLike value directly (a string or a ZonedDateTime instance), and
  when `item` IS an object whose `timeZone` property is itself `undefined`,
  the WHOLE object is used as the TimeZoneLike value (rather than
  throwing). Restructured to match, and added the missing
  `ISODateTimeWithinLimits` check on the combined date+time when a
  `plainTime` is supplied (the date alone or the time alone can each be
  within their own narrower limits without the combination being valid).
- `std_epoch_zone_from_string` was missing the "no `Z`, no explicit numeric
  offset, but a bracket annotation present" branch (e.g.
  `"2021-08-19T17:30[UTC]"`) -- it always required an offset sign even
  when an annotation could supply the zone on its own. Added the missing
  branch (falls through to the pre-existing `Err(invalid())` only when
  there is truly no zone information at all -- no `Z`, no offset, AND no
  annotation).
- The bare-numeric-offset-as-zone-identifier check (`"+01:00"` used
  directly as a zone, no annotation) only rejected sub-minute offsets by
  their NUMBER value (`ns % 60s != 0`), missing that
  `TimeZoneNumericUTCOffset` forbids a seconds/sub-second component
  *syntactically* -- `"+01:00:00"` must be rejected even though it is
  numerically a whole minute. Added a syntactic check (offset text has a
  fractional part, a 6-digit compact form, or two `:` separators) alongside
  the numeric one.
- `std_temporal_parse`'s `PlainTime` arm discarded its date prefix (before
  a `T`/`t`/space separator) entirely unvalidated -- a syntactically
  invalid or out-of-range discarded date (extended year `-000000`, a
  non-ASCII minus sign, ...) must still throw. Fixed to validate the date
  prefix via `std_temporal_parse_date` whenever it is non-empty (an
  explicit bare `"T15:23:30"` with NO date at all -- an empty prefix --
  is not itself a failure, confirmed by `PlainTime.from`'s own
  `argument-string.js`/`argument-string-with-time-designator.js`; the
  first attempt at this fix validated even the empty case unconditionally
  and regressed 39 files before this distinction was found).
- The `Temporal.ZonedDateTime` constructor's `timeZone` parameter reused
  the same lenient zone resolver as `.from()`/`toZonedDateTime()`
  (`ToTemporalTimeZoneIdentifier`, which accepts a full ISO date-time
  string or a ZonedDateTime instance), but per spec the constructor uses
  the much narrower `ParseTimeZoneIdentifier` (a bare TimeZoneIdentifier
  string only -- an IANA name or a minute-precision numeric offset; even a
  ZonedDateTime instance is a `TypeError`, not reused for its own zone).
  Added `std_epoch_zone_value_strict` and switched the constructor to it --
  this also fixes `ZonedDateTime/timezone-iso-string.js`, a regression an
  earlier segment introduced and reverted while investigating a different,
  related test (`relativeto-propertybag-timezone-string-datetime.js`,
  which is now ALSO fixed as a side effect of the annotation-without-offset
  branch above, along with its Duration/ZonedDateTime siblings across
  compare/round/total/from/equals/since/until -- 6 files that were
  previously left as an unidentified blocker).

Remaining PlainDate-cluster files (8, down from 35): `from`'s
`calendarresolvefields-error-ordering.js`, `month-code-wrong-type.js`/
`monthcode-invalid.js` (known monthCode strict-type corpus inconsistency),
`observable-get-overflow-argument-primitive.js`,
`options-read-before-algorithmic-validation.js`; `add`/`subtract`'s
`argument-duration-max(-plus-min-date).js`, `order-of-operations.js`,
`balance-smaller-units-basic.js`; `toPlainDateTime`/`toZonedDateTime`'s
`argument-string-calendar-annotation.js` and
`argument-string-time-designator-required-for-disambiguation.js` (both are
the still-deferred PlainTime-string-ambiguity-grammar feature -- a string
that could also parse as a date requires an explicit `T` prefix to be
accepted as a time; this needs the real `TemporalTimeString` ambiguity
production, not a local bug fix, and was intentionally not attempted this
checkpoint); `withCalendar`'s `calendar-time-string.js`/
`missing-argument.js`. PlainTime (39 files) is next per the coordinator's
ordering (Duration/PlainDateTime/PlainDate now all worked this ticket). Note:
the PlainDateTime-cluster checkpoint above listed several files as still
open (`toZonedDateTime/timezone-string-datetime.js`, the `withPlainTime`
`argument-string-*.js`/`year-zero.js` family, `calendarresolvefields-error-
ordering.js`, monthCode/`observable-get-overflow-argument-primitive.js`) --
all of the timezone-string-datetime.js/withPlainTime/year-zero/minus-sign
ones were fixed as part of THIS (PlainDate) checkpoint's shared-machinery
fixes, since they all funnel through the same epoch-string/PlainTime-parse
code paths; the rest remain open and are re-listed in the "remaining" set
above.

## JSS-temporal-port item 2 continued: PlainTime cluster (checkpoint)

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` files | 4397 pass / 206 fail | 4414 pass / 189 fail |

+17 pass files, zero lost, verified by a full (path, variant) pass-set diff
against the PlainDate-cluster commit's baseline (`js-stdlib-pd-r4.jsonl`,
captured at 2bfe3d85/14aab57f). Workspace `cargo nextest run --workspace
--locked --no-fail-fast`: 3399/3399 passed. `cargo test --workspace --locked
--doc`: all green.

Root causes fixed (all local bug fixes; no temporal_rs/lumen porting needed):

- `std_temporal_parse_annotations`'s calendar-annotation check unconditionally
  rejected any FIRST `u-ca=` value other than `"iso8601"` -- correct for
  calendar-bearing kinds (this engine's calendar model is iso8601-only,
  ADR-locked) but wrong for `PlainTime`, which has no calendar slot at all
  and never canonicalizes the annotation's value (a real-but-unsupported
  calendar like `"hebrew"`, or an outright unknown one, is simply accepted
  and discarded). Added a `require_iso_calendar` parameter, `false` only
  for `PlainTime`.
- `std_temporal_from_fields`'s ZonedDateTime-instance-argument branch
  returned the FULL local date+time unconditionally instead of zeroing the
  half the target kind doesn't use, unlike the sibling "compatible Temporal
  record" branch just above it (which already does this correctly) --
  `PlainTime`/`PlainDate` `.compare()`/`.equals()` against a ZonedDateTime
  never matched because the receiver's own zeroed date (or time) fields were
  compared against the ZDT's real local date (or time). Fixed to zero
  `fields[3..9]` for `PlainDate` / `fields[0..3]` for `PlainTime`, matching
  the sibling branch's convention.
- `std_temporal_add_values`'s `PlainTime` branch incorrectly threw
  `RangeError` for any duration with a nonzero year/month/week/day component
  -- a rule copied from `Duration.add` (which DOES throw for calendar units,
  since it has no `relativeTo` to redistribute them through). Per spec's
  `AddDurationToTime`, `PlainTime.add`/`.subtract` simply IGNORE a
  duration's date part entirely (`ToInternalDurationRecord` splits it off,
  `AddTime` never consumes it) -- never throw. Fixed to discard the date
  part instead of rejecting it.
- `std_temporal_to_string`'s `calendarName` option read was already
  excluded for `Duration` (no calendar) but not for `PlainTime` (also no
  calendar, per spec's `PlainTime.prototype.toString` algorithm, which never
  reads `calendarName` at all) -- extended the existing exclusion.

Remaining PlainTime-cluster files (11, down from 39): 5 are the deferred
PlainTime-string-ambiguity-grammar feature
(`argument-string-time-designator-required-for-disambiguation.js` across
`compare`/`from`/`equals`/`since`/`until` -- the same gap noted for
PlainDate/PlainDateTime in the two prior checkpoints: a string that could
also parse as a date requires an explicit `T` prefix to be accepted as a
time, needing the real `TemporalTimeString` ambiguity production rather than
a local bug fix); 6 are an unrelated precision cluster not yet investigated
(`add-large-subseconds.js`, `subtract-large-subseconds.js`,
`argument-duration-precision-exact-numerical-values.js` x2 add/subtract,
`order-of-operations.js` x2 add/subtract). PlainYearMonth (29 files) is next
per the coordinator's ordering.

## JSS-temporal-port item 2 continued: PlainYearMonth cluster (checkpoint)

| Selection | Before | After |
|---|---|---|
| `built-ins/Temporal` files | 4432 pass | 4472 pass |

+40 pass files, zero lost, verified by a full (path, variant) pass-set diff
against the PlainTime-cluster baseline (`js-stdlib-pym-base.jsonl`). Workspace
`cargo nextest run --workspace --locked --no-fail-fast`: 3399/3399 passed (3
slow, 17 skipped). `cargo test --workspace --locked --doc`: all green
(`amazing_js`/`amazing_html` doctests re-verified individually).

Root causes fixed (all local bug fixes; no temporal_rs/lumen porting needed):

- `std_temporal_validate` had no `ISOYearMonthWithinLimits` check at all --
  `PlainYearMonth` only ever went through the day-precise
  `PlainDate`/`PlainDateTime` range check (which does not apply to it) or no
  range check beyond plain field bounds. Added the coarser year+boundary-month
  check from the real spec algorithm (year -271821 requires month >= 4; year
  275760 requires month <= 9).
- `std_temporal_construct` read `PlainYearMonth`/`PlainMonthDay`'s trailing
  `referenceISODay`/`referenceISOYear` argument BEFORE validating the
  `calendar` argument, backwards from every constructor's own spec order
  (calendar type-check/canonicalize always precedes both the trailing extra
  argument's read and the final `IsValidISODate` check). Reordered; confirmed
  by `PlainYearMonth`'s own `calendar-invalid.js`.
- `std_temporal_yearmonth_to_date` (`toPlainDate`) set the day field directly
  from the input without constraining it to the target month's length --
  `toPlainDate` always uses `~constrain~` overflow (there is no overflow
  option to read), so day 29 in a common-year February must clamp to 28, not
  throw. Fixed to clamp into `[1, days_in_month]`.
- `PlainYearMonth.prototype.add`/`.subtract` used the object's own
  `referenceISODay` (not day 1) as the anchor for the month-arithmetic
  overflow check, and never validated the day-1 anchor (before or after the
  add) against the full day-precise `PlainDate` range -- per
  `AddDurationToYearMonth`, the anchor is always day 1 of the month, and both
  `CalendarDateFromFields` (before) and `CalendarDateAdd` (after) are gated by
  `ISODateWithinLimits`, narrower at the two extreme years than
  `PlainYearMonth`'s own limits. Fixed; confirmed by
  `throws-if-year-outside-valid-iso-range.js` (throws even for a blank
  duration, from materializing `-271821-04-01` alone).
- `PlainYearMonth.prototype.since`/`.until` had only an ad hoc
  `+-100_000_000` epoch-day sanity check, not the real
  `CalendarDateFromFields`/`CalendarDateAdd`-style range gating, and rounded
  by flattening the whole span to a single unit before re-splitting into
  years+months -- disagreeing with the real per-unit rounding (an increment
  coarser than a month rounds the *remainder after extracting whole years*,
  not the flattened total; confirmed by
  `roundingincrement-as-expected.js`'s "mixed with years" case, which needs
  2y5mo, not 2y6mo). Also added: an early zero-duration return for equal
  year-months (spec's own short-circuit, needed so `minYearMonth.since(
  minYearMonth)` does NOT throw even though materializing its own day-1
  anchor would), and a bracket-candidate range check gated to only run when
  `smallestUnit != month || roundingIncrement != 1` (the real spec algorithm
  skips its NudgeToCalendarUnit bracket search entirely for the default
  case -- confirmed by two regressions caught and fixed in the same pass:
  probing one unit past a valid boundary argument wrongly rejected it, and
  `since`/`until`'s own `argument-string-limits.js` needed the bracket
  search *disabled* for default options).
- `std_temporal_bag`'s Duration-bag field reads used the natural years-down-
  to-nanoseconds order instead of the spec's alphabetical order (days, hours,
  microseconds, milliseconds, minutes, months, nanoseconds, seconds, weeks,
  years) -- a pre-existing, previously-unnoticed bug affecting EVERY kind's
  `.add()`/`.subtract()`/`.compare()`/`Duration.from()`, not just
  PlainYearMonth's. Fixed the shared function; this alone fixed
  `order-of-operations.js` for Duration, Instant, PlainDate, PlainDateTime,
  PlainTime and PlainYearMonth's own add/subtract in one pass (part of the
  +40, spanning kinds outside this cluster).
- `std_temporal_bag`'s monthCode SUITABILITY check (a calendar-dependent
  RangeError) ran immediately inside the shared bag reader, before any
  caller could read/coerce an `overflow` option in between --
  `CalendarYearMonthFromFields`/`CalendarDateFromFields` read
  `GetTemporalOverflowOption` before running `CalendarResolveFields`'s own
  suitability check. Split `std_temporal_bag` to return the raw
  month/monthCode-text pair instead of resolving eagerly, added
  `std_temporal_bag_resolve_month` for callers to invoke at the right point,
  and deferred it past the overflow read in `.from()`'s bag branch only
  (other call sites resolve immediately, matching their own existing,
  untouched order). Fixed `options-read-before-algorithmic-validation.js`
  for `.from()` across PlainDate, PlainDateTime, PlainMonthDay and
  PlainYearMonth in one pass.
- `std_temporal_to_string` unconditionally called the shared
  fractionalSecondDigits/roundingMode/smallestUnit option reader even for
  `PlainDate`/`PlainYearMonth`/`PlainMonthDay`, which have no time-of-day and
  whose spec algorithms only ever call
  `GetTemporalShowCalendarNameOption` -- reading (and finding absent) three
  extra options is observable via a Proxy options bag. Skipped the call
  entirely for those three kinds; fixed `toString/order-of-operations.js`
  for PlainDate, PlainMonthDay and PlainYearMonth in one pass.

Remaining PlainYearMonth-cluster files: 1
(`from/month-code-wrong-type.js`), a deliberately-unresolved,
previously-documented tradeoff (monthCode is read via `ToString` coercion to
keep 13 passing `order-of-operations.js`-style tests; a TypeError-on-
non-string read would flip this one test but regress those 13). Not
revisited this checkpoint.

PlainYearMonth cluster done. Remaining item2 clusters per the coordinator's
ordering: PlainMonthDay (11 files, last surveyed), Instant (22 files), Now (8
files).
