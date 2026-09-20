"""Normalize retained Windows WPT evidence without rerunning the engine.

Usage: python normalize.py collected.json workspace.log workspace.exit source_base output_dir
"""
import hashlib
import json
from pathlib import Path
import re
import sys

collected, workspace, exit_file, source_base, output_dir = sys.argv[1:]
data = json.loads(Path(collected).read_text(encoding="utf-8-sig"))
out = Path(output_dir)
out.mkdir(parents=True, exist_ok=True)
fields = ["id", "classification", "relation", "differing_pixels", "max_difference",
          "actual_sha256", "reference_sha256", "diff_sha256"]

def projection(report):
    return sorted([{key: test[key] for key in fields if key in test}
                   for test in report["tests"]], key=lambda test: test["id"])

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()

summary = {
    "ticket": "LAYOUT-2", "source_base": source_base,
    "wpt_revision": "c2a2fce69e5c2509d9dc28440ec8b42532e5c293",
    "viewport": {"width": 800, "height": 600},
    **{key: data[key] for key in ["baseline_executable", "final_executable",
       "baseline_executable_sha256", "final_executable_sha256", "lockfile_sha256", "sources"]},
    "suites": {},
}
for suite in ["geometry", "floats", "block-collapse", "position", "text-flow", "flex-grid", "paint-compositing"]:
    before = [data["reports"][f'{data["baseline_phase"]}/{suite}-{n}'] for n in [1, 2]]
    after = [data["reports"][f'{data["final_phase"]}/{suite}-{n}'] for n in [1, 2]]
    for runs in [before, after]:
        assert all(r["wpt_revision"] == summary["wpt_revision"] for r in runs), suite
        assert projection(runs[0]) == projection(runs[1]), f"Unstable {suite}"
        assert all(r["counts"][k] == 0 for r in runs for k in ["timeout", "unsupported", "crash"]), suite
    bp, ap = projection(before[0]), projection(after[0])
    bmap = {test["id"]: test for test in bp}
    assert set(bmap) == {test["id"] for test in ap}, f"Selection drift {suite}"
    changes = [{"id": test["id"], "before": bmap[test["id"]]["classification"], "after": test["classification"]}
               for test in ap if bmap[test["id"]]["classification"] != test["classification"]]
    item = {
        "before": before[0]["counts"], "after": after[0]["counts"],
        "before_runs": [r["run_directory"] for r in before], "after_runs": [r["run_directory"] for r in after],
        "before_projection_sha256": digest(bp), "after_projection_sha256": digest(ap),
        "repeat_stable": True, "classification_changes": changes,
    }
    summary["suites"][suite] = item
    result = {"wpt_revision": summary["wpt_revision"], "viewport": after[0]["viewport"],
              "manifest": after[0]["manifest"], "font_set": after[0]["font_set"],
              **item, "before_tests": bp, "after_tests": ap}
    (out / f"{suite}-results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(suite, item["before"], "->", item["after"], changes)
log = Path(workspace).read_text(encoding="utf-8-sig")
rows = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;", log)
assert rows and all(row[0] == "ok" for row in rows)
assert Path(exit_file).read_text(encoding="utf-8-sig").strip() == "0"
summary["workspace"] = {
    "command": "cargo test --workspace --locked", "exit_code": 0,
    "passed": sum(int(row[1]) for row in rows), "failed": sum(int(row[2]) for row in rows),
    "ignored": sum(int(row[3]) for row in rows), "summary_count": len(rows),
    "log_sha256": hashlib.sha256(Path(workspace).read_bytes()).hexdigest(),
}
(out / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
print("workspace", summary["workspace"])
