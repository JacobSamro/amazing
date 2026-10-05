#!/usr/bin/env python3
"""Native public diagnostic contract controls; no engine test or raw transfer."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile

spec = importlib.util.spec_from_file_location('diagnostic', Path(__file__).with_name('gate-failure-diagnostic.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def run(output):
    records = []
    with tempfile.TemporaryDirectory(prefix='amazing-diagnostic-controls-', dir='/tmp') as scratch:
        root = Path(scratch).resolve()
        raw_path, summary_path, terminal_path = (root / name for name in ('raw.log', 'summary.json', 'terminal.json'))
        source, job = 'a' * 40, 'b' * 32
        raw = b'PASS unrelated\nFAIL [0.1s] engine::whole_area\nthread panicked at source.rs:22\nassertion `left == right` failed\n  left: 1\n right: 2\n'
        base_summary = {'source_commit': source, 'job_exit': 1, 'phases': [
            {'phase': 'nextest', 'source_commit': source, 'actual_returncode': 100,
             'argv': ['cargo', 'nextest', 'run', '--workspace'],
             'raw': {'path': str(raw_path), 'bytes': len(raw), 'sha256': module.digest(raw)}}]}
        base_terminal = {'state': 'TERMINAL', 'actual_exit': 1, 'runner_exit': 1,
                         'command_returncode': 1, 'job': {'source_commit': source, 'job_id': job}}
        cases = [('authentic-failure', None, None, True),
                 ('terminal-running', 'state', 'RUNNING', False),
                 ('terminal-unknown', 'state', 'UNKNOWN', False),
                 ('native-zero', 'actual_exit', 0, False),
                 ('runner-mismatch', 'runner_exit', 3, False),
                 ('command-mismatch', 'command_returncode', 3, False),
                 ('native-bool', 'actual_exit', True, False),
                 ('wrong-source', 'source', 'c' * 40, False),
                 ('wrong-job', 'job', 'c' * 32, False),
                 ('summary-source', 'summary-source', 'c' * 40, False),
                 ('summary-exit', 'summary-exit', 0, False),
                 ('phase-pass', 'phase-exit', 0, False),
                 ('phase-source', 'phase-source', 'c' * 40, False),
                 ('missing-phase', 'missing-phase', None, False),
                 ('duplicate-phase', 'duplicate-phase', None, False),
                 ('raw-size', 'raw-size', 999, False),
                 ('raw-hash', 'raw-hash', '0' * 64, False),
                 ('summary-hash', 'summary-hash', '0' * 64, False),
                 ('terminal-hash', 'terminal-hash', '0' * 64, False),
                 ('raw-outside-native', 'raw-path', '/etc/passwd', False),
                 ('raw-symlink', 'symlink', None, False),
                 ('missing-raw', 'missing-raw', None, False),
                 ('no-marker', 'raw-data', b'PASS whole_area\n', False),
                 ('bounded-large-assertion', 'raw-data', b'FAIL whole_area\n' + b'assertion failed ' + b'x' * 200000, True)]
        for name, key, value, expected in cases:
            summary, terminal = copy.deepcopy(base_summary), copy.deepcopy(base_terminal)
            row = summary['phases'][0]
            actual_raw = value if key == 'raw-data' else raw
            if raw_path.exists() or raw_path.is_symlink(): raw_path.unlink()
            raw_path.write_bytes(actual_raw)
            row['raw']['bytes'], row['raw']['sha256'] = len(actual_raw), module.digest(actual_raw)
            if key in ('state', 'actual_exit', 'runner_exit', 'command_returncode'): terminal[key] = value
            elif key == 'source': terminal['job']['source_commit'] = value
            elif key == 'job': terminal['job']['job_id'] = value
            elif key == 'summary-source': summary['source_commit'] = value
            elif key == 'summary-exit': summary['job_exit'] = value
            elif key == 'phase-exit': row['actual_returncode'] = value
            elif key == 'phase-source': row['source_commit'] = value
            elif key == 'missing-phase': summary['phases'] = []
            elif key == 'duplicate-phase': summary['phases'].append(copy.deepcopy(row))
            elif key == 'raw-size': row['raw']['bytes'] = value
            elif key == 'raw-hash': row['raw']['sha256'] = value
            elif key == 'raw-path': row['raw']['path'] = value
            elif key == 'symlink':
                raw_path.unlink(); raw_path.symlink_to(summary_path)
            elif key == 'missing-raw': raw_path.unlink()
            summary_path.write_text(json.dumps(summary)); terminal_path.write_text(json.dumps(terminal))
            summary_sha = value if key == 'summary-hash' else module.digest(summary_path.read_bytes())
            terminal_sha = value if key == 'terminal-hash' else module.digest(terminal_path.read_bytes())
            try:
                payload = module.extract(str(summary_path), summary_sha, str(terminal_path), terminal_sha, source, job, 'nextest')
                result = json.loads(payload)
                assert len(payload) <= module.MAX_OUTPUT
                assert result['original_actual_exit'] == 1 and result['phase_actual_exit'] == 100
                assert result['counts_reconstructed'] is False and result['tests_rerun'] is False
                assert any('whole_area' in line['text'] for line in result['snippets'])
                passed = True
            except (ValueError, FileNotFoundError): passed = False
            assert passed == expected, name
            records.append({'id': name, 'classification': 'PASS'})
    payload = {'schema': 1, 'pass': len(records), 'fail': 0, 'controls': records}
    with Path(output).open('x') as stream: json.dump(payload, stream, sort_keys=True); stream.write('\n')


if __name__ == '__main__':
    import sys
    run(sys.argv[1])
