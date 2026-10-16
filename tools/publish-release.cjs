'use strict'
// 仅发布已校验制品；断点重跑只跳过摘要相同的资产，不覆盖或删除远端内容。
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), assert = require('node:assert/strict'), crypto = require('node:crypto')
const { VERSION } = require('./package-source.cjs')
const hash = bytes => 'sha256:' + crypto.createHash('sha256').update(bytes).digest('hex')
function publish(directory, tag, repository, execute = cp.execFileSync) {
  const gh = args => execute('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
  assert.equal(tag, 'v' + VERSION)
  assert(/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository), '仓库名称无效')
  const release = JSON.parse(fs.readFileSync(path.join(directory, 'release.json'), 'utf8'))
  assert.equal(release.version, VERSION);assert.equal(release.tag, tag)
  const head = execute('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
  assert.equal(release.commit, head)
  const names = fs.readdirSync(directory).sort(), digests = new Map()
  assert.equal(names.length, 17, '只上传完整插件发布清单、公钥及两个版本固定安装器')
  const sums = fs.readFileSync(path.join(directory, 'SHA256SUMS'), 'utf8').trim().split('\n')
  for (const line of sums) {
    const match = /^([a-f0-9]{64})  ([a-zA-Z0-9._-]+)$/.exec(line)
    assert(match && !digests.has(match[2]), '摘要清单格式或重复项异常')
    const digest = hash(fs.readFileSync(path.join(directory, match[2])))
    assert.equal(digest, 'sha256:' + match[1]);digests.set(match[2], digest)
  }
  assert.deepEqual([...digests.keys()].sort(), names.filter(name => name !== 'SHA256SUMS'))
  digests.set('SHA256SUMS', hash(fs.readFileSync(path.join(directory, 'SHA256SUMS'))))
  const prefix = 'repos/' + repository
  const ref = JSON.parse(gh(['api', prefix + '/git/ref/tags/' + tag])).object
  const tagCommit = ref.type === 'tag' ? JSON.parse(gh(['api', prefix + '/git/tags/' + ref.sha])).object.sha : ref.sha
  assert.equal(tagCommit, head, '远端标签必须绑定本次构建提交')
  const read = () => {
    // 标签查询接口不返回草稿；先由命令行定位草稿，再按编号读取资产摘要。
    const metadata = JSON.parse(gh(['release', 'view', tag, '--repo', repository, '--json', 'databaseId']))
    assert(Number.isSafeInteger(metadata.databaseId) && metadata.databaseId > 0, '发布编号无效')
    return JSON.parse(gh(['api', prefix + '/releases/' + metadata.databaseId]))
  }
  let remote
  try { remote = read() } catch (error) {
    if (!/HTTP 404|Not Found \(404\)|release not found/i.test(String(error.stderr))) throw error
    const specific = path.resolve(__dirname, '../docs/RELEASE-' + VERSION + '.md')
    const notes = fs.existsSync(specific) ? specific : path.resolve(__dirname, '../CHANGELOG.md')
    gh(['release', 'create', tag, '--repo', repository, '--verify-tag', '--draft', '--title', '网络面板 ' + VERSION, '--notes-file', notes])
    remote = read()
  }
  for (const item of remote.assets) assert(digests.has(item.name) && item.digest === digests.get(item.name), '远端存在不同或额外资产，拒绝覆盖：' + item.name)
  for (const name of names) {
    if (remote.assets.some(item => item.name === name)) continue
    gh(['release', 'upload', tag, path.join(directory, name), '--repo', repository])
  }
  remote = read()
  assert.equal(remote.assets.length, names.length)
  for (const item of remote.assets) assert.equal(item.digest, digests.get(item.name), item.name)
  gh(['release', 'edit', tag, '--repo', repository, '--draft=false', '--prerelease=false', '--latest'])
  return { tag, commit: head, assets: remote.assets.length }
}
if (require.main === module) {
  const [directory, tag] = process.argv.slice(2)
  assert(directory && tag && process.env.GH_REPO, '需要制品目录、标签与GH_REPO')
  console.log(JSON.stringify(publish(path.resolve(directory), tag, process.env.GH_REPO)))
}
module.exports = { publish }
