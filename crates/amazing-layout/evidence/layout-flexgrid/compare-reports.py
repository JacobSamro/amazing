#!/usr/bin/env python3
"""Compare two render-sweep reports for the LFG tickets.

Usage: compare-reports.py BEFORE.json AFTER.json

BEFORE is a sweep report (``tests:[{id, classification, differing_pixels}]``)
or a raw baseline; AFTER is a ``report.json`` from run-render-sweep-linux.py.
Prints counts and the gains/losses id sets. Scratch tooling, not evidence.
"""
import json
import sys


def load(path):
    with open(path, encoding="utf-8-sig") as handle:
        return json.load(handle)


def classes(doc):
    return {t["id"]: t.get("classification", "pass") for t in doc["tests"]}


def counts(pairs):
    out = {}
    for value in pairs.values():
        out[value] = out.get(value, 0) + 1
    return out


def main():
    before = classes(load(sys.argv[1]))
    after = classes(load(sys.argv[2]))
    print("before:", counts(before))
    print("after :", counts(after))
    gains, losses = [], []
    for case_id, was in before.items():
        now = after.get(case_id)
        if now is None:
            continue
        if was != "pass" and now == "pass":
            gains.append(case_id)
        elif was == "pass" and now != "pass":
            losses.append(case_id)
    print(f"gains={len(gains)} losses={len(losses)}")
    if losses:
        print("LOSSES:")
        for case_id in sorted(losses):
            print("  ", case_id)
    if gains:
        print("GAINS:")
        for case_id in sorted(gains):
            print("  ", case_id)


if __name__ == "__main__":
    main()
