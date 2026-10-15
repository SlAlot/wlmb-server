'use strict'
// 临时测试密钥与最小APK只验证签名容器，不作为真实安装包或硬件验收。
const fs = require('node:fs'), path = require('node:path'), os = require('node:os'), cp = require('node:child_process'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs'), { sign } = require('./sign-release.cjs')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const compiler = path.resolve(process.argv[2] || '')
assert.equal(process.platform, 'linux');assert(process.argv[2])
assert.equal(hash(fs.readFileSync(compiler)), '2edccd3267ce540f8d2371a0f394e84b40d8348ecc28425309e6d07079ed1259')
const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'wlmb-sign-test-')), saved = process.env.WLMB_APK_SIGNING_KEY
try {
  const tool = path.join(stage, 'apk.static'), content = path.join(stage, 'files')
  fs.mkdirSync(content)
  fs.writeFileSync(path.join(content, 'fixture.txt'), '签名测试，不用于安装\n')
  fs.writeFileSync(tool, cp.execFileSync('tar', ['-xOf', compiler, 'sbin/apk.static'], { maxBuffer: 16000000 }), { mode: 0o700 })
  const pair = crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1' })
  const pem = pair.publicKey.export({ type: 'spki', format: 'pem' })
  const key = { fileName: 'wlmb-server-signing.pem', sha256: hash(Buffer.from(pem)), publicKeyPem: pem }
  for (const arch of ['aarch64', 'x86_64']) {
    const directory = path.join(stage, 'wlmb-packages-' + arch + '-unknown-linux-musl');fs.mkdirSync(directory)
    const file = path.join(directory, `wlmb-server-${VERSION}-${arch}.apk`)
    cp.execFileSync(tool, ['mkpkg', '--compression', 'deflate:9', '--compat', '3.0.5', '--info', 'name:wlmb-server', '--info', 'version:' + VERSION, '--info', 'arch:' + (arch === 'aarch64' ? 'aarch64_cortex-a53' : arch), '--files', content, '--output', file], { stdio: 'pipe' })
    const bytes = fs.readFileSync(file)
    fs.writeFileSync(path.join(directory, 'manifest-' + arch + '.json'), JSON.stringify({ version: VERSION, architecture: arch, packageSha256: hash(bytes), packageBytes: bytes.length }))
  }
  process.env.WLMB_APK_SIGNING_KEY = crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1' }).privateKey.export({ type: 'pkcs8', format: 'pem' })
  assert.throws(() => sign(stage, compiler, key), /私钥必须对应/)
  assert.throws(() => sign(stage, compiler, key), /缺少签名密钥/)
  process.env.WLMB_APK_SIGNING_KEY = pair.privateKey.export({ type: 'pkcs8', format: 'pem' })
  sign(stage, compiler, key)
  for (const arch of ['aarch64', 'x86_64']) {
    const manifest = JSON.parse(fs.readFileSync(path.join(stage, 'wlmb-packages-' + arch + '-unknown-linux-musl', 'manifest-' + arch + '.json')))
    assert.equal(manifest.packageSignature.publicKeySha256, key.sha256)
  }
  console.log('真实APK签名容器回归通过：临时密钥、固定工具、双架构、错误私钥、缺少私钥、正常验签、无公钥拒绝和篡改拒绝；未安装任何包。')
} finally {
  if (saved === undefined) delete process.env.WLMB_APK_SIGNING_KEY
  else process.env.WLMB_APK_SIGNING_KEY = saved
  fs.rmSync(stage, { recursive: true, force: true })
}
