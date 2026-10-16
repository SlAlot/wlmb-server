'use strict'
// 仅在通过检查的标签发布任务中使用私钥；不执行制品中的程序，不上传临时密钥。
const fs = require('node:fs'), os = require('node:os'), path = require('node:path'), cp = require('node:child_process'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs')
const { PLATFORMS } = require('./platforms.cjs')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const COMPILER_SHA256 = '2edccd3267ce540f8d2371a0f394e84b40d8348ecc28425309e6d07079ed1259'
function publicKey() {
  const key = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../docs/apk-signing-key.json'), 'utf8'))
  assert.equal(key.schema, 'WLMB-APK-SIGNING-KEY/1')
  assert.equal(key.fileName, 'wlmb-server-signing.pem')
  assert.equal(hash(Buffer.from(key.publicKeyPem)), key.sha256, '公钥文件摘要不匹配')
  const object = crypto.createPublicKey(key.publicKeyPem)
  assert.equal(object.asymmetricKeyType, 'ec')
  assert.equal(object.asymmetricKeyDetails.namedCurve, 'prime256v1')
  return key
}
function sign(directory, compilerPackage, key = publicKey()) {
  assert.equal(process.platform, 'linux', '正式签名仅在隔离的Linux发布任务中执行')
  const secret = process.env.WLMB_APK_SIGNING_KEY
  assert(secret, '缺少签名密钥；拒绝发布未签名APK')
  delete process.env.WLMB_APK_SIGNING_KEY
  const privateKey = crypto.createPrivateKey(secret)
  assert.equal(crypto.createPublicKey(privateKey).export({ type: 'spki', format: 'pem' }), key.publicKeyPem, '私钥必须对应仓库固定公钥')
  assert.equal(hash(fs.readFileSync(compilerPackage)), COMPILER_SHA256, 'APK工具摘要不匹配')
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'wlmb-apk-sign-'))
  fs.chmodSync(stage, 0o700)
  try {
    const tool = path.join(stage, 'apk.static'), trusted = path.join(stage, 'trusted'), empty = path.join(stage, 'empty'), privateFile = path.join(stage, 'signing.key')
    fs.mkdirSync(trusted);fs.mkdirSync(empty)
    fs.writeFileSync(tool, cp.execFileSync('tar', ['-xOf', compilerPackage, 'sbin/apk.static'], { maxBuffer: 16000000 }), { mode: 0o700 })
    fs.writeFileSync(privateFile, secret, { mode: 0o600 })
    fs.writeFileSync(path.join(trusted, key.fileName), key.publicKeyPem)
    const run = args => cp.execFileSync(tool, args, { stdio: ['ignore', 'pipe', 'pipe'] })
    const reject = file => {
      let failed = false
      try { run(['verify', '--keys-dir', empty, file]) } catch { failed = true }
      assert(failed, '没有可信公钥时必须拒绝安装包')
    }
    for (const { id: arch, architecture, packageArchitecture, target } of PLATFORMS) {
      const pkg = path.join(directory, 'wlmb-packages-' + target)
      const file = path.join(pkg, `wlmb-server-${VERSION}-${arch}.apk`), manifestFile = path.join(pkg, 'manifest-' + arch + '.json')
      const manifest = JSON.parse(fs.readFileSync(manifestFile, 'utf8')), before = fs.readFileSync(file)
      assert.equal(manifest.version, VERSION);assert.equal(manifest.architecture, architecture)
      assert.equal(manifest.packageArchitecture, packageArchitecture, '签名包必须匹配指定固件 ABI')
      assert.equal(hash(before), manifest.packageSha256);assert.equal(before.length, manifest.packageBytes)
      run(['verify', '--allow-untrusted', '--keys-dir', empty, file])
      // 签名工具需要允许读取尚未签名的输入；生成后必须独立严格验签，不用于用户安装。
      run(['adbsign', '--allow-untrusted', '--sign-key', privateFile, file])
      run(['verify', '--keys-dir', trusted, file])
      reject(file)
      const signed = fs.readFileSync(file), bad = path.join(stage, 'tampered.apk')
      fs.writeFileSync(bad, signed)
      // 保留原签名解压容器，修改已认证的包名，不把压缩尾部填充变化当成篡改样本。
      run(['adbsign', '--allow-untrusted', '--compression', 'none', bad])
      run(['verify', '--keys-dir', trusted, bad])
      const tampered = fs.readFileSync(bad), at = tampered.indexOf(Buffer.from('wlmb-server'))
      assert(at >= 8, '必须定位受签名保护的包名')
      tampered[at] ^= 1;fs.writeFileSync(bad, tampered)
      let failed = false
      try { run(['verify', '--keys-dir', trusted, bad]) } catch { failed = true }
      assert(failed, '篡改后的安装包必须无法验签')
      manifest.packageSha256 = hash(signed);manifest.packageBytes = signed.length
      manifest.packageSignature = { format: 'apk-v3', publicKeyFile: key.fileName, publicKeySha256: key.sha256 }
      fs.writeFileSync(manifestFile, JSON.stringify(manifest, null, 2) + '\n')
      console.log(JSON.stringify({ arch, version: VERSION, verified: true, rejectsUntrusted: true, rejectsTampering: true, publicKeySha256: key.sha256 }))
    }
  } finally {
    // 临时目录由本函数创建，私钥不进入源码、安装包或工作流附件。
    fs.rmSync(stage, { recursive: true, force: true })
  }
}
if (require.main === module) {
  const [directory, compilerPackage] = process.argv.slice(2)
  assert(directory && compilerPackage, '需要待签名制品目录及固定APK工具包')
  sign(path.resolve(directory), path.resolve(compilerPackage))
}
module.exports = { publicKey, sign }
