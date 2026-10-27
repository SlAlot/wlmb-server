# 构建、下载与发布

如果目标是将插件预装到固件，请看[标准 Feed 集成教程](FIRMWARE-INTEGRATION.zh-CN.md)：添加 `wlmb` Feed，在 `LuCI → Applications` 选择 `wlmb-server`，使用 OpenWrt 的 Rust 工具链从当前 Feed 源码编译，不依赖 Release 安装包。

[正式版下载](https://github.com/SlAlot/wlmb-server/releases/latest)提供ARM64／x86_64的IPK、APK；两个架构均不是all包。附SHA256SUMS、release.json、安装文件清单和逐型号兼容报告。版本、完整提交和摘要以本次Release资产为准。

IPK用于opkg，APK用于apk；包ABI包括aarch64_cortex-a53、aarch64_generic（armsr/armv8）和x86_64。两种ARM64包使用同一通用aarch64-unknown-linux-musl程序，包元数据分别匹配固件ABI，不使用Cortex-A53专用编译指令。其他ABI不可强装。APK首次安装先核对并导入公钥，见[安装说明](APK-INSTALL.zh-CN.md)。摘要不能替代可信来源。

## GitHub Actions

正式 Release 流程直接用锁定 Cargo 依赖交叉编译静态 musl 二进制，再用校验过的打包工具生成 IPK／APK，不构建完整固件。标准 Feed 则走目标固件的 `rust/host` 与 `rust-package.mk`；两条构建路径分开，不能把独立安装包测试当作 Feed 构建成功。

每次源码检查包含 `tools/check-openwrt-feed.cjs`，在 Linux 通过 GNU Make 验证 Feed 符号链接、规则展开、Cargo 参数、文件布局、权限及钩子保护；其中 Cargo 替身仅用于安装规则测试。另提供可手动运行的 [OpenWrt feed SDK build](../.github/workflows/openwrt-feed.yml)，使用官方 SDK Action 对 ARM64／x86_64 进行真实 Feed 构建，产物和日志仅上传为 Actions 附件，不发布 Release、不读取项目签名密钥。首次 SDK Rust 工具链构建可能较慢；以对应运行结果为准，不能将规则测试冒充 SDK 编译成功。

1. main推送或PR执行检查、Rust测试、Clippy及两架构打包，只产生Actions附件。
2. 推送与源码版本一致的vX.Y.Z标签，或发布对应Release，自动执行相同检查。
3. 全部通过后，标签受限的release-signing环境向独立签名步骤提供WLMB_APK_SIGNING_KEY。私钥必须匹配docs/apk-signing-key.json中的固定公钥；验签及无公钥／篡改拒绝测试通过后更新APK摘要。重新核对ELF、版本、HTTPS能力、ABI、摘要和源文件清单。
4. 创建草稿，上传安装包及报告，成功后转成正式Release；失败不覆盖同名资产，也不自动改仓库可见性。

Actions可手动运行：运行引用必须选择现有v标签，填写同一标签，默认只构建，勾选publish才签名发布。main引用不能使用签名环境。后续修改增加版本号，不移动正式标签。Cargo、打包版本、API契约和前端资源入口必须一致。

普通作业只有contents:read，发布步骤才有contents:write；Action固定完整提交。工具核对APK编译器摘要，项目许可及Rust依赖声明随包提供。

正式Release共17项资产：六个安装包、三份安装清单、两份兼容报告、公钥、INSTALL-APK.md、两个安装器、release.json及SHA256SUMS。安装器由已验证的六个安装包生成，内置本版本摘要及固定公钥指纹；安装器本身的摘要也进入SHA256SUMS，首次下载脚本仍需信任官方发布渠道。安装器回归使用临时目录与包管理器替身，不等于硬件安装验收。ARM64程序另经QEMU Cortex-A53执行版本查询，只验证指令基线，不冒充完整固件或模组实测。源码、安装包均不包含私钥；签名临时文件在步骤结束时删除。私钥由维护者在仓库外保管，环境只允许v*标签部署，密钥只能进入该环境的Secret，不能进入变量、日志或构建附件。更换公钥必须同步元数据、指纹、安装说明及签名测试，公告迁移流程；不自动替换用户信任。PR／main的Actions调试包仍未签名，不能冒充正式发布包。

## 本地开发

需要Rust、Node.js 24及目标C工具链。开发依赖不安装到OpenWrt。

```sh
npm ci --prefix tools/ui-tests --ignore-scripts --no-audit --no-fund
node tools/check-project.cjs
node tools/check-openwrt-feed.cjs
node tools/check-comments.cjs
node tools/check-webui.cjs
node tools/check-release-workflow.cjs
node tools/dependency-notices.cjs --check
cargo fmt --manifest-path backend/Cargo.toml --check
cargo clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path backend/Cargo.toml --all-features --locked
```

更新Cargo.lock后运行node tools/dependency-notices.cjs重新生成第三方声明并复核。Windows不覆盖Unix通道测试；Linux CI补模拟端点，仍不等于硬件验收。
