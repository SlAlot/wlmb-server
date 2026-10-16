// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const assert=require('node:assert/strict'),{report}=require('./release-readiness.cjs')
const {compatibility}=require('./compatibility-report.cjs')
const upstream={revision:'reference',models:[{model:'m1',transport:'usb',vendor:'quectel',platform:'qualcomm'},{model:'m1',transport:'pcie',vendor:'quectel',platform:'qualcomm'}]}
const catalog={schema:'WLMB-PROTOCOL-COVERAGE/1',models:[{model:'m1',family:'quectel',platform:'qualcomm',parser:'x',features:{radio:'fixture-partial',cellLock:'implemented-unverified-write',sms:'standard-pdu-needs-firmware-negotiation'},hardwareAccepted:false}]}
let result=report(upstream,catalog)
assert.equal(result.identifiedModels,1);assert.equal(result.transportProfiles,2)
assert.equal(result.protocolComplete,false);assert.equal(result.hardwareComplete,false)
assert.ok(result.protocolGaps.some(row=>row.feature==='bandLock'&&row.state==='missing'))
assert.ok(result.protocolGaps.some(row=>row.feature==='sms'))
assert.ok(!result.protocolGaps.some(row=>row.feature==='cellLock'))
assert.equal(report(upstream,{schema:catalog.schema,models:[]}).missingIdentity.length,2)
for(const key of ['radio','bandLock','cellLock','networkPreference','cellularEnable','sms','smsSharedTransactions','smsNotifications'])catalog.models[0].features[key]='implemented'
result=report(upstream,catalog);assert.equal(result.protocolComplete,true);assert.equal(result.hardwareComplete,false)
assert.throws(()=>report(upstream,{schema:'unknown',models:[]}))
assert.equal(report({revision:'empty',models:[]},{schema:catalog.schema,models:[]}).protocolComplete,false)
const audit=compatibility(upstream,catalog)
assert.equal(audit.summary.models,1);assert.equal(audit.summary.transportProfiles,2)
assert.equal(audit.summary.hardwareComplete,false)
assert.deepEqual(audit.features.bandLock.states.implemented,['m1'])
assert.ok(audit.modes.shared.includes('事务')&&audit.modes.independent.includes('空闲'))
assert.ok(audit.modes.glinet.includes('只读')&&audit.modes.glinet.includes('写入关闭'))
console.log('Release gate: identity, bus/platform coverage, conditional SMS, missing protocols and hardware evidence stay separate.')
