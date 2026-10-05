# Genuine Catch provenance

Qualified with the complete [EDI/FDI/source gate](js-source-fdi-edi-joint.md). EnvKind::Catch marks actual CatchParameter records only. Optional catch and catch-body lexical scopes remain Blocks. EnterCatchValue carries a private actual-parameter flag: real Try uses true, synthetic disposal handlers use false.

Catch retains Block-compatible variable scope, this lookup and With barrier behavior. Record identity supplies the EDI optional Catch exception; no simple-Identifier restriction is invented. No new binding/frame/realm storage or upstream code was introduced by this companion.
