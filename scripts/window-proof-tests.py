#!/usr/bin/env python3
"""Metadata fixtures only: no engine, X server, network or Boat calls."""
import copy
import hashlib
import importlib.util
import pathlib
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('window_proof', pathlib.Path(__file__).with_name('window-proof.py'))
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)

class WindowProof(unittest.TestCase):
    def fixture(self, size=24):
        raw = b'\x89PNG\r\n\x1a\n' + struct.pack('>I',13) + b'IHDR' + struct.pack('>II',800,600)
        raw += b'x' * (size - len(raw))
        image = {'path':'/home/user/amazing/out/js-date/captures/run/run.png',
                 'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),
                 'exe':{'path':'/tmp/amazing-shell-target-js-date/debug/amazing-shell','sha256':'a'*64},
                 'pid':123,'window':{'id':'456','title':'Amazing'},
                 'source_commit':'b'*40,'fixture':'file:///fixture.html',
                 'viewport':{'display_size':[800,600],'window_size':[800,600]},
                 'captured_at':'2026-10-04T12:00:00+00:00'}
        return {'schema':1,'kind':'actual-window-proof','lane':'js-date','images':[image]},raw

    def test_valid_exact_window_metadata_and_bytes(self):
        manifest,raw=self.fixture()
        proof.validate(manifest,'js-date')
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'run.png';p.write_bytes(raw)
            proof.verify_image(manifest['images'][0],p)

    def test_invalid_paths_never_become_transfer_operands(self):
        for path in ['/home/user/amazing/out/js-date/x/*.png',
                     '/home/user/amazing/out/js-date/../net/raw.png',
                     '/home/user/amazing/out/net/raw.png','relative.png',
                     '/home/user/amazing/out/js-date/raw.log',
                     '/home/user/amazing/out/js-date/x.png;echo']:
            with self.subTest(path=path),self.assertRaises(ValueError):proof.exact_path(path,'js-date')

    def test_image_and_whole_proof_budget(self):
        manifest,_=self.fixture(proof.IMAGE_LIMIT+1)
        with self.assertRaisesRegex(ValueError,'512 KiB'):proof.validate(manifest,'js-date')
        manifest,_=self.fixture(proof.IMAGE_LIMIT)
        manifest['images']=[copy.deepcopy(manifest['images'][0]) for _ in range(4)]
        for n,image in enumerate(manifest['images']):image['path']=image['path'].replace('run.png',f'{n}.png')
        with self.assertRaisesRegex(ValueError,'2 MiB'):proof.validate(manifest,'js-date')

    def test_hash_size_and_window_dimensions_mismatch(self):
        manifest,raw=self.fixture()
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'run.png';p.write_bytes(raw)
            image=manifest['images'][0]
            image['sha256']='0'*64
            with self.assertRaisesRegex(ValueError,'hash/size'):proof.verify_image(image,p)
            image['sha256']=hashlib.sha256(raw).hexdigest();image['viewport']['window_size']=[1,1]
            with self.assertRaisesRegex(ValueError,'dimensions'):proof.verify_image(image,p)
            p.write_bytes(raw[:-1])
            with self.assertRaisesRegex(ValueError,'hash/size'):proof.verify_image(image,p)

    def test_identity_timestamp_and_duplicate_image(self):
        for field,value in [('source_commit','not-a-sha'),('fixture',''),('pid',0),('captured_at','2026-10-04T12:00:00')]:
            manifest,_=self.fixture();manifest['images'][0][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):proof.validate(manifest,'js-date')
        manifest,_=self.fixture();manifest['images'].append(copy.deepcopy(manifest['images'][0]))
        with self.assertRaisesRegex(ValueError,'duplicate'):proof.validate(manifest,'js-date')

    def test_gate_fetch_remains_compact_only_and_all_capture_modes_use_window_proof(self):
        scripts=pathlib.Path(__file__).parent
        capture=(scripts/'linux-shell-capture.sh').read_text()
        body=capture.split("REMOTE_BODY=\"$(cat <<'REMOTE'\n",1)[1].split('\nREMOTE\n',1)[0]
        import subprocess
        self.assertEqual(subprocess.run(['bash','-n'],input=body,text=True,capture_output=True).returncode,0)
        self.assertIn('lane-fetch-window.sh',capture)
        self.assertNotIn('lane-fetch.sh',capture)
        self.assertNotIn('*.png',capture)
        self.assertIn('shot "$slug" "$url"',capture)
        self.assertIn('shot "$NAME-$rest"',capture)
        self.assertIn('shot "$NAME"',capture)
        # Existing gate interface rejects image inputs before any remote call.
        import subprocess
        result=subprocess.run(['bash',str(scripts/'lane-fetch.sh'),'js-date','/tmp/unused-window-proof','amazing/out/js-date/x.png'],capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('compact',result.stderr)

if __name__=='__main__':unittest.main()
