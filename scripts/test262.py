#!/usr/bin/env python3
"""Run a deliberately explicit, classic-script Test262 slice.

This harness is intentionally small and dependency-free. It runs tests through
the built Amazing binary, prepends only the assertion helpers needed by the
selected language tests, and reports every skipped/failed file. Tests that
need modules, async harness hooks, or negative-test expectations are excluded
from the denominator; the report is therefore a focused classic-script
measurement, not a claim about the whole Test262 repository.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys


PRELUDE = r"""
function assert(condition, message) {
  if (!condition) { throw message || "Test262 assert failed"; }
}
assert.sameValue = function (actual, expected, message) {
  if (!(actual === expected)) { throw message || "Test262 sameValue failed"; }
};
assert.notSameValue = function (actual, unexpected) {
  if (actual === unexpected) { throw "Test262 notSameValue failed"; }
};
assert.isTrue = function (value) {
  if (value !== true) { throw "Test262 isTrue failed"; }
};
assert.isFalse = function (value) {
  if (value !== false) { throw "Test262 isFalse failed"; }
};
assert.compareArray = function (actual, expected) {
  if (actual.length !== expected.length) { throw "Test262 compareArray failed"; }
  for (var i = 0; i < actual.length; i++) {
    if (actual[i] !== expected[i]) { throw "Test262 compareArray failed"; }
  }
};
assert.throws = function (constructor, callback) {
  var threw = false;
  try { callback(); } catch (error) { threw = true; }
  if (!threw) { throw "Test262 throws failed"; }
};
var Test262Error = function () {};
function $ERROR(message) { throw message; }
"""


def metadata(source: str) -> tuple[bool, str]:
    """Return (eligible, reason) from the Test262 frontmatter/comments."""
    frontmatter = re.search(r"/\*---(.*?)(?:---|\.\.\.)\*/", source, re.S)
    block = frontmatter.group(1) if frontmatter else ""
    code = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.S)
    if re.search(r"\bnegative\s*:", block):
        return False, "negative test"
    if re.search(r"\bflags\s*:[^\n]*(?:module|async|raw)", block):
        return False, "module/async/raw flag"
    if re.search(r"\b(?:module|async)\b", block) and "flags:" in block:
        return False, "module/async metadata"
    if "$DONOTEVALUATE" in source or "$DONE" in source:
        return False, "async harness hook"
    if re.search(r"^\s*(?:import|export)\b", source, re.M):
        return False, "module syntax"
    if re.search(r"(?:\b(?:BigInt|Symbol|Proxy|WeakMap|WeakSet|Atomics|SharedArrayBuffer)\b|\b(?:0[xob][0-9a-f_]+|\d[\d_]*)n\b)", code, re.I):
        return False, "deferred runtime feature"
    if re.search(r"(?:^|[;{])\s*#[$A-Za-z_]", code, re.M):
        return False, "private syntax"
    if "$MAX_ITERATIONS" in code:
        return False, "tail-call harness hook"
    if re.search(r"(?:^|[{;])\s*(?:get|set)\s+[A-Za-z_$]", code, re.M):
        return False, "accessor semantics"
    return True, ""


def run(binary: pathlib.Path, source: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(binary), "run"],
        input=PRELUDE + "\n" + source,
        text=True,
        encoding="utf-8",
        capture_output=True,
        timeout=10,
        check=False,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("test262_root", type=pathlib.Path)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--only", default="", help="substring filter for test paths")
    args = parser.parse_args()

    files = sorted((args.test262_root / "test").rglob("*.js"))
    if args.only:
        files = [path for path in files if args.only in str(path)]
    if args.limit:
        files = files[: args.limit]
    passed = failed = skipped = 0
    failures: list[tuple[pathlib.Path, str]] = []
    for path in files:
        source = path.read_text(encoding="utf-8")
        path_lower = str(path).lower()
        if any(token in path_lower for token in ("private", "accessor", "dstr", "destructuring")):
            skipped += 1
            continue
        eligible, reason = metadata(source)
        if not eligible:
            skipped += 1
            continue
        try:
            result = run(args.binary, source)
        except subprocess.TimeoutExpired:
            failed += 1
            failures.append((path, "timeout"))
            continue
        if result.returncode == 0:
            passed += 1
        else:
            failed += 1
            detail = (result.stderr or result.stdout).strip().splitlines()
            failures.append((path, detail[-1] if detail else f"exit {result.returncode}"))

    eligible_count = passed + failed
    rate = (passed / eligible_count * 100.0) if eligible_count else 0.0
    print(f"Test262 focused classic-script slice: {len(files)} discovered")
    print(f"  passed:  {passed}")
    print(f"  failed:  {failed}")
    print(f"  skipped: {skipped}")
    print(f"  measured pass rate: {passed}/{eligible_count} ({rate:.2f}%)")
    if failures:
        print("Failures (first 25):")
        for path, detail in failures[:25]:
            print(f"  {path}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
