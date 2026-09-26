#!/usr/bin/env python3
"""Builds the SHELL-L4 fleet-wide summary table from every category's
report.json (as scripts/linux-render-wpt.sh copies them back to
.agents/<lane>/wpt-render/<slug>/report.json), and copies each category's
report.json + clusters.json into the evidence directory this script commits.

Owning-lane mapping is a first-pass judgment call keyed by directory name
against the AGENTS.md lane table (css: amazing-css/cascade+values; layout:
amazing-layout block/inline/table/position/floats; layout-flexgrid:
amazing-layout flex*/grid*; text: amazing-text(-provider); replaced-content:
amazing-paint/media/image/svg). Categories with no strong signal default to
`css` as the general-cascade catch-all. Owning lanes should correct this in
their own decision maps rather than treat it as authoritative.

Usage: wpt_render_summary.py <reports-dir> <evidence-dir>
  <reports-dir> is .agents/shell/wpt-render (one <slug>/report.json +
  <slug>/clusters.json per category already copied back by linux-render-wpt.sh).
  Writes <evidence-dir>/<slug>/{report,clusters}.json for every category with
  at least one eligible reftest, plus <evidence-dir>/summary.md and
  <evidence-dir>/summary.json.
"""
from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

LAYOUT_FLEXGRID = {"css-flexbox", "css-grid", "css-gaps"}

TEXT = {
    "css-fonts", "css-font-loading", "css-text", "css-text-decor", "css-ruby",
    "css-highlight-api", "css-writing-modes", "css-speech", "css-size-adjust", "WOFF2",
}

# SHELL-L8: animation/transition/transform timing and pixel-parity-under-motion
# categories move here from REPLACED_CONTENT -- they exercise the compositor's
# animation pipeline (ADR 0006), not a replaced-element's own paint. Static
# transform *rendering* without motion (e.g. a plain matrix() reftest) still
# looks like a REPLACED_CONTENT/paint concern in practice, but WPT doesn't
# split css-transforms that way, so the whole category goes to compositor.
COMPOSITOR = {
    "css-animations", "css-transitions", "css-transforms", "compositing",
    "css-view-transitions", "css-will-change", "css-easing",
}

REPLACED_CONTENT = {
    "css-backgrounds", "css-borders", "css-masking", "css-shadow", "filter-effects",
    "css-images", "css-image-animation", "css-color", "css-color-adjust", "css-color-hdr",
    "css-motion-path", "css-paint-api", "fill-stroke", "css-shapes",
    "css-forced-color-adjust",
}

LAYOUT = {
    "css-align", "css-anchor-position", "css-box", "css-break", "css-contain",
    "css-content", "css-counter-styles", "css-display", "css-exclusions", "css-forms",
    "css-gcpm", "css-inline", "css-lists", "css-logical", "css-multicol",
    "css-overflow", "css-overscroll-behavior", "css-page", "css-rhythm",
    "css-round-display", "css-scroll-anchoring", "css-scrollbars", "css-scroll-snap",
    "css-sizing", "css-tables", "css-viewport", "css-device-adapt", "css-navigation",
    "css-layout-api", "printing", "css-zoom", "geometry",
}

# Everything else (parsing/cascade/values/JS-API categories with no strong
# layout/paint/text/compositor signal, plus svg/html entries) defaults to `css`.
CSS_DEFAULT_NOTE = "default: cascade/values/selectors or a JS-only API, no strong layout/paint/text/compositor signal"


def owning_lane(slug: str) -> str:
    # slug is "css-<category>" for css/ subdirs, or "html-rendering" /
    # "svg-painting" / "svg-shapes" for the non-css ones the brief also asked for.
    if slug.startswith("svg-") or slug == "html-rendering":
        return "replaced-content"
    category = slug[len("css-"):] if slug.startswith("css-") else slug
    if category in LAYOUT_FLEXGRID:
        return "layout-flexgrid"
    if category in TEXT:
        return "text"
    if category in COMPOSITOR:
        return "compositor"
    if category in REPLACED_CONTENT:
        return "replaced-content"
    if category in LAYOUT:
        return "layout"
    return "css"


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: wpt_render_summary.py <reports-dir> <evidence-dir>")
    reports_dir = Path(sys.argv[1]).resolve()
    evidence_dir = Path(sys.argv[2]).resolve()
    evidence_dir.mkdir(parents=True, exist_ok=True)

    rows = []
    totals = {"pass": 0, "mismatch": 0, "error": 0, "timeout": 0, "unsupported": 0, "crash": 0}
    missing = []
    for slug_dir in sorted(reports_dir.iterdir()):
        # SHELL-L7: a category baselined with --compact only has compact.json
        # (no "inventory" key -- it drops everything but schema/counts/tests
        # to keep committed evidence small); fall back to it, and derive
        # "eligible" from the counts sum instead of the full report's
        # inventory field.
        report_path = slug_dir / "report.json"
        is_compact = False
        if not report_path.is_file():
            report_path = slug_dir / "compact.json"
            is_compact = True
        if not report_path.is_file():
            continue
        report = json.loads(report_path.read_text())
        counts = report["counts"]
        eligible = report["inventory"]["eligible_static_self_contained"] if not is_compact else sum(counts.values())
        if eligible == 0:
            continue  # nothing to triage or commit for this category
        for key in totals:
            totals[key] += counts[key]
        lane = owning_lane(slug_dir.name)
        rows.append(
            {
                "slug": slug_dir.name,
                "subdir": report["subdir"],
                "lane": lane,
                "eligible": eligible,
                **counts,
            }
        )
        target = evidence_dir / slug_dir.name
        target.mkdir(parents=True, exist_ok=True)
        shutil.copy(report_path, target / report_path.name)
        clusters_path = slug_dir / "clusters.json"
        if clusters_path.is_file():
            shutil.copy(clusters_path, target / "clusters.json")
        else:
            missing.append(slug_dir.name)

    rows.sort(key=lambda row: row["subdir"])
    lines = [
        "# SHELL-L4 fleet-wide Linux render baseline",
        "",
        f"Pinned WPT revision, viewport and font set: see each category's own `report.json`. "
        f"{len(rows)} categories with at least one eligible self-contained static reftest.",
        "",
        "| Category | Lane | Eligible | Pass | Mismatch | Error | Timeout | Unsupported | Crash |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for row in rows:
        lines.append(
            f"| `{row['subdir']}` | {row['lane']} | {row['eligible']} | {row['pass']} | "
            f"{row['mismatch']} | {row['error']} | {row['timeout']} | {row['unsupported']} | {row['crash']} |"
        )
    lines.append("")
    lines.append(
        "| **Total** | | **{eligible}** | **{pass}** | **{mismatch}** | **{error}** | "
        "**{timeout}** | **{unsupported}** | **{crash}** |".format(
            eligible=sum(row["eligible"] for row in rows), **totals
        )
    )
    (evidence_dir / "summary.md").write_text("\n".join(lines) + "\n")
    (evidence_dir / "summary.json").write_text(json.dumps({"totals": totals, "categories": rows}, indent=2))
    print(f"{len(rows)} categories, {sum(row['eligible'] for row in rows)} eligible reftests")
    print("Totals: " + " ".join(f"{k}={v}" for k, v in totals.items()))
    if missing:
        print(f"missing clusters.json for: {', '.join(missing)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
