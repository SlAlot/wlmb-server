// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict')
const root = path.resolve(__dirname, '..')
const {VERSION,VIEW}=require('./package-source.cjs')
const ignored = new Set(['.git', 'target', 'node_modules', 'tmp', 'dist'])
function walk(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (ignored.has(entry.name)) return []
    const file = path.join(directory, entry.name)
    assert.ok(!entry.isSymbolicLink(), 'Source must not contain symlinks: ' + file)
    return entry.isDirectory() ? walk(file) : [file]
  })
}
const files = walk(root), relative = file => path.relative(root, file).split(path.sep).join('/')
for (const file of files) {
  const name = relative(file)
  assert.ok(!/\.(?:py|pyc|pyo)$/i.test(name), 'Legacy backend prohibited: ' + name)
  assert.ok(!/^(?:package|rust|repository|release)\//.test(name), 'Legacy layout prohibited: ' + name)
  assert.ok(!/^(?:mini-program|license-server|cloudfunctions)\//.test(name), 'Commercial source excluded: ' + name)
  const text = fs.readFileSync(file, 'utf8')
  assert.ok(!/gh[opusr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|--token=[A-Za-z0-9_-]{20,}/.test(text), 'Possible secret: ' + name)
  if (/\.md$/.test(name)) assert.ok(!/datad/i.test(text), 'Only QModem reference should appear in documentation: ' + name)
  if (VERSION === '1.0.0' && /\.md$/.test(name)) assert.ok(!/\b1\.0\.[123]\b|\br44\b|PROGRESS\.md|私有同名包|私有修订|候选版本/.test(text), '首个公开版文档不能包含内部版本记录：' + name)
}
assert.ok(files.some(file => relative(file) === 'backend/src/native_ubus.rs'))
const contract = JSON.parse(fs.readFileSync(path.join(root, 'docs/api-contract.json'), 'utf8'))
assert.equal(contract.backendLanguage, 'Rust'); assert.equal(contract.port, 1117)
const boundary = fs.readFileSync(path.join(root, 'backend/src/boundary.rs'), 'utf8')
assert.deepEqual([...new Set([...boundary.matchAll(/"(\/api\/[a-z_]+)"/g)].map(row => row[1]))].sort(), Object.values(contract.endpoints).filter(row => row.verb === 'POST').map(row => row.path).sort())
assert.ok(!contract.reservedFeatures.includes('simSlot'))
assert.equal(contract.pluginVersion,VERSION)
assert.equal(contract.releaseStatus, VERSION.includes('-') ? 'prerelease' : 'stable', '接口文档发布状态必须与版本一致，不能遗留私有版标记')
const menu=JSON.parse(fs.readFileSync(path.join(root,'frontend/usr/share/luci/menu.d/network-panel.json'),'utf8'))
assert.equal(menu['admin/services/wlmb-server'].action.path,'wlmb-server/'+VIEW,'Version the JS entrypoint to bypass old heuristic cache')
const view = fs.readFileSync(path.join(root, 'frontend/www/luci-static/resources/view/wlmb-server/'+VIEW+'.js'), 'utf8')
assert.ok(view.includes("L.resource('wlmb-server/panel.css') + '?v="), 'Append CSS query after LuCI path validation')
assert.ok(!/L\.resource\(['"][^'"]+\?/.test(view), 'LuCI rejects query strings inside resource paths')
const css = fs.readFileSync(path.join(root, 'frontend/www/luci-static/resources/wlmb-server/panel.css'), 'utf8')
assert.ok(css.includes('.np-card{') && css.includes('#wlmb-panel.np-root'))
assert.ok(css.includes('prefers-color-scheme'))
assert.ok(css.includes('[data-np-appearance=light]') && css.includes('[data-np-appearance=dark]'))
assert.ok(!files.some(file=>/webui-(?:reference|vendor)-|import-(?:webui|reference-pages)\.cjs|frontend-(?:pages|vendor)\.json/.test(relative(file))), 'Removed historical assets must not return to current source')
console.log(`Rust-only plugin layout: ${files.length} source/document files; contract and stylesheet path verified. Browser and device acceptance are separate.`)
