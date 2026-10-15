// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs=require('node:fs'),path=require('node:path')
const {coverage}=require('./qmodem-audit.cjs')
const required=['radio','bandLock','cellLock','networkPreference','cellularEnable','sms','smsSharedTransactions','smsNotifications']
function report(upstream,catalog) {
  const profiles=coverage(upstream,catalog)
  const missingIdentity=profiles.filter(row=>!row.identified||!row.mappingMatches)
  const protocolGaps=[]
  for(const row of profiles) for(const feature of required) {
    const state=row.features[feature]
    const implemented=typeof state==='string' && (state.startsWith('implemented')||state==='fixture-partial'||feature==='smsNotifications'&&state==='bounded-owned-endpoint-cmti-or-poll-fallback')
    if(!implemented) protocolGaps.push({model:row.model,transport:row.transport,platform:row.platform,feature,state:state||'missing'})
  }
  return {schema:'WLMB-RELEASE-READINESS/1',referenceRevision:upstream.revision,identifiedModels:new Set(profiles.filter(row=>row.identified).map(row=>row.model)).size,transportProfiles:profiles.length,missingIdentity,protocolGaps,protocolComplete:profiles.length>0&&missingIdentity.length===0&&protocolGaps.length===0,hardwareComplete:profiles.length>0&&profiles.every(row=>row.hardwareAccepted),scope:'offline-code-coverage-not-hardware-certification',profiles}
}
if(require.main===module) {
  const args=process.argv.slice(2),strict=args.includes('--require-protocol-complete'),files=args.filter(arg=>arg!=='--require-protocol-complete')
  if(files.length!==2) throw new Error('Usage: node tools/release-readiness.cjs CATALOG.json REPORT.json [--require-protocol-complete]')
  const upstream=JSON.parse(fs.readFileSync(path.join(__dirname,'../docs/qmodem-upstream.json'),'utf8'))
  const result=report(upstream,JSON.parse(fs.readFileSync(files[0],'utf8')))
  fs.writeFileSync(files[1],JSON.stringify(result,null,2)+'\n')
  console.log(JSON.stringify({models:result.identifiedModels,profiles:result.transportProfiles,protocolGaps:result.protocolGaps.length,protocolComplete:result.protocolComplete,hardwareComplete:result.hardwareComplete}))
  if(strict&&!result.protocolComplete) process.exitCode=1
}
module.exports={report}
