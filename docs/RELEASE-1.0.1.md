# 网络面板 1.0.1

本次主要改进锁小区体验。默认直接填写PCI和ARFCN／EARFCN，或从当前小区、邻区快速填入；不必先展开高级设置。

- 只展示当前模组协议真正需要的补充参数，共享与独立模式规则一致。
- 唯一候选频段、已报告的SCS自动填写。重叠频段、缺失SCS仍需确认，不伪造参数。
- 无需频段／SCS的协议可仅提交PCI和服务频点；支持手动填写不在邻区列表里的小区。
- 保留操作确认、保存回读及射频状态区分，不自动切飞行模式、拨号或重启。

安装包为ARM64（aarch64_cortex-a53）和x86_64的IPK／APK。以实际包管理器和ABI为准；APK未签名，须核对可信来源及SHA256SUMS后显式允许安装。

此版本没有把未适配协议或未验证硬件改称已支持。实际可用功能以能力门禁和兼容报告为准。小程序仍为闭源客户端，不包含在仓库内。

免费非商业使用须保留SlAlot署名、来源和许可，未经书面授权禁止商用。第三方原有许可不变。

[项目说明](https://github.com/SlAlot/wlmb-server/blob/v1.0.1/README.md) · [API](https://github.com/SlAlot/wlmb-server/blob/v1.0.1/docs/API.zh-CN.md) · [兼容范围](https://github.com/SlAlot/wlmb-server/blob/v1.0.1/docs/MODEM-COMPATIBILITY.md)
