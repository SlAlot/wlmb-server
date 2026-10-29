#!/usr/bin/env python3
"""Exercise the public fnOS web boundary without a modem or administrator secret."""
import http.client
from http.server import ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import threading
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('fnos_web', ROOT/'fnos/app/server.py')
WEB = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WEB)


class FnosWebTests(unittest.TestCase):
    def setUp(self):
        WEB.SESSIONS.clear(); WEB.ATTEMPTS.clear()
        self.server=ThreadingHTTPServer(('127.0.0.1',0),WEB.Handler)
        self.worker=threading.Thread(target=self.server.serve_forever,daemon=True);self.worker.start()
        self.addCleanup(self.server.server_close);self.addCleanup(self.server.shutdown)

    def request(self,path,value=None,headers=None):
        conn=http.client.HTTPConnection(*self.server.server_address,timeout=3)
        self.addCleanup(conn.close)
        conn.request('POST' if value is not None else 'GET',path,
                     json.dumps(value).encode() if value is not None else None,
                     {'Content-Type':'application/json','X-WLMB-Request':'1',**(headers or {})})
        response=conn.getresponse();return response.status,dict(response.getheaders()),response.read()

    def test_forged_nas_headers_do_not_authorize_signal_access(self):
        with patch.object(WEB,'backend',side_effect=AssertionError('Must not reach privileged backend')):
            status,_,_=self.request('/api',{'operation':'status'},{'X-Trim-Isadmin':'true','X-Trim-Userid':'1000'})
        self.assertEqual(status,401)

    def test_validated_login_only_exposes_read_operations(self):
        with patch.object(WEB,'backend',return_value={'ok':True,'data':{'verified':True}}):
            status,headers,_=self.request('/login',{'username':'test-admin','password':'fixture-only'})
        self.assertEqual(status,200)
        cookie=headers['Set-Cookie'];self.assertIn('HttpOnly',cookie);self.assertIn('SameSite=Strict',cookie)
        with patch.object(WEB,'backend',return_value={'ok':True,'data':{}}) as remote:
            self.assertEqual(self.request('/api',{'operation':'status'},{'Cookie':cookie})[0],200)
            self.assertEqual(self.request('/api',{'operation':'at_execute','command':'AT+CFUN=0'},{'Cookie':cookie})[0],404)
            self.assertEqual(remote.call_count,1)

    def test_cross_origin_and_path_traversal_are_rejected(self):
        self.assertEqual(self.request('/login',{'username':'fixture'},{'Origin':'https://unrelated.invalid'})[0],403)
        self.assertEqual(self.request('/../auth.py')[0],404)
        self.assertEqual(self.request('/auth.py')[0],404)


if __name__=='__main__':unittest.main()
