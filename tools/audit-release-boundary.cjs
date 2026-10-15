'use strict'
// 发布审计只输出文件位置和风险类型，不输出秘密值；历史检查不改写Git。
const fs=require('node:fs'),path=require('node:path'),cp=require('node:child_process'),assert=require('node:assert/strict')
const root=path.resolve(__dirname,'..'),issues=[],historyIssues=[]
const repositoryIndex=process.argv.indexOf('--repository')
const repository=repositoryIndex>=0?path.resolve(process.argv[repositoryIndex+1]||''):root
const excluded=new Set(['.git','node_modules','target','tmp','dist'])
const secret=/(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|--token=[A-Za-z0-9_-]{20,}|sk-(?:proj-)?[A-Za-z0-9_-]{35,})/
const commercial=/mini-program\/|license-server\/|cloudfunctions\/|license[-_]wasm|license_public_key|license-signing-key/i
const product=/^(?:backend\/src\/|frontend\/)/
const oldAsset=/(?:webui-(?:reference|vendor)-|import-reference-pages|(?:^|\/)package\/network-panel\/|\.py$)/
function walk(directory){return fs.readdirSync(directory,{withFileTypes:true}).flatMap(entry=>{
 if(excluded.has(entry.name))return []
 const file=path.join(directory,entry.name);if(entry.isSymbolicLink()){issues.push({path:path.relative(root,file),type:'源码符号链接'});return []}
 return entry.isDirectory()?walk(file):[file]
})}
const files=walk(root),md=new Set(files.filter(x=>x.endsWith('.md')).map(x=>path.resolve(x)))
for(const file of files){
 const name=path.relative(root,file).split(path.sep).join('/'),text=fs.readFileSync(file,'utf8')
 if(secret.test(text))issues.push({path:name,type:'疑似秘密凭据'})
 if(commercial.test(name)||/\.(?:py|pyc|pyo|wasm)$/i.test(name)||oldAsset.test(name))issues.push({path:name,type:'非插件发布文件'})
 if(product.test(name)&&commercial.test(text))issues.push({path:name,type:'商业客户端或授权实现引用'})
 if(name.endsWith('.md'))for(const match of text.matchAll(/\]\(([^)]+)\)/g)){
  const link=match[1];if(/^(?:https?:|#|mailto:)/.test(link))continue
  const target=path.resolve(path.dirname(file),link.split('#')[0])
  if(!fs.existsSync(target))issues.push({path:name,type:'失效文档链接',target:link})
 }
}
const notice=fs.readFileSync(path.join(root,'frontend/www/luci-static/resources/wlmb-server/ui-notices.html'),'utf8')
assert.ok(notice.includes('Copyright (c) 2016 Vladimir Kharlampidi')&&notice.includes('Copyright (c) 2026 Lucide Icons and Contributors'),'安装包保留第三方图标许可')
let history=null
if(process.argv.includes('--history')){
 const git=(args,input)=>cp.execFileSync('git',['-C',repository,...args],{input,maxBuffer:128*1024*1024,encoding:input===undefined?'utf8':undefined})
 const objects=git(['rev-list','--objects','--all']).trim().split('\n').map(line=>{const index=line.indexOf(' ');return {id:index<0?line:line.slice(0,index),name:index<0?'':line.slice(index+1)}})
 const catalog=git(['cat-file','--batch-check=%(objectname) %(objecttype) %(objectsize)'],objects.map(x=>x.id).join('\n')+'\n').toString().trim().split('\n')
 const names=new Map(objects.map(x=>[x.id,x.name])),ids=[]
 for(const row of catalog){const [id,type,size]=row.split(' ');if(type==='blob'){if(Number(size)>8*1024*1024){historyIssues.push({path:names.get(id),type:'大历史对象未扫描',id});continue}ids.push(id)}}
 const raw=git(['cat-file','--batch'],ids.join('\n')+'\n');let offset=0,scanned=0
 for(const id of ids){
  const end=raw.indexOf(10,offset);if(end<0)throw Error('历史对象格式错误')
  const parts=raw.subarray(offset,end).toString().split(' '),length=Number(parts[2]);assert.equal(parts[0],id)
  const name=names.get(id)||'',text=raw.subarray(end+1,end+1+length).toString('utf8');offset=end+1+length+1;scanned++
  if(secret.test(text))historyIssues.push({path:name,type:'历史疑似秘密凭据',id})
  if(commercial.test(name))historyIssues.push({path:name,type:'历史商业目录',id})
  if(oldAsset.test(name))historyIssues.push({path:name,type:'历史旧实现／移植资产',id})
 }
 const messages=git(['log','--all','--format=%B']);if(/Co-authored-by:.*(?:codex|openai)/i.test(messages))historyIssues.push({type:'历史共同作者需复核'})
 history={scannedBlobs:scanned,issues:historyIssues,publicHistoryReady:historyIssues.length===0}
}
const report={schema:'WLMB-RELEASE-BOUNDARY/1',currentFiles:files.length,currentClean:issues.length===0,issues,history,visibility:'须另核对GitHub，工具不更改可见性',scope:'源码与历史静态扫描，不代替完整渗透或第三方许可法律复核'}
if(process.argv.includes('--summary')){if(history)report.history={scannedBlobs:history.scannedBlobs,issueCount:history.issues.length,publicHistoryReady:history.publicHistoryReady,types:[...new Set(history.issues.map(x=>x.type))]}}
console.log(JSON.stringify(report,null,2));if(issues.length)process.exitCode=1
