'use strict'
// 电池接口和前端展示回归；使用合成状态，不修改运行设备。
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),assert=require('node:assert/strict')
const {VERSION}=require('./package-source.cjs'),root=path.resolve(__dirname,'..')
const context={window:{}}
vm.runInNewContext(fs.readFileSync(path.join(root,'frontend/www/luci-static/resources/wlmb-server/webui-data-'+VERSION+'.js'),'utf8'),context)
const battery=context.window.WLMBUIData.battery
const state=(percent,charging)=>({system:{stale:false,battery:{supported:true,available:true,stale:false,percent,charging}}})
assert.equal(battery(state(0,false)).label,'0% · 未充电')
assert.equal(battery(state(76,true)).label,'76% · 充电中')
assert.equal(battery(state(50,null)).label,'50% · 充电状态未知')
for(const percent of [null,'76',-1,101,76.5])assert.equal(battery(state(percent,false)).available,false)
const stale=state(76,true);stale.system.battery.stale=true;assert.equal(battery(stale).available,false)
const old=state(76,true);old.system.stale=true;assert.equal(battery(old).available,false)
assert.equal(battery({}).supported,false)
const contract=JSON.parse(fs.readFileSync(path.join(root,'docs/api-contract.json'))).statusExtensions.battery
assert.equal(contract.path,'system.battery');assert.equal(contract.provider.maxAgeSeconds,60)
assert.equal(contract.provider.maxBytes,1024);assert.equal(contract.provider.path,'/var/run/wlmb-server/battery.json')
const service=fs.readFileSync(path.join(root,'backend/src/service.rs'),'utf8')
assert.match(service,/crate::battery::snapshot\(wall_ms\(\)\)/);assert.match(service,/crate::battery::age\(/)
const shell=fs.readFileSync(path.join(root,'frontend/www/luci-static/resources/wlmb-server/webui-shell-'+VERSION+'.js'),'utf8')
assert.match(shell,/power\.supported\?field\('设备电量',power.label\):null/)
assert.match(shell,/snapshot.system\?\.battery/)
console.log('电池字段、能力、新鲜度与WebUI展示回归通过；不代表全部硬件验收。')
