#!/usr/bin/env python3
"""Tests for scripts/src-delta.py: manifest, delta, box-side rebuild, verification and corruption fallback."""
import argparse
import importlib.util
import io
import json
import os
import pathlib
import shutil
import sys
import tarfile
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent


def load(home):
    os.environ['HOME'] = str(home)
    spec = importlib.util.spec_from_file_location('src_delta', HERE / 'src-delta.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write(root, files):
    for name, text in files.items():
        path = pathlib.Path(root) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


class DeltaTests(unittest.TestCase):
    def setUp(self):
        self.temp = pathlib.Path(tempfile.mkdtemp())
        self.mod = load(self.temp / 'home')
        self.base = self.temp / 'base'
        write(self.base, {'Cargo.toml': '[workspace]\n', 'a/one.rs': 'one', 'a/two.rs': 'two', 'gone.txt': 'x'})
        os.symlink('a/one.rs', self.base / 'link')
        (self.base / 'run.sh').write_text('#!/bin/sh\n')
        (self.base / 'run.sh').chmod(0o755)
        # Excluded top-level names never enter the manifest.
        write(self.base, {'target/junk': 'j', '.git/HEAD': 'h', 'source-transfer.manifest.json': '{}'})

    def tearDown(self):
        shutil.rmtree(self.temp, ignore_errors=True)

    def seed_cache(self, root, label):
        manifest = self.mod.build_manifest(root)
        archive = self.temp / 'seed.tar.gz'
        self.mod.pack_tree(str(root), manifest, str(archive))
        self.mod.register(str(archive), manifest, label)
        return manifest

    def target(self):
        target = self.temp / 'target-tree'
        shutil.copytree(self.base, target, symlinks=True)
        (target / 'a/one.rs').write_text('ONE changed')
        (target / 'new/file.rs').parent.mkdir()
        (target / 'new/file.rs').write_text('new')
        (target / 'gone.txt').unlink()
        (target / 'run.sh').chmod(0o644)
        return target

    def apply(self, base_sha, target_manifest, expect_sha=None):
        delta = self.temp / 'delta.tar.gz'
        base_manifest = json.loads((self.mod.CACHE / (base_sha + '.json')).read_text())['manifest']
        self.mod.build_delta(str(self.target_root), target_manifest, base_manifest, delta)
        out = self.temp / 'source.tar.gz'
        args = argparse.Namespace(delta=str(delta), base=base_sha, manifest_sha=expect_sha or self.mod.manifest_identity(target_manifest),
                                  label='t', out=str(out))
        self.mod.cmd_apply(args)
        return out

    def test_manifest_excludes_and_tracks_modes(self):
        manifest = self.mod.build_manifest(self.base)
        self.assertNotIn('target/junk', manifest)
        self.assertNotIn('.git/HEAD', manifest)
        self.assertNotIn('source-transfer.manifest.json', manifest)
        self.assertEqual(manifest['run.sh'][1], 1)
        self.assertEqual(manifest['link'][:3], ['l', 0, 'a/one.rs'])

    def test_delta_rebuild_matches_manifest(self):
        base_manifest = self.seed_cache(self.base, 'a' * 40)
        self.target_root = self.target()
        target_manifest = self.mod.build_manifest(self.target_root)
        base_sha = self.mod.manifest_identity(base_manifest)
        out = self.apply(base_sha, target_manifest)
        # Only changed/added files travel.
        delta = self.temp / 'delta.tar.gz'
        names = tarfile.open(delta).getnames()
        self.assertEqual(sorted(n for n in names if not n.startswith('.delta')), ['a/one.rs', 'new/file.rs', 'run.sh'])
        rebuilt = self.temp / 'rebuilt'
        rebuilt.mkdir()
        with tarfile.open(out) as stream:
            stream.extractall(rebuilt, filter='data')
        self.assertEqual(self.mod.build_manifest(rebuilt), target_manifest)
        self.assertFalse((rebuilt / 'gone.txt').exists())
        # The rebuilt tree is cached for the next delta.
        self.assertIn(self.mod.manifest_identity(target_manifest), [e['manifest_sha256'] for e in self.mod.cache_entries()])

    def test_corrupted_cache_content_is_rejected(self):
        base_manifest = self.seed_cache(self.base, 'b' * 40)
        base_sha = self.mod.manifest_identity(base_manifest)
        # Replace the cached archive with one whose content no longer matches the unchanged-file hashes.
        tampered = self.temp / 'tampered'
        shutil.copytree(self.base, tampered, symlinks=True)
        (tampered / 'a/two.rs').write_text('TAMPERED')
        manifest = self.mod.build_manifest(tampered)
        archive = self.temp / 'tampered.tar.gz'
        self.mod.pack_tree(str(tampered), manifest, str(archive))
        shutil.copyfile(archive, self.mod.CACHE / (base_sha + '.tar.gz'))
        meta = self.mod.CACHE / (base_sha + '.json')
        row = json.loads(meta.read_text()); row['archive_bytes'] = archive.stat().st_size; meta.write_text(json.dumps(row))
        self.target_root = self.target()
        with self.assertRaises(SystemExit) as raised:
            self.apply(base_sha, self.mod.build_manifest(self.target_root))
        self.assertIn('mismatch', str(raised.exception))

    def test_corrupted_cache_archive_is_rejected(self):
        base_manifest = self.seed_cache(self.base, 'c' * 40)
        base_sha = self.mod.manifest_identity(base_manifest)
        path = self.mod.CACHE / (base_sha + '.tar.gz')
        data = bytearray(path.read_bytes()); data[len(data) // 2] ^= 0xFF; path.write_bytes(bytes(data))
        self.target_root = self.target()
        with self.assertRaises((Exception, SystemExit)):
            self.apply(base_sha, self.mod.build_manifest(self.target_root))

    def test_wrong_expected_identity_is_rejected(self):
        base_manifest = self.seed_cache(self.base, 'd' * 40)
        self.target_root = self.target()
        with self.assertRaises(SystemExit):
            self.apply(self.mod.manifest_identity(base_manifest), self.mod.build_manifest(self.target_root), expect_sha='0' * 64)

    def test_cache_keeps_newest_three(self):
        for index in range(5):
            tree = self.temp / f'tree{index}'
            write(tree, {'Cargo.toml': '[workspace]', 'n.txt': str(index)})
            self.seed_cache(tree, f'{index:040d}')
        self.assertEqual(len(self.mod.cache_entries()), 3)

    def test_choose_base_prefers_nearest_commit(self):
        entries = [{'label': 'a' * 40, 'manifest_sha256': '1'}, {'label': 'unknown+wt', 'manifest_sha256': '2'}]
        self.assertEqual(self.mod.choose_base(entries, str(self.temp), '')['manifest_sha256'], '1')


if __name__ == '__main__':
    unittest.main()
