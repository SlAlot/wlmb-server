"""Root-side administrator verification and password-bound remember tokens."""
import base64
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import time

LIFETIME = 30 * 86400


def account(username):
    import grp
    import pwd
    import spwd
    if not isinstance(username,str) or not re.fullmatch(r'[A-Za-z0-9_.-]{1,32}',username):
        raise ValueError('Invalid account')
    user = pwd.getpwnam(username)
    groups = set(os.getgrouplist(username,user.pw_gid))
    administrators = {g.gr_gid for g in grp.getgrall() if g.gr_name in ('Administrators','sudo')}
    hashed = spwd.getspnam(username).sp_pwdp
    if not (user.pw_uid==0 or groups & administrators) or not hashed or hashed.startswith(('!','*')):
        raise ValueError('Administrator required')
    return hashed


class RememberAuth:
    def __init__(self, path, audience, lookup=account):
        self.path,self.audience,self.lookup = Path(path),audience,lookup

    def key(self):
        if not self.path.exists():
            temporary=self.path.with_name('.auth-'+secrets.token_hex(12)+'.tmp')
            fd=os.open(temporary,os.O_WRONLY | os.O_CREAT | os.O_EXCL,0o600)
            try:
                with os.fdopen(fd,'wb') as stream:stream.write(secrets.token_bytes(32))
                try:os.link(temporary,self.path)
                except FileExistsError:pass
            finally:temporary.unlink(missing_ok=True)
        key = self.path.read_bytes()
        if len(key)!=32:raise ValueError('Invalid authentication key')
        return key

    def stamp(self, hashed):
        return hmac.new(self.key(),hashed.encode(),hashlib.sha256).hexdigest()

    def issue(self, username, hashed):
        value = {'user':username,'aud':self.audience,'exp':int(time.time())+LIFETIME,
                 'stamp':self.stamp(hashed),'nonce':secrets.token_hex(12)}
        data = base64.urlsafe_b64encode(json.dumps(value,separators=(',',':')).encode()).rstrip(b'=')
        signature = hmac.new(self.key(),data,hashlib.sha256).hexdigest()
        return data.decode()+'.'+signature

    def verify(self, token):
        if not isinstance(token,str) or not 1<=len(token)<=2048:raise ValueError('Invalid token')
        data,signature = token.rsplit('.',1)
        expected = hmac.new(self.key(),data.encode(),hashlib.sha256).hexdigest()
        if not hmac.compare_digest(signature,expected):raise ValueError('Invalid signature')
        value = json.loads(base64.urlsafe_b64decode(data+'='*(-len(data)%4)))
        if value['aud']!=self.audience or type(value['exp']) is not int or value['exp']<=time.time():
            raise ValueError('Expired token')
        hashed = self.lookup(value['user'])
        if not hmac.compare_digest(value['stamp'],self.stamp(hashed)):
            raise ValueError('Password changed')
        return value['user'],hashed

    def authenticate(self, request):
        try:
            username = request.get('username')
            if request.get('remember_token'):
                remembered,hashed = self.verify(request['remember_token'])
                if username and username!=remembered:raise ValueError('Account changed')
                username = remembered
            else:
                import crypt
                password = request.get('password')
                if not isinstance(password,str) or not 1<=len(password)<=256:raise ValueError('Invalid password')
                hashed = self.lookup(username)
                if not hmac.compare_digest(crypt.crypt(password,hashed) or '',hashed):raise ValueError('Invalid password')
            response = {'ok':True,'verified':True,'username':username}
            if request.get('remember') is True:
                response['remember_token'] = self.issue(username,hashed)
            return response
        except (KeyError,ValueError,TypeError,OSError):
            return {'ok':False}
