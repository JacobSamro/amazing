# Expanded IA-1 routing evidence

All execution was on win-001. This evidence uses integration base
`784dc59aec23e83ac04114a2830f19731e74d902`, not subsequently advanced dev.

- `routing-before.exe`: driver and runner from local commit
  `225c94e6f96c1bbe6f784d7fa1f7fefbe4ba39d6`, after extraction/natural driver
  dimensions but before expanded root mapping. The baseline wrapper replaces
  server Git verification with detached HEAD reading and shortens artifact
  paths only; test assertions, pixels and report IDs are retained.
- `expanded.exe`: candidate source committed as
  `5648fb9fa19773ce4377540cf14b7aec714bb3bc`.
- `executables.json`: immutable Windows executable hashes. Gate sweeps use
  these explicit diagnostic copies, never the mutable Cargo output executable.
- `source-manifest.json`: all 258 recorded source/input files matched Windows;
  manifest SHA-256 is recorded in `native-tests.json`. The refresh script cleared
  first-party build fingerprints before the original expanded build.
- `native-tests.json`: two complete workspace runs and focused shell viewport
  tests. No macOS builds or tests were performed.
- `root-1.json`, `root-2.json`, `summary.json`: synthetic resource/reference
  mapping diagnostics. The summary hash is of the original Windows UTF-8 BOM /
  CRLF artifact; Git may normalize line endings. These earn zero WPT credit.

The workspace and shell tests precede the same-base baseline build. Its
source swaps were restored and verified. Cargo's mutable default executable
is not evidence; use the frozen candidate path above or rebuild after refreshing.
No screenshot/visual shell proof or XML parser conformance is claimed.
XML dispatch, propagation of `XmlErrorKind::Unsupported`, and removal of blanket
XHTML exclusions remain pending. The inherited generic unsupported status and
XHTML exclusions are retained. Git-free runner completion does not complete HTML-1.

A subsequent fleet refresh occurred during the frozen sweeps. All 17 manifest
JSON values and both executable hashes remained unchanged; byte differences in
some manifests were formatting only (`refresh-verification.json`). Source
verification above predates that refresh. The server's refreshed dependency
graph was not rebuilt for these results. Rebase/refresh before any future build.
The two recorded cargo-test runs include 1,134 native passes, 3 native ignores,
and 2 doctest passes each. Future runs use nextest plus explicit doctests under
the updated fleet policy; these historical runs were not rerun with nextest.
