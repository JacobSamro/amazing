#!/usr/bin/env python3
"""Finite native contract check for the public runner's development debug profile."""
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    output = Path(os.environ['OUT']) / 'native-debug-profile'
    output.mkdir(parents=True, exist_ok=False)
    assert os.environ['CARGO_PROFILE_DEV_DEBUG'] == 'line-tables-only'
    assert os.environ['CARGO_PROFILE_TEST_DEBUG'] == 'line-tables-only'
    assert os.environ['CARGO_INCREMENTAL'] == '0'
    fixture = output / 'fixture'
    (fixture / 'src').mkdir(parents=True)
    (fixture / 'Cargo.toml').write_text('''[package]
name = "native-profile-contract"
version = "0.0.0"
edition = "2024"
[workspace]
''')
    (fixture / 'src/lib.rs').write_text('''/// Runner retains ordinary documentation tests.
/// ```
/// assert_eq!(native_profile_contract::identity(3), 3);
/// ```
pub fn identity(value: u32) -> u32 { std::hint::black_box(value) }
#[cfg(test)]
mod tests {
    #[test]
    fn debug_assertions_are_enabled() {
        assert!(cfg!(debug_assertions));
        assert!(std::panic::catch_unwind(|| debug_assert!(false)).is_err());
    }
    #[test]
    fn overflow_checks_are_enabled() {
        let value = std::hint::black_box(u32::MAX);
        assert!(std::panic::catch_unwind(|| value + 1).is_err());
    }
}
''')
    results = []
    def run(name, args, env=None):
        log = output / (name + '.log')
        with log.open('wb') as handle:
            proc = subprocess.run(args, cwd=fixture, env=env, stdout=handle, stderr=subprocess.STDOUT)
        results.append({'name': name, 'exit': proc.returncode, 'raw_path': str(log),
                        'raw_sha256': hashlib.sha256(log.read_bytes()).hexdigest()})
        return proc.returncode
    cargo = ['cargo', 'test', '--offline', '--verbose']
    code = run('line-tests', cargo + ['--lib'])
    code |= run('line-doctests', cargo + ['--doc'])
    # Tiny reference fixture only; independent target, same test source and compiler.
    full = os.environ.copy()
    full.update(CARGO_PROFILE_DEV_DEBUG='2', CARGO_PROFILE_TEST_DEBUG='2',
                CARGO_TARGET_DIR=str(output / 'full-debug-reference-target'))
    code |= run('full-tests', cargo + ['--lib'], full)
    line_root = Path(os.environ['CARGO_TARGET_DIR'])
    executables = {}
    for label, target in [('line', line_root), ('full', Path(full['CARGO_TARGET_DIR']))]:
        bins = [p for p in (target / 'debug/deps').glob('native_profile_contract-*')
                if p.is_file() and os.access(p, os.X_OK) and p.suffix == '']
        assert len(bins) == 1, [str(p) for p in bins]
        binary = bins[0]
        sections = subprocess.check_output(['readelf', '-SW', str(binary)], text=True)
        assert '.debug_line ' in sections
        executables[label] = {'path': str(binary), 'bytes': binary.stat().st_size,
                             'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                             'debug_line_present': True}
    assert 'debuginfo=line-tables-only' in (output / 'line-tests.log').read_text()
    assert 'debuginfo=2' in (output / 'full-tests.log').read_text()
    assert executables['line']['bytes'] < executables['full']['bytes']
    receipt = {'actual_exit': code, 'environment': {k: os.environ[k] for k in
               ['CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_INCREMENTAL']},
               'source_commit': os.environ.get('AMAZING_SOURCE_COMMIT'),
               'tests_per_profile': 2, 'doctests': 1, 'commands': results,
               'executables': executables, 'raw_preserved': True}
    (output / 'compact.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
