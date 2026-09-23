"""Summarize saved Windows measurements; does not run the engine or tests."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).with_name("rc4")
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
for kind in ("scope", "controls", "images", "svg", "focused"):
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

changes = {}
for kind in ("scope", "controls", "images", "svg", "focused"):
    before = read(f"{kind}-before-1.json")["tests"]
    after = read(f"{kind}-after-1.json")["tests"]
    changes[kind] = [
        {"id": a["id"], "before": a["classification"], "after": b["classification"],
         "actual_changed": a.get("actual_sha256") != b.get("actual_sha256"),
         "reference_changed": a.get("reference_sha256") != b.get("reference_sha256")}
        for a, b in zip(before, after, strict=True)
    ]
result = {"fields": FIELDS, "pairs": pairs, "counts": counts, "changes": changes,
          "new_wpt_conformance_passes_claimed": 0}
fixtures = {}
for phase in ("before", "after"):
    a, b = (read(f"fixtures-{phase}-{n}.json") for n in (1, 2))
    fixtures[phase] = {"stable": a == b, "passed": sum(row["match"] for row in a), "total": len(a)}
result["owned_diagnostic_fixtures"] = fixtures
result["fixture_changes"] = [
    {"id": a["id"], "before_match": a["match"], "after_match": b["match"],
     "actual_changed": a["actual_sha256"] != b["actual_sha256"],
     "reference_changed": a["reference_sha256"] != b["reference_sha256"]}
    for a, b in zip(read("fixtures-before-1.json"), read("fixtures-after-1.json"), strict=True)
]
result["scope_cluster_counts"] = {}
for phase in ("before", "after"):
    clusters = {}
    for case in read(f"scope-{phase}-1.json")["tests"]:
        cluster = case["id"].split("__")[0]
        cluster_counts = clusters.setdefault(cluster, {})
        classification = case["classification"]
        cluster_counts[classification] = cluster_counts.get(classification, 0) + 1
    result["scope_cluster_counts"][phase] = clusters
(ROOT / "repeatability.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(counts, indent=2))
print("All paired measurements stable:", all(pair["stable"] for pair in pairs))
