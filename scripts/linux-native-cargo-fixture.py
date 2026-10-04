#!/usr/bin/env python3
"""Qualify native registry placement with actual Opus/OpenH264 builds on Linux."""
import hashlib,json,os,pathlib,subprocess,sys,tempfile

def main():
    native=pathlib.Path(tempfile.mkdtemp(prefix='amazing-registry-fixture-'))
    out=pathlib.Path(os.environ['OUT'])/'native-registry-fixture'
    out.mkdir(exist_ok=False)
    cargo=pathlib.Path(os.environ['CARGO_HOME']).resolve()
    summary={'schema':1,'source_commit':os.environ['COMMIT'],'cargo_home':str(cargo),'phases':{},'registry_manifests':[]}
    result=1
    try:
        assert sys.platform=='linux' and cargo.is_relative_to('/tmp')
        assert cargo.stat().st_dev==pathlib.Path('/tmp').stat().st_dev
        # Preserve the configured native library discovery environment. Record
        # presence only; credentials and arbitrary environment values stay private.
        summary['linux_environment_present']={k:k in os.environ for k in ('CMAKE','VPX_LIB_DIR','VPX_INCLUDE_DIR','VPX_VERSION','PKG_CONFIG','PKG_CONFIG_PATH')}
        summary['macos_linker_flag_in_environment']=any('dynamic_lookup' in os.environ.get(k,'') for k in ('CFLAGS','CXXFLAGS','LDFLAGS','RUSTFLAGS'))
        assert not summary['macos_linker_flag_in_environment']
        env=dict(os.environ,CARGO_TARGET_DIR=str(native/'target'))
        commands=[('public-tooling',['python3','scripts/public-gate-tests.py']),('metadata',['cargo','metadata','--locked','--format-version','1']),('native-libraries',['cargo','build','--locked','-p','openh264-sys2','-p','libopus_sys','--verbose'])]
        for phase,argv in commands:
            log=native/(phase+'.log')
            if phase=='metadata':
                errors=native/'metadata.stderr'
                with log.open('wb') as stream,errors.open('wb') as error_stream:
                    code=subprocess.run(argv,env=env,stdout=stream,stderr=error_stream,timeout=1800).returncode
                summary['metadata_stderr']={'path':str(errors),'bytes':errors.stat().st_size,'sha256':hashlib.file_digest(errors.open('rb'),'sha256').hexdigest()}
            else:
                with log.open('wb') as stream:code=subprocess.run(argv,env=env,stdout=stream,stderr=subprocess.STDOUT,timeout=1800).returncode
            row={'actual_exit':code,'raw':{'path':str(log),'bytes':log.stat().st_size,'sha256':hashlib.file_digest(log.open('rb'),'sha256').hexdigest()}}
            summary['phases'][phase]=row
            if code:raise RuntimeError(phase+' failed')
            if phase=='metadata':
                data=json.loads(log.read_bytes())
                for package in data['packages']:
                    if package.get('source','') and package['source'].startswith('registry+'):
                        p=pathlib.Path(package['manifest_path']).resolve()
                        assert p.is_relative_to(cargo/'registry'/'src'),str(p)
                        summary['registry_manifests'].append(str(p))
                assert any('/openh264-sys2-0.9.8/' in p for p in summary['registry_manifests'])
                assert any('/libopus_sys-0.3.3/' in p for p in summary['registry_manifests'])
        result=0
    except Exception as error:
        summary['error']={'type':type(error).__name__,'message':str(error)[:1000]}
    summary['actual_exit']=result
    text=json.dumps(summary,separators=(',',':'))+'\n'
    (native/'fixture.manifest.json').write_text(text)
    (out/'fixture.manifest.json').write_text(text)
    print(text,flush=True)
    return result

if __name__=='__main__':raise SystemExit(main())
