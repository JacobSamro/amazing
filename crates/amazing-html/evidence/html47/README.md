# HTML47 persistent parser document modes

Fixed dependency base 7a299c09 (NET43 a94e4587) after HTML46 40f6e604 and authorized DOM dependencies
4ce0126d (localf66c6de0),20a85c17 (localf17cdc49). Pinned WPT
c2a2fce69e5c2509d9dc28440ec8b42532e5c293.

Doctype classification now uses the exported DOM QuirksMode and records it
on the parser's document root when selected. Missing/malformed doctypes
select quirks; limited quirks stays distinct although both non-quirks modes
expose CSS1Compat. Srcdoc is locked to NoQuirks. Existing complete, byte,
incremental and resumable parse paths share classification; scripts observe
it at their first parser pause. Document clones preserve the mode.

The owned iframe adapter copies root mode after every incremental graft and
eager import, so child scripts/compatMode see it before execution. A sandbox
without allow-scripts configures ScriptingMode::Disabled, enabling the full
noscript fallback parsing delivered in HTML45. No DOM importer changes.

Meaningful regression coverage: all three modes, legacy/missing doctypes,
every text/byte split, complete bytes, clone, pre-script classification,
srcdoc/fragments, actual iframe script observations, and sandbox fallback.
No new upstream port; the existing attributed doctype classifier is unchanged.

Final native Boat gate: **571 owned passed/6 skipped, 3546 workspace
passed/17 skipped, four doctests passed**. All62 html5lib files retain
**1894/1894 CLI passes** and the expanded probe **1930 passes/6 unsupported**,
with identical corpus hashes and zero lost passes. Full194-file encoding sweep
**11755->12216 passes,867->406 failures**, zero former passes lost; the461
gains include the adopted DOM/NET dependencies and are not credited as HTML47
parser gains. File classifications25pass/19fail/1timeout/8error/141unsupported.
The parent-granted existing driver style fixture now explicitly declares a
doctype to preserve its CSS1Compat intent, while a new owned installed-binding
test proves missing-doctype BackCompat plus standard/limited CSS1Compat.
