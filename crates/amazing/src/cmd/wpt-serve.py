#!/usr/bin/env python3
"""Run the pinned WPT checkout's own wptserve for xhr/ and fetch/ measurements.

Test tooling only. The pinned `tools/wptserve` package executes the unmodified
upstream `.py` handlers, `.sub.` templates, `?pipe=` chains, `.asis` and
`.headers` files, so response semantics are upstream's, not re-derived. The
HTTP/2 and WebSocket stacks wptserve imports at module load are not used by an
HTTP/1.1 server and are replaced with inert stubs when not installed.

Prints `ADDR host:port` once listening. Exits when stdin closes.
"""
import argparse
import importlib.abc
import importlib.machinery
import os
import sys
import threading
import types
from pathlib import Path

STUBBED = ('h2', 'hpack', 'hyperframe', 'pywebsocket3')


class _Anything(types.ModuleType):
    def __getattr__(self, name):
        if name.startswith('__'):
            raise AttributeError(name)
        value = type(name, (), {})
        setattr(self, name, value)
        return value


class _StubFinder(importlib.abc.MetaPathFinder, importlib.abc.Loader):
    def find_spec(self, name, path, target=None):
        if name.split('.')[0] in STUBBED:
            return importlib.machinery.ModuleSpec(name, self, is_package=True)
    def create_module(self, spec):
        module = _Anything(spec.name)
        module.__path__ = []
        return module
    def exec_module(self, module):
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--wpt-root', type=Path, required=True)
    ap.add_argument('--host', default='web-platform.test')
    args = ap.parse_args()
    if os.environ.get('WPT_SERVE_TRACE'):
        import logging
        logging.basicConfig(level=logging.DEBUG, stream=sys.stderr, format='%(name)s %(message)s')
    root = args.wpt_root.resolve()
    sys.path.insert(0, str(root / 'tools' / 'wptserve'))
    try:
        import h2, pywebsocket3  # noqa: F401
    except ImportError:
        sys.meta_path.append(_StubFinder())
    from wptserve import server, routes, stash
    from wptserve.config import ConfigBuilder

    ports = {'http': [80, 8001], 'https': [443, 8443], 'ws': [80], 'wss': [443],
             'h2': [443], 'http-local': [80], 'http-public': [80],
             'https-local': [443], 'https-public': [443]}
    builder = ConfigBuilder(
        None, browser_host=args.host, doc_root=str(root), server_host='127.0.0.1',
        ports=ports, subdomains={'www', 'www1', 'www2', 'xn--n8j6ds53lwwkrqhv28a', 'xn--lve-6lad'},
        not_subdomains={'nonexistent-origin', 'nonexistent'}, check_subdomains=False,
        alternate_hosts={'alt': 'not-' + args.host})
    with stash.StashServer(), builder as config:
        httpd = server.WebTestHttpd(host='127.0.0.1', port=0, doc_root=str(root),
                                    routes=routes.routes, config=config)
        httpd.start()
        print('ADDR 127.0.0.1:%d' % httpd.port, flush=True)
        sys.stdin.read()
        httpd.stop()


if __name__ == '__main__':
    main()
