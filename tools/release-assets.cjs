'use strict'
// 发布前核对两种架构的二进制、包摘要与源文件清单，只整理插件制品。
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { build, VERSION } = require('./package-source.cjs')
const { publicKey } = require('./sign-release.cjs')
const { render } = require('./installer.cjs')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
function prepare(directory, output, commit, tag) {
  assert.equal(tag, 'v' + VERSION, '标签必须和前后端版本一致')
  assert(/^[a-f0-9]{40}$/.test(commit), '制品必须绑定完整提交')
  const files = new Map(), key = publicKey()
  for (const arch of ['aarch64', 'x86_64']) {
    const target = arch + '-unknown-linux-musl', pkg = path.join(directory, 'wlmb-packages-' + target)
    const manifest = JSON.parse(fs.readFileSync(path.join(pkg, 'manifest-' + arch + '.json'), 'utf8'))
    const binary = path.join(directory, 'wlmb-server-' + target, 'wlmb-server')
    assert.equal(manifest.version, VERSION);assert.equal(manifest.architecture, arch);assert.equal(manifest.backend, 'rust')
    assert.equal(manifest.binarySha256, hash(fs.readFileSync(binary)))
    assert.deepEqual(manifest.packageSignature, { format: 'apk-v3', publicKeyFile: key.fileName, publicKeySha256: key.sha256 }, '正式APK必须经过固定公钥签名验证')
    const expected = build(binary, arch, manifest.packageArchitecture)
    assert.deepEqual(manifest.files, expected.manifest.files, '安装文件必须对应本次源码')
    for (const extension of ['apk', 'ipk']) {
      const name = `wlmb-server-${VERSION}-${arch}.${extension}`, bytes = fs.readFileSync(path.join(pkg, name))
      assert.equal(hash(bytes), extension === 'apk' ? manifest.packageSha256 : manifest.ipkSha256)
      assert.equal(bytes.length, extension === 'apk' ? manifest.packageBytes : manifest.ipkBytes)
      if (extension === 'ipk') assert.equal(hash(bytes), hash(expected.ipk), 'IPK必须对应本次打包内容')
      files.set(name, bytes)
    }
    files.set('manifest-' + arch + '.json', Buffer.from(JSON.stringify(manifest, null, 2) + '\n'))
  }
  const reports = path.join(directory, 'wlmb-release-readiness')
  const compatibility = JSON.parse(fs.readFileSync(path.join(reports, 'wlmb-compatibility.json'), 'utf8'))
  const readiness = JSON.parse(fs.readFileSync(path.join(reports, 'wlmb-release-readiness.json'), 'utf8'))
  assert.equal(compatibility.schema, 'WLMB-COMPATIBILITY-AUDIT/1');assert.equal(compatibility.version, VERSION)
  assert.equal(compatibility.summary.identityGaps, 0);assert.equal(compatibility.summary.models, readiness.identifiedModels)
  assert.equal(readiness.schema, 'WLMB-RELEASE-READINESS/1');assert.deepEqual(readiness.missingIdentity, [])
  assert.equal(compatibility.referenceRevision, readiness.referenceRevision)
  for (const name of ['wlmb-compatibility.json', 'wlmb-release-readiness.json']) files.set(name, fs.readFileSync(path.join(reports, name)))
  files.set(key.fileName, Buffer.from(key.publicKeyPem))
  files.set('INSTALL-APK.md', fs.readFileSync(path.resolve(__dirname, '../docs/APK-INSTALL.zh-CN.md')))
  files.set('install.sh', render(files))
  files.set('release.json', Buffer.from(JSON.stringify({
    schema: 'WLMB-RELEASE/1', version: VERSION, tag, commit, license: 'LicenseRef-WLMB-NonCommercial',
    platforms: ['aarch64_cortex-a53', 'x86_64'],
    apkSigning: { publicKeyFile: key.fileName, publicKeySha256: key.sha256 },
    protocolComplete: compatibility.summary.protocolComplete, hardwareComplete: compatibility.summary.hardwareComplete,
    assets: [...files].map(([name, bytes]) => ({ name, bytes: bytes.length, sha256: hash(bytes) }))
  }, null, 2) + '\n'))
  const checksums = [...files].sort(([a], [b]) => a.localeCompare(b)).map(([name, bytes]) => hash(bytes) + '  ' + name).join('\n') + '\n'
  files.set('SHA256SUMS', Buffer.from(checksums))
  assert(!fs.existsSync(output), '输出目录必须全新，不能混入旧制品')
  fs.mkdirSync(output, { recursive: true })
  for (const [name, bytes] of files) fs.writeFileSync(path.join(output, name), bytes)
  return [...files.keys()].sort()
}
if (require.main === module) {
  const [directory, output, tag] = process.argv.slice(2)
  assert(directory && output && tag, '用法：node tools/release-assets.cjs 制品目录 全新输出目录 v版本')
  const commit = cp.execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
  console.log(JSON.stringify({ commit, tag, assets: prepare(path.resolve(directory), path.resolve(output), commit, tag) }))
}
module.exports = { prepare }
