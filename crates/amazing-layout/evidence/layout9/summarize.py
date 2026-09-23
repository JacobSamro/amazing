"""Compare both retained WPT render repeats on one fixed source base."""
import json
from pathlib import Path

here = Path(__file__).parent
reports = here / "reports"
suites = (
    "flex-grid-broad", "flex-grid", "geometry", "tables", "floats",
    "block-collapse", "position", "text-flow", "contents",
    "paint-compositing", "images",
)


def read(phase, suite, repeat):
    return json.loads((reports / f"{phase}-{suite}-{repeat}.json").read_text(encoding="utf-8-sig"))


def projection(report):
    return {
        item["id"]: (
            item["classification"], item.get("actual_sha256"),
            item.get("reference_sha256"), item.get("diff_sha256"),
        )
        for item in report["tests"]
    }


summary = {}
for suite in suites:
    before1, before2 = (read("baseline", suite, repeat) for repeat in (1, 2))
    after1, after2 = (read("final", suite, repeat) for repeat in (1, 2))
    assert before1["wpt_revision"] == after1["wpt_revision"] == "c2a2fce69e5c2509d9dc28440ec8b42532e5c293"
    assert projection(before1) == projection(before2), f"unstable baseline {suite}"
    assert projection(after1) == projection(after2), f"unstable final {suite}"
    before, after = projection(before1), projection(after1)
    assert before.keys() == after.keys(), suite
    gains = [case for case in before if before[case][0] != "pass" and after[case][0] == "pass"]
    losses = [case for case in before if before[case][0] == "pass" and after[case][0] != "pass"]
    changed = [case for case in before if before[case] != after[case]]
    summary[suite] = {
        "before_counts": before1["counts"],
        "after_counts": after1["counts"],
        "gains": gains,
        "losses": losses,
        "changed_images_or_classification": changed,
        "baseline_run_directory": before1["run_directory"],
        "final_run_directory": after1["run_directory"],
    }

(here / "retained-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({suite: {
    "before": value["before_counts"]["pass"],
    "after": value["after_counts"]["pass"],
    "gains": value["gains"], "losses": value["losses"],
} for suite, value in summary.items()}, indent=2))
