# Super-read reference ordering

Recorded published base: `32581ff37f54f216d45af864194cab3c96786dbe`. This independent prerequisite contains only the qualified AST `d7c71686` and native compiler `d60248a5` consumer-order algorithms and their original three grouped tests. It contains no tagged-template, source-text, Super-write, eval-declaration or unrelated lane history.

Super references capture actual this, evaluate the raw computed expression, and then read the current HomeObject prototype. GetValue rejects a nullish base before key conversion and invokes the property getter with actual this. Captured registers preserve the receiver and pending computed key across Await/Yield. Subsequent chain links retain their existing receivers and optional propagation. The AST uses existing reference capture/coercion gateways; no opcode, reference, realm or frame schema changes.

The primary contracts are [SuperProperty evaluation](https://tc39.es/ecma262/multipage/ecmascript-language-expressions.html#sec-super-keyword) and [GetValue](https://tc39.es/ecma262/multipage/ecmascript-data-types-and-values.html#sec-getvalue). The complete Lumen Reference/GetValue algorithm at `f6c9860d44fa52249cfd647122edea184d32c863` is MIT (Copyright 2026 Lucid Softworks limited); exact-commit license verified, file headers and appended notice retained.

Original tests execute 26 actual native/forced-AST cases across generator, async and async-generator bodies: raw-key execution before null-base rejection, arbitrary throw identity, current prototype after parking, actual getter and nested receiver, and derived-this TDZ retained by an arrow after constructor unwind. The native branch asserts actual Bytecode bodies; the AST branch forces original parsed body execution. No expectation or test identity is weakened.

Historical complete-area qualification is recorded by `6d5d55b4d8497ae037d3f0ac440d0db34a6e033c`: workspace 4,362 passes/17 skips plus 4 doctests and 16 paired selected reports without file or variant loss. These older counts are provenance, not a fresh gate on this base. Supported-script workspace/docs, 16-shard Test262 and correlated Super corpus gates remain pending. Raw stays on Boat; only compact fresh results will be added.

The first published-base candidate `331e7e70` failed its actual workspace gate: 4,325 passes, all three new native test groups failed, 17 skips; all 4 doctests passed. Compact authenticated failure context is retained in orchestration scratch. All forced-AST cases passed. The compiler's ordinary suspended Member/Call consumer bypassed the corrected chain helper, loading the Super base before raw-key suspension. The successor routes exactly suspended Super read/call spines (including grouped/nested members) through the existing helper. Assign/update/delete, bare SuperCall, direct eval, class heritage and nonsuspending adapters retain their existing consumers. The original26 assertions are unchanged; an additional opcode-order group verifies dispatch for direct, grouped, nested and standalone reads in all three suspended callable kinds.

The corrected earlier fixed-base workspace passed 4,388/17 skips and 4 docs;
its combined Unicode-head workspace passed 4,394/17 and 4 docs. Two old root
16-shard attempts ended in tooling failures; they remain unqualified and do
not prove this current-base source. This repackage preserves the exact 859d
AST/native algorithms and the original 26 semantic executions plus all 12
native opcode-order executions. Fresh supported gates are required before
publication; no earlier transport cause is claimed fixed.

## Fresh current-base owner qualification

On published base `32581ff37f54f216d45af864194cab3c96786dbe`, tested
engine source `2eea15ee8b4fbe279bdfee22430ca9a92d6cff4b` passed workspace
nextest 4,455/0 failed/17 skipped and 4 doctests; baseline passed 4,451/0/17
and 4 doctests. All four original test groups retain 26 semantic executions
and 12 opcode checks. Supported Linux jobs completed with actual exit 0.

The 14 relevant directories in both release modes retained all 35,583 passing
files and 67,257 passing variants per mode, with zero lost passing files or
variants. Pin `3655e746`, timeout 60 seconds, jobs 8. Exact source, binary,
runner, input and raw SHA receipts are in
[evidence/super-read-current.compact.json](evidence/super-read-current.compact.json).
Raw remains Boat; final packaging changes documentation/evidence only.
The integrator's fresh full 16-shard publication gate remains required. The
older two tooling failures remain historical unqualified attempts.
