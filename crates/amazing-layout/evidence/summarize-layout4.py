"""Normalize collected Windows reports and require two stable runs per selection.

This analyzes artifacts only; it does not run engine code or tests locally.
Usage: python summarize-layout4.py collected.json output-directory
"""
import hashlib
import json
from pathlib import Path
import re
import sys
import subprocess

source = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
out = Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
suites = ("tables", "geometry", "floats", "block-collapse", "position",
          "text-flow", "flex-grid", "paint-compositing", "images", "contents")
reports = {item["name"]: item["report"] for item in source["reports"]}
if len(reports) != len(source["reports"]):
    raise SystemExit("Duplicate reports: collection contains ambiguous reruns")
expected = {f"{phase}-{suite}-{repeat}" for phase in ("baseline", "final")
            for suite in suites for repeat in (1, 2)}
if set(reports) != expected:
    raise SystemExit(f"Missing {sorted(expected-set(reports))}; unexpected {sorted(set(reports)-expected)}")

def write(name, value):
    (out / name).write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")

def projection(report):
    return [{key: test.get(key) for key in
             ("id", "classification", "relation", "actual_sha256", "reference_sha256", "diff_sha256")}
            for test in sorted(report["tests"], key=lambda item: item["id"])]

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()

summary = {"schema": 1, "wpt_revision": "c2a2fce69e5c2509d9dc28440ec8b42532e5c293",
           "baseline_engine_commit": "ffb143b", "implementation_commit": "1419e48", "implementation_source_manifest": "final-sources.json",
           "viewport": [800, 600], "selections": {}}
for suite in suites:
    entry = {}
    for phase in ("baseline", "final"):
        pair = [reports[f"{phase}-{suite}-{i}"] for i in (1, 2)]
        if projection(pair[0]) != projection(pair[1]) or pair[0]["counts"] != pair[1]["counts"]:
            raise SystemExit(f"Unstable {phase}-{suite}")
        entry[phase] = {"counts": pair[0]["counts"], "stable_repeats": 2,
                        "projection_sha256": digest(projection(pair[0]))}
        for i, report in enumerate(pair, 1):
            if report["wpt_revision"] != summary["wpt_revision"]:
                raise SystemExit(f"Wrong WPT revision in {phase}-{suite}-{i}")
            if report["viewport"] != {"width": 800, "height": 600, "device_scale": 1}:
                raise SystemExit(f"Wrong viewport: {phase}-{suite}-{i}")
            report = dict(report)
            report["manifest"] = report["manifest"].replace("\\", "/").split("/src/wt/layout/")[-1]
            report["run_directory"] = report["run_directory"].replace("\\", "/").split("/")[-1]
            write(f"{phase}-{suite}-{i}.json", report)
    before = {t["id"]: t for t in reports[f"baseline-{suite}-1"]["tests"]}
    assert set(before) == {t["id"] for t in reports[f"final-{suite}-1"]["tests"]}
    assert reports[f"baseline-{suite}-1"]["font_set"] == reports[f"final-{suite}-1"]["font_set"]
    entry["classification_changes"] = [
        {"id": t["id"], "before": before[t["id"]]["classification"], "after": t["classification"]}
        for t in reports[f"final-{suite}-1"]["tests"]
        if before[t["id"]]["classification"] != t["classification"]]
    if any(entry[phase]["counts"][kind] for phase in ("baseline", "final")
           for kind in ("timeout", "unsupported", "crash")):
        raise SystemExit(f"Incomplete or failed execution in {suite}")
    regressions = [change for change in entry["classification_changes"]
                   if change["before"] == "pass" and change["after"] != "pass"]
    if regressions:
        raise SystemExit(f"Regression gate failed in {suite}: {regressions}")
    summary["selections"][suite] = entry
    print(suite, entry["baseline"]["counts"], "->", entry["final"]["counts"])


def nextest(name):
    lines = source["logs"][name]
    matches = [re.search(r"(\d+) tests run: (\d+) passed, (\d+) skipped", line) for line in lines]
    matches = [m for m in matches if m]
    if len(matches) != 1:
        raise SystemExit(f"Missing nextest summary: {name}")
    run, passed, skipped = map(int, matches[0].groups())
    if run != passed:
        raise SystemExit(f"Tests failed: {name}")
    return {"passed": passed, "skipped": skipped}

def docs(name):
    matches = [re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", line)
               for line in source["logs"][name]]
    matches = [tuple(map(int, m.groups())) for m in matches if m]
    if not matches:
        raise SystemExit(f"Missing doctest results: {name}")
    values = dict(zip(("passed", "failed", "ignored"), map(sum, zip(*matches))))
    if values["failed"]:
        raise SystemExit(f"Doctests failed: {name}")
    return values

summary["layout_tests"] = {p: nextest(f"layout4-{p}-tests.log") for p in ("dev-baseline", "final")}
summary["layout_doctests"] = {p: docs(f"layout4-{p}-doc.log") for p in ("dev-baseline", "final")}
summary["workspace"] = nextest("layout4-final-workspace.log")
summary["workspace_repeat"] = nextest("layout4-final-workspace-repeat.log")
summary["workspace_doctests"] = docs("layout4-final-workspace-doc.log")
summary["workspace_doctests_repeat"] = docs("layout4-final-workspace-doc-repeat.log")
assert summary["workspace"] == summary["workspace_repeat"]
assert summary["workspace_doctests"] == summary["workspace_doctests_repeat"]
summary["final_source_audit"] = source["audit"]
repository = Path(__file__).resolve().parents[3]
for path, expected_hash in source["gate_files"].items():
    if path in ("crates/amazing-layout/evidence/run-layout4.ps1", "crates/amazing-layout/evidence/contents-manifest.json"):
        data = (repository / path).read_bytes()
    else:
        data = subprocess.check_output(["git", "show", f"{summary['baseline_engine_commit']}:{path}"], cwd=repository)
    assert hashlib.sha256(data).hexdigest() == expected_hash
summary["gate_files"] = source["gate_files"]
for phase, build in source["builds"].items():
    manifest_bytes = (out / f"{phase}-sources.json").read_bytes()
    assert hashlib.sha256(manifest_bytes).hexdigest() == build["source_manifest_sha256"]
    assert len(json.loads(manifest_bytes)) == build["verified_source_files"]
    write(f"{phase}-build.json", build)
summary["builds"] = {p: {k: b[k] for k in (
    "executable_sha256", "image_executable_sha256", "source_manifest_sha256",
    "first_party_fingerprints_and_artifacts_removed", "incremental_compilation",
    "sources_verified_before_build_and_after_every_command")}
    for p, b in source["builds"].items()}
review_path = out / "review" / "review.json"
review = json.loads(review_path.read_text())
for phase, build_phase in (("baseline", "dev-baseline"), ("final", "final")):
    assert review[f"{phase}_executable_sha256"] == source["builds"][build_phase]["executable_sha256"]
    for case in review["cases"].values():
        report = reports[f"{phase}-{case['selection']}-1"]
        test = next(t for t in report["tests"] if t["id"] == case["report_id"])
        for kind in ("actual", "reference"):
            evidence = case[phase][kind]
            assert evidence["ppm_sha256"] == test[f"{kind}_sha256"]
            assert evidence["png_sha256"] == hashlib.sha256((review_path.parent / evidence["png"]).read_bytes()).hexdigest()
summary["pixel_review"] = {"path": "review/review.json", "artifact_and_report_hashes_verified": True}
write("summary.json", summary)
write("test-results.json", source["logs"])
print("layout", summary["layout_tests"], "workspace", summary["workspace"], "doctests", summary["workspace_doctests"])
