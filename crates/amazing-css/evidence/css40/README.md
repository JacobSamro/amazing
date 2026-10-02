CSS40 whole CSS Fonts4 font-variant feature family, fixed CSS39 209bcc86 base.

Seven separately cascading axes, unordered feature grammar/conflict rejection,
normal/none resets, deferred custom-property expansion, independent inheritance,
font shorthand reset and derived idempotent serialization. Ligature, numeric,
East Asian and alternate feature groups serialize in grammar order. Alternate
feature names reject reserved CSS-wide/default names and retain case.

The static parser now evaluates quoted JavaScript literal concatenations; the
old runner treated the quoted + expression as one bogus CSS value. The SAME
revised runner is overlaid on the before tree and used for both measurements.
Dynamic expressions remain explicitly skipped. Font shaping consumers receive
existing longhands; this ticket does not claim new OpenType shaping support.

Boat native /tmp source+target. gate.sh reproduces full workspace/doc and
15-directory parsing corpus plus complete static css-fonts zero-loss render.
Measured results and artifact binding are recorded in gate.json.

Final: CSS385+literal-chain example1, workspace3544/17skips+4doctests; identical runner4105→4126/5126 (+21/lost0). Wholefonts24/32 before/after, all32 unchanged/lost0.
