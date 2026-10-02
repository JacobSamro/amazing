# PlainDate and PlainDateTime calendar field merging

Base: `0deeccb7`, following the calendar-aware PlainYearMonth work.

The native calendar merge helper now handles PlainDate and PlainDateTime,
preserving their calendar-native day and time fields instead of interpreting
supplied year/month/day values as ISO fields. MonthCode remains stable across
year changes, while explicit month or monthCode replaces the other field.
Era and eraYear follow the same replacement rules as PlainYearMonth.

The shared field reader tracks which time fields were supplied. This preserves
an existing PlainDateTime's clock fields when only its calendar date is changed,
while the from() path keeps its existing zero defaults for absent time fields.
All field reads retain the existing alphabetical order and immediate conversion
rules. Overflow validation runs before calendar field resolution.

Specification: <https://tc39.es/proposal-temporal/#sec-temporal.plaindate.prototype.with>
and <https://tc39.es/proposal-temporal/#sec-temporal.plaindatetime.prototype.with>.
This generalizes existing engine helpers; no new upstream code was copied.

Measured on Boat pool-js-lib:

| Selection | Before | After | Lost files |
|---|---:|---:|---:|
| intl402 PlainDate.with | 22/71 | 70/71 | 0 |
| intl402 PlainDateTime.with | 21/70 | 69/70 | 0 |
| intl402/Temporal | 1699/2029 | 1799/2029 | 0 |
| built-ins/Temporal | 4487/4603 | 4487/4603 | 0 |

The 100 gains comprise 96 files in the with methods and four epoch-year
getter tests across the Temporal date types. Both remaining with failures
require real Umm al-Qura month-length data. Final repeat classifications were
identical. Workspace3515 tests passed/17 skipped and4 doctests passed; crate
iteration469 tests passed/1 skipped plus1 doctest. The workspace uses the same
fresh lane target with2 compile jobs and incremental disabled as the preceding
series.
