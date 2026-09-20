"""Summarize saved Windows measurements; does not run the engine or tests."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).with_name("rc2")
FIELDS = (
    "id", "classification", "differing_pixels", "max_difference",
    "actual_sha256", "reference_sha256", "diff_sha256",
)


def read(name):
    return json.loads((ROOT / name).read_text(encoding="utf-8-sig"))


def projection(report):
    return [{key: case.get(key) for key in FIELDS} for case in report["tests"]]


pairs = []
counts = {}
for kind in ("images", "svg", "scope", "focused"):
    counts[kind] = {}
    for phase in ("before", "after"):
        names = [f"{kind}-{phase}-{run}.json" for run in (1, 2)]
        reports = [read(name) for name in names]
        values = [projection(report) for report in reports]
        encoded = json.dumps(values[0], sort_keys=True, separators=(",", ":")).encode()
        stable = values[0] == values[1] and reports[0]["counts"] == reports[1]["counts"]
        pairs.append({"runs": names, "stable": stable,
                      "projection_sha256": hashlib.sha256(encoded).hexdigest()})
        counts[kind][phase] = reports[0]["counts"]

before_svg = read("svg-before-1.json")["tests"]
after_svg = read("svg-after-1.json")["tests"]
svg_unchanged = all(
    a["id"] == b["id"] and a.get("actual_sha256") == b.get("actual_sha256")
    and a.get("reference_sha256") == b.get("reference_sha256")
    for a, b in zip(before_svg, after_svg, strict=True)
)
before_png = read("images-before-1.json")["tests"]
after_png = read("images-after-1.json")["tests"]
png_changes = [
    {"id": a["id"], "before": a["classification"], "after": b["classification"],
     "actual_changed": a.get("actual_sha256") != b.get("actual_sha256"),
     "reference_changed": a.get("reference_sha256") != b.get("reference_sha256")}
    for a, b in zip(before_png, after_png, strict=True)
]
result = {
    "projection": "Ordered cases; listed fields; sorted-key compact UTF-8 JSON",
    "fields": FIELDS, "pairs": pairs, "counts": counts,
    "svg_actual_and_reference_hashes_unchanged": svg_unchanged,
    "svg_conformance_credit": 0,
    "svg_reason": "Selected SVG resources require inline CSS; images are rejected on both sides.",
    "png_changes": png_changes,
}
(ROOT / "repeatability.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(counts, indent=2))
print("All paired measurements stable:", all(pair["stable"] for pair in pairs))
