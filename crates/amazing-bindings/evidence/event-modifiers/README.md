# DOM-B UI event modifiers ready

Implementation `7e014f57`, fixed base `88f37385`. MouseEvent, WheelEvent and
KeyboardEvent now retain all fourteen modifier flags and expose case-sensitive
getModifierState. Numeric dictionary conversion applies the specified signed and
unsigned widths, preserves fractional coordinates, rejects non-finite doubles,
and includes inherited UIEvent.which and WheelEvent.momentum.

Linux workspace **2758 passed /10 skipped**, doctests **4/4**. Complete
**170-source dom/events + uievents/constructors** corpus: **360→376 passing
assertions**, **260→244 failing**, **16 gains /zero lost passes**. Files:
51→52 pass,37→36 fail,20 harness errors and62 timeouts unchanged.
The modifier-state constructor fixture is now **16/16**.

InputEvent target-range sequence conversion awaits a JS iterator/closing seam;
iframe Range remains deferred. Continuing independent legacy element reflection.

Specs: https://w3c.github.io/uievents/ and https://drafts.csswg.org/cssom-view/#extensions-to-the-mouseevent-interface .
Reproduce with dom-b-audit/run-corpus.py and `--directory dom/events --directory uievents/constructors`.
