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


def main() -> int:
    args = sys.argv[1:]
    baseline = None
    if "--baseline" in args:
        i = args.index("--baseline")
        baseline = args[i + 1]
        del args[i : i + 2]
    out, shards = args[0], args[1:]
    reports = [json.load(open(path)) for path in shards]
    provenance = ("test262_revision", "input_sha256", "host_sha256", "runner_sha256", "timeout_seconds")
    identities = set()
    expected_paths = {r["path"] for r in reports[0]["records"]}
    for report in reports:
        for field in provenance:
            if not report.get(field) or report[field] != reports[0].get(field):
                print(f"invalid shard provenance: {field}", file=sys.stderr)
                return 2
        identity = report.get("shard", "")
        try:
            index, total = map(int, identity.split("/"))
        except (ValueError, AttributeError):
            print("invalid shard identity", file=sys.stderr)
            return 2
        paths = [r["path"] for r in report["records"]]
        if total != len(reports) or not 0 <= index < total or index in identities or set(paths) != expected_paths or len(paths) != len(expected_paths):
            print("incomplete or duplicate shard coverage", file=sys.stderr)
            return 2
        identities.add(index)
    merged: dict[str, dict] = {}
    for report in reports:
        for rec in report["records"]:
            ran = not (rec.get("status") == "skipped" and "selection.excluded" in json.dumps(rec))
            if ran or rec["path"] not in merged:
                merged[rec["path"]] = rec
    records = [merged[p] for p in sorted(merged)]
    summary = collections.Counter(r["status"] for r in records)
    json.dump({"schema": 1, "shards": shards, "summary": {"files": dict(summary)}, "records": records},
              open(out, "w"), separators=(",", ":"))
    print("merged", dict(summary))
    if baseline:
        before = {r["path"]: r["status"] for r in json.load(open(baseline))["records"]}
        after = {r["path"]: r["status"] for r in records}
        lost = sorted(p for p, s in before.items() if s == "pass" and after.get(p) != "pass")
        gained = sum(1 for p, s in after.items() if s == "pass" and before.get(p) != "pass")
        print(f"lost {len(lost)} gained {gained}")
        for p in lost[:40]:
            print("  LOST", p, after.get(p))
        return 1 if lost else 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
