# ForIn head evaluation

Recorded published base: `7bc73204149e50b8875c0ab9d1b552cb4e1fe251`.
Whole paired Linux qualification closed on tested source `fa98dac9eeec66f6c7aef38c9e3574d3dbb12be4`; this ready commit adds compact evidence only above those exact production/test bytes.

The actual language compact retained `annexB/language/statements/for-in/nonstrict-initializer.js` as failing. The larger remaining clusters require held module/UTF16/call prerequisites or runtime eval/tail-call gateways. This independent area covers complete ForIn head semantics and their existing body/enumeration contract, rather than narrowing the authored regression to one initializer expression.

Current primary Annex B Initializers in ForIn Statement Heads and ForIn/OfHeadEvaluation were reviewed on 2026-10-04. The parser already preserves allowed sloppy var/BindingIdentifier initializers and rejects strict, lexical, destructuring, multiple-binding and ForOf initializers. Both actual evaluator and VM were skipping those preserved expressions. The head now captures the binding reference without GetValue, evaluates/names the initializer, stores once, then evaluates the right side. This preserves With object-binding identity if the initializer changes property existence. Existing lexical RHS TDZ and per-iteration environments remain distinct.

Algorithm sequencing is adapted from boa-dev/boa `core/engine/src/bytecompiler/statement/loop.rs::compile_for_in_loop` at `39cd11214ecb366200157dab96b3919e61e9a391`, MIT; exact LICENSE-MIT was verified. Amazing retains its own Reference capture and raw write instructions to match the primary ResolveBinding-before-initializer contract. Attribution headers and an appended notice accompany the source. No upstream dependency is added.

AST owns one bounded ForIn initializer helper before existing RHS evaluation. VM owns one ForIn-only prelude using existing ResolveWriteName, NamedEvaluation and StoreReference instructions. The shared ForOf/ForAwait lexical helper is unchanged. No new opcode, state/frame/Env/Fn/coroutine/public schema, call gateway or module policy is introduced.

Ten authored groups cover genuine forced-AST/native callable factories, empty/null/primitive RHS, observable reference resolution and unscopables, anonymous callable/class NamedEvaluation, abrupt initializer/setter/RHS precedence, lexical TDZ/fresh closures/destructuring, prototype/nonenumerable/deletion iteration controls, labels/finally, complete grammar negatives, and original binding retention across generator/async/async-generator suspension, and all directly attached label aliases through nested loops/finally/suspended bodies. Whole language/annexB plus Function and generator/async-generator/Object/Proxy/Reflect corpus selectors protect existing semantics on a fixed same-base pair. Before outcomes are observed rather than whitelisted. No readiness or native parity claim precedes actual results.

## First measured checkpoint and correction

The first fixed source `886135b9` measured 4464 PASS / two new FAIL / 17 SKIP, versus tests-only `8c7cf54d` 4460 PASS / six new FAIL / 17 SKIP; both passed four doctests. The candidate strict native guard stopped before its corpus, and that diagnostic is preserved. One authored anonymous-class expectation was incorrect: current NamedEvaluation calls ClassDefinitionEvaluation with undefined classBinding and inferred name, so anonymous methods retain the mutable outer binding. The corrected group contrasts this with genuine explicitly named class/function immutable self-bindings rather than removing coverage.

The actual remaining production fault was forced-AST labeled Continue: the existing Labeled dispatcher passed loop labels only to While/DoWhile/CFor. ForIn now has a shared owned loop helper receiving every contiguous attached label. Matching Continue/Break targets are handled by that loop; other abrupt completions propagate unchanged. Native existing label metadata is unchanged; the AST dispatch changes only ForIn. Original eight groups remain, with correct positive/negative name expectations and two additional label/finally/suspension groups.

The second whole gate closed all eight original groups, but both added contiguous-label groups detected native factory fallback. The compiler loop predicate now follows attached labels to the terminal iteration statement, preserving the existing pending-label set and Continue/Break lowering; no test expectations changed.

## Qualified whole gate

Tests-only before `b85711db` observed 4460 PASS / exactly eight authored FAIL / 17 SKIP; candidate `fa98dac9` passed all 4468 tests / 17 SKIP. Both passed four doctests and release host builds. Every protected workspace PASS/SKIP identity was retained.

Eight whole corpus selectors ran in default and VM modes with pinned `3655e746`, jobs 8 and 60-second deadlines. Per mode, 29,324 selected files retained 28,910 passes and 53,979 passing variants; +1 file / +1 variant, zero lost files or variants. The gained file is `annexB/language/statements/for-in/nonstrict-initializer.js`. All sixteen controllers completed normally (0/1); final supported wrapper exited 0.

Baseline corpus receipts retain their authentic `8c7cf54d` source: refreshed before production differed only in cfg(test) fixtures, and the actual rebuilt release ELF, runner, pinned input, and every original raw SHA/size matched. Fresh before native/docs/release and all fresh candidate phases were measured. Raw logs/results remain on pool-js-vm under `/home/user/amazing/out/js-vm/for-in-head-*`; exact paths/hashes are in the compact gate map. Candidate shipping ELF is preserved there too.

Primary references: [Annex B ForIn initializers](https://tc39.es/ecma262/multipage/additional-ecmascript-features-for-web-browsers.html#sec-initializers-in-forin-statement-heads), [NamedEvaluation](https://tc39.es/ecma262/multipage/syntax-directed-operations.html#sec-runtime-semantics-namedevaluation). Neither a tooling failure nor a corpus waiver was used: both earlier stops were genuine authored semantic/eligibility diagnostics, corrected in bulk with original coverage retained.
