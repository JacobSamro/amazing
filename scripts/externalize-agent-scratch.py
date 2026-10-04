#!/usr/bin/env python3
"""Move large inactive agent scratch to external storage, preserving every path.

Dry-run by default. Never handles worktrees, tracked files or recent outputs.
Copies are hash verified before replacing local files with symbolic links.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def fingerprint(stat):
    return stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--min-age-hours", type=float, default=2)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve()
    scratch = repo / ".agents"
    destination = args.destination.resolve()
    if not destination.parent.is_dir() or destination.is_relative_to(repo):
        parser.error("destination parent must exist outside the repository")
    if args.min_age_hours < 2:
        parser.error("inactive period must be at least two hours")
    tracked = set(subprocess.check_output(
        ["git", "-C", str(repo), "ls-files", "-z"]).split(b"\0"))
    cutoff = time.time() - args.min_age_hours * 3600
    candidates = []
    for path in scratch.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        stat = path.stat()
        relative = path.relative_to(repo)
        if stat.st_size <= 512 * 1024 or stat.st_mtime >= cutoff:
            continue
        if os.fsencode(relative) in tracked:
            continue
        candidates.append((path, stat))
    needed = sum(stat.st_size for _, stat in candidates)
    if args.apply and shutil.disk_usage(destination.parent).free < needed + 2 * 1024**3:
        parser.error("external destination lacks space plus a 2 GiB reserve")
    result = {"updated": time.time(), "apply": args.apply,
              "destination": str(destination), "eligible_bytes": needed,
              "moved": [], "skipped_changed": [], "errors": []}
    inodes = {}
    for path, stat in candidates:
        relative = path.relative_to(scratch)
        target = destination / relative
        if not args.apply:
            continue
        temporary = target.with_name(target.name + ".copying")
        link = path.with_name(path.name + ".externalizing")
        try:
            if fingerprint(path.stat()) != fingerprint(stat):
                result["skipped_changed"].append(str(relative))
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.exists() or temporary.exists() or link.exists():
                raise FileExistsError("destination or temporary path already exists")
            source_hash = digest(path)
            shutil.copy2(path, temporary)
            if digest(temporary) != source_hash:
                raise OSError("copy hash mismatch")
            if fingerprint(path.stat()) != fingerprint(stat):
                temporary.unlink()
                result["skipped_changed"].append(str(relative))
                continue
            os.replace(temporary, target)
            os.symlink(target, link)
            os.replace(link, path)
            key = (stat.st_dev, stat.st_ino)
            inodes[key] = (stat.st_nlink, stat.st_blocks * 512,
                           inodes.get(key, (0, 0, 0))[2] + 1)
            result["moved"].append({"path": str(relative), "bytes": stat.st_size,
                                    "sha256": source_hash})
        except OSError as error:
            result["errors"].append({"path": str(relative), "error": str(error)})
            if temporary.exists():
                temporary.unlink()
            if link.is_symlink():
                link.unlink()
    result["local_allocated_bytes_reclaimed"] = sum(
        allocated for links, allocated, moved in inodes.values() if moved == links)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"eligible_files": len(candidates), "moved_files": len(result["moved"]),
                      "reclaimed_bytes": result["local_allocated_bytes_reclaimed"],
                      "errors": len(result["errors"])}))
    return int(bool(result["errors"]))


if __name__ == "__main__":
    raise SystemExit(main())
