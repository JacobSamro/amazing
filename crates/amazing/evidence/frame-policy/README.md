# Navigation-only frame embedding checks

The canonical NetworkService checks X-Frame-Options and CSP frame-ancestors only when the actual pending request mode is Navigate. Final navigation responses remain checked; redirect heads remain skipped. Script, Image, Font and Fetch responses do not apply frame embedding policy. Initiating request CSP is unchanged.

Fixed baseline7e4dfece contains published660e9a15 plus identical READY CSS54/55 and whole READY HTML56. Source0c59ee53 plus fixture-only8a47da1d; no facade/session/library source is included. The grouped actual HTTP regression covers navigation denial, all four non-navigation destinations accepting the same headers, redirect following and permissive final navigation.

Linux native4113→4114 passed,17 skipped; four doctests on both sides. Whole pinned fetch/api183 files at60s/jobs8:40→41 passing files,1291→1297 named passing assertions, zero lost files/assertion multiplicities. The six added cases are an unchanged-source deadline recovery already passing in both sequential selections, not a causal policy gain claim. Actual NET37 list is61 files (its old comment says62):58 passing files and1045 named assertions unchanged, zero loss. All input IDs/source hashes and binary hashes are retained under results/.

Run through MAIN scripts/linux-run.sh integrator-runtime --host boat --ref5feca69f -- 'bash crates/amazing/evidence/frame-policy/gate.sh'. The first immutable BEFORE completed before the new redirect fixture correction; corrected AFTER used FRAME_POLICY_AFTER_ONLY=1 with exact BEFORE service hash validation. Final comparator488cd0e3 fixes only the stale file-count assertion, validating the actual declared IDs against all raw records; no engine run was repeated for that metadata fix.
