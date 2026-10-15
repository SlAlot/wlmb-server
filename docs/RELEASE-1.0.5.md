# 网络面板 1.0.5

修复1.0.4一键安装器在部分OpenWrt／ImmortalWrt上将APK默认CPU架构误认为实际包ABI的问题。旧发布不覆盖。

- APK优先读取`/etc/apk/arch`中的固件包架构；只有未配置时使用`apk --print-arch`默认值。
- 正确处理默认`aarch64`、实际包ABI为`aarch64_cortex-a53`的常见固件；仅接受明确配置支持的ABI，不把所有ARM64设备都当成同一ABI。
- 多架构配置只选择已提供且唯一的匹配项；未知、冲突、跨CPU家族时仍拒绝，不添加架构强制参数。
- 扩展一键安装器回归，覆盖真实固件架构配置、检查模式、未知ABI及歧义拒绝。
- 修正共享队列测试夹具的锁释放方式，避免并行子进程临时继承夹具锁造成偶发失败；生产锁逻辑未改变。

继续提供一条命令安装、离线目录、仅检查及显式同意的自动化入口，固定同版摘要与公钥指纹，APK严格验签。安装器不常驻，不修改拨号、WiFi或防火墙配置，不重启设备。

[一键安装教程](https://github.com/SlAlot/wlmb-server/blob/v1.0.5/docs/INSTALL.zh-CN.md) · [项目说明](https://github.com/SlAlot/wlmb-server/blob/v1.0.5/README.md) · [兼容范围](https://github.com/SlAlot/wlmb-server/blob/v1.0.5/docs/MODEM-COMPATIBILITY.md)

支持的包ABI仍为`aarch64_cortex-a53`与`x86_64`，没有扩大模组协议或硬件实测范围。首次运行安装器仍需信任官方发布来源；公钥与1.0.3相同，小程序仍闭源且仅通过API接入。
