'use strict'
// 从锁定依赖的原始许可文件生成声明，不把本机缓存路径写进安装包。
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), assert = require('node:assert/strict')
const root = path.resolve(__dirname, '..')
const cargo = process.env.WLMB_CARGO || 'cargo'
const selected = new Map()
for (const target of ['aarch64-unknown-linux-musl', 'x86_64-unknown-linux-musl']) {
  const metadata = JSON.parse(cp.execFileSync(cargo, ['metadata', '--format-version', '1', '--locked', '--all-features', '--filter-platform', target, '--manifest-path', path.join(root, 'backend/Cargo.toml')], { encoding: 'utf8', maxBuffer: 16000000 }))
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node])), packages = new Map(metadata.packages.map(pkg => [pkg.id, pkg]))
  const seen = new Set(), pending = [metadata.resolve.root]
  while (pending.length) {
    const id = pending.pop()
    if (seen.has(id)) continue
    seen.add(id)
    const pkg = packages.get(id)
    if (pkg.name !== 'wlmb-server') selected.set(id, pkg)
    for (const dep of nodes.get(id).deps) pending.push(dep.pkg)
  }
}
const lines = ['Third-party Rust dependency notices / Rust第三方依赖声明', '', '仅列出Cargo.lock锁定的依赖；各项继续遵守原许可，不受本项目非商业条款替代。', 'Locked dependencies retain their original licenses; the project license does not replace them.', '']
for (const pkg of [...selected.values()].sort((a, b) => (a.name + '@' + a.version).localeCompare(b.name + '@' + b.version))) {
  const directory = path.dirname(pkg.manifest_path)
  const names = fs.readdirSync(directory).filter(name => /^(?:LICEN[CS]E|COPYING|NOTICE)(?:[._-].*)?$/i.test(name) && fs.statSync(path.join(directory, name)).isFile()).sort()
  if (pkg.license_file) {
    const name = path.relative(directory, pkg.license_file)
    assert(!name.startsWith('..') && !path.isAbsolute(name), '依赖许可路径越界')
    if (!names.includes(name)) names.push(name)
  }
  assert(names.length, '依赖缺少可分发许可文本：' + pkg.name + '@' + pkg.version)
  lines.push('='.repeat(72), pkg.name + ' ' + pkg.version, 'License: ' + (pkg.license || 'see license file'))
  if (pkg.repository) lines.push('Source: ' + pkg.repository)
  for (const name of names) lines.push('', '--- ' + name.replaceAll('\\', '/') + ' ---', fs.readFileSync(path.join(directory, name), 'utf8').replace(/\r\n/g, '\n').trim())
  lines.push('')
}
const target = path.join(root, 'packaging/files/usr/share/wlmb-server/THIRD-PARTY-NOTICES')
const content = lines.join('\n') + '\n'
if (process.argv.includes('--check')) assert.equal(fs.readFileSync(target, 'utf8').replace(/\r\n/g, '\n'), content, '第三方许可声明与锁定依赖不一致')
else { fs.mkdirSync(path.dirname(target), { recursive: true });fs.writeFileSync(target, content) }
console.log(JSON.stringify({ dependencies: selected.size, licenseBytes: Buffer.byteLength(content), checked: process.argv.includes('--check') }))
