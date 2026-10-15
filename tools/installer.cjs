'use strict'
// 将已核对的正式制品摘要嵌入一次性安装脚本，不使用动态接口拼接安装命令。
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs'), { publicKey } = require('./sign-release.cjs')
function render(files, channel = 'official') {
  assert(['official', 'gh-proxy'].includes(channel), '安装通道必须是已审核的固定入口')
  const key = publicKey(), replacements = { VERSION, KEY_SHA256: key.sha256,
    DOWNLOAD_PREFIX: channel === 'gh-proxy' ? 'https://gh-proxy.com/' : '',
    SCRIPT_NAME: channel === 'gh-proxy' ? 'install-gh-proxy.sh' : 'install.sh' }
  for (const arch of ['aarch64', 'x86_64']) for (const extension of ['apk', 'ipk']) {
    const bytes = files.get(`wlmb-server-${VERSION}-${arch}.${extension}`)
    assert(Buffer.isBuffer(bytes) && bytes.length > 0 && bytes.length < 32 * 1024 * 1024, '缺少已验证的安装包')
    const name = extension.toUpperCase() + '_' + arch.toUpperCase()
    replacements[name + '_SHA256'] = crypto.createHash('sha256').update(bytes).digest('hex')
    replacements[name + '_BYTES'] = String(bytes.length)
  }
  let text = fs.readFileSync(path.resolve(__dirname, 'install-template.sh'), 'utf8').replace(/\r\n/g, '\n')
  for (const [key, value] of Object.entries(replacements)) {
    assert(text.includes('@' + key + '@'), '模板必须使用固定发布资料：' + key)
    text = text.replaceAll('@' + key + '@', value)
  }
  assert(!/@[A-Z0-9_]+@/.test(text), '安装器含有未替换字段')
  return Buffer.from(text)
}
module.exports = { render }
