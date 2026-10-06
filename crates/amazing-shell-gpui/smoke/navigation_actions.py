#!/usr/bin/env python3
"""Drive the genuine Amazing window; assertions consume UI state and actual server events."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

PREFIX='AMAZING_NAVIGATION_STATE '
def state(log):
    states=[line[len(PREFIX):].rstrip('\r\n') for line in log.read_text(errors='replace').splitlines(keepends=True) if line.endswith('\n') and line.startswith(PREFIX)]
    return json.loads(states[-1]) if states else None

def run(args):
    out=Path(args.out);out.mkdir(parents=True,exist_ok=False)
    log=Path(args.log);fixture=Path(args.fixture_log)
    results={}; identity='suite:startup'; manifest=out/'window.manifest.json';window_size='1280x800';display_size='1280x800'
    def await_condition(identity, predicate):
        end=time.monotonic()+8
        while time.monotonic()<end:
            current=state(log)
            if current and predicate(current):
                if identity=='font-ready':
                    assert 'AMAZING_FONT_REGISTERED family="NavProof"' in log.read_text(), 'real font must decode and register before ready'
                results[identity]='pass';return current
            time.sleep(.025)
        results[identity]='fail'
        raise AssertionError('actual window condition failed: '+identity+'; state='+str(state(log)))
    def xdo(*command):
        if command and command[0] in ('key','type','click'):
            before=subprocess.check_output(['xdotool','getwindowfocus'],text=True).strip()
            if before!=args.window:
                subprocess.run(['xdotool','windowfocus','--sync',args.window],check=True,timeout=10)
            after=subprocess.check_output(['xdotool','getwindowfocus'],text=True).strip()
            assert after==args.window, 'actual keyboard focus is outside target window'
            with (out/'input.jsonl').open('a') as stream:
                stream.write(json.dumps({'command':command[0],'before_window':before,'after_window':after})+'\n')
        subprocess.run(['xdotool',*command],check=True,timeout=10)
    def shot(name, filename):
        geometry=subprocess.check_output(['xdotool','getwindowgeometry','--shell',args.window],text=True)
        dimensions=dict(line.split('=',1) for line in geometry.splitlines() if '=' in line)
        assert [int(dimensions['WIDTH']),int(dimensions['HEIGHT'])]==[int(n) for n in window_size.split('x')], 'actual window did not apply requested resize'
        group=Path(filename).stem
        expected_text=''
        if group=='product-smoke' and name!='09-tab-closed':
            expected_text='The web, from first principles.' if name in ('00-launch','01-address-bar-focused','03-navigated-back') else 'Search the web'
        elif group=='viewport-resize':expected_text='The web, from first principles.'
        elif group=='navigation':expected_text={'01-delayed-document':'The web, from first principles.','02-other-tab-usable':'Normal page','03-author-once':'Executed once','03a-font-block':'Fonts ready','03b-font-fallback':'Fonts ready','03c-font-ready':'Fonts ready','03d-toolbar-loading':'Fonts block','03e-toolbar-stopped':'Fonts block','04-final-normal':'Normal page'}[name]
        elif group=='continuation':expected_text='Normal page' if name=='04-after-closed-normal' else 'Continuation ready'
        elif group=='font-display':expected_text={'01-font-block':'Fonts block','02-font-block-fallback':'Fonts block','03-font-block-loaded':'Fonts block','04-font-swap':'Fonts swap','05-font-fallback-expired':'Fonts fallback','06-font-fallback-late':'Fonts fallback','07-font-optional-expired':'Fonts optional','08-font-optional-late':'Fonts optional'}[name]
        # AT names can already be current while a resize reflow is still pending.
        # Wait for the exact measured physical window before the public proof checks it.
        end=time.monotonic()+30
        wanted=[int(n) for n in window_size.split('x')]
        while time.monotonic()<end:
            records=[line for line in log.read_text(errors='strict').splitlines() if line.startswith(('AMAZING_VIEWPORT ','AMAZING_VIEWPORT_FAILURE '))]
            if records and records[-1].startswith('AMAZING_VIEWPORT '):
                fields=dict(word.split('=',1) for word in records[-1].split()[1:])
                measured=[round(float(fields['window_css_width'])*float(fields['device_scale'])),round(float(fields['window_css_height'])*float(fields['device_scale']))]
                if int(fields['pid'])==int(args.pid) and float(fields['device_scale'])==float(args.scale) and measured==wanted:break
            os.kill(int(args.pid),0);time.sleep(.025)
        else:raise RuntimeError('actual measured frame never reached requested window geometry')
        atspi=out/(name+'.atspi.json')
        subprocess.run(['/usr/bin/python3','crates/amazing-shell-gpui/smoke/navigation_atspi.py',
            '--pid',args.pid,'--out',str(atspi),'--expected-document',
            '0' if group=='product-smoke' and name=='09-tab-closed' else '1','--expect-text',expected_text],check=True,timeout=15)
        results['atspi:'+name]='pass'
        subprocess.run(['python3',args.proof,'capture','--lane',args.lane,'--manifest',str(manifest),'--image',str(out/(name+'.png')),
            '--native-output-root',args.native_output_root,'--exe',args.exe,'--window',args.window,'--pid',args.pid,'--source-commit',args.source,'--fixture',str(filename)+' action='+name,
            '--size',display_size,'--engine-log',str(log),'--device-scale',args.scale],check=True,timeout=15)
        if group=='font-display' or name in ('03a-font-block','03b-font-fallback','03c-font-ready','03d-toolbar-loading','03e-toolbar-stopped'):
            scale=float(args.scale);x=round(20*scale);y=round(150*scale)
            # The first glyphs lie in the left web-content region. At DPR2 the
            # native engine badge overlaps the former 500 CSS-pixel crop.
            width=min(round(250*scale),int(dimensions['WIDTH'])-x);height=round(64*scale)
            pixels=subprocess.check_output(['convert',str(out/(name+'.png')),'-crop',f'{width}x{height}+{x}+{y}','+repage','-depth','8','rgb:-'],timeout=15)
            assert len(pixels)==width*height*3, 'actual native ink crop size mismatch'
            ink=sum(1 for offset in range(0,len(pixels),3) if pixels[offset]<100 and pixels[offset+1]<130 and pixels[offset+2]<150 and pixels[offset+1]>pixels[offset]+10 and pixels[offset+2]>pixels[offset+1])
            invisible=name in ('03a-font-block','01-font-block','03d-toolbar-loading')
            assert (ink==0 if invisible else ink>20), 'actual target window font ink disagrees with native font-display phase'
            (out/(name+'.ink.json')).write_text(json.dumps({'source':args.source,'pid':args.pid,'scale':scale,'image_sha256':hashlib.sha256((out/(name+'.png')).read_bytes()).hexdigest(),'crop':[x,y,width,height],'navy_ink_pixels':ink,'expected_invisible':invisible,'typed_state':state(log)},separators=(',',':'))+'\n')
            results['font-ink:'+name]='pass'
        if group=='continuation' and name in ('01-image-before-font','02-stopped-continuation'):
            scale=float(args.scale);x=round(20*scale);y=round(150*scale)
            width=min(round(500*scale),int(dimensions['WIDTH'])-x);height=min(round(350*scale),int(dimensions['HEIGHT'])-y)
            pixels=subprocess.check_output(['convert',str(out/(name+'.png')),'-crop',f'{width}x{height}+{x}+{y}','+repage','-depth','8','rgb:-'],timeout=15)
            assert len(pixels)==width*height*3, 'actual continuation crop size mismatch'
            red=sum(1 for offset in range(0,len(pixels),3) if pixels[offset]>200 and pixels[offset+1]<50 and pixels[offset+2]<50)
            loaded=name=='01-image-before-font'
            assert (red>100 if loaded else red==0), 'actual target image ink disagrees with retained resource state'
            (out/(name+'.ink.json')).write_text(json.dumps({'source':args.source,'pid':args.pid,'scale':scale,'image_sha256':hashlib.sha256((out/(name+'.png')).read_bytes()).hexdigest(),'crop':[x,y,width,height],'red_image_pixels':red,'expected_loaded':loaded,'typed_state':state(log)},separators=(',',':'))+'\n')
            results['continuation-ink:'+name]='pass'


    original_oracles={
        '02-omnibox-navigated':lambda s:s['committed']=='amazing://newtab',
        '03-navigated-back':lambda s:s['committed']=='amazing://status',
        '04-navigated-forward':lambda s:s['committed']=='amazing://newtab',
        '05-find-in-page-opened':lambda s:s['find_open'],
        '06-find-in-page-query-typed':lambda s:s['find_query']=='Amazing',
        '07-find-in-page-closed':lambda s:not s['find_open'],
        '08-new-tab-opened':lambda s:s['active']==6 and s['tabs'][6]['open'],
        '09-tab-closed':lambda s:not s['tabs'][6]['open'],
    }
    try:
        for filename in args.actions:
            for number,line in enumerate(Path(filename).read_text().splitlines(),1):
                line=line.strip()
                if not line or line.startswith('#'):continue
                verb,_,rest=line.partition(' ')
                identity=Path(filename).stem+':'+str(number)+':'+verb
                if verb=='nav':
                    xdo('key','ctrl+l');xdo('type','--delay','1','--',args.url+rest);xdo('key','Return')
                elif verb=='key':xdo('key',rest)
                elif verb=='type':xdo('type','--delay','10','--',rest)
                elif verb=='wait':time.sleep(float(rest))
                elif verb=='resize':
                    width,height=rest.split('x');xdo('windowsize','--sync',args.window,width,height);window_size=rest
                elif verb in ('click','clickcss'):
                    x,y=rest.split()
                    if verb=='clickcss': x,y=(str(round(float(value)*float(args.scale))) for value in (x,y))
                    xdo('mousemove','--window',args.window,x,y)
                    geometry=dict(line.split('=',1) for line in subprocess.check_output(['xdotool','getwindowgeometry','--shell',args.window],text=True).splitlines() if '=' in line)
                    location=dict(line.split('=',1) for line in subprocess.check_output(['xdotool','getmouselocation','--shell'],text=True).splitlines() if '=' in line)
                    assert [int(location['X']),int(location['Y'])]==[int(geometry['X'])+int(x),int(geometry['Y'])+int(y)], 'actual pointer did not reach requested window location'
                    xdo('click','1')
                elif verb=='expect':
                    name,_,expression=rest.partition(' ')
                    await_condition(name,lambda s:bool(eval(expression,{'__builtins__':{}},{'s':s,'t':s['tabs'][s['active']]})))
                elif verb in ('requested','cancelled'):
                    pieces=rest.split();path=pieces[0];count=int(pieces[1]) if len(pieces)>1 else 1
                    event='request' if verb=='requested' else 'cancelled'
                    await_condition(identity,lambda _:sum(record.get('event')==event and record.get('path')==path
                        for record in [json.loads(line) for line in (fixture.read_text().splitlines(keepends=True) if fixture.exists() else []) if line.endswith('\n')])>=count)
                elif verb=='release':
                    with urllib.request.urlopen(args.url+'/release/'+rest,timeout=3) as response:response.read()
                elif verb=='shot':
                    if not args.baseline and rest in original_oracles:await_condition('original:'+rest,original_oracles[rest])
                    shot(rest,filename)
                else:raise ValueError('unknown action '+verb)
                results[identity]='pass'
    except Exception:
        if identity not in results: results[identity]='fail'
        raise
    finally:
        raw={'source':args.source,'scale':args.scale,'results':results,'state':state(log)}
        (out/'results.json').write_text(json.dumps(raw,separators=(',',':'))+'\n')
        hashes={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in [log,fixture,out/'input.jsonl',*out.glob('*.atspi.json'),*out.glob('*.ink.json')] if path.exists()}
        compact={'source':args.source,'scale':args.scale,'passed':sum(value=='pass' for value in results.values()),
            'failed':sum(value=='fail' for value in results.values()),'classifications':results,'raw_sha256':hashes}
        (out/'compact.json').write_text(json.dumps(compact,separators=(',',':'))+'\n')
    if manifest.exists():subprocess.run(['python3',args.proof,'validate',str(manifest),args.lane],check=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser()
    for name in ('out','log','fixture-log','proof','lane','exe','window','pid','source','scale','url'):parser.add_argument('--'+name,required=True)
    parser.add_argument('--actions',action='append',required=True)
    parser.add_argument('--baseline',action='store_true');parser.add_argument('--native-output-root',required=True)
    run(parser.parse_args())
