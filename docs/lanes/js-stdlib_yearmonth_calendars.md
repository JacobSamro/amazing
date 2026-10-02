# PlainYearMonth calendar arithmetic and field merging

Built on the ZonedDateTime.with series, fixed fleet base `2eb1229a`.

Calendar arithmetic now materializes day 1 of the receiver's calendar month,
then converts that full date into ISO. It does not overwrite the ISO day with
1 or restore an old reference ISO day after arithmetic. `with()` reads native
calendar fields and merges month/monthCode and year/era pairs before resolving
back to ISO. Missing leap months respect reject/constrain overflow.

Non-ISO year-month differences reuse the calendar-aware date difference and
rounding helpers at native month-start anchors. Complete ISO reference dates
with non-ISO annotations are accepted and canonicalized; abbreviated YYYY-MM
forms still require ISO calendars. PlainDate.toPlainYearMonth uses the same
native-month-start conversion.

The anchor conversion alone does not impose PlainDate bounds: PlainYearMonth
has broader limits. Arithmetic and difference callers impose the narrower
PlainDate bounds when their algorithms materialize a full date. This separation
preserves the boundary conversion in `toPlainYearMonth/limits.js`.

Specification: <https://tc39.es/proposal-temporal/#sec-temporal-adddurationtoyearmonth>
and <https://tc39.es/proposal-temporal/#sec-temporal-differencetemporalplainyearmonth>.
No new external code was copied; existing calendar and difference helpers keep
their original upstream attribution.

Boat pool-js-lib corpus results, compared with the preceding ZonedDateTime.with
series:

| Selection | Before | After | Lost files |
|---|---:|---:|---:|
| PlainYearMonth add/subtract/with/since/until (intl402) | 68/190 | 190/190 | 0 |
| intl402/Temporal | 1573 | 1699 | 0 |
| built-ins/Temporal | 4487 | 4487 | 0 |
| amazing-js nextest | — | 469 passed, 1 skipped | — |
| amazing-js doctest | — | 1 passed | — |

The 126 intl402 gains include all 122 remaining files in the assigned methods
and four additional construction/conversion files. The final workspace gate
passed on Boat using a fresh lane target, two compile jobs and disabled
incremental compilation: 3515 tests passed, 17 skipped, and all four workspace
doctests passed (a11y, devtools, html and js). This resolves the build-environment blocker recorded in the
preceding ZonedDateTime.with series. Both Temporal selections were rerun after
the workspace checks with complete result files. Umm al-Qura's tabular
approximation remains a separate data limitation.
