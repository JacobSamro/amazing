# Template content patching

Recorded published engine base: 8de1d70abea03528a2483b58bc7bfbd8a1a6d3bd.
Qualified shipping source: 878ca8a76e66f85a0f8d1fa0e3331ef436836ffc.

Linux workspace: 4,628 → 4,651 passed, zero failures, 17 skipped; four doctests on each side. All fourteen original WPT selections retain passing assertion multiplicities and passing files, with 379 additional template assertions. Static trees retain 1,930 passes and executed trees retain 1,936; XMLTS retains all 1,864 passes and unchanged residual classifications across 2,377 records. Compact counts, raw hashes and remote paths are in `crates/amazing-dom/evidence/template-content-patching/qualification.summary.json`.

The original test-compilation failure and interrupted profile-mismatch attempt remain raw diagnostics with no qualification credit. The clean corrected AFTER completed with actual exit zero against the completed fixed BEFORE; every measured phase exited zero.

Primary HTML §4.12.3 (reviewed 2026-10-05): https://html.spec.whatwg.org/multipage/scripting.html#the-template-element.

The native template carries one initial-null insertion association, with target/start/end node generations. Detachment/adoption preserves actual identities; stale end independently becomes null; recycled start or target cannot redirect parser insertion. Fresh allocation, clone and cross-arena import initialize new associations to null. The existing GC retirement seam still requires its caller to prove no native association or JS wrapper reaches the node.

`template_insertion`, `set_template_insertion` and `prepare_template_content_patching` are the shared parser contract. Preparation uses existing exact UTF-16 ProcessingInstruction attributes, ordinary DOM descendants/sibling nesting and canonical removals. It snapshots intervening siblings and rechecks each current parent before removal. No second PI parser, HTML state map or observer-derived authority. `HTMLTemplateElement.htmlFor` uses the normal exact DOMString reflection/reaction machinery.

HTML owns parser entry/start/insertion/end routing. Its implementation must remove genuine saved markers and clear state at completion. All original 414 template-for cases and 364 reflection cases stay unchanged. `streamAppendHTMLUnsafe` is a real Streams entrypoint dependency; the current published bindings have no such operation. No fixture-only append wrapper is introduced and no installed streaming readiness is claimed. Current-standard content patching is qualified separately from proposed streaming APIs, which are absent from current HTML §8.5 IDL. Their unchanged corpus failures remain measured; no streaming implementation or complete-pass claim is made.

Incremental parser forward/reverse mirrors translate live template state through their actual node mappings, including detached open templates and script-created detached/foreign markers. This preserves same-node parser custody; ordinary clone/import still initializes state to null.
