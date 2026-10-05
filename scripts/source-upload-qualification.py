#!/usr/bin/env python3
"""Whole native source staging/transport tooling controls, never engine gates."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    scripts = Path(__file__).resolve().parent
    modules = []
    suite = unittest.TestSuite()
    for name in ('native-input-tests.py', 'source-upload-tests.py'):
        path = scripts / name
        spec = importlib.util.spec_from_file_location(path.stem, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[path.stem] = module
        spec.loader.exec_module(module)
        modules.append(module)
        suite.addTests(unittest.TestLoader().loadTestsFromModule(module))
    if not suite.countTestCases():
        raise RuntimeError('Whole source transport control suite is empty')

    class Results(unittest.TextTestResult):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.controls = []
        def addSuccess(self, test):
            super().addSuccess(test); self.controls.append({'id': test.id(), 'classification': 'pass'})
        def addFailure(self, test, error):
            super().addFailure(test, error); self.controls.append({'id': test.id(), 'classification': 'fail'})
        def addError(self, test, error):
            super().addError(test, error); self.controls.append({'id': test.id(), 'classification': 'error'})
        def addSubTest(self, test, subtest, error):
            super().addSubTest(test, subtest, error)
            if error is not None:
                self.controls.append({'id': subtest.id(), 'classification': 'fail' if issubclass(error[0], test.failureException) else 'error'})
        def addSkip(self, test, reason):
            super().addSkip(test, reason); self.controls.append({'id': test.id(), 'classification': 'skip', 'reason': reason})

    result = unittest.TextTestRunner(verbosity=2, resultclass=Results).run(suite)
    files = ('linux-run.sh', 'source-upload.py', 'source-upload-tests.py', 'source-upload-qualification.py',
             'native-inputs.py', 'native-input-tests.py', 'completed-job.py', 'gate-summary.py')
    summary = {'schema': 1, 'source': args.source,
               'scope': 'Native source transport and staging tooling only; synthetic packets and actual children, no engine conformance',
               'counts': {'tests': result.testsRun, 'passed': sum(r['classification'] == 'pass' for r in result.controls),
                          'failed': len(result.failures), 'errors': len(result.errors), 'skipped': len(result.skipped)},
               'controls': result.controls,
               'native_control_receipts': [row for module in modules for row in getattr(module, 'CONTROL_RECEIPTS', [])],
               'public_sources': {name: {'bytes': (scripts / name).stat().st_size,
                    'sha256': hashlib.sha256((scripts / name).read_bytes()).hexdigest()} for name in files},
               'prior_supervisor_qualification': 'Published5d020117; unchanged completed-job bytes, native36/36 controls retained in existing compact',
               'actual_suite_exit': int(not result.wasSuccessful())}
    output = Path(args.output); output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(summary, separators=(',', ':')) + '\n')
    print(json.dumps({'summary': str(output), 'counts': summary['counts'], 'actual_exit': summary['actual_suite_exit']}), flush=True)
    return summary['actual_suite_exit']


if __name__ == '__main__':
    raise SystemExit(main())
