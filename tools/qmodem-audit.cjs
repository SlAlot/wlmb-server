// 开发校验工具；不随插件常驻，不操作供网路由器。
'use strict'
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto')
const { execFileSync } = require('node:child_process')
const root = path.resolve(__dirname, '..')
const base = 'application/qmodem/files/usr/share/qmodem/'
const normalize = value => value.toLowerCase().replaceAll('_', '-').replace(/[^a-z0-9-]/g, '')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const bounded = (directory, name) => {
  if (!name || name.includes('\\') || name.split('/').some(part => !part || part === '..' || part === '.')) throw new Error('Invalid source path')
  const file = path.join(directory, ...name.split('/'))
  if (fs.statSync(file).size > 8 * 1024 * 1024) throw new Error('Oversized source')
  const resolved = fs.realpathSync(file), relative = path.relative(fs.realpathSync(directory), resolved)
  if (relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('Source escapes checkout')
  return fs.readFileSync(resolved)
}
function modelsFrom(document) {
  const inventory = document.modem_support
  if (!inventory || typeof inventory !== 'object' || Array.isArray(inventory)) throw new Error('Unknown support schema')
  const rows = [], seen = new Set()
  for (const [bus, models] of Object.entries(inventory)) {
    if (!['usb', 'pcie', 'device'].includes(bus)) throw new Error('New transport requires review: ' + bus)
    if (!models || typeof models !== 'object' || Array.isArray(models)) throw new Error('Invalid model map')
    if (bus === 'device') continue
    for (const [model, info] of Object.entries(models)) {
      if (!/^[a-zA-Z0-9_-]{1,128}$/.test(model) || !info || typeof info !== 'object') throw new Error('Invalid model metadata')
      for (const name of ['manufacturer', 'platform']) if (typeof info[name] !== 'string' || !/^[a-z0-9_-]{1,40}$/.test(info[name])) throw new Error('Missing protocol identity')
      const key = bus + ':' + normalize(model)
      if (seen.has(key) || rows.length >= 1024) throw new Error('Duplicate/oversized inventory')
      seen.add(key)
      rows.push({ model: normalize(model), vendor: info.manufacturer, platform: info.platform, transport: bus })
    }
  }
  return rows.sort((a, b) => (a.model + ':' + a.transport).localeCompare(b.model + ':' + b.transport))
}
function differences(before, after) {
  const key = row => row.model + ':' + row.transport
  const old = new Map(before.models.map(row => [key(row), row])), next = new Map(after.models.map(row => [key(row), row]))
  const changes = []
  for (const [id, row] of next) {
    if (!old.has(id)) changes.push({ type: 'model-added', ...row })
    else if (JSON.stringify(old.get(id)) !== JSON.stringify(row)) changes.push({ type: 'model-protocol-changed', before: old.get(id), after: row })
  }
  for (const [id, row] of old) if (!next.has(id)) changes.push({ type: 'model-removed', ...row })
  for (const name of new Set([...Object.keys(before.files), ...Object.keys(after.files)])) {
    if (before.files[name] !== after.files[name]) changes.push({ type: name === 'LICENSE' ? 'license-changed' : 'source-changed', path: name })
  }
  return changes
}
function coverage(snapshot, compiled) {
  if (compiled.schema !== 'WLMB-PROTOCOL-COVERAGE/1' || !Array.isArray(compiled.models)) throw new Error('Wrong compiled catalog')
  const local = new Map(compiled.models.map(row => [row.model, row]))
  if (local.size !== compiled.models.length) throw new Error('Duplicate compiled models')
  return snapshot.models.map(upstream => {
    const row = local.get(upstream.model)
    const family = upstream.vendor === 'huawei' ? 'tdtech' : upstream.vendor
    return { ...upstream, identified: !!row, mappingMatches: !!row && row.family === family && row.platform === upstream.platform,
      parser: row?.parser ?? null, state: row?.state ?? 'missing', features: row?.features ?? {}, hardwareAccepted: row?.hardwareAccepted === true }
  })
}
function snapshot(directory) {
  const git = args => execFileSync('git', ['-C', directory, ...args], { encoding: 'utf8', timeout: 20000, maxBuffer: 4 * 1024 * 1024 }).trim()
  const revision = git(['rev-parse', 'HEAD'])
  if (!/^[a-f0-9]{40}$/.test(revision) || git(['status', '--porcelain', '--untracked-files=no'])) throw new Error('Audit requires a clean committed upstream checkout')
  const candidates = git(['ls-files']).split('\n').filter(name => name === 'LICENSE' || name === base + 'modem_support.json' || name === base + 'modem_port_rule.json' || name === base + 'modem_scan.sh' || name === base + 'generic.sh' || name.startsWith(base + 'vendor/') || name.startsWith(base + 'cmds/') || /^application\/(?:qmodem_smsd|libqmodem-sms|modem_scan|tom_modem|ubus_at_daemon|sms-tool_q)\/(?:(?:src|files)\/|Makefile$|LICENSE)/.test(name))
  if (candidates.length > 512 || !candidates.includes('LICENSE')) throw new Error('Unexpected upstream tree')
  const blob = name => {
    bounded(directory, name)
    return execFileSync('git', ['-C', directory, 'show', revision + ':' + name], { timeout: 20000, maxBuffer: 8 * 1024 * 1024 })
  }
  const files = Object.fromEntries(candidates.sort().map(name => [name, hash(blob(name))]))
  return { schema: 'WLMB-QMODEM-REFERENCE/1', repository: 'https://github.com/FUjr/QModem', revision,
    scope: 'hardware-facts-and-source-hashes-only', hashMode: 'committed-git-blob-sha256', models: modelsFrom(JSON.parse(blob(base + 'modem_support.json'))), files }
}
function main(args) {
  const options = {}
  for (let i = 0; i < args.length; i += 2) {
    if (!['--upstream', '--binary', '--report', '--accept-revision'].includes(args[i]) || !args[i + 1] || options[args[i]]) throw new Error('Use --upstream CHECKOUT --binary LOCAL_BINARY [--report JSON] [--accept-revision EXACT_SHA]')
    options[args[i]] = args[i + 1]
  }
  if (!options['--upstream'] || !options['--binary']) throw new Error('Explicit upstream checkout and local compiled binary required')
  const current = snapshot(path.resolve(options['--upstream']))
  const binary = path.resolve(options['--binary'])
  const relative = path.relative(path.join(root, 'backend', 'target'), binary)
  if (relative.startsWith('..') || path.isAbsolute(relative) || !/^wlmb-server(?:\.exe)?$/.test(path.basename(binary))) throw new Error('Binary must be built inside backend/target')
  const compiled = JSON.parse(execFileSync(binary, ['--catalog'], { encoding: 'utf8', timeout: 10000, maxBuffer: 1024 * 1024 }))
  const baselineFile = path.join(root, 'docs/qmodem-upstream.json')
  const baseline = fs.existsSync(baselineFile) ? JSON.parse(fs.readFileSync(baselineFile, 'utf8')) : null
  const rows = coverage(current, compiled), drift = baseline ? differences(baseline, current) : [{ type: 'baseline-missing' }]
  const report = { schema: 'WLMB-QMODEM-AUDIT/1', baselineRevision: baseline?.revision ?? null, upstreamRevision: current.revision,
    noRouterAccess: true, runtimeDependency: 'none', drift, summary: { transportProfiles: rows.length, models: new Set(rows.map(row => row.model)).size,
      identityGaps: rows.filter(row => !row.identified).length, mappingGaps: rows.filter(row => !row.mappingMatches).length,
      pendingRadioProfiles: rows.filter(row => row.features.radio !== 'fixture-partial').length, hardwareAcceptedProfiles: rows.filter(row => row.hardwareAccepted).length }, coverage: rows }
  if (options['--accept-revision']) {
    if (options['--accept-revision'] !== current.revision) throw new Error('Baseline update requires the exact reviewed revision')
    fs.writeFileSync(baselineFile, JSON.stringify(current, null, 2) + '\n')
  }
  if (options['--report']) fs.writeFileSync(path.resolve(options['--report']), JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ ...report, coverage: undefined }, null, 2))
}
module.exports = { modelsFrom, differences, coverage }
if (require.main === module) {
  try { main(process.argv.slice(2)) } catch (error) { console.error(error.message); process.exitCode = 1 }
}
