# LAYOUT-3 final gate evidence

Measurements use Windows `win-001`, lane `layout`, an 800 × 600 viewport,
and WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The matched baseline is dev `79464ca`; the implementation is `7de4c47`
(rebased from predecessor `a0157c3`, with identical layout source).

Only the `baseline-recovery` and `final-recovery` builds and the runs below
`C:/Users/server/target/layout/layout3-recovery-evidence` count as this gate.
Earlier developmental, predecessor release, and `*-finish` runs are superseded.
An external refresh at Windows local 2026-09-20 12:09:51 replaced CSS/DOM and
seven layout files after the first workspace gates. Collection rejected that
source drift. Although all four pinned executables were unchanged, this lane
restored its exact commit with a uniquely named archive, rebuilt both baseline
and final from cleared first-party caches, and restarted every measurement in
the separate recovery directory. The original finishing epoch is not credited.
Restoring an archive leaves newer, extra files behind: two CSS integration tests
and five DOM source modules from the external refresh caused the first recovery
workspace build to fail against the older pinned API. Those files and leftover
AppleDouble metadata were moved to a lane-local quarantine, not deleted. The
successful recovery build follows that cleanup; collection now checks the exact
Rust/Cargo source set in addition to file hashes.
`summary.json` records measured counts and stable normalized result/pixel
hashes. The individual reports preserve actual/reference/diff SHA256s.

The lane ran `bash scripts/refresh-lane-trees.sh layout`, checked the refreshed
source hashes, temporarily synced baseline layout source, built the baseline,
then re-scp'd all final layout source. Both builds removed first-party cached
artifacts and fingerprints and disabled incremental compilation. Build records
include compiler freshness and source verification before/after commands.
Source manifests cover tracked Rust source and Cargo manifests/lockfiles.
The final workspace gate runs twice; the nine WPT selections run twice for
both baseline and final, using immutable named executables.

## Reproduction

Run `../build-layout3.ps1` on Windows with `-Phase`, `-SourceManifestPath`, and
`-BuildImages`; use `-FullWorkspace` for the final build. Source manifests must
match the intended checkout before starting. Then invoke `../run-layout3.ps1`
with the pinned driver and image executable paths, the complete lane-local
WPT checkout, a new output directory, and `-Phase baseline` or `-Phase final`.
It runs both repetitions by default. Do not reuse an output directory from a
previous measurement. `../collect-layout3.ps1` collects this named gate epoch
and verifies source, executable and log hashes plus command exit codes. Then run
`../summarize-layout3.py collected.json output-directory` to compare repetitions.
This Python step analyzes retained evidence only; engine builds/tests run on
Windows exclusively.

## Pixel review and limits

The retained `review/` images were created by the predecessor from actual and
reference WPT PPMs. The final gate rechecks those original PPM hashes; the PNGs
are lossless conversions, not browser screenshots or invented reference images.
Both cases lose raw pass credit, and remain mismatches:

- `baseline-empty-cell-001`: baseline test/reference both show two full-width
  stacked rectangles. Final output has table-sized cells, but the empty-cell
  baseline placement still differs. The old equality did not exercise correct
  table layout.
- `table-row-group-nested-anonymous-001`: baseline test/reference both stack all
  four cells vertically. Final output places them on one row, with a remaining
  gap at the nested anonymous-group boundary. The old equality was insensitive
  to the intended table-row behavior.

The four `collapsing-border-model-011` through `014` raw gains do not establish
complete collapsed-border paint support. Paint has not adopted the resolved
`table_decoration()` seam. Inherited `border-spacing` and side-border shorthand
expansion remain routed CSS follow-ups. See the lane decision map for the
bounded layout contract and other unsupported cases.
