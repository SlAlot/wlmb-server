# APK首次安装

**推荐使用[一键安装器](INSTALL.zh-CN.md)**：确认一次后自动选择包、核对公钥并验签安装。以下保留完全手动安装流程。

适用于使用apk的OpenWrt／ImmortalWrt，APK不是安卓应用。IPK用户使用README中的opkg流程。

## 1. 下载

从[1.0.6正式发布页](https://github.com/SlAlot/wlmb-server/releases/tag/v1.0.6)下载APK、`SHA256SUMS`及`wlmb-server-signing.pem`。优先查看`/etc/apk/arch`确认固件包ABI，未配置时才参考`apk --print-arch`：ARM64包ABI为`aarch64_cortex-a53`，x86_64为`x86_64`，其他ABI不可强装。

用自己的文件传输工具将公钥上传到`/tmp/wlmb-server-signing.pem`，或SSH下载固定版本：

```sh
wget -O /tmp/wlmb-server-signing.pem https://github.com/SlAlot/wlmb-server/releases/download/v1.0.6/wlmb-server-signing.pem
```

## 2. 首次信任公钥

信任公钥等于允许签名者提供可修改系统的软件。只从本项目正式发布页获取，在README与公钥资料中交叉核对指纹。公钥可以公开，不是密码；私钥不应下载。

公钥文件SHA-256：

```text
402d9e009fb2064df25dfccec6855b037c039c22c61a95febc9f400d4b53d8ec
```

先执行校验，只有显示`OK`后才继续：

```sh
echo '402d9e009fb2064df25dfccec6855b037c039c22c61a95febc9f400d4b53d8ec  /tmp/wlmb-server-signing.pem' | sha256sum -c -
```

核对成功后导入：

```sh
mkdir -p /etc/apk/keys
cp /tmp/wlmb-server-signing.pem /etc/apk/keys/wlmb-server-signing.pem
chmod 644 /etc/apk/keys/wlmb-server-signing.pem
```

只添加这一个公钥，不替换固件的原有公钥，不关闭签名检查。尚未获得信任的安装包不能自己完成首次信任，包内放公钥也不能解决这个问题。

## 3. 正常上传安装

校对APK摘要与同版`SHA256SUMS`，进入**LuCI → 系统 → 软件包 → 上传软件包**安装，之后进入**服务 → 网络面板**。首次导入后，同一公钥签名的后续版本无需重复导入，也不需要`--allow-untrusted`。

SSH也可以正常验证和安装，文件名按实际下载内容替换：

```sh
apk verify /tmp/wlmb-server-1.0.6-aarch64.apk
apk add /tmp/wlmb-server-1.0.6-aarch64.apk
```

安装默认启动服务和开机自启，不接管拨号。缺少依赖时使用固件自己的软件源，不混用不同固件的软件源。

## 常见问题

- `UNTRUSTED signature`：确认是1.0.3及以后正式签名APK，公钥已导入且指纹正确。旧的未签名包不会因为导入公钥而变可信。
- 摘要不符或`BAD signature`：停止安装并重新下载，不通过忽略检查掩盖损坏。
- GitHub访问失败：电脑下载后上传，不使用不明镜像，不关闭HTTPS检查。
- 恢复出厂后：公钥可能丢失，需要重新导入。
- 卸载插件不会自动删除系统信任公钥。不再信任此项目时可手动删除`/etc/apk/keys/wlmb-server-signing.pem`，不要删除其他公钥。

本项目提供独立安装包，不包含供`apk update`查询的网络面板软件源。

[APK公钥机制](https://github.com/alpinelinux/apk-tools/blob/master/doc/apk-keys.5.scd) · [OpenWrt APK说明](https://openwrt.org/docs/guide-user/additional-software/apk)
