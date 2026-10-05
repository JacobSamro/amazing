# Inline CSP producer

Recorded base: `02855a2b4cd9ca54e0210dff9df117a5da235323`.
Primary source: https://w3c.github.io/webappsec-csp/, editor draft 16 September 2026;
verified upstream HEAD `b1bad631b091cdca904138d6947a16b89edfd2b4`.

`CspPolicyList::check_inline(&CspInlineContext) -> CspInlineCheck` implements
CSP3 4.2.3, 6.7.3 and 6.8.2 for script elements, script attributes, javascript
navigation, style elements and style attributes. Policy fallback, nonceability,
nonce/hash source grammar, exact nonce comparisons, UTF16-to-USV/UTF8 hashing,
unsafe-inline suppression, unsafe-hashes and current non-parser-inserted inline
strict-dynamic behavior are native decisions. Existing std-only SRI SHA2 is
reused; no dependency or upstream algorithm port is introduced.

Delivery provenance distinguishes Header/Meta and survives policy cloning.
Response parsing discards policies whose directive set is empty (CSP3 2.2.2),
so an empty CSP header does not trigger HTML nonce hiding. An empty source list
within a real directive, such as `script-src`, is a real header policy.
Meta parsing strips report-uri/frame-ancestors/sandbox and never invents a
report-only policy. HTML owns connected-head/prospective meta delivery timing.

Context source, nonce and attribute strings are exact UTF16. Caller must use
actual element parser-inserted state, internal cryptographic nonce, nonce
attribute presence and tokenizer duplicate-attribute provenance. HTML/DOM own
producing those native states; hidden getAttribute nonce is not substituted.
Document policy selection, origin and document generations belong to the driver.
Opaque element identity is returned unchanged for the driver's generation-fenced
report queue. Reporting URLs remove credentials/fragments, nonHTTP schemes only;
legacy violatedDirective equals effectiveDirective, while source_directive records
fallback for native diagnostics. All policies produce independent violations,
and report-only never blocks. Element samples retain exact first 40 UTF16 units.
Navigation reporting uses
CSP3 4.2.4: no element or sample is attached. Declared report-to values retain
presence even when empty; the reporter must prefer report-to over report-uri
when present, then validate/resolve the group and endpoints.

The caller queues actual securitypolicyviolation delivery, rechecks connected
shadow-including-root at dispatch, and handles report endpoint resolution and
transport. This lower crate produces decisions and report data; it does not claim
browser event/network delivery on an unintegrated driver.

Validation: independent FIPS/hashlib vectors and complete native inline-policy
controls; whole pinned WPT source-file selections across default-src/script-src/
script-src-attr-elem/unsafe-hashes/nonce-hiding/meta/parsing/securitypolicyviolation/
navigation/style-src/style-src-attr-elem. Existing runner platform exclusions remain
explicit unsupported, never passes.

Final Linux source `3283059369f61ade0bf909f3394448d7631709b0` and base above:
workspace 4628 → 4642 passed (14 new native groups), 17 skipped both sides;
doctests 4 → 4. All five final phases exited 0; captured compact fetch exited 0.
Pinned CSP selection: 220 files, 32 pass / 27 fail / 66 timeout / 24 error /
71 unsupported both sides; 81 passing subtests, 136 failing subtests both sides.
Exact lost and gained file/subtest lists are empty. This lower producer has no
installed-browser CSP pass gains. Compact receipts, classifications and remote
raw hashes/paths are in `crates/amazing-fetch/evidence/inline-csp/`.

An initial candidate failed SHA2 digest visibility (E0603); its stale-binary
corpus result was rejected. The corrected whole gate passed; the final whole
gate then qualified navigation/report-to normative reporting fixes. Raw output
and failed branch `lane/net-inline-csp-failed-968a16e4` are preserved.
Integrator still owns the merged-candidate 16-shard Test262/workspace gate.

Integrator candidate `4df3d64d5235d4e52c50f57ceb57eb0ff3356c3f` on published `0409c41f9f1b4c195b7e5a92e7dff33189eab6d8`: 4,698 workspace tests passed, 17 skipped; four doctests passed. Full 16-shard pinned Test262 retained 52,666 passing files and 101,179 passing variants, zero losses. Five shipping engine/control files match the qualified owner source exactly. Compact root proof: `docs/lanes/integrator_inline_csp_20261005.json`. Browser consumer installation remains the separate whole ScriptElement integration.
