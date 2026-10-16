# 网络面板 1.0.9

修复 armsr/armv8 无法安装的问题，新增其使用的 aarch64_generic 包架构。保留 Cortex-A53 和 x86_64 安装包，不覆盖旧版发布。

- 新增 wlmb-server-1.0.9-aarch64_generic.ipk 和 .apk，分别用于 opkg 与 apk 固件。
- 官方 install.sh、加速 install-gh-proxy.sh、仅检查和离线模式均按固件包 ABI 选包，内置对应摘要，不绕过架构或签名检查。
- GitHub Actions 自动构建三种 ABI 的六个安装包；两种 ARM64 包共用通用静态 Rust 程序，分别生成正确的包元数据、签名和清单。
- 增加通用 ARM64 的选包、下载、损坏拒绝、验签、ABI 歧义拒绝及发布完整性回归。ARM64 程序经 QEMU 执行版本查询，不将模拟器测试宣称为真实 armsr/armv8 设备验收。

一键安装命令不变，请重新下载新版脚本：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

可选加速入口：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

第三方代理不是项目自营服务，首次脚本仍需可信来源。安装保留插件配置，不修改拨号、WiFi、QModem或防火墙配置，不重启设备。本版不扩大模组协议及硬件实测范围；外部共享通道争用仍遵循已有兼容边界。小程序及商业授权代码不包含在公开仓库或制品中。
