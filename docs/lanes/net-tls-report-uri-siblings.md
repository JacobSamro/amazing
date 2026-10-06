# CSP report-uri sibling admission

Recorded ready base `5efe03ecd6ebf0cbeacca0a4dc24aa55a06f8f58` contains the
independently published prerequisite `582b84dc` (published as `5efe03ec`), original
report/task lifecycle and reserved Report routing. This ticket changes only native
Fetch Reporting producer paths and includes no dependency history.
No Cargo changes or new dependencies. No copied code or new notices.

Primary checked 2026-10-06: CSP 5.5
https://w3c.github.io/webappsec-csp/#report-violation resolves each legacy token,
skips invalid URL tokens and fetches each independently while ignoring its
response. A later rejected request cannot cancel an earlier admitted sibling.
Report-to presence suppresses the legacy route even when its group is empty or
unregistered. Reserved Report routing, original origin/client/security/referrer,
same-origin credentials and redirect:error remain unchanged.

Permissive port search found Rust header-policy builders, not a verified native
browser submission/lifecycle algorithm compatible with this transport. Servo
is reference-only MPL. This is an adaptation of our existing request builder
and bounded dispatcher onto the primary per-token loop, not copied third-party
code or a replacement transport.

The prior qualified atomic rollback test ID is retained, but its specified
expectation is corrected and strengthened with a real first-sibling POST and
terminal response. Its old passing assertion/code/evidence stays on `582`/`7e`;
no old engine failure is fabricated. Existing malformed-token skip and violation
URL resolution were already implemented and are preserved, not invented gaps.

`queue_csp_report_admissions` and `queue_prepared_csp_report_admissions` expose a
move-only `CspReportAdmissions`: original document, route, ordered token index,
parsed supported endpoint when present, and typed rejection or actual ReportId.
The record retains original native lifetime and prepared scalar credit until
consumed/dropped. It creates neither a global nor another report capability.
Invalid/unsupported/credentialed URLs have an InvalidEndpoint observation.
Existing Vec-returning APIs keep compatibility: return all admitted IDs when
any succeeded; otherwise preserve the first typed policy/transport/quota error
or NoEndpoint when all tokens were skipped. No partial IDs are hidden behind Err.

The 64 KiB serialized application queue credit remains unchanged and is checked
before each individual submit. Rejected siblings consume no delivery credit;
admitted siblings remain in the real dispatcher and finish normally. Prepared
snapshot credit remains separately bounded and now stays with the move-only
outcome record. This does not claim exact allocator/TLS/socket memory totals.

Whole native controls retain original receiving/Report/certificate/output/CSP
families and add real peers for partial capacity, malformed/unsupported tokens,
credentials, duplicates/order/history-relative URLs, mixed-content denial,
actual queued-byte pressure from a genuine test report, original retirement and
foreign service, all five redirect errors with successful siblings, modern
presence/empty/missing groups, and asynchronous terminal transport failure.
No installed Window/SPVE/Worker completion or disposal proof is claimed.

Full source and gate command/profile/storage/baseline receipts must freeze
before native allocation. Whole workspace + doctests + full original native
corpus + original eleven-directory CSP selection are required. Qualification
numbers belong in the compact evidence only after actual native/fetch exit.

Initial whole source `4d01964b` ran all controls and failed only the new capacity
fixture. Its data URI comma was actually a serialized CSP policy separator, so
its final /two did not belong to report-uri. The corrected capacity fixture uses
a comma-free unsupported FTP URI; all four-token admission assertions remain.
A separate real-peer control retains the original comma input and verifies two
parsed policies, three actual report-uri tokens and only the genuine /one POST.
No production behavior or original assertions are weakened by this correction.

## Qualified native result

Exact corrected source `bfc757ff` passed whole workspace 5,199 (17 skipped),
doctests 39 and full native TLS/net/fetch/process corpus 844 (3 skipped), gaining
nine controls with zero lost original IDs. Whole eleven-directory pinned CSP
retained all original 32 passing files/81 passing subtests. Native job
`9a443c288dfc4d63be7dd194d4d80e95`, controller 41511 and compact fetch 22853
all exited zero. All six phases/strict collectors/storage passed; raw, source,
generated files and all 1,109 ELF artifacts stay on the original sandbox.

The full successful `7e778e27` qualification supplies the before maps (engine
bytes exactly match prerequisite582); its actual source identities remain
unchanged in the compact envelope. The storage artifact reference separately
retains the genuine `d29` BEFORE source/profile/host without pretending those
ELFs were measured as7e. Initial4d complete failure and every ID/hash remain
losslessly available. No test/tooling retry changed production assertions or
waived a loss; the correction fixed the malformed fixture's policy boundary.
Root alone runs the authoritative merged-current workspace/full16 gate and
publishes. Installed Window/SPVE/Worker proof remains outside this native ticket.
