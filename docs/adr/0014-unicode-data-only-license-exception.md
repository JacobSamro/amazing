# ADR 0014 — Official Unicode data and tests

Status: accepted (2026-10-04), explicitly approved by the project owner.
Owner: integrator. Applies to Unicode data and conformance tests only.

## Decision

The owner approved “Allow Unicode data only” in response to the request to
include official Unicode 17 character data and tests under Unicode-3.0 while
keeping the Rust algorithm MIT licensed.

This is a narrow exception to ADR 0013 and the MIT/Apache-2.0 port-source rule:

- Official Unicode 17 data and tests may be included under Unicode-3.0.
- Rust algorithm ports still require MIT or Apache-2.0, exact upstream commit
  and path attribution, and an appended third-party notice.
- Include the Unicode license and notices, official source URLs, version,
  checksums, and reproducible generation instructions with the adopted data.
- Generated tables must retain their Unicode data provenance.
- This approval does not authorize other code licenses, CLDR or IANA data,
  new crate dependencies, or changes to the engine's architecture.

## Validation

Normalization work must cover the complete official normalization test corpus
and the relevant pinned Test262 directories, preserve raw UTF-16 semantics,
and pass the existing Linux workspace, doctest and zero-loss corpus gates.
License authorization does not replace implementation or conformance evidence.
