#!/usr/bin/env python3
"""Whole custody policy controls; no Boat operations."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


def load(name, file):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(file))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

c=load('custody','evidence-custody.py')
r=load('reaper','boat-idle-reaper.py')

class CustodyTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.directory=Path(self.temporary.name)
        self.receipt=dict(schema=2,lane='js-lang',source_commit='a'*40,box_id='bx_test',host='user@127.0.0.1',port=2222,job_id='b'*32,output_root='amazing/out/js-lang/job',command_sha256='c'*64,execution_sha256='d'*64,source_archive_sha256='e'*64)
        raw=json.dumps(self.receipt).encode()
        self.row=dict(schema=1,active=True,endpoint=self.receipt,receipt_hex=raw.hex(),receipt_sha256=hashlib.sha256(raw).hexdigest(),created_epoch=1000,custody_deadline_epoch=2800,reason='unknown-exit')
        self.path=self.directory/('bx_test-'+'b'*32+'.json')
    def tearDown(self): self.temporary.cleanup()
    def save(self): c.publish(self.path,self.row)
    def test_absent_no_hold(self):
        self.assertFalse(c.decision(self.directory,'bx_test',1100)['held'])
    def test_unknown_and_terminal_both_hold(self):
        for reason in ['unknown-exit','terminal-unconsumed']:
            self.row['reason']=reason;c.publish(self.path,self.row,replace=True)
            self.assertTrue(c.decision(self.directory,'bx_test',1100)['held'])
    def test_unrelated_box_no_hold(self):
        self.save();self.assertFalse(c.decision(self.directory,'bx_other',1100)['held'])
    def test_finite_extension_and_expired_stop_veto(self):
        self.save()
        self.assertEqual(c.decision(self.directory,'bx_test',1100)['extension_seconds'],900)
        self.assertEqual(c.decision(self.directory,'bx_test',2750)['extension_seconds'],50)
        expired=c.decision(self.directory,'bx_test',3000)
        self.assertTrue(expired['held']);self.assertEqual(expired['extension_seconds'],0)
    def test_explicit_consumption_releases(self):
        self.row.update(active=False,consumption_sha256='f'*64);self.save()
        self.assertFalse(c.decision(self.directory,'bx_test',1100)['held'])
    def test_duplicate_registration_never_overwrites(self):
        self.save();before=self.path.read_bytes()
        with self.assertRaises(FileExistsError):c.publish(self.path,self.row)
        self.assertEqual(before,self.path.read_bytes())
    def test_bad_auth_or_identity_no_renewal(self):
        for field,value in [('receipt_sha256','0'*64),('custody_deadline_epoch',10000),('reason','busy')]:
            row=dict(self.row);row[field]=value;c.publish(self.path,row,replace=True)
            result=c.decision(self.directory,'bx_test',1100)
            self.assertTrue(result['held']);self.assertTrue(result['errors']);self.assertEqual(result['extension_seconds'],0)
    def test_insecure_or_symlink_no_authority(self):
        self.save();self.path.chmod(0o644)
        self.assertTrue(c.decision(self.directory,'bx_test',1100)['errors'])
        self.path.unlink();self.path.symlink_to('/nonexistent')
        self.assertTrue(c.decision(self.directory,'bx_test',1100)['errors'])
    def test_malformed_record_vetoes_without_renewal(self):
        self.path.write_bytes(b'{');self.path.chmod(0o600)
        result=c.decision(self.directory,'bx_test',1100)
        self.assertTrue(result['held']);self.assertEqual(result['extension_seconds'],0)
    def test_extension_never_shortens_or_exceeds_hold(self):
        hold={'extension_seconds':50}
        self.assertEqual(r.custody_extension({'archiveAfter':'1970-01-01T00:20:00Z'},hold,1100),0)
        self.assertEqual(r.custody_extension({'archiveAfter':'1970-01-01T00:18:30Z'},hold,1100),50)
        self.assertEqual(r.custody_extension({},hold,1100),0)

    def test_managed_held_box_never_idle_or_resize_stops(self):
        self.save()
        sandbox=dict(id='bx_test',name='amazing-test',type='large',archiveAfter='1970-01-01T00:18:30Z')
        for switches in [[], ['--no-auto-extend'], ['--extend-custody','--no-auto-extend']]:
            with patch('sys.argv',['reaper',*switches]), patch.object(r,'running_sandboxes',return_value=[sandbox]), patch.object(r,'managed_ids',return_value={'bx_test'}), patch.object(r,'wanted_types',return_value={'bx_test':'default'}), patch.object(r,'starts_left_this_hour',return_value=50), patch.object(r.custody,'DEFAULT_DIRECTORY',self.directory), patch.object(r.time,'time',return_value=1100), patch.object(r.time,'sleep',side_effect=RuntimeError('end control')), patch.object(r,'boat') as boat:
                with self.assertRaisesRegex(RuntimeError,'end control'):r.main()
                boat.assert_not_called()
    def test_explicit_custody_renewal_has_finite_exact_ttl(self):
        self.row['custody_deadline_epoch']=1150;self.save()
        sandbox=dict(id='bx_test',name='amazing-test',type='default',archiveAfter='1970-01-01T00:18:30Z')
        with patch('sys.argv',['reaper','--extend-custody']), patch.object(r,'running_sandboxes',return_value=[sandbox]), patch.object(r,'managed_ids',return_value={'bx_test'}), patch.object(r,'wanted_types',return_value={}), patch.object(r,'starts_left_this_hour',return_value=50), patch.object(r.custody,'DEFAULT_DIRECTORY',self.directory), patch.object(r.time,'time',return_value=1100), patch.object(r.time,'sleep',side_effect=RuntimeError('end control')), patch.object(r,'boat',return_value={'event':'extended'}) as boat:
            with self.assertRaisesRegex(RuntimeError,'end control'):r.main()
            boat.assert_called_once_with('extend','bx_test','--ttl','50')

if __name__=='__main__':unittest.main()
