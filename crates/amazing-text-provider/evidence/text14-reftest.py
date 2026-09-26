#!/usr/bin/env python3
"""Focused Linux corpus for the pinned font-kerning reftests.

Runs the same `amazing render-wpt` fixed-viewport command the Windows harness
drives, with AMAZING_WPT_ROOT pointing at a checkout of the pinned WPT revision.
For these five cases the WPT assertion allows zero differing pixels, so an exact
PPM byte compare is the upstream check; mismatches also record the differing
pixel count, bounding box and contiguous row bands.

The upstream HTML and support font are not committed. Recreate the subset at
the pinned revision:
  css/css-fonts/font-kerning-01..05.html and their -ref.html
  css/css-fonts/support/fonts/FontWithFancyFeatures.otf
then run from the repo root after `cargo build -p amazing --locked`:
  python3 crates/amazing-text-provider/evidence/text14-reftest.py <subset-root> \
    target/debug/amazing <out.json> [<ppm-dir>]
"""
from __future__ import annotations

import json
import hashlib
import os
import re
import subprocess
import sys
from pathlib import Path

LINK_RE = re.compile(r"<link\b[^>]*>", re.IGNORECASE)
ATTR_RE = re.compile(r"([:\w-]+)\s*=\s*(['\"])(.*?)\2", re.DOTALL)
CASES = [
    "font-kerning-01.html",
    "font-kerning-02.html",
    "font-kerning-03.html",
    "font-kerning-04.html",
    "font-kerning-05.html",
]
WIDTH, HEIGHT = 800, 600


def reference_of(test: Path) -> Path:
    source = test.read_text(encoding="utf-8-sig", errors="replace")
    for tag in LINK_RE.findall(source):
        attrs = {name.lower(): value for name, _, value in ATTR_RE.findall(tag)}
        if attrs.get("rel", "").lower() == "match" and "href" in attrs:
            return (test.parent / attrs["href"]).resolve()
    raise ValueError("no rel=match reference")


def read_ppm(path: Path):
    data = path.read_bytes()
    fields, index = [], 0
    while len(fields) < 4:
        while index < len(data) and data[index] in b" \t\r\n":
            index += 1
        if index < len(data) and data[index] == 0x23:  # comment
            while index < len(data) and data[index] != 0x0A:
                index += 1
            continue
        start = index
        while index < len(data) and data[index] not in b" \t\r\n":
            index += 1
        fields.append(data[start:index])
    index += 1  # single whitespace after maxval
    width, height, maxval = int(fields[1]), int(fields[2]), int(fields[3])
    pixels = data[index:]
    if maxval != 255 or len(pixels) != width * height * 3:
        raise ValueError(f"unsupported PPM {width}x{height} maxval={maxval} bytes={len(pixels)}")
    return pixels


def diff_pixels(a: bytes, b: bytes) -> int:
    if a == b:
        return 0
    return sum(1 for i in range(0, min(len(a), len(b)), 3) if a[i : i + 3] != b[i : i + 3])


def diff_detail(a: bytes, b: bytes, width: int, height: int) -> dict:
    """Bounding box and contiguous row bands of differing pixels."""
    rows = [0] * height
    min_x, min_y, max_x, max_y = width, height, -1, -1
    for i in range(0, min(len(a), len(b)), 3):
        if a[i : i + 3] == b[i : i + 3]:
            continue
        pixel = i // 3
        x, y = pixel % width, pixel // width
        rows[y] += 1
        min_x, min_y = min(min_x, x), min(min_y, y)
        max_x, max_y = max(max_x, x), max(max_y, y)
    bands = []
    for y, count in enumerate(rows):
        if count == 0:
            continue
        if bands and bands[-1][1] == y - 1:
            bands[-1][1] = y
            bands[-1][2] += count
        else:
            bands.append([y, y, count])
    return {
        "differing_bbox": [min_x, min_y, max_x, max_y] if max_y >= 0 else None,
        "differing_row_bands": bands,
    }


def render(binary: Path, source: Path, out: Path, root: Path) -> tuple[bool, str]:
    env = dict(os.environ)
    env["AMAZING_WPT_ROOT"] = str(root)
    process = subprocess.run(
        [str(binary), "render-wpt", str(source), str(out), str(WIDTH), str(HEIGHT)],
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
        env=env,
    )
    return process.returncode == 0, (process.stderr or process.stdout).strip()


def main() -> int:
    root = Path(sys.argv[1]).resolve()
    binary = Path(sys.argv[2]).resolve()
    out_json = Path(sys.argv[3]) if len(sys.argv) > 3 else None
    ppm_dir = Path(sys.argv[4]) if len(sys.argv) > 4 else None
    if ppm_dir:
        ppm_dir.mkdir(parents=True, exist_ok=True)
    work = Path("/tmp/text14-kern")
    work.mkdir(parents=True, exist_ok=True)
    results, passed = [], 0
    for name in CASES:
        test = root / "css/css-fonts" / name
        case = {"id": f"css-fonts__{name}", "classification": None}
        try:
            reference = reference_of(test)
        except (OSError, ValueError) as error:
            case.update(classification="error", detail=str(error))
            results.append(case)
            continue
        test_ppm, ref_ppm = work / f"{name}.test.ppm", work / f"{name}.ref.ppm"
        test_ok, test_log = render(binary, test, test_ppm, root)
        ref_ok, ref_log = render(binary, reference, ref_ppm, root)
        if not test_ok or not ref_ok:
            case.update(classification="crash", detail=f"test={test_log!r} ref={ref_log!r}")
        else:
            test_px, ref_px = read_ppm(test_ppm), read_ppm(ref_ppm)
            case["actual_sha256"] = hashlib.sha256(test_ppm.read_bytes()).hexdigest()
            case["reference_sha256"] = hashlib.sha256(ref_ppm.read_bytes()).hexdigest()
            differing = diff_pixels(test_px, ref_px)
            case["differing_pixels"] = differing
            if differing == 0:
                case["classification"] = "pass"
                passed += 1
            else:
                case["classification"] = "mismatch"
                case.update(diff_detail(test_px, ref_px, WIDTH, HEIGHT))
            if ppm_dir:
                (ppm_dir / f"{name}.test.ppm").write_bytes(test_ppm.read_bytes())
                (ppm_dir / f"{name}.ref.ppm").write_bytes(ref_ppm.read_bytes())
        results.append(case)
    report = {
        "schema": 1,
        "selection": "text14-pinned-font-kerning",
        "wpt_revision": "c2a2fce69e5c2509d9dc28440ec8b42532e5c293",
        "viewport": {"width": WIDTH, "height": HEIGHT},
        "binary": str(binary),
        "counts": {"pass": passed, "mismatch": len(results) - passed, "total": len(results)},
        "tests": results,
    }
    text = json.dumps(report, indent=2)
    print(text)
    if out_json:
        out_json.write_text(text)
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
