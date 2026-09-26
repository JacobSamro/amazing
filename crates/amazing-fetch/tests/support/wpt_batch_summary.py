#!/usr/bin/env python3
"""NET-41: summarize a net3*/net41_*_batch.sh log.

Each batch script prints "TEST <path>", then the installed wpt-harness's own
single-line JSON report for that file, then "EXIT <code>". This scans stdin
(or a file argument) for those triples and prints per-file status plus
aggregated file-level and subtest-level counts, so a before/after comparison
is a diff of two summaries instead of two raw logs.

Usage: wpt_batch_summary.py <log-file> [--diff <other-summary.json>]
"""
import json
import sys


def parse(path):
    results = {}
    current = None
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if line.startswith("TEST "):
                current = line[len("TEST "):]
            elif line.startswith("{") and current is not None:
                try:
                    report = json.loads(line)
                except json.JSONDecodeError:
                    continue
                results[current] = report
                current = None
    return results


def summarize(results):
    file_counts = {"pass": 0, "fail": 0, "timeout": 0, "error": 0, "unsupported": 0}
    subtest_counts = {"pass": 0, "fail": 0, "timeout": 0, "notrun": 0, "error": 0}
    for report in results.values():
        status = report.get("status", "error")
        file_counts[status] = file_counts.get(status, 0) + 1
        for key, value in report.get("counts", {}).items():
            subtest_counts[key] = subtest_counts.get(key, 0) + value
    return file_counts, subtest_counts


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    results = parse(sys.argv[1])
    file_counts, subtest_counts = summarize(results)
    total = len(results)
    print(f"files: {total} total, {json.dumps(file_counts)}")
    print(f"subtests: {json.dumps(subtest_counts)}")
    for name in sorted(results):
        status = results[name].get("status")
        if status != "pass":
            detail = results[name].get("detail")
            print(f"  {status:12} {name}  {detail}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
