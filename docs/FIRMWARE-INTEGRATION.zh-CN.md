# 通过标准 Feed 编译进 OpenWrt 固件

本仓库提供 OpenWrt 标准包配方：添加 Feed、更新并安装包索引，在 menuconfig 中选中 `wlmb-server`，即可随固件编译。**直接从当前 Feed 的 Rust 源码编译，不下载或解包 Release，不需要手写 Makefile。**

插件后端、LuCI WebUI、rpcd 入口、ACL、初始化脚本、默认配置及许可声明一起打包。继续使用 `wlmb-server` 包名，避免与原有一体安装包产生前后端文件归属冲突；不是单独叫 `luci-app-wlmb-server` 的第二个包。

## 快速开始

### 1. 添加 Feed

在 **OpenWrt／ImmortalWrt 完整源码树根目录**的 `feeds.conf.default` 中追加一行；如果项目已经有生效的 `feeds.conf`，则添加到该文件。两者不要重复配置：

```text
src-git wlmb https://github.com/SlAlot/wlmb-server.git;main
```

### 2. 更新与安装 Feed

首次准备源码树时，更新并安装全部已有 Feed，让 `packages` 中的 Rust 构建辅助、`luci` 和系统依赖一并就绪：

```sh
./scripts/feeds update -a
./scripts/feeds install -a
make menuconfig
```

已有完整构建环境、只是新增本插件时，可以只更新该 Feed：

```sh
./scripts/feeds update wlmb
./scripts/feeds install -a -p wlmb
make menuconfig
```

第二种写法以其他 Feed 已更新、依赖可解析为前提。若出现缺少 `rust/host`、`luci-base` 或 Rust 辅助文件，请按首次准备流程更新依赖，不忽略错误。

### 3. 选择插件

先选择真实设备的 **Target System／Subtarget／Target Profile**，再进入：

```text
LuCI → Applications → wlmb-server
```

将网络面板设为 **`<*>`**，保存配置。`<M>` 只生成安装包，不会把插件预装进固件。包配方会选择 `luci-base`、`rpcd`、`rpcd-mod-iwinfo`、`ubus`、`uci` 和运行库；固件还需有自己的 LuCI 登录／Web 服务方案，例如原有的 `luci` 或 `luci-ssl` 集合。

### 4. 编译固件

```sh
make defconfig
make download -j8
make -j"$(nproc)"
```

只想先验证插件包，可以运行：

```sh
make package/feeds/wlmb/wlmb-server/compile V=s
```

最终固件位于 `bin/targets/`；检查镜像的 manifest 包含 `wlmb-server`。IPK／APK 格式、包 ABI、依赖及签名由**目标固件自己的构建系统**决定，不使用项目 Release 的签名私钥，也不需要先导入项目的 APK 公钥才能从源码构建。

## 构建条件

- 使用 Linux 或配置好的 WSL2／Linux 虚拟机，按目标固件的开发文档准备完整构建环境，不在路由器本机编译固件。
- 当前配方仅开放 **ARM64（`aarch64`）和 x86_64**。ARM64 的具体 ABI 由目标决定，包括常见的 Cortex-A53、armsr/armv8；不要把 Rust 二进制包改成 `all`，也不宣称其他 CPU 已经过验证。
- 源码采用 Rust 2024 edition，`Cargo.toml` 声明最低 Rust 1.85；同时需满足锁定依赖要求。配方使用 OpenWrt `packages/lang/rust/rust-package.mk` 与 `rust/host`，启用 `pushplus-https`、保留 Cargo.lock。Rust/Cargo 是构建主机依赖，不会安装到路由器。
- 保留与固件分支匹配的默认 `packages`、`luci` Feed。旧快照若自带过时 Rust，请先更新同分支工具链；不混用其他分支的系统库、内核驱动，不自动改用主机 rustup 或预编译程序掩盖错误。
- 首次构建 Rust 工具链与下载 Cargo 依赖可能耗时较长；首次构建需要网络，不承诺离线缓存尚未准备时可直接编译。
- 模组仍需适配当前内核的 USB、串口、QMI、MBIM、MHI 等驱动及拨号组件。插件不替代驱动，不接管拨号；QModem 不是硬依赖，存在已适配共享接口时复用，否则按独立模式的保护规则发现端点。

## 配方与源码对应

[包配方](../openwrt/wlmb-server/Makefile)位于 `openwrt/wlmb-server/Makefile`。Feed 安装后的包目录是符号链接，配方会解析真实 Feed 根目录，再从仓库的 `backend/`、`frontend/`、`packaging/` 和所需数据文件准备源码。

修改实现后，修改的是这份 Feed 中的源码，不是另行下载的发布载荷。需要强制重建时：

```sh
make package/feeds/wlmb/wlmb-server/clean
make package/feeds/wlmb/wlmb-server/compile V=s
```

`main` 会持续变化；对外发布固件前，记录 OpenWrt 提交、各 Feed 提交、`.config` 和构建日志。使用 `src-git` 的固定提交形式可提高可复现性：

```text
src-git wlmb https://github.com/SlAlot/wlmb-server.git^完整提交SHA
```

选择包含本配方的实际提交，不要照抄占位符；旧 `v1.1.3` 标签早于标准 Feed 配方，不能用旧标签验证新集成方式。更新源码版本时保持 Cargo、配方、前端资源与 API 契约一致，见[构建与发布](BUILD.md)。

## 刷机验收

先确认镜像与硬件匹配，保留设备救援与配置备份方案，再按目标固件的刷机流程操作。首次启动后，进入 **LuCI → 服务 → 网络面板**。

```sh
wlmb-server --version
/etc/init.d/wlmb-server status
uci -q get network_panel.main.boot_enabled
ubus -v list network-panel
logread -e wlmb-server
```

默认 `boot_enabled='1'`，初始化脚本交给 procd 管理；固件根文件系统生成阶段不会在构建主机上启动服务。检查服务与页面正常、整机重启后可自启。LuCI 使用该固件自己的管理端口，API／小程序接入使用真实 LAN 地址和插件的 `1117` 端口，只向可信局域网开放。

按本机真实能力验收信号、载波、邻区、WiFi和短信，不为测试自动锁频、切飞行模式或重启模组。编译成功不代表所有模组、所有接口模式已通过实机验证，见[兼容说明](MODEM-COMPATIBILITY.md)。

内置到只读固件后，卸载不会回收只读层空间，恢复出厂通常会恢复内置版本。请测试后续升级、配置保留和恢复出厂行为，不打包个人密码、短信、Token 或运行缓存。

## 常见问题

- **menuconfig 中找不到包**：确认当前生效的 Feed 配置正确，已更新并安装 `wlmb`，目标架构为 ARM64 或 x86_64，并查看 Feed 索引报错。
- **Rust 辅助或构建依赖缺失**：检查 `feeds/packages/lang/rust/rust-package.mk`，重新更新匹配分支的 `packages` Feed 并安装依赖。
- **Rust 版本不足／Cargo 下载失败**：按实际日志处理工具链或网络问题，不删除 Cargo.lock，不静默切换 Release 二进制。
- **只生成安装包，没有进入固件**：确认配置为 `CONFIG_PACKAGE_wlmb-server=y`，不是 `=m`，并查看固件 manifest。
- **包已内置但没有页面**：检查固件自身的 LuCI Web 服务、菜单、ACL、rpcd 和初始化日志；插件没有强制替换固件的 Web 服务。
- **其他 Feed 也提供同名包**：先确认文件来源，只保留一个 `wlmb-server` 配方，不靠强制覆盖掩盖冲突。

## 许可与参考

集成及分发时保留 SlAlot 署名、项目来源、[项目许可](../LICENSE)和第三方声明。允许免费非商业使用，未经授权不得用于商业固件或产品；不是 MIT／GPL 许可。微信小程序源码不属于本仓库，不参与固件构建。

Feed 接入形式参考 [QModem 快速开始](https://github.com/FUjr/QModem/blob/main/README.zh-cn.md)；包规则和 Rust 构建采用 [OpenWrt 软件包体系](https://openwrt.org/docs/guide-developer/packages)与[官方 Rust 辅助](https://github.com/openwrt/packages/blob/openwrt-24.10/lang/rust/rust-package.mk)。这不表示复制 QModem 的拨号或模组控制逻辑。
