// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
require('./check-cell-draft.cjs')
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict'), crypto = require('node:crypto')
const {JSDOM,VirtualConsole} = require('./ui-tests/node_modules/jsdom')
const {VERSION,VIEW} = require('./package-source.cjs')
const resources = path.join(__dirname,'../frontend/www/luci-static/resources')
// 资源使用完整语义版本命名，保留测试修订后缀。
const revision=VERSION
const shell = fs.readFileSync(path.join(resources,'wlmb-server/webui-shell-'+revision+'.js'),'utf8')
const adapter = fs.readFileSync(path.join(resources,'wlmb-server/webui-data-'+revision+'.js'),'utf8')
const icons = fs.readFileSync(path.join(resources,'wlmb-server/webui-icons-'+revision+'.js'),'utf8')
const source = fs.readFileSync(path.join(resources,'view/wlmb-server/'+VIEW+'.js'),'utf8')
assert.ok(!/WLMBVendor|WLMBReference|webui-vendor|webui-reference/.test(source+shell))
assert.ok(!/sendCommand|setInterval|WebSocket|dangerouslySetInnerHTML|innerHTML/.test(shell+adapter),'Presentation has no automatic transport or markup injection')
assert.ok(Buffer.byteLength(shell+adapter+icons)<100000,'Bounded independent UI resources')
const feedbackContext={window:{}}
require('node:vm').runInNewContext(adapter,feedbackContext)
const feedback=feedbackContext.window.WLMBUIData.lockFeedback
const lockItem={modem:'m1',operation:{rat:'NR',mode:'cell',entries:[{band:78,arfcn:627264,pci:488}]},receipt:{savedVerified:true,acceptedAtMs:1000}}
const lockRadio={stale:false,serving:[{rat:'NR',band:'n78',channel:627264,pci:488,observedAtMs:1001,stale:false}]}
assert.ok(feedback([lockItem],lockRadio,'m1').includes('符合所选条件'))
assert.ok(feedback([lockItem],{...lockRadio,serving:[{...lockRadio.serving[0],observedAtMs:999}]},'m1').includes('等待新的驻留'))
assert.ok(feedback([lockItem],{...lockRadio,serving:[{...lockRadio.serving[0],channelType:'center'}]},'m1').includes('等待新的驻留'))
assert.ok(feedback([lockItem],lockRadio,'other').includes('模组已更换'))
assert.ok(feedback([{...lockItem,receipt:{savedVerified:false}}],lockRadio,'m1').includes('不要重复提交'))
assert.ok(!fs.readdirSync(path.join(resources,'wlmb-server')).some(name=>/^webui-(vendor|reference)-/.test(name)),'Historical copied assets are not current source')
const errors=[]
const cssSource=fs.readFileSync(path.join(resources,'wlmb-server/panel.css'),'utf8')
const lockLayoutDom=new JSDOM('<style></style><div class="np-root"><div class="np-lock-layout"><div class="np-lock-controls"><label class="np-lock-field"><span>网络制式</span><select class="np-input"></select></label><label class="np-lock-field"><span>锁定方式</span><select class="np-input"></select></label></div><details open class="np-lock-advanced"><div class="np-toolbar"><input aria-label="PCI"><select hidden aria-label="SCS"></select></div></details><div class="np-lock-footer"><p class="np-lock-message"></p></div></div></div>')
lockLayoutDom.window.document.querySelector('style').textContent=cssSource
for(const theme of ['light','dark']){
 lockLayoutDom.window.document.documentElement.setAttribute('data-theme',theme)
 const style=node=>lockLayoutDom.window.getComputedStyle(lockLayoutDom.window.document.querySelector(node))
 assert.equal(style('.np-lock-layout').display,'grid');assert.equal(style('.np-lock-layout').gap,'20px')
 assert.equal(style('.np-lock-controls').gap,'16px');assert.equal(style('.np-lock-field').gap,'8px')
 assert.equal(style('.np-lock-advanced .np-toolbar').display,'flex','Hiding optional SCS cannot hide PCI/channel inputs')
 assert.equal(style('.np-lock-message').display,'none','Empty feedback does not leave a blank row')
}
lockLayoutDom.window.close()
assert.ok(cssSource.includes('max-width:760px'))
assert.ok(cssSource.includes('repeat(auto-fit,minmax(min(180px,100%),1fr))'),'桌面参数按内容区宽度排布')
assert.ok(cssSource.includes('.np-signal-grid .np-parameter-reading strong{font-size:28px}'),'极窄屏数值不能挤掉单位')
assert.ok(cssSource.includes('.np-parameter-reading{flex-wrap:wrap}')&&cssSource.includes('.np-parameter-reading span{white-space:nowrap}'),'数值和单位保持可换行的独立块')
assert.ok(cssSource.includes('.np-signal-grid.three>.np-signal-parameter:last-child{grid-column:1/-1}'))
assert.ok(cssSource.includes('repeat(4,minmax(0,1fr))'))
assert.ok(cssSource.includes('repeat(5,minmax(0,1fr))'))
assert.ok(cssSource.includes('env(safe-area-inset-bottom)'))
assert.ok(cssSource.includes('.np-overview-bottom>.np-native-card{height:auto;min-width:0;display:flex;flex-direction:column}'))
assert.ok(cssSource.includes('align-items:stretch')&&cssSource.includes('contain:size;overflow:auto'),'短信内容不撑高设备状态所在行')
assert.ok(cssSource.includes('.np-workspace-navigation{position:fixed;top:18px;'),'导航从首次显示起固定，不随宿主头部滚动跳位')
assert.ok(!cssSource.includes('np-details-column'),'Remove uneven independently stacked overview columns')
assert.ok(cssSource.includes('.np-webui-workspace{border-radius:0!important}'))
assert.ok(cssSource.includes('body.np-luci-surface,body .np-luci-surface{background:var(--np-page-surface)!important'))
assert.ok(cssSource.includes('.np-luci-surface>header:after{display:none!important}'),'Disable the host header decorative band only while this plugin owns its surface')
assert.ok(!cssSource.includes('np-overview-two-column'),'Remove obsolete equal-height row rules')
assert.ok(cssSource.includes('.np-webui-workspace.is-collapsed{--np-sidebar-width:68px}'))
assert.ok(!cssSource.includes('min-height:max(680px')&&!cssSource.includes('margin-top:auto'),'No viewport filler or footer pinning')
assert.ok(cssSource.includes('.np-recent-sms .np-empty-state{min-height:0;'))
assert.ok(cssSource.includes('#wlmb-panel .np-section-tabs .np-btn.selected{background:var(--np-card)!important;color:var(--np-text)!important'), 'Neutral tabs outrank the generic selected-action rule')
assert.ok(!/wlmb-ant|np-reference|backdrop-filter/.test(cssSource))
assert.ok(cssSource.includes('.np-root .np-btn[aria-busy=true]:before')&&cssSource.includes('.np-root .np-toggle[aria-busy=true] .np-track:before'),'Unified loading ring for actions and toggles')
assert.ok(cssSource.includes('.np-root *:before,.np-root *:after{transition:none!important;animation:none!important'),'Reduced motion also stops pseudo-element spinners')
assert.ok(!/transition:\s*all\b/.test(cssSource),'Only explicit inexpensive transition properties')
const virtual = new VirtualConsole()
virtual.on('jsdomError',error=> { if (!error.message.includes('Could not parse CSS stylesheet')) errors.push(error); })
const dom = new JSDOM('<!doctype html><html><body></body></html>',{url:'http://fixture.invalid/cgi-bin/luci/admin/services/wlmb-server',runScripts:'outside-only',pretendToBeVisual:true,virtualConsole:virtual})
const w=dom.window, doc=w.document
let scrollCalls=0
const scrollTargets=[]
w.HTMLElement.prototype.scrollIntoView=function(){scrollCalls++;scrollTargets.push(this)}
const computed=w.getComputedStyle.bind(w)
w.getComputedStyle=node=>computed(node)
let reducedMotion=false
const motionListeners=new Set(),animations=[]
w.matchMedia=query=>({get matches(){return query.includes('prefers-reduced-motion')&&reducedMotion},addListener(){},removeListener(){},addEventListener(_,fn){motionListeners.add(fn)},removeEventListener(_,fn){motionListeners.delete(fn)}})
w.HTMLElement.prototype.animate=function(frames,options){
  let resolve,reject
  const animation={node:this,frames,options,cancelled:false,finished:new Promise((yes,no)=>{resolve=yes;reject=no}),cancel(){if(!this.cancelled){this.cancelled=true;reject(new Error('cancelled'))}},finish(){resolve()}}
  animations.push(animation);return animation
}
w.ResizeObserver=class{observe(){} unobserve(){} disconnect(){}}
w.fetch=()=>{throw new Error('Unexpected network')}
w.WebSocket=class{constructor(){throw new Error('Upstream WebSocket must never execute')}}
w.XMLHttpRequest=class{constructor(){throw new Error('Unexpected network')}}
const calls=[],notices=[]
let running=true,statusError=false,serviceError=false,statusResponse=null
let smsCache={loaded:false,messages:[],stale:true},smsError=false
const copied=[]
doc.execCommand=action=>{assert.equal(action,'copy');copied.push(doc.activeElement.value);return true}
const status={backend:'rust',selectedModem:'fixture',capabilities:{at:true,wifi:true,deviceList:true,lockBand:true,lockCell:true,sms:true,smsForward:true,smsSend:false,smsDelete:false,configure:true},modems:[{id:'fixture',model:'Another modem <img src=x>',port:'/dev/ttyUSB9',shared:true}],radio:{observedAtMs:Date.now(),serving:[{rat:'NR',band:'n78',channel:627264,pci:488,networkMode:'5G SA',rsrp:-81,rsrq:-10,sinr:16}],carriers:[{rat:'NR',band:'n78',channel:630000,dlBandwidthMHz:100},{rat:'NR',band:'n78',channel:636664,dlBandwidthMHz:100}],neighbors:[{rat:'NR',band:'n78',channel:627264,pci:216,rsrp:-92}],lockPresets:{NR:[28,41,78],LTE:[1,3]}},system:{board:{model:'Fixture OpenWrt'},cpuUsage:7,memory:{usedPercent:25,usedBytes:1048576,totalBytes:4194304},lan:{devices:[],wireless:[]}}}
function E(tag,attrs={},children=[]) {
  const node=doc.createElement(tag)
  for(const [key,value] of Object.entries(attrs||{})) {
    if(typeof value==='function') node.addEventListener(key,value)
    else if(key==='class') node.className=value
    else if(key==='readonly') node.readOnly=!!value
    else if(key==='value') node.value=value
    else if(value!=null)node.setAttribute(key,String(value))
  }
  for(const child of [children].flat(Infinity)) if(child!=null) node.appendChild(child instanceof w.Node?child:doc.createTextNode(String(child)))
  return node
}
Object.assign(w,{E,view:{extend:value=>value},poll:{add(){}},L:{resource:name=>'/luci-static/resources/'+name,url:name=>'/cgi-bin/luci/'+name,hasViewPermission:()=>true},ui:{addNotification:(_,node)=>notices.push(node.textContent),showModal(title,nodes,...classes){doc.querySelector('.np-dialog')?.remove();const modal=E('div',{class:['modal',...classes].join(' ')},[E('h4',{},title),nodes]);doc.body.appendChild(modal);return modal},hideModal(){doc.querySelector('.np-dialog')?.remove()}}})
w.rpc={declare:options=>(...args)=>{
  calls.push({method:options.method,args})
  if(options.method==='status'&&statusError)return Promise.resolve({ok:false,error:{message:'fixture status timeout'}})
  if(options.method==='service_status'&&serviceError)return Promise.resolve({ok:false,error:{message:'fixture service timeout'}})
  if(options.method==='status'&&statusResponse)return statusResponse()
  if(options.method==='sms_list'&&smsError)return Promise.resolve({ok:false,error:{message:'fixture cache unavailable'}})
  const data=options.method==='service_status'?{running,backend:'rust',bootEnabled:true,controlReady:running,version:VERSION}:options.method==='status'?status:options.method==='settings'?{interval:3}:options.method==='sms_forward_settings'?{enabled:false,pollSeconds:5,minimumPollSeconds:3,tokenSet:false,runtime:{state:'disabled'}}:options.method==='sms_list'?smsCache : {}
  return Promise.resolve({ok:true,data})
}}
w.eval(icons);w.eval(adapter);w.eval(shell)
for(const name of ['WLMBIcons','WLMBUIData','WLMBWebUI'])assert.equal(w[name].version,revision,'Every loaded UI adapter must match the versioned entrypoint')
const page=w.eval('(function(){'+source+'\n})()')
const pause=()=>new Promise(resolve=>setTimeout(resolve,15))
async function settle(test) { const until=Date.now()+7000;while(!test()) {assert.ok(Date.now()<until,'Offline UI did not settle: '+notices.join(' | '));await pause();} }
async function main() {
  const styles=doc.createElement('style');styles.textContent='.luci-content{max-width:1440px;margin:auto}.main{padding-inline:32px}\n'+fs.readFileSync(path.join(resources,'wlmb-server/panel.css'),'utf8');doc.head.appendChild(styles)
  doc.body.classList.add('c2000-ui')
  const host=doc.createElement('main');host.className='main';const container=doc.createElement('div');container.className='luci-content';host.appendChild(container);doc.body.appendChild(host)
  const otherContainer=doc.createElement('div');otherContainer.className='luci-content';doc.body.appendChild(otherContainer)
  const root=page.render(await page.load());container.appendChild(root)
  await settle(()=>root.classList.contains('np-webui-ready'))
  assert.equal(root.querySelector('.np-webui-mount .np-page-title').textContent,'网络信息')
  await settle(()=>root.querySelectorAll('.np-menu-button').length===5)
  assert.equal(root.querySelectorAll('.np-workspace-navigation').length,1,'One scoped secondary navigation, not nested application chrome')
  assert.equal(root.querySelectorAll('.np-navigation-brand').length,1)
  assert.equal(root.querySelectorAll('.np-menu-button.is-active').length,1)
  const guideLinks=[...root.querySelectorAll('a.np-help-link')],guideCalls=calls.length
  assert.ok(guideLinks.length>=8,'侧栏、连接与各功能教程共用帮助按钮')
  assert.ok(root.querySelector('.np-navigation-guide.np-btn.np-help-link'))
  assert.ok(guideLinks.every(node=>!node.textContent.includes('↗')&&node.target==='_blank'&&node.rel.includes('noopener')))
  const guideAppearance=root.querySelector('.np-heading-actions button')
  for(const mode of ['light','dark','auto']){
    guideAppearance.click();assert.equal(root.dataset.npAppearance,mode)
    const resolved=computed(root).getPropertyValue('--np-scheme').trim()
    assert.ok(guideLinks.every(node=>new w.URL(node.href).searchParams.get('appearance')===resolved),'新窗口教程继承主界面的实际主题')
  }
  assert.equal(calls.length,guideCalls,'教程主题变更不查询设备')
  const collapse=root.querySelector('.np-collapse-toggle'),beforeCollapse=calls.length
  assert.ok(collapse.querySelector('svg.np-sidebar-icon rect')&&collapse.querySelector('.np-sidebar-direction'))
  assert.equal(collapse.textContent,'','Standard panel-left-close/open glyphs, no competing label')
  const collapseSvg=collapse.querySelector('svg')
  collapse.click();assert.ok(root.querySelector('.np-webui-workspace.is-collapsed'))
  assert.equal(collapse.querySelector('.np-sidebar-direction').getAttribute('d'),'m14 9 3 3-3 3')
  assert.equal(collapse.getAttribute('aria-expanded'),'false');assert.equal(collapse.getAttribute('aria-label'),'展开导航')
  assert.equal(w.sessionStorage.getItem('wlmb-server.sidebar'),'collapsed');assert.equal(calls.length,beforeCollapse)
  collapse.click();assert.ok(!root.querySelector('.np-webui-workspace.is-collapsed'));assert.equal(collapse.getAttribute('aria-expanded'),'true')
  assert.equal(collapse.querySelector('svg'),collapseSvg,'Preserve the icon for state transition')
  const navigation=root.querySelector('[aria-label="网络面板功能导航"]')
  assert.deepEqual([...navigation.querySelectorAll('.np-menu-button')].map(node=>Number(node.dataset.group)),[0,1,2,3,4],'Only five parents, not duplicated child pages')
  assert.deepEqual([...navigation.querySelectorAll('.np-menu-button')].map(node=>node.textContent),['概览','网络','WiFi','短信','设置'])
  function goPage(title){
    const parents={'网络信息':0,'邻区扫描':1,'频段与小区':1,'WiFi 与设备':2,'短信':3,'短信转发':3,'设备与流量':4,'AT 控制台':4,'插件设置':4}
    navigation.querySelector('[data-group="'+parents[title]+'"]').click()
    if(root.querySelector('.np-page-title').textContent!==title){const child=[...root.querySelectorAll('.np-section-tabs button')].find(node=>node.textContent===title);assert.ok(child,'Grouped child '+title);child.click()}
  }
  host.scrollTop=58
  navigation.dispatchEvent(new w.KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}))
  await settle(()=>root.querySelector('.np-page-title').textContent==='邻区扫描')
  assert.equal(host.scrollTop,0);assert.equal(scrollTargets.at(-1),host,'Keep the host header in view, not align the nested workspace past it')
  navigation.dispatchEvent(new w.KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}))
  await settle(()=>root.querySelector('.np-page-title').textContent==='WiFi 与设备')
  navigation.dispatchEvent(new w.KeyboardEvent('keydown',{key:'Home',bubbles:true,cancelable:true}))
  await settle(()=>root.querySelector('.np-page-title').textContent==='网络信息')
  await settle(()=>root.querySelector('.np-network-overview')?.textContent.includes('信号看板'))
  assert.ok(container.classList.contains('np-luci-fullwidth'))
  assert.ok(host.classList.contains('np-luci-host'))
  assert.equal(computed(container).maxWidth,'none','Remove the actual theme 1440px cap only in this view')
  assert.equal(computed(otherContainer).maxWidth,'1440px','Do not alter other LuCI pages')
  await settle(()=>root.querySelector('.np-recent-sms')?.textContent.includes('暂无短信预览'))
  assert.ok(root.querySelector('.np-overview-bottom .np-recent-sms'),'SMS card stays present even when empty')
  const overview=root.querySelector('.np-network-overview')
  assert.ok(overview.children[0].classList.contains('np-signal-board'))
  assert.equal(overview.children[1].querySelector('.np-native-title').textContent,'载波聚合信息','Carriers immediately follow signal in DOM and mobile reading order')
  assert.equal(overview.children[2].querySelector('.np-native-title').textContent,'网络与速率')
  assert.deepEqual([...overview.querySelector('.np-overview-bottom').querySelectorAll('.np-native-title')].map(title=>title.textContent),['设备状态','最近短信'])
  assert.equal(overview.querySelectorAll('.np-connection-rates .np-info-field').length,4)
  assert.equal(overview.querySelectorAll('.np-connection-fields .np-info-field').length,8)
  assert.ok(overview.querySelector('.np-device-status-grid').textContent.includes('CPU 占用'))
  assert.ok(!overview.textContent.includes('共用短信缓存 · 不额外轮询模组；复制不会标为已读。'))
  assert.ok(!root.textContent.includes('独立客户端，授权与界面单独管理')&&!overview.textContent.includes('点击读取，不额外轮询模组'),'首版移除多余介绍文案，不留空提示行')
  assert.equal(overview.querySelector('.np-recent-sms .np-empty-state .np-hint'),null)
  const stableSignal=overview.querySelector('.np-signal-board'),stableDevice=overview.querySelector('.np-device-status-grid'),stableSms=overview.querySelector('.np-recent-sms')
  const removals=[];const updateObserver=new w.MutationObserver(records=>records.forEach(record=>removals.push(...record.removedNodes)))
  updateObserver.observe(overview,{childList:true,subtree:true})
  status.system.cpuUsage=8.2;await page.update(true);await pause()
  assert.equal(overview.querySelector('.np-signal-board'),stableSignal)
  assert.equal(overview.querySelector('.np-device-status-grid'),stableDevice)
  assert.equal(overview.querySelector('.np-recent-sms'),stableSms)
  assert.ok(stableDevice.textContent.includes('8.2%'),'仅CPU变化也更新设备状态')
  assert.ok(!removals.some(node=>node===stableSignal||node===stableSms||node.contains?.(stableSms)),'后台刷新不移走首页区块和短信卡')
  updateObserver.disconnect()
  statusError=true;await page.update(true)
  assert.equal(page.selected,'fixture');assert.equal(page.snapshot.radio.serving.length,1)
  assert.ok(overview.textContent.includes('状态暂未更新'))
  assert.ok(overview.querySelector('[data-metric="rsrp"] .np-parameter-reading').textContent.includes('81'))
  assert.equal(page.smsBlocked('sms'),true,'保留展示不放开失败状态下的写入')
  page.lastGoodStatusAt=Date.now()-16000;await page.update(true)
  assert.equal(page.snapshot.radio.stale,true);assert.equal(page.snapshot.system.stale,true)
  assert.ok(overview.querySelector('[data-metric="rsrp"] .np-parameter-reading').textContent.includes('—'),'过期时不继续显示为实时值')
  statusError=false;await page.update(true)
  serviceError=true;await page.update(true)
  assert.equal(page.selected,'fixture');assert.equal(page.snapshot.radio.serving.length,1)
  assert.equal(page.runtime.failure,'fixture service timeout')
  serviceError=false;await page.update(true)
  let finishOld;statusResponse=()=>{statusResponse=null;return new Promise(resolve=>{finishOld=resolve})}
  const oldUpdate=page.update(true);await settle(()=>finishOld)
  await page.update(true);finishOld({ok:true,data:{modems:[],radio:{}}});await oldUpdate
  assert.equal(page.selected,'fixture','较旧的延迟回复不覆盖新快照')
  statusError=true;page.selected='other';await page.update(true)
  assert.equal(page.snapshot.radio.serving.length,0,'切换模组后不借用上一模组的数据')
  assert.equal(Object.keys(page.snapshot.capabilities).length,0)
  statusError=false;page.selected='fixture';await page.update(true)
  assert.ok(doc.body.classList.contains('np-luci-surface'))
  assert.equal(computed(root.querySelector('.np-webui-workspace')).borderRadius,'0')
  const beforeErrorPaint=animations.length
  status.radio.errors=[{command:'AT^DSAMBR=8',code:'AT_UNSUPPORTED',message:'AT_UNSUPPORTED'}];await page.update(true)
  assert.ok(!overview.textContent.includes('AT_UNSUPPORTED')&&!page.overview.textContent.includes('AT^DSAMBR'),'Raw optional AT errors do not return in native fallback either')
  assert.equal(animations.length,beforeErrorPaint)
  const carrier=root.querySelector('.np-carrier-card'),details=carrier.querySelector('details')
  assert.equal(details.open,false);assert.ok(carrier.querySelector('.np-carrier-essential').textContent.includes('630000'))
  assert.ok(carrier.querySelector('.np-carrier-essential').textContent.includes('100 MHz'))
  assert.ok(details.textContent.includes('下行频率')&&details.textContent.includes('上行频点'),'Extra fields retained, not deleted')
  details.open=true;await pause();assert.ok(details.open)
  const pollAnimations=animations.length;status.radio.observedAtMs++;await page.update(true);await pause()
  assert.equal(root.querySelector('.np-carrier-details').open,true,'Polling retains expanded detail state')
  assert.equal(animations.length,pollAnimations,'Polling never replays page or expanded-detail entry')
  const priorAnimations=animations.length
  goPage('邻区扫描');goPage('WiFi 与设备')
  assert.ok(animations.length>priorAnimations,'Navigation uses finite entry animation')
  assert.ok(animations.slice(priorAnimations).some(animation=>animation.cancelled),'Rapid navigation cancels old page entry')
  assert.ok(animations.every(animation=>animation.options.duration<=200&&!animation.options.iterations),'No unbounded page animation')
  reducedMotion=true;motionListeners.forEach(fn=>fn())
  const reducedCount=animations.length;goPage('网络信息');goPage('邻区扫描')
  assert.equal(animations.length,reducedCount,'Reduced-motion navigation is instant')
  reducedMotion=false;goPage('网络信息')
  const refreshButton=root.querySelector('.np-heading-actions .primary'),originalRefresh=page.webuiBridge.refresh
  let finishRefresh;page.webuiBridge.refresh=()=>new Promise(resolve=>{finishRefresh=resolve})
  refreshButton.click();assert.equal(refreshButton.getAttribute('aria-busy'),'true');assert.equal(refreshButton.disabled,true)
  refreshButton.click();await settle(()=>finishRefresh);finishRefresh();await settle(()=>refreshButton.getAttribute('aria-busy')==='false')
  page.webuiBridge.refresh=originalRefresh
  const smsReadsAfterSelection=calls.filter(call=>call.method==='sms_list').length
  assert.equal(smsReadsAfterSelection,2,'初始化与切回模组各读取一次缓存，状态错误不额外查询短信')
  await page.update(false);await page.update(false)
  const scrollAfterTick=scrollCalls;await page.update(false);assert.equal(scrollCalls,scrollAfterTick,'Background updates never scroll the page')
  assert.equal(calls.filter(call=>call.method==='sms_list').length,smsReadsAfterSelection,'Signal ticks do not repeatedly read SMS or query AT')
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('网络与速率'))
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('载波聚合信息'))
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('实时下载'))
  assert.ok([...root.querySelectorAll('.np-network-overview .np-network-registration')].some(node=>node.textContent==='已驻留'),'Do not color an unknown home/roaming state as an error')
  assert.equal(root.querySelector('.np-network-overview').textContent.includes('主载波'),false,'Do not assume the first unknown carrier is primary')
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('载波 1 · 角色未报告'),'Actual rendered unknown-role label, not just the props adapter')
  assert.ok(root.textContent.includes('Fixture OpenWrt'))
  assert.equal(root.querySelectorAll('.np-signal-parameter').length,3)
  assert.ok(root.querySelector('.np-signal-grid.three'))
  assert.deepEqual([...root.querySelectorAll('.np-mobile-button')].map(node=>node.textContent),['概览','网络','WiFi','短信','设置'])
  status.radio.serving[0].rssi=-75;await page.update(true)
  assert.equal(root.querySelectorAll('.np-signal-parameter').length,4)
  assert.ok(root.querySelector('[data-metric="rssi"]').textContent.includes('一般'),'RSSI has its own thresholds, not RSRP thresholds')
  delete status.radio.serving[0].rssi;await page.update(true)
  assert.equal(root.querySelectorAll('.np-signal-parameter').length,3)
  const appearance=root.querySelector('.np-heading-actions button')
  appearance.click();assert.equal(root.getAttribute('data-np-appearance'),'light')
  assert.equal(host.style.getPropertyValue('--np-page-surface'),'#f3f5f9','Outer padding follows explicit light appearance')
  assert.equal(computed(root).getPropertyValue('--np-card').trim(),'#fff')
  appearance.click();assert.equal(root.getAttribute('data-np-appearance'),'dark')
  assert.equal(host.style.getPropertyValue('--np-page-surface'),'#121214','Outer padding follows explicit dark appearance')
  assert.equal(computed(root).getPropertyValue('--np-card').trim(),'#1c1c1e')
  for(const [name,value] of Object.entries({bg:'#121214',soft:'#2a2a2d',text:'#f5f5f7',muted:'#aeaeb2',line:'#38383d'}))assert.equal(computed(root).getPropertyValue('--np-'+name).trim(),value,'Neutral dark '+name+' matches the mini-program')
  appearance.click();assert.equal(root.getAttribute('data-np-appearance'),'auto')
  status.radio.subscription={dlKbps:2000000,ulKbps:200000,apn:'internet',stale:false};status.radio.sessionQos={cid:1,qci:6,stale:false};
  status.radio.networkOperator={name:'中国联通',stale:false};
  await page.update(true);await settle(()=>root.querySelector('.np-network-overview').textContent.includes('QCI 6'));
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('中国联通'))
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('2000'));
  assert.ok(root.querySelector('.np-network-overview').textContent.includes('internet'));
  status.radio.subscription.stale=true;status.radio.sessionQos.stale=true;
  await page.update(true);await settle(()=>!root.querySelector('.np-network-overview').textContent.includes('QCI 6'));
  assert.ok(!root.querySelector('.np-network-overview').textContent.includes('internet'),'Expired APN-AMBR is not displayed');
  status.capabilities.smsReceivingConfigure=true;status.radio.smsReceiving={enabled:false,stale:false,observedAtMs:Date.now()};
  await page.update(true);
  assert.equal(page.imsToggle.input.disabled,false);assert.equal(page.imsToggle.input.checked,false);
  status.radio.smsReceiving.stale=true;await page.update(true);
  assert.equal(page.imsToggle.input.disabled,true,'Unknown/stale IMS state cannot be toggled');
  status.capabilities.smsReceivingConfigure=false;delete status.radio.smsReceiving;
  await page.update(true);assert.equal(page.imsToggle.input.disabled,true);
  assert.ok(page.imsNote.textContent.includes('不发送通用开启命令'));
  assert.equal(root.querySelectorAll('img[src=x]').length,0,'Modem data is text, not HTML')
  for(const title of ['邻区扫描','WiFi 与设备','频段与小区','短信','短信转发','设备与流量','AT 控制台','插件设置','网络信息']) {
    goPage(title)
    await settle(()=>root.querySelector('.np-page-title').textContent===title)
    assert.equal(page.pages.filter(node=>!node.hidden).length,1,'Only selected page is visible')
  }
  goPage('短信转发');await settle(()=>!page.forwardPending)
  const forwardTab=root.querySelector('.np-section-tabs [data-page="7"]')
  goPage('短信');goPage('短信转发')
  assert.equal(root.querySelector('.np-section-tabs [data-page="7"]'),forwardTab,'Keep segmented buttons alive for state transitions and focus')
  await pause();await settle(()=>!page.forwardPending)
  const beforeActiveClick=calls.length
  navigation.querySelector('[data-group="3"]').click();await pause()
  assert.equal(root.querySelector('.np-page-title').textContent,'短信转发','Active parent does not reset the child')
  assert.equal(calls.length,beforeActiveClick,'Active parent is a no-op, not another settings/cache fetch')
  goPage('WiFi 与设备');navigation.querySelector('[data-group="3"]').click()
  assert.equal(root.querySelector('.np-page-title').textContent,'短信转发','Parent remembers the child when returning')
  goPage('网络信息')
  assert.ok(calls.some(call=>call.method==='sms_list'))
  assert.ok(calls.some(call=>call.method==='sms_forward_settings'))
  const S=w.WLMBUIData
  for(const [body,expected] of [
    ['【测试】您的验证码是：123456，5分钟内有效。','123456'],['123456 是您的验证码','123456'],
    ['Your verification code is A2B3C4.','A2B3C4'],['验证码：１２３４５６','123456'],
    ['验证码123456；验证码123456','123456'],['验证码123456；验证码654321',null],
    ['今日消费123456元，请联系10086',null],['验证码请联系10086获取',null],
    ['验证码：2026年10月12日开始发送',null],['验证码：12345678901',null],
    ['验证码：<svg onload=alert(1)>',null]
  ])assert.equal(S.verificationCode(body),expected,body)
  assert.equal(S.messageTime('26/10/12,10:00:00+32'),Date.UTC(2026,9,12,2))
  assert.equal(S.messageTime('26/13/12,10:00:00+32'),0)
  assert.equal(S.messageTime('1791741600'),1791741600000)
  assert.equal(S.messageTime('unknown'),0)
  const smsRows=[
    {id:'one',phone:'10086',content:'验证码123456 <img src=x>',timestamp:'2026-10-12T02:00:00Z',direction:'incoming',unread:true},
    {id:'old',phone:'10010',content:'普通通知',timestamp:'2026-10-11T02:00:00Z',direction:'incoming'},
    {id:'part',phone:'10000',content:'验证码654321',timestamp:'2026-10-12T03:00:00Z',direction:'incoming',complete:false},
    {id:'sent',phone:'10086',content:'验证码111111',timestamp:'2026-10-12T04:00:00Z',direction:'outgoing'},
    {id:'no-time',phone:'10086',content:'验证码222222',timestamp:'',direction:'unknown'}
  ]
  assert.deepEqual(Array.from(S.recentSms({messages:smsRows}),item=>item.id),['part','one','old'])
  assert.equal(S.recentSms({messages:smsRows})[0].code,null,'No copy for incomplete multipart SMS')
  assert.ok(!S.recentSms({messages:smsRows}).some(item=>item.id==='sent'))
  await settle(()=>!page.recentSmsPending)
  smsCache={loaded:true,messages:smsRows,observedAtMs:Date.now(),stale:false};await page.readSms(false)
  await settle(()=>root.querySelector('.np-sms-code')?.textContent==='123456')
  assert.equal(root.querySelectorAll('img[src=x]').length,0,'SMS content must never be interpreted as markup')
  const codeButton=root.querySelector('.np-sms-code');codeButton.focus()
  codeButton.dispatchEvent(new w.MouseEvent('click',{bubbles:true,detail:1}));await pause()
  assert.equal(copied.length,0,'Do not copy on a single pointer click')
  codeButton.dispatchEvent(new w.MouseEvent('dblclick',{bubbles:true,detail:2}));await settle(()=>copied.length===1)
  await pause()
  assert.equal(copied[0],'123456','HTTP clipboard fallback copies only the recognized code')
  assert.equal(doc.activeElement,codeButton,'Restore focus after fallback')
  status.radio.observedAtMs++;await page.update(true)
  assert.equal(doc.activeElement,codeButton,'Live overview updates preserve SMS keyboard focus')
  codeButton.dispatchEvent(new w.MouseEvent('click',{bubbles:true,detail:0}));await settle(()=>copied.length===2)
  await pause()
  assert.ok(notices.includes('验证码已复制'))
  assert.ok(!calls.some(call=>call.method==='sms_read_state'),'Preview/copy does not change read state')
  await assert.rejects(page.copySmsCode({id:'one',code:'654321'}),/短信已更新/)
  const beforeFailure=copied.length,successNotices=notices.filter(n=>n==='验证码已复制').length
  doc.execCommand=()=>false
  codeButton.dispatchEvent(new w.MouseEvent('dblclick',{bubbles:true,detail:2}));await settle(()=>notices.some(n=>n.includes('复制失败')))
  assert.equal(copied.length,beforeFailure);assert.equal(notices.filter(n=>n==='验证码已复制').length,successNotices)
  Object.defineProperty(w,'isSecureContext',{configurable:true,value:true})
  Object.defineProperty(w.navigator,'clipboard',{configurable:true,value:{writeText:value=>{copied.push(value);return Promise.resolve()}}})
  codeButton.dispatchEvent(new w.MouseEvent('dblclick',{bubbles:true,detail:2}));await settle(()=>copied.length===beforeFailure+1)
  await pause()
  smsError=true;await assert.rejects(page.readSms(false),/fixture cache/)
  await settle(()=>root.querySelector('.np-recent-sms')?.textContent.includes('读取失败'))
  assert.equal(root.querySelectorAll('.np-recent-sms-item').length,3,'Keep known cache on RPC error, explicitly labeled')
  smsError=false
  const cacheCalls=calls.filter(call=>call.method==='sms_list').length
  const first=page.readSms(false),second=page.readSms(false)
  assert.equal(first,second,'Concurrent widgets share one pending cache RPC')
  await first
  assert.equal(calls.filter(call=>call.method==='sms_list').length,cacheCalls+1)
  const oldRequest=page.readSms(false)
  status.selectedModem='second';page.draw(status)
  await oldRequest
  await settle(()=>root.querySelector('.np-recent-sms')?.textContent.includes('暂无短信预览'))
  assert.equal(page.recentSms.messages.length,0,'Do not show previous modem SMS after selection changes')
  assert.equal(page.smsRows.textContent.includes('验证码123456'),false)
  status.selectedModem='fixture';page.draw(status)
  smsCache={loaded:true,messages:[]};await page.readSms(false)
  await settle(()=>root.querySelector('.np-recent-sms')?.textContent.includes('暂无收到的短信'))
  const neighborNav={click(){goPage('邻区扫描')}};neighborNav.click()
  await settle(()=>root.querySelector('.np-neighbor-presentation')?.textContent.includes('PCI:'))
  assert.ok(root.querySelector('.np-neighbor-presentation').textContent.includes('未读取'))
  assert.ok(root.querySelector('.np-neighbor-presentation').textContent.includes('n78'),'Actual neighbor band label survives the original JSX mapping')
  const grid=root.querySelector('.np-neighbor-grid')
  assert.ok(grid && !grid.style.maxHeight,'Natural page scroll, not a fixed desktop-only scroller')
  const lock=[...root.querySelectorAll('.np-neighbor-presentation button')].find(node=>node.textContent==='锁定');assert.ok(lock);lock.click()
  await settle(()=>root.querySelector('.np-page-title').textContent==='频段与小区')
  assert.equal(page.pages[5].querySelector('[aria-label="服务频点"]').value,'627264')
  assert.equal(page.pages[5].querySelector('[aria-label="小区 PCI"]').value,'216')
  assert.ok(!calls.some(call=>/_execute$|_prepare$|service_control|configure|sms_refresh/.test(call.method)),'Selecting a neighbor must only fill a draft')
  const D=w.WLMBUIData
  assert.equal(D.network({...status,radio:{...status.radio,carriers:[{rat:'NR',role:'scc',channel:636664,dlBandwidthMHz:100}]}}).carrierInfo[0].title,'辅助载波 1')
  assert.equal(D.network({...status,radio:{...status.radio,carriers:[{rat:'NR',role:'pcc'},{rat:'NR',role:'scc'}]}}).carrierInfo[1].title,'辅助载波 1')
  assert.equal(D.network({...status,radio:{...status.radio,carriers:[{rat:'NR',role:'unknown'}]}}).dlBandwidth,'—')
  assert.equal(D.sinrQuality(NaN).text,'未读取')
  const sorted=D.neighbors({...status,radio:{neighbors:[{rat:'LTE',channel:1300,pci:1,rsrp:null},{rat:'NR',channel:633984,pci:2,bandCandidates:['n77','n78'],rsrp:-90},{rat:'NR',channel:633984,pci:3,rsrp:-80}]}})
  assert.deepEqual(Array.from(sorted,row=>row.pci),[3,2,1])
  assert.equal(sorted[1].bandLabel,'n77 / n78（候选）')
  assert.equal(D.neighbors({...status,radio:{...status.radio,stale:true}})[0].lockable,false)
  assert.equal(D.neighbors({...status,radio:{...status.radio,stale:true}})[0].rsrp,'—')
  assert.equal(D.neighbors({...status,radio:{neighbors:[{rat:'LTE',channel:1300,pci:1,channelType:'center'}]}})[0].lockable,true,'LTE center EARFCN can fill a draft')
  assert.equal(D.neighbors({...status,radio:{neighbors:[{rat:'NR',channel:630000,pci:1,channelType:'center'}]}})[0].lockable,false,'Do not confuse NR center with SSB')
  neighborNav.click();await settle(()=>root.querySelector('.np-neighbor-presentation'))
  const search=root.querySelector('[aria-label="筛选邻区"]')
  const setValue=Object.getOwnPropertyDescriptor(w.HTMLInputElement.prototype,'value').set
  setValue.call(search,'no matches');search.dispatchEvent(new w.Event('input',{bubbles:true}))
  await settle(()=>root.querySelector('.np-neighbor-presentation').textContent.includes('共 0 个'))
  setValue.call(search,'');search.dispatchEvent(new w.Event('input',{bubbles:true}))
  await settle(()=>root.querySelector('.np-neighbor-presentation').textContent.includes('共 1 个'))
  const lteFilter=[...root.querySelectorAll('.np-neighbor-filter button')].find(node=>node.textContent==='4G');lteFilter.click()
  await settle(()=>root.querySelector('.np-neighbor-presentation').textContent.includes('共 0 个'))
  assert.ok(root.querySelector('.np-neighbor-presentation').textContent.includes('暂无邻区数据'))
  const allFilter=[...root.querySelectorAll('.np-neighbor-filter button')].find(node=>node.textContent.replace(/\s/g,'')==='全部');allFilter.click()
  await settle(()=>root.querySelector('.np-neighbor-presentation').textContent.includes('共 1 个'))
  root.style.setProperty('--np-scheme','dark');doc.documentElement.setAttribute('data-theme','dark')
  await settle(()=>root.querySelector('.np-webui-content').getAttribute('data-color-mode')==='dark')
  root.style.setProperty('--np-scheme','light');doc.documentElement.setAttribute('data-theme','light')
  await settle(()=>root.querySelector('.np-webui-content').getAttribute('data-color-mode')==='light')
  for (const [scheme,palette] of [['light',{card:'#ffffff',text:'#1f2329',soft:'#f5f7fa',muted:'#717680',line:'#f0f0f0'}],['dark',{card:'#1f1f1f',text:'#e8e8e8',soft:'#141414',muted:'#a0a0a0',line:'#303030'}]]) {
    doc.documentElement.setAttribute('data-theme',scheme==='light'?'dark':'light')
    root.style.setProperty('--np-scheme',scheme)
    for(const [key,value] of Object.entries(palette))root.style.setProperty('--np-'+key,value)
    page.editWifi({id:'wifi0',band:'2g',ssid:'Fixture',authMode:'psk2',enabled:true,modeSupported:true,channelSupported:true,securitySupported:true,bandwidthSupported:true,modeOptions:[],bandwidthOptions:[],channelOptions:[],securityOptions:[]})
    const dialog=doc.querySelector('.np-dialog');assert.ok(dialog)
    assert.equal(dialog.getAttribute('data-np-theme'),scheme)
    for(const [key,value] of Object.entries(palette))assert.equal(dialog.style.getPropertyValue('--np-'+key),value,'Modal inherits panel '+key)
    assert.equal(dialog.querySelector('h4').textContent,'编辑无线网络')
    assert.ok(dialog.querySelectorAll('.np-row').length>=6)
    const wifiSwitch=dialog.querySelector('[aria-label="启用 WiFi"]')
    assert.equal(wifiSwitch.checked,true);assert.equal(wifiSwitch.defaultChecked,true)
    assert.ok(wifiSwitch.closest('.np-toggle'));assert.ok(wifiSwitch.nextElementSibling.classList.contains('np-track'))
    w.ui.hideModal()
  }
  const savedHostAction=page.hostAction;let wifiDraft
  page.hostAction=operation=>{wifiDraft=operation;return Promise.resolve()}
  page.editWifi({id:'unknown',band:'5g',ssid:'Old',runtimeEnabled:undefined})
  reducedMotion=true;const beforeReducedDialog=animations.length
  page.editWifi({id:'unknown',band:'5g',ssid:'Old',runtimeEnabled:undefined})
  assert.equal(animations.length,beforeReducedDialog,'Reduced-motion modal is instant');reducedMotion=false
  w.ui.hideModal();page.editWifi({id:'unknown',band:'5g',ssid:'Old',runtimeEnabled:undefined})
  const currentUnknown=doc.querySelector('.np-dialog'),currentSwitch=currentUnknown.querySelector('[aria-label="启用 WiFi"]')
  assert.equal(currentSwitch.disabled,true);assert.equal(currentSwitch.indeterminate,true)
  currentUnknown.querySelector('[aria-label="WiFi 名称"]').value='Renamed'
  const next=[...currentUnknown.querySelectorAll('button')].find(node=>node.textContent==='下一步');next.click();assert.equal(next.getAttribute('aria-busy'),'true')
  await settle(()=>wifiDraft);assert.equal(Object.hasOwn(wifiDraft.changes,'enabled'),false,'Unknown status cannot silently disable WiFi')
  await settle(()=>next.getAttribute('aria-busy')==='false')
  page.hostAction=savedHostAction;w.ui.hideModal()
  status.system.lan={wireless:[{id:'live',band:'5g',ssid:'Fixture',enabled:true}],controls:{stale:false,profiles:[{id:'live',band:'5g',ssid:'Fixture',enabled:true}]}}
  status.system.lan.devices=[{name:'Computer',type:'有线',mac:'02:00:00:00:00:11',ip:'192.168.1.2',rateScope:'port',rateInterface:'eth1',rates:{downloadBytesPerSecond:125000,uploadBytesPerSecond:12500}}]
  status.system.lan.ports=[{interface:'eth1',peerCount:1,rates:{downloadBytesPerSecond:125000,uploadBytesPerSecond:12500}}]
  page.draw(status);goPage('WiFi 与设备')
  assert.ok(page.networks.textContent.includes('网口 eth1 · 端口统计'));assert.ok(page.networks.textContent.includes('1.00 Mbps'));assert.ok(page.networks.textContent.includes('网口实时速率'))
  for(const [reason,message] of [['FLOW_OFFLOAD_ENABLED','已启用流量加速，上网控制暂不可用'],['FIREWALL_UNAVAILABLE','无法读取防火墙状态'],['WAN_INTERFACE_UNCONFIRMED','上网接口尚未确认'],['HOST_STATE_STALE','控制状态待更新']]){
    status.system.lan.deviceBlockSupported=false;status.system.lan.deviceBlockReason=reason;page.draw(status)
    assert.ok(page.networks.textContent.includes(message),'Show the actual guard restriction: '+reason)
    assert.ok(!page.networks.textContent.includes('当前系统未开放'))
    assert.ok(![...page.networks.querySelectorAll('button')].some(node=>node.textContent==='限制上网'),'Do not bypass the backend guard')
  }
  ;[...page.networks.querySelectorAll('button')].find(node=>node.textContent==='编辑').click()
  await settle(()=>doc.querySelector('.np-dialog [aria-label="启用 WiFi"]'))
  assert.equal(doc.querySelector('.np-dialog [aria-label="启用 WiFi"]').checked,true)
  assert.ok(doc.querySelector('.np-dialog').textContent.includes('实际运行中'))
  w.ui.hideModal();delete status.system.lan;page.draw(status)
  assert.equal(navigation.querySelector('[data-group="2"] svg path').getAttribute('d'),w.WLMBIcons.paths.wifi,'Reuse client WiFi glyph, not unrelated generic lines')
  root.style.setProperty('--np-scheme','light');doc.documentElement.setAttribute('data-theme','light')
  assert.ok(!calls.some(call=>/_execute$|_prepare$|service_control|configure|sms_refresh/.test(call.method)),'Navigation must not cause writes or AT refresh')
  const savedLockSnapshot=page.lockSnapshot,savedLockCapabilities=page.capabilities;
  page.lockSnapshot={...status.radio,lockPresets:{NR:[],LTE:[1,3]}};page.capabilities={...savedLockCapabilities,lockBand:true,bandLock:true,lockCell:false,cellLock:false};page.refreshLock(true)
  const lteOnlyRat=root.querySelector('[aria-label="锁定制式"]'),lteOnlyMode=root.querySelector('[aria-label="锁定类型"]')
  assert.equal(lteOnlyRat.value,'LTE');assert.equal(lteOnlyRat.children[0].disabled,true)
  assert.equal(lteOnlyMode.value,'band');assert.equal(lteOnlyMode.children[1].hidden,true)
  page.lockSnapshot=savedLockSnapshot;page.capabilities=savedLockCapabilities;lteOnlyRat.value='NR';page.refreshLock(true)
  status.capabilities.lockBand=status.capabilities.lockCell=false;page.draw(status)
  await settle(()=>root.querySelectorAll('.np-menu-button').length===5)
  goPage('频段与小区')
  assert.ok([...root.querySelectorAll('.np-section-tabs button')].some(node=>node.textContent==='频段与小区'))
  const readOnlyLock=page.pages[5];
  assert.ok([...readOnlyLock.querySelectorAll('button')].find(node=>node.textContent==='检查并保存').disabled)
  assert.ok(readOnlyLock.textContent.includes('仅展示型号资料'))
  assert.ok(readOnlyLock.querySelector('.np-lock-advanced summary').textContent.includes('补充参数'))
  assert.ok(readOnlyLock.querySelector('.np-lock-basic [aria-label="小区 PCI"]'),'手动输入PCI不再藏在高级折叠中')
  assert.ok(readOnlyLock.querySelector('.np-lock-basic [aria-label="服务频点"]'),'手动输入服务频点始终可见')
  goPage('网络信息')
  const originalLayout=w.WLMBUIData.network
  w.WLMBUIData.network=function(){throw new Error('fixture render failure')}
  status.radio.observedAtMs++
  page.webuiBridge.redraw()
  await settle(()=>!root.classList.contains('np-webui-ready'))
  assert.equal(motionListeners.size,0,'Fallback releases reduced-motion listener')
  assert.ok(animations.filter(animation=>animation.node===root.querySelector('.np-native-presentation')).every(animation=>animation.cancelled),'Fallback cancels outstanding page animation')
  w.WLMBUIData.network=originalLayout
  assert.ok(root.querySelector('.np-tabs') && !root.querySelector('.np-tabs').hidden,'Fallback navigation restored')
  assert.equal(page.pages.filter(node=>!node.hidden).length,1)
  assert.ok(root.textContent.includes('Fixture OpenWrt'))
  assert.ok(notices.some(text=>text.includes('兼容界面')))
  root.remove();await pause()
  assert.ok(!container.classList.contains('np-luci-fullwidth')&&!host.classList.contains('np-luci-host'),'Restore theme ancestors after leaving the view')
  assert.ok(!doc.body.classList.contains('np-luci-surface'))
  assert.equal(host.style.getPropertyValue('--np-page-surface'),'')
  const fit=w.eval('('+source.slice(source.indexOf('function fitLuci('),source.indexOf('function copyCode(')).trim()+')')
  const outer=E('div',{class:'main'},[E('aside',{class:'main-left'},'router navigation'),E('div',{class:'main-right'},[E('header',{class:'bg-primary'},'router status'),E('div',{id:'maincontent'},[E('div',{class:'container'},[E('div',{id:'view'},[])])]),E('footer',{},'LuCI credits')])]);doc.body.appendChild(outer)
  const right=outer.querySelector('.main-right'),left=outer.querySelector('.main-left'),argonRoot=E('div',{class:'np-root'},'short page');outer.querySelector('#view').appendChild(argonRoot)
  doc.body.classList.remove('c2000-ui')
  right.style.setProperty('--np-page-surface','#998877','important')
  doc.body.style.setProperty('--np-page-surface','#887766','important')
  const rightPriority=right.style.getPropertyPriority('--np-page-surface'),bodyPriority=doc.body.style.getPropertyPriority('--np-page-surface')
  const restore=fit(argonRoot)
  assert.ok(right.classList.contains('np-luci-surface'));assert.ok(!left.classList.contains('np-luci-surface'))
  argonRoot.style.setProperty('--np-bg','#121214');restore.sync()
  assert.equal(right.style.getPropertyValue('--np-page-surface'),'#121214')
  argonRoot.style.setProperty('--np-bg','#f3f5f9');restore.sync()
  assert.equal(right.style.getPropertyValue('--np-page-surface'),'#f3f5f9')
  assert.equal(computed(right.querySelector('header')).backgroundColor,'rgba(0, 0, 0, 0)');assert.equal(computed(right.querySelector('footer')).backgroundColor,'rgba(0, 0, 0, 0)')
  restore();assert.equal(right.style.getPropertyValue('--np-page-surface'),'#998877');assert.equal(right.style.getPropertyPriority('--np-page-surface'),rightPriority)
  assert.equal(doc.body.style.getPropertyValue('--np-page-surface'),'#887766');assert.equal(doc.body.style.getPropertyPriority('--np-page-surface'),bodyPriority)
  assert.ok(!right.classList.contains('np-luci-surface'));outer.remove();doc.body.style.removeProperty('--np-page-surface');doc.body.classList.add('c2000-ui')
  assert.equal(computed(container).maxWidth,'1440px');assert.ok(doc.body.classList.contains('c2000-ui'))
  dom.window.close()
  assert.deepEqual(errors,[])
  console.log('Independent UI offline checks passed: continuous/restore-safe Argon and generic surfaces, square workspace, unified overview/rates/status, no home AT dump; finite motion, busy guards, cache/SMS/OTP, safe drafts, 9 pages. Not visual acceptance.')
}
require('./check-guide.cjs')().then(main).catch(error=>{console.error(error);dom.window.close();process.exitCode=1})
