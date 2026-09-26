#!/usr/bin/env python3
"""Run the fixed STORE-5 IndexedDB WPT selection through Amazing's installed runner."""
import concurrent.futures
import json
import os
import pathlib
import subprocess
import sys
import tempfile

binary, wpt_root, output = map(pathlib.Path, sys.argv[1:4])
selection = pathlib.Path(__file__).with_name("baseline-summary.json")
files = [entry["file"] for entry in json.loads(selection.read_text())["files"]]
output.mkdir(parents=True, exist_ok=True)

def run(test):
    environment = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="profile-", dir=output) as profile:
        environment["AMAZING_STORAGE_DIR"] = profile
        result = subprocess.run(
            [str(binary), "wpt-harness", str(wpt_root), test, "800", "600", "30000"],
            text=True, capture_output=True, timeout=45, env=environment,
        )
    try:
        report = json.loads(result.stdout.strip().splitlines()[-1])
    except (IndexError, ValueError):
        report = {"test": test, "status": "error", "detail": result.stdout + result.stderr,
                  "counts": {"pass": 0, "fail": 0, "timeout": 0, "notrun": 0, "error": 1}}
    (output / (pathlib.Path(test).name.replace(".", "_") + ".json")).write_text(json.dumps(report, separators=(",", ":")) + "\n")
    return report

with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    reports = list(pool.map(run, files))
totals = {key: sum(report.get("counts", {}).get(key, 0) for report in reports)
          for key in ("pass", "fail", "timeout", "notrun", "error")}
statuses = {status: sum(report.get("status") == status for report in reports)
            for status in ("pass", "fail", "timeout", "error", "unsupported")}
summary = {"base": "34049fcc", "wpt_revision": "c2a2fce69e5c2509d9dc28440ec8b42532e5c293",
           "files": len(files), "totals": totals, "statuses": statuses}
(output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, sort_keys=True))
