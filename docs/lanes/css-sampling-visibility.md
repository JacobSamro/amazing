# Paint sampling and visibility source closure

Recorded source base: `c4a90149f8bfcec565807bf678431335f5902d64`.
The original coherent qualification `11f180e001264a413cc6512b5cad4442236d4062`
had 4631 workspace passes, three failures, 17 skips, four doctests, 109 GPUI
passes, and all twenty render directories at 1327 before/after passes with
zero lost identities. This successor is source-prepared, not qualified.

The 1264 differing filter pixels were the first three rows and two columns
of the 270 by 230 local effect surface: `270*3 + 230*2 - 2*3`. Visibility was
using local pixel centers with world property trees, and the viewport scroll
clip inherited the transformed descendant's matrix. Local effect sampling
now retains its source lattice while testing the corresponding world center.
Paint captures the actual clip-owner transform and ancestor scroll, and the
scrollport-owner transform. Generic sources keep their existing explicit
item-space scroll API; Paint uses the owner-aware API. Clip culling and exact
point admission share the same owner matrix.

Finite zero-area Content records contain no owned ink. Their item indices,
keys, property records and command ranges remain in the view; only the empty
scene chunk is omitted. Generic invalid scene bounds still fail. The cached
CPU fixture now retains the presenter that received the original uploads;
cached uploads remain empty and an unseeded consumer still rejects them.

Property-tree snapshot version 3 carries ownership. Version 1 lacks it and
version 2 remains the existing unsupported stripped-effects marker. Frame
and resource versions remain unchanged. GPU wire version 3 explicitly carries
clip scroll ownership and optional scrollport transforms, rejects previous
wire versions, and validates all referenced IDs during tree reconstruction.
The effect extension keeps its own version and restores the current complete
tree format.

Original pixel, retained/cache and generic bilinear controls are preserved.
Three additional groups cover owner mutation and snapshot rejection, finite
no-ink admission, and all 45 combinations of five device scales, three signed
translations and three overflow modes with real local filters and cached and
snapshot parity. One wire group preserves/rejects genuine owner references.

Qualification must use one complete workspace/doctest/GPUI and original
twenty-directory render sweep on Boat. Raw output stays on the host; compact
counts, every loss, source/binary/corpus identities and hashes are fetched.
