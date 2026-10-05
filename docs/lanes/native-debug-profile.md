# Native debug profile tooling

Recorded dev base ac72a9687c8a1ff76c3d0eb47db98a321ad263b8. Frozen qualified source 7cefeb9b833b3a84015529e22af7f131148015c4. This tooling ticket sets native development/test debugging to line-tables-only through the public runner, without changing Cargo manifests, debug assertions or overflow checks. Existing captured executables and raw outputs are preserved.

Public native qualification owner13455 exited0; captured job427d4c1b0083479e8936da92fe4dcf5c on bx_5sy3b3ht exited0; compact fetch exited0. Both profiles passed two assertion/overflow controls, one doctest passed, actual verbose compiler flags matched the requested settings, and both captured ELFs retain debug line sections. Fixture ELF bytes6999464 versus7035432. This small fixture comparison does not measure whole-workspace storage savings. Public endpoint receipt pins exact source; raw fixture summary's optional AMAZING_SOURCE_COMMIT field is null because the runner does not export that variable.

Separate authorized terminal cleanup on bx_fge8dzbh removed34063 exact classified rebuildable Cargo cache paths after confirming no live controller/compiler/renderer. Available bytes0→8861167616;1430 executable/shared ELF paths verified unchanged. Raw originals and source paths were untouched. Exact deletion paths remain in ignored local scratch; compact summary records their hashes. That cleanup is an operational result, not evidence that e177 image consumer passed. Its two exhausted tooling attempts remain unqualified.

Official Cargo profile reference: https://doc.rust-lang.org/cargo/reference/profiles.html#debug .
