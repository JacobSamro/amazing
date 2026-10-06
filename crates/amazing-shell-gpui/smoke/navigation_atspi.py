#!/usr/bin/env python3
"""Read the genuine live shell's AT-SPI tree; native Linux fixture control only."""
import argparse,hashlib,json,os,time
from pathlib import Path
import gi
gi.require_version('Atspi','2.0')
from gi.repository import Atspi

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--pid',type=int,required=True);parser.add_argument('--out',required=True);parser.add_argument('--expected-document',choices=('0','1'),default='1');parser.add_argument('--expect-text',default='')
    args=parser.parse_args();deadline=time.monotonic()+12;last=[]
    while time.monotonic()<deadline:
        os.kill(args.pid,0)
        desktop=Atspi.get_desktop(0)
        for index in range(desktop.get_child_count()):
            app=desktop.get_child_at_index(index)
            if app.get_process_id()!=args.pid:continue
            nodes=[];queue=[(app,0)];truncated=False
            while queue:
                node,depth=queue.pop(0)
                if len(nodes)>=512 or depth>32:truncated=True;break
                name=node.get_name();role=node.get_role_name();count=node.get_child_count()
                nodes.append({'name':name,'role':role,'children':count,'depth':depth})
                for child in range(count):queue.append((node.get_child_at_index(child),depth+1))
            last=nodes
            # Main's live document bridge projects real web document and heading nodes.
            # Checking the target PID prevents a desktop/portal tree from satisfying this.
            document=any('document' in n['role'].lower() for n in nodes)
            content=any(n['role'].lower() in ('heading','link') and n['name'] for n in nodes)
            window=any(n['role'].lower() in ('frame','window') for n in nodes)
            expected_text=not args.expect_text or any(args.expect_text in n['name'] for n in nodes)
            if window and not truncated and expected_text and (args.expected_document=='0' or (document and content)):
                out=Path(args.out);out.write_text(json.dumps({'kind':'actual-live-shell-atspi','pid':args.pid,'expected_text':args.expect_text,'expected_text_matched':expected_text,'expected_document':args.expected_document=='1','document':document,'named_web_content':content,'nodes':nodes},separators=(',',':'))+'\n')
                print(json.dumps({'pid':args.pid,'nodes':len(nodes),'expected_text':args.expect_text,'expected_text_matched':expected_text,'expected_document':args.expected_document=='1','document':document,'named_web_content':content,'path':str(out),'sha256':hashlib.sha256(out.read_bytes()).hexdigest()},separators=(',',':')))
                return
        time.sleep(.1)
    raise RuntimeError('target shell never exposed the live web accessibility tree: '+str(last[:16]))

if __name__=='__main__':main()
