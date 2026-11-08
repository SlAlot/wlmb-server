# 网络面板 1.1.7

- 更新 LuCI 顶部导航及邻区筛选，适配手机窄屏与桌面，支持键盘切换。
- 首页新增综合信号评分，以 RSRP、SINR 和可用的 RSRQ 计算。载波和带宽单独显示；保留 RSSI 及设备上报的 NSA 独立测量。
- 保留 1.1.6 的网络制式选择、原有 1117 接口与电池状态接口。具体能力以模组、固件和通道为准。

## 手动安装

用 `opkg print-architecture` 或 `apk --print-arch` 查看设备 ABI。发布附件提供 `aarch64_cortex-a53`（文件名后缀为 `aarch64`）、`aarch64_generic`（包括 armsr/armv8）和 `x86_64`。OPKG 固件下载 IPK，APK 固件下载已签名 APK；首次手动安装 APK 时，按附件 `INSTALL-APK.md` 核对并导入发布公钥。

[安装指南](https://github.com/SlAlot/wlmb-server/blob/v1.1.7/docs/INSTALL.zh-CN.md) · [固件集成](https://github.com/SlAlot/wlmb-server/blob/v1.1.7/docs/FIRMWARE-INTEGRATION.zh-CN.md) · [API](https://github.com/SlAlot/wlmb-server/blob/v1.1.7/docs/API.zh-CN.md)
