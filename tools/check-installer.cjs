'use strict'
// 安装器使用临时目录及包管理器替身回归；不安装真实包，不写宿主系统公钥或网络配置。
const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), cp = require('node:child_process'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs'), { render } = require('./installer.cjs'), { publicKey } = require('./sign-release.cjs')
const files = new Map()
for (const arch of ['aarch64', 'x86_64']) for (const ext of ['apk', 'ipk']) files.set(`wlmb-server-${VERSION}-${arch}.${ext}`, Buffer.from(arch + ':' + ext + ':仅安装器流程替身'))
const source = render(files).toString(), key = publicKey()
assert(!/@[A-Z0-9_]+@/.test(source))
assert(source.includes("VERSION='" + VERSION + "'") && source.includes(key.sha256))
assert(!/--allow-untrusted|--no-check-certificate|--force-|curl[^\n]*\|\s*sh|eval\s|uci\s+(?:set|commit)|(?:^|\n)\s*(?:reboot|sysupgrade)\b/.test(source))
assert(source.includes('--simulate') && source.includes('--noaction') && source.includes('PKG_SHA') && source.includes('--keys-dir'))
assert.throws(() => render(new Map()), /安装包/)
if (process.platform !== 'linux') {
  console.log('安装器版本、固定摘要、无校验绕过及模板检查通过；POSIX流程回归由Linux CI执行。')
  process.exit(0)
}
const suite = fs.mkdtempSync(path.join(os.tmpdir(), 'wlmb-installer-suite-'))
function write(file, value, mode) { fs.mkdirSync(path.dirname(file), { recursive: true });fs.writeFileSync(file, value, mode ? { mode } : undefined) }
function fixture(name, options = {}) {
  const dir = path.join(suite, name), bin = path.join(dir, 'bin'), bundle = path.join(dir, 'bundle'), trust = path.join(dir, 'keys'), server = path.join(dir, 'server'), log = path.join(dir, 'actions.jsonl')
  fs.mkdirSync(bin, { recursive: true });fs.mkdirSync(bundle);fs.mkdirSync(trust)
  for (const [file, bytes] of files) write(path.join(bundle, file), bytes)
  write(path.join(bundle, key.fileName), key.publicKeyPem)
  write(path.join(dir, 'config-kept'), '已有配置不删除\n')
  const config = { manager: options.manager || 'apk', abi: options.abi || 'aarch64_cortex-a53', ...options, dir, bundle, trust, server, log }
  const marker = version => '#!/bin/sh\nprintf "%s\\n" "WLMB_BUILD=' + version + ';pushplus-https=1"\n'
  if (options.current) write(server, marker(options.current), 0o755)
  if (options.conflictingKey) write(path.join(trust, key.fileName), '另一个公钥')
  if (options.badPackage) fs.appendFileSync(path.join(bundle, `wlmb-server-${VERSION}-aarch64.apk`), '损坏')
  if (options.badKey) fs.appendFileSync(path.join(bundle, key.fileName), '损坏')
  if (options.linkedPackage) {
    const file = path.join(bundle, `wlmb-server-${VERSION}-aarch64.apk`)
    fs.renameSync(file, file + '.original');fs.symlinkSync(file + '.original', file)
  }
  const program = `const fs=require('node:fs'),path=require('node:path');const c=${JSON.stringify(config)},a=process.argv.slice(2),name=path.basename(process.argv[1]);
fs.appendFileSync(c.log,JSON.stringify({name,a})+'\\n');
if(name==='id'){console.log(c.nonRoot?'1000':'0');process.exit(0)}
if(name==='sleep')process.exit(0);
if(name==='df'){console.log('Filesystem 1024-blocks Used Available Capacity Mounted');console.log('fixture 1000000 1 '+(c.lowSpace?'1':'999999')+' 1% /');process.exit(0)}
if(name==='uci'){console.log('192.168.66.1');process.exit(0)}
if(name==='ubus'){console.log(JSON.stringify({data:{running:!c.notReady}}));process.exit(0)}
if(name==='curl'){if(c.downloadFails)process.exit(22);const out=a[a.indexOf('--output')+1],file=path.basename(a[a.length-1]);fs.copyFileSync(path.join(c.bundle,file),out);process.exit(0)}
if(name==='apk'&&a.includes('--print-arch')){if(c.manager!=='apk')process.exit(1);console.log(c.abi);process.exit(0)}
if(name==='opkg'&&a.includes('print-architecture')){if(c.manager!=='opkg')process.exit(1);console.log('arch all 1\\narch '+c.abi+' 10');process.exit(0)}
if(a.includes('verify')){if(c.badSignature)process.exit(1);console.log('fixture signature: OK');process.exit(0)}
if(a.includes('status')){if(c.missingDependency)process.exit(1);console.log('Status: install ok installed');process.exit(0)}
if(a.includes('update'))process.exit(c.preflightFails?1:0);
if(a.includes('--simulate')||a.includes('--noaction'))process.exit(c.preflightFails?1:0);
if(a.includes('add')||a.includes('install')){if(c.installFails)process.exit(1);fs.writeFileSync(c.server,${JSON.stringify(marker(VERSION))},{mode:0o755});process.exit(0)}
process.exit(1);`
  for (const name of ['apk', 'opkg', 'ubus', 'uci', 'id', 'df', 'sleep', 'curl']) write(path.join(bin, name), '#!' + process.execPath + '\n' + program, 0o755)
  const file = path.join(dir, 'install.sh')
  // 测试仅重定位系统路径及命令搜索目录，生产脚本不接受这些测试覆盖参数。
  const script = source.replace('PATH=/usr/sbin:/usr/bin:/sbin:/bin', 'PATH=' + bin + ':/usr/sbin:/usr/bin:/sbin:/bin').replaceAll('/etc/apk/keys', trust).replaceAll('/usr/sbin/wlmb-server', server).replaceAll('/tmp/wlmb-installer.lock', path.join(dir, 'lock'))
  write(file, script)
  cp.execFileSync('sh', ['-n', file])
  const args = options.online ? [] : ['--offline', bundle]
  if (options.check) args.push('--check')
  if (!options.cancel && !options.noConfirm) args.push('--yes')
  const result = cp.spawnSync('sh', [file, ...args], { input: options.cancel ? 'n\n' : '', encoding: 'utf8', timeout: 20000 })
  assert(!result.error, name + ': ' + result.error)
  const output = result.stdout + result.stderr
  const actions = fs.existsSync(log) ? fs.readFileSync(log, 'utf8').trim().split('\n').map(row => JSON.parse(row)) : []
  const installs = actions.filter(row => ['apk', 'opkg'].includes(row.name) && (row.a.includes('add') || row.a.includes('install')) && !row.a.includes('--simulate') && !row.a.includes('--noaction'))
  assert.equal(fs.readFileSync(path.join(dir, 'config-kept'), 'utf8'), '已有配置不删除\n')
  assert(!fs.existsSync(path.join(dir, 'lock')), name + ': 安装锁必须释放')
  if (options.fail) {
    assert.notEqual(result.status, 0, name + ': 应拒绝\n' + output)
    assert(!output.includes('安装完成：'), name + ': 失败不能显示安装完成')
  } else assert.equal(result.status, 0, name + '\n' + output)
  const expectedInstalls = options.fail ? (options.installFails || options.notReady ? 1 : 0) : options.check || options.cancel ? 0 : 1
  assert.equal(installs.length, expectedInstalls, name)
  const keyPath = path.join(trust, key.fileName)
  if (options.conflictingKey) assert.equal(fs.readFileSync(keyPath, 'utf8'), '另一个公钥')
  else if (config.manager === 'apk' && (installs.length || !options.fail && !options.check && !options.cancel)) assert.equal(fs.readFileSync(keyPath, 'utf8'), key.publicKeyPem)
  else assert(!fs.existsSync(keyPath), name + ': 取消、检查或预检失败不能残留新公钥')
  if (options.check) assert(!actions.some(row => row.a.includes('update') || row.a.includes('--simulate') || row.a.includes('--noaction')))
  if (!options.fail && !options.check && !options.cancel) assert(output.includes('192.168.66.1:1117'))
  return { actions, output }
}
try {
  fixture('apk-arm');fixture('apk-x86', { abi: 'x86_64' })
  fixture('ipk-arm', { manager: 'opkg' });fixture('ipk-x86', { manager: 'opkg', abi: 'x86_64' })
  fixture('check', { check: true });fixture('cancel', { cancel: true })
  fixture('confirmation-required', { noConfirm: true, fail: true })
  fixture('unsupported-abi', { abi: 'aarch64_generic', fail: true })
  fixture('bad-package', { badPackage: true, fail: true });fixture('bad-key', { badKey: true, fail: true })
  fixture('bad-signature', { badSignature: true, fail: true });fixture('key-conflict', { conflictingKey: true, fail: true })
  fixture('preflight', { preflightFails: true, fail: true })
  fixture('install-failure', { installFails: true, fail: true })
  fixture('not-ready', { notReady: true, fail: true })
  fixture('non-root', { nonRoot: true, fail: true });fixture('space', { lowSpace: true, fail: true })
  fixture('downgrade', { current: '99.0.0', fail: true });fixture('upgrade', { current: '1.0.3' })
  fixture('symlink', { linkedPackage: true, fail: true })
  fixture('ipk-offline-dependency', { manager: 'opkg', missingDependency: true, fail: true })
  fixture('online', { online: true });fixture('download-failure', { online: true, downloadFails: true, fail: true })
  console.log('安装器23组POSIX流程回归通过：双包管理器／双架构、在线／离线、确认、检查、摘要、验签、空间、依赖、公钥冲突、失败回滚、版本和服务检查；替身不视为实机安装验收。')
} finally { fs.rmSync(suite, { recursive: true, force: true }) }
