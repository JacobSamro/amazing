#!/usr/bin/env python3
"""Merge Test262 shard outputs into one result and compare it with a baseline.

Each shard (test262.py --shard K/N) records every file, marking the ones other
shards own as skipped/selection.excluded. The merged record for a path is the
one shard that actually ran it.

Usage: test262-merge.py OUT.json SHARD.json... [--baseline BASE.json]
Prints the file summary and, with --baseline, the passes lost and gained.
Exit status 1 when any baseline pass was lost.
"""

from __future__ import annotations

import collections
import json
import sys
import pathlib
import hashlib
import importlib.util
_spec = importlib.util.spec_from_file_location("gate_summary", pathlib.Path(__file__).with_name("gate-summary.py"))
_helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_helpers)


def main() -> int:
    args = sys.argv[1:]
    baseline = None
    if "--baseline" in args:
        i = args.index("--baseline")
        baseline = args[i + 1]
        del args[i : i + 2]
    out, shards = args[0], args[1:]
    reports = [json.load(open(path)) for path in shards]
    for report in reports:
        report["records"] = _helpers.unpack(report)
    provenance = ("test262_revision", "input_sha256", "host_sha256", "runner_sha256", "timeout_seconds", "source_commit")
    identities = set()
    expected_paths = {r["path"] for r in reports[0]["records"]}
    for report in reports:
        for field in provenance:
            if field == "source_commit" and all("source_commit" not in r for r in reports):
                continue
            if not report.get(field) or report[field] != reports[0].get(field):
                print(f"invalid shard provenance: {field}", file=sys.stderr)
                return 2
        identity = report.get("shard", "")
        try:
            index, total = map(int, identity.split("/"))
        except (ValueError, AttributeError):
            print("invalid shard identity", file=sys.stderr)
            return 2
        if report.get("schema") == 2 and (report.get("job_exit") != 0 or report.get("runner_exit") not in (0, 1)):
            print("shard job did not complete", file=sys.stderr)
            return 2
        paths = [r["path"] for r in report["records"]]
        if total != len(reports) or not 0 <= index < total or index in identities or len(paths) != len(set(paths)) or (report.get("schema") != 2 and set(paths) != expected_paths):
            print("incomplete or duplicate shard coverage", file=sys.stderr)
            return 2
        identities.add(index)
    merged: dict[str, dict] = {}
    owners = collections.Counter()
    for report in reports:
        for rec in report["records"]:
            ran = not (rec.get("status") == "skipped" and "selection.excluded" in json.dumps(rec))
            if ran and not (report.get("schema") != 2 and "_FIXTURE" in rec["path"].split("/")[-1]):
                owners[rec["path"]] += 1
            if ran or rec["path"] not in merged:
                merged[rec["path"]] = rec
    expected_paths = set(merged)
    compact_reports = [r for r in reports if r.get("schema") == 2]
    digest = hashlib.sha256(json.dumps(sorted(expected_paths), separators=(",", ":")).encode()).hexdigest()
    if any(r["discovered"] != len(expected_paths) or r["paths_sha256"] != digest for r in compact_reports):
        print("incomplete compact shard coverage", file=sys.stderr)
        return 2
    if any(owners[p] != 1 for p in expected_paths
           if compact_reports or "_FIXTURE" not in p.split("/")[-1]):
        print("missing or duplicate executed shard ownership", file=sys.stderr)
        return 2
    records = [merged[p] for p in sorted(merged)]
    summary = collections.Counter(r["status"] for r in records)
    variant_summary = collections.Counter(v["status"] for r in records for v in r["variants"])
    _helpers.write(out, {"schema": 2, "provenance": {k: reports[0].get(k) for k in provenance}, "shards": [str(pathlib.Path(p).resolve()) for p in shards],
                         "shard_sha256": {str(pathlib.Path(p).resolve()): hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in shards},
                         "summary": {"files": dict(summary), "variants": dict(variant_summary)}})
    print("merged", dict(summary))
    if baseline:
        base = json.load(open(baseline))
        if "records" in base or "compact_records" in base:
            before_records = _helpers.unpack(base)
        else:
            before_records = []
            for shard in base["shards"]:
                before_records.extend(r for r in _helpers.unpack(json.load(open(shard)))
                                      if r.get("reason") != "selection.excluded")
        if base.get("provenance") and any(base["provenance"][k] != reports[0][k] for k in ("test262_revision", "input_sha256", "timeout_seconds")):
            print("baseline corpus or timeout differs", file=sys.stderr)
            return 2
        before = {r["path"]: r["status"] for r in before_records}
        after = {r["path"]: r["status"] for r in records}
        lost = sorted(p for p, s in before.items() if s == "pass" and after.get(p) != "pass")
        gained = sum(1 for p, s in after.items() if s == "pass" and before.get(p) != "pass")
        before_variants = {(r["path"], v["variant"]): v["status"]
                           for r in before_records for v in r["variants"]}
        after_variants = {(r["path"], v["variant"]): v["status"]
                          for r in records for v in r["variants"]}
        lost_variants = sorted(key for key, status in before_variants.items()
                               if status == "pass" and after_variants.get(key) != "pass")
        evidence = json.load(open(out))
        evidence.update(lost_files=lost, lost_variants=lost_variants, gained_files=gained)
        _helpers.write(out, evidence)
        print(f"lost {len(lost)} files/{len(lost_variants)} variants gained {gained}")
        for p in lost[:40]:
            print("  LOST", p, after.get(p))
        return 1 if lost or lost_variants else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
