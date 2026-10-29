#!/usr/bin/python3
"""Root-only NAS account verification; signed remember tokens stay password-bound."""
import json
import os
from pathlib import Path
import sys
from remember import RememberAuth

try:
    if os.geteuid()!=0:raise ValueError('Root required')
    request=json.loads(sys.stdin.buffer.read(8193))
    if not isinstance(request,dict):raise ValueError('Invalid request')
    state=Path(os.environ['TRIM_PKGVAR'])/'auth-secret'
    response=RememberAuth(state,'wlmb-fnos').authenticate(request)
except (KeyError,ValueError,OSError,TypeError):response={'ok':False}
print(json.dumps(response))
sys.exit(0 if response['ok'] else 1)
