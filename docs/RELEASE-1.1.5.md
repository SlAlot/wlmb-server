# 网络面板 1.1.5

- 新增 OpenWrt 电池状态，通过已有 1117 端口的 `/api/status` 返回 `system.battery` 与 `capabilities.battery`。
- 标准电池节点自动识别；特殊固件可由开发者通过固定格式运行时文件接入，不执行厂商命令，不开放远程写入。
- 区分真实零电量、未知充电状态、无电池、多电池歧义、无效／不可信文件及过期数据，不猜测数值。
- WebUI 设备状态按实际能力显示电量及充电状态，无电池不增加空白占位。
- 提供[开发者电池接入说明](https://github.com/SlAlot/wlmb-server/blob/v1.1.5/docs/BATTERY.zh-CN.md)，更新 API 与前后端资源版本。

继续提供 aarch64_cortex-a53、aarch64_generic、x86_64 的 APK／IPK，以及官方与加速一键安装脚本。保留原发布公钥和验证流程，不覆盖旧包。

不同固件的电池驱动和采集方式可能不同，本版提供统一规范，不等同于所有设备已完成硬件验证。
