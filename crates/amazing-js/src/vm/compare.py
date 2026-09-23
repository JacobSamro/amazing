"""Compare full Test262 reports for pass retention and VM coverage."""

import json
import pathlib
import sys


def load(path):
    return json.loads(pathlib.Path(path).read_text(encoding="utf-8"))


def variants(report):
    return {(record["path"], variant["variant"]): variant
            for record in report["records"] for variant in record["variants"]}


def main(baseline_path, after_path):
    baseline = load(baseline_path)
    after = load(after_path)
    for field in ("test262_revision", "input_sha256", "runner_sha256", "timeout_seconds",
                  "capabilities", "filter", "limit"):
        if baseline[field] != after[field]:
            raise SystemExit(f"mismatched {field}")
    before_files = {record["path"]: record for record in baseline["records"]}
    after_files = {record["path"]: record for record in after["records"]}
    before_variants = variants(baseline)
    after_variants = variants(after)
    lost_files = sorted(path for path, record in before_files.items()
                        if record["status"] == "pass" and after_files[path]["status"] != "pass")
    gained_files = sorted(path for path, record in before_files.items()
                          if record["status"] != "pass" and after_files[path]["status"] == "pass")
    lost_variants = sorted([path, variant] for (path, variant), record in before_variants.items()
                           if record["status"] == "pass" and after_variants[(path, variant)]["status"] != "pass")
    gained_variants = sorted([path, variant] for (path, variant), record in before_variants.items()
                             if record["status"] != "pass" and after_variants[(path, variant)]["status"] == "pass")
    result = {
        "base_files": baseline["summary"]["files"],
        "after_files": after["summary"]["files"],
        "base_variants": baseline["summary"]["variants"],
        "after_variants": after["summary"]["variants"],
        "lost_files": lost_files,
        "gained_files": gained_files,
        "lost_variants": lost_variants,
        "gained_variants": gained_variants,
        "base_classification_sha256": baseline["classification_sha256"],
        "after_classification_sha256": after["classification_sha256"],
    }
    print(json.dumps(result, indent=2))
    return int(bool(lost_files or lost_variants))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
