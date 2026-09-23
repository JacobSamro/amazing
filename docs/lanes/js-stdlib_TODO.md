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

## Next: Temporal

Implement ISO calendar Temporal types with UTC and fixed offset time zones.
Store epoch nanoseconds as `i128` internally. JSB-2 owns arbitrary precision
BigInt and must land before BigInt-returning Temporal getters can be exposed.
Named IANA zones and non-ISO calendars remain outside this host's scope.
