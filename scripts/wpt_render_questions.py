#!/usr/bin/env python3
"""Appends one `-` triage line per owning lane to .agents/<lane>/questions.md,
naming that lane's top 3 mismatch clusters (by count, across every category
SHELL-L4 attributed to it) from the SHELL-L4 fleet-wide Linux render
baseline. Source data is each category's clusters.json
(scripts/wpt_render_cluster.py), already copied into the evidence directory
by scripts/wpt_render_summary.py.

Usage: wpt_render_questions.py <evidence-dir> <main-checkout-dir> [--date YYYY-MM-DD]
  [--ticket LABEL] [--only-lane LANE] [--slug-prefix PREFIX]
  <evidence-dir> is crates/amazing-shell-gpui/evidence/wpt-render (one
  <slug>/{report,clusters}.json, or {compact,clusters}.json for a
  --compact-baselined category, per category).
  <main-checkout-dir> is the repo checkout that holds .agents/ (gitignored,
  not the lane worktree) -- e.g. /Users/jMac/Projects/OpenSource/amazing.
  --ticket names the run in the appended line (default "SHELL-L4, Linux
  render baseline"); --only-lane appends for exactly one owning lane instead
  of recomputing and re-appending for all five, so a later, smaller run (a
  new corpus subtree, not every category) does not add noise to lanes it
  did not touch. --slug-prefix restricts which evidence subdirectories count
  at all (e.g. "svg-"), so a small addition's clusters are not drowned out by
  the top-3-by-count selection across a lane's whole, much larger, existing
  category set.
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wpt_render_summary import owning_lane  # noqa: E402

LANES = ["css", "layout", "layout-flexgrid", "text", "replaced-content", "compositor"]


def main() -> int:
    if len(sys.argv) < 3:
        raise SystemExit("usage: wpt_render_questions.py <evidence-dir> <main-checkout-dir> [--date YYYY-MM-DD]")
    evidence_dir = Path(sys.argv[1]).resolve()
    main_dir = Path(sys.argv[2]).resolve()
    date = time.strftime("%Y-%m-%d", time.gmtime())
    if "--date" in sys.argv:
        date = sys.argv[sys.argv.index("--date") + 1]
    ticket = "SHELL-L4, Linux render baseline"
    if "--ticket" in sys.argv:
        ticket = sys.argv[sys.argv.index("--ticket") + 1]
    only_lane = None
    if "--only-lane" in sys.argv:
        only_lane = sys.argv[sys.argv.index("--only-lane") + 1]
    slug_prefix = None
    if "--slug-prefix" in sys.argv:
        slug_prefix = sys.argv[sys.argv.index("--slug-prefix") + 1]

    per_lane: dict[str, list[dict]] = {lane: [] for lane in LANES}
    for slug_dir in sorted(evidence_dir.iterdir()):
        if slug_prefix and not slug_dir.name.startswith(slug_prefix):
            continue
        clusters_path = slug_dir / "clusters.json"
        report_path = slug_dir / "report.json"
        if not report_path.is_file():
            report_path = slug_dir / "compact.json"
        if not (clusters_path.is_file() and report_path.is_file()):
            continue
        lane = owning_lane(slug_dir.name)
        if lane not in per_lane:
            continue
        if only_lane and lane != only_lane:
            continue
        report = json.loads(report_path.read_text())
        clustered = json.loads(clusters_path.read_text())
        for entry in clustered["clusters"]:
            per_lane[lane].append(
                {
                    "subdir": report["subdir"],
                    "signature": entry["signature"],
                    "count": entry["count"],
                    "examples": entry["examples"],
                }
            )

    for lane, entries in per_lane.items():
        entries.sort(key=lambda item: -item["count"])
        top = entries[:3]
        if not top:
            continue
        parts = []
        for rank, entry in enumerate(top, start=1):
            example = entry["examples"][0] if entry["examples"] else "?"
            parts.append(f"({rank}) {entry['subdir']}: {entry['signature']} -- {entry['count']} cases, e.g. {example}")
        line = (
            f"- {date} shell ({ticket}, no win-001 sweep involved): "
            f"top mismatch clusters for your owned categories: {'; '.join(parts)}. "
            f"Full per-test reports and diff/actual/reference sha256: "
            f"crates/amazing-shell-gpui/evidence/wpt-render/<category>/{{report,clusters}}.json "
            f"(category->lane mapping is a first-pass heuristic in scripts/wpt_render_summary.py, "
            f"correct it in your own decision map if it misattributed a category to you).\n"
        )
        questions_path = main_dir / ".agents" / lane / "questions.md"
        with open(questions_path, "a") as handle:
            handle.write(line)
        print(f"appended to {questions_path}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
