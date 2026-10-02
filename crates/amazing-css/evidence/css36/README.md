CSS-36 supplies the complete font-size-adjust metric/value grammar with typed
font/geometry resolution seams. CSS and SVG use one parser. Pure math computes
and clamps to nonnegative numbers; from-font and container-dependent math remain
explicit typed inputs until the font/layout consumer supplies real metrics.

Fixedbase f865b7b637796e10dae6e491c34558d9bf07223c, identical corrected CSS-33
runner, pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
Parsing3599→3626/4805,+27 exact,lost0;498files,169nonliteral skips equally.
Workspace3530passes/17skips+4docs;crate371passes. Whole static fonts24/32,
all32 classifications unchanged,lost0. Before map is CSS35's final artifact-bound
render result, the exact baseline code for this series. Boat native /tmp source
and target; viewport800×600,4jobs,20s render timeout.

Six from-font and one container-geometry computed assertions still need the
fixture's loaded font and container. The static CSS runner does not load those
resources. Public font_size_adjust APIs preserve their inputs rather than
substituting guessed font metrics. Font consumers must resolve before inheriting
computed numbers and map the chosen metric into text's size normalization.
