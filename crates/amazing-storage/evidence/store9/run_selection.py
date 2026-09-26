#!/usr/bin/env python3
"""Run the widened STORE-9 IndexedDB WPT selection through Amazing's installed runner.

    python run_selection.py <amazing-binary> <wpt-root> <output-dir> [base-label] [--detail]

Reads `selection.json` next to this script for the pinned file list. Each file runs
in its own fresh AMAZING_STORAGE_DIR profile so databases do not leak between tests.
With --detail, also prints every non-pass subtest's name/status/message to stdout
(after the summary line) for clustering triage.
"""
import concurrent.futures
import json
import os
import pathlib
import subprocess
import sys
import tempfile

binary, wpt_root, output = map(pathlib.Path, sys.argv[1:4])
rest = sys.argv[4:]
detail = "--detail" in rest
rest = [a for a in rest if a != "--detail"]
base = rest[0] if rest else "unknown"
here = pathlib.Path(__file__).resolve().parent
selection = json.loads((here / "selection.json").read_text())
files = [entry["file"] for entry in selection["files"]]
output.mkdir(parents=True, exist_ok=True)


def run(test):
    environment = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="profile-", dir=output) as profile:
        environment["AMAZING_STORAGE_DIR"] = profile
        try:
            result = subprocess.run(
                [str(binary), "wpt-harness", str(wpt_root), test, "800", "600", "30000"],
                text=True, capture_output=True, timeout=90, env=environment,
            )
        except subprocess.TimeoutExpired:
            result = None
    name = pathlib.Path(test).name.replace(".", "_") + ".jsonl"
    if result is None:
        report = {"test": test, "status": "timeout", "detail": "runner timeout",
                  "counts": {"pass": 0, "fail": 0, "timeout": 1, "notrun": 0, "error": 0},
                  "subtests": []}
    else:
        try:
            report = json.loads(result.stdout.strip().splitlines()[-1])
        except (IndexError, ValueError):
            report = {"test": test, "status": "error",
                      "detail": (result.stdout + result.stderr)[-4000:],
                      "counts": {"pass": 0, "fail": 0, "timeout": 0, "notrun": 0, "error": 1},
                      "subtests": []}
    (output / name).write_text(json.dumps(report, separators=(",", ":")) + "\n")
    return report


with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    reports = list(pool.map(run, files))
totals = {key: sum(report.get("counts", {}).get(key, 0) for report in reports)
          for key in ("pass", "fail", "timeout", "notrun", "error")}
statuses = {status: sum(report.get("status") == status for report in reports)
            for status in ("pass", "fail", "timeout", "error", "unsupported")}
summary = {"base": base, "wpt_revision": selection["wpt_revision"],
           "files": len(files), "totals": totals, "statuses": statuses,
           "per_file": [{"file": test, "status": report.get("status"),
                         "counts": report.get("counts", {})}
                        for test, report in zip(files, reports)]}
(output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in ("base", "files", "totals", "statuses")}, sort_keys=True))

if detail:
    for test, report in zip(files, reports):
        if report.get("status") == "pass":
            continue
        print(f"### {test} [{report.get('status')}]")
        if report.get("detail"):
            print("DETAIL:", report["detail"][:2000])
        status_names = {0: "pass", 1: "fail", 2: "timeout", 3: "notrun"}
        for subtest in report.get("subtests", []):
            code = subtest.get("status")
            if code != 0:
                print(f"  - {subtest.get('name')!r} status={status_names.get(code, code)} message={subtest.get('message')!r}")
