# 网络面板 · wlmb-server

[English](README.en.md) · [下载正式版](https://github.com/SlAlot/wlmb-server/releases/latest) · [API](docs/API.zh-CN.md)

面向OpenWrt的轻量模组信息与管理插件，作者 **SlAlot**。**Rust后端＋独立LuCI WebUI**，默认端口 **1117**，不接管拨号。根据qmodem为参考开发接入，自动选择已适配的模组协议及共享／独立通道。

**1.0.0首个公开版本**。源码公开，允许免费非商业使用，使用及分发须保留作者和来源；**不允许未经授权的商用**。采用[自定义非商业许可](LICENSE)，不是MIT／GPL或OSI认可的开源许可。第三方组件保留各自原有许可。

## 安装

1. 在[Releases](https://github.com/SlAlot/wlmb-server/releases/latest)下载与设备匹配的包。当前提供ARM64（包ABI：`aarch64_cortex-a53`）和x86_64（`x86_64`），不适用于所有CPU／ABI。
2. 按实际包管理器选择：24.10通常为IPK／opkg，25.12通常为APK／apk；自定义固件以自身配置为准。APK不是安卓应用，不要强制忽略架构检查。
3. 校对`SHA256SUMS`后上传安装包，通过系统包管理器安装；进入 **LuCI → 服务 → 网络面板**。安装默认启用服务及开机启动。

命令行示例，文件名按实际下载内容替换：

```sh
# IPK固件
opkg install /tmp/wlmb-server-1.0.0-aarch64.ipk

# APK固件：独立包未签名，确认来源及摘要后才允许安装
apk add --allow-untrusted /tmp/wlmb-server-1.0.0-aarch64.apk
```

缺少依赖时使用固件自身软件源安装LuCI、rpcd、rpcd-mod-iwinfo和uci，不要混用不同固件的软件源。仅在可信局域网使用，不向公网开放1117。

## 功能

- USB／PCIe模组发现及身份核验；有QModem时核验共享接口，否则使用自己的空闲AT端点，不抢占串口。
- 信号、服务小区、载波聚合、邻区、运营商、签约速率、QCI及温度，按真实返回和有效期展示。
- 点选锁频／锁小区、操作确认和保存回读；不自动飞行模式、拨号或重启。
- WiFi名称／密码／启停、制式／加密／带宽／信道、双频同名同密码；在线设备与上网限制。
- 实时速率、今日／周期流量、起始日和限额；没有可靠计数的字段留空。
- 短信收取／发送／删除、独立已读、PushPlus转发；事件优先、默认10秒兜底，关闭相应任务后停止后台收取／投递。
- 深浅主题、手机布局、PC折叠侧栏、统一弹窗与减少动态效果支持。

目录覆盖91个身份、131份总线配置；**不等于全部功能或固件均已实测**。当前温度协议89个、锁频81个、锁小区63个、邻区64个、载波聚合71个，仍受固件和通道限制。见[兼容说明](docs/MODEM-COMPATIBILITY.md)和[逐项复查](docs/RELEASE-AUDIT.md)。

## 配合微信小程序

微信打开「**网络面板助手**」，连接设备WiFi，选择 **openwrt**，填写自己的管理地址，例如`192.168.1.1:1117`，再登录设备。

小程序是独立闭源商业客户端，免费／专业功能以客户端说明为准。插件WebUI不依赖小程序付费授权。本仓库**仅包含OpenWrt插件**，不含小程序、商业Web、授权服务、授权WASM或签名密钥。其他客户端可依据[API](docs/API.zh-CN.md)独立接入；商业使用本插件需另行授权。

## 开发与自动发布

`backend/` Rust；`frontend/` WebUI；`packaging/`安装配置与第三方声明；`tools/`构建和测试；`docs/`API及兼容文档。

```sh
cargo test --locked --all-features --manifest-path backend/Cargo.toml
cargo clippy --locked --all-targets --all-features --manifest-path backend/Cargo.toml -- -D warnings
```

推送与源码版本一致的`vX.Y.Z`标签后，Actions自动检查、交叉编译并生成两架构IPK／APK，核对清单后上传Release，同时提供SHA-256、构建提交和逐型号报告。失败不发布新资产，不覆盖同名资产。见[构建与发布](docs/BUILD.md)。

[更新记录](CHANGELOG.md) · [1.0.0说明](docs/RELEASE-1.0.0.md) · [安全](docs/SECURITY.md) · [性能](docs/PERFORMANCE.md) · [QModem来源](docs/QMODEM.md) · [图标许可](docs/FRONTEND-SOURCES.md)
