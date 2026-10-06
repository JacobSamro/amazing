#!/usr/bin/env python3
"""Pinned loopback fixture: delayed documents, resources, scripts, fonts and failures."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import time
from urllib.parse import urlsplit

class Fixture(ThreadingHTTPServer):
    daemon_threads = False
    def __init__(self, address, output):
        self.output = Path(output)
        self.output.touch(exist_ok=False)
        self.lock = threading.Lock()
        self.releases = set()
        super().__init__(address, Handler)
    def record(self, **event):
        with self.lock:
            with self.output.open('a') as stream:
                stream.write(json.dumps(event, separators=(',', ':')) + '\n')

class Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *_): pass
    def do_GET(self):
        path = urlsplit(self.path).path
        self.server.record(event='request', path=path, cookie=self.headers.get('Cookie', ''))
        if path.startswith('/release/'):
            with self.server.lock: self.server.releases.add(path.rsplit('/', 1)[1])
            self.respond('text/plain', b'released'); return
        if path.startswith('/slow-'):
            token = path.split('/', 2)[1]
            deadline = time.monotonic() + 20
            while token not in self.server.releases and time.monotonic() < deadline:
                # Browser cancellation closes the real connection during the barrier.
                import select
                readable, _, _ = select.select([self.connection], [], [], .02)
                if readable:
                    import socket
                    if not self.connection.recv(1, socket.MSG_PEEK):
                        self.server.record(event='cancelled', path=path); return
            if token not in self.server.releases:
                self.server.record(event='barrier-timeout', path=path); self.close_connection=True; return
        if path == '/failure': self.close_connection = True; return
        if path == '/redirect':
            self.respond('text/plain', b'', 302, [('Location', '/normal')]); return
        styles = "body{margin:24px;background:#f8fafc;color:#123456}h1{font-size:32px}p{font-size:18px}"
        routes = {
            '/normal': ('text/html', '<title>Normal</title><h1>Normal page</h1><p>Ready to browse</p>'),
            '/slow-document/page': ('text/html', '<title>Slow document</title><h1>Slow document</h1>'),
            '/resource': ('text/html', '<title>Resources</title><link rel=stylesheet href=/slow-style/page.css><h1>Resources ready</h1><img src=/slow-image/pixel.ppm>'),
            '/slow-style/page.css': ('text/css', styles),
            '/slow-image/pixel.ppm': ('image/x-portable-pixmap', 'P3\n1 1\n255\n20 40 60\n'),
            '/script': ('text/html', '<title>Before script</title><h1 id=message>Before script</h1><img id=dynamic><script src=/slow-script/page.js></script>'),
            '/slow-script/page.js': ('text/javascript', "var n=Number(localStorage.getItem('runs')||'0')+1;localStorage.setItem('runs',String(n));document.title='Once '+n;document.querySelector('#message').textContent='Executed once';document.querySelector('#dynamic').setAttribute('src','/slow-mutated/pixel.ppm');"),
            '/slow-mutated/pixel.ppm': ('image/x-portable-pixmap', 'P3\n1 1\n255\n20 40 60\n'),
            '/font': ('text/html', '<title>Fonts</title><link rel=stylesheet href=/slow-font-style/page.css><h1>Fonts ready</h1>'),
            '/slow-font-style/page.css': ('text/css', "@import url('/slow-import/page.css');@font-face{font-family:NavProof;src:url('/slow-font/font.ttf')}h1{font-family:NavProof}"),
            '/slow-import/page.css': ('text/css', styles),
        }
        if path.startswith('/font-continuation/'):
            case=path.rsplit('/',1)[1]
            assert case in ('complete','stop','supersede','close')
            body=f"<title>Continuation initial</title><style>@font-face{{font-family:NavProof;src:url('/slow-cont-font-{case}/font.ttf');font-display:swap}}h1{{font-family:NavProof}}img{{width:20px;height:20px}}</style><h1>Continuation ready</h1><img id=late><script>var n=Number(localStorage.getItem('continuationRuns')||'0')+1;localStorage.setItem('continuationRuns',String(n));setTimeout(function(){{var m=Number(localStorage.getItem('continuationCallbacks')||'0')+1;localStorage.setItem('continuationCallbacks',String(m));document.querySelector('#late').setAttribute('src','/slow-cont-image-{case}/pixel.ppm');document.title='Continuation '+n+' callback '+m;}},20);</script>"
            self.respond('text/html', ('<!doctype html><style>'+styles+'</style>'+body).encode());return
        if path.startswith('/slow-cont-image-'):
            self.respond('image/x-portable-pixmap',b'P3\n1 1\n255\n255 0 0\n');return
        if path.startswith('/font-mode/'):
            mode=path.rsplit('/',1)[1]
            assert mode in ('block','swap','fallback','optional')
            body=f'<title>Fonts {mode}</title><style>@font-face{{font-family:NavProof;src:url(/slow-{mode}/font.ttf);font-display:{mode}}}h1{{font-family:NavProof}}</style><h1>Fonts {mode}</h1>'
            self.respond('text/html', ('<!doctype html><style>'+styles+'</style>'+body).encode());return
        if path == '/slow-font/font.ttf' or (path.endswith('/font.ttf') and (path.split('/')[1] in ('slow-block','slow-swap','slow-fallback','slow-optional') or path.startswith('/slow-cont-font-'))):
            # Native template's real font bytes, fetched through the CORS font service.
            font=Path('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf')
            self.respond('font/ttf', font.read_bytes(), headers=[('Access-Control-Allow-Origin','*')]); return
        kind, body = routes.get(path, ('text/html', '<title>Missing</title><h1>Missing fixture route</h1>'))
        if kind == 'text/html': body='<!doctype html><html><head><style>'+styles+'</style></head><body>'+body+'</body></html>'
        self.respond(kind, body.encode())
    def respond(self, kind, body, status=200, headers=()):
        self.send_response(status)
        self.send_header('Content-Type', kind); self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        for name,value in headers:self.send_header(name,value)
        self.end_headers()
        try:
            self.wfile.write(body); self.wfile.flush()
            self.server.record(event='response', path=urlsplit(self.path).path, status=status)
        except (BrokenPipeError, ConnectionResetError): self.server.record(event='cancelled',path=urlsplit(self.path).path)
        self.close_connection=True

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--port',type=int,default=8087);parser.add_argument('--output',required=True);parser.add_argument('--ready',required=True)
    args=parser.parse_args();server=Fixture(('127.0.0.1',args.port),args.output)
    Path(args.ready).write_text(json.dumps({'url':'http://127.0.0.1:'+str(server.server_port)}))
    server.serve_forever(poll_interval=.05)
