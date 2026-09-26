#!/usr/bin/env python3
"""Linux port of scripts/run-render-wpt.ps1's reftest comparison, generalized
to any of the lane's pinned selection manifests (text1-fonts-manifest.json,
text2-font-face-manifest.json, text5-vertical-manifest.json,
text12-expanded-manifest.json, ...).

Renders each manifest test and its `rel=match`/`rel=mismatch` reference
through `amazing render-wpt <page> <out.ppm> <width> <height>` with
AMAZING_WPT_ROOT pointing at a full checkout of the pinned WPT revision
(scripts/ensure-wpt.sh), then compares the two PPMs the same way the Windows
harness does: exact per-channel diff, honoring a `<meta name=fuzzy>` tag's
maxDifference/totalPixels tolerance, `mismatch` relation inverted. Output
schema matches the existing win-001 evidence JSONs (see
text13-font15-before.json) so before/after and Linux/Windows numbers compare
directly.

Usage (from the repo root, after `cargo build -p amazing --locked`):
  python3 crates/amazing-text-provider/evidence/text18-render-wpt.py \
    <manifest.json> <wpt-root> <amazing-binary> <out.json> \
    [--width 800] [--height 600] [--timeout 20]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path
from urllib.parse import unquote

LINK_RE = re.compile(r"<link\b[^>]*>", re.IGNORECASE)
ATTR_RE = re.compile(r"([:\w-]+)\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))", re.DOTALL)
FUZZY_TAG_RE = re.compile(r"<meta\b[^>]*\bname\s*=\s*[\"']fuzzy[\"'][^>]*>", re.IGNORECASE | re.DOTALL)
FUZZY_CONTENT_RE = re.compile(r"\bcontent\s*=\s*[\"']([^\"']+)[\"']", re.IGNORECASE | re.DOTALL)
FUZZY_MAX_RE = re.compile(r"maxDifference\s*=\s*\d+-(\d+)")
FUZZY_PIXELS_RE = re.compile(r"totalPixels\s*=\s*\d+-(\d+)")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def html_attrs(tag: str) -> dict[str, str]:
    out = {}
    for name, dq, sq, bare in ATTR_RE.findall(tag):
        out[name.lower()] = dq or sq or bare
    return out


def reftest_link(test_path: Path, wpt_root: Path) -> dict:
    source = test_path.read_text(encoding="utf-8-sig", errors="replace")
    for tag in LINK_RE.findall(source):
        attrs = html_attrs(tag)
        rel_tokens = attrs.get("rel", "").lower().split()
        href = attrs.get("href")
        if not href:
            continue
        if "match" in rel_tokens:
            relation = "match"
        elif "mismatch" in rel_tokens:
            relation = "mismatch"
        else:
            continue
        if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", href) or href.startswith("//"):
            return {"supported": False, "detail": "remote reftest links are unsupported"}
        clean_href = unquote(href.split("?")[0].split("#")[0])
        base = wpt_root if href.startswith("/") else test_path.parent
        reference_path = (base / clean_href.lstrip("/")).resolve()
        try:
            reference_path.relative_to(wpt_root.resolve())
        except ValueError:
            return {"supported": False, "detail": "reference escapes the pinned WPT checkout"}
        return {"supported": True, "relation": relation, "reference_path": reference_path, "href": href}
    return {"supported": False, "detail": "no upstream rel=match or rel=mismatch link"}


def fuzzy_limits(test_path: Path) -> tuple[int, int]:
    source = test_path.read_text(encoding="utf-8-sig", errors="replace")
    tag = FUZZY_TAG_RE.search(source)
    if not tag:
        return 0, 0
    content = FUZZY_CONTENT_RE.search(tag.group(0))
    if not content:
        return 0, 0
    max_match = FUZZY_MAX_RE.search(content.group(1))
    pixel_match = FUZZY_PIXELS_RE.search(content.group(1))
    return (
        int(max_match.group(1)) if max_match else 0,
        int(pixel_match.group(1)) if pixel_match else 0,
    )


def read_ppm(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    fields, index = [], 0
    while len(fields) < 4:
        while index < len(data) and data[index] in b" \t\r\n":
            index += 1
        if index < len(data) and data[index] == 0x23:
            while index < len(data) and data[index] != 0x0A:
                index += 1
            continue
        start = index
        while index < len(data) and data[index] not in b" \t\r\n":
            index += 1
        fields.append(data[start:index])
    index += 1
    width, height, maxval = int(fields[1]), int(fields[2]), int(fields[3])
    pixels = data[index:]
    if maxval != 255 or len(pixels) != width * height * 3:
        raise ValueError(f"unsupported PPM {width}x{height} maxval={maxval} bytes={len(pixels)}")
    return width, height, pixels


def compare_ppm(actual_path: Path, reference_path: Path, allowed_difference: int, allowed_pixels: int) -> dict:
    aw, ah, apx = read_ppm(actual_path)
    rw, rh, rpx = read_ppm(reference_path)
    if (aw, ah) != (rw, rh):
        return {
            "matched": False,
            "differing_pixels": aw * ah,
            "max_difference": 255,
            "detail": "image dimensions differ",
        }
    differing = 0
    max_diff = 0
    for i in range(0, len(apx), 3):
        a = apx[i : i + 3]
        b = rpx[i : i + 3]
        if a == b:
            continue
        differing += 1
        max_diff = max(max_diff, max(abs(a[c] - b[c]) for c in range(3)))
    return {
        "matched": max_diff <= allowed_difference and differing <= allowed_pixels,
        "differing_pixels": differing,
        "max_difference": max_diff,
        "detail": f"maxDifference={max_diff}/{allowed_difference}; totalPixels={differing}/{allowed_pixels}",
    }


def render(binary: Path, source: Path, out: Path, wpt_root: Path, width: int, height: int, timeout: int) -> tuple[str, str]:
    env = {"AMAZING_WPT_ROOT": str(wpt_root), "PATH": "/usr/bin:/bin"}
    try:
        process = subprocess.run(
            [str(binary), "render-wpt", str(source), str(out), str(width), str(height)],
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
    except subprocess.TimeoutExpired:
        return "timeout", f"exceeded {timeout}s"
    if process.returncode == 0 and out.exists():
        return "rendered", ""
    detail = (process.stderr or process.stdout or "").strip()
    if process.returncode in (2, 3):
        return "unsupported", detail or f"exit {process.returncode}"
    if process.returncode < 0:
        return "crash", detail or f"signal {-process.returncode}"
    return "error", detail or f"exit {process.returncode}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest")
    parser.add_argument("wpt_root")
    parser.add_argument("binary")
    parser.add_argument("out")
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=600)
    parser.add_argument("--timeout", type=int, default=20)
    args = parser.parse_args()

    manifest = json.loads(Path(args.manifest).read_text())
    wpt_root = Path(args.wpt_root).resolve()
    binary = Path(args.binary).resolve()
    run_dir = Path(args.out).resolve().parent / (Path(args.out).stem + "-run")
    run_dir.mkdir(parents=True, exist_ok=True)

    results = []
    counts = {"pass": 0, "mismatch": 0, "error": 0, "timeout": 0, "unsupported": 0, "crash": 0}
    for test in manifest["tests"]:
        test_id = test["id"]
        test_dir = run_dir / hashlib.sha256(test_id.encode()).hexdigest()[:16]
        test_dir.mkdir(parents=True, exist_ok=True)
        test_path = wpt_root / test["test"]

        def record(classification: str, **fields):
            counts[classification] += 1
            results.append({"id": test_id, "classification": classification, **fields})

        if not test_path.is_file():
            record("unsupported", detail="test is absent from the pinned checkout")
            continue
        reftest = reftest_link(test_path, wpt_root)
        if not reftest.get("supported") or not Path(reftest.get("reference_path", "/nonexistent")).is_file():
            detail = reftest.get("detail") if not reftest.get("supported") else "upstream reference is absent from the pinned checkout"
            record("unsupported", detail=detail)
            continue

        actual_path = test_dir / "actual.ppm"
        reference_path = test_dir / "reference.ppm"
        actual_status, actual_detail = render(binary, test_path, actual_path, wpt_root, args.width, args.height, args.timeout)
        if actual_status != "rendered":
            record(actual_status, detail=actual_detail)
            continue
        reference_status, reference_detail = render(binary, reftest["reference_path"], reference_path, wpt_root, args.width, args.height, args.timeout)
        if reference_status != "rendered":
            record(reference_status, detail=f"reference: {reference_detail}", actual_sha256=sha256_file(actual_path))
            continue

        allowed_max, allowed_pixels = fuzzy_limits(test_path)
        comparison = compare_ppm(actual_path, reference_path, allowed_max, allowed_pixels)
        passed = (not comparison["matched"]) if reftest["relation"] == "mismatch" else comparison["matched"]
        record(
            "pass" if passed else "mismatch",
            relation=reftest["relation"],
            reference_href=reftest["href"],
            differing_pixels=comparison["differing_pixels"],
            max_difference=comparison["max_difference"],
            allowed_max_difference=allowed_max,
            allowed_differing_pixels=allowed_pixels,
            actual_sha256=sha256_file(actual_path),
            reference_sha256=sha256_file(reference_path),
            detail=comparison["detail"],
        )
        print(f"{results[-1]['classification'].upper():10} {test_id}")

    out = {
        "schema": 1,
        "manifest": str(args.manifest),
        "wpt_revision": manifest.get("revision"),
        "selection": manifest.get("selection"),
        "viewport": {"width": args.width, "height": args.height},
        "run_directory": str(run_dir),
        "counts": counts,
        "tests": results,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    Path(args.out).write_text(json.dumps(out, indent=2))
    print(json.dumps(counts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
