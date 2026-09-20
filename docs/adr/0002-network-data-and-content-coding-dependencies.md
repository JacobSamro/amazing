# ADR 0002 — Pinned Public Suffix List and a vetted brotli decoder

Status: accepted (2026-09-21)

## Context

Two network capabilities could not be built under the std-only rule without
either accepting an external dependency or shipping something knowingly wrong.
Both were surfaced by the `net` lane with written evaluations in
`docs/lanes/net_TODO.md`, and both were escalated rather than decided by the
lane or the integrator, because a new external dependency is a project-level
decision.

**Registrable domains.** Cookie domain matching, `SameSite`, and site-based
partitioning all need eTLD+1. It cannot be computed from a hostname. Guessing
the last two labels is wrong in both directions — it splits sibling subdomains
that belong to one site, and merges unrelated sites under multi-label suffixes.
The interim full-host fallback is also wrong, just more conservatively.

**Brotli.** `br` is widely served. Hand-writing a conformant decoder is a much
larger job than the DEFLATE that already landed, and advertising an encoding the
engine cannot decode is not an option.

## Decision

Accept both, as narrowly scoped exceptions in the established pattern of
decisions 4d (text provider), 6 (TLS) and 6b (codecs): dependency-internal code
stays outside every engine core, first-party adapters forbid unsafe, and
dependency types never cross the seam.

- The **Public Suffix List** is vendored as pinned data, not as a crate, at an
  exact upstream commit and SHA-256, licence retained, ICANN and PRIVATE
  sections both included. Updates are reviewed pull requests with
  boundary-change tests and a persisted-key migration review. Never a silent
  runtime fetch.
- **Brotli decoding** uses a vetted adapter pinning `brotli-decompressor`,
  decode only, with the caller-selected output bound enforced before
  allocation.

Recorded as `ARCHITECTURE.md` sections 6c and 6d.

## Consequences

- `amazing-fetch` keeps a `RegistrableDomain` provider seam whose default fails
  closed; only the provider changes when the snapshot updates. NET-5 was written
  against that seam before this decision, so no rework is needed.
- A PSL update can change cookie and cache keys already written to a user's
  profile. Persisted-key migration is therefore part of the update procedure,
  not an afterthought.
- The network brotli path decodes attacker-supplied bodies, which is why it is a
  separate exception from the font-scoped approval in decision 4d rather than an
  extension of it.
- The std-only rule still holds where it matters: DEFLATE, gzip and zlib remain
  hand-written, and the engine cores gain no third-party code.

## Alternatives rejected

- **Ship gzip only.** Defensible and already working, but leaves real-page
  compatibility on the table for a coding that is near-universally served.
- **Hand-write brotli.** Consistent with the std-only rule, but the cost is out
  of proportion to the benefit at this stage of the project.
- **Guess eTLD+1 from label counts.** Produces silently wrong site boundaries,
  which is worse than the conservative fallback it would replace.
