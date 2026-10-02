#!/usr/bin/env python3
"""Run VM thread-retention and Unicode-harness timing probes on Linux only.

Usage: python3 crates/amazing-js/src/vm/probes.py HOST_BINARY TEST262_ROOT
The host binary must have been rebuilt from the current lane before this run.
"""
import json
import os
import pathlib
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path("scripts").resolve()))
import test262 as runner


def probe(name, source, helpers, vm=False, timeout=90):
    env = os.environ.copy()
    env.pop("AMAZING_JS_VM", None)
    env.pop("AMAZING_JS_VM_COMPILE_LOG", None)
    env["AMAZING_JS_VM_TRACE"] = "1"
    if vm:
        env["AMAZING_JS_VM"] = "1"
    peak = {"threads": 0, "rss_kb": 0}
    started = time.monotonic()
    process = subprocess.Popen([sys.argv[1]], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)

    def sample():
        while process.poll() is None:
            try:
                for line in pathlib.Path(f"/proc/{process.pid}/status").read_text().splitlines():
                    if line.startswith("Threads:"):
                        peak["threads"] = max(peak["threads"], int(line.split()[1]))
                    if line.startswith("VmRSS:"):
                        peak["rss_kb"] = max(peak["rss_kb"], int(line.split()[1]))
            except (FileNotFoundError, ProcessLookupError):
                break
            time.sleep(0.05)

    sampling = threading.Thread(target=sample)
    sampling.start()
    timed_out = False
    try:
        stdout, stderr = process.communicate(runner.request(source, "non-strict", helpers), timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        process.kill()
        stdout, stderr = process.communicate()
    sampling.join()
    return {"name": name, "vm": vm, "seconds": round(time.monotonic() - started, 3),
            "timeout": timed_out, "exit_code": process.returncode, **peak,
            "stdout": stdout.decode(errors="replace")[-2048:],
            "stderr": stderr.decode(errors="replace")[-2048:]}


root = pathlib.Path(sys.argv[2])
retention = """
var settle, completed=0;
var pending=new Promise(function(r){settle=r});
async function work(){
    using resource=null;
    await pending; await pending;
    completed++;
}
var tail;
for(var n=0;n<5000;n++) tail=work();
print('parked 5000 using bodies');
tail.then(function(){print('completed '+completed)});
settle();
"""
print(json.dumps(probe("5000-using-async-bodies", retention, []), indent=2), flush=True)
helpers = [("regExpUtils.js", (root / "harness/regExpUtils.js").read_text())]
source = "print(buildString({loneCodePoints:[],ranges:[[0,0x10ffff]]}).length);"
for vm in (False, True):
    print(json.dumps(probe("unicode-buildString", source, helpers, vm=vm), indent=2), flush=True)
