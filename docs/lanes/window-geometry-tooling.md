# Whole-window capture geometry

Fixed base: `7bc73204149e50b8875c0ab9d1b552cb4e1fe251`.
This tooling ticket also includes the source-only host-Bash assembly correction
from `58beaa62`; the original failed smoke and its raw output remain unchanged.

The prior producer imported a window and then assumed its PNG dimensions must
equal the full client geometry. ImageMagick's actual
[XGetWindowImage implementation](https://github.com/ImageMagick/ImageMagick6/blob/main/magick/xwindow.c)
clips the requested client rectangle against the root display before XGetImage.
[Import's public interface](https://imagemagick.org/import/) also distinguishes
window frame/border inclusion. Thus an off-screen window can produce a smaller
PNG; resizing that PNG would falsely claim full-window proof. The failed smoke's
actual bounds were not saved, so this source fact does not establish its exact
historical geometry.

The dedicated Xvfb harness now sizes the genuine X window to the explicitly
requested viewport and moves it to the display origin before input actions.
Every shot uses one shared producer: it captures PID, executable SHA, X window
ID, root-relative client bounds and actual display dimensions before import,
rejects any off-screen portion, invokes import with frame/border disabled, and
rechecks all identity and geometry afterward. PNG dimensions must equal the
complete unchanged client size. These observations and the capture interval
are recorded in the image's provenance and validated again during transfer.
No PNG resize, crop, dimension tolerance, or reference change is introduced.

Capture failures emit a separate compact diagnostic with actual observed bounds
and, if an image was created, its dimensions/size/hash. They cannot enter the
successful image manifest. Invalid image paths also cannot publish a failure
file outside the lane output directory. All raw output remains on Boat.

Local metadata-only checks: 12 window fixtures and 9 existing gate fixtures,
shell syntax and diff checks. Fixtures include all ten shot transactions,
full/partial/off-screen bounds, display mismatch, geometry/PID changes during
capture, foreign window/PID rejection, PNG cropping, exact hash/size/budget
checks, and real host-Bash assembly. No local engine build or sandbox run was
used for source preparation. Actual Boat qualification is a separate step.

The first actual geometry qualification produced all ten verified remote shots,
but transfer failed because SSH read the remaining image-list stdin. The
supported transfer now gives both SSH and SCP null stdin. An end-to-end
metadata fixture with stdin-consuming mock transports verifies all ten exact
images are published and checked. This ticket permits one clean qualification
rerun; the original remote proof is unchanged.

Actual Boat qualification completed on exact source `4e722311` using the
original ten-action fixture and 1280×800 dedicated display. All ten full-window
PNGs passed identity/geometry/hash verification and were inspected. Back/forward
navigation returned the expected status/new-tab pages; find showed `Amazing`
with 1/2 matches and visible highlights; closing the new tab returned to the
seeded ECMA-262 tab. The actual rendered engine surface is 960×570 inside the
1280×800 shell. No engine or fixture code changed.

A separate `news-home` window proof also passed capture/transfer checks and was
inspected: Daily Ledger header/navigation and six story-card thumbnails are
visible, with lower content clipped by the actual engine viewport. Its existing
comparison reported mismatch (22,338 absolute-error pixels), and the wrapper
returned 2. This is retained, not converted to conformance success. The clean
smoke returned 0; the first qualification's transfer-only failure is preserved.

Compact per-image hashes, executable provenance, actual exits and proof paths
are in `window-geometry-qualification.json`. PNGs remain in ignored local
visual-proof scratch; raw logs and comparator output remain on Boat. The two
qualified source commits remain preserved on `lane/window-geometry-tooling-ready`;
the READY squash has identical tooling bytes plus this compact report.
