'use strict'
// 固件包架构与处理器架构分开维护；同一通用 ARM64 程序生成不同 ABI 的安装包。
const PLATFORMS = Object.freeze([
  { id: 'aarch64', architecture: 'aarch64', packageArchitecture: 'aarch64_cortex-a53' },
  { id: 'aarch64_generic', architecture: 'aarch64', packageArchitecture: 'aarch64_generic' },
  { id: 'x86_64', architecture: 'x86_64', packageArchitecture: 'x86_64' }
].map(row => Object.freeze({ ...row, target: row.architecture + '-unknown-linux-musl' })))
function platform(architecture, packageArchitecture) {
  const value = PLATFORMS.find(row => row.architecture === architecture && row.packageArchitecture === packageArchitecture)
  if (!value) throw new Error('Package ABI tag mismatch')
  return value
}
module.exports = { PLATFORMS, platform }
