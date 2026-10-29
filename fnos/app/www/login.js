'use strict';
class AppLogin {
 constructor({request,open,content,key}){
  this.request=request;this.open=open;this.content=content;this.key=key;this.visible=false;this.busy=false;
  this.view=document.querySelector('#auth-view');this.form=document.querySelector('#login-form');
  this.username=document.querySelector('#username');this.password=document.querySelector('#password');
  this.remember=document.querySelector('#remember');this.auto=document.querySelector('#auto-login');
  this.notice=document.querySelector('#login-notice');this.submit=this.form.querySelector('[type=submit]');
  this.prefs={};try{this.prefs=JSON.parse(localStorage.getItem(key)||'{}')}catch{}
  this.username.value=typeof this.prefs.username==='string'?this.prefs.username:'';
  this.remember.checked=this.prefs.remember===true;this.auto.checked=this.prefs.auto===true&&this.remember.checked;
  this.form.addEventListener('submit',event=>{event.preventDefault();this.login(false)});
  this.auto.addEventListener('change',()=>{if(this.auto.checked)this.remember.checked=true;this.save()});
  this.remember.addEventListener('change',()=>{if(!this.remember.checked){this.auto.checked=false;this.remembered=false;this.password.placeholder='请输入管理员密码';this.password.required=true;this.request('forget',{}).catch(()=>{})}this.save()});
  document.querySelectorAll('[data-logout]').forEach(button=>button.addEventListener('click',()=>this.logout()));
  document.querySelectorAll('[data-auth-theme]').forEach(button=>button.addEventListener('click',()=>{
   let value='auto';try{value=sessionStorage.getItem('wlmb-server.appearance')||'auto'}catch{}
   value=value==='auto'?'light':value==='light'?'dark':'auto';try{sessionStorage.setItem('wlmb-server.appearance',value)}catch{}this.theme(value);
  }));
  let theme='auto';try{theme=sessionStorage.getItem('wlmb-server.appearance')||'auto'}catch{}this.theme(theme);
 }
 theme(value){
  const root=document.querySelector('.np-root');if(root)root.dataset.npAppearance=value;
  document.querySelectorAll('[data-auth-theme]').forEach(button=>button.title='外观：'+({auto:'自动',light:'浅色',dark:'深色'}[value]||'自动'));
 }
 save(){
  try{localStorage.setItem(this.key,JSON.stringify({username:this.username.value.trim(),remember:this.remember.checked,auto:this.auto.checked&&this.remember.checked}))}catch{}
 }
 showState(visible){
  this.visible=visible;this.view.hidden=!visible;this.content.hidden=visible;
  document.body.classList.toggle('login-visible',visible);
  const loading=document.querySelector('#auth-loading');if(loading)loading.hidden=true;
 }
 async start(){
  try{await this.open();this.showState(false)}catch(error){await this.show(error.status===401?'':error.message)}
 }
 async show(message=''){
  if(this.visible)return;
  this.showState(true);this.notice.textContent=message;
  try{
   const value=await this.request('login-state',{});this.remembered=value.remembered===true;
   if(this.remembered){this.username.value=value.username||this.username.value;this.remember.checked=true;this.password.placeholder='密码已记住，可直接登录';this.password.required=false}
   else{this.password.placeholder='请输入管理员密码';this.password.required=true}
   if(this.auto.checked&&this.remembered)await this.login(true);
  }catch(error){this.notice.textContent=error.message||'暂时无法连接，请稍后重试'}
 }
 async login(automatic){
  if(this.busy)return;
  if(!automatic&&!this.password.value&&!this.remembered){this.password.reportValidity();return}
  this.busy=true;this.submit.disabled=true;this.submit.textContent=automatic?'正在自动登录…':'正在登录…';this.notice.textContent='';
  try{
   await this.request(automatic?'resume':'login',{username:this.username.value.trim(),password:this.password.value,remember:this.remember.checked});
   this.save();this.password.value='';this.showState(false);await this.open();
  }catch(error){
   this.showState(true);this.notice.textContent=error.message||'登录失败';
   if(automatic){this.auto.checked=false;this.save()}
   if(error.status===401){this.remembered=false;this.password.required=true;this.password.placeholder='请重新输入管理员密码'}
  }finally{this.busy=false;this.submit.disabled=false;this.submit.textContent='登录'}
 }
 async logout(){
  await this.request('logout',{});this.auto.checked=false;this.remember.checked=false;this.remembered=false;this.password.value='';this.save();this.visible=false;await this.show();
 }
}
window.AppLogin=AppLogin;
