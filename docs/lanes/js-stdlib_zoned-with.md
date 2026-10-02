# ZonedDateTime.with calendar field merging

Base: `2eb1229a`. Linux validation uses Boat via the project wrapper.

`with()` now reads era and eraYear in alphabetical field order for calendars
that expose eras, and treats supplied era fields as replacement of the
receiver's year/era pair. Presence/type validation precedes monthCode and day
range validation. Changing a year without changing the month preserves the
receiver's monthCode across calendars with leap months; an absent leap month
constrains or throws according to overflow.

Ethioaa native year is consistently the Amete Alem year in field records and
era resolution, with its 5500-year offset applied only when converting to ISO.
Offset strings are parsed even with the `ignore` offset option.

Specification: <https://tc39.es/proposal-temporal/#sec-temporal.zoneddatetime.prototype.with>.
These are adaptations of existing engine helpers; no new upstream code was
copied. Existing upstream attribution remains in the calendar module.

Baseline `with` failures: 1 built-ins file and 23 intl402 files. Umm al-Qura's
tabular-civil approximation remains a separate calendar-data limitation.
Boat pool-js-lib, exact source checkpoint `696b216e`:

| Check | Before | After | Lost files |
|---|---:|---:|---:|
| built-ins/Temporal | 4486 | 4487 | 0 |
| intl402/Temporal | 1519 | 1573 | 0 |
| amazing-js nextest | — | 469 passed, 1 skipped | — |
| amazing-js doctest | — | 1 passed | — |

22 of the 23 intl402 `ZonedDateTime.with` failures now pass. The ethioaa
consistency fix also improves construction and arithmetic across PlainDate,
PlainDateTime, PlainYearMonth and ZonedDateTime.

Workspace validation was attempted three times on Boat but failed before
tests ran: missing dependency rlib formats (skrifa, then read_fonts/writeable/
p256/crossbeam_epoch, and unicode_ident in a fresh lane target). This is routed
to the integrator as a build-environment blocker; no workspace success is claimed.
