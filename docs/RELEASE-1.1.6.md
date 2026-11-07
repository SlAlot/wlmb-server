# 网络面板 1.1.6

在 LuCI「服务 → 网络面板 → 设置 → 网络制式」读取并调整模组的选网偏好。选项按型号、固件和通道开放；支持时可选择自动选网、仅 5G、仅 4G、仅 3G 或仅 2G。

- 支持独立模式及已验证的 QModem 共享通道；部分旧版 QModem 可直接使用原生选网接口。
- 只有回读一致才提示已保存，当前实际驻留以信号看板为准。切换可能短暂断网，仅 5G 通常需要 SA 覆盖。插件不会自动重发、拨号或重启。
- 保留 1117 端口、原有状态接口和电池数据。功能已做离线回归，不代表所有固件均完成真机验证。

## 手动安装

先在设备上用 `opkg print-architecture` 或 `apk --print-arch` 确认 ABI。发布附件提供 `aarch64_cortex-a53`（文件名后缀 `aarch64`）、`aarch64_generic`（含 armsr/armv8）和 `x86_64`。使用 OPKG 的固件下载 IPK，使用 APK 的固件下载已签名 APK。首次用 APK 手动安装时按附件 `INSTALL-APK.md` 核对并导入公钥，无需关闭签名验证。

[安装指南](https://github.com/SlAlot/wlmb-server/blob/v1.1.6/docs/INSTALL.zh-CN.md) · [固件集成](https://github.com/SlAlot/wlmb-server/blob/v1.1.6/docs/FIRMWARE-INTEGRATION.zh-CN.md) · [API](https://github.com/SlAlot/wlmb-server/blob/v1.1.6/docs/API.zh-CN.md)
