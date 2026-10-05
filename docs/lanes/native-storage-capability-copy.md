# Native storage capability and explicit copy

Recorded base: `64f5d9b86c1cda019f73aedb2dd4f7137a3b7663`.
This is pure public tooling; no engine algorithm or Cargo changes.

The viewport baseline job actually passed all 93 GPUI tests and then returned
101 for a bin-only doctest request. Its retention step separately failed EXDEV18
although source and destination both reported st_dev64769. The old target and
raw logs remain intact. Neither failure is relabeled as successful.

`hardlink` now tests the actual link syscall on every retained artifact into a
fresh destination-owned probe namespace before publishing payloads. Device IDs
are diagnostics, not capability proof. Unsupported links fail without copying
or retiring the target. Existing early distinct-device refusal is retained.

The new explicit policy `retention_method: "hardlink-or-copy"` permits copy
fallback only for the recorded unsupported-link errno family. It computes the
unique missing payload bytes before any copy, requires those bytes to fit the
caller retention budget and actual destination free space plus reserve, and
verifies source/destination byte counts and SHA256. Each native probe result
stays in the raw session; compact receipts contain counts, bounded diagnostics
and the raw hash. No error triggers an implicit copy.

Under this new policy, a successful freeze retains the original target by
default. `retire_after_retention: true` is a separate explicit opt-in to the
existing verified terminal-target retirement protocol. The new boolean is
accepted only for the new policy, so an explicit false is never ignored under
a legacy policy. Existing copy/hardlink
policies retain their established lifecycle for compatibility with all 33
original controls. No previously existing proof or unrelated target is deleted.

`publish-raw --directory /tmp/<private-terminal-directory> --destination
<fresh-native-HOME-directory>` invokes the actual rename syscall. An EXDEV
leaves the source intact unless the caller explicitly supplies sufficient
`--copy-budget-bytes`. The guard includes all payload bytes, file/directory
metadata allowance and actual destination reserve. Every copied file is
size/hash verified, source and destination inventories must remain unchanged,
and successful copying always keeps the original source. Existing destinations,
symlinks and special files are rejected. Only a caller that has already observed
its producer's actual terminal exit may publish its private raw directory.
This tool does not certify an unknown producer or expand process authority.

The complete native tooling job keeps the original 33 controls and adds ten
real filesystem/ELF controls: actual EXDEV, no-budget refusal, explicitly
budgeted copy, independent retirement opt-in, original ELF bytes and native
replay, successful hardlink inode identity, real raw rename/copy, existing-proof
preservation and structured plan forwarding. /dev/shm provides real EXDEV;
its noexec policy is respected by verifying the saved ELF and replaying the
same bytes on the native executable /tmp device. No injected errno is used.
Full source is frozen before the bulk Linux job; no local build or test.
