// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs = require('node:fs'), path = require('node:path'), vm = require('node:vm'), assert = require('node:assert/strict')
const root = path.join(__dirname, '..', 'frontend')
class Element {
  constructor(tag, attrs = {}, children = []) {
    this.tag = tag; this.attrs = { ...attrs }; this.children = []; this.listeners = {}; this.disabled = false
    Object.assign(this, attrs)
    this.classes = new Set((attrs.class || '').split(/\s+/).filter(Boolean))
    this.classList = { toggle: (name, enabled) => enabled ? this.classes.add(name) : this.classes.delete(name) }
    this.replaceChildren(...(Array.isArray(children) ? children : [children]))
  }
  appendChild(child) { this.children.push(child); return child }
  replaceChildren(...children) { this.children = children.flat(Infinity).filter(value => value !== null && value !== undefined) }
  setAttribute(name, value) { this.attrs[name] = value }
  addEventListener(name, callback) { this.listeners[name] = callback }
  get textContent() { return this.children.map(child => child instanceof Element ? child.textContent : String(child)).join('') }
  set textContent(value) { this.children = [String(value)] }
}
const E = (tag, attrs, children) => new Element(tag, attrs, children)
const calls = [], notifications = [], modals = []
let running = false, boot = true, modal = null
let nextReceipt = 0
const receipts = new Map()
const status = { capabilities:{at:true,lockBand:true,lockCell:true,configure:true},selectedModem: 'test', modems: [{ id: 'test', model: '<script>not HTML</script>', port: '/dev/ttyUSB1', shared: true }],
  radio: { lockPresets:{NR:[1,28,41,78,79],LTE:[1,3,5,8,34,38,39,40,41]}, serving: [{ rat: 'NR', channel:633984, pci:505, networkMode: '5G SA', band: 'n78', rsrp: -81, rsrq: -5, sinr: 16 }], carriers: [], neighbors: [], observedAtMs: Date.now() },
  system: { cpuUsage: 10, memory: { usedPercent: 25, usedBytes: 1048576, totalBytes: 4194304 }, uptimeSeconds: 600, stale: false } }
const runtime = () => ({ running, bootEnabled: boot, controlReady: running, version: '0.2.0-dev' })
const rpc = { declare: options => (...args) => {
  calls.push({ method: options.method, args })
  if (options.method === 'service_status') return Promise.resolve({ ok: true, data: runtime() })
  if (options.method === 'service_control') {
    if (args[0] === 'boot') boot = args[1]
    else running = args[0] !== 'stop'
    return Promise.resolve({ ok: true, data: { accepted: true, service: runtime() } })
  }
  if (!running) return Promise.resolve({ ok: false, error: { message: '插件服务尚未启动' } })
  if (['lock_status','lock_prepare','lock_execute','host_prepare','host_execute'].includes(options.method)) {
    const job = 'job-' + nextReceipt++
    receipts.set(job,{state:'done',result: options.method === 'lock_status' ? {groups:{},activation:'unverified'} : options.method.endsWith('_prepare') ? {warning:'仅保存',command:'typed-lock',revision:'r',confirmation:'c'} : {accepted:true,activation:'unverified'}})
    return Promise.resolve({ok:true,data:{job,state:'pending'}})
  }
  if (options.method === 'control_result') return Promise.resolve({ok:true,data:receipts.get(args[0])})
  if (options.method === 'at_prepare') return Promise.resolve({ok:true,data:{readOnly:true,confirmation:'console-once'}})
  if (['refresh','configure','at_execute'].includes(options.method)) {
    const job='control-job-'+nextReceipt++
    receipts.set(job,{state:'done',result:options.method==='at_execute'?{response:'synthetic modem response',readOnly:true}:{accepted:true}})
    return Promise.resolve({ok:true,data:{job,state:'pending'}})
  }
  if (options.method === 'sms_list') return Promise.resolve({ok:true,data:{loaded:true,readStatePersistent:true,messages:[{id:'a'.repeat(64),phone:'10086',content:'<script>SMS text</script>',timestamp:'2026-10-10',direction:'incoming',unread:true,readStateSource:'plugin'}]}})
  if (options.method === 'sms_read_state') return Promise.resolve({ok:true,data:{saved:true,id:'a'.repeat(64),unread:false}})
  if (options.method === 'sms_forward_settings') return Promise.resolve({ok:true,data:{enabled:false,tokenSet:true,pollSeconds:5,minimumPollSeconds:3,runtime:{state:'disabled'}}})
  if (options.method === 'sms_prepare') return Promise.resolve({ok:true,data:{confirmation:'once',warning:'确认短信操作'}})
  if (['sms_refresh','sms_execute','sms_forward_configure'].includes(options.method)) {
    const job='sms-job-'+nextReceipt++;receipts.set(job,{state:'done',result:{accepted:true}});return Promise.resolve({ok:true,data:{job,state:'pending'}})
  }
  return Promise.resolve({ ok: true, data: options.method === 'status' ? status : options.method === 'settings' ? { interval: 3, allowUnsafeAt: false } : {} })
} }
const context = { E, rpc, view: { extend: value => value }, poll: { add: () => {} },
  L: { resource: value => '/luci-static/resources' + (/^(?:[a-zA-Z0-9_.%,;-]+\/)*[a-zA-Z0-9_.%,;-]+$/.test(value) ? '/' + value : '/'), url: value => '/cgi-bin/luci/' + value, hasViewPermission: () => true },
  ui: { addNotification: (...args) => notifications.push(args), showModal: (title, nodes, ...classes) => { assert.deepEqual(classes,['np-root','np-dialog']); modal = E('div', {class:classes.join(' ')}, nodes); modals.push(title); return modal }, hideModal: () => {} }, setTimeout, Date, Promise, Number, Array, String, JSON, Error }
const {VERSION,VIEW}=require('./package-source.cjs')
context.window={}
vm.runInNewContext(fs.readFileSync(path.join(root,'www/luci-static/resources/wlmb-server/webui-data-'+VERSION+'.js'),'utf8'),context)
const source = fs.readFileSync(path.join(root, 'www/luci-static/resources/view/wlmb-server/'+VIEW+'.js'), 'utf8')
const view = vm.runInNewContext('(function(){' + source + '\n})()', context)
const all = node => [node, ...node.children.filter(child => child instanceof Element).flatMap(all)]
async function main() {
  const loaded = await view.load()
  assert.equal(loaded[1].failure, '插件服务尚未启动')
  const dom = view.render(loaded)
  const stylesheet = all(dom).find(node => node.tag === 'link' && node.attrs.rel === 'stylesheet')
  const stylesheetUrl=new URL(stylesheet.attrs.href,'http://fixture.invalid')
  assert.equal(stylesheetUrl.pathname,'/luci-static/resources/wlmb-server/panel.css','Use actual LuCI resource path validation: append query outside L.resource()')
  assert.equal(stylesheetUrl.searchParams.get('v'),VERSION)
  assert.equal(stylesheetUrl.searchParams.get('rev'),'release1_0_7_r1','首版候选不能读取此前同版本的缓存资源')
  assert.ok(fs.readFileSync(path.join(root, 'www/luci-static/resources/wlmb-server/panel.css'), 'utf8').includes('.np-card{'))
  assert.ok(dom.textContent.includes('网络面板'))
  assert.ok(dom.textContent.includes('已停止'))
  assert.equal(view.startButton.disabled, false)
  assert.equal(view.stopButton.disabled, true)
  await view.control('start')
  assert.equal(view.startButton.disabled, true)
  assert.equal(view.stopButton.disabled, false)
  assert.ok(view.overview.textContent.includes('10%'))
  assert.ok(view.overview.textContent.includes('<script>not HTML</script>'))
  assert.ok(!view.overview.textContent.includes('RSSI'))
  assert.ok(view.cells.textContent.includes('当前服务小区'))
  assert.ok(view.cells.textContent.includes('-81 dBm'), 'Real serving measurements must not disappear in a center-only inventory')
  assert.ok(!all(dom).some(node => node.tag === 'header' || node.tag === 'h2' || node.tag === 'h3'), 'Do not inherit legacy theme header/heading clipping')
  const inventory = view.cellTable('载波', [{rat:'NR',role:'unknown',channel:630000,channelType:'center',band:'n78',dlBandwidthMHz:100,ulBandwidthMHz:100}])
  assert.ok(!all(inventory).some(node => node.tag === 'th' && node.textContent === 'RSRP'), 'All-unknown measurement columns must not look like a broken primary signal')
  assert.ok(!source.includes('innerHTML'))
  const lockNodes = all(view.pages[5])
  lockNodes.find(node => node.attrs['aria-label'] === '锁定制式').value = 'NR'
  lockNodes.find(node => node.attrs['aria-label'] === '锁定类型').value = 'cell'
  assert.ok(!lockNodes.some(node=>node.attrs['aria-label']==='锁定记录'),'Locking must not require raw text records')
  lockNodes.find(node=>node.attrs['aria-label']==='已读小区').value='0'
  lockNodes.find(node=>node.attrs['aria-label']==='已读小区').listeners.change()
  assert.equal(lockNodes.find(node=>node.attrs['aria-label']==='服务频点').value,'633984')
  assert.equal(lockNodes.find(node=>node.attrs['aria-label']==='NR SCS').value,'','Never guess unreported SCS')
  const missingScs=lockNodes.find(node=>node.attrs['aria-label']==='NR SCS')
  const missingScsRow=lockNodes.find(node=>node.children.includes(missingScs))
  assert.equal(missingScsRow.hidden,false,'Only the missing SCS supplement is shown')
  assert.equal(lockNodes.find(node=>node.tag==='details'&&node.attrs.class==='np-lock-advanced').open,false)
  assert.equal(lockNodes.find(node=>node.attrs['aria-label']==='小区频段').value,'n78')
  missingScs.value='1';missingScs.listeners.change()
  assert.equal(missingScsRow.hidden,true,'A completed supplement collapses without opening manual inputs')
  assert.ok(view.pages[5].textContent.includes('PCI 505'))
  assert.ok(!calls.some(row=>row.method==='lock_prepare'||row.method==='lock_execute'),'Picking and filling a missing SCS never sends a write')
  const originalRadio = view.lockSnapshot
  view.lockSnapshot = {...originalRadio,serving:[{rat:'LTE',band:'B3',channel:1300,pci:25,channelType:'center'}],neighbors:[
    {rat:'LTE',band:'B3',channel:1350,pci:26,channelType:'center'},
    {rat:'LTE',band:'B3',channel:1400,pci:27,channelType:'center',stale:true}]}
  const lockRat=lockNodes.find(node=>node.attrs['aria-label']==='锁定制式'),choice=lockNodes.find(node=>node.attrs['aria-label']==='已读小区')
  lockRat.value='LTE';lockRat.listeners.change()
  assert.equal(choice.children.length,3,'LTE EARFCN is a valid center channel; discard stale cells only')
  choice.value='0';choice.listeners.change()
  assert.equal(lockNodes.find(node=>node.attrs['aria-label']==='服务频点').value,'1300')
  view.lockSnapshot=originalRadio;lockRat.value='NR';lockRat.listeners.change()
  choice.value='0';choice.listeners.change()
  missingScs.value='1';missingScs.listeners.change()
  assert.ok(lockNodes.find(node=>node.tag==='details'&&node.attrs.class==='np-lock-result'),'Raw lock state is optional diagnostic detail')
  const save = lockNodes.find(node => node.tag === 'button' && node.textContent === '检查并保存')
  async function waitUntil(condition) {
    const deadline = Date.now() + 5000
    while (!condition()) { assert.ok(Date.now() < deadline,'Fixture did not settle'); await new Promise(resolve => setTimeout(resolve,10)) }
  }
  view.interval.value = 7
  const beforeSave=calls.filter(call=>call.method==='configure').length
  view.saveButton.listeners.click()
  assert.equal(view.saveButton.attrs['aria-busy'],'true')
  view.saveButton.disabled=false;view.saveButton.listeners.click()
  await waitUntil(() => !view.saveButton.disabled)
  await waitUntil(() => !view.saveButton.busy)
  assert.equal(calls.filter(call=>call.method==='configure').length,beforeSave+1,'Busy guard survives disabled-state redraw')
  assert.equal(view.saveButton.attrs['aria-busy'],'false','Settled action releases unified loading state')
  assert.equal(calls.findLast(row => row.method==='configure').args[0],7)
  assert.ok(notifications.some(row=>row[1].textContent.includes('设置已保存')),'Settings save resolves its asynchronous receipt')
  const consoleNodes=all(view.pages[2])
  consoleNodes.find(node=>node.attrs['aria-label']==='AT 指令').value='AT+CSQ'
  const consoleSend=consoleNodes.find(node=>node.tag==='button' && node.textContent==='发送指令')
  consoleSend.listeners.click()
  await waitUntil(()=>!consoleSend.disabled)
  assert.equal(consoleNodes.find(node=>node.attrs['aria-label']==='指令响应').value,'synthetic modem response')
  assert.equal(calls.findLast(row=>row.method==='at_execute').args[2],'console-once')
  save.listeners.click()
  await waitUntil(() => modals.at(-1) === '确认锁定配置？')
  assert.deepEqual(JSON.parse(calls.findLast(row => row.method === 'lock_prepare').args[1]),{rat:'NR',mode:'cell',entries:[{band:78,arfcn:633984,pci:505,scsType:1}]})
  all(modal).find(node => node.tag === 'button' && node.textContent === '取消').click()
  await waitUntil(() => !save.disabled)
  assert.equal(calls.filter(row => row.method === 'lock_execute').length,0)
  const previousModals = modals.length
  save.listeners.click()
  await waitUntil(() => modals.length > previousModals)
  all(modal).find(node => node.tag === 'button' && node.textContent === '理解风险并保存').click()
  await waitUntil(() => !save.disabled)
  assert.equal(calls.filter(row => row.method === 'lock_execute').length,1)
  assert.ok(lockNodes.find(node => node.attrs['aria-label'] === '锁定状态和结果').value.includes('unverified'))
  let wifiOperation
  const hostAction = view.hostAction
  view.hostAction = operation => { wifiOperation=operation;return Promise.resolve() }
  view.editWifi({band:'5g',ssid:'original',enabled:true,mode:'HE',modeSupported:true,modeOptions:[{value:'HE',label:'WiFi 6'},{value:'EHT',label:'WiFi 7'}],
    authMode:'sae-mixed',securitySupported:true,securityOptions:[{value:'sae-mixed',label:'WPA2 / WPA3'}],
    bandwidth:'HE160',bandwidthSupported:true,bandwidthOptions:[{value:'HE80',mode:'HE',label:'80 MHz'},{value:'HE160',mode:'HE',label:'160 MHz'},{value:'EHT80',mode:'EHT',label:'80 MHz'}]})
  const generation=all(modal).find(node=>node.attrs['aria-label']==='无线代际'),width=all(modal).find(node=>node.attrs['aria-label']==='WiFi 频宽')
  assert.equal(width.children.length,2)
  generation.value='EHT';generation.listeners.change()
  assert.equal(width.value,'EHT80');assert.equal(width.children.length,1)
  assert.ok(!width.textContent.includes('HE')&&!width.textContent.includes('EHT'))
  all(modal).find(node=>node.tag==='button'&&node.textContent==='下一步').listeners.click()
  await waitUntil(()=>wifiOperation)
  assert.equal(wifiOperation.changes.mode,'EHT');assert.equal(wifiOperation.changes.bandwidth,'EHT80')
  view.hostAction=hostAction
  view.editWifi({band:'2g',ssid:'original',enabled:true})
  all(modal).find(node => node.attrs['aria-label'] === 'WiFi 名称').value = 'new name'
  const password = all(modal).find(node => node.attrs['aria-label'] === 'WiFi 新密码')
  password.value = 'new-password'
  all(modal).find(node => node.tag === 'button' && node.textContent === '下一步').listeners.click()
  await waitUntil(() => modals.at(-1) === '保存设备设置？')
  assert.equal(password.value, '', 'Clear the edit field after creating the typed operation')
  assert.deepEqual(JSON.parse(calls.findLast(row => row.method === 'host_prepare').args[0]),{kind:'wifi_edit',band:'2g',changes:{ssid:'new name',password:'new-password'}})
  all(modal).find(node => node.tag === 'button' && node.textContent === '取消').click()
  await new Promise(resolve => setTimeout(resolve,10))
  assert.equal(calls.filter(row => row.method === 'host_execute').length,0, 'Cancel never changes wireless')
  view.editBudget({enabled:false,limitBytes:0,startDay:1,threshold:90,cutoffSupported:false})
  all(modal).find(node => node.attrs['aria-label'] === '启用月限额').checked = true
  all(modal).find(node => node.attrs['aria-label'] === '月限额 GB').value = '10'
  all(modal).find(node => node.attrs['aria-label'] === '统计起始日').value = '15'
  const cutoff = all(modal).find(node => node.attrs['aria-label'] === '达到限额停止转发')
  assert.equal(cutoff.disabled,true)
  cutoff.checked = true
  all(modal).find(node => node.tag === 'button' && node.textContent === '下一步').listeners.click()
  await waitUntil(() => modals.at(-1) === '保存设备设置？')
  assert.deepEqual(JSON.parse(calls.findLast(row => row.method === 'host_prepare').args[0]),{kind:'traffic_limit',enabled:true,limitBytes:10737418240,startDay:15,threshold:90,cutoffEnabled:false})
  all(modal).find(node => node.tag === 'button' && node.textContent === '确认保存').click()
  await waitUntil(() => calls.filter(row => row.method === 'host_execute').length === 1)
  await new Promise(resolve => setTimeout(resolve,10))
  context.L.hasViewPermission = () => false
  view.readonly = true
  const hostCalls = calls.filter(row => row.method.startsWith('host_')).length
  await assert.rejects(view.hostAction({kind:'wifi_unify',enabled:true}), /写权限/)
  assert.equal(calls.filter(row => row.method.startsWith('host_')).length,hostCalls)
  view.readonly = false
  context.L.hasViewPermission = () => true
  await view.control('boot', false)
  assert.equal(view.boot.input.checked, false)
  const cancel = view.confirm('stop')
  all(modal).find(node => node.tag === 'button' && node.textContent === '取消').click()
  await cancel
  assert.equal(running, true)
  const confirm = view.confirm('stop')
  all(modal).find(node => node.tag === 'button' && node.textContent === '确认').click()
  await confirm
  assert.equal(running, false)
  assert.equal(view.startButton.disabled, false)
  assert.equal(calls.filter(call => call.method === 'service_control' && call.args[0] === 'stop').length, 1, 'No retry/replay of service writes')
  const tabs = all(dom).filter(node => node.attrs.role === 'tab')
  tabs[1].listeners.click()
  assert.equal(view.pages[1].hidden, false)
  assert.equal(view.pages[0].hidden, true)
  assert.equal(tabs[1].attrs['aria-selected'], 'true')
  const menu = JSON.parse(fs.readFileSync(path.join(root, 'usr/share/luci/menu.d/network-panel.json')))
  assert.equal(menu['admin/services/wlmb-server'].title, '网络面板')
  assert.equal(menu['admin/services/wlmb-server'].action.path, 'wlmb-server/'+VIEW)
  const acl = JSON.parse(fs.readFileSync(path.join(root, 'usr/share/rpcd/acl.d/network-panel.json')))['luci-app-network-panel']
  assert.ok(acl.read.ubus['network-panel'].includes('service_status'))
  assert.ok(!acl.read.ubus['network-panel'].includes('service_control'))
  assert.ok(acl.write.ubus['network-panel'].includes('service_control'))
  for (const name of ['host_prepare','host_execute']) {
    assert.ok(acl.write.ubus['network-panel'].includes(name))
    assert.ok(!acl.read.ubus['network-panel'].includes(name))
  }
  const help = fs.readFileSync(path.join(root, 'www/luci-static/resources/wlmb-server/guide.html'), 'utf8')
  for (const name of ['connect', 'radio', 'help']) assert.ok(help.includes('id="' + name + '"'))
  assert.ok(help.includes('功能按模组实际能力开放') && help.includes('设置 → 短信转发') && help.includes('连接只需三步'), 'Concise tutorial guides mini-program use without claiming every modem is validated')
  assert.ok(!/https?:\/\//i.test(help), 'Tutorial uses local assets only')
  assert.ok(help.includes('src="guide.js?v=') && [...help.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].every(row=>!row[1].trim()), 'Only the bounded local tutorial script is allowed')
  view.runtime = {...view.runtime,running:true,controlReady:true,backend:'rust'}
  view.draw({...status,backend:'rust',capabilities:{configure:false,at:false,wifi:false,lockBand:false}})
  assert.equal(view.saveButton.disabled,true)
  for (const index of [2,4]) assert.equal(view.tabs.children[index].hidden,true,'Unintegrated native controls must be hidden')
  assert.equal(view.tabs.children[5].hidden,false,'Known band inventory remains visible without write authority')
  assert.ok(all(view.pages[5]).find(node=>node.tag==='button'&&node.textContent==='检查并保存').disabled)
  view.draw({...status,backend:'rust',capabilities:{configure:false,wifi:true,deviceList:true,at:false,lockBand:false}})
  assert.equal(view.tabs.children[4].hidden,false,'Native LAN reading opens WiFi/device status without authorizing writes')
  assert.equal(view.tabs.children[2].hidden,true)
  assert.equal(view.tabs.children[5].hidden,false)
  const oldLan = {stale:true,deviceListSupported:true,devices:[{name:'OLD_STATION_SHOULD_NOT_SHOW',mac:'02:00:00:00:00:01',ip:'192.168.1.2'}],wireless:[{ssid:'OLD_RUNTIME_SHOULD_NOT_SHOW'}],controls:{stale:false,profiles:[{id:'main',band:'5g',ssid:'Configured AP',enabled:true}]}};
  view.draw({...status,backend:'rust',system:{...status.system,lan:oldLan}})
  assert.ok(view.networks.textContent.includes('Configured AP'),'Keep independently verified configuration editable')
  assert.ok(view.networks.textContent.includes('设备快照已过期'))
  assert.ok(!view.networks.textContent.includes('OLD_STATION_SHOULD_NOT_SHOW'))
  assert.ok(!view.networks.textContent.includes('OLD_RUNTIME_SHOULD_NOT_SHOW'))
  view.draw({...status,backend:'rust',system:{...status.system,lan:{...oldLan,stale:false,complete:false,devices:[],wireless:[]}}})
  assert.ok(view.networks.textContent.includes('设备列表尚未完整读取'))
  assert.ok(!view.networks.textContent.includes('暂未发现在线设备'))
  running=true
  status.backend='rust';status.capabilities={sms:true,smsSend:true,smsDelete:true,smsForward:true}
  await view.update(true)
  assert.equal(view.tabs.children[6].hidden,false);assert.equal(view.tabs.children[7].hidden,false)
  view.editWifi({band:'5g',ssid:'Panel',enabled:true,channel:'36',bandwidth:'EHT160',channelSupported:true,bandwidthSupported:true,channelOptions:[{value:'36',label:'信道 36'},{value:'40',label:'信道 40'}],bandwidthOptions:[{value:'EHT80',label:'80 MHz'},{value:'EHT160',label:'160 MHz'}]})
  all(modal).find(node=>node.attrs['aria-label']==='WiFi 信道').value='40'
  all(modal).find(node=>node.attrs['aria-label']==='WiFi 频宽').value='EHT80'
  all(modal).find(node=>node.tag==='button'&&node.textContent==='下一步').listeners.click()
  await waitUntil(()=>modals.at(-1)==='保存设备设置？')
  assert.deepEqual(JSON.parse(calls.findLast(row=>row.method==='host_prepare').args[0]),{kind:'wifi_edit',band:'5g',changes:{channel:'40',bandwidth:'EHT80'}})
  all(modal).find(node=>node.tag==='button'&&node.textContent==='取消').click()
  await new Promise(resolve=>setTimeout(resolve,10))
  await view.readSms(true);assert.ok(view.smsRows.textContent.includes('<script>SMS text</script>'))
  assert.ok(!all(view.smsRows).some(node => node.tag === 'script'))
  const mark=all(view.smsRows).find(node=>node.tag==='button'&&node.textContent==='标为已读')
  assert.ok(mark);mark.listeners.click();await waitUntil(()=>calls.some(row=>row.method==='sms_read_state'))
  assert.equal(calls.filter(row=>row.method==='sms_refresh').length,1,'Read-state changes only reread cache, not the modem')
  const deletion=view.deleteSms('test','a'.repeat(64));await waitUntil(() => modals.at(-1)==='删除这条短信？')
  all(modal).find(node => node.tag === 'button' && node.textContent === '取消').click();await deletion
  assert.equal(calls.filter(row => row.method==='sms_execute').length,0)
  await view.readForward();assert.equal(view.forwardToken.value,undefined,'No saved secret is returned to a password field')
  assert.equal(Number(view.forwardInterval.value),5);assert.equal(Number(view.forwardInterval.min),3)
  view.forwardToken.value='FixturePushToken123456';view.forwardInterval.value=3;view.forwardEnabled.input.checked=true
  view.forwardSaveButton.listeners.click();await waitUntil(() => calls.some(row => row.method==='sms_forward_configure'))
  assert.equal(calls.find(row=>row.method==='sms_forward_configure').args[2],3,'The new interval must reach the RPC request')
  view.draw(status);assert.equal(view.forwardSaveButton.disabled,true,'Periodic cache redraw must not re-enable an in-flight forwarding write')
  await waitUntil(() => !view.forwardSaveButton.disabled)
  assert.equal(view.forwardToken.value,'','Clear secret input before sending configuration')
  view.readonly=true
  const beforeReadonly=calls.length
  await assert.rejects(view.deleteSms('test','a'.repeat(64)),/写权限/)
  assert.equal(calls.length,beforeReadonly,'Read-only state blocks before preparing a SMS mutation')
  view.readonly=false;view.draw({...status,capabilities:{sms:true,smsSend:false,smsDelete:false,smsForward:false},smsMode:'compat-read-only'})
  assert.equal(view.smsSendButton.disabled,true);assert.equal(view.tabs.children[6].hidden,false);assert.equal(view.tabs.children[7].hidden,false)
  assert.equal(view.forwardSaveButton.disabled,true);assert.ok(view.forwardUnavailable.textContent.includes('未开放转发'))
  for (const name of ['sms_refresh','sms_read_state','sms_prepare','sms_execute','sms_forward_configure']) {assert.ok(acl.write.ubus['network-panel'].includes(name));assert.ok(!acl.read.ubus['network-panel'].includes(name))}
  console.log('OpenWrt LuCI fixture checks passed: offline controls, state, cancel/confirm, no replay, tabs, ACL, tutorial. Not visual/live LuCI validation.')
}
main().catch(error => { console.error(error); process.exitCode = 1 })
