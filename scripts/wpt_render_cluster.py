#!/usr/bin/env python3
"""Groups a wpt_render_compare.py report's mismatches by what visibly differs,
so a lane can triage its largest visual-failure clusters first instead of
opening every diff image by hand.

For each mismatch, reads the actual/reference/diff PPM triple
wpt_render_compare.py leaves next to report.json (those stay on the sandbox;
only this summary travels) and computes:
  - the diff bounding box: where the differing pixels are, as a size bucket
    (full-frame/large-region/medium-region/small-region) and a 3x3 location
    bucket (e.g. "top-left", "center", "bottom-right");
  - a coarse colour-delta bucket from the average actual-vs-reference colour
    sampled on a grid inside that box (lighter/darker/redder/bluer/greener/
    "same tone, different shape").
Mismatches sharing a (size, location, colour) signature are one cluster.
This is a triage heuristic, not a classifier -- it is meant to cut "let me
look at all 61 mismatches one by one" down to "here are the 3 shapes of
failure and an example of each."

Usage: wpt_render_cluster.py <report.json> [--top N]
Writes <run_directory>/clusters.json (schema below) and prints the top N.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wpt_render_compare import read_ppm  # noqa: E402  (reuses the P6 PPM reader)

GRID = 8  # sample an 8x8 grid inside the bounding box for the colour bucket


def diff_bbox(diff_pixels: bytes, width: int, height: int):
    """Magenta (255,0,255) marks a differing pixel, white (255,255,255) a
    matching one (see wpt_render_compare.compare_ppm). The green channel
    alone (0 vs 255) tells them apart, so scan rows with fast C-level
    bytes.find/rfind instead of a per-pixel Python loop."""
    green = diff_pixels[1::3]
    min_x = min_y = None
    max_x = max_y = -1
    for y in range(height):
        row = green[y * width:(y + 1) * width]
        first = row.find(0)
        if first == -1:
            continue
        last = row.rfind(0)
        if min_y is None:
            min_y = y
        max_y = y
        if min_x is None or first < min_x:
            min_x = first
        if last > max_x:
            max_x = last
    if min_x is None:
        return None
    return min_x, min_y, max_x, max_y


def size_bucket(bbox, width: int, height: int) -> str:
    x0, y0, x1, y1 = bbox
    area_fraction = ((x1 - x0 + 1) * (y1 - y0 + 1)) / max(1, width * height)
    if area_fraction >= 0.6:
        return "full-frame"
    if area_fraction >= 0.15:
        return "large-region"
    if area_fraction >= 0.02:
        return "medium-region"
    return "small-region"


def location_bucket(bbox, width: int, height: int) -> str:
    x0, y0, x1, y1 = bbox
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    column = "left" if cx < width / 3 else "right" if cx > 2 * width / 3 else "center"
    row = "top" if cy < height / 3 else "bottom" if cy > 2 * height / 3 else "middle"
    if row == "middle" and column == "center":
        return "center"
    if row == "middle":
        return column
    if column == "center":
        return row
    return f"{row}-{column}"


def sampled_average(pixels: bytes, width: int, bbox) -> tuple[float, float, float]:
    x0, y0, x1, y1 = bbox
    xs = _grid_points(x0, x1)
    ys = _grid_points(y0, y1)
    totals = [0, 0, 0]
    count = 0
    for y in ys:
        row = y * width * 3
        for x in xs:
            offset = row + x * 3
            totals[0] += pixels[offset]
            totals[1] += pixels[offset + 1]
            totals[2] += pixels[offset + 2]
            count += 1
    count = max(1, count)
    return totals[0] / count, totals[1] / count, totals[2] / count


def _grid_points(lo: int, hi: int):
    span = hi - lo
    if span <= 0:
        return [lo]
    step = max(1, span // (GRID - 1)) if GRID > 1 else span
    points = sorted({min(hi, lo + i * step) for i in range(GRID)})
    return points


def colour_bucket(actual_avg, reference_avg) -> str:
    dr, dg, db = (actual_avg[index] - reference_avg[index] for index in range(3))
    brightness = (dr + dg + db) / 3
    if abs(brightness) < 10 and max(abs(dr), abs(dg), abs(db)) < 15:
        return "same tone, different shape"
    if abs(brightness) >= 25:
        return "lighter than reference" if brightness > 0 else "darker than reference"
    dominant = max((dr, "redder"), (dg, "greener"), (db, "bluer"), (-dr, "less red"), (-dg, "less green"), (-db, "less blue"))
    return dominant[1] + " than reference"


def cluster_report(report: dict, report_path: Path):
    run_directory = report_path.parent
    width, height = report["viewport"]["width"], report["viewport"]["height"]
    clusters: dict[str, dict] = {}
    unreadable = 0
    for test in report["tests"]:
        if test["classification"] != "mismatch":
            continue
        directory_id = test.get("id")
        # The run directory keys per-test folders by sha256(id)[:16], not the
        # id itself; recompute it the same way wpt_render_compare.py does.
        import hashlib

        folder = run_directory / hashlib.sha256(directory_id.encode("utf-8")).hexdigest()[:16]
        diff_path, actual_path, reference_path = folder / "diff.ppm", folder / "actual.ppm", folder / "reference.ppm"
        if not (diff_path.is_file() and actual_path.is_file() and reference_path.is_file()):
            unreadable += 1
            continue
        diff_width, diff_height, diff_pixels = read_ppm(diff_path)
        bbox = diff_bbox(diff_pixels, diff_width, diff_height)
        if bbox is None:
            unreadable += 1
            continue
        _, _, actual_pixels = read_ppm(actual_path)
        _, _, reference_pixels = read_ppm(reference_path)
        actual_avg = sampled_average(actual_pixels, diff_width, bbox)
        reference_avg = sampled_average(reference_pixels, diff_width, bbox)
        signature = f"{size_bucket(bbox, width, height)}, {location_bucket(bbox, width, height)}, {colour_bucket(actual_avg, reference_avg)}"
        bucket = clusters.setdefault(signature, {"signature": signature, "count": 0, "examples": []})
        bucket["count"] += 1
        if len(bucket["examples"]) < 5:
            bucket["examples"].append(directory_id)
    ranked = sorted(clusters.values(), key=lambda item: -item["count"])
    return {
        "schema": 1,
        "subdir": report["subdir"],
        "mismatch_count": sum(1 for test in report["tests"] if test["classification"] == "mismatch"),
        "unreadable": unreadable,
        "clusters": ranked,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", help="path to a wpt_render_compare.py report.json")
    parser.add_argument("--top", type=int, default=3)
    args = parser.parse_args()

    report_path = Path(args.report).resolve()
    report = json.loads(report_path.read_text())
    clustered = cluster_report(report, report_path)
    out_path = report_path.parent / "clusters.json"
    out_path.write_text(json.dumps(clustered, indent=2))
    print(f"Clusters: {out_path}")
    for entry in clustered["clusters"][: args.top]:
        print(f"  {entry['count']:>4}  {entry['signature']}  (e.g. {', '.join(entry['examples'][:2])})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
