#!/usr/bin/env python3
"""Run the complete public gate tooling contracts; raw output remains native."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

SUITES = ('completed-job-tests.py', 'completed-job-preflight-tests.py',
          'completed-job-lifecycle-tests.py', 'completed-job-large-command-tests.py',
          'native-input-tests.py', 'test262-job-state-tests.py')

def main():
    source = os.environ['COMMIT']
    root = Path(os.environ['OUT']) / 'public-gate-contracts' / source
    root.mkdir(parents=True, exist_ok=False)
    result = {'schema': 1, 'source_commit': source, 'suites': {},
              'started_ns': time.time_ns(), 'failed_suites': [], 'tests': 0}
    for name in SUITES:
        raw = root / (name + '.log')
        with raw.open('xb') as output:
            process = subprocess.run([sys.executable, '-I', str(Path(__file__).parent / name)],
                                     stdout=output, stderr=subprocess.STDOUT, timeout=300)
            output.flush(); os.fsync(output.fileno())
        data = raw.read_bytes()
        count = re.search(rb'Ran (\d+) tests? in', data)
        tests = int(count.group(1)) if count else 0
        result['suites'][name] = {'actual_exit': process.returncode, 'tests': tests,
            'source_sha256': hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest(),
            'raw': {'path': str(raw), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}}
        result['tests'] += tests
        if process.returncode or not count:
            result['failed_suites'].append(name)
            # Finite diagnostic tail is observation, not a transferred raw log.
            print(json.dumps({'suite': name, 'actual_exit': process.returncode,
                'diagnostic_tail': data.decode(errors='replace').splitlines()[-30:]}), flush=True)
        else:
            print(json.dumps({'suite': name, 'actual_exit': 0, 'tests': tests}), flush=True)
    result['finished_ns'] = time.time_ns()
    result['actual_exit'] = 1 if result['failed_suites'] else 0
    compact = root / 'contracts.summary.json'
    with compact.open('x') as output:
        json.dump(result, output, sort_keys=True); output.write('\n'); output.flush(); os.fsync(output.fileno())
    print(json.dumps({'summary': str(compact), 'actual_exit': result['actual_exit'], 'tests': result['tests']}), flush=True)
    return result['actual_exit']

if __name__ == '__main__':
    sys.exit(main())
