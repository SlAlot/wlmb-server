// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs=require('node:fs'),os=require('node:os'),path=require('node:path'),assert=require('node:assert/strict'),zlib=require('node:zlib')
const {build,VERSION,ARCHIVE_MTIME,VIEW}=require('./package-source.cjs')
function untar(gzip){
  const raw=zlib.gunzipSync(gzip,{maxOutputLength:16000000}),files=new Map()
  for(let at=0;at+512<=raw.length;){
    const h=raw.subarray(at,at+512);if(h.every(v=>v===0))break
    const text=(a,b)=>h.toString('utf8',a,b).split('\0')[0]
    const name=text(0,100),size=parseInt(text(124,136).trim(),8),sum=parseInt(text(148,156).trim(),8)
    assert.equal(h.reduce((total,b,i)=>total+(i>=148&&i<156?32:b),0),sum)
    assert.ok(!files.has(name));assert.ok(!name.split('/').includes('..'));assert.ok(at+512+size<=raw.length)
    files.set(name,{bytes:raw.subarray(at+512,at+512+size),kind:String.fromCharCode(h[156]),link:text(157,257),mode:parseInt(text(100,108),8),mtime:parseInt(text(136,148),8)})
    at+=512+Math.ceil(size/512)*512
  }
  return files
}
const directory=fs.mkdtempSync(path.join(os.tmpdir(),'wlmb-package-fixture-')),file=path.join(directory,'fixture-elf')
try{
  const elf=Buffer.alloc(256);elf.write('7f454c46',0,'hex');elf[4]=2;elf[5]=1;elf.writeUInt16LE(62,18);elf.write(`WLMB_BUILD=${VERSION};pushplus-https=1`,64)
  fs.writeFileSync(file,elf)
  const result=build(file,'x86_64'),outer=untar(result.ipk)
  for(const bytes of [result.ipk,result.tar,outer.get('data.tar.gz').bytes,outer.get('control.tar.gz').bytes]) assert.equal(bytes[9],3,'压缩头不应取决于构建宿主系统')
  assert.deepEqual([...outer.keys()],['debian-binary','data.tar.gz','control.tar.gz']);assert.equal(outer.get('debian-binary').bytes.toString(),'2.0\n')
  const data=untar(outer.get('data.tar.gz').bytes),control=untar(outer.get('control.tar.gz').bytes)
  assert.equal(data.get('usr/sbin/wlmb-server').mode,0o755)
  const frontend='www/luci-static/resources/view/wlmb-server/'+VIEW+'.js'
  const menu=JSON.parse(data.get('usr/share/luci/menu.d/network-panel.json').bytes.toString())
  assert.equal(menu['admin/services/wlmb-server'].action.path,'wlmb-server/'+VIEW)
  assert.equal(data.get(frontend).mtime,ARCHIVE_MTIME)
  assert.ok(ARCHIVE_MTIME>1767225600,'Do not reuse the old fixed cache timestamp')
  assert.equal(untar(result.tar).get('files/'+frontend).mtime,ARCHIVE_MTIME)
  assert.equal(data.get('usr/libexec/rpcd/network-panel').kind,'2');assert.equal(data.get('usr/libexec/rpcd/network-panel').link,'/usr/sbin/wlmb-server')
  assert.ok(![...data.keys()].some(n=>n.startsWith('lib/apk/')))
  assert.match(control.get('control').bytes.toString(),/Architecture: x86_64/)
  assert.match(control.get('control').bytes.toString(),/Depends: rpcd, rpcd-mod-iwinfo, luci-base, uci/)
  assert.equal(control.get('conffiles').bytes.toString(),'/etc/config/network_panel\n')
  for(const name of ['postinst','prerm']){assert.equal(control.get(name).mode,0o755);assert.ok(control.get(name).bytes.toString().includes('IPKG_INSTROOT'))}
  assert.throws(()=>build(file,'aarch64'),/architecture/)
  assert.throws(()=>build(file,'x86_64','aarch64_cortex-a53'),/ABI/)
  elf.write(`WLMB_BUILD=${VERSION};pushplus-https=0`,64);fs.writeFileSync(file,elf)
  assert.throws(()=>build(file,'x86_64'),/incomplete/)
  console.log('APK payload/IPK container regressions passed: separate metadata, owned hooks, symlink, ABI/version/HTTPS checks. Synthetic header only, not binary acceptance.')
}finally{fs.unlinkSync(file);fs.rmdirSync(directory)}
