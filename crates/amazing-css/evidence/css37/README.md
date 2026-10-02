CSS37 contextual blockification, fixed CSS36 3347187d base.

Boat native /tmp source+target; workspace3534/17skips +4doctests, CSS375.
Identical html/body/div computed-target runner on both sides: parsing3779→3783/5126, four exact gains/zero lost.
Full static display27/35, tables59/82, sizing91/138 and contain108/229: all484 classifications unchanged/zero lost.
Before/after maps are bound to the actual renderer artifact. gate.sh reproduces the after gate; gate.json records numbers.

Computed display/float normalization and original-display metadata are consumed by the layout lane separately.
