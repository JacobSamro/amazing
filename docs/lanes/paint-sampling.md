# Painter transform sampling

Recorded base: `00b10ab96cfdaa14f25dbe1dfec33d651045bb9e`.

The DPR control fails actual/full equality at 1.25x: CSS translate(2px,3px)
becomes physical translate(2.5px,3.75px). Full painting inverse-maps each
physical pixel center and fetches the containing source pixel. Ordinary retained
tiles instead interpolated four pixels for fractional transforms.

`RasterSampling` makes this source contract explicit. Generic compositor
scenes keep premultiplied bilinear interpolation. `PaintViewSource` selects
nearest sampling, matching the genuine painter at all densities. Direct render,
frame presentation, caches and both snapshot paths preserve the same selection.
Existing effect-pass local surfaces already use the painter's nearest sample.
A GPU presenter lacking nearest texture selection must use the genuine CPU
composition path for fractional nearest transforms rather than substitute its
texture interpolation. Integer translations require no interpolation.

Original DPR assertions and generic bilinear controls remain unchanged. Added
native matrices cover densities1/1.25/1.5/2/3, positive/negative fractional
translations, alpha, clipping, direct/full/retained/cache and snapshot agreement.
Source is prepared only; Date owns the combined native/docs/GPUI, whole20
render and real-window qualification. No local engine test or competing gate.

GPU process transport uses a separate `SNAPSHOT_WIRE_VERSION = 2` payload
header and explicit sampling byte (0 bilinear, 1 nearest). The unchanged
compositor resource/property snapshot versions follow it. Historical V1
policyless payloads are rejected with a version error; no receiver invents a
policy for a missing field. Unknown bytes, truncated policies and trailing
payloads are rejected. Codec controls round-trip both actual policies through
`GpuFrame::from_compositor` and compare reconstructed presenter pixels. The
IPC fuzz seed explicitly records its authentic generic bilinear policy.
