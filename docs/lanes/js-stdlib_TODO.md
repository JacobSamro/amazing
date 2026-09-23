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
