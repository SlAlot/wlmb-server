#!/usr/bin/python3
"""Local administrator verification; invoked only through the private backend."""
import crypt
import grp
import hmac
import json
import os
import pwd
import re
import spwd
import sys

try:
    assert os.geteuid() == 0
    request = json.loads(sys.stdin.buffer.read(8193))
    username, password = request.get('username'), request.get('password')
    assert isinstance(username, str) and re.fullmatch(r'[A-Za-z0-9_.-]{1,32}', username)
    assert isinstance(password, str) and 1 <= len(password) <= 256
    account = pwd.getpwnam(username)
    groups = set(os.getgrouplist(username, account.pw_gid))
    administrators = {g.gr_gid for g in grp.getgrall() if g.gr_name in ('Administrators', 'sudo')}
    assert account.pw_uid == 0 or groups & administrators
    hashed = spwd.getspnam(username).sp_pwdp
    assert hashed and not hashed.startswith(('!', '*'))
    assert hmac.compare_digest(crypt.crypt(password, hashed) or '', hashed)
    print('{"ok":true}')
except (AssertionError, KeyError, ValueError, OSError, TypeError):
    print('{"ok":false}')
    sys.exit(1)
