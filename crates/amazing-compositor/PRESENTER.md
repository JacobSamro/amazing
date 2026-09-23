# Compositor frame presenter contract

`Compositor::build_frame` produces a complete scene description for one viewport. It
does not flatten or display pixels. A presenter implements `Presenter`, applies
`frame.resources`, then draws pass 0 in quad order. `ReferenceCpuPresenter` is the
pixel oracle for this protocol.

## Tile resources

Each `ResourceId` identifies one tile bitmap. IDs are monotonically allocated and
never reused by a compositor. An upload replaces the entire bitmap for its ID;
an eviction ends its lifetime. The presenter must apply evictions and uploads
before reading any quad in that frame. A frame can refer to resources uploaded
in earlier frames. A new presenter needs a fresh compositor or a complete
resource snapshot; a midstream frame alone cannot reconstruct its resources.

Upload rows run top to bottom, pixels left to right, with no row padding. Each
pixel is **sRGB, straight-alpha RGBA8**. The reference presenter interpolates
samples in premultiplied RGBA, converts the sample back to straight RGB, then
blends with integer alpha rounding. A GPU backend may store premultiplied
texels internally, but must avoid sRGB conversion or double premultiplication
that changes the intended compositing result. Filter at tile edges using
adjacent tiles or a texture atlas with gutters; a transparent edge clamp is not
equivalent to the reference presenter.

## Coordinates and passes

Rects use logical device pixels at the supplied viewport scale, with a top-left
origin and half-open coverage. `Matrix4` is row-major and maps column vectors;
the transform already includes property-tree scroll and transform accumulation.
Evaluate coverage at pixel centers `(x + 0.5, y + 0.5)`. `TileQuad.rect` is the
tile's local-space rectangle. `content_rect` bounds its layer content.
`clip_rect` is a conservative device-space scissor. Rounded clip shape coverage
requires the included property trees, not only the scissor.

Pass 0 is the root. A `PassQuad` references a later pass by ID. Draw that pass
into a transparent surface and composite it at the `PassQuad` position using its
opacity and blend mode. Quads are in back-to-front order. A tile inside an
effect pass has unit opacity and normal blend; the pass applies the group
effect once. `SolidColorQuad` is a device-space fill. Passes may be cached by
the presenter, but each frame's ordered quads and property trees are canonical.

## Damage

`damage` contains conservative device-space rectangles where the output may
have changed. The first frame and viewport-size changes damage the whole
viewport. Empty damage permits reuse of the prior presented result. Resource
uploads can occur with no visible damage only if they affect tiles outside the
current viewport; a presenter still applies their deltas. Presentation time is
excluded from `build_frame` timing.
