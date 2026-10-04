#!/usr/bin/env python3
"""Check the committed ticket packaging before allocating a merge gate.

Only newly added/modified evidence counts; historical evidence is not rewritten.
Each commit is one ticket, so limits apply separately when gating a few tickets.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess

FILE_LIMIT = 512 * 1024
TICKET_LIMIT = 2 * 1024 * 1024
REPO = Path(__file__).resolve().parent.parent


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(REPO), *args])


def evidence(path: str) -> bool:
    parts = Path(path).parts
    return "evidence" in parts or parts[:2] == ("docs", "lanes")


def check(base: str, ref: str, ready: bool = False) -> dict:
    base_sha = git("rev-parse", "--verify", base + "^{commit}").decode().strip()
    head = git("rev-parse", "--verify", ref + "^{commit}").decode().strip()
    errors = []
    if subprocess.run(["git", "-C", str(REPO), "merge-base", "--is-ancestor", base_sha, head]).returncode:
        errors.append("Recorded base must be an ancestor of candidate")
    commits = git("rev-list", "--reverse", base_sha + ".." + head).decode().splitlines()
    if ready and len(commits) != 1:
        errors.append("A ready branch must contain exactly one squashed ticket commit")
    tickets = []
    for sha in commits:
        title = git("show", "-s", "--format=%s", sha).decode().strip()
        parents = git("show", "-s", "--format=%P", sha).decode().split()
        if re.search(r"\bWIP\b", title, re.IGNORECASE):
            errors.append(f"{sha[:12]}: WIP commit title is forbidden")
        if len(parents) > 1:
            errors.append(f"{sha[:12]}: merge commits are forbidden")
        names = git("diff-tree", "--root", "--no-commit-id", "-r", "--name-only", "--diff-filter=AMT", "-z", sha).split(b"\0")
        files = []
        for raw in names:
            if not raw:
                continue
            path = raw.decode("utf-8", "surrogateescape")
            if not evidence(path):
                continue
            size = int(git("cat-file", "-s", sha + ":" + path))
            files.append({"path": path, "bytes": size})
            if size > FILE_LIMIT:
                errors.append(f"{sha[:12]}: {path} is {size} bytes, over {FILE_LIMIT}")
        total = sum(row["bytes"] for row in files)
        if total > TICKET_LIMIT:
            errors.append(f"{sha[:12]}: evidence totals {total} bytes, over {TICKET_LIMIT}")
        tickets.append({"commit": sha, "title": title, "evidence_bytes": total, "evidence_files": files})
    return {"qualified": not errors, "base": base_sha, "candidate": head,
            "limits": {"file_bytes": FILE_LIMIT, "ticket_bytes": TICKET_LIMIT},
            "tickets": tickets, "errors": errors}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/dev")
    parser.add_argument("--ref", default="HEAD")
    parser.add_argument("--ready", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = check(args.base, args.ref, args.ready)
    raw = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(raw)
    print(raw, end="")
    return 0 if result["qualified"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
