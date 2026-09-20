"""Normalize collected Windows reports and require two stable runs per selection.

This analyzes artifacts only; it does not run engine code or tests locally.
Usage: python summarize-layout3.py collected.json output-directory
"""
import hashlib
import json
from pathlib import Path
import re
import sys

source = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
out = Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
suites = ("tables", "geometry", "floats", "block-collapse", "position",
          "text-flow", "flex-grid", "paint-compositing", "images")
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
           "baseline_engine_commit": "79464ca", "implementation_commit": '7de4c47',
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
            report = dict(report)
            report["manifest"] = report["manifest"].replace("\\", "/").split("/src/wt/layout/")[-1]
            report["run_directory"] = report["run_directory"].replace("\\", "/").split("/")[-1]
            write(f"{phase}-{suite}-{i}.json", report)
    before = {t["id"]: t for t in reports[f"baseline-{suite}-1"]["tests"]}
    entry["classification_changes"] = [
        {"id": t["id"], "before": before[t["id"]]["classification"], "after": t["classification"]}
        for t in reports[f"final-{suite}-1"]["tests"]
        if before[t["id"]]["classification"] != t["classification"]]
    if any(entry[phase]["counts"][kind] for phase in ("baseline", "final")
           for kind in ("timeout", "unsupported", "crash")):
        raise SystemExit(f"Incomplete or failed execution in {suite}")
    if suite not in ("tables", "geometry"):
        regressions = [change for change in entry["classification_changes"]
                       if change["before"] == "pass" and change["after"] != "pass"]
        if regressions:
            raise SystemExit(f"Regression gate failed in {suite}: {regressions}")
    summary["selections"][suite] = entry
    print(suite, entry["baseline"]["counts"], "->", entry["final"]["counts"])

def counts(lines):
    matches = [tuple(map(int, match.groups())) for line in lines
               if (match := re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", line))]
    return dict(zip(("passed", "failed", "ignored"), map(sum, zip(*matches)))) | {"targets": len(matches)}

summary["layout_tests"] = {phase: counts(lines) for phase, lines in source["units"].items()}
summary["workspace"] = counts(source["workspace"])
summary["workspace_repeat"] = counts(source["workspace_repeat"])
summary["final_source_audit"] = source["audit"]
for phase, build in source["builds"].items():
    manifest = json.loads((out / f"{phase}-sources.json").read_text())
    manifest_hash = hashlib.sha256((out / f"{phase}-sources.json").read_bytes()).hexdigest()
    if manifest_hash != build["source_manifest_sha256"] or len(manifest) != build["verified_source_files"]:
        raise SystemExit(f"Source manifest provenance differs: {phase}")
if summary["workspace"] != summary["workspace_repeat"]:
    raise SystemExit("Workspace repetitions disagree")
summary["builds"] = {phase: {key: data[key] for key in
    ("executable_sha256", "source_manifest_sha256", "first_party_fingerprints_and_artifacts_removed",
     "incremental_compilation", "sources_verified_before_build_and_after_every_command")}
    for phase, data in source["builds"].items()}
summary["builds"]["baseline-recovery"]["image_executable_sha256"] = source["baseline_image_sha256"]
summary["builds"]["final-recovery"]["image_executable_sha256"] = source["builds"]["final-recovery"]["image_executable_sha256"]
review_file = out / "review" / "review.json"
if review_file.exists():
    review = json.loads(review_file.read_text())
    for case_id, case in review["cases"].items():
        for phase in ("baseline", "final"):
            test = next(t for t in reports[f"{phase}-tables-1"]["tests"] if t["id"] == case_id)
            for image in ("actual", "reference"):
                expected_hash = case[phase][image]["ppm_sha256"]
                if test[f"{image}_sha256"] != expected_hash:
                    raise SystemExit(f"Retained review image is stale: {phase}-{case_id}-{image}")
                png = out / "review" / case[phase][image]["png"]
                if hashlib.sha256(png.read_bytes()).hexdigest() != case[phase][image]["png_sha256"]:
                    raise SystemExit(f"Retained PNG hash differs: {png}")
    review["final_gate_revalidated"] = {
        "baseline_executable": "layout3-baseline-recovery.exe",
        "final_executable": "layout3-final-recovery.exe",
        "all_retained_ppm_hashes_match_credited_reports": True,
        "all_png_hashes_verified": True,
    }
    write("review/review.json", review)
for phase, data in source["builds"].items():
    write(f"{phase}-build.json", data)
write("summary.json", summary)
print("layout", summary["layout_tests"], "workspace", summary["workspace"])
