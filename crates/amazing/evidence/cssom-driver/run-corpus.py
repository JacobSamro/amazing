"""Measure the whole pinned CSSOM testharness area through the installed driver."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("label")
parser.add_argument("--timeout-ms", type=int, default=5000)
parser.add_argument("--jobs", type=int, default=8)
parser.add_argument("--baseline", type=Path)
parser.add_argument("--recheck-timeouts", action="store_true",
    help="Recheck formerly passing files/subtests lost to deadlines at 60 seconds")
args = parser.parse_args()
root = Path.home() / "corpora/wpt"
revision = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
assert revision == "c2a2fce69e5c2509d9dc28440ec8b42532e5c293", revision
area = root / "css/cssom"
selection = sorted(path for path in area.rglob("*") if path.is_file()
    and path.suffix in (".html", ".htm", ".xhtml", ".xht")
    and "testharness.js" in path.read_text(errors="replace"))
assert selection, "empty CSSOM corpus"
output = Path(os.environ["OUT"]) / ("cssom-" + args.label)
output.mkdir(parents=True, exist_ok=True)
binary = output / "amazing"
shutil.copy2(Path(os.environ["CARGO_TARGET_DIR"]) / "release/amazing", binary)

def run(path, timeout_ms=None):
    timeout_ms = args.timeout_ms if timeout_ms is None else timeout_ms
    test = path.relative_to(root).as_posix()
    with tempfile.TemporaryDirectory(prefix="cssom-storage-") as storage:
        try:
            result = subprocess.run([str(binary), "wpt-harness", str(root), test,
                "800", "600", str(timeout_ms)], capture_output=True, text=True,
                timeout=timeout_ms / 1000 + 15,
                env={**os.environ, "AMAZING_STORAGE_DIR": storage})
            try:
                record = json.loads(result.stdout)
            except json.JSONDecodeError:
                record = {"status": "crash", "counts": {}, "detail": result.stderr[-2000:]}
        except subprocess.TimeoutExpired:
            record = {"status": "timeout", "counts": {}, "detail": "process deadline"}
    record["test"] = test
    print(test, record.get("status"), record.get("counts"), flush=True)
    return record

with ThreadPoolExecutor(max_workers=args.jobs) as pool:
    records = list(pool.map(run, selection))
report = {"revision": revision, "label": args.label, "timeout_ms": args.timeout_ms,
    "jobs": args.jobs, "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "selection_sha256": hashlib.sha256("\n".join(p.relative_to(root).as_posix() for p in selection).encode()).hexdigest(),
    "results": records}
if args.baseline:
    before = json.loads(args.baseline.read_text())
    assert before["revision"] == revision
    assert before["selection_sha256"] == report["selection_sha256"]
    def passes(results):
        files, subtests = set(), set()
        for record in results:
            test = record["test"]
            if record["status"] == "pass":
                files.add(test)
            occurrences = {}
            for subtest in record.get("subtests", []):
                name = subtest["name"]
                number = occurrences.get(name, 0)
                occurrences[name] = number + 1
                if subtest["status"] == 0:
                    subtests.add((test, name, number))
        return files, subtests
    old_files, old_subtests = passes(before["results"])
    if args.recheck_timeouts:
        previously_passing = old_files | {test for test, _, _ in old_subtests}
        deadline_lost = [record for record in records if record["test"] in previously_passing
            and (record["status"] == "timeout" or record.get("counts", {}).get("timeout", 0))]
        if deadline_lost:
            originals = {record["test"]: record for record in deadline_lost}
            with ThreadPoolExecutor(max_workers=args.jobs) as pool:
                rechecks = list(pool.map(lambda record: run(root / record["test"], 60000), deadline_lost))
            corrected = {record["test"]: record for record in rechecks}
            for record in rechecks:
                record["timeout_recheck_ms"] = 60000
                record["original_deadline_result"] = originals[record["test"]]
            records[:] = [corrected.get(record["test"], record) for record in records]
    new_files, new_subtests = passes(records)
    report["delta"] = {"gained_files": sorted(new_files-old_files),
        "lost_files": sorted(old_files-new_files), "gained_subtests": sorted(new_subtests-old_subtests),
        "lost_subtests": sorted(old_subtests-new_subtests)}
(output / "results.json").write_text(json.dumps(report, separators=(",", ":")) + "\n")
print("REPORT", output / "results.json", flush=True)
if args.baseline:
    print("DELTA", {key: len(value) for key, value in report["delta"].items()}, flush=True)
    assert not report["delta"]["lost_files"] and not report["delta"]["lost_subtests"], "lost passing cases"
