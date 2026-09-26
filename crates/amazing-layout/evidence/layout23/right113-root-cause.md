# `CSS2/positioning/right-113.xht` loss: root cause, and why it is not layout

The prior checkpoint described this as "the reference's PNG is absent from the
render." That phrasing was imprecise. The file is present and valid; Amazing's
own sandboxed image decoder fails to decode it, and fails identically for
every other PNG tried, including files that have nothing to do with this
fixture. This is the already-tracked IPC-10/RC-21 utility-decoder bug, not a
missing resource and not an `amazing-layout` defect.

## What the test actually needs

- `right-113.xht` (RTL `right: inherit`) draws its own black box purely from
  CSS (border-right); it does not load an image.
- Its reference, `right-103-ref.xht`, draws the matching black box with
  `<img src="support/black15x15.png" width="96" height="96">`.
- LAYOUT-23's RTL/margin fix made the *test* side correctly render a
  black 96×96 rectangle at x104..199, y51..146, with zero red (CSS2.2 RTL
  `right`/margin resolution is correct). The *reference* side still renders
  blank there because its `<img>` never decodes.

## Proof the file is fine

`support/black15x15.png` is byte-identical (`sha256
4797fabddca807b837064f5640b4603674053cdc1d9f2f5cd6f1cb90c5fe1caa`) across
every copy in the pinned WPT checkout (`css/CSS2/{positioning,backgrounds,
borders,text,bidi-text,floats-clear,linebox}/support/`,
`css/css-display/run-in/support/`, `css/css-fonts/support/`). `file` reports
"PNG image data, 15 x 15, 8-bit/color RGB, non-interlaced"; parsing its chunks
by hand gives a legal chunk sequence (`IHDR, pHYs, sRGB, gAMA, cHRM, IDAT,
IEND`) and `zlib.decompress` on the `IDAT` payload yields exactly 690 bytes
(15 rows × (1 filter byte + 15×3 RGB) — the exact expected size for a
non-interlaced 8-bit truecolor 15×15 image). There is nothing wrong with this
file.

## Proof the decode failure is content-independent (not this file, not PNG-shape)

Direct probes with a locally built `release/amazing` binary
(`amazing render-wpt <file> <out> 800 600`), same host:

| Input | Result |
| --- | --- |
| `support/black15x15.png` (the fixture) | `Io(Error { kind: UnexpectedEof, message: "failed to fill whole buffer" })` |
| `support/blue96x96.png` (unrelated fixture, different chunk layout incl. `PLTE`) | same error |
| A hand-built minimal 2×2 PNG with only `IHDR`/`IDAT`/`IEND`, no ancillary chunks | same error |

Every PNG probed on this host fails identically. The failure is not a
property of `black15x15.png`.

## Where it actually happens

`crates/amazing/src/page.rs::load_image` calls
`PageMedia.decoder.decode(&bytes)`, i.e.
`UtilityImageDecoder::decode` in
`crates/amazing/src/runtime_host_replaced_content.rs`. In a release, non-test
build that decoder does not touch `amazing-image`/`amazing-media` decode
logic directly: it round-trips the bytes through a sandboxed child process
via `ProcessHost::decode_untrusted(amazing_process::utility::IMAGE, bytes,
Duration::from_secs(5))` (`runtime_host_replaced_content.rs:107-108`). The
`Io(UnexpectedEof)` comes back from that IPC round trip, in
`amazing-process`/`amazing-ipc`, not from any layout code and not from the
PNG codec itself (the in-process `amazing-image` codec is only reached as a
fallback for BMP/PPM/SVG when the *sandbox itself* fails to spawn — PNG is
never in that fallback list, so a PNG always goes through the utility IPC
path). `amazing-layout` has no image-decoding code at all; it only consumes
already-decoded `IntrinsicSize`/pixels through `ReplacedElementProvider`.

This exact signature is a **known, already-open, cross-lane-tracked bug**,
not something newly discovered here:

- `.agents/ipc/questions.md` ("RC-21 Windows IMAGE utility blocker"): the
  same `Io(Error { kind: UnexpectedEof, message: "failed to fill whole
  buffer" })` from the same `decode_untrusted` path, reproduced on win-002
  with an unrelated 2×1 PPM fixture.
- `.agents/replaced-content/questions.md` ("ROUTED ipc IPC-10 urgent Windows
  utility failure"): the same failure blocking RC-21's own win-002 gate.

Those reports were Windows-only. This session reproduces the identical
failure signature on a Linux Boat sandbox with three independent PNGs
(including a synthetic one with no relation to any WPT fixture), which is new
information for whoever is diagnosing IPC-10/RC-21: the bug is not
Windows-AppContainer-specific. It reproduces after the utility process
apparently spawns/boots successfully (no `spawn_sandboxed_utility` error is
raised; the failure surfaces later, on the first `decode_untrusted` call),
which narrows it toward the per-request frame/data channel or response
framing rather than process bootstrap itself.

## Why this was an "accidental pass" before LAYOUT-23, not a LAYOUT-23 regression

`right113-diagnosis.json` (already committed) shows the *before* snapshot had
**zero** black pixels in both the actual and the reference: the old RTL
`right: inherit` bug meant the test never drew its box either, so the two
blank-in-different-ways renders happened to be byte-equal. LAYOUT-23 fixed the
test side; the reference side was never fixed by anyone because it depends on
a decoder path that is broken independent of this ticket, this fixture, and
this lane. Both `RC-21` (the utility image decoder) and its Windows failure
report on `.agents/ipc/questions.md` predate `749555ca` (LAYOUT-23's fixed
base) and this ebb96de6 checkpoint; nothing in this Linux investigation or in
LAYOUT-23's source changes introduced or worsened it.

## Disposition

This is recorded as a documented, owner-external exception in `gate.sh`
(case id `CSS2__positioning__right-113.xht` in the `position` corpus only),
not silently dropped: the gate script still reports it as an "accepted
owner-external loss" every run, with this file as the citation, until IPC-10
lands a fix and the fixture is re-measured. No `amazing-layout` source change
can fix this; the fix belongs in `amazing-process`/`amazing-ipc`'s
`decode_untrusted` request/response path.
