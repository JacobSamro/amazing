# Script source UTF-16

Recorded base `ab19e359edbab188e1881addb0001f8febdc1d15`; qualified engine source `6e4ce98c5c6a94ecc6ecfdb89b60009887a1936f`. HTML classic/external/module JavaScript source, queued snapshots, Script.text and DevTools source wire now preserve actual DOMString units. Existing execution settings, realm/currentScript identity and source admission controls remain. Legacy scalar module resolve remains an explicit compatibility boundary.

Linux whole pair: nextest5055→5070 PASS/17skip, doctests36→36;20 Test262 reports and8 WPT reports across default+VM, zero lost files/variants/subtests. WPT gains2 files/12 subtests total; all15 added native controls pass. Compact proof keeps original compile and launcher failures. Native raw/targets/ELFs stay on their original hosts; root owns archive/lifecycle. Integrator full16/current-merged gate remains required.
