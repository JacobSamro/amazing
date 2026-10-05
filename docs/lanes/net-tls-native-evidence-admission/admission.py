"""Admit a fresh gate output with its already-recorded capacity preflight.

This helper does not run Cargo, move raw output, infer native job status, change
Boat lifecycle or relax an engine assertion. Callers still own terminal proof.
"""
import json
import pathlib
import re


def admit_output(root, source, phase):
    root = pathlib.Path(root)
    if not re.fullmatch(r"[0-9a-f]{40}", source) or phase not in ("before", "after"):
        raise ValueError("invalid source/phase")
    if not root.is_absolute() or root.resolve() != root:
        raise ValueError("output must be absolute without symlink components")
    if root.is_symlink() or (root.exists() and not root.is_dir()):
        raise ValueError("output must be an ordinary directory")
    if not root.exists():
        root.mkdir(parents=True)
        return root
    if {entry.name for entry in root.iterdir()} != {"space.summary.json"}:
        raise ValueError("output contains previous or unexpected gate artifacts")
    receipt = root / "space.summary.json"
    if receipt.is_symlink() or not receipt.is_file():
        raise ValueError("capacity receipt is not an ordinary file")
    data = json.loads(receipt.read_text())
    if data.get("source_commit") != source or data.get("phase") != phase:
        raise ValueError("capacity source/phase mismatch")
    available, required = data.get("native_free_bytes"), data.get("minimum_free_bytes")
    if type(available) is not int or type(required) is not int or required <= 0 or available < required:
        raise ValueError("capacity was not admitted")
    # Retain the receipt; gate logs/compact files may now be written alongside
    # it. A later admission sees those files and rejects stale output reuse.
    return root
