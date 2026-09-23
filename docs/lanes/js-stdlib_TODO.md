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
