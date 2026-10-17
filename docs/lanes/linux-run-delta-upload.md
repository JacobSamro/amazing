# linux-run delta source upload (2026-10-18)

`scripts/linux-run.sh` used to upload the full source archive (about 75 MB) on every Boat run. With
many concurrent runs the Mac uplink collapsed and uploads hit the 900 s timeout.

Now each box keeps up to three verified source archives under `~/amazing/src-cache` (one regular
file plus a small meta file per entry, so nothing is written file by file through the lazy HOME
filesystem; HOME survives stop/resume). `scripts/src-delta.py`:

1. Mac: builds a manifest (`path -> mode/sha256`, symlinks by target) of exactly the tree the
   archive is cut from. The full archive is now cut from that file list, so both paths describe the
   same files (the old `--exclude='./target-*'` also dropped nested `target-*` files; macOS `._`
   files are no longer added).
2. Mac asks the box for its cache entries (one short ssh), chooses the entry nearest the target by
   `git rev-list --count a...b` (working-tree runs use their HEAD), fetches that entry's manifest
   once (kept in `.agents/.src-manifests`), and tars only the changed/added files plus a deletion
   list and the changed manifest entries.
3. Box: extracts the cached archive in `/tmp`, applies the delta, recomputes the manifest of the whole
   tree and requires its identity hash to equal the Mac's; only then does it write the pipeline's
   `source.tar.gz` (and registers it in the cache, pruning to the newest three). The bootstrap's own
   archive sha check then runs on that box-built archive unchanged.
4. Any failure (no cache, base missing/corrupt, mismatch, timeout) falls back to today's full upload,
   which then seeds the cache (`src-delta.py register`). `AMAZING_FULL_UPLOAD=1` forces the full
   upload.

Receipts: `<source-transfer receipt>.delta.json` records mode, source commit, manifest sha256, base
manifest sha256 and label, changed/deleted file counts, delta bytes, box archive sha/bytes and
seconds; `.delta-send.json` is the usual `source-upload.py` send receipt for the delta file. The
Mac-side archive sha still identifies the full archive in `source-transfer.manifest.json`; the
manifest hash is the identity both sides verify in delta mode. `test262-boat.sh` and
`linux-render-wpt.sh` call `linux-run.sh` and need no change. A freshly created box has an empty
cache and takes one full upload first.

Measured on lane box r2-temporal (working-tree runs, uncontended uplink):

| run | source bytes sent | time |
|-----|-------------------|------|
| cold, full upload | 74,658,854 | send 73.8 s (second sample 36.2 s); run 129 s |
| delta, 2 files changed (first version, full manifest shipped) | 477,993 | delta stage 24 s; run 49 s |
| delta, 2 files changed (final) | 20,599 | delta stage 13 s; run 36 s |
| cache entry deliberately corrupted (byte flipped) | falls back to full 74,658,440 | send 36.2 s; run 81 s, run still passes |

Tests: `python3 scripts/src-delta-tests.py` (7 tests: manifest exclusions/modes, delta rebuild,
tampered cache content, corrupted cache archive, wrong identity, keep-three pruning, base choice).
