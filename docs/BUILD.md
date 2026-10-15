# 构建、下载与发布

[正式版下载](https://github.com/SlAlot/wlmb-server/releases/latest)提供ARM64／x86_64的IPK、APK；两个架构均不是all包。附SHA256SUMS、release.json、安装文件清单和逐型号兼容报告。版本、完整提交和摘要以本次Release资产为准。

IPK用于opkg，APK用于apk；ARM64包ABI为aarch64_cortex-a53，x86_64包ABI为x86_64。其他ABI不可强装。包未签名，摘要核对不能替代可信下载来源。

## GitHub Actions

已参考[OpenWrt官方SDK Action](https://github.com/openwrt/gh-action-sdk)及[SDK容器](https://github.com/openwrt/docker)。本项目直接用锁定Cargo依赖交叉编译静态musl二进制，再用校验过的打包工具生成IPK／APK，不构建完整固件；没有声称使用SDK Action构建本项目。

1. main推送或PR执行检查、Rust测试、Clippy及两架构打包，只产生Actions附件。
2. 推送与源码版本一致的vX.Y.Z标签，或发布对应Release，自动执行相同检查。
3. 全部通过后重新核对ELF、版本、HTTPS能力、ABI、包摘要和源文件清单。
4. 创建草稿，上传安装包及报告，成功后转成正式Release；失败不覆盖同名资产，也不自动改仓库可见性。

Actions可手动运行：填写现有标签，默认只构建，勾选publish才上传。后续修改增加版本号，不移动正式标签。Cargo、打包版本、API契约和前端资源入口必须一致。

普通作业只有contents:read，发布步骤才有contents:write；Action固定完整提交。工具核对APK编译器摘要，项目许可及Rust依赖声明随包提供。

## 本地开发

需要Rust、Node.js 24及目标C工具链。开发依赖不安装到OpenWrt。

```sh
npm ci --prefix tools/ui-tests --ignore-scripts --no-audit --no-fund
node tools/check-project.cjs
node tools/check-comments.cjs
node tools/check-webui.cjs
node tools/check-release-workflow.cjs
node tools/dependency-notices.cjs --check
cargo fmt --manifest-path backend/Cargo.toml --check
cargo clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path backend/Cargo.toml --all-features --locked
```

更新Cargo.lock后运行node tools/dependency-notices.cjs重新生成第三方声明并复核。Windows不覆盖Unix通道测试；Linux CI补模拟端点，仍不等于硬件验收。
