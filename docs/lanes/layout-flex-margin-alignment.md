# Flex used margins and cross alignment

Qualified fixed base: dcee3e6202fa31b8543c2ef83a28cafe73f13dc1.
This independent ticket imports no held Canvas/table, container-effects, font,
output-demand, CSS Text or DPR sources. Production source is identical to the
rendered3e064abd; tests are identical to native-qualified dfbb134b.

Auto margins now remain used geometry instead of overwriting computed
properties. Both physical cross sides resolve against their actual line;
overflow preserves genuine writing-mode block/inline-start independently of
wrap reversal. Row center/end counts padding/border once. Final stretched lines
realign auto-margin items, wrapped columns honor line packing, and row stretch
honors authored min/max and actual definite line size. Shaping, providers,
positioned solvers, flexible-length and line-collection algorithms are unchanged.

Primary: CSS Flexbox1, published14October2025, sections8.1/9.4–9.6,
https://www.w3.org/TR/css-flexbox-1/, read2026-10-05.
Port: DioxusLabs/taffy src/compute/flexbox.rs at
2c81b8e09d172c2e4d7747be2b1e5a03e0f30bf0, MIT; LICENSE SHA256
f97daf1a0124413dccf399a4e6626b4b74acd05282f80b6d64ac82225650b77a.
Copyright(c)2018VislyInc.; Copyright(c)2026TaffyAuthors. Exact file header and
append-only THIRD_PARTY_NOTICES row accompany the adaptation. Published
negative-overflow behavior is integrated with our authentic axis mapping.

Linux qualification on public layout/pool-layout bx_ncen6t35:
workspace4517→4527passed,17skipped, all4doctests passed both sides.
Seven complete WPT directories: flexbox338→349/523, grid413/945,
sizing99/138, position45/54, display28/35, containment155/229,
overflow83/343. Total1161→1172/2267,+11gains/0losses, no other classification
changes/errors/crashes/timeouts. Exact gain IDs and full compact maps are in
[evidence/flex-used-margins/summary.json](evidence/flex-used-margins/summary.json).

The first native candidate reported four new fixture failures: higher-specificity
helper defaults overrode authored120px/60px dimensions. The test-only correction
preserves every original geometry assertion and adds actual dimension guards.
Full workspace+docs rerun passed alltennewgroups; production bytes were unchanged,
so the completed whole render comparison is retained without another render run.
An earlier attempt failed before build on an oversized stopped snapshot; root
replaced the canonical alias without deleting that source/raw snapshot.

Raw remains remote; compact ticket packet is below512KB/file and2MB total.
No external decoded-resource42, geometry4 capture or Test262 qualification is
claimed. Runtime source review closed the authentic RTL/vertical flow-start
finding. Next independent proposal: genuine grid margin-box/signed-overflow
alignment; current historical family counts are not used as a fresh baseline.
