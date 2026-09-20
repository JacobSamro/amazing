//! Bounded brotli decoding for HTTP content coding.
//!
//! Approved as a project-level dependency exception in `ARCHITECTURE.md` 6d and
//! ADR 0002. This is a **separate** exception from the font-scoped approval in
//! decision 4d, even though `brotli-decompressor` is already pinned
//! transitively through `wuff` in `amazing-text-provider`, because this path
//! decodes attacker-supplied response bodies.
//!
//! Therefore: the caller-selected maximum decoded size is enforced **before**
//! allocation, so a compression bomb fails closed rather than expanding first.
//! Decode only — there is no encoder. Dependency types never cross this
//! boundary; `amazing-net` receives bounded owned bytes and errors.
//!
//! Skeleton registered by the integrator; the `net` lane implements it under
//! NET-6.

#![forbid(unsafe_code)]
