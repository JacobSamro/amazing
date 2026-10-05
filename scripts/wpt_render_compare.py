#!/usr/bin/env python3
"""Linux counterpart of scripts/inventory-render-wpt.ps1 + scripts/run-render-wpt.ps1.

Given a WPT checkout and a `css/<category>` subdirectory, this discovers the
same class of self-contained static reftests the Windows inventory script
selects (HTML-parser-compatible, local match/mismatch reference, no scripts,
no embedded resources, no CSS url()/@font-face/@import, no external
stylesheet), renders each pair with the headless `amazing render-wpt`
subcommand, pixel-compares the PPM outputs with the test's `<meta
name="fuzzy">` tolerance, and writes one report.json with the same
classification vocabulary as the Windows runner: pass, mismatch, error,
timeout, unsupported, crash.

Stdlib only, matching the engine's own zero-dependency policy. Python and
Rust are both acceptable per the ticket; Python was chosen so lanes can run
this without an extra crate/build step.

Was serial (one render-wpt subprocess pair at a time); --jobs N runs N
candidates' render+compare pipelines concurrently on separate threads (each
subprocess call releases the GIL while waiting), useful for a full-fleet
sweep across many WPT categories.

SHELL-L5 turned this into a zero-loss gate: --baseline <path> diffs this
run's per-test classifications against a previous report.json or --compact
compact.json (either shape is accepted) and exits 2 if any previously-passing
test regressed ("lost"). --compact additionally writes compact.json, an
id -> classification map with none of report.json's per-test detail
(relation/pixel counts/sha256/stdout) -- the file meant to be committed as a
future --baseline, so evidence size does not grow with every run the way a
full report.json would.

Usage:
  wpt_render_compare.py --wpt-root DIR --subdir css/css-backgrounds \\
      --exe PATH/TO/amazing --out OUT_DIR
      [--width 800] [--height 600] [--timeout 20] [--limit N] [--jobs N]
      [--baseline PATH] [--compact] [--font-set STRING]
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import html
import json
import math
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
import urllib.parse
from pathlib import Path

DOCUMENT_EXTENSIONS = {".html", ".htm", ".xht", ".xhtml", ".svg"}
# .svg is intentionally NOT XML_EXTENSIONS: the driver's document_kind()
# (crates/amazing/src/document.rs) already dispatches ".svg" to its XML
# parser and renders script-free XML documents fine (require_static_xml()
# only refuses <script>/inline-handler content); the .xht/.xhtml exclusion
# below predates this ticket and is a separate, still-open limitation this
# ticket does not touch.
XML_EXTENSIONS = {".xht", ".xhtml"}

# SHELL-L7: raw .svg WPT reftests commonly embed HTML-behavior elements
# (reftest link, a reftest-wait/testharness <script>) through an
# `xmlns:html="http://www.w3.org/1999/xhtml"` declaration, so they appear as
# `<html:link rel="match" ...>` / `<html:script>` rather than bare tags --
# e.g. svg/animations/animate-rewind-freeze-indefinite.svg. Every tag-name
# match below tolerates that (or any other) XML namespace prefix.
NS_PREFIX = r"(?:[\w-]+:)?"
LINK_TAG_RE = re.compile(r"<" + NS_PREFIX + r"link\b[^>]*>", re.I | re.S)
SCRIPT_RE = re.compile(r"<" + NS_PREFIX + r"script\b", re.I | re.S)
# Inline event-handler attributes (onload="test()", etc.) run script without
# a <script> tag; the SHELL-L4 fleet sweep found 3/7168 "eligible" reftests
# that were actually testharness-style pages invoking `test()`/
# `getComputedStyle()` this way, misclassified as engine "crash" instead of
# excluded as "scripted" (renderer exit 1 from a body-load-listener
# ReferenceError, not a real render failure).
INLINE_EVENT_HANDLER_RE = re.compile(r"\bon\w+\s*=", re.I)
TESTHARNESS_RE = re.compile(r"testharness\.js", re.I | re.S)
FLAGS_META_RE = re.compile(r"<" + NS_PREFIX + r"meta\b[^>]*\bname\s*=\s*[\"']flags[\"']", re.I | re.S)
# "svg" was dropped from this list (SHELL-L7): an inline <svg> element is
# self-contained static content, not a foreign resource fetch, and excluding
# it as "embedded-resource" was quietly zeroing out the entire svg/ corpus
# (901 of its 1256+901 documents are .html wrappers around inline <svg>) as
# well as any css/* test that renders an inline SVG. SVG's own <image> element
# (raster/external-content fetch, the actual svg/ analogue of <img>) is added
# in its place.
EMBEDDED_RESOURCE_RE = re.compile(r"<" + NS_PREFIX + r"(?:img|object|iframe|video|audio|canvas|image)\b", re.I | re.S)
CSS_RESOURCE_RE = re.compile(r"url\s*\(|@font-face|@import\b", re.I | re.S)
STYLESHEET_LINK_RE = re.compile(r"<" + NS_PREFIX + r"link\b[^>]*\brel\s*=\s*[\"']stylesheet[\"']", re.I | re.S)
FUZZY_META_RE = re.compile(r"<" + NS_PREFIX + r"meta\b[^>]*>", re.I | re.S)
REMOTE_SCHEME_RE = re.compile(r"^[A-Za-z][A-Za-z0-9+.\-]*:")


def html_attribute(tag: str, name: str) -> str | None:
    pattern = re.compile(
        r"\b" + re.escape(name) + r"\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s>]+))",
        re.I | re.S,
    )
    match = pattern.search(tag)
    if not match:
        return None
    for group in match.groups():
        if group is not None:
            return group
    return None


def sha256_hex(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def reftest_links(test_path: Path, source: str, wpt_root: Path):
    """Resolve every reference, rejecting unsupported links conservatively."""
    links = []
    for tag in LINK_TAG_RE.finditer(source):
        rel = html_attribute(tag.group(0), "rel")
        href = html_attribute(tag.group(0), "href")
        if not rel or not href:
            continue
        tokens = rel.lower().split()
        if "match" in tokens:
            relation = "match"
        elif "mismatch" in tokens:
            relation = "mismatch"
        else:
            continue
        if REMOTE_SCHEME_RE.match(href) or href.startswith("//"):
            return {"supported": False, "reason": "remote-reference"}
        clean_href = urllib.parse.unquote(href.split("?", 1)[0].split("#", 1)[0])
        base = wpt_root if href.startswith("/") else test_path.parent
        reference_path = (base / clean_href.lstrip("/")).resolve()
        try:
            reference_path.relative_to(wpt_root)
        except ValueError:
            return {"supported": False, "reason": "escaping-reference"}
        links.append({
            "relation": relation,
            "reference_path": reference_path,
            "href": href,
        })
    return ({"supported": True, "links": links} if links else
            {"supported": False, "reason": "not-reftest"})


def classify_candidate(test_path: Path, wpt_root: Path):
    """Check the test and every reference against static-render eligibility.

    Eligible candidates carry all resolved links. An unsupported alternate
    cannot silently disappear from a test's match/mismatch requirements.
    """
    source = test_path.read_text(errors="replace")
    testharness = bool(TESTHARNESS_RE.search(source))
    link = reftest_links(test_path, source, wpt_root)
    if not link["supported"]:
        return {"eligible": False, "reason": link["reason"], "testharness": testharness}
    if test_path.suffix.lower() in XML_EXTENSIONS:
        return {"eligible": False, "reason": "xml-parser-mode", "testharness": testharness}
    sources = [source]
    for reference in link["links"]:
        reference_path = reference["reference_path"]
        if not reference_path.is_file():
            return {"eligible": False, "reason": "missing-reference", "testharness": testharness}
        if reference_path.suffix.lower() in XML_EXTENSIONS:
            return {"eligible": False, "reason": "xml-reference-mode", "testharness": testharness}
        sources.append(reference_path.read_text(errors="replace"))
    combined = "\n".join(sources)
    if SCRIPT_RE.search(combined) or INLINE_EVENT_HANDLER_RE.search(combined):
        reason = "scripted"
    elif FLAGS_META_RE.search(combined):
        reason = "flagged-environment"
    elif EMBEDDED_RESOURCE_RE.search(combined):
        reason = "embedded-resource"
    elif CSS_RESOURCE_RE.search(combined):
        reason = "css-resource"
    elif STYLESHEET_LINK_RE.search(combined):
        reason = "external-stylesheet"
    else:
        reason = None
    if reason:
        return {"eligible": False, "reason": reason, "testharness": testharness}
    return {
        "eligible": True,
        "links": link["links"],
        "testharness": testharness,
    }


def discover(wpt_root: Path, subdir: str, limit: int):
    root = (wpt_root / subdir).resolve()
    if not root.is_dir():
        raise SystemExit(f"wpt_render_compare: no such directory: {root}")
    files = sorted(
        path for path in root.rglob("*") if path.is_file() and path.suffix.lower() in DOCUMENT_EXTENSIONS
    )
    reasons: dict[str, int] = {}
    eligible = []
    reftests = 0
    testharness_count = 0
    for path in files:
        relative = path.relative_to(wpt_root).as_posix()
        outcome = classify_candidate(path, wpt_root)
        if outcome["testharness"]:
            testharness_count += 1
        if outcome["eligible"]:
            reftests += 1
            eligible.append(
                {
                    "path": relative,
                    "abs_path": path,
                    "links": outcome["links"],
                    "hash": sha256_hex(relative),
                }
            )
        else:
            if outcome["reason"] not in ("not-reftest",):
                reftests += 1 if outcome["reason"] not in ("remote-reference",) else 0
            reasons[outcome["reason"]] = reasons.get(outcome["reason"], 0) + 1
    eligible.sort(key=lambda item: (item["hash"], item["path"]))
    selected = eligible if limit <= 0 else eligible[:limit]
    inventory = {
        "document_files": len(files),
        "testharness_files": testharness_count,
        "reftests": reftests,
        "eligible_static_self_contained": len(eligible),
        "selected": len(selected),
        "excluded": reasons,
    }
    return selected, inventory


def read_ppm(path: Path):
    data = path.read_bytes()
    tokens = []
    index = 0
    while len(tokens) < 4:
        while index < len(data) and data[index] in b"\t\n\r ":
            index += 1
        if index < len(data) and data[index:index + 1] == b"#":
            while index < len(data) and data[index] != 0x0A:
                index += 1
            continue
        start = index
        while index < len(data) and data[index] not in b"\t\n\r ":
            index += 1
        if start == index:
            raise ValueError(f"invalid PPM header in {path}")
        tokens.append(data[start:index].decode("ascii"))
    if tokens[0] != "P6" or tokens[3] != "255":
        raise ValueError(f"only binary 8-bit P6 PPM is supported: {path}")
    index += 1  # single whitespace separator before the pixel data
    width, height = int(tokens[1]), int(tokens[2])
    length = width * height * 3
    pixels = data[index:index + length]
    if len(pixels) != length:
        raise ValueError(f"PPM payload length does not match {width}x{height}: {path}")
    return width, height, pixels


def write_ppm(path: Path, width: int, height: int, pixels: bytes) -> None:
    header = f"P6\n{width} {height}\n255\n".encode("ascii")
    path.write_bytes(header + pixels)


def fuzzy_metadata(test_source: str, test_url: str, reference_urls: set[str]):
    """Read WPT's named/positional integer ranges per reference.

    Implemented from https://web-platform-tests.org/writing-tests/reftests.html
    rather than treating a missing regex match as a zero tolerance.
    """
    metadata = {}
    names = ("maxDifference", "totalPixels")
    for match in FUZZY_META_RE.finditer(test_source):
        tag = match.group(0)
        if html_attribute(tag, "name") != "fuzzy":
            continue
        content = html_attribute(tag, "content")
        if content is None:
            raise ValueError("fuzzy metadata has no content")
        content = html.unescape(content).strip()
        prefix, separator, values = content.rpartition(":")
        key = urllib.parse.urljoin(test_url, prefix) if separator else None
        if key is not None and key not in reference_urls:
            raise ValueError(f"fuzzy metadata selects an unknown reference: {prefix}")
        if key in metadata:
            raise ValueError("duplicate fuzzy metadata for a reference")
        fields = (values if separator else content).split(";")
        if len(fields) != 2:
            raise ValueError("fuzzy metadata requires two parameters")
        named, positional = {}, []
        for field in fields:
            name, equal, value = field.partition("=")
            if equal:
                name = name.strip()
                if name not in names or name in named:
                    raise ValueError("invalid or duplicate fuzzy parameter")
            else:
                value = field
            integers = value.strip().split("-")
            if len(integers) not in (1, 2) or any(not re.fullmatch(r"\s*[0-9]+\s*", n) for n in integers):
                raise ValueError("fuzzy bounds must be nonnegative integer ranges")
            bounds = (int(integers[0]), int(integers[-1]))
            if bounds[0] > bounds[1]:
                raise ValueError("fuzzy range is reversed")
            if equal:
                named[name] = bounds
            else:
                positional.append(bounds)
        result = []
        for name in names:
            if name in named:
                result.append(named[name])
            elif positional:
                result.append(positional.pop(0))
            else:
                raise ValueError("missing fuzzy parameter")
        if positional:
            raise ValueError("unused fuzzy parameter")
        metadata[key] = tuple(result)
    return metadata


def compare_ppm(actual_path: Path, reference_path: Path, diff_path: Path, allowed_difference: tuple[int, int], allowed_pixels: tuple[int, int]):
    actual_w, actual_h, actual_pixels = read_ppm(actual_path)
    reference_w, reference_h, reference_pixels = read_ppm(reference_path)
    if (actual_w, actual_h) != (reference_w, reference_h):
        return {
            "matched": False,
            "differing_pixels": actual_w * actual_h,
            "max_difference": 255,
            "detail": "image dimensions differ",
        }
    diff = bytearray(len(actual_pixels))
    differing = 0
    max_difference = 0
    for offset in range(0, len(actual_pixels), 3):
        pixel_different = False
        for channel in range(3):
            difference = abs(actual_pixels[offset + channel] - reference_pixels[offset + channel])
            if difference > 0:
                pixel_different = True
            if difference > max_difference:
                max_difference = difference
        if pixel_different:
            differing += 1
            diff[offset], diff[offset + 1], diff[offset + 2] = 255, 0, 255
        else:
            diff[offset], diff[offset + 1], diff[offset + 2] = 255, 255, 255
    write_ppm(diff_path, actual_w, actual_h, bytes(diff))
    exact_allowed = (differing == 0 and allowed_pixels[0] == 0) or (max_difference == 0 and allowed_difference[0] == 0)
    matched = exact_allowed or (allowed_difference[0] <= max_difference <= allowed_difference[1]
                               and allowed_pixels[0] <= differing <= allowed_pixels[1])
    return {
        "matched": matched,
        "differing_pixels": differing,
        "max_difference": max_difference,
        "detail": f"maxDifference={max_difference}/{allowed_difference}; totalPixels={differing}/{allowed_pixels}",
    }


def invoke_fixed_render(exe: Path, input_path: Path, output_path: Path, width: int, height: int, timeout: float, wpt_root: Path, log_stem: Path):
    env = dict(os.environ)
    env["AMAZING_WPT_ROOT"] = str(wpt_root)
    try:
        completed = subprocess.run(
            [str(exe), "render-wpt", str(input_path), str(output_path), str(width), str(height)],
            env=env,
            capture_output=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as timeout_error:
        (log_stem.with_suffix(".stdout.txt")).write_bytes(timeout_error.stdout or b"")
        (log_stem.with_suffix(".stderr.txt")).write_bytes(timeout_error.stderr or b"")
        return {"classification": "timeout", "detail": f"exceeded {timeout}s"}
    except OSError as os_error:
        return {"classification": "crash", "detail": f"process did not start: {os_error}"}
    stdout = completed.stdout.decode(errors="replace")
    stderr = completed.stderr.decode(errors="replace")
    log_stem.with_suffix(".stdout.txt").write_text(stdout)
    log_stem.with_suffix(".stderr.txt").write_text(stderr)
    # Driver protocol (crates/amazing/src/cmd/wpt.rs + render.rs): exit 2 is an
    # XML document error, exit 3 is an unimplemented XML feature; anything
    # else nonzero, or a missing output file, is an ordinary crash.
    if completed.returncode in (2, 3):
        return {
            "classification": "unsupported" if completed.returncode == 3 else "error",
            "detail": stderr.strip(),
        }
    if completed.returncode != 0 or not output_path.is_file():
        return {
            "classification": "crash",
            "detail": f"renderer exit code {completed.returncode}: {stderr.strip()}",
        }
    return {"classification": "rendered", "detail": stdout.strip()}


def run_one_test(candidate, exe: Path, wpt_root: Path, run_directory: Path, width: int, height: int, timeout: float):
    """Render once, then require any match and every mismatch reference.

    Pure function of its
    arguments (own subprocess calls, own test_directory) so callers can run
    many of these concurrently across threads -- each candidate writes only
    to its own directory_id subdirectory."""
    test_id = re.sub(r"[^A-Za-z0-9_.\-]", "__", candidate["path"])
    directory_id = sha256_hex(test_id)[:16]
    test_directory = run_directory / directory_id
    test_directory.mkdir(parents=True, exist_ok=True)
    test_path = candidate["abs_path"]
    actual_path = test_directory / "actual.ppm"
    reference_output_path = test_directory / "reference.ppm"
    diff_path = test_directory / "diff.ppm"

    actual_run = invoke_fixed_render(
        exe, test_path, actual_path, width, height, timeout, wpt_root, test_directory / "actual"
    )
    if actual_run["classification"] != "rendered":
        return {
            "id": test_id,
            "classification": actual_run["classification"],
            "detail": actual_run["detail"],
        }
    test_source = test_path.read_text(errors="replace")
    test_url = test_path.as_uri()
    try:
        metadata = fuzzy_metadata(test_source, test_url, {
            urllib.parse.urljoin(test_url, link["href"]) for link in candidate["links"]
        })
    except ValueError as error:
        return {"id": test_id, "classification": "error", "detail": str(error)}
    evaluations = []
    for index, link in enumerate(candidate["links"]):
        reference_url = urllib.parse.urljoin(test_url, link["href"])
        max_difference, total_pixels = metadata.get(reference_url, metadata.get(None, ((0, 0), (0, 0))))
        suffix = "" if index == 0 else f"_{index}"
        reference_output = test_directory / f"reference{suffix}.ppm"
        diff_output = test_directory / f"diff{suffix}.ppm"
        reference_run = invoke_fixed_render(
            exe, link["reference_path"], reference_output, width, height, timeout, wpt_root,
            test_directory / f"reference{suffix}",
        )
        evaluation = {"relation": link["relation"], "reference_href": link["href"],
                      "satisfied": False,
                      "allowed_max_difference": max_difference[1],
                      "allowed_differing_pixels": total_pixels[1],
                      "allowed_difference_range": max_difference,
                      "allowed_pixel_range": total_pixels}
        if reference_run["classification"] != "rendered":
            evaluation.update(render_failed=reference_run["classification"],
                              detail=f"reference: {reference_run['detail']}")
        else:
            comparison = compare_ppm(actual_path, reference_output, diff_output,
                                     max_difference, total_pixels)
            evaluation.update(
                satisfied=(not comparison["matched"] if link["relation"] == "mismatch"
                           else comparison["matched"]),
                differing_pixels=comparison["differing_pixels"],
                max_difference=comparison["max_difference"],
                reference_sha256=sha256_file(reference_output),
                diff_sha256=sha256_file(diff_output) if diff_output.is_file() else None,
                detail=comparison["detail"],
            )
        evaluations.append(evaluation)
    matches = [e for e in evaluations if e["relation"] == "match"]
    mismatches = [e for e in evaluations if e["relation"] == "mismatch"]
    passed = (not matches or any(e["satisfied"] for e in matches)) and all(
        e["satisfied"] for e in mismatches
    )
    # An unsuccessful render cannot prove a mismatch. Keep its failure visible
    # if it is needed for the overall verdict, while allowing alternate matches.
    failures = [e for e in evaluations if "render_failed" in e and
                (e["relation"] == "mismatch" or not any(m["satisfied"] for m in matches))]
    classification = "pass" if passed else failures[0]["render_failed"] if failures else "mismatch"
    if passed:
        primary = next(e for e in evaluations if e["satisfied"])
    elif failures:
        primary = failures[0]
    else:
        # An unmatched alternate is harmless when another match succeeds.
        # Prefer the failed mismatch requirement when it decides the verdict.
        primary = next((e for e in mismatches if not e["satisfied"]),
                       next((e for e in matches if not e["satisfied"]), evaluations[0]))
    primary_index = evaluations.index(primary)
    if primary_index and "render_failed" not in primary:
        # Cluster consumers read plain names; their pixels must describe the
        # same reference as the compatibility fields in the report.
        shutil.copyfile(test_directory / f"reference_{primary_index}.ppm", reference_output_path)
        selected_diff = test_directory / f"diff_{primary_index}.ppm"
        if selected_diff.is_file():
            shutil.copyfile(selected_diff, diff_path)
    return {
        "id": test_id,
        "classification": classification,
        **{key: value for key, value in primary.items() if key != "satisfied"},
        "actual_sha256": sha256_file(actual_path),
        "references": evaluations,
    }


def extract_classifications(data: dict) -> dict:
    """Reads a test-id -> classification map out of either JSON shape this
    tool produces: a full report.json (`tests` is a list of per-test dicts
    with `id`/`classification`) or a --compact one (`tests` is already an
    id -> classification map). --baseline accepts either."""
    tests = data.get("tests", {})
    if isinstance(tests, dict):
        return dict(tests)
    return {test["id"]: test["classification"] for test in tests}


def compare_to_baseline(current: dict, baseline: dict) -> dict:
    """Zero-loss-gate comparison: `lost` (was pass, now is not) is the only
    bucket that should ever fail a gate. `gained` is the mirror image;
    `changed` covers a reclassification that does not cross the pass
    boundary (e.g. mismatch -> error); `new`/`removed` cover test ids only
    one side has (a WPT update, a --limit change, or a discovery-filter fix
    like the inline-event-handler one SHELL-L4 landed)."""
    gained, lost, changed, unchanged = [], [], [], []
    for test_id in sorted(set(current) & set(baseline)):
        before, after = baseline[test_id], current[test_id]
        if before == after:
            unchanged.append(test_id)
        elif before != "pass" and after == "pass":
            gained.append(test_id)
        elif before == "pass" and after != "pass":
            lost.append(test_id)
        else:
            changed.append(test_id)
    new = sorted(set(current) - set(baseline))
    removed = sorted(set(baseline) - set(current))
    return {
        "gained": gained, "lost": lost, "changed": changed, "unchanged": unchanged,
        "new": new, "removed": removed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wpt-root", required=True)
    parser.add_argument("--subdir", required=True, help="e.g. css/css-backgrounds")
    parser.add_argument("--exe", required=True, help="path to the amazing binary")
    parser.add_argument("--out", required=True, help="output directory for report.json + per-test artifacts")
    parser.add_argument("--width", type=int, default=800)
    parser.add_argument("--height", type=int, default=600)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--limit", type=int, default=0, help="0 = every eligible reftest in the subdir")
    parser.add_argument(
        "--jobs", type=int, default=1,
        help="concurrent test pipelines (each spawns its own render-wpt subprocesses); "
             "1 = serial (default, matches the original behaviour)",
    )
    parser.add_argument(
        "--baseline", default=None,
        help="a previous report.json or --compact-written compact.json (either shape is accepted) "
             "to diff this run against; prints gained/lost/changed/unchanged/new/removed test-id "
             "counts and exits 2 if any previously-passing test is lost (a zero-loss gate).",
    )
    parser.add_argument(
        "--compact", action="store_true",
        help="also write compact.json next to report.json: {schema, subdir, wpt_revision, counts, "
             "tests: {id: classification}} with none of report.json's per-test detail "
             "(relation/pixel counts/sha256/stdout). Meant for committing as a future --baseline "
             "instead of the full report, so evidence stays small as categories accumulate runs.",
    )
    parser.add_argument(
        "--font-set",
        default="Amazing text provider: WPT @font-face via HarfRust 0.13.2 + Swash 0.2.10; deterministic fallback otherwise",
    )
    args = parser.parse_args()

    wpt_root = Path(args.wpt_root).resolve()
    exe = Path(args.exe).resolve()
    out_dir = Path(args.out).resolve()
    if not exe.is_file():
        raise SystemExit(f"wpt_render_compare: amazing executable not found: {exe}")
    if args.width < 1 or args.width > 8192 or args.height < 1 or args.height > 8192:
        raise SystemExit("wpt_render_compare: width/height must be between 1 and 8192")
    if not math.isfinite(args.timeout) or args.timeout < 1 or args.timeout > 600:
        raise SystemExit("wpt_render_compare: timeout must be finite and between 1 and 600 seconds")

    revision = subprocess.run(
        ["git", "-C", str(wpt_root), "rev-parse", "HEAD"], capture_output=True, text=True
    )
    wpt_revision = revision.stdout.strip() if revision.returncode == 0 else "unknown"

    selected, inventory = discover(wpt_root, args.subdir, args.limit)

    run_name = "run-{}-{}".format(time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()), os.getpid())
    run_directory = out_dir / run_name
    run_directory.mkdir(parents=True, exist_ok=True)

    jobs = max(1, args.jobs)
    results_by_index = {}
    print_lock = threading.Lock()

    def run_and_report(index, candidate):
        result = run_one_test(candidate, exe, wpt_root, run_directory, args.width, args.height, args.timeout)
        with print_lock:
            print(f"{result['classification'].upper():<10} {result['id']}")
        return index, result

    if jobs == 1:
        for index, candidate in enumerate(selected):
            _, result = run_and_report(index, candidate)
            results_by_index[index] = result
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            futures = [pool.submit(run_and_report, index, candidate) for index, candidate in enumerate(selected)]
            for future in concurrent.futures.as_completed(futures):
                index, result = future.result()
                results_by_index[index] = result
    results = [results_by_index[index] for index in range(len(selected))]

    counts = {"pass": 0, "mismatch": 0, "error": 0, "timeout": 0, "unsupported": 0, "crash": 0}
    for result in results:
        counts[result["classification"]] += 1

    metadata = {
        "schema": 1,
        "wpt_revision": wpt_revision,
        "subdir": args.subdir,
        "viewport": {"width": args.width, "height": args.height, "device_scale": 1},
        "font_set": args.font_set,
        "host": socket.gethostname(),
        "generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    report = {**metadata, "run_directory": str(run_directory), "inventory": inventory, "counts": counts, "tests": results}

    baseline_compare = None
    exit_code = 0
    if args.baseline:
        baseline_data = json.loads(Path(args.baseline).read_text())
        baseline_map = extract_classifications(baseline_data)
        current_map = {result["id"]: result["classification"] for result in results}
        baseline_compare = compare_to_baseline(current_map, baseline_map)
        report["baseline_compare"] = {"baseline": str(Path(args.baseline).resolve()), **baseline_compare}
        counted = {key: len(value) for key, value in baseline_compare.items()}
        print(
            "Baseline compare ({}): gained={gained} lost={lost} changed={changed} "
            "unchanged={unchanged} new={new} removed={removed}".format(args.subdir, **counted)
        )
        if baseline_compare["lost"]:
            print(f"LOST (previously passing, now not): {', '.join(baseline_compare['lost'])}")
            exit_code = 2

    report_path = run_directory / "report.json"
    report_path.write_text(json.dumps(report, indent=2))
    latest_path = out_dir / "report.json"
    latest_path.write_text(json.dumps(report, indent=2))
    print(f"Report: {report_path}")
    print(
        "Counts: pass={pass} mismatch={mismatch} error={error} timeout={timeout} "
        "unsupported={unsupported} crash={crash}".format(**counts)
    )

    if args.compact:
        compact = {
            **metadata,
            "counts": counts,
            "tests": {result["id"]: result["classification"] for result in results},
        }
        if baseline_compare is not None:
            compact["baseline_compare"] = report["baseline_compare"]
        compact_path = run_directory / "compact.json"
        compact_path.write_text(json.dumps(compact, indent=2))
        (out_dir / "compact.json").write_text(json.dumps(compact, indent=2))
        print(f"Compact: {compact_path}")

    return exit_code


if __name__ == "__main__":
    sys.exit(main())
