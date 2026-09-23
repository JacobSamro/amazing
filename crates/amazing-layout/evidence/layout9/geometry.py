"""Frozen LAYOUT-9 WPT-derived SVG geometry gate for `amazing layout`."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--binary", required=True)
parser.add_argument("--selection", default=str(pathlib.Path(__file__).with_name("geometry-selection.json")))
parser.add_argument("--output", required=True)
args = parser.parse_args()
selection_path = pathlib.Path(args.selection)
selection_bytes = selection_path.read_bytes()
selection = json.loads(selection_bytes)
binary = pathlib.Path(args.binary)
results = []
with tempfile.TemporaryDirectory(prefix="layout9-svg-") as temp:
    for case in selection["cases"]:
        fixture = pathlib.Path(temp) / (case["id"] + ".html")
        fixture.write_text(
            f"<!doctype html><style>body{{margin:0;width:{case['container_width']}px}}"
            f"svg{{vertical-align:top}}</style><body>{case['svg']}</body>",
            encoding="utf-8",
        )
        run = subprocess.run([str(binary), "layout", str(fixture), "800"],
                             capture_output=True, text=True, timeout=30)
        fragments = re.findall(r"<img node=\d+>@\([^)]*\) ([\d.]+)x([\d.]+)", run.stdout)
        actual = [float(n) for n in fragments[0]] if len(fragments) == 1 else None
        expected = case["expected"]
        passed = run.returncode == 0 and actual is not None and all(
            abs(a - e) <= 0.05 for a, e in zip(actual, expected)
        )
        results.append({"id": case["id"], "source": case["source"],
                        "expected": expected, "actual": actual, "pass": passed,
                        "exit_code": run.returncode, "fragments": len(fragments)})
report = {"wpt_revision": selection["wpt_revision"],
          "selection_sha256": hashlib.sha256(selection_bytes).hexdigest(),
          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
          "passed": sum(item["pass"] for item in results),
          "total": len(results), "results": results}
pathlib.Path(args.output).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(f"{report['passed']}/{report['total']} {report['selection_sha256']} {report['binary_sha256']}")
