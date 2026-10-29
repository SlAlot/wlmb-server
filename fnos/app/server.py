#!/usr/bin/python3
"""Unprivileged fnOS web app; privileged radio work stays on a local socket."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from http.cookies import SimpleCookie
import ipaddress
import json
import mimetypes
from pathlib import Path
import secrets
import socket
import sys
import threading
import time
from urllib.parse import urlsplit
sys.path.insert(0,str(Path(__file__).resolve().parent))
from login import LoginMixin,LoginState

ROOT = Path(__file__).resolve().parent
SOCKET = Path(sys.argv[1]) if __name__ == '__main__' else Path('/nonexistent')
LOGIN = LoginState('wlmb_fnos','/')
SESSIONS, ATTEMPTS, LOCK = LOGIN.sessions,LOGIN.attempts,LOGIN.lock


def backend(request):
    with socket.socket(socket.AF_UNIX) as channel:
        channel.settimeout(12)
        channel.connect(str(SOCKET))
        channel.sendall(json.dumps(request).encode() + b'\n')
        chunks, size = [], 0
        while data := channel.recv(65536):
            size += len(data)
            if size > 262144:
                raise ValueError('Response too large')
            chunks.append(data)
        return json.loads(b''.join(chunks))


class Handler(LoginMixin,BaseHTTPRequestHandler):
    login_state = LOGIN
    def setup(self):
        super().setup();self.connection.settimeout(10)
    def log_message(self, *_):
        pass

    def reply(self, status, payload, mime='application/json', cookie=None, cookies=()):
        data = payload if isinstance(payload, bytes) else json.dumps(payload, ensure_ascii=False).encode()
        self.send_response(status)
        for key, value in {'Content-Type': mime + '; charset=utf-8', 'Content-Length': str(len(data)),
                           'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff',
                           'Referrer-Policy': 'same-origin'}.items():
            self.send_header(key, value)
        if cookie:
            self.send_header('Set-Cookie', cookie)
        for value in cookies:self.send_header('Set-Cookie',value)
        self.end_headers()
        self.wfile.write(data)

    def auth_peer(self):return self.client_address[0]

    def auth_secure(self):
        return ipaddress.ip_address(self.client_address[0]).is_loopback and self.headers.get('X-Forwarded-Proto')=='https'

    def auth_backend(self, request):
        value=backend(request)
        return dict(value.get('data',{}),ok=value.get('ok') is True)

    def auth_reply(self,status,value,cookies):
        if value.get('ok'):payload={'ok':True,'data':{k:v for k,v in value.items() if k!='ok'}}
        else:payload={'ok':False,'error':{'message':value.get('error','登录失败')}}
        self.reply(status,payload,cookies=cookies)

    def local(self):
        address = ipaddress.ip_address(self.client_address[0])
        return address.is_private or address.is_loopback

    def do_GET(self):
        if not self.local():
            self.reply(403, {'ok': False})
            return
        path = urlsplit(self.path).path
        if path=='/logo.png':
            self.reply(200,(ROOT/'ui/images/wlmb-logo-v2_256.png').read_bytes(),'image/png')
            return
        relative = 'index.html' if path == '/' else path.lstrip('/')
        allowed = {'index.html', 'app.js', 'fnos.css', 'help.html', 'panel.css',
                   'webui-data-1.1.3.js', 'webui-icons-1.1.3.js', 'webui-shell-1.1.3.js','login.js','login.css'}
        if relative not in allowed:
            self.reply(404, {'ok': False})
            return
        file = ROOT / 'www' / relative
        try:
            self.reply(200, file.read_bytes(), mimetypes.guess_type(file.name)[0] or 'application/octet-stream')
        except OSError:
            self.reply(404, {'ok': False})

    def do_POST(self):
        if not self.local() or self.headers.get('X-WLMB-Request') != '1':
            self.reply(403, {'ok': False})
            return
        origin = self.headers.get('Origin')
        if origin and urlsplit(origin).netloc != self.headers.get('Host'):
            self.reply(403, {'ok': False})
            return
        if self.headers.get('Content-Type', '').split(';')[0] != 'application/json':
            self.reply(415, {'ok': False})
            return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 8192:
                raise ValueError('Invalid request length')
            request = json.loads(self.rfile.read(length))
            if not isinstance(request, dict):
                raise ValueError('Invalid request')
            if self.path in {'/'+action for action in ('login','resume','forget','logout','login-state')}:
                self.login_action(self.path[1:],request)
                return
            if not self.admin():
                self.reply(401, {'ok': False, 'error': {'message': '请先验证飞牛管理员身份'}})
                return
            if self.path != '/api' or request.get('operation') not in ('status', 'settings', 'diagnostics', 'refresh', 'health'):
                self.reply(404, {'ok': False})
                return
            response = backend(request)
            self.reply(200 if response.get('ok') else response.get('status', 400), response)
        except (ValueError, OSError, socket.timeout):
            self.reply(503, {'ok': False, 'error': {'message': '读取服务暂不可用，请稍后重试'}})


if __name__ == '__main__':
    server = ThreadingHTTPServer(('0.0.0.0', int(sys.argv[2])), Handler)
    server.daemon_threads = True
    server.serve_forever()
