# 网络面板 1.1.1

修复GL.iNet等固件同时列出多个兼容包ABI时，一键安装误报“不支持设备架构”的问题。保留1.1.0的GL.iNet自动识别与功能边界，本版不新增模组写入权限。

- OPKG按固件声明的正优先级依次选择，同分时优先具体架构；APK按持久配置顺序保留同族候选。不会只凭CPU名称猜包。
- 只尝试固件声明支持、项目提供、与内核CPU家族一致的候选。逐个获取并严格验证，通过模拟安装后只正式安装一次；首个成功即停止尝试。
- 候选不可获取或模拟失败时继续下一个；全部失败显示错误。在线最多刷新一次现有源索引后复查，离线不联网补依赖。
- 摘要、APK签名、公钥或跨架构异常立即停止，不以换包跳过安全检查。正式安装失败或服务未就绪时不换候选重装，不强制回滚，也不删除用户配置。
- 官方install.sh、加速install-gh-proxy.sh及离线流程共用修复；--check仍只检查候选架构、摘要及APK签名，不安装、不导入公钥、不模拟依赖安装。

重新下载脚本即可使用新版流程，不要重用此前保存在/tmp的旧脚本：

```sh
wget -O /tmp/wlmb-install.sh https://github.com/SlAlot/wlmb-server/releases/latest/download/install.sh && sh /tmp/wlmb-install.sh
```

可选第三方加速入口：

```sh
wget -O /tmp/wlmb-install-gh-proxy.sh https://gh-proxy.com/https://github.com/SlAlot/wlmb-server/releases/latest/download/install-gh-proxy.sh && sh /tmp/wlmb-install-gh-proxy.sh
```

三种包ABI、六个安装包及APK签名机制不变，不覆盖1.1.0资产。新增离线模拟回归覆盖优先级与回退、双通道、失败终止及单次正式安装；这些测试不代表已在用户设备安装验收。加速站不是项目自营服务，首次脚本仍需可信来源。
