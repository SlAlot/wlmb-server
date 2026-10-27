'use strict'
// 标准 Feed 配方回归；临时工具链仅验证规则展开和安装布局，不冒充 SDK 或 Rust 编译。
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),cp=require('node:child_process'),assert=require('node:assert/strict')
const {VERSION}=require('./package-source.cjs')
const root=path.resolve(__dirname,'..'),recipe=fs.readFileSync(path.join(root,'openwrt/wlmb-server/Makefile'),'utf8')
assert.ok(recipe.includes('PKG_VERSION:='+VERSION))
for(const value of ['PKG_BUILD_DEPENDS:=rust/host','RUST_PKG_FEATURES:=pushplus-https','RUST_PKG_LOCKED:=1','$(call Build/Compile/Cargo,backend)','CATEGORY:=LuCI','SUBMENU:=Applications','@(aarch64||x86_64)','$(realpath $(CURDIR))','/etc/config/network_panel','docs/model-bands.json'])assert.ok(recipe.includes(value),value)
assert.ok(!/releases\/download|\.ipk|\.apk|rustup|allow-untrusted|PKGARCH:=all/.test(recipe),'标准配方不能退回预编译包或跳过信任')
const guide=fs.readFileSync(path.join(root,'docs/FIRMWARE-INTEGRATION.zh-CN.md'),'utf8')
assert.ok(guide.includes('src-git wlmb https://github.com/SlAlot/wlmb-server.git;main'))
assert.ok(guide.includes('LuCI → Applications → wlmb-server'))
if(process.platform==='win32'){
 console.log('Feed 静态检查通过；GNU Make、符号链接和安装权限回归在 Linux CI 执行，不等于 SDK 构建。')
 process.exit(0)
}
const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'wlmb-feed-check-')),feed=path.join(temporary,'source'),sdk=path.join(temporary,'sdk')
function write(name,text){const file=path.join(sdk,name);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,text)}
try{
 for(const name of ['backend/src','backend/Cargo.toml','backend/Cargo.lock','docs/model-bands.json','frontend','packaging','LICENSE','openwrt']){
  const destination=path.join(feed,name);fs.mkdirSync(path.dirname(destination),{recursive:true});fs.cpSync(path.join(root,name),destination,{recursive:true})
 }
 write('rules.mk','BUILD_DIR:=$(TOPDIR)/build\nINCLUDE_DIR:=$(TOPDIR)/include\nINSTALL_DIR:=mkdir -p\nCP:=cp -fpR\nLN:=ln -sf\nINSTALL_BIN:=install -m0755\nINSTALL_CONF:=install -m0600\nINSTALL_DATA:=install -m0644\n')
 write('include/package.mk','PKG_INSTALL_DIR:=$(PKG_BUILD_DIR)/ipkg-install\ndefine BuildPackage\n.PHONY: verify\nverify:\n\t$$(Build/Prepare)\n\t$$(Build/Compile)\n\t$$(call Package/$(1)/install,$$(TOPDIR)/image)\nendef\n')
 write('feeds/packages/lang/rust/rust-package.mk',"define Build/Compile/Cargo\n\tmkdir -p $(PKG_INSTALL_DIR)/bin\n\tprintf '%s\\n' 'fixture-only' > $(PKG_INSTALL_DIR)/bin/wlmb-server\n\tprintf '%s\\n' '$(1)|$(RUST_PKG_FEATURES)|$(RUST_PKG_LOCKED)' > $(TOPDIR)/cargo-invocation\nendef\n")
 const linked=path.join(sdk,'package/feeds/wlmb/wlmb-server');fs.mkdirSync(path.dirname(linked),{recursive:true});fs.symlinkSync(path.join(feed,'openwrt/wlmb-server'),linked,'dir')
 for(const logical of [false,true]){
  const args=['-C',linked,'TOPDIR='+sdk,'verify'];if(logical)args.push('CURDIR='+linked)
  const result=cp.spawnSync('make',args,{encoding:'utf8'});assert.equal(result.status,0,result.stdout+'\n'+result.stderr)
 }
 assert.equal(fs.readFileSync(path.join(sdk,'cargo-invocation'),'utf8').trim(),'backend|pushplus-https|1')
 const image=path.join(sdk,'image'),build=path.join(sdk,'build','wlmb-server-'+VERSION)
 assert.equal(fs.readlinkSync(path.join(image,'usr/libexec/rpcd/network-panel')),'/usr/sbin/wlmb-server')
 for(const [name,mode] of [['usr/sbin/wlmb-server',0o755],['etc/init.d/wlmb-server',0o755],['etc/config/network_panel',0o600],['usr/share/wlmb-server/LICENSE',0o644]])assert.equal(fs.statSync(path.join(image,name)).mode&0o777,mode,name)
 const expected=[]
 function compare(directory,prefix){for(const entry of fs.readdirSync(directory,{withFileTypes:true})){
  const source=path.join(directory,entry.name),name=path.posix.join(prefix,entry.name)
  if(entry.isDirectory())compare(source,name)
  else{expected.push(name);assert.deepEqual(fs.readFileSync(path.join(image,name)),fs.readFileSync(source),name)}
 }}
 compare(path.join(root,'frontend'),'');compare(path.join(root,'packaging/files'),'')
 assert.deepEqual(fs.readFileSync(path.join(build,'docs/model-bands.json')),fs.readFileSync(path.join(root,'docs/model-bands.json')))
 assert.ok(!fs.existsSync(path.join(build,'backend/src/src')),'重新准备不能重复嵌套源码目录')
 const hook=recipe.match(/define Package\/wlmb-server\/postinst\n([\s\S]*?)\nendef/)[1].replace(/\$\$/g,'$')
 const syntax=cp.spawnSync('sh',['-n'],{input:hook,encoding:'utf8'});assert.equal(syntax.status,0,syntax.stderr)
 const staged=cp.spawnSync('sh',['-eu'],{input:hook,env:{...process.env,IPKG_INSTROOT:path.join(sdk,'image')},encoding:'utf8'});assert.equal(staged.status,0,staged.stderr)
 console.log('Feed 规则回归通过：真实路径／符号链接、重复准备、Cargo 参数、'+expected.length+' 个资源、权限、rpcd 链接与构建期钩子保护。临时 Cargo 替身不是 SDK 编译。')
}finally{
 // 仅清理由本测试创建、且已确认位于系统临时目录的专属目录。
 assert.equal(path.dirname(temporary),os.tmpdir());assert.ok(path.basename(temporary).startsWith('wlmb-feed-check-'));fs.rmSync(temporary,{recursive:true,force:true})
}
