# Actual DPR raster lattice

Prepared on recorded origin/dev `7b5a62c10ea9d2588356d83be982a8792eba639c`.
Unbuilt source; no readiness or measured gain is claimed.

CSSOM View defines CSS pixels separately from device pixels. The embedding
provides actual configured CSS viewport, logical output extent and device scale
independently. `RasterViewport::new` checks finite positive values, the painter's
real f32 coordinate representation, checked ceil physical dimensions, and the
existing physical allocation budget. Full-page output is permitted to differ
from the visible configured CSS viewport. Surface dimensions never supply the
viewport or density.

Primary source read 2026-10-05:
https://drafts.csswg.org/cssom-view/#dom-window-devicepixelratio
Own implementation; no copied upstream code or added dependencies.

Layout, computed CSS, media, shaped glyph identity, variations and external
scroll/transform samples stay in CSS coordinates. The private raster lattice
inverse-maps physical centers onto authentic source geometry. Full, raw-local,
baked range and retained rasters share that lattice. Metadata bounds, clip
shapes, filter lengths and affine translations convert once into physical
compositor coordinates; affine linear components are unchanged by uniform
conjugation. View identity includes the entire viewport/density context, even
when two contexts have equal ceiled physical extents.

`GlyphRasterScale` is validated separately from `GlyphCacheKey.size_64`.
The native provider rasterizes at CSS ppem times actual density, with device
phase and density in its cache identity, preserving source font generation,
variations and synthesis. The fallback independently evaluates its own rectangle
coverage on the device lattice; it never scales a CSS bitmap. Missing scaled
TextSystem support is a checked error. Existing scale-one entry points and keys
remain supported. Glyph and layer allocation limits are retained.

Generated gradients reuse the original parser/sampling algorithm with exact CSS
concrete size and explicitly supplied device density. Fractional output ceil is
not a new image geometry basis. Background round/space and repeated phase retain
CSS positions; non-unit background gradients evaluate colors directly at the
original source coordinates. No held ColorInterpolation/Canvas/table/CQ/font or
output-demand branches are imported.

Whole matrices cover viewport/density admission, source geometry, physical raw
and baked partial parity, genuine font outline/fallback/device cache and phase,
CSS stops and fractional concrete sizes, decoded repeated phase, transforms,
rounded overflow and opacity, retained/cached compositor parity and density
invalidation. Shell measured viewport/DPR lifecycle and current-error UI adoption
belong to Date; qualification must include the actual shell and whole render
corpus after that adapter closes.
