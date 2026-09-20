"""Compare saved RC-4 merge-gate reports; does not execute builds/tests."""
import hashlib
import json
import sys
from pathlib import Path

TAG = sys.argv[1] if len(sys.argv) > 1 else "11dd8a8"
ROOT = Path(__file__).with_name("rc4") / f"rebase-{TAG}"
FIELDS = ("id", "classification", "differing_pixels", "max_difference",
          "actual_sha256", "reference_sha256", "diff_sha256")


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def projection(report):
    return [{key: case.get(key) for key in FIELDS} for case in report["tests"]]


pairs = []
for kind in ("scope", "controls", "images", "svg", "focused"):
    a, b = (read(ROOT / f"{kind}-after-{run}.json") for run in (1, 2))
    previous = read(ROOT.parent / f"{kind}-after-1.json")
    before = read(ROOT.parent / f"{kind}-before-1.json")
    ordered = projection(a)
    pairs.append({
        "kind": kind,
        "stable": ordered == projection(b) and a["counts"] == b["counts"],
        "counts": a["counts"],
        "unchanged_from_original_after": ordered == projection(previous),
        "unchanged_from_original_before": ordered == projection(before),
        "projection_sha256": hashlib.sha256(json.dumps(
            ordered, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
    })
a, b = (read(ROOT / f"fixtures-after-{run}.json") for run in (1, 2))
previous = read(ROOT.parent / "fixtures-after-1.json")
result = {"fields": FIELDS, "pairs": pairs,
          "owned_diagnostics": {"stable": a == b, "passed": sum(x["match"] for x in a),
                                "total": len(a), "unchanged_from_original_after": a == previous},
          "new_wpt_conformance_passes_claimed": 0}
(ROOT / "repeatability.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
