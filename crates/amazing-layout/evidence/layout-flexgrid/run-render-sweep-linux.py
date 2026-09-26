#!/usr/bin/env python3
"""Linux iteration sweep for a pinned WPT reftest manifest.

This is the Linux twin of ``scripts/run-render-wpt.ps1``: same manifest
schema, same pass/mismatch classification and fuzzy meta handling, but it
downloads the selected test and reference files at the pinned revision from
raw.githubusercontent.com instead of reading a full WPT checkout. Windows
remains the gate platform; these numbers only drive iteration.

Usage:
  run-render-sweep-linux.py MANIFEST --binary PATH --output DIR \
      [--wpt-root DIR] [--width 800] [--height 600] [--timeout 20]
"""
import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request

RAW = "https://raw.githubusercontent.com/web-platform-tests/wpt/{revision}/{path}"


def fetch(path, revision, root):
    local = os.path.join(root, path)
    if os.path.isfile(local):
        return local
    os.makedirs(os.path.dirname(local), exist_ok=True)
    url = RAW.format(revision=revision, path=path)
    try:
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
    except Exception as exc:  # noqa: BLE001 - report and continue
        raise RuntimeError(f"download {url} failed: {exc}") from exc
    if data[:3] == b"404":
        raise RuntimeError(f"missing at pinned revision: {path}")
    with open(local, "wb") as handle:
        handle.write(data)
    return local


def reftest_link(test_path, test_rel):
    with open(test_path, "r", encoding="utf-8", errors="replace") as handle:
        source = handle.read()
    for tag in re.findall(r"(?is)<link\b[^>]*>", source):
        rel = re.search(r'(?is)\brel\s*=\s*(?:"([^"]*)"|\'([^\']*)\'|([^\s>]+))', tag)
        href = re.search(r'(?is)\bhref\s*=\s*(?:"([^"]*)"|\'([^\']*)\'|([^\s>]+))', tag)
        if not rel or not href:
            continue
        rel_value = next(g for g in rel.groups() if g is not None)
        href_value = next(g for g in href.groups() if g is not None)
        tokens = rel_value.lower().split()
        relation = "match" if "match" in tokens else ("mismatch" if "mismatch" in tokens else None)
        if relation is None:
            continue
        if re.match(r"^[A-Za-z][A-Za-z0-9+.-]*:", href_value) or href_value.startswith("//"):
            return None
        href_value = re.split(r"[?#]", href_value, maxsplit=1)[0]
        from urllib.parse import unquote
        href_value = unquote(href_value)
        base = "" if href_value.startswith("/") else os.path.dirname(test_rel) + "/"
        resolved = os.path.normpath(base + href_value.lstrip("/"))
        return relation, resolved
    return None


def fuzzy_limits(test_path):
    with open(test_path, "r", encoding="utf-8", errors="replace") as handle:
        source = handle.read()
    tag = re.search(r'(?is)<meta\b[^>]*\bname\s*=\s*["\']fuzzy["\'][^>]*>', source)
    max_diff, max_pixels = 0, 0
    if not tag:
        return max_diff, max_pixels
    content = re.search(r'(?is)\bcontent\s*=\s*["\']([^"\']+)["\']', tag.group(0))
    if not content:
        return max_diff, max_pixels
    value = content.group(1)
    match = re.search(r"maxDifference\s*=\s*\d+-(\d+)", value)
    if match:
        max_diff = int(match.group(1))
    match = re.search(r"totalPixels\s*=\s*\d+-(\d+)", value)
    if match:
        max_pixels = int(match.group(1))
    return max_diff, max_pixels


def read_ppm(path):
    with open(path, "rb") as handle:
        data = handle.read()
    tokens = []
    index = 0
    while len(tokens) < 4:
        while index < len(data) and data[index] in (9, 10, 13, 32):
            index += 1
        if index < len(data) and data[index] == 35:
            while index < len(data) and data[index] != 10:
                index += 1
            continue
        start = index
        while index < len(data) and data[index] not in (9, 10, 13, 32):
            index += 1
        if start == index:
            raise ValueError(f"invalid PPM header: {path}")
        tokens.append(data[start:index].decode("ascii"))
    index += 1
    width, height = int(tokens[1]), int(tokens[2])
    length = width * height * 3
    pixels = data[index:index + length]
    if len(pixels) != length:
        raise ValueError(f"PPM payload length mismatch: {path}")
    return width, height, pixels


def compare(actual, reference, allowed_diff, allowed_pixels):
    aw, ah, ap = read_ppm(actual)
    rw, rh, rp = read_ppm(reference)
    if (aw, ah) != (rw, rh):
        return False, aw * ah, 255, "image dimensions differ"
    differing = 0
    maximum = 0
    for offset in range(0, len(ap), 3):
        pixel_differs = False
        for channel in range(3):
            delta = abs(ap[offset + channel] - rp[offset + channel])
            if delta > 0:
                pixel_differs = True
            if delta > maximum:
                maximum = delta
        if pixel_differs:
            differing += 1
    matched = maximum <= allowed_diff and differing <= allowed_pixels
    return matched, differing, maximum, f"maxDifference={maximum}/{allowed_diff}; totalPixels={differing}/{allowed_pixels}"


def render(binary, wpt_root, input_path, output_path, width, height, timeout, log):
    env = dict(os.environ)
    env["AMAZING_WPT_ROOT"] = wpt_root
    try:
        proc = subprocess.run(
            [binary, "render-wpt", input_path, output_path, str(width), str(height)],
            capture_output=True, timeout=timeout, env=env, check=False,
        )
    except subprocess.TimeoutExpired:
        return "timeout", "exceeded timeout"
    with open(log + ".stdout.txt", "wb") as handle:
        handle.write(proc.stdout)
    with open(log + ".stderr.txt", "wb") as handle:
        handle.write(proc.stderr)
    if proc.returncode in (2, 3):
        return ("unsupported" if proc.returncode == 3 else "error"), proc.stderr.decode("utf-8", "replace").strip()
    if proc.returncode != 0 or not os.path.isfile(output_path):
        return "crash", f"exit {proc.returncode}: {proc.stderr.decode('utf-8', 'replace').strip()}"
    return "rendered", proc.stdout.decode("utf-8", "replace").strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--wpt-root", default=os.path.expanduser("~/wpt"))
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=600)
    parser.add_argument("--timeout", type=int, default=20)
    args = parser.parse_args()

    with open(args.manifest, encoding="utf-8-sig") as handle:
        manifest = json.load(handle)
    revision = manifest["revision"]
    os.makedirs(args.output, exist_ok=True)
    os.makedirs(args.wpt_root, exist_ok=True)

    results = []
    counts = {"pass": 0, "mismatch": 0, "error": 0, "timeout": 0, "unsupported": 0, "crash": 0}
    ref_cache = {}
    for case in manifest["tests"]:
        case_id = case["id"]
        test_rel = case["test"]
        case_dir = os.path.join(args.output, re.sub(r"[^A-Za-z0-9_.-]", "_", case_id))
        os.makedirs(case_dir, exist_ok=True)
        try:
            test_path = fetch(test_rel, revision, args.wpt_root)
        except RuntimeError as exc:
            counts["unsupported"] += 1
            results.append({"id": case_id, "classification": "unsupported", "detail": str(exc)})
            continue
        link = reftest_link(test_path, test_rel)
        if link is None:
            counts["unsupported"] += 1
            results.append({"id": case_id, "classification": "unsupported", "detail": "no local match/mismatch link"})
            continue
        relation, ref_rel = link
        try:
            ref_path = fetch(ref_rel, revision, args.wpt_root)
        except RuntimeError as exc:
            counts["unsupported"] += 1
            results.append({"id": case_id, "classification": "unsupported", "detail": str(exc)})
            continue
        actual = os.path.join(case_dir, "actual.ppm")
        kind, detail = render(args.binary, args.wpt_root, os.path.abspath(test_path), os.path.abspath(actual),
                              args.width, args.height, args.timeout, os.path.join(case_dir, "actual"))
        if kind != "rendered":
            counts[kind] += 1
            results.append({"id": case_id, "classification": kind, "detail": detail})
            continue
        ref_output = ref_cache.get(ref_rel)
        if ref_output is None:
            ref_output = os.path.join(case_dir, "reference.ppm")
            ref_kind, ref_detail = render(args.binary, args.wpt_root, os.path.abspath(ref_path),
                                          os.path.abspath(ref_output), args.width, args.height,
                                          args.timeout, os.path.join(case_dir, "reference"))
            if ref_kind != "rendered":
                counts[ref_kind] += 1
                results.append({"id": case_id, "classification": ref_kind, "detail": f"reference: {ref_detail}"})
                continue
            ref_cache[ref_rel] = ref_output
        allowed_diff, allowed_pixels = fuzzy_limits(test_path)
        matched, differing, maximum, compare_detail = compare(actual, ref_output, allowed_diff, allowed_pixels)
        passed = (not matched) if relation == "mismatch" else matched
        classification = "pass" if passed else "mismatch"
        counts[classification] += 1
        results.append({
            "id": case_id, "classification": classification, "relation": relation,
            "differing_pixels": differing, "max_difference": maximum,
            "allowed_max_difference": allowed_diff, "allowed_differing_pixels": allowed_pixels,
            "detail": compare_detail,
        })
        print(f"{classification.upper():9s} {case_id} ({compare_detail})", flush=True)

    report = {
        "schema": 1, "wpt_revision": revision, "manifest": args.manifest,
        "viewport": {"width": args.width, "height": args.height},
        "binary": args.binary, "counts": counts, "tests": results,
    }
    report_path = os.path.join(args.output, "report.json")
    with open(report_path, "w") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    print(f"Report: {report_path}")
    print("Counts: " + " ".join(f"{k}={v}" for k, v in counts.items()))


if __name__ == "__main__":
    sys.exit(main())
