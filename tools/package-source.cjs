// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto'), zlib = require('node:zlib')
const root = path.resolve(__dirname, '..'), hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const { platform } = require('./platforms.cjs')
const VERSION = '1.1.0'
const VIEW = 'index-v' + VERSION.replace(/[.-]/g,'_') + '_release'
// 从语义版本生成固定时间戳，保证资源更新与归档可重复。
const match = /^(\d+)\.(\d+)\.(\d+)(?:-r(\d+))?$/.exec(VERSION)
if(!match) throw new Error('Invalid release version')
const parts = match.slice(1).map(value=>Number(value||0))
if(parts.some(n=>n>99)) throw new Error('Release timestamp version component overflow')
const ARCHIVE_MTIME = 1767225600 + parts.reduce((n,part)=>n*100+part,0)
function octal(header, offset, length, value) { header.write(value.toString(8).padStart(length-1,'0')+'\0',offset,length,'ascii') }
function archive(rows) {
  const all = [], names = new Set()
  for (const row of rows) {
    if(names.has(row.name) || row.name.length>99 || !/^[a-zA-Z0-9_./-]+$/.test(row.name) || row.name.split('/').includes('..')) throw new Error('Unsafe archive name')
    names.add(row.name)
    const h = Buffer.alloc(512), bytes = row.bytes || Buffer.alloc(0)
    h.write(row.name); octal(h,100,8,row.mode || 0o644); octal(h,108,8,0); octal(h,116,8,0)
    octal(h,124,12,row.link ? 0 : bytes.length); octal(h,136,12,ARCHIVE_MTIME)
    h.fill(32,148,156); h[156] = row.link ? 50 : row.directory ? 53 : 48
    if(row.link) h.write(row.link,157,100,'ascii')
    h.write('ustar\0',257); h.write('00',263); h.write('root',265); h.write('root',297)
    const sum = h.reduce((s,b)=>s+b,0); h.write(sum.toString(8).padStart(6,'0')+'\0 ',148,8,'ascii')
    all.push(h)
    if(bytes.length) { all.push(bytes); const padding=(512-bytes.length%512)%512; if(padding) all.push(Buffer.alloc(padding)) }
  }
  all.push(Buffer.alloc(1024))
  const compressed = zlib.gzipSync(Buffer.concat(all),{level:9})
  // 固定为目标系统的压缩头，避免构建宿主系统改变制品摘要。
  compressed[9] = 3
  return compressed
}
function build(binary,architecture,packageArchitecture=architecture) {
  const bytes = fs.readFileSync(binary), expected = {aarch64:183,x86_64:62}[architecture]
  if(!expected || bytes.toString('hex',0,4)!=='7f454c46' || bytes[4]!==2 || bytes[5]!==1 || bytes.readUInt16LE(18)!==expected) throw new Error('ELF architecture mismatch')
  if(!bytes.includes(Buffer.from(`WLMB_BUILD=${VERSION};pushplus-https=1`))) throw new Error('Stale or incomplete binary: release requires exact version and PushPlus HTTPS')
  platform(architecture, packageArchitecture)
  const source = [{name:'usr/sbin/wlmb-server',bytes,mode:0o755},{name:'usr/libexec/rpcd/network-panel',link:'/usr/sbin/wlmb-server',mode:0o755}]
  function tree(dir,prefix) {
    for(const entry of fs.readdirSync(dir,{withFileTypes:true})) {
      if(entry.isSymbolicLink()) throw new Error('Source symlink prohibited')
      const file = path.join(dir,entry.name), name=prefix ? prefix+'/'+entry.name : entry.name
      if(entry.isDirectory()) tree(file,name)
      else source.push({name,bytes:Buffer.from(fs.readFileSync(file,'utf8').replace(/\r\n/g,'\n')),mode:name==='etc/init.d/wlmb-server'?0o755:0o644})
    }
  }
  tree(path.join(root,'frontend'),''); tree(path.join(root,'packaging/files'),'')
  source.push({name:'usr/share/wlmb-server/LICENSE',bytes:Buffer.from(fs.readFileSync(path.join(root,'LICENSE'),'utf8').replace(/\r\n/g,'\n'))})
  const ipkSource = source.map(row=>({...row}))
  const metadata='lib/apk/packages/wlmb-server', config=source.find(r=>r.name==='etc/config/network_panel').bytes
  source.push({name:metadata+'.conffiles',bytes:Buffer.from('/etc/config/network_panel\n')})
  source.push({name:metadata+'.conffiles_static',bytes:Buffer.from('/etc/config/network_panel '+hash(config)+'\n')})
  source.push({name:metadata+'.list',bytes:Buffer.from([...source.map(r=>r.name),metadata+'.list'].sort().map(n=>'/'+n+'\n').join(''))})
  const post=Buffer.from('#!/bin/sh\n/etc/init.d/wlmb-server stop >/dev/null 2>&1 || true\nif [ "$(uci -q get network_panel.main.boot_enabled)" = "0" ]; then /etc/init.d/wlmb-server disable; else /etc/init.d/wlmb-server enable; fi\n/etc/init.d/rpcd reload >/dev/null 2>&1 || /etc/init.d/rpcd restart >/dev/null 2>&1\n/etc/init.d/wlmb-server start\n')
  const payload=source.map(row=>({...row,name:'files/'+row.name}))
  payload.push({name:'hooks/post-install',bytes:post,mode:0o755},{name:'hooks/post-upgrade',bytes:post,mode:0o755})
  const dirs=new Set()
  for(const r of payload) { let parent=path.posix.dirname(r.name); while(parent!=='.') {dirs.add(parent);parent=path.posix.dirname(parent)} }
  const tar=archive([...dirs].sort().map(name=>({name,directory:true,mode:0o755})).concat(payload))
  const ipkDirs=new Set()
  for(const row of ipkSource){let parent=path.posix.dirname(row.name);while(parent!=='.'){ipkDirs.add(parent);parent=path.posix.dirname(parent)}}
  const data=archive([...ipkDirs].sort().map(name=>({name,directory:true,mode:0o755})).concat(ipkSource))
  const control=Buffer.from(`Package: wlmb-server\nVersion: ${VERSION}\nArchitecture: ${packageArchitecture}\nMaintainer: SlAlot\nSection: net\nPriority: optional\nDepends: rpcd, rpcd-mod-iwinfo, luci-base, uci\nInstalled-Size: ${ipkSource.reduce((sum,row)=>sum+(row.bytes?.length||0),0)}\nDescription: Network Panel native Rust service\n`)
  const postinst=Buffer.from('#!/bin/sh\n[ -z "$IPKG_INSTROOT" ] || exit 0\n'+post.toString('utf8').replace(/^#!\/bin\/sh\n/,''))
  const prerm=Buffer.from('#!/bin/sh\n[ -z "$IPKG_INSTROOT" ] || exit 0\n/etc/init.d/wlmb-server stop >/dev/null 2>&1 || true\n/etc/init.d/wlmb-server disable >/dev/null 2>&1 || true\nexit 0\n')
  const controlTar=archive([{name:'control',bytes:control},{name:'conffiles',bytes:Buffer.from('/etc/config/network_panel\n')},{name:'postinst',bytes:postinst,mode:0o755},{name:'prerm',bytes:prerm,mode:0o755}])
  const ipk=archive([{name:'debian-binary',bytes:Buffer.from('2.0\n')},{name:'data.tar.gz',bytes:data},{name:'control.tar.gz',bytes:controlTar}])
  return {tar,ipk,manifest:{name:'wlmb-server',version:VERSION,architecture,packageArchitecture,backend:'rust',binarySha256:hash(bytes),ipkSha256:hash(ipk),ipkBytes:ipk.length,files:source.map(r=>({path:r.name,sha256:r.link?null:hash(r.bytes),link:r.link || null}))}}
}
module.exports={build,VERSION,ARCHIVE_MTIME,VIEW}
