// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto'),{execFileSync}=require('node:child_process')
const root=path.resolve(__dirname,'..'),relative='application/qmodem/files/usr/share/qmodem/modem_support.json'
function bands(value) {
  if(value==null||value===''||value==='0')return []
  if(typeof value!=='string'||!/^\d+(?:\/\d+)*$/.test(value))throw Error('Unreviewed band format')
  const out=[...new Set(value.split('/').map(Number))].sort((a,b)=>a-b)
  if(out.length>128||out.some(n=>!Number.isInteger(n)||n<1||n>1024))throw Error('Invalid band facts')
  return out
}
function facts(document) {
  const out=[]
  for(const[transport,models]of Object.entries(document.modem_support||{})) {
    if(transport==='device')continue
    if(!['usb','pcie'].includes(transport))throw Error('Unknown transport')
    for(const[model,row]of Object.entries(models))out.push({model:model.toLowerCase().replaceAll('_','-'),transport,LTE:bands(row.lte_band),NRSA:bands(row.sa_band),NRNSA:bands(row.nsa_band)})
  }
  return out.sort((a,b)=>(a.model+':'+a.transport).localeCompare(b.model+':'+b.transport))
}
if(require.main===module) {
  const checkout=path.resolve(process.argv[2]||''),git=args=>execFileSync('git',['-C',checkout,...args],{encoding:'utf8',timeout:20000,maxBuffer:2*1024*1024}).trim()
  const baseline=JSON.parse(fs.readFileSync(path.join(root,'docs/qmodem-upstream.json'),'utf8'))
  if(!process.argv[2]||git(['rev-parse','HEAD'])!==baseline.revision||git(['status','--porcelain','--untracked-files=no']))throw Error('Reviewed clean upstream revision required')
  const raw=execFileSync('git',['-C',checkout,'show',baseline.revision+':'+relative],{maxBuffer:2*1024*1024})
  const sha256=crypto.createHash('sha256').update(raw).digest('hex')
  if(sha256!==baseline.files[relative])throw Error('Upstream facts drift requires review')
  const document={schema:'WLMB-MODEL-BANDS/1',revision:baseline.revision,sourceSha256:sha256,scope:'hardware-facts-not-lock-protocol-support',profiles:facts(JSON.parse(raw))}
  fs.writeFileSync(path.join(root,'docs/model-bands.json'),JSON.stringify(document,null,2)+'\n')
  console.log('Imported reviewed band facts: '+document.profiles.length+' transport profiles')
}
module.exports={facts,bands}
