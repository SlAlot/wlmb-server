'use strict'
// 教程离线回归只核对节点、配色与布局约束，不操作浏览器或设备。
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict')
const {JSDOM}=require('./ui-tests/node_modules/jsdom')
const {VERSION,VIEW}=require('./package-source.cjs')
const base=path.join(__dirname,'../frontend/www/luci-static/resources'),directory=path.join(base,'wlmb-server')
const html=fs.readFileSync(path.join(directory,'guide.html'),'utf8'),script=fs.readFileSync(path.join(directory,'guide.js'),'utf8')
const shared=fs.readFileSync(path.join(directory,'panel.css'),'utf8'),css=fs.readFileSync(path.join(directory,'guide.css'),'utf8')
async function check(){
 const make=(url,stored='auto',blocked=false)=>{
  const dom=new JSDOM(html,{url,runScripts:'outside-only',pretendToBeVisual:true}),w=dom.window,d=w.document
  const style=d.createElement('style');style.textContent=shared+'\n'+css;d.head.appendChild(style)
  w.fetch=()=>{throw new Error('教程不允许请求接口')};w.XMLHttpRequest=class{constructor(){throw new Error('教程不允许请求接口')}}
  if(blocked)Object.defineProperty(w,'sessionStorage',{get(){throw new Error('存储被阻止')}})
  else if(!url.startsWith('file:'))w.sessionStorage.setItem('wlmb-server.appearance',stored)
  w.eval(script);return dom
 }
 const dom=make('http://192.168.66.1/luci-static/resources/wlmb-server/guide.html?appearance=dark#locks'),w=dom.window,d=w.document
 try{
  for(const element of d.querySelectorAll('[src],link[href]')){
   const local=element.getAttribute('src')||element.getAttribute('href'),url=new URL(local,w.location.href)
   assert.equal(url.origin,w.location.origin);assert.equal(url.searchParams.get('v'),VERSION)
   assert.ok(fs.existsSync(path.join(directory,path.basename(url.pathname))))
  }
  const view=fs.readFileSync(path.join(base,'view/wlmb-server/'+VIEW+'.js'),'utf8')
  for(const [,id] of view.matchAll(/guide\('#([^']+)'\)/g))assert.ok(d.getElementById(id),'每个说明入口都有对应章节：'+id)
  for(const link of d.querySelectorAll('a[href^="#"]'))assert.ok(d.getElementById(link.hash.slice(1)))
  assert.equal(d.querySelectorAll('h1').length,1);assert.equal(d.querySelectorAll('.np-guide-steps>li').length,3)
  assert.ok(d.getElementById('locks').open);assert.equal(d.getElementById('radio').open,false)
  w.location.hash='#radio';w.dispatchEvent(new w.HashChangeEvent('hashchange'));assert.ok(d.getElementById('radio').open)
  w.location.hash='#%E0%A4%A';assert.doesNotThrow(()=>w.dispatchEvent(new w.HashChangeEvent('hashchange')))
  const root=d.getElementById('wlmb-panel'),theme=d.getElementById('guide-theme')
  assert.equal(root.dataset.npAppearance,'dark');assert.equal(w.getComputedStyle(root).getPropertyValue('--np-bg').trim(),'#121214')
  theme.click();assert.equal(root.dataset.npAppearance,'auto')
  theme.click();assert.equal(root.dataset.npAppearance,'light');assert.equal(w.getComputedStyle(root).getPropertyValue('--np-bg').trim(),'#f3f5f9')
  theme.click();assert.equal(root.dataset.npAppearance,'dark');assert.equal(w.sessionStorage.getItem('wlmb-server.appearance'),'dark')
  assert.equal(d.getElementById('guide-address').textContent,'192.168.66.1:1117')
  let attempts=0;d.execCommand=action=>{assert.equal(action,'copy');assert.equal(w.getSelection().toString(),'192.168.66.1:1117');attempts++;return true}
  d.getElementById('guide-copy').click();await Promise.resolve();assert.equal(attempts,1);assert.ok(d.getElementById('guide-feedback').textContent.includes('已复制'))
  Object.defineProperty(w,'isSecureContext',{value:true});Object.defineProperty(w.navigator,'clipboard',{value:{writeText:async()=>{throw new Error('拒绝剪贴板')}}})
  d.getElementById('guide-copy').click();await new Promise(resolve=>setImmediate(resolve));assert.equal(attempts,2)
  d.execCommand=()=>false;d.getElementById('guide-copy').click();await new Promise(resolve=>setImmediate(resolve));assert.ok(d.getElementById('guide-feedback').textContent.includes('未能自动复制'));assert.equal(d.getElementById('guide-copy').disabled,false)
  assert.ok(!/[↗←]/.test(html+view),'不使用文字箭头装饰教程入口')
  assert.ok(!/fetch\(|XMLHttpRequest|WebSocket|setInterval|innerHTML/.test(script),'教程无采集、轮询或动态标记注入')
  assert.ok(css.includes('repeat(3,minmax(0,1fr))')&&css.includes('@media(max-width:760px)')&&css.includes('grid-template-columns:minmax(0,1fr)'))
  assert.ok(!/#[0-9a-f]{3,8}\b/i.test(css.replace(/color:#fff/g,'')),'布局不另建偏色的主题色板')
  assert.ok(shared.includes('.np-root .np-help-link:before')&&shared.includes('prefers-reduced-motion:reduce'))
 }finally{w.close()}
 for(const [url,expected,stored,blocked] of [
  ['http://router.lan:8080/guide.html','router.lan:1117','light',false],
  ['http://[fd00::1]/guide.html','[fd00::1]:1117','dark',true],
  ['file:///guide.html','你的管理地址:1117','auto',true],
  ['http://router.lan/guide.html?appearance=unknown','router.lan:1117','invalid',false]
 ]){
  const fixture=make(url,stored,blocked)
  try{const doc=fixture.window.document;assert.equal(doc.getElementById('guide-address').textContent,expected);assert.equal(doc.getElementById('guide-copy').hidden,url.startsWith('file:'));assert.ok(['auto','light','dark'].includes(doc.body.dataset.npAppearance));assert.doesNotThrow(()=>doc.getElementById('guide-theme').click())}finally{fixture.window.close()}
 }
 console.log('教程离线回归通过：深浅主题、章节、地址复制、拒绝权限回退和窄屏约束；不是浏览器视觉验收。')
}
module.exports=check
if(require.main===module)check().catch(error=>{console.error(error);process.exitCode=1})
