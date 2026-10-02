# Umm al-Qura calendar data

Base: `79f6f464`.

Ported `lucid-softworks/lumen`'s `crates/lumen/src/umalqura.rs` at
`f6c9860d44fa52249cfd647122edea184d32c863`, MIT. The license at that commit is
MIT, copyright2026 Lucid Softworks limited. The complete year-start and
month-length tables cover AH1300–1600, inclusive. Upstream generated their
numeric facts from ICU's islamcal.cpp; no ICU implementation code was copied.

New owned file `builtins_temporal_umalqura.rs` carries the attribution header,
and an append-only notice row names that file in THIRD_PARTY_NOTICES.md. The
calendar module includes the tables without changing the shared interpreter
file or adding dependencies.

Within the table range, ISO-to-calendar conversion finds the year by binary
searching year-starts and then walks the twelve months' lengths. The inverse
uses that year's start and preceding month lengths. Month lengths, dayOfYear,
daysInYear and inLeapYear use the same tables. The final year includes its full
last month. Outside AH1300–1600, retain the upstream's tabular-civil fallback.

Source: <https://github.com/lucid-softworks/lumen/blob/f6c9860d44fa52249cfd647122edea184d32c863/crates/lumen/src/umalqura.rs>.
License: <https://github.com/lucid-softworks/lumen/blob/f6c9860d44fa52249cfd647122edea184d32c863/LICENSE>.

Boat corpus comparison against the preceding PlainDate/PlainDateTime.with
series: intl402 Temporal1799→1842/2029 (+43 passing files), built-ins
Temporal4487/4603 unchanged, zero lost files. Every remaining with-family
failure (ZonedDateTime, PlainDate and PlainDateTime) now passes. Crate469 tests
passed/1 skipped and1 doctest passed. Final Boat workspace3515 tests passed/
17 skipped, all four doctests passed, and repeated corpus classifications were
identical. Cumulative across the resumed four series:324 gained Temporal files,
zero lost, on fixed fleet base2eb1229a.
