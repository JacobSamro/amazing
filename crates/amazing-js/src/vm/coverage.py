"""Run the pinned Test262 runner and record which test variants used bytecode.

Usage matches scripts/test262.py, with AMAZING_JS_VM=1 and
AMAZING_JS_VM_TRACE=1 in the environment. The runner and protocol remain
unchanged; this wrapper only retains the VM trace written to child stderr.
"""

import importlib.util
import json
import pathlib
import subprocess
import sys


runner_path = pathlib.Path(__file__).resolve().parents[4] / "scripts" / "test262.py"
spec = importlib.util.spec_from_file_location("amazing_test262_runner", runner_path)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
original_classify = runner.classify


def execute(binary, payload, timeout):
    try:
        child = subprocess.run([str(binary)], input=payload, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return runner.result("timeout", "execution.deadline")
    except OSError as error:
        return runner.result("fail", "runner.launch", detail=str(error))
    if child.returncode:
        return runner.result(
            "fail", "host.abnormal-exit", exit_code=child.returncode,
            detail=child.stderr.decode("utf-8", errors="replace")[-4096:],
        )
    lines = child.stdout.decode("utf-8", errors="replace").rstrip("\r\n").split("\n")
    try:
        final = lines[-1]
        if not final.startswith("TEST262_RESULT "):
            raise ValueError("missing result frame")
        outcome = json.loads(final[len("TEST262_RESULT "):])
        if outcome.get("protocol") != 1 or outcome.get("phase") not in {
            "parse", "runtime", "harness-parse", "harness-runtime", "complete"
        } or not isinstance(outcome.get("error_type"), str):
            raise ValueError("invalid host result")
        markers = [line.removeprefix("AMAZING_VM ") for line in
                   child.stderr.decode("utf-8", errors="replace").splitlines()
                   if line.startswith("AMAZING_VM ")]
        fields = 0
        cursor = 0
        while cursor + 4 <= len(payload):
            size = int.from_bytes(payload[cursor:cursor + 4], "little")
            cursor += 4 + size
            fields += 1
        helpers = (fields - 2) // 2
        main_marker = 1 + helpers  # one inspector script, then each helper
        outcome["vm_backend"] = markers[main_marker] if len(markers) > main_marker else None
        return {"outcome": outcome}
    except (ValueError, IndexError, AttributeError) as error:
        return runner.result("fail", "runner.protocol", detail=str(error))


def classify(meta, outcome):
    verdict = original_classify(meta, outcome)
    verdict["vm_backend"] = outcome.get("vm_backend")
    return verdict


runner.execute = execute
runner.classify = classify


def coverage(report):
    counts = {"compiled": {}, "fallback": {}, "unclassified": {}}
    for record in report["records"]:
        backends = [v.get("vm_backend") for v in record["variants"]]
        if backends and all(b == "compiled" for b in backends):
            group = "compiled"
        elif "fallback" in backends:
            group = "fallback"
        else:
            group = "unclassified"
        status = record["status"]
        counts[group][status] = counts[group].get(status, 0) + 1
    compiled = sum(counts["compiled"].values())
    fallback = sum(counts["fallback"].values())
    eligible = compiled + fallback
    return {
        "discovered_files": len(report["records"]),
        "eligible_files": eligible,
        "compiled_files": compiled,
        "coverage_of_eligible_percent": round(compiled * 100 / eligible, 4) if eligible else 0,
        "coverage_of_discovered_percent": round(compiled * 100 / len(report["records"]), 4),
        "file_status_by_backend": counts,
        "classification_sha256": report["classification_sha256"],
        "input_sha256": report["input_sha256"],
        "host_sha256": report["host_sha256"],
    }


if __name__ == "__main__":
    code = runner.main()
    if "--output" in sys.argv:
        output = pathlib.Path(sys.argv[sys.argv.index("--output") + 1])
        if output.exists():
            result = coverage(json.loads(output.read_text(encoding="utf-8")))
            sidecar = output.with_suffix(".coverage.json")
            sidecar.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            print(json.dumps(result, indent=2), flush=True)
    sys.exit(code)
