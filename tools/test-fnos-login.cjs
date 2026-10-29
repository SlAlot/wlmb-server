const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {JSDOM}=require('./ui-tests/node_modules/jsdom');
const root=path.resolve(__dirname,'..');
const fixtures=['fnos/app/www'];
async function test(folder){
 const html=fs.readFileSync(path.join(root,folder,'index.html'),'utf8').replace(/<script[^>]*src=[^>]*><\/script>/g,'');
 const source=fs.readFileSync(path.join(root,folder,'login.js'),'utf8');
 const key='fixture.login',secret='fixture-password-never-store';
 let remembered=false,authenticated=false,resumes=0;
 const sessions=[];
 function page(prefs){
  const dom=new JSDOM(html,{url:'http://127.0.0.1:1118/',runScripts:'outside-only'});sessions.push(dom);
  const w=dom.window;if(prefs)w.localStorage.setItem(key,prefs);w.eval(source);
  const content=w.document.querySelector('#app-content,#workspace');
  const auth=new w.AppLogin({key,content,open:async()=>{if(!authenticated)throw Object.assign(Error('login required'),{status:401})},request:async(action,value)=>{
   if(action==='login-state')return {remembered,username:remembered?'fixture-admin':''};
   if(action==='logout'||action==='forget'){remembered=false;authenticated=false;return {}}
   if(action==='resume'){resumes++;assert(remembered)}
   else assert(value.password===secret||remembered);
   remembered=value.remember===true;authenticated=true;return {username:'fixture-admin'};
  }});
  return {w,auth,content};
 }
 try{
  let p=page();await p.auth.start();assert(p.auth.visible);assert(p.w.document.querySelector('#password').required);
  p.auth.username.value='fixture-admin';p.auth.password.value=secret;p.auth.remember.checked=true;p.auth.auto.checked=true;
  await p.auth.login(false);assert(!p.auth.visible);assert(!p.content.hidden);
  const prefs=p.w.localStorage.getItem(key);assert(!prefs.includes(secret));assert(!prefs.includes('token'));
  authenticated=false;p=page(prefs);await p.auth.start();assert.equal(resumes,1);assert(!p.auth.visible);
  await p.auth.logout();assert(p.auth.visible);assert(!p.auth.auto.checked);assert(!p.auth.remember.checked);
  remembered=true;authenticated=false;resumes=0;p=page(JSON.stringify({username:'fixture-admin',remember:true,auto:false}));
  await p.auth.start();assert(p.auth.visible);assert.equal(resumes,0);assert.equal(p.auth.username.value,'fixture-admin');assert(!p.auth.password.required);
  await p.auth.login(false);assert(!p.auth.visible);assert.equal(p.auth.password.value,'');
  console.log(folder+': login, remember, automatic restore, manual restore and logout passed');
 }finally{sessions.forEach(x=>x.window.close())}
}
(async()=>{for(const f of fixtures)await test(f)})().catch(e=>{console.error(e);process.exitCode=1});
