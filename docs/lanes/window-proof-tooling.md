# Actual-window artifact transfer

Base: `fc48838bba9e664746147e4d70437be09b785a4e`.

`linux-shell-capture.sh` now requires `--ref REF` and uses a separate
`lane-fetch-window.sh` interface for single captures, action shots, and bounded
real-pages selections. Each imported window is checked against its actual
running executable and X window PID/geometry before the process exits. Its
manifest records executable SHA, source commit, fixture/arguments, display and
window dimensions, window ID/title, UTC timestamp, and exact PNG path/hash/size.

The transfer validates explicit lane-owned PNG paths, rejects traversal/globs,
duplicate names, oversize images, and size/hash/dimension mismatches. Each PNG
is limited to 512 KiB; the complete manifest plus images is limited to 2 MiB.
Names identify unique proof directories; existing remote or local proof files
are not overwritten. Use `--only` or `--limit` to fit a real-pages selection
within that limit. Images are not resized to meet it.

`lane-fetch.sh` stays compact-only. Raw logs, comparator reports, JSONL, diff
images and archives stay on Boat. Real-pages comparison still uses its existing
classifications and exit status; fetched window artifacts are visual product
proof, independently inspectable and separate from conformance counts. Capture
cleanup runs in a subshell so the supported wrapper's lease cleanup remains in
place.

Local metadata verification: six window-proof fixtures plus nine existing
gate-tooling fixtures passed; shell syntax and `git diff --check` passed. The
fixtures cover exact metadata, invalid paths, per-image/aggregate budgets,
hash/size/window mismatch, missing identity/timezone, duplicate images, and
all three public caller routes. No local engine build, Boat allocation, or
actual product smoke was performed for this tooling ticket. An actual window
proof still requires the supported capture command on an authorized sandbox.

Example:

```sh
bash scripts/linux-shell-capture.sh js-date --ref HEAD --name product-check \
  --size 800x600 --real-pages --only news-static
```
