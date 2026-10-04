#!/usr/bin/env python3
"""Omit recoverable historical raw evidence copies from local worktrees.

Dry-run by default. --apply changes only sparse-checkout materialization;
Git objects, branches, source and modified evidence are preserved.
"""
import argparse
import json
from pathlib import Path
import subprocess


def git(tree, *args):
    return subprocess.check_output(["git", "-C", str(tree), *args])


def thin(tree, apply):
    changed = set(git(tree, "diff", "HEAD", "--name-only", "-z").split(b"\0"))
    candidates = []
    for entry in git(tree, "ls-tree", "-r", "-l", "-z", "HEAD").split(b"\0"):
        if not entry:
            continue
        metadata, raw_path = entry.split(b"\t", 1)
        fields = metadata.split()
        if fields[1] != b"blob" or fields[3] == b"-" or raw_path in changed:
            continue
        name = raw_path.decode("utf-8", "surrogateescape")
        parts = Path(name).parts
        size = int(fields[3])
        if "evidence" not in parts and parts[:2] != ("docs", "lanes"):
            continue
        if not (size > 512 * 1024 or name.endswith(".jsonl")):
            continue
        target = tree / name
        if not target.is_file() or target.is_symlink():
            continue
        candidates.append({"path": name, "bytes": size})
    if apply and candidates:
        sparse = Path(git(tree, "rev-parse", "--path-format=absolute", "--git-path", "info/sparse-checkout").decode().strip())
        existing = sparse.read_text() if sparse.exists() else "/*\n"
        # Preserve an existing sparse source selection instead of expanding it.
        def literal(name):
            return "".join("\\" + c if c in "\\*?[]#! " else c for c in name)
        patterns = existing.rstrip("\n") + "\n" + "".join("!/" + literal(row["path"]) + "\n" for row in candidates)
        subprocess.run(["git", "-C", str(tree), "sparse-checkout", "set", "--no-cone", "--stdin"], input=patterns.encode(), check=True)
    removed = [row for row in candidates if not (tree / row["path"]).exists()] if apply else []
    return {"tree": str(tree), "apply": apply, "eligible_files": len(candidates),
            "eligible_bytes": sum(row["bytes"] for row in candidates),
            "removed_files": len(removed), "removed_bytes": sum(row["bytes"] for row in removed),
            "modified_paths_preserved": len(changed - {b""}), "files": candidates}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trees", nargs="+", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    rows = []
    for tree in args.trees:
        try:
            rows.append(thin(tree.resolve(), args.apply))
        except (subprocess.CalledProcessError, OSError) as error:
            rows.append({"tree": str(tree), "error": str(error)})
    result = {"rows": rows, "removed_bytes": sum(row.get("removed_bytes", 0) for row in rows)}
    if args.output:
        args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"trees": len(rows), "removed_bytes": result["removed_bytes"],
                      "errors": [row for row in rows if "error" in row]}))
    return int(any("error" in row for row in rows))


if __name__ == "__main__":
    raise SystemExit(main())
