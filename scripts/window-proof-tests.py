#!/usr/bin/env python3
"""Metadata fixtures only: no engine, X server, network or Boat calls."""
import copy
import hashlib
import importlib.util
import pathlib
import struct
import tempfile
import types
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
        observed={'exe':image['exe'],'pid':123,'window_id':'456','geometry':[0,0,800,600],'display_size':[800,600]}
        image['capture']={'backend':'ImageMagick import +frame +border -window',
                          'before':copy.deepcopy(observed),'after':copy.deepcopy(observed),
                          'began_at':'2026-10-04T12:00:00+00:00'}
        return {'schema':1,'kind':'actual-window-proof','lane':'js-date','images':[image]},raw

    def test_actual_engine_viewport_and_fractional_density_metadata(self):
        manifest,_=self.fixture()
        image=manifest['images'][0]
        image['engine_viewport']={'pid':123,'source_commit':'b'*40,'css_width':380.5,
            'css_height':250.25,'device_scale':2.,'raster_width':761,'raster_height':501,
            'window_css_width':400.,'window_css_height':300.}
        proof.validate(manifest,'js-date')
        for key,value in [('pid',124),('source_commit','c'*40),('raster_width',760),
                          ('window_css_width',401.),('device_scale',float('nan'))]:
            bad=copy.deepcopy(manifest);bad['images'][0]['engine_viewport'][key]=value
            with self.assertRaises(ValueError):proof.validate(bad,'js-date')

    def test_actual_frame_log_requires_success_pid_fields_and_platform_scale(self):
        with tempfile.TemporaryDirectory() as d:
            path=pathlib.Path(d)/'shell.log'
            args=types.SimpleNamespace(engine_log=str(path),device_scale='2',source_commit='b'*40)
            line='AMAZING_VIEWPORT pid=123 css_width=380.5 css_height=250.25 device_scale=2 raster_width=761 raster_height=501 window_css_width=400 window_css_height=300'
            path.write_text(line+'\n')
            self.assertEqual(proof.engine_viewport(args)['raster_width'],761)
            path.write_text(line+'\nAMAZING_VIEWPORT_FAILURE pid=123\n')
            with self.assertRaises(ValueError):proof.engine_viewport(args)
            path.write_text(line+'\n');args.device_scale='1'
            with self.assertRaises(ValueError):proof.engine_viewport(args)
            path.write_text('not a native frame\n')
            with self.assertRaises(ValueError):proof.engine_viewport(args)

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

    def test_native_owner_requires_exact_lane_tmp_path_and_retains_png_budgets(self):
        owner='/tmp/amazing-viewport-baked-output/js-date'
        manifest,_=self.fixture();manifest['native_output_root']=owner
        manifest['images'][0]['path']=owner+'/captures/run/run.png'
        proof.validate(manifest,'js-date')
        for root in ['/tmp','/tmp/js-date/..','/home/user/js-date','/tmp/owner/net',owner+'/']:
            bad=copy.deepcopy(manifest);bad['native_output_root']=root
            with self.subTest(root=root),self.assertRaises(ValueError):proof.validate(bad,'js-date')
        for path in ['/tmp/amazing-viewport-baked-output/net/x.png',owner+'-other/x.png',owner+'/../x.png',owner+'/./x.png',owner+'/x.log']:
            with self.subTest(path=path),self.assertRaises(ValueError):proof.exact_path(path,'js-date',owner)
        bad=copy.deepcopy(manifest);bad['images'][0]['bytes']=proof.IMAGE_LIMIT+1
        with self.assertRaisesRegex(ValueError,'512 KiB'):proof.validate(bad,'js-date')

    def test_native_capture_records_original_window_identity_and_all_ten_shots(self):
        manifest,raw=self.fixture();observed=manifest['images'][0]['capture']['before']
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            owner=pathlib.Path(d)/'js-date';owner.mkdir();stored=owner/'proof.window.manifest.json'
            def imported(command,check):pathlib.Path(command[-1]).write_bytes(raw)
            with mock.patch.object(proof,'snapshot',return_value=observed),mock.patch.object(proof.subprocess,'run',side_effect=imported),mock.patch.object(proof.subprocess,'check_output',return_value='Amazing'):
                for n in range(10):
                    args=types.SimpleNamespace(image=str(owner/f'shot-{n}.png'),native_output_root=str(owner),lane='js-date',size='800x600',window='456',pid=123,source_commit='b'*40,fixture=f'original-actions shot={n}',manifest=str(stored))
                    proof.capture(args)
            recorded=__import__('json').loads(stored.read_text());proof.validate(recorded,'js-date')
            self.assertEqual(recorded['native_output_root'],str(owner));self.assertEqual(len(recorded['images']),10)
            for image in recorded['images']:
                self.assertEqual(image['capture']['before'],image['capture']['after'])
                proof.verify_image(image,owner/pathlib.Path(image['path']).name)

    def test_native_failure_publishes_diagnostic_and_symlink_escape_is_rejected(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as d:
            owner=pathlib.Path(d)/'js-date';owner.mkdir();image=owner/'shot.png'
            args=types.SimpleNamespace(image=str(image),native_output_root=str(owner),lane='js-date',source_commit='b'*40,fixture='actual fixture',pid=123,window='456')
            with mock.patch.object(proof,'snapshot',side_effect=ValueError('actual failed window')):
                proof.failure(args,ValueError('original failure'))
            self.assertTrue(image.with_suffix('.failure.manifest.json').is_file())
            outside=pathlib.Path(d)/'outside';outside.mkdir();link=owner/'escape';link.symlink_to(outside,target_is_directory=True)
            args.image=str(link/'shot.png')
            with self.assertRaises(ValueError):proof.capture(args)

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

    def test_whole_window_geometry_requires_visible_stable_bounds(self):
        proof.check_geometry([0,0,800,600],[800,600],[800,600])
        proof.check_geometry([10,20,300,200],[800,600],[800,600])
        for bounds in [[-1,0,800,600],[0,-1,800,600],[1,0,800,600],[0,1,800,600],[0,0,801,600],[0,0,0,600]]:
            with self.subTest(bounds=bounds),self.assertRaises(ValueError):proof.check_geometry(bounds,[800,600],[800,600])
        with self.assertRaisesRegex(ValueError,'actual display'):
            proof.check_geometry([0,0,800,600],[1024,768],[800,600])
        manifest,_=self.fixture();manifest['images'][0]['capture']['after']['geometry'][0]=1
        with self.assertRaisesRegex(ValueError,'unstable'):proof.validate(manifest,'js-date')

    def test_snapshot_requires_actual_process_and_window_identity(self):
        with tempfile.TemporaryDirectory() as d:
            exe=pathlib.Path(d)/'amazing-shell';exe.write_bytes(b'fixture executable')
            args=types.SimpleNamespace(pid=123,exe=str(exe),window='456')
            original_readlink=proof.os.readlink
            def readlink(path,*args,**kwargs):
                return str(exe.resolve()) if str(path)=='/proc/123/exe' else original_readlink(path,*args,**kwargs)
            def observed(command,text=True):
                if command[1]=='getwindowpid':return '123'
                if command[1]=='getdisplaygeometry':return '800 600'
                return 'WINDOW=456\nX=0\nY=0\nWIDTH=800\nHEIGHT=600\nSCREEN=0\n'
            with mock.patch.object(proof.os,'readlink',side_effect=readlink),mock.patch.object(proof.subprocess,'check_output',side_effect=observed):
                state=proof.snapshot(args)
                self.assertEqual(state['geometry'],[0,0,800,600])
                self.assertEqual(state['exe']['sha256'],proof.digest(exe))
            with mock.patch.object(proof.os,'readlink',side_effect=readlink),mock.patch.object(proof.subprocess,'check_output',return_value='124'):
                with self.assertRaisesRegex(ValueError,'actual shell PID'):proof.snapshot(args)
            with mock.patch.object(proof.os,'readlink',side_effect=readlink),mock.patch.object(proof.subprocess,'check_output',side_effect=['123','WINDOW=457\n']):
                with self.assertRaisesRegex(ValueError,'another window'):proof.snapshot(args)

    def test_backend_records_all_ten_action_shots_before_and_after(self):
        manifest,raw=self.fixture();observed=manifest['images'][0]['capture']['before']
        with tempfile.TemporaryDirectory() as d:
            out=pathlib.Path(d);stored=out/'proof.window.manifest.json'
            def imported(command,check):
                self.assertEqual(command[:5],['import','-silent','+frame','+border','-window'])
                pathlib.Path(command[-1]).write_bytes(raw)
            with mock.patch.object(proof,'exact_path',side_effect=lambda path,lane:path),mock.patch.object(proof,'snapshot',return_value=observed),mock.patch.object(proof.subprocess,'run',side_effect=imported),mock.patch.object(proof.subprocess,'check_output',return_value='Amazing'):
                for n in range(10):
                    args=types.SimpleNamespace(image=str(out/f'shot-{n}.png'),lane='js-date',size='800x600',window='456',pid=123,source_commit='b'*40,fixture=f'original-actions shot={n}',manifest=str(stored))
                    proof.capture(args)
            recorded=__import__('json').loads(stored.read_text())
            self.assertEqual(len(recorded['images']),10)
            self.assertTrue(all(i['capture']['before']==i['capture']['after'] for i in recorded['images']))
            for image in recorded['images']:proof.verify_image(image,out/pathlib.Path(image['path']).name)

    def test_backend_rejects_crop_geometry_change_or_pid_change(self):
        manifest,raw=self.fixture();observed=manifest['images'][0]['capture']['before']
        for mode in ['cropped','moved','pid']:
            with self.subTest(mode=mode),tempfile.TemporaryDirectory() as d:
                out=pathlib.Path(d);args=types.SimpleNamespace(image=str(out/'shot.png'),lane='js-date',size='800x600',window='456',pid=123,source_commit='b'*40,fixture='actions',manifest=str(out/'proof.window.manifest.json'))
                after=copy.deepcopy(observed)
                if mode=='moved':after['geometry'][0]=1
                if mode=='pid':after['pid']=124
                pixels=raw if mode!='cropped' else raw[:16]+struct.pack('>II',799,600)
                with mock.patch.object(proof,'exact_path',side_effect=lambda path,lane:path),mock.patch.object(proof,'snapshot',side_effect=[observed,after]),mock.patch.object(proof.subprocess,'run',side_effect=lambda command,check:pathlib.Path(command[-1]).write_bytes(pixels)):
                    with self.assertRaises(ValueError):proof.capture(args)
                self.assertFalse(pathlib.Path(args.manifest).exists())

    def test_explicit_transfer_publishes_all_ten_without_stdin_consumption(self):
        import subprocess, json
        scripts=pathlib.Path(__file__).parent
        manifest,raw=self.fixture()
        manifest['images']=[copy.deepcopy(manifest['images'][0]) for _ in range(10)]
        for n,image in enumerate(manifest['images']):image['path']=image['path'].replace('run.png',f'shot-{n}.png')
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d);target=root/'scripts';target.mkdir();fake=root/'bin';fake.mkdir();server=root/'server';server.mkdir()
            subprocess.run(['git','init','-q',d],check=True)
            for name in ('lane-fetch-window.sh','window-proof.py'):(target/name).write_bytes((scripts/name).read_bytes())
            for image in manifest['images']:(server/pathlib.Path(image['path']).name).write_bytes(raw)
            (server/'run.window.manifest.json').write_text(json.dumps(manifest))
            (target/'linux-run.sh').write_text('#!/bin/bash\necho fixture-host\n')
            (target/'lane-fetch.sh').write_text('#!/bin/bash\ncp "'+str(server/'run.window.manifest.json')+'" "$2/$(basename "$3")"\n')
            (fake/'ssh').write_text("""#!/usr/bin/env python3
import sys,hashlib,pathlib,shlex
assert not sys.stdin.read(), 'SSH consumed remaining manifest image list'
p=shlex.split(sys.argv[-1])[2]
f=pathlib.Path(SERVER_ROOT)/pathlib.Path(p).name
print(hashlib.sha256(f.read_bytes()).hexdigest()+'  '+p)
print(f.stat().st_size)
""".replace('SERVER_ROOT',repr(str(server))))
            (fake/'scp').write_text('#!/usr/bin/env python3\nimport sys,pathlib,shutil\nassert not sys.stdin.read(), "SCP consumed manifest image list"\nsrc=pathlib.Path('+repr(str(server))+')/pathlib.Path(sys.argv[-2]).name\nshutil.copyfile(src,sys.argv[-1])\n')
            for tool in fake.iterdir():tool.chmod(0o755)
            env=__import__('os').environ.copy();env['PATH']=str(fake)+':'+env['PATH']
            dest=root/'received'
            result=subprocess.run(['bash',str(target/'lane-fetch-window.sh'),'js-date',str(dest),'amazing/out/js-date/captures/run/run.window.manifest.json'],env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(len(list(dest.glob('*.png'))),10)
            for image in manifest['images']:proof.verify_image(image,dest/pathlib.Path(image['path']).name)

    def test_capture_assembly_preserves_remote_variables_on_host_bash(self):
        import subprocess
        scripts=pathlib.Path(__file__).parent
        with tempfile.TemporaryDirectory() as d:
            root=pathlib.Path(d); target=root/'scripts';target.mkdir()
            subprocess.run(['git','init','-q',str(root)],check=True)
            for name in ('linux-shell-capture.sh','linux-shell-capture-remote.sh','window-proof.py','linux-capture-session.py'):
                (target/name).write_bytes((scripts/name).read_bytes())
            (root/'fixture').write_text('source')
            subprocess.run(['git','-C',d,'add','.'],check=True)
            subprocess.run(['git','-C',d,'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-qm','fixture'],check=True)
            # No network or engine: the wrapper only saves its command argument.
            (target/'linux-run.sh').write_text('#!/bin/bash\nprintf "%s" "${@: -1}" > "'+str(root/'command')+'"\nexit 37\n')
            (target/'lane-fetch-window.sh').write_text('#!/bin/bash\nexit 0\n')
            result=subprocess.run(['bash',str(target/'linux-shell-capture.sh'),'js-date','--ref','HEAD','--actions','actions','--','--url','file:///a b.html'],capture_output=True,text=True)
            self.assertEqual(result.returncode,37,result.stderr)
            command=(root/'command').read_text()
            self.assertIn('case "$verb"',command)
            self.assertIn("'file:///a b.html'",command)

    def test_gate_fetch_remains_compact_only_and_all_capture_modes_use_window_proof(self):
        scripts=pathlib.Path(__file__).parent
        capture=(scripts/'linux-shell-capture.sh').read_text()
        body=(scripts/'linux-shell-capture-remote.sh').read_text()
        import subprocess
        self.assertEqual(subprocess.run(['bash','-n'],input=body,text=True,capture_output=True).returncode,0)
        self.assertIn('lane-fetch-window.sh',capture)
        self.assertNotIn('lane-fetch.sh',capture)
        self.assertNotIn('*.png',capture)
        self.assertIn('shot "$slug" "$url"',body)
        self.assertIn('shot "$NAME-$rest"',body)
        self.assertIn('shot "$NAME"',body)
        # Existing gate interface rejects image inputs before any remote call.
        import subprocess
        result=subprocess.run(['bash',str(scripts/'lane-fetch.sh'),'js-date','/tmp/unused-window-proof','amazing/out/js-date/x.png'],capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('compact',result.stderr)

if __name__=='__main__':unittest.main()
