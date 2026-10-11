#!/usr/bin/env python3
"""Summarise an AMAZING_JS_VM_STATS file (see crates/amazing-js/src/vm/stats.rs).

  scripts/js-vm-coverage-summary.py vmstats.tsv [--out compact.json]

Line format: <pid> TAB P TAB <vm|fallback|tree> TAB <reason>   (program)
             <pid> TAB F TAB <ok|fail> TAB <reason>            (callable compile)
             <pid> TAB C TAB <vm_calls> TAB <tree_calls>       (calls executed)
A "test body" is the last program event of each process (Test262 runs the
harness files first, then the test, one process per variant).
"""
import collections, json, sys

def main():
    path = sys.argv[1]
    out = sys.argv[sys.argv.index("--out") + 1] if "--out" in sys.argv else None
    prog = collections.Counter(); prog_reasons = collections.Counter()
    fn = collections.Counter(); fn_reasons = collections.Counter()
    calls = [0, 0]
    last = {}
    for line in open(path, errors="replace"):
        f = line.rstrip("\n").split("\t")
        if len(f) < 3: continue
        kind = f[1]
        if kind == "P":
            reason = f[3] if len(f) > 3 else ""
            prog[f[2]] += 1
            if f[2] != "vm": prog_reasons[f"{f[2]}:{reason}"] += 1
            last[f[0]] = (f[2], reason)
        elif kind == "F":
            fn[f[2]] += 1
            if f[2] == "fail": fn_reasons[f[3] if len(f) > 3 else ""] += 1
        elif kind == "C" and len(f) >= 4:
            calls[0] += int(f[2]); calls[1] += int(f[3])
    body = collections.Counter(v[0] for v in last.values())
    body_reasons = collections.Counter(f"{v[0]}:{v[1]}" for v in last.values() if v[0] != "vm")
    def share(c):
        t = sum(c.values()) or 1
        return {k: {"count": n, "percent": round(100 * n / t, 3)} for k, n in c.most_common()}
    result = {
        "programs_all": share(prog), "programs_test_body": share(body),
        "program_fallback_reasons_all": dict(prog_reasons.most_common(20)),
        "program_fallback_reasons_test_body": dict(body_reasons.most_common(20)),
        "function_compiles": share(fn), "function_compile_fail_reasons_top20": dict(fn_reasons.most_common(20)),
        "function_compile_fail_reasons_total_kinds": len(fn_reasons),
        "calls_executed": {"vm": calls[0], "tree": calls[1],
                           "vm_percent": round(100 * calls[0] / max(1, sum(calls)), 3)},
        "processes": len(last),
    }
    text = json.dumps(result, indent=1)
    if out: open(out, "w").write(text + "\n")
    print(text)

main()
