# RC-1 revised — image adapter and measured render #7 scope

Form controls were removed from RC-1 by the integrator. Layout owns LAYOUT-2
(box construction); the integrator owns GPUI control descriptors. CSS color
parsing is unchanged pending CSS-2. No media-element lifecycle is implemented.

## Measured priority

WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, baseline engine
`e9c1e21`, 800×600 at scale 1. The dedicated selection is now committed under
`tests/wpt-replaced-content/`; it replaces the earlier inconclusive screen of
the unrelated 60-case paint diagnostic. Eight SHA-ranked HTML match candidates
per cluster produce 48 entries / 41 unique upstream tests. Direct source
markers overlap; the full inventory and selection are retained for audit.

| Cluster | HTML candidates | Scripted candidates | Pass | Mismatch | Unsupported | Renderer error | Non-pass rank |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Canvas | 533 | 532 | 0 | 0 | 0 | 8 | 1 (8/8) |
| Controls | 40 | 26 | 1 | 2 | 0 | 5 | 2 (7/8) |
| Images/fitting | 308 | 72 | 2 | 5 | 0 | 1 | tied 3 (6/8) |
| SVG | 353 | 82 | 2 | 4 | 1 | 1 | tied 3 (6/8) |
| Embedded content | 132 | 23 | 2 | 5 | 0 | 1 | tied 3 (6/8) |
| Document media | 61 | 13 | 2 | 5 | 0 | 1 | tied 3 (6/8) |

Timeouts: zero throughout. Totals: 9 pass / 21 mismatch / 0 timeout /
1 unsupported / 17 renderer errors (the runner's `crash` classification).
Before and after counts are identical. These rank observed non-passes in the
bounded sample, not estimated global feature gains. Matching pixels can be
inert; renderer errors include missing JS prerequisites. No causal assertion
that implementing one surface fixes every case in its cluster is justified.

Dependency-aware sequence:

1. Deliver bounded raster-image decoding and fitting now; this needs no new DOM
   lifecycle and is the revised ticket's owned implementation.
2. Deliver controls through LAYOUT-2 plus integrator-owned presentation/state
   descriptors; the measured sample has 7/8 non-passes.
3. Establish DOM/JS canvas/context/worker bindings before Canvas 2D rendering:
   all eight selected canvas cases currently exit with renderer errors and
   532/533 screened canvas candidates contain scripts. In parallel, scope
   SVG geometry/paint independently; it ties the image/embedded/media sample
   in non-pass count and should not be smuggled into the image decoder.
4. Add embedded-document surfaces after document lifecycle, origin and process
   seams; add HTMLMediaElement after DOM state/events/timing. Equal measured
   non-pass counts do not establish an order between those two.

## Implementation and ownership

- `amazing-image`: safe adapter using exact `image` 0.25.9 with PNG/JPEG/GIF/
  WebP/ICO/BMP/PNM enabled, explicit format allowlist, input/dimension/output
  caps, decoder allocation budget and panic containment. Output is the owned
  `amazing_media::Image`; dependency types never cross the adapter.
- Integrator-directed placement: `amazing-image` is a member of the standalone
  `amazing-shell-gpui` workspace and shares its lockfile. It has no separate
  workspace marker or lockfile. Root workspace membership/dependencies stay
  unchanged. Cargo metadata and the locked shell check accept the arrangement.
- `amazing-media::ImageSize`: natural dimensions, definite axis overrides,
  one-auto-axis aspect ratio, explicit zero, invalid/overflow rejection.
- `amazing-paint::image`: all five object-fit modes; content-box clipping,
  keyword/percentage/pixel object positions and common edge-offset syntax;
  provider hook plus `StyledImages` retains per-inline image presentation.
  Unsupported position expressions such as calc() fall back to center.
- Raster sampling is bounded, validates buffer sizes and preserves straight
  alpha through effect layers. No paint color-parser changes.
- Minimal authorized shell patch delegates decoding, uses the size helper,
  passes computed image styles into paint and caps retained image RGBA at
  128 MiB. GPUI still owns presentation and keeps its Frame/RgbaImage use.

Limits default to 16 MiB encoded input, 8192 per dimension, 64 MiB returned
RGBA and 128 MiB decoder working budget per image. Codec-internal limits are
best-effort; no total-process memory bound is claimed. Animation playback,
EXIF orientation and ICC conversion are outside this slice.

## Evidence and boundary

The static PNG diagnostic moves 2/12 raw matches to 0/12, with 12 mismatches
and no timeouts/unsupported/errors after decoding. The two former matches
were inert: references still omit background-image painting, but actual images
now paint. Floats and inline CSS geometry remain additional dependencies.
This is **zero credited WPT gains**, not a completed CSS Images conformance gate.
The unchanged focused paint selection remains 24/24 in both repeated runs.
The 48-entry scope selection remains 9/48 raw passes. Full results and hash
projections are under `tests/wpt-replaced-content/`.

Windows validation: paint 62 passed, media 7 passed; image adapter 4 unit plus
3 integration tests passed; full root workspace 733 passed, 0 failed,
3 ignored; standalone shell `cargo check --locked` passed. The workspace
excludes the shell/adapter, which are validated separately.

Integration follow-ups: root driver PageMedia still decodes BMP/PPM only and
needs an explicit product decision before adopting shell-side codecs. Layout
must preserve CSS used image sizes, borders/padding and containing-block
constraints for inline replaced content. Background-image painting and floats
are prerequisites for crediting the retained upstream PNG reftests. Resolve
these in owning lanes; do not modify upstream references to obtain passes.

Implementation references: [CSS Images 3 object fitting](https://drafts.csswg.org/css-images-3/#the-object-fit),
[image 0.25.9 reader](https://docs.rs/image/0.25.9/image/struct.ImageReader.html),
[decoder limits](https://docs.rs/image/0.25.9/image/struct.Limits.html).
