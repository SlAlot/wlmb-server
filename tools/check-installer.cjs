'use strict'
// 安装器使用临时目录及包管理器替身回归；不安装真实包，不写宿主系统公钥或网络配置。
const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), cp = require('node:child_process'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs'), { render } = require('./installer.cjs'), { publicKey } = require('./sign-release.cjs')
const { PLATFORMS } = require('./platforms.cjs')
const files = new Map()
for (const { id: arch } of PLATFORMS) for (const ext of ['apk', 'ipk']) files.set(`wlmb-server-${VERSION}-${arch}.${ext}`, Buffer.from(arch + ':' + ext + ':仅安装器流程替身'))
const source = render(files).toString(), key = publicKey()
const proxySource = render(files, 'gh-proxy').toString()
assert(source.includes("DOWNLOAD_PREFIX=''"))
assert(proxySource.includes("DOWNLOAD_PREFIX='https://gh-proxy.com/'"))
assert.equal(proxySource.replace("DOWNLOAD_PREFIX='https://gh-proxy.com/'", "DOWNLOAD_PREFIX=''").replace('sh install-gh-proxy.sh', 'sh install.sh'), source, '加速通道只替换固定下载入口与脚本名，不改变验证及安装行为')
assert.throws(() => render(files, 'https://example.com/'), /固定入口/)
assert(!/@[A-Z0-9_]+@/.test(source))
assert(source.includes("VERSION='" + VERSION + "'") && source.includes(key.sha256))
assert(!/--allow-untrusted|--no-check-certificate|--force-|curl[^\n]*\|\s*sh|eval\s|uci\s+(?:set|commit)|(?:^|\n)\s*(?:reboot|sysupgrade)\b/.test(source))
assert(source.includes('--simulate') && source.includes('--noaction') && source.includes('PKG_SHA') && source.includes('--keys-dir'))
assert(source.includes('--check-ready') && !source.includes('ubus call network-panel service_status'))
assert.throws(() => render(new Map()), /安装包/)
if (process.platform !== 'linux') {
  console.log('安装器版本、固定摘要、无校验绕过及模板检查通过；POSIX流程回归由Linux CI执行。')
  process.exit(0)
}
const suite = fs.mkdtempSync(path.join(os.tmpdir(), 'wlmb-installer-suite-'))
let cases = 0
function write(file, value, mode) { fs.mkdirSync(path.dirname(file), { recursive: true });fs.writeFileSync(file, value, mode ? { mode } : undefined) }
function fixture(name, options = {}) {
  cases++
  const dir = path.join(suite, name), bin = path.join(dir, 'bin'), bundle = path.join(dir, 'bundle'), trust = path.join(dir, 'keys'), server = path.join(dir, 'server'), log = path.join(dir, 'actions.jsonl')
  fs.mkdirSync(bin, { recursive: true });fs.mkdirSync(bundle);fs.mkdirSync(trust)
  for (const [file, bytes] of files) write(path.join(bundle, file), bytes)
  write(path.join(bundle, key.fileName), key.publicKeyPem)
  write(path.join(dir, 'config-kept'), '已有配置不删除\n')
  const config = { manager: options.manager || 'apk', abi: options.abi || 'aarch64_cortex-a53', ...options, dir, bundle, trust, server, log }
  const marker = version => '#!' + process.execPath + '\n' + `const fs=require('node:fs'),c=${JSON.stringify(config)},a=process.argv.slice(2);fs.appendFileSync(c.log,JSON.stringify({name:'wlmb-server',a})+'\\n');if(a[0]==='--version'){console.log('WLMB_BUILD=${version};pushplus-https=1');process.exit(0)}if(a[0]==='--check-ready'){const checks=fs.readFileSync(c.log,'utf8').split('\\n').filter(v=>v.includes('--check-ready')).length;process.exit(c.notReady||c.staleDaemon||checks<=(c.readyDelay||0)?1:0)}process.exit(2);\n`
  if (options.current) write(server, marker(options.current), 0o755)
  if (options.conflictingKey) write(path.join(trust, key.fileName), '另一个公钥')
  if (options.existingKey) write(path.join(trust, key.fileName), key.publicKeyPem)
  if (options.archConfig !== undefined) write(path.join(dir, 'apk-arch'), options.archConfig)
  const selected = PLATFORMS.find(row => row.packageArchitecture === config.abi)
  const selectedName = selected && `wlmb-server-${VERSION}-${selected.id}.${config.manager === 'opkg' ? 'ipk' : 'apk'}`
  if (options.badPackage) fs.appendFileSync(path.join(bundle, selectedName), '损坏')
  if (options.badKey) fs.appendFileSync(path.join(bundle, key.fileName), '损坏')
  if (options.linkedPackage) {
    const file = path.join(bundle, selectedName)
    fs.renameSync(file, file + '.original');fs.symlinkSync(file + '.original', file)
  }
  const program = `const fs=require('node:fs'),path=require('node:path');const c=${JSON.stringify(config)},a=process.argv.slice(2),name=path.basename(process.argv[1]);
fs.appendFileSync(c.log,JSON.stringify({name,a})+'\\n');
if(name==='id'){console.log(c.nonRoot?'1000':'0');process.exit(0)}
if(name==='sleep')process.exit(0);
if(name==='df'){console.log('Filesystem 1024-blocks Used Available Capacity Mounted');console.log('fixture 1000000 1 '+(c.lowSpace?'1':'999999')+' 1% /');process.exit(0)}
if(name==='uci'){console.log('192.168.66.1');process.exit(0)}
if(name==='ubus'){console.log(JSON.stringify({ok:false,error:{code:'SESSION_EXPIRED'}}));process.exit(0)}
if(name==='curl'){if(c.downloadFails)process.exit(22);const out=a[a.indexOf('--output')+1],file=path.basename(a[a.length-1]);fs.copyFileSync(path.join(c.bundle,file),out);process.exit(0)}
if(name==='apk'&&a.includes('--print-arch')){if(c.manager!=='apk')process.exit(1);console.log(c.defaultAbi||c.abi);process.exit(0)}
if(name==='opkg'&&a.includes('print-architecture')){if(c.manager!=='opkg')process.exit(1);console.log(c.opkgArchitectures||'arch all 1\\narch '+c.abi+' 10');process.exit(0)}
if(a.includes('verify')){if(c.badSignature)process.exit(1);console.log('fixture signature: OK');process.exit(0)}
if(a.includes('status')){if(c.missingDependency)process.exit(1);console.log('Status: install ok installed');process.exit(0)}
if(a.includes('update'))process.exit(c.preflightFails?1:0);
if(a.includes('--simulate')||a.includes('--noaction'))process.exit(c.preflightFails?1:0);
if(a.includes('add')||a.includes('install')){if(c.installFails)process.exit(1);fs.writeFileSync(c.server,${JSON.stringify(marker(VERSION))},{mode:0o755});process.exit(0)}
process.exit(1);`
  for (const name of ['apk', 'opkg', 'ubus', 'uci', 'id', 'df', 'sleep', 'curl']) write(path.join(bin, name), '#!' + process.execPath + '\n' + program, 0o755)
  const file = path.join(dir, 'install.sh')
  // 测试仅重定位系统路径及命令搜索目录，生产脚本不接受这些测试覆盖参数。
  const script = (options.proxy ? proxySource : source).replace('PATH=/usr/sbin:/usr/bin:/sbin:/bin', 'PATH=' + JSON.stringify(bin + ':/usr/sbin:/usr/bin:/sbin:/bin')).replaceAll('/etc/apk/keys', trust).replaceAll('/etc/apk/arch', path.join(dir, 'apk-arch')).replaceAll('/usr/sbin/wlmb-server', server).replaceAll('/tmp/wlmb-installer.lock', path.join(dir, 'lock'))
  write(file, script)
  cp.execFileSync('sh', ['-n', file])
  const args = options.online ? [] : ['--offline', bundle]
  if (options.check) args.push('--check')
  if (!options.cancel && !options.noConfirm && !options.confirm) args.push('--yes')
  const result = cp.spawnSync('sh', [file, ...args], { input: options.cancel ? 'n\n' : options.confirm ? 'y\n' : '', encoding: 'utf8', timeout: 20000 })
  assert(!result.error, name + ': ' + result.error)
  const output = result.stdout + result.stderr
  const actions = fs.existsSync(log) ? fs.readFileSync(log, 'utf8').trim().split('\n').map(row => JSON.parse(row)) : []
  for (const row of actions.filter(row => row.name === 'curl')) {
    const url = row.a.at(-1), prefix = options.proxy ? 'https://gh-proxy.com/' : ''
    assert(url.startsWith(prefix + 'https://github.com/SlAlot/wlmb-server/releases/download/v' + VERSION + '/'), '只下载本通道固定版本资产')
    assert(!url.includes('/latest/') && !url.includes('#'), '不混用动态版本或网页片段地址')
  }
  const installs = actions.filter(row => ['apk', 'opkg'].includes(row.name) && (row.a.includes('add') || row.a.includes('install')) && !row.a.includes('--simulate') && !row.a.includes('--noaction'))
  assert.equal(fs.readFileSync(path.join(dir, 'config-kept'), 'utf8'), '已有配置不删除\n')
  assert(!fs.existsSync(path.join(dir, 'lock')), name + ': 安装锁必须释放')
  if (options.fail) {
    assert.notEqual(result.status, 0, name + ': 应拒绝\n' + output)
    assert(!output.includes('安装完成：'), name + ': 失败不能显示安装完成')
  } else {
    assert.equal(result.status, 0, name + '\n' + output)
    if (!options.cancel) assert(output.includes(config.abi), name + ': 必须识别固件包 ABI')
  }
  for (const row of installs) assert(row.a.some(value => path.basename(value) === selectedName), name + ': 不能选择其他 ABI 的安装包')
  if (!options.fail && options.online && !options.cancel) assert(actions.some(row => row.name === 'curl' && path.basename(row.a.at(-1)) === selectedName), name + ': 必须下载匹配 ABI 的安装包')
  const expectedInstalls = options.fail ? (options.installFails || options.notReady || options.staleDaemon ? 1 : 0) : options.check || options.cancel ? 0 : 1
  assert.equal(installs.length, expectedInstalls, name)
  const keyPath = path.join(trust, key.fileName)
  if (options.conflictingKey) assert.equal(fs.readFileSync(keyPath, 'utf8'), '另一个公钥')
  else if (options.existingKey || config.manager === 'apk' && (installs.length || !options.fail && !options.check && !options.cancel)) assert.equal(fs.readFileSync(keyPath, 'utf8'), key.publicKeyPem)
  else assert(!fs.existsSync(keyPath), name + ': 取消、检查或预检失败不能残留新公钥')
  if (options.check) assert(!actions.some(row => row.a.includes('update') || row.a.includes('--simulate') || row.a.includes('--noaction')))
  assert(!actions.some(row => row.name === 'ubus'), name + ': 安装器不能依赖无会话网页RPC')
  const checks = actions.filter(row => row.name === 'wlmb-server' && row.a.includes('--check-ready'))
  if (options.notReady || options.staleDaemon) assert.equal(checks.length, 10)
  else if (!options.fail && !options.check && !options.cancel) assert.equal(checks.length, 1 + (options.readyDelay || 0))
  else assert.equal(checks.length, 0)
  if (!options.fail && !options.check && !options.cancel) assert(output.includes('192.168.66.1:1117'))
  return { actions, output }
}
try {
  fixture('apk-arm');fixture('apk-x86', { abi: 'x86_64' })
  fixture('ipk-arm', { manager: 'opkg' });fixture('ipk-x86', { manager: 'opkg', abi: 'x86_64' })
  fixture('check', { check: true });fixture('cancel', { cancel: true })
  fixture('confirmation-required', { noConfirm: true, fail: true })
  fixture('unsupported-abi', { abi: 'aarch64_cortex-a72', fail: true })
  fixture('bad-package', { badPackage: true, fail: true });fixture('bad-key', { badKey: true, fail: true })
  fixture('bad-signature', { badSignature: true, fail: true });fixture('key-conflict', { conflictingKey: true, fail: true })
  fixture('preflight', { preflightFails: true, fail: true })
  fixture('install-failure', { installFails: true, fail: true })
  fixture('not-ready', { notReady: true, fail: true })
  fixture('stale-daemon', { staleDaemon: true, fail: true })
  fixture('delayed-ready', { readyDelay: 3 })
  fixture('ipk-not-ready', { manager: 'opkg', notReady: true, fail: true })
  fixture('non-root', { nonRoot: true, fail: true });fixture('space', { lowSpace: true, fail: true })
  fixture('downgrade', { current: '99.0.0', fail: true });fixture('upgrade', { current: '1.0.3' })
  fixture('symlink', { linkedPackage: true, fail: true })
  fixture('ipk-offline-dependency', { manager: 'opkg', missingDependency: true, fail: true })
  fixture('online', { online: true });fixture('download-failure', { online: true, downloadFails: true, fail: true })
  fixture('directory with spaces');fixture('explicit-confirm', { confirm: true })
  fixture('non-root-check', { nonRoot: true, check: true })
  fixture('existing-key', { existingKey: true });fixture('existing-key-preflight', { existingKey: true, preflightFails: true, fail: true })
  fixture('feed-refresh-failure', { online: true, preflightFails: true, fail: true })
  fixture('openwrt-configured-abi', { defaultAbi: 'aarch64', archConfig: 'aarch64_cortex-a53\n' })
  fixture('openwrt-abi-check', { defaultAbi: 'aarch64', archConfig: 'aarch64_cortex-a53\n', check: true })
  fixture('openwrt-abi-list', { defaultAbi: 'aarch64', archConfig: 'aarch64\naarch64_cortex-a53\n' })
  fixture('generic-abi-not-assumed', { defaultAbi: 'aarch64', archConfig: 'aarch64\n', fail: true })
  fixture('configured-abi-unknown', { archConfig: 'aarch64_cortex-a72\n', fail: true })
  fixture('configured-abi-ambiguous', { archConfig: 'aarch64_cortex-a53\nx86_64\n', fail: true })
  fixture('configured-abi-cross-family', { defaultAbi: 'x86_64', archConfig: 'aarch64_cortex-a53\n', fail: true })
  fixture('configured-x86', { abi: 'x86_64', defaultAbi: 'x86_64', archConfig: 'x86_64\n' })
  fixture('proxy-apk-online', { proxy: true, online: true })
  fixture('proxy-ipk-online', { proxy: true, online: true, manager: 'opkg' })
  fixture('proxy-check', { proxy: true, online: true, check: true })
  fixture('proxy-offline', { proxy: true })
  fixture('proxy-bad-package', { proxy: true, online: true, badPackage: true, fail: true })
  fixture('proxy-bad-key', { proxy: true, online: true, badKey: true, fail: true })
  fixture('proxy-bad-signature', { proxy: true, online: true, badSignature: true, fail: true })
  fixture('proxy-download-failure', { proxy: true, online: true, downloadFails: true, fail: true })
  for (const manager of ['apk', 'opkg']) {
    fixture('generic-' + manager + '-offline', { manager, abi: 'aarch64_generic' })
    fixture('generic-' + manager + '-online', { manager, abi: 'aarch64_generic', online: true })
    fixture('generic-' + manager + '-check', { manager, abi: 'aarch64_generic', online: true, check: true })
    fixture('generic-' + manager + '-proxy', { manager, abi: 'aarch64_generic', online: true, proxy: true })
    fixture('generic-' + manager + '-corrupt', { manager, abi: 'aarch64_generic', badPackage: true, fail: true })
  }
  fixture('generic-apk-configured', { abi: 'aarch64_generic', defaultAbi: 'aarch64', archConfig: 'aarch64_generic\n' })
  fixture('generic-apk-configured-check', { abi: 'aarch64_generic', defaultAbi: 'aarch64', archConfig: 'aarch64\naarch64_generic\n', check: true })
  fixture('generic-signature', { abi: 'aarch64_generic', badSignature: true, fail: true })
  fixture('generic-apk-cross-family', { defaultAbi: 'x86_64', archConfig: 'aarch64_generic\n', fail: true })
  fixture('generic-apk-ambiguous', { defaultAbi: 'aarch64', archConfig: 'aarch64_generic\naarch64_cortex-a53\n', fail: true })
  fixture('generic-ipk-ambiguous', { manager: 'opkg', opkgArchitectures: 'arch all 1\narch aarch64_generic 10\narch aarch64_cortex-a53 20', fail: true })
  fixture('generic-ipk-duplicate', { manager: 'opkg', abi: 'aarch64_generic', opkgArchitectures: 'arch all 1\narch aarch64_generic 10\narch aarch64_generic 10' })
  console.log('安装器' + cases + '组POSIX流程回归通过：双包管理器／三种ABI、在线／离线、确认、检查、摘要、验签、空间、依赖、公钥冲突、失败回滚、版本和服务检查；替身不视为实机安装验收。')
} finally { fs.rmSync(suite, { recursive: true, force: true }) }
