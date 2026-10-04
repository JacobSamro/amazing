#!/usr/bin/env python3
"""Whole html5lib corpus through the decoded-input, script-mode probe."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("corpus", type=Path)
parser.add_argument("--binary", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--execute-binary", type=Path, help="Installed driver scripted-tree probe; static parser observations remain unchanged")
args = parser.parse_args()
repo = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("html5lib", repo / "scripts/html5lib.py")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
resources = args.corpus / "html/syntax/parsing/resources"
records = []
manifest = []
for path in sorted(resources.glob("*.dat")):
    cases = module.load_cases(path)
    manifest.append({"file": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "cases": len(cases)})
    blocks = path.read_text(encoding="utf-8-sig").split("#data\n")[1:]
    for case in cases:
        mode = "disabled" if "#script-off" in blocks[case.index - 1].split("#document\n", 1)[0] else "enabled"
        command = [str(args.binary), mode]
        if case.source_file.startswith("scripted_") and mode == "enabled":
            if args.execute_binary is None:
                records.append({"id": f"{case.source_file}:{case.index}", "status": "unsupported", "mode": mode, "script_variant": True, "reason": "requires JavaScript execution by an embedding host"})
                continue
            command = [str(args.execute_binary), "html5lib-tree", mode, "execute"]
        if case.context:
            command.append(case.context)
        process = subprocess.run(command, input=case.data, text=True, encoding="utf-8", capture_output=True, timeout=10)
        actual = module.canonical_actual(process.stdout)
        status = "pass" if process.returncode == 0 and actual == case.expected else "fail"
        record = {"id": f"{case.source_file}:{case.index}", "status": status, "mode": mode, "script_variant": case.skip_reason is not None}
        if status != "pass":
            record.update(data=case.data, expected=case.expected, actual=actual, stderr=process.stderr)
        records.append(record)
result = {"revision": "c2a2fce69e5c2509d9dc28440ec8b42532e5c293", "manifest": manifest, "records": records, "passed": sum(r["status"] == "pass" for r in records), "failed": sum(r["status"] == "fail" for r in records), "unsupported": sum(r["status"] == "unsupported" for r in records)}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, ensure_ascii=False) + "\n")
print(json.dumps({k: result[k] for k in ("passed", "failed", "unsupported")}))
for record in records:
    if record["status"] == "fail":
        print(json.dumps(record, ensure_ascii=False))
raise SystemExit(bool(result["failed"]))
