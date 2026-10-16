'use strict'
// 兼容报告来自已编译目录，不把身份映射、字段门禁和硬件验收混为一谈。
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict')
const {report}=require('./release-readiness.cjs'),{VERSION}=require('./package-source.cjs')
function compatibility(upstream,catalog){
 const readiness=report(upstream,catalog),models=catalog.models
 const features={}
 for(const row of models){
  assert.equal(typeof row.model,'string');assert.ok(row.features&&typeof row.features==='object')
  for(const [feature,state] of Object.entries(row.features)){
   assert.equal(typeof state,'string')
   const entry=features[feature]??={states:{}}
   ;(entry.states[state]??=[]).push(row.model)
  }
 }
 for(const entry of Object.values(features))for(const names of Object.values(entry.states))names.sort()
 return {schema:'WLMB-COMPATIBILITY-AUDIT/1',version:VERSION,referenceRevision:upstream.revision,scope:'compiled-code-and-runtime-gates-not-hardware-certification',
  summary:{models:models.length,transportProfiles:readiness.transportProfiles,identityGaps:readiness.missingIdentity.length,protocolComplete:readiness.protocolComplete,hardwareComplete:readiness.hardwareComplete},
  modes:{shared:'相同解析器；读写取决于已核验QModem共享接口、事务或短信服务',glinet:'动态发现CPU／USB共享AT并复用相同目录；PDU只读与转发，无已核验短信事件，短信发送／删除和锁定写入关闭',independent:'相同解析器；必须有核验为空闲的AT端点及固件能力'},features,readiness,models}
}
module.exports={compatibility}
if(require.main===module){
 const [input,output]=process.argv.slice(2)
 if(!input||!output)throw new Error('需要目录JSON（或标准输入-）与输出报告路径')
 const upstream=JSON.parse(fs.readFileSync(path.join(__dirname,'../docs/qmodem-upstream.json'),'utf8'))
 const value=compatibility(upstream,JSON.parse(fs.readFileSync(input==='-'?0:input,'utf8')))
 fs.writeFileSync(output,JSON.stringify(value,null,2)+'\n')
 console.log(JSON.stringify(value.summary))
}
