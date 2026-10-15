// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs=require('node:fs'),os=require('node:os'),path=require('node:path'),cp=require('node:child_process'),crypto=require('node:crypto')
const {build,VERSION}=require('./package-source.cjs')
const hash=b=>crypto.createHash('sha256').update(b).digest('hex')
function main(){
  const [binary,arch,abi,compilerPackage]=process.argv.slice(2)
  if(process.platform!=='linux'||!binary||!['x86_64','aarch64'].includes(arch)||!abi||!compilerPackage)throw new Error('Linux build requires binary, architecture, package ABI and pinned compiler package')
  const compilerBytes=fs.readFileSync(compilerPackage)
  if(hash(compilerBytes)!=='2edccd3267ce540f8d2371a0f394e84b40d8348ecc28425309e6d07079ed1259')throw new Error('APK build compiler checksum mismatch')
  const {tar,ipk,manifest}=build(binary,arch,abi)
  const stage=fs.mkdtempSync(path.join(os.tmpdir(),'wlmb-package-'))
  const tool=path.join(stage,'apk.static'),archive=path.join(stage,'payload.tar.gz'),output=path.join(stage,'wlmb-server.apk')
  fs.writeFileSync(tool,cp.execFileSync('tar',['-xOf',path.resolve(compilerPackage),'sbin/apk.static'],{maxBuffer:16000000}),{mode:0o755})
  fs.writeFileSync(archive,tar)
  cp.execFileSync('tar',['-xzf',archive,'-C',stage])
  cp.execFileSync(tool,['mkpkg','--compression','deflate:9','--compat','3.0.5','--info','name:wlmb-server','--info','version:'+VERSION,'--info','arch:'+manifest.packageArchitecture,'--info','license:LicenseRef-WLMB-NonCommercial','--info','description:Network Panel native Rust service','--info','maintainer:SlAlot','--info','depends:rpcd rpcd-mod-iwinfo luci-base uci','--script','post-install:'+path.join(stage,'hooks/post-install'),'--script','post-upgrade:'+path.join(stage,'hooks/post-upgrade'),'--files',path.join(stage,'files'),'--output',output],{stdio:'inherit'})
  cp.execFileSync(tool,['verify','--allow-untrusted',output],{stdio:'inherit'})
  const payload=fs.readFileSync(output),dist=path.resolve(__dirname,'../dist')
  fs.mkdirSync(dist,{recursive:true});manifest.packageSha256=hash(payload);manifest.packageBytes=payload.length
  fs.writeFileSync(path.join(dist,`wlmb-server-${VERSION}-${arch}.apk`),payload)
  fs.writeFileSync(path.join(dist,`wlmb-server-${VERSION}-${arch}.ipk`),ipk)
  fs.writeFileSync(path.join(dist,`manifest-${arch}.json`),JSON.stringify(manifest,null,2)+'\n')
  console.log(JSON.stringify({version:VERSION,arch,abi:manifest.packageArchitecture,apkBytes:payload.length,ipkBytes:ipk.length,apkSha256:manifest.packageSha256}))
}
main()
