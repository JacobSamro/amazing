# RC-4b: fixed-base pinned render remeasurement

Fixed dev source: `253d9ec56a3612df8e495ea0cac49813fffbe2ee`; WPT pin `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; platform win-001. The primary absolute refresh script archived this `origin/dev` revision into only the replaced-content tree. All 339 tracked Rust/Cargo/harness/fixture inputs matched the local fixed base, and root plus isolated-image metadata passed with `--locked`. A shared REFRESH-HOLD protected both builds and the sweep. Both the build and the full sweep acquired slots through the primary absolute `with-build-slot.sh` invoked with `bash`.

**Before** means this same dev source with only the five RC-4 source-file changes removed: four files were overlaid from merge `b402d84`'s first parent (`ed6a44c`) and the newly added `svg_transform.rs` was absent. Both renderers were built in separate target directories. The files were then restored in a `finally` block, and all 339 current-source hashes matched again. **After** uses the unmodified fixed dev source; both renderers were rebuilt. Executable SHA-256s, sizes and build times are in `artifacts.json`; the before file hashes and after restore check are separate evidence. No local build/test was run.

Each of five pinned selections ran twice in each phase. All ten pairs have stable classifications, pixel differences, and actual/reference/diff SHA-256s. Every before per-case projection equals its after projection. Counts in **each** of the four run sets:

| Selection | Pass | Mismatch | Error | Timeout | Unsupported | Renderer error |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 48-entry scope | 26 | 4 | 0 | 0 | 0 | 18 |
| 24 controls | 22 | 2 | 0 | 0 | 0 | 0 |
| 12 PNG | 12 | 0 | 0 | 0 | 0 | 0 |
| 12 SVG raw | 12 | 0 | 0 | 0 | 0 | 0 |
| 24 focused paint | 24 | 0 | 0 | 0 | 0 | 0 |

**No classification, pixel hash, or WPT count moved** between before and after. The current after per-case projections are also identical to the older `ffb143b` after packet; its report format lacked the now explicit `error: 0` count field. The SVG matches remain raw only: unsupported inline CSS means **zero conformance credit**. The owned 9-case fixture proof was outside this five-selection ticket and was not rerun.

`repeatability.json` contains every projection hash and the empty per-case change lists. The 20 collected reports preserve complete classifications and actual/reference hashes. This is a fixed-base RC-4b measurement, not a broad WPT conformance claim.
