'use strict'
// 仅用合成ELF／包数据测试发布门禁，不将此用例视为真实安装包验收。
const fs = require('node:fs'), os = require('node:os'), path = require('node:path'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { build, VERSION } = require('./package-source.cjs'), { prepare } = require('./release-assets.cjs')
const { publish } = require('./publish-release.cjs')
const { publicKey } = require('./sign-release.cjs')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'wlmb-release-fixture-'))
assert(path.dirname(directory) === path.resolve(os.tmpdir()) && path.basename(directory).startsWith('wlmb-release-fixture-'))
function write(file, bytes) { fs.mkdirSync(path.dirname(file), { recursive: true });fs.writeFileSync(file, bytes) }
try {
  for (const [arch, machine, abi] of [['aarch64', 183, 'aarch64_cortex-a53'], ['x86_64', 62, 'x86_64']]) {
    const target = arch + '-unknown-linux-musl', binary = path.join(directory, 'wlmb-server-' + target, 'wlmb-server')
    const bytes = Buffer.alloc(256);bytes.write('7f454c46', 0, 'hex');bytes[4] = 2;bytes[5] = 1;bytes.writeUInt16LE(machine, 18);bytes.write(`WLMB_BUILD=${VERSION};pushplus-https=1`, 64)
    write(binary, bytes)
    const result = build(binary, arch, abi), apk = Buffer.from('合成APK，仅用于摘要测试')
    result.manifest.packageSha256 = hash(apk);result.manifest.packageBytes = apk.length
    const key = publicKey()
    result.manifest.packageSignature = { format: 'apk-v3', publicKeyFile: key.fileName, publicKeySha256: key.sha256 }
    const pkg = path.join(directory, 'wlmb-packages-' + target)
    write(path.join(pkg, `wlmb-server-${VERSION}-${arch}.apk`), apk)
    write(path.join(pkg, `wlmb-server-${VERSION}-${arch}.ipk`), result.ipk)
    write(path.join(pkg, `manifest-${arch}.json`), JSON.stringify(result.manifest))
  }
  const report = path.join(directory, 'wlmb-release-readiness'), ref = 'a'.repeat(40)
  write(path.join(report, 'wlmb-compatibility.json'), JSON.stringify({ schema: 'WLMB-COMPATIBILITY-AUDIT/1', version: VERSION, referenceRevision: ref, summary: { models: 91, identityGaps: 0, protocolComplete: false, hardwareComplete: false } }))
  write(path.join(report, 'wlmb-release-readiness.json'), JSON.stringify({ schema: 'WLMB-RELEASE-READINESS/1', referenceRevision: ref, identifiedModels: 91, missingIdentity: [] }))
  const output = path.join(directory, 'release'), files = prepare(directory, output, 'b'.repeat(40), 'v' + VERSION)
  assert.equal(files.length, 13)
  assert(fs.readFileSync(path.join(output, 'install.sh'), 'utf8').includes("VERSION='" + VERSION + "'"))
  const key = publicKey()
  assert.equal(hash(fs.readFileSync(path.join(output, key.fileName))), key.sha256)
  assert.equal(JSON.parse(fs.readFileSync(path.join(output, 'release.json'))).protocolComplete, false, '不能为了发布改成全模组通过')
  let remote, uploadCalls = 0, failUpload = true
  const execute = (program, args) => {
    if (program === 'git') return 'b'.repeat(40) + '\n'
    assert.equal(program, 'gh')
    if (args[0] === 'api') {
      if (args[1].includes('/git/ref/tags/')) return JSON.stringify({ object: { type: 'commit', sha: 'b'.repeat(40) } })
      assert(args[1].endsWith('/releases/123'), '草稿必须按发布编号读取，不能按标签查询')
      assert(remote)
      return JSON.stringify(remote)
    }
    assert.equal(args[0], 'release')
    if (args[1] === 'view') {
      if (!remote) throw Object.assign(new Error('不存在'), { stderr: 'release not found' })
      return JSON.stringify({ databaseId: 123 })
    }
    if (args[1] === 'create') { assert(args.includes('--draft'));remote = { draft: true, assets: [] };return '' }
    if (args[1] === 'upload') {
      uploadCalls++
      if (failUpload && uploadCalls === 4) throw new Error('模拟上传中断')
      const name = path.basename(args[3]);assert(!remote.assets.some(item => item.name === name))
      remote.assets.push({ name, digest: 'sha256:' + hash(fs.readFileSync(args[3])) });return ''
    }
    assert.equal(args[1], 'edit');assert.equal(remote.assets.length, 13);remote.draft = false;return ''
  }
  assert.throws(() => publish(output, 'v' + VERSION, 'SlAlot/wlmb-server', execute), /模拟上传中断/)
  assert.equal(remote.assets.length, 3);assert.equal(remote.draft, true, '资产未齐不能发布草稿')
  failUpload = false
  assert.equal(publish(output, 'v' + VERSION, 'SlAlot/wlmb-server', execute).assets, 13)
  const completed = uploadCalls
  publish(output, 'v' + VERSION, 'SlAlot/wlmb-server', execute)
  assert.equal(uploadCalls, completed, '相同制品重跑不重复上传')
  remote.assets[0].digest = 'sha256:' + 'c'.repeat(64)
  assert.throws(() => publish(output, 'v' + VERSION, 'SlAlot/wlmb-server', execute), /拒绝覆盖/)
  assert.equal(uploadCalls, completed)
  assert.throws(() => prepare(directory, output, 'b'.repeat(40), 'v' + VERSION), /全新/)
  assert.throws(() => prepare(directory, path.join(directory, 'bad'), 'b'.repeat(40), 'v9.9.9'), /标签/)
  const apk = path.join(directory, 'wlmb-packages-aarch64-unknown-linux-musl', `wlmb-server-${VERSION}-aarch64.apk`)
  fs.appendFileSync(apk, '篡改')
  assert.throws(() => prepare(directory, path.join(directory, 'bad'), 'b'.repeat(40), 'v' + VERSION))
  assert(!fs.existsSync(path.join(directory, 'bad')), '校验失败不能产生发布资产')
  write(apk, Buffer.from('合成APK，仅用于摘要测试'))
  const manifestFile = path.join(directory, 'wlmb-packages-x86_64-unknown-linux-musl', 'manifest-x86_64.json')
  const manifest = JSON.parse(fs.readFileSync(manifestFile, 'utf8'))
  delete manifest.packageSignature;write(manifestFile, JSON.stringify(manifest))
  assert.throws(() => prepare(directory, path.join(directory, 'unsigned'), 'b'.repeat(40), 'v' + VERSION), /正式APK/)
  console.log('发布制品门禁回归通过：版本、架构、摘要、源码清单、报告、无覆盖和失败不产出；合成包不作硬件证据。')
} finally { fs.rmSync(directory, { recursive: true, force: true }) }
