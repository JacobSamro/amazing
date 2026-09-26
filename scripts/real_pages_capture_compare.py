#!/usr/bin/env python3
"""Per-page record + report builder for the tests/real-pages screenshot
regression harness (SHELL-L6). Driven by scripts/linux-shell-capture.sh's
--real-pages mode, which does the capturing (Xvfb + the real amazing-shell
binary + xdotool + ImageMagick `import`) and calls this script twice per
page's lifecycle:

  --record    once per page, right after its actual.png (or lack of one) is
              known: classifies the page (pass/mismatch/new/crash/excluded),
              runs `compare -metric AE` against a baseline PNG if one exists
              and the page isn't excluded, writes a diff PNG only for an
              actual mismatch (evidence stays small: no diff PNG for pass,
              new or excluded pages), and appends one JSON line to
              <out-dir>/pages.jsonl.

  --finalize  once at the end: reads pages.jsonl, writes <out-dir>/report.json
              (schema, captured_on, host, counts, one entry per page) and
              prints a summary line. Exits 2 if any non-excluded page is
              mismatch, crash or error -- pixel regression against a
              committed baseline is always a failure here (unlike the WPT
              render gate, a screenshot fixture is not expected to change
              pixel-for-pixel without a deliberate baseline update).

A page is "new" (not a failure) when no baseline PNG exists yet for it --
the first run of this harness establishes baselines by promoting `new`
captures into crates/amazing-shell-gpui/evidence/real-pages/. "excluded"
covers fixtures whose content is genuinely non-deterministic across runs
(tests/real-pages/assets/clock.js renders Date.now(); see EXCLUDE_SLUGS)
and is never counted toward pass/fail.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

# Fixtures that render wall-clock time (assets/clock.js: `Date.now()` written
# straight into the DOM) and so will never pixel-match a fixed baseline two
# runs apart. Captured for visual inspection like every other page, but never
# gated. Re-check this list with `grep -l clock.js tests/real-pages/pages/*`
# if the corpus changes.
EXCLUDE_SLUGS = {"news-live", "app-calendar"}


def run_compare(actual: Path, baseline: Path, diff_out: Path, fuzz: str) -> tuple[str, int | None]:
    """Returns (status, differing_pixel_count). status is 'match', 'differs',
    or 'error' (dimension mismatch, unreadable file, etc -- compare's message
    lands in the returned count's place as None)."""
    result = subprocess.run(
        ["compare", "-metric", "AE", "-fuzz", f"{fuzz}%", str(actual), str(baseline), str(diff_out)],
        capture_output=True, text=True,
    )
    text = (result.stderr or result.stdout or "").strip()
    try:
        # Classic ImageMagick prints a plain integer pixel count; HDRI builds
        # (e.g. the Mac's Homebrew ImageMagick 7 Q16-HDRI) print a float
        # ("200.261 (0.500654)") -- parse as float and round either way.
        count = int(round(float(text.split()[0])))
    except (ValueError, IndexError):
        return "error", None
    return ("match" if count == 0 else "differs"), count


def cmd_record(args: argparse.Namespace) -> int:
    out_dir = Path(args.out_dir)
    actual = out_dir / f"{args.slug}.png"
    record = {
        "slug": args.slug, "category": args.category, "title": args.title,
        "url": args.url,
    }
    excluded = args.slug in EXCLUDE_SLUGS
    if not args.had_window or not actual.exists():
        record["classification"] = "crash"
        record["detail"] = "no shell window / no capture produced within --wait"
    elif excluded:
        record["classification"] = "excluded"
        record["detail"] = "known non-deterministic fixture (clock.js), captured but not gated"
    elif not args.baseline:
        record["classification"] = "new"
        record["detail"] = "no committed baseline yet"
    else:
        baseline = Path(args.baseline)
        if not baseline.exists():
            record["classification"] = "new"
            record["detail"] = f"no committed baseline yet at {baseline}"
        else:
            diff_path = out_dir / f"{args.slug}-diff.png"
            status, count = run_compare(actual, baseline, diff_path, args.fuzz)
            if status == "match":
                record["classification"] = "pass"
                record["differing_pixels"] = 0
                diff_path.unlink(missing_ok=True)
            elif status == "differs":
                record["classification"] = "mismatch"
                record["differing_pixels"] = count
                record["baseline"] = str(baseline)
            else:
                record["classification"] = "error"
                record["detail"] = "compare failed (dimension mismatch or unreadable PNG)"
                diff_path.unlink(missing_ok=True)
    with open(out_dir / "pages.jsonl", "a") as f:
        f.write(json.dumps(record) + "\n")
    print(f"{record['classification']:9s} {args.slug}" + (f" (ae={record['differing_pixels']})" if "differing_pixels" in record and record["differing_pixels"] else ""))
    return 0


def cmd_finalize(args: argparse.Namespace) -> int:
    out_dir = Path(args.out_dir)
    jsonl = out_dir / "pages.jsonl"
    pages = [json.loads(line) for line in jsonl.read_text().splitlines() if line.strip()] if jsonl.exists() else []
    counts: dict[str, int] = {}
    for page in pages:
        counts[page["classification"]] = counts.get(page["classification"], 0) + 1
    report = {
        "schema": 1,
        "captured_on": args.captured_on,
        "lane": args.lane,
        "host": "pool-desktop Boat sandbox (Xvfb + Mesa lavapipe software Vulkan, no GPU)",
        "capture_tool": "scripts/linux-shell-capture.sh --real-pages",
        "fuzz_pct": args.fuzz,
        "counts": counts,
        "pages": pages,
    }
    (out_dir / "report.json").write_text(json.dumps(report, indent=2))
    print("Counts: " + " ".join(f"{k}={v}" for k, v in sorted(counts.items())))
    failing = [p["slug"] for p in pages if p["classification"] in ("mismatch", "crash", "error")]
    if failing:
        print(f"FAILING ({len(failing)}): {', '.join(failing)}")
        return 2
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)

    record = sub.add_parser("record")
    record.add_argument("--out-dir", required=True)
    record.add_argument("--slug", required=True)
    record.add_argument("--category", default="")
    record.add_argument("--title", default="")
    record.add_argument("--url", default="")
    record.add_argument("--baseline", default="")
    record.add_argument("--fuzz", default="0")
    record.add_argument("--had-window", type=int, default=0)
    record.set_defaults(func=cmd_record)

    finalize = sub.add_parser("finalize")
    finalize.add_argument("--out-dir", required=True)
    finalize.add_argument("--lane", default="shell")
    finalize.add_argument("--captured-on", required=True)
    finalize.add_argument("--fuzz", default="0")
    finalize.set_defaults(func=cmd_finalize)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
