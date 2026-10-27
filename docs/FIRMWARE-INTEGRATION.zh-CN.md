# 将网络面板预装进 OpenWrt 固件

面向自行构建 OpenWrt／ImmortalWrt 固件的开发者。预装后，用户可直接进入 **LuCI → 服务 → 网络面板**，无需再执行一键安装脚本。插件仍不接管拨号，也不需要小程序才能使用 WebUI。

**先说明：本仓库目前不是 OpenWrt feed，没有可直接被 menuconfig 识别的包 Makefile。仅把本仓库克隆到 `package/` 并不能完成集成。** 本文提供两条可操作路径：

- 已有匹配的 ImageBuilder：将正式安装包加入镜像，适合不修改插件的固件作者。
- 已有完整固件源码树：使用本文的本地包配方，将已核验的插件文件交给固件构建系统重新打包，适合需要 menuconfig 选项的项目。

两条路径默认复用正式版 Rust 二进制，**不等于在固件构建过程中重新编译 Rust 源码**。需要修改插件实现时，见后面的“修改 Rust 源码”。

## 一、准备与版本选择

在 Linux 构建环境中操作；Windows 开发者请使用 Linux 虚拟机或配置好的 WSL2。不要在路由器上编译完整固件。准备目标固件要求的构建依赖，以及 `curl`、`tar`、`sha256sum`；以下下载命令使用 Bash。

1. 固定 OpenWrt 分支、设备 Target／Subtarget／Profile 和插件版本，保留构建记录；不要在可复现构建中使用 `releases/latest`。
2. 以目标固件的**包 ABI**选包，而非只看 CPU 商品名。已有同款运行设备时，可参考 `opkg print-architecture`，或 APK 固件的 `/etc/apk/arch`；同时核对构建配置。
3. 包管理器以目标固件为准，不以系统版本号猜测。ImageBuilder 必须与目标固件、设备和包格式匹配，不混入其他分支的软件源或内核模块。

当前正式包的对应关系如下；本文以 `1.1.3` 为固定示例，更新时同步修改版本、下载包和构建记录。

| 目标包 ABI | 发布文件名后缀 | Rust 目标 |
| --- | --- | --- |
| `aarch64_cortex-a53` | `aarch64` | `aarch64-unknown-linux-musl` |
| `aarch64_generic`，常见于 armsr/armv8 | `aarch64_generic` | `aarch64-unknown-linux-musl` |
| `x86_64` | `x86_64` | `x86_64-unknown-linux-musl` |

两个 ARM64 包使用通用 ARM64 程序，但包元数据不同。其他 ABI 暂无对应正式包，不能强制安装或把包声明成 `all` 来绕过检查。

### 下载并校验所选安装包

以下变量需要按目标修改：`WLMB_SUFFIX` 取上表的文件名后缀；ImageBuilder 的 `WLMB_FORMAT` 选 `ipk` 或 `apk`。完整源码树方案统一选 `ipk`，仅用于提取文件，原因见第三节。

```bash
set -euo pipefail
WLMB_VERSION=1.1.3
WLMB_SUFFIX=aarch64
WLMB_FORMAT=ipk
WLMB_ASSETS="$(mktemp -d)"
WLMB_BASE="https://github.com/SlAlot/wlmb-server/releases/download/v${WLMB_VERSION}"
WLMB_FILE="wlmb-server-${WLMB_VERSION}-${WLMB_SUFFIX}.${WLMB_FORMAT}"
cd "$WLMB_ASSETS"
curl -fL --retry 3 "$WLMB_BASE/$WLMB_FILE" -o "$WLMB_FILE"
curl -fL --retry 3 "$WLMB_BASE/SHA256SUMS" -o SHA256SUMS
awk -v name="$WLMB_FILE" '$2 == name { print; found=1 } END { if (!found) exit 1 }' SHA256SUMS | sha256sum -c -
```

只有校验显示 `OK` 才继续。摘要用于发现文件损坏，不能代替对发布来源的信任。安装包、摘要来自同一个固定版本；不要执行下载失败后留下的文件。

## 二、ImageBuilder 预装

### 1. 加入本地软件包

进入已解压的、与目标设备匹配的 ImageBuilder 根目录；以下命令沿用上一步 Bash 会话中的变量：

```bash
mkdir -p packages
cp "$WLMB_ASSETS/$WLMB_FILE" packages/
make info
```

从 `make info` 的结果中选取真实 Profile，不要照抄其他设备的名称。`packages/` 中只保留本次目标需要的网络面板版本和 ABI，避免多版本候选互相干扰。

### 2. APK 的额外信任步骤

**IPK 跳过本小节。** APK ImageBuilder 在构建时也需要信任插件发布公钥，不能只把公钥放进最终镜像的 `FILES` 目录。

```bash
curl -fL --retry 3 "$WLMB_BASE/wlmb-server-signing.pem" -o "$WLMB_ASSETS/wlmb-server-signing.pem"
printf '%s  %s\n' '402d9e009fb2064df25dfccec6855b037c039c22c61a95febc9f400d4b53d8ec' "$WLMB_ASSETS/wlmb-server-signing.pem" | sha256sum -c -
mkdir -p keys files/etc/apk/keys
cp "$WLMB_ASSETS/wlmb-server-signing.pem" keys/wlmb-server-signing.pem
cp "$WLMB_ASSETS/wlmb-server-signing.pem" files/etc/apk/keys/wlmb-server-signing.pem
chmod 644 keys/wlmb-server-signing.pem files/etc/apk/keys/wlmb-server-signing.pem
```

上面的 `keys/` 用于构建时验证，`files/etc/apk/keys/` 用于刷机后验证同一发布密钥签名的升级包。保留固件原有公钥，不替换整个密钥目录。不同分支若调整了 ImageBuilder 的密钥目录，应按该分支的 `APK_KEYS` 设置处理，见本节末尾的上游参考。

公钥指纹以本仓库 [APK 安装说明](APK-INSTALL.zh-CN.md)和[公钥元数据](apk-signing-key.json)为准；将来换钥时重新核对，不盲目沿用示例。不使用 `--allow-untrusted` 或关闭全局签名校验解决错误。

### 3. 生成固件

将 `实际设备Profile` 替换成 `make info` 列出的值：

```bash
WLMB_PROFILE='实际设备Profile'
mkdir -p files
make image PROFILE="$WLMB_PROFILE" \
  PACKAGES="wlmb-server luci rpcd rpcd-mod-iwinfo uci" \
  FILES="$PWD/files"
```

IPK 场景不需要额外文件时也可省略 `FILES` 参数。已有 overlay 时请使用自己的完整目录，合并上述公钥文件，不要覆盖原有文件。已有 `PACKAGES` 清单则追加这些包，保留原有驱动、拨号及网络配置；完整 LuCI 已由固件其他包提供时可按固件方案选择对应 LuCI 集合。

依赖必须来自该 ImageBuilder 对应的软件源。不要将一键安装脚本塞进首次启动任务，也不要在镜像生成失败时改为忽略依赖、架构或签名。输出位于 `bin/targets/`；检查镜像的 manifest 确实包含 `wlmb-server` 和所需依赖。

## 三、完整固件源码树集成

这个方案使用本地包配方，最终由**目标固件自己的构建系统**生成 IPK 或 APK。这里下载 IPK 只是为了取得插件文件树，不是在 APK 固件里安装 IPK，也不会沿用其他系统的软件包数据库。

### 1. 提取插件文件

先按第一节以 `WLMB_FORMAT=ipk` 下载、校验。然后进入你已有的 OpenWrt 源码树根目录，在同一个 Bash 会话执行：

```bash
test "$WLMB_FORMAT" = ipk
test ! -e package/wlmb-server
mkdir -p package/wlmb-server/files
tar -xOf "$WLMB_ASSETS/$WLMB_FILE" data.tar.gz | tar -xz -C package/wlmb-server/files
test -x package/wlmb-server/files/usr/sbin/wlmb-server
test -f package/wlmb-server/files/etc/init.d/wlmb-server
test -f package/wlmb-server/files/etc/config/network_panel
test -L package/wlmb-server/files/usr/libexec/rpcd/network-panel
```

本项目 `1.1.3` 的 IPK 外层是 gzip tar，包含 `data.tar.gz`，不是 `ar` 格式；不要对其执行 `ar x`。未来格式变化时先检查归档结构。仅提取已校验的官方包，不执行未知归档中的程序。

保留整个文件树，包括 LuCI 静态资源、菜单、ACL、rpcd 入口、UCI 默认配置、初始化脚本、项目许可和第三方声明；不要只复制二进制。

### 2. 添加包配方

新建 `package/wlmb-server/Makefile`，内容如下。**命令行开头必须使用 Tab，不是空格**；如果更换了载荷版本，同时修改 `PKG_VERSION`。

```makefile
include $(TOPDIR)/rules.mk

PKG_NAME:=wlmb-server
PKG_VERSION:=1.1.3
PKG_RELEASE:=1
PKG_LICENSE:=LicenseRef-WLMB-NonCommercial
PKG_BUILD_DIR:=$(BUILD_DIR)/$(PKG_NAME)-$(PKG_VERSION)

include $(INCLUDE_DIR)/package.mk

define Package/wlmb-server
  SECTION:=net
  CATEGORY:=Network
  SUBMENU:=WWAN
  TITLE:=Network Panel modem WebUI
  DEPENDS:=+rpcd +rpcd-mod-iwinfo +luci-base +uci
endef

define Package/wlmb-server/description
  Rust modem information service and LuCI WebUI.
endef

define Package/wlmb-server/conffiles
/etc/config/network_panel
endef

define Build/Prepare
	$(INSTALL_DIR) $(PKG_BUILD_DIR)
endef

define Build/Configure
endef

define Build/Compile
endef

define Package/wlmb-server/install
	$(INSTALL_DIR) $(1)
	$(CP) ./files/. $(1)/
	$(INSTALL_DIR) $(1)/etc/rc.d
	$(LN) ../init.d/wlmb-server $(1)/etc/rc.d/S95wlmb-server
	$(LN) ../init.d/wlmb-server $(1)/etc/rc.d/K10wlmb-server
endef

$(eval $(call BuildPackage,wlmb-server))
```

该配方不下载新版本、不执行主机上的插件程序、不重编译 Rust。提取的二进制必须匹配当前 Target 的架构和上述 ABI；默认包架构由固件构建系统确定，不能改成 `all`。切换 Target 时重新选择载荷，不要把同一目录直接用于不兼容的平台。

### 3. 选择并编译

在固件原有 feeds 配置和目标配置上操作，不要覆盖 `.config`：

```bash
make menuconfig
```

在 **Network → WWAN → wlmb-server** 选择 `<*>`。`<M>` 只构建安装包，不会预装进固件。另确认已选适合设备的 LuCI、模组驱动和拨号管理组件；本插件不替代这些组件。

```bash
make defconfig
make package/wlmb-server/compile V=s
make -j"$(nproc)"
```

包编译失败时先查看 `V=s` 的实际错误；并行构建失败时可单线程重试定位，不通过强制跳过错误发布镜像。最终检查 `bin/targets/` 的固件及 manifest。

这个配方将服务的启动链接一并预装；默认配置 `network_panel.main.boot_enabled='1'` 允许开机启动。若固件作者有意关闭开机启动，同时设置该配置并调整自己的服务策略。不要在构建主机上执行 `/etc/init.d/wlmb-server start`。

本地重打包版本由固件作者维护，签名使用其构建系统的密钥，不是本项目的发布私钥。用户后续通过官方 APK 升级时，仍需按安装说明导入官方公钥，并确认包版本排序；自行打包不自动获得官方升级源。

## 四、修改 Rust 源码

若只想预装原版，跳过本节。若修改实现，应重新编译并验证，不能把主机程序改名当作路由器程序。

1. 从本仓库固定标签检出源码，例如 `v1.1.3`；编译和打包版本、前端入口与 API 契约保持一致。
2. 当前源码使用 Rust 2024 edition，最低 Rust `1.85`。准备 Node.js `24`、对应 Rust 目标及目标 C 工具链；旧固件 SDK 的 Rust 版本不一定够用。
3. 按[构建说明](BUILD.md)运行检查与测试，并参考[实际 CI 工具链配置](../.github/workflows/check.yml)。仅执行 `rustup target add` 不会自动装好目标 C 编译器及链接器。
4. 配置好对应工具链后编译，例如 ARM64：

```bash
cargo build --manifest-path backend/Cargo.toml \
  --release --all-features --locked --target aarch64-unknown-linux-musl
```

x86_64 改为 `x86_64-unknown-linux-musl`。保留 `--all-features`，其中包含短信转发需要的 HTTPS 能力。编译完成后，把对应 `backend/target/<Rust目标>/release/wlmb-server` 替换到第三节的 `files/usr/sbin/wlmb-server`，保留可执行权限；前端、配置或资源有修改时也应同步对应文件，而不是沿用旧载荷。

对改版重新执行项目检查、目标架构运行测试及真机验收。当前教程不提供未经验证的、直接在任意 OpenWrt Rust feed 下构建的配方；第三节是预编译载荷集成，不应宣传为完整 Rust 源码交叉编译。

## 五、刷机后的验收

先保留设备救援和配置备份方案，确认镜像适用于该硬件，再按固件自己的流程刷机。不要为了验收自动锁频、重启模组、切飞行模式或改拨号。

在目标设备检查：

```sh
wlmb-server --version
/etc/init.d/wlmb-server status
uci -q get network_panel.main.boot_enabled
ubus -v list network-panel
logread -e wlmb-server
```

- LuCI 中出现“服务 → 网络面板”，页面、图标、深浅主题及刷新正常；LuCI 若不在 80 端口，使用该固件真实端口访问。
- 小程序或其他 API 客户端使用设备真实 LAN 地址与插件的 `1117` 端口，不把 LuCI 端口当作 API 端口；只向可信局域网开放。
- 检查本机真实支持的信号、载波、邻区、WiFi和短信能力；没有可靠数据时不应伪造数值。存在 QModem 时复用现有接口，不额外抢占其 AT 串口。
- 重启整机后验证服务按配置自启；缺少 rpcd 方法时检查 rpcd 入口、ACL、依赖及启动日志，不先重启模组。
- 确认出厂镜像不含作者自己的密码、短信、Token、设备标识或测试缓存。不要把运行设备的整个 `/etc/wlmb-server` 打包给用户。

内置到只读固件的包与运行后安装的包不同：卸载不能从只读层回收空间，恢复出厂通常会恢复内置版本。固件作者应测试升级、配置保留和恢复出厂后的行为。

## 六、分发边界与常见错误

- **menuconfig 找不到插件**：本仓库本身不是 feed，使用第三节的本地包配方，并确认目录位于固件源码树中。
- **缺依赖／架构不符**：核对 Target、包 ABI、包格式和固件自身软件源；不使用忽略依赖或强制架构选项。
- **APK `UNTRUSTED signature`**：ImageBuilder 的构建密钥目录和最终系统密钥目录分别处理；完整源码树重打包则按固件自己的签名机制处理。
- **有菜单但没有信息**：先检查服务、rpcd、模组通道及原管理组件；预装成功不代表每种模组固件的全部能力都已验证。见[系统接口边界](SYSTEM-INTERFACES.md)与[模组兼容说明](MODEM-COMPATIBILITY.md)。
- **分发固件**：保留 SlAlot 作者署名、项目来源、[项目许可](../LICENSE)及第三方声明。当前允许免费非商业使用及分发，未经授权不得用于商业固件或产品；不是 MIT／GPL 许可。小程序不在本仓库中，也不需要打包进固件。

本教程已按当前发布包结构及上游构建入口核对；不代表已在每个 ImageBuilder、固件分支或硬件上完成整机编译与刷机测试。首次集成应走完整验收，再对外分发。

## 参考

- [OpenWrt ImageBuilder 使用说明](https://openwrt.org/docs/guide-user/additional-software/imagebuilder)
- [OpenWrt ImageBuilder 实现：本地包、密钥目录与镜像生成参数](https://github.com/openwrt/openwrt/blob/main/target/imagebuilder/files/Makefile)
- [OpenWrt 软件包定义](https://openwrt.org/docs/guide-developer/packages)
- [网络面板构建与发布](BUILD.md) · [安装与升级](INSTALL.zh-CN.md) · [API 文档](API.zh-CN.md)
