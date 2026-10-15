'use strict'
// 只验证草稿补齐规则，不连接设备，也不执行锁定。
const fs = require('node:fs'), path = require('node:path'), vm = require('node:vm'), assert = require('node:assert/strict')
const { VERSION } = require('./package-source.cjs')
const context = { window: {} }
vm.runInNewContext(fs.readFileSync(path.join(__dirname, '../frontend/www/luci-static/resources/wlmb-server/webui-data-' + VERSION + '.js'), 'utf8'), context)
const resolve = (input, options) => JSON.parse(JSON.stringify(context.window.WLMBUIData.resolveCellDraft(input, options)))
const input = { rat: 'NR', pci: '0', arfcn: '627264' }
const measured = { rat: 'NR', pci: 0, channel: 627264, band: 'n78', scsType: 1 }
const cases = [
  [input, { choices: [measured] }, { band: 'n78', scsType: 1 }],
  [input, {}, { missingBand: true, missingScs: true }],
  [input, { fields: { bandRequired: false, scsRequired: false } }, {}],
  [{ rat: 'LTE', pci: 503, arfcn: 131972 }, { fields: { bandRequired: false } }, {}],
  [input, { choices: [{ ...measured, stale: true }] }, { missingBand: true, missingScs: true }],
  [input, { choices: [{ ...measured, channelType: 'center' }] }, { missingBand: true, missingScs: true }],
  [input, { choices: [{ ...measured, pci: null }] }, { missingBand: true, missingScs: true }],
  [input, { choices: [measured, { ...measured, band: 'n77', scsType: 0 }] }, { missingBand: true, missingScs: true }],
  [input, { ranges: [{ band: 78, first: 620000, last: 653333 }] }, { band: 'n78', missingScs: true }],
  [input, { ranges: [{ band: 78, first: 620000, last: 653333 }, { band: 77, first: 620000, last: 680000 }] }, { missingBand: true }],
  [{ ...input, band: 'n78', scsType: 0 }, {}, { band: 'n78', scsType: 0 }],
  [{ ...input, band: 'B3' }, {}, { error: true, missingBand: true }],
  [{ ...input, pci: '' }, {}, { error: true }],
  [{ ...input, pci: 1008 }, {}, { error: true }],
  [{ ...input, arfcn: '6e5' }, {}, { error: true }],
  [{ ...input, arfcn: 3279166 }, {}, { error: true }],
  [{ ...input, scsType: 5 }, {}, { error: true }],
  [{ ...input, rat: 'LTE', pci: 504 }, {}, { error: true }]
]
for (const [draft, options, expected] of cases) {
  const result = resolve(draft, options)
  for (const [key, value] of Object.entries(expected)) {
    const actual = key === 'error' ? !!result.error : ['band', 'scsType'].includes(key) ? result.cell?.[key] : result[key]
    assert.equal(actual, value, JSON.stringify({ draft, options, result }))
  }
  if (options.fields?.bandRequired === false) assert(!Object.hasOwn(result.cell, 'band'))
  if (options.fields?.scsRequired === false) assert(!Object.hasOwn(result.cell, 'scsType'))
}
console.log('小区草稿回归通过：18组手填／已读候选、唯一频段、真实SCS、零值、重叠频段、过期与非法输入。')
module.exports = { cases, resolve }
