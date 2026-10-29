"""Shared HTTP login flow. Persistent cookies contain signed tokens, never passwords."""
from http.cookies import SimpleCookie,CookieError
import secrets
import threading
import time


class LoginState:
    def __init__(self, name, path):
        self.name,self.path = name,path
        self.sessions,self.attempts = {},{}
        self.lock = threading.Lock()


class LoginMixin:
    def cookie_value(self, name):
        try:return SimpleCookie(self.headers.get('Cookie',''))[name].value
        except (KeyError,ValueError,CookieError):return ''

    def login_cookie(self, name, token, age):
        secure = '; Secure' if self.auth_secure() else ''
        return name+'='+token+'; Path='+self.login_state.path+'; HttpOnly; SameSite=Strict; Max-Age='+str(age)+secure

    def admin(self):
        state = self.login_state
        with state.lock:
            value = state.sessions.get(self.cookie_value(state.name))
            return bool(value and value[0]==self.auth_peer() and value[1]>time.monotonic())

    def login_action(self, action, payload):
        state = self.login_state
        remember_name = state.name+'_remember'
        if action in ('forget','logout'):
            cookies = [self.login_cookie(remember_name,'',0)]
            if action=='logout':
                with state.lock:state.sessions.pop(self.cookie_value(state.name),None)
                cookies.append(self.login_cookie(state.name,'',0))
            self.auth_reply(200,{'ok':True},cookies)
            return
        token = self.cookie_value(remember_name)
        if action=='login-state':
            response = self.auth_backend({'operation':'authenticate','remember_token':token}) if token else {}
            cookies = [self.login_cookie(remember_name,'',0)] if token and not response.get('ok') else []
            self.auth_reply(200,{'ok':True,'remembered':bool(response.get('ok')),'username':response.get('username','')},cookies)
            return
        now,peer = time.monotonic(),self.auth_peer()
        with state.lock:
            for key in list(state.attempts):
                if not state.attempts[key] or now-state.attempts[key][-1]>=60:state.attempts.pop(key,None)
            attempts = [at for at in state.attempts.get(peer,[]) if now-at<60]
            if len(attempts)>=5 or len(state.attempts)>=128 and peer not in state.attempts:
                self.auth_reply(429,{'ok':False,'error':'尝试次数过多，请一分钟后重试'},[])
                return
            state.attempts[peer] = attempts+[now]
        request = {'operation':'authenticate','remember':payload.get('remember') is True,'username':payload.get('username')}
        if action=='resume' or not payload.get('password'):
            request['remember_token'] = token
        else:request['password'] = payload.get('password')
        response = self.auth_backend(request)
        if not response.get('ok'):
            self.auth_reply(401,{'ok':False,'error':'管理员账号或密码错误，或已记住的登录已失效'},
                            [self.login_cookie(remember_name,'',0)] if action=='resume' else [])
            return
        with state.lock:
            for key,(_,until) in list(state.sessions.items()):
                if until<=now:state.sessions.pop(key,None)
            if len(state.sessions)>=32:
                self.auth_reply(429,{'ok':False,'error':'登录设备过多，请稍后重试'},[])
                return
            session = secrets.token_urlsafe(32)
            state.sessions[session] = (peer,now+3600)
            state.attempts[peer] = []
        cookies = []
        if request['remember'] and response.get('remember_token'):
            cookies.append(self.login_cookie(remember_name,response['remember_token'],30*86400))
        else:cookies.append(self.login_cookie(remember_name,'',0))
        cookies.append(self.login_cookie(state.name,session,3600))
        self.auth_reply(200,{'ok':True,'username':response.get('username','')},cookies)
